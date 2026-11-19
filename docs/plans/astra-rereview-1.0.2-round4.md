# Astra's round-4 re-review of the 1.0.2 fixes

Astra's answer to `astra-rereview-1.0.2-round4-prompt.md`, 19 November 2026. It reviewed the
seven round-3 commits through `8a6bc0d`. Local source links below are relative to the
repository root.

**Status: fixed for 1.0.3** in `3b94fec` (S4 #5, S1 #3) and `a50608c` (S1 #4, S2 #4, S3 #2,
S4 #4, S5, S6), not yet reviewed again. How:
- **S4 #5:** the original is made read-only before its last stream check (Windows then
  refuses a new stream on it), and only then marked for deletion with FileDispositionInfoEx,
  ignoring read-only. Exiting at any point leaves a checked original going, or the original
  kept. Astra's smaller fix, keeping every original, would have made linking save nothing.
- **S1 #3:** volume and full 128-bit id (FILE_ID_INFO). An item not known so, in a folder
  changed since the scan, is held with its subtree. A file root gets its id. The preview
  uses `Spare::is_new` itself.
- **S5:** the final reopen compares the volume and full id, without delete sharing; it
  leaves the folder when they can't be read.
- **S6:** children are opened without delete sharing. The replacement test swaps the
  folder at the real reopen step. Removing either guard fails a test (checked).

Originally: deferred to the next release (1.0.3). None of these findings is a regression
from 1.0.1, and the user chose to release 1.0.2 with them open. Why each one is deferred:
- The S4 #5 window needs a new stream to land on a duplicate's original within the moment
  between its deletion mark and the final check, and Sprawl to exit in that same moment.
- The S1 #3 unknown-ID case falls back to 1.0.1's timestamp checks. It applies only on
  ReFS, or where Windows' listing gives no IDs.
- The rest don't delete anything that was in the review.

---

**Changes requested.** Astra reviewed the seven implementation commits through `8a6bc0d`
against the round-3 findings. The release suite passed in a verified non-administrator
Windows shell on NTFS: **389 passed, 0 failed, 10 ignored**. The most consequential
remaining issues:
- a duplicate's original data can be lost if Sprawl exits during linking;
- clean-up can go ahead when a changed folder holds an entry whose identity Sprawl can't
  verify.

## Section 1 — Clean-up safety

| Finding | Result | Evidence and smallest remaining change |
|---|---|---|
| **S1 #3 · Changed folders** | **Partly fixed** | A differing known ID now protects the entry. But once a changed parent passes its own ID check, a child with `SeenAt::Unknown` can still pass on old timestamps alone. This matters on ReFS, and when directory enumeration falls back to a format without IDs. The comparison also omits the volume, and a directly scanned file returns before its root ID is assigned. Require a matching **volume and full file ID** for each entry admitted under the changed-folder rule. Hold an unknown child and its subtree. Assign the file root's identity before the early return. `crates/sprawl-advisor/src/clean.rs:245` · `crates/sprawl-core/src/walk.rs:140` · [Microsoft's ReFS identity contract](https://learn.microsoft.com/en-us/windows/desktop/api/fileapi/ns-fileapi-by_handle_file_information) |
| **S1 #4 · Package tools** | **Partly fixed** | pnpm and Yarn are held. Both Go invocations receive the requested environment. Both npm invocations name `_logs`, but the probe starts before Sprawl holds the cache, and `_logs` itself is never checked or held. npm can rotate logs during that probe, so a redirected `_logs` can affect files outside the reviewed cache, even if the later check refuses the clean-up. Hold the cache and a verified plain `_logs` child through **both** invocations, or hold npm. `crates/sprawl-advisor/src/clean.rs:345` · [npm's log rotation](https://raw.githubusercontent.com/npm/cli/latest/lib/utils/log-file.js) |
| **S1 #6 · Keep list** | **Fixed** | The read starts at AppData, opens each component through its held parent, accepts a missing component, and reads an existing leaf through the same chain. `crates/sprawl-core/src/local.rs:552` |
| **S1 #9 · Unknown mappings** | **Fixed for the reported cases** | An unknown source letter covers every letter without a positive local classification. A zero drive mask also covers every letter for a UNC exclusion. The settings test now supplies its own mapping. `crates/sprawl-advisor/src/exclusions.rs:160` |
| **S1 #10 · Local share backing folder** | **Fixed** | Holding clean-up when the live backing folder is unknown is appropriate: a redirected personal folder could be inside the deletion scope, and the saved registry value can't establish where it is now. `crates/sprawl-advisor/src/safety.rs:698` |
| **S1 #1 follow-up · Per-user writes** | **Fixed for the reported race** | Appends serialize within the process and retry sharing conflicts. The log is opened under its held parent without delete sharing, and rotation uses that parent's handle. The 160-line concurrent append test passed. `crates/sprawl-advisor/src/store.rs:94` |
| **`pin()` follow-up** | **Fixed for the leaf** | The leaf now requests data access and denies delete sharing. The file and directory rename tests passed. `crates/sprawl-core/src/local.rs:439` |

The S1 #3 change also leaves **preview and execution on different rules**. Preview still
counts files from timestamps alone, so it can promise space from a replacement that
execution will keep. Use the same read-only identity decision when calculating the preview.
`crates/sprawl-advisor/src/clean.rs:571`

## Section 2 — Codex output

**S2 #4 · Partly fixed.** The reader now has to finish, and malformed tails and drain
timeouts are rejected. One stale-answer case remains. `turn.started` clears the completion
flag but keeps the previous answer. If the final turn completes without an agent message,
Sprawl returns the earlier turn's text with the final turn's usage. Clear the saved answer
at each turn start, and require an answer from the turn that completed.
`crates/sprawl-advisor/src/llm/codex.rs:356`

## Section 3 — Restricted clients

- **S3 #2 · Partly fixed.** The restricted owner index tells case-only siblings apart, and
  Recent collection keeps the shortcut's spelling. Two attribution gaps remain:
  - `attach_usage` still picks the first case-insensitive folder match before `scoped_to`
    copies the app's last-use time, so a restricted owner lookup can show its sibling's
    usage. Resolve usage against the exact admitted node, or omit ambiguous usage.
  - Exact Recent matching compares the drive letter literally, although admission
    normalizes it, so a valid `c:\…` record can disappear on lookup under `C:\…`.
    Normalize the drive prefix the same way everywhere, while keeping component case.

  `crates/sprawl-advisor/src/facts.rs:271` · `:205` · `crates/sprawl-advisor/src/used.rs:40`
- **S3 #4 · Fixed.** The pipe comment correctly names the connection's opener as the
  authenticated context and explains the one-request lifetime.
  `crates/sprawl-advisor/src/pipe.rs:89`

## Section 4 — Destructive actions and disclosure

- **S4 #5 · Changes requested, high priority.** The NTFS fixture passed: after the deletion
  mark, all three attempted opens failed. The remaining data-loss window is **before the
  final stream check**. If a stream arrived before the mark and Sprawl exits after setting
  it, Windows closes the handle, and the original can disappear before Sprawl checks the
  stream. A failed attempt to clear the mark has the same outcome. The smallest safe change
  is to keep `.sprawl-original` and credit zero bytes until disposal can preserve data
  across process exit. The NTFS fixture doesn't establish a ReFS guarantee.
  `crates/sprawl-advisor/src/clean.rs:1259` · [Windows deletion-on-close behavior](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-deletefile)
- **S4 #4 · Collection fixed, disclosure partly fixed.** Recording the opened root's
  device information addresses the lookup at publish time, and `FILE_REMOTE_DEVICE` is a
  documented signal for a Windows network redirector. The remaining defect shows in the
  mixed summary: it says every remaining folder is on this PC, then says one of those
  folders' location is unknown. Print the confirmed network, local and unknown counts
  separately. The current test checks the opening and closing phrases and misses the
  contradiction between them. `crates/sprawl/src/coverage.rs:114` · [Microsoft's redirector requirements](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-_fsrtl_advanced_fcb_header-fsrtlregisteruncprovider)
- **S4 #3 · Fixed.** Mixed outcomes now name the "To the Recycle Bin" group, including
  Bin + Tool, Link, Compress and Shrink. `crates/sprawl/src/cleanup.rs:1416`

## Sections 5 and 6 — Codex controls and folder teardown

| Finding | Result | Evidence and smallest remaining change |
|---|---|---|
| **S5 #1 · Warning and tool inventory** | **Fixed** | The warning states the accepted risk directly. The check requires an `additional_tools` record and validates its contents. `crates/sprawl-advisor/src/llm/codex.rs:22` · `:230` |
| **S5 · Run-folder ownership** | **Creation fixed, removal partly fixed** | Exclusive creation relative to the parent returns the owned handle. Final removal drops that handle and then trusts a reopened folder's legacy 64-bit index, which Windows allows to be non-unique on ReFS and to be reused after deletion. Dispose through continuously verified identity, or leave the empty folder when that identity can't be established. `crates/sprawl-core/src/local.rs:538` · [Windows file IDs](https://learn.microsoft.com/en-us/windows/desktop/api/fileapi/ns-fileapi-by_handle_file_information) |
| **S5 #2 · Stop** | **Fixed for the requested checkpoints** | The probe's exit wait, the launch path after logging, and the stdin writer all check Stop. `crates/sprawl-advisor/src/llm/codex.rs:168` · `:414` |
| **S5 #3 · Quoted paths** | **Fixed** | A quoted Windows path runs through the closing quote on the same line. The spaces-and-Markdown regression test passed. `crates/sprawl/src/ui.rs:319` |
| **S6 · Fixture teardown** | **Partly fixed** | The read-only hard-link test passed. But recursive teardown still opens children with delete sharing, so a child folder can move out of the owned tree while recursion continues through its handle. Deny delete sharing on each child through recursion and disposal. Also, the replacement-folder test takes `a.held` before `drop(a)`, which makes `Drop` return at once. Exercise the replacement at the real reopen step, so that removing the identity check would fail the test. `crates/sprawl-core/src/local.rs:522` · `:535` · `:649` |

Review coverage: source inspection and the unelevated release suite. ReFS behaviour, live
Codex and package-manager runs, screenshots, and cross-account pipe behaviour remain
unverified. Astra made no repository edits.

## Verdicts

| Section | Verdict | Deciding issue |
|---|---|---|
| 1 · Clean-up safety | **Changes requested** | Unknown child identity, and an unchecked npm log destination |
| 2 · Codex output | **Changes requested** | A completed turn can return an earlier turn's answer |
| 3 · Restricted clients | **Changes requested** | Last-use evidence can still come from a case-only sibling |
| 4 · Destructive actions | **Changes requested** | Process exit can discard an original before its final stream check |
| 5 · Codex and Markdown | **Changes requested** | Final run-folder removal relies on a released, legacy identity |
| 6 · Reliability | **Changes requested** | Teardown can follow a moved child, and the replacement test skips the guard |
