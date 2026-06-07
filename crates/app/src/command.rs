use shared::AppConfig;
use shared::DaemonStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    ShowSettings,
    StartRecording,
    StopRecording,
    SaveConfig(AppConfig),
    DaemonAppeared(DaemonStatus),
    DaemonVanished(DaemonStatus),
    DaemonStatusChanged(DaemonStatus),
    Quit,
}
