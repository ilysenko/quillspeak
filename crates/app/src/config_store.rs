use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::BaseDirs;
use shared::AppConfig;

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
        let config: AppConfig = toml::from_str(&contents)
            .with_context(|| format!("failed to parse config {}", self.path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        config.validate()?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory {}", parent.display())
            })?;
        }
        let contents = toml::to_string_pretty(config).context("failed to encode config as TOML")?;
        fs::write(&self.path, contents)
            .with_context(|| format!("failed to write config {}", self.path.display()))?;
        Ok(())
    }
}
