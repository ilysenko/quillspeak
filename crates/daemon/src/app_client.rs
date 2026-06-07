use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use shared::{APP_BUS_NAME, APP_INTERFACE, APP_OBJECT_PATH, DaemonStatus, ShortcutRuntimeConfig};
use tracing::{debug, info, warn};
use zbus::blocking::{Connection, Proxy};

#[derive(Debug)]
pub struct AppClientHandle {
    client: AppClient,
    join_handle: Option<JoinHandle<()>>,
}

impl AppClientHandle {
    pub fn spawn() -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let join_handle = thread::Builder::new()
            .name("myapp-daemon-app-client".to_string())
            .spawn(move || app_client_loop(command_rx))
            .ok();
        Self {
            client: AppClient { command_tx },
            join_handle,
        }
    }

    pub fn client(&self) -> AppClient {
        self.client.clone()
    }
}

impl Drop for AppClientHandle {
    fn drop(&mut self) {
        let _ = self.client.command_tx.send(AppClientCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take()
            && let Err(error) = join_handle.join()
        {
            warn!(?error, "daemon app client worker panicked during shutdown");
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppClient {
    command_tx: mpsc::Sender<AppClientCommand>,
}

impl AppClient {
    pub fn send_hotkey_down(&self, shortcut_id: &str) {
        self.send(AppClientCommand::Hotkey {
            method: HotkeyMethod::Down,
            shortcut_id: shortcut_id.to_string(),
        });
    }

    pub fn send_hotkey_up(&self, shortcut_id: &str) {
        self.send(AppClientCommand::Hotkey {
            method: HotkeyMethod::Up,
            shortcut_id: shortcut_id.to_string(),
        });
    }

    pub fn notify_status(&self, status: DaemonStatus) {
        self.send(AppClientCommand::DaemonStatus(status));
    }

    fn send(&self, command: AppClientCommand) {
        if let Err(error) = self.command_tx.send(command) {
            debug!(?error, "failed to enqueue app D-Bus command");
        }
    }
}

#[derive(Debug)]
enum AppClientCommand {
    Hotkey {
        method: HotkeyMethod,
        shortcut_id: String,
    },
    DaemonStatus(DaemonStatus),
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyMethod {
    Down,
    Up,
}

impl HotkeyMethod {
    pub const fn method_name(self) -> &'static str {
        match self {
            Self::Down => "HotkeyDown",
            Self::Up => "HotkeyUp",
        }
    }
}

fn app_client_loop(command_rx: mpsc::Receiver<AppClientCommand>) {
    info!("daemon app client worker started");
    for command in command_rx {
        match command {
            AppClientCommand::Hotkey {
                method,
                shortcut_id,
            } => {
                if let Err(error) = send_hotkey_method(method, &shortcut_id) {
                    debug!(
                        ?error,
                        shortcut_id,
                        method = method.method_name(),
                        "failed to send hotkey event to app"
                    );
                }
            }
            AppClientCommand::DaemonStatus(status) => {
                if let Err(error) = notify_app_status(status) {
                    debug!(?error, "could not notify app about daemon status");
                }
            }
            AppClientCommand::Shutdown => break,
        }
    }
    info!("daemon app client worker stopped");
}

pub fn send_hotkey_method(method: HotkeyMethod, shortcut_id: &str) -> Result<()> {
    info!(
        method = method.method_name(),
        shortcut_id,
        bus_name = APP_BUS_NAME,
        "calling app D-Bus method"
    );
    let connection = Connection::session().context("failed to connect to the D-Bus session bus")?;
    let proxy =
        app_proxy(&connection).context("failed to create app D-Bus proxy; is myapp running?")?;

    proxy
        .call::<_, _, ()>(method.method_name(), &(shortcut_id.to_string(),))
        .with_context(|| {
            format!(
                "failed to call app method {}; is myapp running?",
                method.method_name()
            )
        })?;

    info!(
        method = method.method_name(),
        shortcut_id, "sent hotkey event to app"
    );
    Ok(())
}

pub fn request_shortcut_config() -> Result<ShortcutRuntimeConfig> {
    info!(
        method = "GetShortcutConfig",
        bus_name = APP_BUS_NAME,
        "calling app D-Bus method"
    );
    let connection = Connection::session().context("failed to connect to the D-Bus session bus")?;
    let proxy = app_proxy(&connection).context("failed to create app D-Bus proxy")?;
    proxy
        .call("GetShortcutConfig", &())
        .context("failed to call app GetShortcutConfig")
}

fn notify_app_status(status: DaemonStatus) -> Result<()> {
    info!(
        daemon_status = %status.display_label(),
        method = "DaemonStatus",
        bus_name = APP_BUS_NAME,
        "calling app D-Bus method"
    );
    let connection = Connection::session().context("failed to connect to the D-Bus session bus")?;
    let proxy = app_proxy(&connection).context("failed to create app D-Bus proxy")?;
    proxy
        .call::<_, _, ()>("DaemonStatus", &(status.as_wire_str().to_string(),))
        .context("failed to call app DaemonStatus")
}

fn app_proxy(connection: &Connection) -> Result<Proxy<'_>> {
    Proxy::new(connection, APP_BUS_NAME, APP_OBJECT_PATH, APP_INTERFACE)
        .context("failed to create app D-Bus proxy")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_method_names_match_app_dbus_contract() {
        assert_eq!(HotkeyMethod::Down.method_name(), "HotkeyDown");
        assert_eq!(HotkeyMethod::Up.method_name(), "HotkeyUp");
    }
}
