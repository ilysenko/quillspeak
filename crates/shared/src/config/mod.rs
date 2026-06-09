use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

mod audio;
mod language;
mod model;
mod output;
mod shortcut;

pub use audio::AudioInputRef;
pub use language::{
    AUTO_LANGUAGE_VALUE, SUPPORTED_LANGUAGES, SupportedLanguage, supported_language_label,
};
pub use model::{DEFAULT_MODEL_ID, MODEL_CATALOG, ModelCatalogEntry, model_catalog_entry};
pub use output::{OutputAction, PasteShortcut, ResolvedOutput, ScriptOutput, ShortcutOutput};
pub use shortcut::{
    DEFAULT_SHORTCUT_ID, DEFAULT_SHORTCUT_NAME, LinuxSignal, ShortcutChord, ShortcutKey,
    ShortcutModifiers, ShortcutProfile, ShortcutTrigger, next_shortcut_id, normalize_accelerator,
};

pub const CONFIG_SCHEMA_VERSION: u32 = 9;
pub const INHERIT_VALUE: &str = "default";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("unsupported config schema version: {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("shortcut cannot be empty")]
    EmptyShortcut,
    #[error("shortcut id cannot be empty")]
    EmptyShortcutId,
    #[error("shortcut must include a non-modifier key")]
    MissingShortcutKey,
    #[error("unsupported shortcut key: {0}")]
    UnsupportedShortcutKey(String),
    #[error("duplicate shortcut: {0}")]
    DuplicateShortcut(String),
    #[error("duplicate linux signal trigger: {0}")]
    DuplicateSignal(String),
    #[error("duplicate shortcut id: {0}")]
    DuplicateShortcutId(String),
    #[error("missing default shortcut profile")]
    MissingDefaultShortcut,
    #[error("unsupported mode: {0}")]
    UnsupportedMode(String),
    #[error("unsupported hotkey backend: {0}")]
    UnsupportedBackend(String),
    #[error("unsupported compute backend: {0}")]
    UnsupportedComputeBackend(String),
    #[error("unsupported model id: {0}")]
    UnsupportedModel(String),
    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),
    #[error("script output path cannot be empty")]
    EmptyScriptPath,
    #[error("linux signal trigger cannot be empty")]
    EmptySignal,
    #[error("{0} paste shortcut cannot be empty")]
    EmptyPasteShortcut(&'static str),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyMode {
    #[default]
    PushToTalk,
}

impl HotkeyMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PushToTalk => "push_to_talk",
        }
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyBackend {
    #[default]
    Auto,
    Disabled,
    X11,
}

impl HotkeyBackend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Disabled => "disabled",
            Self::X11 => "x11",
        }
    }
}

impl TryFrom<&str> for HotkeyBackend {
    type Error = ConfigError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "auto" => Ok(Self::Auto),
            "disabled" => Ok(Self::Disabled),
            "x11" => Ok(Self::X11),
            other => Err(ConfigError::UnsupportedBackend(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeBackend {
    #[default]
    Auto,
    Cpu,
    Vulkan,
    Cuda,
    Rocm,
    OpenVino,
}

impl ComputeBackend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::Vulkan => "vulkan",
            Self::Cuda => "cuda",
            Self::Rocm => "rocm",
            Self::OpenVino => "openvino",
        }
    }
}

impl TryFrom<&str> for ComputeBackend {
    type Error = ConfigError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "auto" => Ok(Self::Auto),
            "cpu" => Ok(Self::Cpu),
            "vulkan" => Ok(Self::Vulkan),
            "cuda" => Ok(Self::Cuda),
            "rocm" => Ok(Self::Rocm),
            "openvino" => Ok(Self::OpenVino),
            other => Err(ConfigError::UnsupportedComputeBackend(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralConfig {
    pub mode: HotkeyMode,
    pub hotkey_backend: HotkeyBackend,
    pub default_input: AudioInputRef,
    pub default_model_id: String,
    pub default_language: String,
    pub compute_backend: ComputeBackend,
    pub keep_model_loaded: bool,
    pub default_output: OutputAction,
}

impl GeneralConfig {
    fn normalized(mut self) -> Result<Self, ConfigError> {
        self.default_model_id = normalize_model_id(&self.default_model_id)?;
        self.default_language = normalize_language_ref(&self.default_language, false)?;
        self.default_output.validate()?;
        Ok(self)
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            mode: HotkeyMode::PushToTalk,
            hotkey_backend: HotkeyBackend::Auto,
            default_input: AudioInputRef::SystemDefault,
            default_model_id: DEFAULT_MODEL_ID.to_string(),
            default_language: AUTO_LANGUAGE_VALUE.to_string(),
            compute_backend: ComputeBackend::Auto,
            keep_model_loaded: true,
            default_output: OutputAction::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    pub general: GeneralConfig,
    pub shortcuts: Vec<ShortcutProfile>,
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchemaVersion(self.schema_version));
        }

        let mut ids = HashSet::new();
        let mut keyboard_accelerators = HashSet::new();
        let mut linux_signals = HashSet::new();
        let mut default_count = 0;
        for shortcut in &self.shortcuts {
            if shortcut.id == DEFAULT_SHORTCUT_ID {
                default_count += 1;
            }
            if !ids.insert(shortcut.id.clone()) {
                return Err(ConfigError::DuplicateShortcutId(shortcut.id.clone()));
            }
            if shortcut.enabled {
                match &shortcut.trigger {
                    ShortcutTrigger::Keyboard { accelerator } => {
                        let normalized = normalize_accelerator(accelerator)?;
                        if !keyboard_accelerators.insert(normalized.clone()) {
                            return Err(ConfigError::DuplicateShortcut(normalized));
                        }
                    }
                    ShortcutTrigger::LinuxSignal {
                        start_signal,
                        stop_signal,
                    } => {
                        for signal in unique_shortcut_signals(start_signal, stop_signal)? {
                            if !linux_signals.insert(signal.clone()) {
                                return Err(ConfigError::DuplicateSignal(signal));
                            }
                        }
                    }
                }
            }
        }

        if default_count == 0 {
            return Err(ConfigError::MissingDefaultShortcut);
        }

        Ok(())
    }

    pub fn normalized(mut self) -> Result<Self, ConfigError> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchemaVersion(self.schema_version));
        }
        self.general = self.general.normalized()?;
        self.shortcuts = self
            .shortcuts
            .into_iter()
            .map(ShortcutProfile::normalized)
            .collect::<Result<Vec<_>, _>>()?;
        self.validate()?;
        Ok(self)
    }

    pub fn default_shortcut(&self) -> &ShortcutProfile {
        self.shortcuts
            .iter()
            .find(|shortcut| shortcut.id == DEFAULT_SHORTCUT_ID)
            .expect("validated config must contain the default shortcut")
    }

    pub fn shortcut_by_id(&self, shortcut_id: &str) -> Option<&ShortcutProfile> {
        self.shortcuts
            .iter()
            .find(|shortcut| shortcut.id == shortcut_id)
    }

    pub fn enabled_shortcuts(&self) -> impl Iterator<Item = &ShortcutProfile> {
        self.shortcuts
            .iter()
            .filter(|shortcut| shortcut.enabled && shortcut.trigger.is_configured())
    }

    pub fn enabled_keyboard_shortcuts(&self) -> impl Iterator<Item = &ShortcutProfile> {
        self.shortcuts.iter().filter(|shortcut| {
            shortcut.enabled
                && shortcut
                    .trigger
                    .keyboard_accelerator()
                    .is_some_and(|accelerator| !accelerator.trim().is_empty())
        })
    }

    pub fn resolved_model_id<'a>(&'a self, shortcut: &'a ShortcutProfile) -> &'a str {
        if shortcut.model_id == INHERIT_VALUE {
            &self.general.default_model_id
        } else {
            &shortcut.model_id
        }
    }

    pub fn resolved_language<'a>(&'a self, shortcut: &'a ShortcutProfile) -> &'a str {
        if shortcut.language == INHERIT_VALUE {
            &self.general.default_language
        } else {
            &shortcut.language
        }
    }

    pub fn resolved_output<'a>(&'a self, shortcut: &'a ShortcutProfile) -> ResolvedOutput<'a> {
        match &shortcut.output {
            ShortcutOutput::Default => ResolvedOutput::General(&self.general.default_output),
            ShortcutOutput::Custom { action } => ResolvedOutput::Custom(action),
        }
    }
}

fn unique_shortcut_signals(
    start_signal: &LinuxSignal,
    stop_signal: &LinuxSignal,
) -> Result<Vec<String>, ConfigError> {
    let start_signal = start_signal.duplicate_key()?;
    let stop_signal = stop_signal.duplicate_key()?;
    Ok(if start_signal == stop_signal {
        vec![start_signal]
    } else {
        vec![start_signal, stop_signal]
    })
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            general: GeneralConfig::default(),
            shortcuts: vec![ShortcutProfile::default_profile()],
        }
    }
}

pub(crate) fn inherit_value() -> String {
    INHERIT_VALUE.to_string()
}

pub(crate) fn normalize_model_id(input: &str) -> Result<String, ConfigError> {
    let value = input.trim();
    if model_catalog_entry(value).is_some() {
        Ok(value.to_string())
    } else {
        Err(ConfigError::UnsupportedModel(value.to_string()))
    }
}

pub(crate) fn normalize_model_ref(input: &str) -> Result<String, ConfigError> {
    let value = input.trim();
    if value == INHERIT_VALUE || model_catalog_entry(value).is_some() {
        Ok(value.to_string())
    } else {
        Err(ConfigError::UnsupportedModel(value.to_string()))
    }
}

pub(crate) fn normalize_language_ref(
    input: &str,
    allow_inherit: bool,
) -> Result<String, ConfigError> {
    let value = input.trim().to_ascii_lowercase();
    if language::is_supported_language_ref(&value, allow_inherit) {
        Ok(value)
    } else {
        Err(ConfigError::UnsupportedLanguage(input.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_multi_shortcut_contract() {
        let config = AppConfig::default();

        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
        assert_eq!(config.general.mode, HotkeyMode::PushToTalk);
        assert_eq!(config.general.hotkey_backend, HotkeyBackend::Auto);
        assert_eq!(config.general.default_input, AudioInputRef::SystemDefault);
        assert!(config.general.keep_model_loaded);
        assert_eq!(config.shortcuts.len(), 1);
        assert_eq!(config.default_shortcut().id, DEFAULT_SHORTCUT_ID);
        assert_eq!(config.default_shortcut().name, DEFAULT_SHORTCUT_NAME);
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
        assert!(encoded.contains("[general]"));
        assert!(encoded.contains("[[shortcuts]]"));
        assert!(encoded.contains("id = \"default\""));
    }

    #[test]
    fn old_config_is_rejected() {
        let result = toml::from_str::<AppConfig>(
            r#"
mode = "push_to_talk"
hotkey_backend = "disabled"
hotkey = "Ctrl-Alt-F"
"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let config = AppConfig {
            schema_version: 3,
            ..AppConfig::default()
        };

        assert_eq!(
            config.normalized(),
            Err(ConfigError::UnsupportedSchemaVersion(3))
        );
    }

    #[test]
    fn rejects_schema_without_default_input() {
        let result = toml::from_str::<AppConfig>(
            r#"
schema_version = 9

[general]
mode = "push_to_talk"
hotkey_backend = "auto"
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
keep_model_loaded = true
default_output = { copy_to_clipboard = true }

[[shortcuts]]
id = "default"
name = "Default"
enabled = true
trigger = { type = "keyboard", accelerator = "Ctrl+Alt+Space" }
model_id = "default"
language = "default"
output = { type = "default" }
"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn rejects_schema_without_keep_model_loaded() {
        let result = toml::from_str::<AppConfig>(
            r#"
schema_version = 9

[general]
mode = "push_to_talk"
hotkey_backend = "auto"
default_input = { type = "system_default" }
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
default_output = { copy_to_clipboard = true }

[[shortcuts]]
id = "default"
name = "Default"
enabled = true
trigger = { type = "keyboard", accelerator = "Ctrl+Alt+Space" }
model_id = "default"
language = "default"
output = { type = "default" }
"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn rejects_missing_default_shortcut() {
        let mut config = AppConfig::default();
        config.shortcuts[0].id = "other".to_string();

        assert_eq!(
            config.normalized(),
            Err(ConfigError::MissingDefaultShortcut)
        );
    }

    #[test]
    fn rejects_removed_auto_paste_output() {
        let mut config = AppConfig::default();
        config.general.default_output.copy_to_clipboard = false;

        let encoded = toml::to_string(&config).expect("config should encode");
        let mut value = toml::from_str::<toml::Value>(&encoded).expect("config should parse");
        value
            .get_mut("general")
            .and_then(|general| general.get_mut("default_output"))
            .and_then(|output| output.as_table_mut())
            .expect("default output should be a table")
            .insert("auto_paste".to_string(), toml::Value::Boolean(true));
        let encoded = toml::to_string(&value).expect("config should encode");

        let decoded = toml::from_str::<AppConfig>(&encoded)
            .expect_err("auto_paste is not part of the current output schema");

        assert!(decoded.to_string().contains("auto_paste"));
    }

    #[test]
    fn rejects_removed_daemon_backend() {
        let result = toml::from_str::<AppConfig>(
            r#"
schema_version = 9

[general]
mode = "push_to_talk"
hotkey_backend = "daemon"
default_input = { type = "system_default" }
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
keep_model_loaded = true
default_output = { copy_to_clipboard = true }

[[shortcuts]]
id = "default"
name = "Default"
enabled = true
trigger = { type = "keyboard", accelerator = "Ctrl+Alt+Space" }
model_id = "default"
language = "default"
output = { type = "default" }
"#,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("daemon"));
    }

    #[test]
    fn rejects_removed_portal_backend() {
        let result = toml::from_str::<AppConfig>(
            r#"
schema_version = 9

[general]
mode = "push_to_talk"
hotkey_backend = "portal"
default_input = { type = "system_default" }
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
keep_model_loaded = true
default_output = { copy_to_clipboard = true }

[[shortcuts]]
id = "default"
name = "Default"
enabled = true
trigger = { type = "keyboard", accelerator = "Ctrl+Alt+Space" }
model_id = "default"
language = "default"
output = { type = "default" }
"#,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("portal"));
    }

    #[test]
    fn config_round_trips_copy_output() {
        let mut config = AppConfig::default();
        config.general.default_output = OutputAction {
            copy_to_clipboard: true,
            ..OutputAction::default()
        };

        let encoded = toml::to_string(&config).expect("config should encode");
        let decoded: AppConfig = toml::from_str::<AppConfig>(&encoded)
            .expect("config should decode")
            .normalized()
            .expect("config should normalize");

        assert_eq!(decoded, config);
        assert!(encoded.contains("copy_to_clipboard = true"));
    }

    #[test]
    fn normalizes_shortcut_strings() {
        assert_eq!(
            normalize_accelerator("Control-Alt-space"),
            Ok("Ctrl+Alt+Space".to_string())
        );
        assert_eq!(
            normalize_accelerator("Ctrl+Space"),
            Ok("Ctrl+Space".to_string())
        );
        assert_eq!(
            normalize_accelerator("ctrl+alt+f"),
            Ok("Ctrl+Alt+F".to_string())
        );
        assert_eq!(
            normalize_accelerator("Super+F12"),
            Ok("Super+F12".to_string())
        );
        assert_eq!(
            normalize_accelerator("Ctrl+-"),
            Ok("Ctrl+Minus".to_string())
        );
        assert_eq!(
            normalize_accelerator("Ctrl-Minus"),
            Ok("Ctrl+Minus".to_string())
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

    #[test]
    fn disabled_shortcut_can_have_empty_accelerator() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::Keyboard {
            accelerator: String::new(),
        };
        config.shortcuts[0].enabled = false;

        let normalized = config
            .normalized()
            .expect("disabled empty shortcut is valid");

        assert!(!normalized.default_shortcut().enabled);
        assert_eq!(
            normalized.default_shortcut().trigger.keyboard_accelerator(),
            Some("")
        );
    }

    #[test]
    fn rejects_duplicate_enabled_accelerators() {
        let mut config = AppConfig::default();
        config.shortcuts.push(ShortcutProfile {
            id: "second".to_string(),
            name: "Second".to_string(),
            enabled: true,
            trigger: config.default_shortcut().trigger.clone(),
            model_id: INHERIT_VALUE.to_string(),
            language: INHERIT_VALUE.to_string(),
            output: ShortcutOutput::Default,
        });

        assert!(matches!(
            config.normalized(),
            Err(ConfigError::DuplicateShortcut(_))
        ));
    }

    #[test]
    fn rejects_duplicate_shortcut_ids() {
        let mut config = AppConfig::default();
        config.shortcuts.push(ShortcutProfile::default_profile());

        assert_eq!(
            config.normalized(),
            Err(ConfigError::DuplicateShortcutId(
                DEFAULT_SHORTCUT_ID.to_string()
            ))
        );
    }

    #[test]
    fn rejects_empty_script_path() {
        let mut config = AppConfig::default();
        config.shortcuts[0].output = ShortcutOutput::custom(OutputAction::script(String::new()));

        assert_eq!(config.normalized(), Err(ConfigError::EmptyScriptPath));
    }

    #[test]
    fn normalizes_linux_signal_aliases() {
        let trigger = toml::from_str::<ShortcutTrigger>(
            r#"
type = "linux_signal"
start_signal = "User 1"
stop_signal = "SIGUSR2"
"#,
        )
        .expect("signal trigger should decode");

        assert_eq!(
            trigger,
            ShortcutTrigger::LinuxSignal {
                start_signal: LinuxSignal::new("User 1"),
                stop_signal: LinuxSignal::new("SIGUSR2"),
            }
        );

        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = trigger;
        let normalized = config
            .normalized()
            .expect("signal trigger should normalize");
        assert_eq!(
            normalized.default_shortcut().trigger,
            ShortcutTrigger::LinuxSignal {
                start_signal: LinuxSignal::sigusr1(),
                stop_signal: LinuxSignal::sigusr2(),
            }
        );
    }

    #[test]
    fn duplicate_linux_signals_across_shortcuts_are_rejected() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::default_linux_signal();
        config.shortcuts.push(ShortcutProfile {
            id: "second".to_string(),
            name: "Second".to_string(),
            enabled: true,
            trigger: ShortcutTrigger::default_linux_signal(),
            model_id: INHERIT_VALUE.to_string(),
            language: INHERIT_VALUE.to_string(),
            output: ShortcutOutput::Default,
        });

        assert_eq!(
            config.normalized(),
            Err(ConfigError::DuplicateSignal("SIGUSR2".to_string()))
        );
    }

    #[test]
    fn arbitrary_non_empty_linux_signal_text_is_allowed() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::LinuxSignal {
            start_signal: LinuxSignal::new("MY_SIGNAL_A"),
            stop_signal: LinuxSignal::new("MY_SIGNAL_B"),
        };

        let normalized = config
            .normalized()
            .expect("arbitrary signal text should persist");

        assert_eq!(
            normalized.default_shortcut().trigger,
            ShortcutTrigger::LinuxSignal {
                start_signal: LinuxSignal::new("MY_SIGNAL_A"),
                stop_signal: LinuxSignal::new("MY_SIGNAL_B"),
            }
        );
    }

    #[test]
    fn empty_linux_signal_text_is_rejected() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::LinuxSignal {
            start_signal: LinuxSignal::new(" "),
            stop_signal: LinuxSignal::sigusr2(),
        };

        assert_eq!(config.normalized(), Err(ConfigError::EmptySignal));
    }

    #[test]
    fn same_start_stop_signal_inside_one_shortcut_is_valid_toggle() {
        let mut config = AppConfig::default();
        config.shortcuts[0].trigger = ShortcutTrigger::default_linux_signal();

        let normalized = config
            .normalized()
            .expect("same signal on one shortcut should be a valid toggle");

        assert!(matches!(
            &normalized.default_shortcut().trigger,
            ShortcutTrigger::LinuxSignal {
                start_signal,
                stop_signal,
            }
            if *start_signal == LinuxSignal::sigusr2() && *stop_signal == LinuxSignal::sigusr2()
        ));
    }

    #[test]
    fn parses_shortcut_chords_for_runtime_backends() {
        let chord = ShortcutChord::parse("Ctrl+Alt+F").expect("valid shortcut");
        assert!(chord.modifiers.ctrl);
        assert!(chord.modifiers.alt);
        assert!(!chord.modifiers.shift);
        assert_eq!(chord.key, ShortcutKey::Character('F'));

        let chord = ShortcutChord::parse("Ctrl+Space").expect("valid shortcut");
        assert!(chord.modifiers.ctrl);
        assert!(!chord.modifiers.alt);
        assert_eq!(chord.key, ShortcutKey::Space);

        let chord = ShortcutChord::parse("Super+F12").expect("valid shortcut");
        assert!(chord.modifiers.super_key);
        assert_eq!(chord.key, ShortcutKey::Function(12));

        let chord = ShortcutChord::parse("Ctrl+Minus").expect("valid shortcut");
        assert!(chord.modifiers.ctrl);
        assert_eq!(chord.key, ShortcutKey::Character('-'));
    }

    #[test]
    fn catalog_contains_default_and_ukrainian() {
        assert!(model_catalog_entry(DEFAULT_MODEL_ID).is_some());
        assert_eq!(supported_language_label("uk"), Some("Ukrainian"));
    }
}
