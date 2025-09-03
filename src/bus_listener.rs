use std::{
    any::{Any, TypeId},
    marker::PhantomData,
};

use binrw::Endian;
use serde::de::DeserializeOwned;
use tokio::sync::{mpsc::UnboundedReceiver, oneshot};
use tracing::info;
use uuid::Uuid;

use crate::{
    Handle, ReceiveError,
    errors::MsgBusHandleError,
    messages::{ClientMessage, RegistrationStatus},
    routing::{Address, EndpointId, Packet, Payload},
    traits::{BusRider, BusRiderRpc},
};

#[derive(Debug)]
/// Helper struct returned by [Handle::register_anycast()]
pub struct BusListener<T: BusRider> {
    pub(crate) rx: UnboundedReceiver<ClientMessage>,
    pub(crate) registration_status: RegistrationStatus,
    pub(crate) _pd: PhantomData<T>,
    pub(crate) handle: Handle,
    pub(crate) endpoint_id: Uuid,
}

impl<T: BusRider> Drop for BusListener<T> {
    fn drop(&mut self) {
        self.handle.unregister_endpoint(self.endpoint_id);
    }
}

impl<T: BusRider + DeserializeOwned> BusListener<T> {
    /// Returns T until an error occurs.  There is no recovery from an
    /// error and the connection should be considered closed.
    ///
    /// If this was created from a failed registration it will return (errors::ReceiveError::RegistrationFailed)
    pub async fn recv(&mut self) -> Result<T, ReceiveError> {
        loop {
            let new_msg = self.rx.recv().await;

            match new_msg {
                None => {
                    return Err(ReceiveError::ConnectionClosed);
                }
                Some(msg) => match msg {
                    ClientMessage::Message(packet) => {
                        let payload: T = match packet.payload {
                            crate::routing::Payload::BusRider(bus_rider) => {
                                match (bus_rider as Box<dyn Any>).downcast::<T>() {
                                    Ok(b) => *b,
                                    Err(_) => continue,
                                }
                            }
                            crate::routing::Payload::Bytes(items) => {
                                match serde_cbor::from_slice(&items) {
                                    Ok(p) => p,
                                    Err(_) => continue,
                                }
                            }
                        };
                        return Ok(payload);
                    }

                    ClientMessage::FailedRegistration(_uuid, msg) => {
                        return Err(ReceiveError::RegistrationFailed(msg));
                    }
                    ClientMessage::Shutdown => {
                        println!("Shutdown in BusListener");
                        return Err(ReceiveError::Shutdown);
                    }
                    ClientMessage::SuccessfulRegistration(_uuid) => {
                        self.registration_status = RegistrationStatus::Registered;
                        continue;
                    }
                },
            };
        }
    }
    /// Check if the client is registered with the bus.
    pub async fn wait_for_registration(&mut self) -> RegistrationStatus {
        match &self.registration_status {
            RegistrationStatus::Registered => {}
            RegistrationStatus::Failed(_msg) => {}
            RegistrationStatus::Pending => {
                let res = self.rx.recv().await;
                info!("Received message: {:?}", res);
                match res {
                    Some(ClientMessage::SuccessfulRegistration(_)) => {
                        self.registration_status = RegistrationStatus::Registered;
                    }
                    Some(ClientMessage::FailedRegistration(_, msg)) => {
                        self.registration_status = RegistrationStatus::Failed(msg.clone());
                    }
                    _ => {
                        self.registration_status =
                            RegistrationStatus::Failed("Unknown error".to_string());
                    }
                }
            }
        }
        self.registration_status.clone()
    }
}

impl<T: BusRiderRpc + DeserializeOwned> BusListener<T> {
    /// Similar to recv() but with a enum denoting a message or a RPC request
    pub async fn recv_rpc(&mut self) -> Result<RpcRequest<T>, ReceiveError> {
        loop {
            let new_msg = self.rx.recv().await;

            match new_msg {
                None => {
                    return Err(ReceiveError::ConnectionClosed);
                }
                Some(msg) => match msg {
                    ClientMessage::Message(packet) => {
                        let Packet {
                            payload, reply_to, ..
                        } = packet;

                        let tx = match reply_to {
                            Some(tx) => tx,
                            None => continue,
                        };
                        let payload = payload.reveal();
                        if let Some(payload) = payload {
                            return Ok(RpcRequest::<T>::new(tx, payload, self.handle.clone()));
                        } else {
                            continue;
                        }
                    }
                    ClientMessage::FailedRegistration(_uuid, msg) => {
                        return Err(ReceiveError::RegistrationFailed(msg));
                    }
                    ClientMessage::Shutdown => {
                        println!("Shutdown in BusListener");
                        return Err(ReceiveError::Shutdown);
                    }
                    ClientMessage::SuccessfulRegistration(_uuid) => todo!(),
                },
            };
        }
    }
}

// enum RpcResponseType<T>
// where
//     T: BusRiderRpc,
// {
//     Remote(Uuid),
//     Local(oneshot::Sender<T::Response>),
// }

pub struct RpcRequest<T>
where
    T: BusRiderRpc,
{
    response: EndpointId,
    payload: Option<T>,
    handle: Handle,
}

impl<T> RpcRequest<T>
where
    T: BusRiderRpc,
{
    pub fn new(response: EndpointId, payload: T, handle: Handle) -> RpcRequest<T> {
        Self {
            response,
            payload: Some(payload),
            handle,
        }
    }

    /// First call will return the payload, subsequent calls will return None.
    pub fn payload(&mut self) -> Option<T> {
        self.payload.take()
    }

    pub fn respond(self, response: T::Response) -> Result<(), MsgBusHandleError> {
        self.handle.send_to_uuid(self.response, response)
        // .map_err(|payload| MsgBusHandleError::SendError(payload))
    }
}
