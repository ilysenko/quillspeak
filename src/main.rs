mod activity;
mod app;
mod audio;
mod config;
mod hotkey;
mod settings;
mod tray;
mod whisper;

fn main() -> anyhow::Result<()> {
    app::run()
}
