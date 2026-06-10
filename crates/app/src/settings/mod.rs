use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{AppConfig, DEFAULT_SHORTCUT_ID};

use crate::audio::AudioInputDevice;
use crate::command::AppCommand;
use crate::hotkey::ShortcutTriggerCapabilities;
use crate::models::ModelRowState;
use crate::transcription::WhisperRuntimeStatus;

mod draft;
mod pages;
mod shortcut_recorder;
mod sidebar;
mod widgets;

pub use draft::SettingsDraft;

#[derive(Clone)]
pub struct SettingsState {
    draft: SettingsDraft,
    audio_input_devices: Rc<RefCell<Vec<AudioInputDevice>>>,
    model_states: Rc<RefCell<Vec<ModelRowState>>>,
    ready_model_ids: Rc<RefCell<HashSet<String>>>,
    whisper_runtime_status: Rc<RefCell<WhisperRuntimeStatus>>,
    shortcut_trigger_capabilities: ShortcutTriggerCapabilities,
    command_tx: mpsc::Sender<AppCommand>,
}

pub struct SettingsWindow {
    window: adw::ApplicationWindow,
    stack: gtk::Stack,
    sidebar: sidebar::SettingsSidebar,
    toast_overlay: adw::ToastOverlay,
    state: SettingsState,
    status_page: Rc<RefCell<Option<pages::status::StatusPage>>>,
    general_page: Rc<RefCell<Option<pages::general::GeneralPage>>>,
    models_page: Rc<RefCell<Option<pages::models::ModelsPage>>>,
}

pub struct SettingsWindowInit {
    pub config: AppConfig,
    pub audio_input_devices: Vec<AudioInputDevice>,
    pub model_states: Vec<ModelRowState>,
    pub ready_model_ids: HashSet<String>,
    pub whisper_runtime_status: WhisperRuntimeStatus,
    pub shortcut_trigger_capabilities: ShortcutTriggerCapabilities,
    pub command_tx: mpsc::Sender<AppCommand>,
}

impl SettingsWindow {
    pub fn new(application: &adw::Application, init: SettingsWindowInit) -> Self {
        let SettingsWindowInit {
            config,
            audio_input_devices,
            model_states,
            ready_model_ids,
            whisper_runtime_status,
            shortcut_trigger_capabilities,
            command_tx,
        } = init;
        let draft = SettingsDraft::new(config);
        draft.coerce_trigger_capabilities(shortcut_trigger_capabilities);
        let state = SettingsState {
            draft,
            audio_input_devices: Rc::new(RefCell::new(audio_input_devices)),
            model_states: Rc::new(RefCell::new(model_states)),
            ready_model_ids: Rc::new(RefCell::new(ready_model_ids)),
            whisper_runtime_status: Rc::new(RefCell::new(whisper_runtime_status)),
            shortcut_trigger_capabilities,
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
            .default_height(940)
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
            status_page: Rc::new(RefCell::new(None)),
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
        self.state
            .draft
            .coerce_trigger_capabilities(self.state.shortcut_trigger_capabilities);
        self.render(visible);
    }

    pub fn assign_factory_model_to_shortcuts(&self, model_id: &str) {
        let visible = self.stack.visible_child_name().map(|name| name.to_string());
        self.state.draft.assign_factory_model_to_shortcuts(model_id);
        self.render(visible);
    }

    pub fn refresh_live_state(
        &self,
        model_states: Vec<ModelRowState>,
        ready_model_ids: HashSet<String>,
    ) {
        let visible = self.stack.visible_child_name().map(|name| name.to_string());
        self.state.model_states.replace(model_states);
        self.state.ready_model_ids.replace(ready_model_ids);
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

    pub fn update_whisper_runtime_status(&self, status: WhisperRuntimeStatus) {
        let visible = self.stack.visible_child_name().map(|name| name.to_string());
        self.state.whisper_runtime_status.replace(status);
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

    pub fn update_save_status(&self, status: &str) {
        self.toast_overlay.add_toast(adw::Toast::new(status));
    }

    fn render(&self, preferred_page: Option<String>) {
        render_stack(
            &self.stack,
            &self.sidebar,
            &self.state,
            preferred_page,
            &self.status_page,
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
    status_page_slot: &Rc<RefCell<Option<pages::status::StatusPage>>>,
    general_page_slot: &Rc<RefCell<Option<pages::general::GeneralPage>>>,
    models_page_slot: &Rc<RefCell<Option<pages::models::ModelsPage>>>,
) {
    status_page_slot.replace(None);
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
        let status_page_slot = Rc::clone(status_page_slot);
        let general_page_slot = Rc::clone(general_page_slot);
        let models_page_slot = Rc::clone(models_page_slot);
        move |preferred_page| {
            render_stack(
                &stack,
                &settings_sidebar,
                &state,
                preferred_page,
                &status_page_slot,
                &general_page_slot,
                &models_page_slot,
            )
        }
    });

    let ready_model_ids = state.ready_model_ids.borrow().clone();
    let app_pages = vec![
        sidebar::SidebarPage::new("status", "Status"),
        sidebar::SidebarPage::new("general", "General"),
        sidebar::SidebarPage::new("models", "Models"),
    ];
    let mut shortcut_pages = Vec::new();

    let status_page = pages::status::build(state.whisper_runtime_status.borrow().clone());
    stack.add_titled(
        &widgets::scrollable_page(status_page.widget()),
        Some("status"),
        "Status",
    );
    status_page_slot.replace(Some(status_page));

    let general_page = pages::general::build(
        &config,
        state.audio_input_devices.borrow().clone(),
        state.draft.clone(),
    );
    stack.add_titled(general_page.widget(), Some("general"), "General");
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
            state.shortcut_trigger_capabilities,
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
        state.shortcut_trigger_capabilities,
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
        .unwrap_or_else(|| "status".to_string());
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
