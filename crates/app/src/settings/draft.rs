use std::cell::RefCell;
use std::rc::Rc;

use shared::{
    AppConfig, ConfigError, DEFAULT_SHORTCUT_ID, ShortcutProfile, ShortcutTrigger, next_shortcut_id,
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

        for shortcut in &mut self.config.borrow_mut().shortcuts {
            if matches!(shortcut.trigger, ShortcutTrigger::Keyboard { .. }) {
                shortcut.trigger = ShortcutTrigger::default_linux_signal();
                shortcut.enabled = true;
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

    pub fn add_shortcut(&self, capabilities: ShortcutTriggerCapabilities) -> ShortcutProfile {
        let mut config = self.config.borrow_mut();
        let id = next_shortcut_id(&config.shortcuts);
        let name = format!("Shortcut {}", config.shortcuts.len() + 1);
        let mut shortcut = ShortcutProfile::new_profile(id, name);
        if !capabilities.keyboard_available() {
            shortcut.trigger = ShortcutTrigger::default_linux_signal();
        }
        config.shortcuts.push(shortcut.clone());
        shortcut
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

#[cfg(test)]
mod tests {
    use shared::{OutputAction, ShortcutOutput};

    use super::*;

    #[test]
    fn updates_shortcut_by_id_after_removing_another_shortcut() {
        let draft = SettingsDraft::new(AppConfig::default());
        let first = draft.add_shortcut(ShortcutTriggerCapabilities::KeyboardAndSignals);
        let second = draft.add_shortcut(ShortcutTriggerCapabilities::KeyboardAndSignals);

        assert!(draft.remove_shortcut(&first.id));
        draft.update_shortcut(&second.id, |shortcut| {
            shortcut.output = ShortcutOutput::custom(OutputAction::default());
        });

        let config = draft.snapshot();
        assert_eq!(config.shortcuts.len(), 2);
        assert_eq!(config.shortcuts[0].id, DEFAULT_SHORTCUT_ID);
        assert_eq!(config.shortcuts[1].id, second.id);
        assert_eq!(
            config.shortcuts[1].output,
            ShortcutOutput::custom(OutputAction::default())
        );
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

        let shortcut = draft.add_shortcut(ShortcutTriggerCapabilities::SignalsOnly);

        assert_eq!(shortcut.trigger, ShortcutTrigger::default_linux_signal());
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
    fn keeps_keyboard_shortcuts_when_keyboard_is_available() {
        let draft = SettingsDraft::new(AppConfig::default());

        draft.coerce_trigger_capabilities(ShortcutTriggerCapabilities::KeyboardAndSignals);

        assert_eq!(
            draft.snapshot().shortcuts[0].trigger,
            ShortcutTrigger::default_keyboard()
        );
    }
}
