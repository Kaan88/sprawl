# Hand-off: the P3 backlog, and polish Part 2 begun (2026-11-21, by Opus)

Everything below is **committed**. `cargo test --workspace -j 1` is green at the commit:
111 (sprawl) / 258 (advisor) / 87 (core) / 22 (install) / 3 (release), no failures, no warnings.

This supersedes `handoff-polish-fix.md`, whose steps 1 to 8 are done (the row flash and panel
blink; `reconcile.rs` is live and the user has confirmed the feel).

Read first: `docs/plans/plan-polish.md` (the plan, Parts 1 to 5), and the memory files
`feedback-polish-baseline.md` (the user's taste) and `reference-dev-tools.md` (how to build,
screenshot and release on this PC).

## Part 1 of the three rules

`-j 1` always. Never `cargo fmt`. `CARGO_TARGET_DIR=target/alt` for every build and test, because
the user's own Sprawl is usually open on `target/release`. Commit, push and release only on the
user's word.

---

## What was finished

### The rest of Fable's P3 findings

Backlog P3-5 to P3-9, core P3-4 to P3-8, installer P3-9 to P3-11. Each is written up for the
safety review in `.claude/fable-review-queue.md` (gitignored), in two standalone entries that can
be read without this document. The user-visible half is in `CHANGELOG.md` under Unreleased.

Three of them are worth knowing about when touching the same code again:

- **The hard-link publish window** (`sprawl-advisor/src/clean.rs`). Between a copy's rename to
  `*.sprawl-original` and the `hard_link` that gives its name back, the file has no name of its
  own. `Publishing::begin()` raises a count and `wait_for_publish` (called from `exit_now`,
  `commands.rs`) waits up to 2 s for it on a close. A plain count is **not** enough: the first
  version was a poll, and a copy could enter the window after the last check and before
  `process::exit`. It is now a Dekker pair — `CLOSING` is stored before the wait, `begin()` loads
  it after raising the count and returns `None` when a close is waiting, and the copy is left
  alone. Keep both stores `SeqCst`.
- **The MCP pipe watchdog** (`sprawl-advisor/src/pipe.rs`). A client that connects and sends
  nothing used to hold the pipe against every other client. The obvious fix **deadlocks**: the
  pipe handle is synchronous, so a `DisconnectNamedPipe` from another thread queues behind the
  very `read_line` it is meant to end, and both threads wait for ever. (It did: it hung the test
  run and held the test binary open, which showed up as `link.exe 1104` in unrelated builds.)
  The watchdog now holds a duplicate of the *serving thread's* handle (`this_thread()`) and calls
  `CancelSynchronousIo`; the read ends with ERROR_OPERATION_ABORTED and the serving thread
  disconnects as usual at the bottom of its loop.
- **Linking refuses more than it used to** (`clean.rs` `link_with`). A copy whose owner,
  permissions (SDDL via `GetSecurityInfo`) or read-only / hidden / system / compressed /
  encrypted marks differ from the keeper is left alone, because one file can only have one set of
  each. Inherited ACEs are part of the comparison, so two copies in different folders almost
  always differ: Link now works mainly within one folder, and the finding's text says so.

### Polish Part 2: one place for the look

`crates/sprawl/src/tokens.rs` is new and is the whole of the mechanism.

- A `Look` (one field per option of the Theme page) makes a token table. Today it has one value
  per field, and the table holds **today's exact colours**, so this part of the work moves values
  without changing one.
- A token is written **only as a whole XAML attribute value**: `Foreground="{$text3}"`.
  `expand()` replaces `="{$name}"`; anything else is left exactly as it is, so a typo fails at
  load on one element rather than quietly painting the wrong colour.
- It is safe against the user's own text because `ui::esc` writes `{}` in front of anything
  starting with `{`: a file named `{$text1}` is shown, not replaced. There is a test for that.
- `expand` runs in the two places every string passes: `state::load` (before `theme::themed`, so
  the light rewrite still happens afterwards exactly as before) and `reconcile::show` (before the
  diff, so both sides of a comparison and every attribute set on a live control see real values).

**The brace trap.** Most XAML in this app is built with `format!`, and inside a format string the
braces must be doubled: `Foreground="{{$text3}}"`. A single brace there fails to compile, so that
mistake cannot survive. A doubled brace in a *plain* string compiles and silently never expands.
`tokens::tests::every_token_the_app_writes_is_a_known_name_in_an_attribute_value` walks every
`.rs` file under `crates/sprawl/src` and fails on both: an unknown name, a token that is not a
whole attribute value, and one that opens doubled and closes single.

16 tokens so far, 325 literals migrated across 15 files:
`surface1/2/3`, `hover`, `pressed`, `line1/2`, `shade`, `text1/2/3`, `selected`, `rest`, `warn`,
`good`, `link`.

---

## Where to pick it up

### Part 2, what is left

1. **Corner radii** (129 uses, 11 distinct values) and **font sizes** (466 uses, 15 distinct).
   Tokenise them **by value** (`rc4`, `rc7`, `fs12`, `fs13`…), not by role. That is enough for the
   two options that need them — Text size scales every size token by a factor, Corners scales
   every corner token — and it avoids inventing a role for 466 sites. Role tokens can come later
   where an option really needs one.
2. **`look.rs` writes some of both as Setter values**, not attributes: `Geo` / `style_with` emit
   `<Setter Property="CornerRadius" Value="{}"/>` and the same for `FontSize`, and `ui::markdown`
   takes its size and colour as Rust arguments. Six shared styles and four call sites are
   invisible to any attribute-level scan — they need the token applied in Rust.
3. **The colour long tail**: 108 of the 133 distinct colours occur one to three times, and the
   greys include near-duplicates (`#FF9AA3B8`, `#FFA0A8B8`, `#FF7A8296`, `#FF6E7789`, and
   `#FFE6EAF2` beside `#FFE6EBF5`). Tokenising them forces a merge decision that changes pixels,
   which Part 2 must not do. **Leave them until Part 3**, where the values change anyway.
4. **Real light values** replace `theme::to_light`'s lightness flipping (plan Part 2.4), and then
   the test that fails on any raw `#AARRGGBB` or numeric `CornerRadius` outside `look.rs` /
   `tokens.rs` (allow the map's colours, the file-type colours, the risk colours).

Exempt from all of it, by name: `advisor.rs::risk_hex` (the four risk colours), `paint.rs`
(`CLASSIC`, `PALETTES`, `colour`) for the file types, and the map's own colours
(`treemap.rs::palette_colour`, `history.rs::change_colour`, `views/flows.rs::risk_colour`,
`galaxy.rs`). None of those are hex literals except `risk_hex`.

### Tools that exist now

- `tools/look-literals.ps1` — the inventory of every colour, corner radius and font size written
  by hand, with per-file counts. `-Csv <path>` writes a flat file, so the before/after can be
  diffed. Run it before and after each batch.
- `target/qw/shots.ps1 -exe <path> -into <name>` — every screen (tree, files, advisor, cleaners,
  about, settings, search, galaxy) in **dark and light**, into `target/qw/shots/<name>`. It writes
  `theme=1` or `theme=2` into a throwaway `APPDATA` so neither run follows this PC's setting.
  (`dark-cleaners` currently catches the app still reading its folders: give that screen more
  `wait` steps before using it as a baseline.)
- `target/qw/diff-shots.ps1 -a <folder> -b <folder>` — a pixel compare of two such folders.
  **Read this before trusting it:** run-to-run noise with an identical binary is 0.05 % to 1 % on
  every screen (the scan time reads 0.03 or 0.04 s, free space ticks over, the map seeds
  differently), and `dark-advisor` can differ by 90 % because it catches a different frame. The
  diff proves the screens still render; it cannot prove pixel equality. For that, use the static
  test in `tokens.rs`.

### Parts 3 to 5

Not started. `plan-polish.md` has them in full: the Sprawl look's values, the Theme page and its
21 options, and the checks. Part 4's storage question (one `look.*` key per option in
`settings.ini`, and whether the open window can take a new look without a restart) is still open
and the plan says not to promise the no-restart path before it is seen working.

## Two decisions only the user can make

Both are from installer P3-10 and are deliberately not implemented.

1. **The release signing key.** `%USERPROFILE%\.sprawl-release\signing.key` is 64 hex characters
   in a file with inherited ACLs, and `signing-backup.key` is beside it, so one theft takes both.
   The cheap half (tighten the ACL, move the backup offline) costs nothing and should happen
   whatever else is decided. Beyond that: DPAPI stops the file working elsewhere but not a
   process running as the user; a passphrase (Argon2id → XChaCha20-Poly1305) is the one that
   survives a file-grabbing infostealer; a token or a separate signing machine is the full answer
   and is probably more than this project needs today.
2. **Authenticode for `Sprawl-Setup-<v>.exe`.** Updates are already covered by Sprawl's own
   Ed25519 manifest chain; what is not covered is the first install, where Windows says "Unknown
   publisher". An OV certificate or Azure Trusted Signing (~$10/month) removes that and lets
   SmartScreen reputation accrue to the certificate instead of to each new file. If it is bought,
   the signing step goes in `sprawl-release::build()` **after** ISCC and **before** the Setup is
   hashed for the manifest.

## Update (same day, later session): items 1 and 2 done, not committed

Every `FontSize` and `CornerRadius` is now a by-value token (`fs8`..`fs30`, `fs12_5`, `rc2`..`rc14`);
`look.rs` setters go through `tokens::fs()` / `tokens::rc()`, which fall back to the bare number when no
token has that value. New tests: `no_font_size_or_corner_radius_is_written_as_a_number`,
`a_size_from_rust_uses_its_token`. Still runtime numbers, not tokens: `ui::markdown`'s `FontSize="{size}"`
and one `FontSize="{}"` in `reconcile.rs`; the Text size option must scale those in Rust. 112 app tests green;
screens checked with `shots.ps1` (`target/qw/shots/tokens-sizes` against `tok-b2b`): differences are noise only.

Item 4, first half, also done (not committed): every token has its own `light` value, equal to what
`theme::flip` gave at each site (proved by `tokens` test walking every site). `state::load` now flips the
hand-written literals first and expands tokens after; `reconcile` diffs the token text and `set` resolves a
token through `tokens::resolve`. `warnmark` / `goodmark` are the fill versions of warn / good. Light screens
match the previous run within noise (`target/qw/shots/tokens-light`). Still to do: the raw-colour test, which
waits for the colour long tail (Part 3).

## Update 3 (2026-11-21, evening): Part 3 under way, session stopped for an MCP restart

Committed: d1c7d88 (Ink pass 1: every colour a token, opaque ground, popup panels, accent bars).

Open work, none of it merged:
1. **Main checkout, uncommitted** (Flash): `look.rs` `stock()` = 54 WinUI lightweight-styling brush keys +
   ControlCornerRadius 4 / OverlayCornerRadius 8 + TextControl thicknesses; `tokens.rs` adds computed `accent` /
   `onaccent` tokens, `tokens::get()` for element content, and corner values rc5/rc6 -> 4, rc7/rc9 -> 8.
   Flash reported 119 tests green and a release build; Opus has NOT reviewed or re-run it yet. Review the
   diff, run `cargo test -p sprawl -j 1`, screenshot dark and light (`target/qw/shots.ps1`), then commit.
2. **Branch `worktree-agent-ad283dac3cda264f2`, commit e00658f** (Sonnet, Part 3.6): `ui::note` / `note_with` /
   `note_text` = explanation text at 13 px text2 with semibold text1 lead words; used by every Settings note
   (`wizard.rs` `help()`), the AI step paragraphs, the clean-up bar explanation and Search's empty-state intro.
   **Based on 6d0e70b**, so merging into main conflicts with the token commits: resolve to tokens, and change
   the literal `FontSize="13"` in `note_with` to `{$fs13}`. Not yet seen on screen: check no clipping.
3. **Branch `worktree-agent-ae6ed21a38b28d968`, commit 7b700ff** (Sonnet, Part 3.4, finished): `look.rs` only.
   `template_with` states are storyboards (Normal reset; hover wash 0.10, Content up 1 px; pressed wash 0.20,
   down 1 px, scale 0.98), ExponentialEase EaseOut 6, 140 ms, 0 s when `win32::animations_on()` is false (read
   once at start). The transform is on Content only (arrow and tab underline stay put). `SprawlChip` corner is a
   literal "999" (pills): add a pill corner token at merge. Shadow skipped. 110 tests green on its base.
   **Based on 6d0e70b**: merge after item 1, resolving `look.rs` against the token commits and `stock()`.

Still to do in Part 3 after that: noticed in screenshots, white % text on a light accent bar reads poorly;
the focus ring (accent outline + 3 px ring) needs a template; tick drawing / switch slide motion. Then Part 4.
