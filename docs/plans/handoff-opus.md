# Handoff: implementing the audit roadmap

**Status, 2026-11-18: done.** Phases 0 to 5, the look-and-feel pass and the Windows menu were built, reviewed and released as 0.3.0. This file is kept as the record of what was asked. One thing below was overruled later by the user: the administrator switches do not ask on first use; a switch that is on is the answer, and there is one warning above them in Settings.

You are implementing fixes in Sprawl (Rust + WinUI 3 disk analyser, this repository). A Fable session audited version 0.2.5 on 2026-11-17 and will review your work. Work one phase at a time and hand each phase back for review before starting the next.

## Read first

1. `docs/plans/audit-2026-11-17-final.md`: measurements, the real clean-up test, the engine analysis and the roadmap (section 11).
2. `docs/plans/audit-2026-11-17.md`: every finding with file and line. IDs such as SAF-1 come from here. Its section 7 (uninstaller), the UNI findings, accessibility and map-view items are out of scope.
3. `.claude/fable-review-queue.md`: the standing review rules and what earlier reviews left open.

Line numbers in the reports are from commit `3eec3d0`. Check each against the code before you change it; if a finding no longer matches, say so in the hand-back and do not guess.

## Decisions already made by the user

- Sprawl is not an uninstaller. Leave the hand-off as it is, apart from the label and doc fixes below.
- Accessibility and the number of map views are not for now.
- Search should be powerful but need not compete with Everything: no persistent index, no USN replay.
- Privacy cleaning is wanted, later (phase 5), after its prerequisites.

## Rules for working in this repository

- Build with `CARGO_TARGET_DIR=target/alt`. The user often has Sprawl open: never kill their copy, and if a build fails because the exe is locked, stop and say so.
- Never run `cargo fmt` (no rustfmt config; it reformats the whole crate).
- `crates/sprawl/src/bindings.rs` is generated. Do not edit it by hand.
- Test the app with a throwaway profile: set `APPDATA` and `LOCALAPPDATA` to a scratch folder. `sprawl --screenshot out.png <folder>` with `SPRAWL_SWITCH=<steps>` drives it (steps are listed in `crates/sprawl/src/state.rs`, `shot_tick`).
- A real clean-up may only be run on fixtures: scan only the fixture folder, with the two variables pointing inside it, and create no tool-command fixtures (pip, npm and the like act on the real machine). Never clean, delete or uninstall anything on the user's real profile.
- Do not call an AI provider or download community cleaners unless a task needs it; say so if you do.
- Every safety-relevant change gets a test. Run `cargo test --release -p sprawl-core -p sprawl-advisor -p sprawl-install` before each hand-back; all 113 passed at the start.
- Commit locally, one commit per item or small group, with the finding ID in the message. Do not push, bump the version, edit CHANGELOG or release: that happens after review.
- If a fix needs a product decision the reports do not settle, pick the more conservative behaviour, note it, and carry on.

## Phase 0: safety and honesty (do this first, then stop for review)

| # | ID | What to do | Done when |
|---|---|---|---|
| 1 | SAF-2 | `crates/sprawl/src/main.rs`, the SearchBox KeyDown handler: a row from another drive (place > 0) must set `search.more.sel` and clear `selected`/`marked`, as `table_key` in `tables.rs` does. The "already selected" test must be place-aware too | Small-folder scan, Ctrl+F with other drives loaded, `*.iso`, Enter: no crash, no scan node selected. Add the path to the review queue's list of guarded paths |
| 2 | SAF-1 | The executor (`crates/sprawl-advisor/src/clean.rs`) must keep anything created or modified after the scan time, and report how many items it kept. Pass the scan time in from `cleanup_start`. Tool commands cannot honour this: say so in their review text | A test with a file added between scan and run: the file survives and is counted. The review text is true for every action type |
| 3 | SAF-4 | Paths the user chose to keep (`Memory` decisions) are passed to `clean::run` as skips, at any depth inside a target | A test: a kept folder inside a ticked parent survives. The review lists what is kept |
| 4 | SAF-5 | "Save as a script" exports exactly the ticked findings. With nothing ticked, export nothing and say so | No unticked item appears in the script |
| 5 | SAF-3 | `llm/models.rs`: take the key through `openai_key()` (https or localhost only). Fetch the list on Save or on an explicit "Load models" action, not while typing. Do not send a key saved for one host to another host | A test that an http non-local address gets no Authorization header |
| 6 | SAF-6 | When Sprawl is elevated: refuse the Claude Code provider with a one-line reason, and refuse tool commands whose program resolves from a user-writable folder | Checked by a unit test on the gate |
| 7 | Defaults | For new installs: "Search Every Drive When Search Opens" off; community download off until switched on; the first-run wizard's AI page starts on "Off" with the detection shown as a suggestion; no findings ticked automatically (keep "Tick the safe ones") | Existing settings files keep their values |
| 8 | CLN-2 (part) | A keep-days or pattern part must not show a size the run cannot free. Minimum: label these sizes "up to", and count the real total at review time by listing the folder with the executor's own test | The fixture case "old logs" no longer promises 3.8 MB and frees 0 |
| 9 | Wording | AI disclosure (file contents can be sent; not every request is logged for Claude Code; web lookups can be allowed for a session); "Everything it claims is checked" becomes "every finding it adds"; amber text on Recycle Bin and Windows.old says it cannot be undone; "Uninstall X..." becomes "Open Installed apps…" (or "Hand to <name>…"); clean-up log times in local time; "from N items" counts only items that freed something; F1 shortcut text lists all five tabs; "Delete works in…" includes Search | Strings match behaviour |
| 10 | REL-1 | Catch a panic in the Advisor analysis thread and show an error with "Check again" | A forced panic no longer leaves the tab loading |
| 11 | SAF-7, SAF-8, SAF-10 | Findings outside `%USERPROFILE%` are not ticked for the user and are at least amber; `closed = "must"` without process names is refused at load; found-by-shape findings carry a heuristic flag that goal mode and the script exclude | Tests for each |
| 12 | Docs | README "Status", `docs/uninstaller-handoff.md` (the setting is under Options > Settings), the review queue's standing rule that the app never deletes | Match the code |

Two small extras the user asked for, in the same phase:

- **Drive letters flicker on the Search tab while drives are read.** Cause: `render_more` in `crates/sprawl/src/search_more.rs` rebuilds the whole strip from XAML every 500 ms, so the toggle buttons are recreated and shift as the text changes length. Build the strip once, update only the text, and keep the chips at a fixed position (before the text, or give the text a fixed width).
- **The same strip is drawn over the last result row**, and the top "Indexing…" pill repeats it. Give the strip its own row and remove one of the two displays.

- **Double-clicking a search result does nothing useful; it should open the file.** The user reports that nothing happens. Find out what the handler does today (`row_tapped` with `double = true` in `crates/sprawl/src/tables.rs`, and the Search branch in `crates/sprawl/src/search.rs`), then make it: double-click or Enter on a file opens it with its default program; on a folder, opens it in Explorer. Rows from other drives behave the same. Keep "show in the tree" as Ctrl+Enter and as a menu item. Check that the path still exists first and say so in the status line if it does not.

## Settings and the wizard (part of phase 3; decided by the user on 2026-11-17)

Two rules. **The wizard asks only what matters on the first day**: anything about consent, privacy or the network, and the few choices that change how the app behaves from the first scan. **Everything the wizard asks is also in Settings**, in the same words, and Settings has the rest. Both read and write the same keys; there is one place in code that describes a setting (name, help line, default, key), used by both.

### The wizard

Keep it to these pages, each skippable, nothing switched on that sends or keeps data unless the user chooses it:

1. **Welcome.**
2. **AI.** Provider (starts on Off, the detected one offered as a suggestion), model, key; under it, shown only when a provider is chosen: "Let the AI search the web (asks each time)", off. The disclosure text.
3. **Network.** "Check for updates automatically" (on), "Fetch community cleaners" (off), and "Work offline: no update check, no community download, no AI" (off; switching it on greys the others and the AI page's provider).
4. **Scanning and search.** Always run as administrator (with what it changes: faster scans, no Claude Code, no hand-off), "Scan with Sprawl" in Explorer, which other drives Search may read (none ticked), keep recent searches (on).
5. **History.** Keep scan history (the user's choice, with what is stored: every file name of each scanned folder, on this PC), space for it.
6. **Cleaning.** "Send my own files to the Recycle Bin instead of deleting them" (on), privacy cleaners (off; only when phase 5 exists).
7. **Look.** Palette, animations. The "Start in" choice goes away: the app opens in the view last used.

The uninstaller path and the Claude Code MCP line leave the wizard; they stay in Settings.

### Settings: what must be there

Everything in the wizard, plus what the final report's section 9 lists, plus these that exist today only as menu items, panel controls or constants in code:

| Setting | Today | Default | Section |
|---|---|---|---|
| Units; binary (GiB, shown as GB) or decimal | Options menu; binary only | Best fit, binary | General |
| Scan as soon as a drive is picked | Options menu | on | Scanning |
| Watch for changes after a scan ("Live Change Pulses") | Options menu | on | Scanning |
| Folders to skip when scanning | none | empty | Scanning |
| After a scan: stay, or open the Advisor | none | stay | Scanning |
| Map: names, 3D and tilt, shading, mirror, free space, size by allocated, antialiasing | Options menu and Colours and Tweaks | as now | Map |
| Never touch (paths the user chose to keep), with Add and Remove | Remembered, per finding | empty | Cleaning |
| Open Cleaners with my usual ticks | implicit | on | Cleaning |
| Old scan warning after N minutes | constant, 60 | 60 | Cleaning, Advanced |
| Smallest finding listed | `rules::DEFAULT_MIN`, 50 MB | 50 MB | Advisor, Advanced |
| Smallest cleaner part listed | `cleaners::PART_MIN`, 1 MB | 1 MB | Advisor, Advanced |
| A project counts as idle after | `PROJECT_IDLE_DAYS`, 30 | 30 days | Advisor, Advanced |
| An application counts as unused after | `UNUSED_DAYS`, 180, and 1 GB | 180 days | Advisor, Advanced |
| A leftover needs no change for, and its smallest size | `leftovers::IDLE_DAYS` 180, `MIN_BYTES` 50 MB | as now | Advisor, Advanced |
| "Not now" hides a finding for | `memory::LATER_DAYS`, 30 | 30 days | Advisor, Advanced |
| Duplicates: smallest file, how much to read | 50 MB, 40 GB in `dupes.rs` | as now | Advisor, Advanced |
| Community cleaners: fetch every | `community::EVERY`, 7 days | 7 days | Cleaning |
| Full snapshots kept per folder | `history::KEEP_FULL`, 3 | 3 | History |
| Saved scans Search may use | `search_more::MAX_SNAPSHOTS`, 3 | 3 | Search |
| Most results shown from other drives | `search_more::MAX_HITS`, 100,000 | 100,000 | Search, Advanced |
| AI request log: Open, Clear; forget kept AI results | file only | – | AI and privacy |
| History: size on disk, Clear | none | – | History |
| Reset this section; reset everything; open the data folder | none | – | each section, General |

Do **not** expose, because they are safety limits and not preferences: the never-list itself (the user may add to it, never remove from it), the risk floors for anything not built in, the confidence floor for green, size and count caps on downloaded or model-written input, the scan-time protection ("new since the scan"), and the typing delay.

A threshold changed by the user never makes anything green or ticked that would not be otherwise: lowering "idle after" below a floor (7 days for projects, 30 for applications and leftovers) is not allowed. Say the floor in the control.

Each setting has a test that its default is the documented one, and the wizard and Settings share the strings.

## When Sprawl runs as administrator: lockouts become options (decided by the user on 2026-11-18)

The user does not want things simply switched off when Sprawl is elevated. Each refusal that exists only because the thing would run as administrator becomes a setting, **off by default**, with the risk said plainly beside it. They sit together in Settings, under Scanning, below "Always run as administrator", headed "While Sprawl runs as administrator, also allow":

| Setting | Key | What it allows | Warning shown beside it |
|---|---|---|---|
| The Windows menu | `adminShellMenu` | "Windows menu…" on a row while elevated | Explorer's menu add-ons from other programs will run as administrator inside Sprawl |
| Handing an application to your uninstaller | `adminHandOff` | Starting the user's uninstaller program while elevated | That program will run as administrator |
| Claude Code | `adminClaudeCode` | The Claude Code provider while elevated | Claude Code, found through your PATH, will run as administrator |
| Clean-up commands from your own folders | `adminUserTools` | A built-in tool command (npm, pip, and the like) whose program sits in a folder your account can change | Any program of that name in your folders will run as administrator |

Rules:

- Off by default, never switched on by the wizard, never by "Reset" to anything but off. Not on the wizard at all.
- The switch is the only thing that changes: every other check stays (one row at a time, not during a clean-up, the review, the never-list, and so on).
- Where the thing is refused today, the message names the setting: "Not as administrator. You can allow it in Settings > Scanning." Where it is allowed by the setting, the first use in a session asks once, with the same warning and "Don't ask again this session".
- When a setting is on and Sprawl is not elevated, nothing changes.
- Tests: each key off refuses as today; on allows; the defaults are off; Reset leaves them off.

## Phase 4 design: Search that does not need its syntax remembered

Decided by the user on 2026-11-17. The query language and the compact index are already in `crates/sprawl-core` (`query.rs`, `compact.rs`, merged from the `search-core` branch). Phase 4 wires them into the Search tab. The rule for all of it: **the box always holds the real query as text.** Every control below writes into the box or reads from it, so typed and picked filters cannot drift apart, and a query can be saved or copied as text.

```
[ search box                                   ] [?] [saved ▾]
 Kind ▾   Type ▾   Size ▾   Modified ▾   Where ▾   More ▾
 (error or hint line)
```

1. **Dropdown chips under the box.**
   - Kind: files, folders (`file:`, `folder:`).
   - Type: pictures, video, audio, documents, archives, installers, code, and "other…" to type extensions (`ext:`). Take the groups from `sprawl_core::kinds` so they match the rest of the app.
   - Size: empty, under 1 MB, 1 to 100 MB, 100 MB to 1 GB, over 1 GB, custom range (`size:`).
   - Modified: today, last 7 days, last month, this year, a year or older, custom dates (`dm:`).
   - Where: this scan, each other drive (the existing drive chips move here or sit beside it), and "inside this folder…" (`path:`).
   - More: exact name, case-sensitive, regex, depth.
   - Picking an option adds its term to the box and lights the chip with the chosen value; clicking a lit chip removes the term. Typing a term by hand lights the same chip: the chips are drawn from the parsed query, not from their own state. One term per chip kind; picking again replaces it.
2. **A "?" cheat sheet.** A flyout with `query::HELP`, one line per feature with an example. Clicking an example puts it in the box. The same list is the empty state before anything is typed.
3. **Suggestions while typing.** When the word under the caret is a keyword with a colon and nothing or a prefix after it (`dm:`, `ext:`, `size:`), a short list appears below the box: the named values for that keyword. Up and Down move in it, Enter or Tab takes one, Esc closes it, and typing on filters it. When no list is open, Enter and Down keep their meaning (go to the results). When the word is the start of a keyword (`ex`, `dm`), offer the keyword itself. The suggestions must never steal a key from plain typing: if nothing is highlighted, Enter goes to the results as today.
4. **Errors under the box.** Show `Query::errors()` in the hint line, in plain words, as the user types. A query with an error finds nothing; say so there, not as "Nothing found".
5. **Saved and recent searches.** A dropdown at the right of the box: the last ten queries and the ones the user pinned. Stored as text in the settings folder. Search history is the user's data: a "Clear" entry in the dropdown, and a setting to keep no history.
6. No separate "advanced search" form.

Also in phase 4, as the reports list: Esc returns to the box, multi-select with Copy Path, a path column, each other drive showing when it was read with a Refresh, drives read through `CompactIndex`, a drive switched off cancelling its read, and the result cap re-applied on a sort change.

## The Windows context menu (do with phase 4, or earlier if the user asks)

The user wants the real Explorer right-click menu to be reachable from rows in Tree View, File View and Search. Add one item at the bottom of Sprawl's own row menu, "Windows menu…" (and Shift+right-click as a shortcut), which shows the shell's menu for the row's path. Keep Sprawl's menu as the default: it is faster and its Delete goes through the safety checks.

Design points to respect:

- Use the shell interfaces (`IContextMenu` for the item, shown with `TrackPopupMenu`, then `InvokeCommand`). The `windows` crate features needed are mostly enabled already in `crates/sprawl/Cargo.toml`.
- Shell extensions are third-party code loaded into Sprawl's process. **Do not offer the item when Sprawl runs as administrator**: they would run elevated. Say why in a tooltip.
- A badly written extension can crash the host. Note this risk in the hand-back; do not try to isolate it in a helper process unless the user asks.
- Anything done through that menu (delete, rename, move, 7-Zip, and so on) bypasses Sprawl's never-list and leaves the tree stale. When the menu closes, rescan the row's parent folder and update the tree the way `apply_rescans` does after a clean-up.
- Works for one row. With several rows picked, either pass all of them to the shell or disable the item; do not silently act on one.
- For rows from other drives or saved scans, check the path exists first.

## Later phases (each after the previous one is reviewed)

- **Phase 1, performance:** Folder sort (build keys once, then a per-tree folder rank, off the UI thread; it took 32.9 s for 2.46 M hits); `set_tree`, filter and snapshot encoding off the UI thread; typing delay about 50 ms. Use `crates/sprawl-core/examples/bench_search.rs` to show before and after.
- **Phase 2, engine:** final report section 4, items 1 to 10. Start with partial findings (carve-outs) and graded build outputs. `crates/sprawl-advisor/examples/gates.rs` and `advise` show the effect.
- **Phase 3, Cleaners, Advisor and Settings:** the section "Phase 3 design" below replaces the "one Clean workspace" idea in the reports. Settings: final report section 9.
- **Phase 4, search power:** final report section 7.
- **Phase 5, privacy cleaning:** final report section 8.

Ask the user before starting phase 3: it changes the layout.

`docs/plans/future-features.md` lists six features for after phase 5. They are not part of this work; do not start them.

## Phase 3 design: Cleaners as a direct tool, Advisor as the scan's advice

Decided by the user on 2026-11-17. The user wants Cleaners as its own tab so it can be used directly, the way BleachBit is, and wants the AI on that tab. Both tabs stay. What changes is that they stop being two views of one panel.

**Cleaners tab**

1. **Works without a scan.** When the tab opens and the current scan does not cover the cleaners' folders (no scan, another drive, a single folder), scan just the folders the cleaner definitions name (`scan_many` over the expanded `present` and part paths) and build the list from that. Say which it is using: "from this scan" or "checked just now". Every finding still passes `safety::check` against the tree it came from.
2. **Layout.** Left: a tree of applications with tick boxes, parts underneath, grouped as today. Right: a details pane for the selected part (what it is, what it costs, where it lives, in-use state, keep-days). No map on this tab. The foot keeps the tick total and the two buttons below.
3. **Preview and Clean.** Preview lists the exact files that would go, with the real total, counted with the executor's own tests, and deletes nothing. Clean runs the existing review and executor. The review shows the preview's numbers.
4. **Ticks are the user's and are remembered here.** The tab opens with the ticks from last time ("my usual"). Nothing is ticked for the user. A part with a real cost (signs out, long re-download, cannot be undone) says so when it is ticked, not only at the review.
5. **One engine.** Ticks, review, executor and log stay shared with the Advisor.

**Advisor tab**

1. Remove its Cleaners sub-tab.
2. The Advisor still presents what the cleaners found in the scan; the user wants that after scanning a drive. Show it as one group, not as loose rows mixed into the findings: a "Cleaners" group with its total ("Caches of 12 apps, 2.7 GB"), the largest few applications listed under it, each tickable right there, and "See all in Cleaners" to open the tab. A tick here is the same tick as in Cleaners. Nothing a cleaner found may be missing from the Advisor's total or from "tick enough for N GB".
3. The Advisor keeps what is specific to the scan: build outputs, old installers, unused apps, duplicates, growth, hidden space, tips, AI findings.
4. Collapse the map while the Advisor is open.

**AI on the Cleaners tab (required)**

1. **It must work with Claude Code.** Today the Claude Code chat has no tools (`llm/agent.rs`, `tools_ok`), so "Suggest what to clean" cannot produce the "Tick them" card on the provider the wizard suggests first. Give that chat the cleaning tools through the same MCP route the analysis run uses, with the same allow-list. Until that works, hide the chips that need tools for that provider and say why.
2. **Placement.** The AI gets its own column on the right, or a panel that opens beside the list. An answer must never push the cleaner list out of view. With AI off, the same place shows one "Set up AI" button and nothing else.
3. **Suggestions show in the list.** A suggested part gets a mark and its reason on the row and in the details pane; "Tick them" applies all, and each can be ticked or dismissed alone. Ticks from the AI still go through the review. The AI may suggest green and amber parts only, never privacy parts.
4. **Give it true numbers.** Send the preview's sizes, not the scan's estimates, and what each part actually freed last time. In the live test the model recommended a part that frees nothing because it was given the inflated size.
5. **Ask about the selected part.** The "?" on a part asks about that part and shows the answer in the details pane.
6. **Unknown applications.** Make "Sprawl doesn't know these N applications" a visible group with "Ask the AI to look", leading to the existing propose, second opinion and Approve steps.
7. **Honesty.** The chat prompt asks the model to say when it is not sure, and a line under each answer states that answers are not checked the way findings are. Declined suggestions are remembered so they are not offered again.
8. **Stop works at once**, and the status line shows that a request is running and to which provider.

**The AI's web lookup (decided by the user on 2026-11-17)**

The chat already has a `web_lookup` tool that asks before each search. Keep it, for every provider that can call tools (Anthropic, OpenAI-compatible, and Claude Code once its chat has tools), and tighten it:

1. **A switch in Settings, off by default:** "Let the AI search the web (asks each time)". While it is off the tool is not offered to the model at all, in chat or anywhere else.
2. **Names, not paths.** Before the query is shown or sent, Sprawl removes anything that is a path: drive letters with a path, `\Users\<name>\`, the user's name and the PC's name. A query that is still a path after that is refused with a message to the model to ask by application or folder name. Test it.
3. **Every query is visible.** After "Allow for this session", each query still appears in the status line as it goes out, and in the request log.
4. **Results are hints.** What comes back is marked to the model as untrusted text from the web. Nothing changes in what a model's finding may be: at least orange, never ticked for the user, checked against the scan. Add a test that a finding reported after a web lookup gets no better treatment.
5. Never in Search, and never without a question from the user in the chat: no background lookups, none during the analysis run.

## Handing back for review

When a phase is finished, send one message to the reviewing session with SendMessage, addressed to:

`WizTree clone project reorganization`

Put in it:

- the commit range;
- one line per item: done, changed from the plan (and why), or skipped (and why);
- the test result line, and what you ran by hand;
- anything you found that the reports got wrong;
- open questions.

Then stop and wait. Do not start the next phase until the review comes back.

If the message cannot be delivered (the session is closed), add an entry to `.claude/fable-review-queue.md` under "Open" with the same content, tell the user it is queued, and stop.
