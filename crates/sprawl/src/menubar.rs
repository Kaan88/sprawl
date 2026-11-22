//! The menu bar: File, Options and Help.
//!
//! The menus themselves, their rows, keys, sub-menus and pointer are every menu's (menu.rs). What
//! WinUI's `MenuBar` did on its own is done here:
//!
//! * the pointer moving onto another title while a menu is open opens that one instead;
//! * Alt (pressed and let go alone) puts the keyboard on the bar and takes it back; Left and
//!   Right move along the bar, and switch menus while one is open; Down, Enter and Space open;
//!   Esc closes a menu and puts the focus back on its title, and Esc on the bar gives the focus
//!   back to where it was; Alt+F, Alt+O and Alt+H open the menus (WinUI's access keys);
//! * each item's shortcut is in the menu's own column, and works with the menu closed (the bar
//!   holds the keyboard accelerators);
//! * a check or a dot runs its command when its mark changes, however it changed;
//! * a click on the open menu's title closes it (the click outside a menu is the menu's);
//! * each title is a menu item to UI Automation, with expand and collapse.

use std::cell::RefCell;

use windows_core::{Interface, HSTRING};

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, ToggleButton};
use crate::bindings::Microsoft::UI::Xaml::Controls::Control;
use crate::bindings::Microsoft::UI::Xaml::Input::KeyboardAccelerator;
use crate::bindings::Microsoft::UI::Xaml::{FocusState, FrameworkElement, UIElement, VisualStateManager};
use crate::menu::{Key, Kind, MenuShape, Row};
use crate::state::{find, keep};

/// The menus: the button's x:Name, its title, and its access key.
pub(crate) const MENUS: [(&str, &str, char); 3] = [("MenuFile", "File", 'F'), ("MenuOptions", "Options", 'O'), ("MenuHelp", "Help", 'H')];

// ------------------------------------------------------------------------------- the markup

thread_local! {
    /// The bar's menus as the last [`xaml`] made them (its three first, then the sub-menus), for
    /// [`bind`] and [`set_checked`].
    static SHAPE: RefCell<Vec<MenuShape>> = const { RefCell::new(Vec::new()) };
}

/// A title as WinUI's `MenuBarItem` draws it (generic.xaml, `controls:MenuBarItem`): its brushes,
/// margin, corner and focus rectangle, and the title in a button of its own with the item's
/// padding and transparent fills, as WinUI's template has it. The inner button takes no input;
/// the outer one is the control. Its states are not `CommonStates`: [`tick`] sets them as
/// `MenuBarItem` does (open over pressed over hovered), so the button's own pointer states never
/// put a hover over the open menu's title.
const STYLE: &str = r##"<Style x:Key="SprawlBarItem" TargetType="DropDownButton">
<Setter Property="Foreground" Value="{ThemeResource MenuBarItemForeground}"/>
<Setter Property="Background" Value="{ThemeResource MenuBarItemBackground}"/>
<Setter Property="BorderThickness" Value="{ThemeResource MenuBarItemBorderThickness}"/>
<Setter Property="BorderBrush" Value="{ThemeResource MenuBarItemBorderBrush}"/>
<Setter Property="Margin" Value="{ThemeResource MenuBarItemMargin}"/>
<Setter Property="CornerRadius" Value="{ThemeResource ControlCornerRadius}"/>
<Setter Property="Padding" Value="0"/>
<Setter Property="VerticalAlignment" Value="Stretch"/>
<Setter Property="HorizontalAlignment" Value="Left"/>
<Setter Property="IsTabStop" Value="True"/>
<Setter Property="UseSystemFocusVisuals" Value="True"/>
<Setter Property="FocusVisualMargin" Value="-3"/>
<Setter Property="Template"><Setter.Value>
<ControlTemplate TargetType="DropDownButton">
<Grid x:Name="ContentRoot" Background="{TemplateBinding Background}" CornerRadius="{TemplateBinding CornerRadius}">
<Grid.Resources>
<SolidColorBrush x:Key="ButtonBackground" Color="Transparent"/>
<SolidColorBrush x:Key="ButtonBackgroundPointerOver" Color="Transparent"/>
<SolidColorBrush x:Key="ButtonBackgroundPressed" Color="Transparent"/>
<SolidColorBrush x:Key="ButtonBackgroundDisabled" Color="Transparent"/>
</Grid.Resources>
<VisualStateManager.VisualStateGroups>
<VisualStateGroup x:Name="BarFill">
<VisualState x:Name="FillNormal"/>
<VisualState x:Name="FillHover"><VisualState.Setters><Setter Target="Background.Background" Value="{ThemeResource MenuBarItemBackgroundPointerOver}"/><Setter Target="Background.BorderBrush" Value="{ThemeResource MenuBarItemBorderBrushPointerOver}"/></VisualState.Setters></VisualState>
<VisualState x:Name="FillPressed"><VisualState.Setters><Setter Target="Background.Background" Value="{ThemeResource MenuBarItemBackgroundPressed}"/><Setter Target="Background.BorderBrush" Value="{ThemeResource MenuBarItemBorderBrushPressed}"/></VisualState.Setters></VisualState>
<VisualState x:Name="FillOpen"><VisualState.Setters><Setter Target="Background.Background" Value="{ThemeResource MenuBarItemBackgroundSelected}"/><Setter Target="Background.BorderBrush" Value="{ThemeResource MenuBarItemBorderBrushSelected}"/></VisualState.Setters></VisualState>
</VisualStateGroup>
<VisualStateGroup x:Name="BarText">
<VisualState x:Name="TextNormal"/>
<VisualState x:Name="TextHover"><VisualState.Setters><Setter Target="ContentButton.Foreground" Value="{ThemeResource ButtonForegroundPointerOver}"/></VisualState.Setters></VisualState>
<VisualState x:Name="TextPressed"><VisualState.Setters><Setter Target="ContentButton.Foreground" Value="{ThemeResource ButtonForegroundPressed}"/></VisualState.Setters></VisualState>
</VisualStateGroup>
</VisualStateManager.VisualStateGroups>
<Border x:Name="Background" Background="{TemplateBinding Background}" CornerRadius="{TemplateBinding CornerRadius}" BorderThickness="{TemplateBinding BorderThickness}" BorderBrush="{TemplateBinding BorderBrush}"/>
<Button x:Name="ContentButton" Content="{TemplateBinding Content}" Foreground="{TemplateBinding Foreground}" Background="Transparent" BorderThickness="0" VerticalAlignment="Stretch" Padding="{StaticResource MenuBarItemButtonPadding}" IsTabStop="False" IsHitTestVisible="False" AutomationProperties.AccessibilityView="Raw"/>
</Grid>
</ControlTemplate>
</Setter.Value></Setter>
</Style>"##;

/// The bar's XAML, for the page's second row: a title button for each menu, holding its menu
/// (`menus`, in [`MENUS`]'s order), and every row's shortcut. As WinUI's MenuBar: 40 high, the
/// titles 4 in from it, one tab stop for the whole bar. The menus' and rows' styles are the
/// app's (menu.rs, in look.rs's resources).
pub(crate) fn xaml(menus: [Vec<Row>; 3]) -> String {
    let mut shapes: Vec<MenuShape> = MENUS.iter().map(|&(name, _, _)| MenuShape::new(name, "", &format!("{name}List"))).collect();
    let mut keys = String::new();
    let mut buttons = String::new();
    for (i, rows) in menus.iter().enumerate() {
        let (name, title, key) = MENUS[i];
        let inner = crate::menu::rows_xaml(rows, i, "Bar", &mut shapes, &mut keys);
        buttons += &format!(
            r##"<DropDownButton x:Name="{name}" Content="{title}" AccessKey="{key}" Style="{{StaticResource SprawlBarItem}}" AutomationProperties.AutomationControlType="MenuItem"><DropDownButton.Flyout>{}</DropDownButton.Flyout></DropDownButton>"##,
            crate::menu::flyout("", &format!("{name}List"), "BottomEdgeAlignedLeft", &inner)
        );
    }
    SHAPE.with(|s| *s.borrow_mut() = shapes.clone());
    crate::menu::pend("Bar", shapes);
    format!(
        r##"<StackPanel x:Name="MenuBar" Grid.Row="1" Margin="6,0,0,0" Orientation="Horizontal" MinHeight="40" Background="Transparent" TabFocusNavigation="Once" KeyboardAcceleratorPlacementMode="Hidden" AutomationProperties.Name="Menu bar" AutomationProperties.AutomationControlType="MenuBar"><StackPanel.Resources>{STYLE}</StackPanel.Resources><StackPanel.KeyboardAccelerators>{keys}</StackPanel.KeyboardAccelerators>{buttons}</StackPanel>"##
    )
}

// ------------------------------------------------------------------ what the keys and pointer do
//
// Plain logic, without WinUI, so it can be tested.

/// What to do on the bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Act {
    None,
    /// Put the keyboard's focus on this title.
    Focus(usize),
    /// Open this menu, from the keyboard (its first item takes the focus).
    Open(usize),
    /// Close the open menu and open this one.
    Switch(usize),
    /// Give the focus back to where it was before the bar took it.
    Leave,
}

/// The title before or after `at`, round the end.
pub(crate) fn step(at: usize, n: usize, forward: bool) -> usize {
    if forward { (at + 1) % n } else { (at + n - 1) % n }
}

/// A key on title `at` with no menu open (Enter and Space are the button's own: a click).
/// A letter is a menu's access key, as after Alt.
pub(crate) fn on_title(at: usize, n: usize, key: Key, keys: &[char]) -> Act {
    match key {
        Key::Left => Act::Focus(step(at, n, false)),
        Key::Right => Act::Focus(step(at, n, true)),
        Key::Down | Key::Up => Act::Open(at),
        Key::Escape => Act::Leave,
        Key::Letter(c) => keys.iter().position(|k| k.eq_ignore_ascii_case(&c)).map_or(Act::None, Act::Open),
        Key::Home | Key::End | Key::Other => Act::None,
    }
}

/// Alt pressed and let go with nothing else between: the classic way onto a menu bar.
#[derive(Default, Debug)]
pub(crate) struct AltTap {
    down: bool,
    clean: bool,
}

impl AltTap {
    pub(crate) fn key_down(&mut self, alt: bool) {
        if alt {
            // held, it repeats: only the first press starts a tap
            if !self.down {
                self.down = true;
                self.clean = true;
            }
        } else {
            self.clean = false;
        }
    }

    /// Whether this let-go ends a tap.
    pub(crate) fn key_up(&mut self, alt: bool) -> bool {
        if !alt {
            return false;
        }
        let tap = self.down && self.clean;
        *self = AltTap::default();
        tap
    }
}

/// Whether a virtual key is Alt (either one).
pub(crate) fn is_alt(vk: i32) -> bool {
    matches!(vk, 18 | 164 | 165)
}

/// A title's two states (fill, text), as `MenuBarItem` picks them: the open menu's title is
/// selected whatever the pointer does; the title's text follows the pointer, as WinUI's inner
/// button's does.
pub(crate) fn look(open: bool, pressed: bool, over: bool) -> (&'static str, &'static str) {
    let fill = if open {
        "FillOpen"
    } else if pressed {
        "FillPressed"
    } else if over {
        "FillHover"
    } else {
        "FillNormal"
    };
    let text = if pressed {
        "TextPressed"
    } else if over {
        "TextHover"
    } else {
        "TextNormal"
    };
    (fill, text)
}

/// The other rows of a group, when one is picked.
fn group_of(shapes: &[MenuShape], name: &str) -> Vec<String> {
    let Some(group) = shapes.iter().flat_map(|m| &m.rows).find(|r| r.name == name && r.kind == Kind::Radio).map(|r| r.group.clone()) else { return Vec::new() };
    shapes.iter().flat_map(|m| &m.rows).filter(|r| r.kind == Kind::Radio && r.group == group && r.name != name).map(|r| r.name.clone()).collect()
}

// ------------------------------------------------------------------------------- the live bar

struct Bar {
    /// Each title's states, as last set.
    shown: Vec<(&'static str, &'static str)>,
    /// What had the focus before Alt put it on the bar.
    before: Option<UIElement>,
    alt: AltTap,
    /// Whether the window had the keyboard on the last frame (losing it ends an Alt tap).
    foreground: bool,
}

thread_local! {
    static BAR: RefCell<Option<Bar>> = const { RefCell::new(None) };
}

/// `f` on the bar, unless it is in use (an event raised while it changes something).
fn bar<R>(f: impl FnOnce(&mut Bar) -> R) -> Option<R> {
    BAR.with(|b| match b.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

fn keys() -> Vec<char> {
    MENUS.iter().map(|m| m.2).collect()
}

/// Wire the bar on the page made at start, and again on the one a theme change makes. The menus
/// of the page start again here ([`crate::menu::reset`]).
pub(crate) fn bind(root: &FrameworkElement) -> windows_core::Result<()> {
    crate::menu::reset(&root.cast()?);
    crate::menu::bind(root, "Bar", crate::menu::How::Bar)?;
    let shapes = SHAPE.with(|s| s.borrow().clone());
    // a check or a dot runs its command when its mark changes, however it changed: a click,
    // Enter or Space, or a screen reader's Toggle or Select (which click nothing)
    for r in shapes.iter().flat_map(|s| &s.rows) {
        if matches!(r.kind, Kind::Toggle | Kind::Radio) {
            let t: ToggleButton = find(root, &r.name)?;
            let name = r.name.clone();
            keep(t.Checked(move |_, _| marked(&name))?);
            if r.kind == Kind::Toggle {
                let name = r.name.clone();
                keep(t.Unchecked(move |_, _| marked(&name))?);
            }
        }
    }
    // the titles' own keys: along the bar, and open
    for (i, &(name, _, _)) in MENUS.iter().enumerate() {
        let title: UIElement = find(root, name)?;
        keep(title.KeyDown(move |_, e| {
            let Some(e) = e.as_ref() else { return };
            if let Ok(true) = e.Handled() {
                return;
            }
            let Ok(k) = e.Key() else { return };
            // a letter with Ctrl is a shortcut, not a menu's key
            let (ctrl, _) = crate::win32::ctrl_shift_down();
            let key = Key::of(k.0);
            if ctrl && matches!(key, Key::Letter(_)) {
                return;
            }
            let act = on_title(i, MENUS.len(), key, &keys());
            if act != Act::None {
                let _ = e.SetHandled(true);
                crate::menu::set_narrow(false);
                run(act);
            }
        })?);
    }
    // the shortcuts, on the bar: they work with the menus closed, as a menu item's did
    for r in shapes.iter().flat_map(|s| &s.rows).filter(|r| !r.key.is_empty()) {
        let acc: KeyboardAccelerator = find(root, &format!("{}Acc", r.name))?;
        let name = r.name.clone();
        keep(acc.Invoked(move |_, e| {
            if let Some(e) = e.as_ref() {
                let _ = e.SetHandled(true);
            }
            activate(&name);
        })?);
    }
    // Alt alone: onto the bar and off it again. Seen on its way down to the focused control
    // (tunnelling), before WinUI's access keys or any control can take it. (Every key is noted
    // for the menus: one opening just after was opened by the keyboard.)
    let page: UIElement = root.cast()?;
    keep(page.PreviewKeyDown(|_, e| {
        if let Some(k) = e.as_ref().and_then(|e| e.Key().ok()) {
            dev(&format!("key down {}", k.0));
            bar(|b| b.alt.key_down(is_alt(k.0)));
            crate::menu::note_key();
        }
    })?);
    keep(page.PreviewKeyUp(|_, e| {
        if let Some(k) = e.as_ref().and_then(|e| e.Key().ok()) {
            crate::menu::note_key();
            alt_up(k.0);
        }
    })?);
    BAR.with(|b| *b.borrow_mut() = Some(Bar { shown: vec![("", ""); MENUS.len()], before: None, alt: AltTap::default(), foreground: false }));
    Ok(())
}

thread_local! {
    /// Marks being set by this code ([`set_checked`]), not by the user: no command runs.
    static SETTING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Whether row `name` runs its command on its click (main.rs binds that): not a check or a dot,
/// which run theirs when their mark changes.
pub(crate) fn runs_on_click(name: &str) -> bool {
    SHAPE.with(|s| s.borrow().iter().flat_map(|m| &m.rows).find(|r| r.name == name).is_none_or(|r| !matches!(r.kind, Kind::Toggle | Kind::Radio)))
}

/// The user changed row `name`'s mark (a dot: picked it; its others are let go by WinUI only
/// while on screen, so here too): the menus close and its command runs, once.
fn marked(name: &str) {
    if SETTING.with(|c| c.get()) > 0 {
        return;
    }
    crate::menu::close_all();
    if let Some(root) = crate::menu::root().and_then(|r| r.cast::<FrameworkElement>().ok()) {
        if SHAPE.with(|s| s.borrow().iter().flat_map(|m| &m.rows).any(|r| r.name == name && r.kind == Kind::Radio)) {
            let _ = set_checked(&root, name, true);
        }
    }
    dev(&format!("marked {name}"));
    crate::state::with(|s| s.command(name));
}

/// Whether the check or dot row `name` is ticked.
pub(crate) fn checked(root: &FrameworkElement, name: &str) -> windows_core::Result<bool> {
    find::<ToggleButton>(root, name)?.IsChecked()
}

/// Tick or untick the check or dot row `name`; a dot ticked unticks the rest of its group
/// (WinUI's radio buttons only do so while they are on screen, and a menu's mostly are not).
pub(crate) fn set_checked(root: &FrameworkElement, name: &str, on: bool) -> windows_core::Result<()> {
    SETTING.with(|c| c.set(c.get() + 1));
    let set = (|| {
        find::<ToggleButton>(root, name)?.SetIsChecked(Some(on))?;
        if on {
            for other in SHAPE.with(|s| group_of(&s.borrow(), name)) {
                if let Ok(t) = find::<ToggleButton>(root, &other) {
                    t.SetIsChecked(Some(false))?;
                }
            }
        }
        Ok(())
    })();
    SETTING.with(|c| c.set(c.get() - 1));
    set
}

/// A row's shortcut was pressed: a check flips (a dot is picked) and its command runs, as
/// a click on it would.
fn activate(name: &str) {
    let Some(kind) = SHAPE.with(|s| s.borrow().iter().flat_map(|m| &m.rows).find(|r| r.name == name).map(|r| r.kind)) else { return };
    let page = crate::menu::root().and_then(|r| r.cast::<FrameworkElement>().ok());
    if let Some(root) = &page {
        match kind {
            Kind::Toggle => {
                let on = checked(root, name).unwrap_or(false);
                let _ = set_checked(root, name, !on);
            }
            Kind::Radio => {
                let _ = set_checked(root, name, true);
            }
            Kind::Item | Kind::Sub => {}
        }
    }
    dev(&format!("shortcut {name}"));
    crate::state::with(|s| s.command(name));
}

/// A menu has begun to open: an Alt let go after it is not Alt alone.
pub(crate) fn alt_reset() {
    bar(|b| b.alt = AltTap::default());
}

fn alt_up(vk: i32) {
    if bar(|b| b.alt.key_up(is_alt(vk))) != Some(true) {
        return;
    }
    dev("alt tap");
    // on the bar: off it again; elsewhere: onto its first title, remembering where from
    let Some(root) = crate::menu::root() else { return };
    let titles = crate::menu::titles();
    let before = bar(|b| b.before.clone()).flatten();
    let focused = crate::menu::focused(&root);
    let on_bar = focused.as_ref().is_some_and(|f| titles.iter().any(|t| t.cast::<UIElement>().is_ok_and(|t| &t == f)));
    if on_bar {
        leave(before);
    } else {
        bar(|b| b.before = focused);
        if let Some(t) = titles.first() {
            let _ = t.Focus(FocusState::Keyboard);
        }
    }
}

/// Give the focus back to what had it before the bar.
fn leave(before: Option<UIElement>) {
    bar(|b| b.before = None);
    if let Some(e) = before {
        let _ = e.Focus(FocusState::Programmatic);
    }
}

/// The menus put the focus back where it was: the bar's Alt is done with.
pub(crate) fn refocused() {
    bar(|b| b.before = None);
}

pub(crate) fn run(act: Act) {
    // (Esc off the bar: back to what had the focus before Alt, or before the menu opened)
    let titles = crate::menu::titles();
    let top = crate::menu::open_title();
    let before = bar(|b| b.before.clone()).flatten().or_else(crate::menu::outside);
    let open = |j: usize| {
        if let Some(m) = crate::menu::title_menu(j) {
            crate::menu::show(m);
        }
    };
    match act {
        Act::None => {}
        Act::Focus(j) => {
            if let Some(t) = titles.get(j) {
                let _ = t.Focus(FocusState::Keyboard);
            }
        }
        Act::Open(j) => {
            if let Some(t) = titles.get(j) {
                let _ = t.Focus(FocusState::Keyboard);
            }
            open(j);
        }
        Act::Switch(j) => {
            if top == Some(j) {
                return;
            }
            crate::menu::close_all();
            if let Some(t) = titles.get(j) {
                let _ = t.Focus(FocusState::Keyboard);
            }
            open(j);
        }
        Act::Leave => leave(before),
    }
}

/// Switch to menu `j` from the pointer (no item takes the focus).
fn switch_by_pointer(j: usize) {
    if crate::menu::open_title() == Some(j) {
        return;
    }
    crate::menu::close_all();
    crate::menu::set_narrow(true);
    if let Some(t) = crate::menu::titles().get(j) {
        let _ = t.Focus(FocusState::Pointer);
    }
    if let Some(m) = crate::menu::title_menu(j) {
        crate::menu::show(m);
    }
}

/// The pointer moved to `p` while title `top`'s menu is open: onto another title, its menu
/// instead (whether it was).
pub(crate) fn hover(p: (f64, f64), top: usize) -> bool {
    let Some(root) = crate::menu::root() else { return false };
    let titles = crate::menu::titles();
    let rects: Vec<(f64, f64, f64, f64)> = titles.iter().filter_map(|t| crate::menu::rect(&t.cast().ok()?, &root)).collect();
    if rects.len() != titles.len() {
        return false;
    }
    match crate::menu::hit(p.0, p.1, &rects) {
        Some(j) if j != top => {
            switch_by_pointer(j);
            true
        }
        _ => false,
    }
}

/// Called once per frame (from menu.rs's): the Alt latch, and the titles' looks.
pub(crate) fn tick(hwnd: Option<crate::win32::HWND>) {
    // Alt let go after the window lost the keyboard (Alt+Tab away and back), or with a pointer
    // press between, is no tap: the latch starts again
    bar(|b| {
        let fg = hwnd.is_some_and(crate::win32::is_foreground);
        if (b.foreground && !fg) || crate::win32::mouse_button_down() {
            b.alt = AltTap::default();
        }
        b.foreground = fg;
    });
    looks();
}

/// The titles' states, as `MenuBarItem` sets them.
fn looks() {
    let titles = crate::menu::titles();
    let open = crate::menu::open_title();
    let Some(shown) = bar(|b| b.shown.clone()) else { return };
    for (i, t) in titles.iter().enumerate() {
        let Ok(button) = t.cast::<ButtonBase>() else { continue };
        let want = look(open == Some(i), button.IsPressed().unwrap_or(false), button.IsPointerOver().unwrap_or(false));
        if shown.get(i) != Some(&want) {
            let c: &Control = t;
            let _ = VisualStateManager::GoToState(c, &HSTRING::from(want.0), true);
            let _ = VisualStateManager::GoToState(c, &HSTRING::from(want.1), true);
            bar(|b| {
                if let Some(s) = b.shown.get_mut(i) {
                    *s = want;
                }
            });
        }
    }
}

/// dev: a line in the timing log (SPRAWL_TIMING), for the screenshot runs that drive the bar.
fn dev(what: &str) {
    crate::state::timing(&format!("menubar: {what}"), std::time::Instant::now());
}

/// dev: the pointer arriving on title `j` while a menu is open, as the menus' tick finds it (the
/// screenshot runs cannot move the real pointer).
pub(crate) fn dev_hover(j: usize) {
    if crate::menu::any_open() && j < MENUS.len() {
        switch_by_pointer(j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::Item;

    const KEYS: [char; 3] = ['F', 'O', 'H'];

    #[test]
    fn left_and_right_go_round_the_bar() {
        assert_eq!(on_title(0, 3, Key::Right, &KEYS), Act::Focus(1));
        assert_eq!(on_title(2, 3, Key::Right, &KEYS), Act::Focus(0));
        assert_eq!(on_title(0, 3, Key::Left, &KEYS), Act::Focus(2));
        assert_eq!(on_title(1, 3, Key::Left, &KEYS), Act::Focus(0));
    }

    #[test]
    fn down_opens_esc_leaves_and_a_letter_opens_its_menu() {
        assert_eq!(on_title(1, 3, Key::Down, &KEYS), Act::Open(1));
        assert_eq!(on_title(1, 3, Key::Up, &KEYS), Act::Open(1));
        assert_eq!(on_title(1, 3, Key::Escape, &KEYS), Act::Leave);
        assert_eq!(on_title(0, 3, Key::Letter('H'), &KEYS), Act::Open(2));
        assert_eq!(on_title(0, 3, Key::Letter('o'), &KEYS), Act::Open(1));
        assert_eq!(on_title(0, 3, Key::Letter('Z'), &KEYS), Act::None);
        assert_eq!(on_title(0, 3, Key::Other, &KEYS), Act::None);
        assert_eq!(on_title(0, 3, Key::Home, &KEYS), Act::None);
    }

    #[test]
    fn alt_alone_is_a_tap_and_alt_with_another_key_is_not() {
        let mut a = AltTap::default();
        a.key_down(true);
        a.key_down(true); // held: repeats
        assert!(a.key_up(true));
        // Alt+F: no tap when Alt is let go
        a.key_down(true);
        a.key_down(false);
        a.key_up(false);
        assert!(!a.key_up(true));
        // a key let go that was pressed before Alt does not spoil it
        a.key_down(true);
        assert!(!a.key_up(false));
        assert!(a.key_up(true));
        // a let-go without its press (Alt pressed in another window)
        assert!(!a.key_up(true));
        assert!(is_alt(18) && is_alt(164) && is_alt(165) && !is_alt(70));
    }

    #[test]
    fn the_pointer_finds_its_title() {
        use crate::menu::hit;
        let rects = [(10.0, 4.0, 40.0, 32.0), (58.0, 4.0, 70.0, 32.0), (136.0, 4.0, 50.0, 32.0)];
        assert_eq!(hit(12.0, 10.0, &rects), Some(0));
        assert_eq!(hit(60.0, 35.9, &rects), Some(1));
        assert_eq!(hit(54.0, 10.0, &rects), None, "between titles");
        assert_eq!(hit(140.0, 40.0, &rects), None, "below the bar");
        assert_eq!(hit(185.9, 4.0, &rects), Some(2));
    }

    #[test]
    fn a_title_looks_as_menubaritem_draws_it() {
        assert_eq!(look(false, false, false), ("FillNormal", "TextNormal"));
        assert_eq!(look(false, false, true), ("FillHover", "TextHover"));
        assert_eq!(look(false, true, true), ("FillPressed", "TextPressed"));
        // open: selected, whatever the pointer does
        assert_eq!(look(true, false, false), ("FillOpen", "TextNormal"));
        assert_eq!(look(true, false, true), ("FillOpen", "TextHover"));
    }

    fn menus() -> [Vec<Row>; 3] {
        let item = |n: &str, k: &str, m: &str| Row::Item(Item::new(n, n).keys(k, m));
        let toggle = |n: &str, on: bool| Row::Toggle { name: n.into(), text: n.into(), checked: on, key: String::new(), mods: String::new(), tip: String::new() };
        let radio = |n: &str, on: bool| Row::Radio { group: "g".into(), name: n.into(), text: n.into(), checked: on };
        [
            vec![item("a", "F5", ""), Row::Sep, item("b", "O", "Control")],
            vec![item("c", "", ""), Row::Sub { text: "Show".into(), rows: vec![toggle("t", true), Row::Sub { text: "Deeper".into(), rows: vec![radio("r1", true), radio("r2", false)] }] }],
            vec![toggle("u", false)],
        ]
    }

    #[test]
    fn the_bar_holds_three_titles_with_their_menus_access_keys_and_shortcuts() {
        let x = xaml(menus());
        for (name, title, key) in MENUS {
            assert!(x.contains(&format!(r#"<DropDownButton x:Name="{name}" Content="{title}" AccessKey="{key}""#)), "{x}");
            assert!(x.contains(&format!(r#"<StackPanel x:Name="{name}List""#)), "{name}");
        }
        assert_eq!(x.matches(r#"<Flyout "#).count(), 5, "the three menus and two sub-menus");
        assert!(!x.contains("<MenuFlyout"), "no WinUI menus: they open in a window of their own");
        // every flyout is drawn as a menu, and WinUI's own entrance is off in each
        assert_eq!(x.matches(r#"AreOpenCloseAnimationsEnabled="False" FlyoutPresenterStyle="{StaticResource SprawlMenu}""#).count(), 5);
        // every title and row is a menu item to UI Automation
        assert_eq!(x.matches(r#"AutomationProperties.AutomationControlType="MenuItem""#).count(), 3 + 9);
        // the shortcuts are the bar's, and shown in their rows
        assert!(x.contains(r#"<KeyboardAccelerator x:Name="aAcc" Key="F5"/>"#));
        assert!(x.contains(r#"<KeyboardAccelerator x:Name="bAcc" Key="O" Modifiers="Control"/>"#));
        assert!(x.contains(r#"Tag="Ctrl+O" AutomationProperties.AutomationControlType="MenuItem" AutomationProperties.AcceleratorKey="Ctrl+O""#));
        // the states are the bar's own, never the button's CommonStates
        for state in ["FillNormal", "FillHover", "FillPressed", "FillOpen", "TextNormal", "TextHover", "TextPressed"] {
            assert!(x.contains(&format!(r#"<VisualState x:Name="{state}""#)), "{state}");
        }
        // drawn with WinUI's own resources (the rows' in the app's: menu.rs)
        for key in ["MenuBarItemMargin", "MenuBarItemButtonPadding", "MenuBarItemBackgroundSelected"] {
            assert!(x.contains(key), "{key}");
        }
        let s = crate::menu::styles();
        for key in ["MenuFlyoutPresenterThemePadding", "MenuFlyoutItemThemePaddingNarrow", "MenuFlyoutItemPlaceholderThemeThickness", "MenuFlyoutSubItemBackgroundSubMenuOpened", "MenuFlyoutSeparatorThemePadding", "ToggleMenuFlyoutItemKeyboardAcceleratorTextForeground"] {
            assert!(s.contains(key), "{key}");
        }
    }

    #[test]
    fn the_menus_shape_knows_each_rows_menu_and_each_sub_menus_row() {
        let _ = xaml(menus());
        let shapes = SHAPE.with(|s| s.borrow().clone());
        assert_eq!(shapes.len(), 5);
        assert_eq!(shapes[1].rows.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![Kind::Item, Kind::Sub]);
        assert_eq!(shapes[1].rows[1].sub, Some(3));
        assert_eq!(shapes[1].rows[1].name, "BarSub3");
        assert_eq!(shapes[3].parent, Some((1, 1)));
        assert_eq!(shapes[3].rows[1].sub, Some(4));
        assert_eq!(shapes[4].parent, Some((3, 1)));
        assert_eq!(shapes[4].list, "BarSub4List");
        assert!(!shapes[0].checks && !shapes[1].checks && shapes[3].checks && shapes[4].checks && shapes[2].checks);
        assert_eq!(shapes[0].rows[0].key, "F5");
        assert_eq!(group_of(&shapes, "r1"), vec!["r2".to_string()]);
        assert!(group_of(&shapes, "t").is_empty());
    }

    #[test]
    fn the_rows_xaml_is_well_formed() {
        let x = xaml(menus());
        // every element opened is closed, in order (a cheap check of the generated markup)
        let mut stack: Vec<String> = Vec::new();
        let mut rest = x.as_str();
        while let Some(at) = rest.find('<') {
            rest = &rest[at + 1..];
            let end = rest.find('>').unwrap();
            let tag = &rest[..end];
            if let Some(name) = tag.strip_prefix('/') {
                assert_eq!(stack.pop().as_deref(), Some(name), "{tag}");
            } else if !tag.ends_with('/') {
                stack.push(tag.split([' ', '\n']).next().unwrap().to_string());
            }
            rest = &rest[end + 1..];
        }
        assert!(stack.is_empty(), "{stack:?}");
    }
}
