use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use shared::{APP_BUS_NAME, APP_OBJECT_PATH, DaemonStatus, ShortcutRuntimeConfig};
use tracing::{error, info};
use zbus::{blocking::connection, fdo, interface};

use crate::command::AppCommand;

pub struct AppDbusHandle {
    shutdown_tx: mpsc::Sender<()>,
    join_handle: Option<JoinHandle<()>>,
}

impl AppDbusHandle {
    pub fn spawn(
        command_tx: mpsc::Sender<AppCommand>,
        shortcut_config: Arc<Mutex<ShortcutRuntimeConfig>>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let join_handle = thread::spawn(move || {
            if let Err(error) = run_dbus_service(command_tx, shortcut_config, shutdown_rx, ready_tx)
            {
                error!(?error, "app D-Bus service stopped");
            }
        });

        match ready_rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => error!(error, "app D-Bus service failed during startup"),
            Err(error) => error!(?error, "timed out waiting for app D-Bus service startup"),
        }

        Self {
            shutdown_tx,
            join_handle: Some(join_handle),
        }
    }

    pub fn shutdown(&mut self) {
        let _ = self.shutdown_tx.send(());
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl Drop for AppDbusHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_dbus_service(
    command_tx: mpsc::Sender<AppCommand>,
    shortcut_config: Arc<Mutex<ShortcutRuntimeConfig>>,
    shutdown_rx: mpsc::Receiver<()>,
    ready_tx: mpsc::Sender<Result<(), String>>,
) -> zbus::Result<()> {
    let service = AppDbusService {
        command_tx,
        shortcut_config,
    };
    let connection = connection::Builder::session()?
        .name(APP_BUS_NAME)?
        .serve_at(APP_OBJECT_PATH, service)?
        .build();

    let _connection = match connection {
        Ok(connection) => connection,
        Err(error) => {
            let _ = ready_tx.send(Err(format!("{error:#}")));
            return Err(error);
        }
    };

    info!(
        bus_name = APP_BUS_NAME,
        object_path = APP_OBJECT_PATH,
        "app D-Bus interface is running"
    );
    let _ = ready_tx.send(Ok(()));

    let _ = shutdown_rx.recv();
    Ok(())
}

struct AppDbusService {
    command_tx: mpsc::Sender<AppCommand>,
    shortcut_config: Arc<Mutex<ShortcutRuntimeConfig>>,
}

#[interface(name = "org.example.MyApp.App1")]
impl AppDbusService {
    #[zbus(name = "HotkeyDown")]
    fn hotkey_down(&self, shortcut_id: String) -> fdo::Result<()> {
        info!(
            method = "HotkeyDown",
            shortcut_id, "received app D-Bus method"
        );
        self.send_command(AppCommand::StartRecording(shortcut_id))
    }

    #[zbus(name = "HotkeyUp")]
    fn hotkey_up(&self, shortcut_id: String) -> fdo::Result<()> {
        info!(
            method = "HotkeyUp",
            shortcut_id, "received app D-Bus method"
        );
        self.send_command(AppCommand::StopRecording(shortcut_id))
    }

    #[zbus(name = "DaemonStatus")]
    fn daemon_status(&self, status: String) -> fdo::Result<()> {
        let status = DaemonStatus::from(status.as_str());
        info!(
            daemon_status = %status.display_label(),
            method = "DaemonStatus",
            "received app D-Bus method"
        );
        self.send_command(AppCommand::DaemonStatusChanged(status))
    }

    #[zbus(name = "GetShortcutConfig")]
    fn get_shortcut_config(&self) -> fdo::Result<ShortcutRuntimeConfig> {
        info!(method = "GetShortcutConfig", "received app D-Bus method");
        self.shortcut_config
            .lock()
            .map(|config| {
                info!(
                    shortcut_count = config.shortcuts.len(),
                    configured = config.is_configured(),
                    method = "GetShortcutConfig",
                    "returning app shortcut config over D-Bus"
                );
                config.clone()
            })
            .map_err(|error| {
                fdo::Error::Failed(format!("failed to read shortcut runtime config: {error}"))
            })
    }
}

impl AppDbusService {
    fn send_command(&self, command: AppCommand) -> fdo::Result<()> {
        self.command_tx
            .send(command)
            .map_err(|error| fdo::Error::Failed(format!("failed to enqueue app command: {error}")))
    }
}
