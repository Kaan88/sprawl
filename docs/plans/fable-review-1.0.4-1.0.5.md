# Fable review: Sprawl 1.0.4 and the 1.0.5 work so far

Reviewer: Fable (outside safety review), 2026-11-21.
Scope: `cab2211..HEAD` (fc2d97c) plus the uncommitted working tree on `main` (light mode,
underline page tabs, several custom AIs with a key per host).
Method: read only. Nothing was built, run or tested; every finding below comes from reading the
code, and the ones that depend on git's or Windows' behaviour say so.

## Verdict: changes requested

Three P1 findings (two can delete user data for good against what the review screen says or
implies, one runs programs chosen by a scanned folder), six P2, nine P3.
1.0.4 is already released with P1-1, P1-2, P1-3 and P2-1 in it, so those are the ones to fix first.

Per section of `docs/plans/astra-review-1.0.4-prompt.md`:

| Section | Verdict |
|---|---|
| 1 MFT read | Accept (P3-8) |
| 2 Astra's 1.0.3-fix findings | Accept |
| 3 Compression choice | Accept |
| 4 Windows' clean-ups in the app | 1.0.5 change required (P1-1) |
| 5 UI | Accept |
| 6 Dev folders | 1.0.5 change required (P1-2, P1-3) |
| 7 Reasoning effort | Accept |

The 1.0.5 queue items: 1 (G2) change required (P2-1); 2 (proposals block) change required (P2-3);
3 (deferred check) change required (P2-2); 4 (typed key) change required (P2-4); 5, 6 accept;
7 (key per host) accept with P3-1 to P3-3.

---

## P1: data loss or security

### P1-1. Emptying the Recycle Bin in the same run destroys what that run just sent to the bin

- `crates/sprawl/src/cleanup.rs:370` (`MACHINE_WIDE` makes `win.recycle-bin` tickable and runnable in the app, new in 1.0.4)
- `crates/sprawl/src/cleanup.rs:471-478` (`goes_to_bin`: only orange items, the user's own files, or a forced tick)
- `crates/sprawl/src/cleanup.rs:794-850` (the work list is built in list order; the worker runs it in that order)
- `crates/sprawl-advisor/rules/builtin.toml:341-348` (`Clear-RecycleBin -Force`, every drive)

Scenario: the user ticks one of their own folders (orange; the review says it goes to the
Recycle Bin) and "Recycle Bin" (amber, also what "tick enough for N GB" may pick now that it is
tickable). When the bin item sits after the own folder in the list, the run moves the folder to
`$Recycle.Bin\<sid>` and a moment later `Clear-RecycleBin -Force` deletes it for good. The
review told the user the folder was recoverable. Before 1.0.4 this could not happen in the app
(the bin rule was "Another account's", so untickable).

Nothing in `cleanup_start` orders or separates the two; the "cannot be undone" warning
(`cleanup.rs:1496`) speaks only about the bin item itself.

Fix (smallest): in `cleanup_start`, when the run has any item with `recycle == true`, either
run `win.recycle-bin` first (move it to the front of `work`/`items`), or hold it with "run it
on its own: this clean-up sends files to the bin". Running it first is enough and keeps one
click. Add a test on the ordering.

### P1-2. Git is run automatically inside every scanned repository, and a repository's own config can start programs

- `crates/sprawl-advisor/src/devgit.rs:42-74` (`run_git`), `:79-81` (`git_allowed`), `:161-262` (the calls)
- called from `advise_all` (`crates/sprawl-advisor/src/lib.rs:262`), which runs after every scan with no user action

The request asked: "could a repository's own config still run a program during `status`, `log`
or `rev-parse`?" Yes. Only `core.fsmonitor` is switched off. Still honoured from the scanned
repository's `.git/config` (or the config of whatever a worktree's `.git` file points at):

- `filter.<name>.clean` / `.process` with a `.gitattributes` entry: `git status` runs the
  filter on files whose index entry looks stale. This is the classic "git status in an
  untrusted repository" vector.
- `log.showSignature=true` with `gpg.program=<anything>`: run by `git log` and by
  `git stash list` (which is a log) on a signed commit.
- A worktree's `.git` file is a pointer chosen by the folder: `gitdir: \\host\share\x` makes
  git read its config from a network share (also an NTLM handshake to that host).
- At clean time, `git gc` also honours `gc.recentObjectsHook` (git 2.42+; I believe it is read
  from repository config, not verified here), and `git worktree remove` runs the same status
  machinery.

Scenario: the user extracts an archive (or plugs in a drive, or scans a share) that contains a
folder with a prepared `.git`, 200 MB of padding and old file times (both thresholds are met by
file times and sizes the archive controls). They scan it with Sprawl. The advice run calls
`git status` there and the repository's filter program runs as the user. Git's `safe.directory`
check does not help: the extracted files belong to the user.

When Sprawl is elevated the calls are skipped unless the user allowed "tools from their own
folders" (`elevated_refusal("git status")`; git is not in System32, so the gate holds by
default; good). But that permission was asked for clean-ups the user reviews; here it turns
into git running as administrator in every repository of every scan, unreviewed.

Fix:
1. Never run the advice-time git calls when elevated, whatever `ELEVATED_USER_TOOLS` says.
2. Before asking git about a repository, read its config yourself (`.git/config`, and for a
   worktree the common dir's `config` and `config.worktree`) and skip the repository when it
   has anything outside a small allowlist (`[core]` with known-harmless keys, `[remote]`,
   `[branch]`, `[user]`, `[submodule]`, `[lfs]`-free). In particular skip on any `[filter`,
   `[diff`, `[merge`, `[gpg`, `[include`, `[includeIf`, `[alias`, `[credential`, `[gc]`
   `recentObjectsHook`, `core.sshCommand`, `core.pager`, `core.hooksPath`, `core.worktree`,
   `core.fsmonitor`, `extensions.*` other than `worktreeConfig`.
3. For a worktree, require the `gitdir:` target to be a local path inside the scanned tree
   (no UNC), before any git call.
4. Add `-c log.showSignature=false -c core.hooksPath=NUL -c protocol.allow=never` and
   `GIT_CONFIG_NOSYSTEM`-independent hardening as defence in depth; they do not replace 2.
5. The same config check again in the clean-up path before `worktree remove` / `gc` run.

### P1-3. "Clone you can download again" can delete work that is not on the remote, and it is deleted for good

- `crates/sprawl-advisor/src/devgit.rs:228-262`
- `crates/sprawl/src/cleanup.rs:471-478` (`goes_to_bin` needs orange)

What the queue entry says ("Delete, to the Recycle Bin when it is on") is not what the code
does: the finding is amber, and only orange items go to the bin. A clone outside the user-data
folders (`C:\dev\x`, `D:\code\x`, `%USERPROFILE%\projects\x`) stays amber and `Action::Remove`
deletes it permanently. Clones under Documents or `Users\*\source` are raised to orange by
`safety::check` and do go to the bin.

What the "everything is on its remote" test misses:

- **Commits on a detached HEAD.** `git log --branches --not --remotes` does not look at HEAD.
  A checkout of a tag or a commit with local commits on top has a clean status, no stash, one
  worktree, and is offered. Use `HEAD --branches --tags --not --remotes`.
- **Local tags** not pushed (same fix).
- **An origin that is not a server.** `origin` may be a local path or `file://` (a clone of
  another folder that may be gone, or that is itself in the same clean-up). "git clone brings
  it back" is then false. Require `https://`, `ssh://`, `git://` or `user@host:` form.
- **Remote-tracking refs are a memory, not the remote.** After 180+ idle days the branch may
  have been deleted or force-pushed upstream. The evidence line says "every branch is on its
  remote"; it should say "was, when last fetched".
- **Files hidden from status**: ignored files (said in the evidence, good), and tracked files
  marked `assume-unchanged` / `skip-worktree` with local edits (not said, and not seen by
  `status`). `git ls-files -v` shows them; skip the clone when any are set.
- `.git/hooks`, `.git/config` (credentials, local settings), `.git/info/exclude`.

Scenario: `D:\work\client-x`, last touched 7 months ago, HEAD detached with three local
commits made during a bisect, plus a `.env`. "Tick enough for 40 GB" picks it (amber, built-in);
the user confirms a long review list; the folder is deleted, not recycled. The commits and the
`.env` are gone.

Fix: make `dev.old-clone` orange (it then goes to the bin, needs the "my own files" tick and is
never picked by the goal), and add the ref, tag, origin-form and `ls-files -v` checks. Until
then I would not offer it as a Delete at all (Info, like `dev.same-clone`).

The same `assume-unchanged` gap applies to `dev.worktree` (git's own refusal does not see such
edits either): add the `ls-files -v` check there too.

---

## P2: wrong behaviour

### P2-1. The 5 s Stop timeout ends the run while the worker is still acting, and drops its result

- `crates/sprawl/src/cleanup.rs:1028-1040` (`STOP_WAIT`), worker at `:860-905`

The timeout fires for anything still `Running` five seconds after Stop, not only a dead network
drive. Things that routinely take longer after Stop: `rescan_after(&path)` of a large folder
(it runs after the item, before `Msg::Finished`), one large file inside `wof::one` (a multi-GB
file under LZX is not interruptible, `crates/sprawl-core/src/wof.rs:161-200`), and a single
long file-system call.

Consequences when it fires:

- `Msg::Finished` is never received: bytes freed are not counted, the tree is not updated
  (deleted files stay on the map and in findings), learning is not recorded, and a compress
  that did compress files is **not written to `memory.compressed`**, so it is not in the
  Compressed list to undo and is offered again. The message blames a network drive.
- `cleanup.running` is now None while a thread is still working. Every guard that reads it
  stops guarding: a second clean-up of the same items can start (`cleanup.rs:753`), a scan
  (`state.rs:318`), an update install (`update.rs:356,403`), "Restart as administrator"
  (`commands.rs:209`), and closing the window no longer waits (`main.rs:169`).

The author's two questions: a late `Msg` cannot reach a newer run (new channel, old `rx`
dropped: correct). A second run on the same items can start: yes.

Fix: keep the worker's `JoinHandle` (or an `Arc<AtomicBool>` it clears on exit) in the app
state when the run is abandoned, and treat "an abandoned worker is still alive" like
`running.is_some()` in those guards, with a line in the foot ("still waiting for Windows to
return from <item>"). Start the 5 s clock only while the worker is inside `clean::run_with`
(have it send a `Msg::Measuring(i)` before `rescan_after`), or simply keep draining the old
channel in the background so a late `Finished` still records the compress and the rescan.

### P2-2. A check owed after an approval can be skipped for the session, and the clean-up can run on the old findings

- `crates/sprawl/src/advisor.rs:4061-4069` (`recheck_if_owed`), `:975-981` (Report), `crates/sprawl/src/cleanup.rs:1726-1745` (the foot is drawn "whatever section is open"), `:752-756` (`cleanup_start`)

Two holes:

1. **Edge-triggered only.** The check starts on the redraw where the user leaves Remembered,
   and only if nothing is running then. If a check is already running (approve while "Checking
   again" from a scan, an Undo, or a previous leave), `advisor.running` is true, the edge is
   consumed, the running check finishes with the old rules, `check_finished(began)` correctly
   leaves the debt, and nothing starts another check until the next scan. The same when a
   scan job is running at that moment. The findings on screen are then from before the
   approval for the rest of the session.
2. **Clean-up from Remembered.** The selection bar, Review and "Clean now" are in the foot on
   every section. A user can approve a rule (say an information rule that marks a folder as
   theirs, orange or red) and, without leaving Remembered, review and clean the ticks made
   before. `cleanup_start` re-runs `safety::check`, which does not read the rules, so the old
   classification is what runs. Before 1.0.5 the approval restarted the check at once.

Fix: (a) make it level-triggered: when not on Remembered and `check_owed()` and nothing runs,
start; also from the Report handler when the debt is still there after `check_finished`
(guard against a loop with a "failed once for this gen" flag, since `Msg::Failed` leaves the
debt). (b) In `cleanup_start` and when opening the review, if `memory.check_owed()`: run the
check first and say "Your rules changed: checking again before cleaning."

### P2-3. The "From the report only" note is cut off by the 100-character limit

- `crates/sprawl-advisor/src/llm/agent.rs:88-97` (`block_calls` appends the note)
- `crates/sprawl-advisor/src/memory.rs:594-600` (`propose` then runs `clean_text`, which keeps 100 characters; `crates/sprawl-advisor/src/facts.rs:396`)

The note is 55 characters and is appended before the cut, so any `why` longer than about 44
characters loses part or all of it. Most real reasons are longer; a model steered by a folder
name only has to write a long one. The answer to "can the note be removed by the model's own
text" is yes, by length alone. The test uses a 15-character `why`.

Fix: cut the model's `why` to 100 minus the note's length before appending (or carry a
`from_report: bool` on `Proposal` and draw the note in the UI, which cannot be crowded out).
Cleaner proposals from a block get no such note at all; give their parts the same marker.

Otherwise the block path is sound: only `propose_rule` / `propose_cleaner` can come out of it,
both go through `Tools::call` and `Memory::propose*` (id, pattern, risk floor, 50/20 caps), a
cleaner must match the scan and still gets the second opinion, the block is removed from the
shown answer also when unreadable or unclosed, and nothing is used before approval. Ten per
answer is fine.

### P2-4. A typed, unsaved key is sent to another provider when the Provider dropdown changes

- `crates/sprawl/src/advisor.rs:1221-1240` (provider change: reads the fields, does not clear `ai_key_typed`)
- `crates/sprawl/src/advisor.rs:1862-1868` (every redraw: `models.want(p, &u, &ai_key_typed)` for any provider but OpenAI-compatible)
- `crates/sprawl/src/advisor.rs:1925-1929` (the typed key is put back into the new form's box)
- `crates/sprawl/src/wizard.rs:752` (the wizard does the same with `WizKey`)
- `crates/sprawl-advisor/src/llm/models.rs:52-55`

Scenario: Provider is OpenAI-compatible at `https://api.deepseek.com/v1`; the user pastes the
DeepSeek key and then picks "Anthropic API" in the dropdown (wrong provider chosen first, a
common slip). The redraw fetches Anthropic's model list at once with the typed key as
`x-api-key`: a DeepSeek key sent to api.anthropic.com with no button pressed. The other way
round, an Anthropic key typed and then OpenAI-compatible picked: the key sits masked in the
Key box, and Load models or Save sends or binds it to whatever address is there. The same
when a preset changes the address after a key was typed (`advisor.rs:2188-2206`).

The rule "a typed key goes only to the address typed with it" holds inside `models::list`; it
is the app that carries the key across.

Fix: clear `ai_key_typed` (and the PasswordBox) whenever the provider changes or the host of
the address changes (preset pick, custom AI pick already does it); in the wizard clear
`WizKey` in `wizard_provider` when the provider differs from the one the key was typed under.
Remember the (provider, host) the key was typed for and pass the key to `want` only on a match.

### P2-5. "Tick enough for N GB" can now tick things that cannot be undone

- `crates/sprawl/src/cleanup.rs:739-749`, `crates/sprawl-advisor/src/plan.rs:29`, `crates/sprawl/src/cleanup.rs:370-376`

Before 1.0.4 the Recycle Bin, Windows.old, the component store, old worktrees and git gc were
untickable in the app, so the goal never picked them. Now any of them that is amber, built-in
and has a command is a candidate (I did not confirm which of them `plan::plan` ranks in; the
filter at `plan.rs:29` admits amber built-ins with a command). The Recycle Bin and Windows.old
are "gone for good" by Sprawl's own wording (`advisor.rs:588`), and cleanmgr opens a UAC prompt
and a window mid-run.

Fix: leave `gone_for_good(f)` items, `dev.worktree` and `dev.old-clone` out of
`cleanup_select_enough`; the user ticks those by hand.

### P2-6. `git worktree remove` is built from a path git reported, for a repository layout that may not match

- `crates/sprawl-advisor/src/devgit.rs:178-181, 199`

`<main>` is the parent of `--git-common-dir`. For a worktree of a bare repository
(`D:\repos\x.git`), the parent is `D:\repos`, which is not a repository: the command fails
(harmless) or, if `D:\repos` happens to be inside another repository, runs against that one
(git then refuses an unknown worktree; still the wrong place to run). Use
`git --git-dir=<common> worktree remove <path>` instead of `-C <parent>`.
Also `rev-parse` is asked before the idle/size test's `clean` check only for worktrees; fine.

---

## P3: minor

1. **A key can't be removed from the app.** `advisor.rs:1268-1274` calls `set_key` only for a
   non-empty key, so the new delete branch (`config.rs:221-223`) is unreachable from the
   Advisor and the wizard, and the Remove message ("until you save an empty key for it",
   `advisor.rs:1217`) promises a way that isn't there. Add "Forget this key" beside the Key box,
   or delete the host's key on Remove when no other custom AI uses that host.
2. **Keys are per host, not per host and port.** `host_of` drops the port, so a key saved for
   `localhost:11434` is also sent to `localhost:1234`, and one for `example.com:8443` to
   `example.com`. Low risk; put the port in the credential name when it is not the default.
3. **The legacy single key without a recorded host follows the saved address**
   (`agent.rs:730`: `key_host(..).or_else(|| host_of(&AiConfig::load().base_url))`). Unchanged
   code, but with several custom AIs the saved address now changes routinely: a user with a
   pre-host key for A who adds custom AI B without a key gets A's key sent to B (https still
   required). Migrate once at start (write `/host` from the address saved then) and drop the
   fallback.
4. **Renaming a custom AI to an existing name replaces that entry silently**
   (`config.rs:117-123`, case-insensitive). Refuse or ask.
5. **Theme detection starts `reg` by name** (`crates/sprawl/src/theme.rs:31-36`). Rust looks in
   the application folder first; a per-user install run elevated would start a planted
   `reg.exe` as administrator. Use the registry API or `%SystemRoot%\System32\reg.exe`.
   Also `contains("0x1")` matches other output; parse the value.
6. **Light mode was not seen.** `to_light` rewrites every `"#AARRGGBB"`; brushes made in code
   and the map are not touched. Please screenshot the review list, the "cannot be undone"
   lines, held items and the risk chips in light mode before release: a warning that is
   unreadable is a safety defect. User text cannot trip the rewrite (`ui::esc` escapes quotes).
7. **`run_git` leaves processes behind on timeout** (`devgit.rs:68-72`): `kill()` without
   `wait()`, no job object, so git's children live on and the reader thread stays blocked.
   `BUDGET` is checked only between repositories; one slow repository can cost 5 x 15 s.
8. **MFT layout has no coverage check** (`crates/sprawl-core/src/mft.rs:869-895`). After the
   extension records are read, nothing verifies the runs cover VCN 0 to the end without a gap
   or overlap; a missed extent would give a smaller tree instead of falling back to the walk.
   Add the check. The known 100k-entry difference against the walk should be explained before
   it is called hard links: a tree that is silently short makes folder sizes and "nothing
   newer inside" facts wrong. `read_nonresident` itself is fine (zero header ends the list,
   sparse runs are zero-filled, order and length are checked, 16 MB cap, errors fail the read).
9. **`Clear-RecycleBin` under another administrator account** empties that account's bin, not
   the user's, and frees nothing shown. Hold `win.recycle-bin` when the elevated account differs
   from the signed-in one.

---

## Checked and found fine

- **Branch rescans and volume serials** (`replace_branch`, `Tree::replace_volumes`,
  `settle_overlaps`): entries under the branch go, unknown hides an outer serial, whole-name
  lower-case comparison. Serial 0 as unknown errs on the careful side.
- **`note_compressed`**: completion decided first; a clean retry completes a partial record and
  keeps the earlier saving; nothing recorded for a folder never compressed.
- **Zero-ID files** are skipped as changed in `wof::one`.
- **Compression choice**: only the algorithm word changes; `ticked_findings` applies the pick,
  so review, context check and `cleanup_start` see the command that runs; an unknown algorithm
  is refused. (I did not trace `cleanup_context_sync` to confirm a pick changed after the
  review forces a new review.)
- **`machine_wide`**: needs `builtin && trusted` and the exact rule id; a local or AI rule with
  the same id or command stays untickable. `held_in_app` and `elevated_risk` still apply.
- **`no_bin` / `held_here`**: a UNC path or a non-fixed drive is held, never deleted for good
  instead; the review and the run use the same function. Holding every non-fixed type is the
  right default. A `subst` drive reports its target's type; a link to a share is a reparse
  point the scan does not follow.
- **Late messages after Stop**: the old channel is dropped with the `Running`; the old thread's
  sends fail and it starts nothing more.
- **Key rule** (`key_for`, `openai_key`, `key_elsewhere`, `saved_key_for`): a per-host key is
  bound to its own host; plain http only to this PC; `OPENAI_API_KEY` only to api.openai.com.
  Credential names: `split_url` allows only letters, digits, `-`, `.` (or a parsed IPv6
  address) in a host, lower-cased, so `@`, `/` or a port cannot reach or collide with another
  target (`Sprawl/openai-compatible`, `.../host`).
- **Leaving the key on Remove**: acceptable, given P3-1 is fixed.
- **Effort**: a level is sent only when in the model's list; Claude Code and Codex take
  lower-case ASCII only, Codex never `ultra`, so a hand-edited `ai.toml` cannot inject a flag
  or a TOML value; the Codex checks see the same `-c`. Fetched levels are filtered by `plain`.
  The wrong-model race is closed by `effort_drawn` plus `pick`. Whether `max` changes Codex's
  tools I cannot tell from the code; the three checks are the right safeguard.
- **Memory counters**: `rules_gen` / `rules_seen` survive the TOML round trip, an old file owes
  nothing, a check that began before a change does not settle it.
- **`newest_file`**: same traversal as the two copies it replaces (files only, links not
  followed below the start).
- **B7 `own_active`**: `cleanup_start` and `own_start` still refuse while a scan job runs.
- **Delete card and Stop**: display only; Stop sets the existing flag.
- **`-Wait` on cleanmgr**: passes `elevated_tool_ok`; note cleanmgr started with RunAs is
  outside the job, so Stop ends PowerShell, not Disk Clean-up (expected for a GUI).
- **Presets**: addresses are https or localhost; the dropdown only fills the box. I did not
  check the ten URLs against the providers' current documentation.

## What I read, and how closely

Read fully (diff and surrounding code): `llm/agent.rs` (both diffs), `config.rs`, `devgit.rs`
(code, not its tests), `recycle.rs`, `remedies.rs`, `facts.rs`/`projects.rs`/`confidence.rs`
changes, `memory.rs` changes and `propose*`/`approve*`, `llm/effort.rs`, `llm/codex.rs`,
`llm/claude_code.rs`, `llm/anthropic.rs`, `llm/openai.rs`, `llm/models.rs` diffs,
`http::split_url`, `clean.rs` `run_powershell` / `elevated_*`, `cleanup.rs` diff plus
`cleanup_start`, the worker, `cleanup_poll`, `cleanup_finish` head, `advisor.rs` both diffs,
`cleaners.rs`, `commands.rs`, `wizard.rs`, `state.rs`, `main.rs`, `prefs.rs` diffs,
`sprawl-core/src/lib.rs` and `wof.rs` diffs, `mft_layout` and `read_nonresident`, `theme.rs`
(first 125 lines), the 1.0.4 request and the top two queue entries.

Skimmed or not read: `bindings.rs` and `types.txt` (generated), `look.rs` and `ui.rs` (UI),
`treemap.rs`, `views/packed.rs`, `welcome.rs`, `models.rs` (app), the MFT timing changes and
its test, `devgit` tests, `safety::check` (only the risk floors), `plan.rs` (one filter line),
the dated "2026-11-20" queue sections (their content matches the 1.0.4 request, which I read),
`CHANGELOG.md`, the plan documents and mock-ups, `Cargo.lock`.

Not done: no build, no tests, no run of the app, no git, DISM or cleanmgr. Claims about git's
config keys (P1-2) and about what `status` hides (P1-3) are from knowledge of git, not from a
run here; `gc.recentObjectsHook`'s config scope in particular should be confirmed.
