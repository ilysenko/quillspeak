use crate::external_trigger::{ExternalTriggerRequest, ExternalTriggerResponse};
use crate::output::{
    ClipboardCopyOutcome, ClipboardCopySource, ClipboardPasteOutcome, OutputScriptResult,
};
use crate::transcription::TranscriptionRequest;
use crate::transcription::TranscriptionResult;
use crate::transcription::WhisperRuntimeStatus;
use shared::AppConfig;

pub type DownloadId = u64;

#[derive(Debug)]
pub enum AppCommand {
    ShowSettings,
    ToggleRecording,
    LinuxSignalReceived(i32),
    ExternalTrigger {
        request: ExternalTriggerRequest,
        deadline: std::time::Instant,
        response_tx: std::sync::mpsc::Sender<ExternalTriggerResponse>,
    },
    StartRecording(String),
    StopRecording(String),
    RecordingStartCueFinished {
        recording_id: u64,
        shortcut_id: String,
        result: Result<(), String>,
    },
    RecordingDurationLimitReached {
        recording_id: u64,
        shortcut_id: String,
    },
    AudioCaptureStarted {
        recording_id: u64,
        shortcut_id: String,
        input_label: String,
        startup_latency_ms: u128,
        first_callback_latency_ms: Option<u128>,
    },
    AudioCaptureStartFailed {
        recording_id: u64,
        shortcut_id: String,
        error: String,
    },
    AudioCaptureStopped {
        recording_id: u64,
        shortcut_id: String,
        result: Result<Box<TranscriptionRequest>, String>,
    },
    TranscriptionFinished {
        recording_id: u64,
        shortcut_id: String,
        result: Result<Box<TranscriptionResult>, String>,
    },
    WhisperRuntimeStatusChanged(WhisperRuntimeStatus),
    RefreshTrayRecordingPhase,
    OutputScriptFinished {
        recording_id: u64,
        shortcut_id: String,
        result: Result<OutputScriptResult, String>,
    },
    ClipboardCopyFinished {
        source: ClipboardCopySource,
        result: Result<ClipboardCopyOutcome, String>,
    },
    ClipboardPasteFinished {
        source: ClipboardCopySource,
        result: Result<ClipboardPasteOutcome, String>,
    },
    SpeakerRestoreFinished {
        recording_id: u64,
        shortcut_id: String,
    },
    AudioInputDevicesRefreshed(Vec<crate::audio::AudioInputDevice>),
    ClearHistory,
    CopyHistoryText {
        recording_id: u64,
        shortcut_id: String,
        text: String,
    },
    SaveConfig(AppConfig),
    DownloadModel(String),
    CancelModelDownload(String),
    DeleteModel(String),
    ModelDownloadProgress {
        download_id: DownloadId,
        model_id: String,
        downloaded: u64,
        total: Option<u64>,
    },
    ModelDownloadVerifying {
        download_id: DownloadId,
        model_id: String,
        downloaded: u64,
        total: Option<u64>,
    },
    ModelDownloadFinished {
        download_id: DownloadId,
        model_id: String,
        outcome: ModelDownloadOutcome,
    },
    ShutdownComplete,
    Quit,
}

impl AppCommand {
    /// Answers commands that carry a reply channel so external clients are not
    /// left waiting out their response timeout; returns the command back when
    /// no reply was pending.
    pub fn reject_pending_reply(self, reason: &str) -> Option<Self> {
        match self {
            Self::ExternalTrigger { response_tx, .. } => {
                let _ = response_tx.send(ExternalTriggerResponse::rejected(reason));
                None
            }
            other => Some(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelDownloadOutcome {
    Completed,
    Canceled,
    Failed(String),
}
