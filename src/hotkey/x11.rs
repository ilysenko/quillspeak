use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};

use crate::activity::PushToTalkEvent;

use super::{HotkeyEdgeFilter, HotkeySpec, SharedHotkeyHandler, dispatch_filtered_push_to_talk};

pub(super) struct X11HotkeyBackend {
    handler: SharedHotkeyHandler,
    edge_filter: Arc<HotkeyEdgeFilter>,
    active_hotkey_id: Arc<Mutex<Option<u32>>>,
    listener_started: AtomicBool,
    state: Mutex<X11State>,
}

#[derive(Default)]
struct X11State {
    configured_hotkey: Option<String>,
    manager: Option<GlobalHotKeyManager>,
    hotkey: Option<HotKey>,
}

impl X11HotkeyBackend {
    pub(super) fn new(handler: SharedHotkeyHandler) -> Self {
        Self {
            handler,
            edge_filter: Arc::new(HotkeyEdgeFilter::default()),
            active_hotkey_id: Arc::new(Mutex::new(None)),
            listener_started: AtomicBool::new(false),
            state: Mutex::new(X11State::default()),
        }
    }

    pub(super) fn configure(&self, spec: &HotkeySpec) -> Result<()> {
        self.ensure_listener_started()?;

        let mut state = self.state.lock().expect("X11 hotkey state was poisoned");
        if state
            .configured_hotkey
            .as_deref()
            .is_some_and(|hotkey| hotkey == spec.canonical())
        {
            return Ok(());
        }

        let hotkey = spec.to_global_hotkey()?;
        if state.manager.is_none() {
            state.manager = Some(
                GlobalHotKeyManager::new().context("failed to initialize X11 hotkey manager")?,
            );
        }
        state
            .manager
            .as_ref()
            .expect("X11 hotkey manager should be initialized")
            .register(hotkey)
            .with_context(|| format!("failed to register X11 hotkey `{}`", spec.canonical()))?;

        let old_hotkey = state.hotkey.replace(hotkey);
        state.configured_hotkey = Some(spec.canonical().to_string());
        self.edge_filter.reset();
        self.active_hotkey_id
            .lock()
            .expect("X11 active hotkey id was poisoned")
            .replace(hotkey.id());

        if let Some(old_hotkey) = old_hotkey
            && old_hotkey.id() != hotkey.id()
            && let Some(manager) = state.manager.as_ref()
            && let Err(error) = manager.unregister(old_hotkey)
        {
            eprintln!("Failed to unregister previous X11 hotkey: {error:#}");
        }

        Ok(())
    }

    pub(super) fn deactivate(&self) -> Result<()> {
        let mut state = self.state.lock().expect("X11 hotkey state was poisoned");
        let old_hotkey = state.hotkey.take();
        if let Some(hotkey) = old_hotkey
            && let Some(manager) = state.manager.as_ref()
        {
            manager
                .unregister(hotkey)
                .context("failed to unregister X11 hotkey")?;
        }
        state.configured_hotkey = None;
        self.edge_filter.reset();
        self.active_hotkey_id
            .lock()
            .expect("X11 active hotkey id was poisoned")
            .take();
        Ok(())
    }

    fn ensure_listener_started(&self) -> Result<()> {
        if self
            .listener_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(());
        }

        let handler = Arc::clone(&self.handler);
        let edge_filter = Arc::clone(&self.edge_filter);
        let active_hotkey_id = Arc::clone(&self.active_hotkey_id);
        if let Err(error) = thread::Builder::new()
            .name("voice-x11-hotkey".to_string())
            .spawn(move || {
                while let Ok(event) = GlobalHotKeyEvent::receiver().recv() {
                    let active_id = *active_hotkey_id
                        .lock()
                        .expect("X11 active hotkey id was poisoned");
                    if active_id != Some(event.id) {
                        continue;
                    }

                    let event = match event.state {
                        HotKeyState::Pressed => PushToTalkEvent::Pressed,
                        HotKeyState::Released => PushToTalkEvent::Released,
                    };
                    dispatch_filtered_push_to_talk(&handler, &edge_filter, event);
                }
            })
        {
            self.listener_started.store(false, Ordering::SeqCst);
            return Err(error).context("failed to spawn X11 hotkey listener");
        }

        Ok(())
    }
}
