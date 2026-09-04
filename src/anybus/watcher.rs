use crate::{AnyBusStatusMsg, Handle};
use powerwatch::PowerWatch;

pub(crate) struct Watcher {
    handle: Handle,
}

impl Watcher {
    pub(crate) fn new(handle: Handle) -> Self {
        Self { handle }
    }
    pub(crate) fn start(self) {
        tokio::spawn(async move {
            self.run().await;
        });
    }

    async fn run(self) {
        let Ok(mut status_rx) = self.handle.get_anybus_status_receiver().await else {
            tracing::error!("Failed to get AnyBus status receiver");
            return;
        };

        let (_pw, events) = match PowerWatch::start() {
            Ok(res) => res,
            Err(e) => {
                tracing::error!("Failed to start power watch: {:?}", e);
                return;
            }
        };
        tracing::info!("Power watch started!!!!!");
        loop {
            tokio::select! {
                Ok(event) = events.recv_async() => {
                    tracing::info!("Power event: {:?}", event);
                    let msg = match event {
                        powerwatch::PowerEvent::Suspend => {
                            tracing::info!("System is suspending");
                            Some(AnyBusStatusMsg::Suspending)
                        }
                        powerwatch::PowerEvent::Resume => {
                            tracing::info!("System has resumed");

                            Some(AnyBusStatusMsg::Resuming)
                        }
                        powerwatch::PowerEvent::Shutdown => {

                            tracing::info!("System is shutting down");
                            None
                        }
                        powerwatch::PowerEvent::ScreenLocked => {
                            tracing::info!("Screen is locked");
                            None
                        }
                        powerwatch::PowerEvent::ScreenUnlocked => {
                            tracing::info!("Screen is unlocked");
                           None
                        }
                    };
                    if let Some(msg) = msg {
                        self.handle.send(msg).ok();
                    }
                }
                status = status_rx.recv() => {
                    match status {
                        Ok(status) => {
                            tracing::info!("AnyBus status: {:?}", status);
                            // Handle AnyBus status updates here
                            if let AnyBusStatusMsg::ShuttingDown = status {
                                tracing::info!("AnyBus is shutting down, stopping watcher");
                                break;
                            }
                        }
                        Err(err) => {
                            tracing::error!("Error receiving AnyBus status: {:?}", err);
                            break;
                        }
                    }
                }
            }
        }
    }
}
