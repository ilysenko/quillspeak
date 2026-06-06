use shared::DaemonStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    ShowSettings,
    StartRecording,
    StopRecording,
    DaemonStatusChanged(DaemonStatus),
    Quit,
}
