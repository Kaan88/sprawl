# For GPT Astra: review of the fixes for your 1.0.3 review

This file is the whole request: what to do, what changed, the decisions, and one item per
change. Paths are relative to the repository root.

## What to do

You reviewed `61795e0..754062b -- crates` and requested changes. Everything you raised was then
fixed in one commit, `cab2211`, except S4 #5, which the user accepted as a known risk (below).
**1.0.3 was released from `cab2211`** on the user's word, without waiting for this review.
Anything you find now goes into 1.0.4.

`docs/plans/astra-review-1.0.3.md` records each fix, finding by finding, with its test.

Please review **only this commit** (`git diff 754062b..cab2211 -- crates`):

- **Your findings:** for each one (the headings and IDs of your 1.0.3 review), say whether it
  is fixed. If it isn't, give the smallest acceptable remaining change, as a 1.0.4 item.
- **New problems:** report any problem this commit introduces, with its file and line. The
  `Tree::volumes` field touches every place a scan tree is built; please check that none
  loses it by mistake.
- **Verdicts:** end with one verdict per section below.

Read-only: please don't push, release, register tasks, or run clean-ups or compression
outside temporary fixtures.

## Checks already run

- `cargo test --release --workspace --no-fail-fast` at `cab2211`, in a **non-administrator**
  shell (`net session` refused): 404 passed, 0 failed, 10 ignored.
  - app: 86 passed, 1 ignored;
  - advisor: 227 passed, 6 ignored;
  - core: 75 passed, 2 ignored;
  - install: 16 passed;
  - speed guard: 1 ignored (it runs in the release build, and passed there).
- The access-denied test ran its real branch, not its elevated skip.
- Not done:
  - the clean-up panel hasn't been seen in the running app;
  - no real compression of a large application folder;
  - no TrustedInstaller-owned files;
  - no ReFS volume;
  - no real npm or Codex run;
  - no mutation checks this round.

## The user's decision on S4 #5

Asked whether to keep every original (your smallest change), turn linking off, or accept the
risk, the user chose: **accept the risk and note it.**

The read-only-before-check design stays. Loss needs both of these, in the same instant:

- another program clears read-only through attribute-only access, then adds a stream, between
  the last check and the delete mark;
- Sprawl ends between the mark and the post-mark check.

The risk is documented at the mark, in `crates/sprawl-advisor/src/clean.rs:1318`. Please don't
re-raise it as a blocker. If you see a way to close the gap that keeps linking useful, say so
as a 1.0.4 suggestion.

## Section 1: the compressor (`crates/sprawl-core/src/wof.rs`)

- **The WOF structure.** `one()` (`:241`) passes `WOF_FILE_COMPRESSION_INFO_V1` (algorithm,
  flags = 0; 8 bytes, `:263`) to `WofSetFileDataLocation` with provider 2. `algorithm()`
  (`:327`) reads the stored algorithm back through `FSCTL_GET_EXTERNAL_BACKING`
  (`WOF_EXTERNAL_INFO`, then `FILE_PROVIDER_EXTERNAL_INFO_V1`).
  Test: `each_algorithm_is_stored_as_asked` (`:412`) checks all four algorithms against the
  backing metadata and reads each file back.
- **Open errors.** Only errors 32 and 33 count as `in_use` (`:245`). Error 5, and any other
  open error, counts as `failed`, with its reason in `first_error`.
  Test: `a_file_access_is_denied_to_is_a_failure_not_in_use` (`:491`). An `icacls /deny`
  read-data rule is put on the fixture after the plan, and `/reset` afterwards.
- **A replacement between plan and compress.**
  - The plan keeps each file's listed id (`FILE_ID_EXTD_DIR_INFO`, 0 where the listing has
    none, such as FAT or a 128-bit ReFS id).
  - The worker compares that id with the handle's index (`:251`).
  - On the open handle, it then checks again: a plain file, one name, at least 4 KB, not
    NTFS-compressed, not WOF-backed.
  - Test: `a_file_replaced_after_the_plan_is_left_alone` (`:454`): one file is replaced by
    another plain file, and one shrinks.
- **A kept junction.** `plan()` checks the reparse tag before the caller's `check` (`:136`).
  Any reparse point other than a WOF file refuses the whole folder, even a kept one.
  Test: `a_kept_link_still_refuses_the_folder` (`:477`).
- **Streams.** Before and after are both `on_disk()` (`:292`), the sum of every stream's
  allocation, on the same handle.
  - The buffer grows on `ERROR_MORE_DATA` or `ERROR_INSUFFICIENT_BUFFER`, up to 8 MB.
  - A stream list that is cut short is an error.
  - A file compressed without both measurements counts in the new `Done::unmeasured` and adds
    nothing to `before` or `after`.
  - Test: `the_same_streams_are_measured_before_and_after` (`:432`), with a 2 MB alternate
    stream.

## Section 2: clean-up safety

- **[P1] Surviving hard links** (`clean.rs` `link_with`, `:1248`):
  - A copy with `links != 1` is left alone (`:1283`), before it is renamed or its attributes
    change.
  - Every copy that is linked is credited in full.
  - Test: `linking_preserves_a_concurrent_replacement_and_external_alias` (`:2215`). It now
    expects the copy to be left alone and the external alias to stay writable; it writes to
    the alias.
- **S1 #3, volume identity:**
  - **The field.** `sprawl_core::Tree` has `volumes: Vec<(PathBuf, u64)>`
    (`crates/sprawl-core/src/lib.rs:127`). It holds the FILE_ID_INFO volume serial of each
    real scan root, which is the same 64-bit source `win::Info::volume` uses.
  - **Where it is read:**
    - `walk::inventory` (`walk.rs:130`), from the pinned root handle, before the walk;
    - `mft::scan_multi` (`mft.rs:1095`), before the MFT is read;
    - `local::volume_id` (`local.rs:251`) and `volume_serial` (`lib.rs:146`) are the helpers.
  - **Carried by:**
    - `combine()` (`lib.rs:341`, multi-root scans);
    - `subtree()` (`:567`);
    - `filter::filter_tree`;
    - `snapshot::folders_only`.
  - **Left empty on purpose** (ids unknown, so the existing hold applies): trees read from CSV
    files or saved snapshots, and test trees.
  - **The lookup.** `Tree::volume_of(p)` (`lib.rs:133`) takes the deepest root holding `p`,
    with names compared whole and case ignored. `seen_in()` (`clean.rs:198`) uses it in place
    of opening `tree.root_path`, which was the synthetic `Multiple` for a multi-root scan.
  - **The decision.** `Spare::is_new` holds an item whose current volume is known and differs
    from the scanned one, whatever its times (`clean.rs:259`).
  - Test: `each_scan_root_keeps_the_volume_it_was_scanned_on` (`clean.rs:2795`). It scans two
    roots and checks both serials and `SeenAt`. With the serials altered, an unchanged file is
    held in the preview.
- **S4 #5:** an accepted risk (above).

## Section 3: the rest

- **S3 #2, case-only scan roots** (`crates/sprawl-advisor/src/lib.rs:109`,
  `find_path_unambiguous`):
  - Where several scan roots match `path` with case ignored, only the root spelled exactly as
    in `path` is taken. A drive letter's case never counts.
  - When none is spelled that way, nothing matches, so the usage is left out.
  - Test: `a_case_only_scan_root_is_never_taken_by_chance` (`used.rs:314`).
- **S5, OwnedDir's final removal** (`crates/sprawl-core/src/local.rs`):
  - `OwnedDir` has an `anchor` (`:488`): an attribute-only handle that shares everything.
  - It is opened by name while the hold still denies delete sharing, so the name is still
    this folder. It is checked for the same full id (`:531`).
  - Attribute-only handles take no part in sharing checks, so it doesn't block the final
    DELETE open. While it is open, the folder's id can't be given to another folder.
  - Drop removes the folder only if the reopened handle and the anchor, both open, carry the
    folder's full id (`:610`). Without an anchor, the empty folder is left.
  - The existing swap test, `a_folder_put_in_place_of_an_owned_one_stays`, passes.
- **S2 #4, S4 #4, S6:** you found these fixed. They are unchanged.

## Section 4: user-facing changes (`crates/sprawl/src/cleanup.rs`, `crates/sprawl-advisor/src/memory.rs`)

- **Ticks:**
  - `Cleanup::bind(None)` (`cleanup.rs:90`) no longer drops the context or the ticks. This
    happens when the Advisor restarts with no report yet. Only the review, `single`, `armed`
    and the consents are reset. The next report reconciles the ticks as before.
  - `cleanup_finish` unticks only items that started (`:1083`), so the `Waiting` items of a
    stopped batch stay ticked.
  - Test: `ticks_wait_through_a_refresh_with_no_report` (`:2077`).
- **Partial compression:**
  - `memory::Compressed` has `partial` (`memory.rs:160`). It defaults to false through serde,
    so older records count as complete.
  - A run is complete when it succeeded, wasn't stopped, and had no file in use, changed or
    failed (`cleanup.rs:1069`). `compressed_already` hides only complete records (`:542`).
  - A partial run is still recorded, so it can be undone.
  - A rerun keeps the earlier saving: earlier plus this run, capped at the folder's size
    (`cleanup.rs:1073`).
  - Compressions no longer go through `memory.decide(Done)` (`:1040`).
- **Header** (`:1111`): when only compressions run, it says "X of Y gone through", plus the
  saving once one has finished.
- **Log** (`:1765`): "compressed in place (alg, in Sprawl): N compressed, N compressed
  already, N in use, N changed since the plan, N failed", the unmeasured count, and the first
  error. It no longer writes `ran: compact.exe ...`.

## Points the author is least sure of

1. **`volume_of` with overlapping roots.** It takes the deepest root holding the path. Is
   that right when a scan holds both `C:\A` and `C:\A\B`?
2. **Trees without volumes.** A snapshot reloaded at start has no `volumes`, so every id is
   unknown, and a changed folder is held whole until it is scanned again. Is that strict
   enough, or too strict?
3. **No listed id.** On FAT, or with 128-bit ReFS ids, the listing gives 0 and the worker
   can't compare ids. It still checks plain, one name, size and backing on the open handle.
   Is that enough?
4. **`bind(None)` keeping ticks.** Can a new scan source reach `bind(None)` and later carry a
   tick onto a different tree's identical finding? Consent is reset either way, and that
   carry-over was already possible from one report to the next.
5. **The anchor's open failing** (access, for one) leaves Sprawl's empty folder behind. Is
   that acceptable?
6. **Summed savings** after a big update can count space the update already replaced. It is
   capped at the folder's size, and it only feeds Undo's estimate.

## Verdicts wanted

| Section | Verdict |
|---|---|
| 1 · Compressor | |
| 2 · Clean-up safety (hard links, S1 #3) | |
| 3 · The rest (S3 #2, S5) | |
| 4 · User-facing changes | |
