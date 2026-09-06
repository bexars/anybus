use std::{collections::HashSet, time::Duration};
use tokio::sync::mpsc;
use web_time::Instant;

use crate::{
    Handle, Realm,
    messages::{NodeMessage, RouterMsg},
    routing::{Advertisement, NodeId, PeerEntry, WirePacket},
};

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
    pub(crate) connection_id: u16,
    pub(crate) stats: PeerStats,
}

impl Peer {
    pub(crate) fn register_peer(
        peer_id: NodeId,
        our_id: NodeId,
        handle: Handle,
        // rx_node: mpsc::Receiver<NodeMessage>,
        realm: Realm,
        connection_id: u16,
    ) -> Self {
        let (peer_tx, rx_node) = tokio::sync::mpsc::channel(32);

        let peer = Self {
            peer_id,
            our_id,
            rx_node,
            handle,
            realm,
            connection_id,
            stats: PeerStats::default(),
        };

        let peer_entry = PeerEntry {
            peer_tx,
            realm: peer.realm.clone(),
        };

        peer.handle.send_broker(RouterMsg::RegisterPeer(
            peer.peer_id,
            peer.connection_id,
            peer_entry,
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

    pub(crate) fn add_endpoints(&mut self, ads: HashSet<Advertisement>) {
        self.handle
            .send_broker(crate::messages::RouterMsg::AddPeerEndpoints(
                self.connection_id,
                ads,
            ));
    }

    pub(crate) fn remove_endpoints(&mut self, ads: HashSet<Advertisement>) {
        self.handle
            .send_broker(crate::messages::RouterMsg::RemovePeerEndpoints(
                self.connection_id,
                ads,
            ));
    }

    pub(crate) fn unregister(&mut self) {
        self.handle
            .send_broker(crate::messages::RouterMsg::UnRegisterPeer(
                self.connection_id,
            ));
    }

    pub(crate) fn close(&mut self) {
        self.rx_node.close();
    }

    pub(crate) fn send_packet(&mut self, packet: WirePacket) {
        self.stats.rx.record(&packet);

        self.handle.send_packet(packet, self.connection_id);
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
