// use tokio_with_wasm::alias as tokio;

#[cfg(feature = "ipc")]
mod ipc;
#[cfg(feature = "ws")]
pub(crate) mod ws;

mod common;

#[cfg(feature = "ws_server")]
pub use ws::WsListenerOptions;
#[cfg(feature = "ws")]
pub use ws::WsRemoteOptions;
#[cfg(feature = "ws")]
pub(crate) use ws::ws_manager::WebsocketManager;

#[cfg(feature = "ipc")]
pub(crate) use ipc::ipc_manager::IpcManager;
