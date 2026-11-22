//! Changing the theme while Sprawl runs. The page's colours are values written into its controls
//! when it is loaded (tokens expanded, hand-written colours turned around for light), so a theme
//! change makes the page again: the same XAML, loaded for the new theme, its controls wired the
//! way start-up wires them ([`crate::bind`]), and what the old page showed carried over by name.
//! Everything the app keeps that was drawn for the old theme is reset first (see
//! [`State::restyle_now`]); the data (the scan, the tab, the selection, the map's camera) is not
//! touched.
//!
//! It runs from a frame, never from inside the event that asked (the Settings page's buttons
//! are part of the page that is being replaced). It waits while a question is open. With the
//! Settings page open it goes ahead (the Theme page's picks show at once on the whole window),
//! and the page is put back as it was: the same section, the same unsaved changes.

use std::cell::Cell;
use std::time::{Duration, Instant};

use windows_core::{Interface, HSTRING};

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{RangeBase, Selector};
use crate::bindings::Microsoft::UI::Xaml::Controls::{ColumnDefinition, Control, FontIcon, PasswordBox, RowDefinition, TextBlock, TextBox, ToggleMenuFlyoutItem, ToggleSwitch};
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement, Visibility};
use crate::scene::Mode;
use crate::state::{find, load, State, View};

thread_local! {
    static PENDING: Cell<bool> = const { Cell::new(false) };
    static POLLED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// A look change was asked for: the page is made again even though the light/dark theme is
    /// the same, so `restyle_now` cannot skip it.
    static LOOK_CHANGED: Cell<bool> = const { Cell::new(false) };
    /// The look in use before the look change waiting to be made, for the change to be undone if
    /// the page cannot be made.
    static LOOK_WAS: std::cell::RefCell<Option<crate::tokens::Look>> = const { std::cell::RefCell::new(None) };
    /// The Theme page's light or dark pick, tried before it is saved: used instead of the
    /// `theme` setting until Save or Cancel.
    static PREVIEW_MODE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// The Theme setting changed (or Windows' app mode did): the page is made again at the next
/// frame, if the theme really is another one.
pub(crate) fn request() {
    PENDING.with(|p| p.set(true));
}

/// The Theme page's own options changed: the look is set, and the page made again at the next
/// frame. The look is not the light/dark theme, so [`State::restyle_frame`] is told to rebuild
/// even when the theme itself stays the same.
pub(crate) fn apply_look(look: crate::tokens::Look) {
    LOOK_WAS.with(|old| *old.borrow_mut() = Some(crate::tokens::current()));
    crate::tokens::set(&look);
    LOOK_CHANGED.with(|l| l.set(true));
    request();
}

/// The Theme page's picks, tried on the whole window before they are saved: the look and the
/// light or dark choice (an index of [`crate::theme::CHOICES`]). Nothing is made again when both
/// are what the window shows or is about to.
pub(crate) fn preview(look: crate::tokens::Look, mode: usize) {
    let mode = mode.to_string();
    let same_mode = PREVIEW_MODE.with(|m| m.borrow().as_deref() == Some(mode.as_str()));
    PREVIEW_MODE.with(|m| *m.borrow_mut() = Some(mode));
    if look != crate::tokens::current() {
        apply_look(look);
    } else if !same_mode {
        request();
    }
}

/// Save or Cancel: the `theme` setting is the light or dark choice again.
pub(crate) fn end_preview() {
    if PREVIEW_MODE.with(|m| m.borrow_mut().take()).is_some() {
        request();
    }
}

/// Every `x:Name` the page's XAML gives a control.
fn names(xaml: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut rest = xaml;
    while let Some(at) = rest.find("x:Name=\"") {
        rest = &rest[at + 8..];
        let Some(end) = rest.find('"') else { break };
        let name = &rest[..end];
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
        rest = &rest[end..];
    }
    out
}

/// What the old page showed, put into the new one: texts, what is shown or hidden, what is
/// ticked or chosen, the sizes the code gave. Whatever the app draws from its own data is
/// drawn again by [`State::restyle_now`] instead. No handler is on the new page yet, so none of
/// this is heard as a change.
fn carry(old: &FrameworkElement, new: &FrameworkElement, names: &[&str]) {
    for &name in names {
        let key = HSTRING::from(name);
        let (Ok(o), Ok(n)) = (old.FindName(&key), new.FindName(&key)) else { continue };
        if let (Ok(a), Ok(b)) = (o.cast::<UIElement>(), n.cast::<UIElement>()) {
            if let Ok(v) = a.Visibility() {
                let _ = b.SetVisibility(v);
            }
            if let Ok(v) = a.Opacity() {
                let _ = b.SetOpacity(v);
            }
        }
        if let (Ok(a), Ok(b)) = (o.cast::<FrameworkElement>(), n.cast::<FrameworkElement>()) {
            if let Some(v) = a.Width().ok().filter(|v| v.is_finite()) {
                let _ = b.SetWidth(v);
            }
            if let Some(v) = a.Height().ok().filter(|v| v.is_finite()) {
                let _ = b.SetHeight(v);
            }
        }
        if let (Ok(a), Ok(b)) = (o.cast::<Control>(), n.cast::<Control>()) {
            if let Ok(v) = a.IsEnabled() {
                let _ = b.SetIsEnabled(v);
            }
        }
        if let (Ok(a), Ok(b)) = (o.cast::<TextBlock>(), n.cast::<TextBlock>()) {
            if let Ok(t) = a.Text() {
                let _ = b.SetText(&t);
            }
        }
        if let (Ok(a), Ok(b)) = (o.cast::<TextBox>(), n.cast::<TextBox>()) {
            if let Ok(t) = a.Text() {
                let _ = b.SetText(&t);
            }
        }
        // a key typed in Settings and not yet saved
        if let (Ok(a), Ok(b)) = (o.cast::<PasswordBox>(), n.cast::<PasswordBox>()) {
            if let Ok(t) = a.Password() {
                let _ = b.SetPassword(&t);
            }
        }
        if let (Ok(a), Ok(b)) = (o.cast::<ToggleMenuFlyoutItem>(), n.cast::<ToggleMenuFlyoutItem>()) {
            if let Ok(v) = a.IsChecked() {
                let _ = b.SetIsChecked(v);
            }
        }
        if let (Ok(a), Ok(b)) = (o.cast::<ToggleSwitch>(), n.cast::<ToggleSwitch>()) {
            if let Ok(v) = a.IsOn() {
                let _ = b.SetIsOn(v);
            }
        }
        if let (Ok(a), Ok(b)) = (o.cast::<Selector>(), n.cast::<Selector>()) {
            if let Ok(v) = a.SelectedIndex() {
                let _ = b.SetSelectedIndex(v);
            }
        }
        if let (Ok(a), Ok(b)) = (o.cast::<RangeBase>(), n.cast::<RangeBase>()) {
            if let Ok(v) = a.Maximum() {
                let _ = b.SetMaximum(v);
            }
            if let Ok(v) = a.Value() {
                let _ = b.SetValue(v);
            }
        }
        if let (Ok(a), Ok(b)) = (o.cast::<FontIcon>(), n.cast::<FontIcon>()) {
            if let Ok(g) = a.Glyph() {
                let _ = b.SetGlyph(&g);
            }
        }
        if let (Ok(a), Ok(b)) = (o.cast::<RowDefinition>(), n.cast::<RowDefinition>()) {
            if let Ok(v) = a.Height() {
                let _ = b.SetHeight(v);
            }
        }
        if let (Ok(a), Ok(b)) = (o.cast::<ColumnDefinition>(), n.cast::<ColumnDefinition>()) {
            if let Ok(v) = a.Width() {
                let _ = b.SetWidth(v);
            }
        }
    }
}

/// The minimise, maximise and close buttons in the theme's colours.
pub(crate) fn caption_buttons(window: &crate::bindings::Microsoft::UI::Xaml::Window) {
    use crate::bindings::Microsoft::UI::Windowing::TitleBarTheme;
    let theme = if crate::theme::light() { TitleBarTheme::Light } else { TitleBarTheme::Dark };
    let _ = window.AppWindow().and_then(|a| a.TitleBar()).and_then(|t| t.SetPreferredTheme(theme));
}

impl State {
    /// Called once per frame, first: a theme change asked for is made here. True when the page
    /// was made again (the rest of the frame waits for the next one).
    pub(crate) fn restyle_frame(&mut self) -> bool {
        // "Like Windows" follows Windows' app mode while Sprawl runs: looked at once a second
        if !PENDING.with(Cell::get) && self.shot.is_none() {
            let due = POLLED.with(|p| p.get().is_none_or(|t| t.elapsed() >= Duration::from_secs(1)));
            if due {
                POLLED.with(|p| p.set(Some(Instant::now())));
                let setting = self.theme_setting();
                if crate::theme::follows_windows(setting.as_deref()) && crate::theme::wanted_light(setting.as_deref()) != crate::theme::light() {
                    request();
                }
            }
        }
        if !PENDING.with(Cell::get) || self.dialogs.open() {
            return false;
        }
        PENDING.with(|p| p.set(false));
        match self.restyle_now() {
            Ok(changed) => changed,
            Err(e) => {
                eprintln!("theme change failed: {e:?}");
                crate::state::text(&self.root, "FootLeft", "Sprawl couldn't change the theme just now; it stays as it was. Try again, or restart Sprawl.");
                false
            }
        }
    }

    /// The light or dark choice in force: the Theme page's while it is being tried, else the
    /// saved setting.
    fn theme_setting(&self) -> Option<String> {
        PREVIEW_MODE.with(|m| m.borrow().clone()).or_else(|| self.settings.get("theme").map(str::to_string))
    }

    /// The Settings page (or the first-run wizard) is on screen.
    fn settings_shown(&self) -> bool {
        find::<UIElement>(&self.root, "Wizard").and_then(|w| w.Visibility()).is_ok_and(|v| v == Visibility::Visible)
    }

    /// Make the page again for the theme the setting asks for, and for the look in use. Ok(false):
    /// both are what the page shows. On an error before the new page is on screen, everything
    /// goes back to the old theme and look.
    fn restyle_now(&mut self) -> windows_core::Result<bool> {
        let setting = self.theme_setting();
        let was = crate::theme::light();
        // a look change makes the page again even when the light/dark theme is the same
        let look_changed = LOOK_CHANGED.with(Cell::take);
        let look_was = LOOK_WAS.with(|old| old.borrow_mut().take());
        if crate::theme::wanted_light(setting.as_deref()) == was && !look_changed {
            return Ok(false);
        }
        let t0 = Instant::now();
        // Settings open: what is typed or picked on its page goes into the draft first, so the
        // page made again shows it
        let settings_open = self.settings_shown();
        if settings_open {
            self.prefs_harvest();
        }
        let set_theme = |setting: Option<&str>| {
            crate::theme::init(setting);
            // the caches that hold a value written for the theme: the token table, the accent
            // (kept per theme), the brushes reconcile made, and the app's own styles
            crate::tokens::reset();
            crate::reconcile::forget_brushes();
            // the shadows on screen: the accent's glow and the shadows' strength, at once
            crate::shade::repaint();
        };
        set_theme(setting.as_deref());
        let made = (|| -> windows_core::Result<(FrameworkElement, windows_core::Result<crate::bindings::Microsoft::UI::Xaml::Controls::SwapChainPanel>)> {
            crate::app::refresh_resources()?;
            let drives = crate::win32::drives();
            let labels: Vec<String> = drives.iter().map(|d| format!("[{}] {}", d.root.trim_end_matches('\\'), d.label)).collect();
            let xaml = crate::ui::page(&labels);
            let root: FrameworkElement = load(&xaml)?;
            carry(&self.root, &root, &names(&xaml));
            // wired before it is shown, so a failure leaves the old page as it was
            let panel = crate::bind(&root);
            Ok((root, panel))
        })();
        let (root, panel) = match made {
            Ok((root, Ok(panel))) => (root, panel),
            Ok((_, Err(e))) | Err(e) => {
                // back to the theme and the look that were in use
                crate::theme::init(Some(if was { "1" } else { "2" }));
                if let Some(look) = look_was {
                    crate::tokens::set(&look);
                } else {
                    crate::tokens::reset();
                }
                crate::reconcile::forget_brushes();
                crate::shade::repaint();
                let _ = crate::app::refresh_resources();
                return Err(e);
            }
        };
        // the new page goes in
        self.window.SetContent(&root.cast::<UIElement>()?)?;
        self.window.SetTitleBar(&find::<UIElement>(&root, "TitleBar")?)?;
        caption_buttons(&self.window);
        // Mica, Acrylic or none, by the look now in use
        crate::look::apply_backdrop(&self.window);
        self.root = root;
        // the map: its panel is the new page's, so its swap chain is made again for it
        self.map.panel = panel;
        self.map.gpu = None;
        self.map.gpu_error = None;
        self.map.dims = (0.0, 0.0, 1.0, 1.0);
        self.map.scene.clear();
        self.map.dirty = true;
        self.map.drawn_once = false;
        // what was drawn for the old page, and is only drawn again when it changes
        self.tables.restyle();
        self.hud.restyle();
        self.search.ui_built = false;
        self.search.chips = None;
        self.search.saved_shape.clear();
        self.search.more.restyle();
        self.map.views.key_shown.clear();
        self.advisor.slot_adv.forget();
        self.advisor.slot_cl.forget();
        self.advisor.slot_foot.forget();
        crate::look::forget_shown();
        // the welcome page is built again at the next frame, if it is what shows
        self.welcome = None;
        // what the app draws from its data
        let mode = self.map.scene.mode;
        let view_on = self.map.views.kind.is_some();
        for (name, m) in [("ModeMap", Mode::Map), ("ModeCity", Mode::City), ("ModeGalaxy", Mode::Galaxy)] {
            crate::look::paint(&self.root, name, crate::look::Pick::Mode, m == mode && !view_on);
        }
        crate::look::paint(&self.root, "ViewMore", crate::look::Pick::Mode, view_on);
        self.set_view(self.view);
        self.refresh_exts();
        if self.view == View::Search {
            self.search_ui_build();
            self.search_box_state();
        }
        self.render_map();
        // the Settings page again, as it was: its section, its picks, its unsaved changes
        if settings_open {
            self.wizard_show();
        }
        crate::state::timing("theme change", t0);
        Ok(true)
    }
}
