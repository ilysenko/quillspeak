use shared::AppConfig;
use shared::DaemonStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    ShowSettings,
    StartRecording,
    StopRecording,
    SaveConfig(AppConfig),
    DaemonStatusChanged(DaemonStatus),
    Quit,
}
