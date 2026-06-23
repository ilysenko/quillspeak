use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use shared::{OutputAction, PasteShortcut, ScriptOutput};
use tracing::{debug, info, warn};

use crate::command::AppCommand;
use crate::transcription::{TranscriptionResult, TranscriptionStatus};

const SCRIPT_TIMEOUT: Duration = Duration::from_secs(30);
const SCRIPT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const SCRIPT_SPAWN_RETRY_DELAY: Duration = Duration::from_millis(10);
const SCRIPT_SPAWN_ATTEMPTS: usize = 3;
const CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(3);
const WAYLAND_TEXT_MIME: &str = "text/plain;charset=utf-8";
const TEXT_FILE_BUSY_OS_ERROR: i32 = 26;

pub struct OutputService {
    worker_tx: mpsc::Sender<OutputWorkerCommand>,
    join_handle: Option<thread::JoinHandle<()>>,
    cancel_requested: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputDelivery {
    NotQueued,
    Queued(OutputCompletion),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputCompletion {
    Script,
    ClipboardCopy,
    ClipboardPaste,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputScriptResult {
    pub script_path: String,
    pub output_text: Option<String>,
    pub output: OutputAction,
    pub shortcut_name: String,
    pub model_id: String,
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardCopyOutcome {
    pub backend: ClipboardBackend,
    pub text_chars: usize,
    pub text_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPasteOutcome {
    pub backend: ClipboardBackend,
    pub shortcut: PasteShortcut,
}

impl OutputService {
    pub fn spawn(command_tx: mpsc::Sender<AppCommand>) -> Result<Self> {
        let (worker_tx, worker_rx) = mpsc::channel();
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let worker_cancel_requested = Arc::clone(&cancel_requested);
        let join_handle = thread::Builder::new()
            .name("quillspeak-output".to_string())
            .spawn(move || output_worker_loop(worker_rx, command_tx, worker_cancel_requested))
            .map_err(|error| anyhow!("failed to spawn output worker: {error}"))?;
        Ok(Self {
            worker_tx,
            join_handle: Some(join_handle),
            cancel_requested,
        })
    }

    pub fn apply(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        result: &TranscriptionResult,
    ) -> OutputDelivery {
        if let TranscriptionStatus::Skipped { reason } = result.status {
            info!(
                shortcut_id,
                model_id = %result.debug.model_id,
                language = %result.debug.language,
                reason = reason.label(),
                "Skipping output action because transcription was skipped"
            );
            return OutputDelivery::NotQueued;
        }

        let text = result.text.trim();
        if text.is_empty() {
            info!(
                shortcut_id,
                model_id = %result.debug.model_id,
                language = %result.debug.language,
                "Skipping output action because recognized text is empty"
            );
            return OutputDelivery::NotQueued;
        }

        if let Some(script) = &result.output.script {
            self.run_script(recording_id, shortcut_id, result, script, text)
        } else {
            self.copy_final_text_if_requested(
                ClipboardCopySource::Transcription {
                    recording_id,
                    shortcut_id: shortcut_id.to_string(),
                },
                &result.output,
                text,
            )
        }
    }

    pub fn shutdown(mut self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
        let _ = self.worker_tx.send(OutputWorkerCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take()
            && let Err(error) = join_handle.join()
        {
            warn!(?error, "output worker panicked during shutdown");
        }
    }

    fn run_script(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        result: &TranscriptionResult,
        script: &ScriptOutput,
        text: &str,
    ) -> OutputDelivery {
        let command = OutputWorkerCommand::RunScript {
            recording_id,
            shortcut_id: shortcut_id.to_string(),
            script_path: script.path.clone(),
            text: text.to_string(),
            output: result.output.clone(),
            shortcut_name: result.debug.shortcut_name.clone(),
            model_id: result.debug.model_id.clone(),
            language: result.debug.language.clone(),
        };
        if self.worker_tx.send(command).is_err() {
            warn!(shortcut_id, script = %script.path, "output worker is not running");
            OutputDelivery::NotQueued
        } else {
            OutputDelivery::Queued(OutputCompletion::Script)
        }
    }

    pub fn copy_final_text_if_requested(
        &self,
        source: ClipboardCopySource,
        output: &OutputAction,
        text: &str,
    ) -> OutputDelivery {
        let Some(text) = clipboard_transport_text_for_output(output, text) else {
            return OutputDelivery::NotQueued;
        };

        let shortcut_id = source.shortcut_id().to_string();
        let source_kind = source.kind();
        let completion = clipboard_completion_for_output(output);
        let paste = output
            .paste_from_clipboard
            .then(|| PasteRequest::from(output));
        let command = OutputWorkerCommand::CopyClipboard {
            source,
            text: text.to_string(),
            paste,
        };
        match self.worker_tx.send(command) {
            Ok(()) => {
                debug!(shortcut_id, source = source_kind, "queued clipboard copy");
                OutputDelivery::Queued(completion)
            }
            Err(error) => {
                warn!(
                    ?error,
                    shortcut_id,
                    source = source_kind,
                    "failed to queue clipboard copy"
                );
                OutputDelivery::NotQueued
            }
        }
    }

    pub fn copy_history_text(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        text: &str,
    ) -> OutputDelivery {
        self.copy_final_text_if_requested(
            ClipboardCopySource::History {
                recording_id,
                shortcut_id: shortcut_id.to_string(),
            },
            &OutputAction::clipboard(),
            text,
        )
    }
}

fn clipboard_transport_text_for_output<'a>(
    output: &OutputAction,
    text: &'a str,
) -> Option<&'a str> {
    if !output.copy_to_clipboard && !output.paste_from_clipboard {
        return None;
    }
    let text = text.trim();
    (!text.is_empty()).then_some(text)
}

fn clipboard_completion_for_output(output: &OutputAction) -> OutputCompletion {
    if output.paste_from_clipboard {
        OutputCompletion::ClipboardPaste
    } else {
        OutputCompletion::ClipboardCopy
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardCopySource {
    Transcription {
        recording_id: u64,
        shortcut_id: String,
    },
    ScriptStdout {
        recording_id: u64,
        shortcut_id: String,
        script_path: String,
    },
    History {
        recording_id: u64,
        shortcut_id: String,
    },
}

impl ClipboardCopySource {
    pub(crate) fn recording_id(&self) -> u64 {
        match self {
            Self::Transcription { recording_id, .. }
            | Self::ScriptStdout { recording_id, .. }
            | Self::History { recording_id, .. } => *recording_id,
        }
    }

    pub(crate) fn shortcut_id(&self) -> &str {
        match self {
            Self::Transcription { shortcut_id, .. }
            | Self::ScriptStdout { shortcut_id, .. }
            | Self::History { shortcut_id, .. } => shortcut_id,
        }
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Transcription { .. } => "transcription",
            Self::ScriptStdout { .. } => "script_stdout",
            Self::History { .. } => "history",
        }
    }

    pub(crate) fn script_path(&self) -> Option<&str> {
        match self {
            Self::Transcription { .. } | Self::History { .. } => None,
            Self::ScriptStdout { script_path, .. } => Some(script_path),
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct PasteRequest {
    shortcut: PasteShortcut,
    custom_x11: String,
    custom_wayland: String,
}

impl From<&OutputAction> for PasteRequest {
    fn from(output: &OutputAction) -> Self {
        Self {
            shortcut: output.paste_shortcut,
            custom_x11: output.paste_custom_x11.clone(),
            custom_wayland: output.paste_custom_wayland.clone(),
        }
    }
}

enum OutputWorkerCommand {
    RunScript {
        recording_id: u64,
        shortcut_id: String,
        script_path: String,
        text: String,
        output: OutputAction,
        shortcut_name: String,
        model_id: String,
        language: String,
    },
    CopyClipboard {
        source: ClipboardCopySource,
        text: String,
        paste: Option<PasteRequest>,
    },
    Shutdown,
}

fn output_worker_loop(
    worker_rx: mpsc::Receiver<OutputWorkerCommand>,
    command_tx: mpsc::Sender<AppCommand>,
    cancel_requested: Arc<AtomicBool>,
) {
    for command in worker_rx {
        if cancel_requested.load(Ordering::Relaxed) {
            break;
        }
        match command {
            OutputWorkerCommand::RunScript {
                recording_id,
                shortcut_id,
                script_path,
                text,
                output,
                shortcut_name,
                model_id,
                language,
            } => {
                let result = run_script(&script_path, &text, &output, &cancel_requested)
                    .map(|mut result| {
                        result.shortcut_name = shortcut_name;
                        result.model_id = model_id;
                        result.language = language;
                        result
                    })
                    .map_err(|error| format!("{error:#}"));
                let _ = command_tx.send(AppCommand::OutputScriptFinished {
                    recording_id,
                    shortcut_id,
                    result,
                });
            }
            OutputWorkerCommand::CopyClipboard {
                source,
                text,
                paste,
            } => {
                let result = copy_text_to_external_clipboard(&text, &cancel_requested);
                let paste_result = result.as_ref().ok().and_then(|copy| {
                    paste.map(|paste| {
                        paste_from_external_clipboard(copy.backend, &paste, &cancel_requested)
                            .map_err(|error| format!("{error:#}"))
                    })
                });
                let copy_result = result.map_err(|error| format!("{error:#}"));
                let paste_source = source.clone();
                let _ = command_tx.send(AppCommand::ClipboardCopyFinished {
                    source,
                    result: copy_result,
                });
                if let Some(result) = paste_result {
                    let _ = command_tx.send(AppCommand::ClipboardPasteFinished {
                        source: paste_source,
                        result,
                    });
                }
            }
            OutputWorkerCommand::Shutdown => break,
        }
    }
}

fn copy_text_to_external_clipboard(
    text: &str,
    cancel_requested: &AtomicBool,
) -> Result<ClipboardCopyOutcome> {
    let backend = detect_clipboard_backend_from_env(|name| std::env::var_os(name))?;
    match backend {
        ClipboardBackend::Wayland => copy_text_to_wayland_clipboard(text, cancel_requested)?,
        ClipboardBackend::X11 => copy_text_to_x11_clipboard(text, cancel_requested)?,
    }
    verify_external_clipboard(backend, text, cancel_requested)?;

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

fn copy_text_to_wayland_clipboard(text: &str, cancel_requested: &AtomicBool) -> Result<()> {
    let commands = clipboard_commands(ClipboardBackend::Wayland);
    ensure_command_in_path(commands.copy, commands.package_hint)?;
    let mut command = Command::new(commands.copy);
    command.args(wayland_copy_args());
    run_clipboard_status_with_stdin(
        &mut command,
        text,
        CLIPBOARD_TIMEOUT,
        "wl-copy",
        cancel_requested,
    )
}

fn copy_text_to_x11_clipboard(text: &str, cancel_requested: &AtomicBool) -> Result<()> {
    let commands = clipboard_commands(ClipboardBackend::X11);
    ensure_command_in_path(commands.copy, commands.package_hint)?;
    let mut command = Command::new(commands.copy);
    command.args(["-selection", "clipboard", "-in"]);
    run_clipboard_status_with_stdin(
        &mut command,
        text,
        CLIPBOARD_TIMEOUT,
        "xclip clipboard copy",
        cancel_requested,
    )
}

fn verify_external_clipboard(
    backend: ClipboardBackend,
    expected_text: &str,
    cancel_requested: &AtomicBool,
) -> Result<()> {
    let actual_text = read_external_clipboard(backend, cancel_requested)?;
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

fn read_external_clipboard(
    backend: ClipboardBackend,
    cancel_requested: &AtomicBool,
) -> Result<String> {
    let output = match backend {
        ClipboardBackend::Wayland => {
            let commands = clipboard_commands(backend);
            ensure_command_in_path(commands.paste, commands.package_hint)?;
            let mut command = Command::new(commands.paste);
            command.args(wayland_paste_args());
            run_output_with_timeout(
                &mut command,
                CLIPBOARD_TIMEOUT,
                "wl-paste",
                cancel_requested,
            )?
        }
        ClipboardBackend::X11 => {
            let commands = clipboard_commands(backend);
            ensure_command_in_path(commands.paste, commands.package_hint)?;
            let mut command = Command::new(commands.paste);
            command.args(["-selection", "clipboard", "-out"]);
            run_output_with_timeout(
                &mut command,
                CLIPBOARD_TIMEOUT,
                "xclip clipboard read",
                cancel_requested,
            )?
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

fn paste_from_external_clipboard(
    backend: ClipboardBackend,
    request: &PasteRequest,
    cancel_requested: &AtomicBool,
) -> Result<ClipboardPasteOutcome> {
    match backend {
        ClipboardBackend::Wayland => paste_with_ydotool(request, cancel_requested)?,
        ClipboardBackend::X11 => paste_with_xdotool(request, cancel_requested)?,
    }
    Ok(ClipboardPasteOutcome {
        backend,
        shortcut: request.shortcut,
    })
}

fn paste_with_xdotool(request: &PasteRequest, cancel_requested: &AtomicBool) -> Result<()> {
    ensure_command_in_path("xdotool", "xdotool")?;
    let key_sequences = xdotool_key_sequences(request)?;
    let mut command = Command::new("xdotool");
    command
        .arg("key")
        .arg("--clearmodifiers")
        .args(key_sequences);
    run_status_without_stdin(
        &mut command,
        CLIPBOARD_TIMEOUT,
        "xdotool paste shortcut",
        cancel_requested,
    )
}

fn paste_with_ydotool(request: &PasteRequest, cancel_requested: &AtomicBool) -> Result<()> {
    ensure_command_in_path("ydotool", "ydotool")?;
    let key_events = ydotool_key_events(request)?;
    let mut command = Command::new("ydotool");
    command.arg("key").args(key_events);
    run_status_without_stdin(
        &mut command,
        CLIPBOARD_TIMEOUT,
        "ydotool paste shortcut",
        cancel_requested,
    )
}

fn xdotool_key_sequences(request: &PasteRequest) -> Result<Vec<String>> {
    let sequences = match request.shortcut {
        PasteShortcut::CtrlV => vec!["ctrl+v".to_string()],
        PasteShortcut::CtrlShiftV => vec!["ctrl+shift+v".to_string()],
        PasteShortcut::Custom => request
            .custom_x11
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>(),
    };
    if sequences.is_empty() {
        bail!("custom xdotool paste shortcut is empty");
    }
    Ok(sequences)
}

fn ydotool_key_events(request: &PasteRequest) -> Result<Vec<String>> {
    let events = match request.shortcut {
        PasteShortcut::CtrlV => vec!["29:1", "47:1", "47:0", "29:0"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        PasteShortcut::CtrlShiftV => vec!["29:1", "42:1", "47:1", "47:0", "42:0", "29:0"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        PasteShortcut::Custom => request
            .custom_wayland
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>(),
    };
    if events.is_empty() {
        bail!("custom ydotool paste shortcut is empty");
    }
    for event in &events {
        validate_ydotool_key_event(event)?;
    }
    Ok(events)
}

fn validate_ydotool_key_event(event: &str) -> Result<()> {
    let Some((keycode, pressed)) = event.split_once(':') else {
        bail!("invalid ydotool key event {event:?}; expected keycode:pressed")
    };
    let keycode = keycode
        .parse::<u16>()
        .with_context(|| format!("invalid ydotool keycode in {event:?}"))?;
    if keycode == 0 {
        bail!("invalid ydotool keycode in {event:?}; keycode must be positive");
    }
    match pressed {
        "0" | "1" => Ok(()),
        _ => bail!("invalid ydotool key state in {event:?}; expected 0 or 1"),
    }
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

fn run_clipboard_status_with_stdin(
    command: &mut Command,
    text: &str,
    timeout: Duration,
    description: &str,
    cancel_requested: &AtomicBool,
) -> Result<()> {
    // Clipboard owners such as wl-copy and xclip may fork and keep inherited
    // pipes open, so wait only for the launcher status and discard its output.
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to spawn {description}"))?;

    let stdin = child.stdin.take().context("failed to open child stdin")?;
    let writer = match spawn_stdin_writer(stdin, text.to_string(), description) {
        Ok(writer) => writer,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let status_result = wait_for_child_status(child, timeout, description, cancel_requested);
    let write_result = join_stdin_writer(writer, description);
    let status = status_result?;
    if !status.success() {
        bail!("{description} exited with status {status}");
    }
    write_result?;

    Ok(())
}

fn run_status_without_stdin(
    command: &mut Command,
    timeout: Duration,
    description: &str,
    cancel_requested: &AtomicBool,
) -> Result<()> {
    let output = run_output_with_timeout(command, timeout, description, cancel_requested)?;
    if !output.status.success() {
        bail!(
            "{description} exited with status {}; stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(())
}

fn run_output_with_timeout(
    command: &mut Command,
    timeout: Duration,
    description: &str,
    cancel_requested: &AtomicBool,
) -> Result<Output> {
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {description}"))?;
    wait_for_child_output(child, timeout, description, cancel_requested)
}

fn wait_for_child_status(
    mut child: Child,
    timeout: Duration,
    description: &str,
    cancel_requested: &AtomicBool,
) -> Result<ExitStatus> {
    let started_at = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }

        if cancel_requested.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{description} canceled during shutdown");
        }

        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{description} timed out after {timeout:?}");
        }

        thread::sleep(SCRIPT_POLL_INTERVAL);
    }
}

fn wait_for_child_output(
    mut child: Child,
    timeout: Duration,
    description: &str,
    cancel_requested: &AtomicBool,
) -> Result<Output> {
    let stdout_reader = match child.stdout.take() {
        Some(stdout) => match spawn_pipe_reader(stdout, description, "stdout") {
            Ok(reader) => Some(reader),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        },
        None => None,
    };
    let stderr_reader = match child.stderr.take() {
        Some(stderr) => match spawn_pipe_reader(stderr, description, "stderr") {
            Ok(reader) => Some(reader),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_pipe_reader(stdout_reader, description, "stdout");
                return Err(error);
            }
        },
        None => None,
    };
    let started_at = Instant::now();
    let mut timed_out = false;
    let mut canceled = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }

        if cancel_requested.load(Ordering::Relaxed) {
            canceled = true;
            let _ = child.kill();
            break child.wait()?;
        }

        if started_at.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            break child.wait()?;
        }

        thread::sleep(SCRIPT_POLL_INTERVAL);
    };

    let stdout = join_pipe_reader(stdout_reader, description, "stdout")?;
    let stderr = join_pipe_reader(stderr_reader, description, "stderr")?;

    if canceled {
        bail!("{description} canceled during shutdown");
    }
    if timed_out {
        bail!("{description} timed out after {timeout:?}");
    }

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn spawn_pipe_reader<R>(
    mut reader: R,
    description: &str,
    stream_name: &'static str,
) -> Result<thread::JoinHandle<io::Result<Vec<u8>>>>
where
    R: Read + Send + 'static,
{
    thread::Builder::new()
        .name(format!("quillspeak-output-{stream_name}"))
        .spawn(move || {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes)?;
            Ok(bytes)
        })
        .with_context(|| format!("failed to spawn {description} {stream_name} reader"))
}

fn join_pipe_reader(
    reader: Option<thread::JoinHandle<io::Result<Vec<u8>>>>,
    description: &str,
    stream_name: &str,
) -> Result<Vec<u8>> {
    let Some(reader) = reader else {
        return Ok(Vec::new());
    };
    reader
        .join()
        .map_err(|error| anyhow!("{description} {stream_name} reader panicked: {error:?}"))?
        .with_context(|| format!("failed to read {description} {stream_name}"))
}

fn spawn_stdin_writer(
    mut stdin: std::process::ChildStdin,
    text: String,
    description: &str,
) -> Result<thread::JoinHandle<io::Result<()>>> {
    thread::Builder::new()
        .name("quillspeak-output-stdin".to_string())
        .spawn(move || stdin.write_all(text.as_bytes()))
        .with_context(|| format!("failed to spawn {description} stdin writer"))
}

fn join_stdin_writer(writer: thread::JoinHandle<io::Result<()>>, description: &str) -> Result<()> {
    writer
        .join()
        .map_err(|error| anyhow!("{description} stdin writer panicked: {error:?}"))?
        .with_context(|| format!("failed to write text to {description} stdin"))
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
    action: &OutputAction,
    cancel_requested: &AtomicBool,
) -> Result<OutputScriptResult> {
    let process_output = run_script_with_timeout(script_path, text, true, cancel_requested)?;
    if !process_output.status.success() {
        bail!(
            "script {} exited with status {}; stderr: {}",
            script_path,
            process_output.status,
            String::from_utf8_lossy(&process_output.stderr).trim()
        );
    }

    let output_text = Some(
        String::from_utf8(process_output.stdout)
            .with_context(|| format!("script {script_path} stdout was not UTF-8"))?,
    );
    let mut output = action.clone();
    output.script = None;

    Ok(OutputScriptResult {
        script_path: script_path.to_string(),
        output_text,
        output,
        shortcut_name: String::new(),
        model_id: String::new(),
        language: String::new(),
    })
}

fn run_script_with_timeout(
    script_path: &str,
    text: &str,
    capture_stdout: bool,
    cancel_requested: &AtomicBool,
) -> Result<std::process::Output> {
    let child = spawn_script_with_retry(script_path, text, capture_stdout)
        .with_context(|| format!("failed to spawn output script {script_path}"))?;
    wait_for_child_output(
        child,
        SCRIPT_TIMEOUT,
        &format!("output script {script_path}"),
        cancel_requested,
    )
}

fn spawn_script_with_retry(script_path: &str, text: &str, capture_stdout: bool) -> Result<Child> {
    for attempt in 0..SCRIPT_SPAWN_ATTEMPTS {
        match spawn_script_once(script_path, text, capture_stdout) {
            Ok(child) => return Ok(child),
            Err(error)
                if error.raw_os_error() == Some(TEXT_FILE_BUSY_OS_ERROR)
                    && attempt + 1 < SCRIPT_SPAWN_ATTEMPTS =>
            {
                thread::sleep(SCRIPT_SPAWN_RETRY_DELAY);
            }
            Err(error) => return Err(error.into()),
        }
    }

    unreachable!("script spawn retry loop should return on success or final error")
}

fn spawn_script_once(
    script_path: &str,
    text: &str,
    capture_stdout: bool,
) -> std::io::Result<Child> {
    let stdout = if capture_stdout {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    Command::new(script_path)
        .arg(text)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs::{self, File};
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    };

    use shared::{OutputAction, ScriptOutput};

    use super::*;
    use crate::transcription::TranscriptionSkipReason;

    #[test]
    fn clipboard_disabled_when_output_does_not_request_it() {
        let output = OutputAction {
            copy_to_clipboard: false,
            ..OutputAction::default()
        };

        assert!(!output.copy_to_clipboard);
    }

    #[test]
    fn script_output_result_carries_final_text_and_delivery_flags() {
        let result = OutputScriptResult {
            script_path: "/bin/echo".to_string(),
            output_text: Some("hello\n".to_string()),
            output: OutputAction::default(),
            shortcut_name: "Default".to_string(),
            model_id: "tiny".to_string(),
            language: "auto".to_string(),
        };

        assert_eq!(result.output_text.as_deref(), Some("hello\n"));
        assert!(result.output.copy_to_clipboard);
    }

    #[test]
    fn output_action_can_run_script_and_copy_final_text() {
        let output = OutputAction {
            copy_to_clipboard: true,
            script: Some(ScriptOutput {
                path: "/bin/echo".to_string(),
            }),
            ..OutputAction::default()
        };

        assert!(output.copy_to_clipboard);
        assert_eq!(
            output.script.as_ref().map(|script| script.path.as_str()),
            Some("/bin/echo")
        );
    }

    #[test]
    fn apply_does_not_queue_output_for_skipped_transcription() {
        let (worker_tx, worker_rx) = mpsc::channel();
        let service = OutputService {
            worker_tx,
            join_handle: None,
            cancel_requested: Arc::new(AtomicBool::new(false)),
        };
        let mut result = crate::transcription::test_support::skipped_transcription_result_fixture(
            TranscriptionSkipReason::NearSilent,
        );
        result.text = "ignored text".to_string();

        assert_eq!(
            service.apply(7, "default", &result),
            OutputDelivery::NotQueued
        );
        assert!(worker_rx.try_recv().is_err());
    }

    #[test]
    fn clipboard_text_for_output_returns_trimmed_non_empty_text() {
        let output = OutputAction::default();

        assert_eq!(
            clipboard_transport_text_for_output(&output, "  hello clipboard  "),
            Some("hello clipboard")
        );
    }

    #[test]
    fn clipboard_text_for_output_skips_empty_text() {
        let output = OutputAction::default();

        assert_eq!(clipboard_transport_text_for_output(&output, "   "), None);
    }

    #[test]
    fn clipboard_text_for_output_skips_disabled_clipboard() {
        let output = OutputAction {
            copy_to_clipboard: false,
            paste_from_clipboard: false,
            ..OutputAction::default()
        };

        assert_eq!(clipboard_transport_text_for_output(&output, "hello"), None);
    }

    #[test]
    fn clipboard_transport_is_required_for_paste_even_when_copy_is_disabled() {
        let output = OutputAction {
            copy_to_clipboard: false,
            paste_from_clipboard: true,
            script: None,
            ..OutputAction::default()
        };

        assert_eq!(
            clipboard_transport_text_for_output(&output, "hello"),
            Some("hello")
        );
    }

    #[test]
    fn clipboard_completion_waits_for_paste_when_paste_is_requested() {
        let copy_only = OutputAction::default();
        assert_eq!(
            clipboard_completion_for_output(&copy_only),
            OutputCompletion::ClipboardCopy
        );

        let paste = OutputAction {
            paste_from_clipboard: true,
            ..OutputAction::default()
        };
        assert_eq!(
            clipboard_completion_for_output(&paste),
            OutputCompletion::ClipboardPaste
        );
    }

    #[test]
    fn clipboard_copy_source_tracks_transcription_context() {
        let source = ClipboardCopySource::Transcription {
            recording_id: 42,
            shortcut_id: "default".to_string(),
        };

        assert_eq!(source.recording_id(), 42);
        assert_eq!(source.shortcut_id(), "default");
        assert_eq!(source.kind(), "transcription");
        assert_eq!(source.script_path(), None);
    }

    #[test]
    fn clipboard_copy_source_tracks_script_context() {
        let source = ClipboardCopySource::ScriptStdout {
            recording_id: 43,
            shortcut_id: "default".to_string(),
            script_path: "/tmp/script".to_string(),
        };

        assert_eq!(source.recording_id(), 43);
        assert_eq!(source.shortcut_id(), "default");
        assert_eq!(source.kind(), "script_stdout");
        assert_eq!(source.script_path(), Some("/tmp/script"));
    }

    #[test]
    fn clipboard_copy_source_tracks_history_context() {
        let source = ClipboardCopySource::History {
            recording_id: 44,
            shortcut_id: "default".to_string(),
        };

        assert_eq!(source.recording_id(), 44);
        assert_eq!(source.shortcut_id(), "default");
        assert_eq!(source.kind(), "history");
        assert_eq!(source.script_path(), None);
    }

    #[test]
    fn run_script_captures_stdout_when_delivery_is_disabled() {
        let script = TestScript::new("stdout-for-history", "printf 'translated:%s' \"$1\"\n");

        let output = OutputAction {
            copy_to_clipboard: false,
            paste_from_clipboard: false,
            ..OutputAction::default()
        };
        let result = run_script(script.path_str(), "hello", &output, never_cancel())
            .expect("script-only output should capture stdout for history");

        assert_eq!(result.output_text.as_deref(), Some("translated:hello"));
        assert!(!result.output.copy_to_clipboard);
    }

    #[test]
    fn run_script_delivers_stdout_when_copy_is_requested() {
        let script = TestScript::new("copy-stdout", "printf 'translated:%s' \"$1\"\n");

        let output = OutputAction::default();
        let result = run_script(script.path_str(), "hello", &output, never_cancel())
            .expect("copy output should capture script stdout");

        assert_eq!(result.output_text.as_deref(), Some("translated:hello"));
        assert!(result.output.copy_to_clipboard);
    }

    #[test]
    fn run_script_delivers_stdout_when_paste_is_requested() {
        let script = TestScript::new("paste-stdout", "printf 'translated:%s' \"$1\"\n");
        let output = OutputAction {
            copy_to_clipboard: false,
            paste_from_clipboard: true,
            ..OutputAction::default()
        };

        let result = run_script(script.path_str(), "hello", &output, never_cancel())
            .expect("paste output should capture script stdout");

        assert_eq!(result.output_text.as_deref(), Some("translated:hello"));
        assert!(result.output.paste_from_clipboard);
    }

    #[test]
    fn run_script_rejects_invalid_utf8_stdout() {
        let script = TestScript::new("invalid-stdout", "printf '\\377'\n");

        let output = OutputAction::default();
        let error = run_script(script.path_str(), "hello", &output, never_cancel())
            .expect_err("script stdout must be UTF-8");

        assert!(error.to_string().contains("stdout was not UTF-8"));
    }

    #[test]
    fn run_script_nonzero_exit_fails_without_delivery_fallback() {
        let script = TestScript::new("nonzero", "printf 'script failed' >&2\nexit 7\n");

        let output = OutputAction {
            copy_to_clipboard: false,
            paste_from_clipboard: false,
            ..OutputAction::default()
        };
        let error = run_script(script.path_str(), "hello", &output, never_cancel())
            .expect_err("nonzero script should fail");

        assert!(error.to_string().contains("script failed"));
        assert!(error.to_string().contains("exited with status"));
    }

    #[test]
    fn run_script_drains_large_stdout_without_deadlock() {
        let script = TestScript::new(
            "large-stdout",
            "i=0\nwhile [ \"$i\" -lt 20000 ]; do printf '0123456789'; i=$((i + 1)); done\n",
        );

        let result = run_script(
            script.path_str(),
            "hello",
            &OutputAction::default(),
            never_cancel(),
        )
        .expect("large stdout should be drained while waiting");

        assert_eq!(result.output_text.as_ref().map(String::len), Some(200_000));
    }

    #[test]
    fn clipboard_stdin_runner_does_not_wait_for_backgrounded_streams() {
        let script = TestScript::new("background-copy", "cat >/dev/null\nsleep 1 &\nexit 0\n");
        let mut command = Command::new(script.path_str());
        let started_at = std::time::Instant::now();

        run_clipboard_status_with_stdin(
            &mut command,
            "hello clipboard",
            Duration::from_millis(300),
            "fake clipboard copy",
            never_cancel(),
        )
        .expect("clipboard copy runner should return after the launcher exits");

        assert!(
            started_at.elapsed() < Duration::from_millis(800),
            "clipboard copy runner waited for a background child"
        );
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
    fn paste_presets_build_expected_commands() {
        let ctrl_v = PasteRequest {
            shortcut: PasteShortcut::CtrlV,
            custom_x11: String::new(),
            custom_wayland: String::new(),
        };
        assert_eq!(xdotool_key_sequences(&ctrl_v).unwrap(), vec!["ctrl+v"]);
        assert_eq!(
            ydotool_key_events(&ctrl_v).unwrap(),
            vec!["29:1", "47:1", "47:0", "29:0"]
        );

        let ctrl_shift_v = PasteRequest {
            shortcut: PasteShortcut::CtrlShiftV,
            custom_x11: String::new(),
            custom_wayland: String::new(),
        };
        assert_eq!(
            xdotool_key_sequences(&ctrl_shift_v).unwrap(),
            vec!["ctrl+shift+v"]
        );
        assert_eq!(
            ydotool_key_events(&ctrl_shift_v).unwrap(),
            vec!["29:1", "42:1", "47:1", "47:0", "42:0", "29:0"]
        );
    }

    #[test]
    fn custom_paste_commands_are_validated() {
        let custom = PasteRequest {
            shortcut: PasteShortcut::Custom,
            custom_x11: "ctrl+v 0xff0d".to_string(),
            custom_wayland: "29:1 47:1 47:0 29:0".to_string(),
        };

        assert_eq!(
            xdotool_key_sequences(&custom).unwrap(),
            vec!["ctrl+v", "0xff0d"]
        );
        assert_eq!(
            ydotool_key_events(&custom).unwrap(),
            vec!["29:1", "47:1", "47:0", "29:0"]
        );

        let bad = PasteRequest {
            custom_wayland: "29:x".to_string(),
            ..custom
        };
        assert!(ydotool_key_events(&bad).is_err());
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
        let text = format!("quillspeak clipboard smoke {}", unique_test_suffix());
        let second_text = format!("quillspeak clipboard smoke second {}", unique_test_suffix());
        let _restore_guard = ClipboardRestoreGuard::new(
            read_external_clipboard(ClipboardBackend::Wayland, never_cancel()).ok(),
        );

        copy_text_to_external_clipboard(&text, never_cancel())
            .expect("external clipboard copy should succeed");

        let backend = detect_clipboard_backend_from_env(|name| std::env::var_os(name))
            .expect("display env should select a clipboard backend");
        assert_eq!(backend, ClipboardBackend::Wayland);

        let actual = read_external_clipboard(backend, never_cancel())
            .expect("clipboard text should be readable");
        assert_eq!(actual, text);

        copy_text_to_external_clipboard(&second_text, never_cancel())
            .expect("second external clipboard copy should succeed");

        let actual = read_external_clipboard(backend, never_cancel())
            .expect("second clipboard text should be readable");
        assert_eq!(actual, second_text);
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
                let _ = copy_text_to_external_clipboard(&original_text, never_cancel());
            }
        }
    }

    fn never_cancel() -> &'static AtomicBool {
        static NEVER_CANCEL: AtomicBool = AtomicBool::new(false);
        &NEVER_CANCEL
    }

    fn test_env<'a>(values: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(*value))
        }
    }

    static TEST_SCRIPT_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_test_suffix() -> String {
        format!(
            "{}-{}",
            std::process::id(),
            TEST_SCRIPT_COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    struct TestScript {
        path: PathBuf,
        dir: PathBuf,
    }

    impl TestScript {
        fn new(name: &str, body: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "quillspeak-output-script-test-{name}-{}",
                unique_test_suffix()
            ));
            fs::create_dir_all(&dir).expect("test script dir should be writable");
            let path = dir.join("script.sh");
            let tmp_path = dir.join("script.sh.tmp");
            let mut file = File::create(&tmp_path).expect("test script should be writable");
            file.write_all(format!("#!/bin/sh\n{body}").as_bytes())
                .expect("test script body should be writable");
            file.sync_all().expect("test script should sync to disk");
            drop(file);
            let mut permissions = fs::metadata(&tmp_path)
                .expect("test script metadata should be readable")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&tmp_path, permissions).expect("test script should be executable");
            fs::rename(&tmp_path, &path).expect("test script should be atomically installed");
            Self { path, dir }
        }

        fn path_str(&self) -> &str {
            path_str(&self.path)
        }
    }

    impl Drop for TestScript {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn path_str(path: &Path) -> &str {
        path.to_str().expect("test path should be valid UTF-8")
    }
}
