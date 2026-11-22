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

## Update 4: items 1-3 of Update 3 are done

8851302 stock controls + 4/8 corners; b127bae merge of press feedback (pill token `rcpill`); 3ee6bd6 merge of
the explanation helper (`ui::note`, fs13). 119 app tests green; screenshots in `target/qw/shots/ink-2`.
Waiting on the user clicking a test build (hover / press feel cannot be judged from pictures). Part 3 left:
white % text on a light accent bar, focus ring template, tick / switch motion, soft shadow. Then Part 4.
The two `.claude/worktrees/agent-*` worktrees are merged and can be removed.

## Update 5 (2026-11-22)

- Merged: instant light/dark (restyle.rs), first-launch speed (d05ebca), Search multi-select + Delete + Show in Explorer fix, Delete for other-drive and saved-scan rows (cd9e774), Theme page in Settings (6fa6d69). 516 workspace tests pass; release build in target/alt2.
- Theme page: presets Sprawl/Windows/Midnight/Paper/Custom, Customise tabs Colour/Shape/Feel/Text and space, Copy/Paste theme, Back to Sprawl's own; stored as `look.<key>` + `look.custom` in settings.ini; Save applies live.
- Not pushed. Waiting on the user's click test (hover/press, Theme page, Search delete on throwaway folders). In the light dropdown screenshot the open popup has a black frame; check whether it shows in the real app or only in the capture.
- Still open: Part 3 tick/switch motion, soft shadow, field focus ring; Part 4 tier 2/3 options (custom accent, strength, backdrop, risk colours via Fable queue, chips, tabs, density...); Part 5 checks and CHANGELOG pass; clean up merged worktrees.

## Update 6 (2026-11-22, paused here)

**Start here, in this order:**
1. Find how bindings are generated (crates/bindgen, tools/setup.ps1; details below).
2. Composition shadows (agent): real `DropShadow`s, no imitations.
3. Review at zoom against the mockup, dark and light.
4. Merge, test, relaunch.

Main is at 0f19180 plus this note; nothing pushed. Merged since Update 5: Theme options always shown
and preset buttons fixed; menus/lists/tooltips tinted (token `menu`), dialogs not; every accent
follows the chosen one (`look::accent()` reads tokens, `windows_accent()` is the registry);
theme picks apply live with Settings open (restyle::preview / end_preview, Cancel restores);
Part 3 soft finish (focus ring via textbox.xaml, tab line fade); Theme tier 2 + Backdrop (9
options, three dropdowns per row); Stronger = most vivid shade (706d9a7); primary button states
fixed (3b070c6). Risk colours queued for Fable in .claude/fable-review-queue.md.

STANDING since today: no shortcuts (memory feedback-no-shortcuts.md). The audit list of earlier
shortcuts is in docs/plans/plan-polish.md, "Audit: shortcuts taken in Parts 3 and 4".

**In progress, decided by the user: real composition shadows.** The shadows on main are still the
banded stacked borders (`look::shadow`, the primary button's `Glow`). Findings:
- ThemeShadow written in XAML draws nothing (needs Receivers, settable only from code).
- The real thing: Microsoft.UI.Composition `DropShadow` (Color, BlurRadius, Offset, Opacity) on a
  `SpriteVisual` set with `ElementCompositionPreview::SetElementChildVisual`, masked to rounded
  corners (CompositionMaskBrush / element alpha), hover opacity animated in code. Match the
  mockup's CSS exactly (docs/mockups/polish-2026-11-21/index.html lines ~118-122, 206-212).
- Needs new bindings: the generator is crates/bindgen (src/main.rs), metadata packages from
  tools/setup.ps1 (Microsoft.UI.winmd holds Microsoft.UI.Composition; Microsoft.UI.Xaml.winmd
  holds Microsoft.UI.Xaml.Hosting). Add Compositor, DropShadow, SpriteVisual, CompositionBrush,
  CompositionMaskBrush, ElementCompositionPreview to the filter and regenerate bindings.rs.
- Branch `worktree-agent-abc547e8ebf995eda` (5c394f4, NOT merged): gradient 9-slice shadows
  (reject: an imitation, grey halo, no accent offset, a notch at Save's bottom-right corner) plus
  parts worth keeping after review: the mockup's press feel (black press wash `washdown`, exact
  timings/easing via key splines, sink overshoot), top highlights (tokens hlcard/hlbtn/hlpri/hlon/
  hlbar, style SprawlMode), Flat dropping everything. Take those, drop `soft`/`soft_xaml`/the
  gradient `shadow` and tokens shd*/glw*, then build the composition shadows on top.
- Review bar: zoomed crops vs the mockup render, dark and light; no banding, accent-tinted
  primary shadow offset below, no notches. Press feel must be clicked by hand.

Then: the rest of the audit list, tier 3 (text size, density, custom accent), Part 5, 1.0.6.
Merged worktrees under .claude/worktrees/ can be cleaned up.

## Update 7 (2026-11-22, overnight; the user asleep, told me to decide and note)

Done: real composition shadows, e014c89, fast-forwarded into main (local, not pushed).
- crates/sprawl/src/shade.rs: per host a SpriteVisual with only a DropShadow, masked by a
  rounded-rectangle ShapeVisual surface, clipped to outside the item (D2D path through an
  IGeometrySource2DInterop written in Rust). Hosts are empty Borders tagged `shade:<kind>`
  (look::shade_host); state::load walks new XAML and attaches on first SizeChanged.
- Plain buttons get a copy of WinUI's Button template (button.xaml) with the host inside;
  shadows follow CommonStates (lift on hover, none on press) with Composition animations.
- Kept from 5c394f4: press wash, exact timings, sink overshoot, top highlights, Flat drops all.
  Dropped: gradient/9-slice shadow, Glow grid, shd*/glw* tokens.

Decisions I made (reasons):
1. Kept the Opus agent on it after the delegation change: risky and important, fits the rule.
2. Light-mode shadow colour is a soft navy (token `shadow`, #8C1B2A60), the agent's choice: the
   mockup is dark only; I checked it at 4x and it reads finished, not grey.
3. Composition blur = 2 x CSS blur: measured by the agent against Edge's render of the mockup CSS.
4. Accepted without a fix: a live accent change reaches shadows only on the next page rebuild;
   buttons in flyouts/popups get no shadow; the Settings dialog's shadow is faint on dark.
   None is a visible flaw in what I reviewed; listed here for the audit.
5. Merged after my review (diff read, crops at 4x dark and light, Flat compared). Not pushed.

Not verified: hover and press feel (needs your clicks). Test exe: target\alt\release\sprawl.exe.
Next, unchanged: the rest of the audit list in plan-polish.md, tier 3, Part 5, 1.0.6.

## Update 8 (2026-11-22, audit list)

Merged on main (local, not pushed): Press-in crash fix; sliding tab line (tabline.rs) on main
and Theme tabs; drawn tick and switch knob; focus rings on ComboBox/PasswordBox/TextBox; menu
open motion for app MenuFlyouts and the ComboBox list; mockup shadow on dropdowns, flyouts and
dialogs; Settings card and dialogs cap to the window (a3c5cf8); smallest window 1000 x 600 at
every scale (eac6fc1); rebuilt menu bar (e8d76fa).

Menu bar (menubar.rs): File, Options, Help are title buttons with plain Flyouts holding our own
rows (copies of WinUI's item, check, radio, sub-item templates), so they fade in with the menu
shadow. Keyboard, access keys, accelerators, hover switching, sub-menu delay, UIA roles and focus
return are reimplemented. A Fable review found 7 defects (Settings read the old rows, UIA toggle
ran no command, Alt latch, clipped hit-testing, nested pass-through, rest timer, Right on a dot);
all fixed. Decisions: F10 stays Zoom in Treemap (Alt alone focuses the bar); rows' parent is a
Pane in UIA; sub-menus with shortcuts are ~19 px wider than WinUI's.

Known, not fixed: Settings takes no focus when it opens (focus stays on the tree behind it,
also via Ctrl+Shift+S). Screenshot runs exit with 0xC000041D after writing the image; 1.0.5
does the same, so not a regression. Dropdown lists keep Windows' placement over the field.

Needs hands: hover/press feel for all press styles; tab-line slide and rapid switching; tick and
knob motion; focus rings; menus by real mouse (title hover switch, sub-menu delay, click outside,
click on a parent row with a sub-menu open, first click with two sub-menu levels open, narrow
padding), Alt tap and Alt+F/O/H, Alt+Tab away and back, Narrator on rows, a non-100% DPI, a short
window where File scrolls; animations off; dropdown black box; Theme sub-tab slide; Search
delete; theme Cancel/Save; Mica/Acrylic over a light desktop.

Open choice for the user: convert the right-click menus the same way.
Next: tier 3 (text size, density, custom accent), Part 5, 1.0.6.
