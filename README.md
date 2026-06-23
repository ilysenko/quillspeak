# QuillSpeak

QuillSpeak is a local-first push-to-talk voice transcription app for Linux
desktops.

Hold a trigger, speak, release, and QuillSpeak transcribes your voice with a
local whisper.cpp model. The final text can be copied to the clipboard, passed
through a script, or pasted into the focused app through external Linux tools.

[Documentation](https://ilysenko.github.io/quillspeak/) |
[Releases](https://github.com/ilysenko/quillspeak/releases) |
[Issues](https://github.com/ilysenko/quillspeak/issues) |
[License](LICENSE)

## What It Is For

QuillSpeak is for local voice typing on Linux:

- push-to-talk dictation,
- quick transcription into text fields,
- per-shortcut model and language settings,
- scriptable text post-processing,
- clipboard copy and optional paste shortcuts.

It is not a cloud transcription service. Audio is recorded locally and
transcribed locally with downloaded whisper.cpp models.

## Where It Works

QuillSpeak is built for Linux desktop sessions with GTK4 and libadwaita.

- X11: the app can capture configured global keyboard shortcuts directly.
- Wayland: use compositor keybindings or an external hotkey tool to call
  `quillspeak trigger`.
- Mixed Wayland/X11 sessions: QuillSpeak treats keyboard hotkeys as
  Wayland-style external triggers.

The app uses CPAL for microphone capture and can use PipeWire or PulseAudio
audio hosts depending on the system.

## Install

Download Debian/Ubuntu packages from
[GitHub Releases](https://github.com/ilysenko/quillspeak/releases).

- `quillspeak`: primary package, built with Vulkan whisper.cpp support and CPU
  fallback.
- `quillspeak-cpu`: CPU-only package for systems where a simpler runtime is
  preferred.

Runtime clipboard and paste behavior may need these tools:

```sh
sudo apt install wl-clipboard xclip xdotool ydotool
```

Speaker mute during recording, when enabled, uses PipeWire tools:

```sh
sudo apt install wireplumber pipewire-bin
```

## Basic Usage

Start the app:

```sh
quillspeak
```

QuillSpeak starts in the background with a tray indicator. Open
**Show Settings** from the tray menu, download a model, configure a shortcut,
then hold the trigger to record.

Tray state:

- white icon: idle,
- red icon: recording,
- orange icon: processing/transcribing.

## X11 And Wayland Triggers

On X11, QuillSpeak can use app-owned keyboard shortcuts. The default keyboard
shortcut is configured in the app settings.

On Wayland, regular desktop apps cannot capture global keyboard shortcuts
directly. Use compositor keybindings, `swhkd`, or another hotkey tool to call
the running app:

```sh
quillspeak trigger Default start
quillspeak trigger Default stop
quillspeak trigger Default toggle
```

The trigger command talks to:

```text
$XDG_RUNTIME_DIR/quillspeak/command.sock
```

Linux signals are also available as a lower-level fallback:

```sh
pkill -USR1 -x quillspeak
pkill -USR2 -x quillspeak
```

Supported signal names are `SIGUSR1`, `SIGUSR2`, `SIGALRM`, and `SIGWINCH`.

## External Tools

QuillSpeak intentionally uses normal Linux desktop tools for clipboard and
paste integration:

- `wl-copy` / `wl-paste` from `wl-clipboard`: Wayland clipboard transport.
- `xclip`: X11 clipboard transport.
- `xdotool`: X11 paste shortcuts.
- `ydotool`: Wayland paste shortcuts; may require its own daemon and
  permissions.
- `wpctl` / PipeWire tools: optional speaker mute while recording.
- `swhkd` or compositor keybindings: external Wayland hotkey integration.

## Limitations

- Linux desktop only; Windows and macOS are not supported.
- Wayland global shortcut capture is external to QuillSpeak.
- Direct text insertion is not implemented; paste uses clipboard transport.
- The main app should not be run with `sudo`.
- A ready downloaded whisper.cpp model is required before transcription.
- Old development config schemas may be replaced with current defaults.

## Files And State

QuillSpeak stores user-owned state under standard XDG paths:

```text
~/.config/quillspeak/config.toml
~/.local/share/quillspeak/models
~/.local/share/quillspeak/history.jsonl
$XDG_RUNTIME_DIR/quillspeak/command.sock
```

## Build From Source

Install Debian/Ubuntu build dependencies:

```sh
sudo apt install build-essential pkg-config cmake clang libclang-dev \
  libasound2-dev libpulse-dev libpipewire-0.3-dev \
  libgtk-4-dev libadwaita-1-dev
```

Run the app from the workspace:

```sh
cargo run -p quillspeak --bin quillspeak
```

Verbose QuillSpeak logs:

```sh
QUILLSPEAK_DEV_LOG=1 cargo run -p quillspeak --bin quillspeak
```

Useful local checks:

```sh
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

## Project Links

- Documentation: https://ilysenko.github.io/quillspeak/
- Releases: https://github.com/ilysenko/quillspeak/releases
- Repository: https://github.com/ilysenko/quillspeak
- Issues: https://github.com/ilysenko/quillspeak/issues

## License

QuillSpeak is free software under the MIT License. See [LICENSE](LICENSE).
