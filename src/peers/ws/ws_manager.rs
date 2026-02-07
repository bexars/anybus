use tokio_with_wasm::alias as tokio;

use std::{fmt::Debug, time::Duration};

use tokio::sync::{
    mpsc::{self},
    watch,
};
// use web_time::Instant;

#[cfg(not(target_family = "wasm"))]
use tokio_tungstenite::connect_async;
// use tokio_tungstenite_wasm::{Message, WebSocketStream};
// #[cfg(feature = "wasm_ws")]
// use tokio_tungstenite_wasm::WebSocketStream;

use tracing::{debug, error, info, trace};
#[cfg(target_family = "wasm")]
use wasm_socket_handle::WsHandle;

#[cfg(feature = "ws_server")]
use crate::peers::ws::WsListenerOptions;
use crate::{
    Handle,
    messages::BusControlMsg,
    peers::{
        Peer, WsRemoteOptions,
        ws::{
            self, WebSockStream, WsActivePeer, WsCommand, WsError, WsMessage, WsPendingPeer,
            ws_peer::InMessage,
        },
    },
    routing::{NodeId, PeerEntry, Realm},
    spawn,
};

#[cfg(feature = "ws_server")]
use crate::peers::ws::create_listener;

/// Helper function to box a State and return it as an Option
// fn b<T: State + 'static>(thing: T) -> Option<Box<dyn State>> {
//     Some(Box::new(thing))
// }

#[derive(Debug, Default)]
#[allow(dead_code)]
enum ManagerState {
    #[default]
    Init,
    Listen,
    HandleCommand(WsCommand),
    NewWsStream {
        stream: WebSockStream,
        pending: Option<WsPendingPeer>,
    },
    ConnectRemote(WsPendingPeer),
    QueueReconnect(WsPendingPeer),
    Error(WsError),
    Shutdown,
}

#[derive(Debug)]
pub(crate) struct WebsocketManager {
    handle: Handle,
    bus_control: watch::Receiver<BusControlMsg>,
    current_peers: Vec<WsActivePeer>,
    tx: mpsc::Sender<WsCommand>,
    rx: mpsc::Receiver<WsCommand>,
    node_id: NodeId,
    #[cfg(feature = "ws_server")]
    ws_listener_options: Option<WsListenerOptions>,
    pending_peers: Vec<WsPendingPeer>,
    disconnected_peers: Vec<WsPendingPeer>,
}

impl WebsocketManager {
    pub(crate) fn new(
        node_id: NodeId,
        handle: Handle,
        bus_control: watch::Receiver<BusControlMsg>,
        #[cfg(feature = "ws_server")] ws_listener_options: Option<WsListenerOptions>,
        ws_peers: Vec<WsRemoteOptions>,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        Self {
            handle,
            bus_control,
            current_peers: Vec::new(),
            tx,
            rx,
            node_id,
            #[cfg(feature = "ws_server")]
            ws_listener_options,
            pending_peers: ws_peers.iter().map(WsPendingPeer::from).collect(),
            disconnected_peers: Vec::new(),
        }
    }

    pub(crate) async fn start(mut self) {
        let mut state = ManagerState::Init;
        loop {
            state = match state {
                ManagerState::Init => self.init().await,
                ManagerState::Listen => self.listen().await,
                ManagerState::HandleCommand(ws_command) => self.handle_command(ws_command).await,
                ManagerState::NewWsStream { stream, pending } => {
                    self.new_ws_stream(stream, pending).await
                }
                ManagerState::ConnectRemote(ws_pending_peer) => {
                    self.connect_remote(ws_pending_peer).await
                }
                ManagerState::QueueReconnect(ws_pending_peer) => {
                    self.queue_reconnect(ws_pending_peer).await
                }
                ManagerState::Error(error) => self.handle_error(error).await,
                ManagerState::Shutdown => break,
            };
        }
    }

    fn get_next_timeout(&self) -> Option<Duration> {
        self.disconnected_peers.first().map(|p| {
            let now = web_time::Instant::now();
            if p.when_ready() <= now {
                Duration::from_secs(0)
            } else {
                p.when_ready() - now
            }
        })
    }

    fn get_next_ready_peer(&mut self) -> Option<WsPendingPeer> {
        if let Some(first) = self.disconnected_peers.first() {
            let now = web_time::Instant::now();
            if first.when_ready() <= now {
                return self.disconnected_peers.remove(0).into();
            }
        }
        None
    }

    async fn init(&mut self) -> ManagerState {
        #[cfg(feature = "ws_server")]
        if let Some(ws_options) = self.ws_listener_options.take() {
            match create_listener(ws_options, self.tx.clone(), self.bus_control.clone()).await {
                Ok(()) => ManagerState::Listen,
                Err(e) => {
                    error!("Failed to create WebSocket listener: {}", e);
                    ManagerState::Error(e)
                }
            }
        } else {
            debug!("WebSocket listener options not provided");
            ManagerState::Listen
        }

        #[cfg(not(feature = "ws_server"))]
        ManagerState::Listen
    }

    async fn listen(&mut self) -> ManagerState {
        if let Some(peer) = self.pending_peers.pop() {
            debug!("Connecting to remote WebSocket peer at {}", peer.url);
            return ManagerState::ConnectRemote(peer);
        };
        // Wait for either a command or a shutdown signal

        if let Some(pending) = self.get_next_ready_peer() {
            debug!("Reconnecting to remote WebSocket peer at {}", pending.url);
            return ManagerState::ConnectRemote(pending);
        }

        let timeout = self.get_next_timeout();

        // #[cfg(not(target_arch = "wasm32"))]
        tokio::select! {
            _ = async {
                if let Some(dur) = timeout {
                    tokio::time::sleep(dur).await;
                } else {
                    futures::future::pending::<()>().await;
                }
            } => {
                if let Some(pending) = self.get_next_ready_peer() {
                    debug!("Reconnecting to remote WebSocket peer at {}", pending.url);
                    return ManagerState::ConnectRemote(pending);
                }
                ManagerState::Listen
            }

            Some(cmd) = self.rx.recv() => {
                ManagerState::HandleCommand(cmd)
                // Handle incoming commands
            }
            Ok(_msg) = self.bus_control.changed() => {
                let msg = self.bus_control.borrow().clone();
                match msg {
                    BusControlMsg::Shutdown => {
                        for peer in self.current_peers.drain(..) {
                            debug!("Shutting down connection to peer {}", peer.peer_id);
                            peer.ws_control.send(ws::WsControl::Shutdown).await.ok();
                        }
                        debug!("WebSocket Manager shutting down");
                        ManagerState::Shutdown
                    }
                    _ => {
                        ManagerState::Listen
                    }
                }
            }
            else => {
                error!("WebSocket Manager closed.  Shutting down Websockets.");
                ManagerState::Shutdown
            }
        }
    }

    async fn handle_command(&mut self, command: WsCommand) -> ManagerState {
        match command {
            #[cfg(feature = "ws_server")]
            WsCommand::NewWsStream(stream, _addr) => ManagerState::NewWsStream {
                stream: stream,
                pending: None,
            },
            WsCommand::PeerClosed(uuid) => {
                debug!("Peer {} closed connection", uuid);
                // Remove the peer from current_peers
                // If the peer was a remote peer, schedule a reconnect
                // _state.current_peers.retain(|p| p.peer_id != uuid);
                if let Some(pos) = self.current_peers.iter().position(|p| p.peer_id == uuid) {
                    let peer = self.current_peers.remove(pos);
                    if let Some(url) = peer.url {
                        let pending = WsPendingPeer::from_url(url);
                        self.disconnected_peers
                            .binary_search_by_key(&pending.when_ready(), |p| p.when_ready())
                            .err()
                            .map(|pos| self.disconnected_peers.insert(pos, pending));
                    }
                }
                ManagerState::Listen
            }
        }
    }

    async fn new_ws_stream(
        &mut self,
        mut stream: WebSockStream,
        pending: Option<WsPendingPeer>,
    ) -> ManagerState {
        // Handshake the Anybus protocol here
        let msg = WsMessage::Hello(self.node_id);
        // let msg = Message::Binary(msg.into());
        trace!("Sending Hello message: {:?}", msg);
        if let Err(e) = stream.send_msg(msg.into()).await {
            error!("Failed to send Hello message: {}", e);
        }
        // #[cfg(not(target_arch = "wasm32"))]
        match tokio::time::timeout(Duration::from_secs(5), stream.next_msg()).await {
            Ok(InMessage::WsMessage(WsMessage::Hello(peer_id))) => {
                debug!("Received Hello from peer: {} ", peer_id);
                let (tx_nodemessage, rx) = tokio::sync::mpsc::channel(32);
                let peer = Peer::new(
                    peer_id,
                    self.node_id,
                    self.handle.clone(),
                    rx,
                    Realm::Global, // WebSocket peers are always in the global realm
                );
                let peer_entry = PeerEntry {
                    peer_tx: tx_nodemessage,
                    realm: Realm::Global,
                };
                self.handle.register_peer(peer_id, peer_entry).await;
                let (tx, rx) = tokio::sync::mpsc::channel(32);
                spawn(ws::ws_peer::run_ws_peer(
                    stream,
                    self.bus_control.clone(),
                    self.tx.clone(),
                    rx,
                    peer,
                ));
                let peer = WsActivePeer {
                    peer_id,
                    url: pending.and_then(|p| Some(p.url)),
                    ws_control: tx,
                };
                self.current_peers.push(peer);
            }
            Ok(other) => {
                error!("Unexpected message: {:?}", other);
            }
            Err(_) => {
                error!("Timeout waiting for Hello response");
            }
        }
        // #[cfg(target_arch = "wasm32")]
        // match self.stream.next_msg().await {
        //     InMessage::WsMessage(WsMessage::Hello(peer_id)) => {
        //         debug!("Received Hello from peer: {} ", peer_id);
        //         let (tx_nodemessage, rx) = tokio::sync::mpsc::unbounded_channel();
        //         let peer = Peer::new(
        //             peer_id,
        //             state.node_id,
        //             state.handle.clone(),
        //             rx,
        //             Realm::Global, // WebSocket peers are always in the global realm
        //         );
        //         let peer_entry = PeerEntry {
        //             peer_tx: tx_nodemessage,
        //             realm: Realm::Global,
        //         };
        //         state.handle.register_peer(peer_id, peer_entry);
        //         let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        //         spawn(ws::ws_peer::run_ws_peer(
        //             self.stream,
        //             state.bus_control.clone(),
        //             state.tx.clone(),
        //             rx,
        //             peer,
        //         ));
        //         let peer = WsActivePeer {
        //             peer_id,
        //             url: self.pending.and_then(|p| Some(p.url)),
        //             ws_control: tx,
        //         };
        //         state.current_peers.push(peer);
        //     }
        //     other => {
        //         error!("Unexpected message: {:?}", other);
        //     }
        // }

        ManagerState::Listen
    }

    async fn connect_remote(&self, ws_pending_peer: WsPendingPeer) -> ManagerState {
        // tokio_native_tls::native_tls::
        #[cfg(target_family = "wasm")]
        let attempt = WsHandle::new(ws_pending_peer.url.as_str()).await;
        #[cfg(not(target_family = "wasm"))]
        let attempt = connect_async(ws_pending_peer.url.as_str()).await;
        match attempt {
            Ok(ws_stream) => {
                #[cfg(not(target_family = "wasm"))]
                {
                    let (stream, _response) = ws_stream;
                    ManagerState::NewWsStream {
                        stream: stream.into(),
                        pending: Some(ws_pending_peer),
                    }
                }
                #[cfg(target_family = "wasm")]
                {
                    let stream = ws_stream;
                    ManagerState::NewWsStream {
                        stream: stream.into(),
                        pending: Some(ws_pending_peer),
                    }
                }
            }
            Err(e) => {
                error!("Failed to connect to remote WebSocket peer: {}", e);

                ManagerState::QueueReconnect(ws_pending_peer)
            }
        }
    }

    async fn queue_reconnect(&mut self, mut ws_pending_peer: WsPendingPeer) -> ManagerState {
        // self.pending.last_attempt = tokio::time::Instant::now();
        // self.pending.backoff = std::cmp::min(self.pending.backoff * 2, Duration::from_secs(300));
        // self.pending.num_attempts += 1;
        ws_pending_peer.record_attempt();
        info!(
            "Scheduling reconnect to {} in {:?}",
            ws_pending_peer.url, ws_pending_peer.backoff
        );
        self.disconnected_peers
            .binary_search_by_key(&ws_pending_peer.when_ready(), |p| p.when_ready())
            .err()
            .map(|pos| self.disconnected_peers.insert(pos, ws_pending_peer));
        ManagerState::Listen
    }

    async fn handle_error(&self, error: WsError) -> ManagerState {
        debug!("WsManager encounted: {:?}", error);
        ManagerState::Listen
    }
}
