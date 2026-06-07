mod app;
mod command;
mod config_store;
mod daemon_client;
mod daemon_monitor;
mod dbus;
mod hotkey;
mod recording;
mod settings;
mod tray;

fn main() -> gtk4::glib::ExitCode {
    init_logging();
    app::run()
}

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
}
