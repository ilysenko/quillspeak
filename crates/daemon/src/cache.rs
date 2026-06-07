use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::BaseDirs;
use shared::ShortcutRuntimeConfig;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct DaemonCacheStore {
    path: PathBuf,
}

impl DaemonCacheStore {
    pub fn new() -> Result<Self> {
        let base_dirs = BaseDirs::new().context("failed to resolve user config directory")?;
        Ok(Self {
            path: base_dirs
                .config_dir()
                .join("myapp-input-daemon/shortcut-cache.toml"),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<ShortcutRuntimeConfig>> {
        if !self.path.exists() {
            return Ok(None);
        }

        let contents = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read daemon cache {}", self.path.display()))?;
        match toml::from_str(&contents) {
            Ok(config) => Ok(Some(config)),
            Err(error) => {
                warn!(
                    ?error,
                    path = %self.path.display(),
                    "ignoring invalid daemon shortcut cache"
                );
                Ok(None)
            }
        }
    }

    pub fn save(&self, config: &ShortcutRuntimeConfig) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create daemon cache directory {}",
                    parent.display()
                )
            })?;
        }
        let contents =
            toml::to_string_pretty(config).context("failed to encode daemon cache as TOML")?;
        fs::write(&self.path, contents)
            .with_context(|| format!("failed to write daemon cache {}", self.path.display()))?;
        Ok(())
    }
}
