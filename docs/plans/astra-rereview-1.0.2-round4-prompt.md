# For GPT Astra: round 4, re-review of the round-3 fixes

This file is the whole request: what to do, the decisions, and one item per finding.

## What to do

You reviewed the round-2 fixes. Your report is `docs/plans/astra-rereview-1.0.2-round3.md`,
with changes requested in all six sections. Each finding you didn't mark fixed is now fixed
on main in 7 commits, 10a498b..8a6bc0d, titled "Astra 1.0.2 round 3, part 1..7". The notes
(CHANGELOG, this file) are in 2a4fed2. Nothing is released or pushed.

Please re-review **only these fixes**, not the rest of the codebase.

- **For each finding below:** say whether it is fixed. If it isn't, give the smallest
  acceptable remaining change. Items are numbered as in your round-3 report (S1 #3 =
  section 1, finding 3).
- **New problems:** report any problem a fix introduces, with its file and line.
- **Verdicts:** end with a verdict per section.

Read-only: please don't push, release, register tasks, or run clean-ups.

## Checks already run

- `cargo test --workspace --no-fail-fast` at 8a6bc0d: app 82 passed / 1 ignored; advisor
  222 / 6 ignored; core 68 / 2 ignored; install 16. No failures, no build warnings.
- `cargo test --release -p sprawl-core --test speed_guard -- --ignored`: inventory 13 ms
  against a plain listing's 30 ms (1093 folders, 2186 files). Recording file IDs didn't slow
  the walk.
- The test shell was **elevated**, so unelevated-only checks skipped themselves. Please run
  `cargo test --release --workspace --no-fail-fast` once from an unelevated shell.
- Not done: no real `codex exec`; no real npm, pip, NuGet or Go clean-up; no screenshots;
  no cross-account pipe test.

## Decisions (please judge them as stated)

1. **S1 #3: scan-time file IDs after all.** Holding every changed folder whole would keep
   Temp and active caches from ever being cleaned. So the round-2 decision is reversed: each
   scanned node now carries the file ID the scan saw, from the folder listing
   (FileIdExtdDirectoryInfo, else FileFullDirectoryInfo) or from the MFT. In a changed
   folder, an entry is gone through only when its ID is the one the scan saw there. A tree
   without IDs (CSV, History) holds a changed folder whole.
2. **S1 #4:** pnpm and Yarn are held like conda, as you suggested. Go is neutralized rather
   than held. npm is bound.
3. **S5 #1:** Codex stays as a risk the user accepted. The warning is your proposed text,
   plus "Choosing Codex accepts that."

## Points the author is least sure of

1. **S4 #5.** Before the original is removed, it is marked delete-on-close
   (FileDispositionInfo, not POSIX), and only then is its stream list read a last time. The
   test shows that, once marked, creating a new stream, opening one, and opening the file
   itself are all refused. Is that the guarantee you asked for, that stream membership stays
   stable through disposal, on NTFS and ReFS?
2. **S1 #3.** A 128-bit file ID whose high half isn't zero (ReFS) is stored as unknown. That
   entry then gets only the time checks, and a changed folder holding it is held whole.
   Should such an entry always be held?
3. **S1 #10.** Without level 2 (Sprawl not running as administrator), the folder of a share
   this PC serves is unknown. A personal folder redirected to such a share then makes the
   never-list incomplete, which holds every clean-up. Is that acceptable, or too strict?
4. **S4 #4.** A pinned root is classified through its own handle (FileFsDeviceInformation:
   FILE_REMOTE_DEVICE, or a DFS or network device type). Are there local volumes that report
   themselves remote, or remote ones that don't?

## The fixes

### Section 1

1. **S1 #3, part 1 (10a498b) + part 2 (ffa2776): changed folders.**
   - Core (`crates/sprawl-core/src/walk.rs`, `dir::list`): FileIdExtdDirectoryInfo (class
     19/20), falling back to FileFullDirectoryInfo (14/15) on errors 1/50/87/124. A 128-bit
     ID with nonzero high bits is stored as 0 (unknown).
   - `Node.id` (`crates/sprawl-core/src/lib.rs`). The MFT path uses `frn | seq << 48`, the
     same value GetFileInformationByHandle gives.
   - `crates/sprawl-advisor/src/clean.rs`, `Spare::is_new`: `SeenAt::Item(id)` with a
     different ID is new; `SeenAt::Nothing` (not in the scan) is new; `SeenAt::Unknown`
     falls back to times. A changed folder (newest > scan) is gone through only when it is
     `Item(id)` with the same ID. Its old entries are then checked one by one the same way.
   - `clean::seen_in(tree)` builds `Spare.seen` from the scan on screen, in
     `crates/sprawl/src/cleaners.rs` and `commands.rs`.
   - Test: `only_the_items_the_scan_saw_are_gone_through`, five scenarios, including a
     same-name, same-length file swapped in and a replacement folder.
2. **S1 #4, part 2 (ffa2776): package tools.**
   - `clean.rs`, `package_tool`: pnpm and Yarn return Err (held), with a sentence telling the
     user to run the tool themselves.
   - Go: `GOENV=off`, `GOFLAGS=` and `GOTOOLCHAIN=local` for both the probe and the run,
     through `PackageTool.fixed`.
   - npm: the probe and the run both end with `--logs-dir "$env:SPRAWL_TOOL_DIR\_logs"`.
   - Test: `a_package_tool_runs_only_on_the_reviewed_cache`.
3. **S1 #6, part 3 (cf3467d): Keep list.**
   - `crates/sprawl-advisor/src/memory.rs`, `load_checked_from(APPDATA, ["Sprawl","advisor"])`
     calls `local::read_checked` (part 1).
   - `read_checked` holds each existing plain folder down from the trusted one. The first
     missing name means there is no list. A link at any level is an error. An existing leaf
     is read through the held chain.
   - Test: a fresh profile, a read through the chain, a dangling link, and a junction above
     the folder and at it.
4. **S1 #9, part 5 (76b3bb0): unknown mappings.**
   - `crates/sprawl-advisor/src/exclusions.rs`, `Mapped.listed` and `maybe_network()`. When
     the share is unknown, every letter not known to be local is covered, present or not.
     With no drive list (a zero mask), a skip given as a share path also covers every
     letter.
   - The settings test uses `configured_with(path, fixed mapping)`, not this PC's Z:.
5. **S1 #10, part 5: share folder.** `crates/sprawl-advisor/src/safety.rs`, `share_folder`:
   `Exists(None)` (level 2 unavailable) is now `Unknown`. The registry fallback is removed,
   and the unused `reg::Key::strings` with it. Effect: a known folder redirected to a share
   of this PC, seen by an unelevated Sprawl, makes the never-list incomplete, which holds
   clean-ups.
6. **Other per-user writes (S1 #1 follow-up), part 3:** `crates/sprawl-advisor/src/store.rs`
   - `ensure_dir` holds through `Directory::child`.
   - `open_own` is `open_in(name, APPEND_DATA|DELETE, SHARE_READ, ..., Kind::File, single)`.
     There is no delete sharing, so the name can't change while it is open.
   - Rotation uses `rename_in` relative to the held folder (NtSetInformationFile class 10
     with RootDirectory).
   - Appends take a process-wide turn and retry error 32 for up to 2 s.
   - Test: 8 threads × 20 lines, all 160 logged; an open log can't be renamed or removed.
7. **`pin()` (the round-2 new finding), part 1:** `crates/sprawl-core/src/local.rs`, `pin()`
   opens the leaf with READ_DATA|READ_ATTRIBUTES and shares only read and write, so it takes
   part in sharing and can't be renamed or deleted. Ancestors stay attribute-only. Renames
   and deletes go through `Directory::open_in`/`rename_in` on a held parent. The compression
   tests (compact.exe given a pinned file's name) still pass. Test:
   `a_pinned_leaf_cant_be_renamed_away`.

### Section 2

1. **S2 #4, part 4 (7404e87): Codex output.** `crates/sprawl-advisor/src/llm/codex.rs`:
   - After exit: `run.end()`, then wait on the reader's channel for at most 10 s. A timeout
     is an error: "couldn't be read to its end".
   - `parse`: an unparseable line is an error; `turn.started` resets completion; the last
     event must be `turn.completed`, coming after the answer.

### Section 3

1. **S3 #2, part 6 (cfa476d): case.**
   - `crates/sprawl-advisor/src/used.rs`: `recent_items` keeps the shortcut's spelling.
     `opened_in(items, path, exact)` ignores case unless `exact`.
   - `crates/sprawl-advisor/src/facts.rs`: `Facts.exact` is set by `scoped_to`. `spelled`
     keeps each location as its source spelled it. `owner_of` matches exact names.
     `owner_map` uses `history::find_path` (exact) on each location.
   - `scoped_to` keeps Recent records only when their own spelling is a path in the tree.
   - `stale.rs` passes `facts.exact`.
   - Test: `a_restricted_view_matches_names_in_their_own_case` (two case-only siblings with
     separate owners; Recent kept only as spelled).
2. **S3 #4, part 6:** `crates/sprawl-advisor/src/pipe.rs` comment: the pipe authenticates
   the connection's opening context, not each write, hence one request per connection.

### Section 4

1. **S4 #3, part 7 (8a6bc0d):** `crates/sprawl/src/cleanup.rs`, `lead_line`: the named Bin
   sentence whenever privacy items or any outcome other than the Recycle Bin is present.
   Tested for Bin + Tool, Link, Compress and Shrink.
2. **S4 #4, part 1 + part 5: network label.**
   - `crates/sprawl-core/src/progress.rs`, `Progress::note_root(root, Option<bool>)`, filled
     by `walk_mode` and by the MFT paths in `scan_auto` and `scan_many_mode`.
   - The value comes from `ScanRoot::on_network()`: an unpinned root (a share) is yes; a
     pinned one is NtQueryVolumeInformationFile(FileFsDeviceInformation) on the held leaf,
     where FILE_REMOTE_DEVICE or a network device type is yes, and an error is None.
   - `crates/sprawl/src/coverage.rs`, `network_roots(roots, recorded)` gives (network,
     unknown). Roots never reached (a stopped scan) count as unknown.
   - `crates/sprawl/src/state.rs` reads `job.progress.roots()` at publish and asks no drive
     anything.
   - Details uses `win32::fixed_drives()` (GetDriveTypeW only, no volume information).
   - Test: `the_network_label_comes_from_the_scan_that_opened_each_root`, including a real
     inventory recording `Some(false)`.
3. **S4 #5, part 2: link streams.** `clean.rs`, `link_with`:
   - After the link is published: `win::delete_on_close(&cf, true)` (FileDispositionInfo,
     not POSIX, undoable while the handle is open).
   - Then the last `named_streams`. Anything found calls `delete_on_close(false)`, and the
     original stays at `.sprawl-original` with no space credited.
   - Then the handle closes.
   - Test: after that last list, a hook tries to create `:later` (create_new), open-or-create
     it, and open the original. All three are refused, and the original is gone.

### Section 5

1. **S5 #1 warning, part 4:** `RISK` is now your text plus "Choosing Codex accepts that."
   The `crates/sprawl-advisor/src/llm/agent.rs` comment no longer says the alternatives
   have no tools. `check_tools` requires an `additional_tools` record.
2. **Run folder, part 1 + part 4:** `codex.rs`, `base()` holds %APPDATA% and creates
   Sprawl\advisor\codex through `Directory::child`. `fresh()` is
   `OwnedDir::create_under(base)`. The OwnedDir is created exclusively through the parent's
   handle. On drop it empties itself through its own handle, reopens its name through the
   parent with DELETE, and removes the folder only if its ID matches; otherwise it leaves it.
3. **Stop, part 4:** checked in the probe's exit wait, before spawning (after
   `log_request`), and in the stdin writer before sending.
4. **S5 #3, part 7:** `crates/sprawl/src/ui.rs`, `path_span`: after a `"`, the path runs to
   the closing quote on the same line. The tests include `"C:\My Files\[draft](old).txt"`.

### Section 6

1. **Fixture teardown, part 1:** `OwnedDir` as above (`crates/sprawl-core/src/local.rs`).
   - `empty_through` opens each child relative to the held folder and removes it with
     `dispose`: FileDispositionInfoEx with POSIX semantics and IGNORE_READONLY_ATTRIBUTE. No
     attribute is ever changed, so a file's other links keep theirs.
   - Where that class isn't supported, the class-4 delete is tried. A read-only child then
     fails and is left behind, with the folder.
   - The hold is dropped only after the folder is empty. The name is reopened through the
     parent, and the folder is removed only if its file ID is the one created.
   - Tests: `removing_a_read_only_file_leaves_its_other_names_as_they_are`,
     `a_folder_put_in_place_of_an_owned_one_stays`.
