# Voice AGENTS.md

## Project Goal

Build a small Rust Linux desktop app for Ubuntu/Linux that provides push-to-talk
voice transcription. The first iteration is a minimal working prototype:

- a tray/status icon in the desktop panel,
- a Settings window,
- local config save/load,
- clean interfaces for hotkey, audio recording, and Whisper recognition,
- real microphone capture feeding the selected Whisper model.

In the current prototype, model loading, direct Whisper transcription,
microphone capture, and global push-to-talk shortcut registration are
implemented. Packaging, model discovery/download tooling, and more polished
runtime status are still future work.

## Current Architecture

The app is a single Rust binary using GTK3 for the Settings window and a
StatusNotifierItem/D-Bus tray implementation through the `ksni` crate as the
primary tray backend.

Modules:

- `app`: starts GTK, loads config, wires services, creates the tray and settings
  window, and enters the GTK main loop.
- `config`: owns `AppConfig` and `ConfigStore`; stores settings as TOML in
  `~/.config/voice/voice.toml` through XDG base directories.
- `tray`: owns tray/status icon logic behind `TrayBackend`; the primary backend
  is `StatusNotifierTray` and the legacy fallback is `AppIndicatorTray`.
- `activity`: owns the push-to-talk state machine for idle, recording,
  processing, and clipboard-copy completion. It updates tray state through
  `TrayBackend` and sends microphone stop/conversion plus Whisper work to a
  background worker thread so GTK stays responsive and release immediately
  changes the tray state from recording to processing.
- `settings`: owns the GTK3 Settings window, microphone selector, and
  save/browse actions.
- `hotkey`: defines `HotkeyBackend`, parses user hotkey strings, and provides
  `AutoHotkeyBackend`. It uses Linux evdev as the primary path on Wayland and
  X11 by reading `/dev/input/event*` directly, with `global-hotkey` retained as
  a real-X11 fallback. The evdev backend tracks exact key press/release state
  and ignores kernel auto-repeat. X11 fallback events pass through
  `HotkeyEdgeFilter`.
- `audio`: defines `AudioRecorder`; current implementation is
  `CpalAudioRecorder`, which captures Linux input audio through CPAL/ALSA,
  downmixes to mono, and resamples to Whisper's expected 16 kHz `i16` buffer.
  `StubAudioRecorder` is retained for tests/future mocks.
- `whisper`: defines `WhisperRecognizer`; current implementation is
  `RuntimeWhisperRecognizer` backed by `whisper-rs`, with `StubWhisperRecognizer`
  retained for tests/future mocks.

The tray code is intentionally isolated so the app is not tightly coupled to any
single tray implementation. StatusNotifierItem uses D-Bus-based desktop
integration and works on both Wayland and X11 environments where the desktop
shell provides a tray host. If SNI is unavailable, the app falls back to
Ayatana/AppIndicator through `appindicator3`. That fallback may print an
upstream deprecation warning for `libayatana-appindicator`; it should only be
used when the primary SNI path cannot start.

Tray visual state is intentionally app-owned rather than theme-owned:

- `Idle`: white icon.
- `Recording`: red icon.
- `Processing`: yellow icon.

After a transcription is copied to the clipboard, the state returns immediately
to `Idle`. StatusNotifierItem uses generated ARGB pixmaps. AppIndicator uses the
embedded SVG assets from `assets/icons` and materializes them under
`~/.cache/voice/icons` at runtime when the legacy fallback is needed.

The hotkey code is intentionally isolated behind `HotkeyBackend`. The evdev
backend does not call `grab()`, so it observes the configured push-to-talk
combination without blocking the desktop or focused app from receiving the same
keys. This means a focused terminal can still echo Alt/Escape sequences such as
`^[` while the app records. It requires read access to `/dev/input/event*`,
which is a sensitive permission because readable input devices can expose
keyboard events. The configured hotkey is parsed from Settings/config each time
`configure_push_to_talk` is called, so changing the hotkey in Settings
re-registers it without restarting the app. If registration fails while saving
Settings, the config file is not updated and the previous working binding
remains active.

## Important Commands

```sh
cargo fmt --check
cargo check
cargo test
cargo run
cargo build --release
cargo build --release --features gpu-vulkan
cargo check --features gpu-cuda
cargo check --features gpu-all
```

Ubuntu development packages expected for this iteration:

```sh
sudo apt install libgtk-3-dev libayatana-appindicator3-dev pkg-config
sudo apt install libasound2-dev
```

`libayatana-appindicator3-dev` is currently needed for the legacy fallback.
The primary tray backend does not use the deprecated Ayatana library.
`libasound2-dev` is needed by CPAL's ALSA backend for microphone capture.
Evdev push-to-talk requires the running user to have read access to
`/dev/input/event*`; on development machines this is often handled by membership
in the `input` group plus a fresh login session.

`whisper-rs` is pinned to `0.13.2`. Its Vulkan feature compiles in this project,
while checked newer versions `0.14.4`, `0.15.1`, and `0.16.0` fail to compile
their Vulkan module because of missing upstream FFI symbols. Do not upgrade the
crate casually until the Vulkan feature is revalidated or the binding is
replaced.

## Coding Conventions

- Keep modules small and focused.
- Prefer explicit traits at system boundaries: tray, hotkey, audio, Whisper.
- Keep GTK-specific code in `settings` and the concrete tray backend only.
- Store persistent settings through `config`; do not write config files from UI
  code directly.
- Use `anyhow::Result` at application boundaries and stub interfaces.
- Avoid hardcoded system paths. Use XDG locations for user config.
- Keep model files out of git; local models belong under `models/` or an XDG
  data directory.
- Keep long-running recording stop/conversion and transcription work off the GTK
  main loop.
- For Whisper transcription, use `language = auto` but do not enable
  `detect_language`; on the current whisper.cpp binding that can return after
  language detection with zero text segments.
- Leave TODO comments at future integration points, but do not fake completed
  integrations.

## Current Implementation Status

Implemented:

- Rust binary scaffold.
- GTK3 Settings window.
- StatusNotifierItem tray menu with `Settings` and `Quit`.
- Legacy AppIndicator fallback for environments where SNI is unavailable.
- Dynamic fallback activation if the SNI watcher goes offline while the app is
  running.
- Tray visual state switching for idle, recording, and processing.
- Optional runtime diagnostic logging with `VOICE_DEBUG=1` for captured audio
  duration and levels, Whisper segments, and clipboard copy success.
- XDG TOML config save/load.
- Text entry for push-to-talk hotkey.
- Microphone selector with system default plus enumerated CPAL input devices.
- Text entry plus file chooser for Whisper model path/name.
- Real Whisper model loading/transcription backend through `whisper-rs`.
- Automatic Whisper backend preference in config, defaulting to `auto`.
- Runtime backend status in Settings, for example `Auto: CPU` or
  `Auto: Vulkan GPU`.
- Global push-to-talk hotkey handling through evdev `/dev/input/event*`, with
  X11 fallback for real X11 sessions.
- Dynamic hotkey re-registration from Settings without app restart.
- Hotkey parser for user-facing strings such as `Ctrl+Alt+Space`.
- Evdev press/release state tracking plus X11 hotkey edge filtering so OS key
  auto-repeat does not restart or stop push-to-talk recording.
- Real CPAL microphone recording with mono 16 kHz conversion for Whisper.
- Empty audio buffers skip Whisper transcription and return the activity state
  to `Idle` immediately.
- Push-to-talk workflow controller records on press, processes on release, and
  prints and copies non-empty transcription results to the GTK clipboard.

Not implemented yet:

- Model discovery/download tooling in the GUI.
- Packaged release profiles/installers.
- Tray/menu status details beyond icon color.

## Next Planned Steps

- Add Whisper model discovery/download tooling. Models are expected to be
  downloaded by a separate command-line utility, not by the GUI in this first
  design.
- Add packaged build profiles for CPU-only, Vulkan-capable, and CUDA-capable
  binaries. `cargo build --features gpu-vulkan` passes locally. Local CUDA and
  `gpu-all` full builds currently fail at link time because `cublas`, `cudart`,
  `cublasLt`, and `culibos` are not installed in the linker path; `cargo check`
  for those features passes.
- Add richer runtime status in the tray/menu, such as selected microphone,
  active backend, last duration, and last error.
- Add a cleaner distribution-time permission story for evdev, such as a small
  helper, udev/logind integration, or a documented installer step.
- Revisit the legacy fallback if a maintained `libayatana-appindicator-glib`
  binding becomes the better option.
- Prepare Nix/NixOS packaging later with explicit native dependencies and a
  `flake.nix`/devShell.
