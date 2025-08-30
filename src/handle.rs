use std::any::{Any, TypeId};
use std::{error::Error, marker::PhantomData};

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;
use tracing::info;
use uuid::Uuid;

use crate::bus_listener::BusListener;
use crate::errors::MsgBusHandleError;
use crate::errors::{self, ReceiveError};
#[cfg(feature = "ipc")]
use crate::messages::NodeMessage;
use crate::messages::{BrokerMsg, ClientMessage, RegistrationStatus};
use crate::route_table::{
    Advertisement, DestinationType, Nexthop, RoutesWatchRx, UnicastDest, UnicastType,
};
use crate::traits::{BusRider, BusRiderRpc, BusRiderWithUuid};

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
    pub async fn register_rpc<T: BusRiderRpc + DeserializeOwned + BusRiderWithUuid>(
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

    pub(crate) fn send_bytes(&self, uuid: Uuid, payload: Vec<u8>) -> Result<(), MsgBusHandleError> {
        // let msg = ClientMessage::Bytes(uuid, payload);
        self.rts_rx
            .borrow()
            .get_route_dest(&uuid)
            .map(|dest| match dest {
                DestinationType::Local(tx) => {
                    _ = tx.send(ClientMessage::Bytes(uuid, payload));
                }
                DestinationType::Remote(peer_uuid) => {
                    let tx = self.rts_rx.borrow().get_peer(&peer_uuid).unwrap();
                    _ = tx.send(NodeMessage::BusRider(uuid, payload));
                }
            })
            .ok_or(MsgBusHandleError::NoRoute)
    }

    /// Sends a single [BusRider] message to the indicated [Uuid].  This can be a Unicast, AnyCast, or Multicast receiver.
    /// The system will deliver regardless of end type
    pub fn send<T: BusRiderWithUuid + Serialize>(
        &self,
        payload: T,
    ) -> Result<(), errors::MsgBusHandleError> {
        let uuid = T::MSGBUS_UUID;
        // let payload = Box::new(payload);
        // let msg = ClientMessage::Message(u, payload);
        self.rts_rx
            .borrow()
            .get_route_dest(&uuid)
            .map(|dest| match dest {
                DestinationType::Local(tx) => {
                    _ = tx.send(ClientMessage::Message(uuid, Box::new(payload)))
                }
                DestinationType::Remote(peer_uuid) => {
                    let vec = bincode2::serialize(&payload).unwrap();
                    let tx = self.rts_rx.borrow().get_peer(&peer_uuid).unwrap();
                    _ = tx.send(NodeMessage::BusRider(uuid, vec));
                }
            })
            .ok_or(MsgBusHandleError::NoRoute)
        // self.inner_send(u, Payload::Rider(payload))
    }

    /// Initiate an RPC request to a remote node
    pub async fn request<T: BusRiderRpc + BusRiderWithUuid>(
        &self,
        payload: T,
    ) -> Result<T::Response, MsgBusHandleError> {
        let u = T::MSGBUS_UUID;
        let payload = Box::new(payload);
        let map = self.rts_rx.borrow();
        dbg!(&map);
        let response = match map.get_route(&u) {
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
                            ClientMessage::Message(_uuid, _bus_rider) => {
                                MsgBusHandleError::SendError
                            }
                            _ => unreachable!(),
                        })?;
                        let rhandle: RpcResponse<T> = RpcResponse::new(resp_rx);
                        Ok(rhandle)
                    }

                    _ => Err(MsgBusHandleError::NoRoute),
                }
            }
            None => {
                // Handle the case when there is no route
                Err(MsgBusHandleError::NoRoute)
            }
        }?;
        response.recv().await
    }

    pub(crate) fn add_peer_endpoints(&self, uuid: Uuid, ads: Vec<Advertisement>) {
        _ = self.tx.send(BrokerMsg::AddPeerEndpoints(uuid, ads));
    }

    #[cfg(feature = "ipc")]
    pub(crate) fn register_peer(&self, uuid: Uuid, tx: UnboundedSender<NodeMessage>) {
        self.tx.send(BrokerMsg::RegisterPeer(uuid, tx)).unwrap();
    }

    #[cfg(feature = "ipc")]
    pub(crate) fn unregister_peer(&self, uuid: Uuid) {
        self.tx.send(BrokerMsg::UnRegisterPeer(uuid)).unwrap();
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
    pub async fn recv(self) -> Result<T::Response, MsgBusHandleError> {
        let rx = self.response;
        let payload = rx.await?;
        let payload: Box<T::Response> = (payload as Box<dyn Any>).downcast().unwrap();
        if TypeId::of::<T::Response>() == (*payload).type_id() {
            // let payload: Box<P> = (payload as Box<dyn Any>).downcast().unwrap();
            // let Some(res) = (*payload).downcast_ref::<T>() else { continue };
            return Ok(*payload);
        };
        Err(MsgBusHandleError::ReceiveError(
            "TODO the correct error".into(),
        ))
        // Err("Bad payload".into())
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
