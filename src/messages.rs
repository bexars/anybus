#[cfg(any(feature = "net", feature = "ipc"))]
use std::collections::HashSet;

use tokio::sync::mpsc::UnboundedSender;

#[cfg(any(feature = "net", feature = "ipc"))]
use crate::routing::{Advertisement, NodeId, WirePacket};
use crate::routing::{EndpointId, Packet, Route};

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum BrokerMsg {
    // RegisterAnycast(Uuid, UnboundedSender<ClientMessage>),
    // RegisterUnicast(Uuid, UnboundedSender<ClientMessage>, UnicastType),
    RegisterRoute(EndpointId, Route),
    Subscribe(EndpointId, UnboundedSender<ClientMessage>),
    DeadLink(EndpointId),
    #[cfg(any(feature = "net", feature = "ipc"))]
    RegisterPeer(NodeId, UnboundedSender<NodeMessage>),
    #[cfg(any(feature = "net", feature = "ipc"))]
    UnRegisterPeer(NodeId),
    #[cfg(any(feature = "net", feature = "ipc"))]
    AddPeerEndpoints(NodeId, HashSet<Advertisement>),
    #[cfg(any(feature = "net", feature = "ipc"))]
    RemovePeerEndpoints(NodeId, HashSet<Advertisement>),
    Shutdown,
}
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum ClientMessage {
    // Message(Uuid, Box<dyn BusRider>),
    // Bytes(Uuid, Vec<u8>),
    // Rpc {
    //     to: Uuid,
    //     reply_to: oneshot::Sender<Box<dyn BusRider>>,
    //     msg: Box<dyn BusRider>,
    // },
    Message(Packet),
    //TODO Make subset of this error
    FailedRegistration(EndpointId, String),
    SuccessfulRegistration(EndpointId),
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum BusControlMsg {
    Run,
    Shutdown,
}

/// Shows the status of a registration attempt.
#[derive(Debug, Default, Clone)]
pub enum RegistrationStatus {
    /// Initial state
    #[default]
    Pending,
    /// Registration successful
    Registered,
    /// Registration failed
    Failed(String),
}

#[cfg(any(feature = "net", feature = "ipc"))]
/// Messages going to the Peer entity that is owned by the connection to a remote peer
#[derive(Debug)]
pub(crate) enum NodeMessage {
    WirePacket(WirePacket),
    Close,
    Advertise(HashSet<Advertisement>),
    #[allow(dead_code)]
    Withdraw(HashSet<Advertisement>),
    // BusRider(EndpointId, Vec<u8>),
}
