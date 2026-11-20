# Plan for 1.0.4

Written 2026-11-20. It collects Astra's review of the 1.0.3 fixes, the user's UI reports, the
in-app commands request, the scan-speed work and the dev-folder feature. Nothing below is built
yet, except where "Done (uncommitted)" says so. UI changes need real screenshots approved by the
user before they are merged (see `status.md`). Safety code goes through an outside review
(Fable, or Astra run by the user), with a self-contained entry in `.claude/fable-review-queue.md`.

## Already done, not committed yet

| # | What | State |
|---|------|-------|
| D1 | **Faster C: scans.** The MFT fast read failed on a fragmented system drive (`$MFT`'s `$ATTRIBUTE_LIST` is non-resident), so Sprawl walked the drive instead. `mft.rs` now reads a non-resident attribute list. Measured: 16.9 s before, 7.6 s after (3.9 s MFT read + 3.7 s sort). `SPRAWL_TIMING=1` writes step times to `%TEMP%\sprawl-timing.log`. | Committed c42c897. |
| D2 | **Effort levels per model** (`sprawl-advisor/src/llm/effort.rs` and the providers), built in a separate session. | Committed on branch effort-levels (79a873a), not merged. |

Follow-ups to D1: sorting (3.7 s) is the next thing to speed up. The fast read finds about 100k
fewer entries than the walk on C: (3.79M vs 3.89M), probably hard links; check this.

## A. Astra's review of the 1.0.3 fixes (safety; outside review after fixing)

| # | What | Fix |
|---|------|-----|
| A1 | **Fixed.** **[P2] Branch rescans drop the new scan's volume serials.** `commands.rs:975` `replace_branch()` copies the new nodes but not `sub.volumes`. After a drive is reassigned and a branch refreshed, freshly scanned files keep the old serial, and the mismatch check holds them. Refreshing a branch of a snapshot-based tree leaves it with no serial at all. | Update the volume entries for the replaced branch: remove the ones under it, add the new scan's, and keep unrelated branches. A new scan with an unknown volume must not take over an old known serial. Tests: snapshot-to-live refresh, and a branch refresh whose volume changed. |
| A2 | **Fixed.** **[P2] A retry that compresses nothing more leaves the record partial.** `cleanup.rs:1064` skips the record update when freed, compressed and already are all zero. Example: run 1 compresses A, B is busy; on retry A is skipped (already WOF) and B doesn't benefit, so `partial: true` stays and the suggestion keeps coming back. The same happens when B was deleted in between. | Work out completion before that check. A clean retry with zero saving completes an existing partial record and keeps its earlier saving. Test the "A already compressed + B doesn't benefit" case. |
| A3 | **Fixed.** Hardening: files with no listed ID (zero ID) are only checked for current eligibility, not that they are the planned file. | Skip zero-ID candidates in the compressor (or, later, keep full IDs). Don't describe this path as protected from replacement. |
| A4 | **Fixed.** Overlapping roots: `seen_in()` finds nodes with `history::find_path()` (first matching root) while `volume_of()` picks the deepest root. | Use one rule for both. When overlapping observations disagree on the volume, return unknown. |
| A5 | **Fixed.** Wording: the Undo estimate is capped by `max(it.bytes, o.freed)`, not by `it.bytes`; old savings can survive app updates. | Fix the description/comment. |

Accepted as is: compressor fixes, clean-up hard-link fix, case-only roots, OwnedDir removal,
UI tick/refresh fixes, `bind(None)` keeping ticks, an empty owned folder left on anchor
failure, the fallback for trees without volumes. S4 #5 remains an accepted risk.

## B. The user's UI reports

| # | What | Plan |
|---|------|------|
| B1 | **No choice of compression type.** | Add an algorithm choice (XPRESS4K / 8K / 16K, LZX) to the compress action, with a sensible default. Mockup first. |
| B2 | **No real vs expected size** in the selection section. | Show the size on disk now and the expected size after the action (per row and total). Mockup first. |
| B3 | **Orange/grey rows jump vertically on the first click.** | Reproduce, find which element changes height on first selection, fix. |
| B4 | **Codex is told "Use Ask Claude".** `sprawl-advisor/src/llm/agent.rs:44` hard-codes the name. | Use the active provider's button name (or a neutral wording). |
| B5 | **Deleting shows only small footer text.** | A visible progress bar (items, bytes) that doesn't block using the tree view. |
| B6 | **The review button is far from the tick buttons; the bar is confusing.** | Redesign the bar so ticking and reviewing sit together. Mockup first. |
| B7 | **Cleaners is confusing.** It reads this account's folders on C: whatever drive is scanned, and any scan start clears its results. | Keep Cleaners' results across drive scans until Cleaners is run again; say plainly that it looks at this account and Windows, not the scanned drive. |

## C. Run commands in the app (DISM and friends)

The built-in rules `win.component-store` (DISM), `win.recycle-bin` and `win.windows-old`
(cleanmgr) currently only show a command. Plan:
- A Run button that runs the rule's own command inside Sprawl, with its output shown.
- When the command needs administrator rights and Sprawl isn't elevated, offer "Restart as
  administrator". If a rule doesn't apply without admin, don't show it to non-admins.
- `cleanup.rs:333` `untickable` blocks Tool commands outside USERPROFILE ("Another account's"):
  add an exception only for machine-wide commands from the built-in rules, never for
  user-made or AI-proposed ones.
- Safety code: outside review.

## D. Dev folders (new feature)

Today `projects.rs` finds projects by marker files and suggests their build outputs once the
project has been idle; active outputs get their own never-ticked group. It skips `.git`
entirely and knows nothing about worktrees, history size or re-cloneable repos. Add:

1. **Old worktrees**: folders whose `.git` is a file pointing to a main repo (Claude Code's
   `.claude/worktrees/*`, `git worktree add` checkouts). Suggest when old, branch merged or
   gone, and clean. Remedy `git worktree remove` (git itself refuses on uncommitted work).
   Records whose folder is gone: `git worktree prune`.
2. **Large `.git`** with many loose objects or packs: remedy `git gc`, run by git.
3. **Old clones that can be fetched again**: untouched for months, nothing uncommitted,
   nothing unpushed, no stashes. Amber, with evidence ("everything is on origin, last commit
   March 2026").
4. **Same remote cloned more than once**: point it out; the user decides.

Safety: Sprawl never deletes inside `.git`. Checks are read-only git calls (`git status
--porcelain`, `git log @{u}..`, `git stash list`, `git worktree list --porcelain`) on the
advisor thread; without git, no suggestion. Any uncommitted or unpushed work means no
suggestion. Builds on C for running git's own commands. Outside review required.

## Suggested order

1. Commit D1 and D2 (after the user's yes).
2. A1–A5 (small, safety), then the outside review.
3. B3, B4, B7 (fixes without new design).
4. C (in-app commands), then outside review.
5. B1, B2, B5, B6 with mockups for approval.
6. D (dev folders), then outside review.
