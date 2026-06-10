use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::AppConfig;

use crate::audio::AudioInputDevice;
use crate::settings::SettingsDraft;
use crate::settings::pages::audio_input::audio_input_dropdown_row;
use crate::settings::widgets::{
    backend_from_index, backend_index, compute_from_index, compute_index, dropdown_row_with_help,
    switch_row,
};

const GENERAL_MAX_WIDTH: i32 = 740;
const GENERAL_TIGHTENING_WIDTH: i32 = 600;

#[derive(Clone)]
pub struct GeneralPage {
    page: adw::Clamp,
}

impl GeneralPage {
    pub fn widget(&self) -> &adw::Clamp {
        &self.page
    }
}

pub fn build(
    config: &AppConfig,
    audio_input_devices: Vec<AudioInputDevice>,
    draft: SettingsDraft,
) -> GeneralPage {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(28)
        .margin_bottom(28)
        .build();
    let page = adw::Clamp::builder()
        .maximum_size(GENERAL_MAX_WIDTH)
        .tightening_threshold(GENERAL_TIGHTENING_WIDTH)
        .hexpand(true)
        .vexpand(true)
        .valign(gtk::Align::Start)
        .child(&content)
        .build();
    let general_group = adw::PreferencesGroup::builder()
        .title("Configuration")
        .build();
    let backend = dropdown_row_with_help(
        "Hotkey backend",
        "Selects the in-app global shortcut backend. Auto uses X11 keyboard grabs only on pure X11 sessions; Linux signal shortcuts still work when this is disabled.",
        &["Auto", "Disabled", "X11"],
        backend_index(config.general.hotkey_backend),
    );
    backend.dropdown.connect_selected_notify({
        let draft = draft.clone();
        move |dropdown| {
            draft.update(|config| {
                config.general.hotkey_backend = backend_from_index(dropdown.selected());
            });
        }
    });
    general_group.add(&backend.row);

    let input = audio_input_dropdown_row(&audio_input_devices, &config.general.audio_input);
    input.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let values = input.values.clone();
        move |dropdown| {
            if let Some(input) = values.get(dropdown.selected() as usize) {
                draft.update(|config| {
                    config.general.audio_input = input.clone();
                });
            }
        }
    });
    general_group.add(&input.row);

    let compute = dropdown_row_with_help(
        "Whisper compute",
        "Chooses how Whisper initializes inference. Auto tries a compiled GPU backend when available and falls back to CPU if auto GPU initialization fails.",
        &["Auto", "CPU", "Vulkan", "CUDA", "ROCm"],
        compute_index(config.general.compute_backend),
    );
    compute.dropdown.connect_selected_notify({
        let draft = draft.clone();
        move |dropdown| {
            draft.update(|config| {
                config.general.compute_backend = compute_from_index(dropdown.selected());
            });
        }
    });
    general_group.add(&compute.row);

    let (keep_model_loaded_row, keep_model_loaded) = switch_row(
        "Keep model loaded",
        "Keeps the last used Whisper model context in memory after a transcription. This makes the next run faster but uses more RAM or GPU memory.",
        config.general.keep_model_loaded,
    );
    keep_model_loaded.connect_active_notify({
        let draft = draft.clone();
        move |switch| {
            draft.update(|config| {
                config.general.keep_model_loaded = switch.is_active();
            });
        }
    });
    general_group.add(&keep_model_loaded_row);

    content.append(&general_group);
    GeneralPage { page }
}
