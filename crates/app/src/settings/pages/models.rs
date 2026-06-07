use std::sync::mpsc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::command::AppCommand;
use crate::models::{ModelRowState, ModelStatus};
use crate::settings::widgets::preferences_page;

pub fn build(
    model_states: Vec<ModelRowState>,
    command_tx: mpsc::Sender<AppCommand>,
) -> adw::PreferencesPage {
    let page = preferences_page("Models");
    let group = adw::PreferencesGroup::builder()
        .title("Whisper.cpp")
        .build();

    for state in model_states {
        let row = adw::ActionRow::builder()
            .title(state.entry.label)
            .subtitle(format!(
                "{} · {}",
                state.entry.size_label,
                state.status.label()
            ))
            .build();
        let action = match state.status {
            ModelStatus::Ready => {
                let button = gtk::Button::builder()
                    .label("Delete")
                    .valign(gtk::Align::Center)
                    .sensitive(!state.referenced)
                    .build();
                button.connect_clicked({
                    let command_tx = command_tx.clone();
                    let model_id = state.entry.id.to_string();
                    move |_| {
                        let _ = command_tx.send(AppCommand::DeleteModel(model_id.clone()));
                    }
                });
                button
            }
            ModelStatus::Downloading { .. } => gtk::Button::builder()
                .label("Downloading")
                .valign(gtk::Align::Center)
                .sensitive(false)
                .build(),
            ModelStatus::NotInstalled | ModelStatus::Error(_) => {
                let button = gtk::Button::builder()
                    .label("Download")
                    .valign(gtk::Align::Center)
                    .build();
                button.connect_clicked({
                    let command_tx = command_tx.clone();
                    let model_id = state.entry.id.to_string();
                    move |_| {
                        let _ = command_tx.send(AppCommand::DownloadModel(model_id.clone()));
                    }
                });
                button
            }
        };
        row.add_suffix(&action);
        group.add(&row);
    }

    page.add(&group);
    page
}
