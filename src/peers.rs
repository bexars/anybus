// use tokio_with_wasm::alias as tokio;

#[cfg(feature = "ipc")]
mod ipc;
#[cfg(feature = "ws")]
pub(crate) mod ws;

use tokio::sync::mpsc::{self};
#[cfg(feature = "ws_server")]
pub use ws::WsListenerOptions;
#[cfg(feature = "ws")]
pub use ws::WsRemoteOptions;
#[cfg(feature = "ws")]
pub(crate) use ws::ws_manager::WebsocketManager;

#[cfg(feature = "ipc")]
pub(crate) use ipc::ipc_manager::IpcManager;

use crate::{
    Handle,
    messages::NodeMessage,
    routing::{NodeId, Realm, WirePacket},
};

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
    pub(crate) fn new(
        peer_id: NodeId,
        our_id: NodeId,
        handle: Handle,
        rx_node: mpsc::Receiver<NodeMessage>,
        realm: Realm,
        connection_id: u16,
    ) -> Self {
        Self {
            peer_id,
            our_id,
            rx_node,
            handle,
            realm,
            connection_id,
            stats: PeerStats::default(),
        }
    }

    pub(crate) async fn recv(&mut self) -> Option<NodeMessage> {
        let msg = self.rx_node.recv().await?;
        if let NodeMessage::WirePacket(ref packet) = msg {
            self.stats.tx.record(packet);
        }
        Some(msg)
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
