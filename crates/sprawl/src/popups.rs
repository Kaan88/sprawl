//! What a dropdown, an API key box and a menu do when they are focused and when they open, as the
//! mockup (docs/mockups/polish-2026-11-21/index.html, `.dd`, `.f` and `.menu`) has them.
//!
//! WinUI draws a focused combo box with its own halo and an accent pill, and opens a menu with a
//! clip reveal; none of that is a resource key, so these are WinUI's own templates (generic.xaml)
//! with the parts changed, loaded as implicit styles by [`crate::look`]'s `stock()`:
//!
//! * **Fields** (a combo box, a password box): the Controls option's focus. Outlined: a 1 px
//!   accent line and a 3 px ring of the accent at 32 %. Soft: a darker fill and a 2 px accent
//!   ring. Both fade in over the motion's time, as the mockup's `transition` does.
//! * **Popups** (a combo box's list, a menu): fade in from 5 px above at scale .97, over the
//!   motion's time and with its curve, from the top left corner.

use windows_core::{Interface, HSTRING};
use windows_numerics::{Vector2, Vector3};

use crate::bindings::Microsoft::UI::Xaml::Controls::MenuFlyoutPresenter;
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::Popup;
use crate::bindings::Microsoft::UI::Xaml::Hosting::ElementCompositionPreview;
use crate::bindings::Microsoft::UI::Xaml::Media::VisualTreeHelper;
use crate::bindings::Microsoft::UI::Xaml::UIElement;
use crate::look::{frames, motion, num, pieces, spline_text, EASE};
use crate::tokens::{Controls, Motion};

/// The mockup's `--m`: how long a field's focus, a menu opening takes.
pub(crate) fn ms(m: Motion) -> u32 {
    match m {
        Motion::Subtle => 160,
        Motion::Lively => 340,
        Motion::None => 0,
    }
}

/// The mockup's `--me`: the curve a menu's travel takes. Lively's overshoots, which a KeySpline
/// can't take: [`frames`] cuts it where it turns.
pub(crate) fn curve(m: Motion) -> [f64; 4] {
    match m {
        Motion::Subtle => [0.2, 0.8, 0.2, 1.0],
        Motion::Lively => [0.3, 1.55, 0.5, 1.0],
        Motion::None => [0.0, 0.0, 1.0, 1.0],
    }
}

/// How far above its place a menu starts, and the scale it starts at (`translateY(-5px) scale(.97)`).
const RISE: f64 = -5.0;
const SMALL: f64 = 0.97;

/// `target`'s Opacity to `to` over `m`'s time on CSS's `ease`, which the mockup's opacity and
/// colour transitions take; at once when there is no motion.
fn fade(target: &str, to: f64, m: Motion) -> String {
    let t = ms(m);
    if t == 0 {
        return format!(r##"<DoubleAnimation Storyboard.TargetName="{target}" Storyboard.TargetProperty="Opacity" To="{}" Duration="0"/>"##, num(to));
    }
    format!(
        r##"<DoubleAnimationUsingKeyFrames Storyboard.TargetName="{target}" Storyboard.TargetProperty="Opacity"><SplineDoubleKeyFrame KeyTime="0:0:{:.3}" Value="{}" KeySpline="{}"/></DoubleAnimationUsingKeyFrames>"##,
        f64::from(t) / 1000.0,
        num(to),
        spline_text(EASE)
    )
}

/// A popup's `target` arriving (`opening`) or leaving: its opacity on `ease`, and its travel
/// and scale on the motion's curve, as the mockup's `.menu` and `.menu.open`. The target holds a
/// `CompositeTransform` as its RenderTransform, with its origin at the top left. With no motion
/// it arrives at once and leaves without a word.
pub(crate) fn reveal(target: &str, opening: bool, m: Motion) -> String {
    let t = ms(m);
    let (o, y, s) = if opening { ((0.0, 1.0), (RISE, 0.0), (SMALL, 1.0)) } else { ((1.0, 0.0), (0.0, RISE), (1.0, SMALL)) };
    let path = |p: &str| format!("(UIElement.RenderTransform).(CompositeTransform.{p})");
    if t == 0 {
        if !opening {
            return String::new();
        }
        let set = |prop: &str, v: f64| {
            format!(r##"<DoubleAnimationUsingKeyFrames Storyboard.TargetName="{target}" Storyboard.TargetProperty="{prop}"><DiscreteDoubleKeyFrame KeyTime="0:0:0" Value="{}"/></DoubleAnimationUsingKeyFrames>"##, num(v))
        };
        return format!("{}{}{}{}", set("Opacity", 1.0), set(&path("TranslateY"), 0.0), set(&path("ScaleX"), 1.0), set(&path("ScaleY"), 1.0));
    }
    let from = |prop: &str, a: f64, rest: String| {
        format!(
            r##"<DoubleAnimationUsingKeyFrames Storyboard.TargetName="{target}" Storyboard.TargetProperty="{prop}"><DiscreteDoubleKeyFrame KeyTime="0:0:0" Value="{}"/>{rest}</DoubleAnimationUsingKeyFrames>"##,
            num(a)
        )
    };
    let c = curve(m);
    let opacity = from(
        "Opacity",
        o.0,
        format!(r##"<SplineDoubleKeyFrame KeyTime="0:0:{:.3}" Value="{}" KeySpline="{}"/>"##, f64::from(t) / 1000.0, num(o.1), spline_text(EASE)),
    );
    format!(
        "{opacity}{}{}{}",
        from(&path("TranslateY"), y.0, frames(y.0, y.1, t, c)),
        from(&path("ScaleX"), s.0, frames(s.0, s.1, t, c)),
        from(&path("ScaleY"), s.0, frames(s.0, s.1, t, c)),
    )
}

/// The parts of a field's focus that differ between the Controls options.
struct Focus {
    /// The ring's width, which is also how far it reaches outside the field.
    width: f64,
    brush: &'static str,
    /// What lies over the field itself while it has the focus: a fill, and a line.
    fill: &'static str,
    line: &'static str,
}

/// The mockup's `.f:focus` and `.dd.open`: under Outlined (`data-field=ring`, line 188) the
/// accent line and a 3 px ring of the accent at 32 %; under Soft (line 191) a darker fill and a
/// 2 px accent ring, with no line of its own. None under Stock, whose fields are WinUI's own.
fn focus(controls: Controls) -> Option<Focus> {
    match controls {
        Controls::Outlined => Some(Focus { width: 3.0, brush: "ring", fill: "none", line: "accent" }),
        Controls::Soft => Some(Focus { width: 2.0, brush: "accent", fill: "fieldfocus", line: "none" }),
        Controls::Stock => None,
    }
}

/// A colour token's reference, or no colour at all for "none".
fn paint(token: &str) -> String {
    if token == "none" { "Transparent".to_string() } else { crate::tokens::refer(token) }
}

/// A field's template: WinUI's own (generic.xaml) with the focus of the Controls option, or
/// nothing under Stock. `template` is [`COMBO_BOX`] or [`PASSWORD_BOX`].
fn field(template: &str, controls: Controls, m: Motion, popup: bool) -> Option<String> {
    let f = focus(controls)?;
    let inner: f64 = crate::tokens::get("rc4").parse().unwrap_or(4.0);
    let fades = |to: f64| format!("{}{}", fade("Ring", to, m), fade("Outline", to, m));
    let mut x = template
        .replace("@FOCUS_IN@", &fades(1.0))
        .replace("@FOCUS_OUT@", &fades(0.0))
        .replace("@RING_MARGIN@", &format!("-{}", num(f.width)))
        // outside the field's own corner by the ring's width, so the ring follows the corner
        .replace("@RING_CORNER@", &num(inner + f.width))
        .replace("@RING_BRUSH@", &paint(f.brush))
        .replace("@RING_WIDTH@", &num(f.width))
        .replace("@OUTLINE_FILL@", &paint(f.fill))
        .replace("@OUTLINE_LINE@", &paint(f.line));
    if popup {
        x = x.replace("@MENU_OPEN@", &reveal("PopupBorder", true, m)).replace("@MENU_CLOSE@", &reveal("PopupBorder", false, m));
    }
    Some(x)
}

const COMBO_BOX: &str = include_str!("combobox.xaml");
const PASSWORD_BOX: &str = include_str!("passwordbox.xaml");

/// The implicit styles for the combo box and the password box, in the look's motion. Nothing
/// under Stock.
pub(crate) fn fields(controls: Controls) -> String {
    let m = motion();
    let mut x = String::new();
    for (target, base, template, popup) in [("ComboBox", "DefaultComboBoxStyle", COMBO_BOX, true), ("PasswordBox", "DefaultPasswordBoxStyle", PASSWORD_BOX, false)] {
        if let Some(t) = field(template, controls, m, popup) {
            x += &format!(
                r#"<Style TargetType="{target}" BasedOn="{{StaticResource {base}}}"><Setter Property="Template"><Setter.Value>{t}</Setter.Value></Setter></Style>"#
            );
        }
    }
    x
}

/// The implicit style of a menu (File, Options, a context menu, a sub-menu): WinUI's own
/// presenter with one change. WinUI draws the menu's acrylic as a layer of its own, which would
/// sit still while the menu scales in ([`watch`]); the menu's colour is opaque, so the layer is
/// dropped.
pub(crate) fn menu() -> String {
    r#"<Style TargetType="MenuFlyoutPresenter" BasedOn="{StaticResource DefaultMenuFlyoutPresenterStyle}"><Setter Property="SystemBackdrop" Value="{x:Null}"/></Style>"#.to_string()
}

/// `xaml` with WinUI's own opening and closing of every `<MenuFlyout>` in it turned off
/// (`FlyoutBase.AreOpenCloseAnimationsEnabled`), so the only entrance a menu has is [`watch`]'s.
pub(crate) fn menus(xaml: &str) -> std::borrow::Cow<'_, str> {
    if !xaml.contains("<MenuFlyout") {
        return std::borrow::Cow::Borrowed(xaml);
    }
    let mut out = String::with_capacity(xaml.len() + 64);
    let mut rest = xaml;
    while let Some(at) = rest.find("<MenuFlyout") {
        let (head, tail) = rest.split_at(at + "<MenuFlyout".len());
        out.push_str(head);
        // only the element itself: not <MenuFlyoutItem>, <MenuFlyoutSeparator>, ...
        if tail.starts_with([' ', '>', '\n', '\r', '\t']) {
            out.push_str(r#" AreOpenCloseAnimationsEnabled="False""#);
        }
        rest = tail;
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

/// How a menu opens, as Composition takes it: one animated value, from where it starts to where it
/// rests, as key frames (when it ends, 0 to 1 of the time; how far along from start to end it is
/// there; the cubic-bezier of the stretch that ends there). Lively's overshoot goes past the end
/// and comes back, so it is several stretches, each of which stays inside 0..1 as
/// [`crate::look::pieces`] cuts it (Composition accepts none outside).
#[derive(Debug, PartialEq)]
pub(crate) struct Channel {
    pub from: f32,
    pub to: f32,
    pub keys: Vec<(f32, f32, [f64; 4])>,
}

impl Channel {
    fn along(from: f64, to: f64, curve: [f64; 4]) -> Channel {
        Channel { from: from as f32, to: to as f32, keys: pieces(curve).into_iter().map(|(t, v, s)| (t as f32, v as f32, s)).collect() }
    }

    /// The value at the end of the key frame `v` (a fraction from start to end).
    fn at(&self, v: f32) -> f32 {
        self.from + (self.to - self.from) * v
    }
}

/// A menu opening: how long it takes, and its opacity, its drop from 5 px above (`translateY`)
/// and its scale from .97, from the top left. Nothing without motion.
#[derive(Debug, PartialEq)]
pub(crate) struct Opening {
    pub ms: u32,
    pub opacity: Channel,
    pub lift: Channel,
    pub scale: Channel,
}

pub(crate) fn opening(m: Motion) -> Option<Opening> {
    let ms = ms(m);
    if ms == 0 {
        return None;
    }
    Some(Opening { ms, opacity: Channel::along(0.0, 1.0, EASE), lift: Channel::along(RISE, 0.0, curve(m)), scale: Channel::along(SMALL, 1.0, curve(m)) })
}

thread_local! {
    /// The popups seen open on the last frame, by identity.
    static OPEN: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Called once per frame, before it is drawn: a menu that has just opened starts from its
/// first frame invisible and fades in from just above ([`opening`]), as `.menu.open` does.
///
/// A menu's popup is WinUI's and has no states to put a storyboard in (an event trigger on the
/// presenter's Loaded never runs in WinUI 3), so this finds the popups newly open and plays the
/// animation on the presenter's Composition visual: opacity, scale (the visual's origin is its
/// top left) and `Translation`, which layout never writes. WinUI's own entrance (a clip reveal
/// and slide) is turned off in every flyout the app builds ([`menus`]), the menu bar's
/// (File, Options, Help: [`crate::menubar`]) among them. The dropdown of a combo box is not
/// done here: its template has the states for it ([`reveal`]).
pub(crate) fn watch() {
    if let Some(Ok(root)) = crate::state::with(|s| s.root.cast::<UIElement>()) {
        let _ = watch_in(&root);
    }
}

fn watch_in(root: &UIElement) -> windows_core::Result<()> {
    let popups = VisualTreeHelper::GetOpenPopupsForXamlRoot(&root.XamlRoot()?)?;
    let mut now = Vec::new();
    for i in 0..popups.Size()? {
        let popup = popups.GetAt(i)?;
        let id = popup.as_raw() as usize;
        now.push(id);
        if !OPEN.with(|o| o.borrow().contains(&id)) {
            let _ = open_menu(&popup);
        }
    }
    OPEN.with(|o| *o.borrow_mut() = now);
    Ok(())
}

fn open_menu(popup: &Popup) -> windows_core::Result<()> {
    let child = popup.Child()?;
    if child.cast::<MenuFlyoutPresenter>().is_err() {
        return Ok(());
    }
    let Some(o) = opening(motion()) else { return Ok(()) };
    let visual = ElementCompositionPreview::GetElementVisual(&child)?;
    let c = visual.Compositor()?;
    let time = windows_time::TimeSpan { duration: i64::from(o.ms) * 10_000 };
    let easing = |s: [f64; 4]| c.CreateCubicBezierEasingFunction(Vector2 { x: s[0] as f32, y: s[1] as f32 }, Vector2 { x: s[2] as f32, y: s[3] as f32 });
    let property = |name: &str| HSTRING::from(name);
    // opacity: invisible for the first frame, then up on `ease`
    visual.SetOpacity(o.opacity.from)?;
    let a = c.CreateScalarKeyFrameAnimation()?;
    for (t, v, s) in &o.opacity.keys {
        a.InsertKeyFrameWithEasingFunction(*t, o.opacity.at(*v), &easing(*s)?)?;
    }
    a.SetDuration(time)?;
    visual.StartAnimation(&property("Opacity"), &a)?;
    // scale, about the visual's top left
    visual.SetScale(Vector3 { x: o.scale.from, y: o.scale.from, z: 1.0 })?;
    let a = c.CreateVector3KeyFrameAnimation()?;
    for (t, v, s) in &o.scale.keys {
        let k = o.scale.at(*v);
        a.InsertKeyFrameWithEasingFunction(*t, Vector3 { x: k, y: k, z: 1.0 }, &easing(*s)?)?;
    }
    a.SetDuration(time)?;
    visual.StartAnimation(&property("Scale"), &a)?;
    // the drop from 5 px above, in `Translation`, which layout never writes (Offset it does)
    ElementCompositionPreview::SetIsTranslationEnabled(&child, true)?;
    let a = c.CreateVector3KeyFrameAnimation()?;
    for (t, v, s) in &o.lift.keys {
        a.InsertKeyFrameWithEasingFunction(*t, Vector3 { x: 0.0, y: o.lift.at(*v), z: 0.0 }, &easing(*s)?)?;
    }
    a.SetDuration(time)?;
    visual.StartAnimation(&property("Translation"), &a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splines_inside(x: &str) {
        let mut seen = 0;
        for part in x.split(r#"KeySpline=""#).skip(1) {
            let text = &part[..part.find('"').unwrap()];
            if text.starts_with('{') {
                continue; // WinUI's own named curve (in the stock overlay storyboards of the combo box)
            }
            let v: Vec<f64> = text.split(',').map(|n| n.parse().unwrap()).collect();
            assert!(v.len() == 4 && v.iter().all(|n| (0.0..=1.0).contains(n)), "{text}");
            seen += 1;
        }
        assert!(seen > 0);
    }

    #[test]
    fn a_popup_opens_from_five_pixels_above_at_97_percent_over_the_motions_time() {
        let open = reveal("P", true, Motion::Subtle);
        assert!(open.contains(r#"Value="-5""#) && open.contains(r#"Value="0.97""#), "{open}");
        // every channel takes 160 ms; opacity on ease, the travel on the mockup's curve
        assert_eq!(open.matches(r#"KeyTime="0:0:0.160""#).count(), 4, "{open}");
        assert!(open.contains(r#"KeySpline="0.25,0.1,0.25,1""#) && open.contains(r#"KeySpline="0.2,0.8,0.2,1""#), "{open}");
        splines_inside(&open);
        let close = reveal("P", false, Motion::Subtle);
        assert!(close.contains(r#"Value="1"/><SplineDoubleKeyFrame"#) && close.contains(r#"Value="0.97""#), "{close}");
    }

    #[test]
    fn lively_takes_340_ms_and_its_overshoot_stays_a_curve_winui_accepts() {
        let open = reveal("P", true, Motion::Lively);
        assert!(open.contains(r#"KeyTime="0:0:0.340""#), "{open}");
        splines_inside(&open);
        // the curve passes its end value and comes back: more than one piece for the travel
        assert!(open.matches("SplineDoubleKeyFrame").count() > 4, "{open}");
    }

    #[test]
    fn no_motion_opens_at_once_and_closes_without_animation() {
        let open = reveal("P", true, Motion::None);
        assert!(!open.contains("Spline") && open.contains(r#"Value="1""#) && open.contains(r#"Value="0""#), "{open}");
        assert_eq!(reveal("P", false, Motion::None), "");
        assert_eq!(opening(Motion::None), None);
    }

    #[test]
    fn a_menu_opens_by_composition_on_the_mockups_numbers() {
        // 160 ms, opacity on ease, from 5 px above and .97, travel on the mockup's curve
        let o = opening(Motion::Subtle).unwrap();
        assert_eq!(o.ms, 160);
        assert_eq!((o.opacity.from, o.opacity.to, o.opacity.keys.clone()), (0.0, 1.0, vec![(1.0, 1.0, EASE)]));
        assert_eq!((o.lift.from, o.lift.to, o.lift.keys.clone()), (-5.0, 0.0, vec![(1.0, 1.0, [0.2, 0.8, 0.2, 1.0])]));
        assert_eq!((o.scale.from, o.scale.to), (0.97, 1.0));
        // Lively: 340 ms, and the overshoot is cut into curves inside 0..1, ending at rest
        let l = opening(Motion::Lively).unwrap();
        assert_eq!(l.ms, 340);
        assert!(l.lift.keys.len() > 1 && l.lift.keys.iter().all(|(_, _, s)| s.iter().all(|n| (0.0..=1.0).contains(n))), "{:?}", l.lift);
        assert!(l.lift.keys.iter().any(|(_, v, _)| *v > 1.0), "it goes past where it rests");
        let end = l.lift.keys.last().unwrap();
        assert_eq!((end.0, end.1), (1.0, 1.0));
        assert_eq!(l.lift.at(end.1), 0.0);
        // the presenter's style drops the acrylic that would stay put while it scales
        let m = menu();
        assert!(m.contains(r#"TargetType="MenuFlyoutPresenter""#) && m.contains(r#"Property="SystemBackdrop" Value="{x:Null}""#), "{m}");
    }

    #[test]
    fn every_menu_flyout_loses_winuis_own_entrance_and_nothing_else_does() {
        let x = r#"<Button><Button.Flyout><MenuFlyout Placement="Bottom"><MenuFlyoutItem Text="a"/><MenuFlyoutSeparator/><MenuFlyoutSubItem Text="b"><MenuFlyoutItem Text="c"/></MenuFlyoutSubItem></MenuFlyout></Button.Flyout><FlyoutBase.AttachedFlyout><MenuFlyout x:Name="M"></MenuFlyout></FlyoutBase.AttachedFlyout></Button>"#;
        let y = menus(x);
        assert_eq!(y.matches(r#"<MenuFlyout AreOpenCloseAnimationsEnabled="False" Placement="Bottom">"#).count(), 1, "{y}");
        assert_eq!(y.matches(r#"<MenuFlyout AreOpenCloseAnimationsEnabled="False" x:Name="M">"#).count(), 1, "{y}");
        assert_eq!(y.matches("AreOpenCloseAnimationsEnabled").count(), 2, "items and sub-items are left as they are: {y}");
        assert!(matches!(menus("<Grid/>"), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn a_dropdown_runs_its_opening_in_its_open_state() {
        let c = field(COMBO_BOX, Controls::Outlined, Motion::Subtle, true).unwrap();
        assert!(c.contains(r#"<VisualState x:Name="Opened"><Storyboard><Double"#) && c.contains(r#"Storyboard.TargetName="PopupBorder""#), "{c}");
        assert!(!c.contains("SplitOpenThemeAnimation") && !c.contains("AnimatedIcon"), "the stock reveal and the animated chevron are gone");
        assert!(!c.contains('@'), "every placeholder is filled: {c}");
        splines_inside(&c);
    }

    #[test]
    fn the_focus_is_the_mockups_for_each_controls_option() {
        // Outlined: the accent line and a 3 px ring of the accent at 32 %, outside the 4 px corner
        let c = field(COMBO_BOX, Controls::Outlined, Motion::Subtle, true).unwrap();
        let ring = format!(r#"<Border x:Name="Ring" Grid.Row="1" Grid.Column="0" Grid.ColumnSpan="2" Margin="-3" CornerRadius="{}" BorderBrush="{{$ring}}" BorderThickness="3""#, 7);
        assert!(c.contains(&ring), "{c}");
        assert!(c.contains(r#"Background="Transparent" BorderBrush="{$accent}""#), "{c}");
        assert!(!c.contains(r#"x:Name="Pill""#) && !c.contains("HighlightBackground"), "WinUI's pill and halo are gone");
        // the ring is shown by the three focused states and fades with the motion's time
        for state in ["Focused", "FocusedPressed", "FocusedDropDown"] {
            let at = c.find(&format!(r#"<VisualState x:Name="{state}">"#)).unwrap();
            assert!(c[at..at + 700].contains(r#"TargetName="Ring""#), "{state}");
        }
        assert!(c.contains(r#"KeyTime="0:0:0.160" Value="1""#));
        // Soft: a darker fill and a 2 px accent ring
        let s = field(COMBO_BOX, Controls::Soft, Motion::Subtle, true).unwrap();
        let soft = format!(r#"Margin="-2" CornerRadius="{}" BorderBrush="{{$accent}}" BorderThickness="2""#, 6);
        assert!(s.contains(&soft) && s.contains(r#"Background="{$fieldfocus}" BorderBrush="Transparent""#), "{s}");
        // Stock: WinUI's own, nothing of ours
        assert!(field(COMBO_BOX, Controls::Stock, Motion::Subtle, true).is_none() && fields(Controls::Stock).is_empty());
        // no motion: the ring comes at once
        let n = field(PASSWORD_BOX, Controls::Outlined, Motion::None, false).unwrap();
        assert!(n.contains(r#"TargetName="Ring" Storyboard.TargetProperty="Opacity" To="1" Duration="0""#), "{n}");
    }

    #[test]
    fn both_fields_become_implicit_styles_on_winuis_own() {
        let f = fields(Controls::Outlined);
        assert!(f.contains(r#"<Style TargetType="ComboBox" BasedOn="{StaticResource DefaultComboBoxStyle}">"#), "{f}");
        assert!(f.contains(r#"<Style TargetType="PasswordBox" BasedOn="{StaticResource DefaultPasswordBoxStyle}">"#), "{f}");
    }

    #[test]
    fn the_api_key_box_has_the_same_ring() {
        let p = field(PASSWORD_BOX, Controls::Outlined, Motion::Subtle, false).unwrap();
        assert!(p.contains(r#"TargetType="PasswordBox""#) && p.contains(r#"x:Name="RevealButton""#), "the reveal button is kept");
        assert!(p.contains(r#"x:Name="Ring""#) && p.contains(r#"BorderBrush="{$ring}" BorderThickness="3""#) && p.contains(r#"x:Name="Outline""#), "{p}");
        let focused = &p[p.find(r#"<VisualState x:Name="Focused">"#).unwrap()..];
        assert!(focused[..300].contains(r#"TargetName="Ring""#));
        assert!(!p.contains('@'), "every placeholder is filled");
        splines_inside(&p);
    }
}
