use std::env;
use std::ffi::OsString;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::command::AppCommand;

pub struct SpeakerMuteService {
    worker_tx: mpsc::Sender<SpeakerMuteCommand>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl SpeakerMuteService {
    pub fn spawn(command_tx: mpsc::Sender<AppCommand>) -> Result<Self> {
        let (worker_tx, worker_rx) = mpsc::channel();
        let join_handle = thread::Builder::new()
            .name("myapp-speaker-mute".to_string())
            .spawn(move || speaker_mute_worker_loop(worker_rx, command_tx))
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
    targets: Vec<ActiveSpeakerMuteTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveSpeakerMuteTarget {
    target: SpeakerMuteTarget,
    previous_muted: bool,
    changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpeakerMuteBackend {
    PipeWire,
    Wpctl,
    Pactl,
}

impl SpeakerMuteBackend {
    const fn command(self) -> &'static str {
        match self {
            Self::PipeWire => "wpctl",
            Self::Wpctl => "wpctl",
            Self::Pactl => "pactl",
        }
    }

    const fn package_hint(self) -> &'static str {
        match self {
            Self::PipeWire => "wireplumber and pipewire-bin",
            Self::Wpctl => "wireplumber",
            Self::Pactl => "pulseaudio-utils",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::PipeWire => "wpctl+pw-dump",
            Self::Wpctl => "wpctl",
            Self::Pactl => "pactl",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpeakerMuteTargetKind {
    PlaybackStream,
    DefaultSink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpeakerMuteTarget {
    backend: SpeakerMuteBackend,
    target_id: String,
    label: String,
    kind: SpeakerMuteTargetKind,
}

impl SpeakerMuteTarget {
    fn pipewire_stream(id: u64, label: String) -> Self {
        Self {
            backend: SpeakerMuteBackend::PipeWire,
            target_id: id.to_string(),
            label,
            kind: SpeakerMuteTargetKind::PlaybackStream,
        }
    }

    fn wpctl_default_sink(backend: SpeakerMuteBackend) -> Self {
        Self {
            backend,
            target_id: "@DEFAULT_AUDIO_SINK@".to_string(),
            label: "Default audio sink".to_string(),
            kind: SpeakerMuteTargetKind::DefaultSink,
        }
    }

    fn pactl_default_sink() -> Self {
        Self {
            backend: SpeakerMuteBackend::Pactl,
            target_id: "@DEFAULT_SINK@".to_string(),
            label: "Default audio sink".to_string(),
            kind: SpeakerMuteTargetKind::DefaultSink,
        }
    }

    const fn kind_label(&self) -> &'static str {
        match self.kind {
            SpeakerMuteTargetKind::PlaybackStream => "playback_stream",
            SpeakerMuteTargetKind::DefaultSink => "default_sink",
        }
    }
}

pub fn speaker_mute_tools_status() -> String {
    if command_in_path("wpctl") && command_in_path("pw-dump") {
        return "Ready: wpctl + pw-dump".to_string();
    }
    if command_in_path("wpctl") {
        return "Limited: wpctl only (missing pw-dump for stream mute)".to_string();
    }
    if command_in_path("pactl") {
        return "Limited: pactl (default sink only)".to_string();
    }
    "Missing: wpctl + pw-dump or pactl".to_string()
}

fn speaker_mute_worker_loop(
    worker_rx: mpsc::Receiver<SpeakerMuteCommand>,
    command_tx: mpsc::Sender<AppCommand>,
) {
    let mut active: Option<ActiveSpeakerMute> = None;
    for command in worker_rx {
        match command {
            SpeakerMuteCommand::Mute {
                recording_id,
                shortcut_id,
            } => {
                let result =
                    mute_playback_for_recording(&mut active, recording_id, shortcut_id.clone())
                        .map_err(|error| format!("{error:#}"));
                let _ = command_tx.send(AppCommand::SpeakerMuteFinished {
                    recording_id,
                    shortcut_id,
                    result,
                });
            }
            SpeakerMuteCommand::Restore {
                recording_id,
                shortcut_id,
            } => {
                restore_playback_for_recording(&mut active, recording_id, &shortcut_id);
                let _ = command_tx.send(AppCommand::SpeakerRestoreFinished {
                    recording_id,
                    shortcut_id,
                });
            }
            SpeakerMuteCommand::Shutdown => {
                restore_active_speaker_mute(&mut active, "shutdown");
                break;
            }
        }
    }
    restore_active_speaker_mute(&mut active, "worker exit");
}

fn mute_playback_for_recording(
    active: &mut Option<ActiveSpeakerMute>,
    recording_id: u64,
    shortcut_id: String,
) -> Result<()> {
    if let Some(active) = active.as_ref() {
        bail!(
            "speaker mute request ignored because recording {} / {} owns the mute state",
            active.recording_id,
            active.shortcut_id
        );
    }

    *active = Some(mute_playback(recording_id, &shortcut_id)?);
    Ok(())
}

fn restore_playback_for_recording(
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

    restore_active_speaker_mute(active, "recording finished");
}

fn mute_playback(recording_id: u64, shortcut_id: &str) -> Result<ActiveSpeakerMute> {
    let backend = detect_speaker_mute_backend()?;
    let targets = speaker_mute_targets(backend)?;
    let mut active_targets = Vec::new();
    let mut failures = Vec::new();

    for target in targets {
        match mute_target(target.clone()) {
            Ok(active_target) => active_targets.push(active_target),
            Err(error) => {
                warn!(
                    ?error,
                    recording_id,
                    shortcut_id,
                    backend = backend.label(),
                    target = %target.target_id,
                    target_label = %target.label,
                    target_kind = target.kind_label(),
                    "failed to mute playback target"
                );
                failures.push(format!("{}: {error:#}", target.label));
            }
        }
    }

    if active_targets.is_empty() {
        if failures.is_empty() {
            bail!("no playback targets found to mute");
        }
        bail!("failed to mute playback targets: {}", failures.join("; "));
    }

    info!(
        recording_id,
        shortcut_id,
        backend = backend.label(),
        targets = active_targets.len(),
        changed = active_targets
            .iter()
            .filter(|target| target.changed)
            .count(),
        failures = failures.len(),
        "playback muted for recording"
    );

    Ok(ActiveSpeakerMute {
        recording_id,
        shortcut_id: shortcut_id.to_string(),
        targets: active_targets,
    })
}

fn restore_active_speaker_mute(active: &mut Option<ActiveSpeakerMute>, reason: &str) {
    let Some(current) = active.take() else {
        return;
    };
    let total = current.targets.len();
    let changed = current
        .targets
        .iter()
        .filter(|target| target.changed)
        .count();
    for target in current.targets.into_iter().rev() {
        if !target.changed {
            debug!(
                recording_id = current.recording_id,
                shortcut_id = %current.shortcut_id,
                backend = target.target.backend.label(),
                target = %target.target.target_id,
                target_label = %target.target.label,
                target_kind = target.target.kind_label(),
                reason,
                "speaker mute restore skipped because target was already muted"
            );
            continue;
        }

        match set_target_muted(&target.target, target.previous_muted) {
            Ok(()) => info!(
                recording_id = current.recording_id,
                shortcut_id = %current.shortcut_id,
                backend = target.target.backend.label(),
                target = %target.target.target_id,
                target_label = %target.target.label,
                target_kind = target.target.kind_label(),
                restored_muted = target.previous_muted,
                reason,
                "speaker mute target restored"
            ),
            Err(error) => warn!(
                ?error,
                recording_id = current.recording_id,
                shortcut_id = %current.shortcut_id,
                backend = target.target.backend.label(),
                target = %target.target.target_id,
                target_label = %target.target.label,
                target_kind = target.target.kind_label(),
                reason,
                "failed to restore speaker mute target"
            ),
        }
    }
    info!(
            recording_id = current.recording_id,
            shortcut_id = %current.shortcut_id,
            targets = total,
            changed,
            reason,
            "speaker mute state restored"
    );
}

fn detect_speaker_mute_backend() -> Result<SpeakerMuteBackend> {
    if command_in_path("wpctl") && command_in_path("pw-dump") {
        return Ok(SpeakerMuteBackend::PipeWire);
    }
    if command_in_path("wpctl") {
        return Ok(SpeakerMuteBackend::Wpctl);
    }
    if command_in_path("pactl") {
        return Ok(SpeakerMuteBackend::Pactl);
    }
    bail!(
        "no speaker mute backend found; install wireplumber and pipewire-bin for wpctl/pw-dump or pulseaudio-utils for pactl"
    )
}

fn speaker_mute_targets(backend: SpeakerMuteBackend) -> Result<Vec<SpeakerMuteTarget>> {
    match backend {
        SpeakerMuteBackend::PipeWire => pipewire_playback_mute_targets(),
        SpeakerMuteBackend::Wpctl => Ok(vec![SpeakerMuteTarget::wpctl_default_sink(backend)]),
        SpeakerMuteBackend::Pactl => Ok(vec![SpeakerMuteTarget::pactl_default_sink()]),
    }
}

fn pipewire_playback_mute_targets() -> Result<Vec<SpeakerMuteTarget>> {
    let output = run_command_output("pw-dump", &["--no-colors"])?;
    let mut targets = pipewire_playback_stream_targets_from_dump(&output)?;
    targets.push(SpeakerMuteTarget::wpctl_default_sink(
        SpeakerMuteBackend::PipeWire,
    ));
    Ok(targets)
}

fn mute_target(target: SpeakerMuteTarget) -> Result<ActiveSpeakerMuteTarget> {
    let previous_muted = read_target_muted(&target)?;
    let changed = !previous_muted;
    if changed {
        set_target_muted(&target, true)?;
    }

    Ok(ActiveSpeakerMuteTarget {
        target,
        previous_muted,
        changed,
    })
}

fn read_target_muted(target: &SpeakerMuteTarget) -> Result<bool> {
    match target.backend {
        SpeakerMuteBackend::PipeWire | SpeakerMuteBackend::Wpctl => {
            let output = run_command_output("wpctl", &["get-volume", target.target_id.as_str()])?;
            parse_wpctl_muted(&output)
        }
        SpeakerMuteBackend::Pactl => {
            let output =
                run_command_output("pactl", &["get-sink-mute", target.target_id.as_str()])?;
            parse_pactl_muted(&output)
        }
    }
}

fn set_target_muted(target: &SpeakerMuteTarget, muted: bool) -> Result<()> {
    match target.backend {
        SpeakerMuteBackend::PipeWire | SpeakerMuteBackend::Wpctl => run_command_status(
            "wpctl",
            &["set-mute", target.target_id.as_str(), mute_arg(muted)],
        ),
        SpeakerMuteBackend::Pactl => run_command_status(
            "pactl",
            &["set-sink-mute", target.target_id.as_str(), mute_arg(muted)],
        ),
    }
    .with_context(|| {
        format!(
            "failed to set speaker mute target {} through {} ({})",
            target.label,
            target.backend.command(),
            target.backend.package_hint()
        )
    })
}

fn mute_arg(muted: bool) -> &'static str {
    if muted { "1" } else { "0" }
}

fn pipewire_playback_stream_targets_from_dump(dump: &str) -> Result<Vec<SpeakerMuteTarget>> {
    let objects =
        serde_json::from_str::<Vec<Value>>(dump).context("failed to parse pw-dump JSON")?;
    let mut targets = Vec::new();
    for object in objects {
        if object.get("type").and_then(Value::as_str) != Some("PipeWire:Interface:Node") {
            continue;
        }
        let Some(props) = object.pointer("/info/props") else {
            continue;
        };
        if prop_str(props, "media.class") != Some("Stream/Output/Audio") {
            continue;
        }
        let id = object
            .get("id")
            .and_then(Value::as_u64)
            .context("PipeWire playback stream is missing numeric id")?;
        let label = playback_stream_label(props).unwrap_or_else(|| format!("PipeWire stream {id}"));
        targets.push(SpeakerMuteTarget::pipewire_stream(id, label));
    }
    Ok(targets)
}

fn playback_stream_label(props: &Value) -> Option<String> {
    [
        "application.name",
        "media.name",
        "node.description",
        "node.name",
    ]
    .into_iter()
    .find_map(|key| prop_str(props, key))
    .map(ToString::to_string)
}

fn prop_str<'a>(props: &'a Value, key: &str) -> Option<&'a str> {
    props
        .get(key)?
        .as_str()
        .filter(|value| !value.trim().is_empty())
}

fn run_command_output(command: &str, args: &[&str]) -> Result<String> {
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

fn run_command_status(command: &str, args: &[&str]) -> Result<()> {
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

    #[test]
    fn pipewire_dump_targets_only_audio_playback_streams() {
        let targets = pipewire_playback_stream_targets_from_dump(
            r#"
[
  {
    "id": 117,
    "type": "PipeWire:Interface:Node",
    "info": {
      "props": {
        "media.class": "Stream/Output/Audio",
        "application.name": "Google Chrome",
        "media.name": "YouTube"
      }
    }
  },
  {
    "id": 59,
    "type": "PipeWire:Interface:Node",
    "info": {
      "props": {
        "media.class": "Audio/Sink",
        "node.description": "Speakers"
      }
    }
  },
  {
    "id": 87,
    "type": "PipeWire:Interface:Node",
    "info": {
      "props": {
        "media.class": "Audio/Source",
        "node.description": "Microphone"
      }
    }
  },
  {
    "id": 66,
    "type": "PipeWire:Interface:Node",
    "info": {
      "props": {
        "media.class": "Video/Source",
        "node.description": "Camera"
      }
    }
  }
]
"#,
        )
        .expect("pw-dump fixture should parse");

        assert_eq!(
            targets,
            vec![SpeakerMuteTarget::pipewire_stream(
                117,
                "Google Chrome".to_string()
            )]
        );
    }

    #[test]
    fn pipewire_dump_uses_fallback_stream_labels() {
        let targets = pipewire_playback_stream_targets_from_dump(
            r#"
[
  {
    "id": 140,
    "type": "PipeWire:Interface:Node",
    "info": {
      "props": {
        "media.class": "Stream/Output/Audio",
        "media.name": "Browser tab"
      }
    }
  },
  {
    "id": 141,
    "type": "PipeWire:Interface:Node",
    "info": {
      "props": {
        "media.class": "Stream/Output/Audio"
      }
    }
  }
]
"#,
        )
        .expect("pw-dump fixture should parse");

        assert_eq!(
            targets,
            vec![
                SpeakerMuteTarget::pipewire_stream(140, "Browser tab".to_string()),
                SpeakerMuteTarget::pipewire_stream(141, "PipeWire stream 141".to_string()),
            ]
        );
    }
}
