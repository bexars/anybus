#[cfg(feature = "resume_watch")]
use psp::monitor::PowerState;

#[cfg(feature = "resume_watch")]
use crate::AnyBusStatusMsg;
#[cfg(feature = "resume_watch")]
use crate::Handle;
#[cfg(feature = "resume_watch")]
use crate::ReceiveError;

/// Watches for changes to network topology and detects resumes from suspension
#[cfg(feature = "resume_watch")]
pub(crate) struct Watcher {
    #[cfg(feature = "resume_watch")]
    handle: Handle,
}
#[cfg(feature = "resume_watch")]
impl Watcher {
    #[cfg(feature = "resume_watch")]
    pub(crate) fn new(handle: Handle) -> Self {
        Self { handle }
    }

    pub(crate) fn start(self) {
        tokio::spawn(self.run());
    }

    async fn run(self) {
        // lock down this registration so nothing else can register this UUID
        #[cfg(feature = "resume_watch")]
        let mut watcher = self
            .handle
            .get_anybus_status_receiver()
            .await
            .expect("Unable to register status receiver");

        #[cfg(feature = "resume_watch")]
        let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel::<()>();

        #[cfg(feature = "resume_watch")]
        let (tx, mut ps_rx) = tokio::sync::mpsc::unbounded_channel::<PowerState>();
        #[cfg(feature = "resume_watch")]
        let handle2 = self.handle.clone();
        #[cfg(all(
            any(target_os = "windows", target_os = "linux", target_os = "macos"),
            feature = "resume_watch"
        ))]
        tokio::task::spawn_blocking(move || {
            let handle = handle2;

            let power_monitor = psp::monitor::PowerMonitor::new();
            let pm_recv = power_monitor.event_receiver();
            if let Err(e) = power_monitor.start_listening() {
                tracing::error!("Unable to start power monitor: {}", e);
            };
            loop {
                let event = pm_recv.try_recv();
                // let clock_time = std::time::SystemTime::now();
                // dbg!(&event, clock_time);
                if let Ok(event) = event {
                    dbg!(&event);
                    tx.send(event).ok();
                    Self::handle_psp_event(event, &handle);
                }
                if kill_rx.try_recv().is_ok() {
                    tracing::info!("Killing power monitor thread");
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
        #[cfg(feature = "resume_watch")]
        let mut kill_loop = false;
        #[cfg(feature = "resume_watch")]
        loop {
            tokio::select! {

                #[cfg(feature = "resume_watch")]
                Some(event) = ps_rx.recv() => {
                    Self::handle_psp_event(event, &self.handle);
                }
                #[cfg(feature = "resume_watch")]
                status = watcher.recv() => {
                    kill_loop = Self::handle_watcher_event(status);
                }
            }
            if kill_loop {
                break;
            }
        }
        #[cfg(feature = "resume_watch")]
        kill_tx.send(()).ok();
    }

    #[cfg(feature = "resume_watch")]
    fn handle_psp_event(event: PowerState, handle: &Handle) {
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

    #[cfg(feature = "resume_watch")]
    fn handle_watcher_event(status: Result<AnyBusStatusMsg, ReceiveError>) -> bool {
        match status {
            Ok(status) => match status {
                AnyBusStatusMsg::ShuttingDown => return true,
                AnyBusStatusMsg::Resuming => {}
                AnyBusStatusMsg::Suspending => {}
                AnyBusStatusMsg::NetworkChanged => {}
            },
            Err(err) => match err {
                crate::ReceiveError::ConnectionClosed => return true,
                crate::ReceiveError::RegistrationFailed(_) => {}
                crate::ReceiveError::DeserializationError(_payload) => {}
                crate::ReceiveError::Shutdown => return true,
                crate::ReceiveError::RpcNoReplyTo => {}
            },
        }
        false
    }
}
