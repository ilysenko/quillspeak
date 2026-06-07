use serde::{Deserialize, Serialize};
use zvariant::Type;

use crate::config::{AppConfig, HotkeyBackend, ShortcutAction};

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
    RunningUnconfigured,
    RunningConfigured,
    PermissionError,
}

impl DaemonStatus {
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::InstalledButNotRunning => "installed_but_not_running",
            Self::RunningUnconfigured => "running_unconfigured",
            Self::RunningConfigured => "running_configured",
            Self::PermissionError => "permission_error",
        }
    }

    pub const fn display_label(self) -> &'static str {
        match self {
            Self::NotInstalled => "Not installed",
            Self::InstalledButNotRunning => "Installed but not running",
            Self::RunningUnconfigured | Self::RunningConfigured => "Running",
            Self::PermissionError => "Permission error",
        }
    }

    pub const fn is_running(self) -> bool {
        matches!(self, Self::RunningConfigured | Self::RunningUnconfigured)
    }
}

impl From<&str> for DaemonStatus {
    fn from(value: &str) -> Self {
        match value {
            "running" | "running_configured" => Self::RunningConfigured,
            "running_unconfigured" => Self::RunningUnconfigured,
            "permission_error" => Self::PermissionError,
            "installed_but_not_running" => Self::InstalledButNotRunning,
            "not_installed" => Self::NotInstalled,
            _ => Self::InstalledButNotRunning,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ShortcutRuntimeBinding {
    pub action: String,
    pub accelerator: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ShortcutRuntimeConfig {
    pub schema_version: u32,
    pub shortcuts: Vec<ShortcutRuntimeBinding>,
}

impl ShortcutRuntimeConfig {
    pub fn is_configured(&self) -> bool {
        self.shortcuts
            .iter()
            .any(|shortcut| shortcut.enabled && !shortcut.accelerator.trim().is_empty())
    }

    pub fn for_daemon(config: &AppConfig, effective_backend: HotkeyBackend) -> Self {
        let daemon_enabled = matches!(effective_backend, HotkeyBackend::Daemon);
        Self {
            schema_version: config.schema_version,
            shortcuts: config
                .shortcuts
                .iter()
                .into_iter()
                .map(|(action, binding)| ShortcutRuntimeBinding {
                    action: action.as_str().to_string(),
                    accelerator: binding.accelerator.clone(),
                    enabled: daemon_enabled && binding.enabled,
                })
                .collect(),
        }
    }
}

impl From<&AppConfig> for ShortcutRuntimeConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            schema_version: config.schema_version,
            shortcuts: config
                .shortcuts
                .iter()
                .into_iter()
                .map(|(action, binding)| ShortcutRuntimeBinding {
                    action: action.as_str().to_string(),
                    accelerator: binding.accelerator.clone(),
                    enabled: binding.enabled,
                })
                .collect(),
        }
    }
}

impl ShortcutRuntimeBinding {
    pub fn action(&self) -> Option<ShortcutAction> {
        ShortcutAction::try_from(self.action.as_str()).ok()
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
        assert_eq!(DaemonStatus::RunningUnconfigured.display_label(), "Running");
        assert_eq!(DaemonStatus::RunningConfigured.display_label(), "Running");
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
    fn converts_config_to_runtime_shape() {
        let wire = ShortcutRuntimeConfig::from(&AppConfig::default());

        assert!(wire.is_configured());
        assert_eq!(wire.shortcuts.len(), 1);
        assert_eq!(wire.shortcuts[0].action, "push_to_talk");
        assert_eq!(wire.shortcuts[0].accelerator, "Ctrl+Space");
        assert!(wire.shortcuts[0].enabled);
    }

    #[test]
    fn daemon_runtime_config_only_enables_daemon_backend() {
        let config = AppConfig::default();
        let daemon_wire = ShortcutRuntimeConfig::for_daemon(&config, HotkeyBackend::Daemon);
        let x11_wire = ShortcutRuntimeConfig::for_daemon(&config, HotkeyBackend::X11);
        let disabled_wire = ShortcutRuntimeConfig::for_daemon(&config, HotkeyBackend::Disabled);

        assert!(daemon_wire.is_configured());
        assert!(daemon_wire.shortcuts[0].enabled);
        assert!(!x11_wire.is_configured());
        assert!(!x11_wire.shortcuts[0].enabled);
        assert!(!disabled_wire.is_configured());
        assert!(!disabled_wire.shortcuts[0].enabled);
    }
}
