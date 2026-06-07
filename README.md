# MyApp Linux Voice Prototype

Minimal Rust Linux desktop prototype for a future push-to-talk transcription app.

The main app is a normal foreground process while developing. It starts with no
visible window, keeps running through a tray/top-bar StatusNotifierItem, and can
be interrupted with Ctrl-C from the terminal. Later, the same runtime can be
wrapped in desktop autostart or a user service without changing the tray, UI, or
D-Bus architecture.

## Workspace

- `crates/app` builds `myapp`, the GTK4/libadwaita main app.
- `crates/daemon` builds `myapp-daemon`, the optional Wayland input daemon.
- `crates/shared` contains config and protocol types shared by both binaries.

## System Dependencies

On Debian/Ubuntu-style systems install the GTK development packages before
building the main app:

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev
```

The daemon and shared crate do not require GTK. Real Wayland hotkey capture uses
Linux evdev devices under `/dev/input/event*`; normal runtime should not use
`sudo`, but your user must have permission to open the keyboard event devices
through your distro's input group, logind ACLs, or a future udev/package rule.

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
- the tray recording item toggles between `Start Recording` and
  `Stop Recording`.

Run the optional daemon:

```sh
cargo run -p daemon --bin myapp-daemon
```

On X11, `hotkey_backend = "auto"` makes the main app capture the shortcut
in-process and the daemon is not needed. On Wayland, `auto` uses the daemon for
precise key down/up events.

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

The tray icon reflects the recording phase:

- white: idle,
- red: recording,
- orange: processing/transcription.

Real hotkey behavior:

- pressing the configured shortcut sends `HotkeyDown` and turns the tray icon
  red,
- releasing any required key sends `HotkeyUp`, turns the icon orange while the
  placeholder transcription runs, then returns it to white,
- unrelated keys are ignored by the daemon and are not logged.

## Configuration

The main app owns user settings and writes TOML to:

```text
~/.config/myapp/config.toml
```

Default config:

```toml
schema_version = 1
mode = "push_to_talk"
hotkey_backend = "auto"

[shortcuts.push_to_talk]
accelerator = "Ctrl+Space"
enabled = true
```

The daemon does not read this config directly. The app sends the current
shortcut runtime config to the daemon over D-Bus. That runtime config is
daemon-effective: shortcuts are enabled only when the resolved backend is
`daemon`; for `disabled` or `x11`, the app sends disabled bindings so the daemon
clears any active watcher. The daemon stores an accepted last-known cache at
`~/.config/myapp-input-daemon/shortcut-cache.toml` so it can start before the app
and still know the last configured shortcuts.

The settings window has a shortcut text field, a `Record` button, and a backend
selector. The recorder captures a focused key combination in the settings dialog
only; global capture is performed by the selected runtime backend. Click `Save`
to persist the shortcut and reconfigure the active backend and daemon.

Backend values:

- `auto`: Wayland uses the daemon, X11 uses the app's X11 backend,
- `disabled`: no global hotkey, tray manual actions still work,
- `x11`: force app-side X11 capture,
- `daemon`: force daemon capture.

## D-Bus Prototype

Session bus names and object paths are defined in `shared`:

- app bus: `org.example.MyApp.App`
- app object: `/org/example/MyApp/App`
- daemon bus: `org.example.MyApp.InputDaemon`
- daemon object: `/org/example/MyApp/InputDaemon`

The app exposes `HotkeyDown`, `HotkeyUp`, `DaemonStatus`, and
`GetShortcutConfig` methods for the daemon. The daemon exposes
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

During development you can restart the daemon after code changes with Ctrl-C in
the daemon terminal, then:

```sh
cargo run -p daemon --bin myapp-daemon
```

If Settings shows `Permission error`, the daemon is running but cannot open the
needed `/dev/input/event*` keyboard device. Do not run the main app with sudo.
For temporary local testing you can run the daemon with elevated permissions,
but the intended install path is a user-level permission rule for the daemon.
If Settings says the daemon is running but the shortcut is unavailable, the
daemon is alive but did not find a usable evdev keyboard device for the
configured shortcut.

## Future Work

The prototype intentionally leaves these unimplemented:

- XDG GlobalShortcuts portal support,
- libinput/logind integration and robust packaged device permissions,
- microphone recording,
- Whisper integration,
- text insertion,
- Flatpak packaging for the main app,
- `.deb` packaging for the optional daemon.
