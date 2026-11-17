# Plan: instant file-name search

A plan only: nothing here is built. Numbers marked "est." are estimates to measure in phase 1.

## Is it redundant? Should it be a tab?

**Partly redundant. Build a small Search tab over the scan Sprawl already has. Do not build
a second Everything.**

What Sprawl has today covers more than it seems:

- The filter box already finds names. `setup` matches anywhere in a name, `*.mp4|*.mkv` and
  `*\cache\*` work, case is ignored (`filter.rs`).
- File View lists **every** file of the scan, not 1000 (`largest_files(t, usize::MAX)` in
  `state.rs`; the README is out of date). Its columns sort. So "filter, then File View" is
  already a name search with sizes.

What that does not give:

- **It is not instant.** The filter waits for a pause, then builds a whole new tree
  (`filter_tree` clones and re-totals millions of nodes) and re-sorts everything.
- **It is destructive.** The map regrows, the selection, the open folders and the back stack
  are lost. Looking for one file should not rearrange the analysis.
- **Folders are not found by name.** A name pattern matches files only. `node_modules` as a
  folder needs the path form `*\node_modules\`.
- **Only the scanned folder.** Nothing outside it, nothing before a scan.

What Everything gives that Sprawl should **not** chase: an index that is always there, for all
drives, kept current by a service. Sprawl is opened now and then, usually without admin.
Everything is free and does that job well. Copying it means a service, a database on disk and
a lot of care for little gain.

Sprawl's own angle is different: **find by name, see how big it is, see it on the map, delete
it through the checked path.** That is worth a tab, because results mix files and folders,
must leave the tree alone, and later may cover more than the scan.

If even that feels like too much: the cheapest version is no tab at all. Make the filter box
narrow File View on every keystroke (phase 1's matcher) and keep the tree rebuild for Enter.

## Where the names come from

| Source | Needs | Covers | Plan |
|---|---|---|---|
| The scan tree in memory | a finished scan (MFT or walk) | the scanned folder | Phase 1. Works without admin. |
| The last full snapshot (`history.rs`, up to 3 kept per root) | history on, an earlier scan | that scan, as of its date | Phase 3. Search before the first scan of a session. Rows marked "as of <date>". |
| The MFT without a scan | admin, NTFS | whole volumes | Phase 4. Little gain for the scanned volume: a scan as admin *is* the MFT read. The gain is other drives, and the rest of the volume when only a folder was scanned (`mft::scan` reads the whole MFT and then drops it). |
| The USN journal | admin, NTFS | changes since a known point | Phase 5, maybe never. Save the journal id and position with a snapshot; on start, load and replay. That is Everything's trick. |

Staying current without the journal: the tree already follows Sprawl's own deletes and branch
rescans (`apply_rescans`). The watcher (`watch.rs`) only pulses the map today. Phase 2 uses
its paths to mark a folder "changed since the scan" in results. Every action checks that the
path still exists first.

Without admin: phases 1 to 3 work in full. Phases 4 and 5 do not; the tab says so once, next
to the existing "Restart as Administrator" hint.

## Memory and speed

A system drive is about 3.4 million nodes. The tree already costs 120 bytes a node plus the
name. Search adds, built on a background thread after the scan:

- one lower-case UTF-8 blob of all names: about 85 MB (est., 25 bytes a name);
- one `u32` offset per node: 14 MB.

Budget: **under 100 MB extra, built in under 1 s, a query answered in under 50 ms** on
3.4 million names (est.; a plain byte scan of 85 MB, split over the cores). No trigram or
suffix index: a linear scan is fast enough and has nothing to keep in step.

- A plain word is a substring search on the blob. Wildcards and path terms fall back to
  `filter::Filter`, slower but rare.
- Path terms build the path only for nodes whose name part could match.
- Each keystroke starts a new search and cancels the old one (a generation counter).
  The UI thread never searches.
- The blob is dropped and rebuilt when the tree is replaced (new scan, filter on or off).
  Deletes and branch rescans rebuild it too; if that shows as a stall, mark dead nodes instead.
- Phase 4, per extra volume: name, parent, size per record, about 45 bytes: 150 MB for
  3.4 million files (est.). Opt-in per drive.

## Syntax (small on purpose)

- `report 2024` : both words, anywhere in the name, any case. Space means "and".
- `"my file"` : quotes keep a space.
- `*.iso`, `img_????.jpg` : `*` and `?`, as in the filter.
- `\steam\ *.vdf` : a word with `\` matches the full path.
- `a|b` : either. `!tmp` : not.
- `>1gb`, `<10kb` : size.
- `folder:` or `file:` : one kind only.

No regex, no dates, no file contents, no saved searches. The same words must mean the same in
the filter box, so the filter's rules are the base and both share one parser.

## Results and actions

- A list like File View: Name, Size, Allocated, Modified, Folder. Files and folders together,
  folders with their totals. Sorted by size, biggest first; headers sort as elsewhere.
- The rows are virtualised already (`tables.rs`), so all matches are listed. The status line
  shows the count and the total size.
- The matched part of the name is bold.
- Actions, all existing code, no new delete path:
  - Enter or double-click: reveal in the tree (`reveal` in `treemap.rs`) and select on the map.
  - Open, Explorer, command prompt, copy path: the existing commands on `selected_path`.
  - Delete, to the Recycle Bin or for good: the existing `delete` and `delete_many`, with
    `delete_refusal` and the marks unchanged.
- Rows from a snapshot or another volume (phases 3, 4) are not tree nodes. They get Open,
  Explorer, copy path and "Scan this folder". **No delete** until they are scanned.

## Tabs and the map

- A fourth tab: Tree View, File View, **Search**, Advisor. A new `View::Search`.
- Ctrl+F opens it and focuses its box (the filter stays on Ctrl+Shift+F).
- The filter box stays what it is: "narrow the whole analysis to these files". Search is
  "where is that file". A button in the Search tab, "Use as filter", hands the text over.
- With a filter on, Search looks in the filtered tree and says so.
- The map does not change shape. Matches glow, the rest dims, as with the file-type
  highlight. Selecting a row selects its tile.

## Privacy

- The index lives in memory and dies with the window. Nothing new is written to disk.
- Nothing leaves the PC. No network code is touched.
- Queries and results are not given to the AI providers, and the MCP server gets no search
  tool in this plan. If that is wanted later, it is its own decision and review.
- Search terms are not saved or logged.

## Phases and checks

1. **Core matcher** (`sprawl-core/src/search.rs`, no UI). Parser, blob, parallel scan.
   Check: tests for every syntax line above; results equal `filter_tree` for the shared
   patterns; a bench on 3.4 million made-up names meets the budget.
2. **The tab.** `View::Search`, the list, Ctrl+F, actions, map glow, "changed since the scan".
   Check: type fast on a real C: scan with no stutter; tree, selection and map are as they
   were after leaving the tab; delete from results goes through the same refusals;
   `--screenshot` picture of the tab.
3. **Search the last snapshot** before a scan. Check: start, type, results within 2 s (est.);
   rows are dated; a path that is gone says so; no delete offered.
4. **Other volumes by MFT** (admin). Check: counts match a full scan of that drive; memory
   within budget; without admin the option is absent, not broken.
5. **USN replay** (admin). Only if phase 3 results prove too stale in real use. Check:
   create, rename, delete files while Sprawl is closed; the index matches a fresh scan; a
   journal that was reset or wrapped falls back to a rescan.

Stop after phase 2 and use it for a while. Phases 3 to 5 are only worth it if people search
before scanning, or outside the scan.

Review: phases 1 and 2 add no delete path and read no new disk structure. Phases 4 and 5
touch raw volume reads and get the usual independent review.

## Left out

- A background service or tray icon, and an index database on disk.
- Searching file contents, regex, date terms, duplicates.
- Network drives and non-NTFS volumes beyond what a scan already holds.
- A global hotkey or a search window outside Sprawl.
- Natural-language search ("videos older than 2 years", from `docs/ideas/crazy-ideas.md`).
- Replacing the filter box.
