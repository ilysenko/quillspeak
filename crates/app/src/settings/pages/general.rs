use std::collections::HashSet;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::AppConfig;

use crate::audio::AudioInputDevice;
use crate::settings::SettingsDraft;
use crate::settings::pages::audio_input::audio_input_dropdown_row;
use crate::settings::pages::output_controls::add_default_output_controls;
use crate::settings::widgets::{
    advanced_hotkey_status, all_model_entries, backend_from_index, backend_index,
    compute_from_index, compute_index, dropdown_row, language_dropdown_row, model_dropdown_row,
    output_tools_status, property_row,
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
    ready_model_ids: HashSet<String>,
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
    let status_group = adw::PreferencesGroup::builder().title("Status").build();
    let advanced_hotkey_row = property_row("Advanced hotkeys", advanced_hotkey_status());
    status_group.add(&advanced_hotkey_row);
    let output_tools = output_tools_status();
    let output_tools_row = property_row("Output tools", &output_tools);
    status_group.add(&output_tools_row);

    let general_group = adw::PreferencesGroup::builder()
        .title("Configuration")
        .build();
    let backend = dropdown_row(
        "Hotkey backend",
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

    let input = audio_input_dropdown_row(&audio_input_devices, &config.general.default_input);
    input.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let values = input.values.clone();
        move |dropdown| {
            if let Some(input) = values.get(dropdown.selected() as usize) {
                draft.update(|config| {
                    config.general.default_input = input.clone();
                });
            }
        }
    });
    general_group.add(&input.row);

    let compute = dropdown_row(
        "Whisper compute",
        &["Auto", "CPU", "Vulkan", "CUDA", "ROCm", "OpenVINO"],
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

    let keep_model_loaded = gtk::Switch::builder()
        .active(config.general.keep_model_loaded)
        .valign(gtk::Align::Center)
        .build();
    let keep_model_loaded_row = adw::ActionRow::builder()
        .title("Keep model loaded")
        .subtitle("Keep the last used Whisper model in memory after transcription")
        .build();
    keep_model_loaded_row.add_suffix(&keep_model_loaded);
    keep_model_loaded_row.set_activatable_widget(Some(&keep_model_loaded));
    keep_model_loaded.connect_active_notify({
        let draft = draft.clone();
        move |switch| {
            draft.update(|config| {
                config.general.keep_model_loaded = switch.is_active();
            });
        }
    });
    general_group.add(&keep_model_loaded_row);

    let model_entries = all_model_entries()
        .into_iter()
        .filter(|entry| ready_model_ids.contains(entry.id))
        .collect::<Vec<_>>();
    let model = model_dropdown_row(
        "Default model",
        &model_entries,
        &config.general.default_model_id,
    );
    model.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let values = model.values.clone();
        move |dropdown| {
            if let Some(model_id) = values.get(dropdown.selected() as usize) {
                draft.update(|config| {
                    config.general.default_model_id = model_id.clone();
                });
            }
        }
    });
    general_group.add(&model.row);

    let language =
        language_dropdown_row("Default language", false, &config.general.default_language);
    language.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let values = language.values.clone();
        move |dropdown| {
            if let Some(language) = values.get(dropdown.selected() as usize) {
                draft.update(|config| {
                    config.general.default_language = language.clone();
                });
            }
        }
    });
    general_group.add(&language.row);

    let output_group = adw::PreferencesGroup::builder()
        .title("Default output")
        .build();
    add_default_output_controls(&output_group, &config.general.default_output, draft);

    content.append(&status_group);
    content.append(&general_group);
    content.append(&output_group);
    GeneralPage { page }
}
