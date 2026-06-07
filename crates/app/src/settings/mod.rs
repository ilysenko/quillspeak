use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{AppConfig, DEFAULT_SHORTCUT_ID, DaemonStatus};

use crate::command::AppCommand;
use crate::models::ModelRowState;

mod draft;
mod pages;
mod shortcut_recorder;
mod widgets;

pub use draft::SettingsDraft;

#[derive(Clone)]
pub struct SettingsState {
    draft: SettingsDraft,
    daemon_status: Rc<Cell<DaemonStatus>>,
    model_states: Rc<RefCell<Vec<ModelRowState>>>,
    ready_model_ids: Rc<RefCell<HashSet<String>>>,
    command_tx: mpsc::Sender<AppCommand>,
}

pub struct SettingsWindow {
    window: adw::ApplicationWindow,
    stack: gtk::Stack,
    toast_overlay: adw::ToastOverlay,
    state: SettingsState,
}

impl SettingsWindow {
    pub fn new(
        application: &adw::Application,
        config: &AppConfig,
        model_states: Vec<ModelRowState>,
        ready_model_ids: HashSet<String>,
        daemon_status: DaemonStatus,
        command_tx: mpsc::Sender<AppCommand>,
    ) -> Self {
        let state = SettingsState {
            draft: SettingsDraft::new(config.clone()),
            daemon_status: Rc::new(Cell::new(daemon_status)),
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
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        let sidebar = gtk::StackSidebar::builder()
            .stack(&stack)
            .width_request(150)
            .build();
        layout.append(&sidebar);
        layout.append(&stack);
        content.append(&layout);

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&content));

        let window = adw::ApplicationWindow::builder()
            .application(application)
            .title("MyApp Settings")
            .default_width(760)
            .default_height(540)
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
            toast_overlay,
            state,
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

    pub fn update_model_states(
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
        let visible = self.stack.visible_child_name().map(|name| name.to_string());
        self.state.daemon_status.set(daemon_status);
        self.render(visible);
    }

    pub fn update_save_status(&self, status: &str) {
        self.toast_overlay.add_toast(adw::Toast::new(status));
    }

    fn render(&self, preferred_page: Option<String>) {
        render_stack(&self.stack, &self.state, preferred_page);
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

pub(super) fn render_stack(
    stack: &gtk::Stack,
    state: &SettingsState,
    preferred_page: Option<String>,
) {
    while let Some(child) = stack.first_child() {
        stack.remove(&child);
    }

    let config = state.draft.snapshot();
    let render_request: Rc<dyn Fn(Option<String>)> = Rc::new({
        let stack = stack.clone();
        let state = state.clone();
        move |preferred_page| render_stack(&stack, &state, preferred_page)
    });

    stack.add_titled(
        &pages::general::build(&config, state.daemon_status.get(), state.draft.clone()),
        Some("general"),
        "General",
    );
    stack.add_titled(
        &pages::models::build(
            state.model_states.borrow().clone(),
            state.command_tx.clone(),
        ),
        Some("models"),
        "Models",
    );

    let ready_model_ids = state.ready_model_ids.borrow().clone();
    for shortcut in &config.shortcuts {
        let title = if shortcut.id == DEFAULT_SHORTCUT_ID {
            "Default".to_string()
        } else {
            shortcut.name.clone()
        };
        stack.add_titled(
            &pages::shortcut::build(
                shortcut,
                ready_model_ids.clone(),
                state.draft.clone(),
                Rc::clone(&render_request),
            ),
            Some(&shortcut_page_name(&shortcut.id)),
            &title,
        );
    }

    stack.add_titled(
        &pages::add_shortcut::build(
            state.draft.clone(),
            ready_model_ids,
            Rc::clone(&render_request),
        ),
        Some("add-shortcut"),
        "Add New",
    );

    let target = preferred_page
        .filter(|name| stack.child_by_name(name).is_some())
        .unwrap_or_else(|| "general".to_string());
    stack.set_visible_child_name(&target);
}

pub(super) fn shortcut_page_name(shortcut_id: &str) -> String {
    format!("shortcut-{shortcut_id}")
}
