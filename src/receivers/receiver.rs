use crate::tokio;
use futures::Stream;
use tokio::sync::mpsc::{self};

use crate::{
    BusDeserialize, BusRider, Handle, ReceiveError, errors::AnyBusHandleError,
    messages::ClientMessage, receivers::anycast_cost::AnycastCost,
    receivers::packet_receiver::PacketReceiver, routing::EndpointId,
};

/// A Receiver receives messages sent to the registered endpoint.
#[derive(Debug)]
pub struct Receiver<T: crate::BusRider> {
    _pd: std::marker::PhantomData<T>,
    packet_receiver: PacketReceiver,
}

impl<T: crate::BusRider + BusDeserialize> Receiver<T> {
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

impl<T: BusRider + BusDeserialize + Unpin> Stream for Receiver<T> {
    type Item = T;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match std::task::ready!(std::pin::Pin::new(&mut this.packet_receiver.rx).poll_recv(cx)) {
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

/// An anycast [`Receiver`] whose registration cost can be changed.
#[derive(Debug)]
pub struct AnycastReceiver<T: BusRider> {
    receiver: Receiver<T>,
    cost: AnycastCost,
}

impl<T: BusRider> AnycastReceiver<T> {
    pub(crate) fn new(
        receiver: Receiver<T>,
        endpoint_id: EndpointId,
        sender: mpsc::Sender<ClientMessage>,
        handle: Handle,
    ) -> Self {
        Self {
            receiver,
            cost: AnycastCost::new(endpoint_id, sender, handle),
        }
    }

    /// Set this listener's cost. Completes when the route table records it.
    pub async fn set_cost(&self, cost: u16) -> Result<(), AnyBusHandleError> {
        self.cost.set_cost(cost).await
    }
}

impl<T: BusRider + BusDeserialize> AnycastReceiver<T> {
    /// Receives the next packet sent to this endpoint.
    pub async fn recv(&mut self) -> Result<T, ReceiveError> {
        self.receiver.recv().await
    }

    /// Polls for an available packet. `None` means no message is waiting.
    pub fn try_recv(&mut self) -> Option<Result<T, ReceiveError>> {
        self.receiver.try_recv()
    }
}

impl<T: BusRider + BusDeserialize + Unpin> Stream for AnycastReceiver<T> {
    type Item = T;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.get_mut().receiver).poll_next(cx)
    }
}
