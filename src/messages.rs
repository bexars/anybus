use std::collections::HashSet;

use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::routing::{Advertisement, EndpointId, NodeId, Packet, Route, WirePacket};

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum BrokerMsg {
    // RegisterAnycast(Uuid, UnboundedSender<ClientMessage>),
    // RegisterUnicast(Uuid, UnboundedSender<ClientMessage>, UnicastType),
    RegisterRoute(EndpointId, Route),
    Subscribe(EndpointId, UnboundedSender<ClientMessage>),
    DeadLink(EndpointId),
    RegisterPeer(NodeId, UnboundedSender<NodeMessage>),
    UnRegisterPeer(NodeId),
    AddPeerEndpoints(NodeId, HashSet<Advertisement>),
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
