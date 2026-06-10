use std::sync::mpsc;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::command::AppCommand;
use crate::history::HistoryEntry;
use crate::settings::widgets::preferences_page;

#[derive(Clone)]
pub struct HistoryPage {
    page: adw::PreferencesPage,
}

impl HistoryPage {
    pub fn widget(&self) -> &adw::PreferencesPage {
        &self.page
    }
}

pub fn build(entries: Vec<HistoryEntry>, command_tx: mpsc::Sender<AppCommand>) -> HistoryPage {
    let page = preferences_page("History");
    let group = adw::PreferencesGroup::builder()
        .title("Transcriptions")
        .build();

    let clear_row = adw::ActionRow::builder()
        .title("Clear History")
        .subtitle("Remove saved transcription history from this computer.")
        .build();
    let clear_button = gtk::Button::builder()
        .label("Clear History")
        .valign(gtk::Align::Center)
        .build();
    clear_button.add_css_class("destructive-action");
    clear_button.connect_clicked({
        let command_tx = command_tx.clone();
        move |button| confirm_clear_history(button, command_tx.clone())
    });
    clear_row.add_suffix(&clear_button);
    group.add(&clear_row);

    if entries.is_empty() {
        group.add(
            &adw::ActionRow::builder()
                .title("No history yet")
                .subtitle("Recognized text will appear here after transcription.")
                .build(),
        );
    } else {
        for entry in entries.iter().rev() {
            group.add(&history_row(entry));
        }
    }

    page.add(&group);
    HistoryPage { page }
}

fn history_row(entry: &HistoryEntry) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(format!(
            "{} - {} - {}",
            format_timestamp(entry.created_at_unix_ms),
            entry.shortcut_name,
            entry.source.label()
        ))
        .subtitle(format!(
            "{}\nModel: {}   Language: {}",
            entry.text, entry.model_id, entry.language
        ))
        .build()
}

fn confirm_clear_history(parent: &gtk::Button, command_tx: mpsc::Sender<AppCommand>) {
    let parent_window = parent
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok());
    let dialog = adw::MessageDialog::builder()
        .heading("Clear history?")
        .body("Remove all saved transcription history from this computer?")
        .close_response("no")
        .default_response("no")
        .build();
    dialog.set_transient_for(parent_window.as_ref());
    dialog.add_responses(&[("no", "No"), ("yes", "Yes")]);
    dialog.set_response_appearance("yes", adw::ResponseAppearance::Destructive);
    dialog.connect_response(None, move |dialog, response| {
        if response == "yes" {
            let _ = command_tx.send(AppCommand::ClearHistory);
        }
        dialog.close();
    });
    dialog.present();
}

fn format_timestamp(created_at_unix_ms: u128) -> String {
    let Some(seconds) = i64::try_from(created_at_unix_ms / 1000).ok() else {
        return created_at_unix_ms.to_string();
    };
    gtk::glib::DateTime::from_unix_local(seconds)
        .and_then(|date_time| date_time.format("%Y-%m-%d %H:%M:%S"))
        .map(|formatted| formatted.to_string())
        .unwrap_or_else(|_| created_at_unix_ms.to_string())
}
