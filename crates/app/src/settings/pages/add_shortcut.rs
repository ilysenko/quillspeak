use std::collections::HashSet;
use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{DEFAULT_MODEL_ID, MODEL_CATALOG};

use crate::hotkey::ShortcutTriggerCapabilities;
use crate::settings::widgets::preferences_page;
use crate::settings::{SettingsDraft, shortcut_page_name};

pub fn build(
    draft: SettingsDraft,
    ready_model_ids: HashSet<String>,
    capabilities: ShortcutTriggerCapabilities,
    request_render: Rc<dyn Fn(Option<String>)>,
) -> adw::PreferencesPage {
    let page = preferences_page("Add New");
    let group = adw::PreferencesGroup::builder().title("Shortcuts").build();
    let row = adw::ActionRow::builder()
        .title("Add New")
        .subtitle("Create a shortcut profile")
        .build();
    let button = gtk::Button::builder()
        .label("Add")
        .valign(gtk::Align::Center)
        .build();
    button.connect_clicked(move |_| {
        let shortcut =
            draft.add_shortcut(capabilities, default_new_shortcut_model(&ready_model_ids));
        request_render(Some(shortcut_page_name(&shortcut.id)));
    });
    row.add_suffix(&button);
    group.add(&row);
    page.add(&group);
    page
}

fn default_new_shortcut_model(ready_model_ids: &HashSet<String>) -> String {
    MODEL_CATALOG
        .iter()
        .find(|entry| ready_model_ids.contains(entry.id))
        .map(|entry| entry.id.to_string())
        .unwrap_or_else(|| DEFAULT_MODEL_ID.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_new_shortcut_model_uses_first_ready_catalog_model() {
        let ready_model_ids = HashSet::from(["small-q8_0".to_string(), "tiny".to_string()]);

        assert_eq!(default_new_shortcut_model(&ready_model_ids), "tiny");
    }

    #[test]
    fn default_new_shortcut_model_falls_back_to_builtin_default() {
        assert_eq!(
            default_new_shortcut_model(&HashSet::new()),
            DEFAULT_MODEL_ID
        );
    }
}
