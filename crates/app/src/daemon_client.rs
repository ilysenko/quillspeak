use std::env;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use directories::BaseDirs;
use shared::{
    DAEMON_BUS_NAME, DAEMON_INTERFACE, DAEMON_OBJECT_PATH, DaemonStatus, PasteShortcut,
    ShortcutRuntimeConfig,
};
use tracing::{debug, info, warn};
use zbus::blocking::{Connection, Proxy};

use crate::command::AppCommand;

#[derive(Debug, Clone, Default)]
pub struct DaemonClient;

impl DaemonClient {
    pub fn status(&self) -> DaemonStatus {
        match self.get_daemon_status() {
            Ok(status) => status,
            Err(error) if is_permission_error(&error) => DaemonStatus::PermissionError,
            Err(error) => {
                debug!(?error, "daemon status query failed");
                if daemon_appears_installed() {
                    DaemonStatus::InstalledButNotRunning
                } else {
                    DaemonStatus::NotInstalled
                }
            }
        }
    }

    pub fn get_daemon_status(&self) -> Result<DaemonStatus> {
        info!(
            bus_name = DAEMON_BUS_NAME,
            method = "GetDaemonStatus",
            "calling daemon D-Bus method"
        );
        let connection = Connection::session().context("failed to connect to session bus")?;
        let proxy = daemon_proxy(&connection)?;
        let status: String = proxy
            .call("GetDaemonStatus", &())
            .context("failed to query daemon status")?;
        info!(
            daemon_status = %DaemonStatus::from(status.as_str()).display_label(),
            method = "GetDaemonStatus",
            "daemon D-Bus method returned"
        );
        Ok(DaemonStatus::from(status.as_str()))
    }

    pub fn update_shortcut_config(&self, config: &ShortcutRuntimeConfig) -> Result<()> {
        info!(
            shortcut_count = config.shortcuts.len(),
            configured = config.is_configured(),
            method = "UpdateShortcutConfig",
            "calling daemon D-Bus method"
        );
        let connection = Connection::session().context("failed to connect to session bus")?;
        let proxy = daemon_proxy(&connection)?;
        let updated: bool = proxy
            .call("UpdateShortcutConfig", config)
            .context("failed to send shortcut config to daemon")?;
        info!(
            updated,
            method = "UpdateShortcutConfig",
            "daemon D-Bus method returned"
        );
        if !updated {
            warn!("daemon rejected shortcut config update");
        }
        Ok(())
    }

    pub fn paste_clipboard(&self, shortcut: PasteShortcut) -> Result<()> {
        info!(
            paste_shortcut = shortcut.as_wire_str(),
            method = "PasteClipboard",
            "calling daemon D-Bus method"
        );
        let connection = Connection::session().context("failed to connect to session bus")?;
        let proxy = daemon_proxy(&connection)?;
        let pasted: bool = proxy
            .call("PasteClipboard", &(shortcut.as_wire_str().to_string(),))
            .context("failed to request daemon clipboard paste")?;
        info!(
            pasted,
            paste_shortcut = shortcut.as_wire_str(),
            method = "PasteClipboard",
            "daemon D-Bus method returned"
        );
        if !pasted {
            anyhow::bail!("daemon rejected clipboard paste request");
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct DaemonClientWorker {
    worker_tx: mpsc::Sender<DaemonClientCommand>,
    join_handle: Option<JoinHandle<()>>,
}

impl DaemonClientWorker {
    pub fn spawn(command_tx: mpsc::Sender<AppCommand>) -> Result<Self> {
        let (worker_tx, worker_rx) = mpsc::channel();
        let join_handle = thread::Builder::new()
            .name("myapp-daemon-client".to_string())
            .spawn(move || daemon_client_worker_loop(worker_rx, command_tx))
            .context("failed to spawn daemon client worker")?;
        Ok(Self {
            worker_tx,
            join_handle: Some(join_handle),
        })
    }

    pub fn probe_status(&self) -> Result<()> {
        self.worker_tx
            .send(DaemonClientCommand::ProbeStatus)
            .map_err(|_| anyhow::anyhow!("daemon client worker is not running"))
    }

    pub fn sync_shortcut_config(&self, config: ShortcutRuntimeConfig) -> Result<()> {
        self.worker_tx
            .send(DaemonClientCommand::SyncShortcutConfig(Box::new(config)))
            .map_err(|_| anyhow::anyhow!("daemon client worker is not running"))
    }

    pub fn paste_clipboard(&self, shortcut: PasteShortcut) -> Result<()> {
        self.worker_tx
            .send(DaemonClientCommand::PasteClipboard(shortcut))
            .map_err(|_| anyhow::anyhow!("daemon client worker is not running"))
    }

    pub fn shutdown(mut self) {
        let _ = self.worker_tx.send(DaemonClientCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take()
            && let Err(error) = join_handle.join()
        {
            warn!(?error, "daemon client worker panicked during shutdown");
        }
    }
}

impl Drop for DaemonClientWorker {
    fn drop(&mut self) {
        let _ = self.worker_tx.send(DaemonClientCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

enum DaemonClientCommand {
    ProbeStatus,
    SyncShortcutConfig(Box<ShortcutRuntimeConfig>),
    PasteClipboard(PasteShortcut),
    Shutdown,
}

fn daemon_client_worker_loop(
    worker_rx: mpsc::Receiver<DaemonClientCommand>,
    command_tx: mpsc::Sender<AppCommand>,
) {
    let client = DaemonClient;
    for command in worker_rx {
        match command {
            DaemonClientCommand::ProbeStatus => {
                let status = client.status();
                let _ = command_tx.send(AppCommand::DaemonStatusChanged(status));
            }
            DaemonClientCommand::SyncShortcutConfig(config) => {
                if let Err(error) = client.update_shortcut_config(&config) {
                    warn!(?error, "daemon shortcut config sync is not available yet");
                }
            }
            DaemonClientCommand::PasteClipboard(shortcut) => {
                if let Err(error) = client.paste_clipboard(shortcut) {
                    warn!(
                        ?error,
                        paste_shortcut = shortcut.as_wire_str(),
                        "daemon clipboard paste is not available"
                    );
                }
            }
            DaemonClientCommand::Shutdown => break,
        }
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
        Ok(true) => DaemonStatus::RunningConfigured,
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
            DaemonStatus::RunningConfigured
        );
        assert_eq!(
            resolve_daemon_status(Err("AccessDenied".to_string()), true),
            DaemonStatus::PermissionError
        );
    }
}
