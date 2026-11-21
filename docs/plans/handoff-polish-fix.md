# Hand-off: the Advisor row flash and panel blink (2026-11-21)

Fable stopped part way at the user's word ("too many tokens, leave the work to Opus"). Nothing
is committed. The polish baseline is in memory (`feedback-polish-baseline.md`).

## The bug

In Advisor findings and the Cleaners list, a row's highlight plays again when the mouse moves
after a click, and the panel blinks. Cause: `render_advisor` (`crates/sprawl/src/advisor.rs`,
about line 1907) builds the whole panel as one XAML string and loads it again on every click.

- A tick (`AdvT{i}` in `wire_advisor`, about line 2168) calls `cleanup_toggle` then
  `render_advisor`: every control is replaced, the one under the pointer too.
- Opening a row (`advisor_open` -> `advisor_rows` -> `advisor_row_swap`, lines 1808-1879) loads
  that row again, so its button is replaced.
- Cleaners rows go through `advisor_indexed` -> `cleaner_command` -> `render_advisor`.

This is also the UI lag of `plan-1.0.5.md` section J.

## The fix chosen

Compare the new XAML string with the one on screen and change only what differs, so a control
that stays keeps its pointer and focus state. Written in `crates/sprawl/src/reconcile.rs`: it
compiles and its 10 tests pass. The half that touches live controls (`Slot`, `apply`, `set`,
`restate`) compiles but has **never run against a window**, and nothing calls it yet.

- `Doc::parse` reads the XAML the app writes; `diff(old, new)` gives ops (`Set` an attribute,
  `Replace` / `Insert` / `Remove` a child) addressed by child-index path; `None` means load whole.
- `Slot::show(host, xaml, in_place)` applies them to the live tree, or loads whole (first time,
  another root, any failure). It returns the pieces it loaded and the names inside each.
- After every update it sets `IsChecked` / `IsOn` / `SelectedIndex` again from the string, so a
  tick box always shows the model.
- `Slot::find(name)` finds an element by its place (a piece loaded later has its own namescope,
  so `FindName` from the panel root misses it). `Slot::text(name, value)` is for status lines
  set from code.
- Its tests check every case by making the ops on a model of the old tree and comparing with
  the new one, including 7000 random trees.

Also done:

- `crates/bindgen/types.txt` has 7 new entries and `bindings.rs` is regenerated
  (`cargo run --offline -j 1 -p sprawl-bindgen`). With the old list the generator reproduces
  the committed file exactly, so the diff is only the additions. It compiles.
- `mod reconcile;` is **not** in `main.rs` (taken out again so the tree builds as before). Add
  it after `mod look;`.

## Left to do, in order

1. Add `mod reconcile;`, then
   `CARGO_TARGET_DIR=target/alt cargo test --offline --release -j 1 -p sprawl reconcile`
   (passed on 2026-11-21, about 50 s; only "never used" warnings until step 2 calls it).
2. `render_advisor`: three slots in the advisor state (AdvHost, ClHost, AdvFoot); replace
   `load` + `SetChild` with `slot.show`. Do the after-load work per loaded piece: wiring, the
   AdvGoal / AdvChatText / AdvAiKey restores and their KeyDown hooks. Restore scroll places only
   when the load was whole.
3. `wire_advisor` (line 2067) probes every possible name. Make it name-driven: for each name in
   a loaded piece, an exact match in the static list calls `advisor_command`; otherwise split
   the trailing digits and dispatch (`AdvF`, `AdvUnc`, `AdvUncF`, `AdvU`, `AdvSgT`, `AdvAiT`,
   `AdvT`, `AdvAlg`, and the `counts` kinds to `advisor_indexed`).
4. `advisor_rows` / `advisor_row_swap`: go through `render_advisor`. The direct swap would leave
   the screen different from the slot's stored string.
5. About 35 lookups use `find(panel or foot, name)` (grep `advisor.panel`, `advisor.foot`):
   use `slot.find`. The `text(p, ...)` sets of `AdvAiStatus`, `AdvChatStatus`, `AdvChatLive`,
   `AdvDupesStatus`, `ClReading`, `AdvCS{i}` and `adv_say` (`AdvMsg`): use `slot.text`.
6. `look::pick` (`look.rs` line 131) writes `Foreground` only for a picked chip. Write it always
   for `Chip` and `Mode` (`REST_TEXT` when off), or a chip is loaded again when picked.
7. Safety: load the foot whole while `cleanup.review`, `running` or `done` is set, so the review
   is never a patched view. Add a standalone entry to `.claude/fable-review-queue.md`.
8. Build, `cargo test --workspace`, then check in the real app (fixture `target\qw\data`, a
   throwaway profile) that a row keeps its highlight after a tick and after opening it.

## The sampler

`docs/mockups/polish-2026-11-21/index.html` is finished: the preview stays put, the picks are in
a column beside it. The user's picks from it are still to come. `.claude/launch.json` has a
`polish-sampler` entry (port 8793) to serve it.
