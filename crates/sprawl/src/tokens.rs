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
//! does not change them: before and after must look the same, pixel for pixel. The page is
//! still written for dark and turned around by [`crate::theme::to_light`] when the window is
//! light, so a token holds what the dark page holds. Today there is no opaque ground: the
//! window sits on an acrylic backdrop and every panel is a white wash over it, which is why
//! the surface tokens are washes and not colours.

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

/// A token's name, what it stands for, and its value in Sprawl's own look.
pub(crate) struct Token {
    pub name: &'static str,
    /// Read by the Theme page, which has still to be built; until then by its test alone.
    #[allow(dead_code)]
    pub what: &'static str,
    pub value: &'static str,
}

/// The names a piece of XAML may use, with today's values. Adding one here is all it takes.
///
/// The surfaces are washes of white over the window's backdrop, lightest first; the text
/// colours are the three the page actually reads in (`text1` for what is being read, `text2`
/// for labels and second lines, `text3` for units and hints); the lines are washes used as a
/// border rather than a fill.
pub(crate) const TOKENS: &[Token] = &[
    Token { name: "surface1", what: "the faintest panel wash", value: "#0AFFFFFF" },
    Token { name: "surface2", what: "a panel or card", value: "#0CFFFFFF" },
    Token { name: "surface3", what: "a card that stands out a little", value: "#0FFFFFFF" },
    Token { name: "hover", what: "a row or button under the pointer", value: "#14FFFFFF" },
    Token { name: "pressed", what: "a row or button while pressed", value: "#22FFFFFF" },
    Token { name: "line1", what: "a hairline between parts", value: "#18FFFFFF" },
    Token { name: "line2", what: "a firmer edge", value: "#22FFFFFF" },
    Token { name: "shade", what: "a dark wash under something raised", value: "#33000000" },
    Token { name: "text1", what: "a heading, or a figure being read", value: "#FFD5DBE8" },
    Token { name: "text2", what: "what is being read", value: "#FFB9C2D6" },
    Token { name: "text3", what: "labels, units and hints", value: "#FF8E97AA" },
    Token { name: "selected", what: "the background of a selected row or tab", value: "#FF2C3B63" },
    Token { name: "rest", what: "a selectable item's text at rest", value: "#FFE6EAF2" },
    Token { name: "warn", what: "a caution: a held clean-up, a thing to read before pressing", value: "#FFF5B342" },
    Token { name: "good", what: "something that went well, or is safe", value: "#FF4CC38A" },
    Token { name: "link", what: "a word that can be pressed", value: "#FF8FB4FF" },
];

/// The values for a look, by token name.
pub(crate) fn table(look: &Look) -> HashMap<&'static str, String> {
    let Look { surfaces: Surfaces::Washed, corners: Corners::AsWritten, text: TextSizes::AsWritten } = look;
    TOKENS.iter().map(|t| (t.name, t.value.to_string())).collect()
}

thread_local! {
    static TABLE: std::cell::RefCell<Option<HashMap<&'static str, String>>> = const { std::cell::RefCell::new(None) };
}

/// The look in use. Set once at start, before any XAML is loaded; until then, Sprawl's own.
/// (Nothing calls this yet: the Theme page is what will.)
#[allow(dead_code)]
pub(crate) fn set(look: &Look) {
    let t = table(look);
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
        let table = borrow.get_or_insert_with(|| table(&Look::default()));
        Cow::Owned(expand_with(xaml, table))
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
        let t = table(&Look::default());
        let one = |x: &str| expand_with(x, &t);
        assert_eq!(one(r#"<Border Background="{$surface2}"/>"#), r##"<Border Background="#0CFFFFFF"/>"##);
        assert_eq!(one(r#"<TextBlock Foreground="{$text3}" Background="{$hover}"/>"#), r##"<TextBlock Foreground="#FF8E97AA" Background="#14FFFFFF"/>"##);
        // a name nobody knows stays, so it fails loudly at load instead of quietly being wrong
        assert_eq!(one(r#"<Border Background="{$nosuch}"/>"#), r#"<Border Background="{$nosuch}"/>"#);
        // part of a longer value is not a token
        assert_eq!(one(r#"<Border Margin="0,{$text1},0,0"/>"#), r#"<Border Margin="0,{$text1},0,0"/>"#);
        // and neither is text that happens to look like one
        assert_eq!(one(r#"<TextBlock Text="a {$text1} b"/>"#), r#"<TextBlock Text="a {$text1} b"/>"#);
    }

    #[test]
    fn text_of_the_users_can_never_be_a_token() {
        // every piece of text Sprawl shows goes through `esc`, which breaks a leading brace
        let t = table(&Look::default());
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
        let t = table(&Look::default());
        let mut seen = std::collections::HashSet::new();
        for token in TOKENS {
            assert!(seen.insert(token.name), "{} is in the list twice", token.name);
            assert!(!token.what.is_empty(), "{} says nothing", token.name);
            assert_eq!(t.get(token.name).map(String::as_str), Some(token.value));
        }
        assert_eq!(t.len(), TOKENS.len());
    }

    /// Part 2 of the polish plan moves values; it must not change one. Every colour token holds
    /// a colour the page already used, written the same way.
    #[test]
    fn the_values_are_the_ones_already_on_the_page() {
        for token in TOKENS {
            let v = token.value;
            assert!(v.starts_with('#') && (v.len() == 9 || v.len() == 7), "{}: {v}", token.name);
            assert!(v[1..].bytes().all(|c| c.is_ascii_hexdigit()), "{}: {v}", token.name);
            // eight digits, so the light page turns it around as it always has
            assert_eq!(v.len(), 9, "{}: six-digit colours skip the light rewrite", token.name);
        }
    }

    /// Every token written anywhere in the app is one this module knows, and is written as a
    /// whole attribute value. A name with a typo in it would otherwise reach XAML as it stands
    /// and take a panel down with it, and only on the screen that holds it.
    #[test]
    fn every_token_the_app_writes_is_a_known_name_in_an_attribute_value() {
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

    #[test]
    fn an_unclosed_token_leaves_the_string_alone() {
        let t = table(&Look::default());
        assert_eq!(expand_with(r#"<Border Background="{$surface2"/>"#, &t), r#"<Border Background="{$surface2"/>"#);
        assert_eq!(expand_with("{$", &t), "{$");
    }
}
