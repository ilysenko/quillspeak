use std::collections::HashSet;
use std::hash::Hash;

use anyhow::{Context, Result};
use shared::{ShortcutAction, ShortcutChord, ShortcutRuntimeConfig};

#[derive(Debug, Clone)]
pub struct RuntimeShortcut {
    pub accelerator: String,
    pub chord: ShortcutChord,
}

impl RuntimeShortcut {
    pub fn push_to_talk_from_config(config: &ShortcutRuntimeConfig) -> Result<Option<Self>> {
        let Some(binding) = config.shortcuts.iter().find(|binding| {
            binding.enabled && binding.action() == Some(ShortcutAction::PushToTalk)
        }) else {
            return Ok(None);
        };

        if binding.accelerator.trim().is_empty() {
            return Ok(None);
        }

        let chord = ShortcutChord::parse(&binding.accelerator)
            .with_context(|| format!("failed to parse shortcut {}", binding.accelerator))?;

        Ok(Some(Self {
            accelerator: binding.accelerator.clone(),
            chord,
        }))
    }
}

#[derive(Debug, Clone)]
pub struct KeyRequirement<K> {
    alternatives: HashSet<K>,
}

impl<K> KeyRequirement<K>
where
    K: Copy + Eq + Hash,
{
    pub fn any(alternatives: impl IntoIterator<Item = K>) -> Self {
        Self {
            alternatives: alternatives.into_iter().collect(),
        }
    }

    fn is_satisfied(&self, pressed: &HashSet<K>) -> bool {
        self.alternatives.iter().any(|key| pressed.contains(key))
    }

    fn watched_keys(&self) -> impl Iterator<Item = K> + '_ {
        self.alternatives.iter().copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTransition {
    Pressed,
    Released,
    Repeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyTransition {
    None,
    HotkeyDown,
    HotkeyUp,
}

#[derive(Debug)]
pub struct HotkeyStateMachine<K> {
    requirements: Vec<KeyRequirement<K>>,
    watched_keys: HashSet<K>,
    pressed: HashSet<K>,
    active: bool,
}

impl<K> HotkeyStateMachine<K>
where
    K: Copy + Eq + Hash,
{
    pub fn new(requirements: Vec<KeyRequirement<K>>) -> Self {
        let watched_keys = requirements
            .iter()
            .flat_map(KeyRequirement::watched_keys)
            .collect();

        Self {
            requirements,
            watched_keys,
            pressed: HashSet::new(),
            active: false,
        }
    }

    pub fn watched_keys(&self) -> &HashSet<K> {
        &self.watched_keys
    }

    pub fn handle_key(&mut self, key: K, transition: KeyTransition) -> HotkeyTransition {
        if !self.watched_keys.contains(&key) {
            return HotkeyTransition::None;
        }

        match transition {
            KeyTransition::Pressed => {
                self.pressed.insert(key);
                if !self.active && self.is_satisfied() {
                    self.active = true;
                    HotkeyTransition::HotkeyDown
                } else {
                    HotkeyTransition::None
                }
            }
            KeyTransition::Released => {
                self.pressed.remove(&key);
                if self.active && !self.is_satisfied() {
                    self.active = false;
                    HotkeyTransition::HotkeyUp
                } else {
                    HotkeyTransition::None
                }
            }
            KeyTransition::Repeat => HotkeyTransition::None,
        }
    }

    pub fn reset(&mut self) -> HotkeyTransition {
        self.pressed.clear();
        if self.active {
            self.active = false;
            HotkeyTransition::HotkeyUp
        } else {
            HotkeyTransition::None
        }
    }

    fn is_satisfied(&self) -> bool {
        self.requirements
            .iter()
            .all(|requirement| requirement.is_satisfied(&self.pressed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT_CTRL: u16 = 29;
    const RIGHT_CTRL: u16 = 97;
    const SPACE: u16 = 57;
    const A: u16 = 30;

    fn ctrl_space_machine() -> HotkeyStateMachine<u16> {
        HotkeyStateMachine::new(vec![
            KeyRequirement::any([LEFT_CTRL, RIGHT_CTRL]),
            KeyRequirement::any([SPACE]),
        ])
    }

    #[test]
    fn starts_only_when_all_required_keys_are_down() {
        let mut machine = ctrl_space_machine();

        assert_eq!(
            machine.handle_key(LEFT_CTRL, KeyTransition::Pressed),
            HotkeyTransition::None
        );
        assert_eq!(
            machine.handle_key(SPACE, KeyTransition::Pressed),
            HotkeyTransition::HotkeyDown
        );
    }

    #[test]
    fn stops_when_any_required_key_is_released() {
        let mut machine = ctrl_space_machine();
        machine.handle_key(LEFT_CTRL, KeyTransition::Pressed);
        machine.handle_key(SPACE, KeyTransition::Pressed);

        assert_eq!(
            machine.handle_key(LEFT_CTRL, KeyTransition::Released),
            HotkeyTransition::HotkeyUp
        );
    }

    #[test]
    fn ignores_repeats_and_unrelated_keys() {
        let mut machine = ctrl_space_machine();
        machine.handle_key(LEFT_CTRL, KeyTransition::Pressed);
        machine.handle_key(SPACE, KeyTransition::Pressed);

        assert_eq!(
            machine.handle_key(SPACE, KeyTransition::Repeat),
            HotkeyTransition::None
        );
        assert_eq!(
            machine.handle_key(A, KeyTransition::Pressed),
            HotkeyTransition::None
        );
    }

    #[test]
    fn accepts_left_or_right_modifier_aliases() {
        let mut machine = ctrl_space_machine();

        machine.handle_key(RIGHT_CTRL, KeyTransition::Pressed);
        assert_eq!(
            machine.handle_key(SPACE, KeyTransition::Pressed),
            HotkeyTransition::HotkeyDown
        );
    }

    #[test]
    fn reset_releases_active_hotkey() {
        let mut machine = ctrl_space_machine();
        machine.handle_key(LEFT_CTRL, KeyTransition::Pressed);
        machine.handle_key(SPACE, KeyTransition::Pressed);

        assert_eq!(machine.reset(), HotkeyTransition::HotkeyUp);
        assert_eq!(machine.reset(), HotkeyTransition::None);
    }
}
