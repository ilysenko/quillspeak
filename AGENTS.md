# MyApp AGENTS.md

This file is guidance for AI coding agents working in this repository.

## Project Goal

Build a minimal Rust Linux desktop prototype for a future voice transcription
tool. The app should behave like a background desktop utility from the user's
point of view, while remaining a normal foreground process during development.

The current goal is architecture and desktop behavior only:

- a Rust workspace with `app`, `daemon`, and `shared` crates,
- a GTK4/libadwaita main app,
- a tray/top-bar StatusNotifierItem indicator,
- a settings window opened only from the indicator menu,
- app-owned TOML configuration,
- zbus-based D-Bus between the main app and the optional daemon,
- placeholder recording/transcription functions that only log messages.

Do not implement real audio recording, Whisper, evdev/libinput input reading,
X11 hotkey capture, Wayland portal shortcuts, Flatpak packaging, or `.deb`
packaging unless explicitly requested.

## Current Architecture

The workspace has three crates:

- `crates/app`: builds `myapp`, the main GTK4/libadwaita desktop app.
- `crates/daemon`: builds `myapp-daemon`, the optional input daemon stub.
- `crates/shared`: shared config structs, D-Bus constants, protocol DTOs, and
  common enums used by both binaries.

The main app is intentionally not daemonized yet. It runs in the foreground so
developers can start it with `cargo run -p app --bin myapp`, read logs in the
terminal, and stop it with Ctrl-C. Internally, it still behaves like a
background desktop app: startup shows no window, the process stays alive through
the application lifecycle hold, and the settings window is opened from the tray.

The optional daemon is a separate process. In this prototype it does not read
real keyboard events. It only exposes basic D-Bus methods and provides CLI
simulation commands:

```sh
cargo run -p daemon --bin myapp-daemon -- --hotkey-down
cargo run -p daemon --bin myapp-daemon -- --hotkey-up
```

Those commands call the main app's D-Bus methods. `HotkeyDown` maps to
`start_recording()`, and `HotkeyUp` maps to `stop_recording()`.

## Important Files

- `Cargo.toml`: workspace definition and shared dependency versions.
- `crates/shared/src/config.rs`: `AppConfig`, hotkey mode/backend enums, and
  config validation.
- `crates/shared/src/protocol.rs`: D-Bus names, paths, interfaces, daemon
  status enum, and hotkey wire config.
- `crates/app/src/app.rs`: main foreground runtime, Ctrl-C handling, command
  pump, lifecycle hold, tray startup, settings startup, and quit path.
- `crates/app/src/tray.rs`: StatusNotifierItem tray implementation and menu.
- `crates/app/src/settings.rs`: GTK4/libadwaita settings window.
- `crates/app/src/dbus.rs`: app-side D-Bus service for daemon-to-app events.
- `crates/app/src/daemon_client.rs`: app-side daemon status and config sync
  client stub.
- `crates/app/src/hotkey/mod.rs`: pluggable hotkey backend boundary.
- `crates/app/src/recording.rs`: placeholder recording/transcription functions.
- `crates/daemon/src/main.rs`: optional daemon service stub and CLI simulation.
- `packaging/systemd/user/myapp-input-daemon.service`: future/manual user
  service for the optional daemon only.
- `README.md`: user-facing build and run instructions.

## Runtime Requirements

The app must always support no-daemon mode:

- daemon absence must never block app startup,
- tray actions must work without the daemon,
- settings must work without the daemon,
- manual `Start Recording` and `Stop Recording` must work without the daemon,
- daemon errors should be logged and reflected as status, not treated as fatal.

During development, the main app must remain a normal foreground process:

- no fork/detach,
- no systemd service for the main app yet,
- Ctrl-C should use the same clean quit path as tray `Quit`,
- logs should remain visible in the terminal.

Keep the architecture ready for future daemonization or autostart by routing
external events through the `AppCommand` command path instead of directly
touching GTK or application state from worker threads.

## D-Bus Contract

Shared constants currently use:

- app bus name: `org.example.MyApp.App`
- app object path: `/org/example/MyApp/App`
- app interface: `org.example.MyApp.App1`
- daemon bus name: `org.example.MyApp.InputDaemon`
- daemon object path: `/org/example/MyApp/InputDaemon`
- daemon interface: `org.example.MyApp.InputDaemon1`

App-side methods:

- `HotkeyDown()`
- `HotkeyUp()`
- `DaemonStatus(status: String)`

Daemon-side methods:

- `Ping() -> bool`
- `GetDaemonStatus() -> String`
- `UpdateHotkeyConfig(config) -> bool`

If you change this contract, update `shared`, app, daemon, README, and tests
together.

## Configuration

The main app owns user configuration. Do not make the daemon read the app's
config file directly.

Default config:

```toml
hotkey = "Ctrl+Space"
mode = "push_to_talk"
hotkey_backend = "disabled"
```

The app stores config under the normal user config directory, currently:

```text
~/.config/myapp/config.toml
```

Later, if packaged as Flatpak, this should naturally live under the Flatpak app
config directory. The daemon may get its own host-side cache later, but current
hotkey config should be sent from the app to the daemon over D-Bus.

## UI Rules

The main app uses GTK4 and libadwaita.

Required behavior:

- do not show a window at startup,
- show a tray/top-bar indicator,
- indicator menu must contain `Show Settings`, `Start Recording`,
  `Stop Recording`, and `Quit`,
- `Show Settings` opens the settings window,
- closing the settings window hides it instead of quitting the app,
- tray `Quit` terminates the app,
- Ctrl-C terminates the app in development mode.

Keep GTK/libadwaita object access on the GTK main thread. Tray callbacks, D-Bus
handlers, and Ctrl-C handlers should send `AppCommand`s.

## Hotkey Architecture

Keep hotkey handling pluggable. Intended future backends:

- `DisabledBackend`
- `X11Backend`
- `PortalBackend`
- `DaemonBackend`

Only `DisabledBackend`, a `DaemonBackend` client stub, and manual tray actions
belong in the current prototype. Do not add real X11, portal, or evdev hotkey
capture until explicitly requested.

Wayland advanced hotkey mode should be considered unavailable when the daemon
is missing. X11 can later support in-process hotkeys, but this prototype should
not implement real X11 capture.

## Commands

Useful checks:

```sh
cargo fmt --all --check
cargo check -p shared -p daemon
cargo test -p shared -p daemon
```

Full app checks require GTK system development packages:

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev
cargo check -p app
```

Run commands:

```sh
cargo run -p app --bin myapp
cargo run -p daemon --bin myapp-daemon
cargo run -p daemon --bin myapp-daemon -- --hotkey-down
cargo run -p daemon --bin myapp-daemon -- --hotkey-up
```

This container may not have `gtk4.pc`, `libadwaita-1.pc`, or
`graphene-gobject-1.0.pc` installed. If app builds fail at pkg-config detection,
report the missing system package issue instead of rewriting the app away from
GTK4/libadwaita.

## Coding Conventions

- Keep crates and modules small.
- Keep shared wire/config types in `shared`; do not duplicate protocol strings.
- Use `anyhow::Result` at binary/runtime boundaries.
- Keep placeholder functions honest: log what would happen, but do not fake
  completed integrations.
- Do not add Electron or Tauri.
- Do not require sudo for the main app.
- Do not make daemon availability a startup requirement.
- Avoid hardcoded user paths; use XDG/directories helpers.
- Add tests for pure logic in `shared` and non-GTK app modules when possible.
- Treat existing user changes as intentional. Do not revert unrelated work.

## Current Verification State

At the time this file was written:

- `cargo fmt --all --check` passes.
- `cargo check -p shared -p daemon` passes.
- `cargo test -p shared -p daemon` passes.
- `cargo check -p app` is blocked in the current container by missing GTK4 and
  related pkg-config system libraries, not by the no-daemon architecture.

## Future Work

Possible later additions, only when requested:

- real X11 in-process hotkey backend,
- XDG GlobalShortcuts portal backend,
- Wayland advanced daemon backend with precise key down/up detection,
- evdev/libinput input reading inside the optional daemon,
- microphone recording,
- Whisper integration,
- text insertion,
- Flatpak packaging for the main app,
- `.deb` packaging for the daemon.
