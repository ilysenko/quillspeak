use std::sync::Mutex;

use anyhow::Result;

#[cfg(test)]
use crate::activity::PushToTalkEvent;

#[cfg(test)]
use super::dispatch_push_to_talk;
use super::{HotkeyBackend, PushToTalkHandler, SharedHotkeyHandler};

#[derive(Default)]
pub struct StubHotkeyBackend {
    configured_hotkey: Mutex<String>,
    handler: SharedHotkeyHandler,
}

impl StubHotkeyBackend {
    #[cfg(test)]
    pub fn configured_hotkey(&self) -> String {
        self.configured_hotkey
            .lock()
            .expect("stub hotkey state was poisoned")
            .clone()
    }

    #[cfg(test)]
    pub fn emit_event(&self, event: PushToTalkEvent) {
        dispatch_push_to_talk(&self.handler, event);
    }
}

impl HotkeyBackend for StubHotkeyBackend {
    fn configure_push_to_talk(&self, hotkey: &str) -> Result<()> {
        self.configured_hotkey
            .lock()
            .expect("stub hotkey state was poisoned")
            .replace_range(.., hotkey);
        Ok(())
    }

    fn set_push_to_talk_handler(&self, handler: PushToTalkHandler) -> Result<()> {
        self.handler
            .lock()
            .expect("stub hotkey handler was poisoned")
            .replace(handler);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn stub_tracks_configured_hotkey() {
        let backend = StubHotkeyBackend::default();

        backend.configure_push_to_talk("Ctrl+Space").unwrap();

        assert_eq!(backend.configured_hotkey(), "Ctrl+Space");
    }

    #[test]
    fn stub_dispatches_push_to_talk_events() {
        let backend = StubHotkeyBackend::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_handler = Arc::clone(&events);

        backend
            .set_push_to_talk_handler(Box::new(move |event| {
                events_for_handler
                    .lock()
                    .expect("test event log was poisoned")
                    .push(event);
            }))
            .unwrap();
        backend.emit_event(PushToTalkEvent::Pressed);
        backend.emit_event(PushToTalkEvent::Released);

        assert_eq!(
            events
                .lock()
                .expect("test event log was poisoned")
                .as_slice(),
            vec![PushToTalkEvent::Pressed, PushToTalkEvent::Released]
        );
    }
}
