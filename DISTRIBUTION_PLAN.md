# Distribution Plan

Research-backed plan for packaging and distributing MyApp on Linux.
Status: plan only — nothing below is implemented yet (researched 2026-06).

## Summary

- Primary channel: `.deb` built with `cargo-deb` inside a `debian:12`
  container, published on GitHub Releases. Zero runtime behavior changes
  needed — external tools, the command socket, signals, X11 grabs, and the
  tray all work as designed in a native package.
- GPU strategy: single static **Vulkan** build as the universal default
  (covers NVIDIA, AMD, Intel through their Vulkan drivers; automatic CPU
  fallback). CUDA as an optional separate `.deb` variant later. ROCm and
  "all backends in one artifact" are explicitly deferred (see Appendix A).
- Flatpak/Flathub is a v2 goal gated on a portal refactor (paste injection,
  Wayland hotkeys). Snap is skipped. AppImage is optional and low priority.

Key version facts discovered during research:

- `libadwaita = "0.9"` with feature `v1_2` sets the floor at libadwaita >= 1.2.
  Ubuntu 22.04 ships 1.1 and is therefore **not supported** by native
  packages. Debian 12 (glibc 2.36, libadwaita 1.2.2) is the build floor;
  one `.deb` built there covers Debian 12/13 and Ubuntu 24.04+.
- The pinned `whisper-rs 0.13.2` / `whisper-rs-sys 0.11.1` vendors
  whisper.cpp **1.7.1**, which predates ggml dynamic backend loading
  (`GGML_BACKEND_DL`) entirely, and its build.rs hardcodes
  `BUILD_SHARED_LIBS=OFF`. Multi-backend-in-one-binary is impossible on this
  pin. whisper-rs development moved to Codeberg
  (<https://codeberg.org/tazz4843/whisper-rs>); latest is 0.16.0, vendoring
  whisper.cpp 1.8.3.
- Static ggml CPU code compiles for the build machine's ISA by default.
  Distributable builds must set `GGML_NATIVE=OFF` (Handy shipped AVX2
  crashes by missing this — cjpais/Handy#91).

## Phase 1 — v1: Vulkan `.deb` on GitHub Releases

### 1.1 Upgrade whisper-rs

- Bump to `whisper-rs = "=0.16.0"` / `whisper-rs-sys = "=0.15.0"`
  (whisper.cpp 1.8.3) in the workspace `Cargo.toml`.
- Re-verify the `vulkan` feature builds and transcribes locally (the README
  pin note exists because Vulkan broke on some whisper-rs releases; retest).
- Drop the direct `whisper-rs-sys` dependency from `crates/app` if it is no
  longer needed after the upgrade.

### 1.2 Portable compilation flags

- Set `GGML_NATIVE=OFF` for release/package builds so the CPU path does not
  use build-host ISA extensions. whisper-rs-sys forwards `WHISPER_*` /
  `CMAKE_*` env vars to cmake; wire this into the release build script or CI
  env rather than developer builds.
- Accept the CPU-path speed loss; Vulkan is the primary inference path.

### 1.3 Desktop integration assets (currently missing entirely)

- `assets/myapp.desktop` — `Type=Application`, `Categories=Utility;`,
  `Icon=myapp`, no window on launch is fine for a tray app.
- Icons under `assets/icons/hicolor/` (at least scalable SVG + 256x256 PNG).
- AppStream metainfo `assets/myapp.metainfo.xml` (required later for
  Flathub, improves GNOME Software / KDE Discover listing for the deb too).
- Optional autostart: ship the same `.desktop` for
  `/etc/xdg/autostart/` and/or a systemd user unit for
  `/usr/lib/systemd/user/myapp.service` as plain package assets. Do not
  auto-enable; document `systemctl --user enable myapp`.

### 1.4 cargo-deb metadata

Add to `crates/app/Cargo.toml`:

```toml
[package.metadata.deb]
name = "myapp"
depends = "$auto"
recommends = "wl-clipboard, xclip"
suggests = "xdotool, ydotool"
section = "sound"
features = ["whisper-vulkan"]
assets = [
    ["target/release/myapp", "usr/bin/", "755"],
    ["../../assets/myapp.desktop", "usr/share/applications/", "644"],
    ["../../assets/icons/hicolor/scalable/apps/myapp.svg",
     "usr/share/icons/hicolor/scalable/apps/", "644"],
    ["../../assets/myapp.metainfo.xml", "usr/share/metainfo/", "644"],
]
```

Notes:

- `$auto` runs `dpkg-shlibdeps` and emits correct `Depends`
  (libgtk-4-1, libadwaita-1-0 (>= 1.2), libvulkan1, libasound2, ...).
  It is only correct when the build runs on the oldest supported distro —
  hence the debian:12 container requirement.
- `swhkd` is not in Debian/Ubuntu archives; document it in README instead of
  referencing it in package relationships.
- CUDA later: add `[package.metadata.deb.variants.cuda]` overriding
  `name = "myapp-cuda"`, `features = ["whisper-cuda"]`, plus
  `conflicts`/`provides` against `myapp`. Built with
  `cargo deb -p app --variant=cuda`.

### 1.5 CI release pipeline (GitHub Actions)

- `runs-on: ubuntu-latest` with `container: debian:12`.
- Install: `build-essential pkg-config cmake clang libclang-dev
  libasound2-dev libpulse-dev libpipewire-0.3-dev libgtk-4-dev
  libadwaita-1-dev libvulkan-dev glslc dpkg-dev`.
- Steps: rust toolchain → `Swatinem/rust-cache` (the whisper.cpp cmake build
  dominates compile time) → `cargo deb -p app` with `GGML_NATIVE=OFF` →
  upload `.deb` + a plain tarball (binary, .desktop, icon, metainfo,
  install.sh) via `softprops/action-gh-release` on tag push.
- Future CUDA leg: matrix `backend: [vulkan, cuda]`; the CUDA job uses an
  `nvidia/cuda:12.x-devel-ubuntu22.04`-class container or
  `Jimver/cuda-toolkit` (no GPU needed to compile). Do not block v1 on this.

### 1.6 AUR

- Hand-write a source `PKGBUILD` (Arch convention; generators are not used
  in practice): `cargo fetch --locked` in `prepare()`,
  `cargo build --frozen --release --features whisper-vulkan` in `build()`.
- `depends=(gtk4 libadwaita pipewire vulkan-icd-loader)`,
  `optdepends=(wl-clipboard xclip xdotool ydotool swhkd)` — all available on
  Arch, unlike Debian.
- Optionally a second `myapp-bin` AUR package repacking the GitHub release.

### 1.7 README / docs updates

- Document supported distros (Debian 12+, Ubuntu 24.04+; Ubuntu 22.04
  unsupported due to libadwaita 1.2 floor).
- Document GPU expectations: Vulkan via mesa (AMD/Intel) or the NVIDIA
  proprietary driver; CPU fallback otherwise.

## Phase 2 — v1.x: update channel and CUDA variant

- Self-hosted apt repository (aptly or reprepro, GPG-signed, served from
  GitHub Pages or S3) so users get updates via `apt upgrade`. GitHub
  Releases alone has no update story.
- Ship the `myapp-cuda` deb variant if NVIDIA users ask for more speed than
  Vulkan provides (expected gap roughly 1.2–2x on NVIDIA; the CUDA runtime
  payload is ~370 MB compressed — keep it out of the default artifact).

## Phase 3 — v2: portal refactor, then Flatpak (and nearly-free Snap)

Flathub is the largest discovery channel, but the current architecture
relies on spawning host binaries (wl-copy/xclip/xdotool/ydotool), which in a
Flatpak requires `--talk-name=org.freedesktop.Flatpak` — a sandbox escape
the Flathub linter flags and only grants as a temporary, justified
exception. Plan for portals instead (this is how Speech Note and Speed of
Sound got accepted):

- Paste injection: XDG RemoteDesktop portal (works on X11 and Wayland,
  inside and outside the sandbox) instead of xdotool/ydotool.
- Clipboard: GDK clipboard inside the sandbox (focused-window writes) or
  the RemoteDesktop clipboard extension.
- Wayland hotkeys: XDG GlobalShortcuts portal (KDE Plasma, GNOME 48+) in
  addition to the existing signal/socket triggers.

What already works in a Flatpak with no code changes (verified in research):

- Tray: `--talk-name=org.kde.StatusNotifierWatcher`.
- Microphone: `--socket=pulseaudio` (pipewire-pulse compatible).
- Vulkan: `--device=dri` + runtime Mesa; ship the Vulkan backend only
  (CUDA on Flathub requires separate addon flatpaks — out of scope).
- Command socket: `--filesystem=xdg-run/myapp:create` exposes
  `$XDG_RUNTIME_DIR/myapp` at the same path to host and sandbox; the host
  needs a tiny `myapp trigger` shim (or socat) since the binary lives in
  the sandbox.
- Host-side `pkill -USR1` still reaches the sandboxed process (one-way PID
  namespace isolation).

Build mechanics: `org.gnome.Platform` runtime + rust-stable SDK extension;
`flatpak-cargo-generator.py` turns `Cargo.lock` into offline sources.
Verified: whisper-rs-sys ships the whole whisper.cpp tree inside the
crates.io package with no cmake FetchContent, so Flathub's offline build
works. Vulkan shader compilation may need glslc from the SDK vulkan tools
extension.

Snap: skip until the portal refactor exists; afterwards a strictly-confined
snap is nearly free. AppImage: optional; GTK4 bundling tooling
(linuxdeploy-plugin-gtk) is effectively unmaintained — only attempt with
the sharun-style approach if there is demand.

## Appendix A — deferred: all GPU backends in one artifact

True single-package multi-backend (CPU variants + Vulkan + CUDA + ROCm,
runtime-selected) is what ollama does and ggml supports via
`GGML_BACKEND_DL=ON + BUILD_SHARED_LIBS=ON + GGML_CPU_ALL_VARIANTS=ON`:
each backend becomes a `libggml-<name>.so` plugin, and
`ggml_backend_load_all()` (exposed raw in whisper-rs-sys >= 0.15, must be
called by the app since whisper.cpp 1.7.6) dlopens whatever is present,
scoring CPU variants by microarch and silently skipping GPU plugins whose
driver is missing.

Why it is deferred:

- whisper-rs-sys build.rs hardcodes static linking; this path requires a
  fork or `[patch.crates-io]` of build.rs, shipping `libwhisper.so` +
  `libggml*.so` with `$ORIGIN` rpaths, and calling the loader at startup.
- Plugins must be built from the exact same ggml commit as the host
  (`GGML_BACKEND_API_VERSION` check at dlopen).
- Payload: CUDA runtime ~370 MB compressed, ROCm ~1 GB and gfx-arch
  specific. Even llama.cpp's official releases split per backend instead of
  shipping one bundle.
- Ecosystem precedent (Handy, Vibe) is a single static Vulkan build.

Revisit only if per-backend deb variants prove insufficient.

## Appendix B — research sources

- whisper.cpp dynamic backend loading: ggml-org/llama.cpp#10469 (DL),
  ggml-org/llama.cpp#10626 (CPU variants), ggml-org/whisper.cpp#3196
  (app must call `ggml_backend_load_all`; reliable from v1.7.6).
- whisper-rs continuation: <https://codeberg.org/tazz4843/whisper-rs>
  (GitHub repo archived 2025-07).
- Flathub policy on host command execution: flatpak-builder-lint
  `finish-args-flatpak-spawn-access`, docs.flathub.org linter/requirements.
- Peer apps: cjpais/Handy (Vulkan-only static, signals on Wayland —
  same trigger architecture as MyApp), thewh1teagle/vibe (deb/rpm,
  Vulkan), mkiol SpeechNote (Flathub, portals, CUDA as addon flatpaks),
  zugaldia/speedofsound (portal-native GTK4).
- Tooling: kornelski/cargo-deb (variants, `$auto` depends, systemd units),
  flatpak/flatpak-builder-tools (cargo generator). cargo-dist does not
  produce debs; cargo-packager is the multi-format alternative if AppImage
  is ever wanted.
