use serde::Serialize;
use tokio::sync::{
    mpsc::UnboundedSender,
    oneshot::{self},
};
use uuid::Uuid;

use crate::{
    route_table::{Advertisement, UnicastType},
    traits::BusRider,
};

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum BrokerMsg {
    RegisterAnycast(Uuid, UnboundedSender<ClientMessage>),
    RegisterUnicast(Uuid, UnboundedSender<ClientMessage>, UnicastType),
    Subscribe(Uuid, UnboundedSender<ClientMessage>),
    DeadLink(Uuid),
    RegisterPeer(Uuid, UnboundedSender<NodeMessage>),
    UnRegisterPeer(Uuid),
    AddPeerEndpoints(Uuid, Vec<Advertisement>),
}
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum ClientMessage {
    Message(Uuid, Box<dyn BusRider>),
    Bytes(Uuid, Vec<u8>),
    Rpc {
        to: Uuid,
        reply_to: oneshot::Sender<Box<dyn BusRider>>,
        msg: Box<dyn BusRider>,
    },

    //TODO Make subset of this error
    FailedRegistration(Uuid, String),
    SuccessfulRegistration(Uuid),
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
    BusRider(Uuid, Vec<u8>),
    Shutdown,
    Advertise(Vec<Advertisement>),
}
