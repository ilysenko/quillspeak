use std::env;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use clap::Parser;
use shared::{
    DAEMON_BUS_NAME, DAEMON_INTERFACE, DAEMON_OBJECT_PATH, DEFAULT_SHORTCUT_ID, DaemonStatus,
    PasteShortcut, ShortcutRuntimeConfig,
};
use tracing::{debug, info, warn};
use zbus::{blocking::connection, interface};

mod app_client;
mod cache;
mod evdev_backend;
mod hotkey;
mod paste;

use app_client::{AppClient, AppClientHandle, HotkeyMethod};
use cache::DaemonCacheStore;
use evdev_backend::EvdevHotkeyHandle;
use paste::PasteServiceHandle;

const DAEMON_DEV_LOG_FILTER: &str = "myapp_daemon=debug,shared=debug,info";

#[derive(Debug, Parser)]
#[command(name = "myapp-daemon")]
#[command(about = "Optional MyApp input daemon prototype")]
struct Cli {
    #[arg(long, conflicts_with = "hotkey_up")]
    hotkey_down: bool,

    #[arg(long, conflicts_with = "hotkey_down")]
    hotkey_up: bool,

    #[arg(long, default_value = DEFAULT_SHORTCUT_ID)]
    shortcut_id: String,
}

fn main() -> Result<()> {
    init_logging();

    let cli = Cli::parse();
    if cli.hotkey_down {
        return app_client::send_hotkey_method(HotkeyMethod::Down, &cli.shortcut_id);
    }
    if cli.hotkey_up {
        return app_client::send_hotkey_method(HotkeyMethod::Up, &cli.shortcut_id);
    }

    run_daemon()
}

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(log_filter())
        .try_init();
}

fn log_filter() -> tracing_subscriber::EnvFilter {
    if let Ok(filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        return filter;
    }

    if env_flag("MYAPP_DEV_LOG") {
        tracing_subscriber::EnvFilter::new(DAEMON_DEV_LOG_FILTER)
    } else {
        tracing_subscriber::EnvFilter::new("info")
    }
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !matches!(value.as_str(), "" | "0" | "false" | "off" | "no")
        })
        .unwrap_or(false)
}

fn run_daemon() -> Result<()> {
    let cache_store = DaemonCacheStore::new()?;
    let app_client_handle = AppClientHandle::spawn();
    let app_client = app_client_handle.client();
    let hotkey_backend = EvdevHotkeyHandle::spawn(app_client.clone());
    let paste_service = PasteServiceHandle::spawn();
    let state = Arc::new(Mutex::new(DaemonState {
        shortcut_config: None,
        cache_store: cache_store.clone(),
        hotkey_backend,
    }));
    let daemon = InputDaemon::new(Arc::clone(&state), app_client.clone(), paste_service);
    let _connection = connection::Builder::session()
        .context("failed to connect to the D-Bus session bus")?
        .name(DAEMON_BUS_NAME)
        .context("failed to request daemon D-Bus name")?
        .serve_at(DAEMON_OBJECT_PATH, daemon)
        .context("failed to register daemon D-Bus object")?
        .build()
        .context("failed to build daemon D-Bus connection")?;

    info!(
        bus_name = DAEMON_BUS_NAME,
        object_path = DAEMON_OBJECT_PATH,
        interface = DAEMON_INTERFACE,
        cache_path = %cache_store.path().display(),
        "myapp input daemon is running"
    );

    if let Err(error) = initialize_shortcut_config(&state, &cache_store) {
        warn!(?error, "failed to initialize daemon shortcut config");
    }
    app_client.notify_status(current_status(&state));

    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = shutdown_tx.send(());
    })
    .context("failed to install Ctrl-C handler")?;

    let _ = shutdown_rx.recv();
    info!("myapp input daemon is shutting down");
    drop(app_client_handle);
    Ok(())
}

fn request_shortcut_config_from_app() -> Result<ShortcutRuntimeConfig> {
    let config = app_client::request_shortcut_config()?;
    log_shortcut_runtime_config("fresh app config", &config);
    Ok(config)
}

fn initialize_shortcut_config(
    state: &Arc<Mutex<DaemonState>>,
    cache_store: &DaemonCacheStore,
) -> Result<()> {
    let shortcut_config = match request_shortcut_config_from_app() {
        Ok(config) => {
            info!("loaded fresh shortcut config from app");
            Some(config)
        }
        Err(error) => {
            warn!(
                ?error,
                "could not request shortcut config from app; trying daemon cache"
            );
            cache_store.load()?
        }
    };

    if let Some(config) = shortcut_config {
        log_shortcut_runtime_config("daemon initial config", &config);
        let mut state = state
            .lock()
            .expect("daemon state mutex should not be poisoned during startup");
        let status = state.hotkey_backend.update_config(&config)?;
        state.cache_store.save(&config)?;
        state.shortcut_config = Some(config);
        info!(
            daemon_status = %status.display_label(),
            "daemon applied initial shortcut config to evdev backend"
        );
    } else {
        info!("daemon started without shortcut config");
    }

    Ok(())
}

fn current_status(state: &Arc<Mutex<DaemonState>>) -> DaemonStatus {
    state
        .lock()
        .map(|state| state.status())
        .unwrap_or(DaemonStatus::PermissionError)
}

#[derive(Debug)]
struct InputDaemon {
    state: Arc<Mutex<DaemonState>>,
    app_client: AppClient,
    paste_service: PasteServiceHandle,
}

impl InputDaemon {
    fn new(
        state: Arc<Mutex<DaemonState>>,
        app_client: AppClient,
        paste_service: PasteServiceHandle,
    ) -> Self {
        Self {
            state,
            app_client,
            paste_service,
        }
    }
}

#[derive(Debug)]
struct DaemonState {
    shortcut_config: Option<ShortcutRuntimeConfig>,
    cache_store: DaemonCacheStore,
    hotkey_backend: EvdevHotkeyHandle,
}

#[interface(name = "org.example.MyApp.InputDaemon1")]
impl InputDaemon {
    fn ping(&self) -> bool {
        info!(method = "Ping", "received daemon D-Bus method");
        true
    }

    fn get_daemon_status(&self) -> String {
        info!(method = "GetDaemonStatus", "received daemon D-Bus method");
        let status = self.status();
        info!(
            daemon_status = %status.display_label(),
            method = "GetDaemonStatus",
            "returning daemon status over D-Bus"
        );
        status.as_wire_str().to_string()
    }

    fn update_shortcut_config(&self, config: ShortcutRuntimeConfig) -> bool {
        info!(
            shortcut_count = config.shortcuts.len(),
            configured = config.is_configured(),
            method = "UpdateShortcutConfig",
            "received daemon D-Bus method"
        );
        log_shortcut_runtime_config("daemon received config update", &config);

        let status = match self.update_config(config) {
            Ok(status) => status,
            Err(error) => {
                warn!(?error, "failed to update daemon shortcut config");
                return false;
            }
        };

        self.app_client.notify_status(status);
        true
    }

    fn paste_clipboard(&self, shortcut: String) -> bool {
        info!(
            paste_shortcut = %shortcut,
            method = "PasteClipboard",
            "received daemon D-Bus method"
        );
        let Some(shortcut) = parse_paste_shortcut(&shortcut) else {
            warn!(
                method = "PasteClipboard",
                "received unsupported paste shortcut"
            );
            return false;
        };

        match self.paste_service.paste_clipboard(shortcut) {
            Ok(()) => {
                info!(
                    paste_shortcut = shortcut.as_wire_str(),
                    method = "PasteClipboard",
                    "completed daemon clipboard paste"
                );
                true
            }
            Err(error) => {
                warn!(
                    ?error,
                    paste_shortcut = shortcut.as_wire_str(),
                    method = "PasteClipboard",
                    "failed daemon clipboard paste"
                );
                false
            }
        }
    }
}

fn parse_paste_shortcut(value: &str) -> Option<PasteShortcut> {
    PasteShortcut::from_wire_str(value.trim())
}

impl InputDaemon {
    fn status(&self) -> DaemonStatus {
        self.state
            .lock()
            .map(|state| state.status())
            .unwrap_or(DaemonStatus::PermissionError)
    }

    fn update_config(&self, config: ShortcutRuntimeConfig) -> Result<DaemonStatus> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| anyhow::anyhow!("daemon state mutex poisoned: {error}"))?;
        let status = state.hotkey_backend.update_config(&config)?;
        state.cache_store.save(&config)?;
        state.shortcut_config = Some(config);
        info!(
            daemon_status = %status.display_label(),
            "updated daemon shortcut runtime config"
        );
        Ok(status)
    }
}

impl DaemonState {
    fn status(&self) -> DaemonStatus {
        self.hotkey_backend
            .current_status_for_config(self.shortcut_config.as_ref())
    }
}

fn log_shortcut_runtime_config(context: &'static str, config: &ShortcutRuntimeConfig) {
    debug!(
        context,
        schema_version = config.schema_version,
        shortcut_count = config.shortcuts.len(),
        configured = config.is_configured(),
        "daemon shortcut runtime config"
    );

    for binding in &config.shortcuts {
        debug!(
            context,
            shortcut_id = %binding.id,
            shortcut_name = %binding.name,
            accelerator = %binding.accelerator,
            enabled = binding.enabled,
            "daemon shortcut binding"
        );

        if is_dev_logging_enabled() {
            info!(
                context,
                shortcut_id = %binding.id,
                shortcut_name = %binding.name,
                accelerator = %binding.accelerator,
                enabled = binding.enabled,
                "daemon dev shortcut binding"
            );
        }
    }
}

fn is_dev_logging_enabled() -> bool {
    env_flag("MYAPP_DEV_LOG")
}

#[cfg(test)]
mod tests {
    use shared::PasteShortcut;

    use super::parse_paste_shortcut;

    #[test]
    fn parse_paste_shortcut_accepts_supported_wire_values() {
        assert_eq!(parse_paste_shortcut("ctrl_v"), Some(PasteShortcut::CtrlV));
        assert_eq!(
            parse_paste_shortcut("ctrl_shift_v"),
            Some(PasteShortcut::CtrlShiftV)
        );
        assert_eq!(
            parse_paste_shortcut(" ctrl_shift_v "),
            Some(PasteShortcut::CtrlShiftV)
        );
    }

    #[test]
    fn parse_paste_shortcut_rejects_unsupported_wire_values() {
        assert_eq!(parse_paste_shortcut(""), None);
        assert_eq!(parse_paste_shortcut("ctrl-c"), None);
        assert_eq!(parse_paste_shortcut("Ctrl+V"), None);
    }
}
