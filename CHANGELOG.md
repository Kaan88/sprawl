# Changelog

Each release of Sprawl is listed here, newest first. The section for a version is what the
in-app updater shows.

## 0.2.3 — 2026-11-17

### New
- **The installer is now an Inno Setup installer**: the usual setup wizard, a choice of folder, an optional desktop shortcut, and Inno's own uninstaller. Copies installed with 0.2.0 to 0.2.2 move over when you run it.
- **Pick the AI model from a list.** In the AI settings and on the Settings page, Model is a dropdown filled from your provider (Anthropic with your key, or the OpenAI-compatible server's list, such as Ollama or LM Studio). If there is no list, you type the name as before, and Sprawl says why.

### Fixed
- The update card and the "Set up Sprawl?" card can be clicked and scrolled again (the welcome screen was covering them).

## 0.2.2 — 2026-11-17

Safer updates, from a safety review of the updater.

### Fixed
- Uninstalling now removes the program files even if you take a while to close the last message, or another Sprawl window is still open.
- Updates keep working if GitHub moves its download servers or the project is renamed.
- If the daily update check fails three times in a row, Sprawl says so instead of staying quiet; one offline start no longer skips the next day's check.
- A future release that adds a file no longer stops older copies from updating.
- Each downloaded file is checked again just before it is used.
- Updates can also be signed with a backup key kept offline.

## 0.2.1 — 2026-11-17

### Fixed
- After an update, the old copy closes its window normally instead of ending in a way Windows could report as a crash.

## 0.2.0 — 2026-11-17

The first release with an installer and automatic updates.

### New
- **Installer and updates.** `Sprawl-Setup-0.2.0.exe` installs Sprawl for you only, with no administrator rights, and adds it to the Start menu and to Settings > Apps. Sprawl looks for a new version once a day (Help > Check for Updates, and a switch to turn it off), shows what changed, and updates itself after you say so. Every update is signed and checked before it is used.
- **The Advisor tab.** Finds what can go: build outputs of idle projects, caches, old installers, temporary files and more, each with why it's safe or what it costs to get back. Clean up inside Sprawl: tick, review, and Sprawl removes it, with a log of what went.
- **Ask your disk.** An optional AI (Claude, OpenAI-compatible or a local model) explains unknown folders and answers questions about the scan. Sprawl can also run as an MCP server for Claude Code.
- **Growth over time.** History keeps your scans; the Growth tab shows what grew and when the drive will be full, and History plays it back as a time-lapse.
- **Seventeen more views** behind the More button, and one Colour switch (Types, Risk, Owner, Growth) for every view, with a key.
- **Multi-select** in the tree and the file list (Ctrl+click, Shift+click), with one confirmation for everything picked.
- One Settings page (Options > Settings, Ctrl+Shift+S) and a Help menu with the keyboard shortcuts.

### Changed
- The window fits your screen on first start and opens where you left it.
- Scan turns into Stop while scanning; F5 scans the folder on screen again.
- Copy Path, Export to CSV and Export File Types moved to Ctrl+Shift+P, E and X (Ctrl+Alt clashed with AltGr).
- Text is at least 12 px, the dimmest grey is brighter, and buttons and ticks have names for screen readers.
- The setup is offered in a small card after your first scan, instead of opening over it.

### Fixed
- A long list in the clean-up review no longer pushes its buttons out of reach.
- Sprawl's own deletes no longer show as changes from outside.
- A colour mode with nothing to show no longer darkens the whole map.

## 0.1.0 — 2026-11-16

The first version named Sprawl: a fast disk space analyser with a treemap, City and Galaxy views, MFT scanning as administrator, and a filter.
