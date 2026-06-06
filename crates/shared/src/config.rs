use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("hotkey cannot be empty")]
    EmptyHotkey,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub hotkey: String,
    pub mode: HotkeyMode,
    pub hotkey_backend: HotkeyBackend,
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.hotkey.trim().is_empty() {
            return Err(ConfigError::EmptyHotkey);
        }

        Ok(())
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Space".to_string(),
            mode: HotkeyMode::PushToTalk,
            hotkey_backend: HotkeyBackend::Disabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_prototype_contract() {
        let config = AppConfig::default();

        assert_eq!(config.hotkey, "Ctrl+Space");
        assert_eq!(config.mode, HotkeyMode::PushToTalk);
        assert_eq!(config.hotkey_backend, HotkeyBackend::Disabled);
    }

    #[test]
    fn config_round_trips_as_toml() {
        let config = AppConfig::default();
        let encoded = toml::to_string(&config).expect("config should encode");
        let decoded: AppConfig = toml::from_str(&encoded).expect("config should decode");

        assert_eq!(decoded, config);
        assert!(encoded.contains("hotkey = \"Ctrl+Space\""));
        assert!(encoded.contains("mode = \"push_to_talk\""));
        assert!(encoded.contains("hotkey_backend = \"disabled\""));
    }

    #[test]
    fn rejects_empty_hotkey() {
        let config = AppConfig {
            hotkey: " ".to_string(),
            ..AppConfig::default()
        };

        assert_eq!(config.validate(), Err(ConfigError::EmptyHotkey));
    }
}
