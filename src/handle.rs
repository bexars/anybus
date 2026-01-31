use std::collections::HashSet;

#[cfg(feature = "serde")]
use serde::Deserialize;
// #[cfg(feature = "serde")]
// use serde::de::DeserializeOwned;
// use std::net::Shutdown;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::info;
use uuid::Uuid;

use crate::BusDeserialize;
use crate::errors::AnyBusHandleError;
use crate::errors::ReceiveError;
// use crate::messages::BrokerMsg;
// #[cfg(any(feature = "net", feature = "ipc"))]
use crate::messages::{BrokerMsg, ClientMessage};
use crate::receivers::Receiver;

use crate::receivers::RpcReceiver;
use crate::routing::Address;
#[cfg(feature = "remote")]
use crate::routing::PeerEntry;
use crate::routing::Realm;
use crate::routing::router::RoutesWatchRx;
#[cfg(feature = "remote")]
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
    /// Registers an anycast style of listener to the given Uuid and type T that will return a [Receiver] for receiving
    /// messages sent to the [Uuid].  Anycast allows multiple listeners to be registered and the lowest cost route will
    /// be used to deliver the message.
    pub async fn register_anycast<T: BusRiderWithUuid + BusDeserialize>(
        &mut self,
    ) -> Result<Receiver<T>, ReceiveError> {
        self.register_anycast_inner(T::ANYBUS_UUID.into(), Realm::Global)
            .await
    }

    /// Same as register_anycast but allows specifying the Uuid to listen on instead of using the one in the [BusRiderWithUuid] trait
    pub async fn register_anycast_uuid<T: BusRider + BusDeserialize>(
        &mut self,
        endpoint_id: Uuid,
    ) -> Result<Receiver<T>, ReceiveError> {
        self.register_anycast_inner(endpoint_id.into(), Realm::Global)
            .await
    }

    async fn register_anycast_inner<T: BusRider + BusDeserialize + for<'de> Deserialize<'de>>(
        &mut self,
        endpoint_id: EndpointId,
        realm: Realm,
    ) -> Result<Receiver<T>, ReceiveError> {
        // let endpoint_id = T::ANYBUS_UUID.into();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let route = Route {
            kind: crate::routing::RouteKind::Anycast,
            #[cfg(feature = "remote")]
            realm,
            via: crate::routing::ForwardTo::Local(tx.clone()),
            cost: 0,
            #[cfg(feature = "remote")]
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
    pub async fn register_unicast<T: BusRiderWithUuid + BusDeserialize>(
        &mut self,
    ) -> Result<Receiver<T>, ReceiveError> {
        self.register_unicast_inner(T::ANYBUS_UUID.into(), Realm::Global)
            .await
    }
    /// Same as register_unicast but allows specifying the Uuid to listen on instead of using the one in the [BusRiderWithUuid] trait
    pub async fn register_unicast_uuid<T: BusRider + BusDeserialize>(
        &mut self,
        endpoint_id: Uuid,
    ) -> Result<Receiver<T>, ReceiveError> {
        self.register_unicast_inner(endpoint_id.into(), Realm::Global)
            .await
    }

    async fn register_unicast_inner<T: BusRider + BusDeserialize>(
        &mut self,
        endpoint_id: EndpointId,
        realm: Realm,
    ) -> Result<Receiver<T>, ReceiveError> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let register_msg = BrokerMsg::RegisterRoute(
            endpoint_id,
            Route {
                kind: crate::routing::RouteKind::Unicast,
                #[cfg(feature = "remote")]
                realm,
                via: crate::routing::ForwardTo::Local(tx.clone()),
                cost: 0,
                #[cfg(feature = "remote")]
                learned_from: Uuid::nil(),
            },
        );
        info!("Send register_msg {:?}", register_msg);

        self.tx.send(register_msg)?;
        self.wait_for_registration(&mut rx, endpoint_id).await?;
        Ok(Receiver::new(endpoint_id, rx, self.clone()))
    }

    /// Register a RPC service with the broker.
    pub async fn register_rpc<T: BusRiderRpc + BusDeserialize + BusRiderWithUuid>(
        &mut self,
    ) -> Result<RpcReceiver<T>, ReceiveError> {
        let endpoint_id = T::ANYBUS_UUID.into();
        self.register_rpc_inner(endpoint_id).await
    }

    /// Register a RPC service with the given Uuid as the endpoint
    pub async fn register_rpc_uuid<T: BusRiderRpc + BusDeserialize>(
        &mut self,
        endpoint_id: Uuid,
    ) -> Result<RpcReceiver<T>, ReceiveError> {
        let endpoint_id = endpoint_id.into();
        self.register_rpc_inner(endpoint_id).await
    }

    async fn register_rpc_inner<T: BusRiderRpc + BusDeserialize>(
        &mut self,
        endpoint_id: EndpointId,
    ) -> Result<RpcReceiver<T>, ReceiveError> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // let mut receiver = Receiver::<T>::new(endpoint_id, rx, self.clone());

        let register_msg = BrokerMsg::RegisterRoute(
            endpoint_id,
            Route {
                kind: crate::routing::RouteKind::Unicast,
                #[cfg(feature = "remote")]
                realm: crate::routing::Realm::Userspace,
                via: crate::routing::ForwardTo::Local(tx.clone()),
                cost: 0,
                #[cfg(feature = "remote")]
                learned_from: Uuid::nil(),
            },
        );
        info!("Send register_msg {:?}", register_msg);

        self.tx.send(register_msg)?;
        self.wait_for_registration(&mut rx, endpoint_id).await?;
        Ok(RpcReceiver::new(endpoint_id, rx, self.clone()))
    }

    /// Broadcast registration, all receivers will get a copy of the message
    pub async fn register_broadcast<T: BusRiderWithUuid + BusDeserialize>(
        &mut self,
    ) -> Result<Receiver<T>, ReceiveError> {
        let broadcast_id = T::ANYBUS_UUID.into();
        self.register_broadcast_inner(broadcast_id, Realm::Global)
            .await
    }

    /// Multicast registration, all receivers will get a copy of the message sent to the given Uuid and type T that will return a [Receiver] for receiving
    pub async fn register_broadcast_uuid<T: BusRider + BusDeserialize>(
        &mut self,
        broadcast_id: Uuid,
    ) -> Result<Receiver<T>, ReceiveError> {
        let broadcast_id = broadcast_id.into();
        self.register_broadcast_inner(broadcast_id, Realm::Global)
            .await
    }
    async fn register_broadcast_inner<T: BusRider + BusDeserialize>(
        &mut self,
        broadcast_id: EndpointId,
        realm: Realm,
    ) -> Result<Receiver<T>, ReceiveError> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let broadcast_msg = BrokerMsg::RegisterRoute(
            broadcast_id,
            Route {
                kind: crate::routing::RouteKind::Broadcast,
                #[cfg(feature = "remote")]
                realm,
                via: crate::routing::ForwardTo::Broadcast(vec![tx], Realm::Global),

                cost: 0,
                #[cfg(feature = "remote")]
                learned_from: Uuid::nil(),
            },
        );
        self.tx.send(broadcast_msg)?;
        self.wait_for_registration(&mut rx, broadcast_id).await?;

        Ok(Receiver::new(broadcast_id, rx, self.clone()))
    }

    // Multicast is broken right now, removing it until fixed.
    /// Multicast registration, all receivers will get a copy of the message
    #[allow(dead_code)]
    pub(crate) async fn register_multicast<T: BusRiderWithUuid + BusDeserialize>(
        &mut self,
    ) -> Result<Receiver<T>, ReceiveError> {
        let broadcast_id = T::ANYBUS_UUID.into();
        self.register_multicast_inner(broadcast_id).await
    }

    /// Multicast registration, all receivers will get a copy of the message sent to the given Uuid and type T that will return a [Receiver] for receiving
    #[allow(dead_code)]

    pub(crate) async fn register_multicast_uuid<T: BusRider + BusDeserialize>(
        &mut self,
        broadcast_id: Uuid,
    ) -> Result<Receiver<T>, ReceiveError> {
        let broadcast_id = broadcast_id.into();
        self.register_multicast_inner(broadcast_id).await
    }
    async fn register_multicast_inner<T: BusRider + BusDeserialize>(
        &mut self,
        broadcast_id: EndpointId,
    ) -> Result<Receiver<T>, ReceiveError> {
        // let broadcast_id = T::ANYBUS_UUID.into();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let local_id = Uuid::now_v7().into();
        let register_msg = BrokerMsg::RegisterRoute(
            local_id,
            Route {
                kind: crate::routing::RouteKind::Unicast,
                #[cfg(feature = "remote")]
                realm: crate::routing::Realm::Process,
                via: crate::routing::ForwardTo::Local(tx.clone()),
                cost: 0,
                #[cfg(feature = "remote")]
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
                #[cfg(feature = "remote")]
                realm: crate::routing::Realm::Global,
                via: crate::routing::ForwardTo::Multicast(HashSet::from([Address::Endpoint(
                    local_id,
                )])),
                cost: 0,
                #[cfg(feature = "remote")]
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

    #[cfg(feature = "remote")]
    pub(crate) fn send_packet(&self, packet: WirePacket, from_peer: NodeId) {
        let map = self.route_watch_rx.borrow();

        map.forward(packet, from_peer);
    }

    /// Sends a single [BusRider] message to the associated UUID in the trait.
    pub fn send<T: BusRiderWithUuid>(&self, payload: T) -> Result<(), AnyBusHandleError> {
        let address = T::ANYBUS_UUID.into();
        self.send_to_address(address, payload)
    }

    /// Sends a single [BusRider] message to the given [Uuid]

    pub fn send_to_uuid<T: BusRider>(
        &self,
        address: Uuid,
        payload: T,
    ) -> Result<(), AnyBusHandleError> {
        let address = address.into();
        self.send_to_address(address, payload)
    }

    /// Sends a single [BusRider] message to the given [Address]
    pub(crate) fn send_to_address<T: BusRider>(
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
                #[cfg(feature = "remote")]
                realm: crate::routing::Realm::Process,
                via: crate::routing::ForwardTo::Local(tx.clone()),
                cost: 0,
                #[cfg(feature = "remote")]
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
        for<'de> T::Response: BusDeserialize,
        // for<'de> T::Response: serde::de::Deserialize<'de>,
    {
        let mut helper = self.rpc_helper().await?;
        helper.request(payload).await
    }

    #[cfg(feature = "remote")]
    pub(crate) fn add_peer_endpoints(&self, uuid: Uuid, ads: HashSet<Advertisement>) {
        _ = self.tx.send(BrokerMsg::AddPeerEndpoints(uuid, ads));
    }
    #[cfg(feature = "remote")]
    pub(crate) fn remove_peer_endpoints(&self, peer_id: Uuid, deletes: HashSet<Advertisement>) {
        _ = self
            .tx
            .send(BrokerMsg::RemovePeerEndpoints(peer_id, deletes));
    }

    #[cfg(feature = "remote")]
    pub(crate) fn register_peer(
        &self,
        uuid: NodeId,
        peer_entry: PeerEntry,
        // tx: UnboundedSender<NodeMessage>,
        // realm: Realm,
    ) {
        use tracing::debug;

        match self.tx.send(BrokerMsg::RegisterPeer(uuid, peer_entry)) {
            Ok(_) => {}
            Err(e) => debug!("Error sending RegisterPeer packet: {:?}", e),
        }
    }

    #[cfg(feature = "remote")]
    pub(crate) fn unregister_peer(&self, uuid: NodeId) {
        _ = self.tx.send(BrokerMsg::UnRegisterPeer(uuid));
    }

    pub(crate) fn unregister_endpoint(&self, endpoint_id: EndpointId) {
        _ = self.tx.send(BrokerMsg::DeadLink(endpoint_id));
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn shutdown(&self) {
        let _ = self.tx.send(BrokerMsg::Shutdown);
    }

    pub(crate) fn send_broker_msg(&self, msg: BrokerMsg) -> Option<()> {
        self.tx.send(msg).ok()
    }

    /// Start building a registration with the builder pattern
    pub fn listener(&self) -> RegistrationBuilder<NoEndpointId, NoCast, NoRpc> {
        RegistrationBuilder {
            endpoint_id: NoEndpointId,
            realm: Realm::default(),
            cast: NoCast,
            rpc_flag: NoRpc,
            handle: self.clone(),
            cost: 0,
        }
    }
}

/// A helper struct that keeps a response channel open for multiple RPC requests
#[derive(Debug)]
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

    /// Request using an object with [BusRiderWithUuid] implemented
    pub async fn request<T: BusRiderRpc + BusRiderWithUuid>(
        &mut self,
        payload: T,
    ) -> Result<T::Response, AnyBusHandleError>
    where
        for<'de> <T as BusRiderRpc>::Response: BusDeserialize,
        // for<'de> <T as BusRiderRpc>::Response: Deserialize<'de>,
    {
        // let map = self.handle.route_watch_rx.borrow();
        let node_id = self.handle.route_watch_rx.borrow().get_node_id();
        let payload = Box::new(payload);
        let to_address: Address = T::ANYBUS_UUID.into();

        self.handle
            .route_watch_rx
            .borrow()
            .send(Packet {
                to: to_address,
                reply_to: Some(Address::Remote(self.response_endpoint_id.into(), node_id)),
                from: None,
                payload: Payload::BusRider(payload),
            })
            .map_err(AnyBusHandleError::SendError)?;
        // drop(map);
        match self.rx.recv().await {
            Some(ClientMessage::Message(val)) => val.payload.reveal().map_err(|p| {
                AnyBusHandleError::ReceiveError(ReceiveError::DeserializationError(p))
            }),
            None => Err(AnyBusHandleError::Shutdown),
            _ => todo!(),
        }
    }

    /// A request using user provided Uuid
    pub async fn request_to_uuid<
        // T: BusRiderRpc + BusRiderWithUuid,
        U: BusRiderRpc,
    >(
        &mut self,
        payload: U,
        endpoint_id: Uuid,
    ) -> Result<U::Response, AnyBusHandleError>
    where
        for<'de> <U as BusRiderRpc>::Response: BusDeserialize,
        // for<'de> <U as BusRiderRpc>::Response: Deserialize<'de>,
    {
        // let map = self.handle.route_watch_rx.borrow();
        let to_address: Address = endpoint_id.into();
        let node_id = self.handle.route_watch_rx.borrow().get_node_id();
        let payload = Box::new(payload);
        // let address: Address = T::ANYBUS_UUID.into();
        // println!("Payload: {:?}", payload);
        // println!("To: {}", to_address);
        self.handle
            .route_watch_rx
            .borrow()
            .send(Packet {
                to: to_address,
                reply_to: Some(Address::Remote(self.response_endpoint_id.into(), node_id)),
                from: None,
                payload: Payload::BusRider(payload),
            })
            .map_err(AnyBusHandleError::SendError)?;
        // println!("Packet sent from inside rpc_helper");
        // drop(map);
        match self.rx.recv().await {
            Some(ClientMessage::Message(val)) => val.payload.reveal().map_err(|p| {
                AnyBusHandleError::ReceiveError(ReceiveError::DeserializationError(p))
            }),
            None => Err(AnyBusHandleError::Shutdown),
            Some(ClientMessage::Shutdown) => Err(AnyBusHandleError::Shutdown),
            _ => todo!(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RegistrationBuilder<EP, CAST, RPC> {
    endpoint_id: EP,
    realm: Realm,
    cast: CAST,
    rpc_flag: RPC,
    cost: u16,
    handle: Handle,
}

pub struct NoEndpointId;
pub struct NoCast;
pub struct EndpointSet(EndpointId);
pub struct CastSet(crate::routing::RouteKind);
pub struct NoRpc;
pub struct RpcSet;

// impl Default for RegistrationBuilder<NoEndpointId, NoCast, NoRpc> {
//     fn default() -> Self {
//         Self {
//             endpoint_id: NoEndpointId,
//             realm: Realm::default(),
//             cast: NoCast,
//             rpc: NoRpc,
//         }
//     }
// }

impl<EP, CAST, RPC> RegistrationBuilder<EP, CAST, RPC> {
    /// Set the realm for this registration, defaults to Realm::Global
    pub fn realm(mut self, realm: Realm) -> Self {
        self.realm = realm;
        self
    }

    pub fn endpoint(self, ep: EndpointId) -> RegistrationBuilder<EndpointSet, CAST, RPC> {
        RegistrationBuilder {
            endpoint_id: EndpointSet(ep),
            realm: self.realm,
            cast: self.cast,
            rpc_flag: self.rpc_flag,
            handle: self.handle,
            cost: self.cost,
        }
    }
}
impl<EP, RPC> RegistrationBuilder<EP, NoCast, RPC> {
    pub fn anycast(self) -> RegistrationBuilder<EP, CastSet, RPC> {
        RegistrationBuilder {
            endpoint_id: self.endpoint_id,
            realm: self.realm,
            cast: CastSet(crate::routing::RouteKind::Anycast),
            rpc_flag: self.rpc_flag,
            handle: self.handle,
            cost: self.cost,
        }
    }

    pub fn unicast(self) -> RegistrationBuilder<EP, CastSet, RPC> {
        RegistrationBuilder {
            endpoint_id: self.endpoint_id,
            realm: self.realm,
            cast: CastSet(crate::routing::RouteKind::Unicast),
            rpc_flag: self.rpc_flag,
            handle: self.handle,
            cost: self.cost,
        }
    }
}
impl<EP> RegistrationBuilder<EP, NoCast, NoRpc> {
    pub fn broadcast(self) -> RegistrationBuilder<EP, CastSet, NoRpc> {
        RegistrationBuilder {
            endpoint_id: self.endpoint_id,
            realm: self.realm,
            cast: CastSet(crate::routing::RouteKind::Broadcast),
            rpc_flag: self.rpc_flag,
            handle: self.handle,
            cost: self.cost,
        }
    }
}

impl<EP> RegistrationBuilder<EP, NoCast, NoRpc> {
    pub fn rpc(self) -> RegistrationBuilder<EP, NoCast, RpcSet> {
        RegistrationBuilder {
            endpoint_id: self.endpoint_id,
            realm: self.realm,
            cast: NoCast,
            rpc_flag: RpcSet,
            handle: self.handle,
            cost: self.cost,
        }
    }
}

impl RegistrationBuilder<EndpointSet, CastSet, NoRpc> {
    /// Finalize the registration and get a [Receiver] for the messages
    pub async fn register<T: BusRider + BusDeserialize>(
        mut self,
    ) -> Result<Receiver<T>, ReceiveError> {
        match self.cast.0 {
            crate::routing::RouteKind::Anycast => {
                self.handle
                    .register_anycast_inner::<T>(self.endpoint_id.0.into(), self.realm)
                    .await
            }
            crate::routing::RouteKind::Unicast => {
                self.handle
                    .register_unicast_inner::<T>(self.endpoint_id.0.into(), self.realm)
                    .await
            }
            crate::routing::RouteKind::Broadcast => {
                self.handle
                    .register_broadcast_inner::<T>(self.endpoint_id.0.into(), self.realm)
                    .await
            }
            crate::routing::RouteKind::Multicast => unimplemented!(),
            crate::routing::RouteKind::Node => unimplemented!(),
        }
    }
}

impl<CAST> RegistrationBuilder<NoEndpointId, CAST, RpcSet> {
    /// Finalize the registration and get a [RpcReceiver] for the messages.
    pub async fn register<T: BusRiderRpc + BusDeserialize + BusRiderWithUuid>(
        mut self,
    ) -> Result<RpcReceiver<T>, ReceiveError> {
        let ep = T::ANYBUS_UUID.into();
        // let config = self.create_config(&self, ep);
        self.handle.register_rpc_inner::<T>(ep).await
    }
}

impl<CAST> RegistrationBuilder<EndpointSet, CAST, RpcSet> {
    /// Finalize the registration and get a [RpcReceiver] for the messages
    pub async fn register<T: BusRiderRpc + BusDeserialize>(
        mut self,
    ) -> Result<RpcReceiver<T>, ReceiveError> {
        self.handle
            .register_rpc_inner::<T>(self.endpoint_id.0.into())
            .await
    }
}

impl RegistrationBuilder<NoEndpointId, CastSet, NoRpc> {
    /// Finalize the registration and get a [Receiver] for the messages
    pub async fn register<T: BusRider + BusDeserialize + BusRiderWithUuid>(
        mut self,
    ) -> Result<Receiver<T>, ReceiveError> {
        let ep = T::ANYBUS_UUID.into();
        let _config = self.create_config(&self, ep);
        match self.cast.0 {
            crate::routing::RouteKind::Anycast => {
                self.handle
                    .register_anycast_inner::<T>(ep, self.realm)
                    .await
            }
            crate::routing::RouteKind::Unicast => {
                self.handle
                    .register_unicast_inner::<T>(ep, self.realm)
                    .await
            }
            crate::routing::RouteKind::Broadcast => {
                self.handle
                    .register_broadcast_inner::<T>(ep, self.realm)
                    .await
            }
            crate::routing::RouteKind::Multicast => unimplemented!(),
            crate::routing::RouteKind::Node => unimplemented!(),
        }
    }
}

impl RegistrationBuilder<NoEndpointId, CastSet, NoRpc> {
    fn create_config(
        &self,
        builder: &RegistrationBuilder<NoEndpointId, CastSet, NoRpc>,
        ep: EndpointId,
    ) -> RegistrationConfig {
        RegistrationConfig {
            endpoint_id: ep,
            realm: builder.realm,
            cast: builder.cast.0,
            rpc: false,
            cost: builder.cost,
        }
    }
}
impl RegistrationBuilder<NoEndpointId, CastSet, RpcSet> {
    pub fn create_config(
        &self,
        builder: &RegistrationBuilder<NoEndpointId, CastSet, RpcSet>,
        ep: EndpointId,
    ) -> RegistrationConfig {
        RegistrationConfig {
            endpoint_id: ep,
            realm: builder.realm,
            cast: builder.cast.0,
            rpc: false,
            cost: builder.cost,
        }
    }
}

pub struct RegistrationConfig {
    pub endpoint_id: EndpointId,
    pub realm: Realm,
    pub cast: crate::routing::RouteKind,
    pub rpc: bool,
    pub cost: u16,
}

impl From<RegistrationBuilder<EndpointSet, CastSet, RpcSet>> for RegistrationConfig {
    fn from(builder: RegistrationBuilder<EndpointSet, CastSet, RpcSet>) -> Self {
        Self {
            endpoint_id: builder.endpoint_id.0,
            realm: builder.realm,
            cast: builder.cast.0,
            rpc: true,
            cost: builder.cost,
        }
    }
}

impl From<RegistrationBuilder<EndpointSet, CastSet, NoRpc>> for RegistrationConfig {
    fn from(builder: RegistrationBuilder<EndpointSet, CastSet, NoRpc>) -> Self {
        Self {
            endpoint_id: builder.endpoint_id.0,
            realm: builder.realm,
            cast: builder.cast.0,
            rpc: false,
            cost: builder.cost,
        }
    }
}
