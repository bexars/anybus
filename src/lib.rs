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
pub use traits::*;

pub use routing::Realm;
