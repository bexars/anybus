use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{
        mpsc::{UnboundedReceiver, UnboundedSender},
        watch,
    },
    time::timeout,
};
use tracing::{debug, error, trace};
use uuid::Uuid;

use crate::{
    Handle,
    messages::BusControlMsg,
    peers::{
        Peer, WsRemoteOptions,
        ws::{self, WsCommand, WsControl, WsListenerOptions, WsMessage, create_listener},
    },
    routing::NodeId,
    spawn,
};

/// Helper function to box a State and return it as an Option
fn b<T: State + 'static>(thing: T) -> Option<Box<dyn State>> {
    Some(Box::new(thing))
}

#[derive(Debug)]
pub(crate) struct WebsocketManager {
    handle: Handle,
    bus_control: watch::Receiver<BusControlMsg>,
    peers: Vec<(Uuid, UnboundedSender<WsControl>)>,
    tx: UnboundedSender<WsCommand>,
    rx: UnboundedReceiver<WsCommand>,
    node_id: NodeId,
    ws_listener_options: Option<WsListenerOptions>,
    ws_peers: Vec<WsRemoteOptions>,
}

impl WebsocketManager {
    pub(crate) fn new(
        node_id: NodeId,
        handle: Handle,
        bus_control: watch::Receiver<BusControlMsg>,
        ws_listener_options: Option<WsListenerOptions>,
        ws_peers: Vec<WsRemoteOptions>,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            handle,
            bus_control,
            peers: Vec::new(),
            tx,
            rx,
            node_id,
            ws_listener_options,
            ws_peers,
        }
    }

    pub(crate) async fn start(mut self) {
        let mut state = Some(Box::new(Start {}) as Box<dyn State>);
        while let Some(next_state) = state.take() {
            debug!("Entering: {:?}", &next_state);
            state = next_state.next(&mut self).await;
        }
    }
}

#[async_trait]
trait State: Send + std::fmt::Debug {
    async fn next(self: Box<Self>, state: &mut WebsocketManager) -> Option<Box<dyn State>>;
}

#[derive(Debug)]
struct Start;

#[derive(Debug)]
struct Listen;

#[derive(Debug)]
struct HandleCommand(WsCommand);

#[derive(Debug)]
struct NewTcpStream {
    stream: tokio::net::TcpStream,
    addr: core::net::SocketAddr,
}

#[derive(Debug)]
struct NewWsStream<S: AsyncRead + AsyncWrite + Unpin> {
    stream: tokio_tungstenite::WebSocketStream<S>,
    // addr: core::net::SocketAddr,
}

#[derive(Debug)]
struct ConnectRemote {
    url: url::Url,
}

#[async_trait]
impl State for Start {
    async fn next(self: Box<Self>, state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        if let Some(ws_options) = state.ws_listener_options.take() {
            match create_listener(ws_options, state.tx.clone(), state.bus_control.clone()).await {
                Ok(()) => b(Listen {}),
                Err(e) => {
                    error!("Failed to create WebSocket listener: {}", e);
                    None
                }
            }
        } else {
            debug!("WebSocket listener options not provided");
            b(Listen)
        }
    }
}

#[async_trait]
impl State for Listen {
    async fn next(self: Box<Self>, state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        if let Some(peer) = state.ws_peers.pop() {
            debug!("Connecting to remote WebSocket peer at {}", peer.url);
            return b(ConnectRemote { url: peer.url });
        };
        // Wait for either a command or a shutdown signal

        tokio::select! {
            Some(cmd) = state.rx.recv() => {
                b(HandleCommand(cmd))
                // Handle incoming commands
            }
            Ok(_msg) = state.bus_control.changed() => {
                match *state.bus_control.borrow() {
                    BusControlMsg::Shutdown => {
                        debug!("WebSocket Manager shutting down");
                         None
                    }
                    _ => {
                        b(Listen)
                    }
                }
            }
            else => {
                error!("WebSocket Manager closed.  Shutting down Websockets.");
                None
            }
        }
    }
}

#[async_trait]
impl State for HandleCommand {
    async fn next(self: Box<Self>, _state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        match self.0 {
            // WsCommand::NewTcpStream(stream, addr) => b(NewTcpStream { stream, addr }),
            WsCommand::NewWsStream(stream, _addr) => b(NewWsStream { stream }),
        }
    }
}

#[async_trait]
impl State for NewTcpStream {
    async fn next(self: Box<Self>, _state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        // Perform the WebSocket handshake here

        debug!("Performing handshake with {}", self.addr);
        // let builder = tokio_native_tls::native_tls::TlsStream::;
        // let connector = builder.build().unwrap();
        // connector.
        let stream_result = tokio_tungstenite::accept_async(self.stream).await;
        match stream_result {
            Ok(ws_stream) => {
                debug!("WebSocket connection established with {}", self.addr);
                b(NewWsStream {
                    stream: ws_stream,
                    // addr: self.addr,
                })
            }
            Err(e) => {
                error!("WebSocket handshake failed with {}: {}", self.addr, e);
                b(Listen {})
            }
        }
    }
}

#[async_trait]
impl<S: AsyncRead + AsyncWrite + Unpin + Send + Sync + std::fmt::Debug + 'static> State
    for NewWsStream<S>
{
    async fn next(mut self: Box<Self>, state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        // Handshake the Anybus protocol here
        let msg = WsMessage::Hello(state.node_id);
        let msg = tokio_tungstenite::tungstenite::Message::Binary(msg.into());
        trace!("Sending Hello message: {:?}", msg);
        if let Err(e) = self.stream.send(msg).await {
            error!("Failed to send Hello message: {}", e);
        }
        match timeout(Duration::from_secs(5), self.stream.next()).await {
            Ok(Some(Err(e))) => {
                error!("Error receiving Hello response : {}", e);
            }
            Ok(Some(Ok(msg))) => {
                trace!("Received message: {:?}", msg);
                match msg {
                    tokio_tungstenite::tungstenite::Message::Binary(bin) => {
                        match serde_cbor::from_slice::<WsMessage>(&bin) {
                            Ok(WsMessage::Hello(peer_id)) => {
                                debug!("Received Hello from peer: {} ", peer_id);
                                let (tx_nodemessage, rx) = tokio::sync::mpsc::unbounded_channel();
                                let peer =
                                    Peer::new(peer_id, state.node_id, state.handle.clone(), rx);
                                state.handle.register_peer(peer_id, tx_nodemessage);
                                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                                spawn(ws::ws_peer::run_ws_peer(
                                    self.stream,
                                    state.bus_control.clone(),
                                    state.tx.clone(),
                                    rx,
                                    peer,
                                ));
                                state.peers.push((peer_id, tx));
                            }
                            Ok(other) => {
                                error!("Unexpected message: {:?}", other);
                            }
                            Err(e) => {
                                error!("Failed to deserialize message: {}", e);
                            }
                        }
                    }
                    other => {
                        error!("Unexpected WebSocket message: {:?}", other);
                    }
                }
            }
            Ok(None) => {
                error!("Connection closed by peer");
            }
            Err(_) => {
                error!("Timeout waiting for Hello response");
            }
        }

        b(Listen)
    }
}

#[async_trait]
impl State for ConnectRemote {
    async fn next(self: Box<Self>, _state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        // tokio_native_tls::native_tls::
        let attempt = tokio_tungstenite::connect_async(self.url.as_str()).await;
        match attempt {
            Ok((ws_stream, response)) => {
                // let addr = ws_stream
                debug!(
                    "Connected to remote WebSocket peer  with response {:?}",
                    response
                );
                b(NewWsStream { stream: ws_stream })
            }
            Err(e) => {
                error!("Failed to connect to remote WebSocket peer: {}", e);
                b(Listen {})
            }
        }
    }
}
