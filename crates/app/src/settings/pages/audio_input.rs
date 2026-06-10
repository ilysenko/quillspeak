use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::AudioInputRef;

use crate::audio::AudioInputDevice;

pub struct AudioInputDropDownRow {
    pub row: adw::ActionRow,
    pub dropdown: gtk::DropDown,
    pub values: Vec<AudioInputRef>,
}

pub fn audio_input_dropdown_row(
    devices: &[AudioInputDevice],
    selected_input: &AudioInputRef,
) -> AudioInputDropDownRow {
    let mut labels = devices
        .iter()
        .map(|device| device.label.clone())
        .collect::<Vec<_>>();
    let mut values = devices
        .iter()
        .map(|device| device.reference.clone())
        .collect::<Vec<_>>();
    let selected_key = selected_input.stable_key();
    if !values
        .iter()
        .any(|input| input.stable_key() == selected_key)
    {
        labels.push(format!("Missing: {}", selected_input.display_label()));
        values.push(selected_input.clone());
    }

    let row = adw::ActionRow::builder()
        .title("Audio input")
        .subtitle(format!(
            "Microphone source used for every recording. System Default is resolved at recording time.\nCurrent: {}",
            selected_input.display_label()
        ))
        .build();
    let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    let dropdown = gtk::DropDown::from_strings(&label_refs);
    let selected = values
        .iter()
        .position(|value| value.stable_key() == selected_key)
        .unwrap_or(0) as u32;
    dropdown.set_selected(selected);
    dropdown.set_valign(gtk::Align::Center);
    row.add_suffix(&dropdown);
    row.set_activatable_widget(Some(&dropdown));
    AudioInputDropDownRow {
        row,
        dropdown,
        values,
    }
}
