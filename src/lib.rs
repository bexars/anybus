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

// use dioxus::Ok;
// use common::random_uuid;
pub use errors::ReceiveError;
mod bus_listener;
pub use bus_listener::BusListener;
pub use handle::RpcResponse;
mod handle;
mod messages;
mod route_table;
mod traits;
pub use handle::Handle;

use sorted_vec::partial::SortedVec;
use std::collections::HashMap;
use tracing::{error, info};

// use tokio::sync::oneshot::Receiver as OneShotReceiver;

// #[cfg(feature = "dioxus")]
// use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};

// #[cfg(feature = "tokio")]
use tokio::{
    // stream:: StreamExt,
    select,
    sync::{
        mpsc::{UnboundedReceiver, UnboundedSender},
        watch::{self, Receiver},
    },
};

// use std::sync::mpsc::{Receiver, Sender};

use uuid::Uuid;

// use crate::router::Router;

mod common;
pub mod errors;

pub use msgbus_macro::bus_uuid;

use crate::messages::{BrokerMsg, ClientMessage};

/// Reference to a foreign instance of [MsgBus]
/// * Could be in the same process, just a different MsgBus instance
#[derive(Debug, Clone)]
#[allow(dead_code)] //TODO
pub(crate) struct Node {
    id: Uuid,
    //TODO store how to get to this node
}

#[derive(Debug, Clone)]
#[allow(dead_code)] //TODO
enum UnicastType {
    Datagram,
    Rpc,
    RpcResponse,
}

#[derive(Debug, Clone)]
pub(crate) enum DestinationType {
    Local(UnboundedSender<ClientMessage>),
    #[allow(dead_code)] //TODO
    Remote(Node),
}

#[derive(Debug, Clone)]
#[allow(dead_code)] //TODO
struct UnicastDest {
    unicast_type: UnicastType,
    dest_type: DestinationType,
}

#[derive(Debug, Clone)]
pub(crate) struct AnycastDest {
    cost: u16,
    dest_type: DestinationType,
}

impl PartialOrd for AnycastDest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.cost.partial_cmp(&other.cost)
    }
}

impl PartialEq for AnycastDest {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) enum Nexthop {
    // AnycastOld(SortedVec<AnycastEntry>),
    Anycast(SortedVec<AnycastDest>),
    Broadcast(tokio::sync::broadcast::Sender<ClientMessage>),
    Unicast(UnicastDest),
    // External(tokio::sync::mpsc::Sender<ClientMessage>),
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

type Routes = HashMap<Uuid, Nexthop>;
type RoutesWatchTx = watch::Sender<Routes>;
type RoutesWatchRx = watch::Receiver<Routes>;

/// The main entry point into the MsgBus system.
pub struct MsgBus {
    // rx: UnboundedReceiver<BrokerMsg>,
    // buscontrol_rx: tokio::sync::watch::Receiver<BusControlMsg>
}

impl MsgBus {
    /// This starts and runs the MsgBus.  The returned [BusControlHandle] is used to shutdown the system.  The [Handle] is
    /// used for normal interaction with the system
    ///
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> BusControlHandle {
        // #[cfg(feature = "tokio")]
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        // #[cfg(feature = "dioxus")]
        // let (tx, rx) = futures::channel::mpsc::unbounded();

        let (bc_tx, bc_rx) = watch::channel(BusControlMsg::Run);

        let map = HashMap::new();

        let (rts_tx, rts_rx) = watch::channel(map.clone());
        let handle = Handle { tx, rts_rx };
        let control_handle = BusControlHandle {
            tx: bc_tx,
            handle: handle.clone(),
        };

        #[cfg(feature = "dioxus")]
        dioxus::prelude::spawn(Self::run(map, rx, bc_rx, rts_tx));
        #[cfg(feature = "tokio")]
        tokio::spawn(Self::run(map, rx, bc_rx, rts_tx));

        control_handle
    }

    async fn run(
        mut map: Routes,
        mut rx: UnboundedReceiver<BrokerMsg>,
        // #[cfg(feature = "tokio")] mut bc_rx: tokio::sync::watch::Receiver<BusControlMsg>,
        mut bc_rx: Receiver<BusControlMsg>,

        rts_tx: RoutesWatchTx,
    ) {
        let mut should_shutdown = false;

        // let id = random_uuid();
        // let router = Router::new(id, bc_rx.clone());
        // dbg!("Created ND Router");
        // let nd_handle = tokio::spawn(router.run());
        // dbg!("Started Neighbor discovery", nd_handle);

        loop {
            // #[cfg(feature = "dioxus")]
            // dioxus::logger::tracing::info!("Entering processing loop");
            if *bc_rx.borrow() == BusControlMsg::Shutdown || should_shutdown {
                shutdown_routing(map);

                break;
            }
            // let change_value = bc_rx.
            select! {
                //KEEP
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
                            // println!("Received shutdown request");
                            should_shutdown = true;
                            // break 'main;
                        }// TOOD log this
                    }
                },
                //AWAY
                //
                // #[cfg(feature = "dioxus")]
                msg = rx.recv() => should_shutdown = Self::process_message(msg, &mut map, &rts_tx),
                // #[cfg(feature = "tokio")]
                // msg = rx.recv().fuse() => should_shutdown = process_message(msg),

            };
        }
    }

    fn process_message(
        msg: Option<BrokerMsg>,
        map: &mut HashMap<Uuid, Nexthop>,
        rts_tx: &watch::Sender<HashMap<Uuid, Nexthop>>,
    ) -> bool {
        // #[cfg(feature = "dioxus")]
        // dioxus::logger::tracing::info!("Processing msg: {:?}", msg);

        let Some(msg) = msg else { return true };
        match msg {
            BrokerMsg::Subscribe(_uuid, _tx) => {
                todo!()
            }
            BrokerMsg::RegisterUnicast(uuid, unbounded_sender, unicast_type) => {
                let current_endpoint = map.get_mut(&uuid);
                match current_endpoint {
                    Some(_endpoint) => {
                        let _ = unbounded_sender.send(ClientMessage::FailedRegistration(
                            uuid,
                            "Duplicate registration".into(),
                        ));
                    }
                    None => {
                        let endpoint = Nexthop::Unicast(UnicastDest {
                            unicast_type,
                            dest_type: DestinationType::Local(unbounded_sender.clone()),
                        });
                        map.insert(uuid, endpoint);
                        if rts_tx.send(map.clone()).is_err() {
                            return true;
                        };
                        let _ = unbounded_sender.send(ClientMessage::SuccessfulRegistration(uuid));
                    }
                }
            }

            BrokerMsg::RegisterAnycast(uuid, tx) => {
                // TODO Make the Routes have Cow elements for ease of cloning
                // let mut new_map = (*map).clone();
                let endpoint = map.get_mut(&uuid);
                match endpoint {
                    // No entry, insert a fresh Vector with one destination
                    None => {
                        let dest = AnycastDest {
                            cost: 0,
                            dest_type: DestinationType::Local(tx.clone()),
                        };
                        let mut v = SortedVec::new();
                        v.insert(dest);
                        let ep = Nexthop::Anycast(v);
                        map.insert(uuid, ep);
                    }

                    // Entry exists, insert a new destination
                    Some(Nexthop::Anycast(v)) => {
                        let dest = AnycastDest {
                            cost: 0,
                            dest_type: DestinationType::Local(tx.clone()),
                        };
                        v.insert(dest);
                    }

                    // Catches these two case
                    //Some(Nexthop::Unicast(_))
                    //Some(Nexthop::Broadcast(_))
                    _ => {
                        let msg = ClientMessage::FailedRegistration(
                            uuid,
                            "Duplicate registration".into(),
                        );
                        let _ = tx.send(msg);
                        return false;
                    }
                };
                // let idx = endpoints.partition_point(|x| x.cost < entry.cost);
                // endpoints.insert(idx, entry);
                // map = Arc::new(new_map);
                // #[cfg(feature = "dioxus")]
                // dioxus::logger::tracing::info!("Sending updated map {:?}", map);
                if rts_tx.send(map.clone()).is_err() {
                    return true;
                }; //  no handles are left so shut it down  TODO log the error
                   // #[cfg(feature = "dioxus")]
                   // dioxus::logger::tracing::info!("Map Sent");
                   // TODO announce registration to peers
                let _ = tx.send(ClientMessage::SuccessfulRegistration(uuid));
            }
            BrokerMsg::DeadLink(uuid) => {
                // let mut new_map = (*map).clone();
                let Some(nexthop) = map.get_mut(&uuid) else {
                    return false;
                };
                match nexthop {
                    Nexthop::Broadcast(_) => todo!(),
                    Nexthop::Anycast(v) => {
                        let size = v.len();
                        v.retain(|dest| match &dest.dest_type {
                            DestinationType::Remote(_) => true,
                            DestinationType::Local(tx) => !tx.is_closed(),
                        });
                        if size != v.len() {
                            // map = Arc::new(new_map);
                            if rts_tx.send(map.clone()).is_err() {
                                return true;
                            }
                        }
                    }
                    Nexthop::Unicast(_unicast_type) => {
                        map.remove(&uuid);
                    }
                };
                // let size = endpoints.len();
                // endpoints.retain(|ep| !ep.dest.is_closed());
                // if size != endpoints.len() {
                //     let map = Arc::new(new_map);
                //     if rts_tx.send(map).is_err() {
                //         should_shutdown = true;
                //     }
                // }
            }
        }
        false
    }
}

fn shutdown_routing(map: Routes) {
    for (_id, entry) in map.iter() {
        match entry {
            Nexthop::Broadcast(_) => todo!(),
            Nexthop::Anycast(sorted_vec) => {
                for entry in sorted_vec.iter() {
                    match &entry.dest_type {
                        DestinationType::Local(tx) => {
                            let _ = tx.send(ClientMessage::Shutdown);
                        }
                        DestinationType::Remote(_uuid) => {} //TODO inform neighbor,
                    }
                }
            }
            Nexthop::Unicast(unicast_dest) => match &unicast_dest.dest_type {
                DestinationType::Local(unbounded_sender) => {
                    let _ = unbounded_sender.send(ClientMessage::Shutdown);
                }
                DestinationType::Remote(_node) => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    // use super::*;

    #[test]
    fn it_works() {}
}
