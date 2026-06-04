mod evdev;
mod spec;
#[cfg(test)]
mod stub;
mod x11;

use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};

use crate::activity::PushToTalkEvent;

pub use spec::HotkeySpec;

pub type PushToTalkHandler = Box<dyn Fn(PushToTalkEvent) + Send + 'static>;
pub(super) type SharedHotkeyHandler = Arc<Mutex<Option<PushToTalkHandler>>>;

#[derive(Debug, Default)]
pub(super) struct HotkeyEdgeFilter {
    is_pressed: AtomicBool,
}

impl HotkeyEdgeFilter {
    pub(super) fn filter(&self, event: PushToTalkEvent) -> Option<PushToTalkEvent> {
        match event {
            PushToTalkEvent::Pressed => self
                .is_pressed
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .ok()
                .map(|_| PushToTalkEvent::Pressed),
            PushToTalkEvent::Released => self
                .is_pressed
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                .ok()
                .map(|_| PushToTalkEvent::Released),
        }
    }

    pub(super) fn reset(&self) {
        self.is_pressed.store(false, Ordering::SeqCst);
    }
}

pub trait HotkeyBackend {
    fn configure_push_to_talk(&self, hotkey: &str) -> Result<()>;
    fn set_push_to_talk_handler(&self, handler: PushToTalkHandler) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveBackend {
    Evdev,
    X11,
}

pub struct AutoHotkeyBackend {
    handler: SharedHotkeyHandler,
    evdev: evdev::EvdevHotkeyBackend,
    x11: x11::X11HotkeyBackend,
    active_backend: Mutex<Option<ActiveBackend>>,
}

impl AutoHotkeyBackend {
    pub fn new() -> Self {
        let handler = Arc::new(Mutex::new(None));

        Self {
            evdev: evdev::EvdevHotkeyBackend::new(Arc::clone(&handler)),
            x11: x11::X11HotkeyBackend::new(Arc::clone(&handler)),
            handler,
            active_backend: Mutex::new(None),
        }
    }
}

impl Default for AutoHotkeyBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl HotkeyBackend for AutoHotkeyBackend {
    fn configure_push_to_talk(&self, hotkey: &str) -> Result<()> {
        let spec = HotkeySpec::parse(hotkey)?;
        let mut errors = Vec::new();

        match self.evdev.configure(&spec) {
            Ok(()) => {
                if let Err(error) = self.x11.deactivate() {
                    eprintln!("Failed to deactivate X11 hotkey fallback: {error:#}");
                }
                self.active_backend
                    .lock()
                    .expect("hotkey backend state was poisoned")
                    .replace(ActiveBackend::Evdev);
                return Ok(());
            }
            Err(error) => {
                errors.push(format!("evdev: {error:#}"));
            }
        }

        if can_use_x11_fallback() {
            match self.x11.configure(&spec) {
                Ok(()) => {
                    if let Err(error) = self.evdev.deactivate() {
                        eprintln!("Failed to deactivate evdev hotkey backend: {error:#}");
                    }
                    self.active_backend
                        .lock()
                        .expect("hotkey backend state was poisoned")
                        .replace(ActiveBackend::X11);
                    return Ok(());
                }
                Err(error) => {
                    errors.push(format!("x11: {error:#}"));
                }
            }
        } else {
            errors.push("x11: unavailable outside a real X11 session".to_string());
        }

        Err(anyhow!(
            "failed to register push-to-talk hotkey `{}` ({})",
            spec.canonical(),
            errors.join("; ")
        ))
    }

    fn set_push_to_talk_handler(&self, handler: PushToTalkHandler) -> Result<()> {
        self.handler
            .lock()
            .expect("hotkey handler state was poisoned")
            .replace(handler);
        Ok(())
    }
}

pub(super) fn dispatch_push_to_talk(handler: &SharedHotkeyHandler, event: PushToTalkEvent) {
    let guard = match handler.lock() {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("Push-to-talk handler lock was poisoned: {error}");
            return;
        }
    };

    if let Some(handler) = guard.as_ref() {
        handler(event);
    }
}

pub(super) fn dispatch_filtered_push_to_talk(
    handler: &SharedHotkeyHandler,
    edge_filter: &HotkeyEdgeFilter,
    event: PushToTalkEvent,
) {
    if let Some(event) = edge_filter.filter(event) {
        dispatch_push_to_talk(handler, event);
    }
}

fn can_use_x11_fallback() -> bool {
    let display = env::var("DISPLAY").ok();
    let wayland_display = env::var("WAYLAND_DISPLAY").ok();
    let session_type = env::var("XDG_SESSION_TYPE").ok();

    can_use_x11_fallback_for_env(
        display.as_deref(),
        wayland_display.as_deref(),
        session_type.as_deref(),
    )
}

fn can_use_x11_fallback_for_env(
    display: Option<&str>,
    wayland_display: Option<&str>,
    session_type: Option<&str>,
) -> bool {
    let has_display = display.is_some_and(|value| !value.trim().is_empty());
    if !has_display {
        return false;
    }

    let session_type = session_type.unwrap_or_default().to_ascii_lowercase();
    if session_type == "wayland" {
        return false;
    }

    wayland_display.is_none() || session_type == "x11"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x11_fallback_is_disabled_in_wayland_sessions() {
        assert!(!can_use_x11_fallback_for_env(
            Some(":0"),
            Some("wayland-0"),
            Some("wayland")
        ));
    }

    #[test]
    fn x11_fallback_is_enabled_in_x11_sessions() {
        assert!(can_use_x11_fallback_for_env(Some(":0"), None, Some("x11")));
    }

    #[test]
    fn edge_filter_emits_one_press_until_release() {
        let filter = HotkeyEdgeFilter::default();

        assert_eq!(
            filter.filter(PushToTalkEvent::Pressed),
            Some(PushToTalkEvent::Pressed)
        );
        assert_eq!(filter.filter(PushToTalkEvent::Pressed), None);
        assert_eq!(filter.filter(PushToTalkEvent::Pressed), None);
        assert_eq!(
            filter.filter(PushToTalkEvent::Released),
            Some(PushToTalkEvent::Released)
        );
    }

    #[test]
    fn edge_filter_ignores_release_without_press() {
        let filter = HotkeyEdgeFilter::default();

        assert_eq!(filter.filter(PushToTalkEvent::Released), None);
        assert_eq!(
            filter.filter(PushToTalkEvent::Pressed),
            Some(PushToTalkEvent::Pressed)
        );
        assert_eq!(
            filter.filter(PushToTalkEvent::Released),
            Some(PushToTalkEvent::Released)
        );
        assert_eq!(filter.filter(PushToTalkEvent::Released), None);
    }

    #[test]
    fn edge_filter_reset_clears_pressed_state() {
        let filter = HotkeyEdgeFilter::default();

        assert_eq!(
            filter.filter(PushToTalkEvent::Pressed),
            Some(PushToTalkEvent::Pressed)
        );
        filter.reset();
        assert_eq!(filter.filter(PushToTalkEvent::Released), None);
        assert_eq!(
            filter.filter(PushToTalkEvent::Pressed),
            Some(PushToTalkEvent::Pressed)
        );
    }
}
