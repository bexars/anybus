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

use common::random_uuid;
use erased_serde::Serialize;
use errors::ReceiveError;

// use futures::{select, FutureExt, StreamExt};
// #[cfg(feature = "dioxus-web")]
use serde::de::DeserializeOwned;
use sorted_vec::partial::SortedVec;
use std::any::{Any, TypeId};
use std::mem::swap;
use std::{collections::HashMap, error::Error, marker::PhantomData};

use tokio::sync::oneshot::Receiver as OneShotReceiver;

use tokio::{
    select,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};

// use std::sync::mpsc::{Receiver, Sender};

use uuid::Uuid;

// use crate::router::Router;

mod common;
pub mod errors;
#[cfg(target_family = "unix")]
// pub mod helper;
// mod neighbor;
// mod router;
pub use msgbus_macro::bus_uuid;

/// Any message handled by [MsgBus] must implemnet this.
///
///
pub trait BusRider: Serialize + Send + Sync + std::fmt::Debug + 'static {
    /// The uuid that should be bound to if it's not overridden during registration

    fn default_uuid(&self) -> Uuid;

    /// The as_any function needs to simply return 'self'
    // ```
    // impl dyn BusRider {
    //    fn as_any(self: Box<Self>) -> Box<dyn std::any::Any> {
    //        self
    //    }
    // }
    // ```
    fn as_any(self: Box<Self>) -> Box<dyn Any>;

    // fn uuid<T>() -> Uuid
    // where T: ?Sized;
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
    Shutdown,
}

#[derive(Debug, Clone)]
pub(crate) struct UnicastEntry {
    cost: u16,
    dest: UnboundedSender<ClientMessage>,
}

impl PartialOrd for UnicastEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.cost.partial_cmp(&other.cost)
    }
}

impl PartialEq for UnicastEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}

#[cfg(feature = "tokio")]
#[allow(dead_code)]
#[derive(Debug, Clone)]

pub(crate) enum Endpoint {
    Unicast(SortedVec<UnicastEntry>),
    Broadcast(tokio::sync::broadcast::Sender<ClientMessage>),
    // External(tokio::sync::mpsc::Sender<ClientMessage>),
}

#[cfg(feature = "dioxus-web")]
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) enum Endpoint {
    Unicast(SortedVec<UnicastEntry>),
    Broadcast(dioxus::prelude::UnboundedSender<ClientMessage>),
    // External(tokio::sync::mpsc::Sender<ClientMessage>),
}

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

    pub fn register_anycast<T: BusRider + DeserializeOwned>(
        &mut self,
        uuid: Uuid,
    ) -> Result<BusListener<T>, Box<dyn Error>> {
        #[cfg(feature = "tokio")]
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        #[cfg(feature = "dioxus-web")]
        let (tx, rx) = futures_channel::mpsc::unbounded();

        let bl = BusListener::<T> {
            rx,
            _pd: PhantomData,
        };

        let register_msg = BrokerMsg::RegisterAnycast(uuid, tx);

        #[cfg(feature = "dioxus-web")]
        self.tx.unbounded_send(register_msg)?;

        #[cfg(feature = "tokio")]
        self.tx.send(register_msg)?;
        Ok(bl)
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
    pub fn send<T: BusRider>(&self, payload: T) -> Result<(), errors::MsgBusHandleError> {
        let u = payload.default_uuid();
        let payload = Box::new(payload);
        let mut msg = Some(ClientMessage::Message(u, payload));
        match self.rts_rx.borrow().get(&u) {
            Some(endpoint) => {
                let mut success = false;
                let mut had_failure = false;
                // TODO Turn this into a scan iterator or just about anything better
                match endpoint {
                    Endpoint::Unicast(v) => {
                        // this is all convoluted to retrieve the original msg and send it to the next possible destination

                        for entry in v.iter() {
                            let mut m = None;
                            swap(&mut m, &mut msg);
                            let m = m.expect("Should never panic here. Problem swapping data");

                            #[cfg(feature = "tokio")]
                            let send_result = entry.dest.send(m);

                            #[cfg(feature = "dioxus-web")]
                            let send_result = entry.dest.unbounded_send(m);

                            match send_result {
                                Ok(_) => {
                                    success = true;
                                    if !had_failure {
                                        return Ok(());
                                    }
                                    break;
                                }
                                // #[cfg(feature = "tokio")]
                                // Err(SendError(m)) => {
                                //     had_failure = true;
                                //     let mut m = Some(m);
                                //     swap(&mut m, &mut msg)
                                // }
                                // #[cfg(feature = "dioxus-web")]
                                Err(e) => {
                                    had_failure = true;
                                    #[cfg(feature = "dioxus-web")]
                                    let mut m = Some(e.into_inner());
                                    #[cfg(feature = "tokio")]
                                    let mut m = Some(e.0);

                                    swap(&mut m, &mut msg)
                                }
                            }
                        }
                    }
                    Endpoint::Broadcast(_) => todo!(),
                    // Endpoint::External(_) => todo!(),
                }

                if had_failure {
                    #[cfg(feature = "tokio")]
                    self.tx.send(BrokerMsg::DeadLink(u));
                    #[cfg(feature = "dioxus-web")]
                    self.tx.unbounded_send(BrokerMsg::DeadLink(u)); // Just ignore if this fails
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

/// Helper struct returned by [Handle::register_anycast()]
pub struct BusListener<T: BusRider> {
    rx: UnboundedReceiver<ClientMessage>,
    _pd: PhantomData<T>,
}

impl<T: BusRider + DeserializeOwned> BusListener<T> {
    /// Returns T until an error occurs.  There is no recovery from an
    /// error and the connection should be considered closed.
    ///
    /// If this was created from a failed registration, the first message will be [RegistrationFailed](errors::ReceiveError::RegistrationFailed)
    pub async fn recv(&mut self) -> Result<T, ReceiveError> {
        loop {
            #[cfg(feature = "tokio")]
            let new_msg = self.rx.recv().await;
            #[cfg(feature = "dioxus-web")]
            let new_msg = self.rx.next().await;

            match new_msg {
                None => {
                    return Err(ReceiveError::ConnectionClosed);
                }
                Some(msg) => {
                    match msg {
                        ClientMessage::Message(_id, payload) => {
                            let payload = payload.as_any();
                            if TypeId::of::<T>() == (*payload).type_id() {
                                let payload: Box<T> = payload.downcast().unwrap();
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
                        } // BrokerMsg::Register(_, _) => panic!("Should never receive Register from the broker"), // Shouldn't receive these so we'll just ignore it
                        // TODO Log the bad register request
                        ClientMessage::Shutdown => {
                            println!("Shutdown in BusListener");
                            return Err(ReceiveError::Shutdown);
                        }
                    }
                }
            };
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

type Routes = HashMap<Uuid, Endpoint>;
type RoutesWatchTx = async_watch::Sender<Routes>;
type RoutesWatchRx = async_watch::Receiver<Routes>;

/// The main entry point into the MsgBus system.
pub struct MsgBus {
    // rx: UnboundedReceiver<BrokerMsg>,
    // buscontrol_rx: tokio::sync::watch::Receiver<BusControlMsg>
}

impl<'a> MsgBus {
    /// This starts and runs the MsgBus.  The returned [BusControlHandle] is used to shutdown the system.  The [Handle] is
    /// used for normal interaction with the system
    ///
    pub fn new() -> (BusControlHandle, Handle) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let (bc_tx, bc_rx) = async_watch::channel(BusControlMsg::Run);

        let map = HashMap::new();

        let (rts_tx, rts_rx) = async_watch::channel(map.clone());

        let control_handle = BusControlHandle { tx: bc_tx };

        let handle = Handle { tx, rts_rx };

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
        mut bc_rx: async_watch::Receiver<BusControlMsg>,

        rts_tx: RoutesWatchTx,
    ) {
        let mut should_shutdown = false;

        let id = random_uuid();
        // let router = Router::new(id, bc_rx.clone());
        // dbg!("Created ND Router");
        // let nd_handle = tokio::spawn(router.run());
        // dbg!("Started Neighbor discovery", nd_handle);

        let mut process_message = |msg: Option<BrokerMsg>| -> bool {
            let Some(msg) = msg else { return true };
            match msg {
                BrokerMsg::Subscribe(_uuid, _tx) => {
                    todo!()
                }
                BrokerMsg::RegisterAnycast(uuid, tx) => {
                    let entry = UnicastEntry {
                        cost: 0,
                        dest: tx.clone(),
                    };
                    // TODO Make the Routes have Cow elements for ease of cloning
                    // let mut new_map = (*map).clone();
                    let endpoint = map.get_mut(&uuid);
                    match endpoint {
                        Some(Endpoint::Unicast(v)) => {
                            v.insert(entry);
                        }
                        Some(Endpoint::Broadcast(_)) => {
                            let msg = ClientMessage::FailedRegistration(uuid);
                            #[cfg(feature = "tokio")]
                            let _ = tx.send(msg);
                            #[cfg(feature = "dioxus-web")]
                            let _ = tx.unbounded_send(msg);
                            return false;
                        }
                        // Some(Endpoint::External(_)) => {}
                        None => {
                            let mut v = SortedVec::new();
                            v.insert(entry);
                            let ep = Endpoint::Unicast(v);
                            map.insert(uuid, ep);
                        }
                    };
                    // let idx = endpoints.partition_point(|x| x.cost < entry.cost);
                    // endpoints.insert(idx, entry);
                    // map = Arc::new(new_map);
                    if rts_tx.send(map.clone()).is_err() {
                        return true;
                    }; //  no handles are left so shut it down  TODO log the error

                    // TODO announce registration to peers
                }
                BrokerMsg::DeadLink(uuid) => {
                    // let mut new_map = (*map).clone();
                    let Some(endpoint) = map.get_mut(&uuid) else {
                        return false;
                    };
                    match endpoint {
                        Endpoint::Unicast(v) => {
                            let size = v.len();
                            v.retain(|ep| !ep.dest.is_closed());
                            if size != v.len() {
                                // map = Arc::new(new_map);
                                if rts_tx.send(map.clone()).is_err() {
                                    return true;
                                }
                            }
                        }
                        Endpoint::Broadcast(_) => todo!(),
                        // Endpoint::External(_) => { todo!() }
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
            if *bc_rx.borrow() == BusControlMsg::Shutdown || should_shutdown {
                shutdown_routing(map);

                break;
            }

            select! {
                _ = bc_rx.changed() => {
                    match *bc_rx.borrow() {
                        BusControlMsg::Run => {},  // should never receive this but it's a non-issue
                        BusControlMsg::Shutdown => {
                            println!("Received shutdown request");
                            should_shutdown = true;
                            // break 'main;
                        }// TOOD log this
                    }
                },

                msg = rx.recv() => should_shutdown = process_message(msg),

            };
        }
    }
}

fn shutdown_routing(map: Routes) {
    for (id, entry) in map.iter() {
        match entry {
            Endpoint::Unicast(v) => {
                for ue in v.iter() {
                    #[cfg(feature = "tokio")]
                    let res = ue.dest.send(ClientMessage::Shutdown);
                    #[cfg(feature = "dioxus-web")]
                    let res = ue.dest.unbounded_send(ClientMessage::Shutdown);

                    match res {
                        Ok(_) => {
                            // println!("Sent shutdown to {id}")
                            // TODO LOG
                        }
                        Err(e) => println!("Send error in shutdown_routing {id} {:?}", e), //TODO LOG
                    };
                }
            }
            Endpoint::Broadcast(_) => todo!(),
            // Endpoint::External(_) => todo!(),
        }
    }
}

#[cfg(test)]
mod tests {
    // use super::*;

    #[test]
    fn it_works() {}
}
