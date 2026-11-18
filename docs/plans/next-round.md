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

## Phase 2: the Advisor and the AI working with you (0.4.x)

- **R7, "Look first" rows can be ticked too.** Keep "Look first" as the label and the advice, but
  add the tick box. A ticked "Look first" row goes to the review in its own group with "you haven't
  looked at these", and goes to the Recycle Bin, not a permanent delete.
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

The user says the order doesn't matter. Phase 0 still comes first, since those are bugs.

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
- **B3** (cdfb66c): done, not yet tried by hand. Measured with the new `SPRAWL_TIMING=1`
  (`%TEMP%\sprawl-timing.log`): each open rebuilt the whole panel (~25 ms) plus a 35-65 ms
  frame. Rows now sit in `AdvRow{i}` Borders; open, close and the in-use answer swap only
  those rows (`advisor_rows`, 2-5 ms), anything else falls back to the full render. The jump
  to the top was focus falling to the first control when the clicked button was destroyed: the
  clicked row's button now takes focus back. To try by hand: scroll the Findings list down,
  open a row, check the list stays put. (Screenshot mode hides its window, and a normal launch
  asks for elevation here, so this wasn't clicked through.)
