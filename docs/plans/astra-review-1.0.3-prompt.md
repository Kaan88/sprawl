# For GPT Astra: review of the 1.0.3 work

This file is the whole request: what to do, what changed, the decisions, and one item per
change. Paths are relative to the repository root.

## What to do

1.0.2 was released with your round-4 findings deferred
(`docs/plans/astra-rereview-1.0.2-round4.md`). Since then, on main:

- `acc323c` reworks compression after a user report (below).
- `3b94fec` and `a50608c` fix every round-4 finding.
- `754062b` holds the notes (CHANGELOG "Unreleased", the round-4 report's status).

Nothing is released.

Please review **only these four commits** (`git diff 61795e0..754062b -- crates`):

- **Round-4 findings:** for each one (numbered as in your report), say whether it is
  fixed. If it isn't, give the smallest acceptable remaining change.
- **The new compressor:** review `crates/sprawl-core/src/wof.rs` and its use as safety
  code: it changes files in place.
- **New problems:** report any problem these commits introduce, with its file and line.
- **Verdicts:** end with one verdict per section below.

Read-only: please don't push, release, register tasks, or run clean-ups or compression
outside temporary fixtures.

## Checks already run

- `cargo test --workspace` at `754062b`: app 84 passed / 1 ignored; advisor 225 / 6
  ignored; core 70 / 2 ignored; install 16. No failures.
- Clippy: no new warnings in the changed code.
- Mutation checks: the S5 identity comparison and the S6 sharing flag were each reverted
  by hand, and in both cases `a_folder_put_in_place_of_an_owned_one_stays` failed, as it
  should.
- The shell was **elevated**, so the unelevated-only checks skipped themselves. Please run
  `cargo test --release --workspace --no-fail-fast` once from an unelevated shell.
- Not done:
  - the compress UI hasn't been seen in the running app;
  - no real compression of a large folder;
  - no ReFS volume;
  - no real npm or Codex run.

## Why the compression changed (user report)

After 1.0.2, a user compressed a 3.4 GB program folder (SOLIDWORKS) and reported:

- It took far longer than similar tools, with no sign of progress.
- The done banner said "They were deleted for good."
- Other ticked items were unticked afterwards, which looked as if they had been deleted.
- The compressed folder was still suggested for compression.

The clean-up log showed that only the compression ran and that nothing was deleted. The
causes and the fixes:

| Cause in 1.0.2 | Fix in `acc323c` |
|---|---|
| compact.exe was run in batches of 64 files, after a full detailed walk, plus a whole-folder measurement walk before and another after. The progress callback was empty. | Sprawl compresses the files itself, on up to 8 threads (`wof.rs`), and reports the bytes gone through. The saving is measured per file, so there are no measurement walks. |
| The banner's words counted every item that freed space as a delete. | `done_words` (`crates/sprawl/src/cleanup.rs:242`) counts deleted, compressed, linked and shrunk items apart. Only a delete is called a delete. |
| `ticked.clear()` ran after every run, including "Clean up just this...". | Only the items that ran are unticked (`cleanup.rs:1064`). |
| A folder listing doesn't show WOF compression, so the scan's `allocated` stayed at full size and the suggestion came back. | `Memory::compressed_already` (`crates/sprawl-advisor/src/memory.rs:536`) hides a compress finding whose folder Sprawl has recorded as compressed, until it grows by more than a tenth. |

## Section 1: the new compressor (`acc323c`)

**`wof::plan`** (`crates/sprawl-core/src/wof.rs:100`):
- It pins the root's chain (`local::directory`), then lists each folder through its own
  handle (`walk::dir::list`).
- Each subfolder is opened through its parent's handle as itself (`local::open_rel`,
  `Kind::Folder`, then `check_plain`), with READ_DATA|READ_ATTRIBUTES, share READ|WRITE
  (no delete). It stays held in `Plan::dirs` until the plan is dropped.
- The whole folder is refused, before anything changes, on:
  - any reparse point, except a file whose tag is WOF's (0x80000017);
  - a cloud, recall or offline attribute;
  - a name that isn't valid UTF-16;
  - an incomplete listing;
  - a path inside or above an excluded folder (the `check` callback).
- Kept paths are skipped, and so are files under 4 KB or carrying the NTFS compressed
  attribute.

**`wof::compress`** (`:158`) and `one` (`:232`), for each planned file, on a worker:
- It opens the file through its held folder's handle with READ_DATA|READ_ATTRIBUTES and
  share READ only, so no other program can write it meanwhile.
- `check_plain(Kind::File)`.
- `FileStandardInfo`: the number of links must be 1, or the file is counted as `changed`.
- `FSCTL_GET_EXTERNAL_BACKING`: a file that is already backed is counted as `already`.
- Then `WofSetFileDataLocation(handle, WOF_PROVIDER_FILE, {1, alg, 0}, 12)`.
  ERROR_COMPRESSION_NOT_BENEFICIAL is fine. A sharing violation or lock counts as
  `in_use`, and anything else as `failed`, with the first error kept.
- The saving is the allocation size before, minus the sum of the file's streams after
  (`FileStreamInfo`).

**`remedies::compress_scope`** (`crates/sprawl-advisor/src/remedies.rs:124`) keeps 1.0.2's
command checks: one of Sprawl's own algorithms, the command naming the reviewed folder, and
exclusions checked before anything is read. It then calls `plan` and `compress`.

**The run** (`crates/sprawl-advisor/src/clean.rs:344`, `crates/sprawl/src/cleanup.rs:805`
and `:820`):
- The outcome carries `wof::Done`.
- What was freed is `before - after`, also when the run was stopped part way.
- Failed files are reported as problems; files in use are not.

**Tests:**
- `a_folder_is_compressed_in_place_through_held_handles` (`wof.rs:314`): compression, an
  unchanged read-back, already compressed, a hard link, a junction refusing the folder, an
  excluded subfolder refusing it.
- `a_file_another_program_writes_is_left_alone` (`wof.rs:360`).
- `compress_checks_the_folder_now` (`remedies.rs:666`).

## Section 2: round-4 findings, clean-up safety

- **S4 #5, the original lost on exit** (`clean.rs` `link_with`, from `:1312`):
  1. After the copy's name is linked to the kept file, the original's handle (now opened
     with FILE_WRITE_ATTRIBUTES too) sets it read-only.
  2. Its streams are read. Any stream found means the attributes are restored and the
     original is kept as `.sprawl-original`.
  3. It is marked for deletion with FileDispositionInfoEx DELETE|IGNORE_READONLY_ATTRIBUTE
     (`win::delete_on_close_readonly`, `:1622`).
  4. Its streams are read again. A failure here calls the mark off and restores the
     attributes.

  This rests on Windows refusing to create a stream on a read-only file. The test
  `a_read_only_original_takes_no_new_stream_and_survives_an_early_end` (`clean.rs:2177`)
  checks that, and that a handle closed before the mark leaves the file whole. Sprawl
  exiting before the mark leaves the original kept (read-only, at the backup name), and
  after the mark, a checked one going.
- **S1 #3, unknown child identity:**
  - `SeenAt::Item` now carries the volume serial of the scanned root (`clean.rs:202`), and
    `win::Info` carries `volume` and `id_exact` (from FILE_ID_INFO: the full 128-bit ID
    equals the short ID).
  - `Spare::is_new` (`:247`) admits a changed folder for an item-by-item pass only when it
    is `known` (`:262`): same ID, an exact ID, same volume.
  - An item that isn't known, whose parent folder changed since the scan, is held, and so
    is everything inside it. The parent is read through `win::open_attrs`.
  - A file scanned on its own gets its ID before the early return
    (`crates/sprawl-core/src/walk.rs:128`).
  - `preview` (`clean.rs:605`) now calls `spare.is_new` through attribute-only handles, so
    it counts by the same rule as the run.
  - Test: `preview_lists_what_would_go_and_removes_nothing` (`:2720`), with an entry without
    an ID in a changed known folder, checked in both preview and run.
- **S1 #4, npm `_logs`:**
  - `hold_before` (`clean.rs:507`) holds the reviewed cache, and a `_logs` in it, from
    before the probe until after the run. `_logs` is created when missing, then opened as
    itself (`hold_place`), and a junction there is refused.
  - Test: `a_package_tool_runs_only_on_the_reviewed_cache` (`:2866`).

## Section 3: round-4 findings, the rest

- **S2 #4, Codex** (`crates/sprawl-advisor/src/llm/codex.rs:359`): `turn.started` clears
  the saved answer. A turn that completes without its own message is the error "Codex's
  last turn ended without an answer". Test: `parses_exec_events` (`:647`).
- **S3 #2, restricted clients:**
  - `attach_usage` uses `find_path_unambiguous` (`crates/sprawl-advisor/src/lib.rs:109`).
    Where several names differ only in case, only the exact spelling matches.
  - Exact Recent matching upper-cases only the drive letter (`used.rs:50`).
  - Test: `a_case_only_sibling_is_never_taken_by_chance` (`used.rs:292`).
- **S4 #4, coverage wording** (`crates/sprawl/src/coverage.rs:118`): the mixed line gives
  the network, local and unknown counts once each. The test (`:249`) compares the whole
  sentence.
- **S5, OwnedDir final removal** (`crates/sprawl-core/src/local.rs:577`):
  - The folder's identity is now `full_identity`: the volume serial and the 128-bit ID
    (`:491`).
  - The final reopen shares no delete. The folder is disposed of only when the ID matches
    (`:590`), and otherwise left.
- **S6, teardown** (`local.rs:550`):
  - `empty_through` opens children without delete sharing.
  - `a_folder_put_in_place_of_an_owned_one_stays` (`:689`) now swaps the folder through a
    test hook at the real reopen step.
  - A second hook moves a child out right after teardown opens it, and the move must fail.

## Section 4: user-facing changes

- **Clean-up panel** (`cleanup.rs`):
  - `running_label` (`:214`) shows "Compressing... n%, X of Y". Y is the review's folder
    size, so n is capped at 99.
  - `compress_label` (`:225`) shows the finished row.
  - The header says "Compressing... X saved so far" when only compressions run.
- **The finding's button** (`crates/sprawl/src/advisor.rs`) says "Compress this...", and
  its line says nothing is deleted.
- **AI chat** (`crates/sprawl/src/ui.rs:30`): `esc_lines` keeps line breaks in a pasted
  question (`\r\n`, `\r`, `\n`) instead of showing U+FFFD. It is used for the user's
  message and the live answer.

## Points the author is least sure of

1. **WOF access.** The compressor's handle has read access only, and that worked in the
   temp fixtures and the tests (elevated). Does `WofSetFileDataLocation` need more on files
   owned by TrustedInstaller under Program Files? Those would be counted as `failed`, with
   a reason. That is safe, but would show as "some files couldn't be compressed".
2. **Between plan and compress.** A file's name could be replaced after the plan. Each
   file is checked when it is opened, and denied to writers while it is compressed. A hard
   link made after the link check would share compressed storage, which is transparent and
   loses no data. Is anything else reachable in that gap?
3. **S4 #5.** A program holding FILE_WRITE_ATTRIBUTES could clear read-only between the
   last check and the mark, then add a stream. Is that acceptable, or should the stream
   read after the mark (already there) decide alone?
4. **S1 #3.**
   - The volume is the scanned root's, read when the clean-up builds its `Spare`, not
     stored at scan time.
   - Trees without IDs (History, CSV) now hold items in any folder that changed since the
     scan, which is stricter than before. Is that the intended trade-off?
5. **The record that hides a compressed folder.** It is the only signal, since the scan
   can't see WOF. The 10% growth threshold is a guess.

## Verdicts wanted

| Section | Verdict |
|---|---|
| 1 · New compressor | |
| 2 · Clean-up safety (S4 #5, S1 #3, S1 #4) | |
| 3 · The rest (S2 #4, S3 #2, S4 #4, S5, S6) | |
| 4 · User-facing changes | |
