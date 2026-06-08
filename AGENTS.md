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
- CPAL microphone capture from the configured default input,
- whisper-rs/whisper.cpp transcription on downloaded ggml models,
- real Linux clipboard output, daemon-side auto-paste, and script output
  actions.

Do not add Electron or Tauri. Do not require the daemon for main app startup.
Do not require sudo for the main app. Do not implement direct text insertion
without clipboard transport, XDG portal shortcuts, Flatpak packaging, or `.deb`
packaging unless explicitly requested.

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
  types for the final-text pipeline: optional script transform, clipboard copy,
  and auto-paste settings.
- `crates/shared/src/protocol.rs`: app/daemon D-Bus names, paths, interfaces,
  daemon status values, and `ShortcutRuntimeConfig`.
- `crates/app/src/main.rs`: app module wiring and tracing setup.
- `crates/app/src/app.rs`: app runtime, command pump, lifecycle hold, Ctrl-C,
  tray/settings startup, config apply/save, daemon sync, model commands,
  recording/audio/transcription orchestration, and quit path.
- `crates/app/src/audio/*`: CPAL input device enumeration, capture stream
  management, mono conversion, and 16 kHz resampling for Whisper.
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
- `crates/app/src/output.rs`: output worker for external clipboard copy,
  clipboard verification, script execution, and transcript paste requests.
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
- `crates/app/src/recording.rs`: recording state machine and start/stop
  logging.
- `crates/app/src/recording/pipeline.rs`: background CPAL capture worker,
  explicit shutdown, capture start/stop commands, and stream recreation after
  pause failures.
- `crates/app/src/transcription/service.rs`: background Whisper worker thread
  and request/result command bridge.
- `crates/app/src/transcription/engine.rs`: high-level Whisper transcription
  flow and segment extraction.
- `crates/app/src/transcription/cache.rs`: last-used model context cache.
- `crates/app/src/transcription/params.rs`: Whisper context parameters,
  auto GPU selection, and CPU fallback.
- `crates/app/src/transcription/skip.rs`: short-capture skip policy, padding,
  and skipped result construction.
- `crates/app/src/transcription/debug_audio.rs`: debug WAV/TOML writing.
- `crates/app/src/transcription/types.rs`: transcription plan/request/result
  types and debug metadata.
- `crates/daemon/src/main.rs`: daemon CLI, D-Bus service, startup flow, app
  config request, status notification, and Ctrl-C handling.
- `crates/daemon/src/cache.rs`: daemon last-known shortcut runtime config cache.
- `crates/daemon/src/hotkey.rs`: daemon hotkey state machine and runtime
  shortcut extraction.
- `crates/daemon/src/evdev_backend.rs`: daemon evdev device scanning, watched
  key filtering, shortcut key mapping, and app hotkey event dispatch.
- `crates/daemon/src/paste.rs`: daemon uinput virtual keyboard worker for
  clipboard paste shortcuts.
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
- arming: `Stop Recording`,
- recording: `Stop Recording`,
- processing: disabled `Processing...`.

Manual tray recording uses `AppCommand::ToggleRecording`. Hotkey backends must
not use the toggle because push-to-talk requires explicit key down/up semantics.
Hotkey down snapshots the shortcut model/language/output/input settings and
requests CPAL audio capture for a shortcut id. Hotkey up stops that same
shortcut and sends the captured audio to the Whisper worker.

`RecordingService` owns only the state machine. `AppRuntime` owns the active
command orchestration. `RecordingPipeline` owns the background CPAL capture
worker. `TranscriptionService` owns background Whisper inference. Output
actions run through workers: clipboard copy happens in the app output worker,
auto-paste is requested from the daemon after a successful final-text clipboard
copy, and scripts run in the app output worker.

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
- `PasteClipboard() -> bool`

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
schema_version = 7

[general]
mode = "push_to_talk"
hotkey_backend = "auto"
default_input = { type = "system_default" }
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
keep_model_loaded = true
default_output = { copy_to_clipboard = true, auto_paste = false }

[[shortcuts]]
id = "default"
name = "Default"
enabled = true
trigger = { type = "keyboard", accelerator = "Ctrl+Alt+Space" }
model_id = "default"
language = "default"
output = { type = "default" }
```

Only schema v7 is supported during development. Do not add old-config
migration paths unless explicitly requested. If the schema changes during
active development, update the current schema and tests directly instead of
layering legacy compatibility.

Output is one simple pipeline: transcript, optional script transform, final
text, optional clipboard copy, optional auto-paste. If script is enabled, its
stdout is the final text and the original transcript must not be copied or
pasted as a fallback. Auto-paste uses fixed `Ctrl+V`; even when
`copy_to_clipboard = false`, auto-paste may copy final text to clipboard as its
transport.

If a local development config is from an older schema, remove
`~/.config/myapp/config.toml` and restart the app to generate the current
default config.

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

- `General`: daemon status, backend, compute backend, default input, default
  model, default language, and default output.
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

## Audio And Transcription

The app records audio through CPAL. Keep audio capture display-server agnostic:
X11/Wayland affects global hotkeys, not microphone capture.

Current audio behavior:

- `GeneralConfig.default_input` stores either `system_default` or a CPAL device
  reference with `host`, `id`, and human label;
- Settings > General lists `System Default` first, then discovered input
  devices;
- `System Default` resolves to the current host default at recording time;
- the default app build enables CPAL's native PipeWire and PulseAudio hosts;
- on modern Ubuntu desktops, prefer native PipeWire and fall back to PulseAudio
  through pipewire-pulse when PipeWire is unavailable;
- the default app build requires `libpipewire-0.3-dev`; use
  `--no-default-features --features audio-pulseaudio` to build the PulseAudio
  fallback without native PipeWire;
- `--no-default-features` is the ALSA-only fallback for debugging;
- audio capture runs on the `myapp-audio-capture` worker thread, not on the GTK
  main thread;
- the input stream must be stopped while idle; start the CPAL stream only while
  recording and pause it again on stop/cancel;
- if pausing a stream fails, preserve the captured audio but drop the stream so
  the next recording recreates it;
- app quit must explicitly shut down and join the capture worker;
- the worker may reuse a constructed stream for the selected input, but it must
  not call `stream.play()` as an idle prewarm;
- recording uses a lock-free ring buffer, requests a bounded input buffer when
  supported, rejects stale callbacks from the previous session by CPAL capture
  timestamps, and reports startup / first accepted callback latency back to the
  app.

Current transcription behavior:

- stop recording converts captured audio to 16 kHz mono `f32` with `rubato`;
- each shortcut resolves its own model, language, compute backend, and output
  snapshot before the worker starts;
- only ready/downloaded models may be used;
- `whisper-rs` is the Rust wrapper over whisper.cpp;
- model contexts are cached inside the transcription worker as one last-used
  model path and compute backend when `keep_model_loaded = true`;
- recognized text is logged at `info`;
- full request/result metadata is logged at `debug`;
- capture diagnostics include input device, audio duration, wall-clock shortcut
  hold duration, startup latency, first accepted callback latency, callback
  count, frames, RMS, peak, dropped samples, missed audio chunks, discarded
  stale callback count, and discarded stale sample count;
- unusable short captures return `TranscriptionStatus::Skipped`, do not load
  Whisper, do not trigger output actions, and are distinct from completed
  transcriptions with empty recognized text;
- very low callback count should warn with capture diagnostics, not skip
  otherwise usable audio;
- empty recognized text should warn with segment count and audio RMS/peak;
- `MYAPP_DEBUG_SAVE_AUDIO=1` writes source WAV, the exact 16 kHz mono WAV sent
  to Whisper, and TOML metadata under `/tmp/myapp-audio-debug`; setting it to a
  directory path writes there instead. Skipped captures may write debug audio
  for diagnosis, but that audio is not sent to Whisper.

Auto language mode should allow whisper.cpp to auto-detect while continuing to
transcribe. Do not set `detect_language=true` for normal transcription because
that whisper.cpp flag exits after language detection and returns no transcript
segments.

Compute backend is selected from config. `auto` enables whisper.cpp GPU usage
when the binary is built with a GPU backend, retries CPU if `auto` GPU
initialization fails at runtime, and otherwise uses CPU behavior. Explicit
Vulkan/CUDA/ROCm selections should fail clearly if the app was not compiled
with the matching Cargo feature or if that runtime backend cannot initialize.
OpenVINO is a future setting placeholder and is not implemented by the current
`whisper-rs` integration. Vulkan is the intended packaged GPU backend so end
users can install a future `.deb` without compiling CUDA locally. The workspace
currently pins `whisper-rs = "=0.13.2"` because that version's Vulkan feature
builds here; retest before upgrading because `0.14.4`, `0.15.1`, and `0.16.0`
currently fail to compile their Vulkan wrapper against generated
`whisper-rs-sys` bindings.

Do not block the GTK main thread with audio capture, model loading, inference,
downloads, hashing, or output execution. Keep those paths worker based and send
state changes back through `AppCommand`.

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
sudo apt install build-essential pkg-config cmake clang libclang-dev libasound2-dev libpulse-dev libpipewire-0.3-dev libgtk-4-dev libadwaita-1-dev
```

Runtime clipboard output also needs external clipboard tools. They are not
guaranteed to be installed by the desktop environment:

```sh
sudo apt install wl-clipboard xclip
```

`wl-clipboard` provides `wl-copy` and `wl-paste` for Wayland. `xclip` is used
for X11. The app must not invoke package managers or auto-install these tools;
if a required tool is missing, log a clear `clipboard copy failed` message with
the package hint. Future packaging may declare these as runtime dependencies or
recommendations.

Auto-paste uses the optional daemon and `/dev/uinput` to emit fixed `Ctrl+V`
after final-text clipboard copy succeeds. Missing uinput permissions must not
block daemon startup; report them when paste is requested.

Default audio uses native PipeWire plus PulseAudio. The PulseAudio-only build is
useful when native PipeWire development files are unavailable, and ALSA-only is
the low-level debug fallback:

```sh
cargo run -p app --bin myapp --no-default-features --features audio-pulseaudio
cargo run -p app --no-default-features --bin myapp
```

Vulkan GPU builds are the intended packaged GPU path:

```sh
sudo apt install libvulkan-dev glslc
cargo check -p app --features whisper-vulkan
cargo run -p app --features whisper-vulkan --bin myapp
cargo run -p app --no-default-features --features whisper-vulkan,audio-pulseaudio --bin myapp
```

Run commands:

```sh
cargo run -p app --bin myapp
cargo run -p app --bin myapp --no-default-features --features audio-pulseaudio
cargo run -p app --no-default-features --bin myapp
cargo run -p daemon --bin myapp-daemon
MYAPP_DEV_LOG=1 cargo run -p app --bin myapp
MYAPP_DEBUG_SAVE_AUDIO=1 MYAPP_DEV_LOG=1 cargo run -p app --features whisper-vulkan --bin myapp
MYAPP_DEV_LOG=1 cargo run -p daemon --bin myapp-daemon
cargo run -p daemon --bin myapp-daemon -- --hotkey-down
cargo run -p daemon --bin myapp-daemon -- --hotkey-up
cargo run -p daemon --bin myapp-daemon -- --hotkey-down --shortcut-id default
```

`MYAPP_DEV_LOG=1` enables debug logs for MyApp crates while keeping dependency
crates at info level. A global `RUST_LOG=debug` is intentionally noisy and may
include PulseAudio/zbus internals.

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
- Do not block the GTK main thread with network, hashing, filesystem-heavy
  work, model loading, audio capture, or Whisper inference.
- Clipboard output runs through the output worker. On Linux, use `wl-copy` /
  `wl-paste` for Wayland and `xclip` for X11, and log success only after an
  external readback verifies the expected text.
- Wayland clipboard commands should explicitly offer/request a text MIME type
  such as `text/plain;charset=utf-8`; do not rely on `wl-copy` MIME inference
  for recognized text.
- Do not use GTK/GDK clipboard self-readback as success proof on Wayland. It can
  prove only that the app can read its own provider, not that other clients can
  paste the text.
- Copy-to-clipboard leaves the final output text in the clipboard. Do not
  restore the previous clipboard value for the copy action.
- Auto-paste uses clipboard transport and must paste the final output text:
  script stdout when script is enabled, otherwise the transcript.
- Daemon-side paste uses uinput key synthesis only; keep it worker based and
  expose it through `PasteClipboard() -> bool` with fixed `Ctrl+V`.
- Use XDG/directories helpers instead of hardcoded user paths.
- Keep incomplete output integrations honest: clipboard and script output are
  real, and auto-paste is clipboard-based. Direct text insertion without
  clipboard transport is still not implemented.
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
- production-grade audio buffering/resampling,
- streaming/VAD transcription,
- direct text insertion into the focused app without clipboard transport,
- Flatpak packaging for the main app,
- `.deb` packaging for the daemon.
