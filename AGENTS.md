# MyApp AGENTS.md

Guidance for AI coding agents working in this repository.

## Project Goal

MyApp is a Rust Linux desktop prototype for a future push-to-talk voice
transcription utility. It should feel like a background desktop app to the
user, but during development the main app remains a normal foreground process
so logs stay visible and Ctrl-C works.

The current prototype includes:

- a Rust workspace with `app` and `shared` crates,
- a GTK4/libadwaita main app,
- a StatusNotifierItem tray/top-bar indicator through `ksni`,
- a settings window opened from the tray,
- app-owned TOML configuration,
- multi-shortcut settings,
- app-side X11 hotkey capture,
- Linux signal shortcut triggers for external Wayland hotkey utilities,
- whisper.cpp model catalog and download management,
- CPAL microphone capture from the configured default input,
- whisper-rs/whisper.cpp transcription on downloaded ggml models,
- real Linux clipboard output and script output actions.

Do not add Electron or Tauri. Do not add a MyApp input daemon, D-Bus hotkey
bridge, evdev capture, uinput paste worker, direct text insertion without
clipboard transport, XDG portal shortcuts, Flatpak packaging, or `.deb`
packaging unless explicitly requested.

## Workspace

- `crates/app`: builds `myapp`, the GTK4/libadwaita desktop app.
- `crates/shared`: shared config structs, model catalog, shortcut parsing,
  languages, output action types, persistence helpers, and app constants.

The app is intentionally not daemonized. Start it with:

```sh
cargo run -p app --bin myapp
```

It starts with no visible window, owns a GTK application hold, shows the tray
indicator, and exits through the same quit path for tray `Quit` and Ctrl-C.

## Current Module Map

- `Cargo.toml`: workspace members, Rust edition/version, shared dependency
  versions.
- `crates/shared/src/config/mod.rs`: `AppConfig`, `GeneralConfig`, config
  schema version, backend/mode/compute enums, validation, normalization, and
  config resolution helpers.
- `crates/shared/src/config/model.rs`: whisper.cpp model catalog, model IDs,
  filenames, URLs, size labels, and SHA-1 hashes.
- `crates/shared/src/config/shortcut.rs`: shortcut profiles, accelerator
  normalization, shortcut IDs, shortcut chord parsing, Linux signal names, and
  shortcut key types.
- `crates/shared/src/config/language.rs`: supported language list including
  `auto`, default inheritance, and Ukrainian.
- `crates/shared/src/config/output.rs`: default and per-shortcut output action
  types for the final-text pipeline: optional script transform and clipboard
  copy.
- `crates/shared/src/lib.rs`: shared exports and app-wide constants such as
  `APP_ID`.
- `crates/app/src/main.rs`: app module wiring and tracing setup.
- `crates/app/src/app.rs`: app runtime, command pump, lifecycle hold, Ctrl-C,
  tray/settings startup, config apply/save, model commands,
  recording/audio/transcription orchestration, Linux signal command handling, and
  quit path.
- `crates/app/src/audio/*`: CPAL input device enumeration, capture stream
  management, mono conversion, and 16 kHz resampling for Whisper.
- `crates/app/src/command.rs`: `AppCommand`, download IDs, and model download
  outcome messages. Worker threads should communicate with app state through
  these commands.
- `crates/app/src/config_store.rs`: app config load/save under the XDG config
  directory.
- `crates/app/src/hotkey/mod.rs`: pluggable app hotkey backend boundary and
  backend resolution.
- `crates/app/src/hotkey/x11.rs`: app-side X11 passive grab backend.
- `crates/app/src/output.rs`: output worker for external clipboard copy,
  clipboard verification, and script execution.
- `crates/app/src/models/*`: model directory, inventory cache, model row
  state composition, download management, cancellation, SHA-1 verification,
  atomic rename, and model deletion.
- `crates/app/src/settings/*`: `SettingsWindow`, unsaved draft state, pages,
  shortcut recorder, sidebar, and GTK/libadwaita helper widgets.
- `crates/app/src/tray.rs`: StatusNotifierItem tray, menu, recording-state
  labels, and generated color icon.
- `crates/app/src/recording.rs`: recording state machine and start/stop
  logging.
- `crates/app/src/recording/pipeline.rs`: background CPAL capture worker,
  explicit shutdown, capture start/stop commands, and stream recreation after
  pause failures.
- `crates/app/src/signal_trigger.rs`: app-owned Linux signal listener for
  external hotkey utilities, registered signal calculation, signal name
  resolution, and pure signal-to-recording action policy.
- `crates/app/src/transcription/*`: Whisper worker, engine, cache, compute
  selection, skip policy, debug audio writing, request/result types, and plan
  construction.
- `README.md`: user-facing build, run, config, hotkey, and troubleshooting
  instructions.

## Runtime Rules

- Do not fork, detach, or daemonize the main app during development.
- Do not add a systemd service for the main app.
- Keep logs visible in the terminal.
- Route Ctrl-C through `AppCommand::Quit`.
- Route tray, hotkey backend, Linux signal, download worker, transcription
  worker, output worker, and audio worker events through `AppCommand`.
- Keep GTK/libadwaita object access on the GTK main thread.
- Worker threads should send commands, not mutate GTK objects or app runtime
  fields directly.
- Keep Linux signal matching and same-signal start/stop decisions as pure logic
  in `signal_trigger.rs`; `AppRuntime` should only log and dispatch the selected
  start/stop action.

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
not use the toggle because push-to-talk requires explicit key down/up
semantics. Linux signal triggers may use the same configured start and stop
signal; the runtime must handle one received signal once, starting when idle and
stopping when the same shortcut is active.

Hotkey down snapshots the shortcut model/language/output/input settings and
requests CPAL audio capture for a shortcut id. Hotkey up stops that same
shortcut and sends the captured audio to the Whisper worker.

`RecordingService` owns only the state machine. `AppRuntime` owns active
command orchestration. `RecordingPipeline` owns background CPAL capture.
`TranscriptionService` owns background Whisper inference. `OutputService` owns
script execution and clipboard copy/verification.

## Configuration

The app is the source of truth for user settings. Current app config path:

```text
~/.config/myapp/config.toml
```

Current schema. Generated defaults are display-aware; on X11-capable sessions
the app creates the keyboard default plus a signal shortcut:

```toml
schema_version = 11

[general]
mode = "push_to_talk"
hotkey_backend = "auto"
default_input = { type = "system_default" }
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
keep_model_loaded = true
default_output = { copy_to_clipboard = true, paste_from_clipboard = false, paste_shortcut = "ctrl_v" }

[[shortcuts]]
id = "default"
name = "Default"
enabled = true
trigger = { type = "keyboard", accelerator = "Ctrl+Alt+Space" }
model_id = "default"
language = "default"
output = { type = "default" }

[[shortcuts]]
id = "signal"
name = "Signal"
enabled = true
trigger = { type = "linux_signal", start_signal = "SIGUSR1", stop_signal = "SIGUSR2" }
model_id = "default"
language = "default"
output = { type = "default" }
```

On Wayland or mixed Wayland/X11 sessions, generated defaults use
`linux_signal` on the permanent `Default` shortcut and Settings shows only
signal trigger controls.

Only schema v11 is supported during development. Do not add old-config
migration paths unless explicitly requested. If the schema changes during
active development, update the current schema and tests directly instead of
layering legacy compatibility; older schemas, including v10, are discarded and
replaced with the current default config.

Supported `compute_backend` values are `auto`, `cpu`, `vulkan`, `cuda`, and
`rocm`. Do not offer or parse OpenVINO unless a future whisper-rs integration
explicitly supports it.

Output is one simple pipeline: transcript, optional script transform, final
text, optional clipboard copy/transport, optional paste shortcut. If script is
enabled, its stdout is the final text and the original transcript must not be
copied as a fallback. Paste from clipboard uses the external clipboard as
transport and then sends a configured `xdotool` or `ydotool` shortcut.

If a local development config is from an older schema, remove
`~/.config/myapp/config.toml` and restart the app to generate the current
default config, or let the app replace unsupported schemas automatically.

## Settings UI

Settings uses GTK4 and libadwaita. It is hidden, not destroyed, on close. It
should not appear at startup.

Current pages:

- `General`: advanced hotkey status, backend, compute backend, default input,
  default model, default language, and default output.
- `Models`: whisper.cpp model download/remove/status management.
- one page per shortcut profile, with `Default` permanent.
- `Add New`: creates a new shortcut profile.

`SettingsDraft` owns the unsaved mutable copy of config. Saving remains
explicit through the `Save` button. Do not write config on every UI change.

On X11, shortcut pages show both keyboard and Linux signal trigger options. On
Wayland or mixed Wayland/X11 sessions, shortcut pages show only Linux signal
trigger controls and new shortcut profiles default to `SIGUSR1` start and
`SIGUSR2` stop. If the default signal pair is already used by an enabled
profile, newly added or coerced duplicate signal profiles should be left
disabled until the user configures unique signals and enables them.

Display capability coercion belongs in `SettingsDraft` before rendering pages.
Do not mutate the draft from a page builder just because a widget is being
rendered; page builders should reflect the current draft and update it only from
explicit user interactions.

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

Readiness rules:

- a model becomes ready only after successful SHA-1 verification and final file
  rename,
- model download workers should be named, cancellable, and canceled from the
  app quit path,
- startup should remove orphan catalog `.part` files before reconciling ready
  inventory,
- after completion, the current process must update in-memory ready model IDs
  immediately,
- a later app start should reconcile inventory with the real final file and
  catalog identity/hash metadata,
- do not mark a model not ready merely because the real file length differs
  from catalog `size_bytes` after a verified download,
- deleting a ready model must update the file, inventory, in-memory ready IDs,
  model rows, and model dropdown choices.

UI rules:

- while downloading, show stable per-row progress and a `Cancel` action,
- while canceling, show a disabled canceling state,
- while verifying, keep progress visible and indicate verification,
- when ready, show `Remove Model`,
- `Remove Model` should ask for confirmation before deleting,
- referenced models should not be silently deleted.

## Audio And Transcription

The app records audio through CPAL. Keep audio capture display-server
agnostic: X11/Wayland affects global hotkeys, not microphone capture.

Current audio behavior:

- `GeneralConfig.default_input` stores either `system_default` or a CPAL device
  reference with `host`, `id`, and human label,
- Settings > General lists `System Default` first, then discovered input
  devices,
- `System Default` resolves to the current host default at recording time,
- the default app build enables CPAL's native PipeWire and PulseAudio hosts,
- on modern Ubuntu desktops, prefer native PipeWire and fall back to
  PulseAudio through pipewire-pulse when PipeWire is unavailable,
- audio capture runs on the `myapp-audio-capture` worker thread, not on the GTK
  main thread,
- the CPAL callback writes only to a short ring buffer; the capture worker
  drains it into a bounded session buffer capped by the maximum recording
  duration,
- the input stream must be stopped while idle,
- app quit must explicitly shut down and join the capture worker.

Current transcription behavior:

- stop recording converts captured audio to 16 kHz mono `f32` with `rubato`,
- each shortcut resolves its own model, language, compute backend, and output
  snapshot before the worker starts,
- only ready/downloaded models may be used,
- `whisper-rs` is the Rust wrapper over whisper.cpp,
- model contexts are cached inside the transcription worker as one last-used
  model path and compute backend when `keep_model_loaded = true`,
- recognized text is logged at `info`,
- full request/result metadata is logged at `debug`,
- unusable short captures return `TranscriptionStatus::Skipped`, do not load
  Whisper, and do not trigger output actions,
- empty recognized text should warn with segment count and audio RMS/peak,
- `MYAPP_DEBUG_SAVE_AUDIO=1` writes debug WAV/TOML files under
  `/tmp/myapp-audio-debug`; setting it to a directory path writes there
  instead.

Auto language mode should allow whisper.cpp to auto-detect while continuing to
transcribe. Do not set `detect_language=true` for normal transcription because
that whisper.cpp flag exits after language detection and returns no transcript
segments.

Compute backend is selected from config. `auto` enables whisper.cpp GPU usage
when the binary is built with a GPU backend, retries CPU if `auto` GPU
initialization fails at runtime, and otherwise uses CPU behavior. Explicit
Vulkan/CUDA/ROCm selections should fail clearly if the app was not compiled
with the matching Cargo feature or if that runtime backend cannot initialize.

Do not block the GTK main thread with audio capture, model loading, inference,
downloads, hashing, or output execution. Keep those paths worker based and send
state changes back through `AppCommand`.

## Hotkey Architecture

Hotkey handling is pluggable. Current app-side backend types:

- `DisabledBackend`
- `X11Backend`

Configured backend values:

- `auto`: resolves to X11 when only `DISPLAY` is present; resolves to disabled
  on Wayland or when no supported display is present,
- `disabled`: no app-side global hotkey, tray manual actions and Linux signal
  triggers still work,
- `x11`: force app-side X11 passive grabs.

Shortcut trigger capabilities are display-derived, not config-derived: only
`DISPLAY` without `WAYLAND_DISPLAY` is considered keyboard-capable. X11 sessions
show keyboard and signal trigger controls. Wayland or mixed Wayland/X11 sessions
show signal trigger controls only.

X11 capture lives in the app and uses passive X11 grabs. The X11 backend sends
`StartRecording(shortcut_id)` on key down and `StopRecording(shortcut_id)` when
the required chord is no longer pressed.

Wayland capture is external to MyApp. Configure shortcut profiles as
`linux_signal`, then use an external utility such as `swhkd` to send Linux
signals to the `myapp` process. Signal fields are text fields; MyApp saves
arbitrary non-empty text, resolves common aliases and numeric signal values in
`signal_trigger.rs`, and logs unsupported values without failing startup.
`SIGUSR1` and `SIGUSR2` are always registered as guard signals; if either signal
does not match an enabled shortcut, MyApp logs the received signal at debug
level and continues running.

When a shortcut uses the same start and stop signal, each received signal is
handled once. If the app is idle it starts that shortcut. If that same shortcut
is arming or recording, the next received signal stops it. Signals for inactive
shortcuts or processing state are ignored with debug logging.

Example external trigger commands:

```sh
pkill -USR1 -x myapp
pkill -USR2 -x myapp
```

The main app should never be run with sudo.

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

Runtime clipboard output and paste shortcuts also need external tools:

```sh
sudo apt install wl-clipboard xclip xdotool ydotool
```

The app must not invoke package managers or auto-install these tools; if a
required tool is missing, log a clear output failure message with the package
hint. `xdotool` handles X11 paste shortcuts. `ydotool` handles Wayland paste
shortcuts and may require its own daemon/permissions outside MyApp.

Run commands:

```sh
cargo run -p app --bin myapp
cargo run -p app --bin myapp --no-default-features --features audio-pulseaudio
cargo run -p app --no-default-features --bin myapp
MYAPP_DEV_LOG=1 cargo run -p app --bin myapp
MYAPP_DEBUG_SAVE_AUDIO=1 MYAPP_DEV_LOG=1 cargo run -p app --features whisper-vulkan --bin myapp
```

`MYAPP_DEV_LOG=1` enables debug logs for MyApp crates while keeping dependency
crates at info level. A global `RUST_LOG=debug` is intentionally noisy and may
include PulseAudio internals.

The app crate requires GTK4/libadwaita system development packages. If a build
fails because `gtk4.pc`, `libadwaita-1.pc`, or related pkg-config files are
missing, report the missing system package issue instead of rewriting the app
away from GTK4/libadwaita.

## Coding Conventions

- Keep crates and modules small.
- Keep shared config/catalog/shortcut types in `shared`; do not duplicate model
  catalog values or shortcut parsing in app code.
- Use `anyhow::Result` at binary/runtime boundaries.
- Prefer pure tests in `shared`, model logic, hotkey state machines, and
  non-GTK app modules.
- Do not touch GTK from worker threads.
- Do not block the GTK main thread with network, hashing, filesystem-heavy
  work, model loading, audio capture, or Whisper inference.
- Clipboard output runs through the output worker. On Linux, use `wl-copy` /
  `wl-paste` for Wayland and `xclip` for X11, and log success only after an
  external readback verifies the expected text.
- Paste from clipboard runs through the output worker after successful clipboard
  verification. Use `xdotool key --clearmodifiers ...` on X11 and
  `ydotool key ...` raw key events on Wayland. Do not restore the in-repo daemon
  or uinput worker for paste.
- Wayland clipboard commands should explicitly offer/request a text MIME type
  such as `text/plain;charset=utf-8`.
- Do not use GTK/GDK clipboard self-readback as success proof on Wayland.
- Copy-to-clipboard leaves the final output text in the clipboard. Do not
  restore the previous clipboard value for the copy action.
- Use XDG/directories helpers instead of hardcoded user paths.
- Keep incomplete output integrations honest: clipboard, script output, and
  external-tool paste shortcuts are real; direct text insertion is not
  implemented.
- Treat existing user changes as intentional. Do not revert unrelated work.
- During this development phase, do not preserve legacy config compatibility
  unless the user explicitly asks for it.

## Current Verification State

The intended verification set is:

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `git diff --check`

`cargo test --workspace` should pass with the default parallel test runner.
Serial `-- --test-threads=1` runs are only for debugging.

For signal changes, also smoke-test the running app manually:

```sh
pkill -USR1 -x myapp
pkill -USR2 -x myapp
```

For a same-signal shortcut, send the same signal twice: the first signal should
start recording and the second should stop the active recording without exiting
the app.

If these fail because system GTK/libadwaita development packages are missing,
report that accurately. If they fail because of Rust code, fix the code or
explain the blocker.

## Future Work

Possible later additions, only when requested:

- XDG GlobalShortcuts portal backend,
- production-grade audio buffering/resampling,
- streaming/VAD transcription,
- direct text insertion into the focused app without clipboard transport,
- Flatpak packaging for the main app,
- `.deb` packaging for the main app.
