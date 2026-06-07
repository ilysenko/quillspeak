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
    APP_ID, AppConfig, DEFAULT_SHORTCUT_ID, DaemonStatus, ResolvedOutput, ShortcutRuntimeConfig,
};
use tracing::{debug, error, info, warn};

use crate::command::{AppCommand, DownloadId, ModelDownloadOutcome};
use crate::config_store::ConfigStore;
use crate::daemon_client::DaemonClient;
use crate::daemon_monitor::DaemonMonitorHandle;
use crate::dbus::AppDbusHandle;
use crate::hotkey::{
    HotkeyBackendHandle, backend_name_for_config, configure_hotkey_backend, resolve_backend_kind,
};
use crate::models::{FinishEffect, ModelDownloadManager, ModelRowState, ModelStore};
use crate::recording::{RecordingPhase, RecordingService, spawn_transcription_job};
use crate::settings::SettingsWindow;
use crate::tray::Tray;

const COMMAND_PUMP_INTERVAL: Duration = Duration::from_millis(50);
const MAX_COMMANDS_PER_PUMP: usize = 128;

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
    model_store: ModelStore,
    download_manager: RefCell<ModelDownloadManager>,
    config: RefCell<AppConfig>,
    daemon_client: DaemonClient,
    daemon_status: Cell<DaemonStatus>,
    shortcut_runtime_config: Arc<Mutex<ShortcutRuntimeConfig>>,
    recording: RefCell<RecordingService>,
    settings_window: RefCell<Option<SettingsWindow>>,
    tray: RefCell<Option<Tray>>,
    hotkey_backend: RefCell<Option<HotkeyBackendHandle>>,
    dbus_handle: RefCell<Option<AppDbusHandle>>,
    daemon_monitor: RefCell<Option<DaemonMonitorHandle>>,
}

impl AppRuntime {
    fn new(application: &adw::Application) -> Result<Rc<Self>> {
        let hold_guard = application.hold();
        let (command_tx, command_rx) = mpsc::channel();

        let config_store = ConfigStore::new()?;
        let model_store = ModelStore::new()?;
        let config = config_store.load_or_create_default().with_context(|| {
            format!(
                "failed to load config from {}",
                config_store.path().display()
            )
        })?;

        let daemon_client = DaemonClient;
        let daemon_runtime_config = daemon_runtime_config_for(&config);
        let shortcut_runtime_config = Arc::new(Mutex::new(daemon_runtime_config.clone()));
        let dbus_handle =
            AppDbusHandle::spawn(command_tx.clone(), Arc::clone(&shortcut_runtime_config));

        let daemon_status = daemon_client.status();
        let hotkey_backend = configure_hotkey_backend(command_tx.clone(), &config);
        sync_shortcut_config_to_daemon(&daemon_client, &daemon_runtime_config);

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

        let daemon_monitor = DaemonMonitorHandle::spawn(command_tx.clone(), daemon_client.clone());
        install_ctrl_c_handler(command_tx.clone());

        let runtime = Rc::new(Self {
            application: application.clone(),
            hold_guard: RefCell::new(Some(hold_guard)),
            command_tx,
            config_store,
            model_store,
            download_manager: RefCell::new(ModelDownloadManager::default()),
            config: RefCell::new(config),
            daemon_client,
            daemon_status: Cell::new(daemon_status),
            shortcut_runtime_config,
            recording: RefCell::new(RecordingService::default()),
            settings_window: RefCell::new(None),
            tray: RefCell::new(tray),
            hotkey_backend: RefCell::new(hotkey_backend),
            dbus_handle: RefCell::new(Some(dbus_handle)),
            daemon_monitor: RefCell::new(Some(daemon_monitor)),
        });

        Self::attach_command_pump(&runtime, command_rx);
        runtime.log_startup_state();
        Ok(runtime)
    }

    fn attach_command_pump(runtime: &Rc<Self>, command_rx: mpsc::Receiver<AppCommand>) {
        let runtime = Rc::clone(runtime);
        gtk::glib::timeout_add_local(COMMAND_PUMP_INTERVAL, move || {
            for _ in 0..MAX_COMMANDS_PER_PUMP {
                let Ok(command) = command_rx.try_recv() else {
                    break;
                };
                runtime.handle_command(command);
            }

            gtk::glib::ControlFlow::Continue
        });
    }

    fn handle_command(&self, command: AppCommand) {
        match command {
            AppCommand::ShowSettings => self.show_settings(),
            AppCommand::ToggleRecording => self.toggle_recording(),
            AppCommand::StartRecording(shortcut_id) => self.start_recording(&shortcut_id),
            AppCommand::StopRecording(shortcut_id) => self.stop_recording(&shortcut_id),
            AppCommand::TranscriptionFinished {
                shortcut_id,
                result,
            } => self.finish_transcription(&shortcut_id, result),
            AppCommand::SaveConfig(config) => self.save_config(config),
            AppCommand::DownloadModel(model_id) => self.download_model(model_id),
            AppCommand::CancelModelDownload(model_id) => self.cancel_model_download(model_id),
            AppCommand::DeleteModel(model_id) => self.delete_model(&model_id),
            AppCommand::ModelDownloadProgress {
                download_id,
                model_id,
                downloaded,
                total,
            } => self.update_model_download_progress(download_id, model_id, downloaded, total),
            AppCommand::ModelDownloadVerifying {
                download_id,
                model_id,
                downloaded,
                total,
            } => self.update_model_download_verifying(download_id, model_id, downloaded, total),
            AppCommand::ModelDownloadFinished {
                download_id,
                model_id,
                outcome,
            } => self.finish_model_download(download_id, model_id, outcome),
            AppCommand::DaemonAppeared(status) => self.handle_daemon_appeared(status),
            AppCommand::DaemonVanished(status) => self.set_daemon_status(status),
            AppCommand::DaemonStatusChanged(status) => self.set_daemon_status(status),
            AppCommand::Quit => self.quit(),
        }
    }

    fn toggle_recording(&self) {
        let phase = self.recording.borrow().phase();
        match phase {
            RecordingPhase::Idle => self.start_recording(DEFAULT_SHORTCUT_ID),
            RecordingPhase::Recording => self.stop_recording(DEFAULT_SHORTCUT_ID),
            RecordingPhase::Processing => {
                info!("Recording toggle ignored while processing audio");
            }
        }
    }

    fn start_recording(&self, shortcut_id: &str) {
        if self.config.borrow().shortcut_by_id(shortcut_id).is_none() {
            warn!(shortcut_id, "Start recording ignored for unknown shortcut");
            return;
        }
        let phase = self.recording.borrow_mut().start_recording(shortcut_id);
        self.set_recording_phase(phase);
    }

    fn stop_recording(&self, shortcut_id: &str) {
        let (phase, job) = self.recording.borrow_mut().stop_recording(shortcut_id);
        self.set_recording_phase(phase);

        if let Some(job) = job {
            spawn_transcription_job(job, self.command_sender());
        }
    }

    fn finish_transcription(&self, shortcut_id: &str, result: Result<(), String>) {
        if result.is_ok() {
            self.apply_transcription_output(shortcut_id);
        }
        let phase = self
            .recording
            .borrow_mut()
            .finish_processing(shortcut_id, &result);
        self.set_recording_phase(phase);
    }

    fn apply_transcription_output(&self, shortcut_id: &str) {
        let config = self.config.borrow();
        let Some(shortcut) = config.shortcut_by_id(shortcut_id) else {
            warn!(
                shortcut_id,
                "No shortcut config found for transcription output"
            );
            return;
        };
        let model_id = config.resolved_model_id(shortcut);
        let language = config.resolved_language(shortcut);
        let output = config.resolved_output(shortcut);

        match output {
            ResolvedOutput::General(action) => info!(
                shortcut_id,
                model_id,
                language,
                output = action.label(),
                "Transcription output placeholder"
            ),
            ResolvedOutput::Clipboard => info!(
                shortcut_id,
                model_id, language, "Would copy recognized text to clipboard"
            ),
            ResolvedOutput::Script(path) => info!(
                shortcut_id,
                model_id,
                language,
                script = path,
                "Would run output script with recognized text argument"
            ),
        }
    }

    fn set_recording_phase(&self, phase: RecordingPhase) {
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.set_recording_phase(phase);
        }
    }

    fn show_settings(&self) {
        if self.settings_window.borrow().is_none() {
            let window = SettingsWindow::new(
                &self.application,
                &self.config.borrow(),
                self.model_row_states(),
                self.model_store.ready_model_ids(),
                self.daemon_status.get(),
                self.command_sender(),
            );
            self.settings_window.replace(Some(window));
        }

        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.update_config(&self.config.borrow());
            window.update_model_states(self.model_row_states(), self.model_store.ready_model_ids());
            window.update_daemon_status(self.daemon_status.get());
            window.present();
        }
    }

    fn save_config(&self, config: AppConfig) {
        if let Err(error) = self.save_config_inner(config) {
            warn!(?error, "failed to save settings config");
            if let Some(window) = self.settings_window.borrow().as_ref() {
                window.update_save_status(&format!("Failed to save settings: {error}"));
            }
        }
    }

    fn save_config_inner(&self, config: AppConfig) -> Result<()> {
        self.config_store.save(&config)?;
        let config = self.config_store.load_or_create_default()?;
        self.apply_config(config);
        Ok(())
    }

    fn apply_config(&self, config: AppConfig) {
        self.hotkey_backend.borrow_mut().take();
        let hotkey_backend = configure_hotkey_backend(self.command_sender(), &config);
        self.hotkey_backend.replace(hotkey_backend);
        let daemon_runtime_config = daemon_runtime_config_for(&config);
        if let Ok(mut runtime_config) = self.shortcut_runtime_config.lock() {
            *runtime_config = daemon_runtime_config.clone();
        }
        sync_shortcut_config_to_daemon(&self.daemon_client, &daemon_runtime_config);

        self.config.replace(config.clone());
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.update_config(&config);
            window.update_model_states(self.model_row_states(), self.model_store.ready_model_ids());
            window.update_save_status("Saved");
        }
    }

    fn download_model(&self, model_id: String) {
        if self.download_manager.borrow().is_active(&model_id) {
            info!(model_id, "model download already in progress");
            return;
        }
        if self.model_store.ready_model_ids().contains(&model_id) {
            info!(
                model_id,
                "model download ignored because model is already ready"
            );
            return;
        }
        let Some(download_id) = self.download_manager.borrow_mut().begin(&model_id) else {
            info!(model_id, "model download already in progress");
            return;
        };
        let handle =
            self.model_store
                .start_download(download_id, model_id.clone(), self.command_sender());
        self.download_manager
            .borrow_mut()
            .attach_handle(&model_id, download_id, handle);
        self.refresh_model_rows();
    }

    fn cancel_model_download(&self, model_id: String) {
        let Some(handle) = self.download_manager.borrow_mut().cancel(&model_id) else {
            info!(
                model_id,
                "model download cancel ignored because no download is active"
            );
            return;
        };
        handle.cancel();
        self.refresh_model_rows();
    }

    fn delete_model(&self, model_id: &str) {
        if self.download_manager.borrow().is_active(model_id) {
            warn!(model_id, "model delete ignored while download is active");
            return;
        }
        if let Err(error) = self
            .model_store
            .delete_model(model_id, &self.config.borrow())
        {
            warn!(?error, model_id, "failed to delete model");
        }
        self.refresh_model_inventory();
    }

    fn update_model_download_progress(
        &self,
        download_id: DownloadId,
        model_id: String,
        downloaded: u64,
        total: Option<u64>,
    ) {
        if !self
            .download_manager
            .borrow_mut()
            .progress(download_id, &model_id, downloaded, total)
        {
            debug!(
                model_id,
                download_id, downloaded, total, "ignoring stale model download progress"
            );
            return;
        }
        self.refresh_model_rows();
    }

    fn update_model_download_verifying(
        &self,
        download_id: DownloadId,
        model_id: String,
        downloaded: u64,
        total: Option<u64>,
    ) {
        if self
            .download_manager
            .borrow_mut()
            .verifying(download_id, &model_id, downloaded, total)
        {
            self.refresh_model_rows();
        } else {
            debug!(
                model_id,
                download_id, "ignoring stale model download verifying event"
            );
        }
    }

    fn finish_model_download(
        &self,
        download_id: DownloadId,
        model_id: String,
        outcome: ModelDownloadOutcome,
    ) {
        let effect = self
            .download_manager
            .borrow_mut()
            .finish(download_id, &model_id, outcome);
        if effect == FinishEffect::Stale {
            info!(
                model_id,
                download_id, "ignoring stale model download outcome"
            );
            return;
        }

        match effect {
            FinishEffect::Completed => {
                match self.model_store.mark_ready(&model_id) {
                    Ok(ready_model_ids) => info!(
                        model_id,
                        ready_model_count = ready_model_ids.len(),
                        "model download completed"
                    ),
                    Err(error) => {
                        warn!(?error, model_id, "failed to update model inventory");
                        self.download_manager
                            .borrow_mut()
                            .set_error(&model_id, format!("{error:#}"));
                    }
                }
                self.refresh_model_inventory();
            }
            FinishEffect::Canceled => {
                info!(model_id, "model download canceled");
                self.refresh_model_rows();
            }
            FinishEffect::Failed(error) => {
                warn!(model_id, error, "model download failed");
                self.refresh_model_rows();
            }
            FinishEffect::Stale => {}
        }
    }

    fn refresh_model_rows(&self) {
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.update_model_states(self.model_row_states(), self.model_store.ready_model_ids());
        }
    }

    fn refresh_model_inventory(&self) {
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.update_model_inventory(
                self.model_row_states(),
                self.model_store.ready_model_ids(),
            );
        }
    }

    fn model_row_states(&self) -> Vec<ModelRowState> {
        let config = self.config.borrow();
        let download_manager = self.download_manager.borrow();
        self.model_store
            .row_states(&config, download_manager.statuses())
    }

    fn handle_daemon_appeared(&self, status: DaemonStatus) {
        self.set_daemon_status(status);
        self.sync_current_shortcut_config_to_daemon();
        self.refresh_daemon_status();
    }

    fn sync_current_shortcut_config_to_daemon(&self) {
        let daemon_runtime_config = daemon_runtime_config_for(&self.config.borrow());
        if let Ok(mut runtime_config) = self.shortcut_runtime_config.lock() {
            *runtime_config = daemon_runtime_config.clone();
        }
        sync_shortcut_config_to_daemon(&self.daemon_client, &daemon_runtime_config);
    }

    fn refresh_daemon_status(&self) {
        let status = self.daemon_client.status();
        self.set_daemon_status(status);
    }

    fn set_daemon_status(&self, status: DaemonStatus) {
        self.daemon_status.set(status);
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.update_daemon_status(status);
        }
    }

    fn quit(&self) {
        info!("Quitting MyApp");
        self.hotkey_backend.borrow_mut().take();
        self.daemon_monitor.borrow_mut().take();
        self.dbus_handle.borrow_mut().take();
        self.tray.borrow_mut().take();
        self.hold_guard.borrow_mut().take();
        self.application.quit();
    }

    fn log_startup_state(&self) {
        let config = self.config.borrow();
        info!(
            config_path = %self.config_store.path().display(),
            shortcut = %config.default_shortcut().accelerator,
            shortcut_count = config.shortcuts.len(),
            mode = %config.general.mode.as_str(),
            backend = %backend_name_for_config(&config),
            daemon_status = %self.daemon_status.get().display_label(),
            model_dir = %self.model_store.root().display(),
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

fn daemon_runtime_config_for(config: &AppConfig) -> ShortcutRuntimeConfig {
    ShortcutRuntimeConfig::for_daemon(config, resolve_backend_kind(config.general.hotkey_backend))
}

fn sync_shortcut_config_to_daemon(
    daemon_client: &DaemonClient,
    runtime_config: &ShortcutRuntimeConfig,
) {
    if let Err(error) = daemon_client.update_shortcut_config(runtime_config) {
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
