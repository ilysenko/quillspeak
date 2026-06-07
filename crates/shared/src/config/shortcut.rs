use serde::{Deserialize, Serialize};

use super::output::ShortcutOutput;
use super::{ConfigError, inherit_value, normalize_language_ref, normalize_model_ref};

pub const DEFAULT_SHORTCUT_ID: &str = "default";
pub const DEFAULT_SHORTCUT_NAME: &str = "Default";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutProfile {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub accelerator: String,
    pub model_id: String,
    pub language: String,
    pub output: ShortcutOutput,
}

impl ShortcutProfile {
    pub fn default_profile() -> Self {
        Self {
            id: DEFAULT_SHORTCUT_ID.to_string(),
            name: DEFAULT_SHORTCUT_NAME.to_string(),
            enabled: true,
            accelerator: "Ctrl+Alt+Space".to_string(),
            model_id: inherit_value(),
            language: inherit_value(),
            output: ShortcutOutput::Default,
        }
    }

    pub fn new_profile(id: String, name: String) -> Self {
        Self {
            id,
            name,
            enabled: true,
            accelerator: String::new(),
            model_id: inherit_value(),
            language: inherit_value(),
            output: ShortcutOutput::Default,
        }
    }

    pub fn normalized(mut self) -> Result<Self, ConfigError> {
        self.id = normalize_id(&self.id);
        if self.id.is_empty() {
            return Err(ConfigError::EmptyShortcutId);
        }
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            self.name = self.id.clone();
        }
        if !self.enabled && self.accelerator.trim().is_empty() {
            self.accelerator.clear();
        } else {
            self.accelerator = normalize_accelerator(&self.accelerator)?;
        }
        self.model_id = normalize_model_ref(&self.model_id)?;
        self.language = normalize_language_ref(&self.language, true)?;
        self.output.validate()?;
        Ok(self)
    }
}

pub fn next_shortcut_id(shortcuts: &[ShortcutProfile]) -> String {
    let mut index = shortcuts.len() + 1;
    loop {
        let candidate = format!("shortcut-{index}");
        if shortcuts.iter().all(|shortcut| shortcut.id != candidate) {
            return candidate;
        }
        index += 1;
    }
}

fn normalize_id(input: &str) -> String {
    input
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

pub fn normalize_accelerator(input: &str) -> Result<String, ConfigError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::EmptyShortcut);
    }

    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut super_key = false;
    let mut key = None;
    let parts: Vec<&str> = if trimmed.contains('+') {
        trimmed.split('+').collect()
    } else {
        trimmed.split('-').collect()
    };

    for raw_part in parts {
        let part = raw_part.trim();
        if part.is_empty() {
            continue;
        }

        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" | "option" => alt = true,
            "shift" => shift = true,
            "super" | "meta" | "cmd" | "command" => super_key = true,
            _ if key.is_none() => key = Some(normalize_key(part)?),
            _ => return Err(ConfigError::UnsupportedShortcutKey(part.to_string())),
        }
    }

    let key = key.ok_or(ConfigError::MissingShortcutKey)?;
    let mut parts = Vec::new();
    if ctrl {
        parts.push("Ctrl".to_string());
    }
    if alt {
        parts.push("Alt".to_string());
    }
    if shift {
        parts.push("Shift".to_string());
    }
    if super_key {
        parts.push("Super".to_string());
    }
    parts.push(key);

    Ok(parts.join("+"))
}

fn normalize_key(input: &str) -> Result<String, ConfigError> {
    let lower = input.to_ascii_lowercase();
    let key = match lower.as_str() {
        "space" => "Space".to_string(),
        "esc" | "escape" => "Escape".to_string(),
        "enter" | "return" => "Enter".to_string(),
        "tab" => "Tab".to_string(),
        "backspace" => "Backspace".to_string(),
        "delete" | "del" => "Delete".to_string(),
        "insert" | "ins" => "Insert".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "pageup" | "page_up" => "PageUp".to_string(),
        "pagedown" | "page_down" => "PageDown".to_string(),
        "-" | "minus" => "Minus".to_string(),
        "=" | "equal" | "equals" => "Equal".to_string(),
        "," | "comma" => "Comma".to_string(),
        "." | "dot" | "period" => "Dot".to_string(),
        "/" | "slash" => "Slash".to_string(),
        ";" | "semicolon" => "Semicolon".to_string(),
        "'" | "apostrophe" | "quote" => "Apostrophe".to_string(),
        "`" | "grave" | "backtick" => "Grave".to_string(),
        "[" | "leftbracket" | "left_bracket" => "LeftBracket".to_string(),
        "]" | "rightbracket" | "right_bracket" => "RightBracket".to_string(),
        "\\" | "backslash" => "Backslash".to_string(),
        value if is_function_key(value) => value.to_ascii_uppercase(),
        value if value.chars().count() == 1 => value.to_ascii_uppercase(),
        _ => return Err(ConfigError::UnsupportedShortcutKey(input.to_string())),
    };

    Ok(key)
}

fn is_function_key(value: &str) -> bool {
    let Some(number) = value.strip_prefix('f') else {
        return false;
    };
    let Ok(number) = number.parse::<u8>() else {
        return false;
    };
    (1..=24).contains(&number)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ShortcutModifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_key: bool,
}

impl ShortcutModifiers {
    pub const fn is_empty(self) -> bool {
        !self.ctrl && !self.alt && !self.shift && !self.super_key
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcutKey {
    Character(char),
    Space,
    Escape,
    Enter,
    Tab,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Function(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShortcutChord {
    pub modifiers: ShortcutModifiers,
    pub key: ShortcutKey,
}

impl ShortcutChord {
    pub fn parse(accelerator: &str) -> Result<Self, ConfigError> {
        let normalized = normalize_accelerator(accelerator)?;
        let mut modifiers = ShortcutModifiers::default();
        let mut key = None;

        for part in normalized.split('+') {
            match part {
                "Ctrl" => modifiers.ctrl = true,
                "Alt" => modifiers.alt = true,
                "Shift" => modifiers.shift = true,
                "Super" => modifiers.super_key = true,
                key_name if key.is_none() => key = Some(parse_shortcut_key(key_name)?),
                other => return Err(ConfigError::UnsupportedShortcutKey(other.to_string())),
            }
        }

        Ok(Self {
            modifiers,
            key: key.ok_or(ConfigError::MissingShortcutKey)?,
        })
    }
}

fn parse_shortcut_key(input: &str) -> Result<ShortcutKey, ConfigError> {
    let key = match input {
        "Space" => ShortcutKey::Space,
        "Escape" => ShortcutKey::Escape,
        "Enter" => ShortcutKey::Enter,
        "Tab" => ShortcutKey::Tab,
        "Backspace" => ShortcutKey::Backspace,
        "Delete" => ShortcutKey::Delete,
        "Insert" => ShortcutKey::Insert,
        "Home" => ShortcutKey::Home,
        "End" => ShortcutKey::End,
        "PageUp" => ShortcutKey::PageUp,
        "PageDown" => ShortcutKey::PageDown,
        "Minus" => ShortcutKey::Character('-'),
        "Equal" => ShortcutKey::Character('='),
        "Comma" => ShortcutKey::Character(','),
        "Dot" => ShortcutKey::Character('.'),
        "Slash" => ShortcutKey::Character('/'),
        "Semicolon" => ShortcutKey::Character(';'),
        "Apostrophe" => ShortcutKey::Character('\''),
        "Grave" => ShortcutKey::Character('`'),
        "LeftBracket" => ShortcutKey::Character('['),
        "RightBracket" => ShortcutKey::Character(']'),
        "Backslash" => ShortcutKey::Character('\\'),
        value if is_function_key(&value.to_ascii_lowercase()) => {
            let number = value[1..]
                .parse::<u8>()
                .map_err(|_| ConfigError::UnsupportedShortcutKey(input.to_string()))?;
            ShortcutKey::Function(number)
        }
        value if value.chars().count() == 1 => {
            let Some(character) = value.chars().next() else {
                return Err(ConfigError::UnsupportedShortcutKey(input.to_string()));
            };
            ShortcutKey::Character(character)
        }
        _ => return Err(ConfigError::UnsupportedShortcutKey(input.to_string())),
    };

    Ok(key)
}
