use crate::output::{
    ClipboardCopyOutcome, ClipboardCopySource, ClipboardPasteOutcome, OutputScriptResult,
};
use crate::transcription::TranscriptionRequest;
use crate::transcription::TranscriptionResult;
use shared::AppConfig;

pub type DownloadId = u64;

#[derive(Debug)]
pub enum AppCommand {
    ShowSettings,
    ToggleRecording,
    LinuxSignalReceived(i32),
    StartRecording(String),
    StopRecording(String),
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
    RefreshTrayRecordingPhase,
    OutputScriptFinished {
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
    AudioInputDevicesRefreshed(Vec<crate::audio::AudioInputDevice>),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelDownloadOutcome {
    Completed,
    Canceled,
    Failed(String),
}
