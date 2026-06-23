# QuillSpeak AGENTS.md

Guidance for AI coding agents working in this repository. This file is for
agent orientation, not for end-user documentation.

## Project Goal

QuillSpeak is a Rust Linux desktop prototype for local push-to-talk voice
transcription. It should feel like a tray/background desktop app to the user,
but during development the main app remains a normal foreground process so logs
stay visible and Ctrl-C works.

Current user-facing features:

- GTK4/libadwaita settings UI.
- StatusNotifierItem tray indicator through `ksni`.
- App-owned TOML config under XDG config.
- Multi-shortcut profiles.
- X11 keyboard shortcut capture.
- Wayland-friendly external trigger command mode.
- Linux signal shortcut triggers.
- whisper.cpp model catalog, download, verification, and removal.
- CPAL microphone capture through PipeWire/PulseAudio hosts.
- whisper-rs/whisper.cpp transcription on downloaded ggml models.
- Local transcription history.
- Script transform, real clipboard copy, and optional paste shortcuts.
- GitHub Pages documentation and GitHub Release `.deb` packages.

Do not add Electron or Tauri. Do not add a QuillSpeak input daemon, D-Bus hotkey
bridge, evdev capture, uinput paste worker, direct text insertion without
clipboard transport, XDG portal shortcuts, or Flatpak packaging unless
explicitly requested.

## Workspace Map

- `crates/app`: builds the `quillspeak` GTK4/libadwaita desktop app.
- `crates/shared`: shared config, model catalog, shortcut parsing, languages,
  output action types, persistence helpers, and app constants.
- `assets`: desktop file, app icon, and AppStream metadata.
- `docs`: static GitHub Pages documentation, images, examples, and release UI.
- `scripts/build-docs.sh`: builds `_site` for Pages.
- `.github/workflows/ci.yml`: Rust checks.
- `.github/workflows/pages.yml`: static docs deployment.
- `.github/workflows/release.yml`: tag-driven Debian package release.
- `README.md`: short public project overview; detailed docs live in `docs/`.
- `LICENSE`: MIT license.

The app is intentionally not daemonized. Start it with:

```sh
cargo run -p quillspeak --bin quillspeak
```

It starts with no visible window, owns a GTK application hold, shows the tray
indicator, and exits through the same quit path for tray `Quit` and Ctrl-C.

## Important App Modules

- `crates/app/src/app.rs`: runtime command pump, lifecycle hold, Ctrl-C, tray,
  settings, config apply/save, model commands, recording/audio/transcription
  orchestration, Linux signal handling, and quit path.
- `crates/app/src/external_trigger.rs`: single-binary command mode and Unix
  socket for `quillspeak trigger <shortcut> <start|stop|toggle>`.
- `crates/app/src/signal_trigger.rs`: pure signal-to-recording action policy.
- `crates/app/src/hotkey/x11.rs`: app-side X11 passive grab backend.
- `crates/app/src/audio/*`: CPAL device enumeration and capture pipeline.
- `crates/app/src/transcription/*`: Whisper worker, compute selection, model
  cache, debug audio, request/result types, and plan construction.
- `crates/app/src/output.rs`: script execution, external clipboard copy,
  readback verification, and paste shortcuts.
- `crates/app/src/models/*`: model directory, inventory, downloads,
  cancellation, SHA-1 verification, atomic rename, and deletion.
- `crates/app/src/settings/*`: settings window, draft state, pages, shortcut
  recorder, sidebar, and widgets.
- `crates/app/src/settings/pages/history.rs`: local transcription history UI.
- `crates/app/src/tray.rs`: tray menu, recording labels, and generated icon.
- `crates/shared/src/config/*`: config schema, model catalog, languages,
  shortcuts, and output action types.

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
  in `signal_trigger.rs`.

The tray indicator is user-facing app state:

- white icon means idle,
- red icon means recording or arming,
- orange icon means processing/transcription.

Manual tray recording uses `AppCommand::ToggleRecording`. Hotkey and command
backends should prefer explicit start/stop edges for push-to-talk.

## Configuration

The app is the source of truth for user settings. Current config path:

```text
~/.config/quillspeak/config.toml
```

Only schema v16 is supported during active development. Do not add old-config
migration paths unless explicitly requested. If the schema changes, update the
current schema and tests directly.

Current core config shape:

```toml
schema_version = 16

[general]
mode = "push_to_talk"
hotkey_backend = "auto"
audio_input = { type = "system_default" }
compute_backend = "auto"
keep_model_loaded = true

[[shortcuts]]
id = "default"
name = "Default"
enabled = true
trigger = { type = "keyboard", accelerator = "Ctrl+Alt+Space" }
model_id = "large-v3-turbo-q5_0"
language = "auto"
mute_output_while_recording = false
beep_on_recording = false
beep_volume_percent = 100
output = { copy_to_clipboard = true, paste_from_clipboard = false, paste_shortcut = "ctrl_v" }
```

Supported `hotkey_backend` values: `auto`, `disabled`, `x11`.
Supported `compute_backend` values: `auto`, `cpu`, `vulkan`, `cuda`, `rocm`.
Do not offer or parse OpenVINO unless whisper-rs support is added later.

Each shortcut owns its model, language, speaker-mute preference, beep settings,
and output pipeline. No shortcut setting inherits from General or `Default`.

Output pipeline:

```text
transcript -> optional script -> final text -> optional clipboard copy -> optional paste
```

The output script receives the transcript as the first command-line argument
(`argv[1]` / `$1`), not stdin. If the script succeeds, stdout becomes final
text. The original transcript must not be copied as fallback after script
errors.

## Settings UI

Settings uses GTK4 and libadwaita. It is hidden, not destroyed, on close, and
should not appear at startup.

Current pages:

- `Status`: runtime/tool readiness and Whisper compute status.
- `General`: hotkey backend, compute backend, audio input, model cache behavior.
- `Models`: whisper.cpp model download/remove/status management.
- `History`: local transcription history, copy row action, clear history.
- one page per shortcut profile, with `Default` permanent.
- `Add New`: creates a new shortcut profile.

`SettingsDraft` owns the unsaved mutable copy of config. Saving remains
explicit through the `Save Changes` button. Do not write config on every UI
change.

On X11, shortcut pages show both keyboard and Linux signal trigger options. On
Wayland or mixed Wayland/X11 sessions, shortcut pages show only Linux signal
trigger controls. Display capability coercion belongs in `SettingsDraft` before
rendering pages.

Shortcut pages should show only ready models as normal choices. If a configured
model is missing, the UI may show a missing marker so the user can fix it.

Avoid raw GTK containers as direct `adw::PreferencesGroup` rows when they cause
GTK focus/listbox warnings. Prefer `adw::PreferencesRow`, `adw::ActionRow`, or a
small row controller that owns valid libadwaita rows.

## Hotkeys, Command Mode, And Signals

Hotkey handling is pluggable:

- `DisabledBackend`
- `X11Backend`

Backend resolution:

- `auto`: X11 only when `DISPLAY` is present and `WAYLAND_DISPLAY` is absent.
- `disabled`: no app-side global hotkey; tray, command mode, and signals still work.
- `x11`: force app-side X11 passive grabs.

Wayland capture is external to QuillSpeak. Prefer compositor keybindings or
`swhkd` calling:

```sh
quillspeak trigger Default start
quillspeak trigger Default stop
quillspeak trigger Default toggle
```

Command mode sends one line to:

```text
$XDG_RUNTIME_DIR/quillspeak/command.sock
```

Shortcut selectors resolve by exact id first, then exact unique display name.
Disabled, missing, ambiguous, and no-op commands should fail with a non-zero
exit code.

Linux signal shortcut values are exactly `SIGUSR1`, `SIGUSR2`, `SIGALRM`, and
`SIGWINCH`. Aliases, numeric values, reserved process-control signals, and
custom names are not supported.

Same-signal shortcuts are handled once per received signal: when idle, the
signal starts that shortcut; when that same shortcut is active, the next matching
signal stops it.

The main app should never be run with `sudo`.

## Audio, Transcription, Models

- Audio capture is display-server agnostic.
- `System Default` audio input resolves to the current host default at recording time.
- The default build enables CPAL PipeWire and PulseAudio hosts.
- CPAL capture runs on the `quillspeak-audio-capture` worker thread.
- Stop recording converts captured audio to 16 kHz mono `f32` with `rubato`.
- Only ready/downloaded models may be used.
- `whisper-rs` is the Rust wrapper over whisper.cpp.
- `keep_model_loaded = true` caches the last-used model path and compute backend.
- `QUILLSPEAK_DEBUG_SAVE_AUDIO=1` writes debug WAV/TOML files under
  `/tmp/quillspeak-audio-debug`; setting it to a directory path writes there.
- Auto language should let whisper.cpp auto-detect while continuing to
  transcribe. Do not set `detect_language=true` for normal transcription.

Model files live under:

```text
~/.local/share/quillspeak/models
```

Readiness is based on successful SHA-1 verification and final file rename, not
only on catalog size.

## Output Tools

Clipboard output runs through the output worker. On Linux:

- Wayland copy/readback: `wl-copy` / `wl-paste` with `text/plain;charset=utf-8`.
- X11 copy/readback: `xclip`.
- X11 paste: `xdotool key --clearmodifiers ...`.
- Wayland paste: `ydotool key ...`.
- Speaker mute: prefer `wpctl` and `pw-dump`; use `pactl` fallback when needed.

Do not use GTK/GDK clipboard self-readback as success proof on Wayland. Do not
restore the previous clipboard value for copy actions.

## Documentation And Releases

Public docs are static HTML/CSS/JS in `docs/`. Keep README short and link to
the docs site for detailed instructions.

Docs build:

```sh
scripts/build-docs.sh _site
```

The Pages workflow publishes `_site`. The release workflow builds:

- `quillspeak`: default Vulkan-enabled package with CPU fallback.
- `quillspeak-cpu`: CPU-only package.
- `SHA256SUMS` and `release-manifest.json`.

Pushing a tag matching `v*` triggers the release workflow.

## Useful Commands

```sh
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

System dependencies for full app builds on Debian/Ubuntu-style systems:

```sh
sudo apt install build-essential pkg-config cmake clang libclang-dev \
  libasound2-dev libpulse-dev libpipewire-0.3-dev \
  libgtk-4-dev libadwaita-1-dev
```

Runtime clipboard/paste tools:

```sh
sudo apt install wl-clipboard xclip xdotool ydotool
```

Run commands:

```sh
cargo run -p quillspeak --bin quillspeak
cargo run -p quillspeak --bin quillspeak --no-default-features --features audio-pulseaudio
cargo run -p quillspeak --no-default-features --bin quillspeak
QUILLSPEAK_DEV_LOG=1 cargo run -p quillspeak --bin quillspeak
QUILLSPEAK_DEBUG_SAVE_AUDIO=1 QUILLSPEAK_DEV_LOG=1 cargo run -p quillspeak --features whisper-vulkan --bin quillspeak
```

`QUILLSPEAK_DEV_LOG=1` enables debug logs for QuillSpeak crates while keeping
dependency crates quieter than global `RUST_LOG=debug`.

If a build fails because `gtk4.pc`, `libadwaita-1.pc`, or related pkg-config
files are missing, report the missing system package issue instead of rewriting
the app away from GTK4/libadwaita.

## Coding Conventions

- Keep crates and modules small.
- Keep shared config/catalog/shortcut types in `shared`.
- Use `anyhow::Result` at binary/runtime boundaries.
- Prefer pure tests in `shared`, model logic, hotkey state machines, and non-GTK app modules.
- Do not touch GTK from worker threads.
- Do not block the GTK main thread with network, hashing, filesystem-heavy
  work, model loading, audio capture, Whisper inference, or output execution.
- Use XDG/directories helpers instead of hardcoded user paths.
- Keep clipboard, script output, and external-tool paste shortcuts real; direct
  text insertion is not implemented.
- Treat existing user changes as intentional. Do not revert unrelated work.
- During active development, do not preserve legacy config compatibility unless
  the user explicitly asks for it.
