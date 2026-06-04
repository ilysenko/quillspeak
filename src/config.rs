use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

use crate::whisper::WhisperBackendPreference;

const APP_CONFIG_DIR: &str = "voice";
const CONFIG_FILE_NAME: &str = "voice.toml";

pub const DEFAULT_PUSH_TO_TALK_HOTKEY: &str = "Ctrl+Alt+Space";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    #[serde(default = "default_push_to_talk_hotkey")]
    pub push_to_talk_hotkey: String,
    #[serde(default)]
    pub whisper_model: String,
    #[serde(default)]
    pub microphone_device: Option<String>,
    #[serde(default)]
    pub whisper_backend: WhisperBackendPreference,
    #[serde(default)]
    pub gpu_device: i32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            push_to_talk_hotkey: default_push_to_talk_hotkey(),
            whisper_model: String::new(),
            microphone_device: None,
            whisper_backend: WhisperBackendPreference::Auto,
            gpu_device: 0,
        }
    }
}

fn default_push_to_talk_hotkey() -> String {
    DEFAULT_PUSH_TO_TALK_HOTKEY.to_string()
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new() -> Result<Self> {
        let base_dirs = BaseDirs::new().context("could not determine XDG base directories")?;
        Ok(Self::from_path(
            base_dirs
                .config_dir()
                .join(APP_CONFIG_DIR)
                .join(CONFIG_FILE_NAME),
        ))
    }

    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<AppConfig> {
        if !self.path.exists() {
            return Ok(AppConfig::default());
        }

        let raw = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read config file {}", self.path.display()))?;
        toml::from_str(&raw)
            .with_context(|| format!("failed to parse config file {}", self.path.display()))
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory {}", parent.display())
            })?;
        }

        let raw = toml::to_string_pretty(config).context("failed to serialize config")?;
        fs::write(&self.path, raw)
            .with_context(|| format!("failed to write config file {}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_uses_defaults() {
        let store = ConfigStore::from_path(test_config_path("missing"));

        assert_eq!(store.load().unwrap(), AppConfig::default());
    }

    #[test]
    fn saves_and_loads_config() {
        let store = ConfigStore::from_path(test_config_path("roundtrip"));
        let config = AppConfig {
            push_to_talk_hotkey: "Alt+Space".to_string(),
            whisper_model: "/models/base.en.bin".to_string(),
            microphone_device: Some("Samson Meteor Mic".to_string()),
            whisper_backend: WhisperBackendPreference::Auto,
            gpu_device: 0,
        };

        store.save(&config).unwrap();

        assert_eq!(store.load().unwrap(), config);
    }

    #[test]
    fn loads_config_with_missing_newer_fields() {
        let store = ConfigStore::from_path(test_config_path("missing-new-fields"));
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(
            store.path(),
            r#"
push_to_talk_hotkey = "Ctrl+Shift+Space"
whisper_model = "/models/medium.bin"
"#,
        )
        .unwrap();

        let config = store.load().unwrap();

        assert_eq!(config.push_to_talk_hotkey, "Ctrl+Shift+Space");
        assert_eq!(config.whisper_model, "/models/medium.bin");
        assert_eq!(config.microphone_device, None);
        assert_eq!(config.whisper_backend, WhisperBackendPreference::Auto);
        assert_eq!(config.gpu_device, 0);
    }

    fn test_config_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "voice-config-test-{}-{name}/voice.toml",
            std::process::id()
        ))
    }
}
