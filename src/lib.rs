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
use erased_serde::Serialize;
use errors::ReceiveError;

use futures::{select, FutureExt, StreamExt};
use itertools::{FoldWhile, Itertools};
// #[cfg(feature = "dioxus-web")]
use serde::de::DeserializeOwned;
use sorted_vec::partial::SortedVec;
use std::any::{Any, TypeId};
use std::mem::swap;
use std::{collections::HashMap, error::Error, marker::PhantomData};

// use tokio::sync::oneshot::Receiver as OneShotReceiver;

#[cfg(feature = "dioxus")]
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};

#[cfg(feature = "tokio")]
use tokio::{
    // select,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};

// use std::sync::mpsc::{Receiver, Sender};

use uuid::Uuid;

// use crate::router::Router;

mod common;
pub mod errors;
// #[cfg(target_family = "unix")]
// pub mod helper;
// mod neighbor;
// mod router;
pub use msgbus_macro::bus_uuid;

/// Any message handled by [MsgBus] must have these properties
///
///
pub trait BusRider: Any + Serialize + Send + Sync + std::fmt::Debug + 'static {
    // The uuid that should be bound to if it's not overridden during registration
    // fn default_uuid(&self) -> Uuid;

    // The as_any function needs to simply return 'self'
    // ```
    // impl dyn BusRider {
    //    fn as_any(self: Box<Self>) -> Box<dyn std::any::Any> {
    //        self
    //    }
    // }
    // ```
    // fn as_any(self: Box<Self>) -> Box<dyn Any>;
}

/// Blanket implementation for any type that has the correct traits
///
impl<T: Any + Serialize + Send + Sync + std::fmt::Debug + 'static> BusRider for T {}

/// Trait for ease of sending over the bus without the client needing to know the UUID of the default receiver
pub trait BusRiderWithUuid: BusRider {
    /// The default Uuid that will be used if not overridden during registration
    const MSGBUS_UUID: Uuid;
}

/// Trait for denoting the type of a returned RPC response
pub trait BusRiderRpc: BusRider {
    /// The type of the response that will be returned by the RPC call
    type Response;
}

#[allow(dead_code)]
#[derive(Debug)]
enum BrokerMsg {
    RegisterAnycast(Uuid, UnboundedSender<ClientMessage>),
    Subscribe(Uuid, UnboundedSender<ClientMessage>),
    DeadLink(Uuid),
}
#[allow(dead_code)]
#[derive(Debug)]
enum ClientMessage {
    Message(Uuid, Box<dyn BusRider>),
    Bytes(Uuid, Vec<u8>),
    Rpc {
        to: &'static Uuid,
        reply_to: UnboundedReceiver<Box<ClientMessage>>,
        msg: Box<dyn BusRider>,
    },
    FailedRegistration(Uuid),
    SuccessfulRegistration(Uuid),
    Shutdown,
}

#[allow(dead_code)] //TODO
enum UnicastType {
    Datagram,
    Rpc,
    RpcResponse,
}

// #[derive(Debug, Clone)]
// pub(crate) struct AnycastEntry {
//     cost: u16,
//     dest: UnboundedSender<ClientMessage>,
// }

// impl PartialOrd for AnycastEntry {
//     fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
//         self.cost.partial_cmp(&other.cost)
//     }
// }

// impl PartialEq for AnycastEntry {
//     fn eq(&self, other: &Self) -> bool {
//         self.cost == other.cost
//     }
// }

#[allow(dead_code)] //TODO
pub(crate) struct Node {
    id: Uuid,
    //TODO store how to get to this node
}

#[derive(Debug, Clone)]
#[allow(dead_code)] //TODO
pub(crate) enum AnycastDestType {
    Local(UnboundedSender<ClientMessage>),
    Remote(Uuid),
}

#[derive(Debug, Clone)]
pub(crate) struct AnycastDest {
    cost: u16,
    dest_type: AnycastDestType,
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

// #[cfg(feature = "tokio")]
#[allow(dead_code)]
#[derive(Debug, Clone)]

pub(crate) enum Nexthop {
    // AnycastOld(SortedVec<AnycastEntry>),
    Anycast(SortedVec<AnycastDest>),
    Broadcast(tokio::sync::broadcast::Sender<ClientMessage>),
    // External(tokio::sync::mpsc::Sender<ClientMessage>),
}

// #[cfg(feature = "dioxus")]
// #[allow(dead_code)]
// #[derive(Debug, Clone)]
// pub(crate) enum Endpoint {
//     Unicast(SortedVec<UnicastEntry>),
//     Broadcast(dioxus::prelude::UnboundedSender<ClientMessage>),
//     // External(tokio::sync::mpsc::Sender<ClientMessage>),
// }

// #[derive(Debug, Clone)]
// pub(crate) enum NextHop {
//     Local(UnboundedSender<ClientMessage>),
//     External(Uuid),
// }

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum BusControlMsg {
    Run,
    Shutdown,
}

/// The handle for talking to the [MsgBus] instance that created it.  It can be cloned freely
#[derive(Debug, Clone)]
pub struct Handle {
    tx: UnboundedSender<BrokerMsg>,
    rts_rx: RoutesWatchRx,
}

impl Handle {
    /// Registers an anycast style of listener to the given Uuid and type T that will return a [BusListener] for receiving
    /// messages sent to the [Uuid]
    // TODO: Cleanup the errors being sent
    pub async fn register_anycast<T: BusRiderWithUuid + DeserializeOwned>(
        &mut self,
        // uuid: Uuid,
    ) -> Result<BusListener<T>, ReceiveError> {
        let uuid = T::MSGBUS_UUID;
        #[cfg(feature = "tokio")]
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        #[cfg(feature = "dioxus")]
        let (tx, rx) = futures_channel::mpsc::unbounded();

        let mut bl = BusListener::<T> {
            rx,
            _pd: PhantomData,
            registration_status: Default::default(),
        };

        let register_msg = BrokerMsg::RegisterAnycast(uuid, tx);

        #[cfg(feature = "dioxus")]
        self.tx.unbounded_send(register_msg)?;

        #[cfg(feature = "tokio")]
        self.tx.send(register_msg)?;

        #[cfg(feature = "tokio")]
        info!("Send register_msg");

        if bl.is_registered().await {
            Ok(bl)
        } else {
            Err(ReceiveError::RegistrationFailed)
        }
    }

    /// Placeholder - Not Implemented
    pub fn register_unicast<T: BusRider + DeserializeOwned>(
        &mut self,
        _uuid: Uuid,
    ) -> Result<BusListener<T>, Box<dyn Error>> {
        todo!()
    }

    /// Placeholder - Not Implemented
    pub fn register_multicast<T: BusRider + DeserializeOwned>(
        &mut self,
        _uuid: Uuid,
    ) -> Result<BusListener<T>, Box<dyn Error>> {
        todo!()
    }

    // pub fn register<T: BusRider + DeserializeOwned>(
    //     &mut self,

    // ) -> Result<BusListener<T>, Box<dyn Error>> {
    //     self.register_with_uuid(T::default_uuid())
    // }

    /// Sends a single [BusRider] message to the indicated [Uuid].  This can be a Unicast, AnyCast, or Multicast receiver.
    /// The system will deliver regardless of end type
    pub fn send<T: BusRiderWithUuid>(&self, payload: T) -> Result<(), errors::MsgBusHandleError> {
        // let u = payload.default_uuid();
        let u = T::MSGBUS_UUID;
        let payload = Box::new(payload);
        let mut msg = Some(ClientMessage::Message(u, payload));
        // let mut msg2 = msg.clone().unwrap();
        #[cfg(feature = "dioxus")]
        dioxus::logger::tracing::info!("Received msg in send(): {:?}", msg);
        match self.rts_rx.borrow().get(&u) {
            Some(endpoint) => {
                let mut success = false;
                let mut had_failure = false;

                // TODO Turn this into a scan iterator or just about anything better
                match endpoint {
                    // Nexthop::AnycastOld(v) => {
                    //     // this is all convoluted to retrieve the original msg and send it to the next possible destination
                    //     for entry in v.iter() {
                    //         let mut m = None;
                    //         swap(&mut m, &mut msg);
                    //         let m = m.expect("Should never panic here. Problem swapping data");

                    //         #[cfg(feature = "tokio")]
                    //         let send_result = entry.dest.send(m);

                    //         #[cfg(feature = "dioxus")]
                    //         let send_result = entry.dest.unbounded_send(m);

                    //         match send_result {
                    //             Ok(_) => {
                    //                 success = true;
                    //                 if !had_failure {
                    //                     return Ok(());
                    //                 }
                    //                 //TODO Something isn't right here.  Why !had_failure?
                    //                 break;
                    //             }
                    //             // #[cfg(feature = "tokio")]
                    //             // Err(SendError(m)) => {
                    //             //     had_failure = true;
                    //             //     let mut m = Some(m);
                    //             //     swap(&mut m, &mut msg)
                    //             // }
                    //             // #[cfg(feature = "dioxus")]
                    //             Err(e) => {
                    //                 had_failure = true;
                    //                 #[cfg(feature = "dioxus")]
                    //                 let mut m = Some(e.into_inner());
                    //                 #[cfg(feature = "tokio")]
                    //                 let mut m = Some(e.0);

                    //                 swap(&mut m, &mut msg)
                    //             }
                    //         }
                    //     }
                    // }
                    Nexthop::Anycast(dests) => {
                        //TODO call deadlink on these
                        let errors = dests
                            .iter()
                            .fold_while(Vec::new(), |mut prev, dest| match &dest.dest_type {
                                AnycastDestType::Local(tx) => {
                                    let mut m = None;
                                    swap(&mut m, &mut msg);
                                    let m = m.unwrap();

                                    let res = tx.unbounded_send(m);
                                    match res {
                                        Ok(_) => {
                                            success = true;
                                            FoldWhile::Done(prev)
                                        }
                                        Err(e) => {
                                            let m = e.into_inner();
                                            let mut m = Some(m);
                                            swap(&mut m, &mut msg);
                                            prev.push(tx.clone());
                                            itertools::FoldWhile::Continue(prev)
                                        }
                                    }
                                }
                                AnycastDestType::Remote(_uuid) => todo!(),
                            })
                            .into_inner();
                        if !errors.is_empty() {
                            had_failure = true;
                        };
                    }
                    Nexthop::Broadcast(_) => todo!(),
                    // Endpoint::External(_) => todo!(),
                }

                if had_failure {
                    #[cfg(feature = "tokio")]
                    self.tx.send(BrokerMsg::DeadLink(u));
                    #[cfg(feature = "dioxus")]
                    let _ = self.tx.unbounded_send(BrokerMsg::DeadLink(u)); // Just ignore if this fails
                }

                if !success {
                    let Some(ClientMessage::Message(_, payload)) = msg else {
                        panic!()
                    };
                    return Err(errors::MsgBusHandleError::SendError(payload));
                }
            }
            None => return Err(errors::MsgBusHandleError::NoRoute),
        };

        // TODO lookup in watched hashmap
        // self.tx.send(Message::Message(u, payload))?;
        Ok(())
    }
}

#[derive(Debug, Default)]
enum RegistrationStatus {
    #[default]
    Pending,
    Registered,
    Failed,
}

#[derive(Debug)]
/// Helper struct returned by [Handle::register_anycast()]
pub struct BusListener<T: BusRider> {
    rx: UnboundedReceiver<ClientMessage>,
    registration_status: RegistrationStatus,
    _pd: PhantomData<T>,
}

impl<T: BusRider + DeserializeOwned> BusListener<T> {
    /// Returns T until an error occurs.  There is no recovery from an
    /// error and the connection should be considered closed.
    ///
    /// If this was created from a failed registration, the first message will be [RegistrationFailed](errors::ReceiveError::RegistrationFailed)
    pub async fn recv(&mut self) -> Result<T, ReceiveError> {
        loop {
            #[cfg(feature = "dioxus")]
            dioxus::logger::tracing::info!("In recv loop: {:?}", self);

            #[cfg(feature = "tokio")]
            let new_msg = self.rx.recv().await;
            #[cfg(feature = "dioxus")]
            let new_msg = self.rx.next().await;
            #[cfg(feature = "dioxus")]
            dioxus::logger::tracing::info!("Received msg: {:?}", new_msg);
            match new_msg {
                None => {
                    return Err(ReceiveError::ConnectionClosed);
                }
                Some(msg) => {
                    match msg {
                        ClientMessage::Message(_id, payload) => {
                            // let payload = payload.as_any();
                            if TypeId::of::<T>() == (*payload).type_id() {
                                let payload: Box<T> = (payload as Box<dyn Any>).downcast().unwrap();
                                // let Some(res) = (*payload).downcast_ref::<T>() else { continue };
                                return Ok(*payload);
                            } else {
                                continue;
                            }
                        }
                        ClientMessage::Bytes(_id, bytes) => match serde_cbor::from_slice(&bytes) {
                            Ok(payload) => return Ok(payload),
                            Err(_) => continue,
                        },
                        ClientMessage::Rpc {
                            to: _,
                            reply_to: _,
                            msg: _,
                        } => todo!(),
                        ClientMessage::FailedRegistration(_uuid) => {
                            return Err(ReceiveError::RegistrationFailed)
                        }
                        ClientMessage::Shutdown => {
                            println!("Shutdown in BusListener");
                            return Err(ReceiveError::Shutdown);
                        }
                        ClientMessage::SuccessfulRegistration(_uuid) => todo!(),
                    }
                }
            };
        }
    }

    /// Similar to recv() but with a enum denoting a message or a RPC request
    pub async fn recv_rpc(&mut self) -> Result<T, ReceiveError> {
        loop {
            #[cfg(feature = "dioxus")]
            dioxus::logger::tracing::info!("In recv loop: {:?}", self);

            #[cfg(feature = "tokio")]
            let new_msg = self.rx.recv().await;
            #[cfg(feature = "dioxus")]
            let new_msg = self.rx.next().await;
            #[cfg(feature = "dioxus")]
            dioxus::logger::tracing::info!("Received msg: {:?}", new_msg);
            match new_msg {
                None => {
                    return Err(ReceiveError::ConnectionClosed);
                }
                Some(msg) => {
                    match msg {
                        ClientMessage::Message(_id, payload) => {
                            let payload: Box<T> = (payload as Box<dyn Any>).downcast().unwrap();
                            if TypeId::of::<T>() == (*payload).type_id() {
                                let payload: Box<T> = (payload as Box<dyn Any>).downcast().unwrap();
                                // let Some(res) = (*payload).downcast_ref::<T>() else { continue };
                                return Ok(*payload);
                            } else {
                                continue;
                            }
                        }
                        ClientMessage::Bytes(_id, bytes) => match serde_cbor::from_slice(&bytes) {
                            Ok(payload) => return Ok(payload),
                            Err(_) => continue,
                        },
                        ClientMessage::Rpc {
                            to: _,
                            reply_to: _,
                            msg: _,
                        } => todo!(),
                        ClientMessage::FailedRegistration(_uuid) => {
                            return Err(ReceiveError::RegistrationFailed)
                        }
                        ClientMessage::Shutdown => {
                            println!("Shutdown in BusListener");
                            return Err(ReceiveError::Shutdown);
                        }
                        ClientMessage::SuccessfulRegistration(_uuid) => todo!(),
                    }
                }
            };
        }
    }

    /// Check if the client is registered with the bus.
    pub async fn is_registered(&mut self) -> bool {
        match self.registration_status {
            RegistrationStatus::Registered => true,
            RegistrationStatus::Failed => false,
            RegistrationStatus::Pending => match self.rx.next().await {
                Some(ClientMessage::SuccessfulRegistration(_)) => {
                    self.registration_status = RegistrationStatus::Registered;
                    true
                }
                _ => {
                    self.registration_status = RegistrationStatus::Failed;
                    false
                }
            },
        }
    }
}

/// Handle for programatically shutting down the system.  Can be wrapped with [helper::ShutdownWithCtrlC] to catch user termination
/// gracefully
pub struct BusControlHandle {
    // #[cfg(feature = "tokio")]
    pub(crate) tx: async_watch::Sender<BusControlMsg>,
}

impl BusControlHandle {
    /// Passes the shutdown command to the MsgBus system and all local listeners.  Immediately withdraws all advertisements from the network.
    ///
    /// If the program is killed by other means it can take up to 40 seconds for other systems to forget the advertisements from this MsgBus
    ///
    pub fn shutdown(&mut self) {
        self.tx.send(BusControlMsg::Shutdown).unwrap_or_default();
    }
}

type Routes = HashMap<Uuid, Nexthop>;
type RoutesWatchTx = async_watch::Sender<Routes>;
type RoutesWatchRx = async_watch::Receiver<Routes>;

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
    pub fn new() -> (BusControlHandle, Handle) {
        #[cfg(feature = "tokio")]
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        #[cfg(feature = "dioxus")]
        let (tx, rx) = futures::channel::mpsc::unbounded();

        let (bc_tx, bc_rx) = async_watch::channel(BusControlMsg::Run);

        let map = HashMap::new();

        let (rts_tx, rts_rx) = async_watch::channel(map.clone());

        let control_handle = BusControlHandle { tx: bc_tx };

        let handle = Handle { tx, rts_rx };

        #[cfg(feature = "dioxus")]
        dioxus::prelude::spawn(Self::run(map, rx, bc_rx, rts_tx));
        #[cfg(feature = "tokio")]
        tokio::spawn(Self::run(map, rx, bc_rx, rts_tx));

        (control_handle, handle)
    }

    /// This runs the MsgBus with no returned BusControlHandle.  Ideal for putting in statics or other places where you don't need to control the bus.
    pub fn init() -> Handle {
        let (_, handle) = Self::new();
        handle
    }

    async fn run(
        mut map: Routes,
        mut rx: UnboundedReceiver<BrokerMsg>,
        // #[cfg(feature = "tokio")] mut bc_rx: tokio::sync::watch::Receiver<BusControlMsg>,
        #[allow(unused)] //TODO  Need to reimplement this with tokio only
        bc_rx: async_watch::Receiver<BusControlMsg>,

        rts_tx: RoutesWatchTx,
    ) {
        let mut should_shutdown = false;

        // let id = random_uuid();
        // let router = Router::new(id, bc_rx.clone());
        // dbg!("Created ND Router");
        // let nd_handle = tokio::spawn(router.run());
        // dbg!("Started Neighbor discovery", nd_handle);

        let mut process_message = |msg: Option<BrokerMsg>| -> bool {
            #[cfg(feature = "dioxus")]
            dioxus::logger::tracing::info!("Processing msg: {:?}", msg);

            let Some(msg) = msg else { return true };
            match msg {
                BrokerMsg::Subscribe(_uuid, _tx) => {
                    todo!()
                }
                BrokerMsg::RegisterAnycast(uuid, tx) => {
                    // TODO Make the Routes have Cow elements for ease of cloning
                    // let mut new_map = (*map).clone();
                    let endpoint = map.get_mut(&uuid);
                    match endpoint {
                        // Some(Nexthop::AnycastOld(v)) => {
                        //     v.insert(entry);
                        // }
                        Some(Nexthop::Broadcast(_)) => {
                            let msg = ClientMessage::FailedRegistration(uuid);
                            #[cfg(feature = "tokio")]
                            let _ = tx.send(msg);
                            #[cfg(feature = "dioxus")]
                            let _ = tx.unbounded_send(msg);
                            return false;
                        }
                        Some(Nexthop::Anycast(v)) => {
                            let dest = AnycastDest {
                                cost: 0,
                                dest_type: AnycastDestType::Local(tx.clone()),
                            };
                            v.insert(dest);
                        }
                        // Some(Endpoint::External(_)) => {}
                        None => {
                            let dest = AnycastDest {
                                cost: 0,
                                dest_type: AnycastDestType::Local(tx.clone()),
                            };
                            let mut v = SortedVec::new();
                            v.insert(dest);
                            let ep = Nexthop::Anycast(v);
                            map.insert(uuid, ep);
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
                    let _ = tx.unbounded_send(ClientMessage::SuccessfulRegistration(uuid));
                }
                BrokerMsg::DeadLink(uuid) => {
                    // let mut new_map = (*map).clone();
                    let Some(nexthop) = map.get_mut(&uuid) else {
                        return false;
                    };
                    match nexthop {
                        // Nexthop::AnycastOld(v) => {
                        //     let size = v.len();
                        //     v.retain(|ep| !ep.dest.is_closed());
                        //     if size != v.len() {
                        //         // map = Arc::new(new_map);
                        //         if rts_tx.send(map.clone()).is_err() {
                        //             return true;
                        //         }
                        //     }
                        // }
                        Nexthop::Broadcast(_) => todo!(),
                        Nexthop::Anycast(v) => {
                            let size = v.len();
                            v.retain(|dest| match &dest.dest_type {
                                AnycastDestType::Remote(_) => true,
                                AnycastDestType::Local(tx) => !tx.is_closed(),
                            });
                            if size != v.len() {
                                // map = Arc::new(new_map);
                                if rts_tx.send(map.clone()).is_err() {
                                    return true;
                                }
                            }
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
                } // _ => { todo!() },
            }
            false
        };

        loop {
            #[cfg(feature = "dioxus")]
            dioxus::logger::tracing::info!("Entering processing loop");
            if *bc_rx.borrow() == BusControlMsg::Shutdown || should_shutdown {
                shutdown_routing(map);

                break;
            }

            select! {
                //KEEP
                // select! {
                // change_value = bc_rx.changed().fuse() => {
                //     #[cfg(feature = "dioxus")]
                //     dioxus::logger::tracing::info!("bc_rx.changed()");
                //     match *bc_rx.borrow() {
                //         BusControlMsg::Run => {},  // should never receive this but it's a non-issue
                //         BusControlMsg::Shutdown => {
                //             println!("Received shutdown request");
                //             should_shutdown = true;
                //             // break 'main;
                //         }// TOOD log this
                //     }
                // },
                //AWAY
                //
                // #[cfg(feature = "dioxus")]
                msg = rx.next().fuse() => should_shutdown = process_message(msg),
                // #[cfg(feature = "tokio")]
                // msg = rx.recv().fuse() => should_shutdown = process_message(msg),

            };
            #[cfg(feature = "dioxus")]
            dioxus::logger::tracing::info!("End of processing loop");
        }
    }
}

fn shutdown_routing(map: Routes) {
    for (_id, entry) in map.iter() {
        match entry {
            // Nexthop::AnycastOld(v) => {
            //     for entry in v.iter() {
            //         #[cfg(feature = "tokio")]
            //         let res = entry.dest.send(ClientMessage::Shutdown);
            //         #[cfg(feature = "dioxus")]
            //         let res = entry.dest.unbounded_send(ClientMessage::Shutdown);

            //         match res {
            //             Ok(_) => {
            //                 // TODO LOG
            //             }
            //             Err(e) => println!("Send error in shutdown_routing {id} {:?}", e), //TODO LOG
            //         };
            //     }
            // }
            Nexthop::Broadcast(_) => todo!(),
            Nexthop::Anycast(sorted_vec) => {
                for entry in sorted_vec.iter() {
                    match &entry.dest_type {
                        AnycastDestType::Local(tx) => {
                            let _ = tx.unbounded_send(ClientMessage::Shutdown);
                        }
                        AnycastDestType::Remote(_uuid) => {} //TODO inform neighbor,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // use super::*;

    #[test]
    fn it_works() {}
}
