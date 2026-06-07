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

On Debian/Ubuntu-style systems install the desktop, audio, and whisper.cpp build
dependencies before building the main app:

```sh
sudo apt install build-essential pkg-config cmake clang libclang-dev libasound2-dev libgtk-4-dev libadwaita-1-dev
```

The default build uses CPAL's ALSA backend because it is widely available and
does not require extra PipeWire/PulseAudio development packages. Native
PipeWire or PulseAudio hosts can be enabled for local testing with Cargo
features:

```sh
cargo run -p app --bin myapp --features audio-pipewire
cargo run -p app --bin myapp --features audio-pulseaudio
```

Those features require the matching system development packages, for example
`libpipewire-0.3-dev` or `libpulse-dev`.

The daemon and shared crate do not require GTK. Real Wayland hotkey capture uses
Linux evdev devices under `/dev/input/event*`; normal runtime should not use
`sudo`, but your user must have permission to open the keyboard event devices
through your distro's input group, logind ACLs, or a future udev/package rule.

## Build And Run

Run the main app in foreground development mode:

```sh
RUST_LOG=info cargo run -p app --bin myapp
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

Verbose development logs for transcription details:

```sh
RUST_LOG=debug cargo run -p app --bin myapp
RUST_LOG=debug MYAPP_DEV_LOG=1 cargo run -p daemon --bin myapp-daemon
```

Simulate daemon hotkey events while `myapp` is running:

```sh
cargo run -p daemon --bin myapp-daemon -- --hotkey-down
cargo run -p daemon --bin myapp-daemon -- --hotkey-up
cargo run -p daemon --bin myapp-daemon -- --hotkey-down --shortcut-id default
```

The first command should make the app log `Start recording`; the second should
make it log `Stop recording` and start transcription if the selected model is
downloaded.

The tray icon reflects the recording phase:

- white: idle,
- red: recording,
- orange: processing/transcription.

Real hotkey behavior:

- pressing a configured shortcut sends `HotkeyDown(shortcut_id)` and turns the
  tray icon red,
- releasing any required key sends `HotkeyUp(shortcut_id)`, turns the icon
  orange while transcription runs, then returns it to white,
- unrelated keys are ignored by the daemon and are not logged.

## Audio And Transcription

The app records audio with CPAL from the General page `Default input` setting.
`System Default` is always available and resolves to the current Linux default
input device at recording time. The settings dropdown also lists discovered
input devices with stable CPAL device IDs when the host backend provides them.
Audio capture is independent of X11/Wayland.

On stop, the app converts captured audio to 16 kHz mono `f32`, runs
`whisper-rs`/whisper.cpp on the model selected by the active shortcut, and logs
recognized text:

```text
recognized text shortcut_id=default model_id=tiny language=auto text="..."
```

`RUST_LOG=debug` prints the full transcription debug structure: shortcut name,
model path, compute backend, input device, capture duration, source sample rate,
Whisper sample count, inference time, and segments.

Models must be downloaded in Settings > Models before they can be used. If a
shortcut points to a model that is not ready, recording stops with a clear log
error and no hidden download starts during the hotkey flow.

The first implementation only logs output actions. `Copy to clipboard` and
`Run script` are not executed yet.

## Configuration

The main app owns user settings and writes TOML to:

```text
~/.config/myapp/config.toml
```

Default config:

```toml
schema_version = 4

[general]
mode = "push_to_talk"
hotkey_backend = "auto"
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
keep_model_loaded = true
default_input = { type = "system_default" }
default_output = { type = "clipboard" }

[[shortcuts]]
id = "default"
name = "Default"
enabled = true
accelerator = "Ctrl+Alt+Space"
model_id = "default"
language = "default"
output = { type = "default" }
```

The daemon does not read this config directly. The app sends the current
shortcut runtime config to the daemon over D-Bus. That runtime config is
daemon-effective: shortcuts are enabled only when the resolved backend is
`daemon`; for `disabled` or `x11`, the app sends disabled bindings so the daemon
clears any active watcher. `keep_model_loaded = true` keeps only the last used
Whisper model context in memory to speed up repeated transcription; `false`
loads and drops the model for every transcription. The daemon stores an accepted last-known cache at
`~/.config/myapp-input-daemon/shortcut-cache.toml` so it can start before the app
and still know the last configured shortcuts. During development only schema v4
is supported; invalid old configs/caches are errors or ignored with a warning,
not migrated.

If a local development config is from an older schema, remove
`~/.config/myapp/config.toml` and restart the app to generate the current
default config.

Settings has `General`, `Models`, `Default`, and `Add New` pages. `Models`
manages downloaded whisper.cpp ggml models under `~/.local/share/myapp/models`.
Shortcut pages choose only ready models, or `Default` to inherit the general
model. Model downloads use `*.part`, progress updates, SHA-1 verification, and
atomic rename.

Each shortcut profile has its own shortcut, model override, language override,
and output action. Output actions are `Copy to clipboard` or `Run script`;
clipboard writing and script execution are still placeholders that log what
would happen with the recognized text.

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

The app exposes `HotkeyDown(shortcut_id)`, `HotkeyUp(shortcut_id)`,
`DaemonStatus`, and `GetShortcutConfig` methods for the daemon. The daemon exposes
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
- production-grade audio buffering/resampling,
- streaming/VAD transcription,
- text insertion,
- clipboard/script output execution,
- Flatpak packaging for the main app,
- `.deb` packaging for the optional daemon.
