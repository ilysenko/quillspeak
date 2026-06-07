use shared::AppConfig;
use shared::DaemonStatus;

use crate::transcription::TranscriptionResult;

pub type DownloadId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    ShowSettings,
    ToggleRecording,
    StartRecording(String),
    StopRecording(String),
    TranscriptionFinished {
        shortcut_id: String,
        result: Result<TranscriptionResult, String>,
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
