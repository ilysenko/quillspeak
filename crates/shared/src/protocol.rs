use serde::{Deserialize, Serialize};
use zvariant::Type;

use crate::config::{AppConfig, HotkeyBackend, HotkeyMode};

pub const APP_ID: &str = "org.example.MyApp";
pub const APP_BUS_NAME: &str = "org.example.MyApp.App";
pub const APP_OBJECT_PATH: &str = "/org/example/MyApp/App";
pub const APP_INTERFACE: &str = "org.example.MyApp.App1";

pub const DAEMON_BUS_NAME: &str = "org.example.MyApp.InputDaemon";
pub const DAEMON_OBJECT_PATH: &str = "/org/example/MyApp/InputDaemon";
pub const DAEMON_INTERFACE: &str = "org.example.MyApp.InputDaemon1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonStatus {
    NotInstalled,
    InstalledButNotRunning,
    Running,
    PermissionError,
}

impl DaemonStatus {
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::InstalledButNotRunning => "installed_but_not_running",
            Self::Running => "running",
            Self::PermissionError => "permission_error",
        }
    }

    pub const fn display_label(self) -> &'static str {
        match self {
            Self::NotInstalled => "Not installed",
            Self::InstalledButNotRunning => "Installed but not running",
            Self::Running => "Running",
            Self::PermissionError => "Permission error",
        }
    }
}

impl From<&str> for DaemonStatus {
    fn from(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "permission_error" => Self::PermissionError,
            "installed_but_not_running" => Self::InstalledButNotRunning,
            "not_installed" => Self::NotInstalled,
            _ => Self::InstalledButNotRunning,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct HotkeyConfig {
    pub hotkey: String,
    pub mode: String,
    pub hotkey_backend: String,
}

impl From<&AppConfig> for HotkeyConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            hotkey: config.hotkey.clone(),
            mode: config.mode.as_str().to_string(),
            hotkey_backend: config.hotkey_backend.as_str().to_string(),
        }
    }
}

impl From<HotkeyConfig> for AppConfig {
    fn from(config: HotkeyConfig) -> Self {
        Self {
            hotkey: config.hotkey,
            mode: HotkeyMode::try_from(config.mode.as_str()).unwrap_or_default(),
            hotkey_backend: HotkeyBackend::try_from(config.hotkey_backend.as_str())
                .unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_status_labels_match_user_facing_contract() {
        assert_eq!(DaemonStatus::NotInstalled.display_label(), "Not installed");
        assert_eq!(
            DaemonStatus::InstalledButNotRunning.display_label(),
            "Installed but not running"
        );
        assert_eq!(DaemonStatus::Running.display_label(), "Running");
        assert_eq!(
            DaemonStatus::PermissionError.display_label(),
            "Permission error"
        );
    }

    #[test]
    fn protocol_constants_are_stable() {
        assert_eq!(APP_BUS_NAME, "org.example.MyApp.App");
        assert_eq!(DAEMON_BUS_NAME, "org.example.MyApp.InputDaemon");
        assert_eq!(APP_OBJECT_PATH, "/org/example/MyApp/App");
        assert_eq!(DAEMON_OBJECT_PATH, "/org/example/MyApp/InputDaemon");
    }

    #[test]
    fn converts_config_to_wire_shape() {
        let wire = HotkeyConfig::from(&AppConfig::default());

        assert_eq!(wire.hotkey, "Ctrl+Space");
        assert_eq!(wire.mode, "push_to_talk");
        assert_eq!(wire.hotkey_backend, "disabled");
    }
}
