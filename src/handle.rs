use std::any::{Any, TypeId};
use std::{error::Error, marker::PhantomData, mem::swap};

use itertools::{FoldWhile, Itertools};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;
use tracing::info;
use uuid::Uuid;

use crate::bus_listener::BusListener;
use crate::errors::MsgBusHandleError;
use crate::messages::{BrokerMsg, ClientMessage};
use crate::{
    errors::{self, ReceiveError},
    BusRider, BusRiderWithUuid, Nexthop, RoutesWatchRx,
};
use crate::{BusRiderRpc, DestinationType, RegistrationStatus, UnicastDest, UnicastType};

/// The handle for talking to the [MsgBus] instance that created it.  It can be cloned freely
#[derive(Debug, Clone)]
pub struct Handle {
    pub(crate) tx: UnboundedSender<BrokerMsg>,
    pub(crate) rts_rx: RoutesWatchRx,
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
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let mut bl = BusListener::<T> {
            rx,
            _pd: PhantomData,
            registration_status: Default::default(),
        };

        let register_msg = BrokerMsg::RegisterAnycast(uuid, tx);
        self.tx.send(register_msg)?;
        // info!("Send register_msg");

        match bl.wait_for_registration().await {
            RegistrationStatus::Pending => unreachable!(),
            RegistrationStatus::Registered => Ok(bl),
            RegistrationStatus::Failed(msg) => Err(ReceiveError::RegistrationFailed(msg)),
        }
    }

    /// Similar to anycast but only one receiver can be registered at a time
    pub async fn register_unicast<T: BusRiderWithUuid + DeserializeOwned>(
        &mut self,
    ) -> Result<BusListener<T>, ReceiveError> {
        let uuid = T::MSGBUS_UUID;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let mut bl = BusListener::<T> {
            rx,
            _pd: PhantomData,
            registration_status: Default::default(),
        };

        let register_msg = BrokerMsg::RegisterUnicast(uuid, tx, UnicastType::Datagram);
        info!("Send register_msg {:?}", register_msg);

        self.tx.send(register_msg)?;

        match bl.wait_for_registration().await {
            RegistrationStatus::Pending => unreachable!(),
            RegistrationStatus::Registered => Ok(bl),
            RegistrationStatus::Failed(msg) => Err(ReceiveError::RegistrationFailed(msg)),
        }
    }

    /// Register a RPC service with the broker.
    pub async fn register_rpc<T: BusRiderRpc + DeserializeOwned>(
        &mut self,
    ) -> Result<BusListener<T>, ReceiveError> {
        let uuid = T::MSGBUS_UUID;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let mut bl = BusListener::<T> {
            rx,
            _pd: PhantomData,
            registration_status: Default::default(),
        };

        let register_msg = BrokerMsg::RegisterUnicast(uuid, tx, UnicastType::Rpc);
        info!("Send register_msg {:?}", register_msg);

        self.tx.send(register_msg)?;

        match bl.wait_for_registration().await {
            RegistrationStatus::Pending => unreachable!(),
            RegistrationStatus::Registered => Ok(bl),
            RegistrationStatus::Failed(msg) => Err(ReceiveError::RegistrationFailed(msg)),
        }
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
        let u = T::MSGBUS_UUID;
        let payload = Box::new(payload);
        match self.rts_rx.borrow().get(&u) {
            Some(endpoint) => {
                let mut msg = Some(ClientMessage::Message(u, payload));

                let mut success = false;
                let mut had_failure = false;

                // TODO Turn this into a scan iterator or just about anything better
                match endpoint {
                    Nexthop::Anycast(dests) => {
                        //TODO call deadlink on these
                        let errors = dests
                            .iter()
                            .fold_while(Vec::new(), |mut prev, dest| match &dest.dest_type {
                                DestinationType::Local(tx) => {
                                    let mut m = None;
                                    swap(&mut m, &mut msg);
                                    let m = m.unwrap();

                                    let res = tx.send(m);
                                    match res {
                                        Ok(_) => {
                                            success = true;
                                            FoldWhile::Done(prev)
                                        }
                                        Err(e) => {
                                            let m = e.0;
                                            let mut m = Some(m);
                                            swap(&mut m, &mut msg);
                                            prev.push(tx.clone());
                                            itertools::FoldWhile::Continue(prev)
                                        }
                                    }
                                }
                                DestinationType::Remote(_uuid) => todo!(),
                            })
                            .into_inner();
                        if !errors.is_empty() {
                            had_failure = true;
                        };

                        if had_failure {
                            // #[cfg(feature = "tokio")]
                            // self.tx.send(BrokerMsg::DeadLink(u));
                            // #[cfg(feature = "dioxus")]
                            let _ = self.tx.send(BrokerMsg::DeadLink(u)); // Just ignore if this fails
                        }

                        if !success {
                            let Some(ClientMessage::Message(_, payload)) = msg else {
                                panic!()
                            };
                            return Err(errors::MsgBusHandleError::SendError(payload));
                        }
                    }
                    Nexthop::Broadcast(_) => todo!(),
                    Nexthop::Unicast(dest) => {
                        info!("Send Unicast {dest:?} {msg:?}");
                        match &dest.dest_type {
                            DestinationType::Local(tx) => {
                                let res = tx.send(msg.unwrap()).map_err(|e| {
                                    let msg = e.0;
                                    if let ClientMessage::Message(_, payload) = msg {
                                        return MsgBusHandleError::SendError(payload);
                                    }
                                    todo!();
                                });
                                return res;
                            }
                            DestinationType::Remote(_node) => todo!(),
                        };
                    }
                }
            }
            None => return Err(errors::MsgBusHandleError::NoRoute(payload)),
        };

        // TODO lookup in watched hashmap
        // self.tx.send(Message::Message(u, payload))?;
        Ok(())
    }

    /// Initiate an RPC request to a remote node
    pub fn request<T: BusRiderRpc>(&self, payload: T) -> Result<RpcResponse<T>, MsgBusHandleError> {
        let u = T::MSGBUS_UUID;
        let payload = Box::new(payload);
        let map = self.rts_rx.borrow();
        dbg!(&map);
        match map.get(&u) {
            Some(endpoint) => {
                // let msg = ClientMessage::Message(u, payload);

                // TODO Turn this into a scan iterator or just about anything better
                match endpoint {
                    Nexthop::Unicast(UnicastDest {
                        unicast_type: UnicastType::Rpc,
                        dest_type: DestinationType::Local(tx),
                    }) => {
                        let (resp_tx, resp_rx) = oneshot::channel::<Box<dyn BusRider>>();
                        let msg = ClientMessage::Rpc {
                            to: u,
                            reply_to: resp_tx,
                            msg: payload,
                        };
                        tx.send(msg).map_err(|value| match value.0 {
                            ClientMessage::Message(_uuid, bus_rider) => {
                                MsgBusHandleError::SendError(bus_rider)
                            }
                            _ => unreachable!(),
                        })?;
                        let rhandle: RpcResponse<T> = RpcResponse::new(resp_rx);
                        Ok(rhandle)
                    }

                    _ => Err(MsgBusHandleError::NoRoute(payload)),
                }
            }
            None => {
                // Handle the case when there is no route
                Err(MsgBusHandleError::NoRoute(payload))
            }
        }
    }
}

/// Represents a response to an RPC request.
#[derive(Debug)]
pub struct RpcResponse<T>
where
    T: BusRiderRpc,
{
    response: oneshot::Receiver<Box<dyn BusRider>>,
    phantom: PhantomData<T>,
}

impl<T> RpcResponse<T>
where
    T: BusRiderRpc,
{
    /// A helper struct that waits for the RPC response and returns it.
    pub async fn recv(self) -> Result<T::Response, Box<dyn Error>> {
        let rx = self.response;
        let payload = rx.await?;
        let payload: Box<T::Response> = (payload as Box<dyn Any>).downcast().unwrap();
        if TypeId::of::<T::Response>() == (*payload).type_id() {
            // let payload: Box<P> = (payload as Box<dyn Any>).downcast().unwrap();
            // let Some(res) = (*payload).downcast_ref::<T>() else { continue };
            return Ok(*payload);
        };
        Err("Bad payload".into())
    }
}

impl<T> RpcResponse<T>
where
    T: BusRiderRpc,
{
    pub(crate) fn new(response: oneshot::Receiver<Box<dyn BusRider>>) -> RpcResponse<T> {
        Self {
            response,
            phantom: PhantomData::<T>,
        }
    }
}
