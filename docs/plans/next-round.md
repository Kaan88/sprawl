# Next round (after 0.3.1)

Written 2026-11-18 from the user's list. It ties together `future-features.md` (F1-F6),
`advisor-next.md` (A1-A6), the new requests (R7-R11) and the MCP report. Nothing here is built yet.
The work goes in phases, and each phase is a release. Code that runs commands, deletes, links,
compresses or scans for an agent gets a Fable review before release, as usual.

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
