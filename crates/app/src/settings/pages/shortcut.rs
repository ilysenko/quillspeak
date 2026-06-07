use std::collections::HashSet;
use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{DEFAULT_SHORTCUT_ID, MODEL_CATALOG, ShortcutProfile, model_catalog_entry};

use crate::settings::SettingsDraft;
use crate::settings::pages::output_controls::add_shortcut_output_controls;
use crate::settings::shortcut_recorder::connect_record_button;
use crate::settings::widgets::{
    language_dropdown_row, preferences_page, shortcut_model_dropdown_row,
};

pub fn build(
    shortcut: &ShortcutProfile,
    ready_model_ids: HashSet<String>,
    draft: SettingsDraft,
    request_render: Rc<dyn Fn(Option<String>)>,
) -> adw::PreferencesPage {
    let shortcut_id = shortcut.id.clone();
    let page = preferences_page(&shortcut.name);
    let group = adw::PreferencesGroup::builder().title("Shortcut").build();

    let name_row = adw::EntryRow::builder()
        .title("Name")
        .text(&shortcut.name)
        .build();
    name_row.set_sensitive(shortcut.id != DEFAULT_SHORTCUT_ID);
    name_row.connect_changed({
        let draft = draft.clone();
        let shortcut_id = shortcut_id.clone();
        move |row| {
            draft.update_shortcut(&shortcut_id, |shortcut| {
                shortcut.name = row.text().to_string();
            });
        }
    });
    group.add(&name_row);

    let shortcut_entry = adw::EntryRow::builder()
        .title("Shortcut")
        .text(&shortcut.accelerator)
        .build();
    shortcut_entry.connect_changed({
        let draft = draft.clone();
        let shortcut_id = shortcut_id.clone();
        move |row| {
            draft.update_shortcut(&shortcut_id, |shortcut| {
                shortcut.accelerator = row.text().to_string();
                shortcut.enabled = !shortcut.accelerator.trim().is_empty();
            });
        }
    });
    let record_button = gtk::Button::builder()
        .label("Record")
        .tooltip_text("Record shortcut")
        .valign(gtk::Align::Center)
        .build();
    shortcut_entry.add_suffix(&record_button);
    connect_record_button(&record_button, &shortcut_entry);
    group.add(&shortcut_entry);

    let ready_entries = MODEL_CATALOG
        .iter()
        .filter_map(|entry| model_catalog_entry(entry.id))
        .filter(|entry| ready_model_ids.contains(entry.id))
        .collect::<Vec<_>>();
    let model = shortcut_model_dropdown_row("Model", &ready_entries, &shortcut.model_id);
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

    let language = language_dropdown_row("Language", true, &shortcut.language);
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
