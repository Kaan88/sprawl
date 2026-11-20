# Fable re-review of the backlog

Fable, 21 November 2026. Read-only source review of the working tree on `main` (HEAD `fc2d97c`
plus the uncommitted 1.0.5 edits another session has in the checkout). Nothing was built, run,
formatted or changed; this file is the only thing written. Line numbers are those of the files
as they are now.

## Verdict

**Changes requested, for one finding outside the listed sections; the listed backlog itself holds.**

- The old "Opus-reviewed, awaiting Fable" work (handle-based clean-up from d22c76f, multi-select
  Delete, plans and picks) is sound as it stands. I found no path in it that deletes, moves or
  links the wrong thing. Most of it was also read by Astra in the pre-1.0 audit (AR01 to AR33)
  and again in the four 1.0.2 rounds and the 1.0.3 review.
- The five "unreviewed / awaiting review" sections were all read by Astra (its sections 2 to 6),
  and the code they cover has not changed in substance since. No new finding there.
- S4 #5 still holds as an accepted risk. It has not got worse.
- The findings below come mostly from code that came after Astra's last report: the 1.0.4
  work (dev folders, Windows' clean-ups in the app). `docs/plans/astra-review-1.0.4-prompt.md`
  exists but there is no Astra report for it, so nobody outside has read that code. The one P1
  is there.

Counts: **P1: 1, P2: 3, P3: 5.**

## Findings

### P1

**1. Scanning a folder can run a program named by a git repository inside it.**
`crates/sprawl-advisor/src/devgit.rs:41-73` (`run_git`), `:82-87` (`find`), `:152` (`clean`),
`:176-187` (worktree), `:240-246` (clone); called from `crates/sprawl-advisor/src/lib.rs:262`
on every advice run, with no click.

`run_git` switches off `core.fsmonitor`, and its comment says a repository's settings can't
start a program. That is not the only setting that does. `git status` hashes any tracked file
whose stat data doesn't match the index, and runs the file through the `filter.<name>.clean`
(or `process`) command when `.gitattributes` assigns one. Both the attribute and the command
come from the repository: `.gitattributes` in the tree, `filter.x.clean = <program>` in
`.git/config`. Git's own guard (`safe.directory`) only refuses repositories owned by another
account, and an extracted archive is owned by the user.

Scenario: the user extracts a downloaded archive that holds a `.git` folder (a "project" or
dataset of 200 MB or more, with file times older than 180 days, which an archive carries). They
scan the drive. The Advisor's analysis reaches `clean(run, &path)`, git runs the filter
command, and the attacker's program runs as the user. With "tools from my own folders as
administrator" switched on it runs as administrator.

Two related points in the same code:
- A worktree's `.git` is a file holding `gitdir: <path>`. `rev-parse --git-common-dir` follows
  it wherever it points, a `\\host\share` path included, so git contacts that server (Windows
  sends the account's credentials hash). Sprawl refuses network paths everywhere else.
- The clean-ups Sprawl runs when ticked have the same exposure: `git worktree remove` checks
  status first (filters again), and `git gc` honours `gc.recentObjectsHook` (git 2.42 and
  later), which is a command.

Not tested by running; it follows from git's documented behaviour.

Suggested fix: before asking git anything about a repository, read its `config` (for a
worktree: the common folder's `config` and `config.worktree`) through a pinned local handle
and skip the repository unless every section and key is on a short allowlist (`core` with the
plain keys such as `repositoryformatversion`, `filemode`, `bare`, `logallrefupdates`,
`symlinks`, `ignorecase`, `autocrlf`; `remote`, `branch`, `user`, `submodule` url/active).
Anything else (`filter`, `diff`, `merge`, `include`, `includeIf`, `core.fsmonitor`,
`core.sshCommand`, `core.hooksPath`, `gc.recentObjectsHook`, `alias`, `credential`, ...) means
no suggestion. Resolve a worktree's `gitdir:` yourself and require it to be local
(`local::lexical` and a pinned folder) before calling git. Apply the same check again just
before `worktree remove` and `gc` run. Correct the comment on `run_git`.

### P2

**2. Emptying the Recycle Bin in the same run erases what that run just sent there.**
`crates/sprawl/src/cleanup.rs:370` (`MACHINE_WIDE`), `:795-909` (the run, in list order),
`:1601-1614` (`outcome`), `:1638-1644` (lead line); `crates/sprawl-advisor/rules/builtin.toml:341-348`.

Since 1.0.4 "Recycle Bin" (`Clear-RecycleBin -Force`) can be ticked and run in the app. Items
run in the report's order. If an own-files item (orange, or a second-click row, both "To the
Recycle Bin") comes before the Recycle Bin row, it is moved to the bin and then the bin is
emptied a moment later. The review says of that group "Explorer can put it back", and the
own-files checkbox says they can be brought back. A user who ticks everything to free the most
space loses those files for good.

Suggested fix: run `win.recycle-bin` before any item whose fate is `Bin` (sort it first in
`work`), or hold it with a reason when the same run has `Bin` items. Either makes the review's
sentence true.

**3. "Clone you can download again" deletes for good on evidence that can be stale.**
`crates/sprawl-advisor/src/devgit.rs:235-268`.

The finding is amber with `Action::Remove`. Only orange and above go to the Recycle Bin
(`goes_to_bin`, `cleanup.rs:471-478`) and ask for the own-files box, so an old clone is deleted
for good, and "tick enough for N GB" (`cleanup_select_enough`, `:740-749`) can pick it.

What is lost that the checks don't see:
- files git ignores (`.env`, local databases, IDE settings): the evidence says so, but they
  are the user's own files, and they go without the bin;
- commits whose remote branch was deleted since the last fetch: `git log --branches --not
  --remotes` compares with the local remote-tracking refs, never the remote, and with any
  remote, not only origin. "Every branch is on its remote" is then untrue;
- changes in files marked skip-worktree or assume-unchanged, which `status` doesn't list;
- tags and notes never pushed.

Scenario: a repository last touched a year ago, its feature branch pushed and later deleted
on the server, a `.env` with keys in it. Sprawl offers it as re-downloadable; the user ticks
it with other amber items; it is deleted permanently.

Suggested fix: make `dev.old-clone` orange (the queue entry asks exactly this: yes). It then
goes to the bin when that setting is on and needs the own-files box. Soften the evidence to
"every branch was on its remote when you last fetched". Consider leaving out a clone that has
any ignored file outside known build-output folders.

**4. Stop, the one-hour limit or "Close anyway" during a virtual disk shrink leaves the disk attached.**
`crates/sprawl-advisor/src/remedies.rs:211-232` (`shrink_command`),
`crates/sprawl-advisor/src/clean.rs:2062-2091` (the job is ended), `crates/sprawl/src/commands.rs:782-788`
and `:756-759` ("Close anyway" calls `std::process::exit`).

The shrink pipes `select vdisk / attach vdisk readonly / compact vdisk / detach vdisk` into
diskpart. Compacting takes minutes. Stop (or closing Sprawl, or the limit) ends the job, which
kills diskpart between `attach` and `detach`. An attachment outlives diskpart, so the `.vhdx`
stays attached read-only. WSL or Docker then fails to start ("file in use") until the user
detaches it in Disk Management or restarts Windows. Nothing tells them why. Not tested by
running.

Suggested fix: when a shrink is stopped or times out, run a short follow-up through the same
checked path (`select vdisk file=...` / `detach vdisk`) before reporting, and say in the
outcome if that failed ("the disk is still attached: restart Windows or detach it in Disk
Management"). For "Close anyway", either wait for that detach or name the consequence in the
question when a shrink is the running item.

### P3

**5. A link run that ends part way leaves `name.sprawl-original` with no recovery and no notice.**
`crates/sprawl-advisor/src/clean.rs:1303-1305` and `:1322-1347`; `commands.rs:782-788`.

Between the rename to `X.sprawl-original` and `hard_link(keep, X)` the copy's name does not
exist. "Close anyway" (an immediate `process::exit`), a crash or power loss in that moment
leaves the file under the backup name only. Later in the same function an exit leaves both the
link and a read-only backup. No data is lost, and the window is microseconds per copy, but
nothing in Sprawl looks for these files afterwards (the name appears nowhere else in the
code). Suggested fix: let "Close anyway" set Stop and wait for the current item, as the tree
Delete's close already does (`commands.rs:767-780`), and have the duplicate search report any
`*.sprawl-original` it meets as "left by an interrupted link".

**6. Rows picked with Ctrl or Shift survive a clean-up's rescan as dead nodes.**
`crates/sprawl/src/commands.rs:1135-1217` (`apply_rescans` remaps `selected` at `:1201` and
`expanded`, not `marked`), `:1034` (`detach` sets `is_dir = true` and sizes to 0), `:704-723`,
`:812-838`.

Pick two folders in the tree, clean something in the Advisor whose path holds one of them
(its branch is replaced by a fresh scan), go back and press Delete. The picked index is now a
detached node: the confirmation lists the right path, but with 0 bytes, so the total is
understated; a picked file is refused as "a file became a folder". What is deleted is still
the path shown, through the live checks, so nothing wrong goes. Suggested fix: in
`apply_rescans`, `self.marked.retain(|&n| alive(t, n))` (or clear it).

**7. A linked copy also takes the kept file's permissions, owner and attributes, which the review doesn't say.**
`crates/sprawl-advisor/src/clean.rs:1258-1305`; text at `crates/sprawl-advisor/src/dupes.rs:238-242`.

The review says a change to one changes all and that each takes the kept one's dates. A hard
link also shares the security descriptor, the read-only, hidden and encrypted bits and NTFS
compression. A copy in a shared folder linked to a keeper in the private profile becomes
unreadable to the other accounts; the other way round, another account that could write the
public file now writes the file the private path opens. Suggested fix: compare owner and DACL
(`GetSecurityInfo` on both handles) and the encrypted, compressed, hidden and system bits, and
leave the copy alone on a difference; or at least say it in the evidence line.

**8. A pipe client that connects and sends nothing blocks the app's MCP bridge for everyone.**
`crates/sprawl-advisor/src/pipe.rs:79-88`. `read_line` has no time limit and the pipe serves
one client at a time (the old L1). Same account only, so this is a nuisance, not a boundary.
Suggested fix: give up on a connection that sends no complete line within a few seconds
(overlapped read with a wait, or a watchdog that calls `DisconnectNamedPipe`).

**9. "Restart as administrator" drops a multi-folder scan, and can fail silently with many ticks.**
`crates/sprawl/src/commands.rs:209-223`, `crates/sprawl/src/state.rs:327`. For a scan of
several folders `root_path` is the literal `Multiple`, which the new copy is told to scan.
Up to 500 `--tick <path>` arguments can pass the 32,767-character command line limit; then
`ShellExecuteW` fails and nothing happens. Read from the code, not run. Suggested fix: pass
each real root, and hand the ticks over in a per-user file written with the checked writer
(or cap by length and say how many were carried).

## Accepted risks

**S4 #5, duplicate linking (`link_with`, `clean.rs:1248-1352`): still holds. Not worse. No change needed now.**
I read the function line by line. The order is: single-link check, byte comparison on held
handles, no named stream on either file, rename to the backup by handle, link at the vacant
name, read-only, stream check, delete mark (`FileDispositionInfoEx`, not POSIX), stream check
again, close. What remains needs all of these in one instant: another program that already
holds or opens the just-renamed backup with attribute-only access, clears read-only, creates
a stream between lines 1326 and 1330, and Sprawl ending between 1330 and 1335. What would be
lost is that new stream only; the main data is the keeper's, byte for byte. The copy has one
name and had no stream when checked. The comment at `:1318-1321` states the risk accurately.
Finding 5 (exit between rename and link) and finding 7 (permissions) are separate from it.

**"Known, accepted for now":**
- `sprawl-rm` TOCTOU in the exported script: **holds, and smaller than before.** "Run now" is
  gone (only the unused `-Report` parameter remains in `export.rs:37`), so Sprawl never runs
  the script itself, elevated or not. The stale attribute test in `RM_FN` (`rules.rs:550`)
  could be narrowed cheaply by reading the attributes again (`$i.Refresh()`) just before
  descending; that doesn't close it. Fine to leave.
- Nested projects judged by the member's own files: holds (errs toward keeping).
- `ownable()` gaps: holds (Owner column only).
- Findings re-resolved by path, case-insensitively: holds. Astra's later fixes made the
  deletion checks case-exact (ids, `seen_in`); what is left is that a tick on `Cache` also
  ticks a sibling `cache` on a case-sensitive folder, and both show in the review.
- Build-output gate only knows `.exe`: holds.
- Prefetch "current" for 14 days: holds.
- Delete footer overwritten: cosmetic; the Delete card now shows progress.
- Analysis thread holds the tree: holds (cost only).
- Programs in outputs matched by name: holds.
- A deleted file returning when the filter is cleared: not re-checked in depth;
  `apply_rescans` now updates the full tree first (`commands.rs:1142-1150`), which looks like
  it covers it.
- Phases 3-4 lows: L10, L11, L17, L18, L19 hold. L16 (`taskkill /T`) is **gone**: tools and
  the AI CLIs run in kill-on-close jobs. L9 (the model's notes and labels persist without
  approval) holds; with the orange floor and the review for anything a model proposes
  (`safety.rs:935-941`) a planted note can steer advice but not delete.

**"Left open" items in the Opus section:**
- L1 one pipe client at a time: holds; see finding 8.
- L4 elevation race, L10 user names in the pipe name, the server-side DACL, `server_is_us` by
  path: all **fixed** since (SID in the name and DACL, token and image checks, one request
  per connection). Covered by Astra (AR24, S3 #4).
- L8 growth against "now": fixed (AR25). L11 `never.complete()` in the MCP process: read-only
  paths; the restricted bridge uses `Never::inert`.
- L7 driven snapshots, L3 duplicate search holding the tree, L12 OneDrive roots, L13 replay
  on the UI thread, the SSE and HTTP-stream items: not re-read; none can delete.
- L8 Dev Drive or VHDX as a fixed drive for moves: holds; there is no in-app move.
- Round 5 L8 `t.errors` after a splice: cosmetic, holds.
- d22c76f "has not had a second read": it has now. See below.

## Status per queue section

| Section | Status |
|---|---|
| Opus-reviewed, awaiting Fable: in-app clean-up (d22c76f and later) | **Fine.** Read in full at HEAD. Also covered by Astra (AR01, AR11, AR20, AR22, S1 #3, S4 #1, S4 #5). Finding 5 is the only thing new. |
| Same section: multi-select Delete (`delete_many`, `picked_nodes`) | **Finding 6 (P3).** The delete path itself is fine and was covered by Astra (AR20, S1 #6). |
| Same section: plans and picks (f5ccfcf, 38d00fd), scripts | **Covered by Astra** (AR12, S1 #4, S2 #2). "Add to plan" no longer exists as described; `script_items` and `export::script` read and fine. |
| Same section: Restart as administrator `--advisor --tick` | **Finding 9 (P3).** Ticks only apply to tickable findings of the new report and the review still asks: fine. |
| Same section: close guard | **Findings 4 and 5.** The Delete's close waits for its item; the clean-up's exits at once. |
| Elevated Sprawl and non-administrator MCP clients | **Covered by Astra** (section 3, four rounds; fixed by round 4). `pipe.rs`, `mcp.rs`, `mcp_scan.rs` unchanged since; `bridge.rs` re-read, fine. Finding 8 is a nuisance only. |
| Fixes from the Opus whole-codebase audit (e4c113e) | **Covered by Astra** (section 2). Not redone. |
| Design branches | **Covered by Astra** (section 4). `outcome`, `lead_line`, `final_action` re-read at HEAD: they agree with the executor. Finding 2 is a new interplay from 1.0.4. |
| After 1.0.2's fixes: Cleaners roots, Codex, Markdown | **Covered by Astra** (section 5). The later diffs to `cleaners.rs` and `ui.rs` were read: no escaping bypass, no new action path. Codex's 1.0.4 and 1.0.5 changes were not read (other queue entries). |
| 1.0.2 (reliability) | **Covered by Astra** (section 6, fixed). `history.rs` unchanged since except a test. |
| Accepted risk S4 #5 | **Holds.** |
| Known, accepted for now | **Holds**; one entry obsolete (L16), one smaller (script). |
| Not in the brief, found on the way: 1.0.4 C and D | **Findings 1, 2, 3.** No outside review exists for 1.0.4. It should get one. |

## What I read

**Fully, line by line:**
- `crates/sprawl-advisor/src/clean.rs` lines 1-2100 (everything but the tests): `action`,
  `Spare::is_new`, `seen_in`, `run_with`, the package-tool binding, `preview`, the privacy
  `set`, `tree`, `whole`, `recycle_one`, `link_with`, the `win` module (share modes, delete,
  rename, stream listing, job), `elevated_tool_ok`, `run_powershell`.
- `crates/sprawl/src/commands.rs` lines 650-1217: `delete_refusal`, `picked_nodes`, `delete`,
  `delete_many`, `manual_delete_paths`, `delete_poll`, `ask_close`, `detach`,
  `replace_branch`, `apply_rescans`.
- `crates/sprawl/src/cleanup.rs` lines 337-920 and 1572-1700: the tick gates, `second_try`,
  `cleanup_start` and its worker, `outcome`, `lead_line`, `final_action`.
- `crates/sprawl-advisor/src/pipe.rs`, `crates/sprawl/src/bridge.rs`.
- `crates/sprawl-advisor/src/devgit.rs` lines 1-300.
- `crates/sprawl-advisor/src/export.rs` lines 1-130 and `RM_FN` in `rules.rs`.
- `crates/sprawl-advisor/src/safety.rs` lines 376-506 (the never-list matchers) and 783-975
  (`check`).
- `report_finding` in `crates/sprawl-advisor/src/llm/tools.rs`; `shrink_command` in
  `remedies.rs`.
- Astra's five reports, and the queue sections named in the brief.

**Skimmed or by diff only:** `cleaners.rs`, `recycle.rs`, `history.rs`, `ui.rs` (diffs since
Astra's round 4), `dupes.rs` (the link text and how the keeper is chosen), `main.rs` (argument
parsing, close hook), `dialogs.rs` (`restart_elevated`, `quote`), the Astra ledger in the queue.

**Not read:** `sprawl-core` (`local.rs`, `walk.rs`, `mft.rs`, `wof.rs`, `identity.rs`),
`privacy.rs`, `mcp.rs`, `mcp_scan.rs`, `memory.rs`, `codex.rs`, `claude_code.rs`, `agent.rs`,
`advisor.rs` beyond `script_items`, `shellmenu.rs`, `admin.rs`, the updater and installer, the
1.0.5 working-tree changes as such. Where a finding or a "fine" above depends on these
(`local::lexical`, `local::directory`, `local::file`, `recycle::bin_for`), I relied on Astra's
reads and on their doc comments.

**Limits:** nothing was run. Findings 1, 4 and 9 follow from the code and from how git,
diskpart and `ShellExecuteW` are documented to behave; none was reproduced.
