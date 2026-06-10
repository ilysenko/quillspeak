use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use shared::{
    AppConfig, ConfigError, DEFAULT_MODEL_ID, DEFAULT_SHORTCUT_ID, ShortcutProfile,
    ShortcutTrigger, next_shortcut_id,
};

use crate::hotkey::ShortcutTriggerCapabilities;

#[derive(Clone)]
pub struct SettingsDraft {
    config: Rc<RefCell<AppConfig>>,
}

impl SettingsDraft {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config: Rc::new(RefCell::new(config)),
        }
    }

    pub fn replace(&self, config: AppConfig) {
        self.config.replace(config);
    }

    pub fn coerce_trigger_capabilities(&self, capabilities: ShortcutTriggerCapabilities) {
        if capabilities.keyboard_available() {
            return;
        }

        let mut occupied_signals = HashSet::new();
        for shortcut in &mut self.config.borrow_mut().shortcuts {
            if matches!(shortcut.trigger, ShortcutTrigger::Keyboard { .. }) {
                shortcut.trigger = ShortcutTrigger::default_linux_signal();
                if shortcut.enabled && signal_keys_conflict(shortcut, &occupied_signals) {
                    shortcut.enabled = false;
                }
            }

            if shortcut.enabled {
                if signal_keys_conflict(shortcut, &occupied_signals) {
                    shortcut.enabled = false;
                } else {
                    insert_shortcut_signal_keys(shortcut, &mut occupied_signals);
                }
            }
        }
    }

    pub fn snapshot(&self) -> AppConfig {
        self.config.borrow().clone()
    }

    pub fn normalized(&self) -> Result<AppConfig, ConfigError> {
        self.snapshot().normalized()
    }

    pub fn update<F>(&self, update: F)
    where
        F: FnOnce(&mut AppConfig),
    {
        update(&mut self.config.borrow_mut());
    }

    pub fn update_shortcut<F>(&self, shortcut_id: &str, update: F)
    where
        F: FnOnce(&mut ShortcutProfile),
    {
        if let Some(shortcut) = self
            .config
            .borrow_mut()
            .shortcuts
            .iter_mut()
            .find(|shortcut| shortcut.id == shortcut_id)
        {
            update(shortcut);
        }
    }

    pub fn add_shortcut(
        &self,
        capabilities: ShortcutTriggerCapabilities,
        model_id: String,
    ) -> ShortcutProfile {
        let mut config = self.config.borrow_mut();
        let id = next_shortcut_id(&config.shortcuts);
        let name = format!("Shortcut {}", config.shortcuts.len() + 1);
        let mut shortcut = ShortcutProfile::new_profile(id, name, model_id);
        if !capabilities.keyboard_available() {
            shortcut.trigger = ShortcutTrigger::default_linux_signal();
            shortcut.enabled = !shortcut_signal_keys(&shortcut).is_some_and(|keys| {
                config
                    .shortcuts
                    .iter()
                    .filter(|shortcut| shortcut.enabled)
                    .flat_map(|shortcut| shortcut_signal_keys(shortcut).unwrap_or_default())
                    .any(|existing| keys.contains(&existing))
            });
        }
        config.shortcuts.push(shortcut.clone());
        shortcut
    }

    pub fn assign_factory_model_to_shortcuts(&self, model_id: &str) {
        for shortcut in &mut self.config.borrow_mut().shortcuts {
            if shortcut.model_id == DEFAULT_MODEL_ID {
                shortcut.model_id = model_id.to_string();
            }
        }
    }

    pub fn remove_shortcut(&self, shortcut_id: &str) -> bool {
        if shortcut_id == DEFAULT_SHORTCUT_ID {
            return false;
        }
        let mut config = self.config.borrow_mut();
        let original_len = config.shortcuts.len();
        config
            .shortcuts
            .retain(|shortcut| shortcut.id != shortcut_id);
        config.shortcuts.len() != original_len
    }
}

fn signal_keys_conflict(shortcut: &ShortcutProfile, occupied_signals: &HashSet<String>) -> bool {
    shortcut_signal_keys(shortcut)
        .is_some_and(|keys| keys.iter().any(|key| occupied_signals.contains(key)))
}

fn insert_shortcut_signal_keys(shortcut: &ShortcutProfile, occupied_signals: &mut HashSet<String>) {
    if let Some(keys) = shortcut_signal_keys(shortcut) {
        occupied_signals.extend(keys);
    }
}

fn shortcut_signal_keys(shortcut: &ShortcutProfile) -> Option<Vec<String>> {
    let ShortcutTrigger::LinuxSignal {
        start_signal,
        stop_signal,
    } = &shortcut.trigger
    else {
        return None;
    };

    let start_signal = start_signal.duplicate_key().ok()?;
    let stop_signal = stop_signal.duplicate_key().ok()?;
    if start_signal == stop_signal {
        Some(vec![start_signal])
    } else {
        Some(vec![start_signal, stop_signal])
    }
}

#[cfg(test)]
mod tests {
    use shared::{DEFAULT_MODEL_ID, OutputAction};

    use super::*;

    #[test]
    fn updates_shortcut_by_id_after_removing_another_shortcut() {
        let draft = SettingsDraft::new(AppConfig::default());
        let first = draft.add_shortcut(
            ShortcutTriggerCapabilities::KeyboardAndSignals,
            DEFAULT_MODEL_ID.to_string(),
        );
        let second = draft.add_shortcut(
            ShortcutTriggerCapabilities::KeyboardAndSignals,
            DEFAULT_MODEL_ID.to_string(),
        );

        assert!(draft.remove_shortcut(&first.id));
        draft.update_shortcut(&second.id, |shortcut| {
            shortcut.output = OutputAction::default();
        });

        let config = draft.snapshot();
        assert_eq!(config.shortcuts.len(), 2);
        assert_eq!(config.shortcuts[0].id, DEFAULT_SHORTCUT_ID);
        assert_eq!(config.shortcuts[1].id, second.id);
        assert_eq!(config.shortcuts[1].output, OutputAction::default());
    }

    #[test]
    fn factory_model_assignment_preserves_unrelated_draft_edits() {
        let draft = SettingsDraft::new(AppConfig::default());
        let custom = draft.add_shortcut(
            ShortcutTriggerCapabilities::KeyboardAndSignals,
            "tiny".to_string(),
        );
        draft.update(|config| {
            config.general.keep_model_loaded = false;
        });
        draft.update_shortcut(&custom.id, |shortcut| {
            shortcut.name = "Edited".to_string();
        });

        draft.assign_factory_model_to_shortcuts("small-q8_0");

        let config = draft.snapshot();
        assert!(!config.general.keep_model_loaded);
        assert_eq!(config.default_shortcut().model_id, "small-q8_0");
        let custom_shortcut = config
            .shortcut_by_id(&custom.id)
            .expect("custom shortcut should remain");
        assert_eq!(custom_shortcut.model_id, "tiny");
        assert_eq!(custom_shortcut.name, "Edited");
    }

    #[test]
    fn default_shortcut_cannot_be_removed() {
        let draft = SettingsDraft::new(AppConfig::default());

        assert!(!draft.remove_shortcut(DEFAULT_SHORTCUT_ID));
        assert_eq!(draft.snapshot().shortcuts.len(), 1);
    }

    #[test]
    fn added_shortcut_uses_signals_when_keyboard_is_unavailable() {
        let draft = SettingsDraft::new(AppConfig::default());

        let shortcut = draft.add_shortcut(
            ShortcutTriggerCapabilities::SignalsOnly,
            DEFAULT_MODEL_ID.to_string(),
        );

        assert_eq!(shortcut.trigger, ShortcutTrigger::default_linux_signal());
    }

    #[test]
    fn added_signal_shortcut_starts_disabled_when_default_pair_is_taken() {
        let draft = SettingsDraft::new(AppConfig::default());
        draft.coerce_trigger_capabilities(ShortcutTriggerCapabilities::SignalsOnly);

        let shortcut = draft.add_shortcut(
            ShortcutTriggerCapabilities::SignalsOnly,
            DEFAULT_MODEL_ID.to_string(),
        );

        assert_eq!(shortcut.trigger, ShortcutTrigger::default_linux_signal());
        assert!(!shortcut.enabled);
        assert!(draft.snapshot().normalized().is_ok());
    }

    #[test]
    fn coerces_keyboard_shortcuts_to_signals_when_keyboard_is_unavailable() {
        let draft = SettingsDraft::new(AppConfig::default());

        draft.coerce_trigger_capabilities(ShortcutTriggerCapabilities::SignalsOnly);

        assert_eq!(
            draft.snapshot().shortcuts[0].trigger,
            ShortcutTrigger::default_linux_signal()
        );
    }

    #[test]
    fn disables_duplicate_signal_profiles_after_keyboard_coercion() {
        let draft = SettingsDraft::new(AppConfig::default());
        let added = draft.add_shortcut(
            ShortcutTriggerCapabilities::KeyboardAndSignals,
            DEFAULT_MODEL_ID.to_string(),
        );

        draft.coerce_trigger_capabilities(ShortcutTriggerCapabilities::SignalsOnly);

        let config = draft.snapshot();
        assert_eq!(config.shortcuts.len(), 2);
        assert!(config.shortcuts[0].enabled);
        assert_eq!(
            config.shortcuts[0].trigger,
            ShortcutTrigger::default_linux_signal()
        );
        let added_shortcut = config
            .shortcuts
            .iter()
            .find(|shortcut| shortcut.id == added.id)
            .expect("added shortcut should remain");
        assert!(!added_shortcut.enabled);
        assert_eq!(
            added_shortcut.trigger,
            ShortcutTrigger::default_linux_signal()
        );
        assert!(config.normalized().is_ok());
    }

    #[test]
    fn keeps_keyboard_shortcuts_when_keyboard_is_available() {
        let draft = SettingsDraft::new(AppConfig::default());

        draft.coerce_trigger_capabilities(ShortcutTriggerCapabilities::KeyboardAndSignals);

        assert_eq!(
            draft.snapshot().shortcuts[0].trigger,
            ShortcutTrigger::default_keyboard()
        );
    }
}
