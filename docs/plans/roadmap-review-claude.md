# Review of Astra's design review and Codex's roadmap

19 November 2026 · Claude (Opus 5.5), after Sprawl 1.0.1 · Reviews
[Astra's design and product review](design-review-1.0.md) and
[Codex's alternative roadmap](alternative-roadmap-astra.md).

**Decisions, 2026-11-19:** the user agreed to real-app screenshots from a throwaway branch
instead of mockups (nothing merges before their approval) and to a reliability 1.0.2 before any
design work. After 1.0.2, the first design release is the small batch in step 2 (agreed).

**Status:** recommendation only. No UI was changed. One small
Advisor bug found while checking was fixed (28524a0, below).

## How this was checked

I took screenshots of the installed 1.0.1 using its own screenshot mode on a scratch fixture:
a 100 MiB folder with an idle Rust project (40 MiB `target`) and a 60 MiB video. I used a
scratch profile, captured Advisor, Search and Cleaners at the app's default size, and ran
no clean-up. Astra's observations were made on 1.0.0; I checked which of them still hold
on 1.0.1.

## Verdict

Astra's diagnosis is right, and every observation I could reproduce still holds on 1.0.1.
Codex's order (scope first, then review clarity, task-local status, coverage, Search) is
mostly right. I'd change three things:

1. **Mockups have to be real.** All of Codex's mockups were rejected for not looking like
   WinUI 3. That will keep happening with HTML: it can't render WinUI controls, fonts or the
   app's map. The UI is XAML strings in Rust, so a change is cheap to make on a throwaway
   branch. The app's `--screenshot` mode then renders it for real, on the same fixture,
   before and after. Proposal: each "mockup" is a pair of real screenshots from a branch.
   Nothing merges until you approve it; a rejected branch is deleted. This replaces "mockups
   before app code" with "real screenshots before merge", so it needs your yes.
2. **Reliability goes before polish.** 1.0 shipped regressions that the user felt at once:
   slow scans, no updates while elevated, deletes refused. Before any design work, do a
   1.0.2 track:
   - Astra's code-review findings on 1.0.1.
   - The runtime checks in [status.md](status.md). The 1.0.1 → 1.0.2 update is the natural
     test of the elevated update hand-off.
   - The History cap check.
   - A scan-speed check before every release: a small benchmark run (an inventory example
     exists in `crates/sprawl-core/examples`), so a 15-55x slowdown can't ship unnoticed again.
3. **Split A2.** Its quick wins are wording and visibility. The grouped review dialog changes
   the clean-up confirmation path, so it needs its own safety review. Ship them separately.

## Item by item

| Item | Holds on 1.0.1? | My view |
|---|---|---|
| A1 scope header | Yes. The picker says "[C:] Local Disk", Total Space 929.1 GiB and the capacity bar describe C:, while the scan is a 100 MiB folder. The folder's own size shows only in the status bar ("10 items, 100.0 MiB") and the table. | Do first. Within the existing card: a "Scanned" line (folder, size, files/folders, time, method) and a separate "Drive C:" line (capacity, free). Small change, large clarity gain. |
| A2 Advisor review | Yes. With AI off, the AI pane still takes a third of the width with a paragraph of setup text, and "Set up AI" appears twice. | Quick wins first: collapse the AI pane to one line while AI is off; hide the empty selection tray; rename "Clean up…" to "Review selection…". Then the grouped review dialog as its own item, with a safety review. |
| A2 (bug, fixed) | "Folders the rules haven't explained" listed the scanned folder itself, here "…\data · 100.0 MiB". This is the duplicate prompt Astra noticed. | Fixed in 28524a0: the scan root (and each root picked for a multiple scan) is passed over. In the next release. |
| A3 Cleaners states | Yes. "Press Check to read the folders…" in the body while the failure shows in the top card and again in the footer bar. | Small; do with the A2 quick wins. |
| B1 coverage details | Not checked (new feature). | Merge into A1: a "Details" link on the new "Scanned" line. It matters more now: skipped folders work with MFT, network drives are read-only, and inventories can miss hidden compression. It should show exclusions, unreadable count, method, time and network/read-only, recorded per scan. |
| B2 Search | Yes. The global Filter box stays beside the Search box. Six filter menus, five drive toggles and a six-line drive-reading list sit above an empty result area. | Do after A. Also show drive reading as one compact progress line. Reading whole drives in the background (C: 3.4 million items) is part of what feels like slow indexing. |
| B3 first scan, B4 dense tables | Partly seen: long folder names clip in the tree root row. | Worth doing, after B2. |
| C1 polish, C2 motion | Not judged. | After A and B, as real screenshots only. The 167 ms motion targets are low value. Keep the colourful map default (Codex already says so). |
| D1 AI evidence links | No. | Hold until A and B are done and AI is actually used. |
| D2 headless report | No. | Only on demand, as Astra says. |

What the roadmap misses:
- **Scan speed as a product feature.** It is Sprawl's core promise and the user's main
  complaint about 1.0. Add a speed check to every release (above).
- **Disclosing the 1.0.1 trade-offs.** The coverage details should show skipped folders,
  network read-only scans and hidden-compression gaps.
- **The backlog.** [status.md](status.md) lists History deltas, compression drift and
  Separate again. None is urgent; History's cap bug is the only correctness item there.

## Proposed order

1. **1.0.2, reliability:** Astra's code findings, runtime checks, History cap check, the
   scan-speed check. No UI change.
2. **Quick-win batch** (one release, real screenshots first): A1 scope lines, the A2 quick
   wins, A3 Cleaners states, with 28524a0 included.
3. **Coverage details** (B1 on top of A1).
4. **Search simplification** (B2), including the compact drive progress.
5. **Grouped review dialog** (the rest of A2), with a safety review.
6. B3 and B4, then C1 and C2. D1 only if wanted; D2 on demand.

## Decisions needed

1. May "mockups" be real screenshots from a throwaway branch, merged only after your approval?
2. Reliability (1.0.2) before design work?
3. Is the quick-win batch in step 2 the right first design release?
