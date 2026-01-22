use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
};

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::{net::TcpStream, sync::mpsc::UnboundedSender};
use tokio_native_tls::{TlsStream, native_tls::Identity};
use tokio_tungstenite::WebSocketStream;
use tracing::error;
use url::Url;
use uuid::Uuid;

use crate::{
    messages::BusControlMsg,
    routing::{Advertisement, WirePacket},
    spawn,
};

pub(super) mod ws_manager;
mod ws_peer;

#[derive(Debug)]
enum WsControl {
    Shutdown,
}

#[derive(Debug)]
enum WsCommand {
    // NewTcpStream(tokio::net::TcpStream, SocketAddr),
    NewWsStream(WebSocketStream<TlsStream<TcpStream>>, SocketAddr),
    PeerClosed(Uuid),
}

#[derive(Debug, Serialize, Deserialize)]
enum WsMessage {
    Hello(Uuid),
    Packet(WirePacket),
    CloseConnection,
    Advertise(HashSet<Advertisement>),
    Withdraw(HashSet<Advertisement>),
}

impl From<WsMessage> for Bytes {
    fn from(msg: WsMessage) -> Self {
        let vec = serde_cbor::to_vec(&msg).expect("");
        Bytes::from(vec)
    }
}

#[derive(Debug, thiserror::Error)]
enum WsError {
    #[error("Error binding to address {}", .0)]
    BindFailure(SocketAddr),
    #[error("TLS Error: {0}")]
    TlsError(#[from] tokio_native_tls::native_tls::Error),
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
struct WsPendingPeer {
    url: Url,
    last_attempt: tokio::time::Instant,
    backoff: std::time::Duration,
    num_attempts: u32,
}

impl WsPendingPeer {
    // fn is_ready(&self) -> bool {
    //     tokio::time::Instant::now().duration_since(self.last_attempt) >= self.backoff
    // }
    fn from_url(url: Url) -> Self {
        Self {
            url,
            last_attempt: tokio::time::Instant::now(),
            backoff: std::time::Duration::from_secs(1),
            num_attempts: 0,
        }
    }

    fn when_ready(&self) -> tokio::time::Instant {
        self.last_attempt + self.backoff
    }

    fn record_attempt(&mut self) {
        self.last_attempt = tokio::time::Instant::now();
        self.num_attempts += 1;
        self.backoff = std::time::Duration::from_secs(2u64.pow(self.num_attempts.min(8)));
    }
}

#[derive(Debug)]
struct WsActivePeer {
    url: Option<Url>,
    peer_id: Uuid,
    ws_control: UnboundedSender<WsControl>,
}

impl From<&WsRemoteOptions> for WsPendingPeer {
    fn from(opts: &WsRemoteOptions) -> Self {
        Self {
            url: opts.url.clone(),
            last_attempt: tokio::time::Instant::now(),
            backoff: std::time::Duration::from_secs(1),
            num_attempts: 0,
        }
    }
}

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

async fn create_listener(
    ws_listener_options: WsListenerOptions,
    ws_command: tokio::sync::mpsc::UnboundedSender<WsCommand>,
    bus_control: tokio::sync::watch::Receiver<BusControlMsg>,
) -> Result<(), WsError> {
    // Create the listener here
    //
    //
    let (cert_path, key_path) = (
        ws_listener_options.cert_path.unwrap_or_default(),
        ws_listener_options.key_path.unwrap_or_default(),
    );
    let acceptor = {
        let cert = std::fs::read(cert_path)?; //.expect("Failed to read certificate");
        let key = std::fs::read(key_path)?; //.expect("Failed to read private key");
        let identity = Identity::from_pkcs8(&cert, &key)?; // .expect("Failed to create identity from pkcs8");
        let acceptor = tokio_native_tls::TlsAcceptor::from(
            tokio_native_tls::native_tls::TlsAcceptor::builder(identity).build()?,
            // .expect("Failed to build TlsAcceptor"),
        );
        acceptor
    };
    let sock_addr = std::net::SocketAddr::new(ws_listener_options.addr, ws_listener_options.port);
    let listener = tokio::net::TcpListener::bind(sock_addr)
        .await
        .map_err(|e| {
            error!("Failed to bind to address {}: {}", sock_addr, e);
            WsError::BindFailure(sock_addr)
        })?;
    spawn(run_ws_listener(listener, ws_command, bus_control, acceptor));
    Ok(())
}

async fn run_ws_listener(
    listener: tokio::net::TcpListener,
    ws_command: UnboundedSender<WsCommand>,
    mut bus_control: tokio::sync::watch::Receiver<BusControlMsg>,
    acceptor: tokio_native_tls::TlsAcceptor,
) {
    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, addr)) => {
                        // Handle the new connection
                        let stream = match acceptor.accept(stream).await {
                            Ok(s) => tokio_tungstenite::accept_async(s).await.unwrap(),
                            Err(e) => {
                                error!("TLS handshake failed with {}: {}", addr, e);
                                continue;
                            }
                        };
                        tracing::info!("Accepted connection from {}", addr);

                        ws_command.send(WsCommand::NewWsStream(stream,addr)).ok();
                    }
                    Err(e) => {
                        error!("Failed to accept connection: {}", e);
                    }
                }
            }
            Ok(_msg) = bus_control.changed() => {
                let msg = *bus_control.borrow();
                match msg {
                    BusControlMsg::Shutdown => {
                        break;
                    }
                    _ => {}
                }
            }

        }
    }
}
