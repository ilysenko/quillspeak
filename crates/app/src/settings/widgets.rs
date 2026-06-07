use std::env;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{
    AUTO_LANGUAGE_VALUE, ComputeBackend, DaemonStatus, HotkeyBackend, INHERIT_VALUE, MODEL_CATALOG,
    ModelCatalogEntry, SUPPORTED_LANGUAGES, model_catalog_entry, supported_language_label,
};

pub struct DropDownRow {
    pub row: adw::ActionRow,
    pub dropdown: gtk::DropDown,
}

pub struct ValueDropDownRow {
    pub row: adw::ActionRow,
    pub dropdown: gtk::DropDown,
    pub values: Vec<String>,
}

pub fn preferences_page(title: &str) -> adw::PreferencesPage {
    adw::PreferencesPage::builder().title(title).build()
}

pub fn property_row(title: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(value)
        .build();
    row.add_css_class("property");
    row
}

pub fn dropdown_row(title: &str, labels: &[&str], selected: u32) -> DropDownRow {
    let row = adw::ActionRow::builder().title(title).build();
    let dropdown = gtk::DropDown::from_strings(labels);
    dropdown.set_selected(selected);
    dropdown.set_valign(gtk::Align::Center);
    row.add_suffix(&dropdown);
    row.set_activatable_widget(Some(&dropdown));
    DropDownRow { row, dropdown }
}

pub fn model_dropdown_row(
    title: &str,
    models: &[ModelCatalogEntry],
    selected_model_id: &str,
) -> ValueDropDownRow {
    let values = models
        .iter()
        .map(|entry| entry.id.to_string())
        .collect::<Vec<_>>();
    let labels = models.iter().map(|entry| entry.label).collect::<Vec<_>>();
    value_dropdown_row(title, labels, values, selected_model_id)
}

pub fn all_model_entries() -> Vec<ModelCatalogEntry> {
    MODEL_CATALOG
        .iter()
        .filter_map(|entry| model_catalog_entry(entry.id))
        .collect()
}

pub fn shortcut_model_dropdown_row(
    title: &str,
    models: &[ModelCatalogEntry],
    selected_model_id: &str,
) -> ValueDropDownRow {
    let mut labels = vec!["Default".to_string()];
    let mut values = vec![INHERIT_VALUE.to_string()];
    for model in models {
        labels.push(model.label.to_string());
        values.push(model.id.to_string());
    }
    if selected_model_id != INHERIT_VALUE && !values.iter().any(|value| value == selected_model_id)
    {
        labels.push(format!("Missing: {selected_model_id}"));
        values.push(selected_model_id.to_string());
    }
    value_dropdown_row_owned(title, labels, values, selected_model_id)
}

pub fn language_dropdown_row(
    title: &str,
    allow_default: bool,
    selected_language: &str,
) -> ValueDropDownRow {
    let mut labels = Vec::new();
    let mut values = Vec::new();
    if allow_default {
        labels.push("Default".to_string());
        values.push(INHERIT_VALUE.to_string());
    }
    labels.push("Auto Detect".to_string());
    values.push(AUTO_LANGUAGE_VALUE.to_string());
    for language in SUPPORTED_LANGUAGES {
        labels.push(language.label.to_string());
        values.push(language.code.to_string());
    }
    value_dropdown_row_owned(title, labels, values, selected_language)
}

fn value_dropdown_row(
    title: &str,
    labels: Vec<&str>,
    values: Vec<String>,
    selected_value: &str,
) -> ValueDropDownRow {
    value_dropdown_row_owned(
        title,
        labels.into_iter().map(ToString::to_string).collect(),
        values,
        selected_value,
    )
}

fn value_dropdown_row_owned(
    title: &str,
    labels: Vec<String>,
    values: Vec<String>,
    selected_value: &str,
) -> ValueDropDownRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(
            supported_language_label(selected_value)
                .or_else(|| model_catalog_entry(selected_value).map(|entry| entry.label))
                .unwrap_or(selected_value),
        )
        .build();
    let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    let dropdown = gtk::DropDown::from_strings(&label_refs);
    let selected = values
        .iter()
        .position(|value| value == selected_value)
        .unwrap_or(0) as u32;
    dropdown.set_selected(selected);
    dropdown.set_valign(gtk::Align::Center);
    row.add_suffix(&dropdown);
    row.set_activatable_widget(Some(&dropdown));
    ValueDropDownRow {
        row,
        dropdown,
        values,
    }
}

pub fn backend_index(backend: HotkeyBackend) -> u32 {
    match backend {
        HotkeyBackend::Auto => 0,
        HotkeyBackend::Disabled => 1,
        HotkeyBackend::X11 => 2,
        HotkeyBackend::Daemon => 3,
        HotkeyBackend::Portal => 0,
    }
}

pub fn backend_from_index(index: u32) -> HotkeyBackend {
    match index {
        1 => HotkeyBackend::Disabled,
        2 => HotkeyBackend::X11,
        3 => HotkeyBackend::Daemon,
        _ => HotkeyBackend::Auto,
    }
}

pub fn compute_index(backend: ComputeBackend) -> u32 {
    match backend {
        ComputeBackend::Auto => 0,
        ComputeBackend::Cpu => 1,
        ComputeBackend::Vulkan => 2,
        ComputeBackend::Cuda => 3,
        ComputeBackend::Rocm => 4,
        ComputeBackend::OpenVino => 5,
    }
}

pub fn compute_from_index(index: u32) -> ComputeBackend {
    match index {
        1 => ComputeBackend::Cpu,
        2 => ComputeBackend::Vulkan,
        3 => ComputeBackend::Cuda,
        4 => ComputeBackend::Rocm,
        5 => ComputeBackend::OpenVino,
        _ => ComputeBackend::Auto,
    }
}

pub fn advanced_hotkey_status(daemon_status: DaemonStatus) -> &'static str {
    match daemon_status {
        DaemonStatus::RunningConfigured => return "Daemon backend available",
        DaemonStatus::RunningUnconfigured => return "Daemon running, shortcut unavailable",
        DaemonStatus::PermissionError => return "Daemon permission error",
        DaemonStatus::NotInstalled | DaemonStatus::InstalledButNotRunning => {}
    }

    if env::var_os("WAYLAND_DISPLAY").is_some() {
        "Unavailable on Wayland without daemon"
    } else if env::var_os("DISPLAY").is_some() {
        "X11 backend available in app"
    } else {
        "Disabled"
    }
}
