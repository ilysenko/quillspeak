use std::collections::HashSet;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{AppConfig, DaemonStatus};

use crate::audio::AudioInputDevice;
use crate::settings::SettingsDraft;
use crate::settings::pages::audio_input::audio_input_dropdown_row;
use crate::settings::pages::output_controls::add_default_output_controls;
use crate::settings::widgets::{
    advanced_hotkey_status, all_model_entries, backend_from_index, backend_index,
    compute_from_index, compute_index, dropdown_row, language_dropdown_row, model_dropdown_row,
    preferences_page, property_row,
};

#[derive(Clone)]
pub struct GeneralPage {
    page: adw::PreferencesPage,
    daemon_status_row: adw::ActionRow,
    advanced_hotkey_row: adw::ActionRow,
}

impl GeneralPage {
    pub fn widget(&self) -> &adw::PreferencesPage {
        &self.page
    }

    pub fn update_daemon_status(&self, daemon_status: DaemonStatus) {
        self.daemon_status_row
            .set_subtitle(daemon_status.display_label());
        self.advanced_hotkey_row
            .set_subtitle(advanced_hotkey_status(daemon_status));
    }
}

pub fn build(
    config: &AppConfig,
    audio_input_devices: Vec<AudioInputDevice>,
    ready_model_ids: HashSet<String>,
    daemon_status: DaemonStatus,
    draft: SettingsDraft,
) -> GeneralPage {
    let page = preferences_page("General");
    let status_group = adw::PreferencesGroup::builder().title("Status").build();
    let daemon_status_row = property_row("Daemon status", daemon_status.display_label());
    status_group.add(&daemon_status_row);
    let advanced_hotkey_row =
        property_row("Advanced hotkeys", advanced_hotkey_status(daemon_status));
    status_group.add(&advanced_hotkey_row);

    let general_group = adw::PreferencesGroup::builder()
        .title("Configuration")
        .build();
    let backend = dropdown_row(
        "Hotkey backend",
        &["Auto", "Disabled", "X11", "Daemon"],
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

    page.add(&status_group);
    page.add(&general_group);
    page.add(&output_group);
    GeneralPage {
        page,
        daemon_status_row,
        advanced_hotkey_row,
    }
}
