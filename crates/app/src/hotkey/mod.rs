use anyhow::Result;
use shared::{AppConfig, HotkeyBackend as HotkeyBackendKind, ShortcutRuntimeConfig};
use tracing::info;

use crate::daemon_client::DaemonClient;

pub trait HotkeyBackend {
    fn name(&self) -> &'static str;
    fn configure(&self, config: &AppConfig) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct DisabledBackend;

impl HotkeyBackend for DisabledBackend {
    fn name(&self) -> &'static str {
        "disabled"
    }

    fn configure(&self, _config: &AppConfig) -> Result<()> {
        info!("Hotkey backend disabled");
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DaemonBackend {
    client: DaemonClient,
}

impl DaemonBackend {
    pub fn new(client: DaemonClient) -> Self {
        Self { client }
    }
}

impl HotkeyBackend for DaemonBackend {
    fn name(&self) -> &'static str {
        "daemon"
    }

    fn configure(&self, config: &AppConfig) -> Result<()> {
        self.client
            .update_shortcut_config(&ShortcutRuntimeConfig::from(config))
    }
}

pub fn backend_name_for_config(config: &AppConfig) -> &'static str {
    match config.hotkey_backend {
        HotkeyBackendKind::Disabled => "disabled",
        HotkeyBackendKind::Daemon => "daemon",
        HotkeyBackendKind::X11 => "x11",
        HotkeyBackendKind::Portal => "portal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::AppConfig;

    #[test]
    fn default_hotkey_backend_is_disabled() {
        assert_eq!(backend_name_for_config(&AppConfig::default()), "disabled");
    }
}
