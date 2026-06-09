use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{OutputAction, PasteShortcut, ScriptOutput, ShortcutOutput};

use crate::settings::SettingsDraft;
use crate::settings::widgets::dropdown_row;

pub fn add_default_output_controls(
    group: &adw::PreferencesGroup,
    output: &OutputAction,
    draft: SettingsDraft,
) {
    let controls = OutputControls::new(output, true);

    controls.run_script_switch.connect_active_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |switch| {
            controls.set_script_sensitive(switch.is_active(), true);
            draft.update(|config| {
                config.general.default_output.script = if switch.is_active() {
                    Some(controls.script_from_rows())
                } else {
                    None
                };
            });
        }
    });

    controls.script_row.connect_changed({
        let draft = draft.clone();
        move |row| {
            draft.update(|config| {
                if let Some(script) = &mut config.general.default_output.script {
                    script.path = row.text().to_string();
                }
            });
        }
    });

    controls.copy_switch.connect_active_notify({
        let draft = draft.clone();
        move |switch| {
            draft.update(|config| {
                config.general.default_output.copy_to_clipboard = switch.is_active();
            });
        }
    });

    controls.paste_switch.connect_active_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |switch| {
            controls.set_paste_sensitive(switch.is_active(), true);
            draft.update(|config| {
                config.general.default_output.paste_from_clipboard = switch.is_active();
            });
        }
    });

    controls.paste_shortcut.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |dropdown| {
            controls.set_paste_sensitive(controls.paste_switch.is_active(), true);
            draft.update(|config| {
                config.general.default_output.paste_shortcut =
                    paste_shortcut_from_index(dropdown.selected());
            });
        }
    });

    controls.paste_custom_x11_row.connect_changed({
        let draft = draft.clone();
        move |row| {
            draft.update(|config| {
                config.general.default_output.paste_custom_x11 = row.text().to_string();
            });
        }
    });

    controls.paste_custom_wayland_row.connect_changed({
        let draft = draft.clone();
        move |row| {
            draft.update(|config| {
                config.general.default_output.paste_custom_wayland = row.text().to_string();
            });
        }
    });

    controls.add_to_group(group);
}

pub fn add_shortcut_output_controls(
    group: &adw::PreferencesGroup,
    shortcut_id: &str,
    output: &ShortcutOutput,
    draft: SettingsDraft,
) {
    let (selected, action) = match output {
        ShortcutOutput::Default => (0, OutputAction::default()),
        ShortcutOutput::Custom { action } => (1, action.clone()),
    };
    let output_row = dropdown_row("Output", &["Default", "Custom"], selected);
    let controls = OutputControls::new(&action, selected == 1);
    let shortcut_id_for_dropdown = shortcut_id.to_string();
    output_row.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |dropdown| {
            let is_custom = dropdown.selected() == 1;
            controls.set_all_sensitive(is_custom);
            draft.update_shortcut(&shortcut_id_for_dropdown, |shortcut| {
                shortcut.output = if is_custom {
                    ShortcutOutput::custom(controls.action_from_rows())
                } else {
                    ShortcutOutput::Default
                };
            });
        }
    });

    let shortcut_id_for_run = shortcut_id.to_string();
    controls.run_script_switch.connect_active_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |switch| {
            controls.set_script_sensitive(switch.is_active(), controls.is_custom_sensitive());
            draft.update_shortcut(&shortcut_id_for_run, |shortcut| {
                if let ShortcutOutput::Custom { action } = &mut shortcut.output {
                    action.script = if switch.is_active() {
                        Some(controls.script_from_rows())
                    } else {
                        None
                    };
                }
            });
        }
    });

    let shortcut_id_for_path = shortcut_id.to_string();
    controls.script_row.connect_changed({
        let draft = draft.clone();
        move |row| {
            draft.update_shortcut(&shortcut_id_for_path, |shortcut| {
                if let ShortcutOutput::Custom { action } = &mut shortcut.output
                    && let Some(script) = &mut action.script
                {
                    script.path = row.text().to_string();
                }
            });
        }
    });

    let shortcut_id_for_copy = shortcut_id.to_string();
    controls.copy_switch.connect_active_notify({
        let draft = draft.clone();
        move |switch| {
            draft.update_shortcut(&shortcut_id_for_copy, |shortcut| {
                if let ShortcutOutput::Custom { action } = &mut shortcut.output {
                    action.copy_to_clipboard = switch.is_active();
                }
            });
        }
    });

    let shortcut_id_for_paste = shortcut_id.to_string();
    controls.paste_switch.connect_active_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |switch| {
            controls.set_paste_sensitive(switch.is_active(), controls.is_custom_sensitive());
            draft.update_shortcut(&shortcut_id_for_paste, |shortcut| {
                if let ShortcutOutput::Custom { action } = &mut shortcut.output {
                    action.paste_from_clipboard = switch.is_active();
                }
            });
        }
    });

    let shortcut_id_for_paste_shortcut = shortcut_id.to_string();
    controls.paste_shortcut.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |dropdown| {
            controls.set_paste_sensitive(
                controls.paste_switch.is_active(),
                controls.is_custom_sensitive(),
            );
            draft.update_shortcut(&shortcut_id_for_paste_shortcut, |shortcut| {
                if let ShortcutOutput::Custom { action } = &mut shortcut.output {
                    action.paste_shortcut = paste_shortcut_from_index(dropdown.selected());
                }
            });
        }
    });

    let shortcut_id_for_paste_x11 = shortcut_id.to_string();
    controls.paste_custom_x11_row.connect_changed({
        let draft = draft.clone();
        move |row| {
            draft.update_shortcut(&shortcut_id_for_paste_x11, |shortcut| {
                if let ShortcutOutput::Custom { action } = &mut shortcut.output {
                    action.paste_custom_x11 = row.text().to_string();
                }
            });
        }
    });

    let shortcut_id_for_paste_wayland = shortcut_id.to_string();
    controls.paste_custom_wayland_row.connect_changed({
        let draft = draft.clone();
        move |row| {
            draft.update_shortcut(&shortcut_id_for_paste_wayland, |shortcut| {
                if let ShortcutOutput::Custom { action } = &mut shortcut.output {
                    action.paste_custom_wayland = row.text().to_string();
                }
            });
        }
    });

    group.add(&output_row.row);
    controls.add_to_group(group);
}

#[derive(Clone)]
struct OutputControls {
    run_script_row: adw::ActionRow,
    run_script_switch: gtk::Switch,
    script_row: adw::EntryRow,
    copy_row: adw::ActionRow,
    copy_switch: gtk::Switch,
    paste_row: adw::ActionRow,
    paste_switch: gtk::Switch,
    paste_shortcut: crate::settings::widgets::DropDownRow,
    paste_custom_x11_row: adw::EntryRow,
    paste_custom_wayland_row: adw::EntryRow,
}

impl OutputControls {
    fn new(output: &OutputAction, custom_sensitive: bool) -> Self {
        let script = output.script.as_ref();
        let has_script = script.is_some();
        let run_script_switch = gtk::Switch::builder()
            .active(has_script)
            .valign(gtk::Align::Center)
            .sensitive(custom_sensitive)
            .build();
        let run_script_row = adw::ActionRow::builder()
            .title("Run script")
            .sensitive(custom_sensitive)
            .build();
        run_script_row.add_suffix(&run_script_switch);
        run_script_row.set_activatable_widget(Some(&run_script_switch));

        let script_row = adw::EntryRow::builder()
            .title("Script path")
            .text(script.map(|script| script.path.as_str()).unwrap_or(""))
            .sensitive(custom_sensitive && has_script)
            .build();

        let copy_switch = gtk::Switch::builder()
            .active(output.copy_to_clipboard)
            .valign(gtk::Align::Center)
            .sensitive(custom_sensitive)
            .build();
        let copy_row = adw::ActionRow::builder()
            .title("Copy to clipboard")
            .sensitive(custom_sensitive)
            .build();
        copy_row.add_suffix(&copy_switch);
        copy_row.set_activatable_widget(Some(&copy_switch));

        let paste_switch = gtk::Switch::builder()
            .active(output.paste_from_clipboard)
            .valign(gtk::Align::Center)
            .sensitive(custom_sensitive)
            .build();
        let paste_row = adw::ActionRow::builder()
            .title("Paste from clipboard")
            .sensitive(custom_sensitive)
            .build();
        paste_row.add_suffix(&paste_switch);
        paste_row.set_activatable_widget(Some(&paste_switch));

        let paste_shortcut = dropdown_row(
            "Paste shortcut",
            &["Ctrl+V", "Ctrl+Shift+V", "Custom"],
            paste_shortcut_index(output.paste_shortcut),
        );
        paste_shortcut
            .row
            .set_sensitive(custom_sensitive && output.paste_from_clipboard);

        let paste_custom_x11_row = adw::EntryRow::builder()
            .title("X11 xdotool keys")
            .text(&output.paste_custom_x11)
            .build();
        let paste_custom_wayland_row = adw::EntryRow::builder()
            .title("Wayland ydotool keys")
            .text(&output.paste_custom_wayland)
            .build();

        Self {
            run_script_row,
            run_script_switch,
            script_row,
            copy_row,
            copy_switch,
            paste_row,
            paste_switch,
            paste_shortcut,
            paste_custom_x11_row,
            paste_custom_wayland_row,
        }
        .with_paste_visibility(custom_sensitive)
    }

    fn add_to_group(&self, group: &adw::PreferencesGroup) {
        group.add(&self.run_script_row);
        group.add(&self.script_row);
        group.add(&self.copy_row);
        group.add(&self.paste_row);
        group.add(&self.paste_shortcut.row);
        group.add(&self.paste_custom_x11_row);
        group.add(&self.paste_custom_wayland_row);
    }

    fn action_from_rows(&self) -> OutputAction {
        OutputAction {
            copy_to_clipboard: self.copy_switch.is_active(),
            paste_from_clipboard: self.paste_switch.is_active(),
            paste_shortcut: paste_shortcut_from_index(self.paste_shortcut.dropdown.selected()),
            paste_custom_x11: self.paste_custom_x11_row.text().to_string(),
            paste_custom_wayland: self.paste_custom_wayland_row.text().to_string(),
            script: self
                .run_script_switch
                .is_active()
                .then(|| self.script_from_rows()),
        }
    }

    fn script_from_rows(&self) -> ScriptOutput {
        ScriptOutput {
            path: self.script_row.text().to_string(),
        }
    }

    fn set_all_sensitive(&self, sensitive: bool) {
        self.run_script_row.set_sensitive(sensitive);
        self.run_script_switch.set_sensitive(sensitive);
        self.copy_row.set_sensitive(sensitive);
        self.copy_switch.set_sensitive(sensitive);
        self.paste_row.set_sensitive(sensitive);
        self.paste_switch.set_sensitive(sensitive);
        self.set_script_sensitive(self.run_script_switch.is_active(), sensitive);
        self.set_paste_sensitive(self.paste_switch.is_active(), sensitive);
    }

    fn set_script_sensitive(&self, has_script: bool, parent_sensitive: bool) {
        self.script_row
            .set_sensitive(parent_sensitive && has_script);
    }

    fn is_custom_sensitive(&self) -> bool {
        self.run_script_row.is_sensitive()
    }

    fn set_paste_sensitive(&self, paste_enabled: bool, parent_sensitive: bool) {
        let custom = paste_shortcut_from_index(self.paste_shortcut.dropdown.selected())
            == PasteShortcut::Custom;
        self.paste_shortcut
            .row
            .set_sensitive(parent_sensitive && paste_enabled);
        self.paste_custom_x11_row
            .set_visible(paste_enabled && custom);
        self.paste_custom_wayland_row
            .set_visible(paste_enabled && custom);
        self.paste_custom_x11_row
            .set_sensitive(parent_sensitive && paste_enabled && custom);
        self.paste_custom_wayland_row
            .set_sensitive(parent_sensitive && paste_enabled && custom);
    }

    fn with_paste_visibility(self, parent_sensitive: bool) -> Self {
        self.set_paste_sensitive(self.paste_switch.is_active(), parent_sensitive);
        self
    }
}

fn paste_shortcut_index(shortcut: PasteShortcut) -> u32 {
    match shortcut {
        PasteShortcut::CtrlV => 0,
        PasteShortcut::CtrlShiftV => 1,
        PasteShortcut::Custom => 2,
    }
}

fn paste_shortcut_from_index(index: u32) -> PasteShortcut {
    match index {
        1 => PasteShortcut::CtrlShiftV,
        2 => PasteShortcut::Custom,
        _ => PasteShortcut::CtrlV,
    }
}
