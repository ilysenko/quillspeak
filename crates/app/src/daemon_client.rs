use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::BaseDirs;
use shared::{
    AppConfig, DAEMON_BUS_NAME, DAEMON_INTERFACE, DAEMON_OBJECT_PATH, DaemonStatus, HotkeyConfig,
};
use tracing::{debug, warn};
use zbus::blocking::{Connection, Proxy};

#[derive(Debug, Clone, Default)]
pub struct DaemonClient;

impl DaemonClient {
    pub fn status(&self) -> DaemonStatus {
        match self.ping() {
            Ok(true) => DaemonStatus::Running,
            Ok(false) => DaemonStatus::InstalledButNotRunning,
            Err(error) if is_permission_error(&error) => DaemonStatus::PermissionError,
            Err(error) => {
                debug!(?error, "daemon ping failed");
                if daemon_appears_installed() {
                    DaemonStatus::InstalledButNotRunning
                } else {
                    DaemonStatus::NotInstalled
                }
            }
        }
    }

    pub fn ping(&self) -> Result<bool> {
        let connection = Connection::session().context("failed to connect to session bus")?;
        let proxy = daemon_proxy(&connection)?;
        proxy.call("Ping", &()).context("failed to ping daemon")
    }

    pub fn get_daemon_status(&self) -> Result<DaemonStatus> {
        let connection = Connection::session().context("failed to connect to session bus")?;
        let proxy = daemon_proxy(&connection)?;
        let status: String = proxy
            .call("GetDaemonStatus", &())
            .context("failed to query daemon status")?;
        Ok(DaemonStatus::from(status.as_str()))
    }

    pub fn update_hotkey_config(&self, config: &AppConfig) -> Result<()> {
        let connection = Connection::session().context("failed to connect to session bus")?;
        let proxy = daemon_proxy(&connection)?;
        let hotkey_config = HotkeyConfig::from(config);
        let updated: bool = proxy
            .call("UpdateHotkeyConfig", &hotkey_config)
            .context("failed to send hotkey config to daemon")?;
        if !updated {
            warn!("daemon rejected hotkey config update");
        }
        Ok(())
    }
}

fn daemon_proxy(connection: &Connection) -> Result<Proxy<'_>> {
    Proxy::new(
        connection,
        DAEMON_BUS_NAME,
        DAEMON_OBJECT_PATH,
        DAEMON_INTERFACE,
    )
    .context("failed to create daemon D-Bus proxy")
}

#[cfg(test)]
pub fn resolve_daemon_status(ping_result: Result<bool, String>, installed: bool) -> DaemonStatus {
    match ping_result {
        Ok(true) => DaemonStatus::Running,
        Ok(false) => DaemonStatus::InstalledButNotRunning,
        Err(error) if error.contains("AccessDenied") || error.contains("permission") => {
            DaemonStatus::PermissionError
        }
        Err(_) if installed => DaemonStatus::InstalledButNotRunning,
        Err(_) => DaemonStatus::NotInstalled,
    }
}

fn daemon_appears_installed() -> bool {
    binary_in_path("myapp-daemon") || user_service_exists()
}

fn binary_in_path(binary: &str) -> bool {
    env::var_os("PATH")
        .into_iter()
        .flat_map(|path| env::split_paths(&path).collect::<Vec<PathBuf>>())
        .any(|dir| dir.join(binary).is_file())
}

fn user_service_exists() -> bool {
    let Some(base_dirs) = BaseDirs::new() else {
        return false;
    };

    base_dirs
        .config_dir()
        .join("systemd/user/myapp-input-daemon.service")
        .is_file()
}

fn is_permission_error(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    text.contains("AccessDenied") || text.contains("Permission") || text.contains("permission")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_status_resolution_handles_absence() {
        assert_eq!(
            resolve_daemon_status(Err("no owner".to_string()), false),
            DaemonStatus::NotInstalled
        );
        assert_eq!(
            resolve_daemon_status(Err("no owner".to_string()), true),
            DaemonStatus::InstalledButNotRunning
        );
    }

    #[test]
    fn daemon_status_resolution_handles_running_and_permission_errors() {
        assert_eq!(
            resolve_daemon_status(Ok(true), false),
            DaemonStatus::Running
        );
        assert_eq!(
            resolve_daemon_status(Err("AccessDenied".to_string()), true),
            DaemonStatus::PermissionError
        );
    }
}
