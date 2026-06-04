use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use gtk::prelude::*;

use crate::audio::AudioRecorder;
use crate::config::{AppConfig, ConfigStore};
use crate::hotkey::HotkeyBackend;
use crate::whisper::WhisperRecognizer;

const SYSTEM_DEFAULT_MIC_ID: &str = "__voice_system_default_microphone__";

pub struct SettingsWindow {
    window: gtk::Window,
    hotkey_entry: gtk::Entry,
    microphone_combo: gtk::ComboBoxText,
    model_entry: gtk::Entry,
    backend_status_label: gtk::Label,
    config: Rc<RefCell<AppConfig>>,
    audio_recorder: Arc<dyn AudioRecorder>,
    whisper_recognizer: Arc<dyn WhisperRecognizer>,
}

impl SettingsWindow {
    pub fn new(
        config: Rc<RefCell<AppConfig>>,
        store: ConfigStore,
        hotkey_backend: Rc<dyn HotkeyBackend>,
        audio_recorder: Arc<dyn AudioRecorder>,
        whisper_recognizer: Arc<dyn WhisperRecognizer>,
    ) -> Self {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("Voice Settings");
        window.set_default_size(560, 220);
        window.set_position(gtk::WindowPosition::Center);
        window.set_icon_name(Some("audio-input-microphone-symbolic"));

        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.set_margin_top(16);
        root.set_margin_bottom(16);
        root.set_margin_start(16);
        root.set_margin_end(16);

        let grid = gtk::Grid::new();
        grid.set_row_spacing(10);
        grid.set_column_spacing(12);

        let hotkey_label = left_label("Push-to-talk hotkey");
        let hotkey_entry = gtk::Entry::new();
        hotkey_entry.set_hexpand(true);

        let microphone_label = left_label("Microphone");
        let microphone_combo = gtk::ComboBoxText::new();
        microphone_combo.set_hexpand(true);

        let model_label = left_label("Whisper model");
        let model_entry = gtk::Entry::new();
        model_entry.set_hexpand(true);
        model_entry.set_placeholder_text(Some("Model path or name"));

        let browse_button = gtk::Button::with_label("Browse...");
        let model_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        model_row.pack_start(&model_entry, true, true, 0);
        model_row.pack_start(&browse_button, false, false, 0);

        grid.attach(&hotkey_label, 0, 0, 1, 1);
        grid.attach(&hotkey_entry, 1, 0, 1, 1);
        grid.attach(&microphone_label, 0, 1, 1, 1);
        grid.attach(&microphone_combo, 1, 1, 1, 1);
        grid.attach(&model_label, 0, 2, 1, 1);
        grid.attach(&model_row, 1, 2, 1, 1);

        let backend_label = left_label("Whisper backend");
        let backend_status_label = gtk::Label::new(None);
        backend_status_label.set_halign(gtk::Align::Start);
        backend_status_label.set_selectable(true);
        grid.attach(&backend_label, 0, 3, 1, 1);
        grid.attach(&backend_status_label, 1, 3, 1, 1);

        let status_label = gtk::Label::new(None);
        status_label.set_halign(gtk::Align::Start);

        let button_row = gtk::ButtonBox::new(gtk::Orientation::Horizontal);
        button_row.set_layout(gtk::ButtonBoxStyle::End);
        button_row.set_spacing(8);

        let save_button = gtk::Button::with_label("Save");
        let close_button = gtk::Button::with_label("Close");
        button_row.add(&close_button);
        button_row.add(&save_button);

        root.pack_start(&grid, false, false, 0);
        root.pack_start(&status_label, false, false, 0);
        root.pack_start(&button_row, false, false, 0);
        window.add(&root);

        let window_for_delete = window.clone();
        window.connect_delete_event(move |_, _| {
            window_for_delete.hide();
            gtk::glib::Propagation::Stop
        });

        let window_for_close = window.clone();
        close_button.connect_clicked(move |_| {
            window_for_close.hide();
        });

        let window_for_browse = window.clone();
        let model_entry_for_browse = model_entry.clone();
        browse_button.connect_clicked(move |_| {
            if let Some(path) = choose_model_file(&window_for_browse) {
                model_entry_for_browse.set_text(&path);
            }
        });

        let save_config = Rc::clone(&config);
        let save_store = store.clone();
        let save_hotkey_backend = Rc::clone(&hotkey_backend);
        let save_audio_recorder = Arc::clone(&audio_recorder);
        let save_whisper_recognizer = Arc::clone(&whisper_recognizer);
        let save_hotkey_entry = hotkey_entry.clone();
        let save_microphone_combo = microphone_combo.clone();
        let save_model_entry = model_entry.clone();
        let save_status_label = status_label.clone();
        let save_backend_status_label = backend_status_label.clone();
        save_button.connect_clicked(move |_| {
            let current_config = save_config.borrow().clone();
            let next_config = AppConfig {
                push_to_talk_hotkey: save_hotkey_entry.text().trim().to_string(),
                whisper_model: save_model_entry.text().trim().to_string(),
                microphone_device: selected_microphone_device(&save_microphone_combo),
                whisper_backend: current_config.whisper_backend,
                gpu_device: current_config.gpu_device,
            };

            match save_settings(
                &next_config,
                &save_store,
                &save_config,
                &save_hotkey_backend,
                &save_audio_recorder,
                &save_whisper_recognizer,
            ) {
                Ok(()) => {
                    save_status_label.set_text("Saved.");
                    save_backend_status_label
                        .set_text(&save_whisper_recognizer.runtime_status().summary());
                }
                Err(error) => {
                    save_status_label.set_text(&format!("Save failed: {error:#}"));
                    save_backend_status_label
                        .set_text(&save_whisper_recognizer.runtime_status().summary());
                }
            }
        });

        let settings_window = Self {
            window,
            hotkey_entry,
            microphone_combo,
            model_entry,
            backend_status_label,
            config,
            audio_recorder,
            whisper_recognizer,
        };
        settings_window.refresh_from_config();
        settings_window
    }

    pub fn present(&self) {
        self.refresh_from_config();
        self.window.show_all();
        self.window.present();
    }

    fn refresh_from_config(&self) {
        let config = self.config.borrow();
        self.hotkey_entry.set_text(&config.push_to_talk_hotkey);
        populate_microphone_combo(
            &self.microphone_combo,
            self.audio_recorder.as_ref(),
            config.microphone_device.as_deref(),
        );
        self.model_entry.set_text(&config.whisper_model);
        self.backend_status_label
            .set_text(&self.whisper_recognizer.runtime_status().summary());
    }
}

fn left_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_halign(gtk::Align::Start);
    label
}

fn choose_model_file(parent: &gtk::Window) -> Option<String> {
    let dialog = gtk::FileChooserDialog::builder()
        .title("Select Whisper model")
        .action(gtk::FileChooserAction::Open)
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_buttons(&[
        ("Cancel", gtk::ResponseType::Cancel),
        ("Select", gtk::ResponseType::Accept),
    ]);

    let response = dialog.run();
    let selected_path = if response == gtk::ResponseType::Accept {
        dialog
            .filename()
            .map(|path| path.to_string_lossy().into_owned())
    } else {
        None
    };

    dialog.close();
    selected_path
}

fn save_settings(
    next_config: &AppConfig,
    store: &ConfigStore,
    config: &Rc<RefCell<AppConfig>>,
    hotkey_backend: &Rc<dyn HotkeyBackend>,
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
    config.replace(next_config.clone());
    Ok(())
}

fn populate_microphone_combo(
    microphone_combo: &gtk::ComboBoxText,
    audio_recorder: &dyn AudioRecorder,
    selected_device_name: Option<&str>,
) {
    microphone_combo.remove_all();
    microphone_combo.append(Some(SYSTEM_DEFAULT_MIC_ID), "System default");

    let mut selected_device_is_available = selected_device_name.is_none();
    match audio_recorder.available_input_devices() {
        Ok(device_names) => {
            for device_name in device_names {
                if selected_device_name == Some(device_name.as_str()) {
                    selected_device_is_available = true;
                }
                microphone_combo.append(Some(&device_name), &device_name);
            }
        }
        Err(error) => {
            eprintln!("Failed to list microphones: {error:#}");
        }
    }

    if let Some(selected_device_name) = selected_device_name {
        if !selected_device_is_available {
            microphone_combo.append(
                Some(selected_device_name),
                &format!("{selected_device_name} (missing)"),
            );
        }
        microphone_combo.set_active_id(Some(selected_device_name));
    } else {
        microphone_combo.set_active_id(Some(SYSTEM_DEFAULT_MIC_ID));
    }
}

fn selected_microphone_device(microphone_combo: &gtk::ComboBoxText) -> Option<String> {
    microphone_combo
        .active_id()
        .map(|device_id| device_id.to_string())
        .filter(|device_id| device_id != SYSTEM_DEFAULT_MIC_ID)
        .filter(|device_id| !device_id.trim().is_empty())
}
