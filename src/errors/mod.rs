//!  Collection of [Error]s returned by various subsystems

use thiserror::Error;

use crate::BusRider;

/// Errors returned by [BusListener::recv()](crate::BusListener::recv())
#[derive(Error, Debug)]

pub enum ReceiveError {
    /// There are no senders left which means the [MsgBus](crate::MsgBus) has force closed this connection, most likely during shutdown
    #[error("Connection closed, possible shutdown")]
    ConnectionClosed,
    /// Error when registering a Uuid that is already exclusively registered.  i.e. register_anycast() on an existing Unicast, or Multicast Uuid
    #[error("Unable to register, possibly already registered as subscribe address")]
    RegistrationFailed,
    /// The system is shutting down now
    #[error("System shutdown requested")]
    Shutdown,
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

/// Errors from various parts of MsgBus
#[derive(Error, Debug)]
pub enum MsgBusHandleError {
    /// Send failed for unknown reason.  Original message is returned in the error
    #[error("Unable to send.  The passed Message is returned within this error")]
    SendError(Box<dyn BusRider>),
    /// The destination [Uuid](uuid::Uuid) is unknown
    #[error("Route not found for that UUID")]
    NoRoute,

    /// Not implemented yet
    #[error("Unable to subscribe, possibly already subscribed as register address")]
    SubscriptionFailed,
    // /// There are no senders left which means the [MsgBus] has force closed this connection, most likely during shutdown
    // #[error("Connection closed, possible shutdown")]
    // ConnectionClosed,
    /// The system is shutting down now
    #[error("System shutdown requested")]
    Shutdown,
}
