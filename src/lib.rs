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

// pub use bus_listener::BusListener;
pub use errors::ReceiveError;
// pub use handle::RpcResponse;
// pub use helper::ShutdownWithCtrlC;
pub use helper::spawn;

// pub use messages::RegistrationStatus;
use tokio::sync::watch::Sender;

use tracing::trace;
pub use traits::*;
// mod bus_listener;
mod handle;
pub use handle::Handle;
pub use handle::RequestHelper;
/// Helper functions for working with the AnyBus system (Currently just spawn() )
pub mod helper;
mod messages;
mod receivers;
pub use receivers::Receiver;
pub use receivers::RpcReceiver;
pub use receivers::rpc_receiver::RpcRequest;
// pub use routing::Address;
// pub use routing::EndpointId;
// mod route_table;
mod routing;
mod traits;
#[cfg(feature = "ipc")]
use peers::IpcManager;

#[cfg(feature = "net")]
/// Network peer discovery and messaging
pub mod peers;

use tokio::{
    // stream:: StreamExt,
    sync::watch::{self},
};

// use std::sync::mpsc::{Receiver, Sender};

use uuid::Uuid;

// use crate::router::Router;

mod common;
pub mod errors;

pub use anybus_macro::bus_uuid;

use crate::messages::BusControlMsg;

use crate::routing::router::Router;

impl AnyBus {}

// type RoutesWatchRx = watch::Receiver<Routes>;

/// The main entry point into the AnyBus system.
#[allow(dead_code)]
#[derive(Debug)]
pub struct AnyBus {
    bc_tx: Sender<BusControlMsg>,
    id: Uuid,
    handle: Handle,
    options: AnyBusBuilder,
    bc_rx: watch::Receiver<BusControlMsg>,
    // router: Router,
}

impl AnyBus {
    /// This starts and runs the AnyBus.  The returned [BusControlHandle] is used to shutdown the system.  The [Handle] is
    /// used for normal interaction with the system
    ///
    // #[allow(clippy::new_ret_no_self)]
    //
    pub fn new() -> AnyBus {
        Self::build(AnyBusBuilder::default())
    }

    fn build(options: AnyBusBuilder) -> AnyBus {
        trace!("Starting AnyBus");
        // dbg!(&options);
        let id = Uuid::now_v7();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let (bc_tx, bc_rx) = watch::channel(BusControlMsg::Run);
        let mut router = Router::new(id, rx, bc_rx.clone());
        let route_watch_rx = router.get_watcher();

        let handle = Handle { tx, route_watch_rx };
        let _router_task = spawn(router.start());

        let msg_bus = AnyBus {
            bc_tx,
            id,
            handle: handle.clone(),
            options,
            bc_rx,
            // router,
        };
        msg_bus
    }

    /// Starts the AnyBus system.  This will start any configured listeners (WebSocket, IPC, etc) and begin processing messages.
    pub fn run(&mut self) {
        #[cfg(feature = "ws")]
        if self.options.ws_listener_options.is_some() || !self.options.ws_remote_options.is_empty()
        {
            trace!("Starting WebSocket Manager");

            let ws_listener = crate::peers::WebsocketManager::new(
                self.id,
                self.handle.clone(),
                self.bc_rx.clone(),
                self.options.ws_listener_options.clone(),
                self.options.ws_remote_options.clone(),
            );

            spawn(ws_listener.start());
        }

        #[cfg(feature = "ipc")]
        if self.options.enable_ipc {
            let manager = IpcManager::new(
                "anybus.ipc".into(),
                self.handle.clone(),
                self.bc_rx.clone(),
                self.id,
            );
            spawn(manager.start());
        };

        if self.options.enable_ctrlc_shutdown {
            self.shutdown_with_ctrlc();
        }
    }

    /// Passes the shutdown command to the AnyBus system and all local listeners.  Immediately withdraws all advertisements from the network.
    ///
    /// If the program is killed by other means it can take up to 40 seconds for other systems to forget the advertisements from this AnyBus
    ///
    pub fn shutdown(&mut self) {
        self.bc_tx
            .send(BusControlMsg::Shutdown)
            .map_err(|e| trace!("Send shutdown error {:?}", e))
            .ok();
    }

    /// Returns a Handle for clients to interact with the AnyBus system.
    /// Expected to be cloned and sent to other parts of your program
    ///
    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    #[cfg(feature = "tokio")]
    /// Convenience function to spawn a task that will listen for Ctrl-C from the terminal and trigger a shutdown of the AnyBus system
    pub fn shutdown_with_ctrlc(&self) {
        _ = spawn(helper::watch_ctrlc(self.handle.clone()));
    }
}

impl Default for AnyBus {
    fn default() -> Self {
        Self::new()
    }
}

// pub struct ShutdownAnyBusHandle {
//     bc_tx: Sender<BusControlMsg>,
// }

/// AnyBusBuilder is a builder pattern for constructing an AnyBus instance with options
#[derive(Debug, Default, Clone)]
pub struct AnyBusBuilder {
    enable_ctrlc_shutdown: bool,
    #[cfg(feature = "ipc")]
    enable_ipc: bool,
    #[cfg(feature = "ws")]
    ws_listener_options: Option<crate::peers::WsListenerOptions>,
    #[cfg(feature = "ws")]
    ws_remote_options: Vec<crate::peers::WsRemoteOptions>,
}
impl AnyBusBuilder {
    /// Creates a new AnyBusBuilder with default options
    pub fn new() -> Self {
        Self {
            enable_ctrlc_shutdown: false,
            #[cfg(feature = "ipc")]
            enable_ipc: false,
            #[cfg(feature = "ws")]
            ws_listener_options: None,
            #[cfg(feature = "ws")]
            ws_remote_options: Vec::new(),
        }
    }

    /// Enables or disables the Ctrl-C shutdown feature.  Default is disabled.
    ///
    /// If enabled, when the user presses Ctrl-C in the terminal, the AnyBus system will be shutdown cleanly
    ///
    pub fn enable_ctrlc_shutdown(mut self, enable: bool) -> Self {
        self.enable_ctrlc_shutdown = enable;
        self
    }

    /// Enables or disables the IPC peer discovery and messaging feature.  Default is enabled.
    ///
    /// If disabled, this AnyBus instance will not be able to discover or communicate with other local AnyBus instances using IPC
    ///
    #[cfg(feature = "ipc")]
    pub fn enable_ipc(mut self, enable: bool) -> Self {
        self.enable_ipc = enable;
        self
    }

    /// Sets the WebSocket listener options.  If set, a WebSocket listener will be started with these options.
    ///
    /// If not set, no WebSocket listener will be started.
    ///
    #[cfg(feature = "ws")]
    pub fn ws_listener(mut self, options: crate::peers::WsListenerOptions) -> Self {
        self.ws_listener_options = Some(options);
        self
    }

    /// Adds a remote WebSocket peer to connect to.  Can be called multiple times to add multiple remote peers.
    pub fn ws_remote(mut self, options: crate::peers::WsRemoteOptions) -> Self {
        self.ws_remote_options.push(options);
        self
    }

    /// Builds and starts the AnyBus instance with the specified options.  Returns the AnyBus instance.
    ///
    pub fn build(self) -> AnyBus {
        let anybus = AnyBus::build(self.clone());

        anybus
    }
}
