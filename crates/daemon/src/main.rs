use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use clap::Parser;
use directories::BaseDirs;
use shared::{
    APP_BUS_NAME, APP_INTERFACE, APP_OBJECT_PATH, DAEMON_BUS_NAME, DAEMON_INTERFACE,
    DAEMON_OBJECT_PATH, DaemonStatus, ShortcutRuntimeConfig,
};
use tracing::{debug, info, warn};
use zbus::{blocking::Proxy, blocking::connection, interface};

#[derive(Debug, Parser)]
#[command(name = "myapp-daemon")]
#[command(about = "Optional MyApp input daemon prototype")]
struct Cli {
    #[arg(long, conflicts_with = "hotkey_up")]
    hotkey_down: bool,

    #[arg(long, conflicts_with = "hotkey_down")]
    hotkey_up: bool,
}

fn main() -> Result<()> {
    init_logging();

    let cli = Cli::parse();
    if cli.hotkey_down {
        return send_hotkey_method("HotkeyDown");
    }
    if cli.hotkey_up {
        return send_hotkey_method("HotkeyUp");
    }

    run_daemon()
}

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
}

fn send_hotkey_method(method_name: &'static str) -> Result<()> {
    info!(
        method = method_name,
        bus_name = APP_BUS_NAME,
        "calling app D-Bus method"
    );
    let connection = zbus::blocking::Connection::session()
        .context("failed to connect to the D-Bus session bus")?;
    let proxy = Proxy::new(&connection, APP_BUS_NAME, APP_OBJECT_PATH, APP_INTERFACE)
        .context("failed to create app D-Bus proxy; is myapp running?")?;

    proxy
        .call::<_, _, ()>(method_name, &())
        .with_context(|| format!("failed to call app method {method_name}; is myapp running?"))?;

    info!(method_name, "sent simulated hotkey event to app");
    Ok(())
}

fn run_daemon() -> Result<()> {
    let cache_store = DaemonCacheStore::new()?;
    let state = Arc::new(Mutex::new(DaemonState {
        shortcut_config: None,
        cache_store: cache_store.clone(),
    }));
    let daemon = InputDaemon::new(Arc::clone(&state));
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
        "myapp input daemon stub is running"
    );

    if let Err(error) = initialize_shortcut_config(&state, &cache_store) {
        warn!(?error, "failed to initialize daemon shortcut config");
    }
    notify_app_status(current_status(&state));

    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = shutdown_tx.send(());
    })
    .context("failed to install Ctrl-C handler")?;

    let _ = shutdown_rx.recv();
    info!("myapp input daemon stub is shutting down");
    Ok(())
}

fn request_shortcut_config_from_app() -> Result<ShortcutRuntimeConfig> {
    info!(
        method = "GetShortcutConfig",
        bus_name = APP_BUS_NAME,
        "calling app D-Bus method"
    );
    let connection = zbus::blocking::Connection::session()
        .context("failed to connect to the D-Bus session bus")?;
    let proxy = Proxy::new(&connection, APP_BUS_NAME, APP_OBJECT_PATH, APP_INTERFACE)
        .context("failed to create app D-Bus proxy")?;
    let config = proxy
        .call("GetShortcutConfig", &())
        .context("failed to call app GetShortcutConfig")?;
    log_shortcut_runtime_config("fresh app config", &config);
    Ok(config)
}

fn notify_app_status(status: DaemonStatus) {
    info!(
        daemon_status = %status.display_label(),
        method = "DaemonStatus",
        bus_name = APP_BUS_NAME,
        "calling app D-Bus method"
    );

    let result = (|| -> Result<()> {
        let connection = zbus::blocking::Connection::session()
            .context("failed to connect to the D-Bus session bus")?;
        let proxy = Proxy::new(&connection, APP_BUS_NAME, APP_OBJECT_PATH, APP_INTERFACE)
            .context("failed to create app D-Bus proxy")?;
        proxy
            .call::<_, _, ()>("DaemonStatus", &(status.as_wire_str().to_string(),))
            .context("failed to call app DaemonStatus")?;
        Ok(())
    })();

    if let Err(error) = result {
        debug!(?error, "could not notify app about daemon status");
    }
}

fn initialize_shortcut_config(
    state: &Arc<Mutex<DaemonState>>,
    cache_store: &DaemonCacheStore,
) -> Result<()> {
    let shortcut_config = match request_shortcut_config_from_app() {
        Ok(config) => {
            cache_store.save(&config)?;
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
        state.shortcut_config = Some(config);
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
}

impl InputDaemon {
    fn new(state: Arc<Mutex<DaemonState>>) -> Self {
        Self { state }
    }
}

#[derive(Debug)]
struct DaemonState {
    shortcut_config: Option<ShortcutRuntimeConfig>,
    cache_store: DaemonCacheStore,
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

        notify_app_status(status);
        true
    }
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
        state.cache_store.save(&config)?;
        state.shortcut_config = Some(config);
        let status = state.status();
        info!(
            daemon_status = %status.display_label(),
            "updated daemon shortcut runtime config"
        );
        Ok(status)
    }
}

impl DaemonState {
    fn status(&self) -> DaemonStatus {
        match &self.shortcut_config {
            Some(config) if config.is_configured() => DaemonStatus::RunningConfigured,
            _ => DaemonStatus::RunningUnconfigured,
        }
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
            action = %binding.action,
            accelerator = %binding.accelerator,
            enabled = binding.enabled,
            "daemon shortcut binding"
        );

        if is_dev_logging_enabled() {
            info!(
                context,
                action = %binding.action,
                accelerator = %binding.accelerator,
                enabled = binding.enabled,
                "daemon dev shortcut binding"
            );
        }
    }
}

fn is_dev_logging_enabled() -> bool {
    std::env::var_os("MYAPP_DEV_LOG").is_some()
}

#[derive(Debug, Clone)]
struct DaemonCacheStore {
    path: PathBuf,
}

impl DaemonCacheStore {
    fn new() -> Result<Self> {
        let base_dirs = BaseDirs::new().context("failed to resolve user config directory")?;
        Ok(Self {
            path: base_dirs
                .config_dir()
                .join("myapp-input-daemon/shortcut-cache.toml"),
        })
    }

    fn load(&self) -> Result<Option<ShortcutRuntimeConfig>> {
        if !self.path.exists() {
            return Ok(None);
        }

        let contents = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read daemon cache {}", self.path.display()))?;
        let config = toml::from_str(&contents)
            .with_context(|| format!("failed to parse daemon cache {}", self.path.display()))?;
        Ok(Some(config))
    }

    fn save(&self, config: &ShortcutRuntimeConfig) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create daemon cache directory {}",
                    parent.display()
                )
            })?;
        }
        let contents =
            toml::to_string_pretty(config).context("failed to encode daemon cache as TOML")?;
        fs::write(&self.path, contents)
            .with_context(|| format!("failed to write daemon cache {}", self.path.display()))?;
        Ok(())
    }

    #[allow(dead_code)]
    fn path(&self) -> &Path {
        &self.path
    }
}
