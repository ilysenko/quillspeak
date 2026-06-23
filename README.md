# QuillSpeak

QuillSpeak is a local-first push-to-talk voice transcription utility for Linux
desktops. Hold a trigger, speak, release, and QuillSpeak transcribes with
whisper.cpp models, then sends the final text through script, clipboard, and
optional paste actions.

Recommended GitHub repository name: `quillspeak`.

## Documentation

The GitHub Pages site lives in `docs/` and is deployed by
`.github/workflows/pages.yml`.

It covers installation, Wayland trigger commands, X11 keyboard grabs, Linux
signal fallback, output actions, configuration paths, and release automation.

Build it locally with:

```sh
scripts/build-docs.sh _site
```

## Install

Tagged releases build Debian packages automatically:

- `quillspeak`: primary package with the Vulkan whisper.cpp backend and CPU
  fallback.
- `quillspeak-cpu`: CPU-only package for simpler systems.

Runtime clipboard and paste integrations use external Linux tools:

```sh
sudo apt install wl-clipboard xclip xdotool ydotool
```

The per-shortcut speaker mute option uses PipeWire tools when enabled:

```sh
sudo apt install wireplumber pipewire-bin
```

## Run From Source

Install Debian/Ubuntu build dependencies:

```sh
sudo apt install build-essential pkg-config cmake clang libclang-dev \
  libasound2-dev libpulse-dev libpipewire-0.3-dev \
  libgtk-4-dev libadwaita-1-dev
```

Run the foreground development app:

```sh
cargo run -p quillspeak --bin quillspeak
```

Verbose QuillSpeak logs:

```sh
QUILLSPEAK_DEV_LOG=1 cargo run -p quillspeak --bin quillspeak
```

The app starts without opening a window, keeps a GTK application hold, shows a
StatusNotifierItem tray indicator, and exits through the same quit path for
tray `Quit` and Ctrl-C.

## Trigger Model

QuillSpeak supports two primary trigger paths.

On X11, `hotkey_backend = "auto"` or `"x11"` lets the app capture configured
keyboard shortcuts directly with X11 passive grabs.

On Wayland, external hotkey tools should call the running app's command mode:

```sh
quillspeak trigger Default start
quillspeak trigger Default stop
quillspeak trigger Default toggle
```

Command mode talks to:

```text
$XDG_RUNTIME_DIR/quillspeak/command.sock
```

Linux signal triggers are also available as a lower-level fallback:

```sh
pkill -USR1 -x quillspeak
pkill -USR2 -x quillspeak
```

Supported signal names are `SIGUSR1`, `SIGUSR2`, `SIGALRM`, and `SIGWINCH`.
Using the same signal for start and stop makes that signal state-aware: idle
starts recording; the next matching signal for the active shortcut stops it.

## Configuration

QuillSpeak owns its user settings and model cache:

```text
~/.config/quillspeak/config.toml
~/.local/share/quillspeak/models
~/.local/share/quillspeak/history.jsonl
```

Only the current development schema is supported. Unsupported local schemas are
discarded and replaced with defaults.

## Release Automation

Push a `v*` tag to build packages and publish a GitHub Release:

```sh
git tag v0.0.1
git push origin v0.0.1
```

The release workflow:

1. Builds `.deb` packages inside a Debian 12 container.
2. Sets `GGML_NATIVE=OFF` for portable CPU fallback code.
3. Uploads `.deb` files, `SHA256SUMS`, and `release-manifest.json`.
4. Lets the Pages workflow refresh the Downloads section from the latest
   GitHub Release metadata.

## Verification

Useful local checks:

```sh
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

If the full app build fails because `gtk4.pc`, `libadwaita-1.pc`, or related
pkg-config files are missing, install the GTK4/libadwaita development packages.
Do not rewrite the app away from GTK4/libadwaita.

## License

QuillSpeak is free software under the MIT License. See [LICENSE](LICENSE).
