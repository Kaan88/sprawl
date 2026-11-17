# Sprawl

A disk space analyser for Windows in **Rust + WinUI 3** (Windows App SDK 1.8), modelled on
[WizTree](https://diskanalyzer.com/): a 1:1 feature match, with a more modern look.

![Sprawl](docs/screenshots/sprawl.png)

## Install

Download `Sprawl-Setup-<version>.exe` from the [latest release](https://github.com/Kaan88/sprawl/releases/latest)
and run it (an Inno Setup installer). It installs for you only, in `%LOCALAPPDATA%\Programs\Sprawl`, with a Start menu
entry and an entry in Settings > Apps for uninstalling. No administrator rights are needed.
Sprawl needs the Windows App Runtime 1.8; if it's missing, Windows offers to install it the
first time Sprawl starts.

Sprawl looks for a new version once a day and asks before updating (Help > Check for Updates;
the switch below it turns the daily check off). Updates are signed: a copy only accepts a release
whose file list carries Sprawl's Ed25519 signature and whose files match it. What changed in
each version is in [CHANGELOG.md](CHANGELOG.md).

## Layout

```
crates/
  sprawl-core/   UI-free logic: scanning, folder totals, file-type stats, squarified treemap layout (+ tests)
  sprawl-advisor/ the Advisor: rules, history, clean-up, AI providers, the MCP server
  sprawl-install/ update plumbing (signed release manifest, swapping files in) and the Apps entry
  sprawl-release/ builds, signs and publishes a release
  sprawl/        the WinUI 3 app (XAML loaded at run time, treemap painted into a WriteableBitmap)
    src/main.rs      start-up: arguments, window, event wiring
    src/app.rs       Windows App Runtime bootstrap, the composed XAML Application, COM interop helpers
    src/state.rs     window state + shared helpers; scanning and the header card
    src/tables.rs    tree view, file view and file-type table
    src/treemap.rs   treemap rendering and mouse handling (pixels come from paint.rs)
    src/commands.rs  menu/button commands
    src/ui.rs        XAML markup for the window and table rows
    src/format.rs, paint.rs, win32.rs   formatting, treemap painting + PNG, plain Win32 calls
    src/bindings.rs  generated WinUI projection - do not edit by hand
  bindgen/         regenerates bindings.rs from the SDK .winmd files (types.txt lists what is projected)
tools/
  setup.ps1        downloads the Windows App SDK NuGet packages into sdk/
  fetch-pkg.ps1    downloads + unzips one NuGet package (used by setup.ps1)
  probe.ps1        read-only check of the build environment (runtime, Windows Kits, VS tools)
installer/
  sprawl.iss       the Inno Setup 7 script for Sprawl-Setup-<version>.exe
docs/
  screenshots/             Sprawl screenshots
```

## Building

Requirements: Rust (MSVC toolchain), Visual Studio Build Tools, and the
[Windows App Runtime 1.8](https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads) installed.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\setup.ps1   # once: fetch SDK packages into sdk\
cargo build --release
.\target\release\sprawl.exe [folder]
```

`cargo test -p sprawl-core` runs the core tests.

`sprawl --screenshot out.png [folder]` scans (default `C:\Program Files`), saves a picture of the
window and exits - handy for checking the look without clicking around.

To use a WinUI type or member that isn't projected yet, add it to `crates/bindgen/types.txt` and
run `cargo run -p sprawl-bindgen` (needs the Windows SDK 10.0.26100 metadata).

## Releasing

1. Raise `version` in `Cargo.toml` (`[workspace.package]`) and add a `## <version>` section to
   `CHANGELOG.md`. Commit and push.
2. Build the release tool on its own, then let it build, sign and publish:

```powershell
cargo build --release -p sprawl-release
.\target\release\sprawl-release build     # builds Sprawl, signs, makes the installer with Inno Setup 7, fills dist\v<version>
.\target\release\sprawl-release publish   # creates the GitHub release (needs gh)
```

The signing key is `%USERPROFILE%\.sprawl-release\signing.key`, made once with
`sprawl-release keygen`. It never goes in the repository. Its public half is `PUBLIC_KEY` in
`crates/sprawl-install/src/lib.rs`; losing the key means installed copies can't take updates
any more (they would need a new installer), so keep a copy somewhere safe.

To try an update on a build that is already current, start it with
`SPRAWL_PRETEND_VERSION=0.0.1`; it then sees the latest release as new.

## Status

Working:

- Scanning: a drive or folder (several at once, or a CSV), from the NTFS MFT when run as
  administrator and by a multi-threaded folder walk otherwise.
- Analysis: tree view and file view with sortable columns and multi-select, the file-type table,
  the filter, and the treemap with City, Galaxy and more views, names and free space drawn on it.
- Row actions: Explorer, command prompt, copy path, and delete to the Recycle Bin or permanently.
  Exports: CSV and PNG.
- Settings and the window are remembered.
- Advisor: findings from built-in rules with risk levels. It can clean up after a review, and it
  leaves anything new since the scan and anything you chose to keep. It can also save a script.
- Cleaners: per-app parts. Community cleaners are off until you turn them on.
- Search: the scan's names, and other drives when you switch them on.
- Optional AI: off by default. It names folders and suggests findings, but never deletes.
- Scan history and growth, and signed updates.

Not built: an uninstaller (Sprawl hands an app to Windows' Installed apps, Steam or a program you
choose) and privacy cleaning (planned).
