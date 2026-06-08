mod app;
mod audio;
mod command;
mod config_store;
mod daemon_client;
mod daemon_monitor;
mod dbus;
mod hotkey;
mod models;
mod output;
mod recording;
mod settings;
mod signal_trigger;
mod transcription;
mod tray;

use std::env;

const APP_LOG_FILTER: &str = "info,pulseaudio::client::reactor=off";
const APP_DEV_LOG_FILTER: &str = "myapp=debug,shared=debug,info,pulseaudio::client::reactor=off";

fn main() -> gtk4::glib::ExitCode {
    init_logging();
    app::run()
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
        tracing_subscriber::EnvFilter::new(APP_DEV_LOG_FILTER)
    } else {
        tracing_subscriber::EnvFilter::new(APP_LOG_FILTER)
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
