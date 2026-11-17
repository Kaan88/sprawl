# Usability pass: final review (Fable 5.2)

Written at c7c07b1, before the fixes in 1bb4f9c, 0404c2b and 3edfe81. Fixed since: the Blocker and Majors 1 to 6 and 8 (Major 7 and 9 partly: dead colour choices now say why; ticks survive tree changes, but focus is still lost after Advisor actions other than ticking), plus the smaller items on F5, "Set up AI", Settings and the AI, and "Colours". Everything else below is still open.

# Sprawl usability: final read after the pass (0370ae0..c7c07b1)

By Fable 5.2, from the code at HEAD and 40 screenshots of the built app (1680x1080, in
`scratchpad\fr\`): `C:\Program Files` (p_*, v_*), `%LOCALAPPDATA%` (l_*, 9 findings, 5 ticked),
`archive\sprawl-demo` (d_*). Nothing was built, edited or deleted. Paths are under
`crates/sprawl/src/`.

What I could not test: a real clean-up run (never pressed), a live AI answer, the wizard's later
pages, a narrow window, a second monitor, a screen reader. Those parts are judged from code only.

## Verdict

The pass did what it set out to do in most places: the AI path ends in one review, the review
says "Deleted for good", colour is one switch with a key, the views are grouped and captioned,
Scan becomes Stop, closing mid clean-up asks, Settings is one page. The Advisor is still short of
room, though, and the pinned foot that fixed "the bar scrolls away" brought a new problem: the
review can now outgrow the pane. Milestone 1's evidence items (cost line, log, amber result) were
not done. Growth still tells three different stories.

---

## Blocker

### B1. The review has no scroll: with more than about 5 items its buttons fall off the pane
- Where: `ui.rs:543` (AdvisorPanel: rows `*` and `Auto`, the foot is a bare Border),
  `cleanup.rs:493` (`advisor_foot_xaml`), `cleanup.rs:420` (one row per ticked item).
- Seen: `l_review.png`. Five ticked items at 1080p fill the Advisor pane to the last pixel: the
  list, the section tabs and the colour bar are gone and "Clean up now / Back" sit on the bottom
  edge. Each further item adds about 42 px. Not seen with 6 or more (no folder to hand had that
  many), but nothing in the code limits the foot's height or lets it scroll, so the buttons are
  pushed out and clipped. A whole-drive scan pre-ticks every green finding, usually more than 5.
  The default window is 90% of the screen, so smaller than my screenshots.
- The same goes for the result card (up to 8 problem lines plus tool output, with the bar under
  it) and the running list (one line per item).
- Fix: give the foot `MaxHeight` = about 60% of the pane and put the review's item rows (and the
  result's problem lines) in their own ScrollViewer, with the heading, the "Deleted for good"
  line, the own-files box and the buttons outside it, always visible. Better still: while the
  review is open, show it in place of the list (row `*`), with only its buttons pinned.

---

## Major

### M1. The Advisor still has no room: 1.5 findings visible
- Where: `advisor.rs:1158-1233` (title, bar, tabs, chips, duplicates row = about 170 px),
  `cleanup.rs:397` (bar = about 135 px), `ui.rs:403-405` (map row keeps 45% of the height).
- Seen: `l_adv.png`: 9 findings, 1.5 on screen. `d_open.png`: an opened finding shows two evidence
  lines; its buttons (Clean up just this, Show the command, Keep it) are under the bar.
- Plan item "header folded into one line; sort and filter chips into a small menu": not done.
- Fix: (a) one header line: summary left, sections as tabs right; Sort and Show as two small
  drop-downs on the tab line; "Find duplicate files" to the end of the list. (b) the bar as one
  line: "5 ticked · 2.6 GB  [Tick safe] [Untick] [need __ GB]  [Clean up...]", the script link in
  a "..." menu. (c) while the Advisor is open, shrink the map row to about 25% (or offer a
  "more room" toggle on the tab line).

### M2. Growth tells three different stories
- Where: `advisor.rs:2054-2069` (`growth_card`), `views/mod.rs:455-456` (key), `history.rs`,
  `views/time.rs` (Tideline).
- Seen, same folder, same minute: History panel "+1.3 GB since the scan 9 hours ago · 6
  snapshots" (`p_hist.png`); Advisor > Growth "shows here once History has scans at least a day
  apart", with none of its three buttons (`p_grow.png`); Colour: Growth darkens the whole map,
  lights nothing, and its key says "Lit: what grew since the scan before" (`p_growth.png`);
  Tideline "No earlier scans of this folder yet" (`v_tide.png`, 4 s after opening: maybe still
  loading, but it doesn't say so).
- Fix: Growth tab always shows what History knows (change since the last scan, what grew most)
  and always shows the three buttons when any earlier scan exists; the day-apart rule only gates
  the "per day" and "full in" lines. Key: when the lens is empty say "Nothing grew since the scan
  before" and leave the map in its normal colours. Tideline: "Loading earlier scans..." while the
  thread runs.

### M3. Milestone 1's evidence items are missing
- No clean-up log: nothing writes `cleanup-log.txt`, no "See what was removed" (grep: no match).
  After OK the result is gone for good; for permanent deletes that is the one record a user needs.
- Result card is always green with a tick, also for "Freed 0 bytes" or when items failed
  (`cleanup.rs:375`). Fix: amber border and warning icon when `problems` is not empty or
  `freed == 0`.
- Review rows show a generic risk phrase and the path, not the finding's own cost line
  (`cleanup.rs:440-449`). Fix: first evidence line that starts "Comes back with", else the first.
- "Tick enough for N GB" says "Ticked 2.1 GB of findings for the 2 GB you need"
  (`advisor.rs:740`) and doesn't name the amber items it added. Fix: "…including 2 that take time
  to get back: node_modules in site, …".
- "Find more with AI" (`advisor.rs:719`) goes to the Ask tab; with AI off the user lands on
  "Set up AI". Fix: label it "Ask the AI to look further" only when AI is on; otherwise
  "Find duplicate files" is the better next step.

### M4. Ctrl/Shift picks linger, and Delete then takes more than the user is looking at
- Where: `tables.rs:564` (right-click), `tables.rs:581-660` (arrow keys, type-to-find),
  `treemap.rs:641` (`reveal`, a click on the map): none clears `marked`. `commands.rs:560`.
- Effect: Ctrl-click A and B, press Down to C (or right-click C, or click a tile), press Delete:
  A, B and C go. The confirm lists them, but the habit from Explorer is that moving on drops the
  old selection.
- Also: the footer total double-counts a folder and a file inside it, while its text says "count
  once" (`tables.rs:674-678`; `picked_nodes` isn't used there). No Shift+arrows, no Ctrl+A, no
  Escape to clear.
- Fix: clear `marked` on a plain arrow key, on a right-click outside the picked rows and in
  `reveal`; Shift+Up/Down extends; Escape clears; the footer sums `picked_nodes()`.

### M5. The path line runs over the view buttons
- Where: `ui.rs:576` (`MapPath`, fixed `Margin="300,0,300,0"`); the button strip is 480 px wide,
  600 px in Waffle.
- Seen: `m_adv.png` ("Colour: Types" unreadable under a long path). Any deep path does it, and
  deep paths are the normal case when hovering the map.
- Fix: three Grid columns (buttons Auto, path `*`, History Auto), path left-trimmed with an
  ellipsis in the middle column.

### M6. The colour switch offers choices that do nothing
- Owner on Treemap, City and Galaxy: nothing changes; the key says "Owner colours show in the
  More views" (`views/mod.rs:452`, `m_owner.png`).
- Galaxy ignores Risk and Growth but the key shows the risk legend (`l_galrisk.png`).
- City with Risk: all grey, the green can't be made out (`l_cityrisk.png`).
- Risk or Growth is remembered (`views/mod.rs:423`): the next start opens on a near-black map,
  and nothing near the map says why except the small key.
- Fix: disable the menu entries a view can't show (or fall back to Types and say so in the key);
  paint City roofs and Galaxy stars from the lens; when a remembered Risk/Growth has nothing to
  light, draw the normal colours and put the reason in the key.

### M7. The wizard interrupts the first result
- Where: `state.rs:276-279`: `wizard_open()` runs the moment the first scan ends, also when that
  scan failed or was stopped. A dimmed, modal, six-step dialog covers the first map the user has
  ever seen (`p_wizard.png`). Plan said "offered" and "cut to plain choices"; it is neither
  (still 6 pages; the welcome page lists 4 of the 5).
- Fix: after the first good scan show one line in the footer or over the map: "Set Sprawl up
  (AI, faster scans, history)  [Set up] [Not now]". Open the wizard only on "Set up".

### M8. Keyboard in the Advisor: every action drops the focus
- Where: `advisor.rs:988-1081`: each command rebuilds the whole panel; only a tick puts the focus
  back (`focus_after`, set at `:1065` only). Enter on a finding, a sort chip, a section tab,
  "Show the command": focus goes to nothing and Tab starts again from the top.
- Plan items: "only the changed row updates on a tick": not done (full rebuild, `:1066`);
  "focus and scroll restored after any rebuild": scroll yes, focus only for ticks.
- Fix: in `advisor_command` and `advisor_open` set `focus_after` to the name of the control
  pressed (it exists again after the rebuild).

### M9. Ticks are lost on any tree change
- Where: `cleanup.rs:115-144`: a new report generation resets the ticks to Sprawl's defaults. A
  Delete in the tree, a filter on and off, or one finished clean-up item changes the generation.
  Plan item "ticks kept across tree changes for paths that still exist": not done (only the
  administrator restart carries them).
- Fix: before `advisor_start`, copy `ticked` into `pending` when `auto` is false.

---

## Minor

1. **Garbled text in the path line.** `views/mod.rs:774` has `"   Â·   {} can be won back"`
   (a mis-encoded "·"). Shows when hovering a folder with findings in any More view. Came in with
   pass 7 (7575325). Fix the literal.
2. **"Types" colours a folder by its biggest child, not its biggest type** (`views/mod.rs:533-545`):
   a folder of 10 GB of .dll with one 300 MB video takes the video's colour. Sum bytes per type.
3. **The map key overlaps or contradicts some views.** Waffle has its own legend by folder while
   the key shows file types, and it covers the last row of squares (`l_waffle.png`); it covers the
   bottom of Flow (`l_flow.png`). Hide the key where the view draws its own, or move it out of
   the drawing area.
4. **The caption collides with views' own labels**: Rose Stack (`v_rose.png`, two texts on top of
   each other), Flow's "File type" heading (`l_flow.png`). Give the caption its own strip, or
   start the views 30 px lower.
5. **Flow's last column showed only "No finding 129.9 GB"** on a scan with 2.6 GB of findings
   (`l_flow.png`). For the one view in "What can go", the part that can go must always get a
   block, however thin.
6. **History from a More view leaves no view button lit** (`history.rs:126-129`: `set_mode`
   returns early, `treemap.rs:484`), and saves Treemap over the remembered view. Light ModeMap;
   put the view back when History closes.
7. **The tick box of an opened finding slides down** to the middle of its details
   (`advisor.rs:2209`, `VerticalAlignment="Center"`; `d_open.png`: gone under the bar). Use Top
   with a 9 px margin.
8. **The Ask tab has three "Ask"s and no explanation when AI is on** (`p_ask.png`): "Ask Claude
   Code", "Ask your disk", "Ask". Plan: the look-further button at the end of Findings, Ask is the
   chat only: not done. Rename the button "Look for more to clean" and move it.
9. **"Set up AI" opens the whole wizard at its welcome page** (`advisor.rs:856`), though the same
   choices open in place under "Change". Make it do what "Change" does.
10. **"Nothing is removed until you press Clean up now."** closes every section, also Ask and
    Growth and scans with nothing to clean (`advisor.rs:1200`). Show it in Findings only, when
    something can be ticked. An empty dark strip is left at the pane's foot when there is no bar
    (`p_ask.png`).
11. **Tips**: the size column shows the folder (7.9 GB), the gain is in the title (2.1 GB), and
    the list sorts by the folder (`advisor.rs:2242`). The how-to is an administrator PowerShell
    line inside the evidence, with no Copy button (`p_cmd.png`). Show the gain; add "Copy the
    command".
12. **No Backspace to go out on the map**; the wheel means "zoom the camera" on the Treemap and
    "go into the folder" in the More views (`treemap.rs:598-625`). Plan's one rule: partly.
13. **More views redraw every frame while Animations is on** (`views/mod.rs:603-622` only stops
    when it is off). A still Rings view keeps the GPU busy.
14. **The current view has no tick in the menu** (`ui.rs:355`, plain items); the button's label
    carries it. Use RadioMenuFlyoutItem.
15. **Settings can turn AI on unasked.** Before the wizard was ever finished, Settings pre-selects
    the detected provider (`wizard.rs:192-193`); Save then stores it though the user came for the
    palette. In `settings_open` always show what is saved.
16. **Settings and menu still overlap**: Options has Settings, Colours and Tweaks, Setup Wizard
    and a Map submenu; "Colors" in three places (`ui.rs:381`, `:384`, `wizard.rs:93`) beside
    "Colour:" and "Colours and Tweaks". Escape doesn't close Settings.
17. **Stale or jargon text**: `history.rs:140` "Options > Setup Wizard > History" (now Settings);
    `state.rs:394/400` "(read from the MFT)", "reads the MFT directly"; `state.rs:319` "Scan
    failed: {raw error}"; welcome "NTFS drive's file table"; About (`ui.rs:546-549`) doesn't
    mention the Advisor and lists six-year-old shortcuts beside the new Help list; the model's
    prompt still says green = "comes back by itself" (`sprawl-advisor/src/llm/agent.rs:36`).
18. **Window place**: only the primary screen's work area counts (`win32.rs` `set_window_place`),
    so a window kept on a second monitor always comes back on the first.
19. **Header fold** hides the brand card under 1250 px and nothing else (`main.rs` SizeChanged);
    the 380 px scan card and five-line info card stay.
20. **Administrator restart with many ticks**: up to 500 `--tick` paths go on one command line
    (`commands.rs:130`); past about 32,000 characters the restart fails with no message. Write
    them to a temp file, or cap by length and say so.
21. **Scan while cleaning** is refused with a footer line, far from the button (`state.rs:210`);
    the button itself looks enabled. Disable it.
22. **Scan / F5 scans the drive in the box**, not the folder on screen: after F3 on a folder, the
    header shows that folder but Scan reads all of C:. Make F5 "scan again" and keep the box for
    picking.
23. **Accessibility**: names on the tick boxes, the lock icon and two History buttons only
    (4 in all). Sort chips don't expose "selected"; the Tweaks close button has no name. 12 px
    minimum: done.
24. **Welcome** sits in the lower half under two empty tables, not full window.
25. Dead scraps from the Plan removal: `advisor.rs:192-193` comment, `:1157 let _ = now`,
    `:1202 if r.secs > 0.0 { "" } else { "" }`, `:2098` comment, `treemap.rs:269 preview = None`.

---

## What works well now

- AI path: "Clean up just this..." opens the same review, in the foot, from any section
  (`d_run.png`). AI findings are in the one list with a badge; the Ask card points to them.
- "Deleted for good, not sent to the Recycle Bin." in amber in the review; "They were deleted for
  good" and the new free space in the result. "Clean up now" is disabled until the own-files box
  is ticked.
- The Advisor takes the full width; the bar, review and progress are drawn in every section;
  messages show in the foot (`d_goal.png`). "Sprawl ticked the safe ones for you."
- Risk words and chips read well. "6 tips, nothing to clean". Tips have a grey dot. The command
  is behind "Show the command" after "Sprawl does this for you".
- One Colour switch with a key; Risk on the Treemap (`l_risk.png`), in Suds (`l_suds.png`) and
  Waffle's "Showing: what can go" (`l_waffle.png`) are clear and useful.
- More views: grouped, one-line descriptions, caption on the map, remembered. Grove now fits.
- Scan becomes Stop. Closing, Exit and a new scan during a clean-up are handled. Own deletes
  don't count as Live. Window remembered. Ctrl+1..3, arrows in the types table, type-to-find.
- Settings is one scrolling page with Cancel and Save; screenshots no longer write settings.
- Help menu with a shortcut list built from the menus, so it can't go stale.

---

## usability-pass.md, item by item

### 1. Trust
| Item | State | Evidence |
|---|---|---|
| "Clean up just this..." opens the review in Findings | Done | `advisor.rs:690-697`, `d_run.png` |
| AI findings once, in Findings; Ask card points there | Done | `advisor.rs:2153-2156`, `:2241` |
| Review and result say "Deleted for good…" | Done | `cleanup.rs:425`, `:375` |
| One-line cost under each review row | Not done | `cleanup.rs:440-449` (generic risk words) |
| "Tick enough" names the amber items | Not done | `advisor.rs:739-743` |
| Clean-up log, "See what was removed" | Not done | no code |
| Result card amber on failure; free space; "Find more" | Partly | free space and link yes; always green (`cleanup.rs:375`) |

### 2. One Advisor, with room
| Item | State | Evidence |
|---|---|---|
| Full width while open | Done | `commands.rs:48-55`, `l_adv.png` |
| Header in one line; chips in a menu | Not done | `advisor.rs:1158-1233` |
| Bar pinned, drawn in every section | Done, but see B1 | `ui.rs:543`, `cleanup.rs:493` |
| AI findings only in Findings; "look further" at the list's end; Ask = chat only | Partly | button still on Ask (`advisor.rs:2099-2107`) |
| Chat answer links a path to its row | Not done | `advisor.rs:1892-1937` |
| AI off: setup choices in place | Partly | "Change" yes; "Set up AI" opens the wizard (`:856`) |
| Messages in the panel; Clean up now disabled until ticked | Done | `cleanup.rs:478-489`, `:459` |
| "Sprawl ticked the safe ones for you" | Done | `cleanup.rs:408` |
| Ticks kept across tree changes | Not done | `cleanup.rs:117-144` |
| Only the changed row on a tick; focus and scroll restored; review in view | Partly | full rebuild; scroll kept; focus only for ticks |

### 3. Words and leftovers
| Item | State | Evidence |
|---|---|---|
| Risk words, chips, "6 tips, nothing to clean" | Done | `advisor.rs:324-341`, `:1145`, `:1221` |
| Info rows show the gain with a neutral dot | Partly | dot yes; size column is the folder |
| Command behind "Show the command" | Done (tips keep it in the evidence) | `advisor.rs:2274-2299`, `p_cmd.png` |
| Rewrites (advisor, agent, mcp, wizard) | Mostly | `history.rs:140`, `agent.rs:36` left |
| Plan code deleted | Done, scraps left | Minor 25 |
| "Remembered"; one spelling; drop Rust+WinUI, "Checked in", MFT; plain scan-failed | Partly | "Colors" x3; MFT x2; raw error |

### 4. Views and colour
| Item | State | Evidence |
|---|---|---|
| One Colour switch for every view; Advisor's buttons gone; no darkening by itself | Partly | Owner not on the 3 main views; Galaxy ignores it |
| Key in the corner; plain "needs…" messages | Done, with M2/M6 errors | `views/mod.rs:431-467` |
| Types = biggest type | Partly | biggest child (`views/mod.rs:533-545`) |
| Menu grouped, described, tick on the current | Partly | no tick |
| Hint as a caption on the map | Done (collides in Rose, Flow) | `ui.rs:580` |
| One click rule; Waffle toggle; Lens re-centres | Partly | no Backspace; wheel differs |
| Views and colour remembered | Done | `views/mod.rs:409-425` |
| Redraw only while moving; Animations off reaches the views | Partly | only when Animations is off |
| History switches to Treemap; times when dates repeat | Done (button not lit) | `history.rs:126`, `views/time.rs:40` |
| Grove, Molecule, Spectrum, Rose Stack fixed | Partly | Grove fine; Molecule clipped at the top, no labels; Spectrum small; Rose labels fine, caption clash |

### 5. Long jobs, window, keyboard
| Item | State | Evidence |
|---|---|---|
| Ask before Exit/close/admin restart; Scan disabled while cleaning; Scan to Stop | Done (restart and scan refuse with a footer line) | `main.rs:129-147`, `commands.rs:124`, `:264`, `state.rs:210`, `:453` |
| Admin restart keeps the Advisor and ticks | Done | `commands.rs:126-133`, `main.rs:444` |
| Window fits and is remembered; header folds | Partly | primary screen only; only the brand card folds |
| Own deletes not "Live" | Done | `state.rs:352-360` |
| Ctrl+1..4; arrows in types; type-to-find; AltGr | Done | `ui.rs:509-512`, `tables.rs:526`, `:619` |
| Accessibility | Partly | 4 names; 12 px done |
| Multi-select with one confirm | Done, with M4 | `tables.rs:470-491`, `commands.rs:607` |

### 6. First run and settings
| Item | State | Evidence |
|---|---|---|
| Welcome first, full window; wizard offered after the first scan, cut down | Partly | half window; opens by itself, 6 pages |
| One Settings page | Done | `wizard.rs:225-242`, `p_settings.png` |
| Help menu, current shortcut list; About leaves the tabs | Done | `ui.rs:346-350`, `commands.rs:110` |

## What the pass made worse
1. The pinned foot: the review, the running list and the result can outgrow the pane (B1), and
   the bar now takes its 135 px in every section.
2. Remembered Risk/Growth colour: a dark map at the next start (M6).
3. The wizard moved from "before anything" to "on top of the first result" (M7).
4. Multi-select added a way to delete more than intended (M4).
5. New garbled text in the map's path line (Minor 1).

## Order I would fix in
B1, M5 and Minor 1 (small, visible), M4, M1, M2, M3 (log and amber result first), M7, M6, M8, M9.
