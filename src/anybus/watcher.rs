#[cfg(feature = "resume_watch")]
use crate::AnyBusStatusMsg;
use crate::Handle;

/// Watches for changes to network topology and detects resumes from suspension
pub(crate) struct Watcher {
    handle: Handle,
}

impl Watcher {
    pub(crate) fn new(handle: Handle) -> Self {
        Self { handle }
    }

    pub(crate) fn start(self) {
        tokio::spawn(self.run());
    }

    async fn run(self) {
        // lock down this registration so nothing else can register this UUID
        let _status_receiver = self
            .handle
            .get_anybus_status_receiver()
            .await
            .expect("Unable to register status receiver");

        #[cfg(all(
            any(target_os = "windows", target_os = "linux", target_os = "macos"),
            feature = "resume_watch"
        ))]
        tokio::task::spawn_blocking(move || {
            let handle = self.handle.clone();
            let power_monitor = psp::monitor::PowerMonitor::new();
            let pm_recv = power_monitor.event_receiver();
            if let Err(e) = power_monitor.start_listening() {
                tracing::error!("Unable to start power monitor: {}", e);
            };
            while let Ok(event) = pm_recv.recv() {
                match event {
                    psp::monitor::PowerState::Unknown => {}
                    psp::monitor::PowerState::Suspend => {
                        tracing::info!("Suspend event received from power monitor");
                        handle.send(AnyBusStatusMsg::Suspending).ok();
                    }
                    psp::monitor::PowerState::Resume => {
                        tracing::info!("Resume event received from power monitor");
                        handle.send(AnyBusStatusMsg::Resuming).ok();
                    }
                    psp::monitor::PowerState::Shutdown => {
                        tracing::info!("Shutdown event received from power monitor");
                        handle.send(AnyBusStatusMsg::ShuttingDown).ok();
                    }
                    psp::monitor::PowerState::ScreenLocked => {}
                    psp::monitor::PowerState::ScreenUnlocked => {}
                };
            }
        });

        // loop {

        // }
    }

    // fn handle_power_event(event: )
}
