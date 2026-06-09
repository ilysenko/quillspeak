use std::collections::HashSet;
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result, anyhow};
use shared::{AppConfig, ShortcutTrigger};
use signal_hook::consts::FORBIDDEN;
use signal_hook::consts::signal::{
    SIGALRM, SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGUSR1, SIGUSR2, SIGWINCH,
};
use signal_hook::iterator::{Handle as SignalHandle, Signals};
use tracing::{debug, info, warn};

use crate::command::AppCommand;

pub struct SignalTriggerService {
    handle: SignalHandle,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl SignalTriggerService {
    pub fn spawn(command_tx: mpsc::Sender<AppCommand>, config: &AppConfig) -> Result<Self> {
        let signal_numbers = configured_signal_numbers(config);
        let mut signals = Signals::new(signal_numbers.iter().copied())
            .context("failed to register Linux signal trigger handlers")?;
        let handle = signals.handle();
        let join_handle = thread::Builder::new()
            .name("myapp-signal-trigger".to_string())
            .spawn(move || {
                for signal in signals.forever() {
                    debug!(signal, "received Linux signal trigger");
                    if command_tx
                        .send(AppCommand::LinuxSignalReceived(signal))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|error| anyhow!("failed to spawn Linux signal trigger thread: {error}"))?;
        info!(
            signals = ?signal_numbers,
            "Linux signal trigger listener configured"
        );

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

pub(crate) fn configured_signal_numbers(config: &AppConfig) -> Vec<i32> {
    let mut signals = HashSet::new();
    for shortcut in config.enabled_shortcuts() {
        let ShortcutTrigger::LinuxSignal {
            start_signal,
            stop_signal,
        } = &shortcut.trigger
        else {
            continue;
        };

        for signal in [start_signal.as_str(), stop_signal.as_str()] {
            match resolve_signal_number(signal) {
                Ok(signal_number) if is_registerable_signal(signal_number) => {
                    signals.insert(signal_number);
                }
                Ok(signal_number) => warn!(
                    shortcut_id = %shortcut.id,
                    signal,
                    signal_number,
                    "Linux signal trigger uses a reserved or forbidden signal"
                ),
                Err(error) => warn!(
                    shortcut_id = %shortcut.id,
                    signal,
                    ?error,
                    "Linux signal trigger could not be resolved"
                ),
            }
        }
    }
    let mut signals = signals.into_iter().collect::<Vec<_>>();
    signals.sort_unstable();
    signals
}

pub(crate) fn resolve_signal_number(input: &str) -> Result<i32> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("signal is empty"));
    }
    if let Ok(number) = trimmed.parse::<i32>() {
        if number <= 0 {
            return Err(anyhow!("signal number must be positive"));
        }
        return Ok(number);
    }

    let mut name = trimmed.to_ascii_uppercase();
    name.retain(|character| {
        !character.is_ascii_whitespace() && character != '_' && character != '-'
    });
    if let Some(rest) = name.strip_prefix("SIG") {
        name = rest.to_string();
    }

    match name.as_str() {
        "USR1" | "USER1" => Ok(SIGUSR1),
        "USR2" | "USER2" => Ok(SIGUSR2),
        "HUP" => Ok(SIGHUP),
        "ALRM" | "ALARM" => Ok(SIGALRM),
        "WINCH" => Ok(SIGWINCH),
        "INT" => Ok(SIGINT),
        "TERM" => Ok(SIGTERM),
        "QUIT" => Ok(SIGQUIT),
        other => Err(anyhow!("unsupported Linux signal: {other}")),
    }
}

pub(crate) fn is_registerable_signal(signal: i32) -> bool {
    !FORBIDDEN.contains(&signal) && !matches!(signal, SIGINT | SIGTERM | SIGQUIT | SIGHUP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_registered_signal_numbers_to_names() {
        assert_eq!(resolve_signal_number("SIGUSR1").unwrap(), SIGUSR1);
        assert_eq!(resolve_signal_number("User 2").unwrap(), SIGUSR2);
        assert_eq!(resolve_signal_number("12").unwrap(), 12);
        assert!(resolve_signal_number("nope").is_err());
        assert!(resolve_signal_number("0").is_err());
    }

    #[test]
    fn skips_unresolved_and_reserved_configured_signals() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::LinuxSignal {
            start_signal: shared::LinuxSignal::new("SIGUSR1"),
            stop_signal: shared::LinuxSignal::new("SIGTERM"),
        };

        assert_eq!(configured_signal_numbers(&config), vec![SIGUSR1]);
    }
}
