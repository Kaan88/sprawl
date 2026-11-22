//! Real drop shadows: a Composition `DropShadow` under a card, a button or an Advisor row, the
//! mockup's CSS box-shadows (docs/mockups/polish-2026-11-21: the Soft depth finish, and the Lift
//! and sink press).
//!
//! XAML cannot draw one (ThemeShadow needs receivers set from code and is one grey), so the XAML
//! only says where a shadow goes, with an empty border whose `Tag` is `shade:<kind>`
//! ([`crate::look::shade_host`]), put behind the item at the item's size and corner. Once the
//! XAML is loaded ([`watch`], from `state::load`) each host gets, as its child visual:
//!
//! - a `SpriteVisual` the host's size with no brush of its own, only a `DropShadow` (colour,
//!   blur, offset, opacity);
//! - the shadow's mask: a `ShapeVisual` filling the host's rounded rectangle, drawn into a
//!   `CompositionVisualSurface`, so the shadow follows the corners;
//! - a clip on the sprite to everything OUTSIDE the host's square rectangle: a CSS box-shadow
//!   is never drawn under its own box, so a see-through row or button shows what is behind it,
//!   not its shadow. The clip is a Direct2D path (a large rectangle with the host's cut out of
//!   it) that Composition reads through `IGeometrySource2DInterop`;
//! - the rounded corners, outside the arc and inside the square, drawn antialiased by
//!   [`Corners`]: the shadow there through a gradient mask that follows the arc. (Clipped by the
//!   path, the arcs drew a stepped dark ring where the glow meets the main button.)
//!
//! (A `LayerVisual` casting the shadow from its content looked simpler, but Composition drew
//! its shadow and content cut off by rectangles round the cards.)
//!
//! A host inside a button's template is reached from the button: the button's first
//! `SizeChanged` comes after its template is applied, even for a button first loaded inside a
//! collapsed panel. That shadow then follows the template's CommonStates (Normal, PointerOver,
//! Pressed, Disabled) with Composition animations on the Press option's time and CSS's `ease`.
//!
//! What a flyout shows is walked too (a `Button.Flyout` or an attached `Flyout`'s content), and
//! the flyouts and dropdowns are kept inside the window, so their own popups are drawn with the
//! page. Each frame [`popups`] looks at the window's open popups: a menu, a flyout or a dropdown
//! list gets the mockup's menu shadow (`0 10px 28px #00000099`) in place of WinUI's ThemeShadow.
//!
//! Every shadow made is listed ([`repaint`]): a theme change (the accent, light or dark) gives
//! the ones still on screen their new colours at once.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use windows_core::{implement, interface, Interface, HRESULT, HSTRING};
use windows_reference::IReference;

use crate::bindings::Microsoft::UI::Composition::{
    CompositionMappingMode, CompositionPath, CompositionPathGeometry, CompositionRadialGradientBrush, CompositionRoundedRectangleGeometry, CompositionVisualSurface, Compositor, ContainerVisual, DropShadow, ICompositionSurface, ShapeVisual, SpriteVisual,
};
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, FlyoutBase, Popup};
use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, Button, ComboBox, ContentControl, Control, Flyout, FlyoutPresenter, MenuFlyoutPresenter, Panel, ToggleSwitch};
use crate::bindings::Microsoft::UI::Xaml::Hosting::ElementCompositionPreview;
use crate::bindings::Microsoft::UI::Xaml::Media::{CompositionTarget, SolidColorBrush, VisualTreeHelper};
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement, VisualStateManager};
use crate::bindings::Windows::Graphics::{IGeometrySource2D, IGeometrySource2D_Impl};
use crate::bindings::Windows::UI::Color;
use crate::state::keep;
use windows_numerics::{Vector2, Vector3};

/// What a shadow is under, each with the mockup's own CSS values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// A card or pane: `0 8px 22px #00000052`.
    Card,
    /// A dialog's card over the page, as the mockup's floating menu: `0 10px 28px #00000099`.
    Dialog,
    /// A menu, a flyout or a dropdown's list: the mockup's `.menu`, `0 10px 28px #00000099`.
    Menu,
    /// A button with a face: `0 1px 2px #0000004d`, under the pointer `0 6px 14px #00000066`.
    Button,
    /// The main button, in the accent: `0 3px 10px` at 45%, under the pointer `0 6px 16px` at
    /// 48%.
    Primary,
    /// An Advisor row: `0 1px 2px #00000033`, under the pointer `0 5px 12px #00000059`.
    Row,
    /// A switch's knob, on the filled track: `0 1px 3px #00000066`, whatever the finish.
    Knob,
}

/// One CSS box-shadow: its downward offset, its blur radius and its colour's alpha.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Cast {
    pub dy: f32,
    pub blur: f32,
    pub alpha: f32,
}

impl Cast {
    /// CSS `box-shadow: none` (what a transition to none runs to: no offset, blur or colour).
    pub(crate) const NONE: Cast = Cast { dy: 0.0, blur: 0.0, alpha: 0.0 };

    /// `0 {dy}px {blur}px` in a colour whose alpha byte is `a` (as the mockup writes it, #RRGGBBAA).
    const fn css(dy: f32, blur: f32, a: u8) -> Cast {
        Cast { dy, blur, alpha: a as f32 / 255.0 }
    }
}

/// How much of a CSS blur radius Composition's `DropShadow.BlurRadius` takes for the same
/// spread. CSS draws a gaussian whose standard deviation is half the blur radius; Composition's
/// is narrower for the same number. Measured under a card (`0 8px 22px #00000052` on the dark
/// page, against the same CSS drawn by Edge), the fall-off matches pixel row by pixel row at
/// twice the CSS radius.
pub(crate) const BLUR: f32 = 2.0;

impl Kind {
    /// The kind a host's `Tag` names (`shade:card` and so on).
    pub(crate) fn from_tag(tag: &str) -> Option<Kind> {
        Some(match tag.strip_prefix("shade:")? {
            "card" => Kind::Card,
            "dialog" => Kind::Dialog,
            "menu" => Kind::Menu,
            "button" => Kind::Button,
            "primary" => Kind::Primary,
            "row" => Kind::Row,
            "knob" => Kind::Knob,
            _ => return None,
        })
    }

    /// The shadow at rest.
    pub(crate) fn rest(self) -> Cast {
        match self {
            Kind::Card => Cast::css(8.0, 22.0, 0x52),
            Kind::Dialog | Kind::Menu => Cast::css(10.0, 28.0, 0x99),
            Kind::Button => Cast::css(1.0, 2.0, 0x4d),
            // color-mix(in srgb, accent 45%, transparent)
            Kind::Primary => Cast { dy: 3.0, blur: 10.0, alpha: 0.45 },
            Kind::Row => Cast::css(1.0, 2.0, 0x33),
            Kind::Knob => Cast::css(1.0, 3.0, 0x66),
        }
    }

    /// The shadow under the pointer when the item lifts (the Lift and sink press).
    pub(crate) fn over(self) -> Cast {
        match self {
            Kind::Button => Cast::css(6.0, 14.0, 0x66),
            Kind::Primary => Cast { dy: 6.0, blur: 16.0, alpha: 0.48 },
            Kind::Row => Cast::css(5.0, 12.0, 0x59),
            Kind::Card | Kind::Dialog | Kind::Menu | Kind::Knob => self.rest(),
        }
    }

    /// The shadow in one of a button's CommonStates. With `lift` (Lift and sink) it grows under
    /// the pointer and goes while pressed (`:active { box-shadow: none }`); otherwise it stays as
    /// at rest. A disabled button has none.
    pub(crate) fn pose(self, state: &str, lift: bool) -> Cast {
        match state {
            "Disabled" => Cast::NONE,
            "PointerOver" if lift => self.over(),
            "Pressed" if lift => Cast::NONE,
            _ => self.rest(),
        }
    }

    /// How far past the item's edge its shadow can reach, for the clip round it: a gaussian is
    /// gone by three standard deviations, and Composition's blur reaches its own radius.
    fn reach(self) -> f32 {
        let (a, b) = (self.rest(), self.over());
        (a.blur.max(b.blur) * BLUR * 1.5 + a.dy.max(b.dy)).ceil() + 4.0
    }
}

/// `#AARRGGBB` as a colour.
pub(crate) fn colour(hex: &str) -> Option<Color> {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).ok()?;
    let v = if hex.len() == 7 { 0xFF00_0000 | v } else { v };
    Some(Color { A: (v >> 24) as u8, R: (v >> 16) as u8, G: (v >> 8) as u8, B: v as u8 })
}

/// The two colours shadows are drawn in: the `shadow` token (its alpha is how strong the page's
/// shadows are) and the accent (the main button's glow).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Tones {
    pub shadow: Color,
    pub accent: Color,
}

impl Tones {
    const BLACK: Color = Color { A: 255, R: 0, G: 0, B: 0 };

    /// The look in use, as the page is drawn now.
    pub(crate) fn now() -> Tones {
        let shadow = colour(&crate::tokens::get("shadow")).unwrap_or(Self::BLACK);
        Tones { shadow, accent: colour(&crate::tokens::get("accent")).unwrap_or(shadow) }
    }

    /// A table of tokens of its own (the Theme page's sample, drawn for the look being picked).
    pub(crate) fn of(table: &std::collections::HashMap<&'static str, String>) -> Tones {
        let shadow = table.get("shadow").and_then(|v| colour(v)).unwrap_or(Self::BLACK);
        Tones { shadow, accent: table.get("accent").and_then(|v| colour(v)).unwrap_or(shadow) }
    }

    /// The shadow's colour, opaque, and how strong the page's shadows are (the `shadow` token's
    /// alpha: full on the dark page, softer on the light one). The main button's is its accent.
    fn paint(self, kind: Kind) -> (Color, f32) {
        let strength = self.shadow.A as f32 / 255.0;
        let base = if kind == Kind::Primary { self.accent } else { self.shadow };
        (Color { A: 255, ..base }, strength)
    }
}

/// One shadow, hung on its host.
struct Shade {
    kind: Kind,
    /// The host's child visual: the shadow and its four corners.
    root: ContainerVisual,
    clip: CompositionPathGeometry,
    sprite: SpriteVisual,
    shape: ShapeVisual,
    surface: CompositionVisualSurface,
    geometry: CompositionRoundedRectangleGeometry,
    /// The shadow on the page, and its twin under the copy the corners are drawn from: the same
    /// values, always set together.
    shadow: [DropShadow; 2],
    corners: Corners,
    radius: f32,
    /// The host's size, for a new display scale.
    size: Mutex<(f32, f32)>,
    /// Colours of its own (the Theme page's sample); `None` follows the look in use, read again
    /// by [`repaint`].
    tones: Option<Tones>,
    /// The page's shadow strength, as an `f32`'s bits (changed by [`repaint`]).
    strength: AtomicU32,
    /// The shadow shown now (or being run to), for [`repaint`] to show again in new colours.
    cast: Mutex<Cast>,
}

thread_local! {
    /// Every shadow made, while its host lives (a host's handlers own its shade).
    static LIVE: RefCell<Live<Shade>> = const { RefCell::new(Live::new()) };
}

/// Weak references to what is made, swept of the dead as the list grows (a page made again
/// drops hundreds at once).
struct Live<T> {
    items: Vec<Weak<T>>,
    /// The list is swept when it grows to this.
    sweep_at: usize,
}

impl<T> Live<T> {
    const fn new() -> Self {
        Live { items: Vec::new(), sweep_at: 64 }
    }

    fn add(&mut self, s: &Arc<T>) {
        if self.items.len() >= self.sweep_at {
            self.items.retain(|w| w.strong_count() > 0);
            self.sweep_at = (self.items.len() * 2).max(64);
        }
        self.items.push(Arc::downgrade(s));
    }

    /// The ones still alive (the dead are swept out).
    fn alive(&mut self) -> Vec<Arc<T>> {
        self.items.retain(|w| w.strong_count() > 0);
        self.items.iter().filter_map(Weak::upgrade).collect()
    }
}

/// The theme changed (the accent, light or dark): every shadow still on screen takes the new
/// colours at once, as a shadow made now would. The Theme page's sample keeps its own.
pub(crate) fn repaint() {
    let live = LIVE.with(|l| l.borrow_mut().alive());
    if live.is_empty() {
        return;
    }
    let now = Tones::now();
    for s in live {
        let _ = s.repaint(now);
    }
}

/// The four rounded corners of a shadow's host. The page's shadow is clipped to outside the
/// host's square rectangle, whose straight edges lie on the host's own, on whole pixels. A clip
/// can't follow an arc as the face's own antialiased edge needs ([`arc_stops`]: clipped along the
/// arc, the corner drew a stepped dark ring). What a rounded corner leaves outside the rounded rectangle, inside the square, is drawn here: a copy
/// of the shadow, drawn offscreen into a surface at the display's scale (a pixel of the surface to
/// a pixel of the screen), shown through a mask that is a radial gradient about the corner's
/// centre: clear inside the corner's arc, opaque outside it, with a soft step just inside the arc
/// ([`arc_stops`]). A gradient is drawn at the screen's own pixels, so the edge is as fine at
/// 150 % as at 100 %. (A shape drawn into a surface is not antialiased.)
struct Corners {
    /// The copy: a sprite casting the twin shadow, in a container at the display's scale.
    copy: SpriteVisual,
    lens: ContainerVisual,
    /// Each corner: what shows it, the copy's part there, and its mask.
    each: Vec<(SpriteVisual, CompositionVisualSurface, CompositionRadialGradientBrush)>,
}

impl Corners {
    fn new(c: &Compositor, root: &ContainerVisual, shadow: &DropShadow) -> windows_core::Result<Corners> {
        // the surfaces draw this tree, which is on no page
        let copy = c.CreateSpriteVisual()?;
        copy.SetShadow(shadow)?;
        let lens = c.CreateContainerVisual()?;
        lens.Children()?.InsertAtTop(&copy)?;
        let copy_root = c.CreateContainerVisual()?;
        copy_root.Children()?.InsertAtTop(&lens)?;
        let mut each = Vec::with_capacity(4);
        for _ in 0..4 {
            let part = c.CreateVisualSurface()?;
            part.SetSourceVisual(&copy_root)?;
            let mask = c.CreateRadialGradientBrush()?;
            mask.SetMappingMode(CompositionMappingMode::Absolute)?;
            let brush = c.CreateMaskBrush()?;
            brush.SetSource(&c.CreateSurfaceBrushWithSurface(&part.cast::<ICompositionSurface>()?)?)?;
            brush.SetMask(&mask)?;
            let sprite = c.CreateSpriteVisual()?;
            sprite.SetBrush(&brush)?;
            root.Children()?.InsertAtTop(&sprite)?;
            each.push((sprite, part, mask));
        }
        Ok(Corners { copy, lens, each })
    }

    /// The host is `w` by `h` with corners of radius `r`, on a display at scale `k`.
    fn resize(&self, w: f32, h: f32, r: f32, k: f32) -> windows_core::Result<()> {
        self.copy.SetSize(Vector2 { x: w, y: h })?;
        self.lens.SetScale(Vector3 { x: k, y: k, z: 1.0 })?;
        let r = if r > 0.0 && mode() == Mode::Corners { r } else { 0.0 };
        // each corner's square, and its arc's centre inside it
        let squares = [((0.0, 0.0), (r, r)), ((w - r, 0.0), (0.0, r)), ((w - r, h - r), (0.0, 0.0)), ((0.0, h - r), (r, 0.0))];
        for ((sprite, part, mask), ((x, y), centre)) in self.each.iter().zip(squares) {
            sprite.SetOffset(Vector3 { x, y, z: 0.0 })?;
            sprite.SetSize(Vector2 { x: r, y: r })?;
            part.SetSourceOffset(Vector2 { x: x * k, y: y * k })?;
            part.SetSourceSize(Vector2 { x: r * k, y: r * k })?;
            if r > 0.0 {
                arc_mask(mask, centre, r, k)?;
            }
        }
        Ok(())
    }
}

/// Where a corner's mask steps from clear to opaque, as fractions of the gradient's radius (`r`,
/// the arc's). The host's own face is antialiased along the arc: a pixel there is part face, part
/// what lies behind. Were the shadow taken away behind it too (as the clip does, and a mask
/// centred on the arc would half do), that pixel would show the face over the bare page, darker
/// than the shadow beside it and the face inside it: a dark ring of stairs round the corner,
/// worst under the main button's glow. So, as along the straight edges, where no pixel is part
/// face, the shadow is taken away only behind pixels the face covers whole: the step lies half a
/// screen pixel (`0.5 / k` view pixels) inside the arc, half a screen pixel wide.
fn arc_stops(r: f32, k: f32) -> (f32, f32, f32) {
    let px = 1.0 / k.max(1.0);
    (((r - px * 0.75) / r).max(0.0), ((r - px * 0.25) / r).max(0.0), r)
}

fn arc_mask(mask: &CompositionRadialGradientBrush, centre: (f32, f32), r: f32, k: f32) -> windows_core::Result<()> {
    let c = mask.Compositor()?;
    let (clear, opaque, reach) = arc_stops(r, k);
    mask.SetEllipseCenter(Vector2 { x: centre.0, y: centre.1 })?;
    mask.SetEllipseRadius(Vector2 { x: reach, y: reach })?;
    let stops = mask.ColorStops()?;
    stops.Clear()?;
    stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(clear, Color { A: 0, R: 0, G: 0, B: 0 })?)?;
    stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(opaque, Color { A: 255, R: 0, G: 0, B: 0 })?)
}

/// dev: how a shadow is cut round its host (`SPRAWL_SHADE`, for screenshot runs that compare
/// them): `old`, the rounded rectangle clipped away (aliased); `noclip`, nothing clipped away.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Corners,
    Old,
    NoClip,
}

fn mode() -> Mode {
    thread_local! {
        static MODE: Mode = match std::env::var("SPRAWL_SHADE").as_deref() {
            Ok("old") => Mode::Old,
            Ok("noclip") => Mode::NoClip,
            _ => Mode::Corners,
        };
    }
    MODE.with(|m| *m)
}

thread_local! {
    /// The display's scale (XamlRoot.RasterizationScale), 0 before the first frame.
    static SCALE: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// dev: `SPRAWL_SCALE=1.5` draws the page magnified, as a display at that scale would, and the
/// shadows' corners for that scale (a screenshot run on a 100 % display).
fn scale_override() -> Option<f32> {
    thread_local! {
        static OVER: Option<f32> = std::env::var("SPRAWL_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).filter(|k| *k >= 1.0);
    }
    OVER.with(|o| *o)
}

fn scale_of(el: &UIElement) -> f32 {
    if let Some(k) = scale_override() {
        return k;
    }
    let known = SCALE.with(|s| s.get());
    if known > 0.0 {
        return known;
    }
    el.XamlRoot().and_then(|x| x.RasterizationScale()).map(|k| k as f32).unwrap_or(1.0)
}

/// Called each frame: on a new display scale (another monitor, another setting), every shadow's
/// corners are drawn again for it.
pub(crate) fn track_scale(root: &UIElement) {
    let k = match scale_override() {
        Some(k) => {
            thread_local! { static MAGNIFIED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
            if !MAGNIFIED.with(|m| m.replace(true)) {
                let _ = ElementCompositionPreview::GetElementVisual(root).and_then(|v| v.SetScale(Vector3 { x: k, y: k, z: 1.0 }));
            }
            k
        }
        None => match root.XamlRoot().and_then(|x| x.RasterizationScale()) {
            Ok(k) if k > 0.0 => k as f32,
            _ => return,
        },
    };
    if SCALE.with(|s| s.replace(k)) == k {
        return;
    }
    for s in LIVE.with(|l| l.borrow_mut().alive()) {
        let (w, h) = s.size.lock().map(|s| *s).unwrap_or((0.0, 0.0));
        if w > 0.0 && h > 0.0 {
            let _ = s.resize_at(w, h, k);
        }
    }
}

impl Shade {
    fn new(host: &UIElement, kind: Kind, radius: f32, tones: Option<Tones>) -> windows_core::Result<Arc<Shade>> {
        let c: Compositor = CompositionTarget::GetCompositorForCurrentThread()?;
        let (tint, strength) = tones.unwrap_or_else(Tones::now).paint(kind);
        // the mask: the host's rounded rectangle, drawn by a shape visual into a surface
        let geometry = c.CreateRoundedRectangleGeometry()?;
        let fill = c.CreateSpriteShapeWithGeometry(&geometry)?;
        fill.SetFillBrush(&c.CreateColorBrushWithColor(Color { A: 255, R: 0, G: 0, B: 0 })?)?;
        let shape = c.CreateShapeVisual()?;
        shape.Shapes()?.Append(&fill)?;
        let surface = c.CreateVisualSurface()?;
        surface.SetSourceVisual(&shape)?;
        // cast by the mask's alpha (the rounded rectangle), not by the sprite's square bounds
        let cast_by = c.CreateSurfaceBrushWithSurface(&surface.cast::<ICompositionSurface>()?)?;
        let twin = || -> windows_core::Result<DropShadow> {
            let shadow = c.CreateDropShadow()?;
            shadow.SetColor(tint)?;
            shadow.SetMask(&cast_by)?;
            Ok(shadow)
        };
        let shadow = [twin()?, twin()?];
        // the sprite has no brush of its own: only its shadow is drawn
        let sprite = c.CreateSpriteVisual()?;
        sprite.SetShadow(&shadow[0])?;
        let clip = c.CreatePathGeometryWithPath(&hole(0.0, 0.0, 0.0, kind.reach())?)?;
        if mode() != Mode::NoClip {
            sprite.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
        }
        let root = c.CreateContainerVisual()?;
        root.Children()?.InsertAtTop(&sprite)?;
        let corners = Corners::new(&c, &root, &shadow[1])?;
        ElementCompositionPreview::SetElementChildVisual(host, &root)?;
        let shade = Arc::new(Shade {
            kind,
            root,
            clip,
            sprite,
            shape,
            surface,
            geometry,
            shadow,
            corners,
            radius,
            size: Mutex::new((0.0, 0.0)),
            tones,
            strength: AtomicU32::new(strength.to_bits()),
            cast: Mutex::new(Cast::NONE),
        });
        if SCALE.with(|s| s.get()) == 0.0 {
            SCALE.with(|s| s.set(scale_of(host)));
        }
        LIVE.with(|l| l.borrow_mut().add(&shade));
        Ok(shade)
    }

    /// The look's colours now (`now`, unless the shade has its own): the colour and strength
    /// change and the shadow in place is shown again in them, at once.
    fn repaint(&self, now: Tones) -> windows_core::Result<()> {
        let (tint, strength) = self.tones.unwrap_or(now).paint(self.kind);
        for s in &self.shadow {
            s.SetColor(tint)?;
        }
        self.strength.store(strength.to_bits(), Ordering::Relaxed);
        let cast = self.cast.lock().map(|c| *c).unwrap_or(Cast::NONE);
        self.show(cast, 0)
    }

    /// The host is `w` by `h` now: the sprite, the mask and its surface take that size, the clip
    /// reaches as far round them as the shadow can, less the host's square, and the corners
    /// take the rest of what lies outside its rounded rectangle.
    fn resize(&self, w: f32, h: f32) -> windows_core::Result<()> {
        self.resize_at(w, h, SCALE.with(|s| s.get()).max(1.0))
    }

    fn resize_at(&self, w: f32, h: f32, k: f32) -> windows_core::Result<()> {
        if let Ok(mut s) = self.size.lock() {
            *s = (w, h);
        }
        let m = self.kind.reach();
        let r = self.radius.min(w / 2.0).min(h / 2.0).max(0.0);
        let size = Vector2 { x: w, y: h };
        self.geometry.SetSize(size)?;
        self.geometry.SetCornerRadius(Vector2 { x: r, y: r })?;
        self.shape.SetSize(size)?;
        self.surface.SetSourceSize(size)?;
        self.sprite.SetSize(size)?;
        // the old way (dev) clipped the rounded rectangle itself away, aliased at its corners
        self.clip.SetPath(&hole(w, h, if mode() == Mode::Old { r } else { 0.0 }, m)?)?;
        self.corners.resize(w, h, r, k)
    }

    /// Show `cast`, over `ms` milliseconds on CSS's `ease` (at once for 0).
    fn show(&self, cast: Cast, ms: u32) -> windows_core::Result<()> {
        if let Ok(mut c) = self.cast.lock() {
            *c = cast;
        }
        let strength = f32::from_bits(self.strength.load(Ordering::Relaxed));
        let (opacity, blur, offset) = (cast.alpha * strength, cast.blur * BLUR, Vector3 { x: 0.0, y: cast.dy, z: 0.0 });
        let (o, b, f) = (HSTRING::from("Opacity"), HSTRING::from("BlurRadius"), HSTRING::from("Offset"));
        if ms == 0 {
            for s in &self.shadow {
                for p in [&o, &b, &f] {
                    s.StopAnimation(p)?;
                }
                s.SetOpacity(opacity)?;
                s.SetBlurRadius(blur)?;
                s.SetOffset(offset)?;
            }
            return Ok(());
        }
        let c = self.shadow[0].Compositor()?;
        let ease = c.CreateCubicBezierEasingFunction(Vector2 { x: 0.25, y: 0.1 }, Vector2 { x: 0.25, y: 1.0 })?;
        let time = windows_time::TimeSpan { duration: i64::from(ms) * 10_000 };
        // one animation of each, started on both: they run in step
        let mut scalars = Vec::new();
        for (p, to) in [(&o, opacity), (&b, blur)] {
            let a = c.CreateScalarKeyFrameAnimation()?;
            a.InsertKeyFrameWithEasingFunction(1.0, to, &ease)?;
            a.SetDuration(time)?;
            scalars.push((p, a));
        }
        let a = c.CreateVector3KeyFrameAnimation()?;
        a.InsertKeyFrameWithEasingFunction(1.0, offset, &ease)?;
        a.SetDuration(time)?;
        for s in &self.shadow {
            for (p, anim) in &scalars {
                s.StartAnimation(p, anim)?;
            }
            s.StartAnimation(&f, &a)?;
        }
        Ok(())
    }
}

/// An element's `Tag`, when it is a string.
fn tag(el: &FrameworkElement) -> Option<String> {
    let t = el.Tag().ok()?;
    Some(t.cast::<IReference<HSTRING>>().ok()?.Value().ok()?.to_string_lossy())
}

/// Hang shadows on every host in `root`, a piece of XAML just loaded (not yet in the window):
/// the tagged borders, and the hosts in the templates of the buttons. Each waits for its first
/// `SizeChanged`, so a shadow is made only for what is laid out, once.
pub(crate) fn watch(root: &UIElement) {
    walk(root, None, 0);
}

/// [`watch`], for XAML drawn in colours of its own (the Theme page's sample): its shadows keep
/// `tones` whatever the look in use.
pub(crate) fn watch_with(root: &UIElement, tones: Tones) {
    walk(root, Some(tones), 0);
}

fn walk(el: &UIElement, tones: Option<Tones>, depth: u32) {
    // a page is far shallower; this only guards a loop
    if depth > 256 {
        return;
    }
    if el.cast::<ButtonBase>().is_ok() {
        follow_button(el, tones);
        // what the button's flyout shows (a DropDownButton's menu, a Flyout's content)
        if let Ok(f) = el.cast::<Button>().and_then(|b| b.Flyout()) {
            flyout(&f, tones, depth);
        }
    } else if el.cast::<ToggleSwitch>().is_ok() {
        follow_switch(el, tones);
    } else if el.cast::<ComboBox>().is_ok() {
        follow_combo(el);
    } else if let Ok(fe) = el.cast::<FrameworkElement>() {
        if let Some(kind) = tag(&fe).as_deref().and_then(Kind::from_tag) {
            follow_host(&fe, kind, tones);
        }
        // a menu or flyout attached to a pane (the rows' menu, Search's More menu)
        if let Ok(f) = FlyoutBase::GetAttachedFlyout(&fe) {
            flyout(&f, tones, depth);
        }
    }
    if let Ok(p) = el.cast::<Panel>() {
        if let Ok(kids) = p.Children() {
            for i in 0..kids.Size().unwrap_or(0) {
                if let Ok(k) = kids.GetAt(i) {
                    walk(&k, tones, depth + 1);
                }
            }
        }
    } else if let Ok(b) = el.cast::<Border>() {
        if let Ok(c) = b.Child() {
            walk(&c, tones, depth + 1);
        }
    } else if let Ok(c) = el.cast::<ContentControl>() {
        if let Ok(c) = c.Content().and_then(|c| c.cast::<UIElement>()) {
            walk(&c, tones, depth + 1);
        }
    }
}

/// A flyout found on the page: drawn inside the window (a popup in a window of its own would cut
/// the menu shadow [`popups`] hangs on it at that window's edge), and what a `Flyout` shows
/// walked like the page, so its buttons get their shadows too.
fn flyout(f: &FlyoutBase, tones: Option<Tones>, depth: u32) {
    let _ = f.SetShouldConstrainToRootBounds(true);
    if let Ok(c) = f.cast::<Flyout>().and_then(|f| f.Content()) {
        walk(&c, tones, depth + 1);
    }
}

/// A combo box: its list is a popup in its template, there by its first size; kept inside the
/// window as a flyout is.
fn follow_combo(el: &UIElement) {
    let Ok(fe) = el.cast::<FrameworkElement>() else { return };
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let Ok(handle) = fe.SizeChanged(move |s, _| {
        if done.swap(true, Ordering::Relaxed) {
            return;
        }
        let Some(fe) = s.as_ref().and_then(|s| s.cast::<FrameworkElement>().ok()) else { return };
        if let Some(p) = first::<Popup>(&fe, 0) {
            let _ = p.SetShouldConstrainToRootBounds(true);
        }
    }) else {
        return;
    };
    keep(handle);
}

/// The first element of type `T` under `el` in the visual tree.
fn first<T: Interface>(el: &FrameworkElement, depth: u32) -> Option<T> {
    if depth > 16 {
        return None;
    }
    let n = VisualTreeHelper::GetChildrenCount(el).ok()?;
    for i in 0..n {
        let Ok(k) = VisualTreeHelper::GetChild(el, i) else { continue };
        if let Ok(t) = k.cast::<T>() {
            return Some(t);
        }
        if let Some(found) = k.cast::<FrameworkElement>().ok().and_then(|k| first::<T>(&k, depth + 1)) {
            return Some(found);
        }
    }
    None
}

/// A tagged border: its shadow is made at its first size and kept to its size after.
fn follow_host(host: &FrameworkElement, kind: Kind, tones: Option<Tones>) {
    let made: Arc<Mutex<Option<Arc<Shade>>>> = Arc::new(Mutex::new(None));
    let Ok(handle) = host.SizeChanged(move |s, _| {
        let Some(fe) = s.as_ref().and_then(|s| s.cast::<FrameworkElement>().ok()) else { return };
        let (Ok(w), Ok(h)) = (fe.ActualWidth(), fe.ActualHeight()) else { return };
        let Ok(mut made) = made.lock() else { return };
        if made.is_none() {
            let radius = fe.cast::<Border>().and_then(|b| b.CornerRadius()).map(|c| c.TopLeft as f32).unwrap_or(0.0);
            let Ok(el) = fe.cast::<UIElement>() else { return };
            let Ok(shade) = Shade::new(&el, kind, radius, tones) else { return };
            let _ = shade.show(kind.rest(), 0);
            *made = Some(shade);
        }
        if let Some(shade) = made.as_ref() {
            let _ = shade.resize(w as f32, h as f32);
        }
    }) else {
        return;
    };
    keep(handle);
}

/// A button: at its first size its template is there, and with it the host (`Shade`, a tagged
/// border first in the template's outer grid) and the states the shadow follows.
fn follow_button(el: &UIElement, tones: Option<Tones>) {
    let Ok(fe) = el.cast::<FrameworkElement>() else { return };
    // None: not looked yet; Some(None): looked, no shadow
    let made: Arc<Mutex<Option<Option<Arc<Shade>>>>> = Arc::new(Mutex::new(None));
    let Ok(handle) = fe.SizeChanged(move |s, _| {
        let Some(fe) = s.as_ref().and_then(|s| s.cast::<FrameworkElement>().ok()) else { return };
        let (Ok(w), Ok(h)) = (fe.ActualWidth(), fe.ActualHeight()) else { return };
        let Ok(mut made) = made.lock() else { return };
        if made.is_none() {
            *made = Some(button_shade(&fe, tones));
        }
        if let Some(Some(shade)) = made.as_ref() {
            let _ = shade.resize(w as f32, h as f32);
        }
    }) else {
        return;
    };
    keep(handle);
}

/// A switch: its knob's host is in its template (look.rs's `switch_template`), there by its first
/// size. The knob slides and the switch sinks by render transforms, which carry the shadow along.
fn follow_switch(el: &UIElement, tones: Option<Tones>) {
    let Ok(fe) = el.cast::<FrameworkElement>() else { return };
    // None: not looked yet; Some(None): looked, no shadow. The shade is kept with the switch, for
    // a theme change to reach it (the knob's size never changes).
    let made: Arc<Mutex<Option<Option<Arc<Shade>>>>> = Arc::new(Mutex::new(None));
    let Ok(handle) = fe.SizeChanged(move |s, _| {
        let Some(fe) = s.as_ref().and_then(|s| s.cast::<FrameworkElement>().ok()) else { return };
        let Ok(mut made) = made.lock() else { return };
        if made.is_some() {
            return;
        }
        *made = Some((|| {
            // the outlined (Windows stock) switch has no host: its knob casts nothing
            let host = tagged(&fe, "shade:knob", 0)?;
            // the knob's size is written in the template, laid out or not
            let (w, h) = (host.Width().ok()?, host.Height().ok()?);
            let radius = host.cast::<Border>().and_then(|b| b.CornerRadius()).map(|c| c.TopLeft as f32).unwrap_or(0.0);
            let shade = Shade::new(&host.cast::<UIElement>().ok()?, Kind::Knob, radius, tones).ok()?;
            let _ = shade.show(Kind::Knob.rest(), 0);
            let _ = shade.resize(w as f32, h as f32);
            Some(shade)
        })());
    }) else {
        return;
    };
    keep(handle);
}

// ---------------------------------------------------------------- menus, flyouts and lists

/// Called each frame: every popup open in the window whose content is a menu, a flyout or a
/// dropdown's list gets the menu shadow, once, in place of WinUI's own ThemeShadow (turned off
/// there: no doubled shadow). A popup in a window of its own (one this app could not keep inside
/// the window) keeps WinUI's shadow: a shadow of ours would be cut at that window's edge.
pub(crate) fn popups(root: &UIElement) {
    let Ok(xr) = root.XamlRoot() else { return };
    let Ok(open) = VisualTreeHelper::GetOpenPopupsForXamlRoot(&xr) else { return };
    for i in 0..open.Size().unwrap_or(0) {
        let Ok(p) = open.GetAt(i) else { continue };
        let Ok(content) = p.Child() else { continue };
        let Some((what, host, body)) = surface(&content) else { continue };
        // done already: the shade is the host's child visual. A list is placed again, each
        // frame: a combo box puts its list where its picked item lines up, open after open.
        if let Ok(sprite) = ElementCompositionPreview::GetElementChildVisual(&host) {
            if what == Surface::List {
                let _ = body.ActualOffset().and_then(|o| sprite.SetOffset(Vector3 { z: 0.0, ..o }));
            }
            continue;
        }
        // A MenuFlyout opens in a window of its own whatever ShouldConstrainToRootBounds says,
        // and that window is only about 10 px wider than the menu each way, at any lift: the
        // menu shadow's 28 px blur would be cut square. WinUI's ThemeShadow stays there.
        if !p.IsConstrainedToRootBounds().unwrap_or(false) {
            continue;
        }
        let t0 = std::time::Instant::now();
        let made = hang_menu(&host, &body, what);
        crate::state::timing(&format!("menu shadow on a {what:?}: {}", if made.is_ok() { "made" } else { "failed" }), t0);
    }
}

/// What a popup's content is, when the menu shadow goes under it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Surface {
    Menu,
    Flyout,
    /// A combo box's list (its template's `PopupBorder`).
    List,
}

/// What a popup's content is, the element the shadow hangs on, and the surface it is cast by:
/// a menu's or flyout's presenter, both. A combo box's list is a border on a canvas, after the
/// canvas that catches a click outside it; the shadow hangs on that outer canvas, placed at the
/// border (hung on the border itself, it is made but never drawn).
fn surface(content: &UIElement) -> Option<(Surface, UIElement, UIElement)> {
    if content.cast::<MenuFlyoutPresenter>().is_ok() {
        return Some((Surface::Menu, content.clone(), content.clone()));
    }
    if content.cast::<FlyoutPresenter>().is_ok() {
        return Some((Surface::Flyout, content.clone(), content.clone()));
    }
    let kids = content.cast::<Panel>().ok()?.Children().ok()?;
    let n = kids.Size().ok()?;
    let border = (0..n).rev().filter_map(|i| kids.GetAt(i).ok()).find(|k| k.cast::<Border>().is_ok())?;
    Some((Surface::List, content.clone(), border))
}

/// WinUI's shadow off, and ours on `host`, kept to `body`'s size and place.
fn hang_menu(host: &UIElement, body: &UIElement, what: Surface) -> windows_core::Result<()> {
    match what {
        Surface::Menu => body.cast::<MenuFlyoutPresenter>()?.SetIsDefaultShadowEnabled(false)?,
        Surface::Flyout => body.cast::<FlyoutPresenter>()?.SetIsDefaultShadowEnabled(false)?,
        Surface::List => {}
    }
    let child = body;
    // the ThemeShadow WinUI casts, and the lift (Translation's z) that sizes it
    child.SetShadow(None::<&crate::bindings::Microsoft::UI::Xaml::Media::Shadow>)?;
    let t = child.Translation()?;
    if t.z != 0.0 {
        child.SetTranslation(Vector3 { z: 0.0, ..t })?;
    }
    let fe: FrameworkElement = child.cast()?;
    let radius = match child.cast::<Control>() {
        Ok(c) => c.CornerRadius()?.TopLeft,
        Err(_) => child.cast::<Border>()?.CornerRadius()?.TopLeft,
    } as f32;
    let shade = Shade::new(host, Kind::Menu, radius, None)?;
    if host != body {
        let o = body.ActualOffset()?;
        shade.root.SetOffset(Vector3 { z: 0.0, ..o })?;
    }
    shade.show(Kind::Menu.rest(), 0)?;
    let (w, h) = (fe.ActualWidth()?, fe.ActualHeight()?);
    if w > 0.0 && h > 0.0 {
        shade.resize(w as f32, h as f32)?;
    }
    keep(fe.SizeChanged(move |s, _| {
        let Some(fe) = s.as_ref().and_then(|s| s.cast::<FrameworkElement>().ok()) else { return };
        let (Ok(w), Ok(h)) = (fe.ActualWidth(), fe.ActualHeight()) else { return };
        let _ = shade.resize(w as f32, h as f32);
    })?);
    Ok(())
}

/// The first element under `el` in the visual tree whose `Tag` is `want`.
fn tagged(el: &FrameworkElement, want: &str, depth: u32) -> Option<FrameworkElement> {
    if depth > 32 {
        return None;
    }
    let n = VisualTreeHelper::GetChildrenCount(el).ok()?;
    for i in 0..n {
        let Some(k) = VisualTreeHelper::GetChild(el, i).ok().and_then(|k| k.cast::<FrameworkElement>().ok()) else { continue };
        if tag(&k).as_deref() == Some(want) {
            return Some(k);
        }
        if let Some(found) = tagged(&k, want, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// Whether a button has a face of its own: a transparent one (a text action, a row drawn by
/// what is behind it) casts no shadow, as a CSS box with no background still would, but the
/// mockup gives those none.
fn has_face(button: &FrameworkElement) -> bool {
    let Ok(bg) = button.cast::<Control>().and_then(|c| c.Background()) else { return false };
    match bg.cast::<SolidColorBrush>() {
        Ok(solid) => solid.Color().map(|c| c.A > 0).unwrap_or(false),
        // a gradient or any other brush is a face
        Err(_) => true,
    }
}

fn button_shade(button: &FrameworkElement, tones: Option<Tones>) -> Option<Arc<Shade>> {
    let root: FrameworkElement = VisualTreeHelper::GetChild(button, 0).ok()?.cast().ok()?;
    let n = VisualTreeHelper::GetChildrenCount(&root).ok()?;
    let (host, mut kind) = (0..n.min(2)).find_map(|i| {
        let h: FrameworkElement = VisualTreeHelper::GetChild(&root, i).ok()?.cast().ok()?;
        let k = tag(&h).as_deref().and_then(Kind::from_tag)?;
        Some((h, k))
    })?;
    // the button's own tag says what it is when it is not a plain button (an Advisor row), or
    // that it has no shadow
    match tag(button).as_deref() {
        Some("shade:none") => return None,
        Some(t) => kind = Kind::from_tag(t).unwrap_or(kind),
        None => {}
    }
    if kind == Kind::Button && !has_face(button) {
        return None;
    }
    let radius = button.cast::<Control>().and_then(|c| c.CornerRadius()).map(|c| c.TopLeft as f32).unwrap_or(0.0);
    let shade = Shade::new(&host.cast().ok()?, kind, radius, tones).ok()?;
    let (lift, _) = crate::look::shadow_motion();
    let mut followed = false;
    if let Ok(groups) = VisualStateManager::GetVisualStateGroups(&root) {
        for i in 0..groups.Size().unwrap_or(0) {
            let Ok(g) = groups.GetAt(i) else { continue };
            if g.Name().map(|n| n != "CommonStates").unwrap_or(true) {
                continue;
            }
            let now = g.CurrentState().and_then(|s| s.Name()).map(|n| n.to_string_lossy()).unwrap_or_default();
            let _ = shade.show(kind.pose(&now, lift), 0);
            let s = shade.clone();
            if let Ok(handle) = g.CurrentStateChanged(move |_, e| {
                let Some(state) = e.as_ref().and_then(|e| e.NewState().ok()).and_then(|s| s.Name().ok()) else { return };
                let (lift, ms) = crate::look::shadow_motion();
                let _ = s.show(kind.pose(&state.to_string_lossy(), lift), ms);
            }) {
                keep(handle);
                followed = true;
            }
        }
    }
    if !followed {
        let _ = shade.show(kind.rest(), 0);
    }
    Some(shade)
}

// ---------------------------------------------------------------- the clip, for Composition

/// Composition reads a path's shape from Direct2D through this interface (d2d1_1.h's
/// `IGeometrySource2DInterop`, which Win2D's geometries implement too).
#[interface("0657AF73-53FD-47CF-84FF-C8492D2A80A3")]
unsafe trait IGeometrySource2DInterop: windows_core::IUnknown {
    fn GetGeometry(&self, value: *mut *mut core::ffi::c_void) -> HRESULT;
    fn TryGetGeometryUsingFactory(&self, factory: *mut core::ffi::c_void, value: *mut *mut core::ffi::c_void) -> HRESULT;
}

/// Everything outside a `w` by `h` rectangle with corners of radius `r`, out to `reach` past it:
/// the part of the page a shadow may darken.
#[implement(IGeometrySource2D, IGeometrySource2DInterop)]
struct Hole {
    w: f32,
    h: f32,
    r: f32,
    reach: f32,
}

/// A Composition path of [`Hole`].
fn hole(w: f32, h: f32, r: f32, reach: f32) -> windows_core::Result<CompositionPath> {
    let source: IGeometrySource2D = Hole { w, h, r, reach }.into();
    CompositionPath::Create(&source)
}

thread_local! {
    static FACTORY: Option<windows::Win32::Graphics::Direct2D::ID2D1Factory> = unsafe {
        windows::Win32::Graphics::Direct2D::D2D1CreateFactory(windows::Win32::Graphics::Direct2D::D2D1_FACTORY_TYPE_MULTI_THREADED, None).ok()
    };
}

impl Hole {
    /// The shape as a Direct2D path from `factory`: the outer rectangle and the rounded one in
    /// it, filled alternately, so the rounded one is a hole. The corners are true arcs.
    fn build(&self, factory: &windows::Win32::Graphics::Direct2D::ID2D1Factory) -> windows::core::Result<windows::Win32::Graphics::Direct2D::ID2D1Geometry> {
        use windows::Win32::Graphics::Direct2D::Common::{D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_FILL_MODE_ALTERNATE, D2D_SIZE_F};
        use windows::Win32::Graphics::Direct2D::{D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL, D2D1_SWEEP_DIRECTION_CLOCKWISE};
        let p = |x: f32, y: f32| d2d_numerics::Vector2 { X: x, Y: y };
        let (w, h, r, m) = (self.w, self.h, self.r, self.reach);
        unsafe {
            let path = factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.SetFillMode(D2D1_FILL_MODE_ALTERNATE);
            sink.BeginFigure(p(-m, -m), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(p(w + m, -m));
            sink.AddLine(p(w + m, h + m));
            sink.AddLine(p(-m, h + m));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            if w > 0.0 && h > 0.0 {
                let arc = |x: f32, y: f32| D2D1_ARC_SEGMENT { point: p(x, y), size: D2D_SIZE_F { width: r, height: r }, rotationAngle: 0.0, sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE, arcSize: D2D1_ARC_SIZE_SMALL };
                sink.BeginFigure(p(r, 0.0), D2D1_FIGURE_BEGIN_FILLED);
                for (line, corner) in [((w - r, 0.0), (w, r)), ((w, h - r), (w - r, h)), ((r, h), (0.0, h - r)), ((0.0, r), (r, 0.0))] {
                    sink.AddLine(p(line.0, line.1));
                    if r > 0.0 {
                        sink.AddArc(&arc(corner.0, corner.1));
                    }
                }
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            }
            sink.Close()?;
            windows::core::Interface::cast(&path)
        }
    }

    /// Hand the geometry out as Direct2D's COM interface (the windows crate's own types are a
    /// different windows-core from the bindings', so it crosses as a raw pointer).
    unsafe fn give(&self, factory: &windows::Win32::Graphics::Direct2D::ID2D1Factory, value: *mut *mut core::ffi::c_void) -> HRESULT {
        if value.is_null() {
            return HRESULT(0x8000_4003_u32 as i32); // E_POINTER
        }
        match self.build(factory) {
            Ok(g) => {
                unsafe { *value = windows::core::Interface::into_raw(g) };
                HRESULT(0)
            }
            Err(e) => HRESULT(e.code().0),
        }
    }
}

impl IGeometrySource2D_Impl for Hole_Impl {}

impl IGeometrySource2DInterop_Impl for Hole_Impl {
    unsafe fn GetGeometry(&self, value: *mut *mut core::ffi::c_void) -> HRESULT {
        match FACTORY.with(|f| f.clone()) {
            Some(f) => unsafe { self.give(&f, value) },
            None => HRESULT(0x8000_4005_u32 as i32), // E_FAIL
        }
    }

    unsafe fn TryGetGeometryUsingFactory(&self, factory: *mut core::ffi::c_void, value: *mut *mut core::ffi::c_void) -> HRESULT {
        match unsafe { <windows::Win32::Graphics::Direct2D::ID2D1Factory as windows::core::Interface>::from_raw_borrowed(&factory) } {
            Some(f) => unsafe { self.give(f, value) },
            None => self.GetGeometry(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values are the mockup's CSS (index.html, data-fancy=depth and data-press=lift).
    #[test]
    fn each_kind_casts_the_mockups_shadow() {
        let a = |byte: u8| byte as f32 / 255.0;
        assert_eq!(Kind::Card.rest(), Cast { dy: 8.0, blur: 22.0, alpha: a(0x52) });
        assert_eq!(Kind::Button.rest(), Cast { dy: 1.0, blur: 2.0, alpha: a(0x4d) });
        assert_eq!(Kind::Button.over(), Cast { dy: 6.0, blur: 14.0, alpha: a(0x66) });
        assert_eq!(Kind::Primary.rest(), Cast { dy: 3.0, blur: 10.0, alpha: 0.45 });
        assert_eq!(Kind::Primary.over(), Cast { dy: 6.0, blur: 16.0, alpha: 0.48 });
        assert_eq!(Kind::Row.rest(), Cast { dy: 1.0, blur: 2.0, alpha: a(0x33) });
        assert_eq!(Kind::Row.over(), Cast { dy: 5.0, blur: 12.0, alpha: a(0x59) });
        assert_eq!(Kind::Dialog.rest(), Cast { dy: 10.0, blur: 28.0, alpha: a(0x99) });
        // the switch's knob: 0 1px 3px #00000066, the same under the pointer
        assert_eq!(Kind::Knob.rest(), Cast { dy: 1.0, blur: 3.0, alpha: a(0x66) });
        assert_eq!(Kind::Knob.over(), Kind::Knob.rest());
        for (tag, kind) in [("shade:card", Kind::Card), ("shade:dialog", Kind::Dialog), ("shade:button", Kind::Button), ("shade:primary", Kind::Primary), ("shade:row", Kind::Row), ("shade:knob", Kind::Knob)] {
            assert_eq!(Kind::from_tag(tag), Some(kind));
        }
        assert_eq!(Kind::from_tag("card"), None);
        assert_eq!(Kind::from_tag("shade:none"), None);
    }

    /// Lift and sink: up under the pointer, none while pressed; the other presses keep the
    /// resting shadow; disabled has none.
    #[test]
    fn a_buttons_shadow_follows_its_state() {
        for kind in [Kind::Button, Kind::Primary, Kind::Row] {
            assert_eq!(kind.pose("Normal", true), kind.rest());
            assert_eq!(kind.pose("PointerOver", true), kind.over());
            assert_eq!(kind.pose("Pressed", true), Cast::NONE);
            assert_eq!(kind.pose("Disabled", true), Cast::NONE);
            assert_eq!(kind.pose("PointerOver", false), kind.rest());
            assert_eq!(kind.pose("Pressed", false), kind.rest());
            assert_eq!(kind.pose("", true), kind.rest(), "no state yet is the resting one");
        }
        // the clip reaches past the furthest the shadow goes
        assert!(Kind::Card.reach() >= 8.0 + 22.0 * BLUR);
        assert!(Kind::Primary.reach() >= 6.0 + 16.0 * BLUR);
    }

    /// Menus, flyouts and dropdown lists: the mockup's `.menu`, 0 10px 28px #00000099, still
    /// under the pointer.
    #[test]
    fn a_menu_casts_the_mockups_menu_shadow() {
        assert_eq!(Kind::Menu.rest(), Cast { dy: 10.0, blur: 28.0, alpha: 0x99 as f32 / 255.0 });
        assert_eq!(Kind::Menu.over(), Kind::Menu.rest());
        assert_eq!(Kind::from_tag("shade:menu"), Some(Kind::Menu));
        assert!(Kind::Menu.reach() >= 10.0 + 28.0 * BLUR);
    }

    /// The colours come from the tokens given: the glow in the accent, the rest in the shadow
    /// colour; the strength is the shadow token's alpha (softer on the light page).
    #[test]
    fn a_shadow_is_painted_from_its_tones() {
        let dark: std::collections::HashMap<&'static str, String> = [("shadow", "#FF000000".to_string()), ("accent", "#FF8AB4FF".to_string())].into_iter().collect();
        let t = Tones::of(&dark);
        assert_eq!(t.paint(Kind::Primary), (Color { A: 255, R: 0x8A, G: 0xB4, B: 0xFF }, 1.0));
        assert_eq!(t.paint(Kind::Card), (Color { A: 255, R: 0, G: 0, B: 0 }, 1.0));
        let light: std::collections::HashMap<&'static str, String> = [("shadow", "#8C1B2A60".to_string()), ("accent", "#FF0078D4".to_string())].into_iter().collect();
        let (tint, strength) = Tones::of(&light).paint(Kind::Menu);
        assert_eq!(tint, Color { A: 255, R: 0x1B, G: 0x2A, B: 0x60 });
        assert!((strength - 0x8C as f32 / 255.0).abs() < 1e-6);
        // another accent is another glow, the same shadow under everything else
        let other: std::collections::HashMap<&'static str, String> = [("shadow", "#FF000000".to_string()), ("accent", "#FFF04E7A".to_string())].into_iter().collect();
        assert_ne!(Tones::of(&other).paint(Kind::Primary), t.paint(Kind::Primary));
        assert_eq!(Tones::of(&other).paint(Kind::Button), t.paint(Kind::Button));
        // the look's own tables carry both tokens, in both themes
        for light in [false, true] {
            let table = crate::tokens::table(&crate::tokens::Look::default(), light);
            let t = Tones::of(&table);
            assert_eq!(Some(t.shadow), colour(&table["shadow"]));
            assert_eq!(Some(t.accent), colour(&table["accent"]));
        }
    }

    /// The list of shadows forgets the dead and keeps the living, however many come and go.
    #[test]
    fn the_list_of_shadows_keeps_only_the_living() {
        let mut live: Live<u32> = Live::new();
        let mut kept = Vec::new();
        for i in 0..1000u32 {
            let a = Arc::new(i);
            live.add(&a);
            if i % 10 == 0 {
                kept.push(a);
            }
        }
        assert!(live.items.len() < 1000, "swept on the way: {}", live.items.len());
        let alive: Vec<u32> = live.alive().iter().map(|a| **a).collect();
        assert_eq!(alive, (0..1000).step_by(10).collect::<Vec<u32>>());
        assert_eq!(live.items.len(), 100);
        drop(kept);
        assert!(live.alive().is_empty());
    }

    /// A corner's mask steps from clear to opaque across half a screen pixel, half a screen pixel
    /// inside the arc (behind the face's own antialiased edge the shadow stays), at any scale.
    #[test]
    fn a_corners_step_lies_just_inside_its_arc() {
        for k in [1.0f32, 1.25, 1.5, 2.0] {
            for r in [2.0f32, 4.0, 7.0, 16.0] {
                let (clear, opaque, reach) = arc_stops(r, k);
                assert_eq!(reach, r);
                assert!(0.0 < clear && clear < opaque && opaque < 1.0, "{r} at {k}");
                assert!(((opaque - clear) * r * k - 0.5).abs() < 1e-4, "half a pixel wide: {r} at {k}");
                assert!(((r - (opaque + clear) / 2.0 * r) * k - 0.5).abs() < 1e-4, "half a pixel inside: {r} at {k}");
            }
        }
    }

    #[test]
    fn colours_read_as_the_tokens_write_them() {
        assert_eq!(colour("#8C1B2A60"), Some(Color { A: 0x8C, R: 0x1B, G: 0x2A, B: 0x60 }));
        assert_eq!(colour("#0078D4"), Some(Color { A: 0xFF, R: 0x00, G: 0x78, B: 0xD4 }));
        assert_eq!(colour("nope"), None);
    }
}
