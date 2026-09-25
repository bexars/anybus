use crate::tokio::sync::{mpsc::Sender, oneshot};

use crate::routing::RegistrationRequest;
#[cfg(feature = "remote")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "remote")]
use crate::routing::{ConnectionId, Cost, Lsa, LsaKey, NodeId, PeerEntry, RealmList, WirePacket};
use crate::{
    BusRiderWithUuid,
    routing::{EndpointId, Packet},
};

#[derive(Debug)]
pub(crate) enum RouterMsg {
    // RegisterEndpoint(EndpointId, EndpointInfo, Sender<ClientMessage>),
    RegisterEndpoint(RegistrationRequest),
    DeadLink(EndpointId),
    #[cfg(feature = "remote")]
    RegisterPeer(NodeId, ConnectionId, PeerEntry, Cost, RealmList),
    #[cfg(feature = "remote")]
    UnRegisterPeer(ConnectionId),
    // #[cfg(feature = "remote")]
    // AddPeerEndpoints(ConnectionId, HashSet<Advertisement>),
    // #[cfg(feature = "remote")]
    // RemovePeerEndpoints(ConnectionId, HashSet<Advertisement>),
    #[cfg(feature = "remote")]
    LsaInbound {
        from: ConnectionId,
        lsa: Lsa,
    },
    #[cfg(feature = "remote")]
    LsaAckInbound {
        from: ConnectionId,
        key: LsaKey,
        seq: u64,
    },
    SetAnycastCost {
        endpoint_id: EndpointId,
        sender: Sender<ClientMessage>,
        cost: u16,
        reply: oneshot::Sender<Result<(), SetAnycastCostError>>,
    },
    Shutdown,
}

#[derive(Debug)]
pub(crate) enum SetAnycastCostError {
    NotAnycast,
    NotRegistered,
}

#[derive(Debug)]
pub(crate) enum SetAnycastCostOutcome {
    Unchanged,
    Changed {
        previous: crate::routing::Cost,
        new: crate::routing::Cost,
        advertise: Option<crate::routing::Cost>,
    },
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
    /// Process or tab is coming back from suspend / background freeze.
    /// Debounced: duplicate Resuming within a few seconds is dropped.
    Resuming,
    /// Suspend is imminent (OS power event) or the page became hidden (WASM).
    Suspending,
    /// When a network change has been detected
    #[cfg(feature = "remote")]
    NetworkChanged,
}

impl BusRiderWithUuid for AnyBusStatusMsg {
    const ANYBUS_UUID: uuid::Uuid = uuid::Uuid::from_u128(0xec785db99c4b46b385f82a107268d674);
}

#[cfg(feature = "remote")]
/// Messages going to the Peer entity that is owned by the connection to a remote peer
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum NodeMessage {
    WirePacket(WirePacket),
    // Advertise(HashSet<Advertisement>),
    // Withdraw(HashSet<Advertisement>),
    Lsa(Lsa),
    LsaAck { key: LsaKey, seq: u64 },
}
