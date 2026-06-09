pub mod config;
pub mod persistence;

pub const APP_ID: &str = "org.example.MyApp";

pub use config::{
    AUTO_LANGUAGE_VALUE, AppConfig, AudioInputRef, CONFIG_SCHEMA_VERSION, ComputeBackend,
    ConfigError, DEFAULT_MODEL_ID, DEFAULT_SHORTCUT_ID, DEFAULT_SHORTCUT_NAME, GeneralConfig,
    HotkeyBackend, HotkeyMode, INHERIT_VALUE, LinuxSignal, MODEL_CATALOG, ModelCatalogEntry,
    OutputAction, PasteShortcut, ResolvedOutput, SUPPORTED_LANGUAGES, ScriptOutput, ShortcutChord,
    ShortcutKey, ShortcutModifiers, ShortcutOutput, ShortcutProfile, ShortcutTrigger,
    SupportedLanguage, model_catalog_entry, next_shortcut_id, normalize_accelerator,
    supported_language_label,
};
