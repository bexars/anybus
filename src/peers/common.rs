use crate::tokio;
use crate::tokio::sync::mpsc;
use crate::tokio::time::Instant;
use std::time::Duration;

use crate::{
    Handle, Realm,
    messages::{NodeMessage, RouterMsg},
    routing::{ConnectionId, Cost, NodeId, PeerEntry, RealmList, WirePacket},
};

#[derive(Debug)]
pub(crate) struct Heartbeat {
    interval: Duration,
    timeout: Duration,
    last_rx: Instant,
    last_ping: Option<Instant>,
    outstanding: Option<u64>,
    next_token: u64,
}

impl Heartbeat {
    pub(crate) fn new(now: Instant, interval: Duration, timeout: Duration) -> Self {
        Self {
            interval,
            timeout,
            last_rx: now,
            last_ping: None,
            outstanding: None,
            next_token: 1,
        }
    }

    pub(crate) fn on_rx(&mut self, now: Instant) {
        self.last_rx = now;
        self.last_ping = None;
        self.outstanding = None;
    }

    /// Local clock jumped (suspend/resume). Do not treat the gap as peer silence.
    pub(crate) fn note_resume(&mut self, now: Instant) {
        self.on_rx(now);
    }

    /// When the driver should next call `Tick`.
    ///
    /// The clock is `interval` (next poke). `timeout` is only a silence
    /// limit, but we still wake by then so we do not oversleep it.
    pub(crate) fn next_deadline(&self) -> Instant {
        let poke_at = match self.last_ping {
            Some(sent) => sent + self.interval,
            None => self.last_rx + self.interval,
        };
        let die_at = self.last_rx + self.timeout;
        poke_at.min(die_at)
    }

    pub(crate) fn timed_out(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_rx) >= self.timeout
    }

    pub(crate) fn ping_due(&self, now: Instant) -> bool {
        let due = match self.last_ping {
            Some(sent) => sent + self.interval,
            None => self.last_rx + self.interval,
        };
        now >= due
    }

    pub(crate) fn take_ping_token(&mut self, now: Instant) -> u64 {
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1);
        self.last_ping = Some(now);
        self.outstanding = Some(token);
        token
    }
}

#[derive(Debug)]
#[allow(unused)]
pub(crate) struct Peer {
    pub(crate) peer_id: NodeId,
    pub(crate) our_id: NodeId,
    rx_node: mpsc::Receiver<NodeMessage>,
    handle: Handle,
    pub(crate) realm: Realm,
    pub(crate) connection_id: ConnectionId,
    pub(crate) cost: Cost,
    pub(crate) stats: PeerStats,
    pub(crate) realms: RealmList,
    pub(crate) name: String,
}

impl Peer {
    pub(crate) fn register_peer(
        peer_id: NodeId,
        our_id: NodeId,
        handle: Handle,
        realm: Realm,
        connection_id: ConnectionId,
        cost: Cost,
        realms: RealmList,
        name: impl Into<String>,
    ) -> Self {
        let (peer_tx, rx_node) = tokio::sync::mpsc::channel(32);

        let peer = Self {
            peer_id,
            our_id,
            rx_node,
            handle,
            realm,
            connection_id,
            cost,
            stats: PeerStats::default(),
            realms,
            name: name.into(),
        };

        let peer_entry = PeerEntry {
            peer_tx,
            // realm: peer.realm.clone(),
        };

        peer.handle.send_broker(RouterMsg::RegisterPeer(
            peer.peer_id,
            peer.connection_id,
            peer_entry,
            peer.cost,
            peer.realms.clone(),
        ));

        peer
    }

    pub(crate) async fn recv(&mut self) -> Option<NodeMessage> {
        let msg = self.rx_node.recv().await?;
        if let NodeMessage::WirePacket(ref packet) = msg {
            self.stats.tx.record(packet);
        }
        Some(msg)
    }

    pub(crate) fn unregister(&mut self) {
        self.handle
            .send_broker(crate::messages::RouterMsg::UnRegisterPeer(
                self.connection_id,
            ));
        self.close();
    }

    fn close(&mut self) {
        self.rx_node.close();
    }

    fn forward_packet(&mut self, packet: WirePacket, connection_id: ConnectionId) {
        self.stats.rx.record(&packet);

        self.handle.forward_packet(packet, connection_id);
    }

    pub(crate) fn handle_node_message(&mut self, node_message: NodeMessage) {
        match node_message {
            NodeMessage::WirePacket(packet) => self.forward_packet(packet, self.connection_id),
            NodeMessage::Lsa(lsa) => self.handle.send_broker(RouterMsg::LsaInbound {
                from: self.connection_id,
                lsa,
            }),
            NodeMessage::LsaAck { key, seq } => self.handle.send_broker(RouterMsg::LsaAckInbound {
                from: self.connection_id,
                key,
                seq,
            }),
        }
    }
}

#[derive(Default, Debug)]
pub(crate) struct PacketByteCounts {
    bytes: usize,
    packets: usize,
}

impl PacketByteCounts {
    pub(crate) fn record(&mut self, packet: &WirePacket) {
        self.bytes += packet.payload.len();
        self.packets += 1;
    }
}

#[derive(Default, Debug)]
pub(crate) struct PeerStats {
    pub(crate) tx: PacketByteCounts,
    pub(crate) rx: PacketByteCounts,
}
