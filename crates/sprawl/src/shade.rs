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
//! - a clip on the sprite to everything OUTSIDE the host's rounded rectangle: a CSS box-shadow
//!   is never drawn under its own box, so a see-through row or button shows what is behind it,
//!   not its shadow. The clip is a Direct2D path (a large rectangle with the rounded rectangle
//!   cut out of it) that Composition reads through `IGeometrySource2DInterop`.
//!
//! (A `LayerVisual` casting the shadow from its content looked simpler, but Composition drew
//! its shadow and content cut off by rectangles round the cards.)
//!
//! A host inside a button's template is reached from the button: the button's first
//! `SizeChanged` comes after its template is applied, even for a button first loaded inside a
//! collapsed panel. That shadow then follows the template's CommonStates (Normal, PointerOver,
//! Pressed, Disabled) with Composition animations on the Press option's time and CSS's `ease`.

use std::sync::{Arc, Mutex};

use windows_core::{implement, interface, Interface, HRESULT, HSTRING};
use windows_reference::IReference;

use crate::bindings::Microsoft::UI::Composition::{
    CompositionPath, CompositionPathGeometry, CompositionRoundedRectangleGeometry, CompositionVisualSurface, Compositor, DropShadow, ICompositionSurface, ShapeVisual, SpriteVisual,
};
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::ButtonBase;
use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, ContentControl, Control, Panel};
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
    /// A button with a face: `0 1px 2px #0000004d`, under the pointer `0 6px 14px #00000066`.
    Button,
    /// The main button, in the accent: `0 3px 10px` at 45%, under the pointer `0 6px 16px` at
    /// 48%.
    Primary,
    /// An Advisor row: `0 1px 2px #00000033`, under the pointer `0 5px 12px #00000059`.
    Row,
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
            "button" => Kind::Button,
            "primary" => Kind::Primary,
            "row" => Kind::Row,
            _ => return None,
        })
    }

    /// The shadow at rest.
    pub(crate) fn rest(self) -> Cast {
        match self {
            Kind::Card => Cast::css(8.0, 22.0, 0x52),
            Kind::Dialog => Cast::css(10.0, 28.0, 0x99),
            Kind::Button => Cast::css(1.0, 2.0, 0x4d),
            // color-mix(in srgb, accent 45%, transparent)
            Kind::Primary => Cast { dy: 3.0, blur: 10.0, alpha: 0.45 },
            Kind::Row => Cast::css(1.0, 2.0, 0x33),
        }
    }

    /// The shadow under the pointer when the item lifts (the Lift and sink press).
    pub(crate) fn over(self) -> Cast {
        match self {
            Kind::Button => Cast::css(6.0, 14.0, 0x66),
            Kind::Primary => Cast { dy: 6.0, blur: 16.0, alpha: 0.48 },
            Kind::Row => Cast::css(5.0, 12.0, 0x59),
            Kind::Card | Kind::Dialog => self.rest(),
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
fn colour(hex: &str) -> Option<Color> {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).ok()?;
    let v = if hex.len() == 7 { 0xFF00_0000 | v } else { v };
    Some(Color { A: (v >> 24) as u8, R: (v >> 16) as u8, G: (v >> 8) as u8, B: v as u8 })
}

/// The shadow's colour, opaque, and how strong the page's shadows are (the `shadow` token's
/// alpha: full on the dark page, softer on the light one). The main button's is its accent.
fn paint(kind: Kind) -> (Color, f32) {
    let shadow = colour(&crate::tokens::get("shadow")).unwrap_or(Color { A: 255, R: 0, G: 0, B: 0 });
    let strength = shadow.A as f32 / 255.0;
    let base = if kind == Kind::Primary { colour(&crate::tokens::get("accent")).unwrap_or(shadow) } else { shadow };
    (Color { A: 255, ..base }, strength)
}

/// One shadow, hung on its host.
struct Shade {
    kind: Kind,
    clip: CompositionPathGeometry,
    sprite: SpriteVisual,
    shape: ShapeVisual,
    surface: CompositionVisualSurface,
    geometry: CompositionRoundedRectangleGeometry,
    shadow: DropShadow,
    radius: f32,
    strength: f32,
}

impl Shade {
    fn new(host: &UIElement, kind: Kind, radius: f32) -> windows_core::Result<Shade> {
        let c: Compositor = CompositionTarget::GetCompositorForCurrentThread()?;
        let (tint, strength) = paint(kind);
        // the mask: the host's rounded rectangle, drawn by a shape visual into a surface
        let geometry = c.CreateRoundedRectangleGeometry()?;
        let fill = c.CreateSpriteShapeWithGeometry(&geometry)?;
        fill.SetFillBrush(&c.CreateColorBrushWithColor(Color { A: 255, R: 0, G: 0, B: 0 })?)?;
        let shape = c.CreateShapeVisual()?;
        shape.Shapes()?.Append(&fill)?;
        let surface = c.CreateVisualSurface()?;
        surface.SetSourceVisual(&shape)?;
        let shadow = c.CreateDropShadow()?;
        shadow.SetColor(tint)?;
        // cast by the mask's alpha (the rounded rectangle), not by the sprite's square bounds
        shadow.SetMask(&c.CreateSurfaceBrushWithSurface(&surface.cast::<ICompositionSurface>()?)?)?;
        // the sprite has no brush of its own: only its shadow is drawn
        let sprite = c.CreateSpriteVisual()?;
        sprite.SetShadow(&shadow)?;
        let clip = c.CreatePathGeometryWithPath(&hole(0.0, 0.0, 0.0, kind.reach())?)?;
        sprite.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
        ElementCompositionPreview::SetElementChildVisual(host, &sprite)?;
        Ok(Shade { kind, clip, sprite, shape, surface, geometry, shadow, radius, strength })
    }

    /// The host is `w` by `h` now: the sprite, the mask and its surface take that size, and
    /// the clip reaches as far round them as the shadow can, less the host's own rounded
    /// rectangle.
    fn resize(&self, w: f32, h: f32) -> windows_core::Result<()> {
        let m = self.kind.reach();
        let r = self.radius.min(w / 2.0).min(h / 2.0).max(0.0);
        let size = Vector2 { x: w, y: h };
        self.geometry.SetSize(size)?;
        self.geometry.SetCornerRadius(Vector2 { x: r, y: r })?;
        self.shape.SetSize(size)?;
        self.surface.SetSourceSize(size)?;
        self.sprite.SetSize(size)?;
        self.clip.SetPath(&hole(w, h, r, m)?)
    }

    /// Show `cast`, over `ms` milliseconds on CSS's `ease` (at once for 0).
    fn show(&self, cast: Cast, ms: u32) -> windows_core::Result<()> {
        let (opacity, blur, offset) = (cast.alpha * self.strength, cast.blur * BLUR, Vector3 { x: 0.0, y: cast.dy, z: 0.0 });
        let (o, b, f) = (HSTRING::from("Opacity"), HSTRING::from("BlurRadius"), HSTRING::from("Offset"));
        if ms == 0 {
            for p in [&o, &b, &f] {
                self.shadow.StopAnimation(p)?;
            }
            self.shadow.SetOpacity(opacity)?;
            self.shadow.SetBlurRadius(blur)?;
            return self.shadow.SetOffset(offset);
        }
        let c = self.shadow.Compositor()?;
        let ease = c.CreateCubicBezierEasingFunction(Vector2 { x: 0.25, y: 0.1 }, Vector2 { x: 0.25, y: 1.0 })?;
        let time = windows_time::TimeSpan { duration: i64::from(ms) * 10_000 };
        for (p, to) in [(&o, opacity), (&b, blur)] {
            let a = c.CreateScalarKeyFrameAnimation()?;
            a.InsertKeyFrameWithEasingFunction(1.0, to, &ease)?;
            a.SetDuration(time)?;
            self.shadow.StartAnimation(p, &a)?;
        }
        let a = c.CreateVector3KeyFrameAnimation()?;
        a.InsertKeyFrameWithEasingFunction(1.0, offset, &ease)?;
        a.SetDuration(time)?;
        self.shadow.StartAnimation(&f, &a)
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
    walk(root, 0);
}

fn walk(el: &UIElement, depth: u32) {
    // a page is far shallower; this only guards a loop
    if depth > 256 {
        return;
    }
    if el.cast::<ButtonBase>().is_ok() {
        follow_button(el);
    } else if let Ok(fe) = el.cast::<FrameworkElement>() {
        if let Some(kind) = tag(&fe).as_deref().and_then(Kind::from_tag) {
            follow_host(&fe, kind);
        }
    }
    if let Ok(p) = el.cast::<Panel>() {
        if let Ok(kids) = p.Children() {
            for i in 0..kids.Size().unwrap_or(0) {
                if let Ok(k) = kids.GetAt(i) {
                    walk(&k, depth + 1);
                }
            }
        }
    } else if let Ok(b) = el.cast::<Border>() {
        if let Ok(c) = b.Child() {
            walk(&c, depth + 1);
        }
    } else if let Ok(c) = el.cast::<ContentControl>() {
        if let Ok(c) = c.Content().and_then(|c| c.cast::<UIElement>()) {
            walk(&c, depth + 1);
        }
    }
}

/// A tagged border: its shadow is made at its first size and kept to its size after.
fn follow_host(host: &FrameworkElement, kind: Kind) {
    let made: Arc<Mutex<Option<Shade>>> = Arc::new(Mutex::new(None));
    let Ok(handle) = host.SizeChanged(move |s, _| {
        let Some(fe) = s.as_ref().and_then(|s| s.cast::<FrameworkElement>().ok()) else { return };
        let (Ok(w), Ok(h)) = (fe.ActualWidth(), fe.ActualHeight()) else { return };
        let Ok(mut made) = made.lock() else { return };
        if made.is_none() {
            let radius = fe.cast::<Border>().and_then(|b| b.CornerRadius()).map(|c| c.TopLeft as f32).unwrap_or(0.0);
            let Ok(el) = fe.cast::<UIElement>() else { return };
            let Ok(shade) = Shade::new(&el, kind, radius) else { return };
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
fn follow_button(el: &UIElement) {
    let Ok(fe) = el.cast::<FrameworkElement>() else { return };
    // None: not looked yet; Some(None): looked, no shadow
    let made: Arc<Mutex<Option<Option<Arc<Shade>>>>> = Arc::new(Mutex::new(None));
    let Ok(handle) = fe.SizeChanged(move |s, _| {
        let Some(fe) = s.as_ref().and_then(|s| s.cast::<FrameworkElement>().ok()) else { return };
        let (Ok(w), Ok(h)) = (fe.ActualWidth(), fe.ActualHeight()) else { return };
        let Ok(mut made) = made.lock() else { return };
        if made.is_none() {
            *made = Some(button_shade(&fe));
        }
        if let Some(Some(shade)) = made.as_ref() {
            let _ = shade.resize(w as f32, h as f32);
        }
    }) else {
        return;
    };
    keep(handle);
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

fn button_shade(button: &FrameworkElement) -> Option<Arc<Shade>> {
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
    let shade = Arc::new(Shade::new(&host.cast().ok()?, kind, radius).ok()?);
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
        for (tag, kind) in [("shade:card", Kind::Card), ("shade:dialog", Kind::Dialog), ("shade:button", Kind::Button), ("shade:primary", Kind::Primary), ("shade:row", Kind::Row)] {
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

    #[test]
    fn colours_read_as_the_tokens_write_them() {
        assert_eq!(colour("#8C1B2A60"), Some(Color { A: 0x8C, R: 0x1B, G: 0x2A, B: 0x60 }));
        assert_eq!(colour("#0078D4"), Some(Color { A: 0xFF, R: 0x00, G: 0x78, B: 0xD4 }));
        assert_eq!(colour("nope"), None);
    }
}
