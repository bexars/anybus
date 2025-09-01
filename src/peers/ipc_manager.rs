use std::{collections::HashSet, sync::Arc};

use async_bincode::tokio::AsyncBincodeStream;
use async_trait::async_trait;
use futures::{SinkExt, StreamExt, future};
use interprocess::local_socket::{
    self, GenericNamespaced, ToNsName as _,
    traits::tokio::{Listener, Stream as _},
};
use thiserror::Error;
use tokio::{
    select,
    sync::{
        RwLock,
        mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
        watch,
    },
};
use tracing::trace;
use uuid::Uuid;

use crate::{
    Handle, Peer,
    messages::BusControlMsg,
    peers::{IpcCommand, IpcControl, IpcMessage, NameHelper, PeerStream, ipc::IpcPeer},
    spawn,
};

fn b<T: State + 'static>(thing: T) -> Option<Box<dyn State>> {
    Some(Box::new(thing))
}

pub(crate) struct IpcManager {
    rendezvous: String,
    handle: Handle,
    bus_control: watch::Receiver<BusControlMsg>,
    peers: Arc<RwLock<Vec<(Uuid, UnboundedSender<IpcControl>)>>>,
    tx: UnboundedSender<IpcCommand>,
    rx: UnboundedReceiver<IpcCommand>,
    uuid: Uuid,
    rendezvous_listener: Option<local_socket::tokio::Listener>,
    peer_listener: Option<local_socket::tokio::Listener>,
}
impl IpcManager {
    pub(crate) fn new(
        rendezvous: String,
        handle: Handle,
        bus_control: watch::Receiver<BusControlMsg>,
        uuid: Uuid,
    ) -> Self {
        let (tx, rx) = unbounded_channel();

        IpcManager {
            rendezvous,
            handle,
            bus_control,
            peers: Default::default(),
            tx,
            rx,
            uuid,
            rendezvous_listener: None,
            peer_listener: None,
        }
    }

    pub(crate) async fn start(mut self) {
        let mut state = Some(Box::new(Creation {}) as Box<dyn State>);
        while let Some(old_state) = state.take() {
            trace!("Entering: {:?}", &old_state);
            state = old_state.next(&mut self).await;
        }
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
        b(ConnectToRendezvous {})
    }
}

#[derive(Debug)]
struct ConnectToRendezvous {}

#[async_trait]
impl State for ConnectToRendezvous {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        let name = state
            .rendezvous
            .clone()
            .to_ns_name::<GenericNamespaced>()
            .unwrap();
        match local_socket::tokio::Stream::connect(name).await {
            Ok(stream) => {
                let stream: AsyncBincodeStream<
                    local_socket::tokio::Stream,
                    IpcMessage,
                    IpcMessage,
                    async_bincode::AsyncDestination,
                > = AsyncBincodeStream::from(stream).for_async();
                b(HandShake {
                    stream,
                    peer_is_master: true,
                    extra_streams: vec![],
                })
            }
            Err(_e) => b(StartRendezvous {}),
        }
    }
}

#[derive(Debug)]
struct HandShake {
    stream: PeerStream,
    peer_is_master: bool,
    extra_streams: Vec<PeerStream>,
}

#[async_trait]
impl State for HandShake {
    async fn next(mut self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        // let mut stream = AsyncBincodeStream::from(self.stream).for_async();
        if let Err(error) = self.stream.send(IpcMessage::Hello(state.uuid)).await {
            return b(HandleError::new(error));
        };
        let hello = self.stream.next().await;
        let peer_id = match hello {
            Some(Ok(IpcMessage::Hello(uuid))) => uuid,
            _ => {
                return b(HandleError {
                    error: std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Did not receive Hello from peer",
                    )
                    .into(),
                });
            }
        };
        b(CreateIpcPeer {
            stream: self.stream,
            peer_id,
            peer_is_master: self.peer_is_master,
            extra_streams: self.extra_streams,
        })
    }
}

#[derive(Debug)]
struct StartRendezvous {}

#[async_trait]
impl State for StartRendezvous {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        let name = state
            .rendezvous
            .clone()
            .to_ns_name::<GenericNamespaced>()
            .unwrap();
        let listener_opts = local_socket::ListenerOptions::new()
            .nonblocking(local_socket::ListenerNonblockingMode::Neither)
            .name(name)
            .reclaim_name(true);

        #[cfg(unix)]
        let _ = {
            use std::path::PathBuf;
            let path = PathBuf::from("/tmp").join(&state.rendezvous);
            _ = std::fs::remove_file(path);
        };
        state.rendezvous_listener = match listener_opts.create_tokio() {
            Ok(rl) => Some(rl),
            Err(e) => return b(HandleError::new(e)),
        };
        b(AnnounceMaster {})
    }
}

#[derive(Debug)]
struct AnnounceMaster {}

#[async_trait]
impl State for AnnounceMaster {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        let _dead_peers = state
            .peers
            .read()
            .await
            .iter()
            .map(|(id, tx)| (id, tx.send(IpcControl::IAmMaster)))
            .filter_map(|(id, res)| if res.is_err() { Some(id) } else { None });
        b(Listen {})
    }
}

#[derive(Debug)]
struct Listen {}

#[async_trait]
impl State for Listen {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        if state.peer_listener.is_none() {
            return b(StartListener {});
        }

        let ipc_listeners = state
            .peer_listener
            .as_ref()
            .map(|l| Box::pin(l.accept()))
            .into_iter()
            .chain(
                state
                    .rendezvous_listener
                    .as_ref()
                    .map(|l| Box::pin(l.accept())),
            );

        let ipc_listeners = future::select_all(ipc_listeners);

        select! {
                    (stream, _idx, _vec) = ipc_listeners => {
                        match stream {
                            Ok(stream) => b(HandShake {
                               stream: AsyncBincodeStream::from(stream).for_async(),
                               peer_is_master: false,
                               extra_streams: vec![],
        }),
                            Err(err) => b(HandleError::new(err)),
                        }
                    }
                    Some(msg) = state.rx.recv() => {
                        b(HandleIpcCommand { command: msg } )
                    }
                    msg = state.bus_control.changed() => {
                        if msg.is_ok() {
                            let control_msg = state.bus_control.borrow();
                            match *control_msg {
                                BusControlMsg::Shutdown=> b(Shutdown {}),
                                BusControlMsg::Run => b(Listen {}),
                            }
                        } else {
                            None
                        }
                    }
                }
    }
}

#[derive(Debug)]
struct Shutdown {}

#[async_trait]
impl State for Shutdown {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        for (_id, tx) in state.peers.write().await.drain(..) {
            let _ = tx.send(IpcControl::Shutdown);
            // state.handle.unregister_peer(id);
        }
        None
    }
}

#[derive(Debug)]
struct StartListener {}

#[async_trait]
impl State for StartListener {
    async fn next(self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        let name = state.uuid.to_name();

        let listener_opts = local_socket::ListenerOptions::new()
            .nonblocking(local_socket::ListenerNonblockingMode::Neither)
            .name(name)
            .reclaim_name(true);
        state.peer_listener = listener_opts.create_tokio().ok(); // If it failed we just won't listen and hope someone else is listening
        b(Listen {})
    }
}

#[derive(Debug)]
struct HandleError {
    error: IpcManagerError,
}

impl HandleError {
    fn new(error: impl Into<IpcManagerError>) -> Self {
        Self {
            error: error.into(),
        }
    }
}

#[async_trait]
impl State for HandleError {
    async fn next(self: Box<Self>, _state: &mut IpcManager) -> Option<Box<dyn State>> {
        eprintln!("IPC Manager error: {:?}", self.error);
        b(Listen {})
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
            IpcCommand::PeerClosed(uuid, was_master) => {
                state.peers.write().await.retain(|(id, _)| *id != uuid);
                if was_master {
                    b(StartRendezvous {})
                } else {
                    b(Listen {})
                }
            }
            IpcCommand::LearnedPeers(mut peer_ids) => {
                let mut streams = Vec::new();
                let existing_peers = state
                    .peers
                    .read()
                    .await
                    .iter()
                    .map(|(id, _)| *id)
                    .collect::<HashSet<_>>();

                peer_ids.retain(|id| !existing_peers.contains(id));

                // let peer_ids: Vec<_> = peer_ids
                //     .difference(&existing_peers)
                //     .filter(|u| **u != state.uuid)
                //     .cloned()
                //     .collect();

                for peer_id in peer_ids {
                    let name = peer_id.to_name();
                    let stream = local_socket::tokio::Stream::connect(name).await;
                    if stream.is_err() {
                        continue;
                    }
                    let stream = stream.unwrap();
                    streams.push(AsyncBincodeStream::from(stream).for_async())
                }

                if streams.is_empty() {
                    return b(Listen {});
                }
                let first = streams.pop();
                b(HandShake {
                    stream: first.unwrap(),
                    peer_is_master: false,
                    extra_streams: streams,
                })
            }
        }
    }
}

#[derive(Debug)]
struct CreateIpcPeer {
    stream: PeerStream,
    peer_id: Uuid,
    peer_is_master: bool,
    extra_streams: Vec<PeerStream>,
}

#[async_trait]
impl State for CreateIpcPeer {
    async fn next(mut self: Box<Self>, state: &mut IpcManager) -> Option<Box<dyn State>> {
        let (tx, rx) = unbounded_channel();
        let (node_tx, node_rx) = unbounded_channel();
        let peer = Peer::new(self.peer_id, state.uuid, state.handle.clone(), node_rx);
        let ipc_peer = IpcPeer::new(
            self.stream,
            state.tx.clone(),
            rx,
            state.peers.clone(),
            peer,
            self.peer_is_master,
        );
        state.peers.write().await.push((self.peer_id, tx));
        state.handle.register_peer(self.peer_id, node_tx);
        _ = spawn(ipc_peer.start());

        let s = self.extra_streams.pop(); // Handle learning multiple peers at once
        match s {
            Some(stream) => b(HandShake {
                stream,
                peer_is_master: false,
                extra_streams: self.extra_streams,
            }),
            None => b(Listen {}),
        }
    }
}

#[derive(Error, Debug)]
pub enum IpcManagerError {
    #[error("Error communicating with IPC peer: {0}")]
    Io(#[from] std::io::Error),
    #[error("Error encoding IPC message: {0}")]
    EncodeError(#[from] bincode::error::EncodeError),
}
