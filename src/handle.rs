#[cfg(any(feature = "net", feature = "ipc"))]
use std::collections::HashSet;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::info;
use uuid::Uuid;

use crate::errors::AnyBusHandleError;
use crate::errors::ReceiveError;
#[cfg(any(feature = "net", feature = "ipc"))]
use crate::messages::NodeMessage;
use crate::messages::{BrokerMsg, ClientMessage};
use crate::receivers::{Receiver, RpcReceiver};

use crate::routing::Address;
use crate::routing::router::RoutesWatchRx;
#[cfg(any(feature = "net", feature = "ipc"))]
use crate::routing::{Advertisement, NodeId, WirePacket};
use crate::routing::{EndpointId, Packet, Payload, Route};

use crate::traits::{BusRider, BusRiderRpc, BusRiderWithUuid};

/// The handle for talking to the [AnyBus] instance that created it.  It can be cloned freely
#[derive(Debug, Clone)]
pub struct Handle {
    pub(crate) tx: UnboundedSender<BrokerMsg>,
    pub(crate) route_watch_rx: RoutesWatchRx,
}

impl Handle {
    /// Registers an anycast style of listener to the given Uuid and type T that will return a [BusListener] for receiving
    /// messages sent to the [Uuid].  Anycast allows multiple listeners to be registered and the lowest cost route will
    /// be used to deliver the message.
    pub async fn register_anycast<T: BusRiderWithUuid + DeserializeOwned>(
        &mut self,
    ) -> Result<Receiver<T>, ReceiveError> {
        self.register_anycast_inner(T::ANYBUS_UUID.into()).await
    }

    async fn register_anycast_inner<T: BusRider + DeserializeOwned>(
        &mut self,
        endpoint_id: EndpointId,
    ) -> Result<Receiver<T>, ReceiveError> {
        // let endpoint_id = T::ANYBUS_UUID.into();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let route = Route {
            kind: crate::routing::RouteKind::Anycast,
            #[cfg(any(feature = "net", feature = "ipc"))]
            realm: crate::routing::Realm::Userspace,
            via: crate::routing::ForwardTo::Local(tx.clone()),
            cost: 0,
            #[cfg(any(feature = "net", feature = "ipc"))]
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

    /// Similar to anycast but only one receiver can be registered at a time
    pub async fn register_unicast<T: BusRiderWithUuid + DeserializeOwned>(
        &mut self,
    ) -> Result<Receiver<T>, ReceiveError> {
        self.register_unicast_inner(T::ANYBUS_UUID.into()).await
    }
    /// Same as register_unicast but allows specifying the Uuid to listen on instead of using the one in the [BusRiderWithUuid] trait
    pub async fn register_unicast_uuid<T: BusRider + DeserializeOwned>(
        &mut self,
        endpoint_id: Uuid,
    ) -> Result<Receiver<T>, ReceiveError> {
        self.register_unicast_inner(endpoint_id.into()).await
    }

    async fn register_unicast_inner<T: BusRider + DeserializeOwned>(
        &mut self,
        endpoint_id: EndpointId,
    ) -> Result<Receiver<T>, ReceiveError> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let register_msg = BrokerMsg::RegisterRoute(
            endpoint_id,
            Route {
                kind: crate::routing::RouteKind::Unicast,
                #[cfg(any(feature = "net", feature = "ipc"))]
                realm: crate::routing::Realm::Userspace,
                via: crate::routing::ForwardTo::Local(tx.clone()),
                cost: 0,
                #[cfg(any(feature = "net", feature = "ipc"))]
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
        let endpoint_id = T::ANYBUS_UUID.into();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // let mut receiver = Receiver::<T>::new(endpoint_id, rx, self.clone());

        let register_msg = BrokerMsg::RegisterRoute(
            endpoint_id,
            Route {
                kind: crate::routing::RouteKind::Unicast,
                #[cfg(any(feature = "net", feature = "ipc"))]
                realm: crate::routing::Realm::Userspace,
                via: crate::routing::ForwardTo::Local(tx.clone()),
                cost: 0,
                #[cfg(any(feature = "net", feature = "ipc"))]
                learned_from: Uuid::nil(),
            },
        );
        info!("Send register_msg {:?}", register_msg);

        self.tx.send(register_msg)?;
        self.wait_for_registration(&mut rx, endpoint_id).await?;
        Ok(RpcReceiver::new(endpoint_id, rx, self.clone()))
    }

    /// Placeholder - Not Implemented
    /// Similar to anycast but only one receiver can be registered at a time
    pub async fn register_multicast<T: BusRiderWithUuid + DeserializeOwned>(
        &mut self,
    ) -> Result<Receiver<T>, ReceiveError> {
        let broadcast_id = T::ANYBUS_UUID.into();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let local_id = Uuid::now_v7().into();
        let register_msg = BrokerMsg::RegisterRoute(
            local_id,
            Route {
                kind: crate::routing::RouteKind::Unicast,
                #[cfg(any(feature = "net", feature = "ipc"))]
                realm: crate::routing::Realm::Process,
                via: crate::routing::ForwardTo::Local(tx.clone()),
                cost: 0,
                #[cfg(any(feature = "net", feature = "ipc"))]
                learned_from: Uuid::nil(),
            },
        );
        info!("Send register_msg {:?}", register_msg);

        self.tx.send(register_msg)?;
        self.wait_for_registration(&mut rx, local_id).await?;

        let broadcast_msg = BrokerMsg::RegisterRoute(
            broadcast_id,
            Route {
                kind: crate::routing::RouteKind::Multicast,
                #[cfg(any(feature = "net", feature = "ipc"))]
                realm: crate::routing::Realm::Global,
                via: crate::routing::ForwardTo::Broadcast(HashSet::from([Address::Endpoint(
                    local_id,
                )])),
                cost: 0,
                #[cfg(any(feature = "net", feature = "ipc"))]
                learned_from: Uuid::nil(),
            },
        );
        self.tx.send(broadcast_msg)?;
        self.wait_for_registration(&mut rx, local_id).await?;

        Ok(Receiver::new(local_id, rx, self.clone()))
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

    #[cfg(any(feature = "net", feature = "ipc"))]
    pub(crate) fn send_packet(&self, packet: WirePacket, from_peer: NodeId) {
        let map = self.route_watch_rx.borrow();

        map.forward(packet, from_peer);
    }

    /// Sends a single [BusRider] message to the associated UUID in the trait.
    pub fn send<T: BusRiderWithUuid>(&self, payload: T) -> Result<(), AnyBusHandleError> {
        let address = T::ANYBUS_UUID.into();
        self.send_to_uuid(address, payload)
    }

    /// Sends a single [BusRider] message to the given [Address]
    pub fn send_to_uuid<T: BusRider>(
        &self,
        address: Address,
        payload: T,
    ) -> Result<(), AnyBusHandleError> {
        let map = self.route_watch_rx.borrow();
        // let address = address.into();
        // let endpoint_id = match address {
        //     Address::Endpoint(uuid) => uuid,
        //     Address::Remote(uuid, _uuid1) => uuid,
        // };
        map.send(Packet {
            to: address,
            reply_to: None,
            from: None,
            payload: Payload::BusRider(Box::new(payload) as Box<dyn BusRider>),
        })
        .map_err(AnyBusHandleError::SendError)
    }

    /// Returns a helper that keeps open a response channel for multiple RPC requests
    pub async fn rpc_helper(
        &self,
        // _pd: std::marker::PhantomData<T>,
    ) -> Result<RequestHelper, AnyBusHandleError> {
        let response_uuid = Uuid::now_v7().into();
        // let to_address = T::ANYBUS_UUID.into();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let register_msg = BrokerMsg::RegisterRoute(
            response_uuid,
            Route {
                kind: crate::routing::RouteKind::Unicast,
                #[cfg(any(feature = "net", feature = "ipc"))]
                realm: crate::routing::Realm::Process,
                via: crate::routing::ForwardTo::Local(tx.clone()),
                cost: 0,
                #[cfg(any(feature = "net", feature = "ipc"))]
                learned_from: Uuid::nil(),
            },
        );
        self.tx
            .send(register_msg)
            .map_err(|_| AnyBusHandleError::SubscriptionFailed)?;
        let returned_uuid =
            if let Some(ClientMessage::SuccessfulRegistration(uuid)) = rx.recv().await {
                uuid
            } else {
                return Err(AnyBusHandleError::SubscriptionFailed);
            };
        if returned_uuid != response_uuid {
            return Err(AnyBusHandleError::SubscriptionFailed);
        };
        Ok(RequestHelper::new(
            response_uuid,
            // to_address,
            rx,
            self.clone(),
        ))
    }

    /// A single RPC call that closes the response channel when done
    pub async fn rpc_once<T: BusRiderRpc + BusRiderWithUuid>(
        &self,
        payload: T,
    ) -> Result<T::Response, AnyBusHandleError>
    where
        for<'de> T::Response: serde::de::Deserialize<'de>,
    {
        let mut helper = self.rpc_helper().await?;
        helper.request(payload).await
    }

    // /// Initiate an RPC request to a remote node
    // pub async fn rpc<T: BusRiderRpc + BusRiderWithUuid>(
    //     &self,
    //     payload: T,
    // ) -> Result<T::Response, AnyBusHandleError>
    // where
    //     for<'de> T::Response: serde::de::Deserialize<'de>,
    // {
    //     let address = T::ANYBUS_UUID.into();
    //     let payload = Box::new(payload);
    //     let response_uuid = Uuid::now_v7().into();

    //     let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    //     let register_msg = BrokerMsg::RegisterRoute(
    //         response_uuid,
    //         Route {
    //             kind: crate::routing::RouteKind::Unicast,
    //             #[cfg(any(feature = "net", feature = "ipc"))]
    //             realm: crate::routing::Realm::Process,
    //             via: crate::routing::ForwardTo::Local(tx.clone()),
    //             cost: 0,
    //             #[cfg(any(feature = "net", feature = "ipc"))]
    //             learned_from: Uuid::nil(),
    //         },
    //     );
    //     self.tx
    //         .send(register_msg)
    //         .map_err(|_| AnyBusHandleError::SubscriptionFailed)?;
    //     let returned_uuid =
    //         if let Some(ClientMessage::SuccessfulRegistration(uuid)) = rx.recv().await {
    //             uuid
    //         } else {
    //             return Err(AnyBusHandleError::SubscriptionFailed);
    //         };
    //     if returned_uuid != response_uuid {
    //         return Err(AnyBusHandleError::SubscriptionFailed);
    //     }
    //     // let endpoint = map.lookup(&endpoint_id).ok_or(AnyBusHandleError::NoRoute)?;
    //     let map = self.route_watch_rx.borrow();
    //     let node_id = map.get_node_id();
    //     map.send(Packet {
    //         to: address,
    //         reply_to: Some(Address::Remote(response_uuid.into(), node_id)),
    //         from: None,
    //         payload: Payload::BusRider(payload),
    //     })
    //     .map_err(AnyBusHandleError::SendError)?;
    //     drop(map);
    //     match rx.recv().await {
    //         Some(ClientMessage::Message(val)) => val.payload.reveal().map_err(|p| {
    //             AnyBusHandleError::ReceiveError(ReceiveError::DeserializationError(p))
    //         }),
    //         None => Err(AnyBusHandleError::Shutdown),
    //         _ => todo!(),
    //     }
    // }
    #[cfg(any(feature = "net", feature = "ipc"))]
    pub(crate) fn add_peer_endpoints(&self, uuid: Uuid, ads: HashSet<Advertisement>) {
        _ = self.tx.send(BrokerMsg::AddPeerEndpoints(uuid, ads));
    }
    #[cfg(any(feature = "net", feature = "ipc"))]
    pub(crate) fn remove_peer_endpoints(&self, peer_id: Uuid, deletes: HashSet<Advertisement>) {
        _ = self
            .tx
            .send(BrokerMsg::RemovePeerEndpoints(peer_id, deletes));
    }

    #[cfg(any(feature = "net", feature = "ipc"))]
    pub(crate) fn register_peer(&self, uuid: NodeId, tx: UnboundedSender<NodeMessage>) {
        use tracing::debug;

        match self.tx.send(BrokerMsg::RegisterPeer(uuid, tx)) {
            Ok(_) => {}
            Err(e) => debug!("Error sending RegisterPeer packet: {:?}", e),
        }
    }

    #[cfg(any(feature = "net", feature = "ipc"))]
    pub(crate) fn unregister_peer(&self, uuid: NodeId) {
        _ = self.tx.send(BrokerMsg::UnRegisterPeer(uuid));
    }

    pub(crate) fn unregister_endpoint(&self, endpoint_id: EndpointId) {
        _ = self.tx.send(BrokerMsg::DeadLink(endpoint_id));
    }

    #[cfg(feature = "tokio")]
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
//     pub async fn recv(self) -> Result<T::Response, AnyBusHandleError> {
//         let rx = self.response;
//         let payload = rx.await?;
//         let payload: Box<T::Response> = (payload as Box<dyn Any>).downcast().unwrap();
//         if TypeId::of::<T::Response>() == (*payload).type_id() {
//             return Ok(*payload);
//         };
//         Err(AnyBusHandleError::ReceiveError(
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

pub struct RequestHelper {
    response_endpoint_id: EndpointId,
    // to_address: Address,
    rx: UnboundedReceiver<ClientMessage>,
    handle: Handle,
}

impl RequestHelper {
    fn new(
        response_endpoint_id: EndpointId,
        // to_address: Address,
        rx: UnboundedReceiver<ClientMessage>,
        handle: Handle,
    ) -> Self {
        Self {
            response_endpoint_id,
            // to_address,
            rx,
            handle,
        }
    }

    pub async fn request<T: BusRiderRpc + BusRiderWithUuid>(
        &mut self,
        payload: T,
    ) -> Result<T::Response, AnyBusHandleError>
    where
        for<'de> <T as BusRiderRpc>::Response: Deserialize<'de>,
    {
        let map = self.handle.route_watch_rx.borrow();
        let node_id = map.get_node_id();
        let payload = Box::new(payload);
        let to_address: Address = T::ANYBUS_UUID.into();

        map.send(Packet {
            to: to_address,
            reply_to: Some(Address::Remote(self.response_endpoint_id.into(), node_id)),
            from: None,
            payload: Payload::BusRider(payload),
        })
        .map_err(AnyBusHandleError::SendError)?;
        drop(map);
        match self.rx.recv().await {
            Some(ClientMessage::Message(val)) => val.payload.reveal().map_err(|p| {
                AnyBusHandleError::ReceiveError(ReceiveError::DeserializationError(p))
            }),
            None => Err(AnyBusHandleError::Shutdown),
            _ => todo!(),
        }
    }

    pub async fn request_to_uuid<
        // T: BusRiderRpc + BusRiderWithUuid,
        U: BusRiderRpc,
    >(
        &mut self,
        payload: U,
        endpoint_id: Uuid,
    ) -> Result<U::Response, AnyBusHandleError>
    where
        for<'de> <U as BusRiderRpc>::Response: Deserialize<'de>,
    {
        let map = self.handle.route_watch_rx.borrow();
        let to_address: Address = endpoint_id.into();
        let node_id = map.get_node_id();
        let payload = Box::new(payload);
        // let address: Address = T::ANYBUS_UUID.into();

        map.send(Packet {
            to: to_address,
            reply_to: Some(Address::Remote(self.response_endpoint_id.into(), node_id)),
            from: None,
            payload: Payload::BusRider(payload),
        })
        .map_err(AnyBusHandleError::SendError)?;
        drop(map);
        match self.rx.recv().await {
            Some(ClientMessage::Message(val)) => val.payload.reveal().map_err(|p| {
                AnyBusHandleError::ReceiveError(ReceiveError::DeserializationError(p))
            }),
            None => Err(AnyBusHandleError::Shutdown),
            _ => todo!(),
        }
    }
}
