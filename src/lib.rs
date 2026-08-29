#![warn(missing_docs)]
//! A crate for easy to configure local messaging between services over the local network
//!
//! This crate makes extensive use of [Uuid] for addressing other services in the network.
//!
//! There are three types of messages in the network (Unicast, AnyCast, MultiCast) and they are determined by how the address
//! is registered with the system
//! * Unicast - [Handle::register_unicast()]
//! * AnyCast - [Handle::register_anycast()]
//! * MultiCast - [Handle::register_multicast()]

mod anybus;
#[cfg(feature = "remote")]
mod codec;
mod common;
pub mod errors;
mod handle;
pub mod helper;
pub(crate) mod localrpc;
mod messages;
#[cfg(feature = "remote")]
/// Network peer discovery and messaging
pub mod peers;
mod receivers;
mod routing;
mod services;
mod traits;

pub use crate::routing::EndpointId;
pub use anybus::AnyBus;
pub use anybus::AnyBusConfig;
pub use anybus::builder::AnyBusBuilder;
pub use anybus_macro::anybus_rpc;
pub use anybus_macro::bus_uuid;
pub use errors::ReceiveError;
pub use handle::Handle;
pub use handle::RequestHelper;
pub use helper::spawn;
pub use messages::AnyBusStatusMsg;
pub use receivers::Receiver;
pub use receivers::RpcReceiver;
pub use receivers::rpc_receiver::RpcRequest;
#[cfg(feature = "serde")]
pub use serde::{Deserialize, Serialize};
pub use traits::*;
pub use uuid::Uuid;

pub use routing::Realm;

/// Common Anybus components
pub mod prelude {
    pub use crate::AnyBus;
    pub use crate::AnyBusBuilder;
    pub use crate::AnyBusConfig;
    pub use crate::AnyBusStatusMsg;
    pub use crate::BusDepot;
    pub use crate::BusRider;
    pub use crate::BusRiderRpc;
    pub use crate::BusRiderWithUuid;
    pub use crate::BusStop;
    pub use crate::EndpointId;
    pub use crate::Handle;
    pub use crate::Realm;
    pub use crate::Uuid;
    pub use crate::anybus_rpc;
    pub use crate::bus_uuid;
    #[cfg(feature = "serde")]
    pub use crate::{Deserialize, Serialize};
    #[cfg(feature = "dioxus")]
    pub use dioxus::prelude::*;
    #[cfg(feature = "tokio")]
    pub use tokio;
}
