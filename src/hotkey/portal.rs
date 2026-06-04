use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result, anyhow, bail};
use ashpd::desktop::CreateSessionOptions;
use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use futures_channel::oneshot;
use futures_util::{FutureExt, StreamExt, pin_mut, select};

use crate::activity::PushToTalkEvent;

use super::{
    HotkeyEdgeFilter, HotkeySpec, SharedHotkeyHandler, dispatch_filter_reset_release,
    dispatch_filtered_push_to_talk, hotkey_debug_enabled,
};

const PUSH_TO_TALK_SHORTCUT_ID: &str = "push-to-talk";

pub(super) struct PortalHotkeyBackend {
    handler: SharedHotkeyHandler,
    state: Mutex<PortalState>,
}

#[derive(Default)]
struct PortalState {
    configured_hotkey: Option<String>,
    registration: Option<PortalRegistration>,
}

impl PortalHotkeyBackend {
    pub(super) fn new(handler: SharedHotkeyHandler) -> Self {
        Self {
            handler,
            state: Mutex::new(PortalState::default()),
        }
    }

    pub(super) fn configure(&self, spec: &HotkeySpec) -> Result<()> {
        let mut state = self.state.lock().expect("portal hotkey state was poisoned");
        if state
            .configured_hotkey
            .as_deref()
            .is_some_and(|hotkey| hotkey == spec.canonical())
            && state
                .registration
                .as_ref()
                .is_some_and(|registration| !registration.is_finished())
        {
            return Ok(());
        }

        let next_registration = PortalRegistration::start(spec.clone(), Arc::clone(&self.handler))?;
        let old_registration = state.registration.replace(next_registration);
        state.configured_hotkey = Some(spec.canonical().to_string());
        drop(state);
        drop(old_registration);

        Ok(())
    }

    pub(super) fn deactivate(&self) -> Result<()> {
        let mut state = self.state.lock().expect("portal hotkey state was poisoned");
        let old_registration = state.registration.take();
        state.configured_hotkey = None;
        drop(state);
        drop(old_registration);
        Ok(())
    }
}

struct PortalRegistration {
    shutdown_sender: Option<oneshot::Sender<()>>,
    join_handle: Option<JoinHandle<()>>,
    handler: SharedHotkeyHandler,
    edge_filter: Arc<HotkeyEdgeFilter>,
}

impl PortalRegistration {
    fn start(spec: HotkeySpec, handler: SharedHotkeyHandler) -> Result<Self> {
        let canonical_hotkey = spec.canonical().to_string();
        let (ready_sender, ready_receiver) =
            mpsc::sync_channel::<std::result::Result<(), String>>(1);
        let (shutdown_sender, shutdown_receiver) = oneshot::channel();
        let thread_hotkey = canonical_hotkey.clone();
        let edge_filter = Arc::new(HotkeyEdgeFilter::default());
        let thread_handler = Arc::clone(&handler);
        let thread_edge_filter = Arc::clone(&edge_filter);

        let join_handle = thread::Builder::new()
            .name("voice-xdg-portal-hotkey".to_string())
            .spawn(move || {
                run_portal_thread(
                    spec,
                    thread_handler,
                    thread_edge_filter,
                    shutdown_receiver,
                    ready_sender,
                );
            })
            .context("failed to spawn XDG Portal hotkey worker")?;

        match ready_receiver
            .recv()
            .context("XDG Portal hotkey worker stopped before reporting registration status")?
        {
            Ok(()) => Ok(Self {
                shutdown_sender: Some(shutdown_sender),
                join_handle: Some(join_handle),
                handler,
                edge_filter,
            }),
            Err(message) => {
                let _ = join_handle.join();
                Err(anyhow!(
                    "failed to bind XDG Portal shortcut `{thread_hotkey}`: {message}"
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

impl Drop for PortalRegistration {
    fn drop(&mut self) {
        if let Some(shutdown_sender) = self.shutdown_sender.take() {
            let _ = shutdown_sender.send(());
        }

        if let Some(join_handle) = self.join_handle.take()
            && join_handle.thread().id() != thread::current().id()
        {
            let _ = join_handle.join();
        }

        dispatch_filter_reset_release(&self.handler, &self.edge_filter);
    }
}

fn run_portal_thread(
    spec: HotkeySpec,
    handler: SharedHotkeyHandler,
    edge_filter: Arc<HotkeyEdgeFilter>,
    shutdown_receiver: oneshot::Receiver<()>,
    ready_sender: SyncSender<std::result::Result<(), String>>,
) {
    let result = async_io::block_on(run_portal_worker(
        spec,
        Arc::clone(&handler),
        Arc::clone(&edge_filter),
        shutdown_receiver,
        ready_sender.clone(),
    ));

    if let Err(error) = result {
        let message = format!("{error:#}");
        if ready_sender.send(Err(message.clone())).is_err() {
            eprintln!("XDG Portal hotkey worker stopped: {message}");
        }
    }

    dispatch_filter_reset_release(&handler, &edge_filter);
}

async fn run_portal_worker(
    spec: HotkeySpec,
    handler: SharedHotkeyHandler,
    edge_filter: Arc<HotkeyEdgeFilter>,
    shutdown_receiver: oneshot::Receiver<()>,
    ready_sender: SyncSender<std::result::Result<(), String>>,
) -> Result<()> {
    let portal = GlobalShortcuts::new()
        .await
        .context("failed to connect to org.freedesktop.portal.GlobalShortcuts")?;
    if portal.version() == 0 {
        bail!("GlobalShortcuts portal is not available");
    }

    let session = portal
        .create_session(CreateSessionOptions::default())
        .await
        .context("failed to create GlobalShortcuts portal session")?;
    let activated_stream = portal
        .receive_activated()
        .await
        .context("failed to subscribe to GlobalShortcuts Activated signal")?;
    let deactivated_stream = portal
        .receive_deactivated()
        .await
        .context("failed to subscribe to GlobalShortcuts Deactivated signal")?;
    pin_mut!(activated_stream);
    pin_mut!(deactivated_stream);

    let shortcut = NewShortcut::new(PUSH_TO_TALK_SHORTCUT_ID, "Voice push-to-talk")
        .preferred_trigger(Some(spec.xdg_trigger()));
    let request = portal
        .bind_shortcuts(&session, &[shortcut], None, BindShortcutsOptions::default())
        .await
        .with_context(|| format!("portal rejected shortcut `{}`", spec.canonical()))?;
    let response = request
        .response()
        .context("failed to read GlobalShortcuts portal bind response")?;
    if !response
        .shortcuts()
        .iter()
        .any(|shortcut| shortcut.id() == PUSH_TO_TALK_SHORTCUT_ID)
    {
        bail!("GlobalShortcuts portal did not bind `{}`", spec.canonical());
    }

    let _ = ready_sender.send(Ok(()));
    eprintln!(
        "Registered XDG Portal push-to-talk hotkey `{}`.",
        spec.canonical()
    );

    let shutdown_receiver = shutdown_receiver.fuse();
    pin_mut!(shutdown_receiver);

    loop {
        select! {
            _ = &mut shutdown_receiver => {
                let _ = session.close().await;
                return Ok(());
            }
            event = activated_stream.next().fuse() => {
                let Some(event) = event else {
                    bail!("GlobalShortcuts Activated signal stream ended");
                };
                if event.shortcut_id() == PUSH_TO_TALK_SHORTCUT_ID {
                    if hotkey_debug_enabled() {
                        eprintln!("XDG Portal raw push-to-talk event: Pressed");
                    }
                    dispatch_filtered_push_to_talk(
                        &handler,
                        &edge_filter,
                        PushToTalkEvent::Pressed,
                    );
                }
            }
            event = deactivated_stream.next().fuse() => {
                let Some(event) = event else {
                    bail!("GlobalShortcuts Deactivated signal stream ended");
                };
                if event.shortcut_id() == PUSH_TO_TALK_SHORTCUT_ID {
                    if hotkey_debug_enabled() {
                        eprintln!("XDG Portal raw push-to-talk event: Released");
                    }
                    dispatch_filtered_push_to_talk(
                        &handler,
                        &edge_filter,
                        PushToTalkEvent::Released,
                    );
                }
            }
        }
    }
}
