use std::sync::mpsc;

use anyhow::{Context, Result};
use clap::Parser;
use shared::{
    APP_BUS_NAME, APP_INTERFACE, APP_OBJECT_PATH, DAEMON_BUS_NAME, DAEMON_INTERFACE,
    DAEMON_OBJECT_PATH, DaemonStatus, HotkeyConfig,
};
use tracing::info;
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
    let daemon = InputDaemon::default();
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

    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = shutdown_tx.send(());
    })
    .context("failed to install Ctrl-C handler")?;

    let _ = shutdown_rx.recv();
    info!("myapp input daemon stub is shutting down");
    Ok(())
}

#[derive(Debug, Default)]
struct InputDaemon {
    hotkey_config: Option<HotkeyConfig>,
}

#[interface(name = "org.example.MyApp.InputDaemon1")]
impl InputDaemon {
    fn ping(&self) -> bool {
        true
    }

    fn get_daemon_status(&self) -> String {
        DaemonStatus::Running.as_wire_str().to_string()
    }

    fn update_hotkey_config(&mut self, config: HotkeyConfig) -> bool {
        self.hotkey_config = Some(config);
        true
    }
}
