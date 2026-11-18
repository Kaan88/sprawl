# Sprawl 0.2.5: final audit report and roadmap

Date: 2026-11-17. Commit `3eec3d0`. This report adds measurements, a real clean-up test, a live AI test and an analysis of the rule engine to the first audit (`audit-2026-11-17.md`), and replaces its roadmap. Finding IDs such as SAF-1 refer to that document.

## 1. What changed after your answers

| Your answer | Effect on the first report |
|---|---|
| The uninstaller was a mistake in the brief | Section 7, UNI-1 to UNI-4 and the "Apps view" proposal are dropped as gaps. What remains is small: the button label, the wrong location in `docs/uninstaller-handoff.md`, and the silent no-op when the owner is not a known app. The hand-off itself is sound. |
| Accessibility is not needed now | A11Y-1 to 6 move to "later". Nothing else depends on them. |
| Ignore map views | COH-4 is dropped. |
| Search should be powerful, not a competitor to Everything | A persistent index and USN replay are no longer required. The priorities become query power, memory and correctness (section 7). |
| Privacy cleaning is wanted | It gets its own phase, with prerequisites (section 8). |

Everything else in the first report stands. Three of its findings are now proven by running the app, not only by reading code (section 2).

## 2. Real clean-up test

**Set-up.** A throwaway profile in the session scratch folder, with `APPDATA` and `LOCALAPPDATA` pointed at it, and only that folder scanned. A finding must be a node in the scanned tree, so nothing outside could be reached. No tool-command items were created, because commands such as `pip cache purge` act on the real machine whatever was scanned. The real app (`target/alt/release/sprawl.exe`) did the work: Advisor, "Clean up…", "Clean up now".

Fixtures: fake Discord, VS Code and Firefox caches; settings and profile files that must survive; a read-only file; a file held open by another process; a junction inside a cache pointing at a folder outside the scan; an unknown app's `Cache`; a log 40 days old next to a new one; a file written between the scan and the clean-up.

| Case | Expected | Result |
|---|---|---|
| Green caches (Discord, VS Code Code Cache, Firefox startup cache and thumbnails) | Deleted | Deleted. Log: 6.0 MB, 2.5 MB, 2.0 MB, 2.0 MB |
| Sentinels beside them (`Local Storage\KEEP.txt`, `User\settings.json`, `places.sqlite`) | Kept | Kept |
| Read-only file in a cache | Deleted | Deleted |
| File held open by another process | Skipped and reported | Skipped; named in the result card and the log with the Windows error |
| Cache holding a junction to an outside folder | Outside folder untouched | The whole cache was held back by the safety check ("1 held back"); the outside folder is intact |
| Unknown app's `Cache` (found by shape) | Amber, not ticked | Not ticked, not touched |
| "Old logs", keep 7 days: one log modified 40 days ago, one new | Old one deleted | **Nothing deleted: "freed 0 bytes, 0 files"**, though the review promised 3.8 MB. It is ticked again afterwards with the same 3.8 MB |
| File created after the scan, before "Clean up now" | Kept, per the review text | **Deleted** (scan 18:44:31, file written 18:44:37, clean-up 18:44:45, log says "4 files") |

**Verdict.** The engine does what a cleaner must: it deleted only inside its targets, did not follow the link, skipped the locked file and reported it, and measured what it freed. Two defects are now proven:

- **SAF-1 is real.** "Anything new since the scan is left alone" is false; the new file was deleted for good.
- **The size shown for keep-days parts can be entirely fictional (CLN-2).** Cause: the analysis sizes the whole folder, while the executor keeps anything whose modified *or created* time is inside the window (`adv/clean.rs:311`). The scan does not record created times, so the analysis cannot know. A copied or restored file is "new" by creation date for a week.

Smaller things seen in the run:

- The result card says "Freed 11.9 MB from 5 items" when one item freed nothing.
- The log stamps times in UTC without saying so (15:43 for an 18:43 run).
- The review list clipped its fifth item at the default window size.

Not tested: tool commands, elevated clean-up, Stop mid-run, a clean-up of thousands of items.

## 3. Performance, measured

Machine: this PC, C: drive, 3.69 million items (3.13 million files), not elevated, so every scan is the folder walk. Release build. Disk cache warm after the first run. Benchmarks: `crates/sprawl-core/examples/bench_search.rs` and `crates/sprawl-advisor/examples/gates.rs` (new, untracked), plus the existing `bench_walk`, `advise` and `snapshot_size`.

| Operation | Measured | Note |
|---|---|---|
| Folder walk, 16 / 8 / 4 threads | 4.5 s / 6.3 s / 11.3 s | The old single-thread scan: 198 s |
| Same scan inside the app | 10.8 s | About twice the bare walk |
| Tree in memory | 631 MB, 179 bytes per item | |
| Search index build | 0.2 s, 97 MB | |
| Query, typical (`report`, `cache >1mb`, `folder:node_modules`) | 5 to 8 ms | |
| Query, broad (`e`: 2.46 M hits; `*.dll`: 140 k hits) | 20 ms, 35 ms | |
| Query with a path term (`\windows\ *.exe`) | 124 ms | |
| Sort results by size, 2.46 M hits | 0.73 s | |
| **Sort results by Folder, 2.46 M hits** | **32.9 s** | 1.3 s for 140 k hits |
| Same sort with each folder path built once | 2.0 s | 16 times faster |
| Filter `*.dll` | 0.41 s | |
| List of all files by size | 0.89 s | Built for File View after every scan |
| File-type table | 0.16 s | |
| Copy of the tree | 0.28 s | Happens on a delete while another thread holds it |
| Rule engine (`advise_all`) | 1.5 s | App inventory: 0.04 s |
| Snapshot: encode / write at zstd 9 / read back | 0.65 s / 0.95 s / 0.98 s | 28 MB full, 5 MB folders only |
| App with Search open, all five drives read, query `e` | 1.65 GB private memory, 5.19 M items | About 320 bytes per item, all in |

**What the numbers say.**

- Matching is not the problem. Search answers in tens of milliseconds on a full drive, so the 140 ms typing delay is now the larger part of the wait; 40 to 60 ms would feel instant.
- **The Folder sort is the one disaster.** A header click runs it on the UI thread (`app/tables.rs:303`, `app/search.rs:220`), building two paths per comparison. Thirty-three seconds of frozen window on a broad query. Fix in two steps: build each key once (2 s), then give every folder a rank in path order once per tree so the sort compares integers (expected near the 0.7 s of the size sort), and move it off the UI thread.
- **Scan completion stalls the window.** `set_tree` runs on the UI thread and builds the all-files list, sorts, and the type table (`app/state.rs:397-413`), then the snapshot is encoded there too. From the parts measured that is roughly 2 seconds of freeze after a full-drive scan. Applying a filter adds its 0.4 s plus the same `set_tree` work. Move all of it to the scan thread and hand the UI a finished bundle.
- **Memory is the limit on search across drives.** 179 bytes per item in the tree, and the index stores every name a second time. Three steps, in order of payoff:
  1. Drives read only for search do not need a `Tree`. A compact record (name bytes, parent as `u32`, size, modified as `u32`) is about 40 bytes plus the name: roughly a quarter of today.
  2. In the main tree, use `u32` indices, keep `files`/`folders` counts for folders only, and store children as a range in one flat array. That should come close to halving it.
  3. Share one name store between the tree and the index.
- The app's scan takes twice the bare walk. Likely the live map and progress updates; worth one profiling session.

Not measured: elevated MFT scans, idle CPU (your own open windows used 0.00 s of CPU over 10 s, so no problem was seen), start-up time, frame times of the map.

## 4. The rule engine: how it decides, and how to make it smarter

### How it works today

Before any model is involved, `advise_all` (`adv/lib.rs:108`) builds findings from seven sources, then passes every one through `safety::check`:

1. About 30 built-in rules: fixed paths with a risk and a remedy.
2. Cleaners: per-app parts with fixed paths.
3. Unused apps: over 1 GB and no start in 180 days (Steam, Prefetch).
4. Build outputs: folders such as `target` or `node_modules`, proven by tool marker files, in projects whose own files have not changed for 30 days.
5. Anomalies: 14 well-known folders at more than twice a hard-coded "usual" size.
6. Tips: compress, OneDrive online-only, move to another drive.
7. Caches "by shape": `Cache`, `Code Cache`, `GPUCache` directly under an unknown AppData app.

It is a path matcher with good safety checks. It is cautious, explainable and fast (1.5 s on 3.7 M items). It is not yet smart: every decision is a fixed path or a fixed threshold, and each finding is all-or-nothing.

### What it did on this disk

`advise` on C: found 72 findings, "98.5 GB in total". Looked at closely:

| Observation | Evidence |
|---|---|
| The largest reclaimable area was dropped entirely | "Temporary files (56.5 GB)" was held back because one `.pst` file sits inside it. In the app this is a small "1 held back by the safety checks (show)" link |
| 106 GB of build outputs were hidden by one threshold | 544 outputs, 109 GB. 57 outputs holding 105.9 GB belong to projects changed in the last 30 days and are not shown at all. Offered: 2 outputs, 353 MB |
| The headline is inflated by things that are not deletes | Seven of the top ten rows are "Compress <folder>" tips listed at the folder's size (7.9 GB for an estimated 2.1 GB gain), marked green. WinSxS is counted at 17.4 GB |
| "By shape" works and is the most general part | It found caches for 20 or so apps no rule knows (Claude, GitHub Desktop, Playnite, Joplin…), all amber |

So the engine reported 98.5 GB, could act on a few GB, and stayed silent about roughly 160 GB that a person would look at first.

### Making it smarter

In order of value.

**1. Partial findings, not vetoes.** When a protected item sits inside a target, keep the finding and carve the item out: "Temporary files: 56.5 GB, 1 protected file kept". The executor already skips never-list names at run time (`adv/clean.rs:231`) and removes links as links; it needs the same skip for never-list patterns such as `*.pst`. The same applies to "links inside": the pre-check refuses the whole folder although the executor handles links safely. Keep the veto only where the *target itself* is protected.

**2. Size by what the action will remove.** For pattern and keep-days parts, compute the size with the executor's own test. That needs one decision: either record creation time in the scan, or stop treating creation time as "new" in the executor. Until then, count at review time by listing the folder, and never show a number the run cannot deliver.

**3. Grade build outputs; do not hide them.** Replace the single 30-day gate with an age per output (newest file inside it) and show active projects' outputs as an unticked group, "Build outputs of projects you are working on", largest first, with the rebuild cost. Go one level down where the tool has structure: `target\debug` against `target\release`, `incremental`, per-configuration `bin\Debug`. On this disk that turns a hidden 106 GB into a list the user can choose from.

**4. One generic Chromium/Electron recogniser.** A folder that holds `Local State` or a `Network\Cookies`/`Cookies` file beside `Cache` is a Chromium profile. That identifies Discord, Slack, Teams, Claude, VS Code and dozens more with certainty, and gives a fixed safe set (`Cache`, `Code Cache`, `GPUCache`, `DawnCache`, `GrShaderCache`, `Service Worker\CacheStorage`, `Crashpad\reports`) and a fixed never set (`Cookies`, `Local Storage`, `IndexedDB`, `Login Data`). It replaces a growing list of per-app cleaners, lifts by-shape results from "check it yourself" to green, and is exactly the knowledge privacy cleaning needs later.

**5. Rank by expected gain and separate advice from deletes.** A tip's size is its estimated saving. Tips, information and deletes are three groups. The headline counts only what Sprawl can remove now.

**6. Use the evidence it already has.** Every finding carries `confidence`, fixed at 0.9 for build outputs and never read. Compute it from signals already gathered (path rule matched, marker files present, owner installed, last start, age of newest file, process running) and use it for ordering and for what may be green.

**7. Leftovers of apps that are gone.** The inventory knows what is installed. An AppData or ProgramData folder whose name matches no installed app, untouched for months, is a strong amber finding: "left behind by an app that is no longer installed". This is the useful half of an uninstaller without becoming one.

**8. Replace hard-coded "usual sizes" with the user's own history.** Snapshots already exist. "This folder grew 20 GB since last week" is better evidence than a constant.

**9. Say what was held back when it is big.** A held finding over 1 GB deserves a row with the reason and the one blocking item.

**10. Widen the never-list before widening anything else.** Add messengers and password managers (Signal, Telegram, WhatsApp, Bitwarden, 1Password, KeePass folders), Opera, Vivaldi and other Chromium browsers, and mail stores besides `.pst` (`.ost`, Thunderbird outside the default path).

## 5. The features as they are

**Tools (no AI).**

| Feature | State | Comment |
|---|---|---|
| Scan, tree, files, types, treemap | Strong | Fast, complete, legible. The reason to install Sprawl |
| Clean-up engine | Strong, with two honesty defects | Proven safe in the test; fix SAF-1 and sizes |
| Advisor findings | Cautious to a fault | See section 4: trustworthy, but it misses the big things |
| Cleaners | Good frame, thin content | 16 apps, uneven browser coverage, no privacy parts yet |
| Search | Fast matcher, rough edges | Crash path (SAF-2), 33 s sort, heavy memory, limited query language |
| Filter | Works | Overlaps Search |
| History and Growth | Useful | Forecast from two points; cannot be cleared; cap not enforced |
| Duplicates | Careful | One row per set, pointing at an arbitrary copy |
| Script export | Useful for IT | Exports more than was ticked (SAF-5) |
| Updates | Good | Signed, reviewed earlier |

**AI.** One live test: your own provider (Claude Code, Haiku), the Cleaners question "Suggest what to clean", throwaway profile. It answered in under a minute, sensibly and honestly, and said outright "I can't change the ticks from here".

- **With Claude Code, chat has no tools** (`adv/llm/agent.rs:80`). "Suggest what to clean" cannot produce the "Tick them" card, cannot propose a cleaner, set a filter or look anything up. Claude Code is the provider the wizard suggests first and the one you use, so the Cleaners AI feature built last is unreachable on the default path. Either give Claude Code chat the tools through the same MCP bridge the analysis uses, or hide tool-dependent chips for that provider.
- **The model repeats the engine's numbers.** It recommended the "old logs, 3.8 MB" item that frees nothing. Better inputs make better advice.
- **The answer pushed the cleaner list off the screen.** The chat card sits above the list and grows.
- Where AI adds real value: naming unknown folders, and proposing cleaners for apps the rules do not know. Where it adds little: choosing among items the rules already rated.
- Still untested: the API providers, `suggest_ticks` end to end, `propose_cleaner`, web lookup, the full analysis run.
- The safety and disclosure findings from the first report stand (SAF-3, SAF-6, SAF-11, AI-4 to AI-7).

## 6. UI: redesign or improvements?

Yes, I looked at it this way. The analyser screens need no redesign. The cleaning side needs a restructure, not a repaint, because the problems are structural:

- **Advisor and Cleaners are one panel shown twice**, with the same foot. The findings list and the cleaners list show the same items.
- **The list gets the least room.** At 1680×1080 the Advisor shows four findings and Cleaners shows two, under a summary, a bar, five sub-tabs, six chips, an AI card, and above a foot and a map that keeps the lower third.
- **Two search-like boxes** (Filter, Search) sit side by side.
- **Row tags look like buttons** ("Delete", "Run a command", "How to").

Recommended shape (option B in the first report, now without an Apps view): **Explore** (tree, files, map), **Clean** (one list grouped by app, then other findings; details in a side drawer; AI in a drawer when it is on; map collapsed), **Search**, **Over time**. One Settings page.

Improvements that need no restructure, in order:

1. Collapse the map on Advisor and Cleaners.
2. Remove the AI card from the top of Cleaners; one "Ask AI" button.
3. Nothing ticked on arrival; "selected X of up to Y".
4. One search box with a "narrow the analysis to this" action.
5. Plain cost words per row and a source tag for anything not built in.

Layout defects seen in the running app:

- The Animations switch in Colours and Tweaks is clipped.
- The "Searching this scan, D:\, E:\…" strip is drawn over the last result row.
- The review list clips its fifth item.
- A chat answer in Cleaners pushes the list out of view.

## 7. Search: powerful, without competing

Fix first: SAF-2 (crash or wrong target on Enter), the Folder sort, drive chips that do not cancel, the 100,000-row cap taken under the wrong sort.

Then make it powerful:

- **Query language:** `ext:`, date filters (`dm:>2026-01`, `dm:lastweek`), `path:`, regex (`regex:`), `size:` ranges, exact match, case switch. A help panel in the empty state.
- **Results:** Enter opens, Ctrl+Enter shows in the tree, Esc returns to the box; multi-select with Copy Path; a path column; saved searches.
- **Scope:** "This scan" by default; other drives opt-in per drive and remembered; each with its read time and a Refresh.
- **Memory:** compact records for search-only drives (section 3).
- **Freshness:** keep the "changed since the scan" mark; survive deletes; say plainly that new files need a rescan.

A persistent index and USN replay are no longer needed for this goal.

## 8. Privacy cleaning: how to add it safely

Prerequisites, all from the first report: `closed = "must"` refused without process names (SAF-8); the community filter hardened or community privacy parts refused outright (SAF-9); no pre-ticking (CLN-1); honest sizes.

Design:

- A separate list, off until switched on in Settings, never ticked for the user, never part of the space total.
- Each part states the consequence, not the mechanism: "Signs you out of every site in Firefox", "Forgets which files you opened recently".
- Browsers must be closed, checked twice, as the engine already does for must-closed parts.
- Start with whole-file parts (cookies, history databases, Recent items, jump lists, thumbnail cache). Editing browser databases stays deferred, as you decided.
- Per browser profile, listed by name.
- No "secure" or "unrecoverable" wording: on an SSD a deleted file cannot be guaranteed gone.
- The Chromium recogniser (section 4, item 4) supplies the paths for every Chromium-based browser at once.

## 9. Settings: what is missing

Beyond merging the six places into one page (first report, section 6):

| Missing setting | Why |
|---|---|
| Never touch (exclusions list) | "Keep" hides a suggestion but protects nothing (SAF-4) |
| Tick safe items automatically (off by default) | Makes today's behaviour a choice |
| Send my own files to the Recycle Bin | The only undo available |
| Folders to skip when scanning | Speed and privacy; none exists |
| Search: which drives, remembered | Today it is all or session-only |
| History: size on disk, Clear, on/off | Cannot be cleared today |
| AI: request log Open and Clear; Forget kept results | Logs hold paths and cannot be cleared |
| Work offline (no update check, no community download, no AI) | One switch for the privacy-minded |
| Thresholds: idle days for projects, unused days for apps, smallest finding | Hard-coded today |
| Privacy cleaning on/off | Needed for section 8 |
| Size units: binary or decimal | Explorer and drive makers disagree; labels say GB for GiB |
| Reset all settings; open the data folder | Recovery and support |
| After a scan: stay, or open Clean | Helps beginners find the Advisor |

Not needed: profiles, import/export, schedules.

## 10. Other things found along the way

- **Tests pass:** 113 tests in `sprawl-core`, `sprawl-advisor` and `sprawl-install`, 2 ignored. The app crate's tests were not run.
- **The app's own writes in screenshot mode:** settings are read-only, but the advisor still wrote `memory.toml` and the clean-up log. Harmless here; worth knowing for dev runs against your real profile.
- **Docs are stale:** README "Status", `docs/plans/search.md` ("nothing here is built"), `docs/plans/ai-advisor.md` ("nothing is executed by Sprawl"), `docs/uninstaller-handoff.md`, and the review queue's standing rule that the app never deletes from the Advisor.
- **First real clean-up:** done today, on fixtures. It should be repeated once on a real profile in a VM before privacy parts ship.
- **Two files added by this audit, untracked:** `crates/sprawl-core/examples/bench_search.rs` and `crates/sprawl-advisor/examples/gates.rs`. Keep them as regression benchmarks or delete them.

## 11. Roadmap

Each phase ends with the Fable review you already require, a version bump and a changelog entry.

**Phase 0: safety and honesty (small changes, do first).**

1. SAF-2: guard Enter/Down in the search box.
2. SAF-1: keep files newer than the scan, and say how many were kept; or remove the sentence now.
3. SAF-3: no stored key to an unchecked address; load models on request.
4. SAF-4: kept paths are skipped by the executor.
5. SAF-5: the script exports ticks only.
6. SAF-6: no Claude Code, and no PATH-resolved tools, when elevated.
7. Defaults: search every drive off; community download off; wizard AI page starts on Off; no automatic ticks.
8. Wording: disclosure text, "safe to delete", amber on Recycle Bin and Windows.old, the Uninstall label and doc, log times in local time, "from 5 items".
9. REL-1: catch a panic in the analysis thread.
10. SAF-7, SAF-8, SAF-10.

**Phase 1: quick performance wins.**

1. Folder sort: keyed, then ranked, then off the UI thread.
2. `set_tree`, filter and snapshot encoding off the UI thread.
3. Typing delay from 140 ms to about 50 ms.
4. Profile the app's scan against the bare walk.

**Phase 2: a smarter engine.**

1. Partial findings (carve-outs for protected items and links).
2. Sizes by action; review-time listing.
3. Graded build outputs.
4. Chromium/Electron recogniser.
5. Ranking by gain; tips and information separated; headline counts actionable space.
6. Leftovers of removed apps; history-based growth findings.
7. Never-list additions.

**Phase 3: Clean workspace and Settings.**

1. One Clean workspace; map collapsed; details drawer.
2. One Settings page with the missing settings.
3. AI as a drawer; tools for Claude Code chat, or chips hidden for it.
4. One search box with a filter action.

**Phase 4: search power.** Query language, result actions, opt-in drives with compact records, saved searches.

**Phase 5: privacy cleaning.** After phases 0 and 2, as in section 8.

**Later.** Accessibility (list semantics, names, themes); tree memory layout; duplicates as sets; history cap and Clear if not done in phase 3.

**Still to verify.** Elevated behaviour (MFT timings, admin clean-up, pipe reachability); a real-profile clean-up in a VM; the API providers and `suggest_ticks`; what the community import really contains; Steam and Epic web caches and sign-in.
