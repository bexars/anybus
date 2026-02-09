use tracing::error;

use crate::{BusRider, BusStop, EndpointId, Handle};

#[allow(dead_code)] //temporary WIP TODO
pub(super) enum ServiceCommand {
    Start,
    Pause,
    Shutdown,
}

pub struct BusStopService<T: BusRider + for<'de> serde::Deserialize<'de>> {
    bus_stop: Box<dyn BusStop<T> + 'static + Send>,
    id: EndpointId,
    handle: Handle,
}

impl<T: BusRider + for<'de> serde::Deserialize<'de>> BusStopService<T> {
    pub fn new(
        bus_stop: impl BusStop<T> + 'static + Send,
        id: EndpointId,
        handle: Handle,
    ) -> BusStopService<T> {
        BusStopService {
            bus_stop: Box::new(bus_stop),
            id,
            handle,
        }
    }

    pub async fn bus_stop_service(self) {
        let mut receiver = match self
            .handle
            .listener()
            .endpoint(self.id)
            .anycast()
            .register::<T>()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                error!("BusStop registration failure {}", e);
                self.bus_stop.on_shutdown(&self.handle);
                return;
            }
        };

        loop {
            while let Ok(msg) = receiver.recv().await {
                self.bus_stop
                    .on_message(msg, &self.handle)
                    .into_iter()
                    .for_each(|bt| {
                        let res = self.handle.send_busticket(bt);
                        if let Err(e) = res {
                            error!("Error sending busticket in BusStop handler: {e}");
                        }
                    });
            }
        }
    }
}
