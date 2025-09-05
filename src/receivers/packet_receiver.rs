use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    errors::ReceiveError,
    messages::ClientMessage,
    routing::{EndpointId, Packet},
};

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
}
impl Drop for PacketReceiver {
    fn drop(&mut self) {
        self.handle.unregister_endpoint(self.endpoint_id);
    }
}
