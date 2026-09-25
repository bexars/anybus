use crate::tokio::sync::mpsc;
use crate::{
    EndpointId, Handle,
    errors::AnyBusHandleError,
    messages::{ClientMessage, RouterMsg, SetAnycastCostError},
};

/// Identity of one anycast listener, used to change its cost.
#[derive(Debug)]
pub(crate) struct AnycastCost {
    endpoint_id: EndpointId,
    sender: mpsc::Sender<ClientMessage>,
    handle: Handle,
}

impl AnycastCost {
    pub(crate) fn new(
        endpoint_id: EndpointId,
        sender: mpsc::Sender<ClientMessage>,
        handle: Handle,
    ) -> Self {
        Self {
            endpoint_id,
            sender,
            handle,
        }
    }

    pub(crate) async fn set_cost(&self, cost: u16) -> Result<(), AnyBusHandleError> {
        let (reply, result) = crate::tokio::sync::oneshot::channel();
        self.handle
            .tx
            .send(RouterMsg::SetAnycastCost {
                endpoint_id: self.endpoint_id,
                sender: self.sender.clone(),
                cost,
                reply,
            })
            .await
            .map_err(|_| AnyBusHandleError::Shutdown)?;
        match result.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(SetAnycastCostError::NotAnycast)) => Err(AnyBusHandleError::NotAnycast),
            Ok(Err(SetAnycastCostError::NotRegistered)) => {
                Err(AnyBusHandleError::ListenerNotRegistered)
            }
            Err(_) => Err(AnyBusHandleError::Shutdown),
        }
    }
}
