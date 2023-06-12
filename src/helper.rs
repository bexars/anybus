//! Helpers
//! * [ShutdownWithCtrlC] - Wrapper around [BusControlHandle] to intercept Ctrl-C

use tokio::{
    select,
    sync::oneshot::{Receiver, Sender},
};

use crate::BusControlHandle;

/// Wrapper struct for handling Ctrl-C input from the terminal.  Receiving Ctrl-C will trigger this to call [BusControlHandle::shutdown()]
///
/// * Unix users should mind the caveat from the Tokio implementation of [tokio::signal::ctrl_c]
pub struct ShutdownWithCtrlC {
    tx: Sender<()>,
}

impl ShutdownWithCtrlC {
    /// Programatically trigger a shutdown of the [crate::MsgBus] system.  This passes through to [crate::BusControlHandle::shutdown()]
    pub fn shutdown(self) {
        let _ = self.tx.send(());
    }
}

impl From<BusControlHandle> for ShutdownWithCtrlC {
    fn from(handle: BusControlHandle) -> Self {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let this = Self { tx };

        tokio::spawn(watch_ctrlc(rx, handle));
        this
    }
}

async fn watch_ctrlc(mut rx: Receiver<()>, mut handle: BusControlHandle) {
    loop {
        select! {
            _ = &mut rx => {
                // This should probably never get hit since it should only come from this function calling for it
                handle.shutdown();
                break;
            },
            _ = tokio::signal::ctrl_c() => {
                println!("Ctrl-C received.  Shutting down");
                handle.shutdown();
                break;
            }

        }
    }
}
