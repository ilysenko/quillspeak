use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use evdev::{AttributeSet, KeyCode, KeyEvent, uinput::VirtualDevice};
use tracing::{debug, info, warn};

const PASTE_TIMEOUT: Duration = Duration::from_secs(2);
const UINPUT_DEVICE_READY_DELAY: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub struct PasteServiceHandle {
    command_tx: mpsc::Sender<PasteCommand>,
    join_handle: Option<JoinHandle<()>>,
}

impl PasteServiceHandle {
    pub fn spawn() -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let join_handle = thread::Builder::new()
            .name("myapp-daemon-paste".to_string())
            .spawn(move || paste_service_loop(command_rx))
            .expect("paste worker thread should spawn");

        Self {
            command_tx,
            join_handle: Some(join_handle),
        }
    }

    pub fn paste_clipboard(&self) -> Result<()> {
        let (result_tx, result_rx) = mpsc::channel();
        let deadline = Instant::now() + PASTE_TIMEOUT;
        self.command_tx
            .send(PasteCommand::Paste {
                deadline,
                result_tx,
            })
            .context("paste worker is not running")?;

        match result_rx.recv_timeout(PASTE_TIMEOUT) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                bail!("paste worker did not respond within {PASTE_TIMEOUT:?}")
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => bail!("paste worker stopped"),
        }
    }
}

impl Drop for PasteServiceHandle {
    fn drop(&mut self) {
        let _ = self.command_tx.send(PasteCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take()
            && let Err(error) = join_handle.join()
        {
            warn!(?error, "paste worker thread panicked during shutdown");
        }
    }
}

#[derive(Debug)]
enum PasteCommand {
    Paste {
        deadline: Instant,
        result_tx: mpsc::Sender<Result<()>>,
    },
    Shutdown,
}

fn paste_service_loop(command_rx: mpsc::Receiver<PasteCommand>) {
    let mut runtime = PasteRuntime::default();

    info!("paste worker thread started");
    for command in command_rx {
        match command {
            PasteCommand::Paste {
                deadline,
                result_tx,
            } => {
                let result = runtime.paste_clipboard(deadline);
                let _ = result_tx.send(result);
            }
            PasteCommand::Shutdown => break,
        }
    }
    info!("paste worker thread stopped");
}

#[derive(Default)]
struct PasteRuntime {
    device: Option<VirtualDevice>,
}

impl PasteRuntime {
    fn paste_clipboard(&mut self, deadline: Instant) -> Result<()> {
        ensure_paste_request_not_expired(deadline, "before preparing virtual keyboard")?;
        let result = {
            let device = self.device()?;
            ensure_paste_request_not_expired(deadline, "before emitting paste shortcut")?;
            emit_paste_sequence(device)
        };
        if result.is_err() {
            self.device = None;
        }
        result
    }

    fn device(&mut self) -> Result<&mut VirtualDevice> {
        if self.device.is_none() {
            self.device = Some(create_virtual_keyboard()?);
        }
        Ok(self
            .device
            .as_mut()
            .expect("virtual keyboard should exist after creation"))
    }
}

fn ensure_paste_request_not_expired(deadline: Instant, context: &'static str) -> Result<()> {
    if Instant::now() >= deadline {
        bail!("paste request expired {context}");
    }
    Ok(())
}

fn create_virtual_keyboard() -> Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    keys.insert(KeyCode::KEY_LEFTCTRL);
    keys.insert(KeyCode::KEY_V);

    let device = VirtualDevice::builder()
        .context("failed to create uinput virtual device builder")?
        .name("MyApp Clipboard Paste")
        .with_keys(&keys)
        .context("failed to configure uinput virtual keyboard keys")?
        .build()
        .context("failed to build uinput virtual keyboard; check /dev/uinput permissions")?;

    thread::sleep(UINPUT_DEVICE_READY_DELAY);
    info!("created uinput virtual keyboard for clipboard paste");
    Ok(device)
}

fn emit_paste_sequence(device: &mut VirtualDevice) -> Result<()> {
    for (key, value) in paste_key_sequence() {
        let event = *KeyEvent::new(key, value);
        device
            .emit(&[event])
            .context("failed to emit Ctrl+V paste key event")?;
    }
    debug!("emitted clipboard paste shortcut");
    Ok(())
}

fn paste_key_sequence() -> Vec<(KeyCode, i32)> {
    vec![
        (KeyCode::KEY_LEFTCTRL, 1),
        (KeyCode::KEY_V, 1),
        (KeyCode::KEY_V, 0),
        (KeyCode::KEY_LEFTCTRL, 0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_v_sequence_presses_and_releases_modifier_and_v() {
        assert_eq!(
            paste_key_sequence(),
            vec![
                (KeyCode::KEY_LEFTCTRL, 1),
                (KeyCode::KEY_V, 1),
                (KeyCode::KEY_V, 0),
                (KeyCode::KEY_LEFTCTRL, 0),
            ]
        );
    }

    #[test]
    fn paste_deadline_accepts_future_instant() {
        let deadline = Instant::now() + Duration::from_secs(1);

        assert!(ensure_paste_request_not_expired(deadline, "during test").is_ok());
    }

    #[test]
    fn paste_deadline_rejects_expired_instant() {
        let deadline = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("test instant should support small subtraction");

        let error = ensure_paste_request_not_expired(deadline, "during test")
            .expect_err("expired paste deadline should be rejected");

        assert!(error.to_string().contains("paste request expired"));
    }
}
