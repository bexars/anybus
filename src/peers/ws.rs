use crate::tokio;

use std::fmt::Display;
#[cfg(feature = "ws_server")]
use std::net::{IpAddr, SocketAddr};
// use tokio_with_wasm::alias as tokio;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use tokio::sync::mpsc::Sender;

use url::Url;

use crate::{
    anybus::config::{WebSocketPeerConfig, WsUrl},
    define_local_rpc,
    messages::NodeMessage,
    routing::NodeId,
};

#[cfg(feature = "ws_server")]
mod listener;
pub(super) mod ws_manager;
mod ws_peer;
// mod ws_peer;

#[cfg(not(target_family = "wasm"))]
mod tg_websock;
#[cfg(not(target_family = "wasm"))]
pub use tg_websock::WebSockStream;

#[cfg(target_family = "wasm")]
mod websys_websock;
#[cfg(target_family = "wasm")]
pub use websys_websock::WebSockStream;

#[derive(Debug)]
pub(crate) enum WsControl {
    Shutdown,
}

// #[derive(Debug)]
pub(crate) enum WsCommand {
    NewWsStream {
        stream: WebSockStream,
        // socket_addr: SocketAddr,
        ws_pending_peer: Option<WsPendingPeer>,
        peer_id: NodeId,
    },
    PeerClosed(NodeId),
    QueueReconnect(WsPendingPeer),
}

impl Debug for WsCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NewWsStream {
                ws_pending_peer: direction,
                ..
            } => f
                .debug_tuple("NewWsStream")
                // .field(socket_addr)
                .field(direction)
                .finish(),
            Self::PeerClosed(arg0) => f.debug_tuple("PeerClosed").field(arg0).finish(),
            Self::QueueReconnect(pending_peer) => {
                f.debug_tuple("QueueReconnect").field(pending_peer).finish()
            }
        }
    }
}
define_local_rpc! {
    WsRpcMessage {
        AddPeer{peer_config: WebSocketPeerConfig} -> Result<(), String>,
        RemovePeer {url: WsUrl} -> Result<(), String>,
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum WsMessage {
    Hello(NodeId),
    NodeMsg(NodeMessage),
    CloseConnection,
    Ping(u64),
    Pong(u64),
}

impl From<WsMessage> for Vec<u8> {
    fn from(msg: WsMessage) -> Self {
        crate::codec::encode(&msg).expect("failed to encode websocket message")
    }
}

impl From<WsMessage> for Bytes {
    fn from(msg: WsMessage) -> Self {
        let vec = crate::codec::encode(&msg).expect("failed to encode websocket message");
        Bytes::from(vec)
    }
}

#[derive(Debug, thiserror::Error)]
enum WsError {
    #[cfg(feature = "ws_server")]
    #[error("Error binding to address {}", .0)]
    BindFailure(SocketAddr),
    #[error("TLS Error: {0}")]
    #[cfg(not(target_family = "wasm"))]
    TlsError(#[from] rustls::Error),
    #[error("File error: {0}")]
    #[cfg(not(target_family = "wasm"))]
    TlsPkiError(#[from] rustls::pki_types::pem::Error),
    #[error("File error: {0}")]
    StandardIo(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
/// Options for connecting to a remote WebSocket peer
pub struct WsRemoteOptions {
    /// The URL of the remote WebSocket peer.  Should start with ws:// or wss://
    pub url: Url,
}

#[derive(Debug)]
pub(crate) enum StreamDirection {
    #[cfg(feature = "ws_server")]
    Inbound,
    Outbound(WebSocketPeerConfig),
}

impl From<&WebSocketPeerConfig> for StreamDirection {
    fn from(peer: &WebSocketPeerConfig) -> Self {
        StreamDirection::Outbound(peer.clone())
    }
}

impl Display for StreamDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "ws_server")]
            StreamDirection::Inbound => write!(f, "Inbound"),
            StreamDirection::Outbound(peer) => write!(f, "Outbound({})", peer),
        }
    }
}

impl PartialEq<WebSocketPeerConfig> for StreamDirection {
    fn eq(&self, other: &WebSocketPeerConfig) -> bool {
        match self {
            #[cfg(feature = "ws_server")]
            StreamDirection::Inbound => false,
            StreamDirection::Outbound(config) => config == other,
        }
    }
}

impl PartialEq<StreamDirection> for WebSocketPeerConfig {
    fn eq(&self, other: &StreamDirection) -> bool {
        match other {
            #[cfg(feature = "ws_server")]
            StreamDirection::Inbound => false,
            StreamDirection::Outbound(config) => self == config,
        }
    }
}

impl PartialEq<StreamDirection> for WsUrl {
    fn eq(&self, other: &StreamDirection) -> bool {
        match other {
            #[cfg(feature = "ws_server")]
            StreamDirection::Inbound => false,
            StreamDirection::Outbound(config) => self == &config.url,
        }
    }
}

impl PartialEq<WsUrl> for StreamDirection {
    fn eq(&self, other: &WsUrl) -> bool {
        match self {
            #[cfg(feature = "ws_server")]
            StreamDirection::Inbound => false,
            StreamDirection::Outbound(config) => &config.url == other,
        }
    }
}

#[derive(Debug)]
pub(crate) struct WsPendingPeer {
    config: WebSocketPeerConfig,
    last_attempt: web_time::Instant,
    backoff: std::time::Duration,
    num_attempts: u32,
}

impl From<WebSocketPeerConfig> for WsPendingPeer {
    fn from(config: WebSocketPeerConfig) -> Self {
        Self {
            config,
            last_attempt: web_time::Instant::now(),
            backoff: std::time::Duration::from_secs(1),
            num_attempts: 0,
        }
    }
}

impl WsPendingPeer {
    fn when_ready(&self) -> web_time::Instant {
        self.last_attempt + self.backoff
    }

    fn record_attempt(&mut self) {
        self.last_attempt = web_time::Instant::now();
        self.num_attempts += 1;
        self.backoff = std::time::Duration::from_secs(2u64.pow(self.num_attempts.min(8)));
    }
}

#[derive(Debug)]
struct WsActivePeer {
    direction: StreamDirection, //TODO why is this an option
    peer_id: NodeId,
    ws_control: Sender<WsControl>,
}

impl From<&WebSocketPeerConfig> for WsPendingPeer {
    fn from(config: &WebSocketPeerConfig) -> Self {
        Self {
            config: config.clone(),
            last_attempt: web_time::Instant::now(),
            backoff: std::time::Duration::from_secs(1),
            num_attempts: 0,
        }
    }
}

#[cfg(feature = "ws_server")]
/// Options for the WebSocket listener
#[derive(Debug, Clone)]
pub struct WsListenerOptions {
    /// The IP address to bind to.
    pub addr: IpAddr,
    /// The port to bind to.
    pub port: u16,
    /// Whether to use TLS (wss://) or not (ws://)
    pub use_tls: bool,
    /// The path to the TLS certificate file (PEM format)
    pub cert_path: Option<String>,
    /// The path to the TLS private key file (PEM format)
    pub key_path: Option<String>,
}

#[cfg(feature = "ws_server")]
impl Default for WsListenerOptions {
    fn default() -> Self {
        Self {
            addr: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            port: 8888,
            use_tls: false,
            cert_path: None,
            key_path: None,
        }
    }
}
