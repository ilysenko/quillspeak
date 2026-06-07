use shared::AppConfig;
use shared::DaemonStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    ShowSettings,
    ToggleRecording,
    StartRecording(String),
    StopRecording(String),
    TranscriptionFinished {
        shortcut_id: String,
        result: Result<(), String>,
    },
    SaveConfig(AppConfig),
    DownloadModel(String),
    DeleteModel(String),
    ModelDownloadProgress {
        model_id: String,
        downloaded: u64,
        total: Option<u64>,
    },
    ModelDownloadFinished {
        model_id: String,
        result: Result<(), String>,
    },
    DaemonAppeared(DaemonStatus),
    DaemonVanished(DaemonStatus),
    DaemonStatusChanged(DaemonStatus),
    Quit,
}
