# Next round (after 0.3.1)

Written 2026-11-18 from the user's list. It ties together `future-features.md` (F1-F6),
`advisor-next.md` (A1-A6), the new requests (R7-R11) and the MCP report. The Progress section
below records what has since been built and released; the phase descriptions retain the design.
**Current status, 2026-11-19:** the agreed continuation shipped in Sprawl 1.0.0. GPT Astra
read the complete review dossier and code, verified all 33 findings Fixed/Done and approved
the release. See [current status and next work](status.md) for remaining ideas, verification
limits and the newly commissioned design/AI review. Dated progress notes below are historical.
The 1.0.1 corrections (Claude, after the user found 1.0.0 slow) are the last Progress entry;
their limitations and open questions are in status.md.

## Remaining work at Codex takeover (2026-11-18)

- **A2 recognition:** installers of installed programs (MSI first, then separately verified
  support for EXE installers); launcher leftovers next. Other kinds in `advisor-next.md`
  need evidence and an honest remedy before they can become actionable findings.
- **A4 goal planning:** ranked options with their costs, and guided moves through supported
  launcher/Windows workflows; show destination capacity, never invent a move command.
- **A5 prevention:** useful lines after cleanup about keeping the same space from growing back.
- **A6 learning:** use Keep/Not now/cleanup history to improve advice, then user-approved AI
  recognisers through the existing proposed-cleaner validation and safety checks.
- **Deferred/smaller:** A1 launcher readers await real data; decimal units, scan exclusions,
  narrow windows, and F1's compressed search keyword/tree mark remain recorded work. Recent
  items/jump lists/Firefox privacy parts and accessibility stay deferred by prior decisions.

Sol (Codex) maintains implementation and review notes from `e86c346`. Simple tasks are to be
delegated to Luna and checked by Sol when that model is available through the agent tools.
A2 now recognises MSI packages, metadata-matched EXE installers and Steam library leftovers.
A4 goal comparison, A5 prevention, A6 learning and the smaller items are implemented;
see Progress for the original verification and limits. Astra's review and the 1.0.0 release
are now complete; historical "pending" entries below retain their original dates.

## Phase 0: bugs first (0.3.2)

| # | What | What's known |
|---|------|--------------|
| B1 | **MCP: `scan_report` always says the rules found "(nothing)"** | Confirmed in code. `mcp.rs` `answer_bridge` and `Local::call` both pass `&[]` as the findings to `llm::report::build`, so the rules' findings never reach that section. In the bridge, pass the Advisor's findings: the app already shares them as text, so share the structured list too. In snapshot mode, run the rules in-process or say "rules not run here". |
| B2 | **MCP: scanning in the app isn't seen until the MCP restarts** | Confirmed in code. `mcp::main` tries the app's pipe once, at start. If the app isn't running then, the server stays on History's newest snapshot for good, and after the app restarts the pipe answers "Sprawl closed the connection". Fix: reconnect on every call, and fall back to a snapshot only when the app is really absent, saying so in the answer. |
| B3 | **Advisor: expanding a row is slow, and opening one lower down jumps the list to the top** | From the A6 note. Likely the whole list is rebuilt on expand. Measure first, then update only that row and keep the scroll offset. |
| B4 | **"Delete permanently" looks like Windows 95** (R8) | It is `MessageBoxW` (`win32.rs` `confirm`/`info`). Replace it with a WinUI `ContentDialog` in Sprawl's style, keeping the default on "No". Also check the other `MessageBoxW` uses. |
| B5 | **File View sorting is slow again** | The user says it used to be instant after one release's optimisation and now takes a second or two. Time a sort on the same scan in 0.2.x, 0.3.0 and 0.3.1 and bisect. Suspects: `tables.rs` `sort_nodes` (the rank path only covers some keys), a full rebuild of the virtualised rows after the sort, or the 0.3.0 activity indicator or the per-frame `bridge::publish_tree`. Add a timed regression check next to `bench_search`. |

R11 (an issue in the Cleaners tab) is dropped, by the user's word.

## Phase 0b: bugs the user found after 0.3.2 (before Phase 2)

| # | What | What's known |
|---|------|--------------|
| B6 | **"Check again" doesn't see what was removed** | The user tried it several times: after a delete, or after a command ran (emptying the Recycle Bin), the same findings come back. Suspects: "Check again" reruns the rules over the same tree, and the tree isn't rescanned where the clean-up or command happened. Look at what "Check again" does, then rescan (or re-measure) the paths behind each finding before the rules run, the way a clean-up already updates the tree for what Sprawl itself deleted. |
| B7 | **History: "Play it in History" seems to do nothing** | The button should switch to the History view, and then play. Today it starts something the user can't see. |
| B8 | **History: Play moves one step and stops** | It should step through the snapshots to the end (or until paused). |
| B9 | **History: scrubbing the timeline freezes the app for almost a second each time** | Each seek likely loads (decompresses) a full snapshot and projects it on the UI thread. Load on a worker, keep the snapshots already loaded, show the nearest one loaded while the next comes. |

## Phase 1: the MCP as a real tool (0.4.0), to discuss before building

The user asked for this design to be discussed first. Today the MCP is query-only, works on one
tree (the app's screen or the newest snapshot), and says "isn't in the scan" for anything else.

**Proposed design**

- **Who scans.** If the app is running, it scans: the MCP asks over the pipe, and the scan shows
  on screen, uses the MFT when elevated, and lands in History. If the app isn't running, the MCP
  process scans in-process with `sprawl-core` (folder walk, or the MFT if elevated), the same engine
  as F5's `--report`. That makes F5 and this one piece of work.
- **Many roots, not one.** An index of scanned roots: the app's open tree, trees the MCP scanned
  this session, and History's newest snapshot for each root (with its age). Every query tool takes
  an optional path and picks the tree that contains it, choosing the freshest.
- **Tools:**
  - `scan_path {path, rescan?, wait_seconds?}` starts or refreshes a scan and returns a job id. With
    `wait_seconds` it blocks up to that long, so a small folder answers in one call.
  - `scan_status {job?}` reports the stage, files and bytes so far, done or failed, and the root.
  - `list_scans` lists each root with its source (app, MCP or History), its age, and whether it
    is complete.
  - The existing tools answer an unscanned path with "not scanned: call scan_path". A stale one
    gets "scanned 3 days ago".
- **More power for the same price:**
  - `search {query}` exposes the Search tab's language (`ext:`, `size:`, `dm:`, `regex:`).
  - `growth {path, since?}` diffs against History.
  - `findings` and `cleaners` return structured rows (path, bytes, rule id, risk, confidence,
    remedy). The answer says "rules ran, none matched" or "rules not run for this tree", never a
    bare "(nothing)".
- **Limits that stay:**
  - Still read-only: nothing deletes. Ticking stays in the app, through `suggest_ticks`, which the
    app replays through its own checks.
  - One scan at a time per process.
  - A scan of a network path is refused.
  - A scan the user didn't start doesn't replace the tree on the user's screen unless they open it
    (the app keeps it as a background tree).
- **Decided by the user (2026-11-18):**
  - An agent's scans go into the app's History.
  - The MCP elevates only as the app's settings say. It follows "Always run as administrator"
    and the admin switches in HKLM, and never asks for elevation itself.
  - In snapshot mode, the rules run in-process, so findings are real everywhere.

**Done (2026-11-18), as far as the user is concerned**: steps 1-3 below in Progress. Left out on
purpose: `cleaners` as rows (its text already carries id, risk and counted bytes) and a
`scan_report` that leads with the scans available. Also decided then: a scan an AI asks for runs
with the user's normal rights only. The app, if it runs as administrator, hands the scan to the
server's own process, and the server uses the folder walk, never the MFT.

## Phase 2: the Advisor and the AI working with you (0.4.x)

- **R7, "Look first" rows can be ticked too.** Keep "Look first" as the label and the advice, but
  add the tick box. A ticked "Look first" row goes to the review in its own group with "you haven't
  looked at these", and goes to the Recycle Bin, not a permanent delete.
- **R7c, findings that aren't safe can still be ticked, on the second try** (the user,
  2026-11-18; not in yet: today only Delete and Command remedies below Red can be ticked). The
  first click on a "Look first" or other untickable row says why it isn't ticked; a second click
  ticks it. It then goes to the review in its own group and to the Recycle Bin. Red (Windows
  itself, what breaks apps) stays untickable unless the user decides otherwise.
- **R7, run "Command" remedies from the app.** Run the shown PowerShell or the owner's tool from
  the review, the same way the Cleaners' tool commands run:
  - an allow-list of known commands only;
  - the command is shown in full;
  - its output is captured and shown;
  - never elevated unless the admin switch allows it;
  - each run is a line in the clean-up log.
  - Fable review.
- **R9, accept the AI's suggestions.** Its findings and its chat suggestions get "Tick this" and
  "Tick all it suggested". Each goes through the same checks as `suggest_ticks`, which means safe
  and amber only and replayed against the scan. AI-ticked rows have their own colour (the purple
  of the existing "AI" badge) until the user touches them.
- **R9b, the AI on the Findings page, like the Cleaners tab** (the user, 2026-11-18). The AI's
  recommendations can be ticked in place, as Cleaners' parts are now. The AI moves into a column
  on the right of the Findings page, as on the Cleaners tab, and the separate Ask tab goes. One
  design for both pages.
- **R12, smarter and more suggestions in the AI box.** More of the ready questions (chips), chosen
  from what is on screen: the selected finding or part, what is ticked, what grew, an app that
  is open. (R10's "Are my choices safe?" is one of them.)
- **R10, "Are my choices safe?"** A chip in the Ask box.
  - The AI gets a tool that lists what is ticked now (Findings and Cleaners, with risk, size,
    paths and "ticked by you / by the AI").
  - It answers per item.
  - It can only advise: unticking stays the user's.
  - "Make sure the AI can actually see what I did" becomes a test: the answer must quote the
    ticked items it was given.

## Phase 3: space features (0.5.x), order as in `future-features.md`

- **F1 Compress in the app.** XPRESS/LZX through `compact.exe` or WOF. Sprawl remembers what it
  compressed, offers "Compress again" after an update, and adds a "Compressed" filter. Fable
  review.
- **F6 Low-space and growth alert.** Needs a light background presence: a scheduled task, not a
  resident process. Pairs with F5.
- **F3 Shrink virtual disks.** WSL `.vhdx` with `wsl --shutdown` then `Optimize-VHD` or
  `diskpart compact`, and Docker's disk. Only with the owner stopped. Fable review.
- **F2 Old versions side by side.** .NET SDKs, Android images, Node, Python and Rust toolchains,
  each removed through its own tool (`dotnet-core-uninstall`, `sdkmanager`, `rustup`, nvm and
  others).
- **F4 Duplicates resolved by linking.** Hard links within a volume only. This needs A3 to find
  the duplicates first. Fable review.

## Phase 4: a smarter Advisor (0.6.x), from `advisor-next.md`

- **A1 Evidence of use.** Last-played times from Epic, GOG, Xbox, Ubisoft and EA, and Windows'
  record of opened files, so a row reads "last played", not "not changed since".
- **A2 Recognise what things are.** An archive next to its extracted copy, installers of
  installed programs, folders left after a game is uninstalled.
- **A3 Duplicates from the scan.** Size buckets, then a hash of the first and last parts, then a
  full hash. This feeds F4.
- **A4 A plan for a goal.** "I need 100 GB on C:" gives ranked options, including moving a game
  to a drive with room, through the launcher's own move where it has one.
- **A5 Prevention tips** after a clean-up.
- **A6 Learning from your choices**, and the AI writing reusable recognisers (as proposed
  cleaners, checked like community ones).

## Smaller items, slotted in where they fit

- **Decimal units** (GB vs GiB) and **folders to skip when scanning**: both belong in Settings.
  Phase 0 or 1, since they are small.
- **Narrow windows:** tree columns cut names at about 1120 px. Phase 0.
- **Accessibility** (screen readers, light and high-contrast themes): not now, maybe never, by
  the user's word.
- **Recent items, jump lists, Firefox history** as privacy parts: still set aside.

## Order

The user says the order doesn't matter. Phase 0 still comes first, since those are bugs, and Phase 0b (B6-B9) comes before Phase 2 for the same reason.

## Progress

Kept up as the work goes. Each line: the item, the commit, what was done, what is left.

- **B1** (8e6021f): done. `report::build` takes `Option<&[Finding]>` and says "none matched" or
  "haven't run". The app shares the Advisor's findings as a list over the pipe
  (`bridge::publish_findings`, `None` while a new analysis runs). Snapshot mode runs the rules
  in-process (`mcp::rules_for`: built-in plus checked local packs, through `advise_all`). The
  in-app chat's report gets the Advisor's findings too.
- **B2** (8e6021f): done. The MCP connects to the app for every call (`mcp::ask_app`), waits up
  to 2 s on a busy pipe (`pipe::Client::connect`), and falls back to History's newest scan
  (`Snapshot`), reloaded when a newer one appears, with "Sprawl isn't running ... taken N ago"
  on every answer. `cleaners` and `set_filter` need the app open. Tried by hand with no app
  serving: scan_report lists the rules' findings. Not yet tried by hand: the app started after
  the MCP (needs an installed build, since the pipe only answers the same program file).
- **B3** (cdfb66c): done, tried by hand. Measured with the new `SPRAWL_TIMING=1`
  (`%TEMP%\sprawl-timing.log`): each open rebuilt the whole panel (~25 ms) plus a 35-65 ms
  frame. Rows now sit in `AdvRow{i}` Borders; open, close and the in-use answer swap only
  those rows (`advisor_rows`, 2-5 ms), anything else falls back to the full render. The jump
  to the top was focus falling to the first control when the clicked button was destroyed: the
  clicked row's button now takes focus back. Tried by hand (2026-11-18, `SPRAWL_FRONT=1` screenshot mode on a C:\ scan): rows
  lower in the list open and close in place, 2-3 ms each, and the list stays put.
- **B4** (c56e6be): done. Not WinUI's `ContentDialog` but Sprawl's own card over the window
  (`ask.rs`, `DlgHost`): the same look without new bindings or async code. Every `MessageBoxW`
  is gone (`win32::confirm`/`info` removed). "Yes" is a callback that checks again (delete:
  same `tree_gen`, node at the same path, `delete_refusal` for each, all before any goes).
  The question owns the keyboard while open; menu commands wait. Tried by hand on a throwaway
  folder: the card shows, the mouse and Esc (with the window in front) say no, nothing was
  deleted. Not tried by hand: Tab/Enter, Esc right after the window comes to the front (the
  refocus added last), "Yes" deleting for real, closing mid clean-up.
- **B5** (e3a1e19): done. Not a single regression: File View never had the rank sort (68558d1
  gave it to Search and the tree only). Timed with `cargo test --release -p sprawl
  bench_files_sort -- --ignored --nocapture` on History's newest C:\ scan (3.1 M files):
  name 4.5 s -> 0.1 s, allocated 3.6 s -> 0.2 s, modified 1.8 s -> 0.3 s, folder 97 s -> 0.2 s.
  The name and folder places (`tables::Ranks`) are made on a worker once the tree shows
  (2.5-4 s on C:\); a click before then waits for them. The bench checks every column's order
  against the old comparison. Not tried by hand in the app (no header-click step in
  screenshot mode); `SPRAWL_TIMING=1` logs "File View sort" for a hand check.

Phase 0 is done (B1-B5). Next per the plan: the release of 0.3.2 waits for the user's word and
the Fable reads queued in `.claude/fable-review-queue.md`; Phase 1 (the MCP as a real tool) is
to be discussed with the user before building.

- 2026-11-18: **0.3.2 released** (cd43053) with Phase 0. Fable reads still queued, B4 first.
- 2026-11-18: **Phase 1 step 1** (5cf92d2): `mcp_scan.rs`: scan_path / scan_status / list_scans in the MCP process, History save per the app's setting, query tools pick the freshest tree holding the path (own scan, app, History). Tried over stdio with the release exe. Next: route scan_path to the running app (scan on screen, background tree), then search, growth, structured findings/cleaners.
- 2026-11-18: **Cleaners tab scroll fix** (2f9c8b7 on main; baf3923 = v0.3.2's tag): the tab jumped to the top on every open or tick (the user's original B3 report was about this tab, not the Advisor list). The 0.3.2 release was amended at the user's word: same version, files replaced, tag moved. Copies that updated to the first 0.3.2 don't get it through the updater; 0.3.3 will carry it to them.
- 2026-11-18: **The amended 0.3.2 couldn't start** for about an hour: built in a worktree without `sdk\`, so without the Windows App Runtime bootstrap. Rebuilt with `sdk` linked before the first build, checked that it starts, files replaced again, the user's install repaired. The release tool now refuses such an exe (6b15c26).
- 2026-11-18: **Phase 1 step 2** (8c96b0b): the running app makes the scans servers ask for (background tree, its rights, History); the server scans by itself only without the app. Tried live (screenshot-mode app, MCP over stdio). Next: search, growth, structured findings/cleaners.
- 2026-11-18: **Update check at every start** (5d61eb0), and once a day while open. Asked by the user.
- 2026-11-18: **Phase 1 step 3** (7df1766): `search` (the Search tab's language over the scan holding the path), `growth` (a folder against History's scan nearest N days ago: then and now, what grew, shrank, came or went), `findings` as rows (risk, rule, what Sprawl would do, confidence, bytes, path; the app publishes the same rows). Tried over stdio with the release exe. Phase 1's list is done except cleaners as rows (the app's text already carries id, risk and counted bytes) and a `scan_report` that leads with the scans available; next: decide with the user whether Phase 1 is complete, then Phase 2.
- 2026-11-18: **AI scans with the user's rights** (35daa45): folder walk only; an elevated app hands the scan to the server's process. Plan got Phase 0b (B6-B9), R7c, R9b, R12.
- 2026-11-18: **B6** (289e347): "Check again" reads each finding's path again on a worker (a file through its folder, nested paths once) and puts it into the tree (`apply_rescans`) before the rules run. Tried in screenshot mode: a 210 MB Steam shader cache deleted outside Sprawl is gone after Check again.
- 2026-11-18: **B7-B9** (07f1033): Play seeks directly (the slider's handler couldn't run from inside a frame: `with` was borrowed, so Play stopped after one step); snapshots are read on a worker with the map on the last one shown meanwhile, Play reads ahead; "Play it in History" switches to Tree View and plays. Tried in screenshot mode over 14 snapshots, no slow frames. Phase 0b is done.
- 2026-11-18: **R9b** (0035057): Findings with a report uses the Cleaners tab's columns: the list left, the AI right (same fold button and setting), each scrolling on its own. The Ask section is gone; asking opens the column. "What the AI found" lists the model's findings with the list's own ticks and "Tick the safe ones it found" (safe and amber, only what the list would let you tick). Tried in screenshot mode with the card forced on for six findings: ticks reach the foot; folding works.
- 2026-11-18: **0.4.0 released** at the user's word ("when you feel like it's worth it, make a release"): Phase 1, Phase 0b, R9b and the startup update check. Before the queued Fable reads.
- 2026-11-18: **R7c** (7991646): a tip or a left-behind folder holding the user's files has a dimmed box; the first click says why it isn't ticked, the second makes it a delete of the whole path to the Recycle Bin (always, whatever the setting) once `safety::check` passes for it as a delete. Red, the AI's findings, privacy parts, other accounts, OneDrive, Windows/programs, drive roots, the profile's own folders and never-list paths get no box. Tried in screenshot mode on a left-behind game folder: the explanation, the tick, the review ("to the Recycle Bin", own-files box). The Edge profile tip was refused by the check and now gets no box.
- 2026-11-18: **R12 + R10** (26abfdf): the AI box's ready questions are built from the screen (ticks, the open row, growth, left-behind folders, then the usual ones; up to three not asked yet stay after an answer). A Findings question carries what is ticked; tried with Claude Code: "Are my choices safe?" was answered for all 18 ticks.
- 2026-11-18: **R9** (b254f24): a Findings question also carries the safe/amber findings that can be ticked, with `finding/N` ids; `suggest_ticks` takes them; the AI column shows the suggestion with a tick each and "Tick all it suggested"; such ticks are purple until touched. Tried with a dev suggestion (`fnsugdemo`). Phase 2 left: R7 (run Command remedies).
- 2026-11-18: **User's UI round** (a360a17): the top-right name/version/logo card is back (folds under 1250 px), the logo drawn in XAML on it and on About; the keyboard shortcuts box (tabs showed as a broken character since B4) is two columns of grouped rows with key caps.
- 2026-11-18: **0.4.1 released** (R7c, R9, R10, R12, the logo card, the shortcuts box), before the queued Fable reads (R7c is a new way into deletion: first in the Fable report).
- 2026-11-18: **R7** (6e9b49a): Command remedies already ran from the review (built-in pack only, through `clean::action`; hidden, output shown, admin switch for elevation). Added: the command and its last output lines in `cleanup-log.txt`, and the review shows the command whole. **Phase 2 is done.**
- 2026-11-18: **F1 step 1** (143218d): the compress tip is a tickable command Sprawl writes (`compact.exe ... /exe:xpress16k|xpress8k`), measured on disk, never auto-ticked; WOF-compressed files leave the estimate. Next: F1 step 2 (a record of what was compressed, "Compress again", Uncompress, the Compressed chip).
- 2026-11-18: **F1 step 2** (b1a4eea): the record of compressed folders in memory, "Compressed (n)" under Show with Uncompress (asked, admin for Program Files) and Forget. Left of F1 for later: the `compressed:` search keyword and a mark in Tree View.
- 2026-11-18: **F6 step 1** (b4eb76c): "Tell me when a drive is nearly full" (General, off): a scheduled task of the account runs `sprawl --check` every 4 hours; one notice per drive per week below the limit, a click opens Sprawl there. Growth alerts wait on the user's answer (own scans only, or a scheduled folders-only scan).
- 2026-11-18: **F6 step 2** (220217e): the user chose own scans by default with a daily scan as an option. "Tell me when a folder grows fast" (off) compares each new scan in History with about a week before; "Also read my user folder once a day" (off) adds a folders-only walk of the profile at background priority, saved to History. Dry run on this PC's History named four folders. F6 is done; next F3.
- 2026-11-18: **F3** (c2e0bad, 3ca6ea7): the WSL and Docker disk tips are a tickable Shrink (built-in rules only): Docker Desktop must be closed, `wsl --shutdown` and a check that WSL is gone, then diskpart `attach vdisk readonly` / `compact vdisk` / `detach vdisk`, as administrator. Win shown as "unknown" until it ran; freed is what the file lost on disk. Checked: tests (conversion, safety, elevated-tool check, PowerShell parse of odd paths), the row and the review on a fake sparse disk. NOT run for real: no WSL on this PC and diskpart needs administrator. Hyper-V, VirtualBox and VMware disks stay tips. Next: F2.
- 2026-11-18: **F2 steps 1-2** (8688ec9): `versions.rs`: older rustup toolchains (not default, override or pinned) and nvm/fnm Node versions (not active, newest or pinned) as amber "Remove version" rows that run the toolchain's own uninstall. Pins read from rust-toolchain(.toml), .nvmrc, .node-version of projects in the scan. Seen on this PC: Rust 1.99.1 offered when only .rustup is scanned (a project in Documents pins it, so a C: scan hides it). Volta has no per-version uninstall: left out. Next: F2 step 3 (Android SDK), 4 (.NET, Python hand-off), 6 (driver leftovers).
- 2026-11-18: **F2 step 3** (5906af5): older Android SDK build-tools, platforms, NDKs and emulator images, removed by `sdkmanager --uninstall`. Kept: the newest of each, what a Gradle file in the scan asks for, any image an AVD uses. On this PC: 4 build-tools and 2 platforms offered, 37.0.0 and android-36.1 kept, both images kept (two AVDs use them). Next: step 4 (.NET, Python hand-off), step 6 (driver leftovers).
- 2026-11-18: **F2 step 4** (23f5c8e): older Python (python.org, found by its PEP 514 registration; kept when a .python-version or a venv's pyvenv.cfg in the scan uses it) and .NET (an older patch of the same major.minor; an older SDK no global.json pins; never another major) as Uninstall rows that open Installed apps. Fixed on the way: the app list dropped every uninstall entry without a folder (the Steam-duplicate filter's `all` of nothing). On this PC: Python 3.12 offered (151 MB). Step 5 (references from projects) is done inside steps 2-4. Next: step 6 (driver leftovers).
- 2026-11-18: **F2 step 6** (d4efcd4): built-in rules for C:\NVIDIA, C:\AMD and C:\Autodesk setup leftovers (amber deletes, 30 days, size floor). Checked by the rule tests only: a full C: scan to see them was stopped by the user. C:\Intel left out. F2 is done. Next: F4 (duplicates by linking, needs A3) or Phase 4.
- 2026-11-18: **A3** (a8c57ba): "Probably N copies" tips from the scan alone, nothing read: files of the same size and name ("film.mkv", "film (1).mkv", "- Copy") over the duplicates floor, and whole folders with the same names and sizes all the way down over 1 GB (a copied game). Find duplicate files replaces the file tips with its read results; folder groups stay. Checked: tests, and the Advisor on a small test folder. Next: F4 (duplicates by linking).
- 2026-11-18: **F4** (05a4fbc): Find duplicate files now offers "Link copies" on a confirmed group with no protected copy, all on one drive, none in OneDrive: the plainest-named copy is kept, every other copy becomes a hard link to it, compared again byte for byte just before with both files held against writes. Nothing deleted or recycled; the review and the own-files box say so, and that a change to one then changes all. Checked: tests (a real link made, a copy changed after the search left alone, forged findings refused) and the row and review in the app on a small test folder. Phase 3 is done. Next: Phase 4 (A1).
- 2026-11-18: **A1 step 1** (e6f4d99): old-and-big rows use evidence of use: the owning app's last start or play (Steam, Prefetch, UserAssist, already gathered) and Explorer's Recent items (each shortcut's target read from its LinkInfo; 156 of 231 on this PC give a path). Use within the year keeps a row off; older use is the title ("last played March 2025") with its source. Checked by tests. Next: step 2 (last-access times where the volume keeps them), step 3 (other launchers' last-played, after checking their files).
- 2026-11-18: **A1 step 2** (4edbb6a): last-access times, only where this PC plainly keeps them (NtfsDisableLastAccessUpdate 0 or 0x80000000; this PC: on) and on NTFS, read from folder listings within 200,000 entries: opened within the year hides an old-and-big row, opened after its last change titles it "last opened", none since says "nothing changed or was opened since". Checked by tests and the real readers on a small folder. Next: step 3 (other launchers' last-played).
- 2026-11-18: **A1 step 3, checked and left for now**: Epic keeps no last-played time on the PC (its manifests have none; play time is in its cloud), and its games already get Prefetch dates by their executables. Ubisoft keeps no play log; its savegames folder would date play, but this PC has one Ubisoft game, under Program Files (never an old-and-big row), and no saves to check a reader against. GOG Galaxy, EA and Battle.net aren't installed here, so their files can't be checked. Build when a PC with them is at hand.
- 2026-11-18: **A2 step 1** (bcdadfa): a .zip of 50 MB or more whose every file is in the folder beside it (x.zip and x\, or x.zip holding x/... extracted where it lies), the same size, is an orange Remove of the archive only. Read from the zip's own index, ZIP64 too; nothing unpacked. Checked: tests (hand-made zips, broken ones, the safety check) and in the app on a zip made and extracted by Windows' own Compress-Archive/Expand-Archive. Next: A2 step 2 (installers of installed programs).

- 2026-11-18: **Codex/Sol takeover; A2 step 2, MSI slice** (a47bac3): MSI packages of at least 10 MB anywhere in the scan can be orange installer findings when their own ProductCode is reported installed by Windows Installer. Protected/app-owned paths, links/cloud placeholders, another account's paths, cached packages and registered source directories are excluded by this recogniser. Nothing is installed or auto-ticked. Reads at most 500 packages, shows at most 50; missing/oversized/failed metadata gives no row. Verified: real MSI-format fixture unchanged after reading, unknown product refused, finding/safety fixtures, read-only native API checks on 32 installed products; release suite 223 passed / 5 ignored, app `cargo check -p sprawl` passed. A test initially depended on USERPROFILE mutated by older tests; its home is now explicit and the full parallel suite passed. No app UI or real cleanup check performed. Not released or pushed; Astra review pending in `.claude/fable-review-queue.md`. **A2 step 2 remains partial: EXE installers need their own verified recognition; launcher leftovers follow.**

- 2026-11-18: **A2 EXE installers and Steam library leftovers** (8ac0ab9): EXEs of at least 10 MB match only an exact registered product and publisher plus installer wording in OriginalFilename and FileDescription, read as version resources without executing the file. Labelled "probably", orange and heuristic: never automatically planned or exported as a script. Steam-style common folders of at least 50 MB absent from a complete, current readable manifest list appear as orange information; a broken/inaccessible manifest suppresses suggestions and other launchers' owned paths are excluded. This is not proof of uninstall, so no removal action is attached. Matching files/libraries outside these supported layouts remain unknown rather than guessed. Tests: identity/publisher/installer/ambiguous cases, safety/plan/export checks, malformed EXE, real Windows version resource, installed/other-owner/broken/empty Steam manifests. Registered games continue through existing owning-app uninstall handoff. No real cleanup or launcher change. Astra pending.
- 2026-11-18: **A5 prevention advice** (9bfe322): successful cleanup items contribute known built-in advice to the result card (npm, pip, Cargo, Windows Temp, Docker, Steam shaders, archives/installers, build outputs). No advice for unknown/local/model/privacy sources; no settings or commands run. Duplicates collapse, at most five tips shown. Verified rendered result at 1120x800 and 1280x900 using a synthetic screenshot-only result (no cleanup); text fits. The screenshot process exits abnormally after capture both with and without this new card, so this is a visual check, not proof of normal shutdown. Full release suite after A2/A5: 227 passed, 6 ignored; app build passed. Next: A4 goal options, then A6 learning and the smaller items.

- 2026-11-18: **A2 self-check fix** (29433b9): EXE installer matching explicitly excludes uninstaller descriptions and unins filenames; "uninstaller" otherwise contains "installer". Regression assertion added and focused tests passed. No broader matching or safety exemption.
- 2026-11-18: **A4 goal alternatives** (fd7f81a): Compare options for the goal: green cleanup, cleanup including amber costs, cleanup plus compression estimates, or cleanup plus registered Steam game moves. Alternatives do not add their totals together; overlapping paths never count twice. Games use scanned allocated bytes for the source gain, logical bytes for destination space, and leave 1 GB headroom. Destination free space is reserved across the proposed moves and volume GUIDs exclude aliases/duplicate capacity. Steam performs moves through Settings > Storage; Sprawl lists the games, destination, planned before/after capacity and cost, without issuing a move command. Existing known-folder Location guidance remains available. Selecting a cleanup/compression option sets eligible ticks only, clears prior privacy confirmations, and goes through the existing review/executor; changing the goal requires comparing again. Core auto-plan now excludes compress/shrink whole-folder sizes and models. Verified: policy test for costs, volume scope, Orange exclusion, overlaps, duplicate games, estimated gain, capacity and aliases; app build; 228 release tests passed, 6 ignored. Scratch UI: 101 MB old Rust output selected through the goal into normal review, 12 MB Orange cache not selected, both fixture files kept; edited goal left nothing ticked and required compare again. Comparison rendered at 1280 and guard/review at 1120/1280 widths. No actual cleanup/compression/Steam move, no provider call. Screenshot exit limitation already recorded under A5. Next: A6 learning/reusable recognisers, smaller items, final roadmap verification; stop before Astra review.

- 2026-11-18: **A6** (868da8b): bounded local cleanup/skip history, Recommended ranking, measured fast-refill cache suppression, explicit Steam-library preference and Forget learning; actions, safety grades and permissions stay unchanged. Automatic selections and goal plans exclude quiet paths, direct manual controls remain. Unexplained folders can be explicitly sent to AI for reusable information recognisers; chat/Claude Code proposals reach the existing approval queue, Orange floor for deletes. Tests: 230 passed, 6 ignored; app build passed, focused final correction passed. Earlier quiet-list UI captured; latest memory/reset captures black, so reset UI remains visually unverified. No live provider call or real cleanup. Sol implemented and reviewed directly because Luna is not exposed to delegation here. Astra still pending. Next: decimal display, scan exclusions, narrow tree and compressed search/mark.

- 2026-11-18: **Remaining settings, narrow tree and F1 search/mark** (e62b252): Settings has decimal display (GB, base 1000) versus binary (GiB, base 1024), and exact absolute skip paths separated by semicolons. Byte counts, query/goal/alert thresholds stay unchanged. App, Search, Cleaners, daily and MCP fresh scans share exclusions and use the folder walker; excluded roots are not entered, excluded entries are not counted. Cleaners skips excluded roots while reading its other roots. Check-again/post-clean rescans honour exclusions. Safety refuses any action whose path or set/link member intersects an excluded scope, including a containing parent; Tree deletion has the same guard. Existing History/CSV remains unchanged. Narrow tree panes below 900 px hide Items/Files/Owner and constrain the folder name within its own column; wider panes restore those columns. `compressed:` / `compressed:no` uses NTFS compression or a scanned WOF tag, on both tree and compact Search indexes; a generic reparse point or small allocated size is not evidence. Tree labels distinguish filesystem flags from Sprawl's historical compression record. No new compression or removal execution.
- **Final implementation checkpoint:** the authorised roadmap continuation is implemented, with the previously agreed deferrals (A1 additional launcher readers pending real data; privacy parts/accessibility; Phase 1 extras intentionally left out) retained. MSI/EXE and Steam-layout recognition and Steam/Windows guided moves retain the supported-scope limits above; unsupported launchers are not guessed. No version bump, release, push or independent Astra review was performed.
- **Final checks:** release suite: 171 advisor + 57 core + 6 install passed (234), 6 ignored. App suite: 58 passed, 1 ignored; app build passed. Final display-cache invalidation passed cargo check. Exclusion traversal/counts, ancestor and link-member refusal, input validation, compressed tree/compact parity, NTFS/WOF versus ordinary links and MFT tag order are tested. A test initially used inherited OR syntax incorrectly, corrected to an explicit `ext:` alternative; new preference defaults were added to the existing exhaustive table. `git diff --check` passed. Later screenshots, including direct launches, remain black; the direct computer-use helper lost its session during capture. Narrow-layout/settings/reset visuals therefore remain unverified and are explicitly queued for Astra/user inspection. No real cleanup, compressor, disk shrink, game move or live model request in this continuation's final checks.

**Closure, 2026-11-19:** Astra subsequently read `.claude/fable-review-queue.md` in full,
reviewed the source and corrections, and marked all 33 findings Fixed/Done. Sprawl 1.0.0 is
released. Earlier implementation checkpoints above precede that review and remain as history.
See [current status](status.md) for deliberate deferrals and the new mockup-first design review.

**1.0.1 corrections, 2026-11-19 (Claude).** The user reported 1.0.0 as slow, with critical
bugs, and asked for a review of Codex's work since e022822. Done in three rounds, then pushed
to main (fed28fa..d5b9d4a and the notes after it) at the user's word; not released.
- Round 1 (the reported slowness): fa7ce20 (per-folder ancestor pinning in inventories;
  network-drive scans), dbbadb9 (per-file opens in evidence of use, compressed-size
  measurement and the daily scan), 39548e2 (whole-folder recycling).
- Round 2 (fuller review): c223324 (elevated copies could never update), da9573d (tree Delete
  froze the window), 081747d (History roots listed twice).
- Round 3 (every remaining changed file read as a diff): 0de55bf (a personal folder
  redirected to a share refused every delete; exclusions read blocked settings saves),
  678a4c2 (snapshot whole-second times vs exact comparisons), f71e6a7 (pre-1.0 alert task
  left running), ed3ce14 (skipped folders turned MFT off).
- Verified: workspace tests (app 66/1, advisor 200/5, core 60/1, installer 16); C:\Program
  Files inventory 0.20 s for 325k files; Y:\Albums (network) 0.15 s. Not run: elevated MFT
  with a skipped folder, the elevated update hand-off, a real alert task migration.
- Review: `.claude/fable-review-queue.md`, "1.0.1 corrections by Claude", items 1-14.
  Limitations, open questions and what still needs a runtime check: [status.md](status.md).
