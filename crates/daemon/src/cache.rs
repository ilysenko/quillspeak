use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::BaseDirs;
use shared::ShortcutRuntimeConfig;
use shared::persistence::atomic_write_text;
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
        match toml::from_str::<ShortcutRuntimeConfig>(&contents) {
            Ok(config) => match config.validate_current_schema() {
                Ok(()) => Ok(Some(config)),
                Err(error) => {
                    warn!(
                        ?error,
                        path = %self.path.display(),
                        "ignoring daemon shortcut cache with unsupported schema"
                    );
                    Ok(None)
                }
            },
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
        let contents =
            toml::to_string_pretty(config).context("failed to encode daemon cache as TOML")?;
        atomic_write_text(&self.path, &contents)
            .with_context(|| format!("failed to write daemon cache {}", self.path.display()))?;
        Ok(())
    }
}
