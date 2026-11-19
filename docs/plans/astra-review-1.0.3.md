# Astra's 1.0.3 review: how each finding was handled

Astra reviewed `61795e0..754062b -- crates` (the 1.0.3 work) and requested changes. All findings
below were fixed in one commit before 1.0.3 was released, except S4 #5. The user accepted S4 #5
as a known risk. Anything a later review finds goes into the next version.

Test run after the fixes: `cargo test --release --workspace --no-fail-fast` in a non-administrator
shell: 404 passed, 0 failed, 10 ignored.

## 1 · New compressor (`crates/sprawl-core/src/wof.rs`)

| Finding | Fix | Test |
|---|---|---|
| [P1] Wrong WOF structure | `one()` passes `WOF_FILE_COMPRESSION_INFO_V1` (algorithm, flags = 0; 8 bytes) to `WofSetFileDataLocation` with provider 2. A new `algorithm()` reads the stored algorithm back from `FSCTL_GET_EXTERNAL_BACKING`. | `each_algorithm_is_stored_as_asked`: all four algorithms, checked against the backing metadata. |
| Access denied counted as in use | Only errors 32 and 33 (sharing and lock violations) count as `in_use`. Error 5 and any other open error is `failed`, with its reason in `first_error`. | `a_file_access_is_denied_to_is_a_failure_not_in_use` (an icacls deny on the fixture; skips itself if backup intent lets the open through). |
| Plain replacement file | The plan keeps each file's listed id (`FILE_ID_EXTD_DIR_INFO`; 0 where the listing has none). The worker compares it with the open handle's index, and on that handle checks again that the file is plain, has one name, holds 4 KB or more, and isn't NTFS-compressed or WOF-backed. | `a_file_replaced_after_the_plan_is_left_alone` (one file replaced, one shrunk). |
| Kept junction | `plan()` checks the reparse tag before `check()`. Any non-WOF reparse point refuses the whole folder, even a kept one. | `a_kept_link_still_refuses_the_folder`. |
| [P2] Stream accounting | Before and after are both `on_disk()` (every stream's allocation) on the same handle. `on_disk()` grows its buffer on `ERROR_MORE_DATA`/`ERROR_INSUFFICIENT_BUFFER` and fails on a cut-short list. A file compressed without both measurements counts in the new `Done::unmeasured` and adds nothing to before/after. | `the_same_streams_are_measured_before_and_after` (a 2 MB alternate stream). |

## 2 · Clean-up safety (`crates/sprawl-advisor/src/clean.rs`)

- **[P1] Hard-link alias left read-only:** `link_with()` skips a copy whose original has `links != 1` before renaming it or changing its attributes ("left alone: it has another name"). Every linked copy is now credited in full. Test: `linking_preserves_a_concurrent_replacement_and_external_alias` now expects the copy to be left alone and checks that the external alias stays writable.
- **S1 #3 Volume serial read too late:** `sprawl_core::Tree` has a new `volumes: Vec<(PathBuf, u64)>`. It holds the FILE_ID_INFO volume serial of each real scan root, read at scan time: in `walk::inventory` from the pinned root handle, and in `mft::scan_multi` before the MFT is read. `combine()` (multi-root), `subtree()`, `filter` and snapshot compaction carry it over. Trees read from CSV files or saved snapshots have none, so their ids are unknown and the existing fallback (hold unknown identities under a changed parent) applies. `Tree::volume_of(p)` takes the deepest root holding `p`. `seen_in()` uses it in place of opening `tree.root_path` (which was the synthetic `Multiple` for multi-root scans). `Spare::is_new()` holds an item whose current volume is known and differs from the scanned one, whatever its times. Test: `each_scan_root_keeps_the_volume_it_was_scanned_on` (a two-root scan; a changed serial holds an unchanged file).
- **S4 #5 Original lost on exit: accepted risk (the user's decision).** The read-only-before-check design stays. The remaining gap: another program clears read-only and adds a stream between the last check and the delete mark, and Sprawl ends between the mark and the post-mark check. It is documented at the mark in `link_with()`.
- **S1 #4:** fixed already. No change.

## 3 · Remaining round-4 findings

- **S3 #2 Case-only scan roots** (`crates/sprawl-advisor/src/lib.rs`, `find_path_unambiguous`): where several scan roots match `path` ignoring case, only the root spelled exactly as in `path` is taken (a drive letter's case never counts). When no root is spelled that way, nothing matches, so the usage is left out. Test: `used::tests::a_case_only_scan_root_is_never_taken_by_chance`.
- **S5 OwnedDir final removal** (`crates/sprawl-core/src/local.rs`): `OwnedDir` keeps an `anchor`, an attribute-only handle on the folder, opened while the hold still denies delete sharing and checked to have the same full id. Attribute-only handles take no part in sharing checks, so it doesn't block the final DELETE open. While it is open, the folder's id can't be reused. Drop removes the folder only when the reopened handle and the anchor, both open, have the folder's full id. Without an anchor, the empty folder is left. The existing swap test passes with it.
- S2 #4, S4 #4 and S6: fixed already. No change.

## 4 · User-facing (`crates/sprawl/src/cleanup.rs`, `crates/sprawl-advisor/src/memory.rs`)

- **Ticks cleared by the refresh:** `Cleanup::bind(None)` (the Advisor restarting with no report yet) no longer drops the context or the ticks. It only resets the review and consent. The next report reconciles them as before. `cleanup_finish` unticks only items that started, not those still `Waiting` in a stopped batch. Test: `ticks_wait_through_a_refresh_with_no_report`.
- **Partial compression hides the retry:** `memory::Compressed` has a new `partial` field (serde default false, so older records stay complete). A run is complete only if it succeeded, wasn't stopped, and had no in-use, changed or failed files. `compressed_already()` hides only complete records. Partial runs are still recorded, so they can be undone. Compressing again keeps the earlier saving (earlier + now, capped at the folder's size), so Undo's estimate isn't zero. Compressions no longer go through `memory.decide(Done)`, so a partial run can't be hidden that way either. Test: `memory` test extended with a partial record.
- **Header and log:** when only compressions run, the header shows "X of Y gone through" (plus the saving once an item has finished). The log writes "compressed in place (alg, in Sprawl): N compressed, N compressed already, N in use, N changed since the plan, N failed" and the first error, instead of `ran: compact.exe ...`.

## Not verified

The live UI, a large application folder, TrustedInstaller-owned files, ReFS, and live npm and Codex runs (the same gaps as in Astra's review).
