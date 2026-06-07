# MyApp Linux Voice Prototype

Minimal Rust Linux desktop prototype for a future push-to-talk transcription app.

The main app is a normal foreground process while developing. It starts with no
visible window, keeps running through a tray/top-bar StatusNotifierItem, and can
be interrupted with Ctrl-C from the terminal. Later, the same runtime can be
wrapped in desktop autostart or a user service without changing the tray, UI, or
D-Bus architecture.

## Workspace

- `crates/app` builds `myapp`, the GTK4/libadwaita main app.
- `crates/daemon` builds `myapp-daemon`, the optional input daemon stub.
- `crates/shared` contains config and protocol types shared by both binaries.

## System Dependencies

On Debian/Ubuntu-style systems install the GTK development packages before
building the main app:

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev
```

The daemon and shared crate do not require GTK.

## Build And Run

Run the main app in foreground development mode:

```sh
cargo run -p app --bin myapp
```

Expected behavior:

- no settings window appears at startup,
- the tray/top-bar indicator appears,
- the app keeps running after the settings window is closed,
- Ctrl-C exits the app from the terminal,
- tray `Quit` exits the app,
- tray `Start Recording` and `Stop Recording` log placeholder messages.

Run the optional daemon stub:

```sh
cargo run -p daemon --bin myapp-daemon
```

Verbose development logs:

```sh
RUST_LOG=debug MYAPP_DEV_LOG=1 cargo run -p daemon --bin myapp-daemon
```

Simulate daemon hotkey events while `myapp` is running:

```sh
cargo run -p daemon --bin myapp-daemon -- --hotkey-down
cargo run -p daemon --bin myapp-daemon -- --hotkey-up
```

The first command should make the app log `Start recording`; the second should
make it log `Stop recording`.

## Configuration

The main app owns user settings and writes TOML to:

```text
~/.config/myapp/config.toml
```

Default config:

```toml
schema_version = 1
mode = "push_to_talk"
hotkey_backend = "disabled"

[shortcuts.push_to_talk]
accelerator = "Ctrl+Space"
enabled = true
```

The daemon does not read this config directly. The app sends the current
shortcut runtime config to the daemon over D-Bus. The daemon stores a
last-known cache at `~/.config/myapp-input-daemon/shortcut-cache.toml` so it can
start before the app and still know the last configured shortcuts.

The settings window has a shortcut text field plus a `Record` button. The
recorder captures a focused key combination in the settings dialog only; it does
not implement global hotkey capture yet. Click `Save` to persist the recorded
shortcut and send it to the daemon if the daemon is running.

## D-Bus Prototype

Session bus names and object paths are defined in `shared`:

- app bus: `org.example.MyApp.App`
- app object: `/org/example/MyApp/App`
- daemon bus: `org.example.MyApp.InputDaemon`
- daemon object: `/org/example/MyApp/InputDaemon`

The app exposes `HotkeyDown`, `HotkeyUp`, `DaemonStatus`, and
`GetShortcutConfig` methods for the daemon prototype. The daemon exposes
`Ping`, `GetDaemonStatus`, and `UpdateShortcutConfig`.

Daemon status is synchronized in both directions:

- the daemon requests config from the app and reports `DaemonStatus` when it
  starts,
- the app watches the daemon D-Bus name and refreshes status when the daemon
  appears or vanishes,
- when the daemon appears, the app pushes the current shortcut config again.

## Optional Daemon Service

A future installable user service is included at:

```text
packaging/systemd/user/myapp-input-daemon.service
```

It is for manual testing and future packaging only. The main app does not depend
on this service, does not require sudo, and must work when the daemon is missing
or stopped.

## Future Work

The prototype intentionally leaves these unimplemented:

- real X11 in-process hotkey capture,
- XDG GlobalShortcuts portal support,
- Wayland advanced daemon backend using evdev/libinput key down/up events,
- microphone recording,
- Whisper integration,
- text insertion,
- Flatpak packaging for the main app,
- `.deb` packaging for the optional daemon.
