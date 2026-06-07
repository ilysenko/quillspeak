use std::cell::RefCell;
use std::env;
use std::rc::Rc;
use std::sync::mpsc;

use gtk::gdk;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{AppConfig, DaemonStatus, normalize_accelerator};

use crate::command::AppCommand;

pub struct SettingsWindow {
    window: adw::ApplicationWindow,
    daemon_row: adw::ActionRow,
    shortcut_entry: adw::EntryRow,
    mode_row: adw::ActionRow,
    backend_row: adw::ActionRow,
    advanced_hotkey_row: adw::ActionRow,
    save_status_row: adw::ActionRow,
    draft_config: Rc<RefCell<AppConfig>>,
}

impl SettingsWindow {
    pub fn new(
        application: &adw::Application,
        config: &AppConfig,
        daemon_status: DaemonStatus,
        command_tx: mpsc::Sender<AppCommand>,
    ) -> Self {
        let draft_config = Rc::new(RefCell::new(config.clone()));
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();

        let header = adw::HeaderBar::new();
        let save_button = gtk::Button::builder()
            .label("Save")
            .tooltip_text("Save settings")
            .build();
        save_button.add_css_class("suggested-action");
        header.pack_end(&save_button);
        content.append(&header);

        let page = adw::PreferencesPage::builder().title("Settings").build();
        let status_group = adw::PreferencesGroup::builder().title("Status").build();
        let shortcuts_group = adw::PreferencesGroup::builder().title("Shortcuts").build();
        let config_group = adw::PreferencesGroup::builder()
            .title("Configuration")
            .build();

        let daemon_row = property_row("Daemon status", daemon_status.display_label());
        let advanced_hotkey_row =
            property_row("Advanced hotkeys", advanced_hotkey_status(daemon_status));
        let shortcut_entry = adw::EntryRow::builder()
            .title("Push to talk")
            .text(&config.shortcuts.push_to_talk.accelerator)
            .build();
        let record_button = gtk::Button::builder()
            .label("Record")
            .tooltip_text("Record shortcut")
            .valign(gtk::Align::Center)
            .build();
        shortcut_entry.add_suffix(&record_button);
        let mode_row = property_row("Mode", config.mode.as_str());
        let backend_row = property_row("Hotkey backend", config.hotkey_backend.as_str());
        let save_status_row = property_row("Settings", "No changes saved in this session");

        status_group.add(&daemon_row);
        status_group.add(&advanced_hotkey_row);
        shortcuts_group.add(&shortcut_entry);
        config_group.add(&mode_row);
        config_group.add(&backend_row);
        config_group.add(&save_status_row);
        page.add(&status_group);
        page.add(&shortcuts_group);
        page.add(&config_group);
        content.append(&page);

        let window = adw::ApplicationWindow::builder()
            .application(application)
            .title("MyApp Settings")
            .default_width(460)
            .default_height(420)
            .content(&content)
            .build();

        window.connect_close_request(|window| {
            window.hide();
            gtk::glib::Propagation::Stop
        });

        connect_record_button(&record_button, &window, &shortcut_entry, &save_status_row);
        connect_save_button(
            &save_button,
            &shortcut_entry,
            &save_status_row,
            Rc::clone(&draft_config),
            command_tx,
        );

        Self {
            window,
            daemon_row,
            shortcut_entry,
            mode_row,
            backend_row,
            advanced_hotkey_row,
            save_status_row,
            draft_config,
        }
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn update_config(&self, config: &AppConfig) {
        self.draft_config.replace(config.clone());
        self.shortcut_entry
            .set_text(&config.shortcuts.push_to_talk.accelerator);
        self.mode_row.set_subtitle(config.mode.as_str());
        self.backend_row
            .set_subtitle(config.hotkey_backend.as_str());
        self.save_status_row.set_subtitle("Saved");
    }

    pub fn update_daemon_status(&self, daemon_status: DaemonStatus) {
        self.daemon_row.set_subtitle(daemon_status.display_label());
        self.advanced_hotkey_row
            .set_subtitle(advanced_hotkey_status(daemon_status));
    }
}

fn connect_save_button(
    save_button: &gtk::Button,
    shortcut_entry: &adw::EntryRow,
    save_status_row: &adw::ActionRow,
    draft_config: Rc<RefCell<AppConfig>>,
    command_tx: mpsc::Sender<AppCommand>,
) {
    let shortcut_entry = shortcut_entry.clone();
    let save_status_row = save_status_row.clone();
    save_button.connect_clicked(move |_| {
        let mut config = draft_config.borrow().clone();
        config.shortcuts.push_to_talk.accelerator = shortcut_entry.text().to_string();
        match config.normalized() {
            Ok(config) => {
                save_status_row.set_subtitle("Saving...");
                if command_tx.send(AppCommand::SaveConfig(config)).is_err() {
                    save_status_row.set_subtitle("Failed to send save command");
                }
            }
            Err(error) => {
                save_status_row.set_subtitle(&format!("Invalid shortcut: {error}"));
            }
        }
    });
}

fn connect_record_button(
    record_button: &gtk::Button,
    parent: &adw::ApplicationWindow,
    shortcut_entry: &adw::EntryRow,
    save_status_row: &adw::ActionRow,
) {
    let parent = parent.clone();
    let shortcut_entry = shortcut_entry.clone();
    let save_status_row = save_status_row.clone();
    record_button.connect_clicked(move |_| {
        show_shortcut_recorder(&parent, &shortcut_entry, &save_status_row);
    });
}

fn show_shortcut_recorder(
    parent: &adw::ApplicationWindow,
    shortcut_entry: &adw::EntryRow,
    save_status_row: &adw::ActionRow,
) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .focusable(true)
        .build();
    let title = gtk::Label::builder()
        .label("Press a shortcut")
        .halign(gtk::Align::Start)
        .build();
    title.add_css_class("title-3");
    let status_label = gtk::Label::builder()
        .label("Waiting for key combination")
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    let clear_button = gtk::Button::builder().label("Clear").build();
    let cancel_button = gtk::Button::builder().label("Cancel").build();
    actions.append(&clear_button);
    actions.append(&cancel_button);
    content.append(&title);
    content.append(&status_label);
    content.append(&actions);

    let recorder = gtk::Window::builder()
        .title("Record Shortcut")
        .transient_for(parent)
        .modal(true)
        .default_width(360)
        .default_height(160)
        .child(&content)
        .build();

    clear_button.connect_clicked({
        let recorder = recorder.clone();
        let shortcut_entry = shortcut_entry.clone();
        let save_status_row = save_status_row.clone();
        move |_| {
            shortcut_entry.set_text("");
            save_status_row.set_subtitle("Shortcut cleared, not saved");
            recorder.close();
        }
    });

    cancel_button.connect_clicked({
        let recorder = recorder.clone();
        move |_| recorder.close()
    });

    let controller = gtk::EventControllerKey::new();
    controller.connect_key_pressed({
        let recorder = recorder.clone();
        let shortcut_entry = shortcut_entry.clone();
        let save_status_row = save_status_row.clone();
        let status_label = status_label.clone();
        move |_, keyval, _, state| {
            if keyval == gdk::Key::Escape {
                recorder.close();
                return gtk::glib::Propagation::Stop;
            }

            match shortcut_from_key_event(keyval, state) {
                Ok(accelerator) => {
                    shortcut_entry.set_text(&accelerator);
                    save_status_row.set_subtitle("Shortcut recorded, not saved");
                    recorder.close();
                }
                Err(message) => {
                    status_label.set_text(&message);
                }
            }

            gtk::glib::Propagation::Stop
        }
    });
    content.add_controller(controller);

    recorder.present();
    content.grab_focus();
}

fn shortcut_from_key_event(keyval: gdk::Key, state: gdk::ModifierType) -> Result<String, String> {
    if is_modifier_key(keyval) {
        return Err("Press a non-modifier key as part of the shortcut".to_string());
    }

    let modifiers = significant_modifiers(state);
    if !gtk::accelerator_valid(keyval, modifiers) {
        return Err("Shortcut is not valid".to_string());
    }

    let key = key_name_for_config(keyval).ok_or_else(|| "Unsupported shortcut key".to_string())?;
    let mut parts = Vec::new();
    if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
        parts.push("Ctrl".to_string());
    }
    if modifiers.contains(gdk::ModifierType::ALT_MASK) {
        parts.push("Alt".to_string());
    }
    if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
        parts.push("Shift".to_string());
    }
    if modifiers.contains(gdk::ModifierType::SUPER_MASK) {
        parts.push("Super".to_string());
    }
    parts.push(key);

    normalize_accelerator(&parts.join("+")).map_err(|error| error.to_string())
}

fn significant_modifiers(state: gdk::ModifierType) -> gdk::ModifierType {
    let mut modifiers = gdk::ModifierType::empty();
    for mask in [
        gdk::ModifierType::CONTROL_MASK,
        gdk::ModifierType::ALT_MASK,
        gdk::ModifierType::SHIFT_MASK,
        gdk::ModifierType::SUPER_MASK,
    ] {
        if state.contains(mask) {
            modifiers.insert(mask);
        }
    }
    modifiers
}

fn is_modifier_key(keyval: gdk::Key) -> bool {
    matches!(
        keyval,
        gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Shift_Lock
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::Meta_L
            | gdk::Key::Meta_R
    )
}

fn key_name_for_config(keyval: gdk::Key) -> Option<String> {
    if keyval == gdk::Key::Return || keyval == gdk::Key::KP_Enter {
        return Some("Enter".to_string());
    }
    if keyval == gdk::Key::space {
        return Some("Space".to_string());
    }

    if let Some(character) = keyval.to_unicode()
        && !character.is_control()
    {
        return Some(character.to_ascii_uppercase().to_string());
    }

    keyval.name().map(|name| name.to_string())
}

fn property_row(title: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(value)
        .build();
    row.add_css_class("property");
    row
}

fn advanced_hotkey_status(daemon_status: DaemonStatus) -> &'static str {
    if daemon_status.is_running() {
        return "Daemon backend available";
    }

    if env::var_os("WAYLAND_DISPLAY").is_some() {
        "Unavailable on Wayland without daemon"
    } else if env::var_os("DISPLAY").is_some() {
        "Future X11 backend can run in-process"
    } else {
        "Disabled"
    }
}
