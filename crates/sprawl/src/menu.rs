//! Every menu in Sprawl: the menu bar's (File, Options, Help and their sub-menus, menubar.rs),
//! the right-click menus (a row's, the treemap's, a search result's from another drive) and the
//! dropdown buttons' (More views, Colour, Search in, the saved searches, the search chips).
//!
//! WinUI's own `MenuFlyout` opens each menu in a window of its own, a few pixels bigger than the
//! menu, with WinUI's clip reveal and slide: the mockup's `.menu` entrance ([`crate::popups::watch`])
//! and its shadow (shade.rs) would be cut off square there. Here each menu is a plain `Flyout`,
//! which WinUI draws inside the app's window, and the menu in it is rows of our own, drawn as
//! WinUI draws a menu (generic.xaml's `MenuFlyoutPresenter`, `MenuFlyoutItem`,
//! `ToggleMenuFlyoutItem`, `RadioMenuFlyoutItem`, `MenuFlyoutSubItem` and `MenuFlyoutSeparator`:
//! the same brushes, paddings, margins, glyphs, icon column and states). A sub-menu is a flyout of
//! its own, opened from its row.
//!
//! What WinUI's menus did on their own is done here, for every menu alike:
//!
//! * Up, Down, Home and End move between a menu's items, round the ends, past what is off or
//!   hidden; Enter and Space run one; Right (or Enter) on an item with a sub-menu opens it with
//!   its first item focused, and Left or Esc in a sub-menu closes it; Esc closes a menu;
//! * the pointer resting on an item with a sub-menu opens it after Windows' menu delay, and
//!   resting on another item closes it again; an open sub-menu lets the pointer through to the
//!   menus it opened from;
//! * a menu opened from the keyboard has its first item focused, and its items padded as WinUI
//!   pads them for the keyboard; opened with the pointer, the narrower padding;
//! * an item that runs something closes the menus first, as WinUI's do;
//! * when the menus close, the focus goes back to what had it before they opened (the button a
//!   dropdown opened from, the tree a right-click menu opened over);
//! * each item is a menu item to UI Automation, with expand and collapse where it has a menu.
//!
//! The menus are kept by name in sets (a set is one menu and its sub-menus, or the bar's three):
//! a set made again (Search's menus are, when what they list changes) takes the old one's place.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use windows_core::{Interface, HSTRING};

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, FlyoutBase, ToggleButton};
use crate::bindings::Microsoft::UI::Xaml::Controls::{Button, Control};
use crate::bindings::Microsoft::UI::Xaml::Input::{FocusManager, KeyRoutedEventArgs};
use crate::bindings::Microsoft::UI::Xaml::Media::VisualTreeHelper;
use crate::bindings::Microsoft::UI::Xaml::{DependencyObject, FocusState, FrameworkElement, UIElement, Visibility, VisualStateManager};
use crate::bindings::Windows::Foundation::Point;
use crate::menubar::{step, Act};
use crate::state::{find, keep};

// ------------------------------------------------------------------------------- the markup

/// A plain item of a menu. Texts are XAML attribute text already.
#[derive(Clone, Debug, Default)]
pub(crate) struct Item {
    /// Its x:Name: the command it runs, as its menu's code binds it (none: a name is made up).
    pub name: String,
    pub text: String,
    /// Its shortcut (a `VirtualKey` name and its modifiers), shown in the menu's shortcut column.
    pub key: String,
    pub mods: String,
    /// A glyph of the symbol font ("&#xE8E5;"), in WinUI's icon column.
    pub icon: String,
    /// The icon's own colour (an attribute value), over the item's.
    pub icon_fg: String,
    pub tip: String,
    /// Off: greyed, and passed over by the keys.
    pub off: bool,
    /// Not shown until the code shows it.
    pub hidden: bool,
    /// Its text at this size (an attribute value), not the menu's.
    pub size: String,
}

impl Item {
    pub(crate) fn new(name: &str, text: &str) -> Item {
        Item { name: name.into(), text: text.into(), ..Item::default() }
    }

    pub(crate) fn keys(self, key: &str, mods: &str) -> Item {
        Item { key: key.into(), mods: mods.into(), ..self }
    }

    pub(crate) fn icon(self, glyph: &str) -> Item {
        Item { icon: glyph.into(), ..self }
    }

    pub(crate) fn icon_fg(self, colour: &str) -> Item {
        Item { icon_fg: colour.into(), ..self }
    }

    pub(crate) fn tip(self, tip: &str) -> Item {
        Item { tip: tip.into(), ..self }
    }

    pub(crate) fn off(self, off: bool) -> Item {
        Item { off, ..self }
    }

    pub(crate) fn hidden(self) -> Item {
        Item { hidden: true, ..self }
    }

    pub(crate) fn size(self, size: &str) -> Item {
        Item { size: size.into(), ..self }
    }
}

/// A row of a menu. Texts are XAML attribute text already.
#[derive(Clone, Debug)]
pub(crate) enum Row {
    /// Runs the command of its name.
    Item(Item),
    /// A check mark. In the bar's menus, flipped before its command runs (menubar.rs); in the
    /// others, its click is the command's.
    Toggle { name: String, text: String, checked: bool, key: String, mods: String, tip: String },
    /// One of its group is picked.
    Radio { group: String, name: String, text: String, checked: bool },
    /// Opens a menu of its own.
    Sub { text: String, rows: Vec<Row> },
    Sep,
}

/// What a row is, at run time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    Item,
    Toggle,
    Radio,
    Sub,
}

#[derive(Clone, Debug)]
pub(crate) struct RowShape {
    pub name: String,
    pub kind: Kind,
    pub group: String,
    /// Its shortcut as the menu shows it ("Ctrl+Shift+S"), or nothing.
    pub key: String,
    /// For a sub-menu's row: that menu.
    pub sub: Option<usize>,
}

/// A menu as [`rows_xaml`] made it.
#[derive(Clone, Debug)]
pub(crate) struct MenuShape {
    /// The button it opens from (a title, a sub-menu's row, a dropdown button), or none (a
    /// right-click menu).
    pub owner: String,
    /// The flyout's x:Name, or none: the owner's own `Flyout`.
    pub flyout: String,
    /// Its rows' panel.
    pub list: String,
    /// The menu and row it opens from, for a sub-menu.
    pub parent: Option<(usize, usize)>,
    /// Its rows, without the separators.
    pub rows: Vec<RowShape>,
    /// Whether any row has a check mark or a dot: the others' text then lines up with theirs, as
    /// WinUI's `CheckPlaceholder` state does.
    pub checks: bool,
    /// Whether any item has an icon: every item keeps the icon column, as WinUI's
    /// `IconPlaceholder` state does.
    pub icons: bool,
}

impl MenuShape {
    pub(crate) fn new(owner: &str, flyout: &str, list: &str) -> MenuShape {
        MenuShape { owner: owner.into(), flyout: flyout.into(), list: list.into(), parent: None, rows: Vec::new(), checks: false, icons: false }
    }
}

thread_local! {
    /// The menus the last [`xaml`] (or the bar's) made, by set, for [`bind`].
    static PENDING: RefCell<HashMap<String, Vec<MenuShape>>> = RefCell::new(HashMap::new());
}

/// Keep set `set`'s menus for [`bind`] (the bar's, made by menubar.rs).
pub(crate) fn pend(set: &str, shapes: Vec<MenuShape>) {
    PENDING.with(|p| p.borrow_mut().insert(set.to_string(), shapes));
}

/// A shortcut as a menu shows it: WinUI's names for the keys, the modifiers first.
pub(crate) fn key_text(key: &str, mods: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let mut parts: Vec<&str> = mods
        .split(',')
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(|m| match m {
            "Control" => "Ctrl",
            "Menu" => "Alt",
            "Windows" => "Win",
            m => m,
        })
        .collect();
    parts.push(match key {
        "Escape" => "Esc",
        k => k.strip_prefix("Number").unwrap_or(k),
    });
    parts.join("+")
}

/// The flyout a menu is shown in, drawn as WinUI draws a menu (`DefaultMenuFlyoutPresenterStyle`:
/// its colour, line, corner, padding and least size), but inside the window, so the mockup's
/// shadow (shade.rs) has room round it. `Tag` marks it for the mockup's opening
/// ([`crate::popups::watch`]).
const HOLDER: &str = r##"<Style x:Key="SprawlMenu" TargetType="FlyoutPresenter">
<Setter Property="Background" Value="{ThemeResource MenuFlyoutPresenterBackground}"/>
<Setter Property="BorderBrush" Value="{ThemeResource MenuFlyoutPresenterBorderBrush}"/>
<Setter Property="BorderThickness" Value="{ThemeResource MenuFlyoutPresenterBorderThemeThickness}"/>
<Setter Property="Padding" Value="{StaticResource MenuFlyoutPresenterThemePadding}"/>
<Setter Property="MinWidth" Value="{ThemeResource FlyoutThemeMinWidth}"/>
<Setter Property="MinHeight" Value="{StaticResource MenuFlyoutThemeMinHeight}"/>
<Setter Property="MaxWidth" Value="100000"/>
<Setter Property="MaxHeight" Value="100000"/>
<Setter Property="CornerRadius" Value="{ThemeResource OverlayCornerRadius}"/>
<Setter Property="IsTabStop" Value="False"/>
<Setter Property="Tag" Value="menu"/>
<Setter Property="Template"><Setter.Value>
<ControlTemplate TargetType="FlyoutPresenter">
<Border Background="{TemplateBinding Background}" BorderBrush="{TemplateBinding BorderBrush}" BorderThickness="{TemplateBinding BorderThickness}" CornerRadius="{TemplateBinding CornerRadius}" BackgroundSizing="InnerBorderEdge">
<ScrollViewer Margin="{TemplateBinding Padding}" HorizontalScrollMode="Disabled" HorizontalScrollBarVisibility="Disabled" VerticalScrollMode="Auto" VerticalScrollBarVisibility="Auto" IsHorizontalRailEnabled="False" IsVerticalRailEnabled="False" ZoomMode="Disabled" AutomationProperties.AccessibilityView="Raw">
<ContentPresenter Content="{TemplateBinding Content}" ContentTemplate="{TemplateBinding ContentTemplate}" ContentTransitions="{TemplateBinding ContentTransitions}"/>
</ScrollViewer>
</Border>
</ControlTemplate>
</Setter.Value></Setter>
</Style>"##;

/// What every row's style sets, as WinUI's menu items' styles do (and what the stock button's,
/// toggle's and radio button's own styles would set otherwise: their size, alignment, border
/// and focus margin).
fn row_setters(background: &str, foreground: &str) -> String {
    format!(
        r##"<Setter Property="Background" Value="{{ThemeResource {background}}}"/>
<Setter Property="Foreground" Value="{{ThemeResource {foreground}}}"/>
<Setter Property="BorderThickness" Value="0"/>
<Setter Property="Padding" Value="{{StaticResource MenuFlyoutItemThemePadding}}"/>
<Setter Property="FontSize" Value="{{ThemeResource ControlContentThemeFontSize}}"/>
<Setter Property="FontWeight" Value="Normal"/>
<Setter Property="HorizontalAlignment" Value="Stretch"/>
<Setter Property="VerticalAlignment" Value="Top"/>
<Setter Property="HorizontalContentAlignment" Value="Stretch"/>
<Setter Property="VerticalContentAlignment" Value="Center"/>
<Setter Property="MinWidth" Value="0"/>
<Setter Property="MinHeight" Value="0"/>
<Setter Property="IsTabStop" Value="True"/>
<Setter Property="UseSystemFocusVisuals" Value="True"/>
<Setter Property="FocusVisualMargin" Value="0"/>
<Setter Property="CornerRadius" Value="{{ThemeResource ControlCornerRadius}}"/>"##
    )
}

/// A visual state with setters (target, resource) of theme resources, and an `extra` setter text.
fn state(name: &str, setters: &[(&str, &str)], extra: &str) -> String {
    if setters.is_empty() && extra.is_empty() {
        return format!(r##"<VisualState x:Name="{name}"/>"##);
    }
    let s: String = setters.iter().map(|(t, r)| format!(r##"<Setter Target="{t}" Value="{{ThemeResource {r}}}"/>"##)).collect();
    format!(r##"<VisualState x:Name="{name}"><VisualState.Setters>{s}{extra}</VisualState.Setters></VisualState>"##)
}

/// WinUI's narrow padding, for a menu opened with the pointer.
const PADDING: &str = r##"<VisualStateGroup x:Name="PaddingSizeStates"><VisualState x:Name="DefaultPadding"/><VisualState x:Name="NarrowPadding"><VisualState.Setters><Setter Target="LayoutRoot.Padding" Value="{StaticResource MenuFlyoutItemThemePaddingNarrow}"/></VisualState.Setters></VisualState></VisualStateGroup>"##;

/// The shortcut's column, shown in a menu with a shortcut anywhere in it.
const KEY_SHOWN: &str = r##"<VisualStateGroup x:Name="KeyboardAcceleratorTextVisibility"><VisualState x:Name="KeyboardAcceleratorTextCollapsed"/><VisualState x:Name="KeyboardAcceleratorTextVisible"><VisualState.Setters><Setter Target="KeyboardAcceleratorTextBlock.Visibility" Value="Visible"/></VisualState.Setters></VisualState></VisualStateGroup>"##;

/// The room of a check mark and of an icon before the text, in a menu where some row has one
/// (WinUI's `CheckPlaceholderStates`); `icon`: the row has the icon column itself, shown then.
fn placeholder(icon: bool) -> String {
    let shown = |more: &str| if icon { format!(r##"<Setter Target="IconRoot.Visibility" Value="Visible"/>{more}"##) } else { String::new() };
    format!(
        r##"<VisualStateGroup x:Name="CheckPlaceholderStates">{}{}{}{}</VisualStateGroup>"##,
        state("NoPlaceholder", &[], ""),
        state("CheckPlaceholder", &[("TextBlock.Margin", "MenuFlyoutItemPlaceholderThemeThickness")], ""),
        state("IconPlaceholder", &[("TextBlock.Margin", "MenuFlyoutItemPlaceholderThemeThickness")], &shown("")),
        state("CheckAndIconPlaceholder", &[("TextBlock.Margin", "MenuFlyoutItemDoublePlaceholderThemeThickness")], &shown(r##"<Setter Target="IconRoot.Margin" Value="{ThemeResource MenuFlyoutItemPlaceholderThemeThickness}"/>"##)),
    )
}

/// The row's frame: WinUI's `LayoutRoot`, inset by the item margin, with the item's padding.
const ROOT: &str = r##"<Grid x:Name="LayoutRoot" Padding="{TemplateBinding Padding}" Background="{TemplateBinding Background}" Margin="{StaticResource MenuFlyoutItemMargin}" CornerRadius="{TemplateBinding CornerRadius}">"##;

/// An item's icon (its `CommandParameter`: a button has no `Icon`), in WinUI's 16 px box.
const ICON: &str = r##"<Viewbox x:Name="IconRoot" HorizontalAlignment="Left" VerticalAlignment="Center" Width="16" Height="16" Visibility="Collapsed"><ContentPresenter x:Name="IconContent" Content="{TemplateBinding CommandParameter}" Foreground="{TemplateBinding Foreground}" AutomationProperties.AccessibilityView="Raw"/></Viewbox>"##;

/// The row's text (the control's content).
fn text_part(column: u8) -> String {
    format!(r##"<ContentPresenter x:Name="TextBlock" Grid.Column="{column}" Content="{{TemplateBinding Content}}" Foreground="{{TemplateBinding Foreground}}" HorizontalAlignment="{{TemplateBinding HorizontalContentAlignment}}" VerticalAlignment="{{TemplateBinding VerticalContentAlignment}}" TextWrapping="NoWrap" AutomationProperties.AccessibilityView="Raw"/>"##)
}

/// The row's shortcut (its `Tag`), in the menu's shortcut column ([`dress`] lines them up).
fn key_part(column: u8, margin: &str, foreground: &str) -> String {
    format!(r##"<TextBlock x:Name="KeyboardAcceleratorTextBlock" Grid.Column="{column}" Style="{{ThemeResource CaptionTextBlockStyle}}" Text="{{Binding Tag, RelativeSource={{RelativeSource TemplatedParent}}}}" Margin="{margin}" Foreground="{{ThemeResource {foreground}}}" HorizontalAlignment="Right" VerticalAlignment="{{TemplateBinding VerticalContentAlignment}}" Visibility="Collapsed" AutomationProperties.AccessibilityView="Raw"/>"##)
}

/// The check mark or dot of a toggle or radio row.
fn check_part(glyph: &str) -> String {
    format!(r##"<FontIcon x:Name="CheckGlyph" FontFamily="{{ThemeResource SymbolThemeFontFamily}}" Glyph="{glyph}" FontSize="{{StaticResource SprawlMenuGlyph}}" Foreground="{{ThemeResource MenuFlyoutSubItemChevron}}" Opacity="0" Margin="0,0,16,0" AutomationProperties.AccessibilityView="Raw"/>"##)
}

/// The rows' styles: an item (WinUI's `MenuFlyoutItem`), a check (`ToggleMenuFlyoutItem`), a
/// dot (`RadioMenuFlyoutItem`) and a sub-menu's row (`MenuFlyoutSubItem`), and the separator.
fn row_styles() -> String {
    let template = |target: &str, groups: String, columns: &str, parts: String| {
        format!(r##"<Setter Property="Template"><Setter.Value><ControlTemplate TargetType="{target}">{ROOT}<VisualStateManager.VisualStateGroups>{groups}</VisualStateManager.VisualStateGroups><Grid.ColumnDefinitions>{columns}</Grid.ColumnDefinitions>{parts}</Grid></ControlTemplate></Setter.Value></Setter>"##)
    };
    let two = r##"<ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/>"##;
    let three = r##"<ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/>"##;

    // an item: generic.xaml's DefaultMenuFlyoutItemStyle
    let common = format!(
        r##"<VisualStateGroup x:Name="CommonStates">{}{}{}{}</VisualStateGroup>"##,
        state("Normal", &[], ""),
        state("PointerOver", &[("LayoutRoot.Background", "MenuFlyoutItemBackgroundPointerOver"), ("TextBlock.Foreground", "MenuFlyoutItemForegroundPointerOver"), ("IconContent.Foreground", "MenuFlyoutItemForegroundPointerOver"), ("KeyboardAcceleratorTextBlock.Foreground", "MenuFlyoutItemKeyboardAcceleratorTextForegroundPointerOver")], ""),
        state("Pressed", &[("LayoutRoot.Background", "MenuFlyoutItemBackgroundPressed"), ("TextBlock.Foreground", "MenuFlyoutItemForegroundPressed"), ("IconContent.Foreground", "MenuFlyoutItemForegroundPressed"), ("KeyboardAcceleratorTextBlock.Foreground", "MenuFlyoutItemKeyboardAcceleratorTextForegroundPressed")], ""),
        state("Disabled", &[("LayoutRoot.Background", "MenuFlyoutItemBackgroundDisabled"), ("TextBlock.Foreground", "MenuFlyoutItemForegroundDisabled"), ("IconContent.Foreground", "MenuFlyoutItemForegroundDisabled"), ("KeyboardAcceleratorTextBlock.Foreground", "MenuFlyoutItemKeyboardAcceleratorTextForegroundDisabled")], ""),
    );
    let item = format!(
        r##"<Style x:Key="SprawlRowItem" TargetType="Button">{}{}</Style>"##,
        row_setters("MenuFlyoutItemBackground", "MenuFlyoutItemForeground"),
        template("Button", format!("{common}{}{PADDING}{KEY_SHOWN}", placeholder(true)), two, ICON.to_string() + &text_part(0) + &key_part(1, "24,4,0,0", "MenuFlyoutItemKeyboardAcceleratorTextForeground")),
    );

    // a check: DefaultToggleMenuFlyoutItemStyle, its states folded into the toggle button's
    // (whose CommonStates hold Checked, CheckedPointerOver, ...)
    let over = [("LayoutRoot.Background", "MenuFlyoutSubItemBackgroundPointerOver"), ("TextBlock.Foreground", "MenuFlyoutItemForegroundPointerOver"), ("CheckGlyph.Foreground", "MenuFlyoutItemForegroundPointerOver"), ("KeyboardAcceleratorTextBlock.Foreground", "ToggleMenuFlyoutItemKeyboardAcceleratorTextForegroundPointerOver")];
    let pressed = [("LayoutRoot.Background", "MenuFlyoutSubItemBackgroundPressed"), ("TextBlock.Foreground", "MenuFlyoutSubItemForegroundPressed"), ("CheckGlyph.Foreground", "MenuFlyoutSubItemForegroundPressed"), ("KeyboardAcceleratorTextBlock.Foreground", "ToggleMenuFlyoutItemKeyboardAcceleratorTextForegroundPressed")];
    let disabled = [("TextBlock.Foreground", "MenuFlyoutSubItemForegroundDisabled"), ("CheckGlyph.Foreground", "MenuFlyoutSubItemForegroundDisabled"), ("KeyboardAcceleratorTextBlock.Foreground", "ToggleMenuFlyoutItemKeyboardAcceleratorTextForegroundDisabled")];
    let on = r##"<Setter Target="CheckGlyph.Opacity" Value="1"/>"##;
    let toggle_common = format!(
        r##"<VisualStateGroup x:Name="CommonStates">{}{}{}{}{}{}{}{}</VisualStateGroup>"##,
        state("Normal", &[], ""),
        state("PointerOver", &over, ""),
        state("Pressed", &pressed, ""),
        state("Disabled", &disabled, ""),
        state("Checked", &[], on),
        state("CheckedPointerOver", &over, on),
        state("CheckedPressed", &pressed, on),
        state("CheckedDisabled", &disabled, on),
    );
    let toggle = format!(
        r##"<Style x:Key="SprawlRowToggle" TargetType="ToggleButton">{}{}</Style>"##,
        row_setters("MenuFlyoutItemBackground", "MenuFlyoutItemForeground"),
        template("ToggleButton", format!("{toggle_common}{PADDING}{KEY_SHOWN}"), three, check_part("&#xE73E;") + &text_part(1) + &key_part(2, "24,0,0,0", "ToggleMenuFlyoutItemKeyboardAcceleratorTextForeground")),
    );

    // a dot: DefaultRadioMenuFlyoutItemStyle (a radio button keeps its check apart)
    let radio_common = format!(r##"<VisualStateGroup x:Name="CommonStates">{}{}{}{}</VisualStateGroup>"##, state("Normal", &[], ""), state("PointerOver", &over, ""), state("Pressed", &pressed, ""), state("Disabled", &disabled, ""));
    let radio_checks = format!(r##"<VisualStateGroup x:Name="CheckStates">{}{}{}</VisualStateGroup>"##, state("Checked", &[], on), state("Unchecked", &[], ""), state("Indeterminate", &[], ""));
    let radio = format!(
        r##"<Style x:Key="SprawlRowRadio" TargetType="RadioButton">{}{}</Style>"##,
        row_setters("MenuFlyoutItemBackground", "MenuFlyoutItemForeground"),
        template("RadioButton", format!("{radio_common}{radio_checks}{PADDING}"), three, check_part("&#xE915;") + &text_part(1)),
    );

    // a sub-menu's row: DefaultMenuFlyoutSubItemStyle. Its states are set by [`tick`], as WinUI's
    // sub-item sets them (open over pressed over hovered): the button's own CommonStates would
    // drop the open look whenever the pointer leaves the row for the sub-menu.
    let sub_states = format!(
        r##"<VisualStateGroup x:Name="SubStates">{}{}{}{}{}</VisualStateGroup>"##,
        state("SubNormal", &[], ""),
        state("SubPointerOver", &[("LayoutRoot.Background", "MenuFlyoutSubItemBackgroundPointerOver"), ("TextBlock.Foreground", "MenuFlyoutSubItemForegroundPointerOver"), ("SubItemChevron.Foreground", "MenuFlyoutSubItemChevronPointerOver")], ""),
        state("SubPressed", &[("LayoutRoot.Background", "MenuFlyoutSubItemBackgroundPressed"), ("TextBlock.Foreground", "MenuFlyoutSubItemForegroundPressed"), ("SubItemChevron.Foreground", "MenuFlyoutSubItemChevronPressed")], ""),
        state("SubMenuOpened", &[("LayoutRoot.Background", "MenuFlyoutSubItemBackgroundSubMenuOpened"), ("TextBlock.Foreground", "MenuFlyoutSubItemForegroundSubMenuOpened"), ("SubItemChevron.Foreground", "MenuFlyoutSubItemChevronSubMenuOpened")], ""),
        state("SubDisabled", &[("LayoutRoot.Background", "MenuFlyoutSubItemBackgroundDisabled"), ("TextBlock.Foreground", "MenuFlyoutSubItemForegroundDisabled"), ("SubItemChevron.Foreground", "MenuFlyoutSubItemChevronDisabled")], ""),
    );
    let chevron = r##"<FontIcon x:Name="SubItemChevron" Grid.Column="1" Glyph="&#xE974;" FontFamily="{ThemeResource SymbolThemeFontFamily}" FontSize="{StaticResource SprawlMenuGlyph}" Foreground="{ThemeResource MenuFlyoutSubItemChevron}" Margin="{StaticResource MenuFlyoutItemChevronMargin}" MirroredWhenRightToLeft="True" AutomationProperties.AccessibilityView="Raw"/>"##;
    let sub = format!(
        r##"<Style x:Key="SprawlRowSub" TargetType="DropDownButton">{}{}</Style>"##,
        row_setters("MenuFlyoutSubItemBackground", "MenuFlyoutSubItemForeground"),
        template("DropDownButton", format!("{sub_states}{}{PADDING}", placeholder(false)), two, text_part(0) + chevron),
    );
    let sep = r##"<Style x:Key="SprawlRowSep" TargetType="Rectangle"><Setter Property="Fill" Value="{ThemeResource MenuFlyoutSeparatorBackground}"/><Setter Property="Margin" Value="{StaticResource MenuFlyoutSeparatorThemePadding}"/><Setter Property="Height" Value="{StaticResource MenuFlyoutSeparatorHeight}"/></Style>"##;
    // the check, dot and chevron glyphs: 12 whatever the type scale, as in WinUI's templates
    let glyph = r##"<x:Double x:Key="SprawlMenuGlyph">12</x:Double>"##;
    format!("{glyph}{item}{toggle}{radio}{sub}{sep}")
}

/// The menus' styles, for the app's resources (look.rs): every menu on the page and every one
/// loaded later (Search's) finds them there.
pub(crate) fn styles() -> String {
    format!("{HOLDER}{}", row_styles())
}

/// A menu's flyout, holding its rows' panel; `name` its x:Name, if any.
pub(crate) fn flyout(name: &str, list: &str, placement: &str, rows: &str) -> String {
    let name = if name.is_empty() { String::new() } else { format!(r##" x:Name="{name}""##) };
    format!(
        r##"<Flyout{name} Placement="{placement}" ShouldConstrainToRootBounds="True" AreOpenCloseAnimationsEnabled="False" FlyoutPresenterStyle="{{StaticResource SprawlMenu}}"><StackPanel x:Name="{list}" XYFocusKeyboardNavigation="Disabled" TabFocusNavigation="Cycle">{rows}</StackPanel></Flyout>"##
    )
}

/// Menu `me`'s rows: the markup, with its sub-menus (named after `set`) added to `shapes` and its
/// shortcuts to `keys` (the bar's keyboard accelerators).
pub(crate) fn rows_xaml(rows: &[Row], me: usize, set: &str, shapes: &mut Vec<MenuShape>, keys: &mut String) -> String {
    shapes[me].checks = rows.iter().any(|r| matches!(r, Row::Toggle { .. } | Row::Radio { .. }));
    shapes[me].icons = rows.iter().any(|r| matches!(r, Row::Item(i) if !i.icon.is_empty()));
    let mut x = String::new();
    let accelerator = |keys: &mut String, name: &str, key: &str, mods: &str| {
        if key.is_empty() {
            return;
        }
        let mods = if mods.is_empty() { String::new() } else { format!(r##" Modifiers="{mods}""##) };
        *keys += &format!(r##"<KeyboardAccelerator x:Name="{name}Acc" Key="{key}"{mods}/>"##);
    };
    let ak = |shown: &str| if shown.is_empty() { String::new() } else { format!(r##" AutomationProperties.AcceleratorKey="{shown}""##) };
    let tip = |tip: &str| if tip.is_empty() { String::new() } else { format!(r##" ToolTipService.ToolTip="{tip}""##) };
    for (n, row) in rows.iter().enumerate() {
        match row {
            Row::Item(i) => {
                let shown = key_text(&i.key, &i.mods);
                let name = if i.name.is_empty() { format!("{set}R{me}_{n}") } else { i.name.clone() };
                let mut more = tip(&i.tip);
                if i.off {
                    more += r##" IsEnabled="False""##;
                }
                if i.hidden {
                    more += r##" Visibility="Collapsed""##;
                }
                if !i.size.is_empty() {
                    more += &format!(r##" FontSize="{}""##, i.size);
                }
                let open = format!(r##"<Button x:Name="{name}" Style="{{StaticResource SprawlRowItem}}" Content="{}" Tag="{shown}" AutomationProperties.AutomationControlType="MenuItem"{}{more}"##, i.text, ak(&shown));
                if i.icon.is_empty() {
                    x += &format!("{open}/>");
                } else {
                    let fg = if i.icon_fg.is_empty() { String::new() } else { format!(r##" Foreground="{}""##, i.icon_fg) };
                    x += &format!(r##"{open}><Button.CommandParameter><FontIcon Glyph="{}"{fg}/></Button.CommandParameter></Button>"##, i.icon);
                }
                accelerator(keys, &name, &i.key, &i.mods);
                shapes[me].rows.push(RowShape { name, kind: Kind::Item, group: String::new(), key: shown, sub: None });
            }
            Row::Toggle { name, text, checked, key, mods, tip: t } => {
                let shown = key_text(key, mods);
                x += &format!(r##"<ToggleButton x:Name="{name}" Style="{{StaticResource SprawlRowToggle}}" Content="{text}" IsChecked="{checked}" Tag="{shown}" AutomationProperties.AutomationControlType="MenuItem"{}{}/>"##, ak(&shown), tip(t));
                accelerator(keys, name, key, mods);
                shapes[me].rows.push(RowShape { name: name.clone(), kind: Kind::Toggle, group: String::new(), key: shown, sub: None });
            }
            Row::Radio { group, name, text, checked } => {
                x += &format!(r##"<RadioButton x:Name="{name}" Style="{{StaticResource SprawlRowRadio}}" GroupName="{group}" Content="{text}" IsChecked="{checked}" AutomationProperties.AutomationControlType="MenuItem"/>"##);
                shapes[me].rows.push(RowShape { name: name.clone(), kind: Kind::Radio, group: group.clone(), key: String::new(), sub: None });
            }
            Row::Sub { text, rows } => {
                let k = shapes.len();
                let owner = format!("{set}Sub{k}");
                let list = format!("{owner}List");
                let mut shape = MenuShape::new(&owner, "", &list);
                shape.parent = Some((me, shapes[me].rows.len()));
                shapes.push(shape);
                shapes[me].rows.push(RowShape { name: owner.clone(), kind: Kind::Sub, group: String::new(), key: String::new(), sub: Some(k) });
                let inner = rows_xaml(rows, k, set, shapes, keys);
                x += &format!(r##"<DropDownButton x:Name="{owner}" Style="{{StaticResource SprawlRowSub}}" Content="{text}" AutomationProperties.AutomationControlType="MenuItem"><DropDownButton.Flyout>{}</DropDownButton.Flyout></DropDownButton>"##, flyout("", &list, "BottomEdgeAlignedLeft", &inner));
            }
            Row::Sep => x += r##"<Rectangle Style="{StaticResource SprawlRowSep}"/>"##,
        }
    }
    x
}

/// A menu of its own (a right-click menu, a dropdown button's): its flyout's XAML, named `set`,
/// to put in `FlyoutBase.AttachedFlyout` or a button's `Flyout`; `owner` is the button it opens
/// from (none for a right-click menu). [`bind`] wires it once it is loaded.
pub(crate) fn xaml(set: &str, owner: &str, placement: &str, rows: &[Row]) -> String {
    let list = format!("{set}List");
    let mut shapes = vec![MenuShape::new(owner, set, &list)];
    let inner = rows_xaml(rows, 0, set, &mut shapes, &mut String::new());
    pend(set, shapes);
    flyout(set, &list, placement, &inner)
}

// ------------------------------------------------------------------ what the keys and pointer do
//
// Plain logic, without WinUI, so it can be tested.

/// The keys the menus care about.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Key {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
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
            36 => Key::Home,
            35 => Key::End,
            27 => Key::Escape,
            65..=90 => Key::Letter(vk as u8 as char),
            _ => Key::Other,
        }
    }
}

/// What to do in an open menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum InMenu {
    None,
    /// The focus onto this row.
    Move(usize),
    /// Open this row's sub-menu, its first item focused.
    OpenSub(usize),
    /// Close this menu (a sub-menu: the focus back on its row; one of the bar's or a button's:
    /// on its title or button).
    Close,
    /// Something for the bar (Left and Right in one of the bar's own menus).
    Bar(Act),
    /// The menu's key, with nothing to do: kept from the row (a dot's radio button would move
    /// on to the next dot and pick it, without its command).
    Hold,
}

/// Where an open menu is, for the Left and Right keys.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Place {
    /// One of the bar's own menus: its place on the bar and the bar's length.
    Bar(usize, usize),
    /// A sub-menu: Left goes back to its row.
    Sub,
    /// A right-click or a button's menu: Left has nowhere to go.
    Top,
}

/// A key in an open menu, before its rows see it: `at` is the row with the focus (none: the
/// focus is on none of its rows), `kinds` and `enabled` its rows (enabled: on and shown), and
/// `place` where the menu is. As WinUI's menus: Up and Down go round the ends, skipping what
/// can't take the focus; Enter and Space are the rows' own.
pub(crate) fn menu_key(key: Key, at: Option<usize>, kinds: &[Kind], enabled: &[bool], place: Place) -> InMenu {
    let n = kinds.len();
    let can = |i: usize| enabled.get(i).copied().unwrap_or(true);
    let walk = |from: Option<usize>, forward: bool| -> Option<usize> {
        (1..=n)
            .map(|s| match (from, forward) {
                (Some(f), true) => (f + s) % n,
                (Some(f), false) => (f + n - s % n) % n,
                (None, true) => s - 1,
                (None, false) => n - s,
            })
            .find(|&i| can(i))
    };
    let moved = |to: Option<usize>| to.map_or(InMenu::Hold, InMenu::Move);
    match key {
        Key::Down => moved(walk(at, true)),
        Key::Up => moved(walk(at, false)),
        Key::Home => moved(walk(None, true)),
        Key::End => moved(walk(None, false)),
        Key::Right => match at {
            Some(r) if kinds.get(r) == Some(&Kind::Sub) && can(r) => InMenu::OpenSub(r),
            _ => match place {
                Place::Bar(i, len) => InMenu::Bar(Act::Switch(step(i, len, true))),
                Place::Sub | Place::Top => InMenu::Hold,
            },
        },
        Key::Left => match place {
            Place::Bar(i, len) => InMenu::Bar(Act::Switch(step(i, len, false))),
            Place::Sub => InMenu::Close,
            // (WinUI's MenuFlyout keeps Left in a menu that is no sub-menu)
            Place::Top => InMenu::Hold,
        },
        Key::Escape => InMenu::Close,
        Key::Letter(_) | Key::Other => InMenu::None,
    }
}

/// The rectangle under (x, y), among rectangles (left, top, width, height).
pub(crate) fn hit(x: f64, y: f64, rects: &[(f64, f64, f64, f64)]) -> Option<usize> {
    rects.iter().position(|&(l, t, w, h)| x >= l && x < l + w && y >= t && y < t + h)
}

/// A sub-menu's row's state, as WinUI's `MenuFlyoutSubItem` picks it.
pub(crate) fn sub_look(enabled: bool, open: bool, pressed: bool, over: bool) -> &'static str {
    if !enabled {
        "SubDisabled"
    } else if open {
        "SubMenuOpened"
    } else if pressed {
        "SubPressed"
    } else if over {
        "SubPointerOver"
    } else {
        "SubNormal"
    }
}

/// An item's or sub-menu row's room before its text, as WinUI's menu picks it for the menu's
/// rows: room for a check where any row has one, for an icon where any item has one.
pub(crate) fn placeholder_state(checks: bool, icons: bool) -> &'static str {
    match (checks, icons) {
        (false, false) => "NoPlaceholder",
        (true, false) => "CheckPlaceholder",
        (false, true) => "IconPlaceholder",
        (true, true) => "CheckAndIconPlaceholder",
    }
}

/// The open menus under menu `m` (its sub-menus, theirs, ...), deepest first. `parents` gives
/// each menu's parent menu; `open` is in the order they opened.
pub(crate) fn under(m: usize, open: &[usize], parents: &[Option<usize>]) -> Vec<usize> {
    let below = |mut o: usize| {
        while let Some(p) = parents.get(o).copied().flatten() {
            if p == m {
                return true;
            }
            o = p;
        }
        false
    };
    open.iter().rev().copied().filter(|&o| below(o)).collect()
}

/// Rectangle `r` cut to `to` (left, top, width, height; nothing left: zero wide or high).
pub(crate) fn clip(r: (f64, f64, f64, f64), to: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let (l, t) = (r.0.max(to.0), r.1.max(to.1));
    let (rr, b) = ((r.0 + r.2).min(to.0 + to.2), (r.1 + r.3).min(to.1 + to.3));
    (l, t, (rr - l).max(0.0), (b - t).max(0.0))
}

// ------------------------------------------------------------------------------- the live menus

/// What a menu opens from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Opener {
    /// Title `i` of the menu bar.
    Bar(usize),
    /// A row of another menu.
    Sub,
    /// A button: a dropdown, a chip.
    Button,
    /// A right-click, or the menu key, at a point ([`show_at`]).
    Context,
}

/// How a set's first menus open (its sub-menus open from their rows).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum How {
    /// The menu bar's: each from its title, in [`crate::menubar::MENUS`]'s order.
    Bar,
    Button,
    Context,
}

/// A menu on the page.
struct Menu {
    set: String,
    opener: Opener,
    flyout: FlyoutBase,
    owner: Option<Control>,
    list: FrameworkElement,
    rows: Vec<Control>,
    /// The same rows, to tell which has the focus.
    els: Vec<UIElement>,
    /// Its shape, its parent and sub-menus as places in [`Menus::menus`].
    shape: MenuShape,
    /// For a sub-menu: its row's state, as last set.
    sub_shown: &'static str,
    /// For a sub-menu: the menu the pointer goes through to (the one it opened from, or one
    /// further up while the pointer is over that).
    through: Option<usize>,
}

struct Menus {
    root: UIElement,
    /// Every menu bound, by place; a set bound again leaves its old places empty (an event of
    /// the old flyouts arriving late finds nothing there).
    menus: Vec<Option<Menu>>,
    /// The open menus, in the order they opened.
    open: Vec<usize>,
    /// Whether the menus opened with the pointer (WinUI pads their items less).
    narrow: bool,
    hwnd: Option<crate::win32::HWND>,
    /// Where the pointer was on the last frame (window pixels).
    cursor: Option<(i32, i32)>,
    /// dev: where the pointer is said to be (page pixels), for the screenshot runs.
    fake: Option<(f64, f64)>,
    /// When the page last saw a key: a menu opening just after one was opened by the keyboard.
    last_key: Option<Instant>,
    /// A right-click menu [`show_at`] opened for the keyboard (the menu key, Shift+F10).
    keyed: Option<usize>,
    /// A menu opened by the keyboard whose first item is to take the focus (and for how many
    /// more frames to try, while the menu is laid out).
    first_item: Option<(usize, u32)>,
    /// The menu [`show`] has just opened where it belongs, until WinUI says it is open (WinUI
    /// says so a little later, not from inside the call).
    placing: Option<usize>,
    /// A menu its button opened, to open again where it belongs.
    again: Option<usize>,
    /// That menu, until WinUI has said it opened (and it is closed again).
    rejected: Option<usize>,
    /// The row the pointer is on (menu, row).
    on_row: Option<(usize, usize)>,
    /// The row the pointer came to rest on, and since when: a sub-menu opens or closes after
    /// Windows' menu delay.
    resting: Option<(usize, usize, Instant)>,
    delay: Duration,
    /// Menus closed by this code (not by a click outside them).
    closing: Vec<usize>,
    /// What had the focus outside the menus (and the bar's titles), and how it got it, while no
    /// menu was open, or the button a dropdown opened from: where the focus goes back when the
    /// menus close.
    outside: Option<(UIElement, FocusState)>,
    /// The menus have all closed: the focus goes back to [`Menus::outside`] in this many frames
    /// (a menu opening again in the meantime, switching menus, keeps it where it is).
    refocus: Option<u8>,
    /// Esc closed one of the bar's menus or a button's: the focus stays on its title or button.
    esc_to_owner: bool,
}

thread_local! {
    static MENUS: RefCell<Option<Menus>> = const { RefCell::new(None) };
}

/// `f` on the menus, unless they are in use (an event raised while they change something).
fn st<R>(f: impl FnOnce(&mut Menus) -> R) -> Option<R> {
    MENUS.with(|b| match b.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

impl Menus {
    fn get(&self, m: usize) -> Option<&Menu> {
        self.menus.get(m).and_then(Option::as_ref)
    }

    fn get_mut(&mut self, m: usize) -> Option<&mut Menu> {
        self.menus.get_mut(m).and_then(Option::as_mut)
    }

    /// Each menu's parent menu.
    fn parents(&self) -> Vec<Option<usize>> {
        self.menus.iter().map(|m| m.as_ref().and_then(|m| m.shape.parent.map(|p| p.0))).collect()
    }

    fn opener(&self, m: usize) -> Option<Opener> {
        self.get(m).map(|x| x.opener)
    }

    /// The menu and row `el` is, if it is one of the menus' rows.
    fn row_of(&self, el: &UIElement) -> Option<(usize, usize)> {
        self.menus.iter().enumerate().find_map(|(m, menu)| menu.as_ref()?.els.iter().position(|e| e == el).map(|r| (m, r)))
    }

    /// The bar's titles, in its order, with their menus.
    fn titles(&self) -> Vec<(usize, Control)> {
        let mut t: Vec<(usize, usize, Control)> = self
            .menus
            .iter()
            .enumerate()
            .filter_map(|(m, x)| match x.as_ref()?.opener {
                Opener::Bar(i) => Some((i, m, x.as_ref()?.owner.clone()?)),
                _ => None,
            })
            .collect();
        t.sort_by_key(|x| x.0);
        t.into_iter().map(|(_, m, c)| (m, c)).collect()
    }

    /// Whether `el` is one of the bar's titles or one of the menus' rows.
    fn on_menu(&self, el: &UIElement) -> bool {
        self.titles().iter().any(|(_, t)| t.cast::<UIElement>().is_ok_and(|t| &t == el)) || self.row_of(el).is_some()
    }

    /// The menus that opened first (not sub-menus), open now.
    fn tops(&self) -> Vec<usize> {
        self.open.iter().copied().filter(|&o| self.opener(o).is_some_and(|w| w != Opener::Sub)).collect()
    }

    fn key_recent(&self) -> bool {
        self.last_key.is_some_and(|t| t.elapsed() < Duration::from_millis(300))
    }
}

/// The page made at start, or again by a theme change: every menu of the old one is gone.
pub(crate) fn reset(root: &UIElement) {
    let delay = Duration::from_millis(u64::from(crate::win32::menu_show_delay()));
    MENUS.with(|b| {
        let mut b = b.borrow_mut();
        let places = b.as_ref().map_or(0, |m| m.menus.len());
        *b = Some(Menus {
            root: root.clone(),
            // (the old places stay taken: an event from the old page's flyouts finds nothing)
            menus: (0..places).map(|_| None).collect(),
            open: Vec::new(),
            narrow: true,
            hwnd: None,
            cursor: None,
            fake: None,
            last_key: None,
            keyed: None,
            first_item: None,
            placing: None,
            again: None,
            rejected: None,
            on_row: None,
            resting: None,
            delay,
            closing: Vec::new(),
            outside: None,
            refocus: None,
            esc_to_owner: false,
        });
    });
}

/// A check or a dot outside the bar changed its mark: the menus close, as on any row that runs
/// something. (These menus' marks are never set by the code: a menu is made again from what its
/// marks stand for instead, so a change is always the user's.)
fn marked_row(name: &str) {
    dev(&format!("row {name} marked"));
    close_all();
}

/// Run `run` with the new mark whenever check row `name` (in `scope`, a menu outside the bar)
/// changes it, however it changed: a click, Enter, Space, or a screen reader's Toggle, which
/// clicks nothing. `run` is told which way, and must do nothing when that is so already: then
/// the mark and what it stands for can't part, and nothing runs twice.
pub(crate) fn on_mark(scope: &FrameworkElement, name: &str, run: impl Fn(bool) + Clone + Send + 'static) -> windows_core::Result<()> {
    let t: ToggleButton = find(scope, name)?;
    let r = run.clone();
    keep(t.Checked(move |_, _| r(true))?);
    keep(t.Unchecked(move |_, _| run(false))?);
    Ok(())
}

/// Wire set `set`'s menus, made by [`xaml`] (or the bar's), found by name in `scope` once they are
/// loaded; a set bound before is closed and forgotten first. The menus' own keys, pointer and
/// focus are done here; what a row runs is bound by its menu's code afterwards (a row that runs
/// something closes the menus first, as WinUI's do).
pub(crate) fn bind(scope: &FrameworkElement, set: &str, how: How) -> windows_core::Result<()> {
    let Some(shapes) = PENDING.with(|p| p.borrow().get(set).cloned()) else {
        eprintln!("menu: {set}: not made");
        return Ok(());
    };
    // the old set: closed, and its places emptied
    let old = st(|b| {
        let gone: Vec<usize> = (0..b.menus.len()).filter(|&m| b.get(m).is_some_and(|x| x.set == set)).collect();
        let mut hide = Vec::new();
        for &m in &gone {
            if b.open.contains(&m) {
                hide.extend(b.get(m).map(|x| x.flyout.clone()));
            }
            b.menus[m] = None;
        }
        b.open.retain(|o| !gone.contains(o));
        b.closing.retain(|o| !gone.contains(o));
        for slot in [&mut b.keyed, &mut b.placing, &mut b.again, &mut b.rejected] {
            if slot.is_some_and(|m| gone.contains(&m)) {
                *slot = None;
            }
        }
        if b.first_item.is_some_and(|f| gone.contains(&f.0)) {
            b.first_item = None;
        }
        if b.on_row.is_some_and(|f| gone.contains(&f.0)) {
            b.on_row = None;
        }
        if b.resting.is_some_and(|f| gone.contains(&f.0)) {
            b.resting = None;
        }
        hide
    })
    .unwrap_or_default();
    for f in old {
        let _ = f.Hide();
    }
    let Some(base) = st(|b| b.menus.len()) else {
        eprintln!("menu: {set}: no page");
        return Ok(());
    };
    let mut made = Vec::new();
    for (i, shape) in shapes.iter().enumerate() {
        let m = base + i;
        let opener = match (shape.parent, how) {
            (Some(_), _) => Opener::Sub,
            (None, How::Bar) => Opener::Bar(i),
            (None, How::Button) => Opener::Button,
            (None, How::Context) => Opener::Context,
        };
        let owner: Option<Control> = if shape.owner.is_empty() { None } else { Some(find(scope, &shape.owner).inspect_err(|e| eprintln!("menu: {}: {e:?}", shape.owner))?) };
        let flyout: FlyoutBase = match (&owner, shape.flyout.is_empty()) {
            (Some(o), true) => o.cast::<Button>()?.Flyout().inspect_err(|e| eprintln!("menu: flyout of {}: {e:?}", shape.owner))?,
            _ => find(scope, &shape.flyout).inspect_err(|e| eprintln!("menu: {}: {e:?}", shape.flyout))?,
        };
        let list: FrameworkElement = find(scope, &shape.list)?;
        let mut rows = Vec::new();
        let mut els = Vec::new();
        for r in &shape.rows {
            let c: Control = find(scope, &r.name)?;
            els.push(c.cast::<UIElement>()?);
            rows.push(c);
        }
        keep(flyout.Opening(move |_, _| opening(m))?);
        keep(flyout.Opened(move |_, _| opened(m))?);
        keep(flyout.Closed(move |_, _| closed(m))?);
        // a sub-menu lets the pointer through to the menu it opened from, so its rows still
        // light up under the pointer and take a click (which closes the sub-menu)
        if let Some((p, _)) = shape.parent {
            let parent_list: FrameworkElement = find(scope, &shapes[p].list)?;
            flyout.SetOverlayInputPassThroughElement(&parent_list)?;
        }
        // each time it is shown: its rows padded for how it was opened, its shortcuts in a column
        keep(list.Loaded(move |_, _| dress(m))?);
        // its keys, before its rows see them
        keep(list.PreviewKeyDown(move |_, e| {
            if let Some(e) = e.as_ref() {
                list_key(m, e);
            }
        })?);
        // a row that runs something closes the menus first, as WinUI's do, before its
        // command (bound after this) runs. A check or a dot outside the bar does when its mark
        // changes, however it changed (a click, Enter, Space, or a screen reader's Toggle, which
        // clicks nothing), as its command does ([`on_mark`]); the bar's own close in
        // menubar.rs's `marked`
        for (r, c) in shape.rows.iter().zip(&rows) {
            if how != How::Bar && matches!(r.kind, Kind::Toggle | Kind::Radio) {
                let t: ToggleButton = c.cast()?;
                let name = r.name.clone();
                keep(t.Checked(move |_, _| marked_row(&name))?);
                let name = r.name.clone();
                keep(t.Unchecked(move |_, _| marked_row(&name))?);
            } else if r.kind != Kind::Sub {
                let name = r.name.clone();
                keep(c.cast::<ButtonBase>()?.Click(move |_, _| {
                    dev(&format!("row {name} clicked"));
                    close_all();
                })?);
            }
        }
        let mut shape = shape.clone();
        shape.parent = shape.parent.map(|(p, r)| (base + p, r));
        for r in &mut shape.rows {
            r.sub = r.sub.map(|k| base + k);
        }
        let through = shape.parent.map(|p| p.0);
        made.push(Menu { set: set.to_string(), opener, flyout, owner, list, rows, els, shape, sub_shown: "", through });
    }
    st(|b| {
        for (i, menu) in made.into_iter().enumerate() {
            let m = base + i;
            if b.menus.len() <= m {
                b.menus.resize_with(m + 1, || None);
            }
            b.menus[m] = Some(menu);
        }
    });
    dev(&format!("{set} bound at {base}"));
    Ok(())
}

/// The bar's titles, in its order.
pub(crate) fn titles() -> Vec<Control> {
    st(|b| b.titles().into_iter().map(|t| t.1).collect()).unwrap_or_default()
}

/// The menu of title `i`.
pub(crate) fn title_menu(i: usize) -> Option<usize> {
    st(|b| b.titles().get(i).map(|t| t.0)).flatten()
}

/// The title whose menu is open, if one is.
pub(crate) fn open_title() -> Option<usize> {
    st(|b| b.open.first().and_then(|&m| match b.opener(m) {
        Some(Opener::Bar(i)) => Some(i),
        _ => None,
    }))
    .flatten()
}

/// Whether a menu is open.
pub(crate) fn any_open() -> bool {
    st(|b| !b.open.is_empty()).unwrap_or(false)
}

/// The next menus open with the pointer's padding (narrow) or the keyboard's.
pub(crate) fn set_narrow(narrow: bool) {
    st(|b| b.narrow = narrow);
}

/// A key went down or up on the page.
pub(crate) fn note_key() {
    st(|b| b.last_key = Some(Instant::now()));
}

pub(crate) fn root() -> Option<UIElement> {
    st(|b| b.root.clone())
}

/// What had the focus outside the menus.
pub(crate) fn outside() -> Option<UIElement> {
    st(|b| b.outside.as_ref().map(|o| o.0.clone())).flatten()
}

/// The element with the keyboard's focus.
pub(crate) fn focused(root: &UIElement) -> Option<UIElement> {
    let xr = root.XamlRoot().ok()?;
    FocusManager::GetFocusedElementWithRoot(&xr).ok()?.cast().ok()
}

/// Close menu `m` and the sub-menus open under it, as this code's own doing.
fn close_from(m: usize) {
    let Some(flyouts) = st(|b| {
        let mut all = under(m, &b.open, &b.parents());
        all.push(m);
        b.closing.extend(all.iter().copied());
        all.iter().filter_map(|&k| b.get(k).map(|x| x.flyout.clone())).collect::<Vec<_>>()
    }) else {
        return;
    };
    for f in flyouts {
        let _ = f.Hide();
    }
}

/// Close every open menu.
pub(crate) fn close_all() {
    for top in st(|b| b.tops()).unwrap_or_default() {
        close_from(top);
    }
}

/// How far up the title's bottom edge MenuBar opens its menus.
const LIFT: f32 = 2.0;

/// Open menu `m` where it belongs: one of the bar's at its title's bottom left corner, kept off
/// the title should it open upwards (WinUI's MenuBarItem.cpp ShowMenuFlyout); a sub-menu at its
/// row's right edge, level with the row, beside it (MenuFlyoutSubItem); a button's where its
/// flyout's placement puts it. The other sub-menus of the menu it opens from close first.
pub(crate) fn show(m: usize) {
    use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{FlyoutPlacementMode, FlyoutShowOptions};
    use crate::bindings::Windows::Foundation::Rect;
    let Some(Some((flyout, owner, opener, parent))) = st(|b| b.get(m).and_then(|x| Some((x.flyout.clone(), x.owner.clone()?, x.opener, x.shape.parent)))) else { return };
    if opener == Opener::Context {
        return;
    }
    let place = || -> windows_core::Result<()> {
        if opener == Opener::Button {
            return flyout.ShowAt(&owner);
        }
        let top = matches!(opener, Opener::Bar(_));
        let fe: FrameworkElement = owner.cast()?;
        let (w, h) = (fe.ActualWidth()? as f32, fe.ActualHeight()? as f32);
        let opts = FlyoutShowOptions::new()?;
        // WinUI's MenuBar opens its menus 2 px up the title's bottom edge (measured against it
        // pixel for pixel, 2026-11-22)
        let at = if top { Point { X: 0.0, Y: h - LIFT } } else { Point { X: w, Y: 0.0 } };
        opts.SetPosition(Some(at))?;
        opts.SetPlacement(FlyoutPlacementMode::BottomEdgeAlignedLeft)?;
        // (a sub-menu has none: kept off its row, WinUI would put it under the row instead)
        if top {
            opts.SetExclusionRect(Some(Rect { X: 0.0, Y: 0.0, Width: w, Height: h }))?;
        }
        flyout.ShowAtWithOptions(&owner, &opts)
    };
    // an open menu is only moved: WinUI says nothing of it
    if flyout.IsOpen() == Ok(true) {
        let _ = place();
        return;
    }
    // the sub-menus open from the same menu close first
    let others = st(|b| match parent {
        Some((p, _)) => b.open.iter().copied().filter(|&o| b.get(o).and_then(|x| x.shape.parent).is_some_and(|q| q.0 == p)).collect(),
        None => Vec::new(),
    })
    .unwrap_or_default();
    for o in others {
        close_from(o);
    }
    st(|b| {
        b.placing = Some(m);
        b.rejected = None;
    });
    if place().is_err() {
        st(|b| b.placing = None);
    }
}

/// Open the right-click menu whose flyout is named `flyout` at `at` on `target`: its top left
/// corner there (moved in to stay in the window, or opening upwards near its bottom, as WinUI's
/// menus do). `by_key`: for the menu key or Shift+F10, its first item focused and its items
/// padded for the keyboard.
pub(crate) fn show_at(flyout: &str, target: &UIElement, at: Point, by_key: bool) {
    use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{FlyoutPlacementMode, FlyoutShowOptions};
    let Some(Some((m, f))) = st(|b| {
        let m = (0..b.menus.len()).find(|&m| b.get(m).is_some_and(|x| x.shape.flyout == flyout && x.opener == Opener::Context))?;
        let f = b.get(m)?.flyout.clone();
        b.narrow = !by_key;
        b.keyed = by_key.then_some(m);
        // where the focus goes back to when it closes
        if let Some(now) = focused(&b.root).filter(|e| !b.on_menu(e)) {
            let state = now.FocusState().unwrap_or(FocusState::Programmatic);
            b.outside = Some((now, if state == FocusState::Unfocused { FocusState::Programmatic } else { state }));
        }
        if f.IsOpen() != Ok(true) {
            b.placing = Some(m);
        }
        Some((m, f))
    }) else {
        dev(&format!("no menu {flyout}"));
        return;
    };
    let shown = (|| -> windows_core::Result<()> {
        let opts = FlyoutShowOptions::new()?;
        opts.SetPosition(Some(at))?;
        opts.SetPlacement(FlyoutPlacementMode::BottomEdgeAlignedLeft)?;
        f.ShowAtWithOptions(target, &opts)
    })();
    dev(&format!("{flyout} at {:.0},{:.0}{}", at.X, at.Y, if by_key { " for the keyboard" } else { "" }));
    if let Err(e) = shown {
        eprintln!("menu {flyout}: {e:?}");
        st(|b| {
            if b.placing == Some(m) {
                b.placing = None;
            }
        });
    }
}

/// Menu `m` is about to open. Opened by its title or row itself (a click, Enter, Space, an
/// access key, a screen reader's Expand), WinUI would put one of the bar's menus or a sub-menu
/// where a plain flyout goes, and size it for that place: it is closed again before it is laid
/// out or drawn, and opened where it belongs on the next frame ([`tick`]). Opened by [`show`],
/// or a button's or right-click menu, it goes ahead.
fn opening(m: usize) {
    // Alt+F's Alt, let go after the menu opened, is not Alt alone
    crate::menubar::alt_reset();
    let Some(Some(reject)) = st(|b| {
        let menu = b.get(m)?;
        let (opener, owner, flyout) = (menu.opener, menu.owner.clone(), menu.flyout.clone());
        let key = b.key_recent();
        let by_pointer = owner.as_ref().is_some_and(|o| o.FocusState().is_ok_and(|f| f == FocusState::Pointer));
        // a button's menu: the focus goes back to the button when the menus close
        if opener == Opener::Button && b.open.is_empty() {
            if let Some(o) = owner.as_ref().and_then(|o| o.cast::<UIElement>().ok()) {
                let state = o.FocusState().unwrap_or(FocusState::Programmatic);
                b.outside = Some((o, if state == FocusState::Unfocused { FocusState::Programmatic } else { state }));
            }
        }
        if b.placing == Some(m) {
            b.placing = None;
            return Some(None);
        }
        match opener {
            Opener::Bar(_) | Opener::Sub => {
                b.again = Some(m);
                b.rejected = Some(m);
                // one of the bar's, opened by its title: clicked, its items are padded for the
                // pointer (an access key, Alt+F, leaves the focus as the pointer left it: the key
                // decides)
                if opener != Opener::Sub {
                    b.narrow = !key && by_pointer;
                }
                Some(Some(flyout))
            }
            Opener::Button => {
                b.narrow = !key && by_pointer;
                Some(None)
            }
            Opener::Context => {
                b.narrow = !key;
                Some(None)
            }
        }
    }) else {
        return;
    };
    if let Some(flyout) = reject {
        dev(&format!("opening {m} by its button ({}): again on the next frame", if st(|b| b.narrow) == Some(true) { "pointer" } else { "keyboard or screen reader" }));
        let _ = flyout.Hide();
    }
}

/// Menu `m` has opened.
fn opened(m: usize) {
    if let Some(Some(flyout)) = st(|b| {
        (b.rejected == Some(m)).then(|| {
            b.rejected = None;
            b.get(m).map(|x| x.flyout.clone())
        })?
    }) {
        dev(&format!("opened {m} by its button after all: closed"));
        let _ = flyout.Hide();
        return;
    }
    let live = st(|b| {
        if b.get(m).is_none() {
            return false;
        }
        b.open.retain(|&o| o != m);
        b.open.push(m);
        if b.placing == Some(m) {
            b.placing = None;
        }
        // opened by a key (Down, Enter, Space, Alt+F, Left or Right, the menu key): its first
        // item takes the focus, as in any menu opened from the keyboard. A right-click menu goes
        // by how show_at opened it only: a right-click with Ctrl held, or just after an arrow
        // key, is still the pointer's
        let context = b.get(m).is_some_and(|x| x.opener == Opener::Context);
        if b.keyed == Some(m) || (!context && b.key_recent()) {
            b.first_item = Some((m, 10));
        }
        b.keyed = None;
        true
    });
    if live == Some(true) {
        dev(&format!("opened {m}"));
    }
}

/// Menu `m` has closed: the sub-menus under it go too. A sub-menu closed by a click outside it
/// takes the rest with it, unless the click was on a menu still open.
fn closed(m: usize) {
    let Some(Some((ours, kids, sub))) = st(|b| {
        let sub = b.get(m)?.opener == Opener::Sub;
        let ours = if let Some(at) = b.closing.iter().position(|&c| c == m) {
            b.closing.remove(at);
            true
        } else {
            b.rejected == Some(m) || b.again == Some(m)
        };
        let kids = under(m, &b.open, &b.parents());
        b.open.retain(|&o| o != m);
        if b.open.is_empty() && b.rejected != Some(m) && b.again != Some(m) {
            if std::mem::take(&mut b.esc_to_owner) {
                b.refocus = None;
            } else {
                b.refocus = Some(2);
            }
        }
        Some((ours, kids, sub))
    }) else {
        return;
    };
    if ours {
        dev(&format!("closed {m}"));
    } else {
        // (the window's place in the z-order and the mouse's buttons say why, for the log: WinUI
        // closes the menus when the window loses the foreground)
        let fg = st(|b| b.hwnd.is_some_and(crate::win32::is_foreground)).unwrap_or(false);
        dev(&format!("closed {m} from outside (window in front: {fg}, a mouse button down: {})", crate::win32::mouse_button_down()));
    }
    for k in kids {
        close_from(k);
    }
    if !ours && sub && !pointer_on_menu() {
        close_all();
    }
}

/// Whether the pointer is over an open menu.
fn pointer_on_menu() -> bool {
    let Some(Some((root, lists, at))) = st(|b| {
        let at = pointer(b)?;
        Some((b.root.clone(), b.open.iter().filter_map(|&o| b.get(o).map(|x| x.list.clone())).collect::<Vec<_>>(), at))
    }) else {
        return false;
    };
    let rects: Vec<(f64, f64, f64, f64)> = lists.iter().filter_map(|l| shown_rect(l, &root)).collect();
    dev(&format!("pointer at {at:?}; open menus at {rects:?}"));
    hit(at.0, at.1, &rects).is_some()
}

/// The pointer on the page (page pixels), if it is over the window.
fn pointer(b: &Menus) -> Option<(f64, f64)> {
    if b.fake.is_some() {
        return b.fake;
    }
    let p = crate::win32::cursor_in(b.hwnd?)?;
    let scale = b.root.XamlRoot().and_then(|x| x.RasterizationScale()).unwrap_or(1.0);
    Some((f64::from(p.0) / scale, f64::from(p.1) / scale))
}

/// Menu `m` is on screen (its panel is loaded, each time it opens): its rows in the states WinUI's
/// menu gives its items (the padding for how it was opened, room for a check mark or an icon, the
/// shortcut shown), and its checks' shortcuts lined up in one column, as wide as the widest.
fn dress(m: usize) {
    let Some(Some((rows, shape, narrow, list))) = st(|b| b.get(m).map(|x| (x.rows.clone(), x.shape.clone(), b.narrow, x.list.clone()))) else { return };
    let go = |c: &Control, s: &str| VisualStateManager::GoToState(c, &HSTRING::from(s), false);
    // with a shortcut anywhere in the menu, every item keeps the shortcut's column (its margin
    // too, empty), as WinUI's menu does: measured, the Options menu is 24 px wider for it
    let any_key = shape.rows.iter().any(|r| !r.key.is_empty());
    let room = placeholder_state(shape.checks, shape.icons);
    for (c, r) in rows.iter().zip(&shape.rows) {
        let _ = go(c, if narrow { "NarrowPadding" } else { "DefaultPadding" });
        if matches!(r.kind, Kind::Item | Kind::Sub) {
            let _ = go(c, room);
        }
        if matches!(r.kind, Kind::Item | Kind::Toggle) {
            let _ = go(c, if any_key { "KeyboardAcceleratorTextVisible" } else { "KeyboardAcceleratorTextCollapsed" });
        }
    }
    // (WinUI lines up a check's shortcut with the others' left edges; a plain item's sits at the
    // right edge, as measured against WinUI's own menus)
    let keys: Vec<FrameworkElement> = rows.iter().zip(&shape.rows).filter(|(_, r)| !r.key.is_empty() && r.kind == Kind::Toggle).filter_map(|(c, _)| part(&c.cast().ok()?, "KeyboardAcceleratorTextBlock", 0)).collect();
    if !keys.is_empty() {
        for k in &keys {
            let _ = k.SetMinWidth(0.0);
        }
        let _ = list.UpdateLayout();
        let w = keys.iter().filter_map(|k| k.ActualWidth().ok()).fold(0.0, f64::max);
        for k in &keys {
            let _ = k.SetMinWidth(w);
        }
    }
    dev(&format!("menu {m} dressed ({}, {room})", if narrow { "narrow" } else { "wide" }));
}

/// The element named `name` in `el`'s template.
fn part(el: &DependencyObject, name: &str, depth: u32) -> Option<FrameworkElement> {
    if depth > 8 {
        return None;
    }
    let n = VisualTreeHelper::GetChildrenCount(el).ok()?;
    for i in 0..n {
        let Ok(k) = VisualTreeHelper::GetChild(el, i) else { continue };
        if let Ok(fe) = k.cast::<FrameworkElement>() {
            if fe.Name().is_ok_and(|n| n == name) {
                return Some(fe);
            }
        }
        if let Some(found) = part(&k, name, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// A key in open menu `m`, before its rows see it.
fn list_key(m: usize, e: &KeyRoutedEventArgs) {
    if let Ok(true) = e.Handled() {
        return;
    }
    let Ok(k) = e.Key() else { return };
    // Enter on a dot picks it, as on WinUI's (a radio button takes only Space): its command runs
    // when its mark changes; picked already, the menus just close
    if k.0 == 13 {
        let dot = st(|b| focused(&b.root).and_then(|f| b.row_of(&f)).filter(|&(o, r)| o == m && b.get(o).is_some_and(|x| x.shape.rows[r].kind == Kind::Radio)).and_then(|(o, r)| Some(b.get(o)?.rows[r].clone()))).flatten();
        if let Some(Ok(t)) = dot.map(|c| c.cast::<ToggleButton>()) {
            let _ = e.SetHandled(true);
            if t.IsChecked() == Ok(true) {
                close_all();
            } else {
                let _ = t.SetIsChecked(Some(true));
            }
            return;
        }
    }
    let key = Key::of(k.0);
    if matches!(key, Key::Letter(_) | Key::Other) {
        return;
    }
    let Some(Some((at, deepest, kinds, rows, top))) = st(|b| {
        let f = focused(&b.root);
        let at = f.as_ref().and_then(|f| b.row_of(f));
        let menu = b.get(m)?;
        let kinds: Vec<Kind> = menu.shape.rows.iter().map(|r| r.kind).collect();
        let top = match menu.opener {
            Opener::Bar(i) => Place::Bar(i, crate::menubar::MENUS.len()),
            Opener::Sub => Place::Sub,
            Opener::Button | Opener::Context => Place::Top,
        };
        Some((at, b.open.last().copied(), kinds, menu.rows.clone(), top))
    }) else {
        return;
    };
    // a key on another menu's row (a sub-menu's keys pass by here), or with the focus on none:
    // the deepest open menu's
    let at = match at {
        Some((o, _)) if o != m => return,
        Some((_, r)) => Some(r),
        None if deepest != Some(m) => return,
        None => None,
    };
    // (a row the code has hidden takes no focus: passed over as one that is off)
    let enabled: Vec<bool> = rows.iter().map(|c| c.IsEnabled().unwrap_or(true) && c.Visibility().is_ok_and(|v| v == Visibility::Visible)).collect();
    let act = menu_key(key, at, &kinds, &enabled, top);
    dev(&format!("menu {m} key {} on {at:?}: {act:?}", k.0));
    if act == InMenu::None {
        return;
    }
    let _ = e.SetHandled(true);
    match act {
        InMenu::None | InMenu::Hold => {}
        InMenu::Move(r) => {
            let _ = rows[r].Focus(FocusState::Keyboard);
        }
        InMenu::OpenSub(r) => {
            let Some(Some(k)) = st(|b| {
                let k = b.get(m)?.shape.rows[r].sub;
                if let Some(k) = k {
                    b.first_item = Some((k, 10));
                }
                k
            }) else {
                return;
            };
            show(k);
        }
        InMenu::Close => {
            let Some(Some(owner)) = st(|b| {
                let menu = b.get(m)?;
                let owner = menu.owner.clone();
                // one of the bar's or a button's: the focus stays on its title or button (the
                // bar is left with a second Esc); a right-click menu's goes back where it was
                if matches!(menu.opener, Opener::Bar(_) | Opener::Button) {
                    b.esc_to_owner = true;
                }
                Some(owner)
            }) else {
                return;
            };
            close_from(m);
            if let Some(o) = owner {
                let _ = o.Focus(FocusState::Keyboard);
            }
        }
        InMenu::Bar(a) => {
            set_narrow(false);
            crate::menubar::run(a);
        }
    }
}

/// Each frame: with no menu open, what has the focus outside the menus is noted; the menus all
/// closed a moment ago, the focus goes back to it, with the focus state it had (gone from the
/// page: to the tree). Unless the focus has gone somewhere else meanwhile: a dialog, Settings,
/// or wherever the command that closed the menus put it.
fn refocus() {
    let Some(Some(back)) = st(|b| {
        if !b.open.is_empty() || b.placing.is_some() || b.again.is_some() {
            b.refocus = None;
            return None;
        }
        let now = focused(&b.root);
        let free = now.as_ref().is_none_or(|f| b.on_menu(f));
        match b.refocus {
            Some(0) => {
                b.refocus = None;
                // still on a menu (or nowhere): back where it was
                free.then(|| b.outside.clone())
            }
            Some(n) => {
                b.refocus = Some(n - 1);
                None
            }
            None => {
                if !free {
                    if let Some(f) = now {
                        let state = f.FocusState().unwrap_or(FocusState::Programmatic);
                        b.outside = Some((f, state));
                    }
                }
                None
            }
        }
    }) else {
        return;
    };
    let went = back.as_ref().is_some_and(|(el, state)| {
        let state = if *state == FocusState::Unfocused { FocusState::Programmatic } else { *state };
        el.XamlRoot().is_ok() && el.Focus(state).unwrap_or(false)
    });
    if !went {
        if let Some(tree) = st(|b| b.root.cast::<FrameworkElement>().ok()).flatten().and_then(|r| find::<UIElement>(&r, "TreeScroll").ok()) {
            let _ = tree.Focus(FocusState::Programmatic);
        }
    }
    crate::menubar::refocused();
    let name = back.and_then(|(el, _)| el.cast::<FrameworkElement>().ok()).and_then(|f| f.Name().ok()).map(|n| n.to_string_lossy()).unwrap_or_default();
    dev(&format!("focus back {}", if went { format!("where it was ({name:?})") } else { "on the tree".to_string() }));
}

/// Called once per frame: a menu to open again where it belongs, the first item of a menu opened
/// from the keyboard, the bar's titles' and the sub-menus' rows' looks, and the pointer: onto
/// another title while one of the bar's menus is open, or resting on a menu's row.
pub(crate) fn tick() {
    if st(|b| b.hwnd.is_none()) == Some(true) {
        if let Some(h) = crate::state::with(|s| s.hwnd) {
            st(|b| b.hwnd = Some(h));
        }
    }
    crate::menubar::tick(st(|b| b.hwnd).flatten());
    refocus();
    if let Some(Some(m)) = st(|b| b.again.take()) {
        show(m);
    }
    // a menu opened by the keyboard: its first item that takes the focus
    if let Some(Some((m, tries))) = st(|b| b.first_item) {
        let done = st(|b| !b.open.contains(&m)).unwrap_or(true) || tries == 0 || focus_first(m);
        st(|b| b.first_item = if done { None } else { Some((m, tries - 1)) });
    }
    sub_looks();
    let Some((root, open, before, now)) = st(|b| {
        let now = pointer(b);
        let before = std::mem::replace(&mut b.cursor, now.map(|(x, y)| ((x * 64.0) as i32, (y * 64.0) as i32)));
        (b.root.clone(), b.open.clone(), before, now)
    }) else {
        return;
    };
    // nothing open, or the pointer off the window: no row to rest on
    if open.is_empty() || now.is_none() {
        st(|b| {
            b.resting = None;
            b.on_row = None;
        });
    }
    let Some(&top) = open.first() else { return };
    if let Some(p) = now {
        let moved = before != Some(((p.0 * 64.0) as i32, (p.1 * 64.0) as i32));
        if moved {
            // onto another of the bar's titles: its menu instead
            if let Some(Some(Opener::Bar(i))) = st(|b| b.opener(top)) {
                if crate::menubar::hover(p, i) {
                    return;
                }
            }
            pointer_on_rows(p, &root, &open);
        }
    }
    // resting on a row long enough: the sub-menus of its menu's other rows close, and its own
    // opens
    let Some(Some((m, r))) = st(|b| match b.resting {
        Some((m, r, since)) if since.elapsed() >= b.delay => {
            b.resting = None;
            Some((m, r))
        }
        _ => None,
    }) else {
        return;
    };
    let Some((stale, sub)) = st(|b| {
        if !b.open.contains(&m) {
            return (Vec::new(), None);
        }
        let stale: Vec<usize> = b.open.iter().copied().filter(|&o| b.get(o).and_then(|x| x.shape.parent).is_some_and(|(p, pr)| p == m && pr != r)).collect();
        let sub = b.get(m).and_then(|x| x.shape.rows[r].sub).filter(|k| !b.open.contains(k));
        (stale, sub)
    }) else {
        return;
    };
    let had_subs = !stale.is_empty();
    for o in stale {
        close_from(o);
    }
    // the row it rests on takes the focus the sub-menu had
    if had_subs {
        if let Some(Some(el)) = st(|b| b.get(m).map(|x| x.els[r].clone())) {
            let _ = el.Focus(FocusState::Pointer);
        }
    }
    if let Some(k) = sub {
        dev(&format!("pointer rests on menu {m} row {r}: sub-menu {k}"));
        show(k);
    }
}

/// The pointer moved, with menus open: the row it is now on takes the focus (as in WinUI's
/// menus, so the keys go on from there), and starts the wait for its sub-menu.
fn pointer_on_rows(p: (f64, f64), root: &UIElement, open: &[usize]) {
    let Some(rows) = st(|b| open.iter().rev().filter_map(|&m| b.get(m).map(|x| (m, x.els.clone(), x.list.clone()))).collect::<Vec<_>>()) else { return };
    let mut found = None;
    let mut over = None;
    for (m, els, list) in &rows {
        // a tall menu scrolls: only what is in its window counts
        let Some(seen) = shown_rect(list, root) else { continue };
        if over.is_none() && hit(p.0, p.1, &[seen]).is_some() {
            over = Some(*m);
        }
        let rects: Vec<(f64, f64, f64, f64)> = els.iter().map(|e| rect(e, root).map_or((0.0, 0.0, 0.0, 0.0), |r| clip(r, seen))).collect();
        if let Some(r) = hit(p.0, p.1, &rects) {
            found = Some((*m, r, els[r].clone()));
            break;
        }
    }
    pass_through(over);
    let now = found.as_ref().map(|f| (f.0, f.1));
    if st(|b| std::mem::replace(&mut b.on_row, now)) == Some(now) {
        return;
    }
    // off the rows: the wait for a sub-menu ends
    if found.is_none() {
        st(|b| b.resting = None);
    }
    if let Some((m, r, el)) = found {
        // with a sub-menu of this menu open, the focus stays in it until that closes (after
        // the delay, in [`tick`]), as in WinUI's menus
        if st(|b| under(m, &b.open, &b.parents()).is_empty()) == Some(true) {
            let _ = el.Focus(FocusState::Pointer);
        }
        st(|b| b.resting = Some((m, r, Instant::now())));
    }
}

/// Each open sub-menu lets the pointer through to the menu it is over, when that is one it opened
/// from (however far up), else to the menu it opened from: a click on a row further up is the
/// row's, not swallowed closing the sub-menus.
fn pass_through(over: Option<usize>) {
    let Some(changes) = st(|b| {
        let parents = b.parents();
        let mut changes = Vec::new();
        for k in b.open.clone() {
            let Some(parent) = parents.get(k).copied().flatten() else { continue };
            let mut up = Some(parent);
            let mut target = parent;
            while let Some(a) = up {
                if Some(a) == over {
                    target = a;
                    break;
                }
                up = parents.get(a).copied().flatten();
            }
            let Some(list) = b.get(target).map(|x| x.list.clone()) else { continue };
            let Some(menu) = b.get_mut(k) else { continue };
            if menu.through != Some(target) {
                menu.through = Some(target);
                changes.push((menu.flyout.clone(), list));
            }
        }
        changes
    }) else {
        return;
    };
    for (flyout, list) in changes {
        let _ = flyout.SetOverlayInputPassThroughElement(&list);
    }
}

/// The sub-menus' rows' states, as `MenuFlyoutSubItem` sets them.
fn sub_looks() {
    let Some(subs) = st(|b| b.menus.iter().enumerate().filter_map(|(k, x)| x.as_ref().filter(|x| x.opener == Opener::Sub).and_then(|x| Some((k, x.owner.clone()?, x.sub_shown, b.open.contains(&k))))).collect::<Vec<_>>()) else { return };
    for (k, row, shown, open) in subs {
        let Ok(button) = row.cast::<ButtonBase>() else { continue };
        let want = sub_look(row.IsEnabled().unwrap_or(true), open, button.IsPressed().unwrap_or(false), button.IsPointerOver().unwrap_or(false));
        // (not yet templated, the first time its menu opens: tried again next frame)
        if want != shown && VisualStateManager::GoToState(&row, &HSTRING::from(want), false).unwrap_or(false) {
            st(|b| {
                if let Some(x) = b.get_mut(k) {
                    x.sub_shown = want;
                }
            });
        }
    }
}

/// The focus on menu `m`'s first item that can take it; whether one did.
fn focus_first(m: usize) -> bool {
    let Some(Some(rows)) = st(|b| b.get(m).map(|x| x.rows.clone())) else { return true };
    for (k, row) in rows.iter().enumerate() {
        if row.Focus(FocusState::Keyboard).unwrap_or(false) {
            dev(&format!("menu {m}: item {k} has the focus"));
            return true;
        }
    }
    false
}

/// dev: a line in the timing log (SPRAWL_TIMING), for the screenshot runs that drive the menus.
pub(crate) fn dev(what: &str) {
    crate::state::timing(&format!("menu: {what}"), Instant::now());
}

/// dev: the pointer said to be on the middle of the element named `name` (a menu's row), for
/// [`tick`] to find there from now on (the screenshot runs cannot move the real pointer).
pub(crate) fn dev_point(name: &str) {
    let Some(root) = root() else { return };
    let Some(page) = root.cast::<FrameworkElement>().ok() else { return };
    let Ok(el) = find::<UIElement>(&page, name) else { return };
    if let Some((l, t, w, h)) = rect(&el, &root) {
        dev(&format!("pointer onto {name} at {:.0},{:.0}", l + w / 2.0, t + h / 2.0));
        st(|b| b.fake = Some((l + w / 2.0, t + h / 2.0)));
    }
}

/// dev: what has the focus and which menus are open, in the timing log.
pub(crate) fn dev_report() {
    let Some((root, open)) = st(|b| (b.root.clone(), b.open.iter().map(|&o| (o, b.get(o).map(|x| x.set.clone()).unwrap_or_default())).collect::<Vec<_>>())) else { return };
    let what = focused(&root).map_or("nothing".to_string(), |f| {
        let name = f.cast::<FrameworkElement>().and_then(|f| f.Name()).map(|n| n.to_string_lossy()).unwrap_or_default();
        let class = f.cast::<windows_core::IInspectable>().and_then(|i| i.GetRuntimeClassName()).map(|n| n.to_string_lossy()).unwrap_or_default();
        let state = f.FocusState().map(|s| s.0).unwrap_or(-1);
        format!("{class} {name:?} (state {state})")
    });
    dev(&format!("focus on {what}; open menus {open:?}"));
}

/// The part of a menu's rows panel `list` on screen: its rectangle cut to the scroll viewer
/// around it (a menu too tall for the window scrolls).
fn shown_rect(list: &FrameworkElement, root: &UIElement) -> Option<(f64, f64, f64, f64)> {
    use crate::bindings::Microsoft::UI::Xaml::Controls::ScrollViewer;
    let r = rect(&list.cast().ok()?, root)?;
    let mut el: DependencyObject = list.cast().ok()?;
    for _ in 0..8 {
        let Ok(up) = VisualTreeHelper::GetParent(&el) else { break };
        if up.cast::<ScrollViewer>().is_ok() {
            return Some(clip(r, rect(&up.cast().ok()?, root)?));
        }
        el = up;
    }
    Some(r)
}

/// An element's rectangle on the page (left, top, width, height).
pub(crate) fn rect(el: &UIElement, root: &UIElement) -> Option<(f64, f64, f64, f64)> {
    let fe: FrameworkElement = el.cast().ok()?;
    let at = el.TransformToVisual(root).ok()?.TransformPoint(Point { X: 0.0, Y: 0.0 }).ok()?;
    Some((f64::from(at.X), f64::from(at.Y), fe.ActualWidth().ok()?, fe.ActualHeight().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    use Kind::{Item as It, Sub, Toggle};

    #[test]
    fn up_and_down_go_round_a_menu_skipping_what_is_off() {
        let kinds = [It, It, Toggle, It];
        let all = [true; 4];
        assert_eq!(menu_key(Key::Down, None, &kinds, &all, Place::Sub), InMenu::Move(0), "nothing focused: the first");
        assert_eq!(menu_key(Key::Up, None, &kinds, &all, Place::Sub), InMenu::Move(3), "nothing focused: the last");
        assert_eq!(menu_key(Key::Down, Some(1), &kinds, &all, Place::Sub), InMenu::Move(2));
        assert_eq!(menu_key(Key::Down, Some(3), &kinds, &all, Place::Sub), InMenu::Move(0), "round the end");
        assert_eq!(menu_key(Key::Up, Some(0), &kinds, &all, Place::Sub), InMenu::Move(3), "round the start");
        assert_eq!(menu_key(Key::Home, Some(2), &kinds, &all, Place::Sub), InMenu::Move(0));
        assert_eq!(menu_key(Key::End, Some(0), &kinds, &all, Place::Sub), InMenu::Move(3));
        let some = [true, false, true, true];
        assert_eq!(menu_key(Key::Down, Some(0), &kinds, &some, Place::Sub), InMenu::Move(2), "the one that is off is skipped");
        assert_eq!(menu_key(Key::Up, Some(2), &kinds, &some, Place::Sub), InMenu::Move(0));
        assert_eq!(menu_key(Key::Down, Some(0), &[It], &[true], Place::Sub), InMenu::Move(0), "alone: stays");
        assert_eq!(menu_key(Key::Down, None, &[It, It], &[false, false], Place::Sub), InMenu::Hold);
    }

    #[test]
    fn right_opens_a_sub_menu_and_left_and_esc_close_one() {
        let kinds = [It, Sub, It];
        let all = [true; 3];
        // in a sub-menu, or a menu not on the bar
        assert_eq!(menu_key(Key::Right, Some(1), &kinds, &all, Place::Sub), InMenu::OpenSub(1));
        assert_eq!(menu_key(Key::Right, Some(0), &kinds, &all, Place::Sub), InMenu::Hold, "kept from the row: a dot would pick the next");
        assert_eq!(menu_key(Key::Left, Some(0), &kinds, &all, Place::Sub), InMenu::Close, "a sub-menu: back to its row");
        assert_eq!(menu_key(Key::Left, Some(0), &kinds, &all, Place::Top), InMenu::Hold, "a right-click or button's menu stays");
        assert_eq!(menu_key(Key::Right, Some(0), &kinds, &all, Place::Top), InMenu::Hold);
        assert_eq!(menu_key(Key::Right, Some(1), &kinds, &all, Place::Top), InMenu::OpenSub(1));
        assert_eq!(menu_key(Key::Escape, Some(0), &kinds, &all, Place::Top), InMenu::Close);
        assert_eq!(menu_key(Key::Escape, Some(2), &kinds, &all, Place::Sub), InMenu::Close);
        assert_eq!(menu_key(Key::Right, Some(1), &kinds, &[true, false, true], Place::Sub), InMenu::Hold, "an item that is off opens nothing");
    }

    #[test]
    fn in_one_of_the_bars_menus_left_and_right_switch_menus_but_right_opens_a_sub_menu() {
        let kinds = [It, Sub, It];
        let all = [true; 3];
        assert_eq!(menu_key(Key::Right, Some(0), &kinds, &all, Place::Bar(0, 3)), InMenu::Bar(Act::Switch(1)));
        assert_eq!(menu_key(Key::Left, Some(0), &kinds, &all, Place::Bar(0, 3)), InMenu::Bar(Act::Switch(2)));
        assert_eq!(menu_key(Key::Right, Some(2), &kinds, &all, Place::Bar(2, 3)), InMenu::Bar(Act::Switch(0)));
        assert_eq!(menu_key(Key::Right, None, &kinds, &all, Place::Bar(1, 3)), InMenu::Bar(Act::Switch(2)));
        // Right on "Show >" opens the sub-menu instead; Left on it still moves on
        assert_eq!(menu_key(Key::Right, Some(1), &kinds, &all, Place::Bar(1, 3)), InMenu::OpenSub(1));
        assert_eq!(menu_key(Key::Left, Some(1), &kinds, &all, Place::Bar(1, 3)), InMenu::Bar(Act::Switch(0)));
        assert_eq!(menu_key(Key::Escape, Some(1), &kinds, &all, Place::Bar(1, 3)), InMenu::Close);
        assert_eq!(menu_key(Key::Letter('S'), Some(1), &kinds, &all, Place::Bar(1, 3)), InMenu::None);
        assert_eq!(menu_key(Key::Other, Some(1), &kinds, &all, Place::Bar(1, 3)), InMenu::None, "Enter and Space are the row's own");
    }

    #[test]
    fn the_menus_under_a_menu_are_closed_deepest_first() {
        // 0 File, 1 Options, 2 Help, 3 Options > Show, 4 Options > Map, 5 Map > 3D Antialiasing
        let parents = [None, None, None, Some(1), Some(1), Some(4)];
        assert_eq!(under(1, &[1, 4, 5], &parents), vec![5, 4]);
        assert_eq!(under(4, &[1, 4, 5], &parents), vec![5]);
        assert_eq!(under(5, &[1, 4, 5], &parents), Vec::<usize>::new());
        assert_eq!(under(0, &[1, 4, 5], &parents), Vec::<usize>::new());
        assert_eq!(under(1, &[1, 3], &parents), vec![3]);
        // a place left empty (a set bound again) has no parent
        assert_eq!(under(1, &[1, 3], &[None, None, None, None]), Vec::<usize>::new());
    }

    #[test]
    fn keys_map_from_winuis_virtual_keys() {
        assert_eq!(Key::of(37), Key::Left);
        assert_eq!(Key::of(39), Key::Right);
        assert_eq!(Key::of(40), Key::Down);
        assert_eq!(Key::of(36), Key::Home);
        assert_eq!(Key::of(35), Key::End);
        assert_eq!(Key::of(27), Key::Escape);
        assert_eq!(Key::of(70), Key::Letter('F'));
        assert_eq!(Key::of(13), Key::Other, "Enter is the button's click");
    }

    #[test]
    fn a_row_scrolled_out_of_its_menu_is_not_under_the_pointer() {
        let menu = (10.0, 30.0, 200.0, 100.0);
        assert_eq!(clip((10.0, 50.0, 200.0, 30.0), menu), (10.0, 50.0, 200.0, 30.0), "inside: whole");
        assert_eq!(clip((10.0, 120.0, 200.0, 30.0), menu), (10.0, 120.0, 200.0, 10.0), "half out: cut");
        let gone = clip((10.0, 200.0, 200.0, 30.0), menu);
        assert_eq!(hit(50.0, 210.0, &[gone]), None, "below the menu: not hit");
        assert_eq!(hit(50.0, 10.0, &[clip((10.0, 0.0, 200.0, 30.0), menu)]), None, "above it: not hit");
    }

    #[test]
    fn a_sub_menus_row_looks_open_whatever_the_pointer_does() {
        assert_eq!(sub_look(true, false, false, false), "SubNormal");
        assert_eq!(sub_look(true, false, false, true), "SubPointerOver");
        assert_eq!(sub_look(true, false, true, true), "SubPressed");
        assert_eq!(sub_look(true, true, false, false), "SubMenuOpened");
        assert_eq!(sub_look(true, true, true, true), "SubMenuOpened");
        assert_eq!(sub_look(false, true, true, true), "SubDisabled");
    }

    #[test]
    fn the_room_before_the_text_is_winuis_for_checks_and_icons() {
        assert_eq!(placeholder_state(false, false), "NoPlaceholder");
        assert_eq!(placeholder_state(true, false), "CheckPlaceholder");
        assert_eq!(placeholder_state(false, true), "IconPlaceholder");
        assert_eq!(placeholder_state(true, true), "CheckAndIconPlaceholder");
        // and every one of them is a state of the item's template
        let s = styles();
        for name in ["NoPlaceholder", "CheckPlaceholder", "IconPlaceholder", "CheckAndIconPlaceholder"] {
            assert_eq!(s.matches(&format!(r#"<VisualState x:Name="{name}""#)).count(), 2, "{name}: the item's and the sub-menu row's");
        }
        assert!(s.contains(r#"Target="IconRoot.Margin" Value="{ThemeResource MenuFlyoutItemPlaceholderThemeThickness}""#));
        assert!(s.contains("MenuFlyoutItemDoublePlaceholderThemeThickness"));
    }

    #[test]
    fn shortcuts_read_as_winui_shows_them() {
        assert_eq!(key_text("F5", ""), "F5");
        assert_eq!(key_text("F3", "Shift"), "Shift+F3");
        assert_eq!(key_text("O", "Control"), "Ctrl+O");
        assert_eq!(key_text("F", "Control,Shift"), "Ctrl+Shift+F");
        assert_eq!(key_text("Escape", ""), "Esc");
        assert_eq!(key_text("Delete", "Shift"), "Shift+Delete");
        assert_eq!(key_text("F4", "Menu"), "Alt+F4");
        assert_eq!(key_text("", "Control"), "");
    }

    fn well_formed(x: &str) {
        // every element opened is closed, in order (a cheap check of the generated markup)
        let mut stack: Vec<String> = Vec::new();
        let mut rest = x;
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

    #[test]
    fn a_right_click_menu_is_a_plain_flyout_of_rows_with_icons_tips_and_hidden_rows() {
        let rows = vec![
            Row::Item(Item::new("cOpen", "Open").icon("&#xE8E5;")),
            Row::Item(Item::new("cInTree", "Show in the Tree").icon("&#xE8A9;").hidden()),
            Row::Sep,
            Row::Item(Item::new("cGone", "Delete").icon("&#xE74D;").icon_fg("Red")),
            Row::Item(Item::new("", "A label").off(true).size("Big").tip("why")),
            Row::Sub { text: "More".into(), rows: vec![Row::Item(Item::new("deep", "Deep"))] },
        ];
        let x = xaml("RowMenuT", "", "BottomEdgeAlignedLeft", &rows);
        well_formed(&x);
        assert!(x.starts_with(r#"<Flyout x:Name="RowMenuT" Placement="BottomEdgeAlignedLeft" ShouldConstrainToRootBounds="True" AreOpenCloseAnimationsEnabled="False" FlyoutPresenterStyle="{StaticResource SprawlMenu}"><StackPanel x:Name="RowMenuTList""#), "{x}");
        assert!(!x.contains("<MenuFlyout"), "no WinUI menus: they open in a window of their own");
        assert_eq!(x.matches(r#"AutomationProperties.AutomationControlType="MenuItem""#).count(), 6);
        assert!(x.contains(r#"<Button.CommandParameter><FontIcon Glyph="&#xE8E5;"/></Button.CommandParameter>"#), "{x}");
        assert!(x.contains(r#"<FontIcon Glyph="&#xE74D;" Foreground="Red"/>"#), "the icon keeps its own colour");
        assert!(x.contains(r#"x:Name="cInTree" Style="{StaticResource SprawlRowItem}" Content="Show in the Tree" Tag="" AutomationProperties.AutomationControlType="MenuItem" Visibility="Collapsed""#), "{x}");
        assert!(x.contains(r#"x:Name="RowMenuTR0_4" Style="{StaticResource SprawlRowItem}" Content="A label" Tag="" AutomationProperties.AutomationControlType="MenuItem" ToolTipService.ToolTip="why" IsEnabled="False" FontSize="Big"/>"#), "a row without a name gets one: {x}");
        assert!(x.contains(r#"<DropDownButton x:Name="RowMenuTSub1" Style="{StaticResource SprawlRowSub}""#), "{x}");
        let shapes = PENDING.with(|p| p.borrow().get("RowMenuT").cloned()).unwrap();
        assert_eq!(shapes.len(), 2);
        assert_eq!((shapes[0].owner.as_str(), shapes[0].flyout.as_str(), shapes[0].list.as_str()), ("", "RowMenuT", "RowMenuTList"));
        assert!(shapes[0].icons && !shapes[0].checks && !shapes[1].icons);
        assert_eq!(shapes[0].rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["cOpen", "cInTree", "cGone", "RowMenuTR0_4", "RowMenuTSub1"]);
        assert_eq!(shapes[0].rows[4].sub, Some(1));
        assert_eq!(shapes[1].parent, Some((0, 4)));
        assert_eq!(shapes[1].flyout, "", "a sub-menu is its row's own flyout");
    }

    #[test]
    fn a_dropdowns_menu_names_its_button_and_its_checks_carry_their_tips() {
        let rows = vec![Row::Toggle { name: "SxDD".into(), text: "Also drive D:".into(), checked: true, key: String::new(), mods: String::new(), tip: "Search D too".into() }];
        let x = xaml("SxDrivesT", "SxDrivesBtn", "Bottom", &rows);
        well_formed(&x);
        assert!(x.contains(r#"<ToggleButton x:Name="SxDD" Style="{StaticResource SprawlRowToggle}" Content="Also drive D:" IsChecked="true" Tag="" AutomationProperties.AutomationControlType="MenuItem" ToolTipService.ToolTip="Search D too"/>"#), "{x}");
        let shapes = PENDING.with(|p| p.borrow().get("SxDrivesT").cloned()).unwrap();
        assert_eq!(shapes[0].owner, "SxDrivesBtn");
        assert!(shapes[0].checks);
    }

    #[test]
    fn the_styles_are_winuis_menu_drawn_in_a_plain_flyout() {
        let s = styles();
        well_formed(&format!("<R>{s}</R>"));
        assert!(s.contains(r#"<Style x:Key="SprawlMenu" TargetType="FlyoutPresenter">"#) && s.contains(r#"<Setter Property="Tag" Value="menu"/>"#));
        for key in ["SprawlRowItem", "SprawlRowToggle", "SprawlRowRadio", "SprawlRowSub", "SprawlRowSep"] {
            assert!(s.contains(&format!(r#"x:Key="{key}""#)), "{key}");
        }
        // the icon column: WinUI's 16 px box, the icon taking the item's colours in each state
        assert!(s.contains(r#"<Viewbox x:Name="IconRoot" HorizontalAlignment="Left" VerticalAlignment="Center" Width="16" Height="16" Visibility="Collapsed">"#));
        for r in ["MenuFlyoutItemForegroundPointerOver", "MenuFlyoutItemForegroundPressed", "MenuFlyoutItemForegroundDisabled"] {
            assert!(s.contains(&format!(r#"<Setter Target="IconContent.Foreground" Value="{{ThemeResource {r}}}"/>"#)), "{r}");
        }
    }
}
