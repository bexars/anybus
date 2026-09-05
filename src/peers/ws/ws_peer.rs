use std::collections::{HashSet, VecDeque};
use std::time::Duration;
use tokio::{select, sync::mpsc};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, error, trace};
use web_time::Instant;

use crate::peers::common::{Heartbeat, Peer};
// use crate::peers::ws::ws_peer::InMessage;
use crate::{
    messages::NodeMessage,
    peers::ws::{WebSockStream, WsCommand, WsControl, WsMessage},
    routing::{Advertisement, NodeId, WirePacket},
};

const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(45);

/// Wire-adapter type. `WebSockStream::next_msg` and the handshake in
/// `ws_manager` still produce this. The state machine does not.
#[derive(Debug)]
pub(crate) enum InMessage {
    // WsControl(WsControl),
    WsMessage(WsMessage),
    // NodeMessage(NodeMessage),
    WsPeerClosed,
    Unknown,
}

#[derive(Debug)]
enum Event {
    Control(WsControl),
    FromWire(WsMessage),
    FromNode(NodeMessage),
    TransportDead,
    UnknownWire,
    Tick(Instant),
}

#[derive(Debug)]
enum Effect {
    Send(WsMessage),
    CloseSocket,
    AddEndpoints(HashSet<Advertisement>),
    RemoveEndpoints(HashSet<Advertisement>),
    Forward(WirePacket),
    Unregister,
    NotifyPeerClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Open,
    Closing,
    Closed,
}

#[derive(Debug, Clone, Copy)]
enum CloseReason {
    /// We are hanging up. Write a close frame first.
    Local,
    /// Peer already said goodbye. Do not write a close frame.
    Remote,
    /// Socket is already dead. Do not touch it.
    Transport,
}

struct WsPeer {
    phase: Phase,
    effects: VecDeque<Effect>,
    peer_id: NodeId,
    // our_id: NodeId,
    // connection_id: u16,
    hb: Heartbeat,
}

impl WsPeer {
    fn new(connection_id: u16, our_id: NodeId, peer_id: NodeId) -> Self {
        Self::new_with_heartbeat(
            connection_id,
            our_id,
            peer_id,
            Instant::now(),
            DEFAULT_HEARTBEAT_INTERVAL,
            DEFAULT_HEARTBEAT_TIMEOUT,
        )
    }

    fn new_with_heartbeat(
        connection_id: u16,
        our_id: NodeId,
        peer_id: NodeId,
        now: Instant,
        interval: Duration,
        timeout: Duration,
    ) -> Self {
        trace!(
            %our_id,
            %peer_id,
            connection_id,
            ?interval,
            ?timeout,
            "Creating new WsPeer"
        );
        Self {
            phase: Phase::Open,
            effects: VecDeque::new(),
            peer_id,
            // our_id,
            // connection_id,
            hb: Heartbeat::new(now, interval, timeout),
        }
    }

    fn is_done(&self) -> bool {
        matches!(self.phase, Phase::Closed) && self.effects.is_empty()
    }

    fn next_wake(&self) -> Option<Instant> {
        if self.phase == Phase::Open {
            Some(self.hb.next_deadline())
        } else {
            None
        }
    }

    fn poll_effect(&mut self) -> Option<Effect> {
        self.effects.pop_front()
    }

    fn effects_flushed(&mut self) {
        if self.phase == Phase::Closing && self.effects.is_empty() {
            self.phase = Phase::Closed;
        }
    }

    fn emit(&mut self, effect: Effect) {
        self.effects.push_back(effect);
    }

    fn step(&mut self, event: Event) {
        trace!(?event, phase = ?self.phase, "WsPeer event");

        match self.phase {
            Phase::Closed => return,
            Phase::Closing => {
                if matches!(event, Event::TransportDead) {
                    self.effects.clear();
                    self.phase = Phase::Closed;
                }
                return;
            }
            Phase::Open => {}
        }

        match event {
            Event::Control(WsControl::Shutdown) => self.close(CloseReason::Local),

            Event::FromWire(msg) => self.on_wire(msg),

            Event::FromNode(NodeMessage::WirePacket(pkt)) => {
                self.emit(Effect::Send(WsMessage::Packet(pkt)));
            }
            Event::FromNode(NodeMessage::Advertise(ads)) => {
                self.emit(Effect::Send(WsMessage::Advertise(ads)));
            }
            Event::FromNode(NodeMessage::Withdraw(ads)) => {
                self.emit(Effect::Send(WsMessage::Withdraw(ads)));
            }
            Event::FromNode(NodeMessage::Close) => self.close(CloseReason::Local),

            Event::TransportDead => self.close(CloseReason::Transport),
            Event::UnknownWire => debug!("unknown websocket frame"),
            Event::Tick(now) => self.on_tick(now),
        }
    }

    fn on_wire(&mut self, msg: WsMessage) {
        self.hb.on_rx(Instant::now());

        match msg {
            WsMessage::Hello(id) => {
                error!(%id, "unexpected Hello after handshake");
                self.close(CloseReason::Local);
            }
            WsMessage::CloseConnection => self.close(CloseReason::Remote),
            WsMessage::Packet(pkt) => self.emit(Effect::Forward(pkt)),
            WsMessage::Advertise(ads) => self.emit(Effect::AddEndpoints(ads)),
            WsMessage::Withdraw(ads) => self.emit(Effect::RemoveEndpoints(ads)),
            WsMessage::Ping(token) => self.emit(Effect::Send(WsMessage::Pong(token))),
            WsMessage::Pong(_token) => {
                // Any inbound already reset silence in on_rx.
            }
        }
    }

    fn on_tick(&mut self, now: Instant) {
        if self.hb.timed_out(now) {
            error!(
                peer_id = %self.peer_id,
                "websocket heartbeat timed out"
            );
            self.close(CloseReason::Local);
            return;
        }
        if self.hb.ping_due(now) {
            let token = self.hb.take_ping_token(now);
            self.emit(Effect::Send(WsMessage::Ping(token)));
        }
    }

    fn close(&mut self, reason: CloseReason) {
        if self.phase != Phase::Open {
            return;
        }

        match reason {
            CloseReason::Local => {
                self.emit(Effect::Send(WsMessage::CloseConnection));
                self.emit(Effect::CloseSocket);
            }
            CloseReason::Remote => {
                self.emit(Effect::CloseSocket);
            }
            CloseReason::Transport => {}
        }

        self.emit(Effect::Unregister);
        self.emit(Effect::NotifyPeerClosed);
        self.phase = Phase::Closing;
    }
}

pub(crate) async fn run_ws_peer(
    mut stream: WebSockStream,
    tx_command: mpsc::Sender<WsCommand>,
    mut rx_control: mpsc::Receiver<WsControl>,
    mut peer: Peer,
) {
    trace!("Entered run_ws_peer");
    let mut fsm = WsPeer::new(peer.connection_id, peer.our_id, peer.peer_id);
    let mut follow_up: Option<Event> = None;

    loop {
        while let Some(effect) = fsm.poll_effect() {
            trace!(?effect, "applying effect");
            if apply_effect(effect, &mut stream, &tx_command, &mut peer)
                .await
                .is_err()
            {
                follow_up = Some(Event::TransportDead);
                break;
            }
        }

        if follow_up.is_none() {
            fsm.effects_flushed();
        }

        if fsm.is_done() {
            break;
        }

        let event = if let Some(event) = follow_up.take() {
            event
        } else {
            let wake = fsm.next_wake();
            select! {
                msg = stream.next_msg() => wire_to_event(msg),
                Some(msg) = rx_control.recv() => Event::Control(msg),
                Some(msg) = peer.recv() => Event::FromNode(msg),
                _ = sleep_until(wake) => Event::Tick(Instant::now()),
            }
        };

        fsm.step(event);
    }
}

fn wire_to_event(msg: InMessage) -> Event {
    match msg {
        InMessage::WsMessage(m) => Event::FromWire(m),
        InMessage::WsPeerClosed => Event::TransportDead,
        InMessage::Unknown => Event::UnknownWire,
        // InMessage::WsControl(_) | InMessage::NodeMessage(_) => Event::UnknownWire,
    }
}

async fn sleep_until(wake: Option<Instant>) {
    match wake {
        Some(deadline) => {
            let now = Instant::now();
            if deadline > now {
                tokio::time::sleep(deadline - now).await;
            }
        }
        None => futures::future::pending::<()>().await,
    }
}

async fn apply_effect(
    effect: Effect,
    stream: &mut WebSockStream,
    tx_command: &mpsc::Sender<WsCommand>,
    peer: &mut Peer,
) -> Result<(), ()> {
    match effect {
        Effect::Send(msg) => stream.send_msg(msg).await.map_err(|e| {
            error!("Failed to send WsMessage: {e} to {}", peer.peer_id);
        }),
        Effect::CloseSocket => {
            let _ = stream.close_conn().await;
            Ok(())
        }
        Effect::AddEndpoints(ads) => {
            peer.add_endpoints(ads);
            Ok(())
        }
        Effect::RemoveEndpoints(ads) => {
            peer.remove_endpoints(ads);
            Ok(())
        }
        Effect::Forward(pkt) => {
            peer.send_packet(pkt);
            Ok(())
        }
        Effect::Unregister => {
            peer.unregister();
            Ok(())
        }
        Effect::NotifyPeerClosed => tx_command
            .send(WsCommand::PeerClosed(peer.peer_id))
            .await
            .map_err(|e| {
                error!("Failed to send WsCommand: {e}");
            }),
    }
}
