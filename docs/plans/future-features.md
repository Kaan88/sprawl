# Future features

Originally written 2026-11-17, after the audit. The designs below are retained as history.
**Status as of 1.0.0 (2026-11-19):** F1/F2/F3/F4/F6 have shipped within the supported scope
recorded in `next-round.md`; F5's generic headless report remains future work. F1's dedicated
drift/recompression workflow and F4's Separate again action also remain unimplemented.
See [current status](status.md) for the complete backlog and runtime limits. A "Done when"
paragraph below is a design acceptance target, not a claim that every live workflow was tested.

Every feature retains the app's safety checks and explicit review before cleanup. The full
retained Fable queue and current fixes passed Astra review for 1.0.0; no additional Fable read
was requested. Future safety changes still need independent review.

## Order

| Step | Feature | Why here | Rough size |
|---|---|---|---|
| F1 | Compress in the app | The tip, the size estimate and the tool-command runner exist; reversible | Medium |
| F2 | Old versions side by side | Fits the rule engine; likely the largest gain on developer machines | Medium to large |
| F3 | Shrink virtual disks | Small list of cases; needs care with running VMs | Medium |
| F4 | Duplicates resolved by linking | Duplicate search exists; the action is new and must be exact | Medium |
| F5 | Headless report | Parts exist; no UI | Small |
| F6 | Low-space and growth alert | Shipped with the account's scheduled task; no resident background app | Large |
| F7 | History deltas | Built after 1.0.7 (2026-11-23): each scan saves only what changed since the one before | Medium |
| F8 | Registry cleaning | Built after 1.0.7 (2026-11-23): six Explorer lists, backup and Put back; Fable-reviewed | Medium |
| F9 | Secure wiping | Built after 1.0.7 (2026-11-23): Overwrite and delete, and Wipe Free Space (cipher); Fable-reviewed | Medium |
| F10 | Editing browser databases | Built for Firefox history and form history and Chromium autofill (after 1.0.7); Fable-reviewed; selective cookies still to do | Large |

F5 can be done at any time; it helps test the others. F7 to F10 were built on 2026-11-23 by
DeepSeek Flash agents, reviewed by Claude, and F8 to F10 each passed several rounds of outside safety
review (Fable) before they were merged; their conditions are below.

## F7. History deltas

Scan history saves each snapshot as a whole (bounded) tree. Saving runs off the UI thread
(07f1033) and the cap thins old full snapshots (b14d612); what is left:
- Store a snapshot as its changes against the previous one of the same root, with a full one
  every so often so a lost or damaged file costs only a short run.
- Skip saving a subfolder scan already covered by a recent snapshot of its drive.
- Keep whole-second times, the cap's rule (never a root's newest), and the slider's speed.

**Done when.** A day of repeated drive scans takes about a tenth of today's history space, and
every snapshot opens exactly as before, also after one in the middle of a run is lost.

**Built (after 1.0.7):** snapshot format v2 stores a delta against the previous snapshot of the
same root, with a keyframe every so often (`crates/sprawl-advisor/src/snapshot.rs`). The cap and
age thinning remove whole units (a keyframe and its deltas) or rewrite the next delta whole before
its base goes (`history.rs` `plan`/`apply`). A subfolder scan is not saved when a drive scan from
the last day already shows it. Measured: about 5x smaller for 10 snapshots, about 8x for long chains.

## F8. Registry cleaning

MRU lists and the like, from `docs/plans/cleaners.md` "Wanted later". A fixed, built-in list of
HKCU values only, never a key path from a definition (a rule, the community list or the AI);
never Prefetch or UserAssist. Each write takes a backup first and has an undo.

**Built (after 1.0.7):** `crates/sprawl-advisor/src/regclean.rs`, six parts: RunMRU, TypedPaths,
WordWheelQuery, OpenSavePidlMRU, LastVisitedPidlMRU, RecentDocs. Values only, keys stay. Keys are
walked one handle at a time with REG_OPTION_OPEN_LINK (a registry link anywhere is refused). A JSON
backup goes to `%LOCALAPPDATA%\Sprawl\registry-backups` first, listed in `written.json` with its
SHA-256; Put back (result card and Settings > Cleaning) restores only listed, unchanged backups.
The last ten are kept.

## F9. Secure wiping

Shredding files and wiping free space. It must say plainly what it can't promise: on SSDs
(wear levelling, TRIM), on copy-on-write or compressed volumes, in cloud-synced folders and
where shadow copies or the Recycle Bin hold older copies.

**Built (after 1.0.7):** right-click > Overwrite and delete... (`crates/sprawl-advisor/src/overwrite.rs`,
app glue `crates/sprawl/src/wiping.rs`): one pass of random data (BCryptGenRandom) over each file's
data and its named streams, through the checked handle, then delete. Hard links, reparse points and
files changed since the check are left; sparse files are written by range; compressed and WOF files
need their extra room first. A folder that keeps something is left with its own streams untouched.
File > Wipe Free Space on the Selected Drive... runs `cipher /w:X:\` in a job object with a marker
so two Sprawls never run it at once. The question says what can't be promised (SSD, shadow copies...).

## F10. Editing browser databases

Selective cookies, Firefox history (it shares `places.sqlite` with bookmarks) and autofill.
Privacy cleaning used to remove whole files only; this opens a database to remove some rows. It
needs the browser closed, a backup of each file, and a check that the file is the format and
version expected before anything is written.

**Built (after 1.0.7):** Firefox's browsing history without bookmarks and its form history, and the
autofill entries of Chrome, Edge and Brave. The history part also takes out Firefox's page-interaction
records (view times, typed text, search terms) and newtab click records. Each edit copies the
database (and its `-journal`, `-wal` and `-shm`) to `%LOCALAPPDATA%\Sprawl\backups\privacy`, keeps
the last five complete copies (dropping copies older than 30 days; a finished copy carries a
`complete` marker, so a try that never finished never counts as one of the five), and checks the copy
byte for byte; then
it checks `PRAGMA integrity_check`, the tables and columns, and (Firefox) the schema version on a
read-only connection before taking a write handle, and does every delete in one transaction. The
browser is checked again once the transaction is open and again before it is kept (and Firefox's
`parent.lock` / Chromium's `lockfile`). Nothing is VACUUMed, so the file may not shrink. The code is
`crates/sprawl-advisor/src/privacy/edit.rs`; the test fixture is Firefox's own schema
(`firefox_places.sql`, from mozilla-firefox/firefox's `nsPlacesTables.h`, schema 89).

**Still to do:** selective cookies ("keep these sites' cookies").

---

## F1. Compress in the app

**What.** Turn the "Compress <folder>" tip into an action: Sprawl compresses a program or game folder with Windows' transparent file compression, measures the result, and remembers that it did.

**Decisions from the user.**

- Use the newer algorithms (XPRESS4K, XPRESS8K, XPRESS16K, LZX), the ones `compact.exe /exe:` offers, not the old NTFS compression.
- The app remembers which folders it compressed.

**Design.**

- **Algorithm choice.** A setting with four values and a default, plus a per-run override in the review:

  | Algorithm | Character | Suggested use |
  |---|---|---|
  | XPRESS4K | Fastest, smallest saving | Slow CPUs |
  | XPRESS8K | Balanced | Default for programs in daily use |
  | XPRESS16K | Better saving, still fast to read | Default for games and large tools |
  | LZX | Best saving, slowest to compress, a little more CPU when read | Things started rarely |

  The defaults are a starting point; measure on a few real folders before fixing them.
- **How.** Run `compact.exe` from System32 through the existing tool-command runner (hidden, job object, output kept): `compact /c /s /a /i /exe:<algorithm>` on the folder. The alternative is setting the backing per file with the file-system control code directly; start with `compact.exe`, which is Microsoft's own tool for this.
- **What to skip.** Files that are already compressed formats (video, archives, most game packs) gain nothing: estimate from a sample of the folder before offering, and replace today's fixed ratios in `remedies.rs` with that sample. Never compress inside Windows' own folders, or anything on the never-list.
- **Memory.** A new file beside `memory.toml`, one record per folder: path, algorithm, date, size on disk before and after, file count. Uses:
  - The row shows "Compressed with XPRESS16K on <date>, saved 2.1 GB".
  - The tip is not offered again for that folder.
  - **Drift.** An app update writes new, uncompressed files. On a later scan, compare size on disk with the record; when the folder has grown back past a threshold, offer "Compress again (new files since <date>)".
  - **Undo.** "Uncompress" on the same row (`compact /u /s /a /i /exe`).
  - If the folder is gone, drop the record.
- **Seeing what was compressed (asked for by the user).** The app already has places a filter fits, so no new button is needed:
  - Advisor: a fourth "Show" chip beside All, Can clean now and My own files: **Compressed**. It lists every folder in the record with algorithm, date, saving, and its state now (still compressed, grown back, gone), with Compress again and Uncompress on each row.
  - Search: a keyword, `compressed:`, matching files and folders whose size on disk shows compression, so it also finds what was compressed outside Sprawl.
  - Tree View: a small mark on compressed folders, with the saving in the tooltip.
  - Later, if wanted: a "Compressed" colour mode for the map, next to Types, Risk, Owner and Growth.
- **Detecting state without the record.** A folder compressed outside Sprawl should not be offered either. Size on disk well below size, from the scan's allocated figures, is the cheap signal. Check: whether the folder walk reports the compressed size for these files or only the MFT scan does (`core/lib.rs` says the walk rounds length up to the cluster size, which would hide it).
- **Risks to state in the UI.** Compression takes minutes on a big folder and uses the CPU; files in use are skipped; some games with anti-cheat or their own patchers may re-download or refuse compressed files (check a short list of known cases before shipping); needs administrator for folders under Program Files.

**Steps.**

1. Sample-based estimate in place of the fixed ratios; fix the tip's listed size to the estimated saving (already in phase 2 of the audit roadmap).
2. The record file and the "already compressed" detection.
3. The action, review text and measured result.
4. Drift detection and "Compress again".
5. Undo.
6. The setting.
7. The Compressed chip in the Advisor, the `compressed:` search keyword and the tree mark.

**Done when.** A test folder compresses, the row shows the measured saving, a rescan does not offer it again, adding files to it brings back "Compress again", and Uncompress restores the original size on disk.

---

## F2. Old versions side by side

**What.** Find several installed versions of the same thing and offer the older ones.

**Design.**

- A new finding family, "Older versions", grouped by product. Each group lists its versions with size, install date and whether anything on the machine refers to it.
- Start with products that have an official way to remove one version, and use that, never a folder delete:

  | Product | Where the versions are | Removal |
  |---|---|---|
  | .NET SDKs and runtimes | `dotnet --list-sdks`, Program Files\dotnet | The product's own uninstaller entry (hand off to Installed apps) |
  | Android SDK: system images, build-tools, platforms, NDKs | The SDK folder | `sdkmanager --uninstall` |
  | Node versions (nvm for Windows, fnm, volta) | Their version folders | The manager's own uninstall command |
  | Python versions (py launcher, pyenv-win) | Registry and version folders | Installed apps, or the manager's command |
  | Rust toolchains | `rustup toolchain list` | `rustup toolchain uninstall` |
  | Visual Studio workloads and Windows SDKs | Installed apps | Hand off |
  | Driver installer leftovers (`C:\NVIDIA`, `C:\AMD`, extracted setup folders) | Fixed paths | Delete; amber |

- **What "old" means.** Never the newest. Not a version a project on the scanned drive asks for: read `global.json`, `.nvmrc`, `.python-version`, `rust-toolchain.toml`, Gradle's compile SDK from the projects the engine already finds. A version something refers to is shown, marked "used by <project>", and cannot be ticked.
- Risk: amber at least. Program installations stay on the never-list, so everything under Program Files goes through the product's own uninstaller.

**Steps.**

1. The grouping model and the row design.
2. Rust toolchains and Node managers (user-profile folders, simple commands).
3. Android SDK.
4. .NET and Python (hand-off).
5. References from projects.
6. Driver leftovers.

**Done when.** On a machine with three versions of one product, the two older ones are listed with sizes, a version named by a project file is locked with its reason, and removing one goes through that product's own tool.

---

## F3. Shrink virtual disks

**What.** WSL and Docker keep their data in disk files that grow and do not shrink by themselves. Sprawl lists them as information today. Offer the steps that give the space back.

**Design.**

- Show for each disk file: size on disk, and how much is free inside when that can be read cheaply (for WSL, `wsl -d <name> df` while it runs; otherwise say "unknown until tried").
- The action is a sequence the user sees in the review: stop the distribution or Docker, compact the file, start nothing again by itself.
- Disk images are on the never-list, and stay there for deletes. This is a curated built-in tool action that changes the file's size, not its contents, the same way the DISM rule is allowed on WinSxS.
- Must be closed: WSL shut down, Docker Desktop not running. Check twice, as for must-closed cleaner parts. Needs administrator.
- Check before building: which compact method each supported Windows edition has (the Hyper-V PowerShell command is not on Home; `diskpart` is everywhere), and whether current WSL versions offer their own sparse or compact option, in which case use that.
- Other products (VirtualBox, VMware, Hyper-V machines) stay information only, with the vendor's own instructions.

**Steps.**

1. Free-inside estimate and the row.
2. WSL.
3. Docker Desktop.
4. Result measured from the file's size before and after.

**Done when.** A WSL distribution with deleted data inside shrinks, the distribution starts and works afterwards, and the row shows the measured gain.

---

## F4. Duplicates resolved by linking

**What.** Replace identical copies on the same drive with hard links to one file. Every path keeps working; the space of the extra copies is freed. Nothing is "the copy to delete".

**Design.**

- First fix how duplicates are shown (audit finding ADV-5): one set per row group, every copy listed.
- Offer linking only when it is exact and safe:
  - Same volume, NTFS.
  - Byte-for-byte equal, checked again immediately before linking, with both files opened so they cannot change in between.
  - Neither file in use, neither on the never-list, neither a cloud placeholder, same attributes.
- **The warning that must be shown.** After linking, the paths are one file: editing one changes all. So offer it by default only where files are not edited: inside program and game folders, installers, media. For documents and source folders it is off, or orange with the explanation.
- How: create the link under a temporary name beside the duplicate, then replace the duplicate with it in one step, so a failure leaves the original in place.
- Record what was linked in the clean-up log, with every path, so it can be undone by copying.
- Never across users' profiles; never when the two copies have different owners or permissions.

**Steps.**

1. Duplicate sets in the UI.
2. Eligibility rules and their tests.
3. The linking action with the re-check, and failure handling.
4. Measured result and log.
5. "Separate again" for one path (copy the file back to its own).

**Done when.** Two identical files become one on disk, both paths open, the freed space is measured, a file that changed after the search is refused, and a failure half-way leaves both files as they were.

---

## F5. Headless report

**What.** `sprawl --report <out.json> <folder>`: scan, run the rules, write a file, exit. No window. For scripts, scheduled checks and support.

**Design.**

- Reuse what exists: `scan_auto`, `advise_all`, the CSV export, the MCP server's report text.
- Output: JSON by default (drive totals, largest folders, file types, findings with risk, size, path and reason, what was held back and why, scan method and time), `--csv` for the WizTree-compatible tree, `--text` for the short report.
- Read-only by definition: no clean-up from the command line, no AI, no network.
- Exit codes: 0 done, 1 scan failed, 2 bad arguments. Errors on stderr.
- The app is a windowed program, so console output needs checking: either attach to the parent console, or ship a small console front end beside `sprawl.exe` (it must be added to the signed release manifest).
- Options: `--min-size`, `--no-paths` (sizes and titles only, for sharing), `--compare <snapshot>` for growth since an earlier scan.

**Steps.**

1. The JSON shape, written down.
2. The command and exit codes.
3. Console behaviour.
4. `--no-paths` and `--compare`.

**Done when.** The command runs from a scheduled task with no window, writes a valid file, and returns the right exit code for a missing folder.

---

## F6. Low-space and growth alert

**What.** A short notice when a drive is running out or grew quickly: "C: has 12 GB left" or "C: gained 20 GB this week, mostly in <folder>", with a button that opens Sprawl there.

**Design.**

- Off by default. One setting, with the thresholds (free space below N GB or N percent; growth above N GB per week).
- **Free space** is cheap to check and needs no scan.
- **Growth** needs scans. Two choices, to decide when this is built:
  - Compare only the scans the user makes. No background work; the alert comes late or never.
  - A scheduled folders-only scan (the compact snapshot is about 5 MB and the scan takes seconds as administrator, longer without). More useful; costs disk activity the user did not start.
- **Running in the background.** The app has no background presence today. Use a Windows scheduled task that runs the headless command from F5 (`sprawl --check`), not a process that stays in memory. The task is created when the setting is switched on and removed when it is switched off, and Sprawl's uninstaller must remove it.
- The notice is a Windows notification. One per drive per week at most; never a nag.
- No clean-up from the notice. It opens the app.
- Fix first: the growth forecast needs at least three points over a week (audit finding ADV-1), or the alert will cry wolf.

**Steps.**

1. Forecast fix.
2. `sprawl --check` on top of F5: free-space test only.
3. The scheduled task, its setting, and removal on uninstall.
4. The notification and the "open there" link.
5. Growth from scheduled folders-only scans, if chosen.

**Done when.** With the setting on, filling a test drive past the threshold produces one notice, switching the setting off removes the task, and uninstalling Sprawl leaves nothing scheduled.

---

## Look-and-feel pass

**New brief, 2026-11-19:** the user requested an Astra assessment of modern WinUI 3 visuals,
motion, interaction feel, design and usability, plus worthwhile features and AI enhancements.
Mockups must precede implementation of proposed UI changes. This broader brief supersedes
the earlier narrow polish scope below; its old reports remain useful evidence, not a current
claim that all three problems still reproduce. See [current status](status.md).

Asked for by the user on 2026-11-18, for a later release: one pass over how the window looks and moves, after the layout work of phases 3 and 4 has settled. Not a redesign. Start from these three reports and then walk every tab at 100 % and 150 % scaling.

| Seen by the user | Likely cause (to confirm in code) | Direction |
|---|---|---|
| The progress pill at the top right shows numbers that shift sideways as they count | Proportional digits in the UI font, and a pill sized to its text | Tabular figures: the font already has them (`Typography.NumeralAlignment="Tabular"` on the TextBlock). Give the pill a steady width so it does not resize on every update. Apply the same to every counting number: scan status, the HUD, clean-up progress, the search footer |
| Going Search → Cleaners shows that pill; but when Cleaners starts its own read and you switch to Search, nothing similar shows | Each background job draws its own indicator in its own place (`IndexBusy` for drive reads, the strip in Search, the Cleaners head line) | One background-activity indicator, in one place, on every tab, listing what is running: reading drives for Search, reading folders for Cleaners, the Advisor's check, a clean-up. Click it to go to the tab that owns the job |
| The highlight on the tabs looks odd, most of all after clicking one and moving the pointer away | The tabs are plain Buttons whose background is painted by hand in `paint_tabs`; the Button's own hover, pressed and focus looks stay on top of that | Use a control with a real selected state (a selector bar, or toggle buttons with one style) so selected, hover, pressed and keyboard focus are each defined once. Same for the Advisor's chips and the map-mode buttons, which are built the same way |

Also for this pass, from the audit: the Animations switch clipped in Colours and Tweaks, row tags that look like buttons, the brand card that repeats the title bar, and consistent spacing between the tab row, the lists and the foot.

**Done when.** No number moves sideways while it counts; a background job looks the same from every tab; a tab has exactly one highlighted state at rest; and the three reports above cannot be reproduced.

---

## Small requests

**Which tab the app starts on** (asked for by the user on 2026-11-18).

- A setting under General, "Open Sprawl on": the tab used last (default), Tree View, File View, Advisor, Cleaners, or Search.
- Cleaners and Search make sense as a start tab now that neither needs a scan first: Cleaners reads its own folders, Search can start from the remembered drives or a saved scan. Starting on Cleaners must not begin its folder read before the window is drawn; show the tab, then read.
- Tree View, File View and the Advisor show the welcome screen until something is scanned, as today.
- A folder passed on the command line or from "Scan with Sprawl" in Explorer still scans at once and opens Tree View, whatever the setting says. So does the restart as administrator from the Advisor, which returns to the Advisor.
- It replaces the old "Start in" choice (which picked the map view, not the tab) that the wizard is dropping.
- Done when: the chosen tab is the one shown at start, with no scan started by the setting itself, and the three exceptions above still behave as written.

**First launch speed** (asked for by the user on 2026-11-22). Target: the window is usable at once, like an
app that is already open.

- What the user sees: with Search as the start tab, Sprawl shows a black window for about 2 seconds before
  anything is drawn.
- Measure first, with `SPRAWL_TIMING=1` (it logs to %TEMP%\sprawl-timing.log), for each start tab and for a
  cold and a warm start: process start, the elevation relaunch (this PC always runs as administrator, so the
  first process starts a second one), XAML load of the page, the resource dictionary, the first frame, and
  whatever the start tab reads before drawing (Search: remembered drives or a saved scan).
- Likely work: draw the window and the tab's frame first and fill it in afterwards (the rule Cleaners already
  follows); move any file, registry or index reads off the UI thread or after the first frame; load the page in
  pieces if one XAML load is the cost; paint the window's ground colour at once so even the first moment is
  not black; look at what the elevation relaunch costs and whether the unelevated first process can show
  something while it waits.
- Done when: from a double-click the window shows the chosen tab's frame without a black stage, Search is
  usable without waiting, and timings before and after are written down.

**From the user's notes** (copied 2026-11-23; all three shipped in the re-published 1.0.8, built by
DeepSeek Flash, reviewed by Opus, merges d1267d9, c9294b0, ffb4e10).

- Done: Search right-click keeps the picks, and the row menu acts on all of them (Open, Show in
  Explorer, Copy Path, Delete, Delete Permanently; Open and Show in Explorer stop at 10).
- Done: Remembered shows an AI explanation whole and wrapped (capped at 1,000 characters on a word,
  preferences 300, labels 200); trimmed one-line texts show the whole text on hover.
- Done: the AI's tools say when an item was last opened and when a program last ran, and its search
  takes `da:`. Read per path on local fixed NTFS drives only, never for a client that may only see the
  scan; what the reading limit leaves unread is said.

**Open: Fable's two findings on the Search delete path** (2026-11-23, quick review of 782229e..b01bdb0;
shipped as is on the user's word, nothing deletes unannounced since the dialog names every item).

- Low, older, now reachable from the row menu: picks can outlive their rows. Other drives give more
  matches than `searchMaxHits`; Ctrl+click A, B, C; sort by a column, and C can fall off the list;
  right-click A > Delete still asks about and deletes C (Copy Path gives only A and B). Also: picks
  stay after the search box is emptied or a query fails to parse (`more_run`'s early return), so the
  Search bar's Delete / Del ask about rows no longer shown. Fix: `delete_more`
  (search_delete.rs ~331) also requires every pick's (place, id) to be among the current rows, else
  the existing "The results changed since they were picked" refusal; clear `sel`/`picks` in
  `more_run`'s early return; prune picks when a re-sort arrives.
- Low, older, not via the menu: Ctrl+click A, B, C, then Ctrl+click C to un-pick it: `sel` stays C,
  and Del or the Search bar's Delete adds `sel`, so the dialog asks about 3 (C included). The row
  menu gets it right, so keyboard and menu disagree. Fix: one rule in `delete_more`, `more_row`,
  `menu_paths`, `menu_pick_count` and the picked note: with picks, `sel` counts only if picked.
- Still to do: the full Fable review of this release (all three changes, not only the delete path);
  entry in .claude/fable-review-queue.md.

**Requested later (2026-11-23): Search, folders first when sorted by name.** Sorting Search results
by Name should put folders before files (both ways round, as Everything does: folders A-Z then files
A-Z, and reversed within each group). Name sorting runs in more than one place, and each needs it:
the scan's results (search.rs ~482), the other drives' and saved scans' rows (search_more.rs
`compact_val` / `scan_val`, `SortVal::Text`) and the compact index's own sort
(`sprawl_core::compact`, `ix.sort(.., SortBy::Name, ..)`). Check first whether Tree View and File
View already do folders first by name, and keep the Search picks and the 10-at-once limits as they
are. Done when: a mixed result list sorted by Name shows every folder above every file, in either
direction, on the scan on screen and on other drives, with a test for each sort path.

**Search: what to borrow from Everything** (2026-11-23, the user asked to look; not started). Sprawl
already has: words, globs, a|b, !not, path:, size and dm: ranges, ext:, regex:, case:, exact:,
depth:, folder:/file:, compressed:, match highlighting, filter chips (Kind, Type, Size, Modified,
Where, More), saved and recent searches, keyword suggestions, Enter / Ctrl+Enter, and the picks menu.
The audit decision stands: powerful search, not a second Everything (no background service, no
always-live index). In order of value for a disk tool:

1. Folders first when sorted by name (above).
2. The whole list, not the first N. Everything sorts and counts every match; Sprawl caps at
   `searchMaxHits` and re-takes the cap on a re-sort (also the root of Fable's finding 1). At least:
   sort over all matches before the cap, and say "showing the first N of M".
3. More dates: `dc:` (created) and a Created column (the MFT and the folder walk both get it at no
   extra cost; check what the scan keeps now), and `da:` (last opened) for the user too, read per
   match with the same `Accessed` reader and limits the AI's search uses.
4. Folder-shape keywords: `parent:` (only the direct children of a folder), `child:` (folders that
   hold a match), `childcount:` / `childfilecount:` (e.g. folders with more than 10,000 files).
5. Duplicates in the list: `dupe:` (same name), `sizedupe:` (same size), sorted so copies sit together;
   feeds F4 (duplicates resolved by linking).
6. Attributes and path trouble: `attrib:h` / `s` / `r` / link (hidden, system, read-only, junction),
   and `len:>260` for paths too long for many tools.
7. Programs: `dr:` (last run) and run count from Prefetch, which `Facts` already reads for the AI.
8. Type macros if the Type chip doesn't cover them already: `audio:`, `video:`, `pic:`, `doc:`,
   `exe:`, `zip:`.
9. Grouping: `<a b>|c` (Sprawl's | only works inside one word).
10. Row actions Everything users expect: F2 rename, drag a result out to Explorer or another app,
    Ctrl+C copies the file itself (paste in Explorer) with Copy Path staying on Ctrl+Shift+C; check
    what F2 and Ctrl+C do now.
11. Choose columns: show or hide Folder, Type, Created, Accessed, Attributes.

Left out: a live index kept by a service (USN journal watching), content search, the HTTP/ETP server,
thumbnails and the preview pane: costly, against the audit decision, or not what a disk tool is for.

---

## Left out on purpose

Registry cleaning, empty-folder and broken-shortcut finders, "not opened in years" lists, scheduled automatic cleaning, a quarantine folder. Reasons are in the conversation that produced this plan: low value against real risk, unreliable Windows dates, or removing the review that keeps clean-ups safe.
