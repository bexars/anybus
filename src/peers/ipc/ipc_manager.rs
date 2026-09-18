use crate::tokio;
use std::{collections::HashSet, panic::Location, sync::Arc, time::Duration};

use async_bincode::tokio::AsyncBincodeStream;
use async_trait::async_trait;
use futures::{SinkExt, StreamExt, future};
use interprocess::local_socket::{
    self, GenericNamespaced, ToNsName as _,
    traits::tokio::{Listener, Stream as _},
};
use itertools::Itertools;
use thiserror::Error;
use tokio::{
    select,
    sync::{
        RwLock,
        mpsc::{self, channel},
    },
};

use tracing::{debug, error, info};

use crate::{
    AnyBusStatusMsg, Handle, Realm, Receiver,
    peers::{
        common::Peer,
        ipc::{IpcCommand, IpcControl, IpcMessage, IpcPeerStream, NameHelper, ipc_peer::IpcPeer},
    },
    routing::{ConnectionIdCounter, NodeId, RealmList},
    spawn,
};

fn b<T: State + 'static>(thing: T) -> Option<Box<dyn State>> {
    Some(Box::new(thing))
}

fn to_ipc_stream(stream: local_socket::tokio::Stream) -> IpcPeerStream {
    AsyncBincodeStream::from(stream).for_async()
}

const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(45);

pub(crate) struct IpcManager {
    primary_name: String,
    backup_name: String,
    handle: Handle,
    peers: Arc<RwLock<Vec<(NodeId, mpsc::Sender<IpcControl>)>>>,
    tx: mpsc::Sender<IpcCommand>,
    rx: mpsc::Receiver<IpcCommand>,
    our_nodeid: NodeId,
    primary_listener: Option<local_socket::tokio::Listener>,
    backup_listener: Option<local_socket::tokio::Listener>,
    peer_listener: Option<local_socket::tokio::Listener>,
    pending: HashSet<NodeId>,
    anybus_status: Receiver<AnyBusStatusMsg>,
    connection_counter: ConnectionIdCounter,
    heartbeat_interval: Duration,
    heartbeat_timeout: Duration,
    shutting_down: bool,
}
impl IpcManager {
    pub(crate) async fn new(
        rendezvous: String,
        handle: Handle,
        our_nodeid: NodeId,
        connection_counter: ConnectionIdCounter,
    ) -> Self {
        let (tx, rx) = channel(32);
        let anybus_status = handle
            .get_anybus_status_receiver()
            .await
            .expect("Unable to create anybus status receiver");
        IpcManager {
            primary_name: format!("{rendezvous}.primary"),
            backup_name: format!("{rendezvous}.backup"),
            handle,
            peers: Default::default(),
            tx,
            rx,
            our_nodeid,
            primary_listener: None,
            backup_listener: None,
            peer_listener: None,
            pending: HashSet::new(),
            anybus_status,
            connection_counter,
            heartbeat_interval: DEFAULT_HEARTBEAT_INTERVAL,
            heartbeat_timeout: DEFAULT_HEARTBEAT_TIMEOUT,
            shutting_down: false,
        }
    }

    pub(crate) async fn start(mut self) {
        let mut state = Some(Box::new(Creation {}) as Box<dyn State>);
        while let Some(old_state) = state.take() {
            debug!("Entering: {:?}", &old_state);
            state = old_state.next(&mut self).await;
        }
    }

    /// Position among live IPC nodes by NodeId (v7 ids: older = smaller).
    async fn rank(&self) -> usize {
        self.peers
            .read()
            .await
            .iter()
            .map(|p| p.0)
            .chain(Some(self.our_nodeid))
            .sorted_unstable()
            .find_position(|u| *u == self.our_nodeid)
            .map(|(p, _)| p)
            .unwrap()
    }

    async fn directory_peer_list(&self) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = self.peers.read().await.iter().map(|p| p.0).collect();
        ids.push(self.our_nodeid);
        ids
    }

    fn bind_named(name: &str) -> Option<local_socket::tokio::Listener> {
        let ns = name
            .to_string()
            .to_ns_name::<GenericNamespaced>()
            .expect("IPC directory name is hardcoded and tested so shouldn't cause a failure");
        let listener_opts = local_socket::ListenerOptions::new()
            .nonblocking(local_socket::ListenerNonblockingMode::Neither)
            .name(ns)
            .reclaim_name(true);
        match listener_opts.create_tokio() {
            Ok(listener) => Some(listener),
            Err(e) => {
                debug!("Failed to bind IPC name {name}: {e}");
                None
            }
        }
    }

    async fn query_directory(name: &str) -> Option<Vec<NodeId>> {
        let ns = name
            .to_string()
            .to_ns_name::<GenericNamespaced>()
            .expect("IPC directory name is hardcoded and tested so shouldn't cause a failure");
        let stream = match local_socket::tokio::Stream::connect(ns).await {
            Ok(stream) => stream,
            Err(e) => {
                debug!("Failed to connect to IPC directory {name}: {e}");
                return None;
            }
        };
        let mut stream = to_ipc_stream(stream);
        let peers = match stream.next().await {
            Some(Ok(IpcMessage::KnownPeers(ids))) => Some(ids),
            other => {
                debug!("IPC directory {name} did not send KnownPeers: {other:?}");
                None
            }
        };
        stream.close().await.ok();
        peers
    }

    async fn connect_node(id: NodeId) -> Option<IpcPeerStream> {
        let stream = match local_socket::tokio::Stream::connect(id.to_name()).await {
            Ok(stream) => stream,
            Err(e) => {
                tracing::error!("Failed to connect to IPC peer {id}: {e}");
                return None;
            }
        };
        Some(to_ipc_stream(stream))
    }

    fn drop_directory_listeners(&mut self) {
        self.primary_listener = None;
        self.backup_listener = None;
    }
}

#[async_trait]
trait State: Send + std::fmt::Debug {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>>;
}

#[derive(Debug)]
struct Creation {}

#[async_trait]
impl State for Creation {
    async fn next(self: Box<Self>, _state: &mut IpcManager) -> Option<Box<dyn State>> {
        b(StartPeerListener { query: true })
    }
}

#[derive(Debug)]
struct StartPeerListener {
    query: bool,
}

#[async_trait]
impl State for StartPeerListener {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        if state.peer_listener.is_none() {
            let name = state.our_nodeid.to_name();
            let listener_opts = local_socket::ListenerOptions::new()
                .nonblocking(local_socket::ListenerNonblockingMode::Neither)
                .name(name)
                .reclaim_name(true);
            match listener_opts.create_tokio() {
                Ok(listener) => state.peer_listener = Some(listener),
                Err(e) => {
                    error!("Failed to bind IPC peer listener: {e}");
                    return b(Shutdown {});
                }
            }
        }
        if self.query {
            b(QueryDirectory {
                last_bind_failed: false,
            })
        } else {
            b(Listen::default())
        }
    }
}

#[derive(Debug)]
struct QueryDirectory {
    last_bind_failed: bool,
}

#[async_trait]
impl State for QueryDirectory {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        if let Some(ids) = IpcManager::query_directory(&state.primary_name).await {
            return b(DialPeers {
                ids,
                from_directory: true,
            });
        }
        if let Some(ids) = IpcManager::query_directory(&state.backup_name).await {
            return b(DialPeers {
                ids,
                from_directory: true,
            });
        }
        if self.last_bind_failed {
            error!("Unable to connect or create IPC directory. Closing IPC manager");
            return b(Shutdown {});
        }
        b(BindPrimary {})
    }
}

#[derive(Debug)]
struct BindPrimary {}

#[async_trait]
impl State for BindPrimary {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        if state.shutting_down {
            return b(Listen { shutdown: true });
        }
        if state.primary_listener.is_none() {
            match IpcManager::bind_named(&state.primary_name) {
                Some(listener) => state.primary_listener = Some(listener),
                None => {
                    return b(QueryDirectory {
                        last_bind_failed: true,
                    });
                }
            }
        }
        state.backup_listener = None;
        b(Listen::default())
    }
}

#[derive(Debug)]
struct BindBackup {}

#[async_trait]
impl State for BindBackup {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        if state.shutting_down {
            return b(Listen { shutdown: true });
        }
        state.primary_listener = None;
        if state.backup_listener.is_none() {
            state.backup_listener = IpcManager::bind_named(&state.backup_name);
            if state.backup_listener.is_none() {
                debug!("Failed to bind IPC backup directory");
            }
        }
        b(Listen::default())
    }
}

#[derive(Debug)]
struct DialPeers {
    ids: Vec<NodeId>,
    from_directory: bool,
}

#[async_trait]
impl State for DialPeers {
    async fn next(mut self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        if state.shutting_down {
            return b(Listen { shutdown: true });
        }
        let existing: HashSet<NodeId> =
            state.peers.read().await.iter().map(|(id, _)| *id).collect();
        self.ids.retain(|id| {
            *id != state.our_nodeid
                && !existing.contains(id)
                && !state.pending.contains(id)
                && (self.from_directory || *id > state.our_nodeid)
        });

        let mut streams = Vec::new();
        for peer_id in self.ids {
            if let Some(stream) = IpcManager::connect_node(peer_id).await {
                state.pending.insert(peer_id);
                streams.push((peer_id, stream));
            }
        }

        match streams.pop() {
            Some((expected, stream)) => b(HandShake {
                stream,
                expected: Some(expected),
                extra_streams: streams,
            }),
            None => b(ReconcileDirectory {}),
        }
    }
}

#[derive(Debug)]
struct HandShake {
    stream: IpcPeerStream,
    expected: Option<NodeId>,
    extra_streams: Vec<(NodeId, IpcPeerStream)>,
}

#[async_trait]
impl State for HandShake {
    async fn next(mut self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        let next_extra = |this: Box<HandShake>, state: &mut IpcManager, error: IpcManagerError| {
            if let Some(id) = this.expected {
                state.pending.remove(&id);
            }
            let mut extra_streams = this.extra_streams;
            match extra_streams.pop() {
                Some((expected, stream)) => b(HandShake {
                    stream,
                    expected: Some(expected),
                    extra_streams,
                }),
                None => b(HandleError::new(error)),
            }
        };

        if let Err(error) = self.stream.send(IpcMessage::Hello(state.our_nodeid)).await {
            return next_extra(self, state, error.into());
        }
        let hello = self.stream.next().await;
        let peer_id = match hello {
            Some(Ok(IpcMessage::Hello(uuid))) => uuid,
            _ => {
                return next_extra(
                    self,
                    state,
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Did not receive Hello from peer",
                    )
                    .into(),
                );
            }
        };
        if let Some(expected) = self.expected
            && expected != peer_id
        {
            debug!("IPC hello mismatch: expected {expected}, got {peer_id}");
            return next_extra(
                self,
                state,
                std::io::Error::new(std::io::ErrorKind::InvalidData, "IPC hello mismatch").into(),
            );
        }
        b(CreateIpcPeer {
            stream: self.stream,
            peer_id,
            extra_streams: self.extra_streams,
        })
    }
}

#[derive(Debug)]
struct ServeDirectory {
    stream: IpcPeerStream,
}

#[async_trait]
impl State for ServeDirectory {
    async fn next(mut self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        let ids = state.directory_peer_list().await;
        if let Err(error) = self.stream.send(IpcMessage::KnownPeers(ids)).await {
            return b(HandleError::new(error));
        }
        self.stream.close().await.ok();
        b(Listen::default())
    }
}

#[derive(Debug)]
struct ReconcileDirectory {}

#[async_trait]
impl State for ReconcileDirectory {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        if state.shutting_down {
            return b(Listen { shutdown: true });
        }
        match state.rank().await {
            0 => {
                let need_bind = state.primary_listener.is_none();
                info!("IPC rank 0: holding primary directory");
                if need_bind {
                    b(BindPrimary {})
                } else {
                    state.backup_listener = None;
                    b(Listen::default())
                }
            }
            1 => {
                info!("IPC rank 1: holding backup directory");
                b(BindBackup {})
            }
            rank => {
                debug!("IPC rank {rank}: no directory sockets");
                state.drop_directory_listeners();
                b(Listen::default())
            }
        }
    }
}

#[derive(Debug, Default)]
struct Listen {
    shutdown: bool,
}

#[async_trait]
impl State for Listen {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        if !self.shutdown && !state.shutting_down && state.peer_listener.is_none() {
            return b(StartPeerListener { query: false });
        }

        let accepting = !self.shutdown && !state.shutting_down;

        select! {
            result = async {
                if !accepting {
                    return future::pending::<std::io::Result<local_socket::tokio::Stream>>().await;
                }
                match state.peer_listener.as_ref() {
                    Some(listener) => listener.accept().await,
                    None => future::pending().await,
                }
            } => {
                match result {
                    Ok(stream) => b(HandShake {
                        stream: to_ipc_stream(stream),
                        expected: None,
                        extra_streams: vec![],
                    }),
                    Err(err) => b(HandleError::new(err)),
                }
            }
            result = async {
                if !accepting {
                    return future::pending::<std::io::Result<local_socket::tokio::Stream>>().await;
                }
                match state.primary_listener.as_ref() {
                    Some(listener) => listener.accept().await,
                    None => future::pending().await,
                }
            } => {
                match result {
                    Ok(stream) => b(ServeDirectory {
                        stream: to_ipc_stream(stream),
                    }),
                    Err(err) => b(HandleError::new(err)),
                }
            }
            result = async {
                if !accepting {
                    return future::pending::<std::io::Result<local_socket::tokio::Stream>>().await;
                }
                match state.backup_listener.as_ref() {
                    Some(listener) => listener.accept().await,
                    None => future::pending().await,
                }
            } => {
                match result {
                    Ok(stream) => b(ServeDirectory {
                        stream: to_ipc_stream(stream),
                    }),
                    Err(err) => b(HandleError::new(err)),
                }
            }
            Some(msg) = state.rx.recv() => {
                b(HandleIpcCommand { command: msg })
            }
            status = state.anybus_status.recv() => {
                match status {
                    Ok(AnyBusStatusMsg::ShuttingDown) => b(SoftShutdown {}),
                    Err(_) => b(Shutdown {}),
                    _ => b(Listen {
                        shutdown: self.shutdown || state.shutting_down,
                    }),
                }
            }
        }
    }
}

#[derive(Debug)]
struct SoftShutdown {}

#[async_trait]
impl State for SoftShutdown {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        state.shutting_down = true;
        state.peer_listener = None;
        state.drop_directory_listeners();
        b(Listen { shutdown: true })
    }
}

#[derive(Debug)]
struct Shutdown {}

#[async_trait]
impl State for Shutdown {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        state.peer_listener = None;
        state.drop_directory_listeners();
        for (_id, tx) in state.peers.write().await.drain(..) {
            tx.send(IpcControl::Shutdown).await.ok();
        }
        None
    }
}

#[derive(Debug)]
struct HandleError {
    error: IpcManagerError,
    location: &'static Location<'static>,
}

impl HandleError {
    #[track_caller]
    fn new(error: impl Into<IpcManagerError>) -> Self {
        Self {
            error: error.into(),
            location: Location::caller(),
        }
    }
}

#[async_trait]
impl State for HandleError {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        error!(
            "IPC Manager error: {:?} {}:{}",
            self.error,
            self.location.file(),
            self.location.line()
        );
        b(Listen {
            shutdown: state.shutting_down,
        })
    }
}

#[derive(Debug)]
struct HandleIpcCommand {
    command: IpcCommand,
}

#[async_trait]
impl State for HandleIpcCommand {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        match self.command {
            IpcCommand::PeerClosed(uuid) => {
                state.peers.write().await.retain(|(id, _)| *id != uuid);
                state.pending.remove(&uuid);
                tracing::info!("Peer Closed: {uuid}");
                b(ReconcileDirectory {})
            }
            IpcCommand::LearnedPeers(ids) => b(DialPeers {
                ids,
                from_directory: false,
            }),
        }
    }
}

#[derive(Debug)]
struct CreateIpcPeer {
    stream: IpcPeerStream,
    peer_id: NodeId,
    extra_streams: Vec<(NodeId, IpcPeerStream)>,
}

#[async_trait]
impl State for CreateIpcPeer {
    async fn next(mut self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        state.pending.remove(&self.peer_id);

        let already_connected = state
            .peers
            .read()
            .await
            .iter()
            .any(|(id, _)| *id == self.peer_id);
        if already_connected {
            debug!("Dropping duplicate IPC connection to {}", self.peer_id);
            self.stream.close().await.ok();
            return match self.extra_streams.pop() {
                Some((expected, stream)) => b(HandShake {
                    stream,
                    expected: Some(expected),
                    extra_streams: self.extra_streams,
                }),
                None => b(ReconcileDirectory {}),
            };
        }

        let connection_id = state.connection_counter.next();
        let (tx, rx) = channel(32);
        let mut realms: RealmList = Realm::Userspace.into();
        realms.add(Realm::Global);

        let peer = Peer::register_peer(
            self.peer_id,
            state.our_nodeid,
            state.handle.clone(),
            Realm::Userspace, // Always userspace for IPC peers
            connection_id,
            10.into(),
            realms,
        );
        let ipc_peer = IpcPeer::new(
            self.stream,
            state.tx.clone(),
            rx,
            state.peers.clone(),
            peer,
            state.heartbeat_interval,
            state.heartbeat_timeout,
        );

        state.peers.write().await.push((self.peer_id, tx));
        _ = spawn(ipc_peer.start());

        match self.extra_streams.pop() {
            Some((expected, stream)) => b(HandShake {
                stream,
                expected: Some(expected),
                extra_streams: self.extra_streams,
            }),
            None => b(ReconcileDirectory {}),
        }
    }
}

#[derive(Error, Debug)]
pub enum IpcManagerError {
    #[error("Error communicating with IPC peer: {0}")]
    Io(#[from] std::io::Error),
    #[error("Error encoding IPC message")]
    EncodeError(#[from] bincode::error::EncodeError),
}
