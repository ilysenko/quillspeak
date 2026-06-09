use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::BaseDirs;
use shared::persistence::atomic_write_text;
use shared::{AppConfig, CONFIG_SCHEMA_VERSION};
use tracing::warn;

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new() -> Result<Self> {
        let base_dirs = BaseDirs::new().context("failed to resolve user config directory")?;
        Ok(Self {
            path: base_dirs.config_dir().join("myapp/config.toml"),
        })
    }

    #[cfg(test)]
    fn for_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_or_create_default(&self) -> Result<AppConfig> {
        if !self.path.exists() {
            let config = AppConfig::default();
            self.save(&config)?;
            return Ok(config);
        }

        let contents = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read config {}", self.path.display()))?;
        let schema_version = config_schema_version(&contents);
        if schema_version != Some(CONFIG_SCHEMA_VERSION) {
            warn!(
                config_path = %self.path.display(),
                detected_schema_version = ?schema_version,
                current_schema_version = CONFIG_SCHEMA_VERSION,
                "discarding unsupported settings config and writing defaults"
            );
            return self.replace_with_default();
        }

        let config: AppConfig = toml::from_str(&contents)
            .with_context(|| format!("failed to parse config {}", self.path.display()))?;
        let config = config.normalized()?;
        Ok(config)
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        let config = config.clone().normalized()?;
        let contents =
            toml::to_string_pretty(&config).context("failed to encode config as TOML")?;
        atomic_write_text(&self.path, &contents)
            .with_context(|| format!("failed to write config {}", self.path.display()))?;
        Ok(())
    }

    fn replace_with_default(&self) -> Result<AppConfig> {
        let config = AppConfig::default();
        self.save(&config)?;
        Ok(config)
    }
}

fn config_schema_version(contents: &str) -> Option<u32> {
    let value = toml::from_str::<toml::Value>(contents).ok()?;
    let version = value.get("schema_version")?.as_integer()?;
    u32::try_from(version).ok()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn unsupported_schema_config_is_replaced_with_default() {
        let root = temp_config_root();
        let path = root.join("config.toml");
        fs::create_dir_all(&root).expect("test config dir should be writable");
        fs::write(
            &path,
            r#"
schema_version = 4

[general]
mode = "push_to_talk"
"#,
        )
        .expect("old config should be writable");
        let store = ConfigStore::for_path(path.clone());

        let config = store
            .load_or_create_default()
            .expect("old schema should be replaced");
        let contents = fs::read_to_string(&path).expect("replacement config should be readable");

        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
        assert_eq!(
            config_schema_version(&contents),
            Some(CONFIG_SCHEMA_VERSION)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn schema_v7_daemon_config_is_discarded_instead_of_migrated() {
        let root = temp_config_root();
        let path = root.join("config.toml");
        fs::create_dir_all(&root).expect("test config dir should be writable");
        fs::write(
            &path,
            r#"
schema_version = 7

[general]
mode = "push_to_talk"
hotkey_backend = "daemon"
default_input = { type = "system_default" }
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
keep_model_loaded = true
default_output = { copy_to_clipboard = true, auto_paste = true }

[[shortcuts]]
id = "default"
name = "Default"
enabled = true
trigger = { type = "linux_signal", start_signal = "SIGUSR1", stop_signal = "SIGUSR2" }
model_id = "default"
language = "default"
output = { type = "custom", copy_to_clipboard = false, auto_paste = true, script = { path = "/tmp/old-output" } }
"#,
        )
        .expect("v7 config should be writable");
        let store = ConfigStore::for_path(path.clone());

        let config = store
            .load_or_create_default()
            .expect("v7 config should be discarded");
        let contents = fs::read_to_string(&path).expect("replacement config should be readable");

        assert_eq!(config, AppConfig::default());
        assert_eq!(
            config_schema_version(&contents),
            Some(CONFIG_SCHEMA_VERSION)
        );
        assert!(!contents.contains("auto_paste"));
        assert!(!contents.contains("hotkey_backend = \"daemon\""));
        assert!(!contents.contains("linux_signal"));
        assert!(!contents.contains("/tmp/old-output"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn schema_v8_config_is_discarded_instead_of_migrated() {
        let root = temp_config_root();
        let path = root.join("config.toml");
        fs::create_dir_all(&root).expect("test config dir should be writable");
        fs::write(
            &path,
            r#"
schema_version = 8

[general]
mode = "push_to_talk"
hotkey_backend = "auto"
default_input = { type = "system_default" }
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
keep_model_loaded = true
default_output = { copy_to_clipboard = false }

[[shortcuts]]
id = "default"
name = "Default"
enabled = true
trigger = { type = "linux_signal", start_signal = "SIGUSR1", stop_signal = "SIGUSR2" }
model_id = "default"
language = "default"
output = { type = "default" }
"#,
        )
        .expect("v8 config should be writable");
        let store = ConfigStore::for_path(path.clone());

        let config = store
            .load_or_create_default()
            .expect("v8 config should be discarded");
        let contents = fs::read_to_string(&path).expect("replacement config should be readable");

        assert_eq!(config, AppConfig::default());
        assert_eq!(
            config_schema_version(&contents),
            Some(CONFIG_SCHEMA_VERSION)
        );
        assert!(!contents.contains("linux_signal"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn schema_v9_openvino_config_is_discarded_instead_of_parsed() {
        let root = temp_config_root();
        let path = root.join("config.toml");
        fs::create_dir_all(&root).expect("test config dir should be writable");
        fs::write(
            &path,
            r#"
schema_version = 9

[general]
mode = "push_to_talk"
hotkey_backend = "auto"
default_input = { type = "system_default" }
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "openvino"
keep_model_loaded = true
default_output = { copy_to_clipboard = true, paste_from_clipboard = false, paste_shortcut = "ctrl_v" }

[[shortcuts]]
id = "default"
name = "Default"
enabled = true
trigger = { type = "keyboard", accelerator = "Ctrl+Alt+Space" }
model_id = "default"
language = "default"
output = { type = "default" }
"#,
        )
        .expect("v9 config should be writable");
        let store = ConfigStore::for_path(path.clone());

        let config = store
            .load_or_create_default()
            .expect("v9 config should be discarded");
        let contents = fs::read_to_string(&path).expect("replacement config should be readable");

        assert_eq!(config, AppConfig::default());
        assert_eq!(
            config_schema_version(&contents),
            Some(CONFIG_SCHEMA_VERSION)
        );
        assert!(!contents.contains("openvino"));
        let _ = fs::remove_dir_all(root);
    }

    fn temp_config_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("myapp-config-store-test-{suffix}"))
    }
}
