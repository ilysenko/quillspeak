use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use shared::{AppConfig, ShortcutChord, ShortcutKey};
use tracing::{debug, info, warn};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{ConnectionExt, GrabMode, Keycode, Keysym, Mapping, ModMask};

use crate::command::AppCommand;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);

pub struct X11ThreadHandle {
    pub shutdown: Arc<AtomicBool>,
    pub join_handle: thread::JoinHandle<()>,
}

pub fn spawn(
    config: &AppConfig,
    command_tx: mpsc::Sender<AppCommand>,
) -> Result<Option<X11ThreadHandle>> {
    let shortcuts = config
        .enabled_keyboard_shortcuts()
        .map(|shortcut| {
            let accelerator = shortcut
                .trigger
                .keyboard_accelerator()
                .expect("enabled_keyboard_shortcuts yields keyboard triggers");
            let chord = ShortcutChord::parse(accelerator)
                .with_context(|| format!("failed to parse X11 shortcut {accelerator}"))?;
            Ok(X11ShortcutConfig {
                id: shortcut.id.clone(),
                name: shortcut.name.clone(),
                accelerator: accelerator.to_string(),
                chord,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if shortcuts.is_empty() {
        info!("X11 hotkey backend configured without enabled shortcuts");
        return Ok(None);
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    let thread_shutdown = Arc::clone(&shutdown);
    let (startup_tx, startup_rx) = mpsc::channel();

    let join_handle = thread::spawn(move || {
        if let Err(error) = run_x11_backend(shortcuts, command_tx, thread_shutdown, startup_tx) {
            warn!(?error, "X11 hotkey backend stopped");
        }
    });

    match startup_rx.recv_timeout(STARTUP_TIMEOUT) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            shutdown.store(true, Ordering::Relaxed);
            let _ = join_handle.join();
            bail!(error);
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            shutdown.store(true, Ordering::Relaxed);
            bail!("timed out waiting for X11 hotkey backend startup");
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            shutdown.store(true, Ordering::Relaxed);
            let _ = join_handle.join();
            bail!("X11 hotkey backend stopped before reporting startup");
        }
    }

    Ok(Some(X11ThreadHandle {
        shutdown,
        join_handle,
    }))
}

#[derive(Debug, Clone)]
struct X11ShortcutConfig {
    id: String,
    name: String,
    accelerator: String,
    chord: ShortcutChord,
}

fn run_x11_backend(
    shortcuts: Vec<X11ShortcutConfig>,
    command_tx: mpsc::Sender<AppCommand>,
    shutdown: Arc<AtomicBool>,
    startup_tx: mpsc::Sender<Result<(), String>>,
) -> Result<()> {
    let startup_result = (|| {
        let (connection, screen_num) = x11rb::connect(None).context("failed to connect to X11")?;
        let screen = &connection.setup().roots[screen_num];
        let root = screen.root;
        let mut grabs = Vec::with_capacity(shortcuts.len());

        for shortcut in shortcuts {
            let grab = X11Grab::resolve(&connection, shortcut.chord)?;

            if grab.trigger_keycodes.is_empty() {
                bail!(
                    "X11 shortcut trigger key has no keycode mapping for {}",
                    shortcut.accelerator
                );
            }

            for keycode in &grab.trigger_keycodes {
                for modifier_mask in modifier_mask_variants(shortcut.chord) {
                    connection
                        .grab_key(
                            false,
                            root,
                            modifier_mask,
                            *keycode,
                            GrabMode::ASYNC,
                            GrabMode::ASYNC,
                        )?
                        .check()
                        .with_context(|| {
                            format!(
                                "failed to grab X11 shortcut {}; it may already be in use",
                                shortcut.accelerator
                            )
                        })?;
                }
            }
            grabs.push(X11ShortcutGrab { shortcut, grab });
        }
        connection.flush()?;

        Ok((connection, root, grabs))
    })();

    let (connection, root, grabs) = match startup_result {
        Ok(startup) => {
            let _ = startup_tx.send(Ok(()));
            startup
        }
        Err(error) => {
            let _ = startup_tx.send(Err(format!("{error:#}")));
            return Err(error);
        }
    };

    info!(shortcut_count = grabs.len(), "X11 hotkey backend started");
    for grab in &grabs {
        info!(
            shortcut_id = %grab.shortcut.id,
            shortcut_name = %grab.shortcut.name,
            accelerator = %grab.shortcut.accelerator,
            keycode_count = grab.grab.trigger_keycodes.len(),
            "X11 shortcut grabbed"
        );
    }

    let mut active = HashSet::<String>::new();
    while !shutdown.load(Ordering::Relaxed) {
        match connection.poll_for_event()? {
            Some(Event::KeyPress(event)) => {
                debug!(keycode = event.detail, "X11 key press event");
                for shortcut_grab in &grabs {
                    if shortcut_grab.grab.trigger_keycodes.contains(&event.detail)
                        && shortcut_grab.grab.is_pressed(&connection)?
                        && active.insert(shortcut_grab.shortcut.id.clone())
                    {
                        let _ = command_tx.send(AppCommand::StartRecording(
                            shortcut_grab.shortcut.id.clone(),
                        ));
                    }
                }
            }
            Some(Event::KeyRelease(event)) => {
                debug!(keycode = event.detail, "X11 key release event");
                for shortcut_grab in &grabs {
                    if active.contains(&shortcut_grab.shortcut.id)
                        && !shortcut_grab.grab.is_pressed(&connection)?
                    {
                        active.remove(&shortcut_grab.shortcut.id);
                        let _ = command_tx
                            .send(AppCommand::StopRecording(shortcut_grab.shortcut.id.clone()));
                    }
                }
            }
            Some(Event::MappingNotify(event)) => {
                if event.request == Mapping::KEYBOARD || event.request == Mapping::MODIFIER {
                    warn!(
                        "X11 keyboard mapping changed; restart app or resave settings to refresh grabs"
                    );
                }
            }
            Some(_) => {}
            None => thread::sleep(Duration::from_millis(25)),
        }
    }

    for shortcut_grab in &grabs {
        for keycode in &shortcut_grab.grab.trigger_keycodes {
            for modifier_mask in modifier_mask_variants(shortcut_grab.shortcut.chord) {
                let _ = connection.ungrab_key(*keycode, root, modifier_mask);
            }
        }
    }
    let _ = connection.flush();

    for shortcut_id in active {
        let _ = command_tx.send(AppCommand::StopRecording(shortcut_id));
    }

    info!("X11 hotkey backend stopped");
    Ok(())
}

#[derive(Debug)]
struct X11ShortcutGrab {
    shortcut: X11ShortcutConfig,
    grab: X11Grab,
}

#[derive(Debug)]
struct X11Grab {
    trigger_keycodes: HashSet<Keycode>,
    required_keycode_groups: Vec<HashSet<Keycode>>,
}

impl X11Grab {
    fn resolve<C: Connection>(connection: &C, chord: ShortcutChord) -> Result<Self> {
        let mut required_keycode_groups = Vec::new();

        if chord.modifiers.ctrl {
            required_keycode_groups.push(keycodes_for_any_keysym(
                connection,
                &[KEYSYM_CONTROL_L, KEYSYM_CONTROL_R],
            )?);
        }
        if chord.modifiers.alt {
            required_keycode_groups.push(keycodes_for_any_keysym(
                connection,
                &[KEYSYM_ALT_L, KEYSYM_ALT_R],
            )?);
        }
        if chord.modifiers.shift {
            required_keycode_groups.push(keycodes_for_any_keysym(
                connection,
                &[KEYSYM_SHIFT_L, KEYSYM_SHIFT_R],
            )?);
        }
        if chord.modifiers.super_key {
            required_keycode_groups.push(keycodes_for_any_keysym(
                connection,
                &[KEYSYM_SUPER_L, KEYSYM_SUPER_R],
            )?);
        }

        let trigger_keycodes = keycodes_for_any_keysym(connection, &keysyms_for_key(chord.key))?;
        required_keycode_groups.push(trigger_keycodes.clone());

        Ok(Self {
            trigger_keycodes,
            required_keycode_groups,
        })
    }

    fn is_pressed<C: Connection>(&self, connection: &C) -> Result<bool> {
        let keymap = connection.query_keymap()?.reply()?.keys;
        Ok(self.required_keycode_groups.iter().all(|group| {
            group
                .iter()
                .any(|keycode| keycode_is_down(&keymap, *keycode))
        }))
    }
}

fn modifier_mask_variants(chord: ShortcutChord) -> Vec<ModMask> {
    let mut base = ModMask::default();
    if chord.modifiers.ctrl {
        base |= ModMask::CONTROL;
    }
    if chord.modifiers.alt {
        base |= ModMask::M1;
    }
    if chord.modifiers.shift {
        base |= ModMask::SHIFT;
    }
    if chord.modifiers.super_key {
        base |= ModMask::M4;
    }

    vec![
        base,
        base | ModMask::LOCK,
        base | ModMask::M2,
        base | ModMask::LOCK | ModMask::M2,
    ]
}

fn keycodes_for_any_keysym<C: Connection>(
    connection: &C,
    keysyms: &[Keysym],
) -> Result<HashSet<Keycode>> {
    let setup = connection.setup();
    let min_keycode = setup.min_keycode;
    let max_keycode = setup.max_keycode;
    let keycode_count = max_keycode
        .checked_sub(min_keycode)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| anyhow!("invalid X11 keycode range"))?;
    let keyboard_mapping = connection
        .get_keyboard_mapping(min_keycode, keycode_count)?
        .reply()?;
    let per_keycode = usize::from(keyboard_mapping.keysyms_per_keycode);
    let wanted = keysyms.iter().copied().collect::<HashSet<_>>();
    let mut keycodes = HashSet::new();

    for offset in 0..keycode_count {
        let offset = usize::from(offset);
        let start = offset * per_keycode;
        let end = start + per_keycode;
        let Some(symbols) = keyboard_mapping.keysyms.get(start..end) else {
            continue;
        };
        if symbols.iter().any(|keysym| wanted.contains(keysym)) {
            keycodes.insert(min_keycode + u8::try_from(offset)?);
        }
    }

    Ok(keycodes)
}

fn keycode_is_down(keymap: &[u8; 32], keycode: Keycode) -> bool {
    let byte_index = usize::from(keycode / 8);
    let bit_index = keycode % 8;
    keymap
        .get(byte_index)
        .is_some_and(|byte| byte & (1 << bit_index) != 0)
}

fn keysyms_for_key(key: ShortcutKey) -> Vec<Keysym> {
    match key {
        ShortcutKey::Character(character) if character.is_ascii_alphabetic() => {
            vec![
                character.to_ascii_uppercase() as Keysym,
                character.to_ascii_lowercase() as Keysym,
            ]
        }
        ShortcutKey::Character(character) => vec![character as Keysym],
        ShortcutKey::Space => vec![KEYSYM_SPACE],
        ShortcutKey::Escape => vec![KEYSYM_ESCAPE],
        ShortcutKey::Enter => vec![KEYSYM_RETURN],
        ShortcutKey::Tab => vec![KEYSYM_TAB],
        ShortcutKey::Backspace => vec![KEYSYM_BACKSPACE],
        ShortcutKey::Delete => vec![KEYSYM_DELETE],
        ShortcutKey::Insert => vec![KEYSYM_INSERT],
        ShortcutKey::Home => vec![KEYSYM_HOME],
        ShortcutKey::End => vec![KEYSYM_END],
        ShortcutKey::PageUp => vec![KEYSYM_PAGE_UP],
        ShortcutKey::PageDown => vec![KEYSYM_PAGE_DOWN],
        ShortcutKey::Function(number) => vec![KEYSYM_F1 + u32::from(number.saturating_sub(1))],
    }
}

const KEYSYM_BACKSPACE: Keysym = 0xff08;
const KEYSYM_TAB: Keysym = 0xff09;
const KEYSYM_RETURN: Keysym = 0xff0d;
const KEYSYM_ESCAPE: Keysym = 0xff1b;
const KEYSYM_HOME: Keysym = 0xff50;
const KEYSYM_PAGE_UP: Keysym = 0xff55;
const KEYSYM_END: Keysym = 0xff57;
const KEYSYM_PAGE_DOWN: Keysym = 0xff56;
const KEYSYM_INSERT: Keysym = 0xff63;
const KEYSYM_DELETE: Keysym = 0xffff;
const KEYSYM_F1: Keysym = 0xffbe;
const KEYSYM_SHIFT_L: Keysym = 0xffe1;
const KEYSYM_SHIFT_R: Keysym = 0xffe2;
const KEYSYM_CONTROL_L: Keysym = 0xffe3;
const KEYSYM_CONTROL_R: Keysym = 0xffe4;
const KEYSYM_ALT_L: Keysym = 0xffe9;
const KEYSYM_ALT_R: Keysym = 0xffea;
const KEYSYM_SUPER_L: Keysym = 0xffeb;
const KEYSYM_SUPER_R: Keysym = 0xffec;
const KEYSYM_SPACE: Keysym = 0x0020;

#[cfg(test)]
mod tests {
    use super::*;
    use shared::ShortcutModifiers;

    #[test]
    fn creates_lock_modifier_mask_variants() {
        let chord = ShortcutChord {
            modifiers: ShortcutModifiers {
                ctrl: true,
                alt: true,
                shift: false,
                super_key: false,
            },
            key: ShortcutKey::Space,
        };

        let variants = modifier_mask_variants(chord);

        assert_eq!(variants.len(), 4);
        assert!(variants.contains(&(ModMask::CONTROL | ModMask::M1)));
        assert!(variants.contains(&(ModMask::CONTROL | ModMask::M1 | ModMask::LOCK)));
        assert!(variants.contains(&(ModMask::CONTROL | ModMask::M1 | ModMask::M2)));
    }

    #[test]
    fn detects_keycode_down_in_x11_keymap() {
        let mut keymap = [0; 32];
        keymap[57 / 8] = 1 << (57 % 8);

        assert!(keycode_is_down(&keymap, 57));
        assert!(!keycode_is_down(&keymap, 58));
    }
}
