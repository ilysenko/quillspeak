use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use reqwest::blocking::Client;
use sha1::{Digest, Sha1};
use shared::model_catalog_entry;
use tracing::{debug, info};

use crate::command::{AppCommand, DownloadId, ModelDownloadOutcome};
use crate::models::inventory::{model_path, partial_model_path};

const DOWNLOAD_BUFFER_SIZE: usize = 64 * 1024;
const PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(250);
const PROGRESS_MIN_DELTA_BYTES: u64 = 1024 * 1024;
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HTTP_TOTAL_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug)]
pub struct DownloadHandle {
    cancel_requested: Arc<AtomicBool>,
}

impl DownloadHandle {
    pub fn cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        Self {
            cancel_requested: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub fn start_download(
    root: &Path,
    download_id: DownloadId,
    model_id: String,
    command_tx: mpsc::Sender<AppCommand>,
) -> DownloadHandle {
    let root = root.to_path_buf();
    let cancel_requested = Arc::new(AtomicBool::new(false));
    let worker_cancel_requested = Arc::clone(&cancel_requested);
    let worker_command_tx = command_tx.clone();
    let worker_model_id = model_id.clone();
    let thread_name = format!("myapp-model-download-{model_id}");
    let spawn_result = thread::Builder::new().name(thread_name).spawn(move || {
        let outcome = download_model(
            &root,
            download_id,
            &worker_model_id,
            &worker_command_tx,
            &worker_cancel_requested,
        );
        let _ = worker_command_tx.send(AppCommand::ModelDownloadFinished {
            download_id,
            model_id: worker_model_id,
            outcome,
        });
    });
    if let Err(error) = spawn_result {
        let _ = command_tx.send(AppCommand::ModelDownloadFinished {
            download_id,
            model_id,
            outcome: ModelDownloadOutcome::Failed(format!(
                "failed to spawn model download worker: {error}"
            )),
        });
    }
    DownloadHandle { cancel_requested }
}

fn download_model(
    root: &Path,
    download_id: DownloadId,
    model_id: &str,
    command_tx: &mpsc::Sender<AppCommand>,
    cancel_requested: &AtomicBool,
) -> ModelDownloadOutcome {
    let partial = model_catalog_entry(model_id).map(|entry| partial_model_path(root, entry));
    let result = download_model_inner(root, download_id, model_id, command_tx, cancel_requested);
    if !matches!(result, Ok(()))
        && let Some(partial) = partial.as_ref().filter(|partial| partial.exists())
    {
        let _ = fs::remove_file(partial);
    }

    match result {
        Ok(()) => ModelDownloadOutcome::Completed,
        Err(DownloadError::Canceled) => ModelDownloadOutcome::Canceled,
        Err(DownloadError::Failed(error)) => ModelDownloadOutcome::Failed(format!("{error:#}")),
    }
}

fn download_model_inner(
    root: &Path,
    download_id: DownloadId,
    model_id: &str,
    command_tx: &mpsc::Sender<AppCommand>,
    cancel_requested: &AtomicBool,
) -> DownloadResult<()> {
    ensure_not_canceled(cancel_requested)?;
    let entry = model_catalog_entry(model_id).ok_or_else(|| anyhow!("unknown model {model_id}"))?;
    fs::create_dir_all(root)
        .with_context(|| format!("failed to create model directory {}", root.display()))?;
    let target = model_path(root, entry);
    let partial = partial_model_path(root, entry);

    info!(model_id, url = entry.url, "downloading whisper.cpp model");
    let client = Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .timeout(HTTP_TOTAL_TIMEOUT)
        .build()
        .context("failed to build HTTP client")?;
    ensure_not_canceled(cancel_requested)?;
    let mut response = client
        .get(entry.url)
        .send()
        .context("failed to start model download")?
        .error_for_status()
        .context("model download returned HTTP error")?;
    let total = response.content_length().or(Some(entry.size_bytes));
    let mut file = File::create(&partial)
        .with_context(|| format!("failed to create partial model {}", partial.display()))?;
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; DOWNLOAD_BUFFER_SIZE];
    let mut progress = DownloadProgressReporter::new(download_id, model_id, total, command_tx);
    progress.send_initial();

    loop {
        ensure_not_canceled(cancel_requested)?;
        let bytes = response
            .read(&mut buffer)
            .context("failed to read model download stream")?;
        if bytes == 0 {
            break;
        }
        ensure_not_canceled(cancel_requested)?;
        file.write_all(&buffer[..bytes])
            .context("failed to write model download chunk")?;
        downloaded += bytes as u64;
        progress.send_throttled(downloaded);
    }
    ensure_not_canceled(cancel_requested)?;
    progress.send_final(downloaded);
    file.flush().context("failed to flush partial model")?;

    ensure_not_canceled(cancel_requested)?;
    progress.send_verifying(downloaded);
    let sha1 = sha1_file_cancellable(&partial, cancel_requested)?;
    if sha1 != entry.sha1 {
        return Err(anyhow!(
            "downloaded model hash mismatch: expected {}, got {}",
            entry.sha1,
            sha1
        )
        .into());
    }

    ensure_not_canceled(cancel_requested)?;
    fs::rename(&partial, &target).with_context(|| {
        format!(
            "failed to move downloaded model {} to {}",
            partial.display(),
            target.display()
        )
    })?;
    info!(model_id, path = %target.display(), "model download finished");
    Ok(())
}

type DownloadResult<T> = std::result::Result<T, DownloadError>;

#[derive(Debug)]
enum DownloadError {
    Canceled,
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for DownloadError {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed(error)
    }
}

fn ensure_not_canceled(cancel_requested: &AtomicBool) -> DownloadResult<()> {
    if cancel_requested.load(Ordering::Relaxed) {
        return Err(DownloadError::Canceled);
    }
    Ok(())
}

struct DownloadProgressReporter<'a> {
    download_id: DownloadId,
    model_id: &'a str,
    total: Option<u64>,
    command_tx: &'a mpsc::Sender<AppCommand>,
    throttle: ProgressThrottle,
}

impl<'a> DownloadProgressReporter<'a> {
    fn new(
        download_id: DownloadId,
        model_id: &'a str,
        total: Option<u64>,
        command_tx: &'a mpsc::Sender<AppCommand>,
    ) -> Self {
        Self {
            download_id,
            model_id,
            total,
            command_tx,
            throttle: ProgressThrottle::default(),
        }
    }

    fn send_initial(&mut self) {
        self.send_progress(0);
        self.throttle.mark_emitted(0, Instant::now());
    }

    fn send_throttled(&mut self, downloaded: u64) {
        let now = Instant::now();
        if self.throttle.should_emit(downloaded, now) {
            debug!(
                model_id = self.model_id,
                downloaded,
                total = ?self.total,
                "model download progress"
            );
            self.send_progress(downloaded);
            self.throttle.mark_emitted(downloaded, now);
        }
    }

    fn send_final(&mut self, downloaded: u64) {
        self.send_progress(downloaded);
        self.throttle.mark_emitted(downloaded, Instant::now());
    }

    fn send_verifying(&self, downloaded: u64) {
        let _ = self.command_tx.send(AppCommand::ModelDownloadVerifying {
            download_id: self.download_id,
            model_id: self.model_id.to_string(),
            downloaded,
            total: self.total,
        });
    }

    fn send_progress(&self, downloaded: u64) {
        let _ = self.command_tx.send(AppCommand::ModelDownloadProgress {
            download_id: self.download_id,
            model_id: self.model_id.to_string(),
            downloaded,
            total: self.total,
        });
    }
}

fn sha1_file_cancellable(path: &Path, cancel_requested: &AtomicBool) -> DownloadResult<String> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut hasher = Sha1::new();
    let mut buffer = [0_u8; DOWNLOAD_BUFFER_SIZE];

    loop {
        ensure_not_canceled(cancel_requested)?;
        let bytes = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if bytes == 0 {
            break;
        }
        hasher.update(&buffer[..bytes]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug)]
struct ProgressThrottle {
    last_emit_at: Instant,
    last_downloaded: u64,
}

impl Default for ProgressThrottle {
    fn default() -> Self {
        Self {
            last_emit_at: Instant::now(),
            last_downloaded: 0,
        }
    }
}

impl ProgressThrottle {
    fn should_emit(&self, downloaded: u64, now: Instant) -> bool {
        now.saturating_duration_since(self.last_emit_at) >= PROGRESS_MIN_INTERVAL
            && downloaded.saturating_sub(self.last_downloaded) >= PROGRESS_MIN_DELTA_BYTES
    }

    fn mark_emitted(&mut self, downloaded: u64, now: Instant) {
        self.last_emit_at = now;
        self.last_downloaded = downloaded;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_throttle_suppresses_too_frequent_updates() {
        let start = Instant::now();
        let mut throttle = ProgressThrottle::default();
        throttle.mark_emitted(0, start);

        assert!(!throttle.should_emit(PROGRESS_MIN_DELTA_BYTES, start));
        assert!(!throttle.should_emit(
            PROGRESS_MIN_DELTA_BYTES,
            start + PROGRESS_MIN_INTERVAL - Duration::from_millis(1)
        ));
    }

    #[test]
    fn progress_throttle_requires_minimum_byte_delta() {
        let start = Instant::now();
        let mut throttle = ProgressThrottle::default();
        throttle.mark_emitted(0, start);

        assert!(!throttle.should_emit(PROGRESS_MIN_DELTA_BYTES - 1, start + PROGRESS_MIN_INTERVAL));
    }

    #[test]
    fn progress_throttle_emits_after_interval_and_delta() {
        let start = Instant::now();
        let mut throttle = ProgressThrottle::default();
        throttle.mark_emitted(0, start);

        assert!(throttle.should_emit(PROGRESS_MIN_DELTA_BYTES, start + PROGRESS_MIN_INTERVAL));
    }

    #[test]
    fn progress_reporter_always_emits_final_progress() {
        let (command_tx, command_rx) = mpsc::channel();
        let mut reporter = DownloadProgressReporter::new(
            7,
            "ggml-base",
            Some(PROGRESS_MIN_DELTA_BYTES),
            &command_tx,
        );

        reporter.send_initial();
        reporter.send_throttled(512);
        reporter.send_final(PROGRESS_MIN_DELTA_BYTES);

        let commands: Vec<_> = command_rx.try_iter().collect();
        assert_eq!(commands.len(), 2);

        assert!(matches!(
            &commands[0],
            AppCommand::ModelDownloadProgress {
                download_id: 7,
                model_id,
                downloaded: 0,
                total: Some(PROGRESS_MIN_DELTA_BYTES),
            } if model_id == "ggml-base"
        ));
        assert!(matches!(
            &commands[1],
            AppCommand::ModelDownloadProgress {
                download_id: 7,
                model_id,
                downloaded: PROGRESS_MIN_DELTA_BYTES,
                total: Some(PROGRESS_MIN_DELTA_BYTES),
            } if model_id == "ggml-base"
        ));
    }
}
