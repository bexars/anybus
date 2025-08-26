use std::{
    any::{Any, TypeId},
    marker::PhantomData,
};

use serde::de::DeserializeOwned;
use tokio::sync::{mpsc::UnboundedReceiver, oneshot};
use tracing::info;

use crate::{
    errors::MsgBusHandleError,
    messages::ClientMessage,
    traits::{BusRider, BusRiderRpc},
    ReceiveError, RegistrationStatus,
};

#[derive(Debug)]
/// Helper struct returned by [Handle::register_anycast()]
pub struct BusListener<T: BusRider> {
    pub(crate) rx: UnboundedReceiver<ClientMessage>,
    pub(crate) registration_status: RegistrationStatus,
    pub(crate) _pd: PhantomData<T>,
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
                    ClientMessage::Message(_id, payload) => {
                        if TypeId::of::<T>() == (*payload).type_id() {
                            let payload: Box<T> = (payload as Box<dyn Any>).downcast().unwrap();
                            return Ok(*payload);
                        } else {
                            continue;
                        }
                    }
                    ClientMessage::Bytes(_id, bytes) => match serde_cbor::from_slice(&bytes) {
                        Ok(payload) => return Ok(payload),
                        Err(_) => continue,
                    },
                    ClientMessage::Rpc {
                        to: _,
                        reply_to: _,
                        msg: _,
                    } => todo!(),
                    ClientMessage::FailedRegistration(_uuid, msg) => {
                        return Err(ReceiveError::RegistrationFailed(msg))
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
                Some(msg) => {
                    match msg {
                        ClientMessage::Message(_id, _payload) => {
                            // let payload: Box<T> = (payload as Box<dyn Any>).downcast().unwrap();
                            // if TypeId::of::<T>() == (*payload).type_id() {
                            //     let payload: Box<T> = (payload as Box<dyn Any>).downcast().unwrap();
                            //     // let Some(res) = (*payload).downcast_ref::<T>() else { continue };
                            //     return Ok(*payload);
                            // } else {
                            //     continue;
                            // }
                            continue;
                        }
                        ClientMessage::Bytes(_id, _bytes) => {
                            // match serde_cbor::from_slice(&bytes) {
                            // Ok(payload) => return Ok(payload),
                            // Err(_) => continue,
                            continue;
                        }
                        ClientMessage::Rpc {
                            to: _,
                            reply_to: tx,
                            msg: payload,
                        } => {
                            let payload: Box<T> = (payload as Box<dyn Any>).downcast().unwrap();
                            if TypeId::of::<T>() == (*payload).type_id() {
                                let payload: Box<T> = (payload as Box<dyn Any>).downcast().unwrap();
                                // let Some(res) = (*payload).downcast_ref::<T>() else { continue };
                                return Ok(RpcRequest::<T>::new(tx, *payload));
                            } else {
                                continue;
                            }
                        }
                        ClientMessage::FailedRegistration(_uuid, msg) => {
                            return Err(ReceiveError::RegistrationFailed(msg))
                        }
                        ClientMessage::Shutdown => {
                            println!("Shutdown in BusListener");
                            return Err(ReceiveError::Shutdown);
                        }
                        ClientMessage::SuccessfulRegistration(_uuid) => todo!(),
                    }
                }
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
    response: oneshot::Sender<Box<dyn BusRider>>,
    payload: Option<T>,
}

impl<T> RpcRequest<T>
where
    T: BusRiderRpc,
{
    pub fn new(response: oneshot::Sender<Box<dyn BusRider>>, payload: T) -> RpcRequest<T> {
        Self {
            response,
            payload: Some(payload),
        }
    }

    /// First call will return the payload, subsequent calls will return None.
    pub fn payload(&mut self) -> Option<T> {
        self.payload.take()
    }

    pub fn respond(self, response: T::Response) -> Result<(), MsgBusHandleError> {
        self.response
            .send(Box::new(response))
            .map_err(MsgBusHandleError::SendError)
    }
}
