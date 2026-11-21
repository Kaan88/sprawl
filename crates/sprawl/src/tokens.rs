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
/// the table is made from it. Today it holds the one look there is; the options of the Theme
/// page are added here as each is built.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Look {
    pub surfaces: Surfaces,
    pub corners: Corners,
    pub text: TextSizes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Surfaces {
    /// Today's: white washes over the window's backdrop.
    #[default]
    Washed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Corners {
    /// Today's sizes, kept as they are.
    #[default]
    AsWritten,
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
    // it from the Windows accent and the card (see `selected`).
    Token { name: "selected", what: "the background of a selected row or tab", value: "#FF133453", light: "#FFB6D6F2" },
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
    // already sets, `fs12_5` the 12.5, `rc4` a 4 px corner. The value is what was on the page.
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
    Token { name: "rc5", what: "corner radius 5", value: "5", light: "5" },
    Token { name: "rc6", what: "corner radius 6", value: "6", light: "6" },
    Token { name: "rc7", what: "corner radius 7", value: "7", light: "7" },
    Token { name: "rc8", what: "corner radius 8", value: "8", light: "8" },
    Token { name: "rc9", what: "corner radius 9", value: "9", light: "9" },
    Token { name: "rc10", what: "corner radius 10", value: "10", light: "10" },
    Token { name: "rc12", what: "corner radius 12", value: "12", light: "12" },
    Token { name: "rc14", what: "corner radius 14", value: "14", light: "14" },
];

/// The selected row's background: the Windows accent mixed a quarter into the card, so a chosen
/// row reads as chosen without losing the card's tone. Computed rather than written down: the
/// accent is the user's, and it already differs by theme (`look::accent` reads both).
fn selected(light: bool) -> String {
    let card = if light { 0x00F6_F7FC } else { 0x0019_1C26 };
    let accent = crate::look::accent().fill.trim_start_matches('#');
    let accent = u32::from_str_radix(accent, 16).unwrap_or(0x0078_D4);
    let mix = |shift: u32| ((accent >> shift & 0xFF) * 26 + (card >> shift & 0xFF) * 74 + 50) / 100;
    format!("#FF{:02X}{:02X}{:02X}", mix(16), mix(8), mix(0))
}

/// The values for a look: the light page's when `light`, the dark page's otherwise.
pub(crate) fn table(look: &Look, light: bool) -> HashMap<&'static str, String> {
    let Look { surfaces: Surfaces::Washed, corners: Corners::AsWritten, text: TextSizes::AsWritten } = look;
    let mut t: HashMap<&'static str, String> = TOKENS.iter().map(|t| (t.name, if light { t.light } else { t.value }.to_string())).collect();
    t.insert("selected", selected(light));
    t
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
}

/// The look in use. Set once at start, before any XAML is loaded; until then, Sprawl's own.
/// (Nothing calls this yet: the Theme page is what will.)
#[allow(dead_code)]
pub(crate) fn set(look: &Look) {
    let t = table(look, crate::theme::light());
    TABLE.with(|c| *c.borrow_mut() = Some(t));
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
        let table = borrow.get_or_insert_with(|| table(&Look::default(), crate::theme::light()));
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
        let table = borrow.get_or_insert_with(|| table(&Look::default(), crate::theme::light()));
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
            // `selected` is computed from the accent; the table's value is a colour like any other
            if token.name == "selected" {
                let v = &t[token.name];
                assert!(v.len() == 9 && v.starts_with('#'), "selected: {v}");
                continue;
            }
            assert_eq!(t.get(token.name).map(String::as_str), Some(token.value));
        }
        assert_eq!(t.len(), TOKENS.len());
    }

    /// Every value is written the one way the page reads it: a colour as eight digits
    /// (`#AARRGGBB`) on both pages, a size as the number its name says (`fs12_5` is 12.5),
    /// the same number either way.
    #[test]
    fn every_value_is_written_the_way_the_page_reads_it() {
        for token in TOKENS {
            let v = token.value;
            if let Some(suffix) = token.name.strip_prefix("fs").or_else(|| token.name.strip_prefix("rc")) {
                assert_eq!(v, suffix.replace('_', "."), "{}", token.name);
                let n: f64 = v.parse().unwrap_or_else(|_| panic!("{}: {v}", token.name));
                assert!(n > 0.0 && n.is_finite(), "{}: {v}", token.name);
                assert_eq!(token.light, v, "{}: a size is the same on both pages", token.name);
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
}
