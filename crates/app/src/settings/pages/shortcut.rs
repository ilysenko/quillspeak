use std::collections::HashSet;
use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{
    DEFAULT_SHORTCUT_ID, LinuxSignal, MODEL_CATALOG, SUPPORTED_LINUX_SIGNALS, ShortcutProfile,
    ShortcutTrigger, model_catalog_entry,
};

use crate::hotkey::ShortcutTriggerCapabilities;
use crate::settings::SettingsDraft;
use crate::settings::pages::output_controls::add_shortcut_output_controls;
use crate::settings::shortcut_recorder::connect_record_button;
use crate::settings::widgets::{
    dropdown_row_with_help, language_dropdown_row, preferences_page, shortcut_model_dropdown_row,
    switch_row, text_row, value_dropdown_row,
};

pub fn build(
    shortcut: &ShortcutProfile,
    ready_model_ids: HashSet<String>,
    draft: SettingsDraft,
    capabilities: ShortcutTriggerCapabilities,
    request_render: Rc<dyn Fn(Option<String>)>,
) -> adw::PreferencesPage {
    let shortcut_id = shortcut.id.clone();
    let page = preferences_page(&shortcut.name);
    let group = adw::PreferencesGroup::builder().title("Shortcut").build();
    let keyboard_available = capabilities.keyboard_available();

    let name_row = text_row(
        "Name",
        "Display name shown in the Settings sidebar and logs for this shortcut profile.",
        &shortcut.name,
    );
    name_row
        .row
        .set_sensitive(shortcut.id != DEFAULT_SHORTCUT_ID);
    name_row.entry.connect_changed({
        let draft = draft.clone();
        let shortcut_id = shortcut_id.clone();
        move |entry| {
            draft.update_shortcut(&shortcut_id, |shortcut| {
                shortcut.name = entry.text().to_string();
            });
        }
    });
    group.add(&name_row.row);

    let (enabled_row, enabled_switch) = switch_row(
        "Enabled",
        "Controls whether this shortcut profile can start recordings. Disabled profiles remain saved but ignore their trigger.",
        shortcut.enabled,
    );
    enabled_switch.connect_active_notify({
        let draft = draft.clone();
        let shortcut_id = shortcut_id.clone();
        move |switch| {
            draft.update_shortcut(&shortcut_id, |shortcut| {
                shortcut.enabled = switch.is_active();
            });
        }
    });
    group.add(&enabled_row);

    let trigger_is_keyboard =
        keyboard_available && matches!(shortcut.trigger, ShortcutTrigger::Keyboard { .. });
    let trigger = dropdown_row_with_help(
        "Trigger",
        "Selects how this shortcut starts and stops recording. Keyboard is available only on X11; Linux signal works with external Wayland shortcut tools.",
        &["Keyboard shortcut", "Linux signal"],
        if trigger_is_keyboard { 0 } else { 1 },
    );
    if keyboard_available {
        group.add(&trigger.row);
    }

    let shortcut_entry = text_row(
        "Shortcut",
        "Keyboard chord captured by the X11 backend, for example Ctrl+Alt+Space. Empty shortcuts are automatically disabled.",
        shortcut_keyboard_accelerator(shortcut),
    );
    shortcut_entry.row.set_visible(trigger_is_keyboard);
    shortcut_entry.entry.connect_changed({
        let draft = draft.clone();
        let shortcut_id = shortcut_id.clone();
        let enabled_switch = enabled_switch.clone();
        move |entry| {
            let enabled = !entry.text().trim().is_empty();
            enabled_switch.set_active(enabled);
            draft.update_shortcut(&shortcut_id, |shortcut| {
                shortcut.trigger = ShortcutTrigger::Keyboard {
                    accelerator: entry.text().to_string(),
                };
                shortcut.enabled = enabled;
            });
        }
    });
    let record_button = gtk::Button::builder()
        .label("Record")
        .tooltip_text("Record shortcut")
        .valign(gtk::Align::Center)
        .build();
    shortcut_entry.row.add_suffix(&record_button);
    connect_record_button(&record_button, &shortcut_entry.entry);
    group.add(&shortcut_entry.row);

    let (start_signal, stop_signal) = shortcut_signal_pair(shortcut);
    let start_signal_row = signal_dropdown_row("Start signal", &start_signal, !trigger_is_keyboard);
    let stop_signal_row = signal_dropdown_row("Stop signal", &stop_signal, !trigger_is_keyboard);
    start_signal_row.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let shortcut_id = shortcut_id.clone();
        let start_values = start_signal_row.values.clone();
        let stop_values = stop_signal_row.values.clone();
        let stop_dropdown = stop_signal_row.dropdown.clone();
        let enabled_switch = enabled_switch.clone();
        move |dropdown| {
            enabled_switch.set_active(true);
            if let (Some(start_signal), Some(stop_signal)) = (
                start_values.get(dropdown.selected() as usize),
                stop_values.get(stop_dropdown.selected() as usize),
            ) {
                draft.update_shortcut(&shortcut_id, |shortcut| {
                    shortcut.trigger = ShortcutTrigger::LinuxSignal {
                        start_signal: LinuxSignal::new(start_signal.clone()),
                        stop_signal: LinuxSignal::new(stop_signal.clone()),
                    };
                    shortcut.enabled = true;
                });
            }
        }
    });
    stop_signal_row.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let shortcut_id = shortcut_id.clone();
        let start_values = start_signal_row.values.clone();
        let stop_values = stop_signal_row.values.clone();
        let start_dropdown = start_signal_row.dropdown.clone();
        let enabled_switch = enabled_switch.clone();
        move |dropdown| {
            enabled_switch.set_active(true);
            if let (Some(start_signal), Some(stop_signal)) = (
                start_values.get(start_dropdown.selected() as usize),
                stop_values.get(dropdown.selected() as usize),
            ) {
                draft.update_shortcut(&shortcut_id, |shortcut| {
                    shortcut.trigger = ShortcutTrigger::LinuxSignal {
                        start_signal: LinuxSignal::new(start_signal.clone()),
                        stop_signal: LinuxSignal::new(stop_signal.clone()),
                    };
                    shortcut.enabled = true;
                });
            }
        }
    });
    group.add(&start_signal_row.row);
    group.add(&stop_signal_row.row);

    trigger.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let shortcut_id = shortcut_id.clone();
        let shortcut_entry = shortcut_entry.clone();
        let start_signal_row = start_signal_row.clone();
        let stop_signal_row = stop_signal_row.clone();
        let enabled_switch = enabled_switch.clone();
        move |dropdown| {
            let is_keyboard = dropdown.selected() == 0;
            let enabled =
                !is_keyboard || !shortcut_entry.entry.text().to_string().trim().is_empty();
            shortcut_entry.row.set_visible(is_keyboard);
            start_signal_row.row.set_visible(!is_keyboard);
            stop_signal_row.row.set_visible(!is_keyboard);
            if !is_keyboard {
                start_signal_row.dropdown.set_selected(0);
                stop_signal_row.dropdown.set_selected(1);
            }
            enabled_switch.set_active(enabled);
            draft.update_shortcut(&shortcut_id, |shortcut| {
                shortcut.trigger = if is_keyboard {
                    ShortcutTrigger::Keyboard {
                        accelerator: shortcut_entry.entry.text().to_string(),
                    }
                } else {
                    ShortcutTrigger::default_linux_signal()
                };
                shortcut.enabled = enabled;
            });
        }
    });

    let ready_entries = MODEL_CATALOG
        .iter()
        .filter_map(|entry| model_catalog_entry(entry.id))
        .filter(|entry| ready_model_ids.contains(entry.id))
        .collect::<Vec<_>>();
    let model = shortcut_model_dropdown_row(
        "Model",
        "Downloaded Whisper model used by this shortcut. Only ready models are offered; a missing marker means the configured model is not currently installed.",
        &ready_entries,
        &shortcut.model_id,
    );
    model.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let shortcut_id = shortcut_id.clone();
        let values = model.values.clone();
        move |dropdown| {
            if let Some(model_id) = values.get(dropdown.selected() as usize) {
                draft.update_shortcut(&shortcut_id, |shortcut| {
                    shortcut.model_id = model_id.clone();
                });
            }
        }
    });
    group.add(&model.row);

    let language = language_dropdown_row(
        "Language",
        "Language hint passed to Whisper. Auto Detect lets Whisper choose the spoken language for this shortcut.",
        &shortcut.language,
    );
    language.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let shortcut_id = shortcut_id.clone();
        let values = language.values.clone();
        move |dropdown| {
            if let Some(language) = values.get(dropdown.selected() as usize) {
                draft.update_shortcut(&shortcut_id, |shortcut| {
                    shortcut.language = language.clone();
                });
            }
        }
    });
    group.add(&language.row);

    let (mute_output_row, mute_output_switch) = switch_row(
        "Mute speakers while recording",
        "Temporarily mutes the default system output while this shortcut is actively recording, then restores the previous mute state.",
        shortcut.mute_output_while_recording,
    );
    mute_output_switch.connect_active_notify({
        let draft = draft.clone();
        let shortcut_id = shortcut_id.clone();
        move |switch| {
            draft.update_shortcut(&shortcut_id, |shortcut| {
                shortcut.mute_output_while_recording = switch.is_active();
            });
        }
    });
    group.add(&mute_output_row);

    add_shortcut_output_controls(&group, &shortcut_id, &shortcut.output, draft.clone());

    if shortcut.id != DEFAULT_SHORTCUT_ID {
        let delete_row = adw::ActionRow::builder()
            .title("Delete shortcut")
            .subtitle("Remove this shortcut profile")
            .build();
        let delete_button = gtk::Button::builder()
            .label("Delete")
            .valign(gtk::Align::Center)
            .build();
        delete_button.add_css_class("destructive-action");
        delete_button.connect_clicked({
            let draft = draft.clone();
            let shortcut_id = shortcut_id.clone();
            move |_| {
                draft.remove_shortcut(&shortcut_id);
                request_render(Some("general".to_string()));
            }
        });
        delete_row.add_suffix(&delete_button);
        group.add(&delete_row);
    }

    page.add(&group);
    page
}

fn shortcut_keyboard_accelerator(shortcut: &ShortcutProfile) -> &str {
    shortcut
        .trigger
        .keyboard_accelerator()
        .unwrap_or(if shortcut.id == DEFAULT_SHORTCUT_ID {
            "Ctrl+Alt+Space"
        } else {
            ""
        })
}

fn shortcut_signal_pair(shortcut: &ShortcutProfile) -> (LinuxSignal, LinuxSignal) {
    match &shortcut.trigger {
        ShortcutTrigger::LinuxSignal {
            start_signal,
            stop_signal,
        } => (start_signal.clone(), stop_signal.clone()),
        ShortcutTrigger::Keyboard { .. } => (LinuxSignal::sigusr1(), LinuxSignal::sigusr2()),
    }
}

fn signal_dropdown_row(
    title: &str,
    selected_signal: &LinuxSignal,
    visible: bool,
) -> crate::settings::widgets::ValueDropDownRow {
    let labels = SUPPORTED_LINUX_SIGNALS
        .iter()
        .map(|signal| signal.label.to_string())
        .collect::<Vec<_>>();
    let values = SUPPORTED_LINUX_SIGNALS
        .iter()
        .map(|signal| signal.name.to_string())
        .collect::<Vec<_>>();
    let row = value_dropdown_row(
        title,
        "Linux signal sent by an external shortcut tool. Only the listed exact signal names are supported; using the same signal for start and stop toggles recording.",
        labels,
        values,
        selected_signal.as_str(),
    );
    row.row.set_visible(visible);
    row
}
