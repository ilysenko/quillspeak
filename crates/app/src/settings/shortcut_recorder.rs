use gtk::gdk;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use shared::normalize_accelerator;

pub fn connect_record_button(record_button: &gtk::Button, shortcut_entry: &adw::EntryRow) {
    let shortcut_entry = shortcut_entry.clone();
    record_button.connect_clicked(move |_| {
        show_shortcut_recorder(&shortcut_entry);
    });
}

fn show_shortcut_recorder(shortcut_entry: &adw::EntryRow) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .focusable(true)
        .build();
    let title = gtk::Label::builder()
        .label("Press a shortcut")
        .halign(gtk::Align::Start)
        .build();
    title.add_css_class("title-3");
    let status_label = gtk::Label::builder()
        .label("Waiting for key combination")
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    let clear_button = gtk::Button::builder().label("Clear").build();
    let cancel_button = gtk::Button::builder().label("Cancel").build();
    actions.append(&clear_button);
    actions.append(&cancel_button);
    content.append(&title);
    content.append(&status_label);
    content.append(&actions);

    let recorder = gtk::Window::builder()
        .title("Record Shortcut")
        .modal(true)
        .default_width(360)
        .default_height(160)
        .child(&content)
        .build();

    clear_button.connect_clicked({
        let recorder = recorder.clone();
        let shortcut_entry = shortcut_entry.clone();
        move |_| {
            shortcut_entry.set_text("");
            recorder.close();
        }
    });

    cancel_button.connect_clicked({
        let recorder = recorder.clone();
        move |_| recorder.close()
    });

    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    controller.connect_key_pressed({
        let recorder = recorder.clone();
        let shortcut_entry = shortcut_entry.clone();
        let status_label = status_label.clone();
        move |_, keyval, _, state| {
            if keyval == gdk::Key::Escape {
                recorder.close();
                return gtk::glib::Propagation::Stop;
            }

            match shortcut_from_key_event(keyval, state) {
                Ok(accelerator) => {
                    shortcut_entry.set_text(&accelerator);
                    recorder.close();
                }
                Err(message) => {
                    status_label.set_text(&message);
                }
            }

            gtk::glib::Propagation::Stop
        }
    });
    recorder.add_controller(controller);

    recorder.present();
    content.grab_focus();
}

fn shortcut_from_key_event(keyval: gdk::Key, state: gdk::ModifierType) -> Result<String, String> {
    if is_modifier_key(keyval) {
        return Err("Press a non-modifier key as part of the shortcut".to_string());
    }

    let modifiers = significant_modifiers(state);
    let key = key_name_for_config(keyval).ok_or_else(|| "Unsupported shortcut key".to_string())?;
    let mut parts = Vec::new();
    if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
        parts.push("Ctrl".to_string());
    }
    if modifiers.contains(gdk::ModifierType::ALT_MASK) {
        parts.push("Alt".to_string());
    }
    if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
        parts.push("Shift".to_string());
    }
    if modifiers.contains(gdk::ModifierType::SUPER_MASK) {
        parts.push("Super".to_string());
    }
    parts.push(key);

    normalize_accelerator(&parts.join("+")).map_err(|error| error.to_string())
}

fn significant_modifiers(state: gdk::ModifierType) -> gdk::ModifierType {
    let mut modifiers = gdk::ModifierType::empty();
    for mask in [
        gdk::ModifierType::CONTROL_MASK,
        gdk::ModifierType::ALT_MASK,
        gdk::ModifierType::SHIFT_MASK,
        gdk::ModifierType::SUPER_MASK,
    ] {
        if state.contains(mask) {
            modifiers.insert(mask);
        }
    }
    modifiers
}

fn is_modifier_key(keyval: gdk::Key) -> bool {
    matches!(
        keyval,
        gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Shift_Lock
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::Meta_L
            | gdk::Key::Meta_R
    )
}

fn key_name_for_config(keyval: gdk::Key) -> Option<String> {
    if keyval == gdk::Key::Return || keyval == gdk::Key::KP_Enter {
        return Some("Enter".to_string());
    }
    if keyval == gdk::Key::space {
        return Some("Space".to_string());
    }

    if let Some(character) = keyval.to_unicode()
        && !character.is_control()
    {
        return Some(character.to_ascii_uppercase().to_string());
    }

    keyval.name().map(|name| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_ctrl_space() {
        assert_eq!(
            shortcut_from_key_event(gdk::Key::space, gdk::ModifierType::CONTROL_MASK),
            Ok("Ctrl+Space".to_string())
        );
    }

    #[test]
    fn records_ctrl_alt_space() {
        assert_eq!(
            shortcut_from_key_event(
                gdk::Key::space,
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK
            ),
            Ok("Ctrl+Alt+Space".to_string())
        );
    }

    #[test]
    fn rejects_modifier_only_shortcut() {
        assert_eq!(
            shortcut_from_key_event(gdk::Key::Control_L, gdk::ModifierType::CONTROL_MASK),
            Err("Press a non-modifier key as part of the shortcut".to_string())
        );
    }
}
