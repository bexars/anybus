//!  Collection of [Error]s returned by various subsystems
use crate::tokio;

use thiserror::Error;

use crate::{
    messages::ClientMessage,
    routing::{Packet, Payload},
};

/// Errors returned by [BusListener::recv()](crate::BusListener::recv())
#[derive(Error, Debug)]

pub enum ReceiveError {
    /// There are no senders left which means the [AnyBus](crate::AnyBus) has force closed this connection, most likely during shutdown
    #[error("Connection closed, possible shutdown")]
    ConnectionClosed,
    /// Error when registering a Uuid that is already exclusively registered.  i.e. register_anycast() on an existing Unicast, or Multicast Uuid
    #[error("Unable to register, possibly already registered as subscribe address")]
    RegistrationFailed(String),
    /// The system is shutting down now
    #[error("Unable to deserialize message payload")]
    DeserializationError(Payload),
    /// The system is shutting down now
    #[error("System shutdown requested")]
    Shutdown,
    /// Error in the receive calls
    #[error("RPC message received without a reply_to address")]
    RpcNoReplyTo,
}

impl From<futures::channel::mpsc::SendError> for ReceiveError {
    fn from(_: futures::channel::mpsc::SendError) -> Self {
        ReceiveError::ConnectionClosed
    }
}

impl<E> From<futures::channel::mpsc::TrySendError<E>> for ReceiveError {
    fn from(_: futures::channel::mpsc::TrySendError<E>) -> Self {
        ReceiveError::ConnectionClosed
    }
}

impl<E> From<tokio::sync::mpsc::error::SendError<E>> for ReceiveError {
    fn from(_value: tokio::sync::mpsc::error::SendError<E>) -> Self {
        ReceiveError::ConnectionClosed
    }
}

/// WIP to revamp error into Exn
// pub struct AnyBusError {}

/// Errors from various parts of AnyBus
#[derive(Error, Debug)]
pub enum AnyBusHandleError {
    /// Send failed for unknown reason.
    #[error("Unable to send: {0}")]
    // SendError(Box<dyn BusRider>),
    SendError(#[source] SendError),
    /// The destination [Uuid](uuid::Uuid) is unknown
    #[error("Route not found for that UUID")]
    NoRoute,

    /// Not implemented yet
    #[error("Unable to subscribe, possibly already subscribed as register address")]
    SubscriptionFailed,
    // /// There are no senders left which means the [AnyBus] has force closed this connection, most likely during shutdown
    // #[error("Connection closed, possible shutdown")]
    // ConnectionClosed,
    /// The system is shutting down now
    #[error("System shutdown requested")]
    Shutdown,
    /// Error in the receive calls
    #[error("Error in the RPC response: {0}")]
    ReceiveError(#[source] ReceiveError),
}

#[derive(Error, Debug)]
/// SendError returned by [AnyBusHandle::send()]
pub enum SendError {
    /// No route found in forwarding table
    #[error("No Route to Endpoint")]
    NoRoute(Option<Payload>),

    /// Destination Queue is full
    #[error("Queue full")]
    Full(Option<Payload>),
    /// The destination queue is closed
    #[error("Queue closed")]
    Closed(Option<Payload>),
}

impl SendError {
    /// The message that was not sent, when this error kept it.
    pub fn payload(self) -> Option<Payload> {
        match self {
            SendError::NoRoute(payload) | SendError::Full(payload) | SendError::Closed(payload) => {
                payload
            }
        }
    }
}

impl From<tokio::sync::mpsc::error::TrySendError<ClientMessage>> for SendError {
    fn from(value: tokio::sync::mpsc::error::TrySendError<ClientMessage>) -> Self {
        use tokio::sync::mpsc::error::TrySendError;

        match value {
            TrySendError::Full(cm) => {
                if let ClientMessage::Message(Packet { payload, .. }) = cm {
                    SendError::Full(Some(payload))
                } else {
                    SendError::Full(None)
                }
            }
            TrySendError::Closed(cm) => {
                if let ClientMessage::Message(Packet { payload, .. }) = cm {
                    SendError::Closed(Some(payload))
                } else {
                    SendError::Closed(None)
                }
            }
        }
    }
}
