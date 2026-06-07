use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{AppConfig, DEFAULT_SHORTCUT_ID, DaemonStatus};

use crate::audio::AudioInputDevice;
use crate::command::AppCommand;
use crate::models::ModelRowState;

mod draft;
mod pages;
mod shortcut_recorder;
mod sidebar;
mod widgets;

pub use draft::SettingsDraft;

#[derive(Clone)]
pub struct SettingsState {
    draft: SettingsDraft,
    daemon_status: Rc<Cell<DaemonStatus>>,
    audio_input_devices: Rc<RefCell<Vec<AudioInputDevice>>>,
    model_states: Rc<RefCell<Vec<ModelRowState>>>,
    ready_model_ids: Rc<RefCell<HashSet<String>>>,
    command_tx: mpsc::Sender<AppCommand>,
}

pub struct SettingsWindow {
    window: adw::ApplicationWindow,
    stack: gtk::Stack,
    sidebar: sidebar::SettingsSidebar,
    toast_overlay: adw::ToastOverlay,
    state: SettingsState,
    general_page: Rc<RefCell<Option<pages::general::GeneralPage>>>,
    models_page: Rc<RefCell<Option<pages::models::ModelsPage>>>,
}

impl SettingsWindow {
    pub fn new(
        application: &adw::Application,
        config: &AppConfig,
        audio_input_devices: Vec<AudioInputDevice>,
        model_states: Vec<ModelRowState>,
        ready_model_ids: HashSet<String>,
        daemon_status: DaemonStatus,
        command_tx: mpsc::Sender<AppCommand>,
    ) -> Self {
        let state = SettingsState {
            draft: SettingsDraft::new(config.clone()),
            daemon_status: Rc::new(Cell::new(daemon_status)),
            audio_input_devices: Rc::new(RefCell::new(audio_input_devices)),
            model_states: Rc::new(RefCell::new(model_states)),
            ready_model_ids: Rc::new(RefCell::new(ready_model_ids)),
            command_tx: command_tx.clone(),
        };

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

        let layout = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .vexpand(true)
            .build();
        let stack = gtk::Stack::builder()
            .hexpand(true)
            .vexpand(true)
            .hhomogeneous(false)
            .vhomogeneous(false)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        let sidebar = sidebar::SettingsSidebar::new(&stack);
        layout.append(sidebar.widget());
        layout.append(&stack);
        content.append(&layout);

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&content));

        let window = adw::ApplicationWindow::builder()
            .application(application)
            .title("MyApp Settings")
            .default_width(900)
            .default_height(760)
            .content(&toast_overlay)
            .build();

        window.connect_close_request(|window| {
            window.hide();
            gtk::glib::Propagation::Stop
        });

        connect_save_button(
            &save_button,
            state.draft.clone(),
            command_tx,
            toast_overlay.clone(),
        );

        let this = Self {
            window,
            stack,
            sidebar,
            toast_overlay,
            state,
            general_page: Rc::new(RefCell::new(None)),
            models_page: Rc::new(RefCell::new(None)),
        };
        this.render(None);
        this
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn update_config(&self, config: &AppConfig) {
        let visible = self.stack.visible_child_name().map(|name| name.to_string());
        self.state.draft.replace(config.clone());
        self.render(visible);
    }

    pub fn refresh_live_state(
        &self,
        model_states: Vec<ModelRowState>,
        ready_model_ids: HashSet<String>,
        daemon_status: DaemonStatus,
    ) {
        let visible = self.stack.visible_child_name().map(|name| name.to_string());
        self.state.model_states.replace(model_states);
        self.state.ready_model_ids.replace(ready_model_ids);
        self.state.daemon_status.set(daemon_status);
        self.render(visible);
    }

    pub fn update_model_states(
        &self,
        model_states: Vec<ModelRowState>,
        ready_model_ids: HashSet<String>,
    ) {
        self.state.model_states.replace(model_states);
        self.state.ready_model_ids.replace(ready_model_ids);
        if let Some(models_page) = self.models_page.borrow().as_ref() {
            models_page.update(&self.state.model_states.borrow());
        } else {
            let visible = self.stack.visible_child_name().map(|name| name.to_string());
            self.render(visible);
        }
    }

    pub fn update_audio_input_devices(&self, audio_input_devices: Vec<AudioInputDevice>) {
        let visible = self.stack.visible_child_name().map(|name| name.to_string());
        self.state.audio_input_devices.replace(audio_input_devices);
        self.render(visible);
    }

    pub fn update_model_inventory(
        &self,
        model_states: Vec<ModelRowState>,
        ready_model_ids: HashSet<String>,
    ) {
        let visible = self.stack.visible_child_name().map(|name| name.to_string());
        self.state.model_states.replace(model_states);
        self.state.ready_model_ids.replace(ready_model_ids);
        self.render(visible);
    }

    pub fn update_daemon_status(&self, daemon_status: DaemonStatus) {
        self.state.daemon_status.set(daemon_status);
        if let Some(general_page) = self.general_page.borrow().as_ref() {
            general_page.update_daemon_status(daemon_status);
        } else {
            let visible = self.stack.visible_child_name().map(|name| name.to_string());
            self.render(visible);
        }
    }

    pub fn update_save_status(&self, status: &str) {
        self.toast_overlay.add_toast(adw::Toast::new(status));
    }

    fn render(&self, preferred_page: Option<String>) {
        render_stack(
            &self.stack,
            &self.sidebar,
            &self.state,
            preferred_page,
            &self.general_page,
            &self.models_page,
        );
    }
}

fn connect_save_button(
    save_button: &gtk::Button,
    draft: SettingsDraft,
    command_tx: mpsc::Sender<AppCommand>,
    toast_overlay: adw::ToastOverlay,
) {
    save_button.connect_clicked(move |_| match draft.normalized() {
        Ok(config) => {
            if command_tx.send(AppCommand::SaveConfig(config)).is_err() {
                toast_overlay.add_toast(adw::Toast::new("Failed to send save command"));
            }
        }
        Err(error) => {
            toast_overlay.add_toast(adw::Toast::new(&format!("Invalid settings: {error}")));
        }
    });
}

fn render_stack(
    stack: &gtk::Stack,
    settings_sidebar: &sidebar::SettingsSidebar,
    state: &SettingsState,
    preferred_page: Option<String>,
    general_page_slot: &Rc<RefCell<Option<pages::general::GeneralPage>>>,
    models_page_slot: &Rc<RefCell<Option<pages::models::ModelsPage>>>,
) {
    general_page_slot.replace(None);
    models_page_slot.replace(None);
    while let Some(child) = stack.first_child() {
        stack.remove(&child);
    }

    let config = state.draft.snapshot();
    let render_request: Rc<dyn Fn(Option<String>)> = Rc::new({
        let stack = stack.clone();
        let settings_sidebar = settings_sidebar.clone();
        let state = state.clone();
        let general_page_slot = Rc::clone(general_page_slot);
        let models_page_slot = Rc::clone(models_page_slot);
        move |preferred_page| {
            render_stack(
                &stack,
                &settings_sidebar,
                &state,
                preferred_page,
                &general_page_slot,
                &models_page_slot,
            )
        }
    });

    let ready_model_ids = state.ready_model_ids.borrow().clone();
    let app_pages = vec![
        sidebar::SidebarPage::new("general", "General"),
        sidebar::SidebarPage::new("models", "Models"),
    ];
    let mut shortcut_pages = Vec::new();

    let general_page = pages::general::build(
        &config,
        state.audio_input_devices.borrow().clone(),
        ready_model_ids.clone(),
        state.daemon_status.get(),
        state.draft.clone(),
    );
    stack.add_titled(
        &widgets::scrollable_page(general_page.widget()),
        Some("general"),
        "General",
    );
    general_page_slot.replace(Some(general_page));
    let models_page = pages::models::build(
        state.model_states.borrow().clone(),
        state.command_tx.clone(),
    );
    stack.add_titled(
        &widgets::scrollable_page(models_page.widget()),
        Some("models"),
        "Models",
    );
    models_page_slot.replace(Some(models_page));

    for shortcut in &config.shortcuts {
        let title = if shortcut.id == DEFAULT_SHORTCUT_ID {
            "Default".to_string()
        } else {
            shortcut.name.clone()
        };
        let shortcut_page = pages::shortcut::build(
            shortcut,
            ready_model_ids.clone(),
            state.draft.clone(),
            Rc::clone(&render_request),
        );
        stack.add_titled(
            &widgets::scrollable_page(&shortcut_page),
            Some(&shortcut_page_name(&shortcut.id)),
            &title,
        );
        shortcut_pages.push(sidebar::SidebarPage::new(
            shortcut_page_name(&shortcut.id),
            title,
        ));
    }

    let add_shortcut_page = pages::add_shortcut::build(
        state.draft.clone(),
        ready_model_ids,
        Rc::clone(&render_request),
    );
    stack.add_titled(
        &widgets::scrollable_page(&add_shortcut_page),
        Some("add-shortcut"),
        "Add New",
    );
    shortcut_pages.push(sidebar::SidebarPage::new("add-shortcut", "Add New"));

    let target = preferred_page
        .filter(|name| stack.child_by_name(name).is_some())
        .unwrap_or_else(|| "general".to_string());
    stack.set_visible_child_name(&target);
    settings_sidebar.set_sections(
        &[
            sidebar::SidebarSection::new("App", app_pages),
            sidebar::SidebarSection::new("Shortcuts", shortcut_pages),
        ],
        &target,
    );
}

pub(super) fn shortcut_page_name(shortcut_id: &str) -> String {
    format!("shortcut-{shortcut_id}")
}
