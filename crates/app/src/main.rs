mod app;
mod audio;
mod command;
mod config_store;
mod external_trigger;
mod hotkey;
mod models;
mod output;
mod recording;
mod settings;
mod signal_trigger;
mod system_audio;
mod transcription;
mod tray;

use std::env;

const APP_LOG_FILTER: &str = "info,pulseaudio::client::reactor=off";
const APP_DEV_LOG_FILTER: &str = "myapp=debug,shared=debug,info,pulseaudio::client::reactor=off";

fn main() -> gtk4::glib::ExitCode {
    init_logging();
    match external_trigger::parse_invocation(env::args().skip(1)) {
        Ok(external_trigger::ExternalTriggerInvocation::RunApp) => app::run(),
        Ok(external_trigger::ExternalTriggerInvocation::Send(request)) => {
            match external_trigger::send_trigger_request(&request) {
                Ok(()) => gtk4::glib::ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("{error:#}");
                    gtk4::glib::ExitCode::FAILURE
                }
            }
        }
        Err(error) => {
            eprintln!("{error:#}");
            gtk4::glib::ExitCode::FAILURE
        }
    }
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
