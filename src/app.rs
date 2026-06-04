use std::rc::Rc;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use crate::activity::{
    PushToTalkEvent, SystemClipboardWriter, TranscriptionResult, TranscriptionResultSender,
    VoiceActivityController, VoiceActivityState,
};
use crate::audio::{AudioRecorder, CpalAudioRecorder};
use crate::config::{AppConfig, ConfigStore};
use crate::hotkey::{AutoHotkeyBackend, HotkeyBackend};
use crate::settings::SettingsWindow;
use crate::tray::{
    AppIndicatorTray, ManagedTrayBackend, TrayAction, TrayBackend, create_tray_backend,
};
use crate::whisper::{RuntimeWhisperRecognizer, WhisperRecognizer};

pub fn run() -> Result<()> {
    gtk::init().context("failed to initialize GTK")?;

    let (tray_sender, tray_receiver) = tray_channel();

    let store = ConfigStore::new()?;
    let _config_path = store.path().to_path_buf();
    let loaded_config = store.load()?;
    let config = Arc::new(Mutex::new(loaded_config));

    let hotkey_backend: Arc<dyn HotkeyBackend> = Arc::new(AutoHotkeyBackend::default());
    let whisper_recognizer: Arc<dyn WhisperRecognizer> =
        Arc::new(RuntimeWhisperRecognizer::default());
    let audio_recorder: Arc<dyn AudioRecorder> = Arc::new(CpalAudioRecorder::default());

    let primary_tray = create_tray_backend(tray_sender.clone(), VoiceActivityState::Idle);
    let managed_tray = Rc::new(ManagedTrayBackend::new(
        primary_tray,
        VoiceActivityState::Idle,
    ));
    let tray_backend_for_activity: Rc<dyn TrayBackend> = managed_tray.clone();
    let (activity_sender, activity_receiver) = activity_channel();
    let activity_result_sender: TranscriptionResultSender = Arc::new(move |result| {
        if let Err(error) = activity_sender.send(result) {
            eprintln!("Failed to send transcription result to GTK main loop: {error:?}");
        }
    });
    let activity_controller = Rc::new(VoiceActivityController::new(
        tray_backend_for_activity,
        Arc::clone(&audio_recorder),
        Arc::clone(&whisper_recognizer),
        Rc::new(SystemClipboardWriter),
        activity_result_sender,
    ));
    let activity_result_controller = Rc::clone(&activity_controller);
    activity_receiver.attach(None, move |result| {
        if let Err(error) = activity_result_controller.handle_transcription_result(result) {
            eprintln!("Push-to-talk flow failed: {error:#}");
        }
        gtk::glib::ControlFlow::Continue
    });

    let (hotkey_sender, hotkey_receiver) = hotkey_channel();
    let hotkey_activity = Rc::clone(&activity_controller);
    hotkey_receiver.attach(None, move |event| {
        if let Err(error) = hotkey_activity.handle_push_to_talk_event(event) {
            eprintln!("Push-to-talk flow failed: {error:#}");
        }
        gtk::glib::ControlFlow::Continue
    });
    hotkey_backend.set_push_to_talk_handler(Box::new(move |event| {
        if let Err(error) = hotkey_sender.send(event) {
            eprintln!("Failed to send push-to-talk event to GTK main loop: {error:?}");
        }
    }))?;

    configure_backends(
        &config.lock().expect("app config state was poisoned"),
        &hotkey_backend,
        &audio_recorder,
        &whisper_recognizer,
    )?;

    let settings_window = Rc::new(SettingsWindow::new(
        Arc::clone(&config),
        store,
        Arc::clone(&hotkey_backend),
        Arc::clone(&audio_recorder),
        Arc::clone(&whisper_recognizer),
    ));

    let tray_settings = Rc::clone(&settings_window);
    let managed_tray_for_events = Rc::clone(&managed_tray);
    let fallback_sender = tray_sender.clone();
    tray_receiver.attach(None, move |action| {
        match action {
            TrayAction::OpenSettings => tray_settings.present(),
            TrayAction::Quit => gtk::main_quit(),
            TrayAction::StatusNotifierOffline => {
                if !managed_tray_for_events.has_legacy_fallback() {
                    match AppIndicatorTray::new(
                        fallback_sender.clone(),
                        managed_tray_for_events.visual_state(),
                    ) {
                        Ok(tray) => {
                            eprintln!(
                                "StatusNotifier tray went offline; started legacy AppIndicator fallback."
                            );
                            managed_tray_for_events.start_legacy_fallback(Box::new(tray));
                        }
                        Err(error) => {
                            eprintln!(
                                "StatusNotifier tray went offline and legacy fallback failed: {error:#}"
                            );
                            tray_settings.present();
                        }
                    }
                }
            }
            TrayAction::StatusNotifierOnline => {
                if managed_tray_for_events.stop_legacy_fallback() {
                    eprintln!("StatusNotifier tray is online again; stopped legacy fallback.");
                }
            }
        }

        gtk::glib::ControlFlow::Continue
    });
    let _tray_backend_name = managed_tray.backend_name();
    let _activity_controller = activity_controller;

    gtk::main();
    Ok(())
}

fn configure_backends(
    config: &AppConfig,
    hotkey_backend: &Arc<dyn HotkeyBackend>,
    audio_recorder: &Arc<dyn AudioRecorder>,
    whisper_recognizer: &Arc<dyn WhisperRecognizer>,
) -> Result<()> {
    if let Err(error) = hotkey_backend.configure_push_to_talk(&config.push_to_talk_hotkey) {
        eprintln!("Push-to-talk hotkey is not active yet: {error:#}");
    }
    if let Err(error) = audio_recorder.configure_input_device(config.microphone_device.as_deref()) {
        eprintln!("Microphone is not ready yet: {error:#}");
    }
    if let Err(error) = whisper_recognizer.configure_model(
        &config.whisper_model,
        config.whisper_backend,
        config.gpu_device,
    ) {
        eprintln!("Whisper model is not ready yet: {error:#}");
    }
    Ok(())
}

#[allow(deprecated)]
fn hotkey_channel() -> (
    gtk::glib::Sender<PushToTalkEvent>,
    gtk::glib::Receiver<PushToTalkEvent>,
) {
    gtk::glib::MainContext::channel(gtk::glib::Priority::DEFAULT)
}

#[allow(deprecated)]
fn tray_channel() -> (
    gtk::glib::Sender<TrayAction>,
    gtk::glib::Receiver<TrayAction>,
) {
    gtk::glib::MainContext::channel(gtk::glib::Priority::DEFAULT)
}

#[allow(deprecated)]
fn activity_channel() -> (
    gtk::glib::Sender<TranscriptionResult>,
    gtk::glib::Receiver<TranscriptionResult>,
) {
    gtk::glib::MainContext::channel(gtk::glib::Priority::DEFAULT)
}
