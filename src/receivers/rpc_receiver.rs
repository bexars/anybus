use crate::tokio;

use tokio::sync::mpsc::{self};

use crate::{
    BusDeserialize, BusRiderRpc, Handle, ReceiveError,
    errors::AnyBusHandleError,
    messages::ClientMessage,
    receivers::{anycast_cost::AnycastCost, packet_receiver::PacketReceiver},
    routing::{Address, EndpointId},
};

/// A RpcReceiver receives RPC messages sent to the registered endpoint.
#[derive(Debug)]
pub struct RpcReceiver<T: crate::BusRiderRpc> {
    _pd: std::marker::PhantomData<T>,
    packet_receiver: PacketReceiver,
}

impl<T: crate::BusRiderRpc + BusDeserialize> RpcReceiver<T> {
    pub(crate) fn new(
        endpoint_id: EndpointId,
        rx: mpsc::Receiver<ClientMessage>,
        handle: crate::Handle,
    ) -> Self {
        let packet_receiver = PacketReceiver::new(endpoint_id, rx, handle);
        Self {
            packet_receiver,
            _pd: std::marker::PhantomData,
        }
    }
    /// Receives the next packet sent to this endpoint.

    pub async fn recv(&mut self) -> Result<RpcRequest<T>, crate::errors::ReceiveError> {
        recv_rpc(&mut self.packet_receiver).await
    }
}

/// An anycast RPC listener. Several may share an endpoint, and [`Self::set_cost`]
/// changes which one receives the next request.
#[derive(Debug)]
pub struct AnycastRpcReceiver<T: BusRiderRpc> {
    packet_receiver: PacketReceiver,
    cost: AnycastCost,
    _pd: std::marker::PhantomData<T>,
}

impl<T: BusRiderRpc + BusDeserialize> AnycastRpcReceiver<T> {
    pub(crate) fn new(
        endpoint_id: EndpointId,
        rx: mpsc::Receiver<ClientMessage>,
        sender: mpsc::Sender<ClientMessage>,
        handle: Handle,
    ) -> Self {
        Self {
            packet_receiver: PacketReceiver::new(endpoint_id, rx, handle.clone()),
            cost: AnycastCost::new(endpoint_id, sender, handle),
            _pd: std::marker::PhantomData,
        }
    }

    /// Receives the next RPC request sent to this endpoint.
    pub async fn recv(&mut self) -> Result<RpcRequest<T>, ReceiveError> {
        recv_rpc(&mut self.packet_receiver).await
    }

    /// Set this listener's cost. Completes when the route table records it.
    pub async fn set_cost(&self, cost: u16) -> Result<(), AnyBusHandleError> {
        self.cost.set_cost(cost).await
    }
}

async fn recv_rpc<T: BusRiderRpc + BusDeserialize>(
    packet_receiver: &mut PacketReceiver,
) -> Result<RpcRequest<T>, ReceiveError> {
    let packet = packet_receiver.recv().await?;
    let reply_to = packet.reply_to.ok_or(ReceiveError::RpcNoReplyTo)?;
    let payload = packet
        .payload
        .reveal()
        .map_err(|p| ReceiveError::DeserializationError(p))?;
    let handle = packet_receiver.handle.clone();
    Ok(RpcRequest::new(reply_to, payload, handle))
}

/// An RpcRequest is returned by an RpcReceiver when an RPC message is received.  It contains the payload
/// and the address to send the response to.
#[derive(Debug)]
pub struct RpcRequest<T>
where
    T: BusRiderRpc,
{
    response_endpoint_id: Address,
    payload: Option<T>,
    handle: Handle,
}

impl<T> RpcRequest<T>
where
    T: BusRiderRpc,
{
    fn new(response: Address, payload: T, handle: Handle) -> RpcRequest<T> {
        Self {
            response_endpoint_id: response,
            payload: Some(payload),
            handle,
        }
    }

    /// First call will return the payload, subsequent calls will return None.
    pub fn payload(&mut self) -> Option<T> {
        self.payload.take()
    }

    /// Replies to the address in the request and ensures type correctness
    pub fn reply(self, response: T::Response) -> Result<(), AnyBusHandleError> {
        self.handle
            .send_to_address(self.response_endpoint_id, response)
        // .map_err(|payload| AnyBusHandleError::SendError(payload))
    }

    /// A reference to the [[Anybus]] handle to be cloned or used as needed
    pub fn handle(&self) -> &Handle {
        &self.handle
    }
}
