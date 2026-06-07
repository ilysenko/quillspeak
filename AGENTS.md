# MyApp AGENTS.md

This file is guidance for AI coding agents working in this repository.

## Project Goal

MyApp is a Rust Linux desktop prototype for a future push-to-talk voice
transcription utility. It should feel like a background desktop app to the
user, but during development the main app remains a normal foreground process
so logs stay visible and Ctrl-C works.

The current prototype includes:

- a Rust workspace with `app`, `daemon`, and `shared` crates,
- a GTK4/libadwaita main app,
- a StatusNotifierItem tray/top-bar indicator through `ksni`,
- a settings window opened from the tray,
- app-owned TOML configuration,
- multi-shortcut settings,
- app-side X11 hotkey capture,
- optional daemon-side evdev hotkey capture for Wayland,
- zbus-based D-Bus between the app and daemon,
- whisper.cpp model catalog and download management,
- placeholder recording/transcription/output functions that only log.

Do not add Electron or Tauri. Do not require the daemon for main app startup.
Do not require sudo for the main app. Do not implement real microphone
recording, Whisper inference, clipboard insertion, script execution, XDG portal
shortcuts, Flatpak packaging, or `.deb` packaging unless explicitly requested.

## Workspace

- `crates/app`: builds `myapp`, the GTK4/libadwaita desktop app.
- `crates/daemon`: builds `myapp-daemon`, the optional input daemon.
- `crates/shared`: shared config structs, model catalog, D-Bus constants,
  protocol DTOs, shortcut parsing, languages, and output action types.

The app is intentionally not daemonized yet. Start it with:

```sh
cargo run -p app --bin myapp
```

It starts with no visible window, owns a GTK application hold, shows the tray
indicator, and exits through the same quit path for tray `Quit` and Ctrl-C.

The daemon is a separate foreground process during development:

```sh
cargo run -p daemon --bin myapp-daemon
```

It can also simulate app hotkey events:

```sh
cargo run -p daemon --bin myapp-daemon -- --hotkey-down
cargo run -p daemon --bin myapp-daemon -- --hotkey-up
cargo run -p daemon --bin myapp-daemon -- --hotkey-down --shortcut-id default
```

## Current Module Map

- `Cargo.toml`: workspace members, Rust edition/version, shared dependency
  versions.
- `crates/shared/src/config/mod.rs`: `AppConfig`, `GeneralConfig`, config
  schema version, backend/mode/compute enums, validation, normalization, and
  config resolution helpers.
- `crates/shared/src/config/model.rs`: whisper.cpp model catalog, model IDs,
  filenames, URLs, size labels, and SHA-1 hashes.
- `crates/shared/src/config/shortcut.rs`: shortcut profiles, accelerator
  normalization, shortcut IDs, shortcut chord parsing, and shortcut key types.
- `crates/shared/src/config/language.rs`: supported language list including
  `auto`, default inheritance, and Ukrainian.
- `crates/shared/src/config/output.rs`: default and per-shortcut output action
  types.
- `crates/shared/src/protocol.rs`: app/daemon D-Bus names, paths, interfaces,
  daemon status values, and `ShortcutRuntimeConfig`.
- `crates/app/src/main.rs`: app module wiring and tracing setup.
- `crates/app/src/app.rs`: app runtime, command pump, lifecycle hold, Ctrl-C,
  tray/settings startup, config apply/save, daemon sync, model commands,
  recording state, and quit path.
- `crates/app/src/command.rs`: `AppCommand`, download IDs, and model download
  outcome messages. Worker threads should communicate with app state through
  these commands.
- `crates/app/src/config_store.rs`: app config load/save under the XDG config
  directory.
- `crates/app/src/dbus.rs`: app-side D-Bus service for daemon-to-app calls.
- `crates/app/src/daemon_client.rs`: app-side daemon method calls and daemon
  installed/running status probing.
- `crates/app/src/daemon_monitor.rs`: app-side D-Bus `NameOwnerChanged`
  watcher for daemon appeared/vanished events.
- `crates/app/src/hotkey/mod.rs`: pluggable app hotkey backend boundary and
  backend resolution.
- `crates/app/src/hotkey/x11.rs`: app-side X11 passive grab backend.
- `crates/app/src/models/store.rs`: model directory, in-memory ready model IDs,
  model row state composition, download start, and model deletion.
- `crates/app/src/models/inventory.rs`: model readiness inventory cache and
  model/partial path helpers.
- `crates/app/src/models/downloader.rs`: blocking HTTP model download worker,
  cancellation, progress events, SHA-1 verification, and atomic rename.
- `crates/app/src/models/download_manager.rs`: active download state,
  `DownloadId` tracking, stale event filtering, canceling, and transient model
  statuses.
- `crates/app/src/models/view_model.rs`: model row view state, status labels,
  progress formatting, and referenced-model detection.
- `crates/app/src/settings/mod.rs`: `SettingsWindow`, `SettingsState`, stack
  rendering, page controllers, and live UI updates.
- `crates/app/src/settings/draft.rs`: unsaved mutable settings draft and
  shortcut add/remove/update helpers.
- `crates/app/src/settings/pages/general.rs`: daemon/backend/default model/
  language/compute/default output page.
- `crates/app/src/settings/pages/models.rs`: model manager page and row
  controllers.
- `crates/app/src/settings/pages/shortcut.rs`: per-shortcut name, accelerator,
  model, language, and output settings.
- `crates/app/src/settings/pages/add_shortcut.rs`: Add New shortcut page.
- `crates/app/src/settings/pages/output_controls.rs`: shared output action
  controls.
- `crates/app/src/settings/shortcut_recorder.rs`: focused settings-only
  shortcut recorder. This is not global hotkey capture.
- `crates/app/src/settings/widgets.rs`: GTK/libadwaita UI helper widgets and
  dropdown mapping helpers.
- `crates/app/src/tray.rs`: StatusNotifierItem tray, menu, recording-state
  labels, and generated color icon.
- `crates/app/src/recording.rs`: recording state machine and placeholder
  `start_recording`, `stop_recording`, and `transcribe_audio` logging.
- `crates/daemon/src/main.rs`: daemon CLI, D-Bus service, startup flow, app
  config request, status notification, and Ctrl-C handling.
- `crates/daemon/src/cache.rs`: daemon last-known shortcut runtime config cache.
- `crates/daemon/src/hotkey.rs`: daemon hotkey state machine and runtime
  shortcut extraction.
- `crates/daemon/src/evdev_backend.rs`: daemon evdev device scanning, watched
  key filtering, shortcut key mapping, and app hotkey event dispatch.
- `packaging/systemd/user/myapp-input-daemon.service`: future/manual systemd
  user service for the optional daemon only.
- `README.md`: user-facing build, run, config, daemon, and troubleshooting
  instructions.

## Runtime Rules

The app must always support no-daemon mode:

- daemon absence must never block app startup,
- tray actions must work without the daemon,
- settings must work without the daemon,
- manual recording toggle must work without the daemon,
- daemon errors should be logged and reflected as status, not treated as fatal.

During development:

- do not fork, detach, or daemonize the main app,
- do not add a systemd service for the main app,
- keep logs visible in the terminal,
- route Ctrl-C through `AppCommand::Quit`,
- route D-Bus, tray, daemon monitor, hotkey backend, download worker, and
  transcription worker events through `AppCommand`.

Keep GTK/libadwaita object access on the GTK main thread. Worker threads should
send commands, not mutate GTK objects or app runtime fields directly.

## Tray And Recording State

The tray indicator is user-facing app state:

- white icon means idle,
- red icon means recording,
- orange icon means processing/transcription.

The tray menu contains `Show Settings`, one recording action, and `Quit`.
Recording action labels are stateful:

- idle: `Start Recording`,
- recording: `Stop Recording`,
- processing: disabled `Processing...`.

Manual tray recording uses `AppCommand::ToggleRecording`. Hotkey backends must
not use the toggle because push-to-talk requires explicit key down/up semantics.
Hotkey down starts recording for a shortcut id. Hotkey up stops that same
shortcut and starts placeholder processing.

`RecordingService` is the recording state owner. The current implementation
only logs. Do not fake real recording, transcription, clipboard, or script
execution.

## D-Bus Contract

Shared constants currently use:

- app bus name: `org.example.MyApp.App`
- app object path: `/org/example/MyApp/App`
- app interface: `org.example.MyApp.App1`
- daemon bus name: `org.example.MyApp.InputDaemon`
- daemon object path: `/org/example/MyApp/InputDaemon`
- daemon interface: `org.example.MyApp.InputDaemon1`

App-side methods:

- `HotkeyDown(shortcut_id: String)`
- `HotkeyUp(shortcut_id: String)`
- `DaemonStatus(status: String)`
- `GetShortcutConfig() -> ShortcutRuntimeConfig`

Daemon-side methods:

- `Ping() -> bool`
- `GetDaemonStatus() -> String`
- `UpdateShortcutConfig(config: ShortcutRuntimeConfig) -> bool`

If this contract changes, update `shared`, app, daemon, README, and tests
together.

Daemon status synchronization is intentionally redundant and idempotent:

- app starts after daemon: app loads config and calls daemon
  `UpdateShortcutConfig`;
- daemon starts after app: daemon calls app `GetShortcutConfig`;
- Settings Save: app saves config, reloads it, applies it, and calls daemon
  `UpdateShortcutConfig`;
- daemon start/config update: daemon reports app-side `DaemonStatus`;
- app also watches the daemon D-Bus name with `NameOwnerChanged`.

Keep app-first, daemon-first, daemon-restart, daemon-vanished, and no-daemon
scenarios working.

## Configuration

The app is the source of truth for user settings. The daemon must not read the
app config file directly.

Current app config path:

```text
~/.config/myapp/config.toml
```

Current schema:

```toml
schema_version = 2

[general]
mode = "push_to_talk"
hotkey_backend = "auto"
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
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

Only schema v2 is supported during development. Do not add old-config
migration paths unless explicitly requested. If the schema changes during
active development, update the current schema and tests directly instead of
layering legacy compatibility.

The daemon receives daemon-effective runtime config, not the full app config.
`ShortcutRuntimeConfig::for_daemon` enables bindings only when the resolved
backend is `daemon`; for `disabled` or `x11`, the app sends disabled bindings
so the daemon clears active watchers. The daemon may cache that runtime config
at:

```text
~/.config/myapp-input-daemon/shortcut-cache.toml
```

That cache is disposable. App config always wins.

## Settings UI

Settings uses GTK4 and libadwaita. It is hidden, not destroyed, on close. It
should not appear at startup.

Current pages:

- `General`: daemon status, backend, compute backend, default model, default
  language, and default output.
- `Models`: whisper.cpp model download/remove/status management.
- one page per shortcut profile, with `Default` permanent.
- `Add New`: creates a new shortcut profile.

`SettingsDraft` owns the unsaved mutable copy of config. Saving remains
explicit through the `Save` button. Do not write config on every UI change.

Shortcut pages should show only ready models plus `Default` inheritance. If a
selected model is missing, the UI may show a missing marker so the user can fix
the setting, but unavailable models must not be offered as normal choices.

When ready model IDs change, Settings must update all model-dependent controls
without requiring app restart. Progress-only changes should update existing
model row controllers. Inventory changes such as completed download or remove
may rerender the stack to refresh dropdown choices while preserving the visible
page when possible.

Avoid raw GTK containers as direct `adw::PreferencesGroup` rows when they
trigger GTK focus/listbox warnings. Prefer `adw::PreferencesRow`,
`adw::ActionRow`, or a small row controller that owns valid libadwaita rows.

## Model Catalog, Downloads, And Inventory

The model catalog lives in `shared/src/config/model.rs`. It contains model IDs,
labels, filenames, display size labels, URLs, and expected SHA-1 values. The
catalog `size_bytes` is useful for labels, estimates, and fallback progress
totals, but it must not be the only source of truth for local readiness because
remote file sizes can differ from catalog estimates.

Model files live under:

```text
~/.local/share/myapp/models
```

Download flow:

- app sends `AppCommand::DownloadModel(model_id)`;
- `ModelDownloadManager` allocates a `DownloadId` and stores active transient
  state;
- downloader worker writes `*.part`;
- worker sends throttled `ModelDownloadProgress` events;
- worker sends `ModelDownloadVerifying` while hashing;
- worker verifies SHA-1;
- worker atomically renames `*.part` to the final catalog filename;
- worker sends `ModelDownloadFinished`;
- app ignores stale progress/finish events by `DownloadId`;
- completed downloads mark the model ready and refresh model inventory;
- canceled/failed downloads clear or preserve the correct transient state.

Readiness rules:

- a model becomes ready only after successful SHA-1 verification and final file
  rename;
- after completion, the current process must update in-memory ready model IDs
  immediately;
- a later app start should reconcile inventory with the real final file and
  catalog identity/hash metadata;
- do not mark a model not ready merely because the real file length differs
  from catalog `size_bytes` after a verified download;
- deleting a ready model must update the file, inventory, in-memory ready IDs,
  model rows, and model dropdown choices.

UI rules:

- while downloading, show stable per-row progress and a `Cancel` action;
- while canceling, show a disabled canceling state;
- while verifying, keep progress visible and indicate verification;
- when ready, show `Remove Model`;
- `Remove Model` should ask for confirmation before deleting;
- referenced models should not be silently deleted.

## Hotkey Architecture

Hotkey handling is pluggable. Current app-side backend types:

- `DisabledBackend`
- `DaemonBackend`
- `X11Backend`

Configured backend values:

- `auto`: resolves to daemon when `WAYLAND_DISPLAY` is present, X11 when only
  `DISPLAY` is present, otherwise disabled;
- `disabled`: no global hotkey, tray manual actions still work;
- `x11`: force app-side X11 passive grabs;
- `daemon`: force daemon-side evdev capture;
- `portal`: accepted in config enum as a future placeholder but currently
  falls back to disabled behavior.

X11 capture lives in the app and uses passive X11 grabs. The X11 backend sends
`StartRecording(shortcut_id)` on key down and `StopRecording(shortcut_id)` when
the required chord is no longer pressed.

Wayland advanced capture lives in the optional daemon and uses evdev. The
daemon maps configured shortcut chords to watched evdev key codes, opens usable
`/dev/input/event*` devices, filters unrelated keys immediately, and sends
`HotkeyDown(shortcut_id)` / `HotkeyUp(shortcut_id)` to the app over D-Bus.
`EVIOCGRAB` must not be used.

The main app should never be run with sudo. The daemon should normally run as
the user too, but it needs permission to open keyboard event devices. If it
cannot, report `Permission error`. Future packaging should solve this with
user-level permissions, udev/logind integration, or service setup.

## Commands

Useful checks:

```sh
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

System dependencies for the full app build on Debian/Ubuntu-style systems:

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev
```

Run commands:

```sh
cargo run -p app --bin myapp
cargo run -p daemon --bin myapp-daemon
RUST_LOG=debug MYAPP_DEV_LOG=1 cargo run -p daemon --bin myapp-daemon
cargo run -p daemon --bin myapp-daemon -- --hotkey-down
cargo run -p daemon --bin myapp-daemon -- --hotkey-up
cargo run -p daemon --bin myapp-daemon -- --hotkey-down --shortcut-id default
```

The app crate requires GTK4/libadwaita system development packages. If a build
fails because `gtk4.pc`, `libadwaita-1.pc`, or related pkg-config files are
missing, report the missing system package issue instead of rewriting the app
away from GTK4/libadwaita.

The daemon systemd user service file is for future installation and manual
testing only. The main app must not depend on it.

## Coding Conventions

- Keep crates and modules small.
- Keep shared wire/config/catalog/shortcut types in `shared`; do not duplicate
  protocol strings or model catalog values in app/daemon code.
- Use `anyhow::Result` at binary/runtime boundaries.
- Prefer pure tests in `shared`, model logic, hotkey state machines, daemon
  logic, and non-GTK app modules.
- Do not touch GTK from worker threads.
- Do not block the GTK main thread with network, hashing, or filesystem-heavy
  work.
- Use XDG/directories helpers instead of hardcoded user paths.
- Keep placeholder functions honest: log what would happen, but do not pretend
  integration is complete.
- Treat existing user changes as intentional. Do not revert unrelated work.
- During this development phase, do not preserve legacy config compatibility
  unless the user explicitly asks for it.

## Current Verification State

At the time this file was refreshed, the intended verification set is:

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `git diff --check`

If these fail because system GTK/libadwaita development packages are missing,
report that accurately. If they fail because of Rust code, fix the code or
explain the blocker.

## Future Work

Possible later additions, only when requested:

- XDG GlobalShortcuts portal backend,
- robust packaged evdev permissions through udev/logind/service integration,
- microphone recording,
- Whisper inference,
- clipboard insertion,
- script execution with recognized text,
- text insertion into the focused app,
- Flatpak packaging for the main app,
- `.deb` packaging for the daemon.
