//! One place for the look. A piece of XAML writes `Foreground="{$text2}"` instead of a colour
//! of its own, and [`expand`] puts the value in on the way to [`crate::state::load`].
//!
//! Only a whole attribute value is a token: `="{$name}"`, with `name` one of the table below.
//! That is deliberate. Every piece of text Sprawl shows (a file name, a model's answer) goes
//! through [`crate::ui::esc`], which writes `{}` in front of anything starting with `{`, so no
//! text of the user's can ever look like a token. A value made of several parts is one token
//! holding all of it (`Margin="{$rowpad}"`), never a token inside a longer value.
//!
//! **The values here are today's, to the digit.** This part of the work moves the values, it
//! does not change them: before and after must look the same, pixel for pixel. A colour token
//! holds both pages' values: `value` for the dark page, and `light` exactly what
//! [`crate::theme::to_light`] used to make of it, so a token no longer has to be turned around
//! on its way out. The page is still written for dark; a colour written into it by hand is
//! still turned around by `to_light` when the window is light, and the tokens are put in
//! afterwards, already holding the value for the theme in use. Today there is no opaque ground:
//! the window sits on an acrylic backdrop and every panel is a white wash over it, which is
//! why the surface tokens are washes and not colours.

use std::borrow::Cow;
use std::collections::HashMap;

/// Every option of the Theme page, with the values Sprawl's own look has. One field per option;
/// the table is made from it. The first variant of every option is Sprawl's own, so the default
/// `Look` is the look the app has today.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Look {
    pub accent: Accent,
    pub surfaces: Surfaces,
    pub contrast: Contrast,
    pub folders: Folders,
    pub corners: Corners,
    pub motion: Motion,
    pub bars: Bars,
    pub small_text: SmallText,
    pub ticks: Ticks,
    pub text: TextSizes,
}

/// The accent: Windows' own, or one of the presets the Theme page offers.
#[allow(dead_code)] // The Theme page will read every option; until it exists, the tests are the only caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Accent {
    /// The accent Windows is set to (read by [`crate::look::accent`]).
    #[default]
    Windows,
    Azure,
    Iris,
    Violet,
    Sunset,
}

/// The page's surfaces: an opaque ground, a card above it, a row inside the card and a popup
/// over it, with their hairlines and text.
#[allow(dead_code)] // The Theme page will read every option; until it exists, the tests are the only caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Surfaces {
    /// Today's: a near-black page with its own opaque ground, and a light page made from white.
    #[default]
    Ink,
    /// Neutral greys, from #1C1C1C on the dark page.
    Graphite,
    /// Ink with a breath of the accent in each surface.
    Tinted,
    /// True black in dark, true white in light.
    Black,
    /// A blue-grey, lighter than Ink.
    Slate,
    /// A brown-leaning grey.
    WarmGrey,
}

/// How firmly the hairlines and the quieter text are drawn.
#[allow(dead_code)] // The Theme page will read every option; until it exists, the tests are the only caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Contrast {
    #[default]
    Normal,
    /// Firmer hairlines, and the second and third text colours a step closer to the first.
    Strong,
}

/// The colour of a folder icon.
#[allow(dead_code)] // The Theme page will read every option; until it exists, the tests are the only caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Folders {
    /// Today's amber.
    #[default]
    Classic,
    /// The accent, as text on the page.
    Accent,
    /// The quiet text colour.
    Grey,
}

/// How round every corner is, as three sizes: a bar, a control, a card.
#[allow(dead_code)] // The Theme page will read every option; until it exists, the tests are the only caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Corners {
    /// Today's: 2 for a bar, 4 for a control, 8 for a card.
    #[default]
    Four8,
    Six10,
    Eight14,
    Pills,
    Square,
}

/// How the buttons move: how long a state takes, and how far its content travels.
#[allow(dead_code)] // The Theme page will read every option; until it exists, the tests are the only caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Motion {
    /// Today's: 140 ms, a 1 px lift, a press at 0.98.
    #[default]
    Subtle,
    /// 220 ms, a 2 px lift, a press at 0.96.
    Lively,
    /// No travel and no time: only the wash changes.
    None,
}

/// How a percentage bar or the drive's usage bar is drawn.
#[allow(dead_code)] // The Theme page will read every option; until it exists, the tests are the only caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Bars {
    /// Today's: the accent, washed.
    #[default]
    Solid,
    /// The accent running to a lighter shade of it.
    Gradient,
    /// A 4 px bar at the bottom of its cell.
    Thin,
}

/// How an explanation is set: the size, the colour, and whether its opening words lead.
#[allow(dead_code)] // The Theme page will read every option; until it exists, the tests are the only caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SmallText {
    /// Today's: 13 px in the second text colour, the opening words semibold in the first.
    #[default]
    LeadWords,
    /// 13 px in the second text colour, the lead in the same style.
    Plain,
    /// The old look: 12 px in the labels' colour, no lead.
    Grey12,
}

/// The shape of a tick box.
#[allow(dead_code)] // The Theme page will read every option; until it exists, the tests are the only caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Ticks {
    /// Today's: WinUI's square box.
    #[default]
    Square,
    /// A round box: WinUI's check box takes the control's own corner radius.
    Round,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TextSizes {
    /// Today's sizes, kept as they are.
    #[default]
    AsWritten,
}

/// A token's name, what it stands for, and its value on each page: `value` for dark, `light`
/// for the light page.
pub(crate) struct Token {
    pub name: &'static str,
    /// Read by the Theme page, which has still to be built; until then by its test alone.
    #[allow(dead_code)]
    pub what: &'static str,
    pub value: &'static str,
    /// The same token on the light page, designed with its dark value as a pair. It is not
    /// derived: the two pages are drawn separately, so a light value may be any colour the
    /// light page needs. A size is the same number on both.
    pub light: &'static str,
}

/// The names a piece of XAML may use, with the Ink look's values: a dark, near-black page with
/// its own opaque ground, a card a step above it, and a row inside the card a step above that;
/// a light page made the same way from white. Text runs from `text0` (the brightest) through
/// `text1`, `text2`, `text3` to `faint` (the dimmest hint). Lines are the accent, faintly. Every
/// colour token carries the dark page's value and the light page's; a size token is the same
/// number on both.
pub(crate) const TOKENS: &[Token] = &[
    Token { name: "ground", what: "the window's own ground, under everything", value: "#FF11131A", light: "#FFE9ECF4" },
    Token { name: "card", what: "a card or pane", value: "#FF191C26", light: "#FFF6F7FC" },
    Token { name: "row", what: "a row or field inside a card", value: "#FF212533", light: "#FFFFFFFF" },
    Token { name: "popup", what: "a menu, flyout, dialog or floating panel", value: "#FF1E2230", light: "#FFFFFFFF" },
    Token { name: "surface1", what: "the faintest panel wash", value: "#0AFFFFFF", light: "#0A000000" },
    Token { name: "surface2", what: "a panel or card", value: "#0CFFFFFF", light: "#0C000000" },
    Token { name: "surface3", what: "a card that stands out a little", value: "#0FFFFFFF", light: "#0F000000" },
    Token { name: "hover", what: "a row or button under the pointer", value: "#1AFFFFFF", light: "#141B2A60" },
    Token { name: "pressed", what: "a row or button while pressed", value: "#33FFFFFF", light: "#291B2A60" },
    Token { name: "line1", what: "a hairline between parts", value: "#1FA0B4FF", light: "#261B2A60" },
    Token { name: "line2", what: "a firmer edge", value: "#33A0B4FF", light: "#401B2A60" },
    Token { name: "shade", what: "a dark wash under something raised", value: "#33000000", light: "#33FFFFFF" },
    Token { name: "text1", what: "a heading, or a figure being read", value: "#FFEEF1F8", light: "#FF141824" },
    Token { name: "text2", what: "what is being read", value: "#FFB9C2D6", light: "#FF3A4258" },
    Token { name: "text3", what: "labels, units and hints", value: "#FF8791A8", light: "#FF5B647A" },
    // Written for the tests and for a table built before the accent is read; `table()` computes
    // them from the Windows accent and the card (see `selected`, `accent`, `onaccent`).
    Token { name: "selected", what: "the background of a selected row or tab", value: "#FF133453", light: "#FFB6D6F2" },
    Token { name: "accent", what: "the Windows accent as a fill: a check, a switch turned on", value: "#FF0078D4", light: "#FF0078D4" },
    Token { name: "onaccent", what: "a glyph or text on the accent fill", value: "#FFFFFFFF", light: "#FFFFFFFF" },
    Token { name: "rest", what: "a selectable item's text at rest", value: "#FFEEF1F8", light: "#FF141824" },
    Token { name: "warn", what: "a caution: a held clean-up, a thing to read before pressing", value: "#FFF5B342", light: "#FFB8780A" },
    Token { name: "good", what: "something that went well, or is safe", value: "#FF4CC38A", light: "#FF319163" },
    // A caution or a good mark drawn as a fill (a dot or a bar), not as text: a colour fill is
    // not turned around on the light page, so it has a value of its own there.
    Token { name: "warnmark", what: "a caution colour used as a fill (a dot or bar)", value: "#FFF5B342", light: "#FFF5B342" },
    Token { name: "goodmark", what: "a good colour used as a fill (a dot or bar)", value: "#FF4CC38A", light: "#FF4CC38A" },
    Token { name: "link", what: "a word that can be pressed", value: "#FF8FB4FF", light: "#FF0040C2" },
    // The brighter and dimmer ends of the page's text, and the text that sits on a highlight.
    Token { name: "text0", what: "the brightest text: a table row", value: "#FFFFFFFF", light: "#FF0B0E16" },
    Token { name: "label", what: "a section's label, a legend's words", value: "#FF9AA3B8", light: "#FF4A536A" },
    Token { name: "faint", what: "the dimmest hint: an empty view, a note under a panel", value: "#FF707A90", light: "#FF7F889C" },
    Token { name: "seltext", what: "text on a selected or accented strip", value: "#FFBFD4FF", light: "#FF0040C2" },
    Token { name: "wash", what: "the white the styles wash over a pointer", value: "#FFFFFFFF", light: "#FF000000" },
    Token { name: "keycap", what: "a keyboard key cap's fill", value: "#1FFFFFFF", light: "#1F000000" },
    Token { name: "stripe", what: "every other row of a table", value: "#08FFFFFF", light: "#08000000" },
    Token { name: "edge", what: "a firmer edge: a popup's, a rule between parts", value: "#33A0B4FF", light: "#401B2A60" },
    Token { name: "disabled", what: "text on a greyed filled button", value: "#5CFFFFFF", light: "#5C000000" },
    // The opaque panels that sit on the ink page.
    Token { name: "panel", what: "an opaque card or foot on the page", value: "#FF1E2230", light: "#FFFFFFFF" },
    Token { name: "rowopen", what: "a row that is open", value: "#FF262B3B", light: "#FFEEF1FA" },
    Token { name: "scrim", what: "the dark behind a dialog or the wizard", value: "#B8080A10", light: "#B8EFF1F7" },
    // A colour and its wash and line, as one family: the tint behind a block, the block's edge.
    Token { name: "warnwash", what: "a caution's tint behind a block", value: "#14F5B342", light: "#14F5B342" },
    Token { name: "warnline", what: "a caution's edge", value: "#55F5B342", light: "#55F5B342" },
    Token { name: "warnquiet", what: "a soft caution: a program is open, its files are skipped", value: "#FFB9A36A", light: "#FF84703E" },
    Token { name: "goodwash", what: "a good tint behind a block", value: "#144CC38A", light: "#144CC38A" },
    Token { name: "goodline", what: "a good edge", value: "#554CC38A", light: "#554CC38A" },
    // The orange that warns louder than `warn`, and the red of a thing that failed.
    Token { name: "alert", what: "an orange warning as text", value: "#FFFF7A45", light: "#FFC23700" },
    Token { name: "alertmark", what: "an orange warning as a fill (a bar)", value: "#FFFF7A45", light: "#FFFF7A45" },
    Token { name: "alertline", what: "an orange warning's edge", value: "#55FF7A45", light: "#55FF7A45" },
    Token { name: "alertquiet", what: "the lighter orange of a figure being read", value: "#FFFF9A70", light: "#FFC23900" },
    Token { name: "fail", what: "a thing that failed, or a delete for good", value: "#FFFF6B6B", light: "#FFC20000" },
    Token { name: "danger", what: "the fill of a button that deletes for good", value: "#FFC42B1C", light: "#FFC42B1C" },
    Token { name: "dangerline", what: "the edge of a dialog that warns about a delete", value: "#88C42B1C", light: "#88C42B1C" },
    // The accent's tint and edge, the AI's violet, and the fixed blue the figures are drawn in.
    Token { name: "accentwash", what: "the accent's tint behind a running or active block", value: "#142F7BF6", light: "#142F7BF6" },
    Token { name: "accentline", what: "the accent's edge", value: "#552F7BF6", light: "#552F7BF6" },
    Token { name: "accentchip", what: "the accent as a filled button or chip", value: "#332F7BF6", light: "#332F7BF6" },
    Token { name: "aiwash", what: "the AI's violet tint behind a block", value: "#1AA97BF6", light: "#1AA97BF6" },
    Token { name: "ailine", what: "the AI's violet edge", value: "#55A97BF6", light: "#55A97BF6" },
    Token { name: "aichip", what: "the AI's violet badge, an AI-ticked row", value: "#33A97BF6", light: "#33A97BF6" },
    Token { name: "aitext", what: "the word on the AI's violet badge", value: "#FFC9B4FF", light: "#FF3600C2" },
    Token { name: "accenttext", what: "the fixed accent blue as text: a percentage, a sort arrow", value: "#FF4CC2FF", light: "#FF0080C2" },
    Token { name: "accentmark", what: "the fixed accent blue as a fill (a gauge bar)", value: "#FF4CC2FF", light: "#FF4CC2FF" },
    Token { name: "match", what: "the part of a name a search matched", value: "#FF7FD4FF", light: "#FF0081C2" },
    Token { name: "folder", what: "a folder icon's amber", value: "#FFFFC83D", light: "#FFC28B00" },
    // Text sizes and corner radii are named by their value: `fs12` is the 12 px text the page
    // already sets, `fs12_5` the 12.5, `rc4` a 4 px corner. A text size is what was on the page;
    // a corner's name is the radius it was written with, and the look draws it at the look's
    // size (`rc5` and `rc6` at 4, `rc7` and `rc9` at 8).
    Token { name: "fs8", what: "text size 8", value: "8", light: "8" },
    Token { name: "fs9", what: "text size 9", value: "9", light: "9" },
    Token { name: "fs10", what: "text size 10", value: "10", light: "10" },
    Token { name: "fs11", what: "text size 11", value: "11", light: "11" },
    Token { name: "fs12", what: "text size 12", value: "12", light: "12" },
    Token { name: "fs12_5", what: "text size 12.5", value: "12.5", light: "12.5" },
    Token { name: "fs13", what: "text size 13", value: "13", light: "13" },
    Token { name: "fs14", what: "text size 14", value: "14", light: "14" },
    Token { name: "fs15", what: "text size 15", value: "15", light: "15" },
    Token { name: "fs16", what: "text size 16", value: "16", light: "16" },
    Token { name: "fs17", what: "text size 17", value: "17", light: "17" },
    Token { name: "fs18", what: "text size 18", value: "18", light: "18" },
    Token { name: "fs20", what: "text size 20", value: "20", light: "20" },
    Token { name: "fs24", what: "text size 24", value: "24", light: "24" },
    Token { name: "fs30", what: "text size 30", value: "30", light: "30" },
    Token { name: "rc2", what: "corner radius 2", value: "2", light: "2" },
    Token { name: "rc3", what: "corner radius 3", value: "3", light: "3" },
    Token { name: "rc4", what: "corner radius 4", value: "4", light: "4" },
    Token { name: "rc5", what: "corner radius written as 5, drawn at 4", value: "4", light: "4" },
    Token { name: "rc6", what: "corner radius written as 6, drawn at 4", value: "4", light: "4" },
    Token { name: "rc7", what: "corner radius written as 7, drawn at 8", value: "8", light: "8" },
    Token { name: "rc8", what: "corner radius 8", value: "8", light: "8" },
    Token { name: "rc9", what: "corner radius written as 9, drawn at 8", value: "8", light: "8" },
    Token { name: "rc10", what: "corner radius 10", value: "10", light: "10" },
    Token { name: "rc12", what: "corner radius 12", value: "12", light: "12" },
    Token { name: "rc14", what: "corner radius 14", value: "14", light: "14" },
    // A chip's pill: a radius past half the height, which the control clamps, so the ends are
    // round whatever the chip's height. Not a number of its own, hence the name.
    Token { name: "rcpill", what: "the round ends of a chip: half its height (WinUI makes a larger radius an oval)", value: "15", light: "15" },
];

// ---------------------------------------------------------------- the options, as values

/// A colour token's `#AARRGGBB` as a number.
fn argb(s: &str) -> Option<u32> {
    u32::from_str_radix(s.strip_prefix('#')?, 16).ok()
}

/// The `#AARRGGBB` a colour is written as.
fn hex(c: u32) -> String {
    format!("#{c:08X}")
}

/// A six-digit `#RRGGBB` with 35% white mixed in: the lighter end of an accent's gradient. Six
/// digits on purpose — the light page's rewrite turns only eight-digit literals around.
pub(crate) fn lighter(six: &str) -> String {
    let c = u32::from_str_radix(six.trim_start_matches('#'), 16).unwrap_or(0x00_78D4);
    let at = |shift: u32| (channel(c, shift) * 65 + 0xFF * 35 + 50) / 100;
    format!("#{:02X}{:02X}{:02X}", at(16), at(8), at(0))
}

/// One channel of a colour: 16 red, 8 green, 0 blue.
fn channel(c: u32, shift: u32) -> u32 {
    c >> shift & 0xFF
}

/// A colour's WCAG relative luminance, from its red, green and blue (alpha ignored: every text
/// colour this reads is opaque).
fn luminance(c: u32) -> f64 {
    let linear = |shift: u32| {
        let v = channel(c, shift) as f64 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
}

/// The contrast between two colours as WCAG writes it: 1 at the same colour, 21 at black and
/// white.
fn ratio(a: u32, b: u32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

/// `a` moved `p` per cent of the way to `b`, channel by channel, keeping `a`'s alpha. Whole per
/// cents on purpose: the selected row's 26 is the arithmetic the table has always used.
fn mix(a: u32, b: u32, p: u32) -> u32 {
    let at = |shift: u32| (channel(a, shift) * (100 - p) + channel(b, shift) * p + 50) / 100;
    (a & 0xFF00_0000) | (at(16) << 16) | (at(8) << 8) | at(0)
}

/// A hairline's alpha multiplied, capped so a firmer line stays a line.
fn firmer(c: u32, factor: f64, cap: u32) -> u32 {
    let a = ((c >> 24) as f64 * factor).round().min(cap as f64) as u32;
    (a << 24) | (c & 0x00FF_FFFF)
}

/// A colour's hue in degrees, and its saturation and lightness (0..1).
fn hsl(c: u32) -> (f64, f64, f64) {
    let (r, g, b) = (channel(c, 16) as f64 / 255.0, channel(c, 8) as f64 / 255.0, channel(c, 0) as f64 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, s, l)
}

/// The colour a hue, saturation and lightness are drawn as, with `alpha`.
fn from_hsl(h: f64, s: f64, l: f64, alpha: u32) -> u32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let at = |v: f64| ((v + l - c / 2.0) * 255.0).round().clamp(0.0, 255.0) as u32;
    (alpha << 24) | (at(r) << 16) | (at(g) << 8) | at(b)
}

#[allow(dead_code)] // The Theme page will read the names; until it exists, the tests are the only caller.
impl Accent {
    /// The fill on the page: Windows' own accent (already the page's, read by
    /// [`crate::look::accent`]), or the preset's dark or light pair.
    fn fill(self, light: bool) -> u32 {
        match self {
            // Windows' accent is written with six digits (the light page's rewrite leaves it
            // alone), so it is given the opaque alpha a colour token carries.
            Accent::Windows => {
                let six = crate::look::accent().fill.trim_start_matches('#').to_string();
                u32::from_str_radix(&six, 16).map(|c| c | 0xFF00_0000).unwrap_or(0xFF00_78D4)
            }
            Accent::Azure => if light { 0xFF00_5FB8 } else { 0xFF60_CDFF },
            Accent::Iris => if light { 0xFF4F_46E5 } else { 0xFFA5_B4FC },
            Accent::Violet => if light { 0xFF7C_3AED } else { 0xFFC4_A5FF },
            Accent::Sunset => if light { 0xFFC2_410C } else { 0xFFFF_B38A },
        }
    }

    /// Its name in a copied theme.
    fn name(self) -> &'static str {
        match self {
            Accent::Windows => "windows",
            Accent::Azure => "azure",
            Accent::Iris => "iris",
            Accent::Violet => "violet",
            Accent::Sunset => "sunset",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "windows" => Accent::Windows,
            "azure" => Accent::Azure,
            "iris" => Accent::Iris,
            "violet" => Accent::Violet,
            "sunset" => Accent::Sunset,
            _ => return None,
        })
    }
}

/// Black or white, whichever reads better on `fill`: the one rule every accent is held to, so
/// no pick can make a button's text unreadable.
fn on_accent(fill: u32) -> u32 {
    if ratio(0xFF00_0000, fill) >= ratio(0xFFFF_FFFF, fill) {
        0xFF00_0000
    } else {
        0xFFFF_FFFF
    }
}

/// The accent as text on the page: `fill` when it already reaches 4.5:1 against the card,
/// otherwise its HSL lightness stepped a little at a time towards the page's end (lighter on the
/// dark page, darker on the light) until it does.
fn accent_text(fill: u32, card: u32, light: bool) -> u32 {
    let (h, s, mut l) = hsl(fill);
    let mut c = fill;
    for _ in 0..=100 {
        if ratio(c, card) >= 4.5 {
            return c;
        }
        l = if light { (l - 0.01).max(0.0) } else { (l + 0.01).min(1.0) };
        c = from_hsl(h, s, l, fill >> 24);
    }
    c
}

/// The colours a surfaces family writes down, before the ones derived from them: the page's
/// ground, the card above it, a row inside the card and a popup over it, the hairlines, and the
/// three text colours.
#[derive(Clone, Copy)]
struct Base {
    ground: u32,
    card: u32,
    row: u32,
    popup: u32,
    line1: u32,
    line2: u32,
    text1: u32,
    text2: u32,
    text3: u32,
}

/// Ink, as the tokens have always written it.
fn ink(light: bool) -> Base {
    Base {
        ground: if light { 0xFFE9_ECF4 } else { 0xFF11_131A },
        card: if light { 0xFFF6_F7FC } else { 0xFF19_1C26 },
        row: if light { 0xFFFF_FFFF } else { 0xFF21_2533 },
        popup: if light { 0xFFFF_FFFF } else { 0xFF1E_2230 },
        line1: if light { 0x261B_2A60 } else { 0x1FA0_B4FF },
        line2: if light { 0x401B_2A60 } else { 0x33A0_B4FF },
        text1: if light { 0xFF14_1824 } else { 0xFFEE_F1F8 },
        text2: if light { 0xFF3A_4258 } else { 0xFFB9_C2D6 },
        text3: if light { 0xFF5B_647A } else { 0xFF87_91A8 },
    }
}

/// Neutral greys: hairlines are white on the dark page and black on the light one, and the text
/// runs from #F2F2F2 to #9A9A9A (from #1A1A1A to #616161).
fn graphite(light: bool) -> Base {
    Base {
        ground: if light { 0xFFF3_F3F3 } else { 0xFF1C_1C1C },
        card: if light { 0xFFFB_FBFB } else { 0xFF26_2626 },
        row: if light { 0xFFFF_FFFF } else { 0xFF2E_2E2E },
        popup: if light { 0xFFFF_FFFF } else { 0xFF2B_2B2B },
        line1: if light { 0x1A00_0000 } else { 0x1AFF_FFFF },
        line2: if light { 0x2900_0000 } else { 0x2EFF_FFFF },
        text1: if light { 0xFF1A_1A1A } else { 0xFFF2_F2F2 },
        text2: if light { 0xFF40_4040 } else { 0xFFC5_C5C5 },
        text3: if light { 0xFF61_6161 } else { 0xFF9A_9A9A },
    }
}

/// A family's colours for the page, or None for Ink, whose values are the table's own.
fn surfaces_base(surfaces: Surfaces, light: bool, accent: u32) -> Option<Base> {
    Some(match surfaces {
        Surfaces::Ink => return None,
        Surfaces::Tinted => {
            let b = ink(light);
            Base {
                ground: mix(b.ground, accent, 6),
                card: mix(b.card, accent, 6),
                row: mix(b.row, accent, 6),
                popup: mix(b.popup, accent, 6),
                ..b
            }
        }
        Surfaces::Graphite => graphite(light),
        Surfaces::Black => Base {
            ground: if light { 0xFFFF_FFFF } else { 0xFF00_0000 },
            card: if light { 0xFFFF_FFFF } else { 0xFF0D_0D0F },
            row: if light { 0xFFF5_F6F8 } else { 0xFF16_161A },
            popup: if light { 0xFFFF_FFFF } else { 0xFF14_1417 },
            ..graphite(light)
        },
        Surfaces::Slate => Base {
            ground: if light { 0xFFDD_E3EE } else { 0xFF1B_2030 },
            card: if light { 0xFFED_F1F7 } else { 0xFF23_2A3D },
            row: if light { 0xFFFF_FFFF } else { 0xFF2C_3449 },
            popup: if light { 0xFFFF_FFFF } else { 0xFF28_3045 },
            ..ink(light)
        },
        Surfaces::WarmGrey => Base {
            ground: if light { 0xFFEF_ECE8 } else { 0xFF1A_1816 },
            card: if light { 0xFFF8_F6F3 } else { 0xFF24_211E },
            row: if light { 0xFFFF_FFFF } else { 0xFF2D_2A26 },
            popup: if light { 0xFFFF_FFFF } else { 0xFF2A_2723 },
            line1: if light { 0x243A_2A1A } else { 0x1FFF_F0E0 },
            line2: if light { 0x3D3A_2A1A } else { 0x33FF_F0E0 },
            text1: if light { 0xFF1F_1A15 } else { 0xFFF3_EEE8 },
            text2: if light { 0xFF4A_4239 } else { 0xFFCF_C6BC },
            text3: if light { 0xFF6E_655B } else { 0xFFA3_9A90 },
        },
    })
}

#[allow(dead_code)] // The Theme page will read these; until it exists, the tests are the only caller.
impl Surfaces {
    /// Its name in a copied theme.
    fn name(self) -> &'static str {
        match self {
            Surfaces::Ink => "ink",
            Surfaces::Graphite => "graphite",
            Surfaces::Tinted => "tinted",
            Surfaces::Black => "black",
            Surfaces::Slate => "slate",
            Surfaces::WarmGrey => "warmgrey",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "ink" => Surfaces::Ink,
            "graphite" => Surfaces::Graphite,
            "tinted" => Surfaces::Tinted,
            "black" => Surfaces::Black,
            "slate" => Surfaces::Slate,
            "warmgrey" => Surfaces::WarmGrey,
            _ => return None,
        })
    }
}

#[allow(dead_code)] // The Theme page will read these; until it exists, the tests are the only caller.
impl Contrast {
    fn name(self) -> &'static str {
        match self {
            Contrast::Normal => "normal",
            Contrast::Strong => "strong",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "normal" => Contrast::Normal,
            "strong" => Contrast::Strong,
            _ => return None,
        })
    }
}

#[allow(dead_code)] // The Theme page will read these; until it exists, the tests are the only caller.
impl Folders {
    fn name(self) -> &'static str {
        match self {
            Folders::Classic => "classic",
            Folders::Accent => "accent",
            Folders::Grey => "grey",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "classic" => Folders::Classic,
            "accent" => Folders::Accent,
            "grey" => Folders::Grey,
            _ => return None,
        })
    }
}

#[allow(dead_code)] // The Theme page will read these; until it exists, the tests are the only caller.
impl Corners {
    fn name(self) -> &'static str {
        match self {
            Corners::Four8 => "four8",
            Corners::Six10 => "six10",
            Corners::Eight14 => "eight14",
            Corners::Pills => "pills",
            Corners::Square => "square",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "four8" => Corners::Four8,
            "six10" => Corners::Six10,
            "eight14" => Corners::Eight14,
            "pills" => Corners::Pills,
            "square" => Corners::Square,
            _ => return None,
        })
    }

    /// The radius each group is drawn at: bars (`rc2`, `rc3`), controls (`rc4`..`rc6`), cards
    /// (`rc7`..`rc9`, `rc10`, `rc12`, `rc14`).
    fn sizes(self) -> (u32, u32, u32) {
        match self {
            Corners::Four8 => (2, 4, 8),
            Corners::Six10 => (3, 6, 10),
            Corners::Eight14 => (4, 8, 14),
            Corners::Pills => (3, 12, 16),
            Corners::Square => (0, 0, 0),
        }
    }

    /// The radius a token is drawn at, or None for a token that is not a radius.
    fn of(self, token: &str) -> Option<u32> {
        let (bars, controls, cards) = self.sizes();
        Some(match token {
            "rc2" | "rc3" => bars,
            "rc4" | "rc5" | "rc6" => controls,
            "rc7" | "rc8" | "rc9" | "rc10" | "rc12" | "rc14" => cards,
            "rcpill" => {
                if self == Corners::Square {
                    0
                } else {
                    15
                }
            }
            _ => return None,
        })
    }
}

#[allow(dead_code)] // The Theme page will read these; until it exists, the tests are the only caller.
impl Motion {
    fn name(self) -> &'static str {
        match self {
            Motion::Subtle => "subtle",
            Motion::Lively => "lively",
            Motion::None => "none",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "subtle" => Motion::Subtle,
            "lively" => Motion::Lively,
            "none" => Motion::None,
            _ => return None,
        })
    }
}

#[allow(dead_code)] // The Theme page will read these; until it exists, the tests are the only caller.
impl Bars {
    fn name(self) -> &'static str {
        match self {
            Bars::Solid => "solid",
            Bars::Gradient => "gradient",
            Bars::Thin => "thin",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "solid" => Bars::Solid,
            "gradient" => Bars::Gradient,
            "thin" => Bars::Thin,
            _ => return None,
        })
    }
}

#[allow(dead_code)] // The Theme page will read these; until it exists, the tests are the only caller.
impl SmallText {
    fn name(self) -> &'static str {
        match self {
            SmallText::LeadWords => "leadwords",
            SmallText::Plain => "plain",
            SmallText::Grey12 => "grey12",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "leadwords" => SmallText::LeadWords,
            "plain" => SmallText::Plain,
            "grey12" => SmallText::Grey12,
            _ => return None,
        })
    }
}

#[allow(dead_code)] // The Theme page will read these; until it exists, the tests are the only caller.
impl Ticks {
    fn name(self) -> &'static str {
        match self {
            Ticks::Square => "square",
            Ticks::Round => "round",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "square" => Ticks::Square,
            "round" => Ticks::Round,
            _ => return None,
        })
    }
}

#[allow(dead_code)] // The Theme page will read these; until it exists, the tests are the only caller.
impl TextSizes {
    fn name(self) -> &'static str {
        match self {
            TextSizes::AsWritten => "aswritten",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "aswritten" => TextSizes::AsWritten,
            _ => return None,
        })
    }
}

#[allow(dead_code)] // The Theme page will read these; until it exists, the tests are the only caller.
impl Look {
    /// The options as one line, for Copy theme: `accent=iris;surfaces=slate;...`, lowercase.
    pub(crate) fn to_kv(&self) -> String {
        format!(
            "accent={};surfaces={};contrast={};folders={};corners={};motion={};bars={};smalltext={};ticks={};text={}",
            self.accent.name(),
            self.surfaces.name(),
            self.contrast.name(),
            self.folders.name(),
            self.corners.name(),
            self.motion.name(),
            self.bars.name(),
            self.small_text.name(),
            self.ticks.name(),
            self.text.name(),
        )
    }

    /// A line as Copy theme wrote it: only known keys with known values are taken, everything
    /// else is ignored, and the rest starts from Sprawl's own look. It is text from outside.
    pub(crate) fn from_kv(s: &str) -> Look {
        let mut look = Look::default();
        for part in s.split(';') {
            let Some((key, value)) = part.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "accent" => {
                    if let Some(v) = Accent::from_name(value) {
                        look.accent = v;
                    }
                }
                "surfaces" => {
                    if let Some(v) = Surfaces::from_name(value) {
                        look.surfaces = v;
                    }
                }
                "contrast" => {
                    if let Some(v) = Contrast::from_name(value) {
                        look.contrast = v;
                    }
                }
                "folders" => {
                    if let Some(v) = Folders::from_name(value) {
                        look.folders = v;
                    }
                }
                "corners" => {
                    if let Some(v) = Corners::from_name(value) {
                        look.corners = v;
                    }
                }
                "motion" => {
                    if let Some(v) = Motion::from_name(value) {
                        look.motion = v;
                    }
                }
                "bars" => {
                    if let Some(v) = Bars::from_name(value) {
                        look.bars = v;
                    }
                }
                "smalltext" => {
                    if let Some(v) = SmallText::from_name(value) {
                        look.small_text = v;
                    }
                }
                "ticks" => {
                    if let Some(v) = Ticks::from_name(value) {
                        look.ticks = v;
                    }
                }
                "text" => {
                    if let Some(v) = TextSizes::from_name(value) {
                        look.text = v;
                    }
                }
                _ => {}
            }
        }
        look
    }
}

/// A colour already in the table, for the values derived from it. A name that isn't there is a
/// bug here, not a loading error on a page.
fn colour(t: &HashMap<&'static str, String>, name: &str) -> u32 {
    argb(&t[name]).unwrap_or(0)
}

/// The values for a look on a page: today's written values, then whatever its options change —
/// the surfaces family, the contrast, the accent (and the selected row and accent text built
/// from it), the folder icons and the corner radii.
pub(crate) fn table(look: &Look, light: bool) -> HashMap<&'static str, String> {
    let mut t: HashMap<&'static str, String> = TOKENS.iter().map(|t| (t.name, if light { t.light } else { t.value }.to_string())).collect();
    let fill = look.accent.fill(light);

    if let Some(b) = surfaces_base(look.surfaces, light, fill) {
        let entries = [
            ("ground", b.ground),
            ("card", b.card),
            ("row", b.row),
            ("popup", b.popup),
            // the floating panels sit on the popup's colour; an open row is a step above the row
            ("panel", b.popup),
            ("rowopen", mix(b.row, b.text1, 6)),
            ("line1", b.line1),
            ("line2", b.line2),
            ("edge", b.line2),
            // the brightest text, and the two between the written three
            ("text0", if light { mix(b.text1, 0xFF00_0000, 40) } else { 0xFFFF_FFFF }),
            ("text1", b.text1),
            ("text2", b.text2),
            ("text3", b.text3),
            ("rest", b.text1),
            ("label", mix(b.text2, b.text3, 50)),
            ("faint", mix(b.text3, b.ground, 25)),
        ];
        for (name, c) in entries {
            t.insert(name, hex(c));
        }
    }

    if look.contrast == Contrast::Strong {
        for name in ["line1", "line2", "edge"] {
            t.insert(name, hex(firmer(colour(&t, name), 1.8, 0x66)));
        }
        let text2 = mix(colour(&t, "text2"), colour(&t, "text1"), 40);
        let text3 = mix(colour(&t, "text3"), text2, 50);
        let label = mix(colour(&t, "label"), text2, 40);
        let faint = mix(colour(&t, "faint"), text2, 50);
        t.insert("text2", hex(text2));
        t.insert("text3", hex(text3));
        t.insert("label", hex(label));
        t.insert("faint", hex(faint));
    }

    // The selected row is the accent mixed a quarter into the card; a glyph on the accent is
    // black or white, whichever reads; accent text on the page is the fill, moved until it reads.
    let card = colour(&t, "card");
    t.insert("accent", hex(fill));
    t.insert("onaccent", hex(on_accent(fill)));
    t.insert("accenttext", hex(accent_text(fill, card, light)));
    t.insert("selected", hex(mix(card, fill, 26)));

    match look.folders {
        Folders::Classic => {}
        Folders::Accent => {
            let text = t["accenttext"].clone();
            t.insert("folder", text);
        }
        Folders::Grey => {
            let text = t["text3"].clone();
            t.insert("folder", text);
        }
    }

    for token in TOKENS {
        if let Some(n) = look.corners.of(token.name) {
            t.insert(token.name, n.to_string());
        }
    }
    t
}

/// A token's value in the look in use, for XAML that takes it as element CONTENT rather than as
/// an attribute value (`<CornerRadius x:Key="ControlCornerRadius">4</CornerRadius>`), where a
/// token cannot be written. A name in the table is required: a wrong one is a bug here, not a
/// loading error on a page.
pub(crate) fn get(name: &str) -> String {
    debug_assert!(TOKENS.iter().any(|t| t.name == name), "no token {name}");
    TABLE.with(|c| {
        let mut borrow = c.borrow_mut();
        let table = borrow.get_or_insert_with(|| table(&current(), crate::theme::light()));
        table.get(name).cloned().unwrap_or_default()
    })
}

/// The token for a text size, by value: `{$fs12}` for 12, `{$fs12_5}` for 12.5. For the XAML
/// that Rust builds from numbers (look.rs's styles), so a size lives in one place too. A size
/// the look has no token for is written as the number it is.
pub(crate) fn fs(n: f64) -> String {
    sized("fs", n)
}

/// The token for a corner radius, by value: `{$rc4}` for 4.
pub(crate) fn rc(n: f64) -> String {
    sized("rc", n)
}

fn sized(prefix: &str, n: f64) -> String {
    let number = if n.fract() == 0.0 { format!("{}", n as i64) } else { format!("{n}") };
    let name = format!("{prefix}{}", number.replace('.', "_"));
    if TOKENS.iter().any(|t| t.name == name) {
        format!("{{${name}}}")
    } else {
        number
    }
}

thread_local! {
    static TABLE: std::cell::RefCell<Option<HashMap<&'static str, String>>> = const { std::cell::RefCell::new(None) };
    /// The look the table was built from, so a table cleared and built again (a live theme
    /// change) keeps the user's options rather than falling back to Sprawl's own.
    static LOOK: std::cell::RefCell<Option<Look>> = const { std::cell::RefCell::new(None) };
}

/// The look in use. Set at start, and again when the Theme page saves; until then, Sprawl's own.
#[allow(dead_code)]
pub(crate) fn set(look: &Look) {
    let t = table(look, crate::theme::light());
    TABLE.with(|c| *c.borrow_mut() = Some(t));
    LOOK.with(|c| *c.borrow_mut() = Some(look.clone()));
}

/// The look in use, for a rebuild of the table.
#[allow(dead_code)]
pub(crate) fn current() -> Look {
    LOOK.with(|c| c.borrow().clone()).unwrap_or_default()
}

/// `xaml` with every `="{$name}"` replaced by its value. A name that isn't in the table is left
/// exactly as it is, so a mistake shows as a loading error on the one element rather than as a
/// wrong colour nobody notices.
pub(crate) fn expand(xaml: &str) -> Cow<'_, str> {
    if !xaml.contains("{$") {
        return Cow::Borrowed(xaml);
    }
    TABLE.with(|c| {
        let mut borrow = c.borrow_mut();
        let table = borrow.get_or_insert_with(|| table(&current(), crate::theme::light()));
        Cow::Owned(expand_with(xaml, table))
    })
}

/// The value of `raw` when it is a whole token (`{$name}`, a name in the table), otherwise `raw`
/// as it is. For a value used on its own rather than loaded as XAML, as `reconcile::set` does
/// with a value that came back from a diff.
pub(crate) fn resolve(raw: &str) -> Cow<'_, str> {
    let Some(name) = raw.strip_prefix("{$").and_then(|r| r.strip_suffix('}')) else {
        return Cow::Borrowed(raw);
    };
    TABLE.with(|c| {
        let mut borrow = c.borrow_mut();
        let table = borrow.get_or_insert_with(|| table(&current(), crate::theme::light()));
        match table.get(name) {
            Some(value) => Cow::Owned(value.clone()),
            None => Cow::Borrowed(raw),
        }
    })
}

/// A colour token written as a reference, `{$text1}`: for the XAML Rust builds from values rather
/// than writing whole (look.rs's styles, a row's background, a brush's `Color`). The caller puts
/// it in as a whole attribute value, so [`expand`] fills in the value for the theme in use — the
/// token's own light value, rather than a dark one turned around on the way out.
pub(crate) fn refer(name: &str) -> String {
    debug_assert!(TOKENS.iter().any(|t| t.name == name && t.value.starts_with('#')), "no colour token {name}");
    format!("{{${name}}}")
}

fn expand_with(xaml: &str, table: &HashMap<&'static str, String>) -> String {
    let b = xaml.as_bytes();
    let mut out = String::with_capacity(xaml.len() + 64);
    let (mut i, mut last) = (0usize, 0usize);
    while let Some(at) = xaml[i..].find("{$") {
        let start = i + at;
        // a whole attribute value only: ="{$name}"
        let opened = start >= 2 && b[start - 1] == b'"' && b[start - 2] == b'=';
        let Some(end) = xaml[start + 2..].find('}').map(|p| start + 2 + p) else { break };
        let closed = b.get(end + 1) == Some(&b'"');
        if let Some(value) = (opened && closed).then(|| table.get(&xaml[start + 2..end])).flatten() {
            out.push_str(&xaml[last..start]);
            out.push_str(value);
            last = end + 1;
        }
        i = end + 1;
    }
    out.push_str(&xaml[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_attribute_value_is_the_only_token() {
        let t = table(&Look::default(), false);
        let one = |x: &str| expand_with(x, &t);
        assert_eq!(one(r#"<Border Background="{$surface2}"/>"#), r##"<Border Background="#0CFFFFFF"/>"##);
        assert_eq!(one(r#"<TextBlock Foreground="{$text3}" Background="{$hover}"/>"#), r##"<TextBlock Foreground="#FF8791A8" Background="#1AFFFFFF"/>"##);
        // a name nobody knows stays, so it fails loudly at load instead of quietly being wrong
        assert_eq!(one(r#"<Border Background="{$nosuch}"/>"#), r#"<Border Background="{$nosuch}"/>"#);
        // a size and a radius are whole attribute values too
        assert_eq!(one(r#"<TextBlock FontSize="{$fs12}" CornerRadius="{$rc4}"/>"#), "<TextBlock FontSize=\"12\" CornerRadius=\"4\"/>");
        // part of a longer value is not a token
        assert_eq!(one(r#"<Border Margin="0,{$text1},0,0"/>"#), r#"<Border Margin="0,{$text1},0,0"/>"#);
        // and neither is text that happens to look like one
        assert_eq!(one(r#"<TextBlock Text="a {$text1} b"/>"#), r#"<TextBlock Text="a {$text1} b"/>"#);
    }

    #[test]
    fn text_of_the_users_can_never_be_a_token() {
        // every piece of text Sprawl shows goes through `esc`, which breaks a leading brace
        let t = table(&Look::default(), false);
        let name = crate::ui::esc("{$text1}");
        assert_eq!(name, "{}{$text1}");
        let x = format!(r#"<TextBlock Text="{name}"/>"#);
        assert_eq!(expand_with(&x, &t), x, "a file named like a token is shown, not replaced");
    }

    #[test]
    fn a_string_with_no_token_is_not_copied() {
        assert!(matches!(expand(r##"<Border Background="#0CFFFFFF"/>"##), Cow::Borrowed(_)));
    }

    #[test]
    fn every_token_is_named_once_and_says_what_it_is_for() {
        let t = table(&Look::default(), false);
        let mut seen = std::collections::HashSet::new();
        for token in TOKENS {
            assert!(seen.insert(token.name), "{} is in the list twice", token.name);
            assert!(!token.what.is_empty(), "{} says nothing", token.name);
            // computed from the accent, or drawn at the corner option's size, rather than written
            if matches!(token.name, "selected" | "accent" | "onaccent" | "accenttext") {
                let v = &t[token.name];
                assert!(v.len() == 9 && v.starts_with('#'), "{}: {v}", token.name);
                continue;
            }
            if token.name.starts_with("rc") {
                let v = &t[token.name];
                assert!(v.parse::<u32>().is_ok(), "{}: {v}", token.name);
                continue;
            }
            assert_eq!(t.get(token.name).map(String::as_str), Some(token.value));
        }
        assert_eq!(t.len(), TOKENS.len());
    }

    /// Every value is written the one way the page reads it: a colour as eight digits
    /// (`#AARRGGBB`) on both pages, a text size as the number its name says (`fs12_5` is 12.5),
    /// the same number either way. A corner's name is the radius it was written with, before the
    /// look moved the drawings (`rc6` is drawn at 4 today), so it is only checked to be a number.
    #[test]
    fn every_value_is_written_the_way_the_page_reads_it() {
        for token in TOKENS {
            let v = token.value;
            if let Some(suffix) = token.name.strip_prefix("fs") {
                assert_eq!(v, suffix.replace('_', "."), "{}", token.name);
                let n: f64 = v.parse().unwrap_or_else(|_| panic!("{}: {v}", token.name));
                assert!(n > 0.0 && n.is_finite(), "{}: {v}", token.name);
                assert_eq!(token.light, v, "{}: a size is the same on both pages", token.name);
            } else if token.name.starts_with("rc") {
                let n: f64 = v.parse().unwrap_or_else(|_| panic!("{}: {v}", token.name));
                assert!(n > 0.0 && n.is_finite(), "{}: {v}", token.name);
                assert_eq!(token.light, v, "{}: a radius is the same on both pages", token.name);
            } else {
                assert_eq!(v.len(), 9, "{}: {v}", token.name);
                assert_eq!(token.light.len(), 9, "{}: {}", token.name, token.light);
            }
        }
    }

    /// Every `.rs` file of the app, tests included: the places a token can be written. tokens.rs
    /// is where the table is, and bindings.rs is generated, so both are left out.
    fn app_sources() -> Vec<std::path::PathBuf> {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") && p.file_name().is_some_and(|f| f != "tokens.rs" && f != "bindings.rs") {
                    files.push(p);
                }
            }
        }
        assert!(files.len() > 30, "the sources weren't found: {}", files.len());
        files
    }

    /// Every token written anywhere in the app is one this module knows, and is written as a
    /// whole attribute value. A name with a typo in it would otherwise reach XAML as it stands
    /// and take a panel down with it, and only on the screen that holds it.
    #[test]
    fn every_token_the_app_writes_is_a_known_name_in_an_attribute_value() {
        let files = app_sources();
        let known: std::collections::HashSet<&str> = TOKENS.iter().map(|t| t.name).collect();
        let mut seen = 0;
        for path in &files {
            let text = std::fs::read_to_string(path).unwrap();
            let b = text.as_bytes();
            let mut i = 0;
            while let Some(at) = text[i..].find("{$") {
                let start = i + at;
                let end = text[start + 2..].find('}').map(|p| start + 2 + p).unwrap_or(text.len());
                let name = &text[start + 2..end];
                let file = path.file_name().unwrap().to_string_lossy();
                let line = text[..start].lines().count();
                assert!(known.contains(name), "{file}:{line}: {{${name}}} is not a token");
                // written either plainly or doubled inside a format string: `="{$n}"` or `="{{$n}}"`
                let doubled = start >= 1 && b[start - 1] == b'{';
                let open = if doubled { start - 1 } else { start };
                let close = if doubled { end + 2 } else { end + 1 };
                assert!(open >= 2 && b[open - 1] == b'"' && b[open - 2] == b'=', "{file}:{line}: {{${name}}} is not a whole attribute value");
                assert_eq!(b.get(close), Some(&b'"'), "{file}:{line}: {{${name}}} is not a whole attribute value");
                if doubled {
                    assert_eq!(&text[end..close], "}}", "{file}:{line}: {{${name}}} opens doubled and closes single");
                }
                seen += 1;
                i = end + 1;
            }
        }
        assert!(seen > 200, "only {seen} tokens found: the scan missed the sources");
    }

    /// Every colour token is written the one way the page reads it: `#AARRGGBB`, on both pages.
    /// What the two values are is the look's business; that they can be read is this test's.
    #[test]
    fn every_colour_token_is_a_valid_argb_on_both_pages() {
        for token in TOKENS {
            if !token.value.starts_with('#') {
                continue;
            }
            for (page, v) in [("dark", token.value), ("light", token.light)] {
                assert_eq!(v.len(), 9, "{} ({page}): {v}", token.name);
                assert!(v[1..].bytes().all(|c| c.is_ascii_hexdigit()), "{} ({page}): {v}", token.name);
            }
        }
    }

    /// The table holds the light page's values when it is built for it, and the dark ones
    /// otherwise; sizes don't change.
    #[test]
    fn the_table_can_hold_the_light_page() {
        let dark = table(&Look::default(), false);
        let light = table(&Look::default(), true);
        assert_eq!(dark["surface2"], "#0CFFFFFF");
        assert_eq!(light["surface2"], "#0C000000");
        assert_eq!(dark["text3"], "#FF8791A8");
        assert_eq!(light["text3"], "#FF5B647A");
        // the card and the ground are colours of their own, not washes
        assert_eq!(dark["card"], "#FF191C26");
        assert_eq!(light["card"], "#FFF6F7FC");
        // a caution drew as a fill keeps its colour on the light page
        assert_eq!(light["warnmark"], "#FFF5B342");
        // a size is the same on both pages
        assert_eq!(light["fs12"], dark["fs12"]);
    }

    /// A value that is used on its own rather than loaded as XAML (a diff's one attribute) is
    /// resolved from the table; anything that isn't one whole token is left as it is.
    #[test]
    fn a_token_used_on_its_own_is_resolved() {
        assert!(matches!(resolve("{$text3}"), Cow::Owned(_)));
        assert_eq!(resolve("{$text3}"), "#FF8791A8");
        assert_eq!(resolve("{$fs12}"), "12");
        // not a whole token, or not a name in the table: unchanged, and not copied
        assert!(matches!(resolve("#FF8E97AA"), Cow::Borrowed(_)));
        assert_eq!(resolve("0,{$text3}"), "0,{$text3}");
        assert_eq!(resolve("{$nosuch}"), "{$nosuch}");
    }

    /// Font sizes and corner radii are tokens too: a number written straight into the XAML is
    /// what this part of the work removes. The scan is the whole app, fixtures included;
    /// tokens.rs is left out because that is where the table writes the numbers themselves.
    #[test]
    fn no_font_size_or_corner_radius_is_written_as_a_number() {
        let files = app_sources();
        for path in &files {
            let text = std::fs::read_to_string(path).unwrap();
            let file = path.file_name().unwrap().to_string_lossy();
            for attr in ["FontSize", "CornerRadius"] {
                let needle = format!(r#"{attr}=""#);
                let mut i = 0;
                while let Some(at) = text[i..].find(&needle) {
                    let value = i + at + needle.len();
                    let digit = text.as_bytes().get(value).is_some_and(|b| b.is_ascii_digit());
                    let line = text[..value].lines().count();
                    assert!(!digit, "{file}:{line}: {attr}=... is a number, not a token");
                    i = value;
                }
            }
        }
    }

    /// The helpers look.rs builds its styles with: the token when the look has one, the number
    /// itself when it does not (so a size added before its token still shows).
    #[test]
    fn a_size_from_rust_uses_its_token() {
        assert_eq!(fs(12.0), "{$fs12}");
        assert_eq!(fs(12.5), "{$fs12_5}");
        assert_eq!(fs(13.5), "13.5");
        assert_eq!(rc(4.0), "{$rc4}");
        assert_eq!(rc(11.0), "11");
    }

    /// A value taken as element content rather than as an attribute value (a corner radius a
    /// resource dictionary holds): the table's value, for the theme in use.
    #[test]
    fn a_token_can_be_read_for_element_content() {
        assert_eq!(get("rc4"), "4");
        assert_eq!(get("fs12"), "12");
        assert_eq!(get("text3"), "#FF8791A8");
        // the look draws a written 6 at 4
        assert_eq!(get("rc6"), "4");
    }

    /// The accent is the user's, eight digits the way a colour token is written; the colour on
    /// it is black or white, whichever reads better on the fill.
    #[test]
    fn the_accent_is_a_colour_token_both_ways() {
        let accent = get("accent");
        assert_eq!(accent, format!("#FF{}", crate::look::accent().fill.trim_start_matches('#')));
        let fill = argb(&accent).unwrap();
        let on = get("onaccent");
        let better = if ratio(0xFF00_0000, fill) >= ratio(0xFFFF_FFFF, fill) { "#FF000000" } else { "#FFFFFFFF" };
        assert_eq!(on, better);
    }

    #[test]
    fn a_reference_is_resolved_by_the_page_it_is_written_for() {
        assert_eq!(refer("text1"), "{$text1}");
        let dark = table(&Look::default(), false);
        let light = table(&Look::default(), true);
        // the caller puts a reference in as a whole Foreground value, as every site does
        let fg = |name: &str| format!(r#"<TextBlock Foreground="{}"/>"#, refer(name));
        assert_eq!(expand_with(&fg("text1"), &dark), r##"<TextBlock Foreground="#FFEEF1F8"/>"##);
        assert_eq!(expand_with(&fg("text1"), &light), r##"<TextBlock Foreground="#FF141824"/>"##);
        // a second token, to show the table decides and the reference is the same text either way
        assert_eq!(expand_with(&fg("surface2"), &dark), r##"<TextBlock Foreground="#0CFFFFFF"/>"##);
    }

    #[test]
    fn an_unclosed_token_leaves_the_string_alone() {
        let t = table(&Look::default(), false);
        assert_eq!(expand_with(r#"<Border Background="{$surface2"/>"#, &t), r#"<Border Background="{$surface2"/>"#);
        assert_eq!(expand_with("{$", &t), "{$");
    }

    /// Sprawl's own look is the values the app has drawn with: every written token is unchanged,
    /// the accent pair and the selected row are the ones the accent made before, and the corners
    /// are Four8's. Only the light page's accent text moves, to reach its contrast (the old
    /// #0080C2 is 4.03:1 on the card).
    #[test]
    fn the_default_look_is_todays_look() {
        let dark = table(&Look::default(), false);
        let light = table(&Look::default(), true);
        for token in TOKENS {
            if matches!(token.name, "selected" | "accent" | "onaccent" | "accenttext") || token.name.starts_with("rc") {
                continue;
            }
            assert_eq!(dark[token.name], token.value, "{} (dark)", token.name);
            assert_eq!(light[token.name], token.light, "{} (light)", token.name);
        }
        // the accent, the colour on it, and the selected row, as they were worked out before
        let fill = argb(&format!("#FF{}", crate::look::accent().fill.trim_start_matches('#'))).unwrap();
        assert_eq!(dark["accent"], hex(fill));
        assert_eq!(light["accent"], hex(fill));
        let better = if ratio(0xFF00_0000, fill) >= ratio(0xFFFF_FFFF, fill) { "#FF000000" } else { "#FFFFFFFF" };
        assert_eq!(dark["onaccent"], better);
        let card = 0x0019_1C26u32;
        let old = |shift: u32| (channel(fill, shift) * 26 + channel(card, shift) * 74 + 50) / 100;
        assert_eq!(dark["selected"], format!("#FF{:02X}{:02X}{:02X}", old(16), old(8), old(0)));
        // Four8 carries today's written sizes, except the cards the group moves to 8
        for (name, n) in [
            ("rc2", "2"),
            ("rc3", "2"),
            ("rc4", "4"),
            ("rc5", "4"),
            ("rc6", "4"),
            ("rc7", "8"),
            ("rc8", "8"),
            ("rc9", "8"),
            ("rc10", "8"),
            ("rc12", "8"),
            ("rc14", "8"),
            ("rcpill", "15"),
        ] {
            assert_eq!(dark[name], n, "{name} (dark)");
            assert_eq!(light[name], n, "{name} (light)");
        }
    }

    /// The default look and every single-option change from it, on both pages: every colour is a
    /// valid token, and the text on each surface, the glyph on the accent and the accent text on
    /// the card all reach the contrast the Theme page promises.
    #[test]
    fn every_option_keeps_its_contrast() {
        let mut looks = vec![Look::default()];
        for accent in [Accent::Windows, Accent::Azure, Accent::Iris, Accent::Violet, Accent::Sunset] {
            looks.push(Look { accent, ..Look::default() });
        }
        for surfaces in [Surfaces::Ink, Surfaces::Graphite, Surfaces::Tinted, Surfaces::Black, Surfaces::Slate, Surfaces::WarmGrey] {
            looks.push(Look { surfaces, ..Look::default() });
        }
        for contrast in [Contrast::Normal, Contrast::Strong] {
            looks.push(Look { contrast, ..Look::default() });
        }
        for folders in [Folders::Classic, Folders::Accent, Folders::Grey] {
            looks.push(Look { folders, ..Look::default() });
        }
        for corners in [Corners::Four8, Corners::Six10, Corners::Eight14, Corners::Pills, Corners::Square] {
            looks.push(Look { corners, ..Look::default() });
        }
        for look in looks {
            for light in [false, true] {
                let page = if light { "light" } else { "dark" };
                let t = table(&look, light);
                let c = |name: &str| argb(&t[name]).unwrap_or_else(|| panic!("{page} {}: {name} is {}", look.to_kv(), t[name]));
                let on = |a: &str, b: &str| ratio(c(a), c(b));
                for token in TOKENS.iter().filter(|t| t.value.starts_with('#')) {
                    let v = &t[token.name];
                    assert!(
                        v.len() == 9 && v.starts_with('#') && v[1..].bytes().all(|b| b.is_ascii_hexdigit()),
                        "{page} {}: {} is {v}",
                        look.to_kv(),
                        token.name
                    );
                }
                for surface in ["ground", "card", "row"] {
                    let need = [("text1", 7.0), ("text2", 4.5), ("text3", 3.0)];
                    for (text, least) in need {
                        let got = on(text, surface);
                        assert!(got >= least, "{page} {}: {text} on {surface} is {got:.2} to 1, needs {least}", look.to_kv());
                    }
                }
                let got = on("onaccent", "accent");
                assert!(got >= 4.5, "{page} {}: onaccent on accent is {got:.2} to 1", look.to_kv());
                let got = on("accenttext", "card");
                assert!(got >= 4.5, "{page} {}: accenttext on card is {got:.2} to 1", look.to_kv());
            }
        }
    }

    /// The options as one line of text, and back: every combination of the five options, and the
    /// line read from outside takes only known keys with known values.
    #[test]
    fn a_look_survives_a_round_trip_and_ignores_junk() {
        assert_eq!(
            Look::default().to_kv(),
            "accent=windows;surfaces=ink;contrast=normal;folders=classic;corners=four8;motion=subtle;bars=solid;smalltext=leadwords;ticks=square;text=aswritten"
        );
        for accent in [Accent::Windows, Accent::Azure, Accent::Iris, Accent::Violet, Accent::Sunset] {
            for surfaces in [Surfaces::Ink, Surfaces::Graphite, Surfaces::Tinted, Surfaces::Black, Surfaces::Slate, Surfaces::WarmGrey] {
                for contrast in [Contrast::Normal, Contrast::Strong] {
                    for folders in [Folders::Classic, Folders::Accent, Folders::Grey] {
                        for corners in [Corners::Four8, Corners::Six10, Corners::Eight14, Corners::Pills, Corners::Square] {
                            for motion in [Motion::Subtle, Motion::Lively, Motion::None] {
                                for bars in [Bars::Solid, Bars::Gradient, Bars::Thin] {
                                    for small_text in [SmallText::LeadWords, SmallText::Plain, SmallText::Grey12] {
                                        for ticks in [Ticks::Square, Ticks::Round] {
                                            let look = Look { accent, surfaces, contrast, folders, corners, motion, bars, small_text, ticks, text: TextSizes::AsWritten };
                                            assert_eq!(Look::from_kv(&look.to_kv()), look, "{}", look.to_kv());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // only known keys with known values: an unknown key, an unknown value, a wrong case, an
        // empty part and a broken part are all left behind
        let junk = Look::from_kv(
            "accent=iris;surfaces=slate;contrast=loud;nope=1;;corners=SQUARE;folders=accent;motion=lively;bars=gradient;smalltext=grey12;ticks=round;text=aswritten;more junk",
        );
        assert_eq!(
            junk,
            Look {
                accent: Accent::Iris,
                surfaces: Surfaces::Slate,
                folders: Folders::Accent,
                motion: Motion::Lively,
                bars: Bars::Gradient,
                small_text: SmallText::Grey12,
                ticks: Ticks::Round,
                ..Look::default()
            }
        );
        assert_eq!(Look::from_kv(""), Look::default());
    }

    /// The gradient's far stop is six digits: the light page's rewrite leaves it alone.
    #[test]
    fn the_lighter_accent_is_six_digits() {
        assert_eq!(lighter("#000000"), "#595959");
        assert_eq!(lighter("#FFFFFF"), "#FFFFFF");
        let l = lighter(&crate::look::accent().fill);
        assert_eq!(l.len(), 7, "{l}");
        assert!(l.starts_with('#') && l[1..].bytes().all(|b| b.is_ascii_hexdigit()), "{l}");
    }
}
