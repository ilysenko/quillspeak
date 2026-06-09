use std::collections::HashSet;
use std::rc::Rc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::hotkey::ShortcutTriggerCapabilities;
use crate::settings::widgets::preferences_page;
use crate::settings::{SettingsDraft, shortcut_page_name};

pub fn build(
    draft: SettingsDraft,
    _ready_model_ids: HashSet<String>,
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
        let shortcut = draft.add_shortcut(capabilities);
        request_render(Some(shortcut_page_name(&shortcut.id)));
    });
    row.add_suffix(&button);
    group.add(&row);
    page.add(&group);
    page
}
