use shared::AppConfig;
use shared::DaemonStatus;

use crate::transcription::TranscriptionRequest;
use crate::transcription::TranscriptionResult;

pub type DownloadId = u64;

#[derive(Debug)]
pub enum AppCommand {
    ShowSettings,
    ToggleRecording,
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
    DaemonAppeared(DaemonStatus),
    DaemonVanished(DaemonStatus),
    DaemonStatusChanged(DaemonStatus),
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelDownloadOutcome {
    Completed,
    Canceled,
    Failed(String),
}
