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
  recording/audio/transcription orchestration, Linux signal handling, and quit
  path.
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
- `crates/app/src/signal_trigger.rs`: app-owned `SIGUSR1`/`SIGUSR2` listener
  for external hotkey utilities.
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
semantics. Linux signal triggers may intentionally toggle when the configured
start and stop signal are the same.

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

Current schema:

```toml
schema_version = 8

[general]
mode = "push_to_talk"
hotkey_backend = "auto"
default_input = { type = "system_default" }
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
keep_model_loaded = true
default_output = { copy_to_clipboard = true }

[[shortcuts]]
id = "default"
name = "Default"
enabled = true
trigger = { type = "keyboard", accelerator = "Ctrl+Alt+Space" }
model_id = "default"
language = "default"
output = { type = "default" }
```

Only schema v8 is supported during development. Do not add old-config
migration paths unless explicitly requested. If the schema changes during
active development, update the current schema and tests directly instead of
layering legacy compatibility; older schemas, including v7, are discarded and
replaced with the current default config.

Output is one simple pipeline: transcript, optional script transform, final
text, optional clipboard copy. If script is enabled, its stdout is the final
text and the original transcript must not be copied as a fallback. Auto-paste
is not implemented.

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

X11 capture lives in the app and uses passive X11 grabs. The X11 backend sends
`StartRecording(shortcut_id)` on key down and `StopRecording(shortcut_id)` when
the required chord is no longer pressed.

Wayland capture is external to MyApp. Configure shortcut profiles as
`linux_signal`, then use an external utility such as `swhkd` to send `SIGUSR1`
or `SIGUSR2` to the `myapp` process. MyApp listens for those signals in
`signal_trigger.rs`.

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

Runtime clipboard output also needs external clipboard tools:

```sh
sudo apt install wl-clipboard xclip
```

The app must not invoke package managers or auto-install these tools; if a
required tool is missing, log a clear `clipboard copy failed` message with the
package hint.

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
- Wayland clipboard commands should explicitly offer/request a text MIME type
  such as `text/plain;charset=utf-8`.
- Do not use GTK/GDK clipboard self-readback as success proof on Wayland.
- Copy-to-clipboard leaves the final output text in the clipboard. Do not
  restore the previous clipboard value for the copy action.
- Use XDG/directories helpers instead of hardcoded user paths.
- Keep incomplete output integrations honest: clipboard and script output are
  real; auto-paste and direct text insertion are not implemented.
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
