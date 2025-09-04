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

pub use bus_listener::BusListener;
pub use errors::ReceiveError;
// pub use handle::RpcResponse;
// pub use helper::ShutdownWithCtrlC;
pub use helper::spawn;

pub use messages::RegistrationStatus;
use tokio::sync::watch::Sender;

use tracing::trace;
pub use traits::*;
mod bus_listener;
mod handle;
mod helper;
mod messages;
// mod route_table;
mod routing;
mod traits;
pub use handle::Handle;
use peers::IpcManager;

#[cfg(any(feature = "ipc", feature = "net"))]
mod peers;

use tokio::{
    // stream:: StreamExt,
    sync::watch::{self},
};

// use std::sync::mpsc::{Receiver, Sender};

use uuid::Uuid;

// use crate::router::Router;

mod common;
pub mod errors;

pub use msgbus_macro::bus_uuid;

use crate::messages::BusControlMsg;

use crate::routing::router::Router;

impl MsgBus {}

// type RoutesWatchRx = watch::Receiver<Routes>;

/// The main entry point into the MsgBus system.
#[allow(dead_code)]
pub struct MsgBus {
    bc_tx: Sender<BusControlMsg>,
    id: Uuid,
    handle: Handle,
}

impl MsgBus {
    /// This starts and runs the MsgBus.  The returned [BusControlHandle] is used to shutdown the system.  The [Handle] is
    /// used for normal interaction with the system
    ///
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> MsgBus {
        let id = Uuid::now_v7();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let (bc_tx, bc_rx) = watch::channel(BusControlMsg::Run);
        let router = Router::new(id, rx, bc_rx.clone());
        let route_watch_rx = router.get_watcher();

        let handle = Handle { tx, route_watch_rx };

        spawn(router.start());

        let msg_bus = MsgBus {
            bc_tx,
            id,
            handle: handle.clone(),
        };

        #[cfg(feature = "ipc")]
        {
            let manager = IpcManager::new("anybus.ipc".into(), handle, bc_rx, id);
            spawn(manager.start());
        };

        msg_bus
    }

    /// Passes the shutdown command to the MsgBus system and all local listeners.  Immediately withdraws all advertisements from the network.
    ///
    /// If the program is killed by other means it can take up to 40 seconds for other systems to forget the advertisements from this MsgBus
    ///
    pub fn shutdown(&mut self) {
        self.bc_tx
            .send(BusControlMsg::Shutdown)
            .map_err(|e| trace!("Send shutdown error {:?}", e))
            .ok();
    }

    /// Returns a Handle for clients to interact with the MsgBus system.
    /// Expected to be cloned and sent to other parts of your program
    ///
    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    /// Convenience function to spawn a task that will listen for Ctrl-C from the terminal and trigger a shutdown of the MsgBus system
    pub fn shutdown_with_ctrlc(&self) {
        _ = spawn(helper::watch_ctrlc(self.handle.clone()));
    }
}

impl Default for MsgBus {
    fn default() -> Self {
        Self::new()
    }
}

// pub struct ShutdownMsgBusHandle {
//     bc_tx: Sender<BusControlMsg>,
// }
