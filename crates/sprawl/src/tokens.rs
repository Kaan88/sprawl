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
    /// The same token on the light page: for a colour, exactly what [`crate::theme::flip`] made
    /// of `value` under the attribute the token is written for; for a size, the same number.
    pub light: &'static str,
}

/// The names a piece of XAML may use, with today's values. Adding one here is all it takes.
///
/// The surfaces are washes of white over the window's backdrop, lightest first; the text
/// colours are the three the page actually reads in (`text1` for what is being read, `text2`
/// for labels and second lines, `text3` for units and hints); the lines are washes used as a
/// border rather than a fill. Every colour token carries the dark page's value and the light
/// page's; a size token is the same number on both.
pub(crate) const TOKENS: &[Token] = &[
    Token { name: "surface1", what: "the faintest panel wash", value: "#0AFFFFFF", light: "#0A000000" },
    Token { name: "surface2", what: "a panel or card", value: "#0CFFFFFF", light: "#0C000000" },
    Token { name: "surface3", what: "a card that stands out a little", value: "#0FFFFFFF", light: "#0F000000" },
    Token { name: "hover", what: "a row or button under the pointer", value: "#14FFFFFF", light: "#14000000" },
    Token { name: "pressed", what: "a row or button while pressed", value: "#22FFFFFF", light: "#22000000" },
    Token { name: "line1", what: "a hairline between parts", value: "#18FFFFFF", light: "#18000000" },
    Token { name: "line2", what: "a firmer edge", value: "#22FFFFFF", light: "#22000000" },
    Token { name: "shade", what: "a dark wash under something raised", value: "#33000000", light: "#33FFFFFF" },
    Token { name: "text1", what: "a heading, or a figure being read", value: "#FFD5DBE8", light: "#FF171D2A" },
    Token { name: "text2", what: "what is being read", value: "#FFB9C2D6", light: "#FF293246" },
    Token { name: "text3", what: "labels, units and hints", value: "#FF8E97AA", light: "#FF555E71" },
    Token { name: "selected", what: "the background of a selected row or tab", value: "#FF2C3B63", light: "#FF9CABD3" },
    Token { name: "rest", what: "a selectable item's text at rest", value: "#FFE6EAF2", light: "#FF0D1119" },
    Token { name: "warn", what: "a caution: a held clean-up, a thing to read before pressing", value: "#FFF5B342", light: "#FFB8780A" },
    Token { name: "good", what: "something that went well, or is safe", value: "#FF4CC38A", light: "#FF319163" },
    // A caution or a good mark drawn as a fill (a dot or a bar), not as text: a colour fill is
    // not turned around on the light page, so it has a value of its own there.
    Token { name: "warnmark", what: "a caution colour used as a fill (a dot or bar)", value: "#FFF5B342", light: "#FFF5B342" },
    Token { name: "goodmark", what: "a good colour used as a fill (a dot or bar)", value: "#FF4CC38A", light: "#FF4CC38A" },
    Token { name: "link", what: "a word that can be pressed", value: "#FF8FB4FF", light: "#FF0040C2" },
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

/// The values for a look: the light page's when `light`, the dark page's otherwise.
pub(crate) fn table(look: &Look, light: bool) -> HashMap<&'static str, String> {
    let Look { surfaces: Surfaces::Washed, corners: Corners::AsWritten, text: TextSizes::AsWritten } = look;
    TOKENS.iter().map(|t| (t.name, if light { t.light } else { t.value }.to_string())).collect()
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
        assert_eq!(one(r#"<TextBlock Foreground="{$text3}" Background="{$hover}"/>"#), r##"<TextBlock Foreground="#FF8E97AA" Background="#14FFFFFF"/>"##);
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
            assert_eq!(t.get(token.name).map(String::as_str), Some(token.value));
        }
        assert_eq!(t.len(), TOKENS.len());
    }

    /// Part 2 of the polish plan moves values; it must not change one. Every colour token holds
    /// a colour the page already used, written the same way, on both pages; every size token
    /// holds the number its name says (`fs12_5` is 12.5), the same number either way.
    #[test]
    fn the_values_are_the_ones_already_on_the_page() {
        for token in TOKENS {
            let v = token.value;
            if let Some(suffix) = token.name.strip_prefix("fs").or_else(|| token.name.strip_prefix("rc")) {
                assert_eq!(v, suffix.replace('_', "."), "{}", token.name);
                let n: f64 = v.parse().unwrap_or_else(|_| panic!("{}: {v}", token.name));
                assert!(n > 0.0 && n.is_finite(), "{}: {v}", token.name);
                assert_eq!(token.light, v, "{}: a size is the same on both pages", token.name);
            } else {
                for (page, v) in [("dark", token.value), ("light", token.light)] {
                    assert!(v.starts_with('#') && (v.len() == 9 || v.len() == 7), "{} ({page}): {v}", token.name);
                    assert!(v[1..].bytes().all(|c| c.is_ascii_hexdigit()), "{} ({page}): {v}", token.name);
                    // eight digits, so a hand-written colour is turned around as it always has
                    assert_eq!(v.len(), 9, "{} ({page}): six-digit colours skip the light rewrite", token.name);
                }
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

    /// The proof that the light page has not changed a pixel. Before, a token became its dark
    /// value and `to_light` turned that value around under the attribute it was written for;
    /// now the token carries that turned-around value itself. So for every colour token site in
    /// the app, the flip of its dark value, read under its attribute, must be its light value.
    #[test]
    fn a_light_value_is_the_flip_of_the_dark_one_under_its_attribute() {
        let known: HashMap<&str, &Token> = TOKENS.iter().map(|t| (t.name, t)).collect();
        let mut seen = 0;
        for path in app_sources() {
            let text = std::fs::read_to_string(&path).unwrap();
            let b = text.as_bytes();
            let file = path.file_name().unwrap().to_string_lossy();
            let mut i = 0;
            while let Some(at) = text[i..].find("{$") {
                let start = i + at;
                let end = text[start + 2..].find('}').map(|p| start + 2 + p).unwrap_or(text.len());
                let name = &text[start + 2..end];
                let line = text[..start].lines().count();
                // written either plainly or doubled inside a format string: `="{$n}"` or `="{{$n}}"`
                let doubled = start >= 1 && b[start - 1] == b'{';
                let open = if doubled { start - 1 } else { start };
                let close = if doubled { end + 2 } else { end + 1 };
                // the test above proves every site is a whole attribute value; here it is read
                // the way `to_light` reads a colour, from the quote before the value
                assert!(open >= 2 && b[open - 1] == b'"' && b[open - 2] == b'=', "{file}:{line}: {{${name}}} is not a whole attribute value");
                assert_eq!(b.get(close), Some(&b'"'), "{file}:{line}: {{${name}}} is not a whole attribute value");
                let Some(token) = known.get(name) else {
                    i = end + 1;
                    continue;
                };
                if !token.value.starts_with('#') {
                    i = end + 1;
                    continue;
                }
                let attr = crate::theme::attr_before(&text[..open - 1]);
                let dark = u32::from_str_radix(&token.value[1..], 16).unwrap_or_else(|e| panic!("{file}:{line}: {}: {e}", token.name));
                let light = u32::from_str_radix(&token.light[1..], 16).unwrap();
                assert_eq!(crate::theme::flip(dark, attr == "Foreground"), light, "{file}:{line}: {{${name}}} on {attr}: the light page would change");
                seen += 1;
                i = end + 1;
            }
        }
        assert!(seen > 150, "only {seen} colour tokens found: the scan missed the sources");
    }

    /// The table holds the light page's values when it is built for it, and the dark ones
    /// otherwise; sizes don't change.
    #[test]
    fn the_table_can_hold_the_light_page() {
        let dark = table(&Look::default(), false);
        let light = table(&Look::default(), true);
        assert_eq!(dark["surface2"], "#0CFFFFFF");
        assert_eq!(light["surface2"], "#0C000000");
        assert_eq!(dark["text3"], "#FF8E97AA");
        assert_eq!(light["text3"], "#FF555E71");
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
        assert_eq!(resolve("{$text3}"), "#FF8E97AA");
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
    fn an_unclosed_token_leaves_the_string_alone() {
        let t = table(&Look::default(), false);
        assert_eq!(expand_with(r#"<Border Background="{$surface2"/>"#, &t), r#"<Border Background="{$surface2"/>"#);
        assert_eq!(expand_with("{$", &t), "{$");
    }
}
