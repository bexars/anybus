use std::collections::HashSet;
use std::{error::Error, marker::PhantomData};

use serde::de::DeserializeOwned;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::info;
use uuid::Uuid;

use crate::errors::MsgBusHandleError;
use crate::errors::ReceiveError;
#[cfg(feature = "ipc")]
use crate::messages::NodeMessage;
use crate::messages::{BrokerMsg, ClientMessage, RegistrationStatus};
use crate::receivers::{Receiver, RpcReceiver};
#[cfg(feature = "ipc")]
use crate::routing::NodeId;
use crate::routing::router::RoutesWatchRx;
use crate::routing::{Address, Advertisement, EndpointId, Packet, Payload, Route, WirePacket};

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
    ) -> Result<Receiver<T>, ReceiveError> {
        let endpoint_id = T::MSGBUS_UUID.into();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let route = Route {
            kind: crate::routing::RouteKind::Anycast,
            realm: crate::routing::Realm::Userspace,
            via: crate::routing::ForwardTo::Local(tx.clone()),
            cost: 0,
            learned_from: Uuid::nil(),
        };

        let register_msg = BrokerMsg::RegisterRoute(endpoint_id, route);
        self.tx.send(register_msg)?;
        info!("Sent register_msg");
        self.wait_for_registration(&mut rx, endpoint_id).await?;
        return Ok(crate::receivers::Receiver::new(
            endpoint_id,
            rx,
            self.clone(),
        ));
    }

    async fn wait_for_registration(
        &self,
        rx: &mut UnboundedReceiver<ClientMessage>,
        endpoint_id: EndpointId,
    ) -> Result<(), ReceiveError> {
        let registration_response = if let Some(msg) = rx.recv().await {
            msg
        } else {
            return Err(ReceiveError::Shutdown);
        };

        match registration_response {
            ClientMessage::Message(_packet) => {
                drop(rx);
                _ = self.tx.send(BrokerMsg::DeadLink(endpoint_id));
                Err(ReceiveError::RegistrationFailed(
                    "Bad response from Bus".into(),
                ))
            }
            ClientMessage::FailedRegistration(_uuid, reason) => {
                Err(ReceiveError::RegistrationFailed(reason))
            }
            ClientMessage::Shutdown => Err(ReceiveError::Shutdown),
            ClientMessage::SuccessfulRegistration(_uuid) => Ok(()),
        }
    }

    /// Similar to anycast but only one receiver can be registered at a time
    pub async fn register_unicast<T: BusRiderWithUuid + DeserializeOwned>(
        &mut self,
    ) -> Result<Receiver<T>, ReceiveError> {
        let endpoint_id = T::MSGBUS_UUID.into();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let register_msg = BrokerMsg::RegisterRoute(
            endpoint_id,
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
        self.wait_for_registration(&mut rx, endpoint_id).await?;
        Ok(Receiver::new(endpoint_id, rx, self.clone()))
    }

    /// Register a RPC service with the broker.
    pub async fn register_rpc<T: BusRiderRpc + DeserializeOwned + BusRiderWithUuid>(
        &mut self,
    ) -> Result<RpcReceiver<T>, ReceiveError> {
        let endpoint_id = T::MSGBUS_UUID.into();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // let mut receiver = Receiver::<T>::new(endpoint_id, rx, self.clone());

        let register_msg = BrokerMsg::RegisterRoute(
            endpoint_id,
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
        self.wait_for_registration(&mut rx, endpoint_id).await?;
        Ok(RpcReceiver::new(endpoint_id, rx, self.clone()))
    }

    /// Placeholder - Not Implemented
    fn _register_multicast<T: BusRider + DeserializeOwned>(
        &mut self,
        _uuid: Uuid,
    ) -> Result<Receiver<T>, Box<dyn Error>> {
        todo!()
    }

    pub(crate) fn send_packet(&self, packet: WirePacket) {
        let map = self.route_watch_rx.borrow();

        _ = map.send(packet);
    }

    /// Sends a single [BusRider] message to the associated UUID in the trait.
    pub fn send<T: BusRiderWithUuid>(&self, payload: T) -> Result<(), MsgBusHandleError> {
        let endpoint_id: EndpointId = T::MSGBUS_UUID.into();
        self.send_to_uuid(endpoint_id, payload)
    }

    /// Sends a single [BusRider] message to the given [Address]
    pub fn send_to_uuid<T: BusRider>(
        &self,
        endpoint_id: EndpointId,
        payload: T,
    ) -> Result<(), MsgBusHandleError> {
        let map = self.route_watch_rx.borrow();
        // let address = address.into();
        // let endpoint_id = match address {
        //     Address::Endpoint(uuid) => uuid,
        //     Address::Remote(uuid, _uuid1) => uuid,
        // };
        map.send(Packet {
            to: endpoint_id,
            reply_to: None,
            from: None,
            payload: Payload::BusRider(Box::new(payload) as Box<dyn BusRider>),
        })
        .map_err(MsgBusHandleError::SendError)
    }

    /// Initiate an RPC request to a remote node
    pub async fn request<T: BusRiderRpc + BusRiderWithUuid>(
        &self,
        payload: T,
    ) -> Result<T::Response, MsgBusHandleError>
    where
        for<'de> T::Response: serde::de::Deserialize<'de>,
    {
        let endpoint_id = T::MSGBUS_UUID.into();
        let payload = Box::new(payload);
        let map = self.route_watch_rx.borrow();
        let response_uuid = Uuid::now_v7().into();

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
        // let endpoint = map.lookup(&endpoint_id).ok_or(MsgBusHandleError::NoRoute)?;

        map.send(Packet {
            to: endpoint_id,
            reply_to: Some(response_uuid),
            from: None,
            payload: Payload::BusRider(payload),
        })
        .map_err(MsgBusHandleError::SendError)?;

        match rx.recv().await {
            Some(ClientMessage::Message(val)) => val.payload.reveal().map_err(|p| {
                MsgBusHandleError::ReceiveError(ReceiveError::DeserializationError(p))
            }),
            None => Err(MsgBusHandleError::Shutdown),
            _ => todo!(),
        }
    }

    pub(crate) fn add_peer_endpoints(&self, uuid: Uuid, ads: HashSet<Advertisement>) {
        _ = self.tx.send(BrokerMsg::AddPeerEndpoints(uuid, ads));
    }

    pub(crate) fn remove_peer_endpoints(&self, peer_id: Uuid, deletes: HashSet<Advertisement>) {
        _ = self
            .tx
            .send(BrokerMsg::RemovePeerEndpoints(peer_id, deletes));
    }

    #[cfg(feature = "ipc")]
    pub(crate) fn register_peer(&self, uuid: NodeId, tx: UnboundedSender<NodeMessage>) {
        use tracing::debug;

        match self.tx.send(BrokerMsg::RegisterPeer(uuid, tx)) {
            Ok(_) => {}
            Err(e) => debug!("Error sending RegisterPeer packet: {:?}", e),
        }
    }

    #[cfg(feature = "ipc")]
    pub(crate) fn unregister_peer(&self, uuid: NodeId) {
        _ = self.tx.send(BrokerMsg::UnRegisterPeer(uuid));
    }

    pub(crate) fn unregister_endpoint(&self, endpoint_id: EndpointId) {
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
