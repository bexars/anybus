use std::collections::HashSet;
use std::{error::Error, marker::PhantomData};

use serde::de::DeserializeOwned;
use tokio::sync::mpsc::UnboundedSender;
use tracing::info;
use uuid::Uuid;

use crate::bus_listener::BusListener;
use crate::errors::MsgBusHandleError;
use crate::errors::ReceiveError;
#[cfg(feature = "ipc")]
use crate::messages::NodeMessage;
use crate::messages::{BrokerMsg, ClientMessage, RegistrationStatus};
use crate::routing::router::RoutesWatchRx;
use crate::routing::{Address, Advertisement, Packet, Payload, Route, WirePacket};

use crate::traits::{BusRider, BusRiderRpc, BusRiderWithUuid};

/// The handle for talking to the [MsgBus] instance that created it.  It can be cloned freely
#[derive(Debug, Clone)]
pub struct Handle {
    pub(crate) tx: UnboundedSender<BrokerMsg>,
    pub(crate) route_watch_rx: RoutesWatchRx,
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
            handle: self.clone(),
            endpoint_id: uuid,
        };

        let route = Route {
            kind: crate::routing::RouteKind::Anycast,
            realm: crate::routing::Realm::Userspace,
            via: crate::routing::ForwardTo::Local(tx.clone()),
            cost: 0,
            learned_from: Uuid::nil(),
        };

        let register_msg = BrokerMsg::RegisterRoute(uuid, route);
        self.tx.send(register_msg)?;
        info!("Sent register_msg");

        let response = bl.wait_for_registration().await;
        dbg!(&response);
        match response {
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
            handle: self.clone(),
            endpoint_id: uuid,
        };

        let register_msg = BrokerMsg::RegisterRoute(
            uuid,
            Route {
                kind: crate::routing::RouteKind::Unicast,
                realm: crate::routing::Realm::Userspace,
                via: crate::routing::ForwardTo::Local(tx.clone()),
                cost: 0,
                learned_from: Uuid::nil(),
            },
        );
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
            handle: self.clone(),
            endpoint_id: uuid,
        };

        let register_msg = BrokerMsg::RegisterRoute(
            uuid,
            Route {
                kind: crate::routing::RouteKind::Unicast,
                realm: crate::routing::Realm::Userspace,
                via: crate::routing::ForwardTo::Local(tx.clone()),
                cost: 0,
                learned_from: Uuid::nil(),
            },
        );
        info!("Send register_msg {:?}", register_msg);

        self.tx.send(register_msg)?;

        match bl.wait_for_registration().await {
            RegistrationStatus::Pending => unreachable!(),
            RegistrationStatus::Registered => Ok(bl),
            RegistrationStatus::Failed(msg) => Err(ReceiveError::RegistrationFailed(msg)),
        }
    }

    /// Placeholder - Not Implemented
    fn _register_multicast<T: BusRider + DeserializeOwned>(
        &mut self,
        _uuid: Uuid,
    ) -> Result<BusListener<T>, Box<dyn Error>> {
        todo!()
    }

    pub(crate) fn send_packet(&self, packet: WirePacket) {
        let map = self.route_watch_rx.borrow();
        let endpoint = if let Some(endpoint) = map.lookup(&packet.to) {
            endpoint
        } else {
            return;
        };
        _ = endpoint.send(packet);
    }

    /// Sends a single [BusRider] message to the associated UUID in the trait.
    pub fn send<T: BusRiderWithUuid>(&self, payload: T) -> Result<(), MsgBusHandleError> {
        let endpoint_id = T::MSGBUS_UUID;
        self.send_to_uuid(endpoint_id, payload)
    }

    /// Sends a single [BusRider] message to the given [Address]
    pub fn send_to_uuid<T: BusRider>(
        &self,
        address: impl Into<Address>,
        payload: T,
    ) -> Result<(), MsgBusHandleError> {
        let map = self.route_watch_rx.borrow();
        let address = address.into();
        let endpoint_id = match address {
            Address::Local(uuid) => uuid,
            Address::Remote(uuid, _uuid1) => uuid,
        };
        match map.lookup(&endpoint_id) {
            Some(endpoint) => endpoint
                .send(Packet {
                    to: endpoint_id,
                    reply_to: None,
                    from: None,
                    payload: Payload::BusRider(Box::new(payload) as Box<dyn BusRider>),
                })
                .map_err(MsgBusHandleError::SendError),

            None => Err(MsgBusHandleError::NoRoute),
        }
    }

    /// Initiate an RPC request to a remote node
    pub async fn request<T: BusRiderRpc + BusRiderWithUuid>(
        &self,
        payload: T,
    ) -> Result<T::Response, MsgBusHandleError>
    where
        for<'de> T::Response: serde::de::Deserialize<'de>,
    {
        let u = T::MSGBUS_UUID;
        let payload = Box::new(payload);
        let map = self.route_watch_rx.borrow();
        let endpoint = map.lookup(&u).ok_or(MsgBusHandleError::NoRoute)?;
        let response_uuid = Uuid::now_v7();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let register_msg = BrokerMsg::RegisterRoute(
            response_uuid,
            Route {
                kind: crate::routing::RouteKind::Unicast,
                realm: crate::routing::Realm::Process,
                via: crate::routing::ForwardTo::Local(tx.clone()),
                cost: 0,
                learned_from: Uuid::nil(),
            },
        );
        self.tx
            .send(register_msg)
            .map_err(|_| MsgBusHandleError::SubscriptionFailed)?;
        let returned_uuid =
            if let Some(ClientMessage::SuccessfulRegistration(uuid)) = rx.recv().await {
                uuid
            } else {
                return Err(MsgBusHandleError::SubscriptionFailed);
            };
        if returned_uuid != response_uuid {
            return Err(MsgBusHandleError::SubscriptionFailed);
        }
        endpoint
            .send(Packet {
                to: u,
                reply_to: Some(response_uuid),
                from: None,
                payload: Payload::BusRider(payload),
            })
            .map_err(MsgBusHandleError::SendError)?;

        match rx.recv().await {
            Some(ClientMessage::Message(val)) => val.payload.reveal().ok_or(
                MsgBusHandleError::ReceiveError("None received unexpectedly".into()),
            ),
            _ => Err(MsgBusHandleError::ReceiveError(
                "None received unexpectedly".into(),
            )),
        }
    }

    pub(crate) fn add_peer_endpoints(&self, uuid: Uuid, ads: HashSet<Advertisement>) {
        _ = self.tx.send(BrokerMsg::AddPeerEndpoints(uuid, ads));
    }

    pub(crate) fn remove_peer_endpoints(&self, peer_id: Uuid, deletes: Vec<Uuid>) {
        _ = self
            .tx
            .send(BrokerMsg::RemovePeerEndpoints(peer_id, deletes));
    }

    #[cfg(feature = "ipc")]
    pub(crate) fn register_peer(&self, uuid: Uuid, tx: UnboundedSender<NodeMessage>) {
        use tracing::debug;

        match self.tx.send(BrokerMsg::RegisterPeer(uuid, tx)) {
            Ok(_) => {}
            Err(e) => debug!("Error sending RegisterPeer packet: {:?}", e),
        }
    }

    #[cfg(feature = "ipc")]
    pub(crate) fn unregister_peer(&self, uuid: Uuid) {
        _ = self.tx.send(BrokerMsg::UnRegisterPeer(uuid));
    }

    pub(crate) fn unregister_endpoint(&self, endpoint_id: Uuid) {
        _ = self.tx.send(BrokerMsg::DeadLink(endpoint_id));
    }

    pub(crate) fn shutdown(&self) {
        let _ = self.tx.send(BrokerMsg::Shutdown);
    }
}

// /// Represents a response to an RPC request.
// #[derive(Debug)]
// pub struct RpcResponse<T>
// where
//     T: BusRiderRpc,
// {
//     response: oneshot::Receiver<Box<dyn BusRider>>,
//     phantom: PhantomData<T>,
// }

// impl<T> RpcResponse<T>
// where
//     T: BusRiderRpc,
// {
//     /// A helper struct that waits for the RPC response and returns it.
//     pub async fn recv(self) -> Result<T::Response, MsgBusHandleError> {
//         let rx = self.response;
//         let payload = rx.await?;
//         let payload: Box<T::Response> = (payload as Box<dyn Any>).downcast().unwrap();
//         if TypeId::of::<T::Response>() == (*payload).type_id() {
//             return Ok(*payload);
//         };
//         Err(MsgBusHandleError::ReceiveError(
//             "TODO the correct error".into(),
//         ))
//         // Err("Bad payload".into())
//     }
// }

// impl<T> RpcResponse<T>
// where
//     T: BusRiderRpc,
// {
//     pub(crate) fn new(response: oneshot::Receiver<Box<dyn BusRider>>) -> RpcResponse<T> {
//         Self {
//             response,
//             phantom: PhantomData::<T>,
//         }
//     }
// }
