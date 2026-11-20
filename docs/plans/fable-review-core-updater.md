# Fable review: scan core, updater and installer

Reviewed 2026-11-21 at HEAD `fc2d97c`. The uncommitted working tree does not touch `sprawl-core`, `sprawl-install`, `sprawl-release` or `crates/sprawl/src/update.rs` (`git diff HEAD --stat` on those paths is empty). Read only: nothing was built, run or edited.

## Verdict

**Changes requested, none of them a release blocker for memory safety.**

- The scan core has no memory-safety hole that I could find. Every read of on-disk MFT data is bounds-checked, the few `unsafe` blocks are sound, and reparse points are never entered, so there are no cycles.
- The update chain is sound: Ed25519 `verify_strict` over the exact manifest bytes, version bound to the tag and strictly newer than the compiled version, size and SHA-256 per file, re-hash on a write-denied handle, rename by that handle, and no elevated writes.
- What needs work:
  - one elevation-boundary design risk (P1) that undercuts the rest of the elevated hardening;
  - an elevated write by path behind an environment switch (P2);
  - an updater state that blocks all later updates (P2);
  - two MFT accuracy problems (P2).

| Severity | Count |
|---|---|
| P1 (data loss / security) | 1 |
| P2 (wrong behaviour) | 5 |
| P3 (minor) | 12 |

---

## P1

### P1-1. Elevated Sprawl runs from a folder that non-admin code can write

`installer/sprawl.iss:23` (`DefaultDirName={localappdata}\Programs\Sprawl`), `crates/sprawl/src/dialogs.rs:77-82` (`restart_elevated`: `ShellExecuteW("runas", current_exe)`), `crates/sprawl/src/main.rs:130`.

Sprawl installs per user and then routinely elevates that same exe ("Always Run as Administrator", the Advisor's restart as administrator, MFT scans). Any process running as the user at medium integrity can replace `sprawl.exe`, or drop a DLL beside it. The exe loads `Microsoft.WindowsAppRuntime.Bootstrap.dll` from its folder, and non-KnownDLL imports such as `winhttp.dll` and `wofutil.dll` are searched there first.

**Scenario.** Malware running as the user writes `%LOCALAPPDATA%\Programs\Sprawl\wofutil.dll`. The user starts Sprawl with "Always Run as Administrator" on and approves the UAC prompt they see every day. The planted DLL now runs as administrator.

This makes the careful pinning of elevated settings writes and the unelevated update hand-off moot: the attacker does not need those paths. It needs local same-user code and a UAC approval, so it is a UAC-consent hijack, not a remote hole. I found no record that it was raised or accepted in the Astra reports or the queue.

**Fix, in order of strength:**
1. When elevation is wanted, run a copy from a location only administrators can write (a per-machine install in Program Files, updated by an elevated step that verifies the same signed manifest).
2. At minimum, before `runas`, call `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)` early in `main` and load the bootstrap DLL by full path after checking its hash against the signed manifest. This closes DLL planting but not exe replacement.
3. Otherwise record it as an accepted risk beside S4 #5, so later reviews stop treating elevated per-user writes as the boundary.

---

## P2

### P2-1. `timing()` appends to a file by path in `%TEMP%` while elevated

`crates/sprawl-core/src/lib.rs:184-194`. It is compiled into release builds and switched on by the `SPRAWL_TIMING` environment variable. Called from `scan_auto`, `mft::read_mft_device`, `mft::scan_multi` and the app.

The file is `std::env::temp_dir().join("sprawl-timing.log")`, opened with `create(true).append(true)`: no pin, no reparse check, no elevation check. Both `SPRAWL_TIMING` and `TEMP`/`TMP` are user-controlled (HKCU\Environment, or the launching process) and are inherited by the elevated copy.

**Scenario.** A medium-integrity process sets `SPRAWL_TIMING=1` and points `TEMP` at a folder that is a junction into `\RPC Control`, with an object symlink `sprawl-timing.log` to a file of its choice. The next elevated scan creates that file, or appends lines to it, as administrator. Some of the text is attacker-influenced (`scan: MFT scan FAILED ({e})`, paths in the app's `Check again reads ...` lines).

This is the same class as Astra 1.0.2 S1 #1 (elevated settings write). It was listed in the 1.0.4 prompt for Astra, but there is no 1.0.4 Astra report in `docs/plans`.

**Fix:** compile it out of release builds (`#[cfg(debug_assertions)]`), or refuse when `elevation() != Ok(false)`, or write through `local::directory` + `open_in` like other writes. The app has about 15 other `SPRAWL_*` switches I did not read; they deserve the same check.

### P2-2. A recovery folder that can't be retired blocks every later update, after the other windows are closed

`crates/sprawl-install/src/recovery.rs:45-70` (`prepare`), `:121-140` (`cleanup_for`); `crates/sprawl/src/update.rs:409-411`; `installer/sprawl.iss:59-64`.

`prepare` calls `cleanup(dir)` and then `create_dir(.sprawl-recovery)`, which fails if the folder is still there. `cleanup_for` only removes it when `recovery.json` parses and is valid, and the running version equals `old_version` or `new_version` with a hash-identical bundle in the main folder. Otherwise the folder stays for good, and every update ends with "The recovery folder could not be prepared (... already exists)". The advice in that message, "reinstall with Sprawl Setup", does not help: `[InstallDelete]` does not touch `.sprawl-recovery`.

**Reachable states:**
- **Reinstall with a newer Setup.** An update N to N+1 is interrupted between the renames (power loss, kill). The user reinstalls with the current Setup, as the README in the folder says, and that Setup is N+2 or later. The record says old=N, new=N+1; the running copy is N+2. Never retired.
- **Re-released version.** As above, but the user reinstalls the "same" version from a re-released build (1.0.4 was re-released on 2026-11-20). The version matches but the hashes don't. Never retired.
- **Partial record.** The process dies while `recovery.json` is being written (`create_new`, write, `sync_all`). The record doesn't parse. Never retired.

There is also an ordering problem: `update_swap` calls `stop_other_instances()` at `update.rs:409` before `swap_in` at `:411`. Each failed attempt therefore first terminates every other Sprawl window and `--mcp` server, then fails.

**Fix:**
- Retire the folder when the main folder holds a complete bundle and the running exe is in it, whatever versions the record names. The running copy is the proof of a healthy install. Treat an unparseable record the same way when the folder holds only names Sprawl writes.
- Or, when it can't be retired, rename it aside (`.sprawl-recovery.<n>`) and carry on.
- Add the five known names to `[InstallDelete]` and `[UninstallDelete]`.
- Do the preflight (stage hash check, recovery prepare) before closing other instances.

### P2-3. MFT scan: WOF-compressed files get `allocated = 0`

`crates/sprawl-core/src/mft.rs:325-340`. Only the unnamed `$DATA` is read. For a WOF file (`compact /exe`, CompactOS, files compressed by Sprawl's own compressor) the unnamed stream is sparse with nothing allocated, and the compressed bytes live in the named stream `WofCompressedData`. `aflags & 0x8001` is set, so `alloc` becomes the "compressed size" at 0x40, which is 0. The record is tagged `attr::WOF` at `:341-343`, but no size is taken.

**Scenario.** A CompactOS system drive scanned as administrator: several GB under `C:\Windows` show 0 bytes on disk, the "allocated" treemap shrinks, and the total sits well under the drive's used space. After Sprawl compresses a folder, an MFT rescan shows it as taking nothing. A folder walk shows the same files at full rounded size, so the two methods disagree in opposite directions.

`Node::allocated`'s doc comment (`lib.rs:66-68`) promises "the compressed size for compressed/sparse files".

I am confident of the stream layout (`wof::on_disk` in this crate sums streams for exactly this reason) but did not run a scan. `examples/mft_probe.rs` on one `compact /exe` file would confirm it.

**Fix:** in `parse_record`, for a non-resident `AT_DATA` whose name is `WofCompressedData` and whose start VCN is 0, carry its allocated size in `Parsed`. In `Index::merge`, add it to `alloc` when the record has `attr::WOF`. The stream may sit in an extension record, so merge it by base like the name.

### P2-4. MFT scan drops damaged records silently, with the whole subtree, and always reports 0 errors

`crates/sprawl-core/src/mft.rs:291` (`!apply_fixups(rec)` returns None), `:443-455` (`listed` needs an in-use folder as parent), `:556` (`errors: 0`).

A record that starts with `FILE` and is in use but fails its update-sequence check (a torn read while NTFS writes it, a `BAAD` record, real corruption) is treated like a free record. If it is a folder, every item whose parent chain passes through it is unlisted. Nothing is counted: an MFT tree's `errors` is the constant 0.

**Scenario.** One bad record for `C:\Users\<name>` on a volume that needs chkdsk: the whole profile vanishes from the tree, and the scan says nothing was unreadable. Volumes in that state are exactly where people reach for a disk tool.

The coverage text was reworded after Astra S4 #4, but the count is still not produced.

**Fix:** count (a) in-use `FILE` records that fail fixups and `BAAD` records, and (b) after linking, in-use named records that are not listed because their parent is missing or its sequence differs. Put the sum in `Tree::errors`. For (a) on a base folder record, consider failing the MFT scan so `scan_auto` falls back to the walk.

### P2-5. Updating terminates other Sprawl processes without knowing what they are doing

`crates/sprawl-install/src/processes.rs:37-65`, called from `crates/sprawl/src/update.rs:409`.

The updating window refuses while its own scan, clean-up or file operation runs (`update.rs:403`). It then calls `TerminateProcess` on every other process of the same image and account.

**Scenario.** A second window is in the middle of a clean-up or an in-place compression, or `sprawl.exe --mcp` is serving an AI client. The first window's daily update card is clicked. The other process is killed mid-operation with no chance to finish a step, write its log, or release a held `OwnedDir` (whose Drop does the removal).

Identity checking (same handle, exact image, SID) is good and was accepted by Astra. The gap is consent and state, not targeting.

**Fix:** refuse with "close the other Sprawl windows first" as is already done for elevated peers (`:44-47`), or ask them to close (a named event or `WM_CLOSE`) and only wait. Exempt nothing silently. `--check` and `--mcp` can be told to exit the same way.

---

## P3

1. **A crafted NTFS volume can abort or panic the elevated app (no memory corruption).** `mft.rs:93` accepts a non-power-of-two sectors-per-cluster, so `read_len` at `:992` can exceed the 8 MB buffer and the slice at `:1001` panics inside the scoped reader thread. `:961-966` sizes the index from the on-disk `$MFT` size with no cap against the volume size (a huge value aborts on allocation). `:404-411` lets each record with a far-off base reference double the index, up to about 4 billion entries. `lcn * cluster` and `r * rs` (`:829`, `:874`, `:1005`) wrap in release builds (wrong offset, then a read or parse error). Windows' own driver rejects most of these at mount; a removable disk altered after mount, or an image, does not. Fix: require power-of-two cluster size; cap `mft_bytes` at `total_sectors * bytes_per_sector`; never grow the index past the record count; use `checked_mul`.
2. **The MFT is read up to its data size, not its initialized size.** `mft.rs:841`, `:890` take 0x30. If the valid data length (0x38) is ever lower, the tail is stale disk content that may hold old in-use `FILE` records. The parent sequence check limits the damage. Low confidence that this occurs in practice; taking `min(0x30, 0x38)` is free.
3. **Extension records are trusted by record number alone.** `mft.rs:298`, `:399` drop the base reference's sequence number, so an orphaned extension record can add a name or size to an unrelated reused record. Also `$MFT`'s own extension records (base = record 0) read as base records; harmless today (no name), but they bump the live file count.
4. **Folder walk sizes: hard links are counted once per name, and "allocated" is the length rounded up** (`walk.rs:199`), although the handle listing already returns `AllocationSize` at offset 48 (`walk.rs:528` reads its neighbours). Sparse and NTFS-compressed files are overstated in walk mode, and WinSxS-style hard links are counted several times, where the MFT scan counts them once. Worth using offset 48 when listing by handle, and saying which method was used wherever the two can disagree.
5. **A panicking walk worker loses its lists and can hang the walk.** `walk.rs:168` turns a panic into an empty result; that worker's `pending` count is never released (`:255-260`), so the others wait until cancelled. No panic source found in the worker; a guard that decrements on unwind and adds an error would make it safe.
6. **CSV import has no size limits.** `csv.rs:388` reads a line of any length into memory, and the node count is unbounded. A planted multi-GB file with no newline exhausts memory. Names with control characters (possible in MFT names) are written raw by `export_csv` and skipped on re-import. Cap the line at about 70 KB and stop with an error.
7. **Snapshot decode and encode disagree at the edges.** `snapshot.rs:298` rejects separators and `.`/`..` but accepts names ending in a dot or space, wildcards and control characters, which `csv::plausible_path` refuses. Harmless while History trees are only projected onto a live tree; apply the same name rule so that stays true. `encode` writes any `u64` size while `decode` (`:303`) rejects sizes over `i64::MAX`, so a scan holding one absurd size saves a snapshot that never loads.
8. **`replace_branch` keeps the old `id` on the rescanned node.** `crates/sprawl/src/commands.rs:1070-1072` copies `is_dir`, `modified` and `attributes` from the fresh root, but not `id`, while `replace_volumes` switches the branch to the fresh volume. The stale id fails safe (a clean-up refuses), but a rescanned folder that was recreated can't be acted on until a full rescan.
9. **The installer offers an all-users mode the design doesn't support, and uninstall leaves the recovery copy.** `installer/sprawl.iss:27` (`PrivilegesRequiredOverridesAllowed=dialog`). In that mode I expect Inno to write the Apps entry under HKLM while files still go to `{localappdata}`: `setup::inno_dir()` reads HKCU only, so `register` silently does nothing and the Apps version goes stale; with over-the-shoulder elevation the files land in the administrator's profile (not verified by running the installer). Remove the override. `[UninstallDelete]` (`:66`) does not list `.sprawl-recovery`, so a runnable old `sprawl.exe` can remain after uninstall.
10. **Release tool.**
    - `main.rs:95`: the signing key is plain hex under `%USERPROFILE%` with inherited ACLs; any process running as the developer can read it. Use DPAPI or a passphrase, or sign on a separate machine.
    - `Sprawl-Setup-<v>.exe` is not in the signed manifest and (as far as the tool shows) not Authenticode-signed, so a first install trusts GitHub alone.
    - `:134` hashes files from `target/dist/release`, while `:148` lets ISCC read the same folder again later. Build the installer from the hashed copies in `dist`.
    - `build` does not require a clean tree; `publish` (`:183`) tags remote `main`, which need not be the commit that was built. Record the commit at build and check it at publish.
    - A re-release under the same version (as with 1.0.4) never reaches copies that already have that version, because `newer` is strict. Bump the patch number instead.
11. **Small hardening in `sprawl-install`.** `manifest.rs:47` (`valid_name`) accepts device names (`con.exe`, `nul.dll`, `com1.exe`); only reachable with the signing key, but cheap to refuse. `setup.rs:366` builds the legacy uninstall's `cmd` line from `%SystemRoot%`, although `system_dir()` exists (`:36`), and embeds the install path where a `%` in the profile name would be expanded.
12. **No freeze or rollback protection beyond "newer than this copy".** Someone who controls the GitHub repository (not the key) can keep serving an older signed release that is still newer than the victim's. Accepted by most updaters of this size; noted for completeness.

---

## Checked and found fine

**MFT reader (`mft.rs`)**
- All little-endian reads go through `get(..)`.
- `apply_fixups` bounds (`off + cnt*2`, `(cnt-1)*512`).
- The attribute iterator (16-byte minimum, `off + len <= end`, `end` clamped to the record).
- `resident_value`; the `$FILE_NAME` length check before the UTF-16 slice at `:348`.
- `decode_runlist` (length 1-8, offset up to 8, sign extension, `checked_add`, negative LCN refused).
- `map_vcn`.
- `AlignedBuf` (layout, zeroed, same layout on free).
- `Handle` Send/Sync (one reader).

**Fragmented `$MFT` (c42c897), `mft_layout` and `read_nonresident`**
- Attribute-list walk is bounded (`o + 0x1A <= len`, zero length stops).
- 16 MB cap, VCN order enforced, short runlists refused.
- Extension records are located only through runs already known, read sector-aligned, and fixup-checked.
- Errors fall back to the folder walk.

**Threads**
- Reader, parsers and merge cannot deadlock on error or cancel (buffers return through `free_tx`; `stop` is checked each chunk).
- A read error fails the MFT scan as a whole and `scan_auto` walks instead.

**Links and cycles**
- MFT: a tree is built only from records reachable from the root record; a parent cycle is unreachable by construction; `TopTracker` is bounded at 512 steps; `path_hint` at 64.
- Walk: reparse points are listed and never entered (`walk.rs:315`). The root and its ancestors are refused if they are reparse points. Each queued folder is opened as itself and its real path compared in UTF-16 before listing (Astra's earlier case; unchanged).
- Hard links in the MFT are counted once, under the best-ranked name.

**Directory listing parser (`walk::dir::list`)**
- Offsets for `FILE_ID_EXTD_DIR_INFO` and `FILE_FULL_DIR_INFO` are right.
- Every read is checked against the buffer.
- Odd name lengths are refused.
- A 128-bit id is dropped rather than truncated.

**`local.rs`, `identity.rs`**
- Covered across four Astra rounds and unchanged in intent. I re-read both fully and agree with them. `launch_unelevated` holds the shell process while checking SID and elevation, passes no inherited handles, quotes the exe, and restricts arguments to `[A-Za-z0-9.-]`.
- `token_account_sid` and `token_elevated` check lengths.
- `FILE_RENAME_INFO` layout in `rename_in` and `setup::rename_open` is right for x64.

**CSV (`csv.rs`)**
- The quote parser, `plausible_path` (no `..`, streams, wildcards, trailing dot/space, depth 1000), and saturating number parsing.
- The root-collapsing `drain(..cur)` relies on the chain being nodes `0..=cur`; I checked that the builder guarantees it.
- The app refuses deletion from a CSV tree (`commands.rs:670`).

**Snapshots (`sprawl-advisor/src/snapshot.rs`, `history.rs` to line 420)**
- zstd window capped; body, heap and name budgets enforced before allocation.
- Parent distance `1..=i`, so there are no cycles and children follow parents.
- A file with children is refused; trailing bytes are refused.
- Buckets and entries are re-pinned and identity-checked before load or delete.

**Updater**
- `verify` checks the signature before parsing; strict verification; the zero key never verifies.
- Names are restricted to `[A-Za-z0-9._-]` ending in `.exe`/`.dll`, unique ignoring case; `sprawl.exe` is required.
- `check` binds the manifest version to the tag and requires it newer than the compiled version (the `SPRAWL_PRETEND_VERSION` override is debug-only).
- `download` allows HTTPS only, checks the host allowlist on every hop, turns off WinHTTP's own redirects, and enforces a size cap.
- `stage` refuses when elevated or unknown, pins the folder and the stage folder, uses `create_new`, and checks size and hash.
- `swap_in` re-hashes through a write-denied handle and renames that handle (no TOCTOU between verify and publish), with no replace-if-exists; rollback is by handle.
- `verified_launch` holds the new files, hashed, while the new process is spawned.
- No downgrade path.
- `--resume-update` is accepted only when Windows confirms the copy is not elevated and the version parses and is newer; it never re-elevates (`main.rs:130`), and the release still has to be the signed latest.
- Nothing is written to the install folder while elevated (`per_user_only` on every entry, unknown elevation refused).
- `processes.rs`: the `PROCESSENTRY32W` layout is right; verification and termination use one handle; an elevated peer makes the update refuse.
- Uninstall (legacy path) removes only named files and an empty folder, never recursively. The Inno script deletes the scheduled task by SID-qualified name and the old shared task only when its single principal is this account.

---

## What I read

**Fully**
- `sprawl-core`: `mft.rs` lines 1-1397 (all code and the first tests), `walk.rs`, `local.rs`, `identity.rs`, `lib.rs`, `csv.rs`.
- `sprawl-install`: `lib.rs`, `manifest.rs`, `processes.rs`, `recovery.rs`, `setup.rs`.
- `sprawl-release/src/main.rs`.
- `crates/sprawl/src/update.rs`.
- `installer/sprawl.iss`.
- `sprawl-advisor/src/llm/http.rs`: `split_url`, `host_allowed`, `download`, `request_raw`.
- `sprawl-advisor/src/snapshot.rs`: `encode`, `decode`, `folders_only`.
- `sprawl-advisor/src/history.rs` lines 1-420.
- Argument handling and the elevation restart in `main.rs` (`:71-135`, `:545-562`) and `dialogs.rs:75-90`.
- `commands.rs:1044-1089` (`replace_branch`).

**Skimmed**
- `wof.rs` lines 1-330: the compressor, which Astra reviewed for 1.0.3; I looked for parsing bounds only. Its tests were not read.
- `compact.rs` lines 1-140 (search-only index).
- `progress.rs`: exclusion matching only.
- `query.rs`, `search.rs`, `filter.rs`, `kinds.rs`: grep for `unsafe`, regex limits and unwraps only. The regex has size limits.
- `mft.rs` tests from 1397.
- `speed_guard.rs` (first 60 lines).
- The Astra reports and the queue: only to see what was already covered.

**Not read**
- `sprawl-core/examples`.
- `crates/sprawl/src/history.rs` (the app's History view).
- `sprawl-advisor/src/store.rs`.
- `alert.rs` (the scheduled task).
- The other `SPRAWL_*` environment switches in the app.
- Nothing was compiled or run. P2-3 and P3 item 9 in particular should be confirmed on a real machine.
