use std::sync::Arc;

use async_bincode::tokio::AsyncBincodeStream;
use async_trait::async_trait;
use interprocess::local_socket::{
    self, GenericNamespaced, ToNsName as _, traits::tokio::Stream as _,
};
use tokio::sync::{
    RwLock,
    mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    watch,
};
use uuid::Uuid;

use crate::{
    Handle,
    messages::BusControlMsg,
    peers::{IpcCommand, IpcControl, IpcMessage, NameHelper, ipc::IpcPeer},
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

    pub(crate) async fn start(mut self: Self) {
        let mut state = Some(Box::new(Creation {}) as Box<dyn State>);
        loop {
            if let Some(old_state) = state.take() {
                state = old_state.next(&mut self).await;
                // println!("Entered: {:?}", &state);
            } else {
                break;
            }
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
        let stream = match local_socket::tokio::Stream::connect(name).await {
            Ok(stream) => {
                let stream: AsyncBincodeStream<
                    local_socket::tokio::Stream,
                    IpcMessage,
                    IpcMessage,
                    async_bincode::AsyncDestination,
                > = AsyncBincodeStream::from(stream).for_async();
                b(Handshake {
                    stream,
                    peer_is_master: true,
                })
            }
            Err(_e) => b(StartRendezvous {}),
        };
        b(ConnectToRendezvous {})
    }
}

#[derive(Debug)]
struct Handshake {
    stream: AsyncBincodeStream<
        local_socket::tokio::Stream,
        IpcMessage,
        IpcMessage,
        async_bincode::AsyncDestination,
    >,
    peer_is_master: bool,
}

#[async_trait]
impl State for Handshake {
    async fn next(self: Box<Self>, _state: &mut IpcManager) -> Option<Box<dyn State>> {
        b(ConnectToRendezvous {})
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
        state.rendezvous_listener = listener_opts.create_tokio().ok(); // If it failed we just won't listen and hope someone else is listening
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
        b(ConnectToRendezvous {})
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
