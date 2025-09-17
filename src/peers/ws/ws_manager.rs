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

use crate::{
    Handle,
    messages::BusControlMsg,
    peers::{
        Peer, WsRemoteOptions,
        ws::{
            self, WsActivePeer, WsCommand, WsListenerOptions, WsMessage, WsPendingPeer,
            create_listener,
        },
    },
    routing::{NodeId, PeerEntry, Realm},
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
    current_peers: Vec<WsActivePeer>,
    tx: UnboundedSender<WsCommand>,
    rx: UnboundedReceiver<WsCommand>,
    node_id: NodeId,
    ws_listener_options: Option<WsListenerOptions>,
    pending_peers: Vec<WsPendingPeer>,
    disconnected_peers: Vec<WsPendingPeer>,
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
            current_peers: Vec::new(),
            tx,
            rx,
            node_id,
            ws_listener_options,
            pending_peers: ws_peers.iter().map(WsPendingPeer::from).collect(),
            disconnected_peers: Vec::new(),
        }
    }

    pub(crate) async fn start(mut self) {
        let mut state = Some(Box::new(Start {}) as Box<dyn State>);
        while let Some(next_state) = state.take() {
            debug!("Entering: {:?}", &next_state);
            state = next_state.next(&mut self).await;
        }
    }

    fn get_next_timeout(&self) -> Option<Duration> {
        self.disconnected_peers.first().map(|p| {
            let now = tokio::time::Instant::now();
            if p.when_ready() <= now {
                Duration::from_secs(0)
            } else {
                p.when_ready() - now
            }
        })
    }

    fn get_next_ready_peer(&mut self) -> Option<WsPendingPeer> {
        if let Some(first) = self.disconnected_peers.first() {
            let now = tokio::time::Instant::now();
            if first.when_ready() <= now {
                return self.disconnected_peers.remove(0).into();
            }
        }
        None
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

// #[derive(Debug)]
// struct NewTcpStream {
//     stream: tokio::net::TcpStream,
//     addr: core::net::SocketAddr,
// }

#[derive(Debug)]
struct NewWsStream<S: AsyncRead + AsyncWrite + Unpin> {
    stream: tokio_tungstenite::WebSocketStream<S>,
    pending: Option<WsPendingPeer>,
}

#[derive(Debug)]
struct ConnectRemote {
    pending: WsPendingPeer,
}

#[derive(Debug)]
struct QueueReconnect {
    pending: WsPendingPeer,
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
        if let Some(peer) = state.pending_peers.pop() {
            debug!("Connecting to remote WebSocket peer at {}", peer.url);
            return b(ConnectRemote { pending: peer });
        };
        // Wait for either a command or a shutdown signal

        if let Some(pending) = state.get_next_ready_peer() {
            debug!("Reconnecting to remote WebSocket peer at {}", pending.url);
            return b(ConnectRemote { pending });
        }

        let timeout = state.get_next_timeout();

        tokio::select! {
            _ = async {
                if let Some(dur) = timeout {
                    tokio::time::sleep(dur).await;
                } else {
                    futures::future::pending::<()>().await;
                }
            } => {
                if let Some(pending) = state.get_next_ready_peer() {
                    debug!("Reconnecting to remote WebSocket peer at {}", pending.url);
                    return b(ConnectRemote { pending });
                }
                b(Listen)
            }

            Some(cmd) = state.rx.recv() => {
                b(HandleCommand(cmd))
                // Handle incoming commands
            }
            Ok(_msg) = state.bus_control.changed() => {
                match *state.bus_control.borrow() {
                    BusControlMsg::Shutdown => {
                        for peer in state.current_peers.drain(..) {
                            debug!("Shutting down connection to peer {}", peer.peer_id);
                            let _ = peer.ws_control.send(ws::WsControl::Shutdown);
                        }
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
    async fn next(self: Box<Self>, state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        match self.0 {
            WsCommand::NewWsStream(stream, _addr) => b(NewWsStream {
                stream,
                pending: None,
            }),
            WsCommand::PeerClosed(uuid) => {
                debug!("Peer {} closed connection", uuid);
                // Remove the peer from current_peers
                // If the peer was a remote peer, schedule a reconnect
                // _state.current_peers.retain(|p| p.peer_id != uuid);
                if let Some(pos) = state.current_peers.iter().position(|p| p.peer_id == uuid) {
                    let peer = state.current_peers.remove(pos);
                    if let Some(url) = peer.url {
                        let pending = WsPendingPeer::from_url(url);
                        state
                            .disconnected_peers
                            .binary_search_by_key(&pending.when_ready(), |p| p.when_ready())
                            .err()
                            .map(|pos| state.disconnected_peers.insert(pos, pending));
                    }
                }
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
                                let peer = Peer::new(
                                    peer_id,
                                    state.node_id,
                                    state.handle.clone(),
                                    rx,
                                    Realm::Global, // WebSocket peers are always in the global realm
                                );
                                let peer_entry = PeerEntry {
                                    peer_tx: tx_nodemessage,
                                    realm: Realm::Global,
                                };
                                state.handle.register_peer(peer_id, peer_entry);
                                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                                spawn(ws::ws_peer::run_ws_peer(
                                    self.stream,
                                    state.bus_control.clone(),
                                    state.tx.clone(),
                                    rx,
                                    peer,
                                ));
                                let peer = WsActivePeer {
                                    peer_id,
                                    url: self.pending.and_then(|p| Some(p.url)),
                                    ws_control: tx,
                                };
                                state.current_peers.push(peer);
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
    async fn next(mut self: Box<Self>, _state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        // tokio_native_tls::native_tls::
        let attempt = tokio_tungstenite::connect_async(self.pending.url.as_str()).await;
        match attempt {
            Ok((ws_stream, response)) => {
                // let addr = ws_stream
                debug!(
                    "Connected to remote WebSocket peer  with response {:?}",
                    response
                );
                b(NewWsStream {
                    stream: ws_stream,
                    pending: Some(self.pending),
                })
            }
            Err(e) => {
                error!("Failed to connect to remote WebSocket peer: {}", e);

                b(QueueReconnect {
                    pending: self.pending,
                })
            }
        }
    }
}

#[async_trait]
impl State for QueueReconnect {
    async fn next(mut self: Box<Self>, state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        // self.pending.last_attempt = tokio::time::Instant::now();
        // self.pending.backoff = std::cmp::min(self.pending.backoff * 2, Duration::from_secs(300));
        // self.pending.num_attempts += 1;
        self.pending.record_attempt();
        debug!(
            "Scheduling reconnect to {} in {:?}",
            self.pending.url, self.pending.backoff
        );
        state
            .disconnected_peers
            .binary_search_by_key(&self.pending.when_ready(), |p| p.when_ready())
            .err()
            .map(|pos| state.disconnected_peers.insert(pos, self.pending));
        b(Listen {})
    }
}
