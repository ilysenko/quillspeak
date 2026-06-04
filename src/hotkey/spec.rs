use std::borrow::Cow;
use std::str::FromStr;

use anyhow::{Result, anyhow, bail};
use evdev::KeyCode;
use global_hotkey::hotkey::HotKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeySpec {
    canonical: String,
    xdg_trigger: String,
    evdev_hotkey: EvdevHotkeySpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EvdevHotkeySpec {
    modifier_groups: Vec<EvdevKeyGroup>,
    main_key: KeyCode,
}

impl EvdevHotkeySpec {
    pub(super) fn modifier_groups(&self) -> &[EvdevKeyGroup] {
        &self.modifier_groups
    }

    pub(super) fn main_key(&self) -> KeyCode {
        self.main_key
    }

    pub(super) fn relevant_keys(&self) -> Vec<KeyCode> {
        let mut keys = Vec::new();
        for group in &self.modifier_groups {
            for key in group.keys() {
                if !keys.contains(key) {
                    keys.push(*key);
                }
            }
        }
        if !keys.contains(&self.main_key) {
            keys.push(self.main_key);
        }
        keys
    }

    pub(super) fn contains_key(&self, key: KeyCode) -> bool {
        self.main_key == key
            || self
                .modifier_groups
                .iter()
                .any(|group| group.keys().contains(&key))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EvdevKeyGroup {
    keys: Vec<KeyCode>,
}

impl EvdevKeyGroup {
    fn new(keys: Vec<KeyCode>) -> Self {
        Self { keys }
    }

    pub(super) fn keys(&self) -> &[KeyCode] {
        &self.keys
    }
}

impl HotkeySpec {
    pub fn parse(input: &str) -> Result<Self> {
        input.parse()
    }

    pub fn canonical(&self) -> &str {
        &self.canonical
    }

    #[allow(dead_code)]
    pub fn xdg_trigger(&self) -> &str {
        &self.xdg_trigger
    }

    pub(super) fn to_evdev_hotkey(&self) -> EvdevHotkeySpec {
        self.evdev_hotkey.clone()
    }

    pub(super) fn to_global_hotkey(&self) -> Result<HotKey> {
        HotKey::from_str(&self.canonical)
            .map_err(|error| anyhow!("failed to parse X11 hotkey `{}`: {error}", self.canonical))
    }
}

impl FromStr for HotkeySpec {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            bail!("push-to-talk hotkey cannot be empty");
        }

        let mut modifiers = Modifiers::default();
        let mut key = None;

        for raw_token in input.split('+') {
            let token = raw_token.trim();
            if token.is_empty() {
                bail!("hotkey `{input}` contains an empty token");
            }

            if let Some(modifier) = Modifier::parse(token) {
                if key.is_some() {
                    bail!("hotkey `{input}` must list modifiers before the key");
                }
                modifiers.insert(modifier, input)?;
                continue;
            }

            if key.is_some() {
                bail!("hotkey `{input}` contains more than one key");
            }
            key = Some(parse_key(token).ok_or_else(|| {
                anyhow!("unsupported key `{token}` in push-to-talk hotkey `{input}`")
            })?);
        }

        let key = key.ok_or_else(|| anyhow!("hotkey `{input}` must include a non-modifier key"))?;

        let mut canonical_parts = modifiers.canonical_parts();
        canonical_parts.push(key.canonical);

        let mut xdg_parts = modifiers.xdg_parts();
        xdg_parts.push(key.xdg);
        let evdev_key = evdev_key_for_canonical(key.canonical)
            .expect("parsed hotkey key should have an evdev mapping");

        Ok(Self {
            canonical: canonical_parts.join("+"),
            xdg_trigger: xdg_parts.join("+"),
            evdev_hotkey: EvdevHotkeySpec {
                modifier_groups: modifiers.evdev_groups(),
                main_key: evdev_key,
            },
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Modifiers {
    ctrl: bool,
    alt: bool,
    shift: bool,
    logo: bool,
}

impl Modifiers {
    fn insert(&mut self, modifier: Modifier, input: &str) -> Result<()> {
        let slot = match modifier {
            Modifier::Ctrl => &mut self.ctrl,
            Modifier::Alt => &mut self.alt,
            Modifier::Shift => &mut self.shift,
            Modifier::Logo => &mut self.logo,
        };

        if *slot {
            bail!("duplicate modifier in hotkey `{input}`");
        }

        *slot = true;
        Ok(())
    }

    fn canonical_parts(self) -> Vec<&'static str> {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.logo {
            parts.push("Super");
        }
        parts
    }

    fn xdg_parts(self) -> Vec<&'static str> {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("CTRL");
        }
        if self.alt {
            parts.push("ALT");
        }
        if self.shift {
            parts.push("SHIFT");
        }
        if self.logo {
            parts.push("LOGO");
        }
        parts
    }

    fn evdev_groups(self) -> Vec<EvdevKeyGroup> {
        let mut groups = Vec::new();
        if self.ctrl {
            groups.push(EvdevKeyGroup::new(vec![
                KeyCode::KEY_LEFTCTRL,
                KeyCode::KEY_RIGHTCTRL,
            ]));
        }
        if self.alt {
            groups.push(EvdevKeyGroup::new(vec![
                KeyCode::KEY_LEFTALT,
                KeyCode::KEY_RIGHTALT,
            ]));
        }
        if self.shift {
            groups.push(EvdevKeyGroup::new(vec![
                KeyCode::KEY_LEFTSHIFT,
                KeyCode::KEY_RIGHTSHIFT,
            ]));
        }
        if self.logo {
            groups.push(EvdevKeyGroup::new(vec![
                KeyCode::KEY_LEFTMETA,
                KeyCode::KEY_RIGHTMETA,
            ]));
        }
        groups
    }
}

#[derive(Debug, Clone, Copy)]
enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Logo,
}

impl Modifier {
    fn parse(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Self::Ctrl),
            "alt" | "option" => Some(Self::Alt),
            "shift" => Some(Self::Shift),
            "super" | "logo" | "meta" | "cmd" | "command" => Some(Self::Logo),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct KeySpec {
    canonical: &'static str,
    xdg: &'static str,
}

fn parse_key(token: &str) -> Option<KeySpec> {
    let normalized = token.trim().to_ascii_lowercase();

    if let Some(letter) = parse_letter(&normalized) {
        return Some(letter);
    }

    if let Some(digit) = parse_digit(&normalized) {
        return Some(digit);
    }

    if let Some(function_key) = parse_function_key(&normalized) {
        return Some(function_key);
    }

    match normalized.as_str() {
        "space" | "spacebar" => Some(KeySpec {
            canonical: "Space",
            xdg: "space",
        }),
        "enter" | "return" => Some(KeySpec {
            canonical: "Enter",
            xdg: "Return",
        }),
        "esc" | "escape" => Some(KeySpec {
            canonical: "Escape",
            xdg: "Escape",
        }),
        "tab" => Some(KeySpec {
            canonical: "Tab",
            xdg: "Tab",
        }),
        "backspace" | "back_space" => Some(KeySpec {
            canonical: "Backspace",
            xdg: "BackSpace",
        }),
        "delete" | "del" => Some(KeySpec {
            canonical: "Delete",
            xdg: "Delete",
        }),
        "insert" | "ins" => Some(KeySpec {
            canonical: "Insert",
            xdg: "Insert",
        }),
        "home" => Some(KeySpec {
            canonical: "Home",
            xdg: "Home",
        }),
        "end" => Some(KeySpec {
            canonical: "End",
            xdg: "End",
        }),
        "pageup" | "page_up" | "prior" => Some(KeySpec {
            canonical: "PageUp",
            xdg: "Page_Up",
        }),
        "pagedown" | "page_down" | "next" => Some(KeySpec {
            canonical: "PageDown",
            xdg: "Page_Down",
        }),
        "left" | "arrowleft" | "arrow_left" => Some(KeySpec {
            canonical: "Left",
            xdg: "Left",
        }),
        "right" | "arrowright" | "arrow_right" => Some(KeySpec {
            canonical: "Right",
            xdg: "Right",
        }),
        "up" | "arrowup" | "arrow_up" => Some(KeySpec {
            canonical: "Up",
            xdg: "Up",
        }),
        "down" | "arrowdown" | "arrow_down" => Some(KeySpec {
            canonical: "Down",
            xdg: "Down",
        }),
        _ => None,
    }
}

fn parse_letter(normalized: &str) -> Option<KeySpec> {
    let letter = normalized
        .strip_prefix("key")
        .unwrap_or(normalized)
        .chars()
        .collect::<Vec<_>>();
    if letter.len() != 1 || !letter[0].is_ascii_alphabetic() {
        return None;
    }

    match letter[0] {
        'a' => Some(KeySpec {
            canonical: "A",
            xdg: "a",
        }),
        'b' => Some(KeySpec {
            canonical: "B",
            xdg: "b",
        }),
        'c' => Some(KeySpec {
            canonical: "C",
            xdg: "c",
        }),
        'd' => Some(KeySpec {
            canonical: "D",
            xdg: "d",
        }),
        'e' => Some(KeySpec {
            canonical: "E",
            xdg: "e",
        }),
        'f' => Some(KeySpec {
            canonical: "F",
            xdg: "f",
        }),
        'g' => Some(KeySpec {
            canonical: "G",
            xdg: "g",
        }),
        'h' => Some(KeySpec {
            canonical: "H",
            xdg: "h",
        }),
        'i' => Some(KeySpec {
            canonical: "I",
            xdg: "i",
        }),
        'j' => Some(KeySpec {
            canonical: "J",
            xdg: "j",
        }),
        'k' => Some(KeySpec {
            canonical: "K",
            xdg: "k",
        }),
        'l' => Some(KeySpec {
            canonical: "L",
            xdg: "l",
        }),
        'm' => Some(KeySpec {
            canonical: "M",
            xdg: "m",
        }),
        'n' => Some(KeySpec {
            canonical: "N",
            xdg: "n",
        }),
        'o' => Some(KeySpec {
            canonical: "O",
            xdg: "o",
        }),
        'p' => Some(KeySpec {
            canonical: "P",
            xdg: "p",
        }),
        'q' => Some(KeySpec {
            canonical: "Q",
            xdg: "q",
        }),
        'r' => Some(KeySpec {
            canonical: "R",
            xdg: "r",
        }),
        's' => Some(KeySpec {
            canonical: "S",
            xdg: "s",
        }),
        't' => Some(KeySpec {
            canonical: "T",
            xdg: "t",
        }),
        'u' => Some(KeySpec {
            canonical: "U",
            xdg: "u",
        }),
        'v' => Some(KeySpec {
            canonical: "V",
            xdg: "v",
        }),
        'w' => Some(KeySpec {
            canonical: "W",
            xdg: "w",
        }),
        'x' => Some(KeySpec {
            canonical: "X",
            xdg: "x",
        }),
        'y' => Some(KeySpec {
            canonical: "Y",
            xdg: "y",
        }),
        'z' => Some(KeySpec {
            canonical: "Z",
            xdg: "z",
        }),
        _ => None,
    }
}

fn parse_digit(normalized: &str) -> Option<KeySpec> {
    let digit = normalized.strip_prefix("digit").unwrap_or(normalized);
    match digit {
        "0" => Some(KeySpec {
            canonical: "0",
            xdg: "0",
        }),
        "1" => Some(KeySpec {
            canonical: "1",
            xdg: "1",
        }),
        "2" => Some(KeySpec {
            canonical: "2",
            xdg: "2",
        }),
        "3" => Some(KeySpec {
            canonical: "3",
            xdg: "3",
        }),
        "4" => Some(KeySpec {
            canonical: "4",
            xdg: "4",
        }),
        "5" => Some(KeySpec {
            canonical: "5",
            xdg: "5",
        }),
        "6" => Some(KeySpec {
            canonical: "6",
            xdg: "6",
        }),
        "7" => Some(KeySpec {
            canonical: "7",
            xdg: "7",
        }),
        "8" => Some(KeySpec {
            canonical: "8",
            xdg: "8",
        }),
        "9" => Some(KeySpec {
            canonical: "9",
            xdg: "9",
        }),
        _ => None,
    }
}

fn parse_function_key(normalized: &str) -> Option<KeySpec> {
    match normalized {
        "f1" => Some(KeySpec {
            canonical: "F1",
            xdg: "F1",
        }),
        "f2" => Some(KeySpec {
            canonical: "F2",
            xdg: "F2",
        }),
        "f3" => Some(KeySpec {
            canonical: "F3",
            xdg: "F3",
        }),
        "f4" => Some(KeySpec {
            canonical: "F4",
            xdg: "F4",
        }),
        "f5" => Some(KeySpec {
            canonical: "F5",
            xdg: "F5",
        }),
        "f6" => Some(KeySpec {
            canonical: "F6",
            xdg: "F6",
        }),
        "f7" => Some(KeySpec {
            canonical: "F7",
            xdg: "F7",
        }),
        "f8" => Some(KeySpec {
            canonical: "F8",
            xdg: "F8",
        }),
        "f9" => Some(KeySpec {
            canonical: "F9",
            xdg: "F9",
        }),
        "f10" => Some(KeySpec {
            canonical: "F10",
            xdg: "F10",
        }),
        "f11" => Some(KeySpec {
            canonical: "F11",
            xdg: "F11",
        }),
        "f12" => Some(KeySpec {
            canonical: "F12",
            xdg: "F12",
        }),
        "f13" => Some(KeySpec {
            canonical: "F13",
            xdg: "F13",
        }),
        "f14" => Some(KeySpec {
            canonical: "F14",
            xdg: "F14",
        }),
        "f15" => Some(KeySpec {
            canonical: "F15",
            xdg: "F15",
        }),
        "f16" => Some(KeySpec {
            canonical: "F16",
            xdg: "F16",
        }),
        "f17" => Some(KeySpec {
            canonical: "F17",
            xdg: "F17",
        }),
        "f18" => Some(KeySpec {
            canonical: "F18",
            xdg: "F18",
        }),
        "f19" => Some(KeySpec {
            canonical: "F19",
            xdg: "F19",
        }),
        "f20" => Some(KeySpec {
            canonical: "F20",
            xdg: "F20",
        }),
        "f21" => Some(KeySpec {
            canonical: "F21",
            xdg: "F21",
        }),
        "f22" => Some(KeySpec {
            canonical: "F22",
            xdg: "F22",
        }),
        "f23" => Some(KeySpec {
            canonical: "F23",
            xdg: "F23",
        }),
        "f24" => Some(KeySpec {
            canonical: "F24",
            xdg: "F24",
        }),
        _ => None,
    }
}

fn evdev_key_for_canonical(canonical: &str) -> Option<KeyCode> {
    let key_name = match canonical {
        "Space" => Cow::Borrowed("KEY_SPACE"),
        "Enter" => Cow::Borrowed("KEY_ENTER"),
        "Escape" => Cow::Borrowed("KEY_ESC"),
        "Tab" => Cow::Borrowed("KEY_TAB"),
        "Backspace" => Cow::Borrowed("KEY_BACKSPACE"),
        "Delete" => Cow::Borrowed("KEY_DELETE"),
        "Insert" => Cow::Borrowed("KEY_INSERT"),
        "Home" => Cow::Borrowed("KEY_HOME"),
        "End" => Cow::Borrowed("KEY_END"),
        "PageUp" => Cow::Borrowed("KEY_PAGEUP"),
        "PageDown" => Cow::Borrowed("KEY_PAGEDOWN"),
        "Left" => Cow::Borrowed("KEY_LEFT"),
        "Right" => Cow::Borrowed("KEY_RIGHT"),
        "Up" => Cow::Borrowed("KEY_UP"),
        "Down" => Cow::Borrowed("KEY_DOWN"),
        _ => Cow::Owned(format!("KEY_{canonical}")),
    };

    KeyCode::from_str(&key_name).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_hotkey_for_evdev() {
        let spec = HotkeySpec::parse("Ctrl+Alt+Space").unwrap();

        assert_eq!(spec.canonical(), "Ctrl+Alt+Space");
        assert_eq!(spec.xdg_trigger(), "CTRL+ALT+space");

        let evdev = spec.to_evdev_hotkey();
        assert_eq!(evdev.main_key(), KeyCode::KEY_SPACE);
        assert_eq!(evdev.modifier_groups().len(), 2);
        assert_eq!(
            evdev.modifier_groups()[0].keys(),
            &[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_RIGHTCTRL]
        );
        assert_eq!(
            evdev.modifier_groups()[1].keys(),
            &[KeyCode::KEY_LEFTALT, KeyCode::KEY_RIGHTALT]
        );
    }

    #[test]
    fn normalizes_common_aliases() {
        let spec = HotkeySpec::parse("Control + Option + Return").unwrap();

        assert_eq!(spec.canonical(), "Ctrl+Alt+Enter");
        assert_eq!(spec.xdg_trigger(), "CTRL+ALT+Return");
    }

    #[test]
    fn maps_super_to_evdev_meta_modifier() {
        let spec = HotkeySpec::parse("Meta+Shift+R").unwrap();

        assert_eq!(spec.canonical(), "Shift+Super+R");
        assert_eq!(spec.xdg_trigger(), "SHIFT+LOGO+r");
    }

    #[test]
    fn parses_function_keys() {
        let spec = HotkeySpec::parse("F12").unwrap();

        assert_eq!(spec.canonical(), "F12");
        assert_eq!(spec.xdg_trigger(), "F12");
        assert_eq!(spec.to_evdev_hotkey().main_key(), KeyCode::KEY_F12);
    }

    #[test]
    fn rejects_invalid_hotkeys() {
        assert!(HotkeySpec::parse("").is_err());
        assert!(HotkeySpec::parse("Ctrl+Alt").is_err());
        assert!(HotkeySpec::parse("Ctrl++Space").is_err());
        assert!(HotkeySpec::parse("Ctrl+A+B").is_err());
        assert!(HotkeySpec::parse("Ctrl+Unknown").is_err());
    }

    #[test]
    fn converts_to_global_hotkey() {
        let hotkey = HotkeySpec::parse("Ctrl+Alt+Space")
            .unwrap()
            .to_global_hotkey()
            .unwrap();

        assert_eq!(hotkey.to_string(), "control+alt+Space");
    }
}
