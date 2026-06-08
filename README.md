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
sudo apt install build-essential pkg-config cmake clang libclang-dev libasound2-dev libpulse-dev libpipewire-0.3-dev libgtk-4-dev libadwaita-1-dev
```

Clipboard output uses small external runtime tools. They are not guaranteed to
be installed by the desktop environment itself:

```sh
sudo apt install wl-clipboard xclip
```

`wl-clipboard` provides `wl-copy` and `wl-paste` for Wayland. `xclip` is used
for X11. The app does not install these tools automatically or run package
managers itself; if a required tool is missing, it logs `clipboard copy failed`
with the package hint.

The default development build enables CPAL's native PipeWire and PulseAudio
backends. On modern Ubuntu desktops the app prefers PipeWire, falls back to
PulseAudio, and keeps ALSA as the explicit low-level debug fallback:

```sh
cargo run -p app --bin myapp
cargo run -p app --bin myapp --no-default-features --features audio-pulseaudio
cargo run -p app --no-default-features --bin myapp
```

The PipeWire feature requires `libpipewire-0.3-dev`. The final
`--no-default-features` command is an ALSA-only fallback for debugging.
The same fallback works for local validation on systems without PipeWire
development headers:

```sh
cargo check --workspace --no-default-features
cargo test --workspace --no-default-features
cargo clippy --workspace --all-targets --no-default-features -- -D warnings
```

Vulkan is the intended packaged GPU backend because users should be able to
install a future `.deb` without compiling CUDA locally. Builder machines need
Vulkan development dependencies:

```sh
sudo apt install libvulkan-dev glslc
cargo check -p app --features whisper-vulkan
cargo run -p app --features whisper-vulkan --bin myapp
```

If native PipeWire development headers are not installed, Cargo default
features must be disabled explicitly because feature flags are additive:

```sh
cargo run -p app --no-default-features --features whisper-vulkan,audio-pulseaudio --bin myapp
```

The workspace currently pins `whisper-rs = "=0.13.2"` because that version's
Vulkan feature builds here. Newer `whisper-rs` releases should be retested
before upgrading; `0.14.4`, `0.15.1`, and `0.16.0` currently fail to compile
their Vulkan wrapper against their generated `whisper-rs-sys` bindings.

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

Verbose development logs for transcription details:

```sh
MYAPP_DEV_LOG=1 cargo run -p app --bin myapp
MYAPP_DEBUG_SAVE_AUDIO=1 MYAPP_DEV_LOG=1 cargo run -p app --features whisper-vulkan --bin myapp
MYAPP_DEV_LOG=1 cargo run -p daemon --bin myapp-daemon
```

`MYAPP_DEV_LOG=1` enables debug logs for MyApp crates while keeping dependency
crates at info level. A global `RUST_LOG=debug` is intentionally noisy and will
include low-level PulseAudio/zbus internals. If you set `RUST_LOG` explicitly,
use `RUST_LOG=info,pulseaudio::client::reactor=off` to keep the known
PulseAudio reactor disconnect noise hidden.

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

The app keeps the input stream stopped while idle. It starts the CPAL stream
only while recording and pauses it again on stop, so idle logs stay quiet and
the microphone is not held open unnecessarily. Capture requests a bounded input
buffer when the backend reports one, and falls back to the backend default if
that fixed buffer cannot be opened.

On stop, the app converts captured audio to 16 kHz mono `f32` with `rubato`,
runs `whisper-rs`/whisper.cpp on the model selected by the active shortcut, and
logs recognized text:

```text
recognized text shortcut_id=default model_id=tiny language=auto text="..."
```

Captures with too little usable source or prepared audio are reported as
`Skipped` transcription results. They do not load Whisper, do not clear a good
cached model because of an empty transcript, and do not run output actions. A
very low callback count is logged as a capture diagnostic instead of dropping
otherwise valid audio.

`MYAPP_DEV_LOG=1` prints the full transcription debug structure: shortcut name,
model path, compute backend, input device, capture duration, source sample
rate, audio RMS/peak, Whisper sample count, inference time, and segments. Info
logs also include the real audio duration, shortcut wall-clock duration, startup
latency, first accepted audio callback latency, callback count, and discarded
stale callback/sample counts. Empty recognized text also logs segment count and
audio RMS/peak to help distinguish a wrong or silent microphone from a
transcription problem.

Set `MYAPP_DEBUG_SAVE_AUDIO=1` to write source WAV, the exact 16 kHz mono WAV
sent to Whisper, and TOML metadata under `/tmp/myapp-audio-debug`. Set it to a
directory path to choose a different output directory. Skipped captures also
write debug audio when this flag is set, but that audio is diagnostic only and
is not sent to Whisper.

To test the Whisper invocation without GTK, D-Bus, or microphone capture, run
the ignored debug WAV test against a saved 16 kHz mono debug file:

```sh
MYAPP_TEST_WHISPER_MODEL=/path/to/ggml-model.bin \
MYAPP_TEST_WHISPER_WAV=/tmp/myapp-audio-debug/default-...-whisper-16k-mono.wav \
cargo test -p app debug_whisper_wav_transcribes_with_app_params -- --ignored --nocapture
```

There is also an ignored cache regression for repeated transcription after a
short skipped capture:

```sh
MYAPP_TEST_WHISPER_MODEL=/path/to/ggml-model.bin \
MYAPP_TEST_WHISPER_WAV=/tmp/myapp-audio-debug/default-...-whisper-16k-mono.wav \
cargo test -p app debug_whisper_cached_repeated_transcription_survives_short_skip -- --ignored --nocapture
```

Auto language mode is used for normal transcription, but the app does not set
whisper.cpp's `detect_language` flag because that mode exits after language
detection instead of producing transcript segments.

Models must be downloaded in Settings > Models before they can be used. If a
shortcut points to a model that is not ready, recording stops with a clear log
error and no hidden download starts during the hotkey flow.

Output actions are executed after successful transcription. The app can copy
recognized text to the system clipboard and can run a configured script on a
background worker thread. Clipboard writes use external Linux tools so Wayland
and X11 clients see the same selection: install `wl-clipboard` for Wayland or
`xclip` for X11. The app verifies clipboard writes with `wl-paste` or
`xclip -out` before logging success; if copy or verification fails, the app logs
the failure instead of reporting a successful copy.

Copy-to-clipboard intentionally leaves the recognized text in the clipboard. The
app does not restore the previous clipboard content after a successful copy.
Tools that paste text into another app sometimes restore the clipboard because
they use it only as a temporary paste transport; MyApp's current output action
is a real copy action, so restoring the old value would erase the transcript the
user asked to copy.

## Configuration

The main app owns user settings and writes TOML to:

```text
~/.config/myapp/config.toml
```

Default config:

```toml
schema_version = 5

[general]
mode = "push_to_talk"
hotkey_backend = "auto"
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
keep_model_loaded = true
default_input = { type = "system_default" }
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

The daemon does not read this config directly. The app sends the current
shortcut runtime config to the daemon over D-Bus. That runtime config is
daemon-effective: shortcuts are enabled only when the resolved backend is
`daemon`; for `disabled` or `x11`, the app sends disabled bindings so the daemon
clears any active watcher. `keep_model_loaded = true` keeps only the last used
Whisper model context in memory to speed up repeated transcription; `false`
loads and drops the model for every transcription. The daemon stores an accepted
last-known cache at
`~/.config/myapp-input-daemon/shortcut-cache.toml` so it can start before the app
and still know the last configured shortcuts. The daemon validates that runtime
cache schema before applying it; stale or invalid cache files are ignored and
the app config wins the next time the app is available. During development only
the current schema is supported. If the app sees an older local config schema,
it replaces it with a fresh default config instead of migrating it.

Settings has `General`, `Models`, `Default`, and `Add New` pages. `Models`
manages downloaded whisper.cpp ggml models under `~/.local/share/myapp/models`.
Shortcut pages choose only ready models, or `Default` to inherit the general
model. Model downloads use `*.part`, progress updates, SHA-1 verification, and
atomic rename.

Each shortcut profile has its own trigger, model override, language override,
and output pipeline. Triggers can be keyboard shortcuts or Linux signals.
Linux signal triggers accept `SIGUSR1` and `SIGUSR2`; using the same signal for
start and stop makes that signal a toggle. The Handy-compatible default when
switching a shortcut to Linux signals is `SIGUSR2` for both start and stop.

Signal examples:

```sh
pkill -USR2 -x myapp
pkill -USR1 -x myapp
```

Output can copy the transcript to clipboard, run a script with the transcript
as its first argument, and optionally copy successful script stdout to
clipboard.

If transcription logs recognized text but another application cannot paste it,
check the MyApp logs for `Copied text to clipboard` or `clipboard copy
failed` messages. On Wayland, check manually with `wl-paste --no-newline`; on
X11, use `xclip -selection clipboard -out`. If the log says `wl-copy not found`
or `wl-paste not found`, install `wl-clipboard`; if it says `xclip not found`,
install `xclip`.

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
- Flatpak packaging for the main app,
- `.deb` packaging for the optional daemon.
