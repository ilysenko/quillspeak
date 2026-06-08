use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result, anyhow};
use shared::LinuxSignalName;
use signal_hook::consts::signal::{SIGUSR1, SIGUSR2};
use signal_hook::iterator::{Handle as SignalHandle, Signals};
use tracing::{debug, warn};

use crate::command::AppCommand;

pub struct SignalTriggerService {
    handle: SignalHandle,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl SignalTriggerService {
    pub fn spawn(command_tx: mpsc::Sender<AppCommand>) -> Result<Self> {
        let mut signals = Signals::new([SIGUSR1, SIGUSR2])
            .context("failed to register Linux signal trigger handlers")?;
        let handle = signals.handle();
        let join_handle = thread::Builder::new()
            .name("myapp-signal-trigger".to_string())
            .spawn(move || {
                for signal in signals.forever() {
                    let Some(signal) = linux_signal_name(signal) else {
                        warn!(signal, "received unsupported Linux signal trigger");
                        continue;
                    };
                    debug!(signal = signal.as_str(), "received Linux signal trigger");
                    if command_tx
                        .send(AppCommand::LinuxSignalReceived(signal))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|error| anyhow!("failed to spawn Linux signal trigger thread: {error}"))?;

        Ok(Self {
            handle,
            join_handle: Some(join_handle),
        })
    }

    pub fn shutdown(mut self) {
        self.handle.close();
        if let Some(join_handle) = self.join_handle.take()
            && let Err(error) = join_handle.join()
        {
            warn!(
                ?error,
                "Linux signal trigger thread panicked during shutdown"
            );
        }
    }
}

fn linux_signal_name(signal: i32) -> Option<LinuxSignalName> {
    match signal {
        SIGUSR1 => Some(LinuxSignalName::SigUsr1),
        SIGUSR2 => Some(LinuxSignalName::SigUsr2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_registered_signal_numbers_to_names() {
        assert_eq!(linux_signal_name(SIGUSR1), Some(LinuxSignalName::SigUsr1));
        assert_eq!(linux_signal_name(SIGUSR2), Some(LinuxSignalName::SigUsr2));
        assert_eq!(linux_signal_name(0), None);
    }
}
