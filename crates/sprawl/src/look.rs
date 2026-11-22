//! How the window looks, said once: figures that keep their width while they count, and the
//! one way a selectable item (a tab, a chip, a map mode) is drawn.

use windows_core::Interface;

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, FlyoutBase};
use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, Control};
use crate::bindings::Microsoft::UI::Xaml::Media::Brush;
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement, Visibility};
use crate::state::{find, keep, load, text, with, State, View};
use crate::ui;

/// On a TextBlock (or Run) whose numbers change while the user watches: every digit takes the
/// same width, so the text doesn't shift sideways as it counts.
pub(crate) const NUM: &str = r##"Typography.NumeralAlignment="Tabular""##;

/// What the window sits on, by the Backdrop option: nothing (the page's own opaque ground), Mica
/// or Acrylic (the ground is a little see-through then, see `tokens::table`). Set at start and
/// again whenever the page is made again, so a pick shows at once.
pub(crate) fn apply_backdrop(window: &crate::bindings::Microsoft::UI::Xaml::Window) {
    use crate::bindings::Microsoft::UI::Xaml::Media::{DesktopAcrylicBackdrop, MicaBackdrop, SystemBackdrop};
    use crate::tokens::Backdrop;
    let made = match crate::tokens::current().backdrop {
        Backdrop::Solid => None,
        Backdrop::Mica => MicaBackdrop::new().and_then(|b| b.cast::<SystemBackdrop>()).ok(),
        Backdrop::Acrylic => DesktopAcrylicBackdrop::new().and_then(|b| b.cast::<SystemBackdrop>()).ok(),
    };
    // a backdrop that could not be made leaves the window as it is, on its opaque-enough ground
    let _ = window.SetSystemBackdrop(made.as_ref());
}

/// The background of a selected row or tab (a list's line, the Settings sections). Chips and
/// the map's modes take the accent instead ([`Pick::bg`]).
pub(crate) fn selected() -> String {
    crate::tokens::refer("selected")
}

/// A selectable item's text at rest, as the styles set it.
fn rest_text() -> String {
    crate::tokens::refer("rest")
}

/// An accent the look draws with, the way WinUI's own accent buttons draw it: `fill` behind `on`
/// text, and `text` for accent-coloured words on the page. Written "#RRGGBB" (six digits) on
/// purpose: the light-mode rewrite in theme.rs turns only "#AARRGGBB" values around, and these
/// are already the ones for the theme in use.
pub(crate) struct Accent {
    pub fill: String,
    pub on: String,
    pub text: String,
}

/// The accent the look's tokens carry: the Theme page's choice, Windows' own by default.
pub(crate) fn accent() -> Accent {
    // a token holds an opaque alpha, and a value that is already the theme's must not be turned
    // around again by the light page's rewrite, so six digits like `windows_accent`
    let six = |v: String| v.strip_prefix("#FF").map(|c| format!("#{c}")).unwrap_or(v);
    Accent { fill: six(crate::tokens::get("accent")), on: six(crate::tokens::get("onaccent")), text: six(crate::tokens::get("accenttext")) }
}

/// The Windows accent colour (Settings > Personalisation > Colours), read once per theme (the
/// theme can change while Sprawl runs).
pub(crate) fn windows_accent() -> &'static Accent {
    static DARK: std::sync::OnceLock<Accent> = std::sync::OnceLock::new();
    static LIGHT: std::sync::OnceLock<Accent> = std::sync::OnceLock::new();
    let light = crate::theme::light();
    (if light { &LIGHT } else { &DARK }).get_or_init(|| accent_from(palette(), light))
}

/// Windows' accent palette, lightest first: Light3, Light2, Light1, the accent, Dark1, Dark2,
/// Dark3 (WinUI's accent fills and text are made from these).
fn palette() -> Option<[[u8; 3]; 7]> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY};
    let mut buf = [0u8; 32];
    let mut n = buf.len() as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent"),
            &HSTRING::from("AccentPalette"),
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut n),
        )
    };
    (r == ERROR_SUCCESS && n >= 28).then(|| std::array::from_fn(|i| [buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2]]))
}

/// Windows' own blue, for a PC that gives no palette.
const DEFAULT_PALETTE: [[u8; 3]; 7] = [[0x99, 0xEB, 0xFF], [0x4C, 0xC2, 0xFF], [0x00, 0x91, 0xF8], [0x00, 0x78, 0xD4], [0x00, 0x67, 0xC0], [0x00, 0x3E, 0x92], [0x00, 0x1A, 0x68]];

/// The seven shades of Windows' accent as opaque `#AARRGGBB` numbers, lightest first (Accent
/// strength picks among them).
pub(crate) fn palette_colours() -> [u32; 7] {
    let p = palette().unwrap_or(DEFAULT_PALETTE);
    std::array::from_fn(|i| 0xFF00_0000 | (p[i][0] as u32) << 16 | (p[i][1] as u32) << 8 | p[i][2] as u32)
}

/// The accent for a light or dark page, from the palette (Windows' own blue without one):
/// dark pages fill with Light2 and write accent text in Light3, light pages Dark1 and Dark2.
fn accent_from(p: Option<[[u8; 3]; 7]>, light: bool) -> Accent {
    let p = p.unwrap_or(DEFAULT_PALETTE);
    let hex = |c: [u8; 3]| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
    if light {
        Accent { fill: hex(p[4]), on: "#FFFFFF".to_string(), text: hex(p[5]) }
    } else {
        Accent { fill: hex(p[1]), on: "#000000".to_string(), text: hex(p[0]) }
    }
}

/// The accent's gradient as a brush: the fill on the left, a shade with 35% white mixed in on the
/// right. Both stops are six digits, so the light page's rewrite of eight-digit literals leaves
/// them as they are.
pub(crate) fn accent_gradient() -> String {
    let fill = accent().fill;
    format!(
        r##"<LinearGradientBrush StartPoint="0,0.5" EndPoint="1,0.5"><GradientStop Color="{fill}" Offset="0"/><GradientStop Color="{}" Offset="1"/></LinearGradientBrush>"##,
        crate::tokens::lighter(&fill)
    )
}

/// The fill of a button that deletes for good (and of the dialogs that warn about it).
pub(crate) fn danger() -> String {
    crate::tokens::refer("danger")
}

/// The families of selectable items: they differ only in their background at rest.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Pick {
    /// The tab row: nothing at rest.
    Tab,
    /// The Advisor's and Search's chips: a faint pill at rest.
    Chip,
    /// The map's mode buttons, on their dark strip.
    Mode,
    /// The page tabs (Tree View ... Search): plain text, the open one underlined.
    Page,
}

/// The page tabs are drawn as pills (the Tabs option) rather than with a line under them.
fn pill_tabs() -> bool {
    crate::tokens::current().tabs == crate::tokens::Tabs::Pills
}

/// The underline under the open page tab: the accent.
pub(crate) fn underline() -> String {
    accent().fill
}

impl Pick {
    pub(crate) fn rest(self) -> String {
        match self {
            Pick::Tab => "Transparent".to_string(),
            Pick::Chip => crate::tokens::refer("hover"),
            Pick::Mode => crate::tokens::refer("panel"),
            Pick::Page => "Transparent".to_string(),
        }
    }

    /// Its background, selected or not: a selected chip or map mode is filled with the accent,
    /// like WinUI's toggle buttons; a selected row or tab is quieter, so its grey second line
    /// stays readable.
    pub(crate) fn bg(self, on: bool) -> String {
        match (self, on) {
            (Pick::Chip | Pick::Mode, true) => accent().fill,
            (Pick::Tab, true) => selected(),
            _ => self.rest(),
        }
    }

    /// Its text colour when it differs from the style's: on the accent fill.
    pub(crate) fn fg(self, on: bool) -> Option<String> {
        (on && matches!(self, Pick::Chip | Pick::Mode)).then(|| accent().on)
    }

    fn style(self) -> &'static str {
        match self {
            Pick::Chip => "SprawlChip",
            Pick::Mode => "SprawlMode",
            Pick::Page => "SprawlPage",
            _ => "SprawlPick",
        }
    }
}

/// The attributes for one of the main page tabs (Tree View ... Search): like [`pick`] for
/// `Pick::Page`, but with no line of its own. Under the underline option the one line under the
/// row is `tabline.rs`'s, which slides from tab to tab; the pills option is the same as a page tab's.
pub(crate) fn pick_nav(on: bool) -> String {
    if pill_tabs() {
        return pick(Pick::Page, on);
    }
    r##"Style="{StaticResource SprawlNav}" Background="Transparent" BorderBrush="Transparent""##.to_string()
}

/// Light or unlight a main page tab already on screen: only the pills have anything to paint.
pub(crate) fn paint_nav(root: &FrameworkElement, name: &str, on: bool) {
    if pill_tabs() {
        paint(root, name, Pick::Page, on);
    }
}

/// The attributes for a `<Button>` drawn as a selectable item.
pub(crate) fn pick(kind: Pick, on: bool) -> String {
    if kind == Pick::Page && pill_tabs() {
        // a picked chip: the accent behind its text, nothing under it
        let (bg, fg) = if on { (accent().fill, accent().on) } else { ("Transparent".to_string(), rest_text()) };
        return format!(r##"Style="{{StaticResource SprawlPagePill}}" Background="{bg}" Foreground="{fg}" BorderBrush="Transparent""##);
    }
    if kind == Pick::Page {
        return format!(r##"Style="{{StaticResource SprawlPage}}" Background="Transparent" BorderBrush="{}""##, if on { underline() } else { "Transparent".to_string() });
    }
    // written off too, so a chip picked or unpicked is changed in place (an attribute that goes
    // has the control loaded again: see reconcile.rs)
    let fg = match kind {
        Pick::Chip | Pick::Mode => format!(r#" Foreground="{}""#, kind.fg(on).unwrap_or(rest_text())),
        _ => kind.fg(on).map(|c| format!(r#" Foreground="{c}""#)).unwrap_or_default(),
    };
    format!(r##"Style="{{StaticResource {}}}" Background="{}"{fg}"##, kind.style(), kind.bg(on))
}

/// The same for a `<DropDownButton>` (its arrow is part of the look).
pub(crate) fn pick_drop(kind: Pick, on: bool) -> String {
    format!(r##"Style="{{StaticResource SprawlPickDrop}}" Background="{}""##, kind.bg(on))
}

/// Light or unlight the selectable item `name` (a tab, a mode) that is already on screen.
pub(crate) fn paint(root: &FrameworkElement, name: &str, kind: Pick, on: bool) {
    let Ok(c) = find::<Control>(root, name) else { return };
    if kind == Pick::Page && pill_tabs() {
        let (bg, fg) = if on { (accent().fill, accent().on) } else { ("#00000000".to_string(), rest_text()) };
        for (fill, set) in [(bg, true), (fg, false)] {
            if let Ok(br) = load::<Brush>(&format!(r##"<SolidColorBrush {} Color="{fill}"/>"##, ui::NS)) {
                let _ = if set { c.SetBackground(&br) } else { c.SetForeground(&br) };
            }
        }
        return;
    }
    if kind == Pick::Page {
        // the underline is the template's line, in the tab's BorderBrush
        if let Ok(br) = load::<Brush>(&format!(r##"<SolidColorBrush {} Color="{}"/>"##, ui::NS, if on { underline() } else { "#00000000".to_string() })) {
            let _ = c.SetBorderBrush(&br);
        }
        return;
    }
    if let Ok(br) = load::<Brush>(&format!(r##"<SolidColorBrush {} Color="{}"/>"##, ui::NS, kind.bg(on))) {
        let _ = c.SetBackground(&br);
    }
    // the text follows: dark on the accent, the style's own colour off it
    if matches!(kind, Pick::Chip | Pick::Mode) {
        if let Ok(br) = load::<Brush>(&format!(r##"<SolidColorBrush {} Color="{}"/>"##, ui::NS, kind.fg(on).unwrap_or(rest_text()))) {
            let _ = c.SetForeground(&br);
        }
    }
}

/// A tag on a row that says what kind of thing it is ("Delete", "How to"): plain words in the
/// row's quiet colour, with no background or border, so it does not read as a button.
pub(crate) fn label(text: &str) -> String {
    format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" VerticalAlignment="Center" Margin="4,0,0,0"/>"##, ui::esc(text))
}

/// Where a real drop shadow goes: an empty, see-through border that [`crate::shade`] finds by
/// its `Tag` once the XAML is loaded and hangs a Composition `DropShadow` on, clipped to the
/// outside of the border's rounded shape (as a CSS box-shadow, never drawn under the item). Put
/// it in the item's own cell BEFORE the item, the same size and corner as the item, so the shadow
/// is drawn behind it. `kind` is one of [`crate::shade::Kind`]'s tags.
pub(crate) fn shade_host(kind: &str, corner: &str) -> String {
    format!(r##"<Border Tag="shade:{kind}" CornerRadius="{corner}" IsHitTestVisible="False"/>"##)
}

/// The shadow under a card or pane (0 8px 22px), to put in the card's own cell BEFORE the card.
/// A dialog's card (`raised`) floats over the page as a menu does: 0 10px 28px. Nothing when
/// the finish is Flat.
pub(crate) fn shadow(raised: bool) -> String {
    if flat() {
        return String::new();
    }
    if raised {
        shade_host("dialog", &crate::tokens::rc(14.0))
    } else {
        shade_host("card", &crate::tokens::rc(8.0))
    }
}

/// How a button's shadow follows the pointer, as the Press option says: whether it lifts (a
/// bigger shadow under the pointer, none while pressed: Lift and sink only), and how long the
/// change takes in milliseconds (the option's own time, longer when Lively, none when the motion
/// is None or Windows' animation effects are off). CSS eases a shadow with `ease`.
pub(crate) fn shadow_motion() -> (bool, u32) {
    let feel = press_feel(crate::tokens::current().press);
    (feel.shadow, state_ms(motion(), feel.ms))
}

/// The lit top edge of the Soft finish: a 1 px line along the top, as if light came from above,
/// that follows the item's top corners. `token` is one of the `hl` colours. In a template it
/// goes inside the item's border (`inside`); on a card it goes after the card, one pixel in from
/// the card's own border. Nothing when the finish is Flat.
pub(crate) fn top_light(token: &str, corner: &str, inside: bool) -> String {
    if flat() {
        return String::new();
    }
    top_light_xaml(token, corner, inside)
}

/// [`top_light`] whatever the finish is.
pub(crate) fn top_light_xaml(token: &str, corner: &str, inside: bool) -> String {
    let margin = if inside { "" } else { r#" Margin="1,1,1,0""# };
    format!(r##"<Border{margin} CornerRadius="{corner}" BorderThickness="0,1,0,0" BorderBrush="{}" IsHitTestVisible="False"/>"##, crate::tokens::refer(token))
}

/// The Finish option is Flat: no shadow, no glow and no lit edge.
fn flat() -> bool {
    crate::tokens::current().finish == crate::tokens::Finish::Flat
}

/// The motion the buttons are drawn with: Windows' "Animation effects" off wins over the look.
pub(crate) fn motion() -> crate::tokens::Motion {
    if crate::win32::animations_on() {
        crate::tokens::current().motion
    } else {
        crate::tokens::Motion::None
    }
}

/// How long a state takes to arrive: the press option's own time (`ms`), longer when Lively,
/// none when the motion is None.
fn state_time(motion: crate::tokens::Motion, ms: u32) -> String {
    format!("0:0:{:.3}", state_ms(motion, ms) as f64 / 1000.0)
}

/// [`state_time`] in milliseconds.
fn state_ms(motion: crate::tokens::Motion, ms: u32) -> u32 {
    match motion {
        crate::tokens::Motion::Subtle => ms,
        crate::tokens::Motion::Lively => ms * 11 / 7,
        crate::tokens::Motion::None => 0,
    }
}

/// The travel a state makes under `motion`: Lively goes twice as far and presses twice as deep,
/// None keeps the content still so only the wash changes.
fn travel(motion: crate::tokens::Motion, lift: f64, scale: f64) -> (f64, f64) {
    match motion {
        crate::tokens::Motion::Subtle => (lift, scale),
        crate::tokens::Motion::Lively => (lift * 2.0, (1.0 - (1.0 - scale) * 2.0).max(0.9)),
        crate::tokens::Motion::None => (0.0, 1.0),
    }
}

/// One state of a button: the white wash, the black wash, and the content's lift (negative Y is
/// up) and scale.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Pose {
    white: f64,
    black: f64,
    lift: f64,
    scale: f64,
}

/// What the Press option does, as the mockup's four modes: the state under the pointer and the
/// pressed one, the time and the cubic-bezier they take, and whether a shadow comes up under the
/// pointer (Lift and sink only). Pressing washes black, except in the faintest mode.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Feel {
    over: Pose,
    down: Pose,
    ms: u32,
    /// The content's movement: CSS's cubic-bezier (x1, y1, x2, y2). Press in's goes past 1 to
    /// overshoot, which a XAML KeySpline can't take, so [`pieces`] cuts it where it turns.
    spline: [f64; 4],
    shadow: bool,
}

/// CSS's `ease`, which the mockup's washes take whatever the press option is.
const EASE: [f64; 4] = [0.25, 0.1, 0.25, 1.0];

/// A number for XAML: at most four decimals, no trailing zeros.
fn num(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_string() } else { s.to_string() }
}

fn spline_text(s: [f64; 4]) -> String {
    format!("{},{},{},{}", num(s[0]), num(s[1]), num(s[2]), num(s[3]))
}

/// A point on the cubic bezier from (0,0) to (1,1) with control points `s`, at parameter `t`.
fn bezier_at(s: [f64; 4], t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let f = |a: f64, b: f64| 3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t;
    (f(s[0], s[2]), f(s[1], s[3]))
}

/// The curve `s` as consecutive KeySpline pieces that each stay inside 0..1, as XAML requires:
/// (where the piece ends in time, 0..1; where it ends in value, past 1 at an overshoot; its own
/// spline). A curve that stays inside is one piece. One that overshoots (or undershoots) is cut
/// where it turns, which is exact: the pieces together are the same curve (de Casteljau).
fn pieces(s: [f64; 4]) -> Vec<(f64, f64, [f64; 4])> {
    let inside = |v: f64| (0.0..=1.0).contains(&v);
    if inside(s[1]) && inside(s[3]) {
        return vec![(1.0, 1.0, s)];
    }
    // where dy/dt is 0: (3y1 - 3y2 + 1)t² + (2y2 - 4y1)t + y1 = 0
    let (y1, y2) = (s[1], s[3]);
    let (a, b, c) = (3.0 * y1 - 3.0 * y2 + 1.0, 2.0 * y2 - 4.0 * y1, y1);
    let mut turns: Vec<f64> = if a.abs() < 1e-12 {
        if b.abs() < 1e-12 { vec![] } else { vec![-c / b] }
    } else {
        let d = b * b - 4.0 * a * c;
        if d < 0.0 { vec![] } else { vec![(-b - d.sqrt()) / (2.0 * a), (-b + d.sqrt()) / (2.0 * a)] }
    };
    turns.retain(|t| *t > 1e-6 && *t < 1.0 - 1e-6 && !inside(bezier_at(s, *t).1));
    turns.sort_by(|a, b| a.total_cmp(b));
    // split the control polygon at each turn, in the original curve's coordinates
    let lerp = |p: (f64, f64), q: (f64, f64), t: f64| (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
    let mut rest = [(0.0, 0.0), (s[0], s[1]), (s[2], s[3]), (1.0, 1.0)];
    let mut done = 0.0;
    let mut out = Vec::new();
    for t in turns.into_iter().chain(std::iter::once(1.0)) {
        let local = (t - done) / (1.0 - done);
        let [p0, p1, p2, p3] = rest;
        let (a1, b1, c1) = (lerp(p0, p1, local), lerp(p1, p2, local), lerp(p2, p3, local));
        let (a2, b2) = (lerp(a1, b1, local), lerp(b1, c1, local));
        let m = lerp(a2, b2, local);
        let piece = [p0, a1, a2, m];
        rest = [m, b2, c1, p3];
        done = t;
        // the piece in its own box: time and value from its start to its end
        let (x0, v0, x1, v1) = (piece[0].0, piece[0].1, piece[3].0, piece[3].1);
        let nx = |x: f64| ((x - x0) / (x1 - x0)).clamp(0.0, 1.0);
        let ny = |y: f64| ((y - v0) / (v1 - v0)).clamp(0.0, 1.0);
        out.push((x1, v1, [nx(piece[1].0), ny(piece[1].1), nx(piece[2].0), ny(piece[2].1)]));
    }
    out
}

/// `s` with its values held inside 0..1, for a storyboard that doesn't know where it starts from
/// (a state's own): the overshoot itself is played by the transitions between known states.
fn safe(s: [f64; 4]) -> [f64; 4] {
    [s[0], s[1].clamp(0.0, 1.0), s[2], s[3].clamp(0.0, 1.0)]
}

/// The key frames that take a value from `from` to `to` in `ms` along `s`, overshoot included.
fn frames(from: f64, to: f64, ms: u32, s: [f64; 4]) -> String {
    pieces(s)
        .into_iter()
        .map(|(x, v, sp)| {
            format!(
                r##"<SplineDoubleKeyFrame KeyTime="0:0:{:.3}" Value="{}" KeySpline="{}"/>"##,
                x * f64::from(ms) / 1000.0,
                num(from + (to - from) * v),
                spline_text(sp)
            )
        })
        .collect()
}

fn press_feel(press: crate::tokens::Press) -> Feel {
    use crate::tokens::Press;
    let state = |white: f64, black: f64, lift: f64, scale: f64| Pose { white, black, lift, scale };
    match press {
        // up 1 px with a shadow under the pointer, down 1 px at 0.98 when pressed
        Press::LiftSink => Feel { over: state(0.10, 0.0, -1.0, 1.0), down: state(0.0, 0.20, 1.0, 0.98), ms: 140, spline: [0.2, 0.8, 0.2, 1.0], shadow: true },
        // no lift, and a sink to 0.95 that overshoots a little
        Press::PressIn => Feel { over: state(0.13, 0.0, 0.0, 1.0), down: state(0.0, 0.22, 0.0, 0.95), ms: 130, spline: [0.2, 0.9, 0.3, 1.35], shadow: false },
        // no movement: only the wash fades in
        Press::Fade => Feel { over: state(0.13, 0.0, 0.0, 1.0), down: state(0.0, 0.22, 0.0, 1.0), ms: 90, spline: EASE, shadow: false },
        // a very small wash, white both ways, at once
        Press::Faint => Feel { over: state(0.07, 0.0, 0.0, 1.0), down: state(0.12, 0.0, 0.0, 1.0), ms: 0, spline: [0.0, 0.0, 1.0, 1.0], shadow: false },
    }
}

/// One animation of a state's storyboard: `target`'s `prop` to `to`, with the press option's
/// own cubic-bezier (a key frame's KeySpline is exactly one; the overshoot of Press in goes past
/// the value on purpose).
fn animate(target: &str, prop: &str, to: f64, time: &str, spline: &str) -> String {
    format!(
        r##"<DoubleAnimationUsingKeyFrames Storyboard.TargetName="{target}" Storyboard.TargetProperty="{prop}"><SplineDoubleKeyFrame KeyTime="{time}" Value="{to}" KeySpline="{spline}"/></DoubleAnimationUsingKeyFrames>"##
    )
}

/// One state's storyboard, shared by every button: the washes over the item, and the content's
/// lift (negative Y is up) and scale. (A shadow is not XAML: [`crate::shade`] follows the same
/// states in code.)
fn states(feel: &Feel, state: Pose) -> String {
    states_with(motion(), feel, state)
}

/// [`states`] under one motion, so both the look's option and Windows' animations setting reach
/// the same storyboard.
fn states_with(motion: crate::tokens::Motion, feel: &Feel, state: Pose) -> String {
    let d = state_time(motion, feel.ms);
    let (lift, scale) = travel(motion, state.lift, state.scale);
    let t = "(UIElement.RenderTransform).(CompositeTransform.";
    // the washes fade on CSS's ease, as the mockup's do; the content moves on the press's curve
    let (wash, moves) = (spline_text(EASE), spline_text(safe(feel.spline)));
    format!(
        "<Storyboard>{}{}{}{}{}</Storyboard>",
        animate("Wash", "Opacity", state.white, &d, &wash),
        animate("WashDown", "Opacity", state.black, &d, &wash),
        animate("Content", &format!("{t}TranslateY)"), lift, &d, &moves),
        animate("Content", &format!("{t}ScaleX)"), scale, &d, &moves),
        animate("Content", &format!("{t}ScaleY)"), scale, &d, &moves),
    )
}

/// The transitions between the pointer states, for a press curve that overshoots: a state's own
/// storyboard doesn't know where it starts from, so the overshoot (a value past where it ends) is
/// played here, between two known states, as [`frames`]. Nothing for a curve that stays inside,
/// or with no motion.
fn transitions_with(motion: crate::tokens::Motion, feel: &Feel, poses: &[(&str, Pose)]) -> String {
    let ms = state_ms(motion, feel.ms);
    if ms == 0 || pieces(feel.spline).len() == 1 {
        return String::new();
    }
    let d = state_time(motion, feel.ms);
    let wash = spline_text(EASE);
    let t = "(UIElement.RenderTransform).(CompositeTransform.";
    let mut out = String::new();
    for (from_name, from) in poses {
        for (to_name, to) in poses {
            let (a, b) = (travel(motion, from.lift, from.scale), travel(motion, to.lift, to.scale));
            if from_name == to_name || a == b {
                continue;
            }
            let moving = |prop: &str, x: f64, y: f64| {
                format!(r##"<DoubleAnimationUsingKeyFrames Storyboard.TargetName="Content" Storyboard.TargetProperty="{t}{prop})">{}</DoubleAnimationUsingKeyFrames>"##, frames(x, y, ms, feel.spline))
            };
            out += &format!(
                r##"<VisualTransition From="{from_name}" To="{to_name}"><Storyboard>{}{}{}{}{}</Storyboard></VisualTransition>"##,
                animate("Wash", "Opacity", to.white, &d, &wash),
                animate("WashDown", "Opacity", to.black, &d, &wash),
                moving("TranslateY", a.0, b.0),
                moving("ScaleX", a.1, b.1),
                moving("ScaleY", a.1, b.1),
            );
        }
    }
    format!("<VisualStateGroup.Transitions>{out}</VisualStateGroup.Transitions>")
}

/// What a style adds to a button's flat look.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
struct Depth {
    /// The main button: a wash a little stronger than the others.
    primary: bool,
    /// The drop shadow behind the button, by its kind (a [`crate::shade::Kind`] tag): a host
    /// border, `Shade`, under `Root` in an outer grid. Only for a button with a face.
    shade: Option<&'static str>,
    /// The lit top edge: the name of its colour token.
    light: Option<&'static str>,
}

/// The template both selectable styles share: the item's own background (set by `paint` or
/// `pick`: at rest or selected), a white wash over it for the pointer and a black one while
/// pressed (the Press option's values), the content's travel by the look's motion (Subtle, today,
/// is up 1 px, then down 1 px and at 0.98; None keeps it still), and the system's focus
/// rectangle, which shows for the keyboard only. So selected-and-pointer-over is the selected
/// background with the faint wash, and a selected item the pointer has left is the selected
/// background alone. With `line`: a 3 px line under the content in the BorderBrush (the page
/// tabs; they have no border of their own). `depth` adds the Soft finish's lit edge and
/// shadows, which Flat leaves out.
fn template_with(target: &str, arrow: bool, line: bool, grey_off: bool, depth: Depth) -> String {
    let (cols, arrow) = if arrow {
        (
            r##"<Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>"##,
            r##"<FontIcon x:Name="Arrow" Grid.Column="1" Glyph="&#xE70D;" FontSize="{$fs8}" Margin="2,0,10,0" VerticalAlignment="Center" Foreground="{TemplateBinding Foreground}"/>"##,
        )
    } else {
        ("", "")
    };
    let (border, underline) = if line {
        // the line is the tab's BorderBrush: opening a tab swaps it for the accent, closing it for
        // nothing, and the brush fades from one to the other (nothing when motion is off)
        let fade = match motion() {
            crate::tokens::Motion::None => String::new(),
            m => format!(r##"<Border.BackgroundTransition><BrushTransition Duration="{}"/></Border.BackgroundTransition>"##, line_time(m)),
        };
        ("", format!(r##"<Border x:Name="Line" Grid.ColumnSpan="2" VerticalAlignment="Bottom" Height="3" Margin="12,0,12,0" CornerRadius="{{$rc2}}" Background="{{TemplateBinding BorderBrush}}" IsHitTestVisible="False">{fade}</Border>"##))
    } else {
        (r##"BorderBrush="{TemplateBinding BorderBrush}" BorderThickness="{TemplateBinding BorderThickness}""##, String::new())
    };
    // a filled button (the red one) turns grey when it can't be pressed, as WinUI's do; the
    // rest just fade
    let mut disabled = if grey_off {
        r##"<Setter Target="Root.Background" Value="{$hover}"/><Setter Target="Content.Foreground" Value="{$disabled}"/>"##.to_string()
    } else {
        r##"<Setter Target="Content.Opacity" Value="0.4"/>"##.to_string()
    };
    let feel = press_feel(crate::tokens::current().press);
    let (mut over, mut down) = (feel.over, feel.down);
    if depth.primary && crate::tokens::current().press != crate::tokens::Press::Faint {
        // the main button's wash is a little stronger than the others
        over.white = 0.16;
        down.black = 0.26;
    }
    // the drop shadow (the Soft finish): a host border behind Root that shade.rs hangs a
    // Composition shadow on, and moves with the states below (CommonStates) in code
    let shades = match depth.shade {
        Some(kind) if !flat() => shade_host(kind, "{TemplateBinding CornerRadius}").replacen("<Border ", r##"<Border x:Name="Shade" "##, 1),
        _ => String::new(),
    };
    let lit = match depth.light {
        Some(token) if !flat() => {
            disabled += r##"<Setter Target="Light.Opacity" Value="0"/>"##;
            top_light_xaml(token, "{TemplateBinding CornerRadius}", true).replacen("<Border ", r##"<Border x:Name="Light" Grid.ColumnSpan="2" "##, 1)
        }
        _ => String::new(),
    };
    let rest = Pose { white: 0.0, black: 0.0, lift: 0.0, scale: 1.0 };
    let moves = transitions_with(motion(), &feel, &[("Normal", rest), ("PointerOver", over), ("Pressed", down)]);
    let (normal, over, pressed) = (states(&feel, rest), states(&feel, over), states(&feel, down));
    // WinUI runs a template's states only from its outermost element: Root, or the grid that
    // holds the shadow's host and Root
    let vsm = format!(
        r##"<VisualStateManager.VisualStateGroups>
      <VisualStateGroup x:Name="CommonStates">
        {moves}
        <VisualState x:Name="Normal"><VisualState.Storyboard>{normal}</VisualState.Storyboard></VisualState>
        <VisualState x:Name="PointerOver"><VisualState.Storyboard>{over}</VisualState.Storyboard></VisualState>
        <VisualState x:Name="Pressed"><VisualState.Storyboard>{pressed}</VisualState.Storyboard></VisualState>
        <VisualState x:Name="Disabled"><VisualState.Setters>{disabled}</VisualState.Setters></VisualState>
      </VisualStateGroup>
    </VisualStateManager.VisualStateGroups>"##
    );
    let (inner_states, outer_states, open, close) = if shades.is_empty() { (vsm, String::new(), String::new(), "") } else { (String::new(), vsm, format!("<Grid>{shades}"), "</Grid>") };
    format!(
        r##"<ControlTemplate TargetType="{target}">
  {open}
  <Grid x:Name="Root" Background="{{TemplateBinding Background}}" CornerRadius="{{TemplateBinding CornerRadius}}" {border}>
    {cols}
    {lit}
    <Border x:Name="Wash" Grid.ColumnSpan="2" Background="{{$wash}}" Opacity="0" CornerRadius="{{TemplateBinding CornerRadius}}"/>
    <Border x:Name="WashDown" Grid.ColumnSpan="2" Background="{{$washdown}}" Opacity="0" CornerRadius="{{TemplateBinding CornerRadius}}"/>
    <ContentPresenter x:Name="Content" RenderTransformOrigin="0.5,0.5" Content="{{TemplateBinding Content}}" ContentTemplate="{{TemplateBinding ContentTemplate}}" Padding="{{TemplateBinding Padding}}" HorizontalContentAlignment="{{TemplateBinding HorizontalContentAlignment}}" VerticalContentAlignment="{{TemplateBinding VerticalContentAlignment}}" Foreground="{{TemplateBinding Foreground}}" FontSize="{{TemplateBinding FontSize}}" FontFamily="{{TemplateBinding FontFamily}}" FontWeight="{{TemplateBinding FontWeight}}"><ContentPresenter.RenderTransform><CompositeTransform/></ContentPresenter.RenderTransform></ContentPresenter>
    {arrow}
    {underline}
    {inner_states}
  </Grid>
  {outer_states}
  {close}
</ControlTemplate>"##
    )
}

/// How long the tab line takes to change hands, a little longer than a button's state.
fn line_time(motion: crate::tokens::Motion) -> &'static str {
    match motion {
        crate::tokens::Motion::Subtle => "0:0:0.16",
        crate::tokens::Motion::Lively => "0:0:0.24",
        crate::tokens::Motion::None => "0:0:0",
    }
}

/// The size and colours a style sets; a button's own attributes still win.
struct Geo {
    padding: &'static str,
    corner: String,
    size: String,
    weight: &'static str,
    /// The border: its width and colour (a transparent one keeps the height of a WinUI button).
    border: (&'static str, String),
    background: String,
    foreground: String,
    /// What the Soft finish adds: shadows and the lit edge.
    depth: Depth,
}

impl Geo {
    /// The selectable items as they were: small, no border.
    fn small() -> Self {
        Geo { padding: "10,3", corner: crate::tokens::rc(6.0), size: crate::tokens::fs(12.0), weight: "Normal", border: ("0", "Transparent".to_string()), background: "Transparent".to_string(), foreground: rest_text(), depth: Depth::default() }
    }

    /// A WinUI button's size: 14 px text, 4 px corners, 32 px high with its 1 px border.
    fn button() -> Self {
        Geo { padding: "11,5,11,6", corner: crate::tokens::rc(4.0), size: crate::tokens::fs(14.0), ..Geo::small() }.with_border("1", "Transparent".to_string())
    }

    fn with_border(mut self, width: &'static str, colour: String) -> Self {
        self.border = (width, colour);
        self
    }
}

fn style(key: &str, target: &str, arrow: bool) -> String {
    style_with(key, target, arrow, false, false, Geo::small())
}

fn style_with(key: &str, target: &str, arrow: bool, line: bool, grey_off: bool, g: Geo) -> String {
    format!(
        r##"<Style x:Key="{key}" TargetType="{target}">
  <Setter Property="Background" Value="{}"/>
  <Setter Property="Foreground" Value="{}"/>
  <Setter Property="BorderThickness" Value="{}"/>
  <Setter Property="BorderBrush" Value="{}"/>
  <Setter Property="Padding" Value="{}"/>
  <Setter Property="CornerRadius" Value="{}"/>
  <Setter Property="FontSize" Value="{}"/>
  <Setter Property="FontWeight" Value="{}"/>
  <Setter Property="HorizontalContentAlignment" Value="Center"/>
  <Setter Property="VerticalContentAlignment" Value="Center"/>
  <Setter Property="UseSystemFocusVisuals" Value="True"/>
  <Setter Property="FocusVisualMargin" Value="-2"/>
  <Setter Property="Template"><Setter.Value>{}</Setter.Value></Setter>
</Style>"##,
        g.background,
        g.foreground,
        g.border.0,
        g.border.1,
        g.padding,
        g.corner,
        g.size,
        g.weight,
        template_with(target, arrow, line, grey_off, g.depth)
    )
}

/// A stock control's themed brush, replaced by one of the look's colours. WinUI draws the control
/// from these keys; a key of the same name in a dictionary merged after `XamlControlsResources`
/// wins over both of its pages, so one value is enough and no ThemeDictionaries is needed.
fn brush(key: &str, token: &str) -> String {
    // "none" is no colour at all: a line that is not drawn
    let colour = if token == "none" { "Transparent".to_string() } else { crate::tokens::refer(token) };
    format!(r#"<SolidColorBrush x:Key="{key}" Color="{colour}"/>"#)
}

/// A stock control's corner radius, from one of the look's radius tokens. The radius is element
/// content, where a token cannot be written, so it is read from the table as it stands.
fn corner(key: &str, token: &str) -> String {
    format!(r#"<CornerRadius x:Key="{key}">{}</CornerRadius>"#, crate::tokens::get(token))
}

/// A stock control's thickness, likewise element content: one pixel on every side.
fn thickness(key: &str) -> String {
    format!(r#"<Thickness x:Key="{key}">1</Thickness>"#)
}

/// The stock controls (a text box, a combo box, a flyout, a check box, a switch, a button) drawn
/// with the look's colours instead of WinUI's greys: the key names are WinUI 3's own, checked
/// against the Windows App SDK's generic.xaml. A key the system does not know would simply do
/// nothing, so each one is here for a control the app actually shows.
fn stock() -> String {
    let mut x = String::new();
    // the corners of every stock control, and of the panels that float over the page
    for (key, token) in [("ControlCornerRadius", "rc4"), ("OverlayCornerRadius", "rc8")] {
        x += &corner(key, token);
    }
    // the Controls option: outlined (the page's ground under a field, a hairline round it), soft
    // (the row's fill and no line until the pointer or the focus comes), or WinUI's own, where
    // none of the field keys below, nor the text box's template, is ours
    let controls = crate::tokens::current().controls;
    let (field, edge, hover) = if controls == crate::tokens::Controls::Soft { ("row", "none", "line2") } else { ("ground", "line2", "edge") };
    let fields = controls != crate::tokens::Controls::Stock;
    // a text box: the page's ground under it, a hairline that firms up under the pointer and
    // takes the accent with the focus; its placeholder is the dimmest text
    for (key, token) in [
        ("TextControlBackground", field),
        ("TextControlBackgroundPointerOver", field),
        ("TextControlBackgroundFocused", field),
        ("TextControlBorderBrush", edge),
        ("TextControlBorderBrushPointerOver", hover),
        ("TextControlBorderBrushFocused", "accent"),
        ("TextControlForeground", "text1"),
        ("TextControlForegroundPointerOver", "text1"),
        ("TextControlForegroundFocused", "text1"),
        ("TextControlPlaceholderForeground", "text3"),
        ("TextControlPlaceholderForegroundPointerOver", "text3"),
        ("TextControlPlaceholderForegroundFocused", "text3"),
    ] {
        if fields {
            x += &brush(key, token);
        }
    }
    // and it keeps a 1 px line whether or not it has the focus (WinUI draws 2 px under it)
    for key in ["TextControlBorderThemeThickness", "TextControlBorderThemeThicknessFocused"] {
        if fields {
            x += &thickness(key);
        }
    }
    // a combo box: the field on the ground, its list on the menu colour
    for (key, token) in [
        ("ComboBoxBackground", field),
        ("ComboBoxBackgroundPointerOver", if field == "row" { "rowopen" } else { "row" }),
        ("ComboBoxBackgroundPressed", "card"),
        ("ComboBoxBorderBrush", edge),
        ("ComboBoxBorderBrushPointerOver", hover),
        ("ComboBoxForeground", "text1"),
        ("ComboBoxDropDownBackground", "menu"),
        ("ComboBoxDropDownBorderBrush", "line1"),
        // WinUI's own halo behind a focused combo box (stroke and fill, 4 px out), in the ring's
        // colour: the soft ring, with no template of ours
        ("ComboBoxBackgroundFocused", "ring"),
        ("ComboBoxBackgroundBorderBrushFocused", "ring"),
    ] {
        if fields {
            x += &brush(key, token);
        }
    }
    // flyouts, menus and tooltips: the menu colour inside a hairline
    for (key, token) in [
        ("FlyoutPresenterBackground", "menu"),
        ("FlyoutBorderThemeBrush", "line1"),
        ("MenuFlyoutPresenterBackground", "menu"),
        ("MenuFlyoutPresenterBorderBrush", "line1"),
        ("MenuFlyoutItemBackgroundPointerOver", "hover"),
        ("MenuFlyoutItemBackgroundPressed", "pressed"),
        ("ToolTipBackground", "menu"),
        ("ToolTipBorderBrush", "line1"),
        ("ToolTipForeground", "text1"),
    ] {
        x += &brush(key, token);
    }
    // a check box: the accent behind the tick, its glyph on the accent, a hairline when unchecked
    for (key, token) in [
        ("CheckBoxCheckBackgroundFillChecked", "accent"),
        ("CheckBoxCheckBackgroundFillCheckedPointerOver", "accent"),
        ("CheckBoxCheckBackgroundFillCheckedPressed", "accent"),
        ("CheckBoxCheckBackgroundStrokeUnchecked", "line2"),
        ("CheckBoxCheckBackgroundStrokeUncheckedPointerOver", "edge"),
        ("CheckBoxCheckGlyphForegroundChecked", "onaccent"),
        ("CheckBoxCheckGlyphForegroundCheckedPointerOver", "onaccent"),
        ("CheckBoxCheckGlyphForegroundCheckedPressed", "onaccent"),
    ] {
        x += &brush(key, token);
    }
    // a switch: the track on a row, its knob in the second text colour; the accent when on
    for (key, token) in [
        ("ToggleSwitchFillOff", "row"),
        ("ToggleSwitchStrokeOff", "line2"),
        ("ToggleSwitchKnobFillOff", "text2"),
        ("ToggleSwitchFillOn", "accent"),
        ("ToggleSwitchFillOnPointerOver", "accent"),
        ("ToggleSwitchFillOnPressed", "accent"),
        ("ToggleSwitchKnobFillOn", "onaccent"),
        ("ToggleSwitchKnobFillOnPointerOver", "onaccent"),
        ("ToggleSwitchKnobFillOnPressed", "onaccent"),
    ] {
        x += &brush(key, token);
    }
    // the keyboard's focus ring, which WinUI draws in two strokes: the accent outside, the page
    // inside it (generic.xaml's own names for the pair)
    for (key, token) in [
        ("SystemControlFocusVisualPrimaryBrush", "accent"),
        ("SystemControlFocusVisualSecondaryBrush", "ground"),
    ] {
        x += &brush(key, token);
    }
    // a plain button: a row's fill, a step up under the pointer, a card while pressed
    for (key, token) in [
        ("ButtonBackground", "row"),
        ("ButtonBackgroundPointerOver", "rowopen"),
        ("ButtonBackgroundPressed", "card"),
        ("ButtonBorderBrush", "line2"),
        ("ButtonBorderBrushPointerOver", "edge"),
        ("ButtonForeground", "text1"),
        ("ButtonForegroundPointerOver", "text1"),
        ("ButtonForegroundPressed", "text1"),
    ] {
        x += &brush(key, token);
    }
    // (AccentButtonStyle's own setters are read where WinUI defines them and never see these,
    // so SprawlPrimary sets its colours itself; its pointer and press states do read them.)
    // WinUI's own accent brushes, so a primary button, a progress bar and accent-coloured words take the look's accent rather than Windows'
    for (key, token) in [
        ("AccentButtonBackground", "accent"),
        ("AccentButtonBackgroundPointerOver", "accent"),
        ("AccentButtonBackgroundPressed", "accent"),
        ("AccentButtonBorderBrush", "accent"),
        ("AccentButtonBorderBrushPointerOver", "accent"),
        ("AccentButtonBorderBrushPressed", "accent"),
        ("AccentButtonForeground", "onaccent"),
        ("AccentButtonForegroundPointerOver", "onaccent"),
        ("AccentButtonForegroundPressed", "onaccent"),
        ("AccentFillColorDefaultBrush", "accent"),
        ("AccentFillColorSecondaryBrush", "accent"),
        ("AccentFillColorTertiaryBrush", "accent"),
        ("TextOnAccentFillColorPrimaryBrush", "onaccent"),
        ("TextOnAccentFillColorSecondaryBrush", "onaccent"),
        ("AccentTextFillColorPrimaryBrush", "accenttext"),
        ("AccentTextFillColorSecondaryBrush", "accenttext"),
        ("AccentTextFillColorTertiaryBrush", "accenttext"),
        ("ProgressBarForeground", "accent"),
    ] {
        x += &brush(key, token);
    }
    // a round tick box: WinUI's check box binds its box's RadiusX/RadiusY to the control's own
    // CornerRadius (generic.xaml), and its box is 20 px, so half of it is 10. BasedOn keeps the
    // stock template; an implicit style alone would replace it.
    if crate::tokens::current().ticks == crate::tokens::Ticks::Round {
        let half = 10;
        x += &format!(r#"<Style TargetType="CheckBox" BasedOn="{{StaticResource DefaultCheckBoxStyle}}"><Setter Property="CornerRadius" Value="{half}"/></Style>"#);
    }
    // a text box with the focus gets a soft 3 px ring outside its accent line. A key cannot draw
    // that, so this is WinUI's own template (generic.xaml) with one border added, `Ring`, shown
    // by the Focused state; the ring lies outside the box, so nothing moves.
    if fields {
        x += &format!(
            r#"<Style TargetType="TextBox" BasedOn="{{StaticResource DefaultTextBoxStyle}}"><Setter Property="Template"><Setter.Value>{}</Setter.Value></Setter></Style>"#,
            include_str!("textbox.xaml")
        );
    }
    // a plain button sits on a soft shadow (the Soft finish): WinUI's own template (generic.xaml),
    // its states moved to a grid round it, with the shadow's host behind it. A button that has
    // no face of its own (a transparent one) gets no shadow: shade.rs looks before it draws one.
    // The copy leaves out generic.xaml's AnimatedIcon.State setters: loaded through XamlReader
    // they stop the app at start, and no Sprawl button holds an AnimatedIcon.
    if !flat() {
        x += &format!(
            r#"<Style TargetType="Button" BasedOn="{{StaticResource DefaultButtonStyle}}"><Setter Property="Template"><Setter.Value>{}</Setter.Value></Setter></Style>"#,
            include_str!("button.xaml")
        );
    }
    x
}

/// The styles, as a dictionary merged into the application's resources at start (so any
/// XAML loaded later, the Advisor's or Search's included, can name them). The stock controls'
/// own keys come first, so a text box, a menu or a check box matches the look; then the buttons,
/// which follow WinUI: a plain `<Button>` with no size of its own is the standard one;
/// `SprawlPrimary` is the one main action of a place, in the accent; `SprawlDanger`
/// deletes for good; `SprawlSubtle` is a text action with no fill until the pointer is over it.
pub(crate) fn resources() -> String {
    format!(
        r##"<ResourceDictionary {}>{}{}{}{}{}{}{}{}{}{}{}</ResourceDictionary>"##,
        ui::NS,
        stock(),
        style("SprawlPick", "Button", false),
        // the map's modes, on their opaque strip: the lit edge
        style_with("SprawlMode", "Button", false, false, false, Geo { depth: Depth { light: Some("hlbtn"), ..Depth::default() }, ..Geo::small() }),
        style("SprawlPickDrop", "DropDownButton", true),
        style_with("SprawlPage", "Button", false, true, false, Geo::small()),
        // the main page tabs: the same, without the line (the row's shared one is tabline.rs's)
        style_with("SprawlNav", "Button", false, false, false, Geo::small()),
        // chips: pills (the corner is clamped to half the height), a WinUI button's size with its
        // faint edge, filled with the accent when picked
        style_with("SprawlChip", "Button", false, false, false, Geo { corner: crate::tokens::get("rcchip"), depth: Depth { light: Some("hlon"), ..Depth::default() }, ..Geo::button().with_border("1", crate::tokens::refer("line1")) }),
        // a page tab drawn as a picked chip (the Tabs option): the same corner, no edge
        style_with("SprawlPagePill", "Button", false, false, false, Geo { corner: crate::tokens::get("rcchip"), padding: "12,4", ..Geo::small() }),
        style_with("SprawlSubtle", "Button", false, false, false, Geo { foreground: accent().text, ..Geo::button() }),
        style_with("SprawlDanger", "Button", false, false, true, Geo { background: danger(), foreground: "White".into(), weight: "SemiBold", depth: Depth { shade: Some("button"), light: Some("hlon"), ..Depth::default() }, ..Geo::button() }),
        // the one main action: the accent, semibold, with its accent shadow and a brighter lit edge
        style_with("SprawlPrimary", "Button", false, false, true, Geo { background: crate::tokens::refer("accent"), foreground: crate::tokens::refer("onaccent"), weight: "SemiBold", depth: Depth { primary: true, shade: Some("primary"), light: Some("hlpri"), ..Depth::default() }, ..Geo::button().with_border("1", crate::tokens::refer("accent")) }),
    )
}

// ---------------------------------------------------------------- what is running

/// What is running now, read from the state each frame. Nothing here starts or stops a job.
#[derive(Default, Clone, PartialEq, Debug)]
pub(crate) struct Running {
    /// A scan, and what it scans.
    pub scan: Option<String>,
    /// The filter box's filter being made.
    pub filtering: bool,
    /// Search's matches being sorted on a worker.
    pub sorting: bool,
    /// Search indexing the scan on screen.
    pub names: bool,
    /// Drives or saved scans Search is reading.
    pub drives: Vec<String>,
    /// The Cleaners tab reading the cleaners' folders.
    pub cleaners_read: bool,
    /// Community cleaners being fetched.
    pub fetch: bool,
    /// The Cleaners preview counting what a clean-up would remove.
    pub preview: bool,
    /// The Advisor's check of the scan.
    pub check: bool,
    /// The AI looking at the scan.
    pub ai: bool,
    /// The duplicate search.
    pub dupes: bool,
    /// A clean-up, and the tab it was started from (its command).
    pub cleanup: Option<&'static str>,
}

/// One running job: what it is called (steady text: no counts), and the tab that owns it.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Job {
    pub key: &'static str,
    pub what: String,
    /// The command that opens the owning tab.
    pub tab: &'static str,
}

/// A tab's name, from its command.
pub(crate) fn tab_name(tab: &str) -> &'static str {
    match tab {
        "TabTree" => "Tree View",
        "TabFiles" => "File View",
        "TabAdvisor" => "Advisor",
        "TabCleaners" => "Cleaners",
        "TabSearch" => "Search",
        _ => "",
    }
}

/// The jobs to list, the one the user waits on most first: a clean-up, a scan, then the rest.
pub(crate) fn jobs(r: &Running) -> Vec<Job> {
    let mut v = Vec::new();
    let mut add = |key: &'static str, what: String, tab: &'static str| v.push(Job { key, what, tab });
    if let Some(tab) = r.cleanup {
        add("cleanup", "Cleaning up".into(), tab);
    }
    if let Some(s) = &r.scan {
        add("scan", format!("Scanning {s}"), "TabTree");
    }
    if r.filtering {
        add("filter", "Filtering".into(), "TabTree");
    }
    if r.check {
        add("check", "Advisor: checking the scan".into(), "TabAdvisor");
    }
    if r.ai {
        add("ai", "Advisor: the AI is looking".into(), "TabAdvisor");
    }
    if r.dupes {
        add("dupes", "Advisor: finding duplicates".into(), "TabAdvisor");
    }
    if r.cleaners_read {
        add("clread", "Cleaners: reading their folders".into(), "TabCleaners");
    }
    if r.preview {
        add("preview", "Cleaners: counting the preview".into(), "TabCleaners");
    }
    if r.fetch {
        add("fetch", "Cleaners: fetching community cleaners".into(), "TabCleaners");
    }
    if r.names {
        add("names", "Search: indexing this scan".into(), "TabSearch");
    }
    if !r.drives.is_empty() {
        add("drives", format!("Search: reading {}", r.drives.join(", ")), "TabSearch");
    }
    if r.sorting {
        add("sort", "Search: sorting".into(), "TabSearch");
    }
    v
}

/// The pill's one line: the first job, and how many more.
pub(crate) fn summary(jobs: &[Job]) -> (String, String) {
    match jobs {
        [] => (String::new(), String::new()),
        [one] => (one.what.clone(), String::new()),
        [first, rest @ ..] => (first.what.clone(), format!("+{}", rest.len())),
    }
}

/// Jobs shorter than this are not listed: a quick sort or filter would only flicker.
const SHOW_AFTER_MS: u128 = 400;

#[derive(Default)]
struct Shown {
    since: std::collections::HashMap<&'static str, std::time::Instant>,
    listed: Vec<Job>,
    cleanup_tab: Option<&'static str>,
}

thread_local! {
    static SHOWN: std::cell::RefCell<Shown> = std::cell::RefCell::new(Shown::default());
}

/// The Running strip is drawn again at the next frame (the page was made again).
pub(crate) fn forget_shown() {
    SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        s.listed.clear();
        s.cleanup_tab = None;
    });
}

impl State {
    /// What is running, as the state says now.
    fn running(&self, cleanup_tab: Option<&'static str>) -> Running {
        let a = &self.advisor;
        Running {
            scan: self.job.as_ref().map(|j| j.path.file_name().map_or_else(|| j.path.display().to_string(), |n| n.to_string_lossy().into_owned())),
            filtering: self.filtering.is_some(),
            sorting: self.search.sorting.is_some() || self.search.more.sorting_now(),
            names: self.search.building.is_some(),
            drives: self.search.more.reading_now(),
            cleaners_read: a.cl.own_reading.is_some(),
            fetch: a.cl.fetching.is_some(),
            preview: matches!(a.cl.preview, Some(crate::cleaners::PreviewRun::Counting(_))),
            check: a.running,
            ai: a.ai_busy(),
            dupes: a.dupes_busy(),
            cleanup: a.cleanup.running.as_ref().map(|_| cleanup_tab.unwrap_or("TabAdvisor")),
        }
    }

    /// Each frame: the activity indicator beside the filter box, the same on every tab.
    pub(crate) fn busy_poll(&mut self) {
        self.start_frame();
        let mut cleanup_tab = SHOWN.with(|s| s.borrow().cleanup_tab);
        if self.advisor.cleanup.running.is_none() {
            cleanup_tab = None;
        } else if cleanup_tab.is_none() {
            // a clean-up starts from the tab that is open
            cleanup_tab = Some(if self.view == View::Advisor && self.advisor.cleaners_tab_on() { "TabCleaners" } else { "TabAdvisor" });
        }
        let all = jobs(&self.running(cleanup_tab));
        let now = std::time::Instant::now();
        let listed: Vec<Job> = SHOWN.with(|s| {
            let mut s = s.borrow_mut();
            s.cleanup_tab = cleanup_tab;
            s.since.retain(|k, _| all.iter().any(|j| j.key == *k));
            for j in &all {
                s.since.entry(j.key).or_insert(now);
            }
            all.into_iter().filter(|j| j.key == "cleanup" || s.since[j.key].elapsed().as_millis() >= SHOW_AFTER_MS).collect()
        });
        if SHOWN.with(|s| s.borrow().listed == listed) {
            return;
        }
        SHOWN.with(|s| s.borrow_mut().listed = listed.clone());
        let (line, more) = summary(&listed);
        text(&self.root, "BusyText", &line);
        text(&self.root, "BusyMore", &more);
        if let Ok(p) = find::<UIElement>(&self.root, "BusyPill") {
            let _ = p.SetVisibility(if listed.is_empty() { Visibility::Collapsed } else { Visibility::Visible });
        }
        if listed.is_empty() {
            if let Ok(f) = find::<FlyoutBase>(&self.root, "BusyFly") {
                let _ = f.Hide();
            }
        }
        self.busy_list(&listed);
    }

    /// The list under the pill: each job, and a click opens its tab.
    fn busy_list(&self, listed: &[Job]) {
        let Ok(host) = find::<Border>(&self.root, "BusyList") else { return };
        let mut x = format!(r##"<StackPanel {} Spacing="2"><TextBlock Text="Running now" FontSize="{{$fs12}}" Foreground="{{$text3}}" Margin="10,0,10,4"/>"##, ui::NS);
        for (i, j) in listed.iter().enumerate() {
            x += &format!(
                r##"<Button x:Name="BusyGo{i}" {} HorizontalAlignment="Stretch" HorizontalContentAlignment="Stretch" Padding="10,5" ToolTipService.ToolTip="Open {tab}"><Grid ColumnSpacing="16"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBlock Text="{}" TextTrimming="CharacterEllipsis"/><TextBlock Grid.Column="1" Text="{tab}  &#x203A;" Foreground="{{$text3}}"/></Grid></Button>"##,
                pick(Pick::Tab, false),
                ui::esc(&j.what),
                tab = tab_name(j.tab),
            );
        }
        x += "</StackPanel>";
        let Ok(content) = load::<UIElement>(&x) else { return };
        let _ = host.SetChild(&content);
        let Ok(fe) = content.cast::<FrameworkElement>() else { return };
        for (i, j) in listed.iter().enumerate() {
            let tab = j.tab;
            if let Ok(b) = find::<ButtonBase>(&fe, &format!("BusyGo{i}")) {
                if let Ok(t) = b.Click(move |_, _| {
                    with(|s| {
                        if let Ok(f) = find::<FlyoutBase>(&s.root, "BusyFly") {
                            let _ = f.Hide();
                        }
                        s.command(tab);
                    });
                }) {
                    keep(t);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- the tab Sprawl opens on

/// The tabs Sprawl can open on, in the order of `prefs::OPEN_ON` after "the tab used last".
pub(crate) const START_TABS: [&str; 5] = ["TabTree", "TabFiles", "TabAdvisor", "TabCleaners", "TabSearch"];

/// The "Open Sprawl on" setting as an index into `prefs::OPEN_ON` (0: the tab used last).
pub(crate) fn start_choice(v: Option<f64>) -> usize {
    v.filter(|v| v.is_finite() && *v >= 0.0).map_or(0, |v| (v as usize).min(START_TABS.len()))
}

/// The tab to open at start. A folder given on the command line or by Explorer is scanned and
/// shown in Tree View; the restart as administrator from the Advisor goes back to the Advisor;
/// otherwise the setting, where 0 is the tab used last (Tree View when none is known).
pub(crate) fn start_tab(choice: usize, last: Option<&str>, folder: bool, advisor_restart: bool) -> &'static str {
    if advisor_restart {
        return "TabAdvisor";
    }
    if folder {
        return "TabTree";
    }
    match choice {
        1..=5 => START_TABS[choice - 1],
        _ => last.and_then(|l| START_TABS.iter().find(|t| **t == l).copied()).unwrap_or("TabTree"),
    }
}

thread_local! {
    /// Frames drawn so far, and a tab switch that waits for the window to be drawn first.
    static START: std::cell::Cell<(u32, Option<&'static str>)> = const { std::cell::Cell::new((0, None)) };
}

impl State {
    /// Open the tab the start-up chose. Cleaners shows at once but reads its folders only
    /// once the window has been drawn (the read is started a few frames later).
    pub(crate) fn open_on(&mut self, tab: &'static str) {
        match tab {
            "TabTree" => {}
            "TabCleaners" => {
                self.set_cleaners_tab(true);
                self.set_view(View::Advisor);
                START.with(|s| s.set((s.get().0, Some(tab))));
            }
            t => self.command(t),
        }
    }

    /// Each frame: run what waited for the window, and remember the tab in use (saved with the
    /// other settings, for "the tab used last").
    fn start_frame(&mut self) {
        let (n, waiting) = START.with(|s| s.get());
        START.with(|s| s.set((n.saturating_add(1), if n >= 3 { None } else { waiting })));
        if n == 3 {
            if let Some(t) = waiting {
                self.command(t);
            }
        }
        if n < 3 {
            return;
        }
        let now = match self.view {
            View::Tree => "TabTree",
            View::Files => "TabFiles",
            View::Search => "TabSearch",
            View::Advisor if self.advisor.cleaners_tab_on() => "TabCleaners",
            View::Advisor => "TabAdvisor",
            View::About => return,
        };
        if self.settings.get("lastTab") != Some(now) {
            self.settings.set("lastTab", now);
        }
    }
}

// ---------------------------------------------------------------- narrow windows

/// The tree's share of the width beside the file types (which have 1): more in a narrow
/// window, where the tree's fixed columns would leave its names a few letters.
pub(crate) fn tree_share(width: f64) -> f64 {
    if width < 1500.0 {
        2.0
    } else {
        1.4
    }
}

/// The map's share of the height beside the tables (which have 1): less on the Search tab in a
/// short window, where the results need the rows. 0 for the height: not known yet.
pub(crate) fn map_share(search: bool, height: f64) -> f64 {
    if search && height > 0.0 && height < 800.0 {
        0.6
    } else {
        1.25
    }
}

impl State {
    /// The window's size changed.
    pub(crate) fn fit_size(&mut self, w: f64, h: f64) {
        self.fit_width(w);
        if (h - self.win_h).abs() >= 1.0 {
            self.win_h = h;
            self.map_room();
        }
    }

    /// The window's width changed.
    pub(crate) fn fit_width(&mut self, w: f64) {
        use crate::bindings::Microsoft::UI::Xaml::Controls::ColumnDefinition;
        use crate::bindings::Microsoft::UI::Xaml::{GridLength, GridUnitType};
        // a narrow window folds the header: the name and logo card goes first
        if let Ok(c) = find::<crate::bindings::Microsoft::UI::Xaml::UIElement>(&self.root, "BrandCard") {
            use crate::bindings::Microsoft::UI::Xaml::Visibility;
            let _ = c.SetVisibility(if w < 1250.0 { Visibility::Collapsed } else { Visibility::Visible });
        }
        if let Ok(c) = find::<ColumnDefinition>(&self.root, "TreeCol") {
            let _ = c.SetWidth(GridLength { Value: tree_share(w), GridUnitType: GridUnitType::Star });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_window_gives_search_the_rows() {
        assert_eq!(map_share(true, 720.0), 0.6);
        assert_eq!(map_share(true, 1000.0), 1.25);
        assert_eq!(map_share(false, 720.0), 1.25, "Search only");
        assert_eq!(map_share(true, 0.0), 1.25, "height not known yet");
    }

    #[test]
    fn a_narrow_window_widens_the_tree() {
        assert_eq!(tree_share(1680.0), 1.4);
        assert!(tree_share(1280.0) > tree_share(1680.0));
    }

    #[test]
    fn the_start_tab() {
        // the setting
        assert_eq!(start_tab(1, Some("TabSearch"), false, false), "TabTree");
        assert_eq!(start_tab(2, None, false, false), "TabFiles");
        assert_eq!(start_tab(3, None, false, false), "TabAdvisor");
        assert_eq!(start_tab(4, None, false, false), "TabCleaners");
        assert_eq!(start_tab(5, None, false, false), "TabSearch");
        // the tab used last, the default; Tree View when none (or an odd one) is known
        assert_eq!(start_tab(0, Some("TabCleaners"), false, false), "TabCleaners");
        assert_eq!(start_tab(0, None, false, false), "TabTree");
        assert_eq!(start_tab(0, Some("TabAbout"), false, false), "TabTree");
        // a folder from Explorer or the command line: scanned, in Tree View, whatever the setting
        for c in 0..=5 {
            assert_eq!(start_tab(c, Some("TabSearch"), true, false), "TabTree");
            // the restart as administrator from the Advisor goes back to the Advisor
            assert_eq!(start_tab(c, Some("TabSearch"), true, true), "TabAdvisor");
        }
        // the setting as stored
        assert_eq!(start_choice(None), 0);
        assert_eq!(start_choice(Some(4.0)), 4);
        assert_eq!(start_choice(Some(99.0)), 5);
        assert_eq!(start_choice(Some(-1.0)), 0);
        assert_eq!(start_choice(Some(f64::NAN)), 0);
        // one start tab for each choice after the first
        assert_eq!(START_TABS.len() + 1, crate::prefs::OPEN_ON.len());
        for (i, t) in START_TABS.iter().enumerate() {
            assert_eq!(tab_name(t), crate::prefs::OPEN_ON[i + 1]);
        }
    }

    #[test]
    fn the_indicator_lists_what_runs_with_its_tab() {
        assert!(jobs(&Running::default()).is_empty());
        let r = Running {
            drives: vec!["D:".into(), "E:".into()],
            cleaners_read: true,
            check: true,
            cleanup: Some("TabCleaners"),
            ..Default::default()
        };
        let j = jobs(&r);
        let tabs: Vec<&str> = j.iter().map(|j| j.tab).collect();
        assert_eq!(tabs, ["TabCleaners", "TabAdvisor", "TabCleaners", "TabSearch"]);
        assert_eq!(j[0].key, "cleanup");
        assert_eq!(j[3].what, "Search: reading D:, E:");
        // the pill: the first and how many more
        assert_eq!(summary(&j), ("Cleaning up".to_string(), "+3".to_string()));
        assert_eq!(summary(&j[..1]).1, "");
        // a scan belongs to the Tree View, a filter too, a sort to Search
        let r = Running { scan: Some("Games".into()), filtering: true, sorting: true, ..Default::default() };
        let tabs: Vec<&str> = jobs(&r).iter().map(|j| j.tab).collect();
        assert_eq!(tabs, ["TabTree", "TabTree", "TabSearch"]);
        // every owner is a tab with a name, and no line carries a count (the pill keeps still)
        let every = Running {
            scan: Some("x".into()),
            filtering: true,
            sorting: true,
            names: true,
            drives: vec!["data".into()],
            cleaners_read: true,
            fetch: true,
            preview: true,
            check: true,
            ai: true,
            dupes: true,
            cleanup: Some("TabAdvisor"),
        };
        assert_eq!(jobs(&every).len(), 12);
        assert!(jobs(&every).iter().all(|j| !tab_name(j.tab).is_empty() && !j.what.chars().any(|c| c.is_ascii_digit())));
    }

    #[test]
    fn chips_and_modes_take_the_accent_and_rows_stay_quiet() {
        let a = accent();
        // six digits with the hash: the value is a brush colour in XAML
        assert!(a.fill.starts_with('#') && a.fill.len() == 7, "{}", a.fill);
        assert!(a.on.starts_with('#') && a.text.starts_with('#'));
        for k in [Pick::Chip, Pick::Mode] {
            assert_eq!(k.bg(true), a.fill);
            assert_eq!(k.fg(true), Some(a.on.clone()));
            assert_ne!(k.bg(false), a.fill);
            assert_eq!(k.fg(false), None);
            assert!(pick(k, true).contains(&format!(r#"Foreground="{}""#, a.on)));
        }
        // a selected row keeps its grey second line readable: no accent fill, no text colour
        assert_eq!(Pick::Tab.bg(true), selected());
        assert_eq!(Pick::Tab.fg(true), None);
        assert!(pick(Pick::Tab, true).contains("SprawlPick") && !pick(Pick::Tab, true).contains("Foreground"));
        assert!(pick(Pick::Chip, false).contains("SprawlChip"));
        assert!(pick_drop(Pick::Mode, false).contains("SprawlPickDrop"));
    }

    /// The accent is the look's choice, not Windows' own: with Sunset picked, the selected chip,
    /// the page tab's underline and WinUI's accent button all take the token table's fill.
    #[test]
    fn the_accent_is_the_looks_choice() {
        use crate::tokens::{Accent as Choice, Look};
        crate::tokens::set(&Look { accent: Choice::Sunset, ..Look::default() });
        let fill = format!("#{}", crate::tokens::get("accent").trim_start_matches("#FF"));
        let on = format!("#{}", crate::tokens::get("onaccent").trim_start_matches("#FF"));
        assert_eq!(accent().fill, fill);
        assert_eq!(Pick::Chip.bg(true), fill);
        assert_eq!(Pick::Chip.fg(true), Some(on));
        assert_eq!(underline(), fill);
        assert!(pick(Pick::Mode, true).contains(&format!(r#"Background="{fill}""#)), "{}", pick(Pick::Mode, true));
        // WinUI's accent button takes the same token, so a primary button is the look's too
        assert!(resources().contains(r#"x:Key="AccentButtonBackground" Color="{$accent}""#));
        crate::tokens::set(&Look::default());
    }

    #[test]
    fn the_accent_follows_the_palette_and_skips_the_light_rewrite() {
        let p = [[1, 2, 3], [0x97, 0xC8, 0xCE], [5, 6, 7], [8, 9, 10], [0x37, 0x63, 0x72], [0x23, 0x44, 0x52], [0, 0, 0]];
        let dark = accent_from(Some(p), false);
        assert_eq!((dark.fill.as_str(), dark.on.as_str(), dark.text.as_str()), ("#97C8CE", "#000000", "#010203"));
        let light = accent_from(Some(p), true);
        assert_eq!((light.fill.as_str(), light.on.as_str(), light.text.as_str()), ("#376372", "#FFFFFF", "#234452"));
        // Windows' blue when there is no palette
        assert_eq!(accent_from(None, false).fill, "#4CC2FF");
        // six digits: the light page's rewrite leaves it as it is
        let x = format!(r##"<Button Background="{}" Foreground="{}"/>"##, light.fill, light.on);
        assert_eq!(crate::theme::to_light(&x), x);
    }

    /// The stock controls' overrides are loaded as XAML like the styles: every token written in
    /// them is one the look knows, in a whole attribute value, and `state::load`'s expansion
    /// leaves none of them behind. The radii are element content, read from the table.
    #[test]
    fn the_stock_overrides_are_the_looks_values() {
        let r = resources();
        let known: std::collections::HashSet<&str> = crate::tokens::TOKENS.iter().map(|t| t.name).collect();
        let open = format!("{}{}", '{', '$');
        let mut i = 0;
        let mut seen = 0;
        while let Some(at) = r[i..].find(&open) {
            let start = i + at;
            let end = r[start + 2..].find('}').map(|p| start + 2 + p).unwrap_or(r.len());
            let name = &r[start + 2..end];
            assert!(known.contains(name), "not a token: {name}");
            seen += 1;
            i = end + 1;
        }
        assert!(seen > 50, "the stock overrides were not found: {seen}");
        let expanded = crate::tokens::expand(&r);
        assert!(!expanded.contains(&open), "a token was left unexpanded");
        // the corners come from the radius tokens: 4 for a control, 8 for a panel over the page
        assert!(r.contains(r#"<CornerRadius x:Key="ControlCornerRadius">4</CornerRadius>"#));
        assert!(r.contains(r#"<CornerRadius x:Key="OverlayCornerRadius">8</CornerRadius>"#));
        // a text box keeps one pixel of line, focused or not
        assert_eq!(r.matches("<Thickness x:Key=\"TextControlBorderThemeThickness").count(), 2);
        // WinUI's accent button and the rest of its accent brushes are the look's accent
        assert!(r.contains(r#"x:Key="AccentButtonBackground""#), "{r}");
        // and the check box's tick is drawn on the accent
        assert!(expanded.contains(&format!(r##"<SolidColorBrush x:Key="CheckBoxCheckGlyphForegroundChecked" Color="{}"/>"##, crate::tokens::get("onaccent"))));
        // the keyboard's focus ring: the accent outside, the page inside it
        for (key, token) in [("SystemControlFocusVisualPrimaryBrush", "accent"), ("SystemControlFocusVisualSecondaryBrush", "ground")] {
            assert!(
                expanded.contains(&format!(r##"<SolidColorBrush x:Key="{key}" Color="{}"/>"##, crate::tokens::get(token))),
                "{key} is not {token}"
            );
        }
    }

    #[test]
    fn the_styles_define_every_state_once() {
        let r = resources();
        // the templated styles; SprawlPrimary builds on WinUI's own accent button
        for s in ["Normal", "PointerOver", "Pressed", "Disabled"] {
            // the templated styles (pick, mode, drop, page, chip, subtle, danger, primary) each
            // have it, and so does the text box's template
            assert!(r.matches(&format!(r#"x:Name="{s}""#)).count() >= 7, "{s}");
        }
        assert_eq!(r.matches("UseSystemFocusVisuals").count(), 10);
        for k in ["SprawlPick", "SprawlMode", "SprawlPickDrop", "SprawlPage", "SprawlNav", "SprawlPagePill", "SprawlChip", "SprawlSubtle", "SprawlDanger", "SprawlPrimary"] {
            assert_eq!(r.matches(&format!(r#"x:Key="{k}""#)).count(), 1, "{k}");
        }
        // the main button has its own template, with the accent's shadow
        let primary = &r[r.find(r#"x:Key="SprawlPrimary""#).unwrap()..];
        let primary = &primary[..primary.find("</Style>").unwrap()];
        // the shadow is a real one, hung by shade.rs on an empty host behind Root: no XAML
        // imitation (no gradient, no stacked layers) is left
        assert!(primary.contains(r#"<Border x:Name="Shade" Tag="shade:primary" CornerRadius="{TemplateBinding CornerRadius}" IsHitTestVisible="False"/>"#), "{primary}");
        assert!(primary.find(r#"x:Name="Shade""#) < primary.find(r#"x:Name="Root""#), "behind the button");
        assert!(!r.contains("GradientBrush") && !r.contains(r#"x:Name="Glow""#));
        // its states sit on the outer grid, after Root closes: WinUI runs only the outermost's
        let vsm = primary.find("VisualStateManager.VisualStateGroups").unwrap();
        assert_eq!(primary[vsm..].matches("</Grid>").count(), 1, "the main button's states are inside Root");
        assert!(!r[..r.find(r#"x:Key="SprawlPrimary""#).unwrap()].contains("shade:primary"), "only the main button is lit by the accent");
        // the red one and a plain button sit on the black one; a plain button keeps WinUI's own
        // template, its states moved to the grid round it
        assert!(r.contains(r#"<Style TargetType="Button" BasedOn="{StaticResource DefaultButtonStyle}">"#));
        let plain = &r[r.find(r#"<Style TargetType="Button" BasedOn"#).unwrap()..];
        let plain = &plain[..plain.find("</Style>").unwrap()];
        assert!(plain.contains(r#"<Border x:Name="Shade" Tag="shade:button""#) && plain.contains(r#"x:Name="ContentPresenter""#));
        assert!(plain.find("x:Name=\"Shade\"") < plain.find("x:Name=\"ContentPresenter\""));
        assert!(plain.find("</ContentPresenter>") < plain.find("VisualStateManager.VisualStateGroups"), "the states are on the outer grid");
        // the lit edge is a 1 px top border inside the button, bound to the button's corner
        assert!(primary.contains(r#"x:Name="Light""#) && primary.contains(r#"BorderThickness="0,1,0,0" BorderBrush="{$hlpri}""#) && primary.contains("CornerRadius=\"{TemplateBinding CornerRadius}\" BorderThickness=\"0,1,0,0\""));
        // a text box with the focus shows the ring, outside its edge
        assert!(r.contains(r#"x:Name="Ring""#) && r.contains(r#"<Setter Target="Ring.Opacity" Value="1"/>"#) && r.contains(r#"BorderBrush="{$ring}""#));
        // the red one greys out when it can't be pressed, as WinUI's filled buttons do
        let danger = &r[r.find(r#"x:Key="SprawlDanger""#).unwrap()..];
        let danger = &danger[..danger.find("</Style>").unwrap()];
        assert!(danger.contains(&crate::look::danger()) && danger.contains(r#"Target="Root.Background""#));
        // the page tabs: underlined, never filled
        assert!(r.contains("SprawlPage") && r.contains(r#"x:Name="Line""#));
        // the chips are pills: a corner past half the height, which the control clamps
        let chip = &r[r.find(r#"x:Key="SprawlChip""#).unwrap()..];
        let chip = &chip[..chip.find("</Style>").unwrap()];
        assert!(chip.contains(r#"<Setter Property="CornerRadius" Value="15"/>"#), "{chip}");
        assert!(chip.contains(r#"<Setter Property="BorderBrush" Value="{$line1}"/>"#));
        assert_eq!(Pick::Page.bg(true), "Transparent");
        assert!(pick(Pick::Page, true).contains(&underline()) && !pick(Pick::Page, false).contains(&underline()));
        // the main tabs have no line of their own: the row's shared one slides (tabline.rs)
        crate::tokens::set(&crate::tokens::Look::default());
        assert!(pick_nav(true).contains("SprawlNav") && !pick_nav(true).contains(&underline()) && pick_nav(true) == pick_nav(false));
        let r = resources();
        let nav = &r[r.find(r#"x:Key="SprawlNav""#).unwrap()..];
        assert!(!nav[..nav.find("</Style>").unwrap()].contains(r#"x:Name="Line""#));
    }

    /// The time and the travel each motion asks for, and the styles they make when Windows'
    /// "Animation effects" is on (off, None always wins, so the emitted styles are the still ones).
    #[test]
    fn the_buttons_move_as_the_motion_option_says() {
        use crate::tokens::{Look, Motion, Ticks};
        assert_eq!(state_time(Motion::Subtle, 140), "0:0:0.140");
        assert_eq!(state_time(Motion::Lively, 140), "0:0:0.220");
        assert_eq!(state_time(Motion::None, 140), "0:0:0.000");
        assert_eq!(travel(Motion::Subtle, -1.0, 0.98), (-1.0, 0.98));
        assert_eq!(travel(Motion::Lively, -1.0, 0.98), (-2.0, 0.96));
        assert_eq!(travel(Motion::None, -1.0, 0.98), (0.0, 1.0));

        if crate::win32::animations_on() {
            crate::tokens::set(&Look { motion: Motion::Lively, ..Look::default() });
            let r = resources();
            assert!(r.contains(r#"KeyTime="0:0:0.220""#), "Lively's time");
        }

        // the storyboards each motion emits (the pointer-over and pressed states)
        let feel = press_feel(crate::tokens::Press::LiftSink);
        let subtle = states_with(Motion::Subtle, &feel, feel.over);
        assert!(subtle.contains(r#"KeyTime="0:0:0.140""#), "{subtle}");
        assert!(subtle.contains(r#"TranslateY)"><SplineDoubleKeyFrame KeyTime="0:0:0.140" Value="-1""#), "{subtle}");
        let lively = states_with(Motion::Lively, &feel, feel.over);
        assert!(lively.contains(r#"KeyTime="0:0:0.220""#), "{lively}");
        assert!(lively.contains(r#"Value="-2""#), "{lively}");
        let lively_press = states_with(Motion::Lively, &feel, feel.down);
        assert!(lively_press.contains(r#"TranslateY)"><SplineDoubleKeyFrame KeyTime="0:0:0.220" Value="2""#), "{lively_press}");
        assert!(lively_press.contains(r#"ScaleX)"><SplineDoubleKeyFrame KeyTime="0:0:0.220" Value="0.96""#), "{lively_press}");
        let still = states_with(Motion::None, &feel, feel.down);
        assert!(still.contains(r#"KeyTime="0:0:0.000""#), "{still}");
        assert!(still.contains(r#"TranslateY)"><SplineDoubleKeyFrame KeyTime="0:0:0.000" Value="0""#), "{still}");
        assert!(still.contains(r#"ScaleX)"><SplineDoubleKeyFrame KeyTime="0:0:0.000" Value="1""#), "{still}");
        // the wash still changes in each: black while pressed
        assert!(still.contains(r#"TargetName="WashDown" Storyboard.TargetProperty="Opacity"><SplineDoubleKeyFrame KeyTime="0:0:0.000" Value="0.2""#), "{still}");
        assert!(still.contains(r#"TargetName="Wash" Storyboard.TargetProperty="Opacity"><SplineDoubleKeyFrame KeyTime="0:0:0.000" Value="0""#), "{still}");

        // Square is WinUI's own box; Round is the check box's CornerRadius, half of its 20 px
        crate::tokens::set(&Look { motion: Motion::Subtle, ..Look::default() });
        assert!(!resources().contains(r#"<Style TargetType="CheckBox""#));
        crate::tokens::set(&Look { ticks: Ticks::Round, ..Look::default() });
        let r = resources();
        assert!(
            r.contains(r#"<Style TargetType="CheckBox" BasedOn="{StaticResource DefaultCheckBoxStyle}"><Setter Property="CornerRadius" Value="10"/></Style>"#),
            "{r}"
        );
        crate::tokens::set(&Look::default());
    }

    /// WinUI throws on a KeySpline outside 0..1 (Press in crashed the app this way): every one
    /// the styles carry, under each press option and motion, stays inside.
    #[test]
    fn every_key_spline_is_one_winui_accepts() {
        use crate::tokens::{Look, Motion, Press};
        for press in [Press::LiftSink, Press::PressIn, Press::Fade, Press::Faint] {
            for motion in [Motion::Subtle, Motion::Lively, Motion::None] {
                crate::tokens::set(&Look { press, motion, ..Look::default() });
                let r = resources();
                let mut seen = 0;
                for part in r.split(r#"KeySpline=""#).skip(1) {
                    let text = &part[..part.find('"').unwrap()];
                    let v: Vec<f64> = text.split(',').map(|n| n.parse().unwrap()).collect();
                    assert!(v.len() == 4 && v.iter().all(|n| (0.0..=1.0).contains(n)), "{press:?} {motion:?}: {text}");
                    seen += 1;
                }
                assert!(seen > 0);
            }
        }
        crate::tokens::set(&Look::default());
    }

    /// Press in's curve cut at its turn is the same curve: each piece, put back in the whole's
    /// box, lands on the original, and the overshoot is the mockup's (about 12 % past, at 60 %).
    #[test]
    fn an_overshooting_curve_is_cut_exactly() {
        let s = [0.2, 0.9, 0.3, 1.35];
        let p = pieces(s);
        assert_eq!(p.len(), 2);
        let (x, v, _) = p[0];
        assert!((x - 0.6031).abs() < 1e-3 && (v - 1.1188).abs() < 1e-3, "{x} {v}");
        assert_eq!((p[1].0, p[1].1), (1.0, 1.0));
        let (mut x0, mut v0) = (0.0, 0.0);
        for (x1, v1, sp) in p {
            assert!(sp.iter().all(|n| (0.0..=1.0).contains(n)), "{sp:?}");
            for i in 0..=20 {
                let (px, py) = bezier_at(sp, f64::from(i) / 20.0);
                let (gx, gy) = (x0 + (x1 - x0) * px, v0 + (v1 - v0) * py);
                // the original curve's value at that time
                let (mut lo, mut hi) = (0.0, 1.0);
                for _ in 0..60 {
                    let m = (lo + hi) / 2.0;
                    if bezier_at(s, m).0 < gx { lo = m } else { hi = m }
                }
                assert!((bezier_at(s, lo).1 - gy).abs() < 1e-6, "at {gx}");
            }
            (x0, v0) = (x1, v1);
        }
        // a curve inside 0..1 is left whole
        assert_eq!(pieces(EASE), vec![(1.0, 1.0, EASE)]);
    }

    /// Each newer option reaches the styles it names: the press states, the finish, the chip's
    /// corner, the page tabs, and the fields' keys.
    #[test]
    fn the_newer_options_reach_the_styles() {
        use crate::tokens::{Chips, Controls, Finish, Look, Press, Tabs};
        // press feedback: the values each option gives the pointer-over and the pressed state,
        // as the mockup's four modes (the wash is white under the pointer and black pressed)
        let lift = press_feel(Press::LiftSink);
        assert_eq!((lift.over.white, lift.over.lift, lift.down.black, lift.down.lift, lift.down.scale, lift.ms), (0.10, -1.0, 0.20, 1.0, 0.98, 140));
        assert!(lift.shadow && lift.spline == [0.2, 0.8, 0.2, 1.0]);
        let sink = press_feel(Press::PressIn);
        assert_eq!((sink.over.white, sink.over.lift, sink.down.black, sink.down.scale, sink.ms), (0.13, 0.0, 0.22, 0.95, 130));
        assert!(!sink.shadow && sink.spline == [0.2, 0.9, 0.3, 1.35], "the sink overshoots");
        let clear = press_feel(Press::Fade);
        assert_eq!((clear.over.white, clear.down.black, clear.down.scale, clear.over.lift, clear.ms), (0.13, 0.22, 1.0, 0.0, 90));
        let faint = press_feel(Press::Faint);
        assert_eq!((faint.over.white, faint.down.white, faint.down.black, faint.ms), (0.07, 0.12, 0.0, 0));
        // each reaches the main button's template: its spline, and the shadow only for Lift
        for press in [Press::LiftSink, Press::PressIn, Press::Fade, Press::Faint] {
            crate::tokens::set(&Look { press, ..Look::default() });
            let r = resources();
            let primary = &r[r.find(r#"x:Key="SprawlPrimary""#).unwrap()..];
            let primary = &primary[..primary.find("</Style>").unwrap()];
            assert!(primary.contains(&format!(r#"KeySpline="{}""#, spline_text(safe(press_feel(press).spline)))), "{press:?}");
            // the washes fade on CSS's ease whatever the press is, as the mockup's do
            assert!(primary.contains(r#"TargetName="Wash" Storyboard.TargetProperty="Opacity"><SplineDoubleKeyFrame"#) && primary.contains(r#"KeySpline="0.25,0.1,0.25,1""#), "{press:?}");
            // only Press in's overshoot needs transitions between the states
            assert_eq!(primary.contains("<VisualTransition "), press == Press::PressIn, "{press:?}");
            // the shadow grows under the pointer and goes while pressed for Lift only (shade.rs
            // reads this), on the press's own time
            assert_eq!(shadow_motion(), (press == Press::LiftSink, state_ms(motion(), press_feel(press).ms)), "{press:?}");
        }
        crate::tokens::set(&Look::default());

        // finish: Flat has no shadow under a card, no shadow under a button and no lit edge
        assert_eq!(shadow(false), shade_host("card", &crate::tokens::rc(8.0)));
        assert!(shadow(true).contains(r#"Tag="shade:dialog""#));
        let page = |flat: bool| {
            crate::tokens::set(&Look { finish: if flat { Finish::Flat } else { Finish::Soft }, ..Look::default() });
            let all = format!("{}{}{}", crate::ui::page(&[]), resources(), crate::wizard::xaml());
            crate::tokens::set(&Look::default());
            all
        };
        // (token names are written without the opening brace: a bare one in a string would be
        // read as a token by the scanner test)
        let soft = page(false);
        for name in ["$hlcard}", "$hlbtn}", "$hlpri}", "shade:card", "shade:dialog", "shade:button", "shade:primary"] {
            assert!(soft.contains(name), "{name}");
        }
        // no imitation is left: no stacked layers of a shadow colour, no gradient shadow
        for old in ["$lift}", "$glow}", "$shd", "$glw", concat!("GradientStop Color=\"{", "$shd"), "x:Name=\"Glow\"", "x:Name=\"Lift\""] {
            assert!(!soft.contains(old), "{old}");
        }
        let flat_page = page(true);
        for token in ["shade:", "$hlcard", "$hlbtn", "$hlpri", "$hlon", "$hlbar", "x:Name=\"Shade\"", "x:Name=\"Light\"", "BasedOn=\"{StaticResource DefaultButtonStyle}\""] {
            assert!(!flat_page.contains(token), "Flat still has {token}");
        }
        crate::tokens::set(&Look { finish: Finish::Flat, ..Look::default() });
        assert_eq!(shadow(false), "");
        assert_eq!(top_light("hlcard", &crate::tokens::rc(8.0), false), "");
        crate::tokens::set(&Look::default());

        // chips: the corner follows the option
        for (chips, corner) in [(Chips::Pills, "15"), (Chips::Soft, "10"), (Chips::Rect, "4")] {
            crate::tokens::set(&Look { chips, ..Look::default() });
            let r = resources();
            let chip = &r[r.find(r#"x:Key="SprawlChip""#).unwrap()..];
            let chip = &chip[..chip.find("</Style>").unwrap()];
            assert!(chip.contains(&format!(r#"<Setter Property="CornerRadius" Value="{corner}"/>"#)), "{chips:?}: {chip}");
        }

        // tabs: pills are a picked chip, the underline is the border brush
        crate::tokens::set(&Look::default());
        assert!(pick(Pick::Page, true).contains("SprawlPage}") && pick(Pick::Page, true).contains(&underline()));
        crate::tokens::set(&Look { tabs: Tabs::Pills, ..Look::default() });
        let (on, off) = (pick(Pick::Page, true), pick(Pick::Page, false));
        assert!(on.contains("SprawlPagePill") && on.contains(&format!(r#"Background="{}""#, accent().fill)) && on.contains(&format!(r#"Foreground="{}""#, accent().on)), "{on}");
        assert!(off.contains(r#"Background="Transparent""#) && !off.contains(&accent().fill), "{off}");

        // controls: outlined is today's, soft draws no line at rest, stock leaves WinUI's own
        crate::tokens::set(&Look::default());
        let r = resources();
        assert!(r.contains(r#"x:Key="TextControlBorderBrush" Color="{$line2}""#) && r.contains(r#"x:Key="ComboBoxBackground" Color="{$ground}""#));
        assert!(r.contains(r#"x:Name="Ring""#));
        crate::tokens::set(&Look { controls: Controls::Soft, ..Look::default() });
        let r = resources();
        assert!(r.contains(r#"x:Key="TextControlBorderBrush" Color="Transparent""#) && r.contains(r#"x:Key="ComboBoxBackground" Color="{$row}""#));
        assert!(r.contains(r#"x:Key="TextControlBorderBrushFocused" Color="{$accent}""#), "the focus still shows the accent");
        crate::tokens::set(&Look { controls: Controls::Stock, ..Look::default() });
        let r = resources();
        assert!(!r.contains("TextControlBackground") && !r.contains("ComboBoxBackground") && !r.contains(r#"x:Name="Ring""#) && !r.contains("TextControlBorderThemeThickness"), "{r}");
        assert!(r.contains("ControlCornerRadius"), "the corner is still the look's");
        crate::tokens::set(&Look::default());
    }
}
