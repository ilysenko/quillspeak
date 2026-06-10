use std::env;

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::{
    AUTO_LANGUAGE_VALUE, ComputeBackend, HotkeyBackend, ModelCatalogEntry, SUPPORTED_LANGUAGES,
    model_catalog_entry, supported_language_label,
};

#[derive(Clone)]
pub struct DropDownRow {
    pub row: adw::ActionRow,
    pub dropdown: gtk::DropDown,
}

#[derive(Clone)]
pub struct ValueDropDownRow {
    pub row: adw::ActionRow,
    pub dropdown: gtk::DropDown,
    pub values: Vec<String>,
}

#[derive(Clone)]
pub struct TextRow {
    pub row: adw::ActionRow,
    pub entry: gtk::Entry,
}

#[derive(Clone)]
pub struct SliderRow {
    pub row: adw::ActionRow,
    pub scale: gtk::Scale,
}

pub fn preferences_page(title: &str) -> adw::PreferencesPage {
    adw::PreferencesPage::builder().title(title).build()
}

pub fn scrollable_page<W>(page: &W) -> gtk::ScrolledWindow
where
    W: gtk::glib::object::IsA<gtk::Widget>,
{
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .child(page)
        .build()
}

pub fn property_row(title: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(value)
        .build();
    row.add_css_class("property");
    row
}

pub fn dropdown_row_with_help(
    title: &str,
    help: &str,
    labels: &[&str],
    selected: u32,
) -> DropDownRow {
    let row = action_row(title, help);
    let dropdown = gtk::DropDown::from_strings(labels);
    dropdown.set_selected(selected);
    dropdown.set_valign(gtk::Align::Center);
    row.add_suffix(&dropdown);
    row.set_activatable_widget(Some(&dropdown));
    DropDownRow { row, dropdown }
}

pub fn switch_row(title: &str, help: &str, active: bool) -> (adw::ActionRow, gtk::Switch) {
    let row = action_row(title, help);
    let switch = gtk::Switch::builder()
        .active(active)
        .valign(gtk::Align::Center)
        .build();
    row.add_suffix(&switch);
    row.set_activatable_widget(Some(&switch));
    (row, switch)
}

pub fn text_row(title: &str, help: &str, text: &str) -> TextRow {
    let row = action_row(title, help);
    let entry = gtk::Entry::builder()
        .text(text)
        .valign(gtk::Align::Center)
        .hexpand(false)
        .width_chars(24)
        .build();
    row.add_suffix(&entry);
    row.set_activatable_widget(Some(&entry));
    TextRow { row, entry }
}

pub fn percent_slider_row(
    title: &str,
    help: &str,
    value: u8,
    min: u8,
    max: u8,
    step: u8,
) -> SliderRow {
    let row = action_row(title, help);
    let value = value.clamp(min, max);
    let scale = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        f64::from(min),
        f64::from(max),
        f64::from(step),
    );
    scale.set_value(f64::from(value));
    scale.set_digits(0);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    scale.set_width_request(220);
    scale.set_valign(gtk::Align::Center);
    row.add_suffix(&scale);
    row.set_activatable_widget(Some(&scale));
    SliderRow { row, scale }
}

pub fn shortcut_model_dropdown_row(
    title: &str,
    help: &str,
    models: &[ModelCatalogEntry],
    selected_model_id: &str,
) -> ValueDropDownRow {
    let mut labels = Vec::new();
    let mut values = Vec::new();
    for model in models {
        labels.push(model.label.to_string());
        values.push(model.id.to_string());
    }
    if !selected_model_id.trim().is_empty()
        && !values.iter().any(|value| value == selected_model_id)
    {
        labels.push(format!("Missing: {selected_model_id}"));
        values.push(selected_model_id.to_string());
    }
    value_dropdown_row_owned(title, help, labels, values, selected_model_id)
}

pub fn language_dropdown_row(title: &str, help: &str, selected_language: &str) -> ValueDropDownRow {
    let mut labels = Vec::new();
    let mut values = Vec::new();
    labels.push("Auto Detect".to_string());
    values.push(AUTO_LANGUAGE_VALUE.to_string());
    for language in SUPPORTED_LANGUAGES {
        labels.push(language.label.to_string());
        values.push(language.code.to_string());
    }
    value_dropdown_row_owned(title, help, labels, values, selected_language)
}

pub fn value_dropdown_row(
    title: &str,
    help: &str,
    labels: Vec<String>,
    values: Vec<String>,
    selected_value: &str,
) -> ValueDropDownRow {
    value_dropdown_row_owned(title, help, labels, values, selected_value)
}

fn value_dropdown_row_owned(
    title: &str,
    help: &str,
    labels: Vec<String>,
    values: Vec<String>,
    selected_value: &str,
) -> ValueDropDownRow {
    let selected_label = supported_language_label(selected_value)
        .or_else(|| model_catalog_entry(selected_value).map(|entry| entry.label))
        .unwrap_or(selected_value);
    let subtitle = if help.trim().is_empty() {
        selected_label.to_string()
    } else {
        format!("{help}\nCurrent: {selected_label}")
    };
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(&subtitle)
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

fn action_row(title: &str, help: &str) -> adw::ActionRow {
    let builder = adw::ActionRow::builder().title(title);
    if help.trim().is_empty() {
        builder.build()
    } else {
        builder.subtitle(help).build()
    }
}

pub fn backend_index(backend: HotkeyBackend) -> u32 {
    match backend {
        HotkeyBackend::Auto => 0,
        HotkeyBackend::Disabled => 1,
        HotkeyBackend::X11 => 2,
    }
}

pub fn backend_from_index(index: u32) -> HotkeyBackend {
    match index {
        1 => HotkeyBackend::Disabled,
        2 => HotkeyBackend::X11,
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
    }
}

pub fn compute_from_index(index: u32) -> ComputeBackend {
    match index {
        1 => ComputeBackend::Cpu,
        2 => ComputeBackend::Vulkan,
        3 => ComputeBackend::Cuda,
        4 => ComputeBackend::Rocm,
        _ => ComputeBackend::Auto,
    }
}

pub fn advanced_hotkey_status() -> &'static str {
    if env::var_os("WAYLAND_DISPLAY").is_some() {
        "Keyboard shortcuts disabled; Linux signals active"
    } else if env::var_os("DISPLAY").is_some() {
        "Keyboard shortcuts: X11"
    } else {
        "Keyboard shortcuts disabled; Linux signals active"
    }
}

pub fn output_tools_status() -> String {
    if env::var_os("WAYLAND_DISPLAY").is_some()
        || env::var_os("XDG_SESSION_TYPE")
            .and_then(|value| value.into_string().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("wayland"))
    {
        tool_status(&["wl-copy", "wl-paste", "ydotool"])
    } else if env::var_os("DISPLAY").is_some() {
        tool_status(&["xclip", "xdotool"])
    } else {
        "No display backend detected".to_string()
    }
}

fn tool_status(commands: &[&str]) -> String {
    let missing = commands
        .iter()
        .copied()
        .filter(|command| !command_in_path(command))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        format!("Ready: {}", commands.join(", "))
    } else {
        format!("Missing: {}", missing.join(", "))
    }
}

fn command_in_path(command: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|entry| entry.join(command).is_file())
}
