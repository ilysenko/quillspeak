use std::env;
use std::ffi::OsString;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result, anyhow, bail};
use tracing::{debug, info, warn};

pub struct SpeakerMuteService {
    worker_tx: mpsc::Sender<SpeakerMuteCommand>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl SpeakerMuteService {
    pub fn spawn() -> Result<Self> {
        let (worker_tx, worker_rx) = mpsc::channel();
        let join_handle = thread::Builder::new()
            .name("myapp-speaker-mute".to_string())
            .spawn(move || speaker_mute_worker_loop(worker_rx))
            .map_err(|error| anyhow!("failed to spawn speaker mute worker: {error}"))?;
        Ok(Self {
            worker_tx,
            join_handle: Some(join_handle),
        })
    }

    pub fn mute_for_recording(&self, recording_id: u64, shortcut_id: &str) -> Result<()> {
        self.worker_tx
            .send(SpeakerMuteCommand::Mute {
                recording_id,
                shortcut_id: shortcut_id.to_string(),
            })
            .map_err(|_| anyhow!("speaker mute worker is not running"))
    }

    pub fn restore_for_recording(&self, recording_id: u64, shortcut_id: &str) -> Result<()> {
        self.worker_tx
            .send(SpeakerMuteCommand::Restore {
                recording_id,
                shortcut_id: shortcut_id.to_string(),
            })
            .map_err(|_| anyhow!("speaker mute worker is not running"))
    }

    pub fn shutdown(mut self) {
        self.shutdown_inner();
    }

    fn shutdown_inner(&mut self) {
        let _ = self.worker_tx.send(SpeakerMuteCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take()
            && let Err(error) = join_handle.join()
        {
            warn!(?error, "speaker mute worker panicked during shutdown");
        }
    }
}

impl Drop for SpeakerMuteService {
    fn drop(&mut self) {
        self.shutdown_inner();
    }
}

enum SpeakerMuteCommand {
    Mute {
        recording_id: u64,
        shortcut_id: String,
    },
    Restore {
        recording_id: u64,
        shortcut_id: String,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveSpeakerMute {
    recording_id: u64,
    shortcut_id: String,
    backend: SpeakerMuteBackend,
    previous_muted: bool,
    changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpeakerMuteBackend {
    Wpctl,
    Pactl,
}

impl SpeakerMuteBackend {
    const fn command(self) -> &'static str {
        match self {
            Self::Wpctl => "wpctl",
            Self::Pactl => "pactl",
        }
    }

    const fn package_hint(self) -> &'static str {
        match self {
            Self::Wpctl => "wireplumber",
            Self::Pactl => "pulseaudio-utils",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Wpctl => "wpctl",
            Self::Pactl => "pactl",
        }
    }
}

pub fn speaker_mute_tools_status() -> String {
    if command_in_path("wpctl") {
        return "Ready: wpctl".to_string();
    }
    if command_in_path("pactl") {
        return "Ready: pactl".to_string();
    }
    "Missing: wpctl or pactl".to_string()
}

fn speaker_mute_worker_loop(worker_rx: mpsc::Receiver<SpeakerMuteCommand>) {
    let mut active: Option<ActiveSpeakerMute> = None;
    for command in worker_rx {
        match command {
            SpeakerMuteCommand::Mute {
                recording_id,
                shortcut_id,
            } => mute_default_sink_for_recording(&mut active, recording_id, shortcut_id),
            SpeakerMuteCommand::Restore {
                recording_id,
                shortcut_id,
            } => restore_default_sink_for_recording(&mut active, recording_id, &shortcut_id),
            SpeakerMuteCommand::Shutdown => {
                restore_active_default_sink(&mut active, "shutdown");
                break;
            }
        }
    }
    restore_active_default_sink(&mut active, "worker exit");
}

fn mute_default_sink_for_recording(
    active: &mut Option<ActiveSpeakerMute>,
    recording_id: u64,
    shortcut_id: String,
) {
    if let Some(active) = active.as_ref() {
        warn!(
            active_recording_id = active.recording_id,
            active_shortcut_id = %active.shortcut_id,
            recording_id,
            shortcut_id,
            "speaker mute request ignored because another recording owns the mute state"
        );
        return;
    }

    match mute_default_sink(recording_id, &shortcut_id) {
        Ok(mute) => *active = Some(mute),
        Err(error) => warn!(
            ?error,
            recording_id, shortcut_id, "failed to mute speakers for recording"
        ),
    }
}

fn restore_default_sink_for_recording(
    active: &mut Option<ActiveSpeakerMute>,
    recording_id: u64,
    shortcut_id: &str,
) {
    let Some(current) = active.as_ref() else {
        debug!(
            recording_id,
            shortcut_id, "speaker mute restore ignored because no mute is active"
        );
        return;
    };
    if current.recording_id != recording_id || current.shortcut_id != shortcut_id {
        debug!(
            active_recording_id = current.recording_id,
            active_shortcut_id = %current.shortcut_id,
            recording_id,
            shortcut_id, "speaker mute restore ignored for inactive recording"
        );
        return;
    }

    restore_active_default_sink(active, "recording finished");
}

fn mute_default_sink(recording_id: u64, shortcut_id: &str) -> Result<ActiveSpeakerMute> {
    let backend = detect_speaker_mute_backend()?;
    let previous_muted = read_default_sink_muted(backend)?;
    let changed = !previous_muted;
    if changed {
        set_default_sink_muted(backend, true)?;
    }

    info!(
        recording_id,
        shortcut_id,
        backend = backend.label(),
        previous_muted,
        changed,
        "speakers muted for recording"
    );

    Ok(ActiveSpeakerMute {
        recording_id,
        shortcut_id: shortcut_id.to_string(),
        backend,
        previous_muted,
        changed,
    })
}

fn restore_active_default_sink(active: &mut Option<ActiveSpeakerMute>, reason: &str) {
    let Some(current) = active.take() else {
        return;
    };
    if !current.changed {
        debug!(
            recording_id = current.recording_id,
            shortcut_id = %current.shortcut_id,
            backend = current.backend.label(),
            reason,
            "speaker mute restore skipped because sink was already muted"
        );
        return;
    }

    match set_default_sink_muted(current.backend, current.previous_muted) {
        Ok(()) => info!(
            recording_id = current.recording_id,
            shortcut_id = %current.shortcut_id,
            backend = current.backend.label(),
            restored_muted = current.previous_muted,
            reason,
            "speaker mute state restored"
        ),
        Err(error) => warn!(
            ?error,
            recording_id = current.recording_id,
            shortcut_id = %current.shortcut_id,
            backend = current.backend.label(),
            reason,
            "failed to restore speaker mute state"
        ),
    }
}

fn detect_speaker_mute_backend() -> Result<SpeakerMuteBackend> {
    if command_in_path(SpeakerMuteBackend::Wpctl.command()) {
        return Ok(SpeakerMuteBackend::Wpctl);
    }
    if command_in_path(SpeakerMuteBackend::Pactl.command()) {
        return Ok(SpeakerMuteBackend::Pactl);
    }
    bail!(
        "no speaker mute backend found; install wireplumber for wpctl or pulseaudio-utils for pactl"
    )
}

fn read_default_sink_muted(backend: SpeakerMuteBackend) -> Result<bool> {
    match backend {
        SpeakerMuteBackend::Wpctl => {
            let output = run_command_output("wpctl", ["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
            parse_wpctl_muted(&output)
        }
        SpeakerMuteBackend::Pactl => {
            let output = run_command_output("pactl", ["get-sink-mute", "@DEFAULT_SINK@"])?;
            parse_pactl_muted(&output)
        }
    }
}

fn set_default_sink_muted(backend: SpeakerMuteBackend, muted: bool) -> Result<()> {
    match backend {
        SpeakerMuteBackend::Wpctl => run_command_status(
            "wpctl",
            ["set-mute", "@DEFAULT_AUDIO_SINK@", mute_arg(muted)],
        ),
        SpeakerMuteBackend::Pactl => run_command_status(
            "pactl",
            ["set-sink-mute", "@DEFAULT_SINK@", mute_arg(muted)],
        ),
    }
    .with_context(|| {
        format!(
            "failed to set default speaker mute through {} ({})",
            backend.command(),
            backend.package_hint()
        )
    })
}

fn mute_arg(muted: bool) -> &'static str {
    if muted { "1" } else { "0" }
}

fn run_command_output<const N: usize>(command: &str, args: [&str; N]) -> Result<String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn {command}"))?;
    if !output.status.success() {
        bail!(
            "{command} exited with status {}; stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).with_context(|| format!("{command} output was not UTF-8"))
}

fn run_command_status<const N: usize>(command: &str, args: [&str; N]) -> Result<()> {
    let output = Command::new(command)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn {command}"))?;
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "{command} exited with status {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn parse_wpctl_muted(output: &str) -> Result<bool> {
    let line = output.trim();
    if line.is_empty() {
        bail!("wpctl returned empty mute status");
    }
    Ok(line.contains("[MUTED]"))
}

fn parse_pactl_muted(output: &str) -> Result<bool> {
    let value = output
        .trim()
        .strip_prefix("Mute:")
        .map(str::trim)
        .context("pactl mute status did not start with 'Mute:'")?;
    match value {
        "yes" => Ok(true),
        "no" => Ok(false),
        other => bail!("unsupported pactl mute status {other:?}"),
    }
}

fn command_in_path(command: &str) -> bool {
    command_in_path_with(command, |name| env::var_os(name))
}

fn command_in_path_with<F>(command: &str, get_env: F) -> bool
where
    F: Fn(&str) -> Option<OsString>,
{
    let Some(path) = get_env("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|entry| entry.join(command).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wpctl_muted_state() {
        assert!(!parse_wpctl_muted("Volume: 0.35\n").unwrap());
        assert!(parse_wpctl_muted("Volume: 0.35 [MUTED]\n").unwrap());
    }

    #[test]
    fn rejects_empty_wpctl_status() {
        assert!(parse_wpctl_muted("").is_err());
    }

    #[test]
    fn parses_pactl_muted_state() {
        assert!(parse_pactl_muted("Mute: yes\n").unwrap());
        assert!(!parse_pactl_muted("Mute: no\n").unwrap());
    }

    #[test]
    fn rejects_unknown_pactl_status() {
        assert!(parse_pactl_muted("Mute: maybe\n").is_err());
        assert!(parse_pactl_muted("Volume: 50%\n").is_err());
    }
}
