use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{OutputAction, PasteOutput, PasteShortcut, ScriptOutput, ShortcutOutput};

use crate::settings::SettingsDraft;
use crate::settings::widgets::{DropDownRow, dropdown_row};

pub fn add_default_output_controls(
    group: &adw::PreferencesGroup,
    output: &OutputAction,
    draft: SettingsDraft,
) {
    let controls = OutputControls::new(output, true);

    controls.copy_switch.connect_active_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |switch| {
            if !switch.is_active() {
                controls.paste_switch.set_active(false);
            }
            controls.set_paste_sensitive(true);
            draft.update(|config| {
                config.general.default_output.copy_to_clipboard = switch.is_active();
                if !switch.is_active() {
                    config.general.default_output.paste = None;
                }
            });
        }
    });

    controls.paste_switch.connect_active_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |switch| {
            controls.set_paste_sensitive(true);
            draft.update(|config| {
                config.general.default_output.paste =
                    if switch.is_active() && config.general.default_output.copy_to_clipboard {
                        Some(controls.paste_from_rows())
                    } else {
                        None
                    };
            });
        }
    });

    controls
        .paste_shortcut_row
        .dropdown
        .connect_selected_notify({
            let draft = draft.clone();
            let controls = controls.clone();
            move |_| {
                draft.update(|config| {
                    if let Some(paste) = &mut config.general.default_output.paste {
                        paste.shortcut = controls.paste_shortcut_from_rows();
                    }
                });
            }
        });

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

    controls
        .copy_stdout_switch
        .connect_active_notify(move |switch| {
            draft.update(|config| {
                if let Some(script) = &mut config.general.default_output.script {
                    script.copy_stdout_to_clipboard = switch.is_active();
                }
            });
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

    let shortcut_id_for_copy = shortcut_id.to_string();
    controls.copy_switch.connect_active_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |switch| {
            if !switch.is_active() {
                controls.paste_switch.set_active(false);
            }
            controls.set_paste_sensitive(controls.is_custom_sensitive());
            draft.update_shortcut(&shortcut_id_for_copy, |shortcut| {
                if let ShortcutOutput::Custom { action } = &mut shortcut.output {
                    action.copy_to_clipboard = switch.is_active();
                    if !switch.is_active() {
                        action.paste = None;
                    }
                }
            });
        }
    });

    let shortcut_id_for_paste = shortcut_id.to_string();
    controls.paste_switch.connect_active_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |switch| {
            controls.set_paste_sensitive(controls.is_custom_sensitive());
            draft.update_shortcut(&shortcut_id_for_paste, |shortcut| {
                if let ShortcutOutput::Custom { action } = &mut shortcut.output {
                    action.paste = if switch.is_active() && action.copy_to_clipboard {
                        Some(controls.paste_from_rows())
                    } else {
                        None
                    };
                }
            });
        }
    });

    let shortcut_id_for_paste_shortcut = shortcut_id.to_string();
    controls
        .paste_shortcut_row
        .dropdown
        .connect_selected_notify({
            let draft = draft.clone();
            let controls = controls.clone();
            move |_| {
                draft.update_shortcut(&shortcut_id_for_paste_shortcut, |shortcut| {
                    if let ShortcutOutput::Custom { action } = &mut shortcut.output
                        && let Some(paste) = &mut action.paste
                    {
                        paste.shortcut = controls.paste_shortcut_from_rows();
                    }
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

    let shortcut_id_for_stdout = shortcut_id.to_string();
    controls
        .copy_stdout_switch
        .connect_active_notify(move |switch| {
            draft.update_shortcut(&shortcut_id_for_stdout, |shortcut| {
                if let ShortcutOutput::Custom { action } = &mut shortcut.output
                    && let Some(script) = &mut action.script
                {
                    script.copy_stdout_to_clipboard = switch.is_active();
                }
            });
        });

    group.add(&output_row.row);
    controls.add_to_group(group);
}

#[derive(Clone)]
struct OutputControls {
    copy_row: adw::ActionRow,
    copy_switch: gtk::Switch,
    paste_row: adw::ActionRow,
    paste_switch: gtk::Switch,
    paste_shortcut_row: DropDownRow,
    run_script_row: adw::ActionRow,
    run_script_switch: gtk::Switch,
    script_row: adw::EntryRow,
    copy_stdout_row: adw::ActionRow,
    copy_stdout_switch: gtk::Switch,
}

impl OutputControls {
    fn new(output: &OutputAction, custom_sensitive: bool) -> Self {
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

        let paste = output.paste.as_ref();
        let has_paste = paste.is_some();
        let paste_parent_sensitive = custom_sensitive && output.copy_to_clipboard;
        let paste_switch = gtk::Switch::builder()
            .active(has_paste)
            .valign(gtk::Align::Center)
            .sensitive(paste_parent_sensitive)
            .build();
        let paste_row = adw::ActionRow::builder()
            .title("Paste after copy")
            .sensitive(paste_parent_sensitive)
            .build();
        paste_row.add_suffix(&paste_switch);
        paste_row.set_activatable_widget(Some(&paste_switch));

        let paste_shortcut_labels = [
            PasteShortcut::CtrlV.display_label(),
            PasteShortcut::CtrlShiftV.display_label(),
        ];
        let paste_shortcut_row = dropdown_row(
            "Paste shortcut",
            &paste_shortcut_labels,
            paste_shortcut_index(paste.map(|paste| paste.shortcut).unwrap_or_default()),
        );
        let paste_shortcut_sensitive = paste_parent_sensitive && has_paste;
        paste_shortcut_row
            .row
            .set_sensitive(paste_shortcut_sensitive);
        paste_shortcut_row
            .dropdown
            .set_sensitive(paste_shortcut_sensitive);

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

        let copy_stdout_switch = gtk::Switch::builder()
            .active(script.is_some_and(|script| script.copy_stdout_to_clipboard))
            .valign(gtk::Align::Center)
            .sensitive(custom_sensitive && has_script)
            .build();
        let copy_stdout_row = adw::ActionRow::builder()
            .title("Copy script output to clipboard")
            .sensitive(custom_sensitive && has_script)
            .build();
        copy_stdout_row.add_suffix(&copy_stdout_switch);
        copy_stdout_row.set_activatable_widget(Some(&copy_stdout_switch));

        Self {
            copy_row,
            copy_switch,
            paste_row,
            paste_switch,
            paste_shortcut_row,
            run_script_row,
            run_script_switch,
            script_row,
            copy_stdout_row,
            copy_stdout_switch,
        }
    }

    fn add_to_group(&self, group: &adw::PreferencesGroup) {
        group.add(&self.copy_row);
        group.add(&self.paste_row);
        group.add(&self.paste_shortcut_row.row);
        group.add(&self.run_script_row);
        group.add(&self.script_row);
        group.add(&self.copy_stdout_row);
    }

    fn action_from_rows(&self) -> OutputAction {
        OutputAction {
            copy_to_clipboard: self.copy_switch.is_active(),
            paste: (self.copy_switch.is_active() && self.paste_switch.is_active())
                .then(|| self.paste_from_rows()),
            script: self
                .run_script_switch
                .is_active()
                .then(|| self.script_from_rows()),
        }
    }

    fn paste_from_rows(&self) -> PasteOutput {
        PasteOutput {
            shortcut: self.paste_shortcut_from_rows(),
        }
    }

    fn paste_shortcut_from_rows(&self) -> PasteShortcut {
        paste_shortcut_from_index(self.paste_shortcut_row.dropdown.selected())
    }

    fn script_from_rows(&self) -> ScriptOutput {
        ScriptOutput {
            path: self.script_row.text().to_string(),
            copy_stdout_to_clipboard: self.copy_stdout_switch.is_active(),
        }
    }

    fn set_all_sensitive(&self, sensitive: bool) {
        self.copy_row.set_sensitive(sensitive);
        self.copy_switch.set_sensitive(sensitive);
        self.run_script_row.set_sensitive(sensitive);
        self.run_script_switch.set_sensitive(sensitive);
        self.set_paste_sensitive(sensitive);
        self.set_script_sensitive(self.run_script_switch.is_active(), sensitive);
    }

    fn set_paste_sensitive(&self, parent_sensitive: bool) {
        let sensitive = parent_sensitive && self.copy_switch.is_active();
        self.paste_row.set_sensitive(sensitive);
        self.paste_switch.set_sensitive(sensitive);

        let shortcut_sensitive = sensitive && self.paste_switch.is_active();
        self.paste_shortcut_row
            .row
            .set_sensitive(shortcut_sensitive);
        self.paste_shortcut_row
            .dropdown
            .set_sensitive(shortcut_sensitive);
    }

    fn set_script_sensitive(&self, has_script: bool, parent_sensitive: bool) {
        let sensitive = parent_sensitive && has_script;
        self.script_row.set_sensitive(sensitive);
        self.copy_stdout_row.set_sensitive(sensitive);
        self.copy_stdout_switch.set_sensitive(sensitive);
    }

    fn is_custom_sensitive(&self) -> bool {
        self.copy_row.is_sensitive()
    }
}

const fn paste_shortcut_index(shortcut: PasteShortcut) -> u32 {
    match shortcut {
        PasteShortcut::CtrlV => 0,
        PasteShortcut::CtrlShiftV => 1,
    }
}

fn paste_shortcut_from_index(index: u32) -> PasteShortcut {
    match index {
        1 => PasteShortcut::CtrlShiftV,
        _ => PasteShortcut::CtrlV,
    }
}
