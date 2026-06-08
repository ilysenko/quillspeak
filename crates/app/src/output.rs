use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use shared::{OutputAction, PasteOutput, ScriptOutput};
use tracing::{debug, info, warn};

use crate::command::AppCommand;
use crate::transcription::{TranscriptionResult, TranscriptionStatus};

const SCRIPT_TIMEOUT: Duration = Duration::from_secs(30);
const SCRIPT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(3);
const WAYLAND_TEXT_MIME: &str = "text/plain;charset=utf-8";

pub struct OutputService {
    worker_tx: mpsc::Sender<OutputWorkerCommand>,
    join_handle: Option<thread::JoinHandle<()>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputScriptResult {
    pub script_path: String,
    pub clipboard_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardCopyOutcome {
    pub backend: ClipboardBackend,
    pub text_chars: usize,
    pub text_bytes: usize,
}

impl OutputService {
    pub fn spawn(command_tx: mpsc::Sender<AppCommand>) -> Result<Self> {
        let (worker_tx, worker_rx) = mpsc::channel();
        let join_handle = thread::Builder::new()
            .name("myapp-output".to_string())
            .spawn(move || output_worker_loop(worker_rx, command_tx))
            .map_err(|error| anyhow!("failed to spawn output worker: {error}"))?;
        Ok(Self {
            worker_tx,
            join_handle: Some(join_handle),
        })
    }

    pub fn apply(&self, shortcut_id: &str, result: &TranscriptionResult) {
        if let TranscriptionStatus::Skipped { reason } = result.status {
            info!(
                shortcut_id,
                model_id = %result.debug.model_id,
                language = %result.debug.language,
                reason = reason.label(),
                "Skipping output action because transcription was skipped"
            );
            return;
        }

        let text = result.text.trim();
        if text.is_empty() {
            info!(
                shortcut_id,
                model_id = %result.debug.model_id,
                language = %result.debug.language,
                "Skipping output action because recognized text is empty"
            );
            return;
        }

        self.apply_clipboard_if_enabled(shortcut_id, &result.output, text);
        if let Some(script) = &result.output.script {
            self.run_script(shortcut_id, script, text);
        }
    }

    pub fn shutdown(mut self) {
        let _ = self.worker_tx.send(OutputWorkerCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take()
            && let Err(error) = join_handle.join()
        {
            warn!(?error, "output worker panicked during shutdown");
        }
    }

    fn run_script(&self, shortcut_id: &str, script: &ScriptOutput, text: &str) {
        let command = OutputWorkerCommand::RunScript {
            shortcut_id: shortcut_id.to_string(),
            script_path: script.path.clone(),
            text: text.to_string(),
            copy_stdout_to_clipboard: script.copy_stdout_to_clipboard,
        };
        if self.worker_tx.send(command).is_err() {
            warn!(shortcut_id, script = %script.path, "output worker is not running");
        }
    }

    pub fn copy_to_clipboard(&self, source: ClipboardCopySource, text: String) -> Result<()> {
        let command = OutputWorkerCommand::CopyClipboard { source, text };
        self.worker_tx
            .send(command)
            .map_err(|_| anyhow!("output worker is not running"))
    }

    fn apply_clipboard_if_enabled(&self, shortcut_id: &str, output: &OutputAction, text: &str) {
        let Some(text) = clipboard_text_for_output(output, text) else {
            return;
        };

        match self.copy_to_clipboard(
            ClipboardCopySource::Transcription {
                shortcut_id: shortcut_id.to_string(),
                paste: output.paste.clone(),
            },
            text.to_string(),
        ) {
            Ok(()) => debug!(shortcut_id, "queued transcription clipboard copy"),
            Err(error) => warn!(?error, shortcut_id, "failed to queue clipboard copy"),
        }
    }
}

fn clipboard_text_for_output<'a>(output: &OutputAction, text: &'a str) -> Option<&'a str> {
    if !output.copy_to_clipboard {
        return None;
    }
    let text = text.trim();
    (!text.is_empty()).then_some(text)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardCopySource {
    Transcription {
        shortcut_id: String,
        paste: Option<PasteOutput>,
    },
    ScriptStdout {
        shortcut_id: String,
        script_path: String,
    },
}

impl ClipboardCopySource {
    pub(crate) fn shortcut_id(&self) -> &str {
        match self {
            Self::Transcription { shortcut_id, .. } | Self::ScriptStdout { shortcut_id, .. } => {
                shortcut_id
            }
        }
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Transcription { .. } => "transcription",
            Self::ScriptStdout { .. } => "script_stdout",
        }
    }

    pub(crate) fn script_path(&self) -> Option<&str> {
        match self {
            Self::Transcription { .. } => None,
            Self::ScriptStdout { script_path, .. } => Some(script_path),
        }
    }

    pub(crate) fn paste(&self) -> Option<&PasteOutput> {
        match self {
            Self::Transcription { paste, .. } => paste.as_ref(),
            Self::ScriptStdout { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardBackend {
    Wayland,
    X11,
}

impl ClipboardBackend {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Wayland => "wayland-wl-copy",
            Self::X11 => "x11-xclip",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClipboardCommands {
    copy: &'static str,
    paste: &'static str,
    package_hint: &'static str,
}

enum OutputWorkerCommand {
    RunScript {
        shortcut_id: String,
        script_path: String,
        text: String,
        copy_stdout_to_clipboard: bool,
    },
    CopyClipboard {
        source: ClipboardCopySource,
        text: String,
    },
    Shutdown,
}

fn output_worker_loop(
    worker_rx: mpsc::Receiver<OutputWorkerCommand>,
    command_tx: mpsc::Sender<AppCommand>,
) {
    for command in worker_rx {
        match command {
            OutputWorkerCommand::RunScript {
                shortcut_id,
                script_path,
                text,
                copy_stdout_to_clipboard,
            } => {
                let result = run_script(&script_path, &text, copy_stdout_to_clipboard)
                    .map_err(|error| format!("{error:#}"));
                let _ = command_tx.send(AppCommand::OutputScriptFinished {
                    shortcut_id,
                    result,
                });
            }
            OutputWorkerCommand::CopyClipboard { source, text } => {
                let result =
                    copy_text_to_external_clipboard(&text).map_err(|error| format!("{error:#}"));
                let _ = command_tx.send(AppCommand::ClipboardCopyFinished { source, result });
            }
            OutputWorkerCommand::Shutdown => break,
        }
    }
}

fn copy_text_to_external_clipboard(text: &str) -> Result<ClipboardCopyOutcome> {
    let backend = detect_clipboard_backend_from_env(|name| std::env::var_os(name))?;
    match backend {
        ClipboardBackend::Wayland => copy_text_to_wayland_clipboard(text)?,
        ClipboardBackend::X11 => copy_text_to_x11_clipboard(text)?,
    }
    verify_external_clipboard(backend, text)?;

    Ok(ClipboardCopyOutcome {
        backend,
        text_chars: text.chars().count(),
        text_bytes: text.len(),
    })
}

fn detect_clipboard_backend_from_env<F>(get_env: F) -> Result<ClipboardBackend>
where
    F: Fn(&str) -> Option<OsString>,
{
    let wayland_display = non_empty_env(get_env("WAYLAND_DISPLAY"));
    let wayland_session = get_env("XDG_SESSION_TYPE")
        .and_then(|value| value.into_string().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("wayland"));
    if wayland_display || wayland_session {
        return Ok(ClipboardBackend::Wayland);
    }

    if non_empty_env(get_env("DISPLAY")) {
        return Ok(ClipboardBackend::X11);
    }

    bail!("no supported clipboard backend detected; WAYLAND_DISPLAY and DISPLAY are unset")
}

fn non_empty_env(value: Option<OsString>) -> bool {
    value.is_some_and(|value| !value.as_os_str().is_empty())
}

fn copy_text_to_wayland_clipboard(text: &str) -> Result<()> {
    let commands = clipboard_commands(ClipboardBackend::Wayland);
    ensure_command_in_path(commands.copy, commands.package_hint)?;
    let mut command = Command::new(commands.copy);
    command.args(wayland_copy_args());
    run_status_with_stdin(&mut command, text, CLIPBOARD_TIMEOUT, "wl-copy")
}

fn copy_text_to_x11_clipboard(text: &str) -> Result<()> {
    let commands = clipboard_commands(ClipboardBackend::X11);
    ensure_command_in_path(commands.copy, commands.package_hint)?;
    let mut command = Command::new(commands.copy);
    command.args(["-selection", "clipboard", "-in"]);
    run_status_with_stdin(
        &mut command,
        text,
        CLIPBOARD_TIMEOUT,
        "xclip clipboard copy",
    )
}

fn verify_external_clipboard(backend: ClipboardBackend, expected_text: &str) -> Result<()> {
    let actual_text = read_external_clipboard(backend)?;
    if actual_text == expected_text {
        return Ok(());
    }

    bail!(
        "clipboard verification mismatch for {}: expected {} chars/{} bytes, got {} chars/{} bytes",
        backend.as_str(),
        expected_text.chars().count(),
        expected_text.len(),
        actual_text.chars().count(),
        actual_text.len()
    );
}

fn read_external_clipboard(backend: ClipboardBackend) -> Result<String> {
    let output = match backend {
        ClipboardBackend::Wayland => {
            let commands = clipboard_commands(backend);
            ensure_command_in_path(commands.paste, commands.package_hint)?;
            let mut command = Command::new(commands.paste);
            command.args(wayland_paste_args());
            run_output_with_timeout(&mut command, CLIPBOARD_TIMEOUT, "wl-paste")?
        }
        ClipboardBackend::X11 => {
            let commands = clipboard_commands(backend);
            ensure_command_in_path(commands.paste, commands.package_hint)?;
            let mut command = Command::new(commands.paste);
            command.args(["-selection", "clipboard", "-out"]);
            run_output_with_timeout(&mut command, CLIPBOARD_TIMEOUT, "xclip clipboard read")?
        }
    };

    if !output.status.success() {
        bail!(
            "{} clipboard read exited with status {}; stderr: {}",
            backend.as_str(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    String::from_utf8(output.stdout)
        .with_context(|| format!("{} clipboard text was not UTF-8", backend.as_str()))
}

fn clipboard_commands(backend: ClipboardBackend) -> ClipboardCommands {
    match backend {
        ClipboardBackend::Wayland => ClipboardCommands {
            copy: "wl-copy",
            paste: "wl-paste",
            package_hint: "wl-clipboard",
        },
        ClipboardBackend::X11 => ClipboardCommands {
            copy: "xclip",
            paste: "xclip",
            package_hint: "xclip",
        },
    }
}

fn wayland_copy_args() -> [&'static str; 3] {
    ["--type", WAYLAND_TEXT_MIME, "--"]
}

fn wayland_paste_args() -> [&'static str; 3] {
    ["--no-newline", "--type", WAYLAND_TEXT_MIME]
}

fn run_status_with_stdin(
    command: &mut Command,
    text: &str,
    timeout: Duration,
    description: &str,
) -> Result<()> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to spawn {description}"))?;

    let write_result = child
        .stdin
        .take()
        .context("failed to open child stdin")?
        .write_all(text.as_bytes())
        .with_context(|| format!("failed to write text to {description} stdin"));
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }

    let status = wait_for_child_status(&mut child, timeout, description)?;
    if !status.success() {
        bail!("{description} exited with status {status}");
    }

    Ok(())
}

fn run_output_with_timeout(
    command: &mut Command,
    timeout: Duration,
    description: &str,
) -> Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {description}"))?;

    let started_at = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return child
                .wait_with_output()
                .with_context(|| format!("failed to collect {description} output"));
        }

        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{description} timed out after {timeout:?}");
        }

        thread::sleep(SCRIPT_POLL_INTERVAL);
    }
}

fn wait_for_child_status(
    child: &mut std::process::Child,
    timeout: Duration,
    description: &str,
) -> Result<std::process::ExitStatus> {
    let started_at = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }

        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{description} timed out after {timeout:?}");
        }

        thread::sleep(SCRIPT_POLL_INTERVAL);
    }
}

fn ensure_command_in_path(command: &str, package_hint: &str) -> Result<()> {
    if command_in_path(command) {
        return Ok(());
    }

    bail!("{command} not found in PATH; install {package_hint}")
}

fn command_in_path(command: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    command_in_path_entries(command, std::env::split_paths(&path).collect())
}

fn command_in_path_entries(command: &str, entries: Vec<PathBuf>) -> bool {
    entries.iter().any(|entry| entry.join(command).is_file())
}

fn run_script(
    script_path: &str,
    text: &str,
    copy_stdout_to_clipboard: bool,
) -> Result<OutputScriptResult> {
    let output = run_script_with_timeout(script_path, text)?;
    if !output.status.success() {
        bail!(
            "script {} exited with status {}; stderr: {}",
            script_path,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let clipboard_text = if copy_stdout_to_clipboard {
        Some(
            String::from_utf8(output.stdout)
                .with_context(|| format!("script {script_path} stdout was not UTF-8"))?,
        )
    } else {
        None
    };

    Ok(OutputScriptResult {
        script_path: script_path.to_string(),
        clipboard_text,
    })
}

fn run_script_with_timeout(script_path: &str, text: &str) -> Result<std::process::Output> {
    let mut child = Command::new(script_path)
        .arg(text)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn output script {script_path}"))?;

    let started_at = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return child
                .wait_with_output()
                .with_context(|| format!("failed to collect output script {script_path} output"));
        }

        if started_at.elapsed() >= SCRIPT_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            bail!("script {script_path} timed out after {SCRIPT_TIMEOUT:?}");
        }

        thread::sleep(SCRIPT_POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::time::{SystemTime, UNIX_EPOCH};

    use shared::{OutputAction, PasteOutput, PasteShortcut, ScriptOutput};

    use super::*;

    #[test]
    fn clipboard_disabled_when_output_does_not_request_it() {
        let output = OutputAction {
            copy_to_clipboard: false,
            paste: None,
            script: None,
        };

        assert!(!output.copy_to_clipboard);
    }

    #[test]
    fn script_output_result_carries_clipboard_text() {
        let result = OutputScriptResult {
            script_path: "/bin/echo".to_string(),
            clipboard_text: Some("hello\n".to_string()),
        };

        assert_eq!(result.clipboard_text.as_deref(), Some("hello\n"));
    }

    #[test]
    fn output_action_can_request_script_stdout_clipboard_copy() {
        let output = OutputAction {
            copy_to_clipboard: true,
            paste: None,
            script: Some(ScriptOutput {
                path: "/bin/echo".to_string(),
                copy_stdout_to_clipboard: true,
            }),
        };

        assert!(output.copy_to_clipboard);
        assert!(
            output
                .script
                .as_ref()
                .is_some_and(|script| script.copy_stdout_to_clipboard)
        );
    }

    #[test]
    fn clipboard_text_for_output_returns_trimmed_non_empty_text() {
        let output = OutputAction::default();

        assert_eq!(
            clipboard_text_for_output(&output, "  hello clipboard  "),
            Some("hello clipboard")
        );
    }

    #[test]
    fn clipboard_text_for_output_skips_empty_text() {
        let output = OutputAction::default();

        assert_eq!(clipboard_text_for_output(&output, "   "), None);
    }

    #[test]
    fn clipboard_text_for_output_skips_disabled_clipboard() {
        let output = OutputAction {
            copy_to_clipboard: false,
            paste: None,
            script: None,
        };

        assert_eq!(clipboard_text_for_output(&output, "hello"), None);
    }

    #[test]
    fn clipboard_copy_source_tracks_transcription_context() {
        let source = ClipboardCopySource::Transcription {
            shortcut_id: "default".to_string(),
            paste: Some(PasteOutput {
                shortcut: PasteShortcut::CtrlV,
            }),
        };

        assert_eq!(source.shortcut_id(), "default");
        assert_eq!(source.kind(), "transcription");
        assert_eq!(source.script_path(), None);
        assert_eq!(
            source.paste().map(|paste| paste.shortcut),
            Some(PasteShortcut::CtrlV)
        );
    }

    #[test]
    fn clipboard_copy_source_tracks_script_context() {
        let source = ClipboardCopySource::ScriptStdout {
            shortcut_id: "default".to_string(),
            script_path: "/tmp/script".to_string(),
        };

        assert_eq!(source.shortcut_id(), "default");
        assert_eq!(source.kind(), "script_stdout");
        assert_eq!(source.script_path(), Some("/tmp/script"));
        assert_eq!(source.paste(), None);
    }

    #[test]
    fn wayland_env_selects_wl_clipboard_backend() {
        let backend =
            detect_clipboard_backend_from_env(test_env(&[("WAYLAND_DISPLAY", "wayland-0")]))
                .expect("Wayland display should select a clipboard backend");

        assert_eq!(backend, ClipboardBackend::Wayland);
    }

    #[test]
    fn wayland_session_selects_wl_clipboard_backend() {
        let backend =
            detect_clipboard_backend_from_env(test_env(&[("XDG_SESSION_TYPE", "wayland")]))
                .expect("Wayland session should select a clipboard backend");

        assert_eq!(backend, ClipboardBackend::Wayland);
    }

    #[test]
    fn wayland_backend_uses_wl_copy_and_wl_paste() {
        let commands = clipboard_commands(ClipboardBackend::Wayland);

        assert_eq!(commands.copy, "wl-copy");
        assert_eq!(commands.paste, "wl-paste");
        assert_eq!(wayland_copy_args(), ["--type", WAYLAND_TEXT_MIME, "--"]);
        assert_eq!(
            wayland_paste_args(),
            ["--no-newline", "--type", WAYLAND_TEXT_MIME]
        );
    }

    #[test]
    fn x11_env_selects_xclip_backend() {
        let backend = detect_clipboard_backend_from_env(test_env(&[("DISPLAY", ":0")]))
            .expect("X11 display should select a clipboard backend");

        assert_eq!(backend, ClipboardBackend::X11);
    }

    #[test]
    fn missing_display_env_rejects_clipboard_backend() {
        let error = detect_clipboard_backend_from_env(test_env(&[]))
            .expect_err("missing display env should not invent a clipboard backend");

        assert!(
            error
                .to_string()
                .contains("WAYLAND_DISPLAY and DISPLAY are unset")
        );
    }

    #[test]
    #[ignore = "requires a real Wayland session and mutates the system clipboard"]
    fn wayland_clipboard_copy_round_trips_text() {
        let text = format!("myapp clipboard smoke {}", unique_test_suffix());
        let _restore_guard =
            ClipboardRestoreGuard::new(read_external_clipboard(ClipboardBackend::Wayland).ok());

        copy_text_to_external_clipboard(&text).expect("external clipboard copy should succeed");

        let backend = detect_clipboard_backend_from_env(|name| std::env::var_os(name))
            .expect("display env should select a clipboard backend");
        assert_eq!(backend, ClipboardBackend::Wayland);

        let actual = read_external_clipboard(backend).expect("clipboard text should be readable");
        assert_eq!(actual, text);
    }

    struct ClipboardRestoreGuard {
        original_text: Option<String>,
    }

    impl ClipboardRestoreGuard {
        fn new(original_text: Option<String>) -> Self {
            Self { original_text }
        }
    }

    impl Drop for ClipboardRestoreGuard {
        fn drop(&mut self) {
            if let Some(original_text) = self.original_text.take() {
                let _ = copy_text_to_external_clipboard(&original_text);
            }
        }
    }

    fn test_env<'a>(values: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(*value))
        }
    }

    fn unique_test_suffix() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos()
    }
}
