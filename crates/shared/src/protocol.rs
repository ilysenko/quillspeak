use serde::{Deserialize, Serialize};
use zvariant::Type;

use crate::config::{
    AppConfig, CONFIG_SCHEMA_VERSION, ConfigError, HotkeyBackend, ShortcutTrigger,
};

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
#[serde(deny_unknown_fields)]
pub struct ShortcutRuntimeBinding {
    pub id: String,
    pub name: String,
    pub accelerator: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct ShortcutRuntimeConfig {
    pub schema_version: u32,
    pub shortcuts: Vec<ShortcutRuntimeBinding>,
}

impl ShortcutRuntimeConfig {
    pub fn validate_current_schema(&self) -> Result<(), ConfigError> {
        if self.schema_version == CONFIG_SCHEMA_VERSION {
            Ok(())
        } else {
            Err(ConfigError::UnsupportedSchemaVersion(self.schema_version))
        }
    }

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
                .filter_map(|shortcut| {
                    let ShortcutTrigger::Keyboard { accelerator } = &shortcut.trigger else {
                        return None;
                    };
                    Some(ShortcutRuntimeBinding {
                        id: shortcut.id.clone(),
                        name: shortcut.name.clone(),
                        accelerator: accelerator.clone(),
                        enabled: daemon_enabled && shortcut.enabled,
                    })
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
                .filter_map(|shortcut| {
                    let ShortcutTrigger::Keyboard { accelerator } = &shortcut.trigger else {
                        return None;
                    };
                    Some(ShortcutRuntimeBinding {
                        id: shortcut.id.clone(),
                        name: shortcut.name.clone(),
                        accelerator: accelerator.clone(),
                        enabled: shortcut.enabled,
                    })
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::ShortcutTrigger;

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
        assert_eq!(wire.shortcuts[0].id, "default");
        assert_eq!(wire.shortcuts[0].name, "Default");
        assert_eq!(wire.shortcuts[0].accelerator, "Ctrl+Alt+Space");
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

    #[test]
    fn daemon_runtime_config_omits_linux_signal_triggers() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::default_linux_signal();

        let daemon_wire = ShortcutRuntimeConfig::for_daemon(&config, HotkeyBackend::Daemon);

        assert!(!daemon_wire.is_configured());
        assert!(daemon_wire.shortcuts.is_empty());
    }

    #[test]
    fn runtime_config_rejects_unsupported_schema() {
        let mut wire = ShortcutRuntimeConfig::from(&AppConfig::default());
        wire.schema_version = CONFIG_SCHEMA_VERSION - 1;

        assert_eq!(
            wire.validate_current_schema(),
            Err(ConfigError::UnsupportedSchemaVersion(
                CONFIG_SCHEMA_VERSION - 1
            ))
        );
    }
}
