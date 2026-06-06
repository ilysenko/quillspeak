use std::env;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{AppConfig, DaemonStatus};

pub struct SettingsWindow {
    window: adw::ApplicationWindow,
    daemon_row: adw::ActionRow,
    hotkey_row: adw::ActionRow,
    mode_row: adw::ActionRow,
    backend_row: adw::ActionRow,
    advanced_hotkey_row: adw::ActionRow,
}

impl SettingsWindow {
    pub fn new(
        application: &adw::Application,
        config: &AppConfig,
        daemon_status: DaemonStatus,
    ) -> Self {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        content.append(&adw::HeaderBar::new());

        let page = adw::PreferencesPage::builder().title("Settings").build();
        let status_group = adw::PreferencesGroup::builder().title("Status").build();
        let config_group = adw::PreferencesGroup::builder()
            .title("Configuration")
            .build();

        let daemon_row = property_row("Daemon status", daemon_status.display_label());
        let advanced_hotkey_row =
            property_row("Advanced hotkeys", advanced_hotkey_status(daemon_status));
        let hotkey_row = property_row("Hotkey", &config.hotkey);
        let mode_row = property_row("Mode", config.mode.as_str());
        let backend_row = property_row("Hotkey backend", config.hotkey_backend.as_str());

        status_group.add(&daemon_row);
        status_group.add(&advanced_hotkey_row);
        config_group.add(&hotkey_row);
        config_group.add(&mode_row);
        config_group.add(&backend_row);
        page.add(&status_group);
        page.add(&config_group);
        content.append(&page);

        let window = adw::ApplicationWindow::builder()
            .application(application)
            .title("MyApp Settings")
            .default_width(460)
            .default_height(360)
            .content(&content)
            .build();

        window.connect_close_request(|window| {
            window.hide();
            gtk::glib::Propagation::Stop
        });

        Self {
            window,
            daemon_row,
            hotkey_row,
            mode_row,
            backend_row,
            advanced_hotkey_row,
        }
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn update_config(&self, config: &AppConfig) {
        self.hotkey_row.set_subtitle(&config.hotkey);
        self.mode_row.set_subtitle(config.mode.as_str());
        self.backend_row
            .set_subtitle(config.hotkey_backend.as_str());
    }

    pub fn update_daemon_status(&self, daemon_status: DaemonStatus) {
        self.daemon_row.set_subtitle(daemon_status.display_label());
        self.advanced_hotkey_row
            .set_subtitle(advanced_hotkey_status(daemon_status));
    }
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
    if daemon_status == DaemonStatus::Running {
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
