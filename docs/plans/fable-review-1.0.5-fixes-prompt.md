# Fable re-review: the 1.0.5 fixes for the four Fable reports

Written by Claude (Opus 5.5), 2026-11-21, for the next round of outside review.

## Scope

- **What to read:** commit `c554db0` ("Sprawl 1.0.5: Program Files install, Fable's P1/P2 fixes,
  light mode, tabs, custom AIs") against `fc2d97c`. Light mode, the tabs and the custom AIs are in
  the same commit; they were in scope of the first reports and only need a look where they touch
  keys (item 7 below).
- **Released:** v1.0.5, signed, Latest: https://github.com/Kaan88/sprawl/releases/tag/v1.0.5
  (assets: `sprawl.exe`, `Microsoft.WindowsAppRuntime.Bootstrap.dll`, `Sprawl-Setup-1.0.5.exe`,
  `sprawl-update.json` with the new `setup` entry, `sprawl-update.json.sig`).
- **The reports being answered** (all in `docs/plans/`):
  - `fable-review-1.0.4-1.0.5.md` (3 P1, 6 P2, 9 P3), below "1.0.4-1.0.5";
  - `fable-rereview-backlog.md` (1 P1, 3 P2, 5 P3), below "backlog";
  - `fable-review-ai-privacy-mcp.md` (1 P1, 6 P2, 10 P3), below "AI";
  - `fable-review-core-updater.md` (1 P1, 5 P2, 12 P3), below "core".
- **Tests:** `cargo test --workspace` passes: sprawl 91, sprawl-advisor 252, sprawl-core 80,
  sprawl-install 18, no warnings. The release build ran the speed guard test and passed.

## What was not run

Say this plainly in the review, and check the code for it with extra care:

- The Program Files install, the Setup-based update and the removal of the old per-user copy were
  **not run on any PC** before the release (Setup would install into Program Files on the
  developer's machine). The first real run is a user's.
- No screenshots were taken of the new dialogs: "Install Sprawl for all users?", "Close the other
  Sprawl processes?", "Stop and close" / "Close anyway", Forget key.
- A 1.0.4 copy installed per user updates in place (its updater knows nothing of `setup`). The
  1.0.5 it becomes runs from the user's folder, so it no longer runs as administrator, and offers
  to install for all users. Check that this path is safe and that nothing loops (offer, refuse,
  offer again).

## Fixed, by finding

### 1. Elevated Sprawl only from Program Files (core P1-1)

The user chose this over accepting the risk.

- `installer/sprawl.iss`: `DefaultDirName={autopf}\Sprawl`, `PrivilegesRequired=admin`,
  `UsePreviousAppDir=no`, `DisableDirPage=yes`; icons in `{autoprograms}` and `{autodesktop}`.
  [Run] starts Sprawl `runasoriginaluser`; on a silent run with `/RESTARTSPRAWL` it starts it
  after an update.
- `crates/sprawl-install/src/manifest.rs`: `Manifest.setup: Option<FileEntry>`, serde default.
  `verify` requires the name `Sprawl-Setup-<version>.exe`, a 64-hex hash, and no clash with a
  program file. `sprawl-release` signs it with the rest.
- `crates/sprawl-install/src/machine.rs` (new): `program_files()` from `SHGetKnownFolderPath`;
  `is_protected_copy()`; `may_elevate()` (debug build or protected copy); `per_user_install()`
  reads HKCU `Sprawl_is1` and accepts only an `unins*.exe` in that folder, never the running copy;
  `remove_per_user_install(wait)` runs it unelevated with `/VERYSILENT`.
- `crates/sprawl/src/dialogs.rs` `restart_elevated` refuses when `!may_elevate()`;
  `crates/sprawl/src/main.rs` `launch()` gates the "always run as administrator" restart, sets
  `ADMIN_REFUSED`, and (unelevated, protected copy only) removes the old per-user install once,
  re-registering "Scan with Sprawl" if it was on.
- `crates/sprawl/src/update.rs`: a protected copy updates through `fetch_setup` (a new folder
  `%LOCALAPPDATA%\Sprawl\setup-<pid>-<nanos>`, size and SHA-256 checked, `create_new`, reopened
  with `FILE_SHARE_READ` only, hashed again from that handle) and `run_setup` (`ShellExecuteW
  "runas"` with `/SILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS /RESTARTSPRAWL`).

**Check:** is holding the file with read-only sharing until `ShellExecuteW` returns enough (the
elevated process opens the image itself)? Can the per-user uninstaller, run as the user, do
anything the user couldn't? Does a Program Files copy started elevated by someone else skip the
removal (it should do nothing when elevated)? Is `is_protected_copy()` right for a junction or a
renamed Program Files?

### 2. Git never runs where a repository's config could start a program (1.0.4-1.0.5 P1-2, backlog P1)

`crates/sprawl-advisor/src/devgit.rs`:

- `git_allowed()` = `elevation() == Ok(false)`: no git for advice when elevated.
- `config_ok(text)`: every section and key must be on the `PLAIN` allowlist; anything else, a
  continued line or an unreadable header is a no.
- `repo_ok(repo)`: lexical path, no `.git\commondir`, `.git\config` and `.git\config.worktree`
  pass `config_ok`, read through `local::read_checked` (a link is an error).
- `worktree_main(wt)`: Sprawl reads the worktree's `.git` file itself; `gitdir:` must resolve to
  `<main>\.git\worktrees\<name>` with `commondir` = `../..`. Git's `rev-parse` is no longer asked.
  This also answers 1.0.4-1.0.5 P2-6 (a bare repository's worktree is skipped).
- Every git call and clean-up command carries `HARDEN` (`core.fsmonitor=false`,
  `core.untrackedCache=false`, `core.hooksPath=NUL`, `log.showSignature=false`,
  `protocol.allow=never`).
- `crates/sprawl-advisor/src/clean.rs`: a `git ` command runs only after `devgit::recheck`, which
  checks the configs again and requires the command to equal what Sprawl builds for that path now.

**Check:** is the `PLAIN` list too wide (any key on it that git turns into a program, a path or a
network fetch)? Does any git subcommand Sprawl runs read files outside `.git\config` that could
run a program (attributes with a filter driver, `info/` files, the global and system configs)?

### 3. Old clones are the user's own files (1.0.4-1.0.5 P1-3, backlog P2)

`dev.old-clone` is Orange (Recycle Bin, "my own files" asked, never ticked by a goal). It is
offered only when `origin` is a server URL (`server_url`), `git log HEAD --branches --tags --not
--remotes` is empty, and `git ls-files -v` shows no assume-unchanged or skip-worktree entry
(`hidden_from_status`). The same `ls-files -v` check is on `dev.worktree`.

**Check:** stashes, notes refs and other refs outside branches and tags; untracked and ignored
files (they are lost with the clone).

### 4. Emptying the Recycle Bin goes first (1.0.4-1.0.5 P1-1, backlog P2)

`crates/sprawl/src/cleanup.rs` `empty_bin_first`: when any item of a run goes to the bin,
`win.recycle-bin` moves to the front. Test: `emptying_the_bin_goes_before_anything_sent_to_it`.

### 5. An unreadable memory holds clean-ups (AI P1-1)

`crates/sprawl-advisor/src/memory.rs` `Memory::load_app()`: a read error, non-UTF-8 text or a
parse error is an error and the file is left alone (no rename). The app keeps `memory_error`,
saves nothing over the file, says so, and `cleanup_start` refuses. `cleanup_start` also reads the
Keep list from disk again and adds it to the run's keeps. Test:
`an_unreadable_memory_is_an_error_and_left_alone`.

### 6. The P2s

| Finding | Fix | Where |
|---|---|---|
| 1.0.4-1.0.5 P2-1, Stop timeout | The run never ends while its worker acts; after 5 s the item says it is finishing. Closing during a clean-up offers "Stop and close"; asked again, "Close anyway" (also backlog P3 5). | `cleanup.rs`, `commands.rs` `ask_close` |
| 1.0.4-1.0.5 P2-2, owed check | `recheck_if_owed` is level-triggered once per `rules_gen`; `owed_before_cleaning()` holds Review and `cleanup_start`. | `advisor.rs`, `cleanup.rs` |
| 1.0.4-1.0.5 P2-3, "from the report only" | `agent::report_only` keeps the note within 100 characters; block cleaners carry it in `note`. | `llm/agent.rs`, `llm/tools.rs` |
| 1.0.4-1.0.5 P2-4, typed key crosses providers | `forget_typed_key()` on a provider or address change; the wizard empties its key box. | `advisor.rs`, `wizard.rs` |
| 1.0.4-1.0.5 P2-5, goal ticks | `goal_may_pick` leaves out items that can't be undone, worktrees and old clones. | `cleanup.rs` |
| 1.0.4-1.0.5 P2-6, bare repository worktree | Answered by item 2. | `devgit.rs` |
| backlog P2, virtual disk left attached | A stopped shrink runs `remedies::detach_command`; its own exact-form check in `elevated_tool_ok`. | `remedies.rs`, `clean.rs` |
| AI P2-1, AI cleaner parts | Amber only when every path is in AppData, Temp or ProgramData; otherwise Orange. | `memory.rs` `in_app_data` |
| AI P2-2, notes the model left | `context()` lists them apart, as unverified; Remembered marks a model's label "(from the AI)". | `memory.rs`, `advisor.rs` |
| AI P2-3, untrusted file text | `read_manifest` output is wrapped by `mark_file`; the four prompts say names, contents and tool answers are data. | `llm/tools.rs`, `llm/agent.rs` |
| AI P2-4, huge `min_bytes` | Clamped to 2^50 in the tool and in `Memory::propose`. | `llm/tools.rs`, `memory.rs` |
| AI P2-5, removing a key | Forget key; removing a custom AI deletes its host's key unless shared; the old single key goes too when it was that host's. | `advisor.rs`, `config.rs` |
| AI P2-6, two windows | `Memory::merged(base, mine, theirs)`, a three-way merge before each save. Test: `another_windows_keeps_survive_a_save`. | `memory.rs`, `advisor.rs` |
| core P2-1, `timing()` | Off unless Windows confirms Sprawl is not elevated. | `sprawl-core/src/lib.rs` |
| core P2-2, stale recovery folder | `recovery::prepare` sets an old folder aside as `.sprawl-recovery.<n>`. | `sprawl-install/src/recovery.rs` |
| core P2-3, WOF sizes | `WofCompressedData` stream at VCN 0 added to `allocated`. | `sprawl-core/src/mft.rs` |
| core P2-4, damaged records | `is_damaged` and `Index::unlisted` counted into the first tree's `errors`. Test: `wof_sizes_and_what_couldnt_be_read`. | `sprawl-core/src/mft.rs` |
| core P2-5, other Sprawl processes | "Close the other Sprawl processes?" before a per-user update swaps files; a Program Files copy leaves it to Setup's `/CLOSEAPPLICATIONS`. | `update.rs`, `sprawl-install/src/processes.rs` |

### 7. Keys per host (new in 1.0.5, worth a look)

Several custom AIs: each OpenAI-compatible server keeps its key under
`Sprawl/openai-compatible@<host>` and it is sent only to that host. **Check:** host matching
(port, case, IDN, a trailing dot), and what happens to a key when a custom AI's address is edited.

## Not done

- core P2-2, second half: the update preflight still runs after other processes are closed, not
  before.
- AI P2-2, second half: an AI's label is not marked in the Owner column, and the model's notes are
  still stored without a click.
- Every P3 in the four reports is open, except backlog P3 5 (closing during a clean-up).

## Questions for the reviewer

1. The memory merge: an item removed in this window and changed in another; this window's removal
   wins. Is that right for Keeps?
2. Does `Index::unlisted` count records that are legitimately not shown (system files, records
   for streams)? A false count would show as scan errors.
3. Is "can't open the file with no sharing" a fair sign that a virtual disk is still attached?
4. Item 1's questions, and whether anything stops a non-administrator from planting a file or
   folder Setup will trust (`%LOCALAPPDATA%\Sprawl\setup-*` is the user's own folder).
5. Anything in the list above that is fixed in name only: a test that passes without the fix.

Findings go into 1.0.6.
