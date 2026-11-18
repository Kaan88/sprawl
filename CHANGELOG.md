# Changelog

Each release of Sprawl is listed here, newest first. The section for a version is what the
in-app updater shows.

## 0.3.2 — 2026-11-18

### Changed
- **Sprawl asks its questions in its own boxes**, in the window, in place of Windows' message boxes. Esc or No is the default; Yes needs a click, or the keyboard moved onto it first. If the scan changes while a delete question is open, nothing is deleted and Sprawl says why.
- **File View sorts in a fraction of a second** on millions of files. By folder it took up to a minute and a half on a full C: drive.
- Ticking or opening a row in the Advisor no longer rebuilds the whole list, so the list doesn't jump and the click is answered at once.

### Fixed
- **The Cleaners tab jumped back to the top** each time a row was opened or ticked further down the list. It now stays where it was, and so does the column beside it.
- Sprawl's tools for AI apps (MCP): the scan report lists the Advisor's findings again, and says when the rules haven't run yet. With Sprawl closed it answers from History's last saved scan and says how old that is. A tool call after Sprawl was restarted no longer fails.

## 0.3.1 — 2026-11-18

### Changed
- **The four "as administrator" switches are kept where only an administrator can change them** (the machine's registry, for your account). Before, they were in your settings file, which any program you run could edit to turn one on. They can now be changed only in Sprawl running as administrator; elsewhere they show greyed out. A switch you had turned on is off again until you turn it on from an elevated Sprawl.
- After "Tick the safe ones" or "Tick enough", the Advisor shows its Findings, where the ticks are, and each application in its Cleaners group says how many of its parts are ticked.

### Fixed
- The Advisor tab could show its header and nothing under it after a visit to the Cleaners tab.
- Community cleaners were downloaded when Sprawl opened on the Advisor or Cleaners tab even with the setting off.
- A folder name with letters such as ü or ö right after `%TEMP%` or `%APPDATA%` could stop the community download, or the check of a cleaner of your own, with a crash.
- Community cleaners can no longer name your own folders (Documents, Pictures, Downloads...), a BleachBit "file" action removes files only (never a whole folder), and a cleaner of your own can't run as administrator.
- Claude Code's chat: an answer to a web search from a stopped question no longer answers the next question's search.
- Web searches leave out paths written as addresses (`file:///C:/...`), and your user name or computer name inside a word.
- Claude Code's working folder is refused if it has been replaced by a link to somewhere else.
- The Windows menu checks the command chosen and gives add-ons more room for their command names.

## 0.3.0 — 2026-11-18

### New
- **Cleaners tab.** BleachBit-style: each application with its parts (caches, logs, crash dumps), what cleaning costs and whether the application is open. It reads only the folders the cleaners look at, so it works without a full scan. Your usual ticks can come back next time (a setting). Community cleaners from winapp2.ini and BleachBit can be turned on; they are off until you do.
- **Privacy cleaners, off by default** (Settings > Cleaning, "Show privacy cleaners"). Cookies and history of Chrome, Edge and Brave, and cookies of Firefox, one row per browser profile by its name. Each is ticked on its own, never for you, never counted as space, and has its own box to tick in the review. If the browser or another program holds one of its files, nothing in that set is removed and Sprawl says what to close. The files are deleted, not overwritten.
- **Search tab.** File names in the scan and, when you switch them on, your other drives (also without administrator rights). Filters under the box (Kind, Type, Size, Modified, Where), keywords such as `ext:`, `size:`, `dm:`, `path:` and `regex:`, suggestions as you type, saved and recent searches. Enter opens a result, Ctrl+Enter shows it in the tree.
- **Your own files go to the Recycle Bin** when the Advisor cleans up (Settings > Cleaning, on by default). Caches are still deleted, since in the bin they would free nothing. Sprawl counts what the bin already holds.
- **Windows' own right-click menu** on a row, with the add-ons from other programs.
- **The AI on the Cleaners tab**: ask it, and it can suggest parts to tick (safe and amber parts only; you still review). Claude Code's chat gets Sprawl's tools.
- **Work offline** (Settings > Network): one switch that stops the update check, community downloads and every AI request.
- **"Open Sprawl on"** (Settings > General): the tab you used last, or a tab you choose.
- One activity indicator beside the filter box for everything running in the background; click it for the list.

### Changed
- The Advisor finds more, and says how sure it is: folders left behind by removed applications (shown only when they may hold your work), programs built on Chromium, build outputs graded by how idle the project is, and growth taken from your scan history. Tips are ranked by what they gain, apart from deletes.
- **On a drive of games, media or backups** the Advisor now lists the biggest folders and files nothing has changed in for a year, as information: Sprawl doesn't remove them, you decide. Before, such a drive showed "nothing to clean".
- The thresholds the Advisor uses (how old, how idle, how big) are settings, each with a floor it can't go below.
- **Run as administrator:** the Windows menu, handing an app to your uninstaller, Claude Code and clean-up tools from your own folders stay off, as before, but each can now be allowed in Settings > Scanning, under one warning. A switch that is on is your answer: nothing asks again.
- A clean-up never touches a network path, anything new since the scan, or anything you chose to keep; the review holds back what won't run on this PC and says why. A saved script says what it doesn't protect.
- Nothing is read, fetched, chosen or ticked for a new user until they say so.
- Numbers keep their width while they count; tabs, chips and map buttons show pointer-over, pressed and keyboard focus the same way. The map folds away on the Advisor and Cleaners tabs, and on Search in a short window it takes less room.
- Times are shown in local time everywhere. Typing in the filter is quicker, and the tree's work no longer holds up the window.

### Fixed
- The AI on the Cleaners tab answers with nothing scanned (it uses what the tab read), and a part that can't be counted no longer holds the question for ever.
- A change to the AI or administrator settings takes effect when you press Save, not at the next start.
- A problem in the Advisor's analysis ends that check instead of the tab.
- Very long tables no longer stop drawing part-way.
- The result card no longer says files were "deleted for good" when nothing was removed.

## 0.2.5 — 2026-11-17

### Fixed
- **The setup wizard opens by itself the first time Sprawl starts.** Before, it waited for the first scan and then only showed a small "Set up Sprawl?" card, which the update card could cover.

## 0.2.4 — 2026-11-17

### Fixed
- The Model list now loads when you type or change the server address, not only when you switch provider. Before, an address typed afterwards (LM Studio on another PC, say) left you with a text box.
- Switching provider no longer keeps the old provider's model (such as a Claude model for an OpenAI-compatible server).

### Changed
- Claude Code gets a Model list too: the current Claude models (its command line can't list them). **Haiku is now its default**: fast, and the cheapest on your plan's limits.

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
