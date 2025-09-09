use serde::Deserialize;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    BusRiderRpc, Handle, ReceiveError,
    errors::AnyBusHandleError,
    messages::ClientMessage,
    receivers::packet_receiver::PacketReceiver,
    routing::{Address, EndpointId},
};

/// A RpcReceiver receives RPC messages sent to the registered endpoint.
#[derive(Debug)]
pub struct RpcReceiver<T: crate::BusRiderRpc> {
    _pd: std::marker::PhantomData<T>,
    packet_receiver: PacketReceiver,
}

impl<T: crate::BusRiderRpc> RpcReceiver<T>
where
    for<'de> T: Deserialize<'de>,
{
    pub(crate) fn new(
        endpoint_id: EndpointId,
        rx: UnboundedReceiver<ClientMessage>,
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
        let packet = self.packet_receiver.recv().await?;
        let reply_to = packet.reply_to.ok_or(ReceiveError::RpcNoReplyTo)?;
        let payload = packet
            .payload
            .reveal()
            .map_err(|p| ReceiveError::DeserializationError(p))?;
        let handle = self.packet_receiver.handle.clone();
        let rpc_request = RpcRequest::new(reply_to, payload, handle);

        Ok(rpc_request)
    }
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
}
