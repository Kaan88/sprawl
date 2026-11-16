# Sprawl

A disk space analyser for Windows in **Rust + WinUI 3** (Windows App SDK 1.8), modelled on
[WizTree](https://diskanalyzer.com/): a 1:1 feature match, with a more modern look.

![Sprawl](docs/screenshots/sprawl.png)

## Layout

```
crates/
  sprawl-core/   UI-free logic: scanning, folder totals, file-type stats, squarified treemap layout (+ tests)
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
docs/
  reference/screenshots/   the original WizTree 4.33 (main window, File and Options menus)
  reference/analysis/      static analysis of WizTree64.exe (triage, strings)
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

## Status

Working: drive/folder scan (std::fs walk on a background thread), tree view with expand/collapse,
file view (1000 largest files), file-type table with colours, cushion-shaded treemap with hover,
click-to-reveal and file-type highlight, zoom in/out, units menu, show/hide panels, Explorer /
command prompt / copy path, delete to Recycle Bin or permanently, CSV exports, treemap PNG export.

Not yet: reading the NTFS MFT directly (WizTree's speed trick), sortable columns, Select Multiple,
Select CSV file, Reselect Child, filter, context menus, Colors / Tweaks dialogs, free space on the
treemap, folder/file names drawn on the treemap, settings persistence.
