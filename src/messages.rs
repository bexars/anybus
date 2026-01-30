#[cfg(feature = "remote")]
use std::collections::HashSet;

#[cfg(feature = "remote")]
use crate::routing::{Advertisement, NodeId, PeerEntry, WirePacket};
use crate::routing::{EndpointId, Packet, Route};

#[derive(Debug)]
pub(crate) enum BrokerMsg {
    RegisterRoute(EndpointId, Route),
    DeadLink(EndpointId),
    #[cfg(feature = "remote")]
    RegisterPeer(NodeId, PeerEntry),
    #[cfg(feature = "remote")]
    UnRegisterPeer(NodeId),
    #[cfg(feature = "remote")]
    AddPeerEndpoints(NodeId, HashSet<Advertisement>),
    #[cfg(feature = "remote")]
    RemovePeerEndpoints(NodeId, HashSet<Advertisement>),
    Shutdown,
}

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

#[cfg(feature = "remote")]
/// Messages going to the Peer entity that is owned by the connection to a remote peer
#[derive(Debug)]
pub(crate) enum NodeMessage {
    WirePacket(WirePacket),
    Close,
    Advertise(HashSet<Advertisement>),
    Withdraw(HashSet<Advertisement>),
    // BusRider(EndpointId, Vec<u8>),
}
