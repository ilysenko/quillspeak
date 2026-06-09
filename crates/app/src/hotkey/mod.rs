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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutTriggerCapabilities {
    KeyboardAndSignals,
    SignalsOnly,
}

impl ShortcutTriggerCapabilities {
    pub const fn keyboard_available(self) -> bool {
        matches!(self, Self::KeyboardAndSignals)
    }
}

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
        HotkeyBackendKind::X11 => configure_backend(X11Backend::new(command_tx), config),
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

pub fn configured_backend_name(config: &AppConfig) -> &'static str {
    backend_kind_name(config.general.hotkey_backend)
}

pub fn effective_backend_name(config: &AppConfig) -> &'static str {
    backend_kind_name(resolve_backend_kind(config.general.hotkey_backend))
}

fn backend_kind_name(backend: HotkeyBackendKind) -> &'static str {
    match backend {
        HotkeyBackendKind::Auto => "auto",
        HotkeyBackendKind::Disabled => "disabled",
        HotkeyBackendKind::X11 => "x11",
    }
}

pub fn resolve_backend_kind(configured: HotkeyBackendKind) -> HotkeyBackendKind {
    resolve_backend_kind_with_env(configured, |name| env::var_os(name).is_some())
}

pub fn shortcut_trigger_capabilities() -> ShortcutTriggerCapabilities {
    shortcut_trigger_capabilities_with_env(|name| env::var_os(name).is_some())
}

fn resolve_backend_kind_with_env<F>(configured: HotkeyBackendKind, has_env: F) -> HotkeyBackendKind
where
    F: Fn(&str) -> bool,
{
    match configured {
        HotkeyBackendKind::Auto if x11_keyboard_available(&has_env) => HotkeyBackendKind::X11,
        HotkeyBackendKind::Auto => HotkeyBackendKind::Disabled,
        other => other,
    }
}

fn shortcut_trigger_capabilities_with_env<F>(has_env: F) -> ShortcutTriggerCapabilities
where
    F: Fn(&str) -> bool,
{
    if x11_keyboard_available(&has_env) {
        ShortcutTriggerCapabilities::KeyboardAndSignals
    } else {
        ShortcutTriggerCapabilities::SignalsOnly
    }
}

fn x11_keyboard_available<F>(has_env: &F) -> bool
where
    F: Fn(&str) -> bool,
{
    has_env("DISPLAY") && !has_env("WAYLAND_DISPLAY")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_disabled_backend_stays_disabled() {
        let mut config = AppConfig::default();
        config.general.hotkey_backend = HotkeyBackendKind::Disabled;

        assert_eq!(configured_backend_name(&config), "disabled");
        assert_eq!(effective_backend_name(&config), "disabled");
    }

    #[test]
    fn auto_backend_uses_x11_when_only_display_is_set() {
        let backend = resolve_backend_kind_with_env(HotkeyBackendKind::Auto, |name| {
            matches!(name, "DISPLAY")
        });

        assert_eq!(backend, HotkeyBackendKind::X11);
    }

    #[test]
    fn auto_backend_is_disabled_on_wayland() {
        let backend = resolve_backend_kind_with_env(HotkeyBackendKind::Auto, |name| {
            matches!(name, "DISPLAY" | "WAYLAND_DISPLAY")
        });

        assert_eq!(backend, HotkeyBackendKind::Disabled);
    }

    #[test]
    fn auto_backend_is_disabled_without_display() {
        let backend = resolve_backend_kind_with_env(HotkeyBackendKind::Auto, |_| false);

        assert_eq!(backend, HotkeyBackendKind::Disabled);
    }

    #[test]
    fn trigger_capabilities_match_display_server() {
        assert_eq!(
            shortcut_trigger_capabilities_with_env(|name| matches!(name, "DISPLAY")),
            ShortcutTriggerCapabilities::KeyboardAndSignals
        );
        assert_eq!(
            shortcut_trigger_capabilities_with_env(|name| {
                matches!(name, "DISPLAY" | "WAYLAND_DISPLAY")
            }),
            ShortcutTriggerCapabilities::SignalsOnly
        );
        assert_eq!(
            shortcut_trigger_capabilities_with_env(|_| false),
            ShortcutTriggerCapabilities::SignalsOnly
        );
    }

    #[test]
    fn configured_and_effective_backend_names_are_separate() {
        let config = AppConfig::default();

        assert_eq!(configured_backend_name(&config), "auto");
        assert_eq!(
            backend_kind_name(resolve_backend_kind_with_env(
                config.general.hotkey_backend,
                |name| matches!(name, "WAYLAND_DISPLAY"),
            )),
            "disabled"
        );
    }

    #[test]
    fn explicit_x11_backend_stays_x11() {
        let backend = resolve_backend_kind_with_env(HotkeyBackendKind::X11, |_| false);

        assert_eq!(backend, HotkeyBackendKind::X11);
    }
}
