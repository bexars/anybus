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
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::{
    messages::NodeMessage,
    peers::ipc::{IpcCommand, IpcControl, IpcMessage, IpcPeerStream, Peer},
};

fn b<T: State + 'static>(thing: T) -> Option<Box<dyn State>> {
    Some(Box::new(thing))
}

pub(crate) struct IpcPeer {
    // phantom: PhantomData<T>,
    stream: IpcPeerStream,
    ipc_command: UnboundedSender<IpcCommand>,
    ipc_control: UnboundedReceiver<IpcControl>,
    ipc_neighbors: Arc<RwLock<Vec<(Uuid, UnboundedSender<IpcControl>)>>>,
    peer: Peer,
    is_master: bool,
}

impl IpcPeer {
    pub(crate) fn new(
        stream: IpcPeerStream,
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
    pub(crate) async fn start(mut self) {
        let mut state = Some(Box::new(NewConnection {}) as Box<dyn State>);
        while let Some(old_state) = state.take() {
            debug!("Entering: {:?}", &old_state);
            state = old_state.next(&mut self).await;
        }
    }
}

#[async_trait]
trait State: Send + std::fmt::Debug {
    async fn next(self: Box<Self>, _state_machine: &mut IpcPeer) -> Option<Box<dyn State>>;
}

#[derive(Debug)]
struct NewConnection {}

// TODO Probably could elminate this state altogether

#[async_trait]
impl State for NewConnection {
    async fn next(self: Box<Self>, state_machine: &mut IpcPeer) -> Option<Box<dyn State>> {
        info!("New connection to: {}", state_machine.peer.peer_id);
        // We're handed a freshly created 'stream' that has already exchanged Hello's and need to drive it forward
        Some(Box::new(SendPeers {}))
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
                    None  => Some(Box::new(Shutdown {})), // something important crashed, bail out
                }
            }
            peer_msg = state_machine.peer.rx.recv() => {
                match peer_msg {
                    Some(node_msg) => Some(Box::new(NodeMessageReceived {message: node_msg})),
                    None => Some(Box::new(Shutdown {})),  // something important crashed, bail out
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
            NodeMessage::WirePacket(packet) => {
                match state_machine.stream.send(IpcMessage::Packet(packet)).await {
                    Ok(_) => Some(Box::new(WaitForMessages {})),
                    Err(e) => Some(Box::new(HandleError { error: e.into() })),
                }
            }
            NodeMessage::Advertise(vec) => {
                match state_machine.stream.send(IpcMessage::Advertise(vec)).await {
                    Ok(()) => Some(Box::new(WaitForMessages {})),
                    Err(e) => b(HandleError { error: e.into() }),
                }
            }
            NodeMessage::Withdraw(advertisements) => {
                match state_machine
                    .stream
                    .send(IpcMessage::Withdraw(advertisements))
                    .await
                {
                    Ok(()) => Some(Box::new(WaitForMessages {})),
                    Err(e) => b(HandleError { error: e.into() }),
                }
            }
            NodeMessage::Close => {
                match state_machine.stream.send(IpcMessage::CloseConnection).await {
                    Ok(()) => Some(Box::new(ClosePeer {})),
                    Err(e) => b(HandleError { error: e.into() }),
                }
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
            IpcMessage::Packet(wire_packet) => {
                state_machine
                    .peer
                    .handle
                    .send_packet(wire_packet, state_machine.peer.peer_id);
            }
            IpcMessage::BusRider(endpoint_id, items) => {
                _ = state_machine
                    .peer
                    .handle
                    .send_to_address(endpoint_id, items);
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
            }
            IpcMessage::Withdraw(uuids) => {
                state_machine
                    .peer
                    .handle
                    .remove_peer_endpoints(state_machine.peer.our_id, uuids);
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
        _ = state_machine.stream.close().await;

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
