//! Light or dark. The page is drawn for dark, with its colours written into the XAML; in light
//! mode every piece of XAML is rewritten on its way to [`crate::state::load`]: greys and dark
//! tints turn their lightness around (a white wash becomes a black one, light grey text dark
//! grey), coloured text gets dark enough to read on white, and coloured fills (the accent, the
//! risk colours, the file types) stay as they are. The map draws its own colours either way.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

static LIGHT: AtomicBool = AtomicBool::new(false);

/// The choices of the Theme setting, in the order `prefs` lists them.
pub(crate) const CHOICES: [&str; 3] = ["Like Windows", "Light", "Dark"];

/// Decided once at start, from the Theme setting (or Windows' app mode).
pub(crate) fn init(setting: Option<&str>) {
    let light = match setting.and_then(|s| s.parse::<usize>().ok()).unwrap_or(0) {
        1 => true,
        2 => false,
        _ => windows_uses_light(),
    };
    LIGHT.store(light, Ordering::Relaxed);
}

pub(crate) fn light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

/// Windows' "Choose your app mode" (Settings > Personalisation > Colours).
fn windows_uses_light() -> bool {
    let out = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", "AppsUseLightTheme"])
        .creation_flags_no_window()
        .output();
    out.ok().map(|o| String::from_utf8_lossy(&o.stdout).contains("0x1")).unwrap_or(false)
}

trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}

impl NoWindow for std::process::Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0800_0000)
    }
}

/// `xaml` for the theme in use: as written when dark.
pub(crate) fn themed(xaml: &str) -> Cow<'_, str> {
    if light() {
        Cow::Owned(to_light(xaml))
    } else {
        Cow::Borrowed(xaml)
    }
}

/// Every `#AARRGGBB` colour in `xaml` for a light page, and the theme requests turned to Light.
pub(crate) fn to_light(xaml: &str) -> String {
    let xaml = xaml.replace(r#"RequestedTheme="Dark""#, r#"RequestedTheme="Light""#);
    let b = xaml.as_bytes();
    let mut out = String::with_capacity(xaml.len());
    let mut last = 0;
    let mut i = 0;
    while i + 9 <= b.len() {
        // a colour value starts right after a quote: "#AARRGGBB"
        if b[i] == b'#' && i > 0 && b[i - 1] == b'"' && b.get(i + 9) == Some(&b'"') && b[i + 1..i + 9].iter().all(u8::is_ascii_hexdigit) {
            let attr = attr_before(&xaml[..i - 1]);
            out.push_str(&xaml[last..i]);
            let v = u32::from_str_radix(&xaml[i + 1..i + 9], 16).unwrap_or(0);
            out.push_str(&format!("#{:08X}", flip(v, attr == "Foreground")));
            i += 9;
            last = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&xaml[last..]);
    // the window's own backdrop isn't light everywhere (inactive, remote, some drivers): the page
    // brings its own light ground, nearly opaque (after the colours, so it isn't turned around)
    out.replace(r#"x:Name="Root" RequestedTheme="Light" Background="Transparent""#, r##"x:Name="Root" RequestedTheme="Light" Background="#F2F3F4F7""##)
}

/// The attribute name a value belongs to: `Foreground` in `Foreground="#..`.
fn attr_before(s: &str) -> &str {
    let s = s.strip_suffix('=').unwrap_or(s);
    let start = s.rfind(|c: char| c.is_whitespace() || c == '<').map_or(0, |p| p + 1);
    &s[start..]
}

/// One colour for the light page. `text`: it is a Foreground.
pub(crate) fn flip(argb: u32, text: bool) -> u32 {
    let a = argb >> 24;
    let (r, g, b) = (((argb >> 16) & 255) as f32 / 255.0, ((argb >> 8) & 255) as f32 / 255.0, (argb & 255) as f32 / 255.0);
    let (h, s, l) = hsl(r, g, b);
    // muted: greys, and the slightly blue greys and dark tints the dark page is made of
    let muted = s * (1.0 - (2.0 * l - 1.0).abs()) < 0.22;
    let l2 = if muted || l < 0.3 {
        1.0 - l
    } else if text {
        // coloured text: dark enough to read on a light page
        l.min(0.38)
    } else {
        l
    };
    let (r, g, b) = rgb(h, s, l2);
    let q = |x: f32| ((x * 255.0).round().clamp(0.0, 255.0)) as u32;
    (a << 24) | (q(r) << 16) | (q(g) << 8) | q(b)
}

fn hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let t = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (t(h + 1.0 / 3.0), t(h), t(h - 1.0 / 3.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn washes_and_greys_turn_around_and_colours_stay() {
        // a white wash becomes a black one, with the same alpha
        assert_eq!(flip(0x0DFF_FFFF, false), 0x0D00_0000);
        // light grey text becomes dark grey text
        let t = flip(0xFFB9_C2D6, true);
        assert!(((t >> 8) & 255) < 0x50, "{t:08X}");
        // the accent fill and a risk colour stay
        assert_eq!(flip(0xFF2F_7BF6, false), 0xFF2F_7BF6);
        assert_eq!(flip(0xFFFF_7A45, false), 0xFFFF_7A45);
        // light blue text gets dark enough to read
        let t = flip(0xFF4C_C2FF, true);
        let (_, _, l) = hsl(((t >> 16) & 255) as f32 / 255.0, ((t >> 8) & 255) as f32 / 255.0, (t & 255) as f32 / 255.0);
        assert!(l <= 0.39, "{t:08X}");
    }

    #[test]
    fn only_colour_values_are_rewritten() {
        let x = r##"<Grid RequestedTheme="Dark" Background="#0DFFFFFF"><FontIcon Glyph="&#xE790;"/><TextBlock Text="#12345678 is text" Foreground="#FF8E97AA"/></Grid>"##;
        let y = to_light(x);
        assert!(y.contains(r#"RequestedTheme="Light""#));
        assert!(y.contains(r##"Background="#0D000000""##));
        assert!(y.contains("&#xE790;"));
        assert!(y.contains(r##"Text="#12345678 is text""##), "a colour-looking text that isn't a whole value stays");
        assert!(!y.contains("#FF8E97AA"));
    }
}
