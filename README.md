# Glance

A fast, simple photo and video viewer for Windows.

**[Download Glance](https://github.com/LavaAKA/glance/releases/latest/download/Glance-Setup.exe)** · [Website](https://lavaaka.github.io/glance/)

<img src="docs/icon.png" width="64" alt="Glance icon">

## Install

Download `Glance-Setup.exe` from [Releases](https://github.com/LavaAKA/glance/releases) and run it.
Windows SmartScreen may warn because the app isn't code-signed: click **More info → Run anyway**.
Uninstall from **Settings → Apps → Installed apps → Glance**.

## Build from source

Needs Rust with the `x86_64-pc-windows-gnu` toolchain (for `windres`).

```powershell
cargo build --release              # target\release\glance.exe
cargo test --release
cd setup; cargo build --release    # setup\target\release\glance-setup.exe (carries glance.exe inside)
```

## Layout

- `src/` – the app (egui/eframe)
- `setup/` – the installer, `Glance-Setup.exe`
- `installer/` – the uninstall script it installs, and a script-based installer for local builds
- `docs/` – the website (GitHub Pages)
