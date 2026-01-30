#[cfg(feature = "ipc")]
mod ipc;
#[cfg(any(feature = "ws", feature = "wasm_ws"))]
mod ws;
use tokio::sync::mpsc::UnboundedReceiver;
use uuid::Uuid;
#[cfg(any(feature = "ws", feature = "wasm_ws"))]
pub(crate) use ws::ws_manager::WebsocketManager;
#[cfg(any(feature = "ws", feature = "wasm_ws"))]
pub use ws::{WsListenerOptions, WsRemoteOptions};

#[cfg(feature = "ipc")]
pub(crate) use ipc::ipc_manager::IpcManager;

use crate::{Handle, messages::NodeMessage, routing::Realm};

#[derive(Debug)]
pub(crate) struct Peer {
    pub(crate) peer_id: Uuid,
    pub(crate) our_id: Uuid,
    pub(crate) rx: UnboundedReceiver<NodeMessage>,
    pub(crate) handle: Handle,
    pub(crate) realm: Realm,
}

impl Peer {
    pub(crate) fn new(
        peer_id: Uuid,
        our_id: Uuid,
        handle: Handle,
        rx: UnboundedReceiver<NodeMessage>,
        realm: Realm,
    ) -> Self {
        Self {
            peer_id,
            our_id,
            rx,
            handle,
            realm,
        }
    }
}
