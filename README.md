# Voice

Minimal Rust Linux desktop prototype for a future push-to-talk Whisper app.

The tray icon uses StatusNotifierItem/D-Bus as the primary Linux-native backend.
Legacy Ayatana/AppIndicator is kept only as a fallback for desktops where SNI is
not available.

## Run

Install Ubuntu development dependencies:

```sh
sudo apt install libgtk-3-dev libayatana-appindicator3-dev pkg-config
```

`libayatana-appindicator3-dev` is only for the legacy tray fallback. On a normal
SNI-capable desktop, the app should not hit that path.

Microphone capture uses ALSA through CPAL on Linux:

```sh
sudo apt install libasound2-dev
```

Then run:

```sh
cargo run
```

For hotkey/audio diagnostics while testing:

```sh
VOICE_DEBUG=1 cargo run
```

For a release-style run, build once and start the compiled binary:

```sh
cargo build --release
./target/release/voice
```

The app starts a tray/status icon. Open `Settings` from the tray menu to edit:

- push-to-talk hotkey by clicking the hotkey field and pressing a new
  combination,
- microphone device, or system default,
- Whisper model path/name,
- current Whisper backend status.

The Settings window uses a light theme, opens at a size large enough to show
the action buttons, and hides instead of being destroyed when closed from the
window manager. Reopening Settings from the tray shows the same live window
runtime again.

The push-to-talk hotkey is global:

- The app first tries XDG Desktop Portal GlobalShortcuts, the desktop-mediated
  Wayland-safe path. Your desktop may show a system prompt to approve the
  shortcut.
- If the portal is active and Linux evdev is readable, evdev is also used as a
  release guard so physical key release can stop recording even if a desktop
  portal signal is missed.
- If the portal is unavailable or declined, Wayland and X11 can fall back to
  Linux evdev by reading `/dev/input/event*` for exact key press/release events.
- The evdev fallback does not exclusively grab the keyboard, so the desktop and
  focused app can still see the same key combination. If a terminal is focused,
  combinations with `Alt` can show escape characters like `^[`; that comes from
  the terminal receiving the shortcut too, not from the voice app logging raw
  keys.
- Evdev fallback/release guard requires the process to read input devices. On
  many development machines that means the user must be in the `input` group,
  then log out and back in.
- Real X11 sessions can still use the native X11 fallback if portal and evdev
  are not available.
- Changing the hotkey in Settings re-registers it immediately; no restart is
  required.
- If registration fails, Settings shows an error and the old config is kept.

Direct evdev access is sensitive: any process with read access to input devices
can technically observe keyboard events. This prototype only watches the
configured push-to-talk keycodes and does not log or store raw keyboard input.

Supported hotkey text includes combinations like:

```text
Ctrl+Alt+Space
Ctrl+Shift+R
Super+Space
F12
```

The visual egui hotkey recorder currently captures Ctrl/Alt/Shift combinations
and common keyboard keys. `Super` remains supported by the config parser and
runtime backends, but visual capture for the logo key is still a TODO.

Tray icon colors reflect the push-to-talk pipeline:

- white: idle or transcription copied to clipboard,
- red: recording,
- yellow: processing audio with Whisper.

The primary StatusNotifierItem tray renders these colors directly. The legacy
AppIndicator fallback uses bundled SVG icons and writes them to
`~/.cache/voice/icons` when needed.

Settings are saved to:

```text
~/.config/voice/voice.toml
```

The current flow records from the selected microphone while push-to-talk is
held, sends captured audio to the selected Whisper model on key release, and
prints and copies non-empty transcription text to the clipboard.

## Whisper Backend Builds

Default CPU-capable build:

```sh
cargo build --release
```

Vulkan-capable GPU build, recommended as the portable Linux GPU profile:

```sh
cargo build --release --features gpu-vulkan
```

CUDA-capable GPU build, only on systems with CUDA development/link libraries:

```sh
cargo build --release --features gpu-cuda
```

All compiled GPU backends, only on systems with both Vulkan and CUDA build
requirements available:

```sh
cargo build --release --features gpu-all
```

The app defaults to `whisper_backend = "auto"` and automatically falls back to
CPU if the compiled GPU backend cannot load the model. On a normal Linux desktop
with a working Vulkan driver, prefer the `gpu-vulkan` build first. CUDA-capable
packaging still needs validation on clean systems, because development
headers/link libraries vary by distribution and driver setup.

This prototype pins `whisper-rs` to `0.13.2` because its Vulkan feature compiles
in this project. Checked newer `whisper-rs` versions `0.14.4`, `0.15.1`, and
`0.16.0` fail to compile their Vulkan module because of missing upstream FFI
symbols. We can move forward again once that binding/version is fixed or
replaced.

Local model files should be kept under `models/` or another non-git data
directory. `models/` is ignored by git.

If your desktop environment has no StatusNotifier/AppIndicator tray host, no
Linux app can force a panel icon to appear there. In that case this prototype
keeps running and opens the Settings window as a fallback.
