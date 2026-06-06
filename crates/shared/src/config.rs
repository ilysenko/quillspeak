use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("shortcut cannot be empty")]
    EmptyShortcut,
    #[error("shortcut must include a non-modifier key")]
    MissingShortcutKey,
    #[error("unsupported shortcut key: {0}")]
    UnsupportedShortcutKey(String),
    #[error("duplicate shortcut: {0}")]
    DuplicateShortcut(String),
    #[error("unsupported mode: {0}")]
    UnsupportedMode(String),
    #[error("unsupported hotkey backend: {0}")]
    UnsupportedBackend(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyMode {
    PushToTalk,
}

impl HotkeyMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PushToTalk => "push_to_talk",
        }
    }
}

impl Default for HotkeyMode {
    fn default() -> Self {
        Self::PushToTalk
    }
}

impl TryFrom<&str> for HotkeyMode {
    type Error = ConfigError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "push_to_talk" => Ok(Self::PushToTalk),
            other => Err(ConfigError::UnsupportedMode(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyBackend {
    Disabled,
    Daemon,
    X11,
    Portal,
}

impl HotkeyBackend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Daemon => "daemon",
            Self::X11 => "x11",
            Self::Portal => "portal",
        }
    }
}

impl Default for HotkeyBackend {
    fn default() -> Self {
        Self::Disabled
    }
}

impl TryFrom<&str> for HotkeyBackend {
    type Error = ConfigError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "disabled" => Ok(Self::Disabled),
            "daemon" => Ok(Self::Daemon),
            "x11" => Ok(Self::X11),
            "portal" => Ok(Self::Portal),
            other => Err(ConfigError::UnsupportedBackend(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutAction {
    PushToTalk,
}

impl ShortcutAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PushToTalk => "push_to_talk",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::PushToTalk => "Push to talk",
        }
    }
}

impl TryFrom<&str> for ShortcutAction {
    type Error = ConfigError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "push_to_talk" => Ok(Self::PushToTalk),
            other => Err(ConfigError::UnsupportedShortcutKey(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutBinding {
    pub accelerator: String,
    pub enabled: bool,
}

impl ShortcutBinding {
    pub fn new(accelerator: impl Into<String>, enabled: bool) -> Self {
        Self {
            accelerator: accelerator.into(),
            enabled,
        }
    }

    pub fn normalized(mut self) -> Result<Self, ConfigError> {
        self.accelerator = normalize_accelerator(&self.accelerator)?;
        Ok(self)
    }
}

impl Default for ShortcutBinding {
    fn default() -> Self {
        Self::new("Ctrl+Space", true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutSettings {
    pub push_to_talk: ShortcutBinding,
}

impl ShortcutSettings {
    pub fn iter(&self) -> [(ShortcutAction, &ShortcutBinding); 1] {
        [(ShortcutAction::PushToTalk, &self.push_to_talk)]
    }
}

impl Default for ShortcutSettings {
    fn default() -> Self {
        Self {
            push_to_talk: ShortcutBinding::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub mode: HotkeyMode,
    pub hotkey_backend: HotkeyBackend,
    #[serde(default)]
    pub shortcuts: ShortcutSettings,
    #[serde(default, rename = "hotkey", skip_serializing)]
    legacy_hotkey: Option<String>,
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let mut seen = HashSet::new();
        for (_action, binding) in self.shortcuts.iter() {
            if !binding.enabled {
                continue;
            }

            let normalized = normalize_accelerator(&binding.accelerator)?;
            if !seen.insert(normalized.clone()) {
                return Err(ConfigError::DuplicateShortcut(normalized));
            }
        }

        Ok(())
    }

    pub fn normalized(mut self) -> Result<Self, ConfigError> {
        if let Some(legacy_hotkey) = self.legacy_hotkey.take() {
            self.shortcuts.push_to_talk.accelerator = legacy_hotkey;
        }

        self.shortcuts.push_to_talk = self.shortcuts.push_to_talk.normalized()?;
        self.validate()?;
        Ok(self)
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: default_schema_version(),
            mode: HotkeyMode::PushToTalk,
            hotkey_backend: HotkeyBackend::Disabled,
            shortcuts: ShortcutSettings::default(),
            legacy_hotkey: None,
        }
    }
}

fn default_schema_version() -> u32 {
    1
}

pub fn normalize_accelerator(input: &str) -> Result<String, ConfigError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::EmptyShortcut);
    }

    let normalized_separator = trimmed.replace('-', "+");
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut super_key = false;
    let mut key = None;

    for raw_part in normalized_separator.split('+') {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_prototype_contract() {
        let config = AppConfig::default();

        assert_eq!(config.shortcuts.push_to_talk.accelerator, "Ctrl+Space");
        assert_eq!(config.mode, HotkeyMode::PushToTalk);
        assert_eq!(config.hotkey_backend, HotkeyBackend::Disabled);
    }

    #[test]
    fn config_round_trips_as_toml() {
        let config = AppConfig::default();
        let encoded = toml::to_string(&config).expect("config should encode");
        let decoded: AppConfig = toml::from_str::<AppConfig>(&encoded)
            .expect("config should decode")
            .normalized()
            .expect("config should normalize");

        assert_eq!(decoded, config);
        assert!(encoded.contains("[shortcuts.push_to_talk]"));
        assert!(encoded.contains("accelerator = \"Ctrl+Space\""));
        assert!(encoded.contains("hotkey_backend = \"disabled\""));
    }

    #[test]
    fn migrates_legacy_hotkey_field() {
        let decoded: AppConfig = toml::from_str::<AppConfig>(
            r#"
mode = "push_to_talk"
hotkey_backend = "disabled"
hotkey = "Ctrl-Alt-F"
"#,
        )
        .expect("legacy config should decode")
        .normalized()
        .expect("legacy config should normalize");

        assert_eq!(decoded.shortcuts.push_to_talk.accelerator, "Ctrl+Alt+F");
    }

    #[test]
    fn normalizes_shortcut_strings() {
        assert_eq!(
            normalize_accelerator("Control-Alt-space"),
            Ok("Ctrl+Alt+Space".to_string())
        );
        assert_eq!(
            normalize_accelerator("ctrl+alt+f"),
            Ok("Ctrl+Alt+F".to_string())
        );
        assert_eq!(
            normalize_accelerator("Super+F12"),
            Ok("Super+F12".to_string())
        );
    }

    #[test]
    fn rejects_invalid_shortcuts() {
        assert_eq!(normalize_accelerator(" "), Err(ConfigError::EmptyShortcut));
        assert_eq!(
            normalize_accelerator("Ctrl+Alt"),
            Err(ConfigError::MissingShortcutKey)
        );
    }
}
