use std::cell::RefCell;
use std::env;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{AppConfig, DaemonStatus};

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
