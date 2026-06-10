use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use gtk::gio;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{APP_ID, AppConfig, DEFAULT_SHORTCUT_ID, ShortcutTrigger};
use tracing::{debug, error, info, warn};

use crate::audio::{AudioInputDevice, list_input_devices};
use crate::beep::BeepService;
use crate::command::{AppCommand, DownloadId, ModelDownloadOutcome};
use crate::config_store::ConfigStore;
use crate::external_trigger::{
    ExternalTriggerAction, ExternalTriggerRequest, ExternalTriggerResponse, ExternalTriggerService,
    resolve_shortcut_selector,
};
use crate::history::{HistoryEntry, HistorySource, HistoryStore, unix_time_ms_now};
use crate::hotkey::{
    HotkeyBackendHandle, configure_hotkey_backend, configured_backend_name, effective_backend_name,
    shortcut_trigger_capabilities,
};
use crate::models::{FinishEffect, ModelDownloadManager, ModelRowState, ModelStore};
use crate::output::{
    ClipboardCopyOutcome, ClipboardCopySource, ClipboardPasteOutcome, OutputCompletion,
    OutputDelivery, OutputScriptResult, OutputService,
};
use crate::recording::{RecordingPhase, RecordingPipeline, RecordingService};
use crate::settings::{SettingsWindow, SettingsWindowInit};
use crate::signal_trigger::{
    LinuxSignalAction, SignalTriggerService, linux_signal_action_for_recording_state,
    linux_signal_match_for_config, signal_name,
};
use crate::system_audio::SpeakerMuteService;
use crate::transcription::{
    CompiledWhisperBackends, TranscriptionPlan, TranscriptionRequest, TranscriptionResult,
    TranscriptionService, TranscriptionStatus, WhisperRuntimeStatus, build_transcription_plan,
};
use crate::tray::Tray;

const COMMAND_PUMP_INTERVAL: Duration = Duration::from_millis(50);
const MAX_COMMANDS_PER_PUMP: usize = 128;
const TRAY_IDLE_RECONCILE_DELAY: Duration = Duration::from_millis(200);
const MAX_RECORDING_DURATION: Duration = Duration::from_secs(60);

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
    history_store: HistoryStore,
    download_manager: RefCell<ModelDownloadManager>,
    audio_input_devices: RefCell<Vec<AudioInputDevice>>,
    recording_pipeline: RefCell<Option<RecordingPipeline>>,
    speaker_mute_service: RefCell<Option<SpeakerMuteService>>,
    beep_service: RefCell<Option<BeepService>>,
    transcription_service: RefCell<Option<TranscriptionService>>,
    output_service: RefCell<Option<OutputService>>,
    config: RefCell<AppConfig>,
    history_entries: RefCell<Vec<HistoryEntry>>,
    whisper_runtime_status: RefCell<WhisperRuntimeStatus>,
    recording: RefCell<RecordingService>,
    recording_phase: Cell<RecordingPhase>,
    pending_recording_start: RefCell<Option<PendingRecordingStart>>,
    pending_stop_cues: RefCell<HashSet<(u64, String)>>,
    pending_output: RefCell<Option<PendingOutputDelivery>>,
    next_recording_id: Cell<u64>,
    is_quitting: Cell<bool>,
    settings_window: RefCell<Option<SettingsWindow>>,
    tray: RefCell<Option<Tray>>,
    hotkey_backend: RefCell<Option<HotkeyBackendHandle>>,
    signal_trigger_service: RefCell<Option<SignalTriggerService>>,
    external_trigger_service: RefCell<Option<ExternalTriggerService>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingOutputDelivery {
    recording_id: u64,
    shortcut_id: String,
    completion: OutputCompletion,
}

#[derive(Debug, Clone, PartialEq)]
struct PendingRecordingStart {
    recording_id: u64,
    shortcut_id: String,
    plan: TranscriptionPlan,
}

impl PendingOutputDelivery {
    fn matches(&self, recording_id: u64, shortcut_id: &str, completion: OutputCompletion) -> bool {
        self.recording_id == recording_id
            && self.shortcut_id == shortcut_id
            && self.completion == completion
    }

    fn matches_recording(&self, recording_id: u64, shortcut_id: &str) -> bool {
        self.recording_id == recording_id && self.shortcut_id == shortcut_id
    }
}

struct ShutdownServices {
    recording_pipeline: Option<RecordingPipeline>,
    speaker_mute_service: Option<SpeakerMuteService>,
    beep_service: Option<BeepService>,
    output_service: Option<OutputService>,
    signal_trigger_service: Option<SignalTriggerService>,
    external_trigger_service: Option<ExternalTriggerService>,
    transcription_service: Option<TranscriptionService>,
    hotkey_backend: Option<HotkeyBackendHandle>,
}

impl ShutdownServices {
    fn shutdown(self) {
        let Self {
            recording_pipeline,
            speaker_mute_service,
            beep_service,
            output_service,
            signal_trigger_service,
            external_trigger_service,
            transcription_service,
            hotkey_backend,
        } = self;

        drop(hotkey_backend);
        if let Some(mut external_trigger_service) = external_trigger_service {
            external_trigger_service.shutdown();
        }
        if let Some(mut signal_trigger_service) = signal_trigger_service {
            signal_trigger_service.shutdown();
        }
        if let Some(pipeline) = recording_pipeline {
            pipeline.shutdown();
        }
        if let Some(speaker_mute_service) = speaker_mute_service {
            speaker_mute_service.shutdown();
        }
        if let Some(beep_service) = beep_service {
            beep_service.shutdown();
        }
        if let Some(output_service) = output_service {
            output_service.shutdown();
        }
        if let Some(transcription_service) = transcription_service {
            transcription_service.shutdown();
        }
    }
}

impl AppRuntime {
    fn new(application: &adw::Application) -> Result<Rc<Self>> {
        let hold_guard = application.hold();
        let (command_tx, command_rx) = mpsc::channel();

        let config_store = ConfigStore::new()?;
        let model_store = ModelStore::new()?;
        let history_store = HistoryStore::new()?;
        let history_entries = match history_store.load() {
            Ok(entries) => entries,
            Err(error) => {
                warn!(?error, history_path = %history_store.path().display(), "failed to load history");
                Vec::new()
            }
        };
        let audio_input_devices = list_input_devices();
        let config = config_store.load_or_create_default().with_context(|| {
            format!(
                "failed to load config from {}",
                config_store.path().display()
            )
        })?;

        let transcription_service =
            TranscriptionService::spawn(command_tx.clone(), config.general.keep_model_loaded)?;
        let whisper_runtime_status = WhisperRuntimeStatus::initial(config.general.compute_backend);
        let recording_pipeline = RecordingPipeline::spawn(command_tx.clone())?;
        let speaker_mute_service = SpeakerMuteService::spawn(command_tx.clone())?;
        let beep_service = BeepService::spawn(command_tx.clone())?;
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
        let external_trigger_service = match ExternalTriggerService::spawn(command_tx.clone()) {
            Ok(service) => Some(service),
            Err(error) => {
                warn!(
                    ?error,
                    "failed to start external trigger command socket; myapp trigger commands are disabled"
                );
                None
            }
        };
        install_ctrl_c_handler(command_tx.clone());

        let runtime = Rc::new(Self {
            application: application.clone(),
            hold_guard: RefCell::new(Some(hold_guard)),
            command_tx,
            config_store,
            model_store,
            history_store,
            download_manager: RefCell::new(ModelDownloadManager::default()),
            audio_input_devices: RefCell::new(audio_input_devices),
            recording_pipeline: RefCell::new(Some(recording_pipeline)),
            speaker_mute_service: RefCell::new(Some(speaker_mute_service)),
            beep_service: RefCell::new(Some(beep_service)),
            transcription_service: RefCell::new(Some(transcription_service)),
            output_service: RefCell::new(Some(output_service)),
            whisper_runtime_status: RefCell::new(whisper_runtime_status),
            config: RefCell::new(config),
            history_entries: RefCell::new(history_entries),
            recording: RefCell::new(RecordingService::default()),
            recording_phase: Cell::new(RecordingPhase::Idle),
            pending_recording_start: RefCell::new(None),
            pending_stop_cues: RefCell::new(HashSet::new()),
            pending_output: RefCell::new(None),
            next_recording_id: Cell::new(1),
            is_quitting: Cell::new(false),
            settings_window: RefCell::new(None),
            tray: RefCell::new(tray),
            hotkey_backend: RefCell::new(hotkey_backend),
            signal_trigger_service: RefCell::new(Some(signal_trigger_service)),
            external_trigger_service: RefCell::new(external_trigger_service),
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
        if self.is_quitting.get()
            && !matches!(command, AppCommand::ShutdownComplete | AppCommand::Quit)
        {
            if let Some(command) = command.reject_pending_reply("MyApp is quitting") {
                debug!(?command, "ignoring command while app is quitting");
            }
            return;
        }

        match command {
            AppCommand::ShowSettings => self.show_settings(),
            AppCommand::ToggleRecording => self.toggle_recording(),
            AppCommand::LinuxSignalReceived(signal) => self.handle_linux_signal(signal),
            AppCommand::ExternalTrigger {
                request,
                deadline,
                response_tx,
            } => self.handle_external_trigger(request, deadline, response_tx),
            AppCommand::StartRecording(shortcut_id) => {
                let _ = self.start_recording(&shortcut_id);
            }
            AppCommand::StopRecording(shortcut_id) => {
                let _ = self.stop_recording(&shortcut_id);
            }
            AppCommand::RecordingStartCueFinished {
                recording_id,
                shortcut_id,
                result,
            } => self.recording_start_cue_finished(recording_id, &shortcut_id, result),
            AppCommand::RecordingDurationLimitReached {
                recording_id,
                shortcut_id,
            } => self.recording_duration_limit_reached(recording_id, &shortcut_id),
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
            AppCommand::WhisperRuntimeStatusChanged(status) => {
                self.update_whisper_runtime_status(status)
            }
            AppCommand::RefreshTrayRecordingPhase => self.force_refresh_recording_phase(),
            AppCommand::OutputScriptFinished {
                recording_id,
                shortcut_id,
                result,
            } => self.finish_output_script(recording_id, &shortcut_id, result),
            AppCommand::ClipboardCopyFinished { source, result } => {
                self.finish_clipboard_copy(source, result)
            }
            AppCommand::ClipboardPasteFinished { source, result } => {
                self.finish_clipboard_paste(source, result)
            }
            AppCommand::SpeakerRestoreFinished {
                recording_id,
                shortcut_id,
            } => self.speaker_restore_finished(recording_id, &shortcut_id),
            AppCommand::AudioInputDevicesRefreshed(devices) => {
                self.update_audio_input_devices(devices)
            }
            AppCommand::ClearHistory => self.clear_history(),
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
            AppCommand::ShutdownComplete => self.finish_shutdown(),
            AppCommand::Quit => self.quit(),
        }
    }

    fn toggle_recording(&self) {
        let _ = self.toggle_recording_for(DEFAULT_SHORTCUT_ID);
    }

    fn toggle_recording_for(&self, shortcut_id: &str) -> Result<(), String> {
        let phase = self.recording.borrow().phase();
        match phase {
            RecordingPhase::Idle => self.start_recording(shortcut_id),
            RecordingPhase::Arming | RecordingPhase::Recording
                if self.recording.borrow().active_shortcut_id() == Some(shortcut_id) =>
            {
                self.stop_recording(shortcut_id)
            }
            RecordingPhase::Arming | RecordingPhase::Recording => {
                let active_shortcut_id = self
                    .recording
                    .borrow()
                    .active_shortcut_id()
                    .unwrap_or("unknown")
                    .to_string();
                info!(
                    requested_shortcut_id = shortcut_id,
                    active_shortcut_id, "Recording toggle ignored for inactive shortcut"
                );
                Err(format!(
                    "toggle ignored for shortcut '{shortcut_id}' because shortcut \
                     '{active_shortcut_id}' is recording"
                ))
            }
            RecordingPhase::Processing => {
                info!("Recording toggle ignored while processing audio");
                Err("toggle ignored while a recording is being processed".to_string())
            }
        }
    }

    fn handle_linux_signal(&self, signal: i32) {
        let signal_match = {
            let config = self.config.borrow();
            linux_signal_match_for_config(&config, signal)
        };
        let Some(signal_match) = signal_match else {
            debug!(
                signal,
                signal_name = signal_name(signal).unwrap_or("unknown"),
                "Linux signal trigger ignored because no enabled shortcut uses this signal"
            );
            return;
        };

        let (phase, active_shortcut_id) = {
            let recording = self.recording.borrow();
            (
                recording.phase(),
                recording.active_shortcut_id().map(str::to_string),
            )
        };
        let Some(action) = linux_signal_action_for_recording_state(
            signal_match,
            phase,
            active_shortcut_id.as_deref(),
        ) else {
            debug!(
                signal,
                signal_name = signal_name(signal).unwrap_or("unknown"),
                ?phase,
                active_shortcut_id = active_shortcut_id.as_deref().unwrap_or("none"),
                "Linux signal trigger ignored for current recording state"
            );
            return;
        };

        match action {
            LinuxSignalAction::Start(shortcut_id) => {
                let _ = self.start_recording(&shortcut_id);
            }
            LinuxSignalAction::Stop(shortcut_id) => {
                let _ = self.stop_recording(&shortcut_id);
            }
        }
    }

    fn handle_external_trigger(
        &self,
        request: ExternalTriggerRequest,
        deadline: Instant,
        response_tx: mpsc::Sender<ExternalTriggerResponse>,
    ) {
        let received_at = Instant::now();
        if received_at > deadline {
            warn!(
                shortcut = %request.shortcut,
                action = request.action.as_str(),
                "External trigger expired before the runtime processed it; ignoring"
            );
            let _ = response_tx.send(ExternalTriggerResponse::rejected(
                "trigger request expired before MyApp processed it",
            ));
            return;
        }
        debug!(
            shortcut = %request.shortcut,
            action = request.action.as_str(),
            "External trigger received by runtime"
        );
        let shortcut_id = match resolve_shortcut_selector(&self.config.borrow(), &request.shortcut)
        {
            Ok(shortcut_id) => shortcut_id,
            Err(error) => {
                warn!(
                    shortcut = %request.shortcut,
                    action = request.action.as_str(),
                    error,
                    "External trigger rejected"
                );
                let _ = response_tx.send(ExternalTriggerResponse::rejected(error));
                return;
            }
        };

        let outcome = match request.action {
            ExternalTriggerAction::Start => self.start_recording(&shortcut_id),
            ExternalTriggerAction::Stop => self.stop_recording(&shortcut_id),
            ExternalTriggerAction::Toggle => self.toggle_recording_for(&shortcut_id),
        };
        match outcome {
            Ok(()) => {
                debug!(
                    shortcut_id,
                    action = request.action.as_str(),
                    elapsed_ms = received_at.elapsed().as_millis(),
                    "External trigger dispatched by runtime"
                );
                let _ = response_tx.send(ExternalTriggerResponse::accepted());
            }
            Err(reason) => {
                warn!(
                    shortcut_id,
                    action = request.action.as_str(),
                    reason,
                    "External trigger had no effect"
                );
                let _ = response_tx.send(ExternalTriggerResponse::rejected(reason));
            }
        }
    }

    fn start_recording(&self, shortcut_id: &str) -> Result<(), String> {
        if self.recording.borrow().phase() != RecordingPhase::Idle {
            let phase = self.recording.borrow_mut().start_recording(0, shortcut_id);
            self.set_recording_phase(phase);
            return Err(format!(
                "cannot start recording for shortcut '{shortcut_id}': recorder is {phase:?}"
            ));
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
                return Err(format!("{error:#}"));
            }
        };
        let phase = self
            .recording
            .borrow_mut()
            .start_recording(recording_id, shortcut_id);
        self.set_recording_phase(phase);

        if plan.beep_on_recording {
            return self.queue_start_cue_for_plan(plan);
        }

        self.start_capture_for_plan(plan)
    }

    fn queue_start_cue_for_plan(&self, plan: TranscriptionPlan) -> Result<(), String> {
        let recording_id = plan.recording_id;
        let shortcut_id = plan.shortcut_id.clone();
        self.pending_recording_start
            .replace(Some(PendingRecordingStart {
                recording_id,
                shortcut_id: shortcut_id.clone(),
                plan,
            }));

        let result = self
            .beep_service
            .borrow()
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("beep worker is shut down"))
            .and_then(|service| service.play_start_cue(recording_id, &shortcut_id));

        if let Err(error) = result {
            warn!(
                ?error,
                recording_id,
                shortcut_id,
                "failed to queue recording start cue; starting capture immediately"
            );
            let Some(pending) = self.pending_recording_start.borrow_mut().take() else {
                return Err(format!("failed to queue recording start cue: {error:#}"));
            };
            return self.start_capture_for_plan(pending.plan);
        }

        info!(
            recording_id,
            shortcut_id, "recording start cue queued before audio capture"
        );
        Ok(())
    }

    fn recording_start_cue_finished(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        result: std::result::Result<(), String>,
    ) {
        let Some(pending) = self.pending_recording_start.borrow_mut().take() else {
            debug!(
                recording_id,
                shortcut_id, "stale recording start cue ignored because no start is pending"
            );
            return;
        };

        if pending.recording_id != recording_id || pending.shortcut_id != shortcut_id {
            debug!(
                pending_recording_id = pending.recording_id,
                pending_shortcut_id = %pending.shortcut_id,
                recording_id,
                shortcut_id, "stale recording start cue ignored"
            );
            self.pending_recording_start.replace(Some(pending));
            return;
        }

        if let Err(error) = &result {
            warn!(
                recording_id,
                shortcut_id, error, "recording start cue failed; continuing with audio capture"
            );
        }
        let _ = self.start_capture_for_plan(pending.plan);
    }

    fn start_capture_for_plan(&self, plan: TranscriptionPlan) -> Result<(), String> {
        let recording_id = plan.recording_id;
        let shortcut_id = plan.shortcut_id.clone();
        let input_label = plan.input.display_label().to_string();
        self.mute_speakers_for_plan(&plan);

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
                .start_failed(recording_id, &shortcut_id);
            self.set_recording_phase(phase);
            self.restore_speakers_for_recording(recording_id, &shortcut_id);
            return Err(format!("failed to start audio capture: {error:#}"));
        }
        info!(
            recording_id,
            shortcut_id,
            input = input_label,
            "audio capture start requested"
        );
        Ok(())
    }

    fn stop_recording(&self, shortcut_id: &str) -> Result<(), String> {
        if let Some(pending) = self.pending_recording_start.borrow().as_ref()
            && pending.shortcut_id == shortcut_id
        {
            let recording_id = pending.recording_id;
            self.pending_recording_start.borrow_mut().take();
            let phase = self
                .recording
                .borrow_mut()
                .cancel_arming(recording_id, shortcut_id);
            self.set_recording_phase(phase);
            return Ok(());
        }

        let started_at = Instant::now();
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

        debug!(
            shortcut_id,
            recording_id,
            elapsed_ms = started_at.elapsed().as_millis(),
            "recording stop request processed"
        );
        let Some(recording_id) = recording_id else {
            return Err(format!(
                "no active recording to stop for shortcut '{shortcut_id}'"
            ));
        };
        if let Err(error) = stop_result {
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
            let play_stop_cue =
                self.queue_stop_cue_after_speaker_restore_for_shortcut(recording_id, shortcut_id);
            if !self.restore_speakers_for_recording(recording_id, shortcut_id) && play_stop_cue {
                self.play_queued_stop_cue_if_needed(recording_id, shortcut_id);
            }
        }
        Ok(())
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
        self.schedule_recording_duration_limit(recording_id, shortcut_id);
    }

    fn schedule_recording_duration_limit(&self, recording_id: u64, shortcut_id: &str) {
        let command_tx = self.command_sender();
        let shortcut_id = shortcut_id.to_string();
        let _source_id = gtk::glib::timeout_add_local(MAX_RECORDING_DURATION, move || {
            let _ = command_tx.send(AppCommand::RecordingDurationLimitReached {
                recording_id,
                shortcut_id: shortcut_id.clone(),
            });
            gtk::glib::ControlFlow::Break
        });
    }

    fn recording_duration_limit_reached(&self, recording_id: u64, shortcut_id: &str) {
        if !recording_duration_limit_applies(&self.recording.borrow(), recording_id, shortcut_id) {
            debug!(
                recording_id,
                shortcut_id, "recording duration limit ignored for stale recording"
            );
            return;
        }

        info!(
            recording_id,
            shortcut_id,
            max_duration_seconds = MAX_RECORDING_DURATION.as_secs(),
            "recording duration limit reached; stopping recording"
        );
        let _ = self.stop_recording(shortcut_id);
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
        self.restore_speakers_for_recording(recording_id, shortcut_id);
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
        let play_stop_cue = result
            .as_ref()
            .ok()
            .is_some_and(|request| request.beep_on_recording);
        if play_stop_cue {
            self.queue_stop_cue_after_speaker_restore(recording_id, shortcut_id);
        }
        if !self.restore_speakers_for_recording(recording_id, shortcut_id) && play_stop_cue {
            self.play_queued_stop_cue_if_needed(recording_id, shortcut_id);
        }

        match &result {
            Ok(request) => info!(
                recording_id,
                shortcut_id,
                input = %request.audio.input_label,
                capture_duration_ms = request.audio.duration_ms(),
                capture_wall_duration_ms = request.audio.wall_duration_ms(),
                source_frames = request.audio.frame_count(),
                "audio capture stopped; submitting transcription"
            ),
            Err(error) => warn!(
                recording_id,
                shortcut_id, error, "audio capture stopped with error"
            ),
        }

        let result = result.map_err(anyhow::Error::msg).and_then(|request| {
            self.transcription_service
                .borrow()
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("transcription worker is shut down"))
                .and_then(|service| service.submit(request))
        });

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
        if !self
            .recording
            .borrow()
            .is_processing(recording_id, shortcut_id)
        {
            info!(
                recording_id,
                shortcut_id, "ignoring stale transcription result"
            );
            return;
        }

        match result {
            Ok(result) => {
                match &result.status {
                    TranscriptionStatus::Completed => info!(
                        recording_id,
                        shortcut_id,
                        inference_duration_ms = result.debug.inference_duration_ms,
                        text_chars = result.text.chars().count(),
                        "transcription finished; applying output"
                    ),
                    TranscriptionStatus::Skipped { reason } => info!(
                        recording_id,
                        shortcut_id,
                        reason = reason.label(),
                        capture_duration_ms = result.debug.capture_duration_ms,
                        audio_rms = result.debug.audio_rms,
                        audio_peak = result.debug.audio_peak,
                        "transcription skipped; finishing without output"
                    ),
                }
                log_recognized_text(shortcut_id, &result);
                if matches!(result.status, TranscriptionStatus::Completed)
                    && result.output.script.is_none()
                {
                    self.append_history_entry(history_entry_from_transcription(
                        recording_id,
                        shortcut_id,
                        &result,
                        HistorySource::Transcription,
                        &result.text,
                    ));
                }
                match self.apply_transcription_output(recording_id, shortcut_id, &result) {
                    OutputDelivery::Queued(completion) => {
                        self.set_pending_output(recording_id, shortcut_id, completion);
                    }
                    OutputDelivery::NotQueued => {
                        self.finish_processing_now(recording_id, shortcut_id, &Ok(()));
                    }
                }
            }
            Err(error) => {
                let failed: std::result::Result<(), String> = Err(error);
                self.finish_processing_now(recording_id, shortcut_id, &failed);
            }
        }
    }

    fn apply_transcription_output(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        result: &TranscriptionResult,
    ) -> OutputDelivery {
        if let Some(output_service) = self.output_service.borrow().as_ref() {
            output_service.apply(recording_id, shortcut_id, result)
        } else {
            warn!(recording_id, shortcut_id, "output worker is not running");
            OutputDelivery::NotQueued
        }
    }

    fn set_pending_output(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        completion: OutputCompletion,
    ) {
        debug!(
            recording_id,
            shortcut_id,
            ?completion,
            "recording processing is waiting for output delivery"
        );
        self.pending_output.replace(Some(PendingOutputDelivery {
            recording_id,
            shortcut_id: shortcut_id.to_string(),
            completion,
        }));
    }

    fn update_pending_output_completion(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        completion: OutputCompletion,
    ) {
        if self
            .pending_output
            .borrow()
            .as_ref()
            .is_some_and(|pending| pending.matches_recording(recording_id, shortcut_id))
        {
            self.set_pending_output(recording_id, shortcut_id, completion);
        }
    }

    fn finish_pending_output(&self, recording_id: u64, shortcut_id: &str, reason: &str) {
        let Some(pending) = self.pending_output.borrow().as_ref().cloned() else {
            debug!(
                recording_id,
                shortcut_id, reason, "output completion ignored because no output is pending"
            );
            return;
        };
        if !pending.matches_recording(recording_id, shortcut_id) {
            debug!(
                pending_recording_id = pending.recording_id,
                pending_shortcut_id = %pending.shortcut_id,
                recording_id,
                shortcut_id,
                reason, "stale output completion ignored"
            );
            return;
        }

        self.pending_output.borrow_mut().take();
        self.finish_processing_now(recording_id, shortcut_id, &Ok(()));
    }

    fn finish_processing_now<T>(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        result: &std::result::Result<T, String>,
    ) {
        let (phase, accepted) =
            self.recording
                .borrow_mut()
                .finish_processing(recording_id, shortcut_id, result);
        if !accepted {
            info!(
                recording_id,
                shortcut_id, "ignoring stale processing completion"
            );
        }
        if accepted {
            debug!(
                recording_id,
                shortcut_id,
                ?phase,
                "recording processing finished"
            );
        }
        self.set_recording_phase(phase);
    }

    fn mute_speakers_for_plan(&self, plan: &TranscriptionPlan) {
        if !plan.mute_output_while_recording {
            return;
        }
        let result = self
            .speaker_mute_service
            .borrow()
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("speaker mute worker is shut down"))
            .and_then(|service| service.mute_for_recording(plan.recording_id, &plan.shortcut_id));
        if let Err(error) = result {
            warn!(
                ?error,
                recording_id = plan.recording_id,
                shortcut_id = %plan.shortcut_id,
                "failed to queue speaker mute request"
            );
        }
    }

    fn restore_speakers_for_recording(&self, recording_id: u64, shortcut_id: &str) -> bool {
        let result = self
            .speaker_mute_service
            .borrow()
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("speaker mute worker is shut down"))
            .and_then(|service| service.restore_for_recording(recording_id, shortcut_id));
        if let Err(error) = result {
            warn!(
                ?error,
                recording_id, shortcut_id, "failed to queue speaker mute restore request"
            );
            return false;
        }
        true
    }

    fn queue_stop_cue_after_speaker_restore_for_shortcut(
        &self,
        recording_id: u64,
        shortcut_id: &str,
    ) -> bool {
        let enabled = self
            .config
            .borrow()
            .shortcut_by_id(shortcut_id)
            .is_some_and(|shortcut| shortcut.beep_on_recording);
        if enabled {
            self.queue_stop_cue_after_speaker_restore(recording_id, shortcut_id);
        }
        enabled
    }

    fn queue_stop_cue_after_speaker_restore(&self, recording_id: u64, shortcut_id: &str) {
        self.pending_stop_cues
            .borrow_mut()
            .insert((recording_id, shortcut_id.to_string()));
    }

    fn speaker_restore_finished(&self, recording_id: u64, shortcut_id: &str) {
        self.play_queued_stop_cue_if_needed(recording_id, shortcut_id);
    }

    fn play_queued_stop_cue_if_needed(&self, recording_id: u64, shortcut_id: &str) {
        if self
            .pending_stop_cues
            .borrow_mut()
            .remove(&(recording_id, shortcut_id.to_string()))
        {
            self.play_stop_cue(recording_id, shortcut_id);
        }
    }

    fn play_stop_cue(&self, recording_id: u64, shortcut_id: &str) {
        let result = self
            .beep_service
            .borrow()
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("beep worker is shut down"))
            .and_then(|service| service.play_stop_cue(recording_id, shortcut_id));
        if let Err(error) = result {
            warn!(
                ?error,
                recording_id, shortcut_id, "failed to queue recording stop cue"
            );
        }
    }

    fn finish_output_script(
        &self,
        recording_id: u64,
        shortcut_id: &str,
        result: std::result::Result<OutputScriptResult, String>,
    ) {
        if !self
            .pending_output
            .borrow()
            .as_ref()
            .is_some_and(|pending| {
                pending.matches(recording_id, shortcut_id, OutputCompletion::Script)
            })
        {
            debug!(
                recording_id,
                shortcut_id, "stale output script result ignored"
            );
            return;
        }

        match result {
            Ok(result) => {
                info!(
                    recording_id,
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
                if let Some(output_text) = result.output_text.clone() {
                    self.append_history_entry(history_entry_from_script_result(
                        recording_id,
                        shortcut_id,
                        &result,
                        &output_text,
                    ));
                    if let Some(output_service) = self.output_service.borrow().as_ref() {
                        match output_service.copy_final_text_if_requested(
                            ClipboardCopySource::ScriptStdout {
                                recording_id,
                                shortcut_id: shortcut_id.to_string(),
                                script_path: result.script_path.clone(),
                            },
                            &result.output,
                            &output_text,
                        ) {
                            OutputDelivery::Queued(completion) => {
                                self.update_pending_output_completion(
                                    recording_id,
                                    shortcut_id,
                                    completion,
                                );
                            }
                            OutputDelivery::NotQueued => {
                                self.finish_pending_output(
                                    recording_id,
                                    shortcut_id,
                                    "script output did not queue clipboard delivery",
                                );
                            }
                        }
                    } else {
                        warn!(
                            recording_id,
                            shortcut_id,
                            script = %result.script_path,
                            "output worker is not running"
                        );
                        self.finish_pending_output(
                            recording_id,
                            shortcut_id,
                            "output worker unavailable after script",
                        );
                    }
                } else {
                    self.finish_pending_output(
                        recording_id,
                        shortcut_id,
                        "script output finished without clipboard delivery",
                    );
                }
            }
            Err(error) => {
                warn!(recording_id, shortcut_id, error, "output script failed");
                self.finish_pending_output(recording_id, shortcut_id, "output script failed");
            }
        }
    }

    fn finish_clipboard_copy(
        &self,
        source: ClipboardCopySource,
        result: std::result::Result<ClipboardCopyOutcome, String>,
    ) {
        let shortcut_id = source.shortcut_id();
        let recording_id = source.recording_id();
        let copy_source = source.kind();
        let script_path = source.script_path().unwrap_or("");
        let copy_failed = result.is_err();
        match result {
            Ok(result) => {
                info!(
                    recording_id,
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
                recording_id,
                shortcut_id,
                source = copy_source,
                script = script_path,
                error,
                "clipboard copy failed"
            ),
        }

        let pending_completion = self
            .pending_output
            .borrow()
            .as_ref()
            .filter(|pending| pending.matches_recording(recording_id, shortcut_id))
            .map(|pending| pending.completion);
        match pending_completion {
            Some(OutputCompletion::ClipboardCopy) => {
                self.finish_pending_output(recording_id, shortcut_id, "clipboard copy finished");
            }
            Some(OutputCompletion::ClipboardPaste) if copy_failed => {
                self.finish_pending_output(recording_id, shortcut_id, "clipboard copy failed");
            }
            _ => {}
        }
    }

    fn finish_clipboard_paste(
        &self,
        source: ClipboardCopySource,
        result: std::result::Result<ClipboardPasteOutcome, String>,
    ) {
        let shortcut_id = source.shortcut_id();
        let recording_id = source.recording_id();
        let copy_source = source.kind();
        let script_path = source.script_path().unwrap_or("");
        match result {
            Ok(result) => info!(
                recording_id,
                shortcut_id,
                source = copy_source,
                script = script_path,
                clipboard_backend = result.backend.as_str(),
                paste_shortcut = result.shortcut.label(),
                "Pasted text from clipboard"
            ),
            Err(error) => warn!(
                recording_id,
                shortcut_id,
                source = copy_source,
                script = script_path,
                error,
                "clipboard paste failed"
            ),
        }
        if self
            .pending_output
            .borrow()
            .as_ref()
            .is_some_and(|pending| {
                pending.matches(recording_id, shortcut_id, OutputCompletion::ClipboardPaste)
            })
        {
            self.finish_pending_output(recording_id, shortcut_id, "clipboard paste finished");
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
                SettingsWindowInit {
                    config: self.config.borrow().clone(),
                    audio_input_devices: self.audio_input_devices.borrow().clone(),
                    model_states: self.model_row_states(),
                    ready_model_ids: self.model_store.ready_model_ids(),
                    history_entries: self.history_entries.borrow().clone(),
                    whisper_runtime_status: self.whisper_runtime_status.borrow().clone(),
                    shortcut_trigger_capabilities: shortcut_trigger_capabilities(),
                    command_tx: self.command_sender(),
                },
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
        if let Some(mut signal_trigger_service) = self.signal_trigger_service.borrow_mut().take() {
            signal_trigger_service.shutdown();
        }
        match SignalTriggerService::spawn(self.command_sender(), &config) {
            Ok(service) => {
                self.signal_trigger_service.replace(Some(service));
            }
            Err(error) => {
                warn!(?error, "failed to reconfigure Linux signal triggers");
                self.signal_trigger_service.replace(None);
            }
        }
        if let Some(transcription_service) = self.transcription_service.borrow().as_ref() {
            if let Err(error) =
                transcription_service.set_keep_model_loaded(config.general.keep_model_loaded)
            {
                warn!(?error, "failed to update whisper model cache policy");
            }
            if let Err(error) =
                transcription_service.clear_cached_context("settings config changed")
            {
                warn!(
                    ?error,
                    "failed to clear whisper model cache after settings change"
                );
            }
        } else {
            warn!("transcription worker is not running while applying settings config");
        }

        self.config.replace(config.clone());
        self.update_whisper_runtime_status(WhisperRuntimeStatus::initial(
            config.general.compute_backend,
        ));
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
                if let Some(model_path) = model_path {
                    let transcription_service = self.transcription_service.borrow();
                    if let Some(transcription_service) = transcription_service.as_ref()
                        && let Err(error) =
                            transcription_service.clear_cached_model_path(model_path)
                    {
                        warn!(?error, model_id, "failed to clear cached deleted model");
                    }
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
                let had_ready_models = !self.model_store.ready_model_ids().is_empty();
                match self.model_store.mark_ready(&model_id) {
                    Ok(ready_model_ids) => {
                        info!(
                            model_id,
                            ready_model_count = ready_model_ids.len(),
                            "model download completed"
                        );
                        if !had_ready_models {
                            self.assign_first_ready_model_to_factory_shortcuts(&model_id);
                        }
                    }
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

    fn assign_first_ready_model_to_factory_shortcuts(&self, model_id: &str) {
        let Some(config) =
            config_with_factory_shortcut_models(self.config.borrow().clone(), model_id)
        else {
            warn!(
                model_id,
                "failed to assign first ready model because no factory model shortcut exists"
            );
            return;
        };

        if let Err(error) = self.config_store.save(&config) {
            warn!(
                ?error,
                model_id, "failed to persist first ready model on factory model shortcuts"
            );
            return;
        }

        self.config.replace(config.clone());
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.assign_factory_model_to_shortcuts(model_id);
        }
        info!(
            model_id,
            "assigned first ready model to factory model shortcuts"
        );
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

    fn update_audio_input_devices(&self, devices: Vec<AudioInputDevice>) {
        self.audio_input_devices.replace(devices.clone());
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.update_audio_input_devices(devices);
        }
    }

    fn append_history_entry(&self, mut entry: HistoryEntry) {
        entry.text = entry.text.trim().to_string();
        if entry.text.is_empty() {
            return;
        }

        if let Err(error) = self.history_store.append(&entry) {
            warn!(
                ?error,
                history_path = %self.history_store.path().display(),
                recording_id = entry.recording_id,
                shortcut_id = %entry.shortcut_id,
                "failed to append transcription history"
            );
            return;
        }

        self.history_entries.borrow_mut().push(entry);
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.update_history_entries(self.history_entries.borrow().clone());
        }
    }

    fn clear_history(&self) {
        match self.history_store.clear() {
            Ok(()) => {
                self.history_entries.borrow_mut().clear();
                if let Some(window) = self.settings_window.borrow().as_ref() {
                    window.update_history_entries(Vec::new());
                    window.update_save_status("History cleared");
                }
            }
            Err(error) => {
                warn!(?error, "failed to clear transcription history");
                if let Some(window) = self.settings_window.borrow().as_ref() {
                    window.update_save_status(&format!("Failed to clear history: {error}"));
                }
            }
        }
    }

    fn update_whisper_runtime_status(&self, status: WhisperRuntimeStatus) {
        debug!(status = %status.summary(), "whisper runtime status updated");
        self.whisper_runtime_status.replace(status.clone());
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.update_whisper_runtime_status(status);
        }
    }

    fn model_row_states(&self) -> Vec<ModelRowState> {
        let config = self.config.borrow();
        let download_manager = self.download_manager.borrow();
        self.model_store
            .row_states(&config, download_manager.statuses())
    }

    fn quit(&self) {
        if self.is_quitting.replace(true) {
            debug!("Quit ignored because shutdown is already in progress");
            return;
        }

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

        let services = ShutdownServices {
            recording_pipeline: self.recording_pipeline.borrow_mut().take(),
            speaker_mute_service: self.speaker_mute_service.borrow_mut().take(),
            beep_service: self.beep_service.borrow_mut().take(),
            output_service: self.output_service.borrow_mut().take(),
            signal_trigger_service: self.signal_trigger_service.borrow_mut().take(),
            external_trigger_service: self.external_trigger_service.borrow_mut().take(),
            transcription_service: self.transcription_service.borrow_mut().take(),
            hotkey_backend: self.hotkey_backend.borrow_mut().take(),
        };
        spawn_shutdown_worker(self.command_sender(), services);
    }

    fn finish_shutdown(&self) {
        info!("MyApp shutdown complete");
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
            audio_input = %config.general.audio_input.display_label(),
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

fn config_with_factory_shortcut_models(mut config: AppConfig, model_id: &str) -> Option<AppConfig> {
    let mut changed = false;
    for shortcut in &mut config.shortcuts {
        if shortcut.model_id == shared::DEFAULT_MODEL_ID {
            shortcut.model_id = model_id.to_string();
            changed = true;
        }
    }
    changed.then_some(config)
}

fn spawn_shutdown_worker(command_tx: mpsc::Sender<AppCommand>, services: ShutdownServices) {
    let services = Arc::new(Mutex::new(Some(services)));
    let worker_services = Arc::clone(&services);
    let worker_command_tx = command_tx.clone();
    match thread::Builder::new()
        .name("myapp-shutdown".to_string())
        .spawn(move || {
            if let Some(services) = take_shutdown_services(&worker_services) {
                services.shutdown();
            }
            let _ = worker_command_tx.send(AppCommand::ShutdownComplete);
        }) {
        Ok(_) => {}
        Err(error) => {
            warn!(
                ?error,
                "failed to spawn shutdown worker; completing shutdown on main thread"
            );
            if let Some(services) = take_shutdown_services(&services) {
                services.shutdown();
            }
            let _ = command_tx.send(AppCommand::ShutdownComplete);
        }
    }
}

fn take_shutdown_services(
    services: &Arc<Mutex<Option<ShutdownServices>>>,
) -> Option<ShutdownServices> {
    match services.lock() {
        Ok(mut guard) => guard.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

fn trigger_summary(trigger: &ShortcutTrigger) -> String {
    match trigger {
        ShortcutTrigger::Keyboard { accelerator } => format!("keyboard:{accelerator}"),
        ShortcutTrigger::LinuxSignal {
            start_signal,
            stop_signal,
        } if start_signal == stop_signal => {
            format!("signal:{}:start-stop", start_signal.as_str())
        }
        ShortcutTrigger::LinuxSignal {
            start_signal,
            stop_signal,
        } => format!("signal:{}->{}", start_signal.as_str(), stop_signal.as_str()),
    }
}

fn recording_duration_limit_applies(
    recording: &RecordingService,
    recording_id: u64,
    shortcut_id: &str,
) -> bool {
    recording.active_recording_id() == Some(recording_id)
        && recording.active_shortcut_id() == Some(shortcut_id)
        && matches!(
            recording.phase(),
            RecordingPhase::Arming | RecordingPhase::Recording
        )
}

fn history_entry_from_transcription(
    recording_id: u64,
    shortcut_id: &str,
    result: &TranscriptionResult,
    source: HistorySource,
    text: &str,
) -> HistoryEntry {
    HistoryEntry {
        created_at_unix_ms: unix_time_ms_now(),
        recording_id,
        shortcut_id: shortcut_id.to_string(),
        shortcut_name: result.debug.shortcut_name.clone(),
        model_id: result.debug.model_id.clone(),
        language: result.debug.language.clone(),
        source,
        text: text.to_string(),
    }
}

fn history_entry_from_script_result(
    recording_id: u64,
    shortcut_id: &str,
    result: &OutputScriptResult,
    text: &str,
) -> HistoryEntry {
    HistoryEntry {
        created_at_unix_ms: unix_time_ms_now(),
        recording_id,
        shortcut_id: shortcut_id.to_string(),
        shortcut_name: result.shortcut_name.clone(),
        model_id: result.model_id.clone(),
        language: result.language.clone(),
        source: HistorySource::Script,
        text: text.to_string(),
    }
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
    use super::*;

    #[test]
    fn pending_output_matches_recording_and_completion() {
        let pending = PendingOutputDelivery {
            recording_id: 7,
            shortcut_id: "default".to_string(),
            completion: OutputCompletion::ClipboardPaste,
        };

        assert!(pending.matches(7, "default", OutputCompletion::ClipboardPaste));
        assert!(pending.matches_recording(7, "default"));
        assert!(!pending.matches(7, "default", OutputCompletion::ClipboardCopy));
        assert!(!pending.matches_recording(8, "default"));
        assert!(!pending.matches_recording(7, "other"));
    }

    #[test]
    fn config_with_factory_shortcut_models_updates_all_factory_model_shortcuts() {
        let mut config = AppConfig::default();
        config.shortcuts.push(shared::ShortcutProfile::new_profile(
            "signal".to_string(),
            "Signal".to_string(),
            shared::DEFAULT_MODEL_ID.to_string(),
        ));

        let updated = config_with_factory_shortcut_models(config, "small-q8_0")
            .expect("factory model shortcuts exist");

        assert_eq!(updated.default_shortcut().model_id, "small-q8_0");
        assert_eq!(updated.shortcuts[1].model_id, "small-q8_0");
    }

    #[test]
    fn config_with_factory_shortcut_models_preserves_custom_model_shortcuts() {
        let mut config = AppConfig::default();
        config.shortcuts.push(shared::ShortcutProfile::new_profile(
            "custom".to_string(),
            "Custom".to_string(),
            "tiny".to_string(),
        ));

        let updated = config_with_factory_shortcut_models(config, "small-q8_0")
            .expect("factory model shortcut exists");

        assert_eq!(updated.default_shortcut().model_id, "small-q8_0");
        assert_eq!(updated.shortcuts[1].model_id, "tiny");
    }

    #[test]
    fn recording_duration_limit_only_applies_to_active_recording() {
        let mut recording = RecordingService::default();

        assert!(!recording_duration_limit_applies(&recording, 1, "default"));
        recording.start_recording(1, "default");
        assert!(recording_duration_limit_applies(&recording, 1, "default"));
        assert!(!recording_duration_limit_applies(&recording, 2, "default"));
        assert!(!recording_duration_limit_applies(&recording, 1, "other"));
        recording.capture_started(1, "default");
        assert!(recording_duration_limit_applies(&recording, 1, "default"));
        recording.stop_recording("default");
        assert!(!recording_duration_limit_applies(&recording, 1, "default"));
    }
}
