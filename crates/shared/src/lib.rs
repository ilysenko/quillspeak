pub mod config;
pub mod persistence;

pub const APP_ID: &str = "org.example.MyApp";

pub use config::{
    AUTO_LANGUAGE_VALUE, AppConfig, AudioInputRef, CONFIG_SCHEMA_VERSION, ComputeBackend,
    ConfigError, DEFAULT_BEEP_VOLUME_PERCENT, DEFAULT_MODEL_ID, DEFAULT_SHORTCUT_ID,
    DEFAULT_SHORTCUT_NAME, GeneralConfig, HotkeyBackend, HotkeyMode, LinuxSignal, LinuxSignalSpec,
    MAX_BEEP_VOLUME_PERCENT, MIN_BEEP_VOLUME_PERCENT, MODEL_CATALOG, ModelCatalogEntry,
    OutputAction, PasteShortcut, SUPPORTED_LANGUAGES, SUPPORTED_LINUX_SIGNALS, ScriptOutput,
    ShortcutChord, ShortcutKey, ShortcutModifiers, ShortcutProfile, ShortcutTrigger,
    SupportedLanguage, model_catalog_entry, next_shortcut_id, normalize_accelerator,
    supported_language_label,
};
