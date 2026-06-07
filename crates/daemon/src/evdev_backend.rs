use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use evdev::{Device, EventSummary, KeyCode};
use shared::{DaemonStatus, ShortcutChord, ShortcutKey, ShortcutRuntimeConfig};
use tracing::{debug, info, trace, warn};

use crate::app_client::AppClient;
use crate::hotkey::{
    HotkeyStateMachine, HotkeyTransition, KeyRequirement, KeyTransition, RuntimeShortcut,
};

const DEVICE_RESCAN_INTERVAL: Duration = Duration::from_secs(5);
const LOOP_SLEEP: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub struct EvdevHotkeyHandle {
    config_tx: mpsc::Sender<ShortcutRuntimeConfig>,
    permission_error: Arc<AtomicBool>,
    configured: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl EvdevHotkeyHandle {
    pub fn spawn(app_client: AppClient) -> Self {
        let (config_tx, config_rx) = mpsc::channel();
        let permission_error = Arc::new(AtomicBool::new(false));
        let configured = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));

        let join_handle = thread::spawn({
            let permission_error = Arc::clone(&permission_error);
            let shutdown = Arc::clone(&shutdown);
            move || run_evdev_backend(config_rx, permission_error, shutdown, app_client)
        });

        Self {
            config_tx,
            permission_error,
            configured,
            shutdown,
            join_handle: Some(join_handle),
        }
    }

    pub fn update_config(&self, config: &ShortcutRuntimeConfig) -> Result<DaemonStatus> {
        let status = match status_for_config(config) {
            Ok(status) => status,
            Err(error) => {
                self.permission_error.store(false, Ordering::Relaxed);
                self.configured.store(false, Ordering::Relaxed);
                if let Err(send_error) = self.config_tx.send(config.clone()) {
                    warn!(
                        ?send_error,
                        "failed to send invalid shortcut config reset to evdev backend"
                    );
                }
                return Err(error);
            }
        };
        self.permission_error
            .store(status == DaemonStatus::PermissionError, Ordering::Relaxed);
        self.configured
            .store(status == DaemonStatus::RunningConfigured, Ordering::Relaxed);

        if let Err(error) = self.config_tx.send(config.clone()) {
            self.permission_error.store(false, Ordering::Relaxed);
            self.configured.store(false, Ordering::Relaxed);
            return Err(error).context("failed to send shortcut config to evdev backend");
        }

        Ok(status)
    }

    pub fn current_status_for_config(
        &self,
        config: Option<&ShortcutRuntimeConfig>,
    ) -> DaemonStatus {
        if self.permission_error.load(Ordering::Relaxed) {
            return DaemonStatus::PermissionError;
        }

        match config {
            Some(_) if self.configured.load(Ordering::Relaxed) => DaemonStatus::RunningConfigured,
            _ => DaemonStatus::RunningUnconfigured,
        }
    }
}

impl Drop for EvdevHotkeyHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(join_handle) = self.join_handle.take()
            && let Err(error) = join_handle.join()
        {
            warn!(
                ?error,
                "evdev hotkey backend thread panicked during shutdown"
            );
        }
    }
}

fn run_evdev_backend(
    config_rx: mpsc::Receiver<ShortcutRuntimeConfig>,
    permission_error: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    app_client: AppClient,
) {
    let mut runtime = EvdevRuntime::new(app_client);

    info!("evdev hotkey backend thread started");
    while !shutdown.load(Ordering::Relaxed) {
        while let Ok(config) = config_rx.try_recv() {
            if let Err(error) = runtime.apply_config(&config, &permission_error) {
                warn!(?error, "failed to apply evdev shortcut config");
            }
        }

        if let Err(error) = runtime.poll_devices(&permission_error) {
            warn!(?error, "evdev polling failed");
        }

        thread::sleep(LOOP_SLEEP);
    }

    runtime.reset_active();
    info!("evdev hotkey backend thread stopped");
}

struct EvdevRuntime {
    app_client: AppClient,
    machines: Vec<EvdevShortcutMachine>,
    watched_keys: HashSet<KeyCode>,
    devices: Vec<KeyboardDevice>,
    device_set: Option<DeviceSetSnapshot>,
    last_scan: Option<Instant>,
}

struct EvdevShortcutMachine {
    shortcut_id: String,
    shortcut_name: String,
    accelerator: String,
    machine: HotkeyStateMachine<KeyCode>,
}

impl EvdevRuntime {
    fn new(app_client: AppClient) -> Self {
        Self {
            app_client,
            machines: Vec::new(),
            watched_keys: HashSet::new(),
            devices: Vec::new(),
            device_set: None,
            last_scan: None,
        }
    }

    fn apply_config(
        &mut self,
        config: &ShortcutRuntimeConfig,
        permission_error: &AtomicBool,
    ) -> Result<()> {
        let shortcuts = match RuntimeShortcut::from_config(config) {
            Ok(shortcuts) => shortcuts,
            Err(error) => {
                self.disable(permission_error);
                return Err(error);
            }
        };

        if shortcuts.is_empty() {
            info!("evdev hotkey backend has no configured shortcuts");
            self.disable(permission_error);
            return Ok(());
        }

        let mut machines = Vec::with_capacity(shortcuts.len());
        let mut watched_keys = HashSet::new();
        for shortcut in shortcuts {
            let requirements = match requirements_for_chord(shortcut.chord) {
                Ok(requirements) => requirements,
                Err(error) => {
                    self.disable(permission_error);
                    return Err(error);
                }
            };
            let machine = HotkeyStateMachine::new(requirements);
            watched_keys.extend(machine.watched_keys().iter().copied());
            machines.push(EvdevShortcutMachine {
                shortcut_id: shortcut.id,
                shortcut_name: shortcut.name,
                accelerator: shortcut.accelerator,
                machine,
            });
        }

        self.reset_active();
        self.watched_keys = watched_keys;
        self.machines = machines;
        self.devices.clear();
        self.device_set = None;
        self.last_scan = None;
        if let Err(error) = self.rescan_devices(permission_error) {
            permission_error.store(true, Ordering::Relaxed);
            return Err(error);
        }

        info!(
            shortcut_count = self.machines.len(),
            watched_key_count = self.watched_keys.len(),
            device_count = self.devices.len(),
            "evdev hotkey backend configured"
        );
        for machine in &self.machines {
            info!(
                shortcut_id = %machine.shortcut_id,
                shortcut_name = %machine.shortcut_name,
                accelerator = %machine.accelerator,
                "evdev shortcut configured"
            );
        }
        Ok(())
    }

    fn disable(&mut self, permission_error: &AtomicBool) {
        self.reset_active();
        self.machines.clear();
        self.watched_keys.clear();
        self.devices.clear();
        self.device_set = None;
        self.last_scan = None;
        permission_error.store(false, Ordering::Relaxed);
    }

    fn reset_active(&mut self) {
        for machine in &mut self.machines {
            if machine.machine.reset() == HotkeyTransition::HotkeyUp {
                self.app_client.send_hotkey_up(&machine.shortcut_id);
            }
        }
    }

    fn poll_devices(&mut self, permission_error: &AtomicBool) -> Result<()> {
        if self.machines.is_empty() {
            return Ok(());
        }

        if self
            .last_scan
            .is_none_or(|last_scan| last_scan.elapsed() >= DEVICE_RESCAN_INTERVAL)
        {
            self.rescan_devices(permission_error)?;
        }

        for device in &mut self.devices {
            loop {
                match device.device.fetch_events() {
                    Ok(events) => {
                        for event in events {
                            let EventSummary::Key(_, keycode, value) = event.destructure() else {
                                continue;
                            };
                            if !self.watched_keys.contains(&keycode) {
                                continue;
                            }

                            let transition = transition_from_evdev_value(value);
                            if transition == KeyTransition::Repeat {
                                continue;
                            }

                            trace!(
                                path = %device.path.display(),
                                name = %device.name,
                                keycode = keycode.code(),
                                value,
                                ?transition,
                                "evdev watched key event"
                            );
                            for machine in &mut self.machines {
                                if !machine.machine.watched_keys().contains(&keycode) {
                                    continue;
                                }
                                dispatch_transition(
                                    &self.app_client,
                                    &machine.shortcut_id,
                                    machine.machine.handle_key(keycode, transition),
                                );
                            }
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(error) => {
                        warn!(
                            ?error,
                            path = %device.path.display(),
                            "failed to fetch evdev events"
                        );
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    fn rescan_devices(&mut self, permission_error: &AtomicBool) -> Result<()> {
        let scan = open_devices_for_watched_keys(&self.watched_keys)?;
        permission_error.store(scan.permission_error, Ordering::Relaxed);
        let next_device_set = DeviceSetSnapshot::from_devices(&scan.devices);
        if self.device_set.as_ref() != Some(&next_device_set) {
            info!(
                device_count = next_device_set.devices.len(),
                devices = %next_device_set.display_label(),
                permission_error = scan.permission_error,
                "evdev keyboard device set changed"
            );
        } else {
            debug!(
                device_count = next_device_set.devices.len(),
                permission_error = scan.permission_error,
                "evdev keyboard device set unchanged"
            );
        }
        self.device_set = Some(next_device_set);
        self.devices = scan.devices;
        self.last_scan = Some(Instant::now());
        Ok(())
    }
}

fn dispatch_transition(app_client: &AppClient, shortcut_id: &str, transition: HotkeyTransition) {
    match transition {
        HotkeyTransition::HotkeyDown => {
            info!(shortcut_id, "evdev shortcut down");
            app_client.send_hotkey_down(shortcut_id);
        }
        HotkeyTransition::HotkeyUp => {
            info!(shortcut_id, "evdev shortcut up");
            app_client.send_hotkey_up(shortcut_id);
        }
        HotkeyTransition::None => {}
    }
}

fn transition_from_evdev_value(value: i32) -> KeyTransition {
    match value {
        0 => KeyTransition::Released,
        1 => KeyTransition::Pressed,
        2 => KeyTransition::Repeat,
        _ => KeyTransition::Repeat,
    }
}

fn status_for_config(config: &ShortcutRuntimeConfig) -> Result<DaemonStatus> {
    let shortcuts = match RuntimeShortcut::from_config(config) {
        Ok(shortcuts) if !shortcuts.is_empty() => shortcuts,
        Ok(_) => return Ok(DaemonStatus::RunningUnconfigured),
        Err(error) => return Err(error).context("daemon received invalid shortcut config"),
    };

    let mut watched_keys = HashSet::new();
    for shortcut in shortcuts {
        let requirements = requirements_for_chord(shortcut.chord)
            .context("daemon cannot map shortcut to evdev key codes")?;
        watched_keys.extend(HotkeyStateMachine::new(requirements).watched_keys().clone());
    }
    let scan =
        open_devices_for_watched_keys(&watched_keys).context("failed to probe evdev devices")?;
    Ok(status_from_device_scan(
        scan.permission_error,
        scan.devices.len(),
    ))
}

fn status_from_device_scan(permission_error: bool, device_count: usize) -> DaemonStatus {
    if permission_error {
        DaemonStatus::PermissionError
    } else if device_count == 0 {
        DaemonStatus::RunningUnconfigured
    } else {
        DaemonStatus::RunningConfigured
    }
}

fn requirements_for_chord(chord: ShortcutChord) -> Result<Vec<KeyRequirement<KeyCode>>> {
    let mut requirements = Vec::new();

    if chord.modifiers.ctrl {
        requirements.push(KeyRequirement::any([
            KeyCode::KEY_LEFTCTRL,
            KeyCode::KEY_RIGHTCTRL,
        ]));
    }
    if chord.modifiers.alt {
        requirements.push(KeyRequirement::any([
            KeyCode::KEY_LEFTALT,
            KeyCode::KEY_RIGHTALT,
        ]));
    }
    if chord.modifiers.shift {
        requirements.push(KeyRequirement::any([
            KeyCode::KEY_LEFTSHIFT,
            KeyCode::KEY_RIGHTSHIFT,
        ]));
    }
    if chord.modifiers.super_key {
        requirements.push(KeyRequirement::any([
            KeyCode::KEY_LEFTMETA,
            KeyCode::KEY_RIGHTMETA,
        ]));
    }

    requirements.push(KeyRequirement::any([keycode_for_shortcut_key(chord.key)?]));
    Ok(requirements)
}

fn keycode_for_shortcut_key(key: ShortcutKey) -> Result<KeyCode> {
    let keycode = match key {
        ShortcutKey::Character(character) => keycode_for_character(character)?,
        ShortcutKey::Space => KeyCode::KEY_SPACE,
        ShortcutKey::Escape => KeyCode::KEY_ESC,
        ShortcutKey::Enter => KeyCode::KEY_ENTER,
        ShortcutKey::Tab => KeyCode::KEY_TAB,
        ShortcutKey::Backspace => KeyCode::KEY_BACKSPACE,
        ShortcutKey::Delete => KeyCode::KEY_DELETE,
        ShortcutKey::Insert => KeyCode::KEY_INSERT,
        ShortcutKey::Home => KeyCode::KEY_HOME,
        ShortcutKey::End => KeyCode::KEY_END,
        ShortcutKey::PageUp => KeyCode::KEY_PAGEUP,
        ShortcutKey::PageDown => KeyCode::KEY_PAGEDOWN,
        ShortcutKey::Function(number) if (1..=24).contains(&number) => {
            KeyCode::new(KeyCode::KEY_F1.code() + u16::from(number - 1))
        }
        ShortcutKey::Function(number) => bail!("unsupported function key F{number}"),
    };

    Ok(keycode)
}

fn keycode_for_character(character: char) -> Result<KeyCode> {
    let keycode = match character.to_ascii_uppercase() {
        'A' => KeyCode::KEY_A,
        'B' => KeyCode::KEY_B,
        'C' => KeyCode::KEY_C,
        'D' => KeyCode::KEY_D,
        'E' => KeyCode::KEY_E,
        'F' => KeyCode::KEY_F,
        'G' => KeyCode::KEY_G,
        'H' => KeyCode::KEY_H,
        'I' => KeyCode::KEY_I,
        'J' => KeyCode::KEY_J,
        'K' => KeyCode::KEY_K,
        'L' => KeyCode::KEY_L,
        'M' => KeyCode::KEY_M,
        'N' => KeyCode::KEY_N,
        'O' => KeyCode::KEY_O,
        'P' => KeyCode::KEY_P,
        'Q' => KeyCode::KEY_Q,
        'R' => KeyCode::KEY_R,
        'S' => KeyCode::KEY_S,
        'T' => KeyCode::KEY_T,
        'U' => KeyCode::KEY_U,
        'V' => KeyCode::KEY_V,
        'W' => KeyCode::KEY_W,
        'X' => KeyCode::KEY_X,
        'Y' => KeyCode::KEY_Y,
        'Z' => KeyCode::KEY_Z,
        '0' => KeyCode::KEY_0,
        '1' => KeyCode::KEY_1,
        '2' => KeyCode::KEY_2,
        '3' => KeyCode::KEY_3,
        '4' => KeyCode::KEY_4,
        '5' => KeyCode::KEY_5,
        '6' => KeyCode::KEY_6,
        '7' => KeyCode::KEY_7,
        '8' => KeyCode::KEY_8,
        '9' => KeyCode::KEY_9,
        '-' => KeyCode::KEY_MINUS,
        '=' => KeyCode::KEY_EQUAL,
        ',' => KeyCode::KEY_COMMA,
        '.' => KeyCode::KEY_DOT,
        '/' => KeyCode::KEY_SLASH,
        ';' => KeyCode::KEY_SEMICOLON,
        '\'' => KeyCode::KEY_APOSTROPHE,
        '`' => KeyCode::KEY_GRAVE,
        '[' => KeyCode::KEY_LEFTBRACE,
        ']' => KeyCode::KEY_RIGHTBRACE,
        '\\' => KeyCode::KEY_BACKSLASH,
        other => return Err(anyhow!("unsupported evdev shortcut character {other:?}")),
    };

    Ok(keycode)
}

struct DeviceScan {
    devices: Vec<KeyboardDevice>,
    permission_error: bool,
}

struct KeyboardDevice {
    path: PathBuf,
    name: String,
    device: Device,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceSetSnapshot {
    devices: Vec<DeviceIdentity>,
}

impl DeviceSetSnapshot {
    fn from_devices(devices: &[KeyboardDevice]) -> Self {
        let mut devices = devices
            .iter()
            .map(|device| DeviceIdentity {
                path: device.path.clone(),
                name: device.name.clone(),
            })
            .collect::<Vec<_>>();
        devices.sort_by(|left, right| left.path.cmp(&right.path));
        Self { devices }
    }

    fn display_label(&self) -> String {
        if self.devices.is_empty() {
            return "none".to_string();
        }

        self.devices
            .iter()
            .map(|device| format!("{} ({})", device.name, device.path.display()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceIdentity {
    path: PathBuf,
    name: String,
}

fn open_devices_for_watched_keys(watched_keys: &HashSet<KeyCode>) -> Result<DeviceScan> {
    if watched_keys.is_empty() {
        return Ok(DeviceScan {
            devices: Vec::new(),
            permission_error: false,
        });
    }

    let entries = fs::read_dir("/dev/input").context("failed to read /dev/input")?;
    let mut devices = Vec::new();
    let mut saw_permission_error = false;

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                debug!(?error, "failed to read /dev/input entry");
                continue;
            }
        };
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("event"))
        {
            continue;
        }

        let device = match Device::open(&path) {
            Ok(device) => device,
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                saw_permission_error = true;
                debug!(path = %path.display(), "permission denied opening evdev device");
                continue;
            }
            Err(error) => {
                debug!(?error, path = %path.display(), "failed to open evdev device");
                continue;
            }
        };

        if !device_supports_any_key(&device, watched_keys) {
            continue;
        }

        device
            .set_nonblocking(true)
            .with_context(|| format!("failed to set {} nonblocking", path.display()))?;
        let name = device.name().unwrap_or("unknown").to_string();
        debug!(
            path = %path.display(),
            name = %name,
            "opened evdev keyboard device for watched shortcut keys"
        );
        devices.push(KeyboardDevice { path, name, device });
    }

    Ok(DeviceScan {
        permission_error: saw_permission_error && devices.is_empty(),
        devices,
    })
}

fn device_supports_any_key(device: &Device, watched_keys: &HashSet<KeyCode>) -> bool {
    device
        .supported_keys()
        .is_some_and(|keys| watched_keys.iter().any(|key| keys.contains(*key)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{CONFIG_SCHEMA_VERSION, ShortcutModifiers, ShortcutRuntimeBinding};

    #[test]
    fn maps_ctrl_space_to_evdev_requirements() {
        let chord = ShortcutChord {
            modifiers: ShortcutModifiers {
                ctrl: true,
                alt: false,
                shift: false,
                super_key: false,
            },
            key: ShortcutKey::Space,
        };

        let machine = HotkeyStateMachine::new(requirements_for_chord(chord).expect("requirements"));

        assert!(machine.watched_keys().contains(&KeyCode::KEY_LEFTCTRL));
        assert!(machine.watched_keys().contains(&KeyCode::KEY_RIGHTCTRL));
        assert!(machine.watched_keys().contains(&KeyCode::KEY_SPACE));
    }

    #[test]
    fn extracts_runtime_shortcuts() {
        let config = ShortcutRuntimeConfig {
            schema_version: CONFIG_SCHEMA_VERSION,
            shortcuts: vec![ShortcutRuntimeBinding {
                id: "default".to_string(),
                name: "Default".to_string(),
                accelerator: "Ctrl+Alt+F".to_string(),
                enabled: true,
            }],
        };

        let shortcuts = RuntimeShortcut::from_config(&config).expect("valid config");
        let shortcut = shortcuts.first().expect("shortcut");

        assert_eq!(shortcut.id, "default");
        assert_eq!(shortcut.name, "Default");
        assert_eq!(shortcut.accelerator, "Ctrl+Alt+F");
        assert!(shortcut.chord.modifiers.ctrl);
        assert!(shortcut.chord.modifiers.alt);
        assert_eq!(shortcut.chord.key, ShortcutKey::Character('F'));
    }

    #[test]
    fn rejects_runtime_shortcuts_with_wrong_schema() {
        let config = ShortcutRuntimeConfig {
            schema_version: CONFIG_SCHEMA_VERSION - 1,
            shortcuts: Vec::new(),
        };

        assert!(RuntimeShortcut::from_config(&config).is_err());
    }

    #[test]
    fn no_usable_evdev_devices_is_not_configured() {
        assert_eq!(
            status_from_device_scan(false, 0),
            DaemonStatus::RunningUnconfigured
        );
    }

    #[test]
    fn usable_evdev_devices_are_configured() {
        assert_eq!(
            status_from_device_scan(false, 1),
            DaemonStatus::RunningConfigured
        );
    }

    #[test]
    fn permission_error_without_usable_devices_wins() {
        assert_eq!(
            status_from_device_scan(true, 0),
            DaemonStatus::PermissionError
        );
    }

    #[test]
    fn evdev_autorepeat_maps_to_repeat_transition() {
        assert_eq!(transition_from_evdev_value(0), KeyTransition::Released);
        assert_eq!(transition_from_evdev_value(1), KeyTransition::Pressed);
        assert_eq!(transition_from_evdev_value(2), KeyTransition::Repeat);
        assert_eq!(transition_from_evdev_value(99), KeyTransition::Repeat);
    }

    #[test]
    fn device_set_snapshot_is_order_independent() {
        let devices = [
            DeviceIdentity {
                path: PathBuf::from("/dev/input/event2"),
                name: "Second".to_string(),
            },
            DeviceIdentity {
                path: PathBuf::from("/dev/input/event1"),
                name: "First".to_string(),
            },
        ];
        let mut unsorted = devices.to_vec();
        let mut sorted = devices.to_vec();
        sorted.reverse();

        unsorted.sort_by(|left, right| left.path.cmp(&right.path));
        sorted.sort_by(|left, right| left.path.cmp(&right.path));

        assert_eq!(unsorted, sorted);
    }
}
