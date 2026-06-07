use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

use shared::DAEMON_BUS_NAME;
use tracing::{debug, info, warn};
use zbus::blocking::{Connection, MessageIterator};
use zbus::fdo::NameOwnerChanged;
use zbus::names::UniqueName;
use zbus::{MatchRule, message::Type as MessageType};

use crate::command::AppCommand;
use crate::daemon_client::DaemonClient;

#[derive(Debug)]
pub struct DaemonMonitorHandle {
    shutdown: Arc<AtomicBool>,
}

impl DaemonMonitorHandle {
    pub fn spawn(command_tx: mpsc::Sender<AppCommand>, daemon_client: DaemonClient) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);

        thread::spawn(move || {
            if let Err(error) = run_daemon_monitor(command_tx, daemon_client, thread_shutdown) {
                warn!(?error, "daemon status monitor stopped");
            }
        });

        Self { shutdown }
    }
}

impl Drop for DaemonMonitorHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

fn run_daemon_monitor(
    command_tx: mpsc::Sender<AppCommand>,
    daemon_client: DaemonClient,
    shutdown: Arc<AtomicBool>,
) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .add_arg(DAEMON_BUS_NAME)?
        .build();
    let mut iter = MessageIterator::for_match_rule(rule, &connection, Some(4))?;

    info!(bus_name = DAEMON_BUS_NAME, "daemon status monitor started");

    for message in &mut iter {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let message = message?;
        let Some(signal) = NameOwnerChanged::from_message(message) else {
            continue;
        };
        let args = signal.args()?;
        let old_owner: Option<UniqueName<'_>> = args.old_owner().clone().into();
        let new_owner: Option<UniqueName<'_>> = args.new_owner().clone().into();
        let event = classify_name_owner_change(old_owner.is_some(), new_owner.is_some());

        debug!(
            bus_name = %args.name(),
            old_owner = ?old_owner,
            new_owner = ?new_owner,
            ?event,
            "received daemon NameOwnerChanged signal"
        );

        match event {
            Some(DaemonNameEvent::Appeared) => {
                let status = daemon_client.status();
                info!(
                    daemon_status = %status.display_label(),
                    "daemon appeared on D-Bus"
                );
                let _ = command_tx.send(AppCommand::DaemonAppeared(status));
            }
            Some(DaemonNameEvent::Vanished) => {
                let status = daemon_client.status();
                info!(
                    daemon_status = %status.display_label(),
                    "daemon vanished from D-Bus"
                );
                let _ = command_tx.send(AppCommand::DaemonVanished(status));
            }
            None => {}
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonNameEvent {
    Appeared,
    Vanished,
}

fn classify_name_owner_change(
    old_owner_present: bool,
    new_owner_present: bool,
) -> Option<DaemonNameEvent> {
    match (old_owner_present, new_owner_present) {
        (false, true) => Some(DaemonNameEvent::Appeared),
        (true, false) => Some(DaemonNameEvent::Vanished),
        (true, true) => Some(DaemonNameEvent::Appeared),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_daemon_name_owner_changes() {
        assert_eq!(
            classify_name_owner_change(false, true),
            Some(DaemonNameEvent::Appeared)
        );
        assert_eq!(
            classify_name_owner_change(true, false),
            Some(DaemonNameEvent::Vanished)
        );
        assert_eq!(
            classify_name_owner_change(true, true),
            Some(DaemonNameEvent::Appeared)
        );
        assert_eq!(classify_name_owner_change(false, false), None);
    }
}
