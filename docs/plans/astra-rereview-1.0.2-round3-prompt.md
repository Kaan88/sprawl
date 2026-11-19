# Prompt for GPT Astra: round 3, re-review of the round-2 fixes

You re-reviewed the 1.0.2 fixes. Your report is `docs/plans/astra-rereview-1.0.2.md`, with
changes requested in every section. Each finding you didn't mark fixed is now fixed on main
in 13 commits, 5e2955d..1a96085, titled "Astra 1.0.2 re-review, part 1..13". Nothing is
released or pushed.

Please re-review **only these fixes**, not the rest of the codebase.

- **What to work from:** the section "Fixes for Astra's 1.0.2 re-review (round 2)" at the top
  of `.claude/fable-review-queue.md`. It has:
  - one item per finding, numbered as in your report, with its commit, files, functions and
    tests;
  - the decisions the user made;
  - a new finding.
- **For each finding:** say whether it is fixed. If it isn't, give the smallest acceptable
  remaining change.
- **New problems:** report any problem a fix introduces, with its file and line.
- **Verdicts:** end with a verdict per section.

## Decisions the user made (please judge them as stated)

1. **Codex (S5 #1) stays, as a risk the user accepted.**
   - You asked for Codex to be turned off. The user declined.
   - AI setup and the first-start wizard now show this warning whenever Codex is chosen: Codex
     still offers its model a code runner and helper agents, and the check can't prove what it
     will do.
   - AI setup no longer suggests Codex.
   - The check is tightened (model included, blank and unknown output refused, bounded and
     stoppable probes). It is described as a drift check, not a boundary.
   - Please judge whether the warning states the risk clearly. Don't judge whether the check
     enforces report-only execution: it is said not to.
2. **S1 #2: the old shared task is no longer deleted at all** (your minimum).
3. **S1 #3: no scan-time folder IDs.**
   - A changed folder is gone through only when every old entry in it is one the scan saw
     there, with the same name, kind and length.
   - Otherwise it is held whole.

## Points the author is least sure of

1. **S1 #3.** Is name, kind and length, checked for every old entry of a changed folder, an
   acceptable stand-in for object identity? Or should changed folders simply be held? That
   would stop an active cache from being cleaned until a fresh scan.
2. **S1 #4.** The probe and the run share a binding:
   - flags `--cache`, `--cache-dir`, `--store-dir` and `--cache-folder`, each naming
     `$env:SPRAWL_TOOL_DIR`;
   - `GOCACHE` and `NUGET_PACKAGES`.

   The place the probe names is held without delete sharing while the tool runs. Does the
   flag or variable win over settings files for each tool, especially yarn 1 and pnpm? Is
   holding conda the right call?
3. **S1 #6.** Absence of the Keep list is accepted only for a missing leaf whose parent is a
   plain folder, or is absent inside a folder. Is that check of the parent chain enough?
4. **Request log (other per-user writes).** It keeps logging when elevated, through a checked
   append: the folders held, the leaf opened as itself, one link only, the final path checked,
   and rotation through the handle. Is that acceptable in place of skipping the log?
5. **New finding: `local::pin()` doesn't hold its leaf.** It opens with
   FILE_READ_ATTRIBUTES only, so the pinned leaf can be renamed. Should `pin()` hold the leaf,
   given that this would block Sprawl's own renames and deletes of pinned folders?

## Limits of the author's own testing

- The test shell was elevated, so unelevated-only checks skipped themselves. Please run
  `cargo test --release --workspace --no-fail-fast` once from an unelevated shell.
- No real `codex exec` run was made, and no real npm, pip, NuGet, pnpm, yarn or Go clean-up.
- There are no screenshots and no cross-account pipe test.

Read-only: please don't push, release, register tasks, or run clean-ups.
