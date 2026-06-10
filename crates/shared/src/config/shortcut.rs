use serde::{Deserialize, Serialize};

use super::output::OutputAction;
use super::{
    AUTO_LANGUAGE_VALUE, ConfigError, DEFAULT_MODEL_ID, normalize_language_ref, normalize_model_id,
};

pub const DEFAULT_SHORTCUT_ID: &str = "default";
pub const DEFAULT_SHORTCUT_NAME: &str = "Default";
pub const DEFAULT_BEEP_VOLUME_PERCENT: u8 = 100;
pub const MIN_BEEP_VOLUME_PERCENT: u8 = 10;
pub const MAX_BEEP_VOLUME_PERCENT: u8 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinuxSignalSpec {
    pub name: &'static str,
    pub label: &'static str,
}

pub const SUPPORTED_LINUX_SIGNALS: &[LinuxSignalSpec] = &[
    LinuxSignalSpec {
        name: "SIGUSR1",
        label: "SIGUSR1 - user-defined signal 1",
    },
    LinuxSignalSpec {
        name: "SIGUSR2",
        label: "SIGUSR2 - user-defined signal 2",
    },
    LinuxSignalSpec {
        name: "SIGALRM",
        label: "SIGALRM - alarm signal",
    },
    LinuxSignalSpec {
        name: "SIGWINCH",
        label: "SIGWINCH - window size change",
    },
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutProfile {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: ShortcutTrigger,
    pub model_id: String,
    pub language: String,
    pub mute_output_while_recording: bool,
    pub beep_on_recording: bool,
    pub beep_volume_percent: u8,
    pub output: OutputAction,
}

impl ShortcutProfile {
    pub fn default_profile() -> Self {
        Self {
            id: DEFAULT_SHORTCUT_ID.to_string(),
            name: DEFAULT_SHORTCUT_NAME.to_string(),
            enabled: true,
            trigger: ShortcutTrigger::default_keyboard(),
            model_id: DEFAULT_MODEL_ID.to_string(),
            language: AUTO_LANGUAGE_VALUE.to_string(),
            mute_output_while_recording: false,
            beep_on_recording: false,
            beep_volume_percent: DEFAULT_BEEP_VOLUME_PERCENT,
            output: OutputAction::default(),
        }
    }

    pub fn new_profile(id: String, name: String, model_id: String) -> Self {
        Self {
            id,
            name,
            enabled: true,
            trigger: ShortcutTrigger::Keyboard {
                accelerator: String::new(),
            },
            model_id,
            language: AUTO_LANGUAGE_VALUE.to_string(),
            mute_output_while_recording: false,
            beep_on_recording: false,
            beep_volume_percent: DEFAULT_BEEP_VOLUME_PERCENT,
            output: OutputAction::default(),
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
        self.trigger = self.trigger.normalized(self.enabled)?;
        self.model_id = normalize_model_id(&self.model_id)?;
        self.language = normalize_language_ref(&self.language)?;
        self.beep_volume_percent = self
            .beep_volume_percent
            .clamp(MIN_BEEP_VOLUME_PERCENT, MAX_BEEP_VOLUME_PERCENT);
        self.output.validate()?;
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShortcutTrigger {
    Keyboard {
        accelerator: String,
    },
    LinuxSignal {
        start_signal: LinuxSignal,
        stop_signal: LinuxSignal,
    },
}

impl ShortcutTrigger {
    pub fn default_keyboard() -> Self {
        Self::Keyboard {
            accelerator: "Ctrl+Alt+Space".to_string(),
        }
    }

    pub fn default_linux_signal() -> Self {
        Self::LinuxSignal {
            start_signal: LinuxSignal::sigusr1(),
            stop_signal: LinuxSignal::sigusr2(),
        }
    }

    pub fn keyboard_accelerator(&self) -> Option<&str> {
        match self {
            Self::Keyboard { accelerator } => Some(accelerator),
            Self::LinuxSignal { .. } => None,
        }
    }

    pub fn is_configured(&self) -> bool {
        match self {
            Self::Keyboard { accelerator } => !accelerator.trim().is_empty(),
            Self::LinuxSignal { .. } => true,
        }
    }

    fn normalized(mut self, enabled: bool) -> Result<Self, ConfigError> {
        match &mut self {
            Self::Keyboard { accelerator } => {
                if !enabled && accelerator.trim().is_empty() {
                    accelerator.clear();
                } else {
                    *accelerator = normalize_accelerator(accelerator)?;
                }
            }
            Self::LinuxSignal {
                start_signal,
                stop_signal,
            } => {
                *start_signal = start_signal.normalized()?;
                *stop_signal = stop_signal.normalized()?;
            }
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LinuxSignal(String);

impl LinuxSignal {
    pub fn sigusr1() -> Self {
        Self("SIGUSR1".to_string())
    }

    pub fn sigusr2() -> Self {
        Self("SIGUSR2".to_string())
    }

    pub fn sigalrm() -> Self {
        Self("SIGALRM".to_string())
    }

    pub fn sigwinch() -> Self {
        Self("SIGWINCH".to_string())
    }

    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn normalized(&self) -> Result<Self, ConfigError> {
        validate_signal_text(self.as_str())?;
        Ok(Self(self.0.clone()))
    }

    pub fn duplicate_key(&self) -> Result<String, ConfigError> {
        validate_signal_text(self.as_str())?;
        Ok(self.as_str().to_string())
    }
}

impl Default for LinuxSignal {
    fn default() -> Self {
        Self("SIGUSR2".to_string())
    }
}

fn validate_signal_text(input: &str) -> Result<(), ConfigError> {
    if input.trim().is_empty() {
        return Err(ConfigError::EmptySignal);
    }
    if SUPPORTED_LINUX_SIGNALS
        .iter()
        .any(|signal| signal.name == input)
    {
        Ok(())
    } else {
        Err(ConfigError::UnsupportedSignal(input.to_string()))
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
