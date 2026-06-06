use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

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
        let join_handle = thread::spawn(move || {
            if let Err(error) = run_dbus_service(command_tx, shortcut_config, shutdown_rx) {
                error!(?error, "app D-Bus service stopped");
            }
        });

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
) -> zbus::Result<()> {
    let service = AppDbusService {
        command_tx,
        shortcut_config,
    };
    let _connection = connection::Builder::session()?
        .name(APP_BUS_NAME)?
        .serve_at(APP_OBJECT_PATH, service)?
        .build()?;

    info!(
        bus_name = APP_BUS_NAME,
        object_path = APP_OBJECT_PATH,
        "app D-Bus interface is running"
    );

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
    fn hotkey_down(&self) -> fdo::Result<()> {
        self.send_command(AppCommand::StartRecording)
    }

    #[zbus(name = "HotkeyUp")]
    fn hotkey_up(&self) -> fdo::Result<()> {
        self.send_command(AppCommand::StopRecording)
    }

    #[zbus(name = "DaemonStatus")]
    fn daemon_status(&self, status: String) -> fdo::Result<()> {
        self.send_command(AppCommand::DaemonStatusChanged(DaemonStatus::from(
            status.as_str(),
        )))
    }

    #[zbus(name = "GetShortcutConfig")]
    fn get_shortcut_config(&self) -> fdo::Result<ShortcutRuntimeConfig> {
        self.shortcut_config
            .lock()
            .map(|config| config.clone())
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
