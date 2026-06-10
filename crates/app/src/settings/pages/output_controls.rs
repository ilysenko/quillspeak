use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{OutputAction, PasteShortcut, ScriptOutput};

use crate::settings::SettingsDraft;
use crate::settings::widgets::{TextRow, dropdown_row_with_help};

pub fn add_shortcut_output_controls(
    group: &adw::PreferencesGroup,
    shortcut_id: &str,
    output: &OutputAction,
    draft: SettingsDraft,
) {
    let controls = OutputControls::new(output);
    let shortcut_id_for_run = shortcut_id.to_string();
    controls.run_script_switch.connect_active_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |switch| {
            controls.set_script_sensitive(switch.is_active());
            draft.update_shortcut(&shortcut_id_for_run, |shortcut| {
                shortcut.output.script = if switch.is_active() {
                    Some(controls.script_from_rows())
                } else {
                    None
                };
            });
        }
    });

    let shortcut_id_for_path = shortcut_id.to_string();
    controls.script_row.entry.connect_changed({
        let draft = draft.clone();
        move |entry| {
            draft.update_shortcut(&shortcut_id_for_path, |shortcut| {
                if let Some(script) = &mut shortcut.output.script {
                    script.path = entry.text().to_string();
                }
            });
        }
    });

    let shortcut_id_for_copy = shortcut_id.to_string();
    controls.copy_switch.connect_active_notify({
        let draft = draft.clone();
        move |switch| {
            draft.update_shortcut(&shortcut_id_for_copy, |shortcut| {
                shortcut.output.copy_to_clipboard = switch.is_active();
            });
        }
    });

    let shortcut_id_for_paste = shortcut_id.to_string();
    controls.paste_switch.connect_active_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |switch| {
            controls.set_paste_sensitive(switch.is_active());
            draft.update_shortcut(&shortcut_id_for_paste, |shortcut| {
                shortcut.output.paste_from_clipboard = switch.is_active();
            });
        }
    });

    let shortcut_id_for_paste_shortcut = shortcut_id.to_string();
    controls.paste_shortcut.dropdown.connect_selected_notify({
        let draft = draft.clone();
        let controls = controls.clone();
        move |dropdown| {
            controls.set_paste_sensitive(controls.paste_switch.is_active());
            draft.update_shortcut(&shortcut_id_for_paste_shortcut, |shortcut| {
                shortcut.output.paste_shortcut = paste_shortcut_from_index(dropdown.selected());
            });
        }
    });

    let shortcut_id_for_paste_x11 = shortcut_id.to_string();
    controls.paste_custom_x11_row.entry.connect_changed({
        let draft = draft.clone();
        move |entry| {
            draft.update_shortcut(&shortcut_id_for_paste_x11, |shortcut| {
                shortcut.output.paste_custom_x11 = entry.text().to_string();
            });
        }
    });

    let shortcut_id_for_paste_wayland = shortcut_id.to_string();
    controls.paste_custom_wayland_row.entry.connect_changed({
        let draft = draft.clone();
        move |entry| {
            draft.update_shortcut(&shortcut_id_for_paste_wayland, |shortcut| {
                shortcut.output.paste_custom_wayland = entry.text().to_string();
            });
        }
    });

    controls.add_to_group(group);
}

#[derive(Clone)]
struct OutputControls {
    run_script_row: adw::ActionRow,
    run_script_switch: gtk::Switch,
    script_row: TextRow,
    copy_row: adw::ActionRow,
    copy_switch: gtk::Switch,
    paste_row: adw::ActionRow,
    paste_switch: gtk::Switch,
    paste_shortcut: crate::settings::widgets::DropDownRow,
    paste_custom_x11_row: TextRow,
    paste_custom_wayland_row: TextRow,
}

impl OutputControls {
    fn new(output: &OutputAction) -> Self {
        let script = output.script.as_ref();
        let has_script = script.is_some();
        let run_script_switch = gtk::Switch::builder()
            .active(has_script)
            .valign(gtk::Align::Center)
            .build();
        let run_script_row = adw::ActionRow::builder()
            .title("Run script")
            .subtitle("Pass the transcript to an executable script. The script receives the transcript on stdin; if it prints stdout, that text becomes the final output.")
            .build();
        run_script_row.add_suffix(&run_script_switch);
        run_script_row.set_activatable_widget(Some(&run_script_switch));

        let script_row = crate::settings::widgets::text_row(
            "Script path",
            "Absolute path to the executable script used by this shortcut.",
            script.map(|script| script.path.as_str()).unwrap_or(""),
        );
        script_row.row.set_sensitive(has_script);

        let copy_switch = gtk::Switch::builder()
            .active(output.copy_to_clipboard)
            .valign(gtk::Align::Center)
            .build();
        let copy_row = adw::ActionRow::builder()
            .title("Copy to clipboard")
            .subtitle("Copy the final text to the system clipboard after transcription and optional script processing.")
            .build();
        copy_row.add_suffix(&copy_switch);
        copy_row.set_activatable_widget(Some(&copy_switch));

        let paste_switch = gtk::Switch::builder()
            .active(output.paste_from_clipboard)
            .valign(gtk::Align::Center)
            .build();
        let paste_row = adw::ActionRow::builder()
            .title("Paste from clipboard")
            .subtitle("After the final text is verified in the clipboard, send a paste shortcut to the focused app.")
            .build();
        paste_row.add_suffix(&paste_switch);
        paste_row.set_activatable_widget(Some(&paste_switch));

        let paste_shortcut = dropdown_row_with_help(
            "Paste shortcut",
            "Keyboard shortcut sent after clipboard verification. Ctrl+Shift+V is useful for terminals and some chat apps.",
            &["Ctrl+V", "Ctrl+Shift+V", "Custom"],
            paste_shortcut_index(output.paste_shortcut),
        );
        paste_shortcut
            .row
            .set_sensitive(output.paste_from_clipboard);

        let paste_custom_x11_row = crate::settings::widgets::text_row(
            "X11 xdotool keys",
            "Custom xdotool key expression used on X11 when Paste shortcut is set to Custom.",
            &output.paste_custom_x11,
        );
        let paste_custom_wayland_row = crate::settings::widgets::text_row(
            "Wayland ydotool keys",
            "Custom ydotool key sequence used on Wayland when Paste shortcut is set to Custom.",
            &output.paste_custom_wayland,
        );

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
        .with_paste_visibility()
    }

    fn add_to_group(&self, group: &adw::PreferencesGroup) {
        group.add(&self.run_script_row);
        group.add(&self.script_row.row);
        group.add(&self.copy_row);
        group.add(&self.paste_row);
        group.add(&self.paste_shortcut.row);
        group.add(&self.paste_custom_x11_row.row);
        group.add(&self.paste_custom_wayland_row.row);
    }

    fn script_from_rows(&self) -> ScriptOutput {
        ScriptOutput {
            path: self.script_row.entry.text().to_string(),
        }
    }

    fn set_script_sensitive(&self, has_script: bool) {
        self.script_row.row.set_sensitive(has_script);
    }

    fn set_paste_sensitive(&self, paste_enabled: bool) {
        let custom = paste_shortcut_from_index(self.paste_shortcut.dropdown.selected())
            == PasteShortcut::Custom;
        self.paste_shortcut.row.set_sensitive(paste_enabled);
        self.paste_custom_x11_row
            .row
            .set_visible(paste_enabled && custom);
        self.paste_custom_wayland_row
            .row
            .set_visible(paste_enabled && custom);
        self.paste_custom_x11_row
            .row
            .set_sensitive(paste_enabled && custom);
        self.paste_custom_wayland_row
            .row
            .set_sensitive(paste_enabled && custom);
    }

    fn with_paste_visibility(self) -> Self {
        self.set_paste_sensitive(self.paste_switch.is_active());
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
