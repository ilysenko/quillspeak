use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use gtk::gio;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{APP_ID, AppConfig, DEFAULT_SHORTCUT_ID, ShortcutTrigger};
use tracing::{debug, error, info, warn};

use crate::audio::{AudioInputDevice, list_input_devices};
use crate::command::{AppCommand, DownloadId, ModelDownloadOutcome};
use crate::config_store::ConfigStore;
use crate::hotkey::{
    HotkeyBackendHandle, configure_hotkey_backend, configured_backend_name, effective_backend_name,
};
use crate::models::{FinishEffect, ModelDownloadManager, ModelRowState, ModelStore};
use crate::output::{
    ClipboardCopyOutcome, ClipboardCopySource, ClipboardPasteOutcome, OutputScriptResult,
    OutputService,
};
use crate::recording::{RecordingPhase, RecordingPipeline, RecordingService};
use crate::settings::SettingsWindow;
use crate::signal_trigger::{SignalTriggerService, resolve_signal_number};
use crate::transcription::{
    CompiledWhisperBackends, TranscriptionRequest, TranscriptionResult, TranscriptionService,
    TranscriptionStatus, build_transcription_plan,
};
use crate::tray::Tray;

const COMMAND_PUMP_INTERVAL: Duration = Duration::from_millis(50);
const MAX_COMMANDS_PER_PUMP: usize = 128;
const TRAY_IDLE_RECONCILE_DELAY: Duration = Duration::from_millis(200);

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
    audio_input_devices: RefCell<Vec<AudioInputDevice>>,
    recording_pipeline: RefCell<Option<RecordingPipeline>>,
    transcription_service: TranscriptionService,
    output_service: RefCell<Option<OutputService>>,
    config: RefCell<AppConfig>,
    recording: RefCell<RecordingService>,
    recording_phase: Cell<RecordingPhase>,
    next_recording_id: Cell<u64>,
    settings_window: RefCell<Option<SettingsWindow>>,
    tray: RefCell<Option<Tray>>,
    hotkey_backend: RefCell<Option<HotkeyBackendHandle>>,
    signal_trigger_service: RefCell<Option<SignalTriggerService>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LinuxSignalAction {
    Toggle(String),
    Start(String),
    Stop(String),
}

impl AppRuntime {
    fn new(application: &adw::Application) -> Result<Rc<Self>> {
        let hold_guard = application.hold();
        let (command_tx, command_rx) = mpsc::channel();

        let config_store = ConfigStore::new()?;
        let model_store = ModelStore::new()?;
        let audio_input_devices = list_input_devices();
        let config = config_store.load_or_create_default().with_context(|| {
            format!(
                "failed to load config from {}",
                config_store.path().display()
            )
        })?;

        let transcription_service =
            TranscriptionService::spawn(command_tx.clone(), config.general.keep_model_loaded)?;
        let recording_pipeline = RecordingPipeline::spawn(command_tx.clone())?;
        let output_service = OutputService::spawn(command_tx.clone())?;

        let hotkey_backend = configure_hotkey_backend(command_tx.clone(), &config);

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

        let signal_trigger_service = SignalTriggerService::spawn(command_tx.clone(), &config)?;
        install_ctrl_c_handler(command_tx.clone());

        let runtime = Rc::new(Self {
            application: application.clone(),
            hold_guard: RefCell::new(Some(hold_guard)),
            command_tx,
            config_store,
            model_store,
            download_manager: RefCell::new(ModelDownloadManager::default()),
            audio_input_devices: RefCell::new(audio_input_devices),
            recording_pipeline: RefCell::new(Some(recording_pipeline)),
            transcription_service,
            output_service: RefCell::new(Some(output_service)),
            config: RefCell::new(config),
            recording: RefCell::new(RecordingService::default()),
            recording_phase: Cell::new(RecordingPhase::Idle),
            next_recording_id: Cell::new(1),
            settings_window: RefCell::new(None),
            tray: RefCell::new(tray),
            hotkey_backend: RefCell::new(hotkey_backend),
            signal_trigger_service: RefCell::new(Some(signal_trigger_service)),
        });

        Self::attach_command_pump(&runtime, command_rx);
        runtime.prepare_audio_capture();
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
            AppCommand::LinuxSignalReceived(signal) => self.handle_linux_signal(signal),
            AppCommand::StartRecording(shortcut_id) => self.start_recording(&shortcut_id),
            AppCommand::StopRecording(shortcut_id) => self.stop_recording(&shortcut_id),
            AppCommand::AudioCaptureStarted {
                recording_id,
                shortcut_id,
                input_label,
                startup_latency_ms,
                first_callback_latency_ms,
            } => self.audio_capture_started(
                recording_id,
                &shortcut_id,
                &input_label,
                startup_latency_ms,
                first_callback_latency_ms,
            ),
            AppCommand::AudioCaptureStartFailed {
                recording_id,
                shortcut_id,
                error,
            } => self.audio_capture_start_failed(recording_id, &shortcut_id, &error),
            AppCommand::AudioCaptureStopped {
                recording_id,
                shortcut_id,
                result,
            } => self.audio_capture_stopped(recording_id, &shortcut_id, result),
            AppCommand::TranscriptionFinished {
                recording_id,
                shortcut_id,
                result,
            } => self.finish_transcription(recording_id, &shortcut_id, result),
            AppCommand::RefreshTrayRecordingPhase => self.force_refresh_recording_phase(),
            AppCommand::OutputScriptFinished {
                shortcut_id,
                result,
            } => self.finish_output_script(&shortcut_id, result),
            AppCommand::ClipboardCopyFinished { source, result } => {
                self.finish_clipboard_copy(source, result)
            }
            AppCommand::ClipboardPasteFinished { source, result } => {
                self.finish_clipboard_paste(source, result)
            }
            AppCommand::AudioInputDevicesRefreshed(devices) => {
                self.update_audio_input_devices(devices)
            }
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
            AppCommand::Quit => self.quit(),
        }
    }

    fn toggle_recording(&self) {
        self.toggle_recording_for(DEFAULT_SHORTCUT_ID);
    }

    fn toggle_recording_for(&self, shortcut_id: &str) {
        let phase = self.recording.borrow().phase();
        match phase {
            RecordingPhase::Idle => self.start_recording(shortcut_id),
            RecordingPhase::Arming | RecordingPhase::Recording
                if self.recording.borrow().active_shortcut_id() == Some(shortcut_id) =>
            {
                self.stop_recording(shortcut_id);
            }
            RecordingPhase::Arming | RecordingPhase::Recording => {
                info!(
                    requested_shortcut_id = shortcut_id,
                    active_shortcut_id = self
                        .recording
                        .borrow()
                        .active_shortcut_id()
                        .unwrap_or("unknown"),
                    "Recording toggle ignored for inactive shortcut"
                );
            }
            RecordingPhase::Processing => {
                info!("Recording toggle ignored while processing audio");
            }
        }
    }

    fn handle_linux_signal(&self, signal: i32) {
        let Some(action) = self.linux_signal_action(signal) else {
            debug!(
                signal,
                "Linux signal trigger did not match any enabled shortcut"
            );
            return;
        };

        match action {
            LinuxSignalAction::Toggle(shortcut_id) => self.toggle_recording_for(&shortcut_id),
            LinuxSignalAction::Start(shortcut_id) => self.start_recording(&shortcut_id),
            LinuxSignalAction::Stop(shortcut_id) => self.stop_recording(&shortcut_id),
        }
    }

    fn linux_signal_action(&self, signal: i32) -> Option<LinuxSignalAction> {
        linux_signal_action_for_config(&self.config.borrow(), signal)
    }

    fn start_recording(&self, shortcut_id: &str) {
        if self.recording.borrow().phase() != RecordingPhase::Idle {
            let phase = self.recording.borrow_mut().start_recording(0, shortcut_id);
            self.set_recording_phase(phase);
            return;
        }

        let recording_id = self.allocate_recording_id();
        let plan = match build_transcription_plan(
            &self.config.borrow(),
            &self.model_store.ready_model_ids(),
            |entry| self.model_store.model_path(entry),
            recording_id,
            shortcut_id,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                warn!(?error, shortcut_id, "Start recording ignored");
                return;
            }
        };
        let input_label = plan.input.display_label().to_string();
        let phase = self
            .recording
            .borrow_mut()
            .start_recording(recording_id, shortcut_id);
        self.set_recording_phase(phase);

        let start_result = self
            .recording_pipeline
            .borrow()
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("audio capture pipeline is shut down"))
            .and_then(|pipeline| pipeline.start(plan));

        if let Err(error) = start_result {
            warn!(
                ?error,
                recording_id,
                shortcut_id,
                input = input_label,
                "failed to start audio capture"
            );
            let phase = self
                .recording
                .borrow_mut()
                .start_failed(recording_id, shortcut_id);
            self.set_recording_phase(phase);
            return;
        }
        info!(
            recording_id,
            shortcut_id,
            input = input_label,
            "audio capture start requested"
        );
    }

    fn stop_recording(&self, shortcut_id: &str) {
        let (phase, recording_id) = self.recording.borrow_mut().stop_recording(shortcut_id);
        self.set_recording_phase(phase);

        let stop_result = if let Some(recording_id) = recording_id {
            self.recording_pipeline
                .borrow()
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("audio capture pipeline is shut down"))
                .and_then(|pipeline| pipeline.stop(recording_id, shortcut_id))
        } else {
            Ok(())
        };

        if let Some(recording_id) = recording_id
            && let Err(error) = stop_result
        {
            warn!(
                ?error,
                recording_id, shortcut_id, "failed to stop audio capture"
            );
            let _ = self
                .command_sender()
                .send(AppCommand::TranscriptionFinished {
                    recording_id,
                    shortcut_id: shortcut_id.to_string(),
                    result: Err(format!("{error:#}")),
                });
        }
    }

    fn audio_capture_started(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        input_label: &str,
        startup_latency_ms: u128,
        first_callback_latency_ms: Option<u128>,
    ) {
        info!(
            recording_id,
            shortcut_id,
            input = input_label,
            startup_latency_ms,
            first_callback_latency_ms,
            "audio capture started"
        );
        let phase = self
            .recording
            .borrow_mut()
            .capture_started(recording_id, shortcut_id);
        self.set_recording_phase(phase);
    }

    fn audio_capture_start_failed(&self, recording_id: u64, shortcut_id: &str, error: &str) {
        warn!(
            recording_id,
            shortcut_id, error, "audio capture failed to start"
        );
        let phase = self
            .recording
            .borrow_mut()
            .start_failed(recording_id, shortcut_id);
        self.set_recording_phase(phase);
    }

    fn audio_capture_stopped(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        result: std::result::Result<Box<TranscriptionRequest>, String>,
    ) {
        if !self
            .recording
            .borrow()
            .is_processing(recording_id, shortcut_id)
        {
            info!(
                recording_id,
                shortcut_id, "ignoring stale audio capture result"
            );
            return;
        }

        let result = result
            .map_err(anyhow::Error::msg)
            .and_then(|request| self.transcription_service.submit(request));

        if let Err(error) = result {
            warn!(
                ?error,
                recording_id, shortcut_id, "failed to submit transcription job"
            );
            let _ = self
                .command_sender()
                .send(AppCommand::TranscriptionFinished {
                    recording_id,
                    shortcut_id: shortcut_id.to_string(),
                    result: Err(format!("{error:#}")),
                });
        }
    }

    fn finish_transcription(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        result: std::result::Result<Box<TranscriptionResult>, String>,
    ) {
        let (phase, accepted) =
            self.recording
                .borrow_mut()
                .finish_processing(recording_id, shortcut_id, &result);
        if !accepted {
            info!(
                recording_id,
                shortcut_id, "ignoring stale transcription result"
            );
            self.set_recording_phase(phase);
            return;
        }

        self.set_recording_phase(phase);

        if let Ok(result) = &result {
            log_recognized_text(shortcut_id, result);
            self.apply_transcription_output(shortcut_id, result);
        }
    }

    fn apply_transcription_output(&self, shortcut_id: &str, result: &TranscriptionResult) {
        if let Some(output_service) = self.output_service.borrow().as_ref() {
            output_service.apply(shortcut_id, result);
        }
    }

    fn finish_output_script(
        &self,
        shortcut_id: &str,
        result: std::result::Result<OutputScriptResult, String>,
    ) {
        match result {
            Ok(result) => {
                info!(
                    shortcut_id,
                    script = %result.script_path,
                    output_text_chars = result
                        .output_text
                        .as_ref()
                        .map(|text| text.chars().count())
                        .unwrap_or(0),
                    output_text_bytes = result
                        .output_text
                        .as_ref()
                        .map(|text| text.len())
                        .unwrap_or(0),
                    output_text_delivered = result.output_text.is_some(),
                    copy_to_clipboard = result.output.copy_to_clipboard,
                    paste_from_clipboard = result.output.paste_from_clipboard,
                    "output script finished"
                );
                if let Some(output_text) = result.output_text {
                    if let Some(output_service) = self.output_service.borrow().as_ref() {
                        output_service.copy_final_text_if_requested(
                            ClipboardCopySource::ScriptStdout {
                                shortcut_id: shortcut_id.to_string(),
                                script_path: result.script_path.clone(),
                            },
                            &result.output,
                            &output_text,
                        );
                    } else {
                        warn!(
                            shortcut_id,
                            script = %result.script_path,
                            "output worker is not running"
                        );
                    }
                }
            }
            Err(error) => warn!(shortcut_id, error, "output script failed"),
        }
    }

    fn finish_clipboard_copy(
        &self,
        source: ClipboardCopySource,
        result: std::result::Result<ClipboardCopyOutcome, String>,
    ) {
        let shortcut_id = source.shortcut_id();
        let copy_source = source.kind();
        let script_path = source.script_path().unwrap_or("");
        match result {
            Ok(result) => {
                info!(
                    shortcut_id,
                    source = copy_source,
                    script = script_path,
                    clipboard_backend = result.backend.as_str(),
                    text_chars = result.text_chars,
                    text_bytes = result.text_bytes,
                    "Copied text to clipboard"
                );
            }
            Err(error) => warn!(
                shortcut_id,
                source = copy_source,
                script = script_path,
                error,
                "clipboard copy failed"
            ),
        }
    }

    fn finish_clipboard_paste(
        &self,
        source: ClipboardCopySource,
        result: std::result::Result<ClipboardPasteOutcome, String>,
    ) {
        let shortcut_id = source.shortcut_id();
        let copy_source = source.kind();
        let script_path = source.script_path().unwrap_or("");
        match result {
            Ok(result) => info!(
                shortcut_id,
                source = copy_source,
                script = script_path,
                clipboard_backend = result.backend.as_str(),
                paste_shortcut = result.shortcut.label(),
                "Pasted text from clipboard"
            ),
            Err(error) => warn!(
                shortcut_id,
                source = copy_source,
                script = script_path,
                error,
                "clipboard paste failed"
            ),
        }
    }

    fn set_recording_phase(&self, phase: RecordingPhase) {
        let previous = self.recording_phase.get();
        if previous == phase {
            return;
        }

        self.recording_phase.set(phase);
        debug!(?previous, ?phase, "recording phase changed");
        self.update_tray_recording_phase(phase, false);

        if previous == RecordingPhase::Processing && phase == RecordingPhase::Idle {
            self.schedule_tray_idle_reconcile();
        }
    }

    fn update_tray_recording_phase(&self, phase: RecordingPhase, forced: bool) {
        if let Some(tray) = self.tray.borrow().as_ref() {
            let updated = if forced {
                tray.force_refresh_recording_phase(phase)
            } else {
                tray.set_recording_phase(phase)
            };

            if !updated {
                warn!(
                    ?phase,
                    forced, "failed to update StatusNotifierItem recording phase"
                );
            }
        } else {
            debug!(?phase, forced, "recording phase changed without tray");
        }
    }

    fn schedule_tray_idle_reconcile(&self) {
        let command_tx = self.command_sender();
        let _source_id = gtk::glib::timeout_add_local(TRAY_IDLE_RECONCILE_DELAY, move || {
            let _ = command_tx.send(AppCommand::RefreshTrayRecordingPhase);
            gtk::glib::ControlFlow::Break
        });
    }

    fn force_refresh_recording_phase(&self) {
        let phase = self.recording_phase.get();
        debug!(?phase, "forcing StatusNotifierItem recording phase refresh");
        self.update_tray_recording_phase(phase, true);
    }

    fn show_settings(&self) {
        if self.settings_window.borrow().is_none() {
            let window = SettingsWindow::new(
                &self.application,
                &self.config.borrow(),
                self.audio_input_devices.borrow().clone(),
                self.model_row_states(),
                self.model_store.ready_model_ids(),
                self.command_sender(),
            );
            self.settings_window.replace(Some(window));
        }

        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.refresh_live_state(self.model_row_states(), self.model_store.ready_model_ids());
            window.present();
        }
        self.request_audio_input_refresh();
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
        self.signal_trigger_service.borrow_mut().take();
        match SignalTriggerService::spawn(self.command_sender(), &config) {
            Ok(service) => {
                self.signal_trigger_service.replace(Some(service));
            }
            Err(error) => {
                warn!(?error, "failed to reconfigure Linux signal triggers");
                self.signal_trigger_service.replace(None);
            }
        }
        if let Err(error) = self
            .transcription_service
            .set_keep_model_loaded(config.general.keep_model_loaded)
        {
            warn!(?error, "failed to update whisper model cache policy");
        }
        if let Err(error) = self
            .transcription_service
            .clear_cached_context("settings config changed")
        {
            warn!(
                ?error,
                "failed to clear whisper model cache after settings change"
            );
        }

        self.config.replace(config.clone());
        self.prepare_audio_capture();
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
        let model_path = self.model_store.model_path_for_id(model_id);
        match self
            .model_store
            .delete_model(model_id, &self.config.borrow())
        {
            Ok(()) => {
                if let Some(model_path) = model_path
                    && let Err(error) = self
                        .transcription_service
                        .clear_cached_model_path(model_path)
                {
                    warn!(?error, model_id, "failed to clear cached deleted model");
                }
            }
            Err(error) => {
                warn!(?error, model_id, "failed to delete model");
            }
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

    fn request_audio_input_refresh(&self) {
        let command_tx = self.command_sender();
        if let Err(error) = thread::Builder::new()
            .name("myapp-audio-devices".to_string())
            .spawn(move || {
                let devices = list_input_devices();
                let _ = command_tx.send(AppCommand::AudioInputDevicesRefreshed(devices));
            })
        {
            warn!(?error, "failed to spawn audio input device refresh worker");
        }
    }

    fn prepare_audio_capture(&self) {
        let input = self.config.borrow().general.default_input.clone();
        let prepare_result = self
            .recording_pipeline
            .borrow()
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("audio capture pipeline is shut down"))
            .and_then(|pipeline| pipeline.prepare_input(input.clone()));

        if let Err(error) = prepare_result {
            warn!(
                ?error,
                input = %input.display_label(),
                "failed to request audio capture prepare"
            );
        }
    }

    fn update_audio_input_devices(&self, devices: Vec<AudioInputDevice>) {
        self.audio_input_devices.replace(devices.clone());
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.update_audio_input_devices(devices);
        }
    }

    fn model_row_states(&self) -> Vec<ModelRowState> {
        let config = self.config.borrow();
        let download_manager = self.download_manager.borrow();
        self.model_store
            .row_states(&config, download_manager.statuses())
    }

    fn quit(&self) {
        info!("Quitting MyApp");
        let download_handles = self.download_manager.borrow_mut().cancel_all();
        if !download_handles.is_empty() {
            info!(
                download_count = download_handles.len(),
                "canceling active model downloads on quit"
            );
        }
        for handle in download_handles {
            handle.cancel();
        }
        if let Some(pipeline) = self.recording_pipeline.borrow_mut().take() {
            pipeline.shutdown();
        }
        if let Some(output_service) = self.output_service.borrow_mut().take() {
            output_service.shutdown();
        }
        if let Some(signal_trigger_service) = self.signal_trigger_service.borrow_mut().take() {
            signal_trigger_service.shutdown();
        }
        self.hotkey_backend.borrow_mut().take();
        self.tray.borrow_mut().take();
        self.hold_guard.borrow_mut().take();
        self.application.quit();
    }

    fn log_startup_state(&self) {
        let config = self.config.borrow();
        let whisper_backends = CompiledWhisperBackends::current();
        let default_trigger = trigger_summary(&config.default_shortcut().trigger);
        info!(
            config_path = %self.config_store.path().display(),
            shortcut_trigger = %default_trigger,
            shortcut_count = config.shortcuts.len(),
            mode = %config.general.mode.as_str(),
            configured_hotkey_backend = %configured_backend_name(&config),
            effective_hotkey_backend = %effective_backend_name(&config),
            default_input = %config.general.default_input.display_label(),
            keep_model_loaded = config.general.keep_model_loaded,
            whisper_compute = %config.general.compute_backend.as_str(),
            compiled_whisper_backends = %whisper_backends.display_label(),
            whisper_gpu_compiled = whisper_backends.has_gpu(),
            model_dir = %self.model_store.root().display(),
            "MyApp started in foreground development mode"
        );
    }

    fn command_sender(&self) -> mpsc::Sender<AppCommand> {
        self.command_tx.clone()
    }

    fn allocate_recording_id(&self) -> u64 {
        let recording_id = self.next_recording_id.get();
        self.next_recording_id
            .set(recording_id.checked_add(1).unwrap_or(1));
        recording_id
    }
}

fn trigger_summary(trigger: &ShortcutTrigger) -> String {
    match trigger {
        ShortcutTrigger::Keyboard { accelerator } => format!("keyboard:{accelerator}"),
        ShortcutTrigger::LinuxSignal {
            start_signal,
            stop_signal,
        } if start_signal == stop_signal => format!("signal:{}:toggle", start_signal.as_str()),
        ShortcutTrigger::LinuxSignal {
            start_signal,
            stop_signal,
        } => format!("signal:{}->{}", start_signal.as_str(), stop_signal.as_str()),
    }
}

fn linux_signal_action_for_config(config: &AppConfig, signal: i32) -> Option<LinuxSignalAction> {
    for shortcut in &config.shortcuts {
        if !shortcut.enabled {
            continue;
        }

        let ShortcutTrigger::LinuxSignal {
            start_signal,
            stop_signal,
        } = &shortcut.trigger
        else {
            continue;
        };

        let (Ok(start_signal_number), Ok(stop_signal_number)) = (
            resolve_signal_number(start_signal.as_str()),
            resolve_signal_number(stop_signal.as_str()),
        ) else {
            continue;
        };

        if start_signal_number == stop_signal_number && signal == start_signal_number {
            return Some(LinuxSignalAction::Toggle(shortcut.id.clone()));
        }
        if signal == start_signal_number {
            return Some(LinuxSignalAction::Start(shortcut.id.clone()));
        }
        if signal == stop_signal_number {
            return Some(LinuxSignalAction::Stop(shortcut.id.clone()));
        }
    }

    None
}

fn log_recognized_text(shortcut_id: &str, result: &TranscriptionResult) {
    if !matches!(result.status, TranscriptionStatus::Completed) {
        return;
    }

    let text = result.text.trim();
    if text.is_empty() {
        return;
    }

    info!(
        shortcut_id,
        model_id = %result.debug.model_id,
        language = %result.debug.language,
        text,
        "recognized text"
    );
}

fn install_ctrl_c_handler(command_tx: mpsc::Sender<AppCommand>) {
    if let Err(error) = ctrlc::set_handler(move || {
        let _ = command_tx.send(AppCommand::Quit);
    }) {
        warn!(?error, "failed to install Ctrl-C handler");
    }
}

#[cfg(test)]
mod tests {
    use shared::{LinuxSignal, ShortcutTrigger};
    use signal_hook::consts::signal::{SIGUSR1, SIGUSR2};

    use super::*;

    #[test]
    fn same_start_stop_linux_signal_toggles_shortcut() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::default_linux_signal();

        assert_eq!(
            linux_signal_action_for_config(&config, SIGUSR2),
            Some(LinuxSignalAction::Toggle(DEFAULT_SHORTCUT_ID.to_string()))
        );
    }

    #[test]
    fn distinct_linux_signals_start_and_stop_shortcut() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::LinuxSignal {
            start_signal: LinuxSignal::sigusr1(),
            stop_signal: LinuxSignal::sigusr2(),
        };

        assert_eq!(
            linux_signal_action_for_config(&config, SIGUSR1),
            Some(LinuxSignalAction::Start(DEFAULT_SHORTCUT_ID.to_string()))
        );
        assert_eq!(
            linux_signal_action_for_config(&config, SIGUSR2),
            Some(LinuxSignalAction::Stop(DEFAULT_SHORTCUT_ID.to_string()))
        );
    }

    #[test]
    fn keyboard_shortcuts_ignore_linux_signals() {
        let config = AppConfig::default();

        assert_eq!(linux_signal_action_for_config(&config, SIGUSR2), None);
    }
}
