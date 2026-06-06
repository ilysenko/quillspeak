pub mod config;
pub mod protocol;

pub use config::{AppConfig, ConfigError, HotkeyBackend, HotkeyMode};
pub use protocol::{
    APP_BUS_NAME, APP_ID, APP_INTERFACE, APP_OBJECT_PATH, DAEMON_BUS_NAME, DAEMON_INTERFACE,
    DAEMON_OBJECT_PATH, DaemonStatus, HotkeyConfig,
};
