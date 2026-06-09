# MyApp Linux Voice Prototype

Minimal Rust Linux desktop prototype for a future push-to-talk transcription
app.

The app is a normal foreground process while developing. It starts with no
visible window, keeps running through a tray/top-bar StatusNotifierItem, and can
be interrupted with Ctrl-C from the terminal. Later, the same runtime can be
wrapped in desktop autostart or a user service without changing the tray or UI.

## Workspace

- `crates/app` builds `myapp`, the GTK4/libadwaita desktop app.
- `crates/shared` contains config, model catalog, shortcut parsing, language,
  output, and small shared constants.

There is no in-repo input daemon. X11 hotkeys are handled by the app itself.
Wayland hotkeys are expected to come from an external utility that sends Linux
signals to the `myapp` process.

## System Dependencies

On Debian/Ubuntu-style systems install the desktop, audio, and whisper.cpp build
dependencies before building the app:

```sh
sudo apt install build-essential pkg-config cmake clang libclang-dev libasound2-dev libpulse-dev libpipewire-0.3-dev libgtk-4-dev libadwaita-1-dev
```

Clipboard output and paste shortcuts use small external runtime tools:

```sh
sudo apt install wl-clipboard xclip xdotool ydotool
```

`wl-clipboard` provides `wl-copy` and `wl-paste` for Wayland. `xclip` is used
for X11. `xdotool` sends paste shortcuts on X11; `ydotool` sends paste
shortcuts on Wayland. The app does not install these tools automatically; if a
required tool is missing, it logs the failing output action with the package
hint.

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

Vulkan is the intended packaged GPU backend because users should be able to
install a future `.deb` without compiling CUDA locally:

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
before upgrading.

The workspace declares `rust-version = "1.92"` in the root `Cargo.toml`.

## Build And Run

Run the app in foreground development mode:

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

Verbose development logs:

```sh
MYAPP_DEV_LOG=1 cargo run -p app --bin myapp
MYAPP_DEBUG_SAVE_AUDIO=1 MYAPP_DEV_LOG=1 cargo run -p app --features whisper-vulkan --bin myapp
```

`MYAPP_DEV_LOG=1` enables debug logs for MyApp crates while keeping dependency
crates at info level. A global `RUST_LOG=debug` is intentionally noisy. If you
set `RUST_LOG` explicitly, use
`RUST_LOG=info,pulseaudio::client::reactor=off` to keep known PulseAudio
reactor disconnect noise hidden.

The tray icon reflects the recording phase:

- white: idle,
- red: recording,
- orange: processing/transcription.

## Hotkeys

The app has two supported trigger paths:

- X11: `hotkey_backend = "auto"` or `"x11"` lets the app capture configured
  keyboard shortcuts directly with X11 passive grabs; signal shortcuts are also
  available.
- Wayland: shortcut profiles use Linux signal triggers and an external hotkey
  utility such as `swhkd` sends Linux signals to `myapp`.

Settings follows the same display split. On X11 it shows both keyboard shortcut
and Linux signal trigger controls. On Wayland, or in mixed Wayland/X11 sessions,
it shows only signal trigger controls so saved shortcuts match the trigger path
the app can actually receive.

Backend values:

- `auto`: X11 uses the app's X11 backend; Wayland resolves to disabled so
  external signal tooling can drive signal shortcuts.
- `disabled`: no app-side global keyboard capture; tray manual actions and
  Linux signal triggers still work.
- `x11`: force app-side X11 capture.

Linux signal triggers are text fields. Common aliases such as `usr1`, `User 1`,
and `SIGUSR1` resolve to `SIGUSR1`; arbitrary non-empty text is saved, and if it
cannot be resolved to a Linux signal at runtime the app logs a diagnostic and
skips that binding. Using the same signal for start and stop makes that signal a
state-aware start/stop trigger: idle starts, the active shortcut stops. Each
received signal is handled once. `SIGUSR1` and `SIGUSR2` are always registered
as safe guard signals, so an unmatched external signal is logged at debug level
and ignored instead of terminating the app.

Signal examples:

```sh
pkill -USR1 -x myapp
pkill -USR2 -x myapp
```

Example `~/.config/swhkd/swhkdrc` entries for Wayland:

```conf
ctrl + space
    pkill -USR1 -x myapp

ctrl + @space
    pkill -USR1 -x myapp

ctrl + shift + space
    pkill -USR2 -x myapp

ctrl + shift + @space
    pkill -USR2 -x myapp
```

If a shortcut uses the same start and stop signal, the first received signal
starts recording and the next received signal for the active shortcut stops it.
If a shortcut uses distinct start and stop signals, the external utility must
send the matching signal for each edge.

For a quick manual check while the app is running, send the default start and
stop pair:

```sh
pkill -USR1 -x myapp
pkill -USR2 -x myapp
```

For a same-signal shortcut, send the same command twice. The first signal should
start recording, and the second should stop the active recording. Unmatched guard
signals are reported in debug logs and ignored; the app should continue running.

## Audio And Transcription

The app records audio with CPAL from the General page `Default input` setting.
`System Default` is always available and resolves to the current Linux default
input device at recording time. Audio capture is independent of X11/Wayland.

The app keeps the input stream stopped while idle. It starts the CPAL stream
only while recording and pauses it again on stop, so idle logs stay quiet and
the microphone is not held open unnecessarily.

Audio capture uses a short callback ring buffer and drains it on the
`myapp-audio-capture` worker into a bounded session buffer. The callback does
not preallocate storage for the full maximum recording duration.

On stop, the app converts captured audio to 16 kHz mono `f32` with `rubato`,
runs `whisper-rs`/whisper.cpp on the model selected by the active shortcut, and
logs recognized text:

```text
recognized text shortcut_id=default model_id=tiny language=auto text="..."
```

Captures with too little usable source or prepared audio are reported as
`Skipped` transcription results. They do not load Whisper and do not run output
actions.

`MYAPP_DEV_LOG=1` prints the full transcription debug structure: shortcut name,
model path, compute backend, input device, capture duration, source sample
rate, audio RMS/peak, Whisper sample count, inference time, and segments.

Set `MYAPP_DEBUG_SAVE_AUDIO=1` to write source WAV, the exact 16 kHz mono WAV
sent to Whisper, and TOML metadata under `/tmp/myapp-audio-debug`. Set it to a
directory path to choose a different output directory.

To test Whisper without GTK or microphone capture, run the ignored debug WAV
test against a saved 16 kHz mono debug file:

```sh
MYAPP_TEST_WHISPER_MODEL=/path/to/ggml-model.bin \
MYAPP_TEST_WHISPER_WAV=/tmp/myapp-audio-debug/default-...-whisper-16k-mono.wav \
cargo test -p app debug_whisper_wav_transcribes_with_app_params -- --ignored --nocapture
```

Auto language mode is used for normal transcription, but the app does not set
whisper.cpp's `detect_language` flag because that mode exits after language
detection instead of producing transcript segments.

Models must be downloaded in Settings > Models before they can be used. If a
shortcut points to a model that is not ready, recording stops with a clear log
error and no hidden download starts during the hotkey flow.

Model downloads run on named worker threads, can be canceled from the UI, and
are canceled during app quit. On startup the model store removes orphan
catalog `.part` files left behind by interrupted downloads before reconciling
ready inventory.

## Output

Output uses one simple pipeline:

```text
transcript -> optional script transform -> final text -> optional clipboard copy/transport -> optional paste shortcut
```

If a script is enabled, its stdout is the final text; the original transcript is
not copied as a fallback. The script path is executed directly with the
transcript as its first argument and stdin closed. When clipboard copy or paste
is enabled, stdout must be UTF-8 and becomes the final text; when neither is
enabled, stdout is ignored. A 30-second script timeout, spawn failure, or
nonzero exit stops the output pipeline without falling back to the original
transcript.

Clipboard writes use external Linux tools so Wayland and X11 clients see the
same selection. The app verifies clipboard writes with `wl-paste` or
`xclip -out` before logging success.

Copy-to-clipboard intentionally leaves the final text in the clipboard. If
`Paste from clipboard` is enabled, the app first writes and verifies the final
text in the external clipboard, then sends the configured paste shortcut:
`Ctrl+V`, `Ctrl+Shift+V`, or custom `xdotool` / `ydotool` key syntax. Direct
text insertion without clipboard transport is not implemented.

If transcription logs recognized text but another application cannot paste it,
check the MyApp logs for `Copied text to clipboard` or `clipboard copy failed`
messages. On Wayland, check manually with `wl-paste --no-newline`; on X11, use
`xclip -selection clipboard -out`.

## Configuration

The app owns user settings and writes TOML to:

```text
~/.config/myapp/config.toml
```

Generated defaults are display-aware. On X11-capable sessions the app creates
the keyboard default plus a signal shortcut:

```toml
schema_version = 11

[general]
mode = "push_to_talk"
hotkey_backend = "auto"
default_model_id = "large-v3-turbo-q5_0"
default_language = "auto"
compute_backend = "auto"
keep_model_loaded = true
default_input = { type = "system_default" }
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

On Wayland or mixed Wayland/X11 sessions, the generated default shortcut uses
the same `SIGUSR1` start and `SIGUSR2` stop signal trigger, and Settings shows
only signal trigger controls.

If an existing draft contains keyboard triggers while the current session is not
keyboard-capable, Settings converts those draft triggers to the default Linux
signal pair before rendering. If that would create duplicate enabled signal
bindings, the first profile keeps the binding and later duplicates are left
disabled until you configure unique signals and turn them back on. The
conversion is still saved only when you press `Save`.

During development only the current schema is supported. If the app sees an
older local config schema, including v10, it replaces it with a fresh default
config instead of migrating it.

Supported `compute_backend` values are `auto`, `cpu`, `vulkan`, `cuda`, and
`rocm`. OpenVINO is not currently supported by this whisper-rs integration and
is not offered in Settings.

Settings has `General`, `Models`, one page per shortcut profile, and `Add New`
pages. `Models` manages downloaded whisper.cpp ggml models under
`~/.local/share/myapp/models`. Shortcut pages choose only ready models, or
`Default` to inherit the general model.

Each shortcut profile has its own trigger, model override, language override,
and output pipeline. Triggers can be keyboard shortcuts or Linux signals.

TOML output examples:

```toml
default_output = { copy_to_clipboard = true, paste_from_clipboard = false, paste_shortcut = "ctrl_v" }
output = { type = "custom", copy_to_clipboard = false, paste_from_clipboard = true, paste_shortcut = "ctrl_shift_v", script = { path = "/home/igor/myapp-polite-english.sh" } }
output = { type = "custom", copy_to_clipboard = false, paste_from_clipboard = true, paste_shortcut = "custom", paste_custom_x11 = "ctrl+v", paste_custom_wayland = "29:1 47:1 47:0 29:0" }
```

## Verification

Useful checks:

```sh
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

`cargo test --workspace` is expected to pass with Cargo's default parallel test
runner; use `-- --test-threads=1` only as a debugging aid.

If the full app build fails because `gtk4.pc`, `libadwaita-1.pc`, or related
pkg-config files are missing, install the GTK4/libadwaita development packages
instead of rewriting the app away from GTK4/libadwaita.

## Future Work

The prototype intentionally leaves these unimplemented:

- XDG GlobalShortcuts portal support,
- production-grade audio buffering/resampling,
- streaming/VAD transcription,
- direct text insertion without clipboard transport,
- Flatpak packaging for the main app,
- `.deb` packaging for the main app.
