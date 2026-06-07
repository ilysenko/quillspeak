use std::collections::HashMap;
use std::sync::mpsc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::command::AppCommand;
use crate::models::{ModelRowState, ModelStatus};
use crate::settings::widgets::preferences_page;

const ACTION_WIDTH: i32 = 108;

#[derive(Clone)]
pub struct ModelsPage {
    page: adw::PreferencesPage,
    rows: HashMap<String, ModelRow>,
}

impl ModelsPage {
    pub fn widget(&self) -> &adw::PreferencesPage {
        &self.page
    }

    pub fn update(&self, model_states: &[ModelRowState]) {
        for state in model_states {
            if let Some(row) = self.rows.get(state.entry.id) {
                row.update(state);
            }
        }
    }
}

#[derive(Clone)]
struct ModelRow {
    container: gtk::Box,
    row: adw::ActionRow,
    progress: gtk::ProgressBar,
    action_stack: gtk::Stack,
    delete_button: gtk::Button,
    cancel_button: gtk::Button,
    canceling_button: gtk::Button,
}

impl ModelRow {
    fn new(state: &ModelRowState, command_tx: &mpsc::Sender<AppCommand>) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let row = adw::ActionRow::builder().title(state.entry.label).build();

        let action_stack = gtk::Stack::builder()
            .hhomogeneous(false)
            .vhomogeneous(false)
            .transition_type(gtk::StackTransitionType::None)
            .valign(gtk::Align::Center)
            .build();

        let download_button = action_button("Download");
        download_button.connect_clicked({
            let command_tx = command_tx.clone();
            let model_id = state.entry.id.to_string();
            move |_| {
                let _ = command_tx.send(AppCommand::DownloadModel(model_id.clone()));
            }
        });

        let cancel_button = action_button("Cancel");
        cancel_button.connect_clicked({
            let command_tx = command_tx.clone();
            let model_id = state.entry.id.to_string();
            move |_| {
                let _ = command_tx.send(AppCommand::CancelModelDownload(model_id.clone()));
            }
        });

        let canceling_button = action_button("Canceling...");
        canceling_button.set_sensitive(false);

        let delete_button = action_button("Delete");
        delete_button.connect_clicked({
            let command_tx = command_tx.clone();
            let model_id = state.entry.id.to_string();
            move |_| {
                let _ = command_tx.send(AppCommand::DeleteModel(model_id.clone()));
            }
        });

        action_stack.add_named(&download_button, Some("download"));
        action_stack.add_named(&cancel_button, Some("cancel"));
        action_stack.add_named(&canceling_button, Some("canceling"));
        action_stack.add_named(&delete_button, Some("delete"));
        row.add_suffix(&action_stack);

        let progress = gtk::ProgressBar::new();
        progress.set_show_text(true);
        progress.set_hexpand(true);
        progress.set_margin_start(18);
        progress.set_margin_end(18);
        progress.set_margin_bottom(10);
        progress.set_height_request(28);
        progress.set_opacity(0.0);

        container.append(&row);
        container.append(&progress);

        let model_row = Self {
            container,
            row,
            progress,
            action_stack,
            delete_button,
            cancel_button,
            canceling_button,
        };
        model_row.update(state);
        model_row
    }

    fn widget(&self) -> &gtk::Box {
        &self.container
    }

    fn update(&self, state: &ModelRowState) {
        self.row.set_subtitle(&model_subtitle(state));
        self.delete_button.set_sensitive(!state.referenced);
        self.cancel_button.set_sensitive(true);
        self.canceling_button.set_sensitive(false);

        if let Some(progress_label) = state.status.progress_label() {
            self.progress.set_opacity(1.0);
            self.progress.set_text(Some(&progress_label));
            self.progress
                .set_fraction(state.status.progress_fraction().unwrap_or(0.0));
        } else {
            self.progress.set_opacity(0.0);
            self.progress.set_text(None);
            self.progress.set_fraction(0.0);
        }

        self.action_stack
            .set_visible_child_name(match state.status {
                ModelStatus::Ready => "delete",
                ModelStatus::Downloading { .. } => "cancel",
                ModelStatus::Verifying { .. } => "cancel",
                ModelStatus::Canceling { .. } => "canceling",
                ModelStatus::NotInstalled | ModelStatus::Error(_) => "download",
            });
    }
}

pub fn build(model_states: Vec<ModelRowState>, command_tx: mpsc::Sender<AppCommand>) -> ModelsPage {
    let page = preferences_page("Models");
    let group = adw::PreferencesGroup::builder()
        .title("Whisper.cpp")
        .build();
    let mut rows = HashMap::new();

    for state in model_states {
        let model_id = state.entry.id.to_string();
        let row = ModelRow::new(&state, &command_tx);
        group.add(row.widget());
        rows.insert(model_id, row);
    }

    page.add(&group);
    ModelsPage { page, rows }
}

fn action_button(label: &str) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .width_request(ACTION_WIDTH)
        .valign(gtk::Align::Center)
        .build()
}

fn model_subtitle(state: &ModelRowState) -> String {
    format!("{} · {}", state.entry.size_label, state.status.label())
}
