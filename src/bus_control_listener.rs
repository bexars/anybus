use std::{
    any::{Any, TypeId},
    marker::PhantomData,
};

use serde::de::DeserializeOwned;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::info;

use crate::{messages::ClientMessage, BusRider, ReceiveError, RegistrationStatus};

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
    /// If this was created from a failed registration, the first message will be [RegistrationFailed](errors::ReceiveError::RegistrationFailed)
    pub async fn recv(&mut self) -> Result<T, ReceiveError> {
        loop {
            // #[cfg(feature = "dioxus")]
            // dioxus::logger::tracing::info!("In recv loop: {:?}", self);

            // #[cfg(feature = "tokio")]
            let new_msg = self.rx.recv().await;
            // #[cfg(feature = "dioxus")]
            // let new_msg = self.rx.next().await;
            // #[cfg(feature = "dioxus")]
            // dioxus::logger::tracing::info!("Received msg: {:?}", new_msg);
            match new_msg {
                None => {
                    return Err(ReceiveError::ConnectionClosed);
                }
                Some(msg) => {
                    match msg {
                        ClientMessage::Message(_id, payload) => {
                            // let payload = payload.as_any();
                            if TypeId::of::<T>() == (*payload).type_id() {
                                let payload: Box<T> = (payload as Box<dyn Any>).downcast().unwrap();
                                // let Some(res) = (*payload).downcast_ref::<T>() else { continue };
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
                        ClientMessage::SuccessfulRegistration(_uuid) => todo!(),
                    }
                }
            };
        }
    }

    /// Similar to recv() but with a enum denoting a message or a RPC request
    pub async fn recv_rpc(&mut self) -> Result<T, ReceiveError> {
        loop {
            // #[cfg(feature = "dioxus")]
            // dioxus::logger::tracing::info!("In recv loop: {:?}", self);

            // #[cfg(feature = "tokio")]
            let new_msg = self.rx.recv().await;
            // #[cfg(feature = "dioxus")]
            // let new_msg = self.rx.next().await;
            // #[cfg(feature = "dioxus")]
            // dioxus::logger::tracing::info!("Received msg: {:?}", new_msg);
            match new_msg {
                None => {
                    return Err(ReceiveError::ConnectionClosed);
                }
                Some(msg) => {
                    match msg {
                        ClientMessage::Message(_id, payload) => {
                            let payload: Box<T> = (payload as Box<dyn Any>).downcast().unwrap();
                            if TypeId::of::<T>() == (*payload).type_id() {
                                let payload: Box<T> = (payload as Box<dyn Any>).downcast().unwrap();
                                // let Some(res) = (*payload).downcast_ref::<T>() else { continue };
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
                        ClientMessage::SuccessfulRegistration(_uuid) => todo!(),
                    }
                }
            };
        }
    }

    /// Check if the client is registered with the bus.
    pub(crate) async fn wait_for_registration(&mut self) -> RegistrationStatus {
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
