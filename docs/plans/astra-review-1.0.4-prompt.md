# For GPT Astra: review of Sprawl 1.0.4

This file is the whole request: what to do, what changed, and one item per change. Paths are
relative to the repository root.

## What to do

1.0.4 was released from tag `v1.0.4` on the user's word, without waiting for this review.
It was then replaced the same day by a build that also has the effort work (section 7); the
tag now points at the merge.
Anything you find now goes into 1.0.5.

Please review `git diff 7430449..v1.0.4 -- crates` (the `docs` changes are notes). It holds:

| Commit | What |
|---|---|
| `c42c897` | Faster MFT read on fragmented system drives; step timings (section 1) |
| `4c7d4cc` | Fixes for your review of the 1.0.3 fixes (section 2) |
| `e105a81` | The user's UI reports, compression choice, Windows' clean-ups in the app (sections 3-5) |
| `9805add` | Dev folders: worktrees, git gc, re-cloneable clones (section 6) |
| `bff1661` | Delete card position (UI only) |
| `79a873a` (merged in `c6ebade`) | Reasoning effort per model, Provider dropdown (section 7) |

For each section: say whether the change is correct, report any problem it introduces (file
and line, a concrete failure), and give the smallest acceptable fix. End with one verdict per
section: **Accept**, or **1.0.5 change required**.

Read-only: please don't push, release, register tasks, or run clean-ups, compression or git
commands outside temporary fixtures.

## Checks already run

- `cargo test -j 1 --workspace` on the release commit (the merge), in a **non-administrator** shell:
  all passed (app 87, advisor 240, core 79, install 16; ignored tests as before).
  - The real-git worktree test (`devgit::tests::a_real_worktree`) ran its real branch: git was
    present and the shell wasn't elevated.
- A C: scan as administrator, timed with `SPRAWL_TIMING=1`: 16.9 s before `c42c897`, 7.6 s after
  (3.9 s MFT read + 3.7 s sorting).
- One real screenshot of the test build on a fixture folder: the selection bar, the Delete card
  and an "Old git worktree" finding render. Every later capture came out black (display state),
  so the compression dropdown, the Delete card's final position and the Cleaners text were not
  seen on screen.
- Not done: DISM, cleanmgr or Clear-RecycleBin run from the app; a real worktree removal or
  `git gc` from the app; a deliberately fragmented `$MFT` on a real drive (only the synthetic image
  test).

## Section 1: MFT read with a non-resident `$ATTRIBUTE_LIST` (`crates/sprawl-core/src/mft.rs`)

On the user's C:, `$MFT`'s own record 0 had its `$ATTRIBUTE_LIST` stored non-resident (a very
fragmented `$MFT`). `mft_layout` refused that, so `scan_auto` fell back to walking the drive.

- `mft_layout`: a non-resident `$ATTRIBUTE_LIST` is now read with the new `read_nonresident`
  (runlist decoded in VCN order, at most 16 MB). The extension records it names are read for
  the rest of `$DATA`'s runlist, sorted by their starting VCN.
- Test: `reads_a_synthetic_volume_image` builds a second image with the attribute list at LCN 30
  and an extension record holding the runlist from VCN 8.
- Please check: runlist decoding at the end of the list (a zero header), sparse runs (no LCN),
  a list longer than one cluster, and that a malformed list fails the MFT read (then the walk is
  used) rather than producing a wrong tree.
- Known, not fixed: the MFT read finds about 100k fewer entries than the walk on C: (3.79M vs
  3.89M); probably hard links counted once. Not investigated.
- `timing()` (core `lib.rs`) writes step times to `%TEMP%\sprawl-timing.log` only when
  `SPRAWL_TIMING` is set.

## Section 2: your review of the 1.0.3 fixes (`4c7d4cc`)

1. **[P2] Branch rescans dropped the new scan's volume serials.** `replace_branch()`
   (`crates/sprawl/src/commands.rs`) now ends with `Tree::replace_volumes(at, &sub.volumes)`
   (`crates/sprawl-core/src/lib.rs`): entries at or under the branch go, the rescan's come in;
   when the rescan has none covering the branch itself, the branch gets an `UNKNOWN_VOLUME` (0)
   entry, which `volume_of` turns into None, so an outer root's serial isn't lent to fresh ids.
   Tests: `a_rescanned_branch_takes_its_own_volume`, `a_rescan_with_no_volume_does_not_borrow_the_old_one`,
   `a_snapshot_branch_refreshed_live_gets_a_volume`, and volume asserts in
   `a_branch_is_replaced_and_totals_follow`.
2. **[P2] A clean retry that compressed nothing more left the record partial.** The record
   update moved into `note_compressed` (`crates/sprawl/src/cleanup.rs`): completion first; a
   complete run replaces an existing partial record even with zero saving, keeping the earlier
   saving. Test: `compressed_tests::a_clean_retry_that_saves_nothing_completes_the_record`.
3. **Zero-ID files** (`wof.rs` `one`): skipped as changed.
4. **Overlapping roots** (`settle_overlaps`, called by `combine`): an inner root on a different
   volume (or the same path with two serials) becomes unknown.
5. **Wording**: the saving cap is described as `max(it.bytes, o.freed)`.

Please check: serial 0 as "unknown" (a real volume with serial 0 would read as unknown); whole
-name, lower-case root comparison.

## Section 3: compression choice (B1)

- `crates/sprawl-advisor/src/remedies.rs`: `COMPRESS_CHOICES` (xpress4k/8k/16k, lzx) and
  `with_compress_alg(f, alg)`. Only for a finding whose action is one of Sprawl's compress
  commands (`COMPRESS` prefix + algorithm); only the algorithm word is replaced, the quoted
  `<folder>\*` kept, so `compress_scope`'s "names another folder" check still applies. The gain
  estimate is scaled by the algorithms' share of LZX's saving.
- The app keeps the user's pick per path (`Cleanup::algs`) and applies it in `ticked_findings`
  (`with_choices`), so the review, the context check and `cleanup_start`'s safety checks all see
  the command that runs. A dropdown in the item's details sets it; the Advisor is drawn again on
  the next frame (`advisor.rerender`), not inside the dropdown's event.
- Test: `remedies::tests::a_compress_takes_another_algorithm`.
- Please check: that a pick changed after the review is caught by the context check (the
  command differs), and that nothing else reads a compress command from the unmodified finding.

## Section 4: Windows' own clean-ups in the app (C)

- `crates/sprawl/src/cleanup.rs` `MACHINE_WIDE` / `machine_wide()`: `win.component-store` (DISM
  /StartComponentCleanup), `win.recycle-bin` (Clear-RecycleBin -Force) and `win.windows-old`
  (Start-Process cleanmgr ... -Verb RunAs, now with `-Wait` in `rules/builtin.toml`, so the
  saving is measured after it closes) pass `untickable`'s "another account's" gate, only when
  the finding is `builtin && trusted` with that rule id.
- Everything else is unchanged: `clean::action` refuses Tool actions not from the built-in
  pack; `held_in_app` holds admin-needing items when not elevated (the bar offers "Restart as
  administrator"); `elevated_risk` resolves DISM and cleanmgr to System32, and Clear-RecycleBin is
  on its cmdlet list.
- Test: `machine_wide_tests::windows_own_clean_ups_can_be_ticked`; the elevated-command test
  covers the `-Wait` form.
- Please check: whether any of the three should stay out of the in-app run (cleanmgr raises a
  UAC prompt and a GUI; Clear-RecycleBin empties every drive's bin for this account).

## Section 5: the user's UI reports (B2-B7), not safety code

- B2/B6 `cleanup_bar_xaml`: count, Review, Preview and the tick buttons on one row; "On disk now /
  after about / frees about" below; goal ticking and the explanation after.
- B3 `finding_row_inner`: the tick box is in the first grid row beside the item's button; the
  "click again" note and the details go in a second row (the box re-centred before).
- B4 `llm/agent.rs`: the chat prompt names the Ask button instead of "Ask Claude".
- B5 `commands.rs` `delete_card`: a card with a bar (bytes done of the scan's size for the
  items), and Stop, which sets the existing `delete_stop`. Nothing is blocked meanwhile.
- B7 `cleaners.rs` `own_active`: while a scan runs, the Cleaners tab keeps showing its own last
  read (`advice()` returns it before the "scan running" None). `cleanup_start` and `own_start`
  still refuse while a job runs. Please check nothing else assumed `own_active()` implies no job.

## Section 6: dev folders (`crates/sprawl-advisor/src/devgit.rs`, new)

Called from `advise_all` before the build outputs.

- **Git is asked read only.** `run_git`:
  - runs `git -c core.fsmonitor=false -c core.untrackedCache=false -C <dir> ...`;
  - sets `GIT_OPTIONAL_LOCKS=0` and `GIT_TERMINAL_PROMPT=0`, with no window;
  - has 15 s and 1 MB limits, at most 60 repositories and 30 s per advice run.
  - When Sprawl is elevated, git is asked only if `clean::elevated_refusal("git status")` is
    None (the user allowed tools from their own folders). With no answer, nothing is suggested.
- **Old worktree** (`dev.worktree`, Command):
  - which folders: a `.git` file, idle for the project idle days, at least 200 MB, and an empty
    `git status --porcelain=v1 --untracked-files=all`;
  - the command: `git -C '<main>' worktree remove '<worktree>'`, without --force, so git refuses
    a worktree with changes at run time; ignored files go with it, which the evidence says;
  - `<main>` is the parent of `git rev-parse --path-format=absolute --git-common-dir`.
- **git gc** (`dev.git-gc`, Command on the `.git` folder): half the loose objects' size is at
  least 200 MB, or the repository has 30+ packs. Command `git -C '<repo>' gc --quiet`.
- **Old clone** (`dev.old-clone`, Delete, `Action::Remove`, to the Recycle Bin when it is on). It
  needs all of:
  - idle 180+ days and 200 MB+;
  - an origin;
  - an empty status;
  - an empty `git log --branches --not --remotes -1`;
  - an empty `git stash list`;
  - exactly one worktree.

  Remote URLs are shown without a user name or password.
- **Same clone twice** (`dev.same-clone`): Info only.
- The app lets `dev.worktree` and `dev.git-gc` past the "another account's" gate (git acts on
  the folder named). At run time the Tool path is the usual one: `package_tool` returns None, so
  it goes through `tool()` and then `elevated_risk`.
- Seen in the screenshot: the safety checks raise a worktree under Documents to orange ("Your
  own files"), so it needs the user's "my own files" confirmation.
- Tests: `devgit::tests::*`:
  - git's answers scripted;
  - a real-git worktree that is found clean, then not after a new file;
  - every finding passes `safety::check`;
  - no answer means no suggestion.
- Please check:
  - the git invocation (could a repository's own config still run a program during `status`,
    `log` or `rev-parse`?);
  - that a worktree with only ignored files is the only thing `worktree remove` can lose;
  - whether an old clone should be orange rather than amber (its ignored local files, such as
    .env, aren't on the remote).
  - Note that "tick enough for N GB" can pick it like any amber item.

## Section 7: reasoning effort per model, Provider dropdown (`79a873a`)

New module `crates/sprawl-advisor/src/llm/effort.rs`: `levels(provider, base_url, model)` lists
the levels a model really takes, and `pick(cfg, model)` returns the saved level only when it is
in that list (an empty list means no control, nothing sent). Levels are saved per model in
`ai.toml` (`AiConfig.efforts`, serde default).

- Where the levels come from:
  - Anthropic API: `capabilities.effort` from `/v1/models` (`effort::remember_anthropic`, called
    from `models::list`), else a table from the docs. A fetched entry wins, also when it says none.
  - Claude Code: the same table, intersected with what `claude --help` lists for `--effort`.
  - Codex: `supported_reasoning_levels` from `%USERPROFILE%\.codex\models_cache*.json`
    (`codex::efforts`); `ultra` is always left out (automatic task delegation; Sprawl keeps
    `multi_agent` off).
  - OpenAI-compatible: a table for four models, only when the address is empty or
    `https://api.openai.com`.
- How it is sent: Anthropic `output_config.effort`; Chat Completions `reasoning_effort`; Claude
  Code `--effort <level>`; Codex `-c model_reasoning_effort="<level>"`, on the run and on its
  three checks (`mcp list`, `features list`, `debug prompt-input`), so the checked configuration
  is the one that runs.
- Behaviour change: Anthropic used to send `effort: "high"` always; now the user's level, else
  `high` only where the model takes it, else nothing.
- Settings UI (`crates/sprawl/src/advisor.rs`): a Provider dropdown replaces the clipped buttons;
  an Effort dropdown per model, written by `read_ai_fields` only when it was drawn for the same
  model (`effort_drawn`).
- Please check:
  - Codex (safety): can `model_reasoning_effort` at any listed level (`max` included) change the
    tools Codex offers? Is leaving out `ultra` enough?
  - Injection: a level reaches a command line and a TOML `-c` value. Claude Code and Codex take
    lowercase ASCII only (Codex never `ultra`); `effort::plain` filters fetched levels. Enough
    when `ai.toml` is edited by hand?
  - UI race: could a level be saved against the wrong model? (`pick` would still refuse a level
    the model doesn't offer.)

## Verdicts wanted

One per section: 1 MFT read, 2 your 1.0.3-fix findings, 3 compression choice, 4 Windows'
clean-ups, 5 UI, 6 dev folders, 7 reasoning effort.
