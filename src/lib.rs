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
// use tokio_with_wasm::alias as tokio;

// pub use bus_listener::BusListener;
pub use errors::ReceiveError;
// pub use handle::RpcResponse;
// pub use helper::ShutdownWithCtrlC;
pub use helper::spawn;

pub use traits::*;
// mod bus_listener;
mod handle;
pub use handle::Handle;
pub use handle::RequestHelper;
/// Helper functions for working with the AnyBus system (Currently just spawn() )
pub mod helper;
mod messages;
pub use messages::AnyBusStatusMsg;
mod receivers;
pub use receivers::Receiver;
pub use receivers::RpcReceiver;
pub use receivers::rpc_receiver::RpcRequest;
// pub use routing::Address;
// pub use routing::EndpointId;
// mod route_table;
mod routing;
pub use routing::Realm;
mod anybus;
pub use anybus::AnyBus;
pub use anybus::builder::AnyBusBuilder;
#[cfg(feature = "remote")]
mod codec;
pub(crate) mod localrpc;
mod services;
mod traits;

#[cfg(feature = "remote")]
/// Network peer discovery and messaging
pub mod peers;

// use std::sync::mpsc::{Receiver, Sender};

// use crate::router::Router;

mod common;
pub mod errors;

pub use anybus_macro::anybus_rpc;
pub use anybus_macro::bus_uuid;

// use crate::anybus::AnyBus;
pub use crate::routing::EndpointId;

// pub struct ShutdownAnyBusHandle {
//     bc_tx: Sender<BusControlMsg>,
// }
