mod x11;

use std::env;
use std::sync::mpsc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;

use anyhow::Result;
use shared::{AppConfig, HotkeyBackend as HotkeyBackendKind};
use tracing::{info, warn};

use crate::command::AppCommand;

pub trait HotkeyBackend {
    fn name(&self) -> &'static str;
    fn configure(&self, config: &AppConfig) -> Result<Option<HotkeyBackendHandle>>;
}

#[derive(Debug)]
pub struct HotkeyBackendHandle {
    shutdown: Arc<AtomicBool>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl HotkeyBackendHandle {
    fn new(shutdown: Arc<AtomicBool>, join_handle: thread::JoinHandle<()>) -> Self {
        Self {
            shutdown,
            join_handle: Some(join_handle),
        }
    }
}

impl Drop for HotkeyBackendHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

#[derive(Debug, Default)]
pub struct DisabledBackend;

impl HotkeyBackend for DisabledBackend {
    fn name(&self) -> &'static str {
        "disabled"
    }

    fn configure(&self, _config: &AppConfig) -> Result<Option<HotkeyBackendHandle>> {
        info!("Hotkey backend disabled");
        Ok(None)
    }
}

#[derive(Debug, Clone, Default)]
pub struct DaemonBackend;

impl HotkeyBackend for DaemonBackend {
    fn name(&self) -> &'static str {
        "daemon"
    }

    fn configure(&self, _config: &AppConfig) -> Result<Option<HotkeyBackendHandle>> {
        info!("Daemon hotkey backend selected; daemon will receive runtime config over D-Bus");
        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct X11Backend {
    command_tx: mpsc::Sender<AppCommand>,
}

impl X11Backend {
    pub fn new(command_tx: mpsc::Sender<AppCommand>) -> Self {
        Self { command_tx }
    }
}

impl HotkeyBackend for X11Backend {
    fn name(&self) -> &'static str {
        "x11"
    }

    fn configure(&self, config: &AppConfig) -> Result<Option<HotkeyBackendHandle>> {
        let Some(handle) = x11::spawn(config, self.command_tx.clone())? else {
            return Ok(None);
        };
        Ok(Some(HotkeyBackendHandle::new(
            handle.shutdown,
            handle.join_handle,
        )))
    }
}

pub fn configure_hotkey_backend(
    command_tx: mpsc::Sender<AppCommand>,
    config: &AppConfig,
) -> Option<HotkeyBackendHandle> {
    match resolve_backend_kind(config.general.hotkey_backend) {
        HotkeyBackendKind::Disabled => configure_backend(DisabledBackend, config),
        HotkeyBackendKind::Daemon => configure_backend(DaemonBackend, config),
        HotkeyBackendKind::X11 => configure_backend(X11Backend::new(command_tx), config),
        HotkeyBackendKind::Portal => {
            info!("Portal hotkey backend placeholder selected; using disabled backend");
            configure_backend(DisabledBackend, config)
        }
        HotkeyBackendKind::Auto => unreachable!("auto should resolve to a concrete backend"),
    }
}

fn configure_backend<B: HotkeyBackend>(
    backend: B,
    config: &AppConfig,
) -> Option<HotkeyBackendHandle> {
    info!(backend = backend.name(), "configuring hotkey backend");
    match backend.configure(config) {
        Ok(handle) => handle,
        Err(error) => {
            warn!(
                ?error,
                backend = backend.name(),
                "failed to configure hotkey backend"
            );
            None
        }
    }
}

pub fn backend_name_for_config(config: &AppConfig) -> &'static str {
    match resolve_backend_kind(config.general.hotkey_backend) {
        HotkeyBackendKind::Auto => "auto",
        HotkeyBackendKind::Disabled => "disabled",
        HotkeyBackendKind::Daemon => "daemon",
        HotkeyBackendKind::X11 => "x11",
        HotkeyBackendKind::Portal => "portal",
    }
}

pub fn resolve_backend_kind(configured: HotkeyBackendKind) -> HotkeyBackendKind {
    match configured {
        HotkeyBackendKind::Auto if env::var_os("WAYLAND_DISPLAY").is_some() => {
            HotkeyBackendKind::Daemon
        }
        HotkeyBackendKind::Auto if env::var_os("DISPLAY").is_some() => HotkeyBackendKind::X11,
        HotkeyBackendKind::Auto => HotkeyBackendKind::Disabled,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_disabled_backend_stays_disabled() {
        let mut config = AppConfig::default();
        config.general.hotkey_backend = HotkeyBackendKind::Disabled;

        assert_eq!(backend_name_for_config(&config), "disabled");
    }
}
