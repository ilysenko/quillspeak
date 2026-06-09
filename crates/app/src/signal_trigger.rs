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
use crate::recording::RecordingPhase;

pub struct SignalTriggerService {
    handle: SignalHandle,
    join_handle: Option<thread::JoinHandle<()>>,
    closed: bool,
}

impl SignalTriggerService {
    pub fn spawn(command_tx: mpsc::Sender<AppCommand>, config: &AppConfig) -> Result<Self> {
        let signal_numbers = registered_signal_numbers(config);
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
            closed: false,
        })
    }

    pub fn shutdown(&mut self) {
        if !self.closed {
            self.handle.close();
            self.closed = true;
        }
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

impl Drop for SignalTriggerService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinuxSignalAction {
    Start(String),
    Stop(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinuxSignalMatch {
    Start(String),
    Stop(String),
    StartOrStop(String),
}

pub(crate) fn registered_signal_numbers(config: &AppConfig) -> Vec<i32> {
    let mut signals = guard_signal_numbers().into_iter().collect::<HashSet<_>>();
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

pub(crate) fn linux_signal_match_for_config(
    config: &AppConfig,
    signal: i32,
) -> Option<LinuxSignalMatch> {
    for shortcut in &config.shortcuts {
        if !shortcut.enabled {
            continue;
        }

        let ShortcutTrigger::LinuxSignal {
            start_signal,
            stop_signal,
        } = &shortcut.trigger
        else {
            continue;
        };

        let (Ok(start_signal_number), Ok(stop_signal_number)) = (
            resolve_signal_number(start_signal.as_str()),
            resolve_signal_number(stop_signal.as_str()),
        ) else {
            continue;
        };

        if start_signal_number == stop_signal_number && signal == start_signal_number {
            return Some(LinuxSignalMatch::StartOrStop(shortcut.id.clone()));
        }
        if signal == start_signal_number {
            return Some(LinuxSignalMatch::Start(shortcut.id.clone()));
        }
        if signal == stop_signal_number {
            return Some(LinuxSignalMatch::Stop(shortcut.id.clone()));
        }
    }

    None
}

pub(crate) fn linux_signal_action_for_recording_state(
    signal_match: LinuxSignalMatch,
    phase: RecordingPhase,
    active_shortcut_id: Option<&str>,
) -> Option<LinuxSignalAction> {
    match signal_match {
        LinuxSignalMatch::Start(shortcut_id) => Some(LinuxSignalAction::Start(shortcut_id)),
        LinuxSignalMatch::Stop(shortcut_id) => Some(LinuxSignalAction::Stop(shortcut_id)),
        LinuxSignalMatch::StartOrStop(shortcut_id) => match phase {
            RecordingPhase::Idle => Some(LinuxSignalAction::Start(shortcut_id)),
            RecordingPhase::Arming | RecordingPhase::Recording
                if active_shortcut_id == Some(shortcut_id.as_str()) =>
            {
                Some(LinuxSignalAction::Stop(shortcut_id))
            }
            RecordingPhase::Arming | RecordingPhase::Recording | RecordingPhase::Processing => None,
        },
    }
}

fn guard_signal_numbers() -> [i32; 2] {
    [SIGUSR1, SIGUSR2]
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

pub(crate) fn signal_name(signal: i32) -> Option<&'static str> {
    match signal {
        SIGUSR1 => Some("SIGUSR1"),
        SIGUSR2 => Some("SIGUSR2"),
        SIGHUP => Some("SIGHUP"),
        SIGALRM => Some("SIGALRM"),
        SIGWINCH => Some("SIGWINCH"),
        SIGINT => Some("SIGINT"),
        SIGTERM => Some("SIGTERM"),
        SIGQUIT => Some("SIGQUIT"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use shared::{DEFAULT_SHORTCUT_ID, LinuxSignal};

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

        assert_eq!(registered_signal_numbers(&config), vec![SIGUSR1, SIGUSR2]);
    }

    #[test]
    fn guard_user_signals_are_registered_for_keyboard_only_config() {
        let config = AppConfig::default();

        assert_eq!(registered_signal_numbers(&config), vec![SIGUSR1, SIGUSR2]);
    }

    #[test]
    fn signal_names_cover_guard_signals() {
        assert_eq!(signal_name(SIGUSR1), Some("SIGUSR1"));
        assert_eq!(signal_name(SIGUSR2), Some("SIGUSR2"));
        assert_eq!(signal_name(999), None);
    }

    #[test]
    fn default_linux_signals_start_and_stop_shortcut() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::default_linux_signal();

        assert_eq!(
            linux_signal_action_for_config(&config, SIGUSR1, RecordingPhase::Idle, None),
            Some(LinuxSignalAction::Start(DEFAULT_SHORTCUT_ID.to_string()))
        );
        assert_eq!(
            linux_signal_action_for_config(&config, SIGUSR2, RecordingPhase::Idle, None),
            Some(LinuxSignalAction::Stop(DEFAULT_SHORTCUT_ID.to_string()))
        );
    }

    #[test]
    fn same_start_stop_linux_signal_starts_when_idle() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::LinuxSignal {
            start_signal: LinuxSignal::sigusr2(),
            stop_signal: LinuxSignal::sigusr2(),
        };

        assert_eq!(
            linux_signal_action_for_config(&config, SIGUSR2, RecordingPhase::Idle, None),
            Some(LinuxSignalAction::Start(DEFAULT_SHORTCUT_ID.to_string()))
        );
    }

    #[test]
    fn same_start_stop_linux_signal_stops_active_shortcut() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::LinuxSignal {
            start_signal: LinuxSignal::sigusr2(),
            stop_signal: LinuxSignal::sigusr2(),
        };

        assert_eq!(
            linux_signal_action_for_config(
                &config,
                SIGUSR2,
                RecordingPhase::Arming,
                Some(DEFAULT_SHORTCUT_ID)
            ),
            Some(LinuxSignalAction::Stop(DEFAULT_SHORTCUT_ID.to_string()))
        );
        assert_eq!(
            linux_signal_action_for_config(
                &config,
                SIGUSR2,
                RecordingPhase::Recording,
                Some(DEFAULT_SHORTCUT_ID)
            ),
            Some(LinuxSignalAction::Stop(DEFAULT_SHORTCUT_ID.to_string()))
        );
    }

    #[test]
    fn same_start_stop_linux_signal_ignores_inactive_or_processing_state() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::LinuxSignal {
            start_signal: LinuxSignal::sigusr2(),
            stop_signal: LinuxSignal::sigusr2(),
        };

        assert_eq!(
            linux_signal_action_for_config(&config, SIGUSR2, RecordingPhase::Arming, Some("other")),
            None
        );
        assert_eq!(
            linux_signal_action_for_config(
                &config,
                SIGUSR2,
                RecordingPhase::Processing,
                Some(DEFAULT_SHORTCUT_ID)
            ),
            None
        );
    }

    #[test]
    fn distinct_linux_signals_start_and_stop_shortcut() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::LinuxSignal {
            start_signal: LinuxSignal::sigusr1(),
            stop_signal: LinuxSignal::sigusr2(),
        };

        assert_eq!(
            linux_signal_action_for_config(&config, SIGUSR1, RecordingPhase::Idle, None),
            Some(LinuxSignalAction::Start(DEFAULT_SHORTCUT_ID.to_string()))
        );
        assert_eq!(
            linux_signal_action_for_config(&config, SIGUSR2, RecordingPhase::Idle, None),
            Some(LinuxSignalAction::Stop(DEFAULT_SHORTCUT_ID.to_string()))
        );
    }

    #[test]
    fn keyboard_shortcuts_ignore_linux_signals() {
        let config = AppConfig::default();

        assert_eq!(
            linux_signal_action_for_config(&config, SIGUSR2, RecordingPhase::Idle, None),
            None
        );
    }

    fn linux_signal_action_for_config(
        config: &AppConfig,
        signal: i32,
        phase: RecordingPhase,
        active_shortcut_id: Option<&str>,
    ) -> Option<LinuxSignalAction> {
        let signal_match = linux_signal_match_for_config(config, signal)?;
        linux_signal_action_for_recording_state(signal_match, phase, active_shortcut_id)
    }
}
