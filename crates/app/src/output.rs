use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use gtk::prelude::*;
use gtk4 as gtk;
use shared::{OutputAction, ScriptOutput};
use tracing::{debug, info, warn};

use crate::command::AppCommand;
use crate::transcription::{TranscriptionResult, TranscriptionStatus};

const SCRIPT_TIMEOUT: Duration = Duration::from_secs(30);
const SCRIPT_POLL_INTERVAL: Duration = Duration::from_millis(20);

pub struct OutputService {
    worker_tx: mpsc::Sender<OutputWorkerCommand>,
    join_handle: Option<thread::JoinHandle<()>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputScriptResult {
    pub script_path: String,
    pub clipboard_text: Option<String>,
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

        apply_clipboard_if_enabled(shortcut_id, &result.output, text);
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
}

fn apply_clipboard_if_enabled(shortcut_id: &str, output: &OutputAction, text: &str) {
    let Some(text) = clipboard_text_for_output(output, text) else {
        return;
    };

    match copy_text_to_clipboard(
        text,
        ClipboardCopySource::Transcription {
            shortcut_id: shortcut_id.to_string(),
        },
    ) {
        Ok(()) => debug!(
            shortcut_id,
            "queued transcription clipboard copy verification"
        ),
        Err(error) => warn!(
            ?error,
            shortcut_id, "failed to queue transcription clipboard copy"
        ),
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
    },
    ScriptStdout {
        shortcut_id: String,
        script_path: String,
    },
}

impl ClipboardCopySource {
    fn shortcut_id(&self) -> &str {
        match self {
            Self::Transcription { shortcut_id } | Self::ScriptStdout { shortcut_id, .. } => {
                shortcut_id
            }
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Transcription { .. } => "transcription",
            Self::ScriptStdout { .. } => "script_stdout",
        }
    }

    fn script_path(&self) -> Option<&str> {
        match self {
            Self::Transcription { .. } => None,
            Self::ScriptStdout { script_path, .. } => Some(script_path),
        }
    }
}

pub fn copy_text_to_clipboard(text: &str, source: ClipboardCopySource) -> Result<()> {
    let display = gtk::gdk::Display::default().context("no default GDK display")?;
    let clipboard = display.clipboard();
    let expected_text = text.to_string();
    let text_chars = expected_text.chars().count();
    let text_bytes = expected_text.len();
    clipboard.set_text(&expected_text);

    gtk::glib::MainContext::default().spawn_local(async move {
        let actual_text = clipboard.read_text_future().await;
        log_clipboard_copy_verification(
            source,
            &expected_text,
            text_chars,
            text_bytes,
            actual_text,
        );
    });

    Ok(())
}

fn log_clipboard_copy_verification(
    source: ClipboardCopySource,
    expected_text: &str,
    text_chars: usize,
    text_bytes: usize,
    actual_text: std::result::Result<Option<gtk::glib::GString>, gtk::glib::Error>,
) {
    let shortcut_id = source.shortcut_id();
    let copy_source = source.kind();
    let script_path = source.script_path().unwrap_or("");
    match actual_text {
        Ok(Some(actual_text)) if actual_text.as_str() == expected_text => info!(
            shortcut_id,
            source = copy_source,
            script = script_path,
            text_chars,
            text_bytes,
            "Copied text to clipboard"
        ),
        Ok(Some(actual_text)) => warn!(
            shortcut_id,
            source = copy_source,
            script = script_path,
            text_chars,
            text_bytes,
            actual_chars = actual_text.chars().count(),
            actual_bytes = actual_text.len(),
            "clipboard copy verification mismatch"
        ),
        Ok(None) => warn!(
            shortcut_id,
            source = copy_source,
            script = script_path,
            text_chars,
            text_bytes,
            "clipboard copy verification returned no text"
        ),
        Err(error) => warn!(
            ?error,
            shortcut_id,
            source = copy_source,
            script = script_path,
            text_chars,
            text_bytes,
            "clipboard copy verification failed"
        ),
    }
}

enum OutputWorkerCommand {
    RunScript {
        shortcut_id: String,
        script_path: String,
        text: String,
        copy_stdout_to_clipboard: bool,
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
            OutputWorkerCommand::Shutdown => break,
        }
    }
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
    use std::time::{SystemTime, UNIX_EPOCH};

    use shared::{OutputAction, ScriptOutput};

    use super::*;

    #[test]
    fn clipboard_disabled_when_output_does_not_request_it() {
        let output = OutputAction {
            copy_to_clipboard: false,
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
            script: None,
        };

        assert_eq!(clipboard_text_for_output(&output, "hello"), None);
    }

    #[test]
    fn clipboard_copy_source_tracks_transcription_context() {
        let source = ClipboardCopySource::Transcription {
            shortcut_id: "default".to_string(),
        };

        assert_eq!(source.shortcut_id(), "default");
        assert_eq!(source.kind(), "transcription");
        assert_eq!(source.script_path(), None);
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
    }

    #[test]
    #[ignore = "requires a real GTK display and mutates the system clipboard"]
    fn gtk_clipboard_copy_round_trips_text() {
        gtk::init().expect("GTK should initialize with a real display");
        let text = format!("myapp clipboard smoke {}", unique_test_suffix());

        copy_text_to_clipboard(
            &text,
            ClipboardCopySource::Transcription {
                shortcut_id: "test".to_string(),
            },
        )
        .expect("clipboard copy should be queued");

        let display = gtk::gdk::Display::default().expect("GDK display should be available");
        let actual = gtk::glib::MainContext::default()
            .block_on(display.clipboard().read_text_future())
            .expect("clipboard text should be readable")
            .expect("clipboard should contain text");

        assert_eq!(actual.as_str(), text);
    }

    fn unique_test_suffix() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos()
    }
}
