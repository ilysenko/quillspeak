use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use gtk::gio;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{
    APP_ID, AppConfig, DaemonStatus, HotkeyBackend as HotkeyBackendKind, ShortcutRuntimeConfig,
};
use tracing::{error, info, warn};

use crate::command::AppCommand;
use crate::config_store::ConfigStore;
use crate::daemon_client::DaemonClient;
use crate::dbus::AppDbusHandle;
use crate::hotkey::{DaemonBackend, DisabledBackend, HotkeyBackend, backend_name_for_config};
use crate::recording::RecordingController;
use crate::settings::SettingsWindow;
use crate::tray::Tray;

pub fn run() -> gtk::glib::ExitCode {
    let application = adw::Application::builder().application_id(APP_ID).build();
    let runtime_slot: Rc<RefCell<Option<Rc<AppRuntime>>>> = Rc::new(RefCell::new(None));

    application.connect_activate({
        let runtime_slot = Rc::clone(&runtime_slot);
        move |application| {
            if runtime_slot.borrow().is_some() {
                return;
            }

            match AppRuntime::new(application) {
                Ok(runtime) => {
                    runtime_slot.replace(Some(runtime));
                }
                Err(error) => {
                    error!(?error, "failed to start MyApp");
                    application.quit();
                }
            }
        }
    });

    application.run()
}

struct AppRuntime {
    application: adw::Application,
    hold_guard: RefCell<Option<gio::ApplicationHoldGuard>>,
    command_tx: mpsc::Sender<AppCommand>,
    config_store: ConfigStore,
    config: RefCell<AppConfig>,
    daemon_client: DaemonClient,
    daemon_status: Cell<DaemonStatus>,
    shortcut_runtime_config: Arc<Mutex<ShortcutRuntimeConfig>>,
    recording: RefCell<RecordingController>,
    settings_window: RefCell<Option<SettingsWindow>>,
    tray: RefCell<Option<Tray>>,
    dbus_handle: RefCell<Option<AppDbusHandle>>,
}

impl AppRuntime {
    fn new(application: &adw::Application) -> Result<Rc<Self>> {
        let hold_guard = application.hold();
        let (command_tx, command_rx) = mpsc::channel();

        let config_store = ConfigStore::new()?;
        let config = config_store.load_or_create_default().with_context(|| {
            format!(
                "failed to load config from {}",
                config_store.path().display()
            )
        })?;

        let daemon_client = DaemonClient;
        let daemon_status = daemon_client.status();
        configure_hotkey_backend(&daemon_client, &config);
        let shortcut_runtime_config = Arc::new(Mutex::new(ShortcutRuntimeConfig::from(&config)));
        sync_shortcut_config_to_daemon(&daemon_client, &config);

        let tray = match Tray::new(command_tx.clone()) {
            Ok(tray) => {
                info!("StatusNotifierItem tray started");
                Some(tray)
            }
            Err(error) => {
                warn!(?error, "failed to start StatusNotifierItem tray");
                None
            }
        };

        let dbus_handle =
            AppDbusHandle::spawn(command_tx.clone(), Arc::clone(&shortcut_runtime_config));
        install_ctrl_c_handler(command_tx.clone());

        let runtime = Rc::new(Self {
            application: application.clone(),
            hold_guard: RefCell::new(Some(hold_guard)),
            command_tx,
            config_store,
            config: RefCell::new(config),
            daemon_client,
            daemon_status: Cell::new(daemon_status),
            shortcut_runtime_config,
            recording: RefCell::new(RecordingController::default()),
            settings_window: RefCell::new(None),
            tray: RefCell::new(tray),
            dbus_handle: RefCell::new(Some(dbus_handle)),
        });

        Self::attach_command_pump(&runtime, command_rx);
        runtime.log_startup_state();
        Ok(runtime)
    }

    fn attach_command_pump(runtime: &Rc<Self>, command_rx: mpsc::Receiver<AppCommand>) {
        let runtime = Rc::clone(runtime);
        gtk::glib::timeout_add_local(Duration::from_millis(50), move || {
            while let Ok(command) = command_rx.try_recv() {
                runtime.handle_command(command);
            }

            gtk::glib::ControlFlow::Continue
        });
    }

    fn handle_command(&self, command: AppCommand) {
        match command {
            AppCommand::ShowSettings => self.show_settings(),
            AppCommand::StartRecording => self.recording.borrow_mut().start_recording(),
            AppCommand::StopRecording => self.recording.borrow_mut().stop_recording(),
            AppCommand::SaveConfig(config) => self.save_config(config),
            AppCommand::DaemonStatusChanged(status) => self.set_daemon_status(status),
            AppCommand::Quit => self.quit(),
        }
    }

    fn show_settings(&self) {
        if self.settings_window.borrow().is_none() {
            let window = SettingsWindow::new(
                &self.application,
                &self.config.borrow(),
                self.daemon_status.get(),
                self.command_sender(),
            );
            self.settings_window.replace(Some(window));
        }

        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.update_config(&self.config.borrow());
            window.update_daemon_status(self.daemon_status.get());
            window.present();
        }
    }

    fn save_config(&self, config: AppConfig) {
        if let Err(error) = self.save_config_inner(config) {
            warn!(?error, "failed to save settings config");
        }
    }

    fn save_config_inner(&self, config: AppConfig) -> Result<()> {
        self.config_store.save(&config)?;
        let config = self.config_store.load_or_create_default()?;
        self.apply_config(config);
        Ok(())
    }

    fn apply_config(&self, config: AppConfig) {
        configure_hotkey_backend(&self.daemon_client, &config);
        sync_shortcut_config_to_daemon(&self.daemon_client, &config);
        if let Ok(mut runtime_config) = self.shortcut_runtime_config.lock() {
            *runtime_config = ShortcutRuntimeConfig::from(&config);
        }

        self.config.replace(config.clone());
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.update_config(&config);
        }
    }

    fn set_daemon_status(&self, status: DaemonStatus) {
        self.daemon_status.set(status);
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.update_daemon_status(status);
        }
    }

    fn quit(&self) {
        info!("Quitting MyApp");
        self.dbus_handle.borrow_mut().take();
        self.tray.borrow_mut().take();
        self.hold_guard.borrow_mut().take();
        self.application.quit();
    }

    fn log_startup_state(&self) {
        let config = self.config.borrow();
        info!(
            config_path = %self.config_store.path().display(),
            shortcut = %config.shortcuts.push_to_talk.accelerator,
            mode = %config.mode.as_str(),
            backend = %backend_name_for_config(&config),
            daemon_status = %self.daemon_status.get().display_label(),
            "MyApp started in foreground development mode"
        );

        if let Ok(status) = self.daemon_client.get_daemon_status() {
            self.set_daemon_status(status);
        }
    }

    fn command_sender(&self) -> mpsc::Sender<AppCommand> {
        self.command_tx.clone()
    }
}

fn configure_hotkey_backend(daemon_client: &DaemonClient, config: &AppConfig) {
    match config.hotkey_backend {
        HotkeyBackendKind::Disabled => {
            let backend = DisabledBackend;
            info!(backend = backend.name(), "configuring hotkey backend");
            if let Err(error) = backend.configure(config) {
                warn!(?error, "failed to configure disabled hotkey backend");
            }
        }
        HotkeyBackendKind::Daemon => {
            let backend = DaemonBackend::new(daemon_client.clone());
            info!(backend = backend.name(), "configuring hotkey backend");
            if let Err(error) = backend.configure(config) {
                warn!(?error, "daemon hotkey backend is not available yet");
            }
        }
        HotkeyBackendKind::X11 => {
            info!("X11 hotkey backend placeholder selected; using disabled backend for now");
        }
        HotkeyBackendKind::Portal => {
            info!("Portal hotkey backend placeholder selected; using disabled backend for now");
        }
    }
}

fn sync_shortcut_config_to_daemon(daemon_client: &DaemonClient, config: &AppConfig) {
    let runtime_config = ShortcutRuntimeConfig::from(config);
    if let Err(error) = daemon_client.update_shortcut_config(&runtime_config) {
        warn!(?error, "daemon shortcut config sync is not available yet");
    }
}

fn install_ctrl_c_handler(command_tx: mpsc::Sender<AppCommand>) {
    if let Err(error) = ctrlc::set_handler(move || {
        let _ = command_tx.send(AppCommand::Quit);
    }) {
        warn!(?error, "failed to install Ctrl-C handler");
    }
}
