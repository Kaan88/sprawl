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
