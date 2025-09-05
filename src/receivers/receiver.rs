use serde::Deserialize;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    ReceiveError, messages::ClientMessage, receivers::packet_receiver::PacketReceiver,
    routing::EndpointId,
};

pub struct Receiver<T: crate::BusRider> {
    _pd: std::marker::PhantomData<T>,
    packet_receiver: PacketReceiver,
}

impl<T: crate::BusRider> Receiver<T>
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

    pub async fn recv(&mut self) -> Result<T, crate::errors::ReceiveError> {
        let packet = self.packet_receiver.recv().await?;
        packet
            .payload
            .reveal()
            .map_err(|p| ReceiveError::DeserializationError(p))
    }
}
