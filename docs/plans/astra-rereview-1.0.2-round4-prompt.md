# Prompt for GPT Astra: round 4, re-review of the round-3 fixes

You reviewed the round-2 fixes. Your report is `docs/plans/astra-rereview-1.0.2-round3.md`,
with changes requested in all six sections. Each finding you didn't mark fixed is now fixed
on main in 7 commits, 10a498b..8a6bc0d, titled "Astra 1.0.2 round 3, part 1..7". Nothing is
released or pushed.

Please re-review **only these fixes**, not the rest of the codebase.

- **What to work from:** the section "Fixes for Astra's 1.0.2 re-review (round 3)" at the top
  of `.claude/fable-review-queue.md`. It has one item per finding, numbered as in your
  report, with its commit, files, functions and tests, and the decisions below.
- **For each finding:** say whether it is fixed. If it isn't, give the smallest acceptable
  remaining change.
- **New problems:** report any problem a fix introduces, with its file and line.
- **Verdicts:** end with a verdict per section.

## Decisions (please judge them as stated)

1. **S1 #3: scan-time file IDs after all.** Holding every changed folder whole would keep
   Temp and active caches from ever being cleaned. Each scanned node now carries the file
   ID the scan saw: from the folder listing (FileIdExtdDirectoryInfo, else
   FileFullDirectoryInfo), or from the MFT. In a changed folder, an entry is gone through
   only when its ID is the one the scan saw there. A tree without IDs (CSV, History) holds
   a changed folder whole. The release speed guard shows no slowdown.
2. **S1 #4:** pnpm and Yarn are held like conda. Go runs with `GOENV=off`, `GOFLAGS` empty
   and `GOTOOLCHAIN=local`. npm's logs are kept in the reviewed folder.
3. **S5 #1:** Codex stays as an accepted risk. The warning is your proposed text, plus
   "Choosing Codex accepts that."

## Points the author is least sure of

1. **S4 #5.** Before the original is removed, it is marked delete-on-close
   (FileDispositionInfo, not POSIX), and only then is its stream list read a last time. The
   test shows that, once marked, creating a new stream, opening one, and opening the file
   itself are all refused. Is that the guarantee you asked for, that stream membership stays
   stable through disposal, on NTFS and ReFS?
2. **S1 #3.** A 128-bit file ID whose high half isn't zero (ReFS) is stored as unknown, and
   then the time checks alone apply to that entry. Should such an entry be held instead?
3. **S1 #10.** Without level 2 (Sprawl not running as administrator), the folder of a share
   this PC serves is unknown. A personal folder redirected to such a share then makes the
   never-list incomplete, which holds every clean-up. Is that acceptable, or too strict?
4. **S4 #4.** A pinned root is classified through its own handle (FileFsDeviceInformation:
   FILE_REMOTE_DEVICE, or a DFS or network device type). Are there local volumes that report
   themselves remote, or remote ones that don't?

## Limits of the author's own testing

- The test shell was elevated, so unelevated-only checks skipped themselves. Please run
  `cargo test --release --workspace --no-fail-fast` once from an unelevated shell.
- No real `codex exec` run was made, and no real npm, pip, NuGet or Go clean-up.
- There are no screenshots and no cross-account pipe test.

Read-only: please don't push, release, register tasks, or run clean-ups.
