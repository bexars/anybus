// Cribbed the state machine from: https://moonbench.xyz/projects/rust-event-driven-finite-state-machine
use crate::tokio;
use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use tokio::{
    select,
    sync::{
        RwLock,
        mpsc::{self, channel},
    },
    time::{Instant, timeout},
};
use tracing::{debug, error, info};

use crate::{
    Handle, Realm,
    messages::NodeMessage,
    peers::{
        common::{Heartbeat, Peer},
        ipc::{IpcCommand, IpcControl, IpcMessage, IpcPeerStream},
    },
    routing::{ConnectionId, ConnectionIdCounter, NodeId, RealmList},
};

fn b<T: State + 'static>(thing: T) -> Option<Box<dyn State>> {
    Some(Box::new(thing))
}

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(crate) struct IpcPeer {
    stream: IpcPeerStream,
    ipc_command: mpsc::Sender<IpcCommand>,
    ipc_control: mpsc::Receiver<IpcControl>,
    control_tx: mpsc::Sender<IpcControl>,
    ipc_neighbors: Arc<RwLock<Vec<(NodeId, mpsc::Sender<IpcControl>, ConnectionId)>>>,
    peer: Option<Peer>,
    hb: Heartbeat,
    expected: Option<NodeId>,
    our_nodeid: NodeId,
    handle: Handle,
    connection_counter: ConnectionIdCounter,
    established: bool,
}

impl IpcPeer {
    pub(crate) fn new(
        stream: IpcPeerStream,
        ipc_command: mpsc::Sender<IpcCommand>,
        ipc_neighbors: Arc<RwLock<Vec<(NodeId, mpsc::Sender<IpcControl>, ConnectionId)>>>,
        expected: Option<NodeId>,
        our_nodeid: NodeId,
        handle: Handle,
        connection_counter: ConnectionIdCounter,
        heartbeat_interval: Duration,
        heartbeat_timeout: Duration,
    ) -> IpcPeer {
        let (control_tx, ipc_control) = channel(32);
        IpcPeer {
            stream,
            ipc_command,
            ipc_control,
            control_tx,
            ipc_neighbors,
            peer: None,
            hb: Heartbeat::new(Instant::now(), heartbeat_interval, heartbeat_timeout),
            expected,
            our_nodeid,
            handle,
            connection_counter,
            established: false,
        }
    }

    pub(crate) async fn start(mut self) {
        let mut next_state = Some(Box::new(Hello {}) as Box<dyn State>);
        while let Some(cur_state) = next_state.take() {
            debug!("Entering: {:?}", &cur_state);
            next_state = cur_state.next(&mut self).await;
        }
    }

    fn peer_id(&self) -> Option<NodeId> {
        self.peer.as_ref().map(|p| p.peer_id).or(self.expected)
    }
}

#[async_trait]
trait State: Send + std::fmt::Debug {
    async fn next(self: Box<Self>, _state_machine: &mut IpcPeer) -> Option<Box<dyn State>>;
}

#[derive(Debug)]
struct Hello {}

#[async_trait]
impl State for Hello {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        let our_id = state_machine.our_nodeid;
        match timeout(
            HANDSHAKE_TIMEOUT,
            state_machine.stream.send(IpcMessage::Hello(our_id)),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                debug!("IPC hello send failed: {e}");
                return b(AbortHandshake {});
            }
            Err(_) => {
                debug!("IPC hello timed out");
                return b(AbortHandshake {});
            }
        }
        let hello = match timeout(HANDSHAKE_TIMEOUT, state_machine.stream.next()).await {
            Ok(hello) => hello,
            Err(_) => {
                debug!("IPC hello timed out");
                return b(AbortHandshake {});
            }
        };

        let peer_id = match hello {
            Some(Ok(IpcMessage::Hello(id))) => id,
            other => {
                debug!("IPC hello expected Hello, got {other:?}");
                return b(AbortHandshake {});
            }
        };

        if let Some(expected) = state_machine.expected
            && expected != peer_id
        {
            debug!("IPC hello mismatch: expected {expected}, got {peer_id}");
            return b(AbortHandshake {});
        }

        let connection_id = state_machine.connection_counter.next();
        let mut realms: RealmList = Realm::Userspace.into();
        realms.add(Realm::Global);
        state_machine.peer = Some(Peer::register_peer(
            peer_id,
            state_machine.our_nodeid,
            state_machine.handle.clone(),
            Realm::Userspace,
            connection_id,
            10.into(),
            realms,
            "",
        ));

        if let Err(e) = state_machine
            .ipc_command
            .send(IpcCommand::SessionReady {
                peer_id,
                control: state_machine.control_tx.clone(),
                connection_id,
            })
            .await
        {
            debug!("Failed to send SessionReady: {e}");
            return b(AbortHandshake {});
        }

        let accepted = match timeout(HANDSHAKE_TIMEOUT, state_machine.ipc_control.recv()).await {
            Ok(Some(IpcControl::Accepted)) => true,
            Ok(Some(IpcControl::Shutdown | IpcControl::Resume)) | Ok(None) => false,
            Err(_) => {
                debug!("Timed out waiting for session accept");
                false
            }
        };
        if !accepted {
            return b(AbortHandshake {});
        }

        state_machine.established = true;
        info!("New connection to: {peer_id}");
        b(SendPeers {})
    }
}

#[derive(Debug)]
struct AbortHandshake {}

#[async_trait]
impl State for AbortHandshake {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        state_machine.stream.close().await.ok();
        if let Some(peer) = state_machine.peer.as_mut() {
            peer.unregister();
        }
        state_machine
            .ipc_command
            .send(IpcCommand::HandshakeFailed(state_machine.expected))
            .await
            .ok();
        None
    }
}

#[derive(Debug)]
struct SendPeers {}

#[async_trait]
impl State for SendPeers {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        let peer_id = state_machine.peer_id().expect("session established");
        let mut peers = state_machine
            .ipc_neighbors
            .read()
            .await
            .iter()
            .map(|(uuid, _tx, _conn)| *uuid)
            .filter(|u| *u != peer_id)
            .collect::<Vec<_>>();
        peers.push(state_machine.our_nodeid);
        debug!("Sending Peers: {:?}", &peers);
        if peers.is_empty() {
            return b(WaitForMessages {});
        }
        match state_machine
            .stream
            .send(IpcMessage::KnownPeers(peers))
            .await
        {
            Ok(_) => Some(Box::new(WaitForMessages {})),
            Err(e) => Some(Box::new(HandleError { error: e.into() })),
        }
    }
}

#[derive(Debug)]
struct WaitForMessages {}

#[async_trait]
impl State for WaitForMessages {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        let peer = state_machine.peer.as_mut().expect("session established");
        select! {
            msg = state_machine.stream.next() => {
                match msg {
                    Some(Ok(ipc_message)) => {
                        state_machine.hb.on_rx(Instant::now());
                        Some(Box::new(IpcMessageReceived { message: ipc_message}))
                    },
                    Some(Err(e)) => Some(Box::new(HandleError { error: e.into()})),
                    None => Some(Box::new(ClosePeer {})),
                }
            }
            control_msg = state_machine.ipc_control.recv() => {
                match control_msg {
                    Some(control_msg) => Some(Box::new(IpcControlReceived { message: control_msg})),
                    None  => {
                        tracing::error!("control_msg returned None");
                        Some(Box::new(Shutdown {}))},
                }
            }
            peer_msg = peer.recv() => {
                match peer_msg {
                    Some(node_msg) => Some(Box::new(NodeMessageReceived {message: node_msg})),
                    None  => {
                        tracing::error!("peer_msg returned None");
                        Some(Box::new(Shutdown {}))},
                }
            }
            _ = tokio::time::sleep_until(state_machine.hb.next_deadline()) => {
                Some(Box::new(HeartbeatTick {}))
            }
        }
    }
}

#[derive(Debug)]
struct HeartbeatTick {}

#[async_trait]
impl State for HeartbeatTick {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        let now = Instant::now();
        if state_machine.hb.timed_out(now) {
            error!(
                peer_id = %state_machine.peer_id().unwrap_or_default(),
                "IPC heartbeat timed out"
            );
            return Some(Box::new(ClosePeer {}));
        }
        if state_machine.hb.ping_due(now) {
            return b(SendPing {});
        }
        b(WaitForMessages {})
    }
}

#[derive(Debug)]
struct SendPing {}

#[async_trait]
impl State for SendPing {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        let token = state_machine.hb.take_ping_token(Instant::now());
        match state_machine.stream.send(IpcMessage::Ping(token)).await {
            Ok(_) => Some(Box::new(WaitForMessages {})),
            Err(e) => Some(Box::new(HandleError { error: e.into() })),
        }
    }
}

#[derive(Debug)]
struct HandleError {
    error: Box<dyn std::error::Error + Send + Sync>,
}
#[async_trait]
impl State for HandleError {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        error!(
            "Received Error in {} IPC peer handler: {:?}",
            state_machine
                .peer_id()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "handshake".into()),
            self.error
        );
        if state_machine.established {
            Some(Box::new(ClosePeer {}))
        } else {
            b(AbortHandshake {})
        }
    }
}

#[derive(Debug)]
struct NodeMessageReceived {
    message: NodeMessage,
}

#[async_trait]
impl State for NodeMessageReceived {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        match state_machine
            .stream
            .send(IpcMessage::NodeMsg(self.message))
            .await
        {
            Ok(_) => Some(Box::new(WaitForMessages {})),
            Err(e) => Some(Box::new(HandleError { error: e.into() })),
        }
    }
}

#[derive(Debug)]
struct IpcControlReceived {
    message: IpcControl,
}

#[async_trait]
impl State for IpcControlReceived {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        match self.message {
            IpcControl::Shutdown => Some(Box::new(Shutdown {})),
            IpcControl::Accepted => b(WaitForMessages {}),
            IpcControl::Resume => {
                let now = Instant::now();
                state_machine.hb.note_resume(now);
                b(SendPing {})
            }
        }
    }
}

#[derive(Debug)]
struct IpcMessageReceived {
    message: IpcMessage,
}

#[async_trait]
impl State for IpcMessageReceived {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        match self.message {
            IpcMessage::Hello(_uuid) => {}
            IpcMessage::KnownPeers(uuids) => {
                state_machine
                    .ipc_command
                    .send(IpcCommand::LearnedPeers(uuids))
                    .await
                    .map_err(|e| debug!("Failed to send LearnedPeers: {}", e))
                    .ok();
            }
            IpcMessage::NeighborRemoved(_uuid) => {}
            IpcMessage::CloseConnection => return Some(Box::new(ClosePeer {})),
            IpcMessage::Ping(token) => {
                if let Err(e) = state_machine.stream.send(IpcMessage::Pong(token)).await {
                    return Some(Box::new(HandleError { error: e.into() }));
                }
            }
            IpcMessage::Pong(_token) => {}
            IpcMessage::NodeMsg(node_message) => {
                if let Some(peer) = state_machine.peer.as_mut() {
                    peer.handle_node_message(node_message);
                }
            }
        }
        Some(Box::new(WaitForMessages {}))
    }
}

#[derive(Debug)]
struct ClosePeer {}

#[async_trait]
impl State for ClosePeer {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        state_machine.stream.close().await.ok();
        if state_machine.established {
            if let Some(peer_id) = state_machine.peer_id() {
                let connection_id = state_machine
                    .peer
                    .as_ref()
                    .expect("established")
                    .connection_id;
                state_machine
                    .ipc_command
                    .send(IpcCommand::PeerClosed(peer_id, connection_id))
                    .await
                    .ok();
            }
            if let Some(peer) = state_machine.peer.as_mut() {
                peer.unregister();
            }
        } else {
            state_machine
                .ipc_command
                .send(IpcCommand::HandshakeFailed(state_machine.expected))
                .await
                .ok();
        }
        state_machine.ipc_control.close();
        state_machine.stream.close().await.ok();

        None
    }
}

#[derive(Debug)]
struct Shutdown {}

#[async_trait]
impl State for Shutdown {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        state_machine
            .stream
            .send(IpcMessage::CloseConnection)
            .await
            .ok();
        b(ClosePeer {})
    }
}
