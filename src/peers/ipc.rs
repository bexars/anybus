// Cribbed the state machine from: https://moonbench.xyz/projects/rust-event-driven-finite-state-machine

use std::sync::Arc;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use tokio::{
    select,
    sync::{
        RwLock,
        mpsc::{UnboundedReceiver, UnboundedSender},
    },
};
use tracing::{error, trace};
use uuid::Uuid;

use crate::{
    Peer,
    messages::NodeMessage,
    peers::{IpcCommand, IpcControl, IpcMessage, PeerStream},
};

fn b<T: State + 'static>(thing: T) -> Option<Box<dyn State>> {
    Some(Box::new(thing))
}

pub(crate) struct IpcPeer {
    // phantom: PhantomData<T>,
    stream: PeerStream,
    ipc_command: UnboundedSender<IpcCommand>,
    ipc_control: UnboundedReceiver<IpcControl>,
    ipc_neighbors: Arc<RwLock<Vec<(Uuid, UnboundedSender<IpcControl>)>>>,
    peer: Peer,
    is_master: bool,
}

impl IpcPeer {
    pub(crate) fn new(
        stream: PeerStream,
        ipc_command: UnboundedSender<IpcCommand>,
        ipc_control: UnboundedReceiver<IpcControl>,
        ipc_neighbors: Arc<RwLock<Vec<(Uuid, UnboundedSender<IpcControl>)>>>,
        peer: Peer,
        is_master: bool,
    ) -> IpcPeer {
        IpcPeer {
            // phantom: PhantomData,
            stream,
            ipc_command,
            peer,
            ipc_control,
            ipc_neighbors,
            is_master,
        }
    }
    pub(crate) async fn start(mut self: Self) {
        let mut state = Some(Box::new(NewConnection {}) as Box<dyn State>);
        loop {
            if let Some(old_state) = state.take() {
                trace!("Entering: {:?}", &old_state);
                state = old_state.next(&mut self).await;
            } else {
                break;
            }
        }
    }
}

#[async_trait]
trait State: Send + std::fmt::Debug {
    async fn next(self: Box<Self>, _state_machine: &mut IpcPeer) -> Option<Box<dyn State>>;
}

#[derive(Debug)]
struct NewConnection {}

#[async_trait]
impl State for NewConnection {
    async fn next(self: Box<Self>, _state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        // Handle the event for the NewConnection state
        // println!("Handling event in NewConnection state");
        // Transition to the next state (for example, Connected state)
        Some(Box::new(SendPeers {})) // Replace with actual next state
    }
}

#[derive(Debug)]
struct SendPeers {}

#[async_trait]
impl State for SendPeers {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        let peers = state_machine
            .ipc_neighbors
            .read()
            .await
            .iter()
            .map(|(uuid, _tx)| *uuid)
            .filter(|u| *u != state_machine.peer.peer_id)
            .collect();
        match state_machine
            .stream
            .send(IpcMessage::KnownPeers(peers)) //TODO Send actual known peers
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
        select! {
            msg = state_machine.stream.next() => {
                match msg {
                    Some(Ok(ipc_message)) => Some(Box::new(IpcMessageReceived { message: ipc_message})),
                    Some(Err(e)) => Some(Box::new(HandleError { error: e.into()})),
                    None => Some(Box::new(ClosePeer {})),
                }
            }
            control_msg = state_machine.ipc_control.recv() => {
                match control_msg {
                    Some(control_msg) => Some(Box::new(IpcControlReceived { message: control_msg})),
                    None  => Some(Box::new(Shutdown {})),
                }
            }
            peer_msg = state_machine.peer.rx.recv() => {
                match peer_msg {
                    Some(node_msg) => Some(Box::new(NodeMessageReceived {message: node_msg})),
                    None => Some(Box::new(Shutdown {})),
                }
            }
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
            state_machine.peer.peer_id, self.error
        );
        Some(Box::new(ClosePeer {}))
    }
}

#[derive(Debug)]
struct NodeMessageReceived {
    message: NodeMessage,
}

#[async_trait]
impl State for NodeMessageReceived {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        match self.message {
            NodeMessage::BusRider(uuid, vec) => {
                match state_machine
                    .stream
                    .send(IpcMessage::BusRider(uuid, vec))
                    .await
                {
                    Ok(_) => Some(Box::new(WaitForMessages {})),
                    Err(e) => Some(Box::new(HandleError { error: e.into() })),
                }
            }
            // NodeMessage::Shutdown => Some(Box::new(Shutdown {})),
            NodeMessage::Advertise(vec) => {
                _ = state_machine.stream.send(IpcMessage::Advertise(vec)).await;
                Some(Box::new(WaitForMessages {}))
            }
        }
    }
}

#[derive(Debug)]
struct IpcControlReceived {
    message: IpcControl,
}

#[async_trait]
impl State for IpcControlReceived {
    async fn next(self: Box<Self>, _state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        match self.message {
            IpcControl::Shutdown => Some(Box::new(Shutdown {})),
            IpcControl::IAmMaster => b(SendMaster {}),
        }
    }
}

#[derive(Debug)]
struct SendMaster {}

#[async_trait]
impl State for SendMaster {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        match state_machine.stream.send(IpcMessage::IAmMaster).await {
            Ok(_) => Some(Box::new(WaitForMessages {})),
            Err(_) => b(HandleError {
                // TODO Make this a deadlink announcement
                error: "Failed to send IAmMaster".into(),
            }),
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
                _ = state_machine
                    .ipc_command
                    .send(IpcCommand::LearnedPeers(uuids));
            }
            IpcMessage::NeighborRemoved(_uuid) => {}
            IpcMessage::BusRider(uuid, items) => {
                _ = state_machine.peer.handle.send_bytes(uuid, items);
            }
            IpcMessage::CloseConnection => return Some(Box::new(ClosePeer {})),
            IpcMessage::Advertise(ads) => {
                state_machine
                    .peer
                    .handle
                    .add_peer_endpoints(state_machine.peer.peer_id, ads);
            }
            IpcMessage::IAmMaster => {
                state_machine.is_master = true;
                // b(WaitForMessages {})
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
        _ = state_machine.stream.close().await;
        _ = state_machine.ipc_command.send(IpcCommand::PeerClosed(
            state_machine.peer.peer_id,
            state_machine.is_master,
        ));
        state_machine
            .peer
            .handle
            .unregister_peer(state_machine.peer.peer_id);
        state_machine.ipc_control.close();
        state_machine.peer.rx.close();

        None
    }
}

// Received an explicit Shutdown order
// or internal queues are dying and we're just bailing out gracefully
#[derive(Debug)]
struct Shutdown {}

#[async_trait]
impl State for Shutdown {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        _ = state_machine.stream.send(IpcMessage::CloseConnection).await;
        _ = state_machine.stream.close().await;
        None
    }
}
