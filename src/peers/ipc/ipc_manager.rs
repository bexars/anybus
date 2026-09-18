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
    time::{Instant, timeout},
};

use tracing::{debug, error, info};

use crate::{
    AnyBusStatusMsg, Handle, Receiver,
    peers::ipc::{
        DirectoryView, IpcCommand, IpcControl, IpcMessage, IpcPeerStream, NameHelper,
        ipc_peer::IpcPeer,
    },
    routing::{ConnectionId, ConnectionIdCounter, NodeId},
    spawn,
};

fn b<T: State + 'static>(thing: T) -> Option<Box<dyn State>> {
    Some(Box::new(thing))
}

fn to_ipc_stream(stream: local_socket::tokio::Stream) -> IpcPeerStream {
    AsyncBincodeStream::from(stream).for_async()
}

const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);
const DIRECTORY_IO_TIMEOUT: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const DIRECTORY_RETRY_INTERVAL: Duration = Duration::from_millis(500);
const DIRECTORY_HEALTH_INTERVAL: Duration = Duration::from_secs(15);
const PRIMARY_TAKEOVER_MISSES: u8 = 3;
const DIRECTORY_OVERWRITE_AFTER: u8 = 3;

pub(crate) struct IpcManager {
    primary_name: String,
    backup_name: String,
    handle: Handle,
    peers: Arc<RwLock<Vec<(NodeId, mpsc::Sender<IpcControl>, ConnectionId)>>>,
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
    directory_wake_at: Option<Instant>,
    directory_wake_probe: bool,
    primary_misses: u8,
    primary_bind_in_use: u8,
    backup_bind_in_use: u8,
    probe_in_flight: bool,
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
            directory_wake_at: None,
            directory_wake_probe: false,
            primary_misses: 0,
            primary_bind_in_use: 0,
            backup_bind_in_use: 0,
            probe_in_flight: false,
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

    async fn has_unknown_peers(&self, ids: &[NodeId]) -> bool {
        let existing: HashSet<NodeId> = self.peers.read().await.iter().map(|p| p.0).collect();
        ids.iter().any(|id| {
            *id != self.our_nodeid && !existing.contains(id) && !self.pending.contains(id)
        })
    }

    fn bind_named(
        name: &str,
        overwrite: bool,
    ) -> Result<local_socket::tokio::Listener, std::io::Error> {
        let ns = name
            .to_string()
            .to_ns_name::<GenericNamespaced>()
            .expect("IPC directory name is hardcoded and tested so shouldn't cause a failure");
        let listener_opts = local_socket::ListenerOptions::new()
            .nonblocking(local_socket::ListenerNonblockingMode::Neither)
            .name(ns)
            .reclaim_name(true)
            .try_overwrite(overwrite);
        if overwrite {
            info!("Overwriting IPC name {name}");
        }
        match listener_opts.create_tokio() {
            Ok(listener) => Ok(listener),
            Err(e) => {
                debug!("Failed to bind IPC name {name}: {e}");
                Err(e)
            }
        }
    }

    fn bind_retry_delay(err: &std::io::Error) -> Duration {
        if err.kind() == std::io::ErrorKind::AddrInUse {
            DIRECTORY_HEALTH_INTERVAL
        } else {
            DIRECTORY_RETRY_INTERVAL
        }
    }

    async fn query_directory(name: &str) -> Option<DirectoryView> {
        match timeout(DIRECTORY_IO_TIMEOUT, Self::query_directory_inner(name)).await {
            Ok(view) => view,
            Err(_) => {
                debug!("IPC directory {name} timed out");
                None
            }
        }
    }

    async fn query_directory_inner(name: &str) -> Option<DirectoryView> {
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
        let owner = match stream.next().await {
            Some(Ok(IpcMessage::Hello(id))) => id,
            other => {
                debug!("IPC directory {name} did not send Hello: {other:?}");
                stream.close().await.ok();
                return None;
            }
        };
        let peers = match stream.next().await {
            Some(Ok(IpcMessage::KnownPeers(ids))) => ids,
            other => {
                debug!("IPC directory {name} did not send KnownPeers: {other:?}");
                stream.close().await.ok();
                return None;
            }
        };
        stream.close().await.ok();
        Some(DirectoryView { owner, peers })
    }

    async fn query_any_directory(&self) -> Option<Vec<NodeId>> {
        if let Some(view) = Self::query_directory(&self.primary_name).await {
            return Some(view.dial_ids());
        }
        Self::query_directory(&self.backup_name)
            .await
            .map(|view| view.dial_ids())
    }

    fn schedule_directory_wake(&mut self, delay: Duration, probe_primary: bool) {
        let at = Instant::now() + delay;
        match self.directory_wake_at {
            Some(existing) if existing <= at => {
                self.directory_wake_probe |= probe_primary;
            }
            _ => {
                self.directory_wake_at = Some(at);
                self.directory_wake_probe = probe_primary;
            }
        }
    }

    fn schedule_directory_health(&mut self) {
        if !self.shutting_down {
            self.schedule_directory_wake(DIRECTORY_HEALTH_INTERVAL, true);
        }
    }

    /// Query directory names from a task so `Listen` can still `accept()`.
    /// Self-connecting from `ReconcileDirectory` deadlocks: we leave Listen, so
    /// our own `ServeDirectory` never runs and the 2s query times out.
    fn spawn_directory_probe(&mut self) {
        if self.probe_in_flight || self.shutting_down {
            return;
        }
        self.probe_in_flight = true;
        let tx = self.tx.clone();
        let primary_name = self.primary_name.clone();
        let backup_name = self.backup_name.clone();
        let hold_primary = self.primary_listener.is_some();
        let hold_backup = self.backup_listener.is_some();
        spawn(async move {
            let primary = IpcManager::query_directory(&primary_name).await;
            let backup = if hold_backup {
                IpcManager::query_directory(&backup_name).await
            } else {
                None
            };
            tx.send(IpcCommand::DirectoryProbe {
                primary,
                primary_self: hold_primary,
                backup,
                backup_self: hold_backup,
            })
            .await
            .ok();
        });
    }

    async fn connect_node(id: NodeId) -> Option<IpcPeerStream> {
        let connect = local_socket::tokio::Stream::connect(id.to_name());
        let stream = match timeout(CONNECT_TIMEOUT, connect).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => {
                tracing::error!("Failed to connect to IPC peer {id}: {e}");
                return None;
            }
            Err(_) => {
                tracing::error!("Timed out connecting to IPC peer {id}");
                return None;
            }
        };
        Some(to_ipc_stream(stream))
    }

    fn drop_directory_listeners(&mut self) {
        self.primary_listener = None;
        self.backup_listener = None;
        self.directory_wake_at = None;
        self.directory_wake_probe = false;
        self.primary_misses = 0;
        self.primary_bind_in_use = 0;
        self.backup_bind_in_use = 0;
    }

    fn drop_backup(&mut self) {
        self.backup_listener = None;
    }

    fn spawn_peer(&mut self, stream: IpcPeerStream, expected: Option<NodeId>) {
        if let Some(id) = expected {
            self.pending.insert(id);
        }
        let ipc_peer = IpcPeer::new(
            stream,
            self.tx.clone(),
            self.peers.clone(),
            expected,
            self.our_nodeid,
            self.handle.clone(),
            self.connection_counter.clone(),
            self.heartbeat_interval,
            self.heartbeat_timeout,
        );
        spawn(ipc_peer.start());
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
            b(QueryDirectory {})
        } else {
            b(Listen::default())
        }
    }
}

#[derive(Debug)]
struct QueryDirectory {}

#[async_trait]
impl State for QueryDirectory {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        if let Some(ids) = state.query_any_directory().await {
            return b(DialPeers {
                ids,
                from_directory: true,
            });
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
            let overwrite = state.primary_bind_in_use >= DIRECTORY_OVERWRITE_AFTER;
            match IpcManager::bind_named(&state.primary_name, overwrite) {
                Ok(listener) => {
                    state.primary_listener = Some(listener);
                    state.primary_bind_in_use = 0;
                }
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::AddrInUse {
                        state.primary_bind_in_use = state.primary_bind_in_use.saturating_add(1);
                    }
                    debug!("Failed to bind IPC primary directory, retrying");
                    state.spawn_directory_probe();
                    state.schedule_directory_wake(IpcManager::bind_retry_delay(&err), false);
                    return b(Listen::default());
                }
            }
        }
        state.drop_backup();
        state.primary_misses = 0;
        state.schedule_directory_health();
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
            let overwrite = state.backup_bind_in_use >= DIRECTORY_OVERWRITE_AFTER;
            match IpcManager::bind_named(&state.backup_name, overwrite) {
                Ok(listener) => {
                    state.backup_listener = Some(listener);
                    state.backup_bind_in_use = 0;
                    state.schedule_directory_health();
                }
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::AddrInUse {
                        state.backup_bind_in_use = state.backup_bind_in_use.saturating_add(1);
                    }
                    debug!("Failed to bind IPC backup directory, retrying");
                    state.schedule_directory_wake(IpcManager::bind_retry_delay(&err), false);
                }
            }
        } else {
            state.schedule_directory_health();
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
        let existing: HashSet<NodeId> = state.peers.read().await.iter().map(|p| p.0).collect();
        self.ids.retain(|id| {
            *id != state.our_nodeid
                && !existing.contains(id)
                && !state.pending.contains(id)
                && (self.from_directory || *id > state.our_nodeid)
        });

        for peer_id in self.ids {
            if let Some(stream) = IpcManager::connect_node(peer_id).await {
                state.spawn_peer(stream, Some(peer_id));
            }
        }
        // Rank is wrong until SessionReady; reconciling here as rank 0 with an
        // empty peer list busy-loops BindPrimary while Hello is still in flight.
        state.schedule_directory_health();
        b(Listen {
            shutdown: state.shutting_down,
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
        let hello = IpcMessage::Hello(state.our_nodeid);
        let known = IpcMessage::KnownPeers(state.directory_peer_list().await);
        match timeout(DIRECTORY_IO_TIMEOUT, async {
            self.stream.send(hello).await?;
            self.stream.send(known).await
        })
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return b(HandleError::new(error)),
            Err(_) => return b(HandleError::new(IpcManagerError::TimedOut)),
        }
        self.stream.close().await.ok();
        b(Listen::default())
    }
}

#[derive(Debug, Default)]
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
                    state.drop_backup();
                    state.primary_misses = 0;
                    state.schedule_directory_health();
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
        // DialPeers / rank 2+ used to sit here with no timer: no logs, no bind retry,
        // no probe, until an inbound connect happened to arrive.
        if !self.shutdown && !state.shutting_down && state.directory_wake_at.is_none() {
            state.schedule_directory_health();
        }

        let accepting = !self.shutdown && !state.shutting_down;
        let directory_wake = state.directory_wake_at;

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
                    Ok(stream) => {
                        state.spawn_peer(to_ipc_stream(stream), None);
                        b(Listen {
                            shutdown: self.shutdown,
                        })
                    }
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
            _ = async {
                match directory_wake {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => future::pending().await,
                }
            }, if directory_wake.is_some() => {
                let probe = state.directory_wake_probe;
                state.directory_wake_at = None;
                state.directory_wake_probe = false;
                if probe {
                    if state.probe_in_flight {
                        state.schedule_directory_health();
                    } else {
                        state.spawn_directory_probe();
                    }
                    b(Listen {
                        shutdown: self.shutdown || state.shutting_down,
                    })
                } else {
                    b(ReconcileDirectory {})
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
        let senders: Vec<_> = state
            .peers
            .write()
            .await
            .drain(..)
            .map(|(_, tx, _)| tx)
            .collect();
        for tx in senders {
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
            IpcCommand::PeerClosed(uuid, connection_id) => {
                state
                    .peers
                    .write()
                    .await
                    .retain(|(id, _, conn)| !(*id == uuid && *conn == connection_id));
                if !state
                    .peers
                    .read()
                    .await
                    .iter()
                    .any(|(id, _, _)| *id == uuid)
                {
                    state.pending.remove(&uuid);
                }
                tracing::info!("Peer Closed: {uuid} ({connection_id})");
                b(ReconcileDirectory::default())
            }
            IpcCommand::SessionReady {
                peer_id,
                control,
                connection_id,
            } => {
                state.pending.remove(&peer_id);
                let mut peers = state.peers.write().await;
                if let Some(pos) = peers.iter().position(|(id, _, _)| *id == peer_id) {
                    let old_tx = peers[pos].1.clone();
                    debug!("Replacing IPC session to {peer_id} with connection {connection_id}");
                    peers[pos] = (peer_id, control.clone(), connection_id);
                    drop(peers);
                    old_tx.send(IpcControl::Shutdown).await.ok();
                    control.send(IpcControl::Accepted).await.ok();
                } else {
                    peers.push((peer_id, control.clone(), connection_id));
                    drop(peers);
                    control.send(IpcControl::Accepted).await.ok();
                }
                b(ReconcileDirectory::default())
            }
            IpcCommand::HandshakeFailed(id) => {
                if let Some(id) = id {
                    state.pending.remove(&id);
                }
                if state.primary_listener.is_none() && state.peers.read().await.is_empty() {
                    state.schedule_directory_wake(DIRECTORY_RETRY_INTERVAL, false);
                }
                b(Listen {
                    shutdown: state.shutting_down,
                })
            }
            IpcCommand::LearnedPeers(ids) => b(DialPeers {
                ids,
                from_directory: false,
            }),
            IpcCommand::DirectoryProbe {
                primary,
                primary_self,
                backup,
                backup_self,
            } => {
                state.probe_in_flight = false;
                if state.shutting_down {
                    return b(Listen { shutdown: true });
                }

                let mut name_lost = false;
                let mut stolen_ids = Vec::new();
                if primary_self {
                    match &primary {
                        Some(view) if view.owner == state.our_nodeid => {}
                        Some(view) => {
                            info!(
                                "Primary directory is owned by {}, dropping our listener",
                                view.owner
                            );
                            state.primary_listener = None;
                            name_lost = true;
                            stolen_ids.extend(view.dial_ids());
                        }
                        None => {
                            info!("Primary directory name no longer reaches us, rebinding");
                            state.primary_listener = None;
                            name_lost = true;
                        }
                    }
                }
                if backup_self {
                    match &backup {
                        Some(view) if view.owner == state.our_nodeid => {}
                        Some(view) => {
                            info!(
                                "Backup directory is owned by {}, dropping our listener",
                                view.owner
                            );
                            state.backup_listener = None;
                            name_lost = true;
                            stolen_ids.extend(view.dial_ids());
                        }
                        None => {
                            info!("Backup directory name no longer reaches us, rebinding");
                            state.backup_listener = None;
                            name_lost = true;
                        }
                    }
                }
                if !stolen_ids.is_empty() && state.has_unknown_peers(&stolen_ids).await {
                    state.schedule_directory_health();
                    return b(DialPeers {
                        ids: stolen_ids,
                        from_directory: true,
                    });
                }
                if name_lost {
                    return b(ReconcileDirectory {});
                }

                if !primary_self {
                    match &primary {
                        Some(_) => state.primary_misses = 0,
                        None => {
                            state.primary_misses = state.primary_misses.saturating_add(1);
                            debug!(
                                primary_misses = state.primary_misses,
                                "Primary directory unreachable"
                            );
                            if state.primary_misses >= PRIMARY_TAKEOVER_MISSES {
                                info!("Primary directory still unreachable, attempting takeover");
                                return b(BindPrimary {});
                            }
                        }
                    }
                }

                let mut ids = Vec::new();
                if let Some(view) = primary {
                    ids.extend(view.dial_ids());
                }
                if let Some(view) = backup {
                    ids.extend(view.dial_ids());
                }
                if state.has_unknown_peers(&ids).await {
                    state.schedule_directory_health();
                    return b(DialPeers {
                        ids,
                        from_directory: true,
                    });
                }

                state.schedule_directory_health();
                b(Listen {
                    shutdown: state.shutting_down,
                })
            }
        }
    }
}

#[derive(Error, Debug)]
pub enum IpcManagerError {
    #[error("Error communicating with IPC peer: {0}")]
    Io(#[from] std::io::Error),
    #[error("Error encoding IPC message")]
    EncodeError(#[from] bincode::error::EncodeError),
    #[error("IPC operation timed out")]
    TimedOut,
}
