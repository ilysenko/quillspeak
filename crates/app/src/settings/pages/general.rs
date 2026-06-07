use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{AppConfig, DaemonStatus};

use crate::settings::SettingsDraft;
use crate::settings::pages::output_controls::add_default_output_controls;
use crate::settings::widgets::{
    advanced_hotkey_status, all_model_entries, backend_from_index, backend_index,
    compute_from_index, compute_index, dropdown_row, language_dropdown_row, model_dropdown_row,
    preferences_page, property_row,
};

pub fn build(
    config: &AppConfig,
    daemon_status: DaemonStatus,
    draft: SettingsDraft,
) -> adw::PreferencesPage {
    let page = preferences_page("General");
    let status_group = adw::PreferencesGroup::builder().title("Status").build();
    status_group.add(&property_row(
        "Daemon status",
        daemon_status.display_label(),
    ));
    status_group.add(&property_row(
        "Advanced hotkeys",
        advanced_hotkey_status(daemon_status),
    ));

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

    let model_entries = all_model_entries();
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
    page
}
