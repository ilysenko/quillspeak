use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use evdev::{Device, EventSummary, KeyCode};

use crate::activity::PushToTalkEvent;

use super::{
    HotkeySpec, SharedHotkeyHandler, dispatch_push_to_talk, hotkey_debug_enabled,
    spec::EvdevHotkeySpec,
};

const POLL_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvdevDispatchMode {
    Full,
    ReleaseGuard,
}

pub(super) struct EvdevHotkeyBackend {
    handler: SharedHotkeyHandler,
    state: Mutex<EvdevState>,
}

#[derive(Default)]
struct EvdevState {
    configured_hotkey: Option<String>,
    dispatch_mode: Option<EvdevDispatchMode>,
    registration: Option<EvdevRegistration>,
}

impl EvdevHotkeyBackend {
    pub(super) fn new(handler: SharedHotkeyHandler) -> Self {
        Self {
            handler,
            state: Mutex::new(EvdevState::default()),
        }
    }

    pub(super) fn configure(&self, spec: &HotkeySpec) -> Result<()> {
        self.configure_with_mode(spec, EvdevDispatchMode::Full)
    }

    pub(super) fn configure_release_guard(&self, spec: &HotkeySpec) -> Result<()> {
        self.configure_with_mode(spec, EvdevDispatchMode::ReleaseGuard)
    }

    fn configure_with_mode(
        &self,
        spec: &HotkeySpec,
        dispatch_mode: EvdevDispatchMode,
    ) -> Result<()> {
        let mut state = self.state.lock().expect("evdev hotkey state was poisoned");
        if state
            .configured_hotkey
            .as_deref()
            .is_some_and(|hotkey| hotkey == spec.canonical())
            && state.dispatch_mode == Some(dispatch_mode)
            && state
                .registration
                .as_ref()
                .is_some_and(|registration| !registration.is_finished())
        {
            return Ok(());
        }

        let next_registration =
            EvdevRegistration::start(spec.clone(), self.handler.clone(), dispatch_mode)?;
        let old_registration = state.registration.replace(next_registration);
        state.configured_hotkey = Some(spec.canonical().to_string());
        state.dispatch_mode = Some(dispatch_mode);
        drop(state);
        drop(old_registration);

        Ok(())
    }

    pub(super) fn deactivate(&self) -> Result<()> {
        let mut state = self.state.lock().expect("evdev hotkey state was poisoned");
        let old_registration = state.registration.take();
        state.configured_hotkey = None;
        state.dispatch_mode = None;
        drop(state);
        drop(old_registration);
        Ok(())
    }
}

struct EvdevRegistration {
    shutdown_sender: Option<mpsc::Sender<()>>,
    join_handle: Option<JoinHandle<()>>,
}

impl EvdevRegistration {
    fn start(
        spec: HotkeySpec,
        handler: SharedHotkeyHandler,
        dispatch_mode: EvdevDispatchMode,
    ) -> Result<Self> {
        let canonical_hotkey = spec.canonical().to_string();
        let (ready_sender, ready_receiver) =
            mpsc::sync_channel::<std::result::Result<(), String>>(1);
        let (shutdown_sender, shutdown_receiver) = mpsc::channel();
        let thread_hotkey = canonical_hotkey.clone();

        let join_handle = thread::Builder::new()
            .name("voice-evdev-hotkey".to_string())
            .spawn(move || {
                run_evdev_thread(
                    spec,
                    handler,
                    dispatch_mode,
                    shutdown_receiver,
                    ready_sender,
                );
            })
            .context("failed to spawn evdev hotkey worker")?;

        match ready_receiver
            .recv()
            .context("evdev hotkey worker stopped before reporting registration status")?
        {
            Ok(()) => Ok(Self {
                shutdown_sender: Some(shutdown_sender),
                join_handle: Some(join_handle),
            }),
            Err(message) => {
                let _ = join_handle.join();
                Err(anyhow!(
                    "failed to bind evdev shortcut `{thread_hotkey}`: {message}"
                ))
            }
        }
    }

    fn is_finished(&self) -> bool {
        self.join_handle
            .as_ref()
            .is_some_and(|join_handle| join_handle.is_finished())
    }
}

impl Drop for EvdevRegistration {
    fn drop(&mut self) {
        if let Some(shutdown_sender) = self.shutdown_sender.take() {
            let _ = shutdown_sender.send(());
        }

        if let Some(join_handle) = self.join_handle.take()
            && join_handle.thread().id() != thread::current().id()
        {
            let _ = join_handle.join();
        }
    }
}

fn run_evdev_thread(
    spec: HotkeySpec,
    handler: SharedHotkeyHandler,
    dispatch_mode: EvdevDispatchMode,
    shutdown_receiver: mpsc::Receiver<()>,
    ready_sender: SyncSender<std::result::Result<(), String>>,
) {
    let result = run_evdev_worker(
        spec,
        handler,
        dispatch_mode,
        shutdown_receiver,
        ready_sender.clone(),
    );
    if let Err(error) = result {
        let message = format!("{error:#}");
        if ready_sender.send(Err(message.clone())).is_err() {
            eprintln!("Evdev hotkey worker stopped: {message}");
        }
    }
}

fn run_evdev_worker(
    spec: HotkeySpec,
    handler: SharedHotkeyHandler,
    dispatch_mode: EvdevDispatchMode,
    shutdown_receiver: mpsc::Receiver<()>,
    ready_sender: SyncSender<std::result::Result<(), String>>,
) -> Result<()> {
    let trigger = spec.to_evdev_hotkey();
    let mut devices = open_matching_devices(&trigger)?;
    let device_count = devices.len();

    let _ = ready_sender.send(Ok(()));
    match dispatch_mode {
        EvdevDispatchMode::Full => eprintln!(
            "Registered evdev push-to-talk hotkey `{}` across {} input device(s).",
            spec.canonical(),
            device_count
        ),
        EvdevDispatchMode::ReleaseGuard => {
            if hotkey_debug_enabled() {
                eprintln!(
                    "Registered evdev push-to-talk release guard `{}` across {} input device(s).",
                    spec.canonical(),
                    device_count
                );
            }
        }
    }

    let mut state = EvdevHotkeyState::default();
    loop {
        if shutdown_receiver.try_recv().is_ok() {
            dispatch_state_reset(&handler, &mut state, dispatch_mode);
            return Ok(());
        }

        let mut failed_devices = Vec::new();
        for (index, opened_device) in devices.iter_mut().enumerate() {
            match opened_device.device.fetch_events() {
                Ok(events) => {
                    for event in events {
                        let EventSummary::Key(_, key, value) = event.destructure() else {
                            continue;
                        };

                        if let Some(event) =
                            state.apply_key_event(opened_device.id, key, value, &trigger)
                        {
                            dispatch_evdev_push_to_talk(&handler, event, dispatch_mode);
                        }
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => {
                    eprintln!(
                        "Evdev hotkey device `{}` stopped producing events: {error}",
                        opened_device.path.display()
                    );
                    failed_devices.push(index);
                }
            }
        }

        for index in failed_devices.into_iter().rev() {
            let opened_device = devices.swap_remove(index);
            if let Some(event) = state.remove_device(opened_device.id, &trigger) {
                dispatch_evdev_push_to_talk(&handler, event, dispatch_mode);
            }
        }

        if devices.is_empty() {
            dispatch_state_reset(&handler, &mut state, dispatch_mode);
            bail!("all evdev hotkey input devices became unavailable");
        }

        thread::sleep(POLL_INTERVAL);
    }
}

fn dispatch_state_reset(
    handler: &SharedHotkeyHandler,
    state: &mut EvdevHotkeyState,
    dispatch_mode: EvdevDispatchMode,
) {
    if let Some(event) = state.reset() {
        dispatch_evdev_push_to_talk(handler, event, dispatch_mode);
    }
}

fn dispatch_evdev_push_to_talk(
    handler: &SharedHotkeyHandler,
    event: PushToTalkEvent,
    dispatch_mode: EvdevDispatchMode,
) {
    if dispatch_mode == EvdevDispatchMode::ReleaseGuard && event != PushToTalkEvent::Released {
        if hotkey_debug_enabled() {
            eprintln!("Evdev release guard observed push-to-talk event: {event:?}");
        }
        return;
    }

    if hotkey_debug_enabled() {
        match dispatch_mode {
            EvdevDispatchMode::Full => eprintln!("Evdev push-to-talk event: {event:?}"),
            EvdevDispatchMode::ReleaseGuard => {
                eprintln!("Evdev release guard push-to-talk event: {event:?}");
            }
        }
    }
    dispatch_push_to_talk(handler, event);
}

struct OpenedEvdevDevice {
    id: usize,
    path: PathBuf,
    device: Device,
}

fn open_matching_devices(trigger: &EvdevHotkeySpec) -> Result<Vec<OpenedEvdevDevice>> {
    let entries =
        fs::read_dir("/dev/input").context("failed to read /dev/input for evdev hotkeys")?;
    let relevant_keys = trigger.relevant_keys();
    let mut saw_event_node = false;
    let mut permission_denied_count = 0;
    let mut open_error_count = 0;
    let mut next_device_id = 0;
    let mut devices = Vec::new();

    for entry in entries {
        let Ok(entry) = entry else {
            open_error_count += 1;
            continue;
        };
        let path = entry.path();
        if !is_event_device_path(&path) {
            continue;
        }

        saw_event_node = true;
        let device = match Device::open(&path) {
            Ok(device) => device,
            Err(error) => {
                open_error_count += 1;
                if error.kind() == ErrorKind::PermissionDenied {
                    permission_denied_count += 1;
                }
                continue;
            }
        };

        if !device_supports_any_key(&device, &relevant_keys) {
            continue;
        }

        device
            .set_nonblocking(true)
            .with_context(|| format!("failed to enable nonblocking mode for {}", path.display()))?;

        devices.push(OpenedEvdevDevice {
            id: next_device_id,
            path,
            device,
        });
        next_device_id += 1;
    }

    if devices.is_empty() {
        if !saw_event_node {
            bail!("no /dev/input/event* devices were found");
        }
        if permission_denied_count > 0 {
            bail!(
                "no readable evdev keyboard devices matched the hotkey; {} input device(s) denied access. Add the user to the `input` group or install a permission helper",
                permission_denied_count
            );
        }
        if open_error_count > 0 {
            bail!(
                "no readable evdev keyboard devices matched the hotkey; {} input device(s) could not be opened",
                open_error_count
            );
        }
        bail!("no readable evdev keyboard devices support the configured hotkey");
    }

    Ok(devices)
}

fn is_event_device_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("event"))
}

fn device_supports_any_key(device: &Device, keys: &[KeyCode]) -> bool {
    device
        .supported_keys()
        .is_some_and(|supported_keys| keys.iter().any(|key| supported_keys.contains(*key)))
}

#[derive(Default)]
struct EvdevHotkeyState {
    pressed_by_key: HashMap<KeyCode, HashSet<usize>>,
    combo_active: bool,
}

impl EvdevHotkeyState {
    fn apply_key_event(
        &mut self,
        device_id: usize,
        key: KeyCode,
        value: i32,
        trigger: &EvdevHotkeySpec,
    ) -> Option<PushToTalkEvent> {
        if !trigger.contains_key(key) {
            return None;
        }

        match value {
            0 => self.release_key(device_id, key),
            1 => self.press_key(device_id, key),
            2 => return None,
            _ => return None,
        }

        self.update_combo_state(trigger)
    }

    fn remove_device(
        &mut self,
        device_id: usize,
        trigger: &EvdevHotkeySpec,
    ) -> Option<PushToTalkEvent> {
        for pressed_devices in self.pressed_by_key.values_mut() {
            pressed_devices.remove(&device_id);
        }
        self.pressed_by_key
            .retain(|_, pressed_devices| !pressed_devices.is_empty());

        self.update_combo_state(trigger)
    }

    fn reset(&mut self) -> Option<PushToTalkEvent> {
        self.pressed_by_key.clear();
        if self.combo_active {
            self.combo_active = false;
            Some(PushToTalkEvent::Released)
        } else {
            None
        }
    }

    fn press_key(&mut self, device_id: usize, key: KeyCode) {
        self.pressed_by_key
            .entry(key)
            .or_default()
            .insert(device_id);
    }

    fn release_key(&mut self, device_id: usize, key: KeyCode) {
        if let Some(pressed_devices) = self.pressed_by_key.get_mut(&key) {
            pressed_devices.remove(&device_id);
            if pressed_devices.is_empty() {
                self.pressed_by_key.remove(&key);
            }
        }
    }

    fn update_combo_state(&mut self, trigger: &EvdevHotkeySpec) -> Option<PushToTalkEvent> {
        let next_active = self.combo_is_active(trigger);
        match (self.combo_active, next_active) {
            (false, true) => {
                self.combo_active = true;
                Some(PushToTalkEvent::Pressed)
            }
            (true, false) => {
                self.combo_active = false;
                Some(PushToTalkEvent::Released)
            }
            _ => None,
        }
    }

    fn combo_is_active(&self, trigger: &EvdevHotkeySpec) -> bool {
        trigger
            .modifier_groups()
            .iter()
            .all(|group| group.keys().iter().any(|key| self.key_is_pressed(*key)))
            && self.key_is_pressed(trigger.main_key())
    }

    fn key_is_pressed(&self, key: KeyCode) -> bool {
        self.pressed_by_key
            .get(&key)
            .is_some_and(|devices| !devices.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger() -> EvdevHotkeySpec {
        HotkeySpec::parse("Ctrl+Alt+Space")
            .unwrap()
            .to_evdev_hotkey()
    }

    #[test]
    fn state_emits_press_once_until_release() {
        let trigger = trigger();
        let mut state = EvdevHotkeyState::default();

        assert_eq!(
            state.apply_key_event(0, KeyCode::KEY_LEFTCTRL, 1, &trigger),
            None
        );
        assert_eq!(
            state.apply_key_event(0, KeyCode::KEY_LEFTALT, 1, &trigger),
            None
        );
        assert_eq!(
            state.apply_key_event(0, KeyCode::KEY_SPACE, 1, &trigger),
            Some(PushToTalkEvent::Pressed)
        );
        assert_eq!(
            state.apply_key_event(0, KeyCode::KEY_SPACE, 2, &trigger),
            None
        );
        assert_eq!(
            state.apply_key_event(0, KeyCode::KEY_SPACE, 0, &trigger),
            Some(PushToTalkEvent::Released)
        );
    }

    #[test]
    fn state_allows_modifiers_and_key_across_devices() {
        let trigger = trigger();
        let mut state = EvdevHotkeyState::default();

        assert_eq!(
            state.apply_key_event(0, KeyCode::KEY_LEFTCTRL, 1, &trigger),
            None
        );
        assert_eq!(
            state.apply_key_event(1, KeyCode::KEY_RIGHTALT, 1, &trigger),
            None
        );
        assert_eq!(
            state.apply_key_event(2, KeyCode::KEY_SPACE, 1, &trigger),
            Some(PushToTalkEvent::Pressed)
        );
    }

    #[test]
    fn state_keeps_key_pressed_until_all_devices_release_it() {
        let trigger = trigger();
        let mut state = EvdevHotkeyState::default();

        assert_eq!(
            state.apply_key_event(0, KeyCode::KEY_LEFTCTRL, 1, &trigger),
            None
        );
        assert_eq!(
            state.apply_key_event(1, KeyCode::KEY_LEFTCTRL, 1, &trigger),
            None
        );
        assert_eq!(
            state.apply_key_event(0, KeyCode::KEY_LEFTALT, 1, &trigger),
            None
        );
        assert_eq!(
            state.apply_key_event(0, KeyCode::KEY_SPACE, 1, &trigger),
            Some(PushToTalkEvent::Pressed)
        );
        assert_eq!(
            state.apply_key_event(0, KeyCode::KEY_LEFTCTRL, 0, &trigger),
            None
        );
        assert_eq!(
            state.apply_key_event(1, KeyCode::KEY_LEFTCTRL, 0, &trigger),
            Some(PushToTalkEvent::Released)
        );
    }

    #[test]
    fn state_reset_releases_active_combo() {
        let trigger = trigger();
        let mut state = EvdevHotkeyState::default();

        state.apply_key_event(0, KeyCode::KEY_LEFTCTRL, 1, &trigger);
        state.apply_key_event(0, KeyCode::KEY_LEFTALT, 1, &trigger);
        assert_eq!(
            state.apply_key_event(0, KeyCode::KEY_SPACE, 1, &trigger),
            Some(PushToTalkEvent::Pressed)
        );

        assert_eq!(state.reset(), Some(PushToTalkEvent::Released));
        assert_eq!(state.reset(), None);
    }

    #[test]
    fn state_removing_device_releases_active_combo_if_needed() {
        let trigger = trigger();
        let mut state = EvdevHotkeyState::default();

        state.apply_key_event(0, KeyCode::KEY_LEFTCTRL, 1, &trigger);
        state.apply_key_event(0, KeyCode::KEY_LEFTALT, 1, &trigger);
        state.apply_key_event(0, KeyCode::KEY_SPACE, 1, &trigger);

        assert_eq!(
            state.remove_device(0, &trigger),
            Some(PushToTalkEvent::Released)
        );
    }
}
