mod hotkey_widget;

use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use anyhow::Result;
use eframe::egui;
use egui_keybind::{Keybind, Shortcut};

use crate::audio::AudioRecorder;
use crate::config::{AppConfig, ConfigStore};
use crate::hotkey::{HotkeyBackend, HotkeySpec};
use crate::whisper::WhisperRecognizer;

use hotkey_widget::{hotkey_from_shortcut, shortcut_from_hotkey};

const SYSTEM_DEFAULT_MIC_LABEL: &str = "System default";

pub struct SettingsWindow {
    runtime: RefCell<Option<SettingsRuntime>>,
    config: Arc<Mutex<AppConfig>>,
    store: ConfigStore,
    hotkey_backend: Arc<dyn HotkeyBackend>,
    audio_recorder: Arc<dyn AudioRecorder>,
    whisper_recognizer: Arc<dyn WhisperRecognizer>,
}

impl SettingsWindow {
    pub fn new(
        config: Arc<Mutex<AppConfig>>,
        store: ConfigStore,
        hotkey_backend: Arc<dyn HotkeyBackend>,
        audio_recorder: Arc<dyn AudioRecorder>,
        whisper_recognizer: Arc<dyn WhisperRecognizer>,
    ) -> Self {
        Self {
            runtime: RefCell::new(None),
            config,
            store,
            hotkey_backend,
            audio_recorder,
            whisper_recognizer,
        }
    }

    pub fn present(&self) {
        let mut runtime = self.runtime.borrow_mut();
        if let Some(existing_runtime) = runtime.take() {
            if existing_runtime.handle.is_finished() {
                if let Err(error) = existing_runtime.handle.join() {
                    eprintln!("Settings window thread ended with a panic: {error:?}");
                }
            } else {
                existing_runtime.show();
                *runtime = Some(existing_runtime);
                return;
            }
        }

        let (command_sender, command_receiver) = mpsc::channel();
        let repaint_context = Arc::new(Mutex::new(None));
        let config = Arc::clone(&self.config);
        let store = self.store.clone();
        let hotkey_backend = Arc::clone(&self.hotkey_backend);
        let audio_recorder = Arc::clone(&self.audio_recorder);
        let whisper_recognizer = Arc::clone(&self.whisper_recognizer);
        let thread_repaint_context = Arc::clone(&repaint_context);

        match thread::Builder::new()
            .name("voice-egui-settings".to_string())
            .spawn(move || {
                if let Err(error) = run_settings_window(
                    config,
                    store,
                    hotkey_backend,
                    audio_recorder,
                    whisper_recognizer,
                    command_receiver,
                    thread_repaint_context,
                ) {
                    eprintln!("Settings window failed: {error}");
                }
            }) {
            Ok(join_handle) => {
                *runtime = Some(SettingsRuntime {
                    command_sender,
                    repaint_context,
                    handle: join_handle,
                });
            }
            Err(error) => {
                eprintln!("Failed to spawn settings window thread: {error}");
            }
        }
    }
}

struct SettingsRuntime {
    command_sender: Sender<SettingsCommand>,
    repaint_context: Arc<Mutex<Option<egui::Context>>>,
    handle: JoinHandle<()>,
}

impl SettingsRuntime {
    fn show(&self) {
        if let Err(error) = self.command_sender.send(SettingsCommand::Show) {
            eprintln!("Failed to request Settings window presentation: {error:?}");
            return;
        }

        let repaint_context = self
            .repaint_context
            .lock()
            .expect("settings repaint context was poisoned");
        if let Some(ctx) = repaint_context.as_ref() {
            ctx.request_repaint();
        }
    }
}

enum SettingsCommand {
    Show,
}

fn run_settings_window(
    config: Arc<Mutex<AppConfig>>,
    store: ConfigStore,
    hotkey_backend: Arc<dyn HotkeyBackend>,
    audio_recorder: Arc<dyn AudioRecorder>,
    whisper_recognizer: Arc<dyn WhisperRecognizer>,
    command_receiver: Receiver<SettingsCommand>,
    repaint_context: Arc<Mutex<Option<egui::Context>>>,
) -> eframe::Result {
    let options = native_options();
    eframe::run_native(
        "Voice Settings",
        options,
        Box::new(move |creation_context| {
            creation_context
                .egui_ctx
                .set_visuals(egui::Visuals::light());
            repaint_context
                .lock()
                .expect("settings repaint context was poisoned")
                .replace(creation_context.egui_ctx.clone());
            Ok(Box::new(SettingsApp::new(
                config,
                store,
                hotkey_backend,
                audio_recorder,
                whisper_recognizer,
                command_receiver,
            )))
        }),
    )
}

fn native_options() -> eframe::NativeOptions {
    let viewport = egui::ViewportBuilder::default()
        .with_title("Voice Settings")
        .with_app_id("voice-settings")
        .with_inner_size([760.0, 420.0])
        .with_min_inner_size([680.0, 360.0]);

    eframe::NativeOptions {
        viewport,
        centered: true,
        event_loop_builder: Some(Box::new(|builder| {
            configure_event_loop_for_settings_thread(builder);
        })),
        ..Default::default()
    }
}

#[cfg(target_os = "linux")]
fn configure_event_loop_for_settings_thread(
    builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>,
) {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    use winit::platform::x11::EventLoopBuilderExtX11;

    EventLoopBuilderExtX11::with_any_thread(builder, true);
    EventLoopBuilderExtWayland::with_any_thread(builder, true);
}

#[cfg(not(target_os = "linux"))]
fn configure_event_loop_for_settings_thread(
    _builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>,
) {
}

struct SettingsApp {
    config: Arc<Mutex<AppConfig>>,
    store: ConfigStore,
    hotkey_backend: Arc<dyn HotkeyBackend>,
    audio_recorder: Arc<dyn AudioRecorder>,
    whisper_recognizer: Arc<dyn WhisperRecognizer>,
    command_receiver: Receiver<SettingsCommand>,
    hotkey_shortcut: Shortcut,
    last_valid_shortcut: Shortcut,
    draft_hotkey: String,
    draft_model: String,
    draft_microphone: Option<String>,
    microphone_devices: Vec<String>,
    status_message: String,
    backend_status: String,
}

impl SettingsApp {
    fn new(
        config: Arc<Mutex<AppConfig>>,
        store: ConfigStore,
        hotkey_backend: Arc<dyn HotkeyBackend>,
        audio_recorder: Arc<dyn AudioRecorder>,
        whisper_recognizer: Arc<dyn WhisperRecognizer>,
        command_receiver: Receiver<SettingsCommand>,
    ) -> Self {
        let mut app = Self {
            config,
            store,
            hotkey_backend,
            audio_recorder,
            whisper_recognizer,
            command_receiver,
            hotkey_shortcut: Shortcut::NONE,
            last_valid_shortcut: Shortcut::NONE,
            draft_hotkey: String::new(),
            draft_model: String::new(),
            draft_microphone: None,
            microphone_devices: Vec::new(),
            status_message: String::new(),
            backend_status: String::new(),
        };
        app.load_draft_from_config();
        app.refresh_runtime_status();
        app.refresh_microphones();
        app
    }

    fn handle_pending_commands(&mut self, ctx: &egui::Context) {
        while let Ok(command) = self.command_receiver.try_recv() {
            match command {
                SettingsCommand::Show => {
                    ctx.set_visuals(egui::Visuals::light());
                    self.load_draft_from_config();
                    self.refresh_runtime_status();
                    self.refresh_microphones();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
        }
    }

    fn load_draft_from_config(&mut self) {
        let config_snapshot = self
            .config
            .lock()
            .expect("settings config state was poisoned")
            .clone();
        let (hotkey_shortcut, hotkey_status) =
            shortcut_from_hotkey(&config_snapshot.push_to_talk_hotkey)
                .map(|shortcut| (shortcut, String::new()))
                .unwrap_or_else(|error| {
                    (
                        Shortcut::NONE,
                        format!("Current hotkey cannot be edited visually yet: {error:#}"),
                    )
                });

        self.hotkey_shortcut = hotkey_shortcut;
        self.last_valid_shortcut = hotkey_shortcut;
        self.draft_hotkey = config_snapshot.push_to_talk_hotkey;
        self.draft_model = config_snapshot.whisper_model;
        self.draft_microphone = config_snapshot.microphone_device;
        self.status_message = hotkey_status;
    }

    fn refresh_runtime_status(&mut self) {
        self.backend_status = self.whisper_recognizer.runtime_status().summary();
    }

    fn refresh_microphones(&mut self) {
        match self.audio_recorder.available_input_devices() {
            Ok(device_names) => {
                self.microphone_devices = device_names;
            }
            Err(error) => {
                self.microphone_devices.clear();
                self.status_message = format!("Failed to list microphones: {error:#}");
            }
        }
    }

    fn apply_shortcut_change(&mut self) {
        if self.hotkey_shortcut.pointer().is_some() {
            self.hotkey_shortcut = self.last_valid_shortcut;
            self.status_message =
                "Mouse buttons are not supported for push-to-talk yet.".to_string();
            return;
        }

        match hotkey_from_shortcut(&self.hotkey_shortcut) {
            Ok(hotkey) => {
                self.draft_hotkey = hotkey;
                self.last_valid_shortcut = self.hotkey_shortcut;
                self.status_message.clear();
            }
            Err(error) => {
                self.hotkey_shortcut = self.last_valid_shortcut;
                self.status_message = format!("Unsupported hotkey: {error:#}");
            }
        }
    }

    fn save(&mut self) {
        let hotkey = match HotkeySpec::parse(&self.draft_hotkey) {
            Ok(spec) => spec.canonical().to_string(),
            Err(error) => {
                self.status_message = format!("Save failed: {error:#}");
                return;
            }
        };
        let current_config = self
            .config
            .lock()
            .expect("settings config state was poisoned")
            .clone();
        let next_config = AppConfig {
            push_to_talk_hotkey: hotkey,
            whisper_model: self.draft_model.trim().to_string(),
            microphone_device: normalize_microphone(self.draft_microphone.as_deref()),
            whisper_backend: current_config.whisper_backend,
            gpu_device: current_config.gpu_device,
        };

        match save_settings(
            &next_config,
            &self.store,
            &self.config,
            &self.hotkey_backend,
            &self.audio_recorder,
            &self.whisper_recognizer,
        ) {
            Ok(()) => {
                self.draft_hotkey = next_config.push_to_talk_hotkey;
                self.draft_model = next_config.whisper_model;
                self.draft_microphone = next_config.microphone_device;
                self.status_message = "Saved.".to_string();
                self.refresh_runtime_status();
            }
            Err(error) => {
                self.status_message = format!("Save failed: {error:#}");
                self.refresh_runtime_status();
            }
        }
    }

    fn selected_microphone_label(&self) -> String {
        match self.draft_microphone.as_deref() {
            Some(device_name)
                if !self
                    .microphone_devices
                    .iter()
                    .any(|available| available == device_name) =>
            {
                format!("{device_name} (missing)")
            }
            Some(device_name) => device_name.to_string(),
            None => SYSTEM_DEFAULT_MIC_LABEL.to_string(),
        }
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_pending_commands(ctx);
        if ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide(ctx);
        }

        egui::TopBottomPanel::bottom("voice-settings-actions").show(ctx, |ui| {
            ui.add_space(6.0);
            if !self.status_message.is_empty() {
                ui.label(&self.status_message);
                ui.add_space(4.0);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    self.hide(ctx);
                }
                if ui.button("Save").clicked() {
                    self.save();
                }
            });
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.heading("Voice Settings");
                    ui.add_space(8.0);

                    egui::Grid::new("voice-settings-grid")
                        .num_columns(2)
                        .spacing([12.0, 10.0])
                        .show(ui, |ui| {
                            ui.label("Push-to-talk hotkey");
                            ui.horizontal(|ui| {
                                let response = ui.add(
                                    Keybind::new(&mut self.hotkey_shortcut, "push-to-talk-hotkey")
                                        .with_text(""),
                                );
                                if response.changed() {
                                    self.apply_shortcut_change();
                                }
                                ui.label(&self.draft_hotkey);
                            });
                            ui.end_row();

                            ui.label("Microphone");
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("microphone-combo")
                                    .selected_text(self.selected_microphone_label())
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.draft_microphone,
                                            None,
                                            SYSTEM_DEFAULT_MIC_LABEL,
                                        );
                                        for device_name in &self.microphone_devices {
                                            ui.selectable_value(
                                                &mut self.draft_microphone,
                                                Some(device_name.clone()),
                                                device_name,
                                            );
                                        }
                                    });
                                if ui.button("Refresh").clicked() {
                                    self.refresh_microphones();
                                }
                            });
                            ui.end_row();

                            ui.label("Whisper model");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.draft_model)
                                        .desired_width(440.0)
                                        .hint_text("Model path or name"),
                                );
                                if ui.button("Browse...").clicked()
                                    && let Some(path) = rfd::FileDialog::new()
                                        .add_filter("Whisper model", &["bin"])
                                        .pick_file()
                                {
                                    self.draft_model = path.to_string_lossy().into_owned();
                                }
                            });
                            ui.end_row();

                            ui.label("Whisper backend");
                            ui.label(&self.backend_status);
                            ui.end_row();
                        });
                });
        });
    }
}

impl SettingsApp {
    fn hide(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }
}

fn save_settings(
    next_config: &AppConfig,
    store: &ConfigStore,
    config: &Arc<Mutex<AppConfig>>,
    hotkey_backend: &Arc<dyn HotkeyBackend>,
    audio_recorder: &Arc<dyn AudioRecorder>,
    whisper_recognizer: &Arc<dyn WhisperRecognizer>,
) -> Result<()> {
    hotkey_backend.configure_push_to_talk(&next_config.push_to_talk_hotkey)?;
    audio_recorder.configure_input_device(next_config.microphone_device.as_deref())?;
    whisper_recognizer.configure_model(
        &next_config.whisper_model,
        next_config.whisper_backend,
        next_config.gpu_device,
    )?;
    store.save(next_config)?;
    *config.lock().expect("settings config state was poisoned") = next_config.clone();
    Ok(())
}

fn normalize_microphone(device_name: Option<&str>) -> Option<String> {
    device_name
        .map(str::trim)
        .filter(|device_name| !device_name.is_empty())
        .map(ToOwned::to_owned)
}
