use crate::tokio;
use arc_swap::ArcSwap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
// use tokio_with_wasm::alias as tokio;

use tracing::info;
// use uuid::Uuid;

use crate::BusDeserialize;
use crate::BusTicket;
use crate::Realm;
use crate::errors::AnyBusHandleError;
use crate::errors::ReceiveError;
use crate::messages::AnyBusStatusMsg;

use crate::messages::RouterMsg::RegisterEndpoint;
use crate::messages::{ClientMessage, RouterMsg};
use crate::receivers::Receiver;

use crate::receivers::RpcReceiver;
use crate::routing::Address;

#[cfg(feature = "remote")]
use crate::routing::ConnectionId;
use crate::routing::ForwardingTable;
#[cfg(feature = "remote")]
use crate::routing::WirePacket;
use crate::routing::{EndpointId, Packet, Payload, Route};

use crate::spawn;
use crate::traits::{BusRider, BusRiderRpc, BusRiderWithUuid};

/// The handle for talking to the [AnyBus] instance that created it.  It can be cloned freely
#[derive(Debug, Clone)]
pub struct Handle {
    pub(crate) tx: mpsc::Sender<RouterMsg>,
    pub(crate) fib: Arc<ArcSwap<ForwardingTable>>,
}

impl Handle {
    pub(crate) fn shutdown(&self, delay: Option<Duration>) {
        info!("Router shutting down");
        self.send(AnyBusStatusMsg::ShuttingDown).ok();
        if let Some(delay) = delay {
            std::thread::sleep(delay)
        };

        self.send_broker(RouterMsg::Shutdown);
    }

    /// Convenience function to register_broadcast::<AnyBusStatusMsg>
    /// This will be updated with state changes known to the AnyBus system.  See [AnyBusStatusMsg] for details
    pub async fn get_anybus_status_receiver(
        &self,
    ) -> Result<Receiver<AnyBusStatusMsg>, ReceiveError> {
        self.listener()
            .endpoint(AnyBusStatusMsg::ANYBUS_UUID.into())
            .broadcast()
            .realm(Realm::Process)
            .register::<AnyBusStatusMsg>()
            .await
    }

    /// Registers an anycast style of listener to the given Uuid and type T that will return a [Receiver] for receiving
    /// messages sent to the [Uuid].  Anycast allows multiple listeners to be registered and the lowest cost route will
    /// be used to deliver the message.
    pub async fn register_anycast<T: BusRiderWithUuid + BusDeserialize>(
        &self,
    ) -> Result<Receiver<T>, ReceiveError> {
        self.register_anycast_inner(T::ANYBUS_UUID.into(), Realm::Global)
            .await
    }

    /// Same as register_anycast but allows specifying the Uuid to listen on instead of using the one in the [BusRiderWithUuid] trait
    pub async fn register_anycast_uuid<T: BusRider + BusDeserialize>(
        &self,
        endpoint_id: impl Into<EndpointId>,
    ) -> Result<Receiver<T>, ReceiveError> {
        self.register_anycast_inner(endpoint_id.into(), Realm::Global)
            .await
    }

    #[allow(unused_variables)]
    async fn register_anycast_inner<T: BusRider + BusDeserialize>(
        &self,
        endpoint_id: EndpointId,
        realm: Realm,
    ) -> Result<Receiver<T>, ReceiveError> {
        // let endpoint_id = T::ANYBUS_UUID.into();
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        let route = Route {
            kind: crate::routing::RouteKind::Anycast,
            realm,
            _via: crate::routing::ForwardTo::Local(tx.clone()),
            cost: 0.into(),
            #[cfg(feature = "remote")]
            _learned_from: 0.into(),
        };

        let ei = (&route).into();

        let register_msg = RouterMsg::RegisterEndpoint(endpoint_id, ei, tx);
        info!("About to send register_msg");
        self.tx.send(register_msg).await?;
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
        &self,
    ) -> Result<Receiver<T>, ReceiveError> {
        self.register_unicast_inner(T::ANYBUS_UUID.into(), Realm::Global)
            .await
    }
    /// Same as register_unicast but allows specifying the Uuid to listen on instead of using the one in the [BusRiderWithUuid] trait
    pub async fn register_unicast_uuid<T: BusRider + BusDeserialize>(
        &self,
        endpoint_id: impl Into<EndpointId>,
    ) -> Result<Receiver<T>, ReceiveError> {
        self.register_unicast_inner(endpoint_id.into(), Realm::Global)
            .await
    }

    #[allow(unused_variables)]
    async fn register_unicast_inner<T: BusRider + BusDeserialize>(
        &self,
        endpoint_id: EndpointId,
        realm: Realm,
    ) -> Result<Receiver<T>, ReceiveError> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let route = Route {
            kind: crate::routing::RouteKind::Unicast,
            realm,
            _via: crate::routing::ForwardTo::Local(tx.clone()),
            cost: 0.into(),
            #[cfg(feature = "remote")]
            _learned_from: 0.into(),
        };
        let ei = (&route).into();

        let register_msg = RouterMsg::RegisterEndpoint(endpoint_id, ei, tx);

        info!("Send register_msg {:?}", register_msg);

        self.tx.send(register_msg).await?;
        self.wait_for_registration(&mut rx, endpoint_id).await?;
        Ok(Receiver::new(endpoint_id, rx, self.clone()))
    }

    /// Register a RPC service with the broker.
    pub async fn register_rpc<T: BusRiderRpc + BusDeserialize + BusRiderWithUuid>(
        &self,
    ) -> Result<RpcReceiver<T>, ReceiveError> {
        let endpoint_id = T::ANYBUS_UUID.into();
        self.register_rpc_inner(endpoint_id).await
    }

    /// Register a RPC service with the given Uuid as the endpoint
    pub async fn register_rpc_uuid<T: BusRiderRpc + BusDeserialize>(
        &self,
        endpoint_id: impl Into<EndpointId>,
    ) -> Result<RpcReceiver<T>, ReceiveError> {
        let endpoint_id = endpoint_id.into();
        self.register_rpc_inner(endpoint_id).await
    }

    async fn register_rpc_inner<T: BusRiderRpc + BusDeserialize>(
        &self,
        endpoint_id: EndpointId,
    ) -> Result<RpcReceiver<T>, ReceiveError> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        // let mut receiver = Receiver::<T>::new(endpoint_id, rx, self.clone());

        let route = Route {
            kind: crate::routing::RouteKind::Unicast,
            realm: Realm::default(),
            _via: crate::routing::ForwardTo::Local(tx.clone()),
            cost: 0.into(),
            #[cfg(feature = "remote")]
            _learned_from: 0.into(),
        };

        let ei = (&route).into();
        let register_msg = RouterMsg::RegisterEndpoint(endpoint_id, ei, tx);

        info!("Send register_msg {:?}", register_msg);

        self.tx.send(register_msg).await?;
        self.wait_for_registration(&mut rx, endpoint_id).await?;
        Ok(RpcReceiver::new(endpoint_id, rx, self.clone()))
    }

    /// Broadcast registration, all receivers will get a copy of the message
    pub async fn register_broadcast<T: BusRiderWithUuid + BusDeserialize>(
        &self,
    ) -> Result<Receiver<T>, ReceiveError> {
        let broadcast_id = T::ANYBUS_UUID.into();
        self.register_broadcast_inner(broadcast_id, Realm::Global)
            .await
    }

    /// Multicast registration, all receivers will get a copy of the message sent to the given Uuid and type T that will return a [Receiver] for receiving
    pub async fn register_broadcast_uuid<T: BusRider + BusDeserialize>(
        &self,
        broadcast_id: impl Into<EndpointId>,
    ) -> Result<Receiver<T>, ReceiveError> {
        let broadcast_id = broadcast_id.into();
        self.register_broadcast_inner(broadcast_id, Realm::Global)
            .await
    }
    #[allow(unused_variables)]
    async fn register_broadcast_inner<T: BusRider + BusDeserialize>(
        &self,
        broadcast_id: EndpointId,
        realm: Realm,
    ) -> Result<Receiver<T>, ReceiveError> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        let route = Route {
            kind: crate::routing::RouteKind::Broadcast,
            realm,
            _via: crate::routing::ForwardTo::Broadcast(vec![tx.clone()], realm),

            cost: 0.into(),
            #[cfg(feature = "remote")]
            _learned_from: 0.into(),
        };
        let ei = (&route).into();

        let broadcast_msg = RegisterEndpoint(broadcast_id, ei, tx);
        self.tx.send(broadcast_msg).await?;
        self.wait_for_registration(&mut rx, broadcast_id).await?;

        Ok(Receiver::new(broadcast_id, rx, self.clone()))
    }

    async fn wait_for_registration(
        &self,
        rx: &mut mpsc::Receiver<ClientMessage>,
        endpoint_id: EndpointId,
    ) -> Result<(), ReceiveError> {
        let registration_response = if let Some(msg) = rx.recv().await {
            msg
        } else {
            return Err(ReceiveError::Shutdown);
        };

        match registration_response {
            ClientMessage::Message(_packet) => {
                _ = self.send_broker(RouterMsg::DeadLink(endpoint_id));
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
    pub(crate) fn forward_packet(&self, packet: WirePacket, connection_id: ConnectionId) {
        let map = self.fib.load();
        map.forward(packet, connection_id);
    }

    /// Sends a single [BusRider] message to the associated UUID in the trait.
    pub fn send<T: BusRiderWithUuid>(&self, payload: T) -> Result<(), AnyBusHandleError> {
        let address = T::ANYBUS_UUID.into();
        self.send_to_address(address, payload)
    }

    /// Sends a single [BusRider] message to the given [Uuid]

    pub fn send_to_uuid<T: BusRider>(
        &self,
        address: impl Into<EndpointId>,
        payload: T,
    ) -> Result<(), AnyBusHandleError> {
        let address: EndpointId = address.into();
        self.send_to_address(address.into(), payload)
    }

    /// Sends a single [BusRider] message to the given [Address]
    pub(crate) fn send_to_address<T: BusRider>(
        &self,
        address: Address,
        payload: T,
    ) -> Result<(), AnyBusHandleError> {
        let map = self.fib.load();
        map.send(Packet {
            to: address,
            reply_to: None,
            from: map.our_id,
            payload: Payload::BusRider(Box::new(payload) as Box<dyn BusRider>),
        })
        .map_err(AnyBusHandleError::SendError)
    }

    /// Sends a single BusTicket ( a wrapper around a BusRider and Destination)
    pub fn send_busticket(&self, ticket: BusTicket) -> Result<(), AnyBusHandleError> {
        let map = self.fib.load();
        map.send(Packet {
            to: ticket.dest.into(),
            reply_to: None,
            from: map.our_id,
            payload: Payload::BusRider(ticket.rider as Box<dyn BusRider>),
        })
        .map_err(AnyBusHandleError::SendError)
    }

    /// Returns a helper that keeps open a response channel for multiple RPC requests
    pub async fn rpc_helper(
        &self,
        // _pd: std::marker::PhantomData<T>,
    ) -> Result<RequestHelper, AnyBusHandleError> {
        let response_uuid = EndpointId::new().into();
        // let to_address = T::ANYBUS_UUID.into();

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        let route = Route {
            kind: crate::routing::RouteKind::Unicast,
            realm: Realm::Process,
            _via: crate::routing::ForwardTo::Local(tx.clone()),
            cost: 0.into(),
            #[cfg(feature = "remote")]
            _learned_from: 0.into(),
        };

        let ei = (&route).into();

        let register_msg = RouterMsg::RegisterEndpoint(response_uuid, ei, tx);
        self.send_broker(register_msg);
        // .map_err(|_| AnyBusHandleError::SubscriptionFailed)?;
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
        T::Response: BusDeserialize,
    {
        let mut helper = self.rpc_helper().await?;
        helper.request(payload).await
    }

    pub(crate) fn unregister_endpoint(&self, endpoint_id: EndpointId) {
        self.send_broker(RouterMsg::DeadLink(endpoint_id));
    }

    /// Allows internal communication to the Router
    pub(crate) fn send_broker(&self, msg: RouterMsg) {
        // self.tx.try_send(msg).ok();

        let tx = self.tx.clone();
        spawn(async move {
            if let Err(e) = tx.send(msg).await {
                tracing::warn!("Failed to send broker message: {}", &e);
            }
        });
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
    rx: mpsc::Receiver<ClientMessage>,
    handle: Handle,
}

impl RequestHelper {
    fn new(
        response_endpoint_id: EndpointId,
        // to_address: Address,
        rx: mpsc::Receiver<ClientMessage>,
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
        <T as BusRiderRpc>::Response: BusDeserialize,
    {
        // let map = self.handle.route_watch_rx.borrow();
        let fib = self.handle.fib.load();
        let node_id = fib.get_node_id();
        let payload = Box::new(payload);
        let to_address: Address = T::ANYBUS_UUID.into();
        {
            fib.send(Packet {
                to: to_address,
                reply_to: Some(Address::Remote(self.response_endpoint_id.into(), node_id)),
                from: fib.our_id,
                payload: Payload::BusRider(payload),
            })
            .map_err(AnyBusHandleError::SendError)?;
        }
        let incoming = self.rx.recv().await;

        let res = match incoming {
            Some(ClientMessage::Message(val)) => val.payload.reveal().map_err(|p| {
                AnyBusHandleError::ReceiveError(ReceiveError::DeserializationError(p))
            }),
            None => Err(AnyBusHandleError::Shutdown),
            _ => {
                unreachable!()
            }
        };
        res
    }

    /// A request using user provided Uuid
    pub async fn request_to_uuid<U: BusRiderRpc>(
        &mut self,
        payload: U,
        endpoint_id: impl Into<EndpointId>,
    ) -> Result<U::Response, AnyBusHandleError>
    where
        <U as BusRiderRpc>::Response: BusDeserialize,
    {
        let to_address: Address = endpoint_id.into().into();
        let node_id = self.handle.fib.load().get_node_id();
        let payload = Box::new(payload);
        let fib = self.handle.fib.load();

        fib.send(Packet {
            to: to_address,
            reply_to: Some(Address::Remote(self.response_endpoint_id.into(), node_id)),
            from: fib.our_id,
            payload: Payload::BusRider(payload),
        })
        .map_err(AnyBusHandleError::SendError)?;

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
    pub async fn register<T: BusRider + BusDeserialize>(self) -> Result<Receiver<T>, ReceiveError> {
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
        self,
    ) -> Result<RpcReceiver<T>, ReceiveError> {
        let ep = T::ANYBUS_UUID.into();
        self.handle.register_rpc_inner::<T>(ep).await
    }
}

impl<CAST> RegistrationBuilder<EndpointSet, CAST, RpcSet> {
    /// Finalize the registration and get a [RpcReceiver] for the messages
    pub async fn register<T: BusRiderRpc + BusDeserialize>(
        self,
    ) -> Result<RpcReceiver<T>, ReceiveError> {
        self.handle
            .register_rpc_inner::<T>(self.endpoint_id.0.into())
            .await
    }
}

impl RegistrationBuilder<NoEndpointId, CastSet, NoRpc> {
    /// Finalize the registration and get a [Receiver] for the messages
    pub async fn register<T: BusRider + BusDeserialize + BusRiderWithUuid>(
        self,
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
