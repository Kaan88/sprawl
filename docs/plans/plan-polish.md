# Plan: polish (for Opus), 2026-11-21

Written by Fable from the user's answers and picks. It replaces the twelve button directions
(`docs/mockups/buttons-2026-11-21`, deleted at the user's word) and "direction 2" as a target.
What direction 2 left in `crates/sprawl/src/look.rs` is the code to start from, not the goal.
The user's taste is in the memory file `feedback-polish-baseline.md`: read it first.

Work in this order. Commit, push and release only on the user's word. Use `-j 1`. Never run
`cargo fmt`. Add standalone entries to `.claude/fable-review-queue.md` where the safety gate asks.

## Part 1: the row flash and panel blink (first, alone)

Do `docs/plans/handoff-polish-fix.md`, steps 1 to 8. No look change in this part: it must be
judged on feel alone. Done when the user has clicked a test build and the rows no longer flash.

## The look the user picked (the "Sprawl" preset)

Picked on 2026-11-21 in `docs/mockups/polish-2026-11-21/index.html` (the sampler). Its CSS holds
every value below; `theme-page.html` beside it shows them in light too.

| What | Pick | Meaning in the app |
| --- | --- | --- |
| Accent | Windows accent | Keep `look::accent()`. All bars (% of parent, drive, file types) take the accent: no blue, purple or pink gradients of their own. A selected row is the accent mixed 26% into the card colour. |
| Surfaces | Ink | One cool blue-black family. Dark: base `#11131A`, card `#191C26`, row `#212533`, popup `#1E2230`, hairline `#A0B4FF` at 12%, text `#EEF1F8` / `#B9C2D6` / `#8791A8`. Light: base `#E9ECF4`, card `#F6F7FC`, row `#FFFFFF`, popup `#FFFFFF`, hairline `#1B2A60` at 15%, text `#141824` / `#3A4258` / `#5B647A`. No neutral grey panes beside navy panels. |
| Corners | 4 / 8 | Controls, rows, tick boxes 4; cards, panes, dialogs, popups 8; bars 2. Today 11 sizes (2 to 14) are written into the XAML. |
| Chips | Pills | Round ends. Picked: accent fill, on-accent text, semibold. Not picked: 6% wash, 10% hairline. For Findings / Growth / Remembered, Sort, Show, Search's chips and the map modes. |
| Press feedback | Lift and sink | Hover: 10% wash, up 1 px, shadow `0 6px 14px` (the primary button's shadow tinted with the accent). Pressed: 20% wash, down 1 px, scale 0.98, no shadow. 140 ms, `cubic-bezier(.2,.8,.2,1)`. Rows lift 1 px. One behaviour for every button, chip, tab and row. |
| Motion | Subtle | 160 ms, same curve: the tab line slides, menus fade in 5 px, the tick draws, the switch slides. None when Windows' animations are off (the app already reads that setting: commit 0336b42). |
| Finish | Soft depth | A light 1 px edge on top of cards, buttons and rows; a soft shadow under cards; an inner highlight and an accent-tinted shadow on the primary button. |
| Controls | Outlined | Text boxes and dropdowns: 1 px outline, darker fill; in use, accent outline plus a 3 px ring of the accent at 32%. Tick box: outline, 4 px corners, accent fill, drawn tick. Switch: filled track, white knob. Menus, flyouts and dialogs in the popup colour with 8 px corners. |
| Small text | 13 px, first words in white | Explanations only (clean-up bar, Search hints, Settings notes): 13 px in the second text colour, the opening words semibold in the first. Table cells and labels keep their size. |
| Light mode | Both together | Every screen is checked in dark and light before it counts as done. |

Unchanged, at the user's word: the layout and flow, the File / Options / Help menu's place, the
three header boxes, and the density (a little air only where no information is lost).

## Part 2: one place for the look (no visible change yet)

Today about 120 colour values, 11 corner sizes and 15 text sizes are written by hand inside
XAML strings (`ui.rs`, `advisor.rs`, `cleaners.rs`, `cleanup.rs`, `search*.rs`, `wizard.rs`,
`tables.rs`, `welcome.rs`), and `theme.rs` guesses the light colours by flipping lightness when
a string is loaded.

1. Give `look.rs` a `Look` (one value per option of Part 4) and a token table made from it.
2. XAML strings name tokens (for example `Background="$card"`, `CornerRadius="$rc"`,
   `Foreground="$text2"`). `state::load` (the one place every string passes, where
   `theme::themed` runs now) puts the values in. `reconcile.rs` compares strings before that,
   so it is unaffected.
3. Move file by file. First with token values equal to today's, so before and after screenshots
   match: that proves the move changed nothing. A script that lists the literals with counts
   gives the map (`#FF8E97AA`, `#FFB9C2D6` and `#0CFFFFFF` alone are about 250 uses).
4. Real light values replace `theme::to_light`'s flipping. A test fails on any raw
   `#AARRGGBB` or numeric `CornerRadius` left outside `look.rs` (allow the map's own colours,
   the file-type colours and the risk colours).

## Part 3: the Sprawl look

1. Switch the token values to the table above, dark and light.
2. The window sits on `DesktopAcrylicBackdrop` (`main.rs`, about line 236) with white washes
   over it. Ink needs its own ground: an opaque or near-opaque base, as the light page already
   has. Check an inactive window and a remote session.
3. Stock controls: use WinUI's lightweight styling first (override its resource keys in the
   merged dictionary: `ControlCornerRadius`, `OverlayCornerRadius`, the `TextControl*`,
   `ComboBox*`, `CheckBox*`, `ToggleSwitch*`, `Button*` and `AccentButton*` families).
   Write a template only where that cannot reach the look.
4. `look.rs`'s own templates (`template_with`): pills for chips; the hover and pressed states
   as storyboards from the tokens instead of opacity setters; the lift needs a translation and
   a shadow. Buttons, chips, tabs and rows share one set of states.
5. Bars: `tables.rs` `pct_bar` and the five gradient brushes in `ui.rs` take the accent.
6. Explanations: one helper for a note with lead words, used wherever 12 px grey sentences are.

## Part 4: the Theme page (final list, 2026-11-21)

The user saw the mockup (`docs/mockups/polish-2026-11-21/theme-page.html`), asked what else
could be an option, and said to add as much as is sound. This is that list. The mockup shows the
first ten options and how the page works; the rest follow the same pattern.

### The page

- A new "Theme" section in Settings (`wizard.rs`) after General. The Theme dropdown that is in
  General now (Like Windows / Light / Dark) moves there as "Light or dark".
- Layout B of the mockup, settled: a sample card on top that follows each pick at once, then
  the presets, then a Customise button. (No need to ask the user about A or B.)
- Customise opens four groups as underline tabs inside the page: Colour, Shape, Feel, Text and
  space. Each holds at most eight labelled dropdown rows in two columns, so the page fits the
  Settings window at its default size. The user's standing rule: no chip rows, nothing clipped,
  no scrollbar; screenshot each group closed and with a dropdown open.
- Presets: Sprawl (the picks above), Windows (Graphite, rectangles, clear fade, flat, stock
  controls, 13 px), Midnight (Dark, Black, Iris), Paper (Light, Graphite, flat), and Custom
  (the user's own picks, kept when they try a preset and come back). A preset is only a set of
  option values.
- Copy theme and Paste theme: the options as one line of text. On paste accept only known keys
  with known values and ignore the rest; it is outside text, so never more than that.
- "Back to Sprawl's own" resets everything.

### The options (the first choice is Sprawl's)

| Group | Option | Choices | Tier |
| --- | --- | --- | --- |
| Colour | Light or dark | Like Windows, Dark, Light | 1 |
| Colour | Accent | Windows accent, Azure, Iris, Violet, Sunset, Custom colour | 1 (Custom: 3) |
| Colour | Accent strength | As Windows gives it, Stronger | 2 |
| Colour | Surfaces | Ink, Graphite, Tinted, Black, Slate, Warm grey | 1 |
| Colour | Backdrop | Solid, Mica, Acrylic | 1 |
| Colour | Contrast | Normal, Strong | 1 |
| Colour | Risk colours | Standard, Colour-blind safe | 2 |
| Colour | Folder icons | Classic yellow, Accent, Grey | 1 |
| Shape | Corners | 4 / 8, 6 / 10, 8 / 14, Pills, Square | 1 |
| Shape | Chips | Pills, Soft pills, Rectangles | 2 |
| Shape | Tabs | Underline, Pills | 2 |
| Shape | Controls | Outlined, Soft filled, Windows stock | 2 |
| Shape | Tick boxes | Square, Round | 1 |
| Shape | Bars | Solid, Gradient, Thin line | 1 |
| Shape | Selection | Fill, Accent bar, Both | 2 |
| Feel | Press feedback | Lift and sink, Press-in, Clear fade, Faint | 2 |
| Feel | Motion | Subtle, Lively, None | 1 |
| Feel | Finish | Soft depth, Flat | 2 |
| Text and space | Small text | 13 px lead words, 13 px, 12 px grey | 1 |
| Text and space | Text size | 100%, 110%, 125% | 3 |
| Text and space | Density | Compact (today's), Comfortable, Roomy | 3 |

What each new one means:

- **Accent strength.** The user kept the Windows accent but said it looks weak. "Stronger"
  takes the shade of the Windows palette (or shifts a custom colour) that stands out most from
  the surface while its own text still reads.
- **Custom colour.** WinUI's ColorPicker in a flyout. Whatever is picked, Sprawl works out the
  text colour on it (black or white, whichever reads better) and the accent-coloured text on the
  page (lightened or darkened to reach 4.5 to 1 against the surface). The same two rules apply
  to every accent, so no pick can make a button unreadable.
- **Surfaces.** Black is true black in dark and white in light. Slate is a lighter blue-grey
  than Ink. Warm grey leans brown. Each needs its dark and light values.
- **Backdrop.** Solid is an opaque ground. Mica and Acrylic let the desktop show through a
  slightly see-through ground (`MicaBackdrop` and `DesktopAcrylicBackdrop` are both in the
  bindings).
- **Contrast.** Strong makes hairlines and the second and third text colours firmer. It is a
  token change, not support for Windows' High Contrast themes (accessibility is a later piece
  of work, by the audit's decision).
- **Risk colours.** The safe set replaces green, amber, orange and red with colours that stay
  apart for colour-blind users, and gives each risk its own shape (dot, triangle, square,
  diamond), so colour is never the only sign. This touches safety wording nowhere, but the risk
  marks are what the user reads before a clean-up: add it to the review queue.
- **Folder icons.** Yellow is one of the "too many colours". Sprawl's preset keeps classic
  yellow; Accent and Grey are calmer.
- **Tabs.** Pills reuse the chip look.
- **Bars.** Solid is the accent; Gradient runs from the accent to a lighter shade of it; Thin
  line is a 4 px bar under the figure.
- **Selection.** Accent bar is a 3 px rounded line at the left of the selected row, as Files
  and Windows 11 lists have.
- **Text size and Density.** Both scale the tables, whose rows are laid out by hand in
  `tables.rs` (row height, `y_of`, hit tests): the heights must come from one value. Text size
  scales every font size token; Density scales row height and padding. Compact is today's.

### Left out, and why

- **Segmented chips, Glass, Glow.** Each costs more than the rest together. Ask the user
  before building any of them.
- **A Layout page** (compact header, map beside the tables). The user said the layout is fine,
  and it is the largest piece.
- **The map following the theme.** The map has its own colours and its own Tweaks, and it
  stays dark in light mode on purpose.
- **Sliders** for strength or speed, **a corner slider**, **outline or filled icons**, **the %
  inside or beside the bar**, **Outline and Underline chips**. Many more looks to check for
  little gain.

### Order

1. Tier 1: values in the table of the look. Build all of them first.
2. Tier 2: states, templates or a little logic. Presets and Copy / Paste theme come here.
3. Tier 3: what reaches the hand-laid tables (Text size, Density) and the colour picker.

### How it is stored and applied

- One `look.*` key per option in `settings.ini`; a missing key means the preset's value.
  Custom is stored apart.
- The sample follows at once. On Save the open window should take the look without a restart:
  rebuild the tokens and resources and load the window's XAML again. `State` keeps references
  to live elements, so check that first. If it cannot be done safely, the look takes effect at
  the next start, with a "Restart Sprawl" button, and the page says so. Do not promise the
  first before it is seen working.

### Checking this many options

- Not every combination: each choice of each option against the Sprawl preset, in dark and in
  light. That is about 120 pictures, so make it a script.
- Add a dev override for screenshot runs, `SPRAWL_LOOK=key=value;key=value`, read where the
  settings are (screenshot runs are read-only and use a throwaway profile).
- A test over every preset and every single-option change: the text on the accent, the accent
  text on the surface and the three text colours on each surface all reach their contrast.

## Part 5: checks, each part

- Screenshots by the recipe in the memory file `reference-dev-tools.md`: every tab and Settings,
  dark and light, default size, before and after.
- A test build for the user to click. An approved HTML sheet has disappointed in the real app
  once (direction 2), and press feel cannot be judged from pictures.
- `cargo test --workspace -j 1`. `look.rs`'s tests assert today's style list: update them.
- CHANGELOG under Unreleased.
