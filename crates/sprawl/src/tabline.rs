//! The line under the page tabs: one 3 px accent line that slides from the old tab to the new
//! one, changing width as it goes (the mockup's `.uline`, docs/mockups/polish-2026-11-21:
//! `transition: left, width` over the motion time and its easing).
//!
//! [`MAIN`] describes the main page tabs. The XAML has an empty border, `TabLine`, along
//! the bottom of the tab row (`TabRow`). Once the
//! row is in the window the line is made as that border's child visual: a `ShapeVisual` holding
//! one rounded rectangle. Opening a tab ([`select`], from `paint_tabs`) starts two Composition
//! animations on that rectangle, its `Offset` and its geometry's `Size`, from where the line is
//! to the new tab's text, on the look's motion time and easing (the Lively overshoot is cut
//! into keyframes, which Composition's own bezier cannot hold). A layout change (the window,
//! the Advisor's count badge, a look with other fonts) puts the line on its tab at once, with
//! no slide. With the motion None, or Windows' "Animation effects" off, the line jumps.

use std::cell::RefCell;
use std::time::Instant;

use windows_numerics::Vector2;

use crate::bindings::Microsoft::UI::Composition::{CompositionColorBrush, CompositionRoundedRectangleGeometry, CompositionSpriteShape, Compositor, ShapeVisual};
use crate::bindings::Microsoft::UI::Xaml::Hosting::ElementCompositionPreview;
use crate::bindings::Microsoft::UI::Xaml::Media::CompositionTarget;
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement, Visibility};
use crate::state::{find, keep};
use windows_core::{Interface, HSTRING};

/// A row of tabs with a line under it: the border that holds the line (along the bottom of the
/// row, over the same cell as its buttons), the buttons' names, and how far the line is in from
/// a tab's edges, each side: the tabs' own padding, so the line is exactly as wide as the tab's
/// text, as the mockup's is.
pub(crate) struct Row {
    host: &'static str,
    tabs: &'static [&'static str],
    inset: f32,
    /// When a row carries its position across a rebuild, a line made again starts
    /// where the old one was, and slides from there when another tab is lit.
    carry: bool,
}

/// The main page tabs (Tree View ... About).
pub(crate) static MAIN: Row = Row { host: "TabLine", tabs: &["TabTree", "TabFiles", "TabAdvisor", "TabCleaners", "TabSearch", "TabAbout"], inset: 14.0, carry: false };
const HEIGHT: f32 = 3.0;

/// A cubic bezier easing, CSS's `cubic-bezier(x1, y1, x2, y2)`.
type Bezier = (f32, f32, f32, f32);

/// The time and the easing of the slide under a motion: the mockup's `--m` and `--me`.
pub(crate) fn timing(motion: crate::tokens::Motion) -> (u32, Bezier) {
    match motion {
        crate::tokens::Motion::Subtle => (160, (0.2, 0.8, 0.2, 1.0)),
        crate::tokens::Motion::Lively => (340, (0.3, 1.55, 0.5, 1.0)),
        crate::tokens::Motion::None => (0, (0.0, 0.0, 1.0, 1.0)),
    }
}

/// The eased fraction (CSS's output progress) at time fraction `t`: solve the curve's x for its
/// parameter, then read its y.
pub(crate) fn ease(b: Bezier, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let at = |s: f32, a: f32, c: f32| 3.0 * (1.0 - s).powi(2) * s * a + 3.0 * (1.0 - s) * s * s * c + s.powi(3);
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if at(mid, b.0, b.2) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at((lo + hi) / 2.0, b.1, b.3)
}

/// Where the line is, left edge and width.
type Span = (f32, f32);

/// A slide in progress: enough to say where the line is now, if another begins mid-way.
struct Slide {
    from: Span,
    to: Span,
    start: Instant,
    ms: u32,
    curve: Bezier,
}

impl Slide {
    fn now(&self) -> Span {
        let t = self.start.elapsed().as_secs_f32() * 1000.0 / self.ms.max(1) as f32;
        let e = ease(self.curve, t);
        (self.from.0 + (self.to.0 - self.from.0) * e, (self.from.1 + (self.to.1 - self.from.1) * e).max(0.0))
    }

    fn done(&self) -> bool {
        self.start.elapsed().as_millis() >= self.ms as u128
    }
}

struct Line {
    row: &'static Row,
    host: FrameworkElement,
    tabs: Vec<(&'static str, FrameworkElement)>,
    visual: ShapeVisual,
    rect: CompositionSpriteShape,
    geometry: CompositionRoundedRectangleGeometry,
    brush: CompositionColorBrush,
    /// Where the line was last put (its target, when sliding); None while it has no tab.
    at: Option<Span>,
    slide: Option<Slide>,
    /// Where the line of this row's last page ended, until this one has a place to slide from.
    carry: Option<Span>,
}

/// One row's line, and the tab lit, which is kept across the page being made again.
struct Slot {
    row: &'static Row,
    lit: Option<&'static str>,
    line: Option<Line>,
}

thread_local! {
    static SLOTS: RefCell<Vec<Slot>> = const { RefCell::new(Vec::new()) };
}

/// The slot of `row`, made at first use.
fn slot<'a>(slots: &'a mut Vec<Slot>, row: &'static Row) -> &'a mut Slot {
    let i = slots.iter().position(|s| std::ptr::eq(s.row, row)).unwrap_or_else(|| {
        slots.push(Slot { row, lit: None, line: None });
        slots.len() - 1
    });
    &mut slots[i]
}

impl Line {
    fn new(root: &FrameworkElement, row: &'static Row) -> windows_core::Result<Line> {
        let host: FrameworkElement = find(root, row.host)?;
        let c: Compositor = CompositionTarget::GetCompositorForCurrentThread()?;
        let geometry = c.CreateRoundedRectangleGeometry()?;
        geometry.SetCornerRadius(Vector2 { x: 1.5, y: 1.5 })?;
        let rect = c.CreateSpriteShapeWithGeometry(&geometry)?;
        let brush = c.CreateColorBrushWithColor(crate::shade::colour(&crate::look::underline()).unwrap_or(windows_ui_black()))?;
        rect.SetFillBrush(&brush)?;
        let visual = c.CreateShapeVisual()?;
        visual.Shapes()?.Append(&rect)?;
        ElementCompositionPreview::SetElementChildVisual(&host.cast::<UIElement>()?, &visual)?;
        let tabs = row.tabs.iter().filter_map(|n| Some((*n, find::<FrameworkElement>(root, n).ok()?))).collect();
        let line = Line { row, host, tabs, visual, rect, geometry, brush, at: None, slide: None, carry: None };
        // a layout change moves the line without a slide: the host's width (the window), a tab's
        // width (a badge came or went)
        let mut watched: Vec<&FrameworkElement> = line.tabs.iter().map(|t| &t.1).collect();
        watched.push(&line.host);
        for fe in watched {
            if let Ok(h) = fe.SizeChanged(move |_, _| settle(row)) {
                keep(h);
            }
        }
        Ok(line)
    }

    /// Where the lit tab's line goes: None with no tab, or one not laid out (or hidden).
    fn target(&self, lit: Option<&str>) -> Option<Span> {
        let (_, tab) = self.tabs.iter().find(|t| Some(t.0) == lit)?;
        if tab.cast::<UIElement>().ok()?.Visibility().ok()? != Visibility::Visible {
            return None;
        }
        let w = tab.ActualWidth().ok()? as f32 - 2.0 * self.row.inset;
        let x = tab.cast::<UIElement>().ok()?.ActualOffset().ok()?.x + self.row.inset;
        (w > 0.0).then_some((x, w))
    }

    /// Put the line at `to` at once.
    fn jump(&mut self, to: Option<Span>) -> windows_core::Result<()> {
        let (o, s) = (HSTRING::from("Offset"), HSTRING::from("Size"));
        self.rect.StopAnimation(&o)?;
        self.geometry.StopAnimation(&s)?;
        let (x, w) = to.unwrap_or((0.0, 0.0));
        self.rect.SetOffset(Vector2 { x, y: 0.0 })?;
        self.geometry.SetSize(Vector2 { x: w, y: HEIGHT })?;
        self.slide = None;
        self.at = to;
        Ok(())
    }

    /// Slide the line from where it is to `to`, over `ms` on `curve`.
    fn slide_to(&mut self, from: Span, to: Span, ms: u32, curve: Bezier) -> windows_core::Result<()> {
        let c: Compositor = self.rect.Compositor()?;
        // the curve, as keyframes on Composition's own linear easing: the overshoot of the
        // Lively one goes past 1, which a cubic bezier easing function cannot
        const STEPS: u32 = 40;
        let pos = c.CreateVector2KeyFrameAnimation()?;
        let size = c.CreateVector2KeyFrameAnimation()?;
        for i in 1..=STEPS {
            let t = i as f32 / STEPS as f32;
            let e = ease(curve, t);
            let lerp = |a: f32, b: f32| a + (b - a) * e;
            pos.InsertKeyFrame(t, Vector2 { x: lerp(from.0, to.0), y: 0.0 })?;
            size.InsertKeyFrame(t, Vector2 { x: lerp(from.1, to.1).max(0.0), y: HEIGHT })?;
        }
        let time = windows_time::TimeSpan { duration: i64::from(ms) * 10_000 };
        pos.SetDuration(time)?;
        size.SetDuration(time)?;
        // rest values first: if the animation is cut off, the line is where it ends
        self.rect.SetOffset(Vector2 { x: to.0, y: 0.0 })?;
        self.geometry.SetSize(Vector2 { x: to.1, y: HEIGHT })?;
        self.rect.StartAnimation(&HSTRING::from("Offset"), &pos)?;
        self.geometry.StartAnimation(&HSTRING::from("Size"), &size)?;
        self.slide = Some(Slide { from, to, start: Instant::now(), ms, curve });
        self.at = Some(to);
        Ok(())
    }

    /// The line's colour (the accent, for the theme in use) and its host's size.
    fn dress(&self) -> windows_core::Result<()> {
        if let Some(c) = crate::shade::colour(&crate::look::underline()) {
            self.brush.SetColor(c)?;
        }
        let (w, h) = (self.host.ActualWidth()? as f32, self.host.ActualHeight()? as f32);
        self.visual.SetSize(Vector2 { x: w, y: h })
    }

    /// Bring the line to the tab `lit`. With `animate` it slides when it can: it has a place now
    /// (or the last page's line had one), the tab has one, and the motion is not None.
    fn go(&mut self, lit: Option<&str>, animate: bool) {
        let _ = self.dress();
        let to = self.target(lit);
        if self.at == to && self.slide.as_ref().is_none_or(|s| s.done()) {
            if self.slide.as_ref().is_some_and(|s| s.done()) {
                self.slide = None;
            }
            return;
        }
        let (ms, curve) = timing(crate::look::motion());
        let mut from = self.slide.as_ref().map(|s| s.now()).or(self.at);
        // the first place of a line made again: where the last page's line was, once
        if from.is_none() && to.is_some() {
            from = self.carry.take();
        }
        let _ = match (animate && ms > 0, from, to) {
            (true, Some(from), Some(to)) if from != to => self.slide_to(from, to, ms, curve),
            _ => self.jump(to),
        };
    }
}

fn windows_ui_black() -> crate::bindings::Windows::UI::Color {
    crate::bindings::Windows::UI::Color { A: 255, R: 0, G: 0, B: 0 }
}

/// The layout changed: put the line on its tab at once.
fn settle(row: &'static Row) {
    // a handler can run while the line is borrowed (setting a size lays out again): skip it,
    // the call that holds it ends with the line placed
    SLOTS.with(|l| {
        if let Ok(mut l) = l.try_borrow_mut() {
            let s = slot(&mut l, row);
            let lit = s.lit;
            if let Some(line) = s.line.as_mut() {
                // a slide on its way (a badge came or went as the tab opened) goes on to the
                // tab's new place, and so does a line made again with a place to start from; a
                // line at rest is put on its tab at once
                let sliding = line.slide.as_ref().is_some_and(|s| !s.done()) || line.carry.is_some();
                line.go(lit, sliding);
            }
        }
    });
}

/// Open the tab named `lit` (None: no tab is lit) in `row`: the line slides to it. The line is
/// made at the first call for a page, and a page made again (a new theme or look) gets a new
/// one, put straight on its tab; a row that `carry`s starts its new line where the old one was,
/// when another tab is lit.
pub(crate) fn select(root: &FrameworkElement, row: &'static Row, lit: Option<&str>) {
    let lit = lit.and_then(|n| row.tabs.iter().copied().find(|t| *t == n));
    let Ok(host) = find::<FrameworkElement>(root, row.host) else { return };
    // the pills option has no line
    let pills = crate::tokens::current().tabs == crate::tokens::Tabs::Pills;
    let _ = host.cast::<UIElement>().and_then(|h| h.SetVisibility(if pills { Visibility::Collapsed } else { Visibility::Visible }));
    SLOTS.with(|l| {
        let Ok(mut l) = l.try_borrow_mut() else { return };
        let s = slot(&mut l, row);
        let changed = s.lit != lit;
        s.lit = lit;
        let same = s.line.as_ref().is_some_and(|line| line.host.as_raw() == host.as_raw());
        if !same {
            let carry = s.line.as_ref().filter(|_| row.carry && changed).and_then(|line| line.at);
            s.line = Line::new(root, row).ok().map(|mut line| {
                line.carry = carry;
                line
            });
        }
        if let Some(line) = s.line.as_mut() {
            line.go(lit, same || line.carry.is_some());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_easing_runs_from_nothing_to_all() {
        for m in [crate::tokens::Motion::Subtle, crate::tokens::Motion::Lively] {
            let (_, b) = timing(m);
            assert!(ease(b, 0.0).abs() < 1e-3 && (ease(b, 1.0) - 1.0).abs() < 1e-3, "{m:?}");
        }
    }

    /// Subtle leaves quickly and settles; Lively goes past its mark and comes back.
    #[test]
    fn subtle_settles_and_lively_overshoots() {
        let (_, subtle) = timing(crate::tokens::Motion::Subtle);
        assert!(ease(subtle, 0.5) > 0.8 && (0..=100).all(|i| ease(subtle, i as f32 / 100.0) <= 1.001));
        let (ms, lively) = timing(crate::tokens::Motion::Lively);
        assert_eq!(ms, 340);
        assert!((0..=100).any(|i| ease(lively, i as f32 / 100.0) > 1.05), "it passes the mark");
    }

    #[test]
    fn main_tab_line_reserves_its_position_for_the_installed_page() {
        assert_eq!(MAIN.host, "TabLine");
        assert_eq!(MAIN.tabs, ["TabTree", "TabFiles", "TabAdvisor", "TabCleaners", "TabSearch", "TabAbout"]);
        assert!(!MAIN.carry);
    }

    #[test]
    fn motion_none_is_instant() {
        assert_eq!(timing(crate::tokens::Motion::None).0, 0);
        assert_eq!(timing(crate::tokens::Motion::Subtle).0, 160);
    }
}
