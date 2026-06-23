use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result, anyhow};
use tracing::warn;

use crate::command::AppCommand;
use crate::transcription::engine::WhisperEngine;
use crate::transcription::status::WhisperRuntimeStatus;
use crate::transcription::types::TranscriptionRequest;

pub struct TranscriptionService {
    worker_tx: mpsc::Sender<TranscriptionWorkerCommand>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl TranscriptionService {
    pub fn spawn(command_tx: mpsc::Sender<AppCommand>, keep_model_loaded: bool) -> Result<Self> {
        let (worker_tx, worker_rx) = mpsc::channel();
        thread::Builder::new()
            .name("quillspeak-transcription".to_string())
            .spawn(move || transcription_worker_loop(worker_rx, command_tx, keep_model_loaded))
            .context("failed to spawn transcription worker")
            .map(|join_handle| Self {
                worker_tx,
                join_handle: Some(join_handle),
            })
    }

    pub fn submit(&self, request: Box<TranscriptionRequest>) -> Result<()> {
        self.worker_tx
            .send(TranscriptionWorkerCommand::Transcribe(request))
            .map_err(|_| anyhow!("transcription worker is not running"))
    }

    pub fn set_keep_model_loaded(&self, keep_model_loaded: bool) -> Result<()> {
        self.worker_tx
            .send(TranscriptionWorkerCommand::SetKeepModelLoaded(
                keep_model_loaded,
            ))
            .map_err(|_| anyhow!("transcription worker is not running"))
    }

    pub fn clear_cached_model_path(&self, path: std::path::PathBuf) -> Result<()> {
        self.worker_tx
            .send(TranscriptionWorkerCommand::ClearCachedModelPath(path))
            .map_err(|_| anyhow!("transcription worker is not running"))
    }

    pub fn clear_cached_context(&self, reason: impl Into<String>) -> Result<()> {
        self.worker_tx
            .send(TranscriptionWorkerCommand::ClearCachedContext(
                reason.into(),
            ))
            .map_err(|_| anyhow!("transcription worker is not running"))
    }

    pub fn shutdown(mut self) {
        self.shutdown_inner();
    }

    fn shutdown_inner(&mut self) {
        let _ = self.worker_tx.send(TranscriptionWorkerCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take()
            && let Err(error) = join_handle.join()
        {
            warn!(?error, "transcription worker panicked during shutdown");
        }
    }
}

impl Drop for TranscriptionService {
    fn drop(&mut self) {
        self.shutdown_inner();
    }
}

enum TranscriptionWorkerCommand {
    Transcribe(Box<TranscriptionRequest>),
    SetKeepModelLoaded(bool),
    ClearCachedModelPath(std::path::PathBuf),
    ClearCachedContext(String),
    Shutdown,
}

fn transcription_worker_loop(
    worker_rx: mpsc::Receiver<TranscriptionWorkerCommand>,
    command_tx: mpsc::Sender<AppCommand>,
    keep_model_loaded: bool,
) {
    let mut engine = WhisperEngine::new(keep_model_loaded);
    for command in worker_rx {
        match command {
            TranscriptionWorkerCommand::Transcribe(request) => {
                let request = *request;
                let recording_id = request.recording_id;
                let shortcut_id = request.shortcut_id.clone();
                let configured_compute = request.compute_backend;
                let result = engine.transcribe(request);
                let runtime_status = engine.take_runtime_status_update().or_else(|| {
                    result.as_ref().err().map(|error| {
                        WhisperRuntimeStatus::failed(configured_compute, format!("{error:#}"))
                    })
                });
                if let Some(status) = runtime_status {
                    let _ = command_tx.send(AppCommand::WhisperRuntimeStatusChanged(status));
                }
                let result = result.map(Box::new).map_err(|error| format!("{error:#}"));
                let _ = command_tx.send(AppCommand::TranscriptionFinished {
                    recording_id,
                    shortcut_id,
                    result,
                });
            }
            TranscriptionWorkerCommand::SetKeepModelLoaded(keep_model_loaded) => {
                engine.set_keep_model_loaded(keep_model_loaded);
            }
            TranscriptionWorkerCommand::ClearCachedModelPath(path) => {
                engine.clear_cached_model_path(&path);
            }
            TranscriptionWorkerCommand::ClearCachedContext(reason) => {
                engine.clear_cached_context_for_config_change(&reason);
            }
            TranscriptionWorkerCommand::Shutdown => break,
        }
    }
}
