# Usability pass

Based on two independent reviews of d22c76f (Opus 5.5 and Fable 5.2, each looking as a normal user
and as a power user, from the code and screenshots).

## Where they agree

- Script export: fixed. Clean-up updating results: fixed.
- The AI side of the redesign isn't finished: "Add to plan" text remains, AI findings sit in a
  separate tab where the clean-up bar and review aren't drawn (Fable: "Clean up just this..." from
  Ask does nothing visible).
- Clean-up never says it is permanent, while the tree's Delete defaults to the Recycle Bin.
- The Advisor is squeezed into one pane; the tick-and-clean bar scrolls away.
- Advisor messages go to the header status line, far from the button pressed; "Clean up now"
  stays enabled when it can't run.
- Two colour controls that each work half the time; "Types" colours folders a muddy average; no
  legend.
- The 17 views: bare names, no grouping, hint only in an overwritten footer, inconsistent
  click/wheel, choice not remembered. Weak ones: Spectrum, Grove, Molecule, Rose Stack,
  Ridgeline, Tree Rings, Isobars.
- PowerShell still shows in every finding's details.
- Every tick rebuilds the whole panel (focus and scroll lost); thin accessibility.
- Long jobs: Exit, Restart as administrator or a new scan during a clean-up give no warning.
- Fixed 1680x1080 window, not remembered.
- Stale text: advisor.rs:303 (MCP hint points at the script), wizard.rs:63, agent.rs:41, the
  unreachable Plan code.

## Only one noticed

- Opus: views redraw every frame and ignore Animations off; no clean-up log; admin restart drops
  ticks and context; Info rows show folder size with a green dot; review hides each amber item's
  real cost; Sprawl's own deletes counted as "Live" changes; AltGr shortcut clashes.
- Fable: History does nothing in the More views; growth spread over five places; risk words
  mislead ("comes back by itself"); no multi-select in tree and file list; pre-ticked items not
  explained; result panel ends with only OK; same-day scans labelled with the same date.

## Where they differ

- Which views to keep. Both keep Rings, Waffle, Flow, Tideline. Fable also keeps Flame, Suds and
  Stained Glass and drops Library and Strata; Opus keeps Library. The user decides (they said
  they'd eliminate after trying them); the plan only groups and hides.
- Recycle Bin: Opus wants own and AI-suggested files sent there; Fable says "consider". Plan:
  amber and orange items go to the Recycle Bin, green ones (caches) are removed for good, and the
  review says which is which.

## Milestones

### 1. Trust (both blockers)
- Amber and orange items go to the Recycle Bin (per item, through the shell's recycle API, still
  links-as-links: refuse to recycle a folder that holds a junction and fall back to asking).
  Green items removed for good.
- The review and the result say, per item, "To the Recycle Bin" or "Removed for good".
- Under each review row, the finding's one-line cost from its evidence.
- "Tick enough for N GB" names the amber items it added.
- Clean-up log: `%LOCALAPPDATA%\Sprawl\cleanup-log.txt` (time, path, freed, errors), linked as
  "See what was removed".
- Result card: amber when something failed or nothing was freed; show the new free space and a
  "Find more" link instead of only OK.
- Safety review (Opus xhigh) before commit: the recycle path is new deletion code.

### 2. One Advisor, with room
- The Advisor takes the full width while open (the file-types pane hides).
- Header folded into one line; sort and filter chips into a small menu.
- The tick-and-clean bar pinned to the bottom, outside the scroll area; it, the review and the
  progress are drawn whatever section is open.
- AI findings live only in the Findings list (with the AI badge); "Ask the AI to look further"
  at the end of the list. The Ask tab becomes the chat only ("Ask a question"); a chat answer
  that names a finding's path links to its row. When AI is off, the setup choices show in place.
- Every Advisor message shown in the panel next to its button; "Clean up now" disabled until
  the own-files box is ticked.
- A line under the bar when Sprawl ticked things itself: "Sprawl ticked the safe ones for you".
- Ticks kept across tree changes for paths that still exist.
- Only the changed row updates on a tick (no full rebuild); focus and scroll restored after any
  rebuild; the review scrolls into view when it opens.

### 3. Words and leftovers
- Risk words: "Safe to delete (rebuilt when needed)", "Can be got back, takes time", "Your own
  files". Chips: "Can clean now", "My own files". Header says "6 tips, nothing to clean" at zero.
- Info rows show the gain with a neutral dot.
- The command hidden behind "Show the command", after "Sprawl does this for you".
- Rewrite advisor.rs:5, :303, :2203, :2263, agent.rs:41, mcp.rs:45, wizard.rs:53/:63.
- Delete the unreachable Plan code (Section::Plan, AdvPlan, AdvPick, AdvPreview, the plan half
  of ai_card, the unreachable verify step).
- Rename "Memory" to "Remembered"; label the old map buttons' replacement (see 4); one spelling
  (British, as the UI mostly uses); drop "Rust + WinUI 3", "Checked in 0.0 s", MFT jargon in the
  header; plain scan-failed message.

### 4. Views and colour
- One "Colour:" switch beside the view buttons, always visible, for every view including
  Treemap, City and Galaxy; the Advisor's own map buttons removed. Opening the Advisor no longer
  darkens the map by itself. A small key in the map corner for every mode; a plain message when a
  mode has no data ("Risk needs the Advisor's check", "Growth needs an earlier scan").
- "Types" colours a folder by its biggest type, not an average.
- The More menu grouped: Size (Rings, Flame, Waffle, Stained Glass, Suds, Library), What can go
  (Flow, Strata), Over time (Tideline, Tree Rings, Isobars, Rose Stack), Experimental (Grove,
  Lens, Spectrum, Ridgeline, Molecule), each with a one-line description and a tick on the current
  one. The hint as a caption on the map, not the footer.
- One rule everywhere: click selects, double-click goes in, Backspace goes out, wheel zooms.
  Waffle's filter becomes a visible toggle; Lens re-centres on double-click.
- Views and colour remembered.
- Redraw only while something moves; Animations off reaches the views.
- History: opening it switches to Treemap (the time views link to it); time labels show the
  time when scans share a date.
- Fix the visible problems both saw: Grove and Molecule framing (check after the fixes in
  244d020), Spectrum size, Rose Stack labels.

### 5. Long jobs, window, keyboard
- Ask before Exit, closing or Restart as administrator during a clean-up; Scan disabled while
  cleaning; Scan turns into Stop while scanning.
- Restart as administrator passes `--advisor` and the ticked paths; the new copy reopens the
  Advisor with them ticked.
- Window sized to the screen's work area and remembered; the header folds on narrow widths.
- Sprawl's own deletes don't count as "Live" changes.
- Ctrl+1..4 for the tabs; arrow keys in the types table; type-to-find in the tree; Ctrl+Alt
  shortcuts that don't clash with AltGr moved.
- Accessibility: names on tick boxes and buttons, a risk word or icon beside each dot, smallest
  text 12 px at 4.5:1 contrast.
- Multi-select (Ctrl/Shift) in the tree and file list with one confirm for the total.

### 6. First run and settings
- The welcome screen first, full window; the wizard offered after the first scan, cut to plain
  choices.
- One Settings page: look, history, Advisor (uninstaller, Claude Code setup), AI.
- A Help menu with an up-to-date shortcut list; About leaves the tabs.

## Order and checks

1 and 2 first (the two blockers and the flow the user complained about), then 3, 4, 5, 6.
Each milestone: build, screenshots on `C:\Program Files` (display on), tests, commit and push.
Milestone 1 gets an Opus xhigh safety review; the whole pass gets one Fable usability read at
the end.
