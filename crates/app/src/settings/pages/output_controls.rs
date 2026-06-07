use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{OutputAction, ShortcutOutput};

use crate::settings::SettingsDraft;
use crate::settings::widgets::dropdown_row;

pub fn add_default_output_controls(
    group: &adw::PreferencesGroup,
    output: &OutputAction,
    draft: SettingsDraft,
) {
    let selected = match output {
        OutputAction::Clipboard => 0,
        OutputAction::Script { .. } => 1,
    };
    let output_row = dropdown_row("Action", &["Copy to clipboard", "Run script"], selected);
    let script_row = adw::EntryRow::builder()
        .title("Script path")
        .text(match output {
            OutputAction::Script { path } => path,
            OutputAction::Clipboard => "",
        })
        .sensitive(matches!(output, OutputAction::Script { .. }))
        .build();

    output_row.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let script_row = script_row.clone();
        move |dropdown| {
            let is_script = dropdown.selected() == 1;
            script_row.set_sensitive(is_script);
            draft.update(|config| {
                config.general.default_output = if is_script {
                    OutputAction::Script {
                        path: script_row.text().to_string(),
                    }
                } else {
                    OutputAction::Clipboard
                };
            });
        }
    });
    script_row.connect_changed(move |row| {
        draft.update(|config| {
            if matches!(config.general.default_output, OutputAction::Script { .. }) {
                config.general.default_output = OutputAction::Script {
                    path: row.text().to_string(),
                };
            }
        });
    });
    group.add(&output_row.row);
    group.add(&script_row);
}

pub fn add_shortcut_output_controls(
    group: &adw::PreferencesGroup,
    shortcut_id: &str,
    output: &ShortcutOutput,
    draft: SettingsDraft,
) {
    let selected = match output {
        ShortcutOutput::Default => 0,
        ShortcutOutput::Clipboard => 1,
        ShortcutOutput::Script { .. } => 2,
    };
    let output_row = dropdown_row(
        "Output",
        &["Default", "Copy to clipboard", "Run script"],
        selected,
    );
    let script_row = adw::EntryRow::builder()
        .title("Script path")
        .text(match output {
            ShortcutOutput::Script { path } => path,
            ShortcutOutput::Default | ShortcutOutput::Clipboard => "",
        })
        .sensitive(matches!(output, ShortcutOutput::Script { .. }))
        .build();

    let shortcut_id_for_dropdown = shortcut_id.to_string();
    output_row.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let script_row = script_row.clone();
        move |dropdown| {
            let is_script = dropdown.selected() == 2;
            script_row.set_sensitive(is_script);
            draft.update_shortcut(&shortcut_id_for_dropdown, |shortcut| {
                shortcut.output = match dropdown.selected() {
                    1 => ShortcutOutput::Clipboard,
                    2 => ShortcutOutput::Script {
                        path: script_row.text().to_string(),
                    },
                    _ => ShortcutOutput::Default,
                };
            });
        }
    });

    let shortcut_id_for_entry = shortcut_id.to_string();
    script_row.connect_changed(move |row| {
        draft.update_shortcut(&shortcut_id_for_entry, |shortcut| {
            if matches!(shortcut.output, ShortcutOutput::Script { .. }) {
                shortcut.output = ShortcutOutput::Script {
                    path: row.text().to_string(),
                };
            }
        });
    });
    group.add(&output_row.row);
    group.add(&script_row);
}
