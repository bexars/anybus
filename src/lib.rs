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
pub use handle::RpcResponse;
pub use messages::RegistrationStatus;
pub use traits::*;
mod bus_listener;
mod handle;
mod messages;
mod route_table;
mod traits;
pub use handle::Handle;

use tracing::{error, info};

use tokio::{
    // stream:: StreamExt,
    select,
    sync::{
        mpsc::UnboundedReceiver,
        watch::{self, Receiver},
    },
};

// use std::sync::mpsc::{Receiver, Sender};

use uuid::Uuid;

// use crate::router::Router;

mod common;
pub mod errors;

pub use msgbus_macro::bus_uuid;

use crate::messages::BusControlMsg;
use crate::messages::{BrokerMsg, ClientMessage};
use crate::route_table::AnycastDest;
use crate::route_table::DestinationType;
use crate::route_table::RouteTableController;
use crate::route_table::UnicastDest;

/// Reference to a foreign instance of [MsgBus]
/// * Could be in the same process, just a different MsgBus instance
#[derive(Debug, Clone)]
#[allow(dead_code)] //TODO
pub(crate) struct Node {
    id: Uuid,
    //TODO store how to get to this node
}

/// Handle for programatically shutting down the system.  Can be wrapped with [helper::ShutdownWithCtrlC] to catch user termination
/// gracefully
pub struct BusControlHandle {
    // #[cfg(feature = "tokio")]
    pub(crate) tx: watch::Sender<BusControlMsg>,
    pub(crate) handle: Handle,
}

impl BusControlHandle {
    /// Passes the shutdown command to the MsgBus system and all local listeners.  Immediately withdraws all advertisements from the network.
    ///
    /// If the program is killed by other means it can take up to 40 seconds for other systems to forget the advertisements from this MsgBus
    ///
    pub fn shutdown(&mut self) {
        self.tx.send(BusControlMsg::Shutdown).unwrap_or_default();
    }

    /// Returns a Handle for clients to interact with the MsgBus system.
    /// Expected to be cloned and sent to other parts of your program
    ///
    pub fn handle(&self) -> &Handle {
        &self.handle
    }
}

// type RoutesWatchRx = watch::Receiver<Routes>;

/// The main entry point into the MsgBus system.
pub struct MsgBus {}

impl MsgBus {
    /// This starts and runs the MsgBus.  The returned [BusControlHandle] is used to shutdown the system.  The [Handle] is
    /// used for normal interaction with the system
    ///
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> BusControlHandle {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let (bc_tx, bc_rx) = watch::channel(BusControlMsg::Run);

        let route_table_controller = RouteTableController::new();
        let rts_rx = route_table_controller.get_watcher();

        let handle = Handle { tx, rts_rx };

        let control_handle = BusControlHandle {
            tx: bc_tx,
            handle: handle.clone(),
        };

        let fut = Self::run(route_table_controller, rx, bc_rx);

        #[cfg(feature = "dioxus")]
        dioxus::prelude::spawn(fut);
        #[cfg(feature = "tokio")]
        tokio::spawn(fut);

        control_handle
    }

    async fn run(
        mut routes: RouteTableController,
        mut rx: UnboundedReceiver<BrokerMsg>,
        mut bc_rx: Receiver<BusControlMsg>,
    ) {
        let mut should_shutdown = false;

        loop {
            if *bc_rx.borrow() == BusControlMsg::Shutdown || should_shutdown {
                routes.shutdown_routing();
                break;
            }
            select! {
                msg = rx.recv() => should_shutdown = Self::process_message(msg, &mut routes),

                change_res = bc_rx.changed() => {
                    info!("bc_rx.changed() {:?}", change_res);
                    match change_res {
                        Ok(_) => {}
                        Err(e) => {
                            error!("Error receiving bus control message: {:?}", e);
                            should_shutdown = true;
                        }
                    }
                    match *bc_rx.borrow_and_update() {
                        BusControlMsg::Run => {},  // should never receive this but it's a non-issue
                        BusControlMsg::Shutdown => {
                            should_shutdown = true;
                        }// TOOD log this
                    }
                },
            };
        }
    }

    fn process_message(
        msg: Option<BrokerMsg>,
        routes: &mut RouteTableController,
        // rts_tx: &watch::Sender<HashMap<Uuid, Nexthop>>,
    ) -> bool {
        // #[cfg(feature = "dioxus")]
        // dioxus::logger::tracing::info!("Processing msg: {:?}", msg);

        let Some(msg) = msg else { return true };
        match msg {
            BrokerMsg::Subscribe(_uuid, _tx) => {
                todo!()
            }
            BrokerMsg::RegisterUnicast(uuid, unbounded_sender, unicast_type) => {
                let dest = UnicastDest {
                    unicast_type,
                    dest_type: DestinationType::Local(unbounded_sender.clone()),
                };
                let msg = match routes.add_unicast(uuid, dest) {
                    Ok(_) => ClientMessage::SuccessfulRegistration(uuid),
                    Err(_) => {
                        ClientMessage::FailedRegistration(uuid, "Duplicate registration".into())
                    }
                };
                let _todo = unbounded_sender.send(msg); // TODO Make a deadlink update
            }

            BrokerMsg::RegisterAnycast(uuid, tx) => {
                // TODO Make the Routes have Cow elements for ease of cloning
                // let mut new_map = (*map).clone();
                let dest = AnycastDest {
                    cost: 0,
                    dest_type: DestinationType::Local(tx.clone()),
                };
                let msg = match routes.add_anycast(uuid, dest) {
                    Ok(_) => ClientMessage::SuccessfulRegistration(uuid),
                    Err(_) => {
                        ClientMessage::FailedRegistration(uuid, "Duplicate registration".into())
                    }
                };

                let _ = tx.send(msg);
            }
            BrokerMsg::DeadLink(uuid) => {
                // let mut new_map = (*map).clone();
                let _ = routes.dead_link(uuid);
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    // use super::*;

    #[test]
    fn it_works() {}
}
