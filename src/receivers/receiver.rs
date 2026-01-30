use futures::Stream;
use serde::Deserialize;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    BusRider, ReceiveError, messages::ClientMessage, receivers::packet_receiver::PacketReceiver,
    routing::EndpointId,
};

/// A Receiver receives messages sent to the registered endpoint.
#[derive(Debug)]
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

    /// Polls for available packet and returns it or an error.  None is no message waiting
    pub fn try_recv(&mut self) -> Option<Result<T, crate::errors::ReceiveError>> {
        let packet = self.packet_receiver.try_recv();
        match packet {
            Some(Ok(packet)) => match packet.payload.reveal() {
                Ok(msg) => Some(Ok(msg)),
                Err(_payload) => None, // just don't deliver undeserializable messages
            },
            Some(Err(err)) => Some(Err(err)),
            None => None,
        }
    }
}

impl<T: BusRider + Unpin> Stream for Receiver<T>
where
    for<'de> T: Deserialize<'de>,
{
    type Item = T;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match futures::ready!(std::pin::Pin::new(&mut this.packet_receiver.rx).poll_recv(cx)) {
            Some(ClientMessage::Message(packet)) => match packet.payload.reveal() {
                Ok(msg) => std::task::Poll::Ready(Some(msg)),
                Err(_) => std::task::Poll::Pending,
            },
            Some(ClientMessage::Shutdown) => std::task::Poll::Ready(None),
            Some(ClientMessage::FailedRegistration(_, _)) => std::task::Poll::Pending,
            Some(ClientMessage::SuccessfulRegistration(_)) => std::task::Poll::Pending,
            None => std::task::Poll::Ready(None),
        }
    }
}
