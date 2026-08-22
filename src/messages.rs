#[cfg(feature = "remote")]
use std::collections::HashSet;

#[cfg(feature = "remote")]
use crate::routing::{Advertisement, NodeId, PeerEntry, WirePacket};
use crate::{
    BusRiderWithUuid,
    routing::{EndpointId, Packet, Route},
};

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

/// Status returned by AnybusStatusWatcher
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AnyBusStatusMsg {
    /// The bus is shutting down.  Error, ctrl-c or commanded by AnyBus::shutdown()
    ShuttingDown,
    /// Not implemented yet, will be sent when the bus senses a resume from suspension.  Example: Laptop opening and resuming
    Resuming,
    /// When a network change has been detected
    #[cfg(feature = "remote")]
    NetworkChanged,
}

impl BusRiderWithUuid for AnyBusStatusMsg {
    const ANYBUS_UUID: uuid::Uuid = uuid::Uuid::from_u128(0xec785db99c4b46b385f82a107268d674);
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
