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
        if config_schema_version(&contents) != Some(CONFIG_SCHEMA_VERSION) {
            warn!(
                config_path = %self.path.display(),
                "replacing unsupported settings config with defaults"
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

    fn temp_config_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("myapp-config-store-test-{suffix}"))
    }
}
