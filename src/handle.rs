use std::{error::Error, marker::PhantomData, mem::swap};

use itertools::{FoldWhile, Itertools};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::bus_control_listener::BusListener;
use crate::messages::{BrokerMsg, ClientMessage};
use crate::DestinationType;
use crate::{
    errors::{self, ReceiveError},
    BusRider, BusRiderWithUuid, Nexthop, RoutesWatchRx,
};

/// The handle for talking to the [MsgBus] instance that created it.  It can be cloned freely
#[derive(Debug, Clone)]
pub struct Handle {
    pub(crate) tx: UnboundedSender<BrokerMsg>,
    pub(crate) rts_rx: RoutesWatchRx,
}

impl Handle {
    /// Registers an anycast style of listener to the given Uuid and type T that will return a [BusListener] for receiving
    /// messages sent to the [Uuid]
    // TODO: Cleanup the errors being sent
    pub async fn register_anycast<T: BusRiderWithUuid + DeserializeOwned>(
        &mut self,
        // uuid: Uuid,
    ) -> Result<BusListener<T>, ReceiveError> {
        let uuid = T::MSGBUS_UUID;
        // #[cfg(feature = "tokio")]
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        // #[cfg(feature = "dioxus")]
        // let (tx, rx) = futures_channel::mpsc::unbounded();

        let mut bl = BusListener::<T> {
            rx,
            _pd: PhantomData,
            registration_status: Default::default(),
        };

        let register_msg = BrokerMsg::RegisterAnycast(uuid, tx);

        // #[cfg(feature = "dioxus")]
        // self.tx.unbounded_send(register_msg)?;

        // #[cfg(feature = "tokio")]
        self.tx.send(register_msg)?;

        #[cfg(feature = "tokio")]
        info!("Send register_msg");

        if bl.is_registered().await {
            Ok(bl)
        } else {
            Err(ReceiveError::RegistrationFailed)
        }
    }

    /// Placeholder - Not Implemented
    pub fn register_unicast<T: BusRider + DeserializeOwned>(
        &mut self,
        _uuid: Uuid,
    ) -> Result<BusListener<T>, Box<dyn Error>> {
        todo!()
    }

    /// Placeholder - Not Implemented
    pub fn register_multicast<T: BusRider + DeserializeOwned>(
        &mut self,
        _uuid: Uuid,
    ) -> Result<BusListener<T>, Box<dyn Error>> {
        todo!()
    }

    // pub fn register<T: BusRider + DeserializeOwned>(
    //     &mut self,

    // ) -> Result<BusListener<T>, Box<dyn Error>> {
    //     self.register_with_uuid(T::default_uuid())
    // }

    /// Sends a single [BusRider] message to the indicated [Uuid].  This can be a Unicast, AnyCast, or Multicast receiver.
    /// The system will deliver regardless of end type
    pub fn send<T: BusRiderWithUuid>(&self, payload: T) -> Result<(), errors::MsgBusHandleError> {
        let u = T::MSGBUS_UUID;
        let payload = Box::new(payload);
        let mut msg = Some(ClientMessage::Message(u, payload));

        match self.rts_rx.borrow().get(&u) {
            Some(endpoint) => {
                let mut success = false;
                let mut had_failure = false;

                // TODO Turn this into a scan iterator or just about anything better
                match endpoint {
                    Nexthop::Anycast(dests) => {
                        //TODO call deadlink on these
                        let errors = dests
                            .iter()
                            .fold_while(Vec::new(), |mut prev, dest| match &dest.dest_type {
                                DestinationType::Local(tx) => {
                                    let mut m = None;
                                    swap(&mut m, &mut msg);
                                    let m = m.unwrap();

                                    let res = tx.send(m);
                                    match res {
                                        Ok(_) => {
                                            success = true;
                                            FoldWhile::Done(prev)
                                        }
                                        Err(e) => {
                                            let m = e.0;
                                            let mut m = Some(m);
                                            swap(&mut m, &mut msg);
                                            prev.push(tx.clone());
                                            itertools::FoldWhile::Continue(prev)
                                        }
                                    }
                                }
                                DestinationType::Remote(_uuid) => todo!(),
                            })
                            .into_inner();
                        if !errors.is_empty() {
                            had_failure = true;
                        };
                    }
                    Nexthop::Broadcast(_) => todo!(),
                    Nexthop::Unicast(unicast_type) => todo!(),
                }

                if had_failure {
                    // #[cfg(feature = "tokio")]
                    // self.tx.send(BrokerMsg::DeadLink(u));
                    // #[cfg(feature = "dioxus")]
                    let _ = self.tx.send(BrokerMsg::DeadLink(u)); // Just ignore if this fails
                }

                if !success {
                    let Some(ClientMessage::Message(_, payload)) = msg else {
                        panic!()
                    };
                    return Err(errors::MsgBusHandleError::SendError(payload));
                }
            }
            None => return Err(errors::MsgBusHandleError::NoRoute),
        };

        // TODO lookup in watched hashmap
        // self.tx.send(Message::Message(u, payload))?;
        Ok(())
    }
}
