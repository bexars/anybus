use futures::Stream;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    errors::ReceiveError,
    messages::ClientMessage,
    routing::{EndpointId, Packet},
};

#[derive(Debug)]
pub(crate) struct PacketReceiver {
    pub(crate) endpoint_id: EndpointId,
    pub(crate) rx: UnboundedReceiver<ClientMessage>,

    pub(crate) handle: crate::Handle,
}

impl PacketReceiver {
    pub(crate) fn new(
        endpoint_id: EndpointId,
        rx: UnboundedReceiver<ClientMessage>,
        handle: crate::Handle,
    ) -> Self {
        Self {
            endpoint_id,
            rx,
            handle,
        }
    }

    pub(super) async fn recv(&mut self) -> Result<Packet, ReceiveError> {
        loop {
            let new_msg = self.rx.recv().await;

            match new_msg {
                None => {
                    return Err(ReceiveError::Shutdown);
                }
                Some(msg) => match msg {
                    ClientMessage::Message(packet) => {
                        return Ok(packet);
                    }
                    ClientMessage::Shutdown => {
                        return Err(ReceiveError::Shutdown);
                    }
                    ClientMessage::FailedRegistration(_, _) => {
                        continue;
                    }
                    ClientMessage::SuccessfulRegistration(_) => {
                        continue;
                    }
                },
            }
        }
    }

    pub(super) fn try_recv(&mut self) -> Option<Result<Packet, ReceiveError>> {
        let msg = match self.rx.try_recv() {
            Ok(msg) => match msg {
                ClientMessage::Message(packet) => Ok(packet),
                ClientMessage::Shutdown => Err(ReceiveError::Shutdown),
                ClientMessage::FailedRegistration(_, _) => Err(ReceiveError::RegistrationFailed(
                    "Registration failed".into(),
                )),
                ClientMessage::SuccessfulRegistration(_) => Err(ReceiveError::RegistrationFailed(
                    "Unexpected successful registration".into(),
                )),
            },
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                return None;
                // Err(ReceiveError::NoMessageAvailable)
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                Err(ReceiveError::Shutdown)
            }
        };
        Some(msg)
    }
}
impl Drop for PacketReceiver {
    fn drop(&mut self) {
        self.handle.unregister_endpoint(self.endpoint_id);
    }
}

impl Stream for PacketReceiver {
    type Item = Packet;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match this.rx.poll_recv(cx) {
            std::task::Poll::Ready(Some(msg)) => match msg {
                ClientMessage::Message(packet) => std::task::Poll::Ready(Some(packet)),
                ClientMessage::Shutdown => std::task::Poll::Ready(None),
                ClientMessage::FailedRegistration(_, _) => std::task::Poll::Pending,
                ClientMessage::SuccessfulRegistration(_) => std::task::Poll::Pending,
            },
            std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}
