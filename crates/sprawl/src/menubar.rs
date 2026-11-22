//! The menu bar: File, Options and Help.
//!
//! WinUI's own MenuBar keeps each item's flyout to itself, so its menus always open with
//! WinUI's clip reveal and slide, under the mockup's entrance ([`crate::popups::watch`]). Here
//! each title is a button holding a plain `MenuFlyout`, whose own animation is off like every
//! other menu's ([`crate::popups::menus`]), so the mockup's `.menu` is all that plays.
//!
//! The buttons are drawn as WinUI draws a `MenuBarItem` (generic.xaml): the same brushes, margin,
//! padding and corner, and an inner button for the title exactly as WinUI's template has one.
//! What `MenuBar` did on its own is done here:
//!
//! * the pointer moving onto another title while a menu is open opens that one instead;
//! * Alt (pressed and let go alone) puts the keyboard on the bar and takes it back; Left and
//!   Right move along the bar, and switch menus while one is open; Down, Enter and Space open;
//!   Esc closes a menu (WinUI returns the focus to its title), and Esc on the bar gives the
//!   focus back to where it was; Alt+F, Alt+O and Alt+H open the menus (WinUI's access keys,
//!   with their key tips);
//! * a click on the open menu's title closes it (the click outside a menu is the menu's);
//! * each title is a menu item to UI Automation, with expand and collapse.

use std::cell::RefCell;

use windows_core::{Interface, HSTRING};

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, FlyoutBase};
use crate::bindings::Microsoft::UI::Xaml::Controls::{Control, DropDownButton, MenuFlyout, MenuFlyoutPresenter, MenuFlyoutSubItem};
use crate::bindings::Microsoft::UI::Xaml::Input::FocusManager;
use crate::bindings::Microsoft::UI::Xaml::Media::VisualTreeHelper;
use crate::bindings::Microsoft::UI::Xaml::{DependencyObject, FocusState, FrameworkElement, UIElement, VisualStateManager};
use crate::state::{find, keep};

/// The menus: the button's x:Name, its title, and its access key.
pub(crate) const MENUS: [(&str, &str, char); 3] = [("MenuFile", "File", 'F'), ("MenuOptions", "Options", 'O'), ("MenuHelp", "Help", 'H')];

/// A menu-bar title as WinUI's `MenuBarItem` draws it (generic.xaml, `controls:MenuBarItem`):
/// its brushes, margin, corner and focus rectangle, and the title in a button of its own with
/// the item's padding and transparent fills, as WinUI's template has it. The inner button takes
/// no input; the outer one is the control. Its states are not `CommonStates`: [`tick`] sets
/// them as `MenuBarItem` does (open over pressed over hovered), so the button's own pointer
/// states never put a hover over the open menu's title.
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

/// The bar's XAML, for the page's second row: a title button for each menu, holding the menu
/// (`items`, in [`MENUS`]'s order). As WinUI's MenuBar: 40 high, the titles 4 in from it, one
/// tab stop for the whole bar.
pub(crate) fn xaml(items: [&str; 3]) -> String {
    let buttons: String = MENUS
        .iter()
        .zip(items)
        .map(|(&(name, title, key), items)| {
            format!(
                r##"<DropDownButton x:Name="{name}" Content="{title}" AccessKey="{key}" Style="{{StaticResource SprawlBarItem}}" AutomationProperties.AutomationControlType="MenuItem"><DropDownButton.Flyout><MenuFlyout Placement="BottomEdgeAlignedLeft">{items}</MenuFlyout></DropDownButton.Flyout></DropDownButton>"##
            )
        })
        .collect();
    format!(
        r##"<StackPanel x:Name="MenuBar" Grid.Row="1" Margin="6,0,0,0" Orientation="Horizontal" MinHeight="40" Background="Transparent" TabFocusNavigation="Once" AutomationProperties.Name="Menu bar" AutomationProperties.AutomationControlType="MenuBar"><StackPanel.Resources>{STYLE}</StackPanel.Resources>{buttons}</StackPanel>"##
    )
}

// ------------------------------------------------------------------ what the keys and pointer do
//
// Plain logic, without WinUI, so it can be tested.

/// The keys the bar cares about.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Key {
    Left,
    Right,
    Up,
    Down,
    Escape,
    Letter(char),
    Other,
}

impl Key {
    /// From a WinUI `VirtualKey`.
    pub(crate) fn of(vk: i32) -> Key {
        match vk {
            37 => Key::Left,
            38 => Key::Up,
            39 => Key::Right,
            40 => Key::Down,
            27 => Key::Escape,
            65..=90 => Key::Letter(vk as u8 as char),
            _ => Key::Other,
        }
    }
}

/// What to do.
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
        Key::Other => Act::None,
    }
}

/// A key in open menu `at`, before the menu sees it. Only on its own items (`top`; a sub-menu's
/// keys are the sub-menu's), and Right on an item with a sub-menu (`sub`) opens the sub-menu.
pub(crate) fn in_menu(at: usize, n: usize, key: Key, top: bool, sub: bool) -> Act {
    match key {
        _ if !top => Act::None,
        Key::Left => Act::Switch(step(at, n, false)),
        Key::Right if !sub => Act::Switch(step(at, n, true)),
        _ => Act::None,
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

/// The title under (x, y), among the titles' rectangles (left, top, width, height).
pub(crate) fn hit(x: f64, y: f64, rects: &[(f64, f64, f64, f64)]) -> Option<usize> {
    rects.iter().position(|&(l, t, w, h)| x >= l && x < l + w && y >= t && y < t + h)
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

// ------------------------------------------------------------------------------- the live bar

struct Bar {
    root: UIElement,
    titles: Vec<DropDownButton>,
    menus: Vec<FlyoutBase>,
    open: Option<usize>,
    shown: Vec<(&'static str, &'static str)>,
    /// Where the pointer was on the last frame (window pixels).
    cursor: Option<(i32, i32)>,
    /// What had the focus before Alt put it on the bar.
    before: Option<UIElement>,
    alt: AltTap,
    /// The menus' presenters whose keys are already listened to.
    heard: Vec<usize>,
    /// When the page last saw a key: a menu opening just after one was opened by the keyboard.
    last_key: Option<std::time::Instant>,
    /// A menu opened by the keyboard whose first item is to take the focus (and for how many
    /// more frames to try, while the menu is laid out).
    first_item: Option<(usize, u32)>,
    /// The menu [`show`] has just opened where MenuBar puts it, until WinUI says it is open
    /// (WinUI says so a little later, not from inside the call).
    placing: Option<usize>,
    /// A menu the button opened, to open again where MenuBar puts it.
    again: Option<usize>,
    /// That menu, until WinUI has said it opened (and it is closed again).
    rejected: Option<usize>,
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

/// Wire the bar on the page made at start, and again on the one a theme change makes.
pub(crate) fn bind(root: &FrameworkElement) -> windows_core::Result<()> {
    let mut titles = Vec::new();
    let mut menus = Vec::new();
    for (i, &(name, _, _)) in MENUS.iter().enumerate() {
        let title: DropDownButton = find(root, name).inspect_err(|e| eprintln!("menubar: {name}: {e:?}"))?;
        let menu = title.cast::<crate::bindings::Microsoft::UI::Xaml::Controls::Button>()?.Flyout().inspect_err(|e| eprintln!("menubar: flyout: {e:?}"))?;
        keep(menu.Opening(move |_, _| opening(i))?);
        keep(menu.Opened(move |_, _| opened(i))?);
        keep(menu.Closed(move |_, _| {
            bar(|b| {
                if b.open == Some(i) {
                    b.open = None;
                }
            });
        })?);
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
                run(act);
            }
        })?);
        titles.push(title);
        menus.push(menu);
    }
    // Alt alone: onto the bar and off it again. Seen on its way down to the focused control
    // (tunnelling), before WinUI's access keys or any control can take it.
    let page: UIElement = root.cast()?;
    keep(page.PreviewKeyDown(|_, e| {
        if let Some(k) = e.as_ref().and_then(|e| e.Key().ok()) {
            dev(&format!("key down {}", k.0));
            bar(|b| {
                b.alt.key_down(is_alt(k.0));
                b.last_key = Some(std::time::Instant::now());
            });
        }
    })?);
    keep(page.PreviewKeyUp(|_, e| {
        if let Some(k) = e.as_ref().and_then(|e| e.Key().ok()) {
            bar(|b| b.last_key = Some(std::time::Instant::now()));
            alt_up(k.0);
        }
    })?);
    let n = titles.len();
    BAR.with(|b| {
        *b.borrow_mut() = Some(Bar { root: page, titles, menus, open: None, shown: vec![("", ""); n], cursor: None, before: None, alt: AltTap::default(), heard: Vec::new(), last_key: None, first_item: None, placing: None, again: None, rejected: None })
    });
    Ok(())
}

fn alt_up(vk: i32) {
    if bar(|b| b.alt.key_up(is_alt(vk))) != Some(true) {
        return;
    }
    dev("alt tap");
    // on the bar: off it again; elsewhere: onto its first title, remembering where from
    let Some((root, titles, before)) = bar(|b| (b.root.clone(), b.titles.clone(), b.before.clone())) else { return };
    let focused = focused(&root);
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

/// The element with the keyboard's focus.
fn focused(root: &UIElement) -> Option<UIElement> {
    let xr = root.XamlRoot().ok()?;
    FocusManager::GetFocusedElementWithRoot(&xr).ok()?.cast().ok()
}

/// Give the focus back to what had it before the bar.
fn leave(before: Option<UIElement>) {
    bar(|b| b.before = None);
    if let Some(e) = before {
        let _ = e.Focus(FocusState::Programmatic);
    }
}

fn run(act: Act) {
    let Some((titles, menus, open, before)) = bar(|b| (b.titles.clone(), b.menus.clone(), b.open, b.before.clone())) else { return };
    match act {
        Act::None => {}
        Act::Focus(j) => {
            let _ = titles[j].Focus(FocusState::Keyboard);
        }
        Act::Open(j) => {
            let _ = titles[j].Focus(FocusState::Keyboard);
            show(j, &menus[j], &titles[j]);
        }
        Act::Switch(j) => {
            if let Some(i) = open {
                if i == j {
                    return;
                }
                let _ = menus[i].Hide();
            }
            let _ = titles[j].Focus(FocusState::Keyboard);
            show(j, &menus[j], &titles[j]);
        }
        Act::Leave => leave(before),
    }
}

/// Switch to menu `j` from the pointer (no item takes the focus).
fn switch_by_pointer(j: usize) {
    let Some((titles, menus, open)) = bar(|b| (b.titles.clone(), b.menus.clone(), b.open)) else { return };
    if let Some(i) = open {
        if i == j {
            return;
        }
        let _ = menus[i].Hide();
    }
    let _ = titles[j].Focus(FocusState::Pointer);
    show(j, &menus[j], &titles[j]);
}

/// How far up the title's bottom edge MenuBar opens its menus.
const LIFT: f32 = 2.0;

/// Open `menu` under its `title` as WinUI's MenuBarItem does (MenuBarItem.cpp ShowMenuFlyout):
/// at the title's bottom left corner, kept off the title should it open upwards.
fn show(i: usize, menu: &FlyoutBase, title: &DropDownButton) {
    use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{FlyoutPlacementMode, FlyoutShowOptions};
    use crate::bindings::Windows::Foundation::{Point, Rect};
    let place = || -> windows_core::Result<()> {
        let fe: FrameworkElement = title.cast()?;
        let (w, h) = (fe.ActualWidth()? as f32, fe.ActualHeight()? as f32);
        let opts = FlyoutShowOptions::new()?;
        // WinUI's MenuBar opens its menus 2 px up the title's bottom edge (measured against it
        // pixel for pixel, 2026-11-22: otherwise the same menu, 2 px higher)
        opts.SetPosition(Some(Point { X: 0.0, Y: h - LIFT }))?;
        opts.SetPlacement(FlyoutPlacementMode::BottomEdgeAlignedLeft)?;
        opts.SetExclusionRect(Some(Rect { X: 0.0, Y: 0.0, Width: w, Height: h }))?;
        menu.ShowAtWithOptions(title, &opts)
    };
    // an open menu is only moved: WinUI says nothing of it
    if menu.IsOpen() == Ok(true) {
        let _ = place();
        return;
    }
    bar(|b| {
        b.placing = Some(i);
        b.rejected = None;
    });
    if place().is_err() {
        bar(|b| b.placing = None);
    }
}

/// Menu `i` is about to open. Opened by the button itself (a click, Enter, Space, its access
/// key, a screen reader's Expand), WinUI would put it a few pixels lower than MenuBar does, and
/// size it for that place: it is closed again before it is laid out or drawn, and opened where
/// MenuBar puts it on the next frame ([`tick`]). Opened by [`show`], it goes ahead.
fn opening(i: usize) {
    let Some(Some(menu)) = bar(|b| {
        // Alt+F's Alt, let go after the menu opened, is not Alt alone
        b.alt = AltTap::default();
        if b.placing == Some(i) {
            b.placing = None;
            return None;
        }
        b.again = Some(i);
        b.rejected = Some(i);
        Some(b.menus[i].clone())
    }) else {
        return;
    };
    dev(&format!("opening {i} by the button: again on the next frame"));
    let _ = menu.Hide();
}

/// Menu `i` has opened: it is the open one, and its keys are listened to (Left and Right move
/// to the next menu).
fn opened(i: usize) {
    if let Some(Some(menu)) = bar(|b| (b.rejected == Some(i)).then(|| {
        b.rejected = None;
        b.menus[i].clone()
    })) {
        dev(&format!("opened {i} by the button after all: closed"));
        let _ = menu.Hide();
        return;
    }
    let Some(menu) = bar(|b| {
        b.open = Some(i);
        // opened by a key (Down, Enter, Space, Alt+F, Left or Right): its first item takes the
        // focus, as in any menu opened from the keyboard
        if b.last_key.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(300)) {
            b.first_item = Some((i, 10));
        }
        b.menus[i].clone()
    }) else {
        return;
    };
    dev(&format!("opened {i}"));
    let Some(presenter) = presenter(&menu) else {
        dev("no presenter");
        return;
    };
    let id = presenter.as_raw() as usize;
    if bar(|b| b.heard.contains(&id)) != Some(false) {
        return;
    }
    // before the menu sees them: WinUI would move the focus out of the menu with Right (and
    // close it); a sub-menu's keys pass through here too, and are left to it
    let heard = presenter.PreviewKeyDown(move |_, e| {
        let Some(e) = e.as_ref() else { return };
        let Ok(k) = e.Key() else { return };
        let Some((at, root)) = bar(|b| b.open.map(|o| (o, b.root.clone()))).flatten() else { return };
        let Some(f) = focused(&root) else { return };
        let top = within(&f, id);
        let sub = f.cast::<MenuFlyoutSubItem>().is_ok();
        let act = in_menu(at, MENUS.len(), Key::of(k.0), top, sub);
        dev(&format!("menu key {} top {top} sub {sub}: {act:?}", k.0));
        if act != Act::None {
            let _ = e.SetHandled(true);
            run(act);
        }
    });
    if let Ok(h) = heard {
        keep(h);
        bar(|b| b.heard.push(id));
    }
}

/// Whether `el` lies inside the presenter whose identity is `presenter`.
fn within(el: &UIElement, presenter: usize) -> bool {
    let Ok(mut at) = el.cast::<DependencyObject>() else { return false };
    for _ in 0..64 {
        if at.cast::<MenuFlyoutPresenter>().is_ok_and(|p| p.as_raw() as usize == presenter) {
            return true;
        }
        match VisualTreeHelper::GetParent(&at) {
            Ok(p) => at = p,
            Err(_) => return false,
        }
    }
    false
}

/// The presenter a menu is drawn in while it is open: the parent, some way up, of its first
/// item.
fn presenter(menu: &FlyoutBase) -> Option<MenuFlyoutPresenter> {
    let items = menu.cast::<MenuFlyout>().ok()?.Items().ok()?;
    let mut at: DependencyObject = items.GetAt(0).ok()?.cast().ok()?;
    for _ in 0..32 {
        at = VisualTreeHelper::GetParent(&at).ok()?;
        if let Ok(p) = at.cast::<MenuFlyoutPresenter>() {
            return Some(p);
        }
    }
    None
}

/// Called once per frame: each title's look, and the pointer moving onto another title while a
/// menu is open.
pub(crate) fn tick() {
    let Some((root, titles, open, shown, first, again)) = bar(|b| (b.root.clone(), b.titles.clone(), b.open, b.shown.clone(), b.first_item, b.again.take())) else { return };
    if let Some(i) = again {
        if let Some(menu) = bar(|b| b.menus[i].clone()) {
            show(i, &menu, &titles[i]);
        }
    }
    // a menu opened by the keyboard: its first item that takes the focus
    if let Some((i, tries)) = first {
        let done = open != Some(i) || tries == 0 || focus_first(i);
        bar(|b| b.first_item = if done { None } else { Some((i, tries - 1)) });
    }
    // the titles' states
    for (i, t) in titles.iter().enumerate() {
        let Ok(button) = t.cast::<ButtonBase>() else { continue };
        let want = look(open == Some(i), button.IsPressed().unwrap_or(false), button.IsPointerOver().unwrap_or(false));
        if want != shown[i] {
            if let Ok(c) = t.cast::<Control>() {
                let _ = VisualStateManager::GoToState(&c, &HSTRING::from(want.0), true);
                let _ = VisualStateManager::GoToState(&c, &HSTRING::from(want.1), true);
            }
            bar(|b| b.shown[i] = want);
        }
    }
    // the pointer, in the window's own pixels, if it is over this window
    let Some(hwnd) = crate::state::with(|s| s.hwnd) else { return };
    let now = crate::win32::cursor_in(hwnd);
    let before = bar(|b| std::mem::replace(&mut b.cursor, now)).flatten();
    let (Some(i), Some(p)) = (open, now) else { return };
    if before == Some(p) {
        return;
    }
    let scale = root.XamlRoot().and_then(|x| x.RasterizationScale()).unwrap_or(1.0);
    let rects: Vec<(f64, f64, f64, f64)> = titles.iter().filter_map(|t| rect(t, &root)).collect();
    if rects.len() != titles.len() {
        return;
    }
    if let Some(j) = hit(f64::from(p.0) / scale, f64::from(p.1) / scale, &rects) {
        if j != i {
            switch_by_pointer(j);
        }
    }
}

/// The focus on menu `i`'s first item that can take it; whether one did.
fn focus_first(i: usize) -> bool {
    let Some(menu) = bar(|b| b.menus[i].clone()) else { return true };
    let Ok(items) = menu.cast::<MenuFlyout>().and_then(|m| m.Items()) else { return true };
    for k in 0..items.Size().unwrap_or(0) {
        if let Ok(item) = items.GetAt(k).and_then(|x| x.cast::<UIElement>()) {
            if item.Focus(FocusState::Keyboard).unwrap_or(false) {
                dev(&format!("menu {i}: item {k} has the focus"));
                return true;
            }
        }
    }
    false
}

/// dev: a line in the timing log (SPRAWL_TIMING), for the screenshot runs that drive the bar.
fn dev(what: &str) {
    crate::state::timing(&format!("menubar: {what}"), std::time::Instant::now());
}

/// dev: the pointer arriving on title `j` while a menu is open, as [`tick`] finds it (the
/// screenshot runs cannot move the real pointer).
pub(crate) fn dev_hover(j: usize) {
    if bar(|b| b.open.is_some()) == Some(true) && j < MENUS.len() {
        switch_by_pointer(j);
    }
}

/// dev: what has the focus and which menu is open, in the timing log.
pub(crate) fn dev_report() {
    let Some((root, open)) = bar(|b| (b.root.clone(), b.open)) else { return };
    let what = focused(&root).map_or("nothing".to_string(), |f| {
        let name = f.cast::<FrameworkElement>().and_then(|f| f.Name()).map(|n| n.to_string_lossy()).unwrap_or_default();
        let class = f.cast::<windows_core::IInspectable>().and_then(|i| i.GetRuntimeClassName()).map(|n| n.to_string_lossy()).unwrap_or_default();
        format!("{class} {name:?}")
    });
    dev(&format!("focus on {what}; open menu {open:?}"));
}

/// A title's rectangle on the page (left, top, width, height).
fn rect(t: &DropDownButton, root: &UIElement) -> Option<(f64, f64, f64, f64)> {
    let el: UIElement = t.cast().ok()?;
    let fe: FrameworkElement = t.cast().ok()?;
    let at = el.TransformToVisual(root).ok()?.TransformPoint(crate::bindings::Windows::Foundation::Point { X: 0.0, Y: 0.0 }).ok()?;
    Some((f64::from(at.X), f64::from(at.Y), fe.ActualWidth().ok()?, fe.ActualHeight().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn in_an_open_menu_left_and_right_switch_menus_but_leave_sub_menus_alone() {
        assert_eq!(in_menu(0, 3, Key::Right, true, false), Act::Switch(1));
        assert_eq!(in_menu(0, 3, Key::Left, true, false), Act::Switch(2));
        assert_eq!(in_menu(2, 3, Key::Right, true, false), Act::Switch(0));
        // Right on "Show >" opens the sub-menu instead; Left on it still moves on
        assert_eq!(in_menu(1, 3, Key::Right, true, true), Act::None);
        assert_eq!(in_menu(1, 3, Key::Left, true, true), Act::Switch(0));
        // inside the sub-menu, Left closes the sub-menu and Right does nothing of ours
        assert_eq!(in_menu(1, 3, Key::Left, false, false), Act::None);
        assert_eq!(in_menu(1, 3, Key::Right, false, true), Act::None);
        assert_eq!(in_menu(1, 3, Key::Down, true, false), Act::None);
        assert_eq!(in_menu(1, 3, Key::Escape, true, false), Act::None, "Esc is the menu's own");
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
    fn keys_map_from_winuis_virtual_keys() {
        assert_eq!(Key::of(37), Key::Left);
        assert_eq!(Key::of(39), Key::Right);
        assert_eq!(Key::of(40), Key::Down);
        assert_eq!(Key::of(27), Key::Escape);
        assert_eq!(Key::of(70), Key::Letter('F'));
        assert_eq!(Key::of(13), Key::Other, "Enter is the button's click");
    }

    #[test]
    fn the_pointer_finds_its_title() {
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

    #[test]
    fn the_bar_holds_three_titles_with_their_menus_and_access_keys() {
        let x = xaml(["<MenuFlyoutItem x:Name=\"a\"/>", "<MenuFlyoutItem x:Name=\"b\"/>", "<MenuFlyoutItem x:Name=\"c\"/>"]);
        for (name, title, key) in MENUS {
            assert!(x.contains(&format!(r#"<DropDownButton x:Name="{name}" Content="{title}" AccessKey="{key}""#)), "{x}");
        }
        assert_eq!(x.matches(r#"AutomationProperties.AutomationControlType="MenuItem""#).count(), 3);
        assert_eq!(x.matches("<MenuFlyout ").count(), 3);
        // WinUI's own entrance is turned off in each, as in every other menu
        let loaded = crate::popups::menus(&x);
        assert_eq!(loaded.matches(r#"<MenuFlyout AreOpenCloseAnimationsEnabled="False" Placement="BottomEdgeAlignedLeft">"#).count(), 3);
        // the states are the bar's own, never the button's CommonStates
        assert!(!x.contains(r#"x:Name="PointerOver""#) && !x.contains("CommonStates"));
        for state in ["FillNormal", "FillHover", "FillPressed", "FillOpen", "TextNormal", "TextHover", "TextPressed"] {
            assert!(x.contains(&format!(r#"<VisualState x:Name="{state}""#)), "{state}");
        }
        // drawn with MenuBarItem's own resources
        for key in ["MenuBarItemMargin", "MenuBarItemButtonPadding", "MenuBarItemBackgroundSelected", "MenuBarItemForeground"] {
            assert!(x.contains(key), "{key}");
        }
    }
}
