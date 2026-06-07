use std::collections::HashMap;
use std::sync::mpsc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::command::AppCommand;
use crate::models::{ModelRowState, ModelStatus};
use crate::settings::widgets::preferences_page;

const ACTION_WIDTH: i32 = 132;

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
    row: adw::PreferencesRow,
    subtitle: gtk::Label,
    progress: gtk::ProgressBar,
    action_stack: gtk::Stack,
    remove_button: gtk::Button,
    cancel_button: gtk::Button,
    canceling_button: gtk::Button,
}

impl ModelRow {
    fn new(state: &ModelRowState, command_tx: &mpsc::Sender<AppCommand>) -> Self {
        let row = adw::PreferencesRow::builder()
            .title(state.entry.label)
            .build();
        let container = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(10)
            .margin_bottom(10)
            .margin_start(18)
            .margin_end(18)
            .build();
        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .build();
        let labels = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        let title = gtk::Label::builder()
            .label(state.entry.label)
            .xalign(0.0)
            .hexpand(true)
            .build();
        title.add_css_class("heading");
        let subtitle = gtk::Label::builder().xalign(0.0).hexpand(true).build();
        subtitle.add_css_class("dim-label");
        subtitle.set_wrap(true);
        labels.append(&title);
        labels.append(&subtitle);

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

        let remove_button = action_button("Remove Model");
        remove_button.add_css_class("destructive-action");
        remove_button.connect_clicked({
            let command_tx = command_tx.clone();
            let model_id = state.entry.id.to_string();
            let model_label = state.entry.label.to_string();
            move |button| {
                confirm_remove_model(
                    button,
                    command_tx.clone(),
                    model_id.clone(),
                    model_label.clone(),
                );
            }
        });

        action_stack.add_named(&download_button, Some("download"));
        action_stack.add_named(&cancel_button, Some("cancel"));
        action_stack.add_named(&canceling_button, Some("canceling"));
        action_stack.add_named(&remove_button, Some("remove"));
        header.append(&labels);
        header.append(&action_stack);

        let progress = gtk::ProgressBar::new();
        progress.set_show_text(true);
        progress.set_hexpand(true);
        progress.set_height_request(28);
        progress.set_opacity(0.0);

        container.append(&header);
        container.append(&progress);
        row.set_child(Some(&container));

        let model_row = Self {
            row,
            subtitle,
            progress,
            action_stack,
            remove_button,
            cancel_button,
            canceling_button,
        };
        model_row.update(state);
        model_row
    }

    fn widget(&self) -> &adw::PreferencesRow {
        &self.row
    }

    fn update(&self, state: &ModelRowState) {
        self.subtitle.set_label(&model_subtitle(state));
        self.remove_button.set_sensitive(!state.referenced);
        self.remove_button
            .set_tooltip_text(state.referenced.then_some("Model is used by settings"));
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
                ModelStatus::Ready => "remove",
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

fn confirm_remove_model(
    parent: &gtk::Button,
    command_tx: mpsc::Sender<AppCommand>,
    model_id: String,
    model_label: String,
) {
    let parent_window = parent
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok());
    let dialog = adw::MessageDialog::builder()
        .heading("Remove model?")
        .body(format!(
            "Remove {model_label} from this computer? It can be downloaded again later."
        ))
        .close_response("cancel")
        .default_response("cancel")
        .build();
    dialog.set_transient_for(parent_window.as_ref());
    dialog.add_responses(&[("cancel", "Cancel"), ("remove", "Remove Model")]);
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    dialog.connect_response(None, move |dialog, response| {
        if response == "remove" {
            let _ = command_tx.send(AppCommand::DeleteModel(model_id.clone()));
        }
        dialog.close();
    });
    dialog.present();
}

fn model_subtitle(state: &ModelRowState) -> String {
    format!("{} · {}", state.entry.size_label, state.status.label())
}
