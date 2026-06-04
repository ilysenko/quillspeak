use anyhow::{Result, anyhow, bail};
use eframe::egui::{Key, KeyboardShortcut, Modifiers};
use egui_keybind::Shortcut;

use crate::hotkey::HotkeySpec;

pub(super) fn shortcut_from_hotkey(hotkey: &str) -> Result<Shortcut> {
    let spec = HotkeySpec::parse(hotkey)?;
    let mut parts = spec.canonical().split('+').collect::<Vec<_>>();
    let key = parts
        .pop()
        .ok_or_else(|| anyhow!("hotkey must include a key"))?;
    let mut modifiers = Modifiers::NONE;

    for modifier in parts {
        match modifier {
            "Ctrl" => modifiers.ctrl = true,
            "Alt" => modifiers.alt = true,
            "Shift" => modifiers.shift = true,
            "Super" => bail!("Super shortcuts cannot be edited in egui yet"),
            other => bail!("unsupported modifier `{other}`"),
        }
    }

    Ok(Shortcut::new(
        Some(KeyboardShortcut::new(
            modifiers,
            egui_key_from_hotkey_key(key)?,
        )),
        None,
    ))
}

pub(super) fn hotkey_from_shortcut(shortcut: &Shortcut) -> Result<String> {
    if shortcut.pointer().is_some() {
        bail!("mouse buttons are not supported");
    }
    let keyboard = shortcut
        .keyboard()
        .ok_or_else(|| anyhow!("shortcut must include a keyboard key"))?;

    let mut parts = Vec::new();
    if keyboard.modifiers.ctrl || keyboard.modifiers.command {
        parts.push("Ctrl");
    }
    if keyboard.modifiers.alt {
        parts.push("Alt");
    }
    if keyboard.modifiers.shift {
        parts.push("Shift");
    }
    if keyboard.modifiers.mac_cmd {
        bail!("Super shortcuts cannot be edited in egui yet");
    }
    parts.push(hotkey_key_from_egui_key(keyboard.logical_key)?);

    let candidate = parts.join("+");
    Ok(HotkeySpec::parse(&candidate)?.canonical().to_string())
}

fn egui_key_from_hotkey_key(key: &str) -> Result<Key> {
    match key {
        "Space" => Ok(Key::Space),
        "Enter" => Ok(Key::Enter),
        "Escape" => Ok(Key::Escape),
        "Tab" => Ok(Key::Tab),
        "Backspace" => Ok(Key::Backspace),
        "Delete" => Ok(Key::Delete),
        "Insert" => Ok(Key::Insert),
        "Home" => Ok(Key::Home),
        "End" => Ok(Key::End),
        "PageUp" => Ok(Key::PageUp),
        "PageDown" => Ok(Key::PageDown),
        "Left" => Ok(Key::ArrowLeft),
        "Right" => Ok(Key::ArrowRight),
        "Up" => Ok(Key::ArrowUp),
        "Down" => Ok(Key::ArrowDown),
        "A" => Ok(Key::A),
        "B" => Ok(Key::B),
        "C" => Ok(Key::C),
        "D" => Ok(Key::D),
        "E" => Ok(Key::E),
        "F" => Ok(Key::F),
        "G" => Ok(Key::G),
        "H" => Ok(Key::H),
        "I" => Ok(Key::I),
        "J" => Ok(Key::J),
        "K" => Ok(Key::K),
        "L" => Ok(Key::L),
        "M" => Ok(Key::M),
        "N" => Ok(Key::N),
        "O" => Ok(Key::O),
        "P" => Ok(Key::P),
        "Q" => Ok(Key::Q),
        "R" => Ok(Key::R),
        "S" => Ok(Key::S),
        "T" => Ok(Key::T),
        "U" => Ok(Key::U),
        "V" => Ok(Key::V),
        "W" => Ok(Key::W),
        "X" => Ok(Key::X),
        "Y" => Ok(Key::Y),
        "Z" => Ok(Key::Z),
        "0" => Ok(Key::Num0),
        "1" => Ok(Key::Num1),
        "2" => Ok(Key::Num2),
        "3" => Ok(Key::Num3),
        "4" => Ok(Key::Num4),
        "5" => Ok(Key::Num5),
        "6" => Ok(Key::Num6),
        "7" => Ok(Key::Num7),
        "8" => Ok(Key::Num8),
        "9" => Ok(Key::Num9),
        "F1" => Ok(Key::F1),
        "F2" => Ok(Key::F2),
        "F3" => Ok(Key::F3),
        "F4" => Ok(Key::F4),
        "F5" => Ok(Key::F5),
        "F6" => Ok(Key::F6),
        "F7" => Ok(Key::F7),
        "F8" => Ok(Key::F8),
        "F9" => Ok(Key::F9),
        "F10" => Ok(Key::F10),
        "F11" => Ok(Key::F11),
        "F12" => Ok(Key::F12),
        "F13" => Ok(Key::F13),
        "F14" => Ok(Key::F14),
        "F15" => Ok(Key::F15),
        "F16" => Ok(Key::F16),
        "F17" => Ok(Key::F17),
        "F18" => Ok(Key::F18),
        "F19" => Ok(Key::F19),
        "F20" => Ok(Key::F20),
        "F21" => Ok(Key::F21),
        "F22" => Ok(Key::F22),
        "F23" => Ok(Key::F23),
        "F24" => Ok(Key::F24),
        other => bail!("unsupported key `{other}`"),
    }
}

fn hotkey_key_from_egui_key(key: Key) -> Result<&'static str> {
    match key {
        Key::Space => Ok("Space"),
        Key::Enter => Ok("Enter"),
        Key::Escape => Ok("Escape"),
        Key::Tab => Ok("Tab"),
        Key::Backspace => Ok("Backspace"),
        Key::Delete => Ok("Delete"),
        Key::Insert => Ok("Insert"),
        Key::Home => Ok("Home"),
        Key::End => Ok("End"),
        Key::PageUp => Ok("PageUp"),
        Key::PageDown => Ok("PageDown"),
        Key::ArrowLeft => Ok("Left"),
        Key::ArrowRight => Ok("Right"),
        Key::ArrowUp => Ok("Up"),
        Key::ArrowDown => Ok("Down"),
        Key::A => Ok("A"),
        Key::B => Ok("B"),
        Key::C => Ok("C"),
        Key::D => Ok("D"),
        Key::E => Ok("E"),
        Key::F => Ok("F"),
        Key::G => Ok("G"),
        Key::H => Ok("H"),
        Key::I => Ok("I"),
        Key::J => Ok("J"),
        Key::K => Ok("K"),
        Key::L => Ok("L"),
        Key::M => Ok("M"),
        Key::N => Ok("N"),
        Key::O => Ok("O"),
        Key::P => Ok("P"),
        Key::Q => Ok("Q"),
        Key::R => Ok("R"),
        Key::S => Ok("S"),
        Key::T => Ok("T"),
        Key::U => Ok("U"),
        Key::V => Ok("V"),
        Key::W => Ok("W"),
        Key::X => Ok("X"),
        Key::Y => Ok("Y"),
        Key::Z => Ok("Z"),
        Key::Num0 => Ok("0"),
        Key::Num1 => Ok("1"),
        Key::Num2 => Ok("2"),
        Key::Num3 => Ok("3"),
        Key::Num4 => Ok("4"),
        Key::Num5 => Ok("5"),
        Key::Num6 => Ok("6"),
        Key::Num7 => Ok("7"),
        Key::Num8 => Ok("8"),
        Key::Num9 => Ok("9"),
        Key::F1 => Ok("F1"),
        Key::F2 => Ok("F2"),
        Key::F3 => Ok("F3"),
        Key::F4 => Ok("F4"),
        Key::F5 => Ok("F5"),
        Key::F6 => Ok("F6"),
        Key::F7 => Ok("F7"),
        Key::F8 => Ok("F8"),
        Key::F9 => Ok("F9"),
        Key::F10 => Ok("F10"),
        Key::F11 => Ok("F11"),
        Key::F12 => Ok("F12"),
        Key::F13 => Ok("F13"),
        Key::F14 => Ok("F14"),
        Key::F15 => Ok("F15"),
        Key::F16 => Ok("F16"),
        Key::F17 => Ok("F17"),
        Key::F18 => Ok("F18"),
        Key::F19 => Ok("F19"),
        Key::F20 => Ok("F20"),
        Key::F21 => Ok("F21"),
        Key::F22 => Ok("F22"),
        Key::F23 => Ok("F23"),
        Key::F24 => Ok("F24"),
        other => bail!("unsupported key `{}`", other.name()),
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{KeyboardShortcut, Modifiers};

    use super::*;

    #[test]
    fn parses_default_hotkey_to_egui_shortcut() {
        let shortcut = shortcut_from_hotkey("Ctrl+Alt+Space").unwrap();
        let keyboard = shortcut.keyboard().unwrap();

        assert!(keyboard.modifiers.ctrl);
        assert!(keyboard.modifiers.alt);
        assert_eq!(keyboard.logical_key, Key::Space);
    }

    #[test]
    fn converts_egui_shortcut_to_canonical_hotkey() {
        let shortcut = Shortcut::new(
            Some(KeyboardShortcut::new(
                Modifiers::CTRL | Modifiers::SHIFT,
                Key::A,
            )),
            None,
        );

        assert_eq!(hotkey_from_shortcut(&shortcut).unwrap(), "Ctrl+Shift+A");
    }

    #[test]
    fn converts_function_keys() {
        let shortcut = Shortcut::new(Some(KeyboardShortcut::new(Modifiers::ALT, Key::F12)), None);

        assert_eq!(hotkey_from_shortcut(&shortcut).unwrap(), "Alt+F12");
    }

    #[test]
    fn rejects_super_until_egui_exposes_logo_modifier() {
        assert!(shortcut_from_hotkey("Super+F1").is_err());
    }
}
