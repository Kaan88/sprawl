//! The menu bar: File, Options and Help, and their menus.
//!
//! WinUI's own MenuBar and MenuFlyout open each menu in a window of their own, a few pixels
//! bigger than the menu, with WinUI's clip reveal and slide: the mockup's `.menu` entrance
//! ([`crate::popups::watch`]) and its shadow (shade.rs) would be cut off square there. Here each
//! title is a button holding a plain `Flyout`, which WinUI draws inside the app's window, and
//! the menu in it is rows of our own, drawn as WinUI draws a menu (generic.xaml's
//! `MenuFlyoutPresenter`, `MenuFlyoutItem`, `ToggleMenuFlyoutItem`, `RadioMenuFlyoutItem`,
//! `MenuFlyoutSubItem` and `MenuFlyoutSeparator`: the same brushes, paddings, margins, glyphs and
//! states). A sub-menu is a flyout of its own, opened from its row.
//!
//! What `MenuBar` and the menus did on their own is done here:
//!
//! * the pointer moving onto another title while a menu is open opens that one instead;
//! * Alt (pressed and let go alone) puts the keyboard on the bar and takes it back; Left and
//!   Right move along the bar, and switch menus while one is open; Down, Enter and Space open;
//!   Esc closes a menu and puts the focus back on its title, and Esc on the bar gives the focus
//!   back to where it was; Alt+F, Alt+O and Alt+H open the menus (WinUI's access keys);
//! * in a menu: Up, Down, Home and End move between its items, round the ends; Enter and Space
//!   run one; Right (or Enter) on an item with a sub-menu opens it with its first item focused,
//!   and Left or Esc in a sub-menu closes it; the pointer resting on an item with a sub-menu
//!   opens it after Windows' menu delay, and resting on another item closes it again;
//! * a menu opened from the keyboard has its first item focused, and its items padded as WinUI
//!   pads them for the keyboard; opened with the pointer, the narrower padding;
//! * each item's shortcut is in the menu's own column, and works with the menu closed (the bar
//!   holds the keyboard accelerators);
//! * a click on the open menu's title closes it (the click outside a menu is the menu's);
//! * each title and item is a menu item to UI Automation, with expand and collapse where it has
//!   a menu.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows_core::{Interface, HSTRING};

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, FlyoutBase, ToggleButton};
use crate::bindings::Microsoft::UI::Xaml::Controls::{Button, Control, DropDownButton};
use crate::bindings::Microsoft::UI::Xaml::Input::{FocusManager, KeyRoutedEventArgs, KeyboardAccelerator};
use crate::bindings::Microsoft::UI::Xaml::Media::VisualTreeHelper;
use crate::bindings::Microsoft::UI::Xaml::{DependencyObject, FocusState, FrameworkElement, UIElement, VisualStateManager};
use crate::state::{find, keep};

/// The menus: the button's x:Name, its title, and its access key.
pub(crate) const MENUS: [(&str, &str, char); 3] = [("MenuFile", "File", 'F'), ("MenuOptions", "Options", 'O'), ("MenuHelp", "Help", 'H')];

// ------------------------------------------------------------------------------- the markup

/// A row of a menu, as ui.rs lists them. Texts are XAML attribute text already.
#[derive(Clone, Debug)]
pub(crate) enum Row {
    /// Runs the command of its name.
    Item { name: String, text: String, key: String, mods: String },
    /// A check mark, flipped before its command runs.
    Toggle { name: String, text: String, checked: bool, key: String, mods: String },
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
struct RowShape {
    name: String,
    kind: Kind,
    group: String,
    /// Its shortcut as the menu shows it ("Ctrl+Shift+S"), or nothing.
    key: String,
    /// For a sub-menu's row: that menu.
    sub: Option<usize>,
}

/// A menu as [`xaml`] made it: the bar's three first, in [`MENUS`]'s order, then the sub-menus.
#[derive(Clone, Debug)]
struct MenuShape {
    /// The button it opens from: a title, or a sub-menu's row.
    owner: String,
    /// Its rows' panel.
    list: String,
    /// The menu and row it opens from, for a sub-menu.
    parent: Option<(usize, usize)>,
    /// Its rows, without the separators.
    rows: Vec<RowShape>,
    /// Whether any row has a check mark or a dot: the others' text then lines up with theirs, as
    /// WinUI's `CheckPlaceholder` state does.
    checks: bool,
}

thread_local! {
    /// The menus the last [`xaml`] made, for [`bind`] and [`set_checked`].
    static SHAPE: RefCell<Vec<MenuShape>> = const { RefCell::new(Vec::new()) };
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

/// The flyout a menu is shown in, drawn as WinUI draws a menu (`DefaultMenuFlyoutPresenterStyle`:
/// its colour, line, corner, padding and least size), but inside the window, so the mockup's
/// shadow (shade.rs) has room round it. `Tag` marks it for the mockup's opening
/// ([`crate::popups::watch`]).
const HOLDER: &str = r##"<Style x:Key="SprawlBarMenu" TargetType="FlyoutPresenter">
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

/// The room of a check mark before the text, in a menu where some row has one.
const PLACEHOLDER: &str = r##"<VisualStateGroup x:Name="CheckPlaceholderStates"><VisualState x:Name="NoPlaceholder"/><VisualState x:Name="CheckPlaceholder"><VisualState.Setters><Setter Target="TextBlock.Margin" Value="{ThemeResource MenuFlyoutItemPlaceholderThemeThickness}"/></VisualState.Setters></VisualState></VisualStateGroup>"##;

/// The row's frame: WinUI's `LayoutRoot`, inset by the item margin, with the item's padding.
const ROOT: &str = r##"<Grid x:Name="LayoutRoot" Padding="{TemplateBinding Padding}" Background="{TemplateBinding Background}" Margin="{StaticResource MenuFlyoutItemMargin}" CornerRadius="{TemplateBinding CornerRadius}">"##;

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
        state("PointerOver", &[("LayoutRoot.Background", "MenuFlyoutItemBackgroundPointerOver"), ("TextBlock.Foreground", "MenuFlyoutItemForegroundPointerOver"), ("KeyboardAcceleratorTextBlock.Foreground", "MenuFlyoutItemKeyboardAcceleratorTextForegroundPointerOver")], ""),
        state("Pressed", &[("LayoutRoot.Background", "MenuFlyoutItemBackgroundPressed"), ("TextBlock.Foreground", "MenuFlyoutItemForegroundPressed"), ("KeyboardAcceleratorTextBlock.Foreground", "MenuFlyoutItemKeyboardAcceleratorTextForegroundPressed")], ""),
        state("Disabled", &[("LayoutRoot.Background", "MenuFlyoutItemBackgroundDisabled"), ("TextBlock.Foreground", "MenuFlyoutItemForegroundDisabled"), ("KeyboardAcceleratorTextBlock.Foreground", "MenuFlyoutItemKeyboardAcceleratorTextForegroundDisabled")], ""),
    );
    let item = format!(
        r##"<Style x:Key="SprawlRowItem" TargetType="Button">{}{}</Style>"##,
        row_setters("MenuFlyoutItemBackground", "MenuFlyoutItemForeground"),
        template("Button", format!("{common}{PLACEHOLDER}{PADDING}{KEY_SHOWN}"), two, text_part(0) + &key_part(1, "24,4,0,0", "MenuFlyoutItemKeyboardAcceleratorTextForeground")),
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
        template("DropDownButton", format!("{sub_states}{PLACEHOLDER}{PADDING}"), two, text_part(0) + chevron),
    );
    let sep = r##"<Style x:Key="SprawlRowSep" TargetType="Rectangle"><Setter Property="Fill" Value="{ThemeResource MenuFlyoutSeparatorBackground}"/><Setter Property="Margin" Value="{StaticResource MenuFlyoutSeparatorThemePadding}"/><Setter Property="Height" Value="{StaticResource MenuFlyoutSeparatorHeight}"/></Style>"##;
    // the check, dot and chevron glyphs: 12 whatever the type scale, as in WinUI's templates
    let glyph = r##"<x:Double x:Key="SprawlMenuGlyph">12</x:Double>"##;
    format!("{glyph}{item}{toggle}{radio}{sub}{sep}")
}

/// A menu's flyout, holding its rows' panel.
fn flyout(list: &str, rows: &str) -> String {
    format!(
        r##"<Flyout Placement="BottomEdgeAlignedLeft" ShouldConstrainToRootBounds="True" AreOpenCloseAnimationsEnabled="False" FlyoutPresenterStyle="{{StaticResource SprawlBarMenu}}"><StackPanel x:Name="{list}" XYFocusKeyboardNavigation="Disabled" TabFocusNavigation="Cycle">{rows}</StackPanel></Flyout>"##
    )
}

/// Menu `me`'s rows: the markup, with its sub-menus added to `shapes` and its shortcuts to
/// `keys` (the bar's keyboard accelerators).
fn rows_xaml(rows: &[Row], me: usize, shapes: &mut Vec<MenuShape>, keys: &mut String) -> String {
    let checks = rows.iter().any(|r| matches!(r, Row::Toggle { .. } | Row::Radio { .. }));
    shapes[me].checks = checks;
    let mut x = String::new();
    let accelerator = |keys: &mut String, name: &str, key: &str, mods: &str| {
        if key.is_empty() {
            return;
        }
        let mods = if mods.is_empty() { String::new() } else { format!(r##" Modifiers="{mods}""##) };
        *keys += &format!(r##"<KeyboardAccelerator x:Name="{name}Acc" Key="{key}"{mods}/>"##);
    };
    let ak = |shown: &str| if shown.is_empty() { String::new() } else { format!(r##" AutomationProperties.AcceleratorKey="{shown}""##) };
    for row in rows {
        match row {
            Row::Item { name, text, key, mods } => {
                let shown = key_text(key, mods);
                x += &format!(r##"<Button x:Name="{name}" Style="{{StaticResource SprawlRowItem}}" Content="{text}" Tag="{shown}" AutomationProperties.AutomationControlType="MenuItem"{}/>"##, ak(&shown));
                accelerator(keys, name, key, mods);
                shapes[me].rows.push(RowShape { name: name.clone(), kind: Kind::Item, group: String::new(), key: shown, sub: None });
            }
            Row::Toggle { name, text, checked, key, mods } => {
                let shown = key_text(key, mods);
                x += &format!(r##"<ToggleButton x:Name="{name}" Style="{{StaticResource SprawlRowToggle}}" Content="{text}" IsChecked="{checked}" Tag="{shown}" AutomationProperties.AutomationControlType="MenuItem"{}/>"##, ak(&shown));
                accelerator(keys, name, key, mods);
                shapes[me].rows.push(RowShape { name: name.clone(), kind: Kind::Toggle, group: String::new(), key: shown, sub: None });
            }
            Row::Radio { group, name, text, checked } => {
                x += &format!(r##"<RadioButton x:Name="{name}" Style="{{StaticResource SprawlRowRadio}}" GroupName="{group}" Content="{text}" IsChecked="{checked}" AutomationProperties.AutomationControlType="MenuItem"/>"##);
                shapes[me].rows.push(RowShape { name: name.clone(), kind: Kind::Radio, group: group.clone(), key: String::new(), sub: None });
            }
            Row::Sub { text, rows } => {
                let k = shapes.len();
                let owner = format!("BarSub{k}");
                let list = format!("{owner}List");
                shapes.push(MenuShape { owner: owner.clone(), list: list.clone(), parent: Some((me, shapes[me].rows.len())), rows: Vec::new(), checks: false });
                shapes[me].rows.push(RowShape { name: owner.clone(), kind: Kind::Sub, group: String::new(), key: String::new(), sub: Some(k) });
                let inner = rows_xaml(rows, k, shapes, keys);
                x += &format!(r##"<DropDownButton x:Name="{owner}" Style="{{StaticResource SprawlRowSub}}" Content="{text}" AutomationProperties.AutomationControlType="MenuItem"><DropDownButton.Flyout>{}</DropDownButton.Flyout></DropDownButton>"##, flyout(&list, &inner));
            }
            Row::Sep => x += r##"<Rectangle Style="{StaticResource SprawlRowSep}"/>"##,
        }
    }
    x
}

/// The bar's XAML, for the page's second row: a title button for each menu, holding its menu
/// (`menus`, in [`MENUS`]'s order), and every row's shortcut. As WinUI's MenuBar: 40 high, the
/// titles 4 in from it, one tab stop for the whole bar.
pub(crate) fn xaml(menus: [Vec<Row>; 3]) -> String {
    let mut shapes: Vec<MenuShape> = MENUS.iter().map(|&(name, _, _)| MenuShape { owner: name.to_string(), list: format!("{name}List"), parent: None, rows: Vec::new(), checks: false }).collect();
    let mut keys = String::new();
    let mut buttons = String::new();
    for (i, rows) in menus.iter().enumerate() {
        let (name, title, key) = MENUS[i];
        let inner = rows_xaml(rows, i, &mut shapes, &mut keys);
        buttons += &format!(
            r##"<DropDownButton x:Name="{name}" Content="{title}" AccessKey="{key}" Style="{{StaticResource SprawlBarItem}}" AutomationProperties.AutomationControlType="MenuItem"><DropDownButton.Flyout>{}</DropDownButton.Flyout></DropDownButton>"##,
            flyout(&format!("{name}List"), &inner)
        );
    }
    SHAPE.with(|s| *s.borrow_mut() = shapes);
    format!(
        r##"<StackPanel x:Name="MenuBar" Grid.Row="1" Margin="6,0,0,0" Orientation="Horizontal" MinHeight="40" Background="Transparent" TabFocusNavigation="Once" KeyboardAcceleratorPlacementMode="Hidden" AutomationProperties.Name="Menu bar" AutomationProperties.AutomationControlType="MenuBar"><StackPanel.Resources>{STYLE}{HOLDER}{}</StackPanel.Resources><StackPanel.KeyboardAccelerators>{keys}</StackPanel.KeyboardAccelerators>{buttons}</StackPanel>"##,
        row_styles()
    )
}

// ------------------------------------------------------------------ what the keys and pointer do
//
// Plain logic, without WinUI, so it can be tested.

/// The keys the bar and its menus care about.
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

/// What to do in an open menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum InMenu {
    None,
    /// The focus onto this row.
    Move(usize),
    /// Open this row's sub-menu, its first item focused.
    OpenSub(usize),
    /// Close this menu (a sub-menu: the focus back on its row; one of the bar's: on its title).
    Close,
    /// Something for the bar (Left and Right in one of the bar's own menus).
    Bar(Act),
    /// The menu's key, with nothing to do: kept from the row (a dot's radio button would move
    /// on to the next dot and pick it, without its command).
    Hold,
}

/// A key in an open menu, before its rows see it: `at` is the row with the focus (none: the
/// focus is on none of its rows), `kinds` and `enabled` its rows, and `top` the menu's place on
/// the bar and the bar's length, for one of the bar's own menus. As WinUI's menus: Up and Down
/// go round the ends, skipping what can't take the focus; Enter and Space are the rows' own.
pub(crate) fn menu_key(key: Key, at: Option<usize>, kinds: &[Kind], enabled: &[bool], top: Option<(usize, usize)>) -> InMenu {
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
            _ => top.map_or(InMenu::Hold, |(i, len)| InMenu::Bar(Act::Switch(step(i, len, true)))),
        },
        Key::Left => top.map_or(InMenu::Close, |(i, len)| InMenu::Bar(Act::Switch(step(i, len, false)))),
        Key::Escape => InMenu::Close,
        Key::Letter(_) | Key::Other => InMenu::None,
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

/// The rectangle under (x, y), among rectangles (left, top, width, height).
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

/// The other rows of a group, when one is picked.
fn group_of(shapes: &[MenuShape], name: &str) -> Vec<String> {
    let Some(group) = shapes.iter().flat_map(|m| &m.rows).find(|r| r.name == name && r.kind == Kind::Radio).map(|r| r.group.clone()) else { return Vec::new() };
    shapes.iter().flat_map(|m| &m.rows).filter(|r| r.kind == Kind::Radio && r.group == group && r.name != name).map(|r| r.name.clone()).collect()
}

// ------------------------------------------------------------------------------- the live bar

/// A menu on the page.
struct Menu {
    flyout: FlyoutBase,
    owner: DropDownButton,
    list: FrameworkElement,
    rows: Vec<Control>,
    /// The same rows, to tell which has the focus.
    els: Vec<UIElement>,
    shape: MenuShape,
}

struct Bar {
    root: UIElement,
    menus: Vec<Menu>,
    /// The open menus, the bar's first and then its sub-menus, in the order they opened.
    open: Vec<usize>,
    /// Whether the menus opened with the pointer (WinUI pads their items less).
    narrow: bool,
    /// Each title's states, as last set.
    shown: Vec<(&'static str, &'static str)>,
    /// Each sub-menu's row's state, as last set (by menu).
    sub_shown: Vec<&'static str>,
    hwnd: Option<crate::win32::HWND>,
    /// Where the pointer was on the last frame (window pixels).
    cursor: Option<(i32, i32)>,
    /// dev: where the pointer is said to be (page pixels), for the screenshot runs.
    fake: Option<(f64, f64)>,
    /// What had the focus before Alt put it on the bar.
    before: Option<UIElement>,
    alt: AltTap,
    /// When the page last saw a key: a menu opening just after one was opened by the keyboard.
    last_key: Option<Instant>,
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
    /// Whether the window had the keyboard on the last frame (losing it ends an Alt tap).
    foreground: bool,
    /// Each sub-menu's menu that the pointer goes through to (the one it opened from, or one
    /// further up while the pointer is over that).
    through: Vec<Option<usize>>,
    /// What had the focus outside the bar and its menus, and how it got it, while no menu was
    /// open: where the focus goes back when the menus close, as WinUI's MenuBar does.
    outside: Option<(UIElement, FocusState)>,
    /// The menus have all closed: the focus goes back to [`Bar::outside`] in this many frames
    /// (a menu opening again in the meantime, switching menus, keeps it on the bar).
    refocus: Option<u8>,
    /// Esc closed one of the bar's menus: the focus stays on its title (the bar is left with
    /// a second Esc).
    esc_to_title: bool,
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

fn parents(menus: &[Menu]) -> Vec<Option<usize>> {
    menus.iter().map(|m| m.shape.parent.map(|p| p.0)).collect()
}

/// Wire the bar on the page made at start, and again on the one a theme change makes.
pub(crate) fn bind(root: &FrameworkElement) -> windows_core::Result<()> {
    let shapes = SHAPE.with(|s| s.borrow().clone());
    let mut menus = Vec::new();
    for (m, shape) in shapes.iter().enumerate() {
        let owner: DropDownButton = find(root, &shape.owner).inspect_err(|e| eprintln!("menubar: {}: {e:?}", shape.owner))?;
        let flyout = owner.cast::<Button>()?.Flyout().inspect_err(|e| eprintln!("menubar: flyout: {e:?}"))?;
        let list: FrameworkElement = find(root, &shape.list)?;
        let mut rows = Vec::new();
        let mut els = Vec::new();
        for r in &shape.rows {
            let c: Control = find(root, &r.name)?;
            els.push(c.cast::<UIElement>()?);
            rows.push(c);
        }
        keep(flyout.Opening(move |_, _| opening(m))?);
        keep(flyout.Opened(move |_, _| opened(m))?);
        keep(flyout.Closed(move |_, _| closed(m))?);
        // a sub-menu lets the pointer through to the menu it opened from, so its rows still
        // light up under the pointer and take a click (which closes the sub-menu)
        if let Some((p, _)) = shape.parent {
            let parent_list: FrameworkElement = find(root, &shapes[p].list)?;
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
        // command (bound after this, in main.rs) runs
        for (r, c) in shape.rows.iter().zip(&rows) {
            if r.kind != Kind::Sub {
                keep(c.cast::<ButtonBase>()?.Click(|_, _| close_all())?);
            }
            // a check or a dot runs its command when its mark changes, however it changed: a
            // click, Enter or Space, or a screen reader's Toggle or Select (which click nothing)
            let toggle = c.cast::<ToggleButton>();
            if matches!(r.kind, Kind::Toggle | Kind::Radio) {
                let t = toggle?;
                let name = r.name.clone();
                keep(t.Checked(move |_, _| marked(&name))?);
            }
            if r.kind == Kind::Toggle {
                let name = r.name.clone();
                keep(c.cast::<ToggleButton>()?.Unchecked(move |_, _| marked(&name))?);
            }
        }
        menus.push(Menu { flyout, owner, list, rows, els, shape: shape.clone() });
    }
    // the titles' own keys: along the bar, and open
    for (i, menu) in menus.iter().take(MENUS.len()).enumerate() {
        keep(menu.owner.KeyDown(move |_, e| {
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
                bar(|b| b.narrow = false);
                run(act);
            }
        })?);
    }
    // the shortcuts, on the bar: they work with the menus closed, as a menu item's did
    for shape in &shapes {
        for r in shape.rows.iter().filter(|r| !r.key.is_empty()) {
            let acc: KeyboardAccelerator = find(root, &format!("{}Acc", r.name))?;
            let name = r.name.clone();
            keep(acc.Invoked(move |_, e| {
                if let Some(e) = e.as_ref() {
                    let _ = e.SetHandled(true);
                }
                activate(&name);
            })?);
        }
    }
    // Alt alone: onto the bar and off it again. Seen on its way down to the focused control
    // (tunnelling), before WinUI's access keys or any control can take it.
    let page: UIElement = root.cast()?;
    keep(page.PreviewKeyDown(|_, e| {
        if let Some(k) = e.as_ref().and_then(|e| e.Key().ok()) {
            dev(&format!("key down {}", k.0));
            bar(|b| {
                b.alt.key_down(is_alt(k.0));
                b.last_key = Some(Instant::now());
            });
        }
    })?);
    keep(page.PreviewKeyUp(|_, e| {
        if let Some(k) = e.as_ref().and_then(|e| e.Key().ok()) {
            bar(|b| b.last_key = Some(Instant::now()));
            alt_up(k.0);
        }
    })?);
    let n = menus.len();
    let through = parents(&menus);
    let delay = Duration::from_millis(u64::from(crate::win32::menu_show_delay()));
    BAR.with(|b| {
        *b.borrow_mut() = Some(Bar {
            root: page,
            menus,
            open: Vec::new(),
            narrow: true,
            shown: vec![("", ""); MENUS.len()],
            sub_shown: vec![""; n],
            hwnd: None,
            cursor: None,
            fake: None,
            before: None,
            alt: AltTap::default(),
            last_key: None,
            first_item: None,
            placing: None,
            again: None,
            rejected: None,
            on_row: None,
            resting: None,
            delay,
            closing: Vec::new(),
            foreground: false,
            through,
            outside: None,
            refocus: None,
            esc_to_title: false,
        })
    });
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
    close_all();
    if let Some(root) = bar(|b| b.root.clone()).and_then(|r| r.cast::<FrameworkElement>().ok()) {
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
    let page = bar(|b| b.root.clone()).and_then(|r| r.cast::<FrameworkElement>().ok());
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

fn alt_up(vk: i32) {
    if bar(|b| b.alt.key_up(is_alt(vk))) != Some(true) {
        return;
    }
    dev("alt tap");
    // on the bar: off it again; elsewhere: onto its first title, remembering where from
    let Some((root, titles, before)) = bar(|b| (b.root.clone(), titles(b), b.before.clone())) else { return };
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

fn titles(b: &Bar) -> Vec<DropDownButton> {
    b.menus.iter().take(MENUS.len()).map(|m| m.owner.clone()).collect()
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
    // (Esc off the bar: back to what had the focus before Alt, or before the menu opened)
    let Some((titles, top, before)) = bar(|b| (titles(b), b.open.first().copied(), b.before.clone().or_else(|| b.outside.as_ref().map(|o| o.0.clone())))) else { return };
    match act {
        Act::None => {}
        Act::Focus(j) => {
            let _ = titles[j].Focus(FocusState::Keyboard);
        }
        Act::Open(j) => {
            let _ = titles[j].Focus(FocusState::Keyboard);
            show(j);
        }
        Act::Switch(j) => {
            if top == Some(j) {
                return;
            }
            close_all();
            let _ = titles[j].Focus(FocusState::Keyboard);
            show(j);
        }
        Act::Leave => leave(before),
    }
}

/// Switch to menu `j` from the pointer (no item takes the focus).
fn switch_by_pointer(j: usize) {
    let Some((titles, top)) = bar(|b| (titles(b), b.open.first().copied())) else { return };
    if top == Some(j) {
        return;
    }
    close_all();
    bar(|b| b.narrow = true);
    let _ = titles[j].Focus(FocusState::Pointer);
    show(j);
}

/// Close menu `m` and the sub-menus open under it, as this code's own doing.
fn close_from(m: usize) {
    let Some(flyouts) = bar(|b| {
        let mut all = under(m, &b.open, &parents(&b.menus));
        all.push(m);
        b.closing.extend(all.iter().copied());
        all.iter().map(|&k| b.menus[k].flyout.clone()).collect::<Vec<_>>()
    }) else {
        return;
    };
    for f in flyouts {
        let _ = f.Hide();
    }
}

/// Close every open menu.
fn close_all() {
    if let Some(Some(top)) = bar(|b| b.open.first().copied()) {
        close_from(top);
    }
}

/// How far up the title's bottom edge MenuBar opens its menus.
const LIFT: f32 = 2.0;

/// Open menu `m` where it belongs: one of the bar's at its title's bottom left corner, kept off
/// the title should it open upwards (WinUI's MenuBarItem.cpp ShowMenuFlyout); a sub-menu at its
/// row's right edge, level with the row, beside it (MenuFlyoutSubItem). The other sub-menus of
/// the menu it opens from close first.
fn show(m: usize) {
    use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{FlyoutPlacementMode, FlyoutShowOptions};
    use crate::bindings::Windows::Foundation::{Point, Rect};
    let Some((flyout, owner, top)) = bar(|b| (b.menus[m].flyout.clone(), b.menus[m].owner.clone(), b.menus[m].shape.parent.is_none())) else { return };
    let place = || -> windows_core::Result<()> {
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
    let others = bar(|b| match b.menus[m].shape.parent {
        Some((p, _)) => b.open.iter().copied().filter(|&o| b.menus[o].shape.parent.is_some_and(|q| q.0 == p)).collect(),
        None => Vec::new(),
    })
    .unwrap_or_default();
    for o in others {
        close_from(o);
    }
    bar(|b| {
        b.placing = Some(m);
        b.rejected = None;
    });
    if place().is_err() {
        bar(|b| b.placing = None);
    }
}

/// Menu `m` is about to open. Opened by its button itself (a click, Enter, Space, an access
/// key, a screen reader's Expand), WinUI would put it where a plain flyout goes, and size it
/// for that place: it is closed again before it is laid out or drawn, and opened where it
/// belongs on the next frame ([`tick`]). Opened by [`show`], it goes ahead.
fn opening(m: usize) {
    let Some(Some(flyout)) = bar(|b| {
        // Alt+F's Alt, let go after the menu opened, is not Alt alone
        b.alt = AltTap::default();
        if b.placing == Some(m) {
            b.placing = None;
            return None;
        }
        b.again = Some(m);
        b.rejected = Some(m);
        // one of the bar's, opened by its title: clicked, its items are padded for the pointer
        // (an access key, Alt+F, leaves the focus as the pointer left it: the key decides)
        if b.menus[m].shape.parent.is_none() {
            let key = b.last_key.is_some_and(|t| t.elapsed() < Duration::from_millis(300));
            b.narrow = !key && b.menus[m].owner.FocusState().is_ok_and(|f| f == FocusState::Pointer);
        }
        Some(b.menus[m].flyout.clone())
    }) else {
        return;
    };
    dev(&format!("opening {m} by its button ({}): again on the next frame", if bar(|b| b.narrow) == Some(true) { "pointer" } else { "keyboard or screen reader" }));
    let _ = flyout.Hide();
}

/// Menu `m` has opened.
fn opened(m: usize) {
    if let Some(Some(flyout)) = bar(|b| {
        (b.rejected == Some(m)).then(|| {
            b.rejected = None;
            b.menus[m].flyout.clone()
        })
    }) {
        dev(&format!("opened {m} by its button after all: closed"));
        let _ = flyout.Hide();
        return;
    }
    bar(|b| {
        b.open.retain(|&o| o != m);
        b.open.push(m);
        // opened by a key (Down, Enter, Space, Alt+F, Left or Right): its first item takes the
        // focus, as in any menu opened from the keyboard
        if b.last_key.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
            b.first_item = Some((m, 10));
        }
    });
    dev(&format!("opened {m}"));
}

/// Menu `m` has closed: the sub-menus under it go too. A sub-menu closed by a click outside it
/// takes the rest with it, unless the click was on a menu still open.
fn closed(m: usize) {
    let Some((ours, kids, sub)) = bar(|b| {
        let ours = if let Some(at) = b.closing.iter().position(|&c| c == m) {
            b.closing.remove(at);
            true
        } else {
            b.rejected == Some(m) || b.again == Some(m)
        };
        let kids = under(m, &b.open, &parents(&b.menus));
        b.open.retain(|&o| o != m);
        if b.open.is_empty() && b.rejected != Some(m) && b.again != Some(m) {
            if std::mem::take(&mut b.esc_to_title) {
                b.refocus = None;
            } else {
                b.refocus = Some(2);
            }
        }
        (ours, kids, b.menus[m].shape.parent.is_some())
    }) else {
        return;
    };
    dev(&format!("closed {m}{}", if ours { "" } else { " from outside" }));
    for k in kids {
        close_from(k);
    }
    if !ours && sub && !pointer_on_menu() {
        close_all();
    }
}

/// Whether the pointer is over an open menu.
fn pointer_on_menu() -> bool {
    let Some(Some((root, lists, at))) = bar(|b| {
        let at = pointer(b)?;
        Some((b.root.clone(), b.open.iter().map(|&o| b.menus[o].list.clone()).collect::<Vec<_>>(), at))
    }) else {
        return false;
    };
    let rects: Vec<(f64, f64, f64, f64)> = lists.iter().filter_map(|l| shown_rect(l, &root)).collect();
    dev(&format!("pointer at {at:?}; open menus at {rects:?}"));
    hit(at.0, at.1, &rects).is_some()
}

/// The pointer on the page (page pixels), if it is over the window.
fn pointer(b: &Bar) -> Option<(f64, f64)> {
    if b.fake.is_some() {
        return b.fake;
    }
    let p = crate::win32::cursor_in(b.hwnd?)?;
    let scale = b.root.XamlRoot().and_then(|x| x.RasterizationScale()).unwrap_or(1.0);
    Some((f64::from(p.0) / scale, f64::from(p.1) / scale))
}

/// Menu `m` is on screen (its panel is loaded, each time it opens): its rows in the states WinUI's
/// menu gives its items (the padding for how it was opened, room for a check mark, the shortcut
/// shown), and its checks' shortcuts lined up in one column, as wide as the widest.
fn dress(m: usize) {
    let Some((rows, shape, narrow, list)) = bar(|b| (b.menus[m].rows.clone(), b.menus[m].shape.clone(), b.narrow, b.menus[m].list.clone())) else { return };
    let go = |c: &Control, s: &str| VisualStateManager::GoToState(c, &HSTRING::from(s), false);
    // with a shortcut anywhere in the menu, every item keeps the shortcut's column (its margin
    // too, empty), as WinUI's menu does: measured, the Options menu is 24 px wider for it
    let any_key = shape.rows.iter().any(|r| !r.key.is_empty());
    for (c, r) in rows.iter().zip(&shape.rows) {
        let _ = go(c, if narrow { "NarrowPadding" } else { "DefaultPadding" });
        if matches!(r.kind, Kind::Item | Kind::Sub) {
            let _ = go(c, if shape.checks { "CheckPlaceholder" } else { "NoPlaceholder" });
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
    dev(&format!("menu {m} dressed ({})", if narrow { "narrow" } else { "wide" }));
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

/// The menu and row `el` is, if it is one of the menus' rows.
fn row_of(menus: &[Menu], el: &UIElement) -> Option<(usize, usize)> {
    menus.iter().enumerate().find_map(|(m, menu)| menu.els.iter().position(|e| e == el).map(|r| (m, r)))
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
        let dot = bar(|b| focused(&b.root).and_then(|f| row_of(&b.menus, &f)).filter(|&(o, r)| o == m && b.menus[o].shape.rows[r].kind == Kind::Radio).map(|(o, r)| b.menus[o].rows[r].clone())).flatten();
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
    let Some((root, at, deepest, kinds, rows, top)) = bar(|b| {
        let f = focused(&b.root);
        let at = f.as_ref().and_then(|f| row_of(&b.menus, f));
        let menu = &b.menus[m];
        let kinds: Vec<Kind> = menu.shape.rows.iter().map(|r| r.kind).collect();
        let top = menu.shape.parent.is_none().then_some((m, MENUS.len()));
        (b.root.clone(), at, b.open.last().copied(), kinds, menu.rows.clone(), top)
    }) else {
        return;
    };
    let _ = root;
    // a key on another menu's row (a sub-menu's keys pass by here), or with the focus on none:
    // the deepest open menu's
    let at = match at {
        Some((o, _)) if o != m => return,
        Some((_, r)) => Some(r),
        None if deepest != Some(m) => return,
        None => None,
    };
    let enabled: Vec<bool> = rows.iter().map(|c| c.IsEnabled().unwrap_or(true)).collect();
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
            let Some(Some(k)) = bar(|b| {
                let k = b.menus[m].shape.rows[r].sub;
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
            let Some(owner) = bar(|b| {
                // one of the bar's: the focus stays on its title
                if b.menus[m].shape.parent.is_none() {
                    b.esc_to_title = true;
                }
                b.menus[m].owner.clone()
            }) else {
                return;
            };
            close_from(m);
            let _ = owner.Focus(FocusState::Keyboard);
        }
        InMenu::Bar(a) => {
            bar(|b| b.narrow = false);
            run(a);
        }
    }
}

/// Whether `el` is one of the bar's titles or one of its menus' rows.
fn on_bar(b: &Bar, el: &UIElement) -> bool {
    titles(b).iter().any(|t| t.cast::<UIElement>().is_ok_and(|t| &t == el)) || row_of(&b.menus, el).is_some()
}

/// Each frame: with no menu open, what has the focus outside the bar is noted; the menus all
/// closed a moment ago, the focus goes back to it, with the focus state it had (gone from the
/// page: to the tree). Unless the focus has gone somewhere else meanwhile: a dialog, Settings,
/// or wherever the command that closed the menus put it.
fn refocus() {
    let Some(Some(back)) = bar(|b| {
        if !b.open.is_empty() || b.placing.is_some() || b.again.is_some() {
            b.refocus = None;
            return None;
        }
        let now = focused(&b.root);
        let free = now.as_ref().is_none_or(|f| on_bar(b, f));
        match b.refocus {
            Some(0) => {
                b.refocus = None;
                // still on the bar (or nowhere): back where it was
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
        if let Some(tree) = bar(|b| b.root.cast::<FrameworkElement>().ok()).flatten().and_then(|r| find::<UIElement>(&r, "TreeScroll").ok()) {
            let _ = tree.Focus(FocusState::Programmatic);
        }
    }
    bar(|b| b.before = None);
    dev(&format!("focus back {}", if went { "where it was" } else { "on the tree" }));
}

/// Called once per frame: a menu to open again where it belongs, the first item of a menu opened
/// from the keyboard, the titles' and sub-menus' rows' looks, and the pointer: onto another
/// title while a menu is open, or resting on a menu's row.
pub(crate) fn tick() {
    if bar(|b| b.hwnd.is_none()) == Some(true) {
        if let Some(h) = crate::state::with(|s| s.hwnd) {
            bar(|b| b.hwnd = Some(h));
        }
    }
    // Alt let go after the window lost the keyboard (Alt+Tab away and back), or with a pointer
    // press between, is no tap: the latch starts again
    bar(|b| {
        let fg = b.hwnd.is_some_and(crate::win32::is_foreground);
        if (b.foreground && !fg) || crate::win32::mouse_button_down() {
            b.alt = AltTap::default();
        }
        b.foreground = fg;
    });
    refocus();
    if let Some(Some(m)) = bar(|b| b.again.take()) {
        show(m);
    }
    // a menu opened by the keyboard: its first item that takes the focus
    if let Some(Some((m, tries))) = bar(|b| b.first_item) {
        let done = bar(|b| !b.open.contains(&m)).unwrap_or(true) || tries == 0 || focus_first(m);
        bar(|b| b.first_item = if done { None } else { Some((m, tries - 1)) });
    }
    looks();
    let Some((root, open, before, now, titles)) = bar(|b| {
        let now = pointer(b);
        let before = std::mem::replace(&mut b.cursor, now.map(|(x, y)| ((x * 64.0) as i32, (y * 64.0) as i32)));
        (b.root.clone(), b.open.clone(), before, now, titles(b))
    }) else {
        return;
    };
    // nothing open, or the pointer off the window: no row to rest on
    if open.is_empty() || now.is_none() {
        bar(|b| {
            b.resting = None;
            b.on_row = None;
        });
    }
    let Some(&top) = open.first() else { return };
    if let Some(p) = now {
        let moved = before != Some(((p.0 * 64.0) as i32, (p.1 * 64.0) as i32));
        if moved {
            // onto another title: its menu instead
            let rects: Vec<(f64, f64, f64, f64)> = titles.iter().filter_map(|t| rect(&t.cast().ok()?, &root)).collect();
            if rects.len() == titles.len() {
                if let Some(j) = hit(p.0, p.1, &rects) {
                    if j != top {
                        switch_by_pointer(j);
                        return;
                    }
                }
            }
            pointer_on_rows(p, &root, &open);
        }
    }
    // resting on a row long enough: the sub-menus of its menu's other rows close, and its own
    // opens
    let Some(Some((m, r))) = bar(|b| match b.resting {
        Some((m, r, since)) if since.elapsed() >= b.delay => {
            b.resting = None;
            Some((m, r))
        }
        _ => None,
    }) else {
        return;
    };
    let Some((stale, sub)) = bar(|b| {
        if !b.open.contains(&m) {
            return (Vec::new(), None);
        }
        let stale: Vec<usize> = b.open.iter().copied().filter(|&o| b.menus[o].shape.parent.is_some_and(|(p, pr)| p == m && pr != r)).collect();
        let sub = b.menus[m].shape.rows[r].sub.filter(|k| !b.open.contains(k));
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
        if let Some(el) = bar(|b| b.menus[m].els[r].clone()) {
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
    let Some(rows) = bar(|b| open.iter().rev().map(|&m| (m, b.menus[m].els.clone(), b.menus[m].list.clone())).collect::<Vec<_>>()) else { return };
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
    if bar(|b| std::mem::replace(&mut b.on_row, now)) == Some(now) {
        return;
    }
    // off the rows: the wait for a sub-menu ends
    if found.is_none() {
        bar(|b| b.resting = None);
    }
    if let Some((m, r, el)) = found {
        // with a sub-menu of this menu open, the focus stays in it until that closes (after
        // the delay, in [`tick`]), as in WinUI's menus
        if bar(|b| under(m, &b.open, &parents(&b.menus)).is_empty()) == Some(true) {
            let _ = el.Focus(FocusState::Pointer);
        }
        bar(|b| b.resting = Some((m, r, Instant::now())));
    }
}

/// Each open sub-menu lets the pointer through to the menu it is over, when that is one it opened
/// from (however far up), else to the menu it opened from: a click on a row further up is the
/// row's, not swallowed closing the sub-menus.
fn pass_through(over: Option<usize>) {
    let Some(changes) = bar(|b| {
        let parents = parents(&b.menus);
        let mut changes = Vec::new();
        for &k in &b.open {
            let Some(parent) = parents[k] else { continue };
            let mut up = parents[k];
            let mut target = parent;
            while let Some(a) = up {
                if Some(a) == over {
                    target = a;
                    break;
                }
                up = parents[a];
            }
            if b.through[k] != Some(target) {
                b.through[k] = Some(target);
                changes.push((b.menus[k].flyout.clone(), b.menus[target].list.clone()));
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

/// The titles' and the sub-menus' rows' states, as `MenuBarItem` and `MenuFlyoutSubItem` set them.
fn looks() {
    let Some((titles, open, shown, subs, sub_shown)) = bar(|b| {
        let subs: Vec<(usize, DropDownButton)> = b.menus.iter().enumerate().skip(MENUS.len()).map(|(k, m)| (k, m.owner.clone())).collect();
        (titles(b), b.open.clone(), b.shown.clone(), subs, b.sub_shown.clone())
    }) else {
        return;
    };
    for (i, t) in titles.iter().enumerate() {
        let Ok(button) = t.cast::<ButtonBase>() else { continue };
        let want = look(open.first() == Some(&i), button.IsPressed().unwrap_or(false), button.IsPointerOver().unwrap_or(false));
        if want != shown[i] {
            if let Ok(c) = t.cast::<Control>() {
                let _ = VisualStateManager::GoToState(&c, &HSTRING::from(want.0), true);
                let _ = VisualStateManager::GoToState(&c, &HSTRING::from(want.1), true);
            }
            bar(|b| b.shown[i] = want);
        }
    }
    for (k, row) in subs {
        let Ok(button) = row.cast::<ButtonBase>() else { continue };
        let Ok(c) = row.cast::<Control>() else { continue };
        let want = sub_look(c.IsEnabled().unwrap_or(true), open.contains(&k), button.IsPressed().unwrap_or(false), button.IsPointerOver().unwrap_or(false));
        // (not yet templated, the first time its menu opens: tried again next frame)
        if want != sub_shown[k] && VisualStateManager::GoToState(&c, &HSTRING::from(want), false).unwrap_or(false) {
            bar(|b| b.sub_shown[k] = want);
        }
    }
}

/// The focus on menu `m`'s first item that can take it; whether one did.
fn focus_first(m: usize) -> bool {
    let Some(rows) = bar(|b| b.menus[m].rows.clone()) else { return true };
    for (k, row) in rows.iter().enumerate() {
        if row.Focus(FocusState::Keyboard).unwrap_or(false) {
            dev(&format!("menu {m}: item {k} has the focus"));
            return true;
        }
    }
    false
}

/// dev: a line in the timing log (SPRAWL_TIMING), for the screenshot runs that drive the bar.
fn dev(what: &str) {
    crate::state::timing(&format!("menubar: {what}"), Instant::now());
}

/// dev: the pointer arriving on title `j` while a menu is open, as [`tick`] finds it (the
/// screenshot runs cannot move the real pointer).
pub(crate) fn dev_hover(j: usize) {
    if bar(|b| !b.open.is_empty()) == Some(true) && j < MENUS.len() {
        switch_by_pointer(j);
    }
}

/// dev: the pointer said to be on the middle of the element named `name` (a menu's row), for
/// [`tick`] to find there from now on (the screenshot runs cannot move the real pointer).
pub(crate) fn dev_point(name: &str) {
    let Some(root) = bar(|b| b.root.clone()) else { return };
    let Some(page) = root.cast::<FrameworkElement>().ok() else { return };
    let Ok(el) = find::<UIElement>(&page, name) else { return };
    if let Some((l, t, w, h)) = rect(&el, &root) {
        dev(&format!("pointer onto {name} at {:.0},{:.0}", l + w / 2.0, t + h / 2.0));
        bar(|b| b.fake = Some((l + w / 2.0, t + h / 2.0)));
    }
}

/// dev: what has the focus and which menus are open, in the timing log.
pub(crate) fn dev_report() {
    let Some((root, open)) = bar(|b| (b.root.clone(), b.open.clone())) else { return };
    let what = focused(&root).map_or("nothing".to_string(), |f| {
        let name = f.cast::<FrameworkElement>().and_then(|f| f.Name()).map(|n| n.to_string_lossy()).unwrap_or_default();
        let class = f.cast::<windows_core::IInspectable>().and_then(|i| i.GetRuntimeClassName()).map(|n| n.to_string_lossy()).unwrap_or_default();
        format!("{class} {name:?}")
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

/// Rectangle `r` cut to `to` (left, top, width, height; nothing left: zero wide or high).
pub(crate) fn clip(r: (f64, f64, f64, f64), to: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let (l, t) = (r.0.max(to.0), r.1.max(to.1));
    let (rr, b) = ((r.0 + r.2).min(to.0 + to.2), (r.1 + r.3).min(to.1 + to.3));
    (l, t, (rr - l).max(0.0), (b - t).max(0.0))
}

/// An element's rectangle on the page (left, top, width, height).
fn rect(el: &UIElement, root: &UIElement) -> Option<(f64, f64, f64, f64)> {
    let fe: FrameworkElement = el.cast().ok()?;
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
        assert_eq!(on_title(0, 3, Key::Home, &KEYS), Act::None);
    }

    use Kind::{Item, Sub, Toggle};

    #[test]
    fn up_and_down_go_round_a_menu_skipping_what_is_off() {
        let kinds = [Item, Item, Toggle, Item];
        let all = [true; 4];
        assert_eq!(menu_key(Key::Down, None, &kinds, &all, None), InMenu::Move(0), "nothing focused: the first");
        assert_eq!(menu_key(Key::Up, None, &kinds, &all, None), InMenu::Move(3), "nothing focused: the last");
        assert_eq!(menu_key(Key::Down, Some(1), &kinds, &all, None), InMenu::Move(2));
        assert_eq!(menu_key(Key::Down, Some(3), &kinds, &all, None), InMenu::Move(0), "round the end");
        assert_eq!(menu_key(Key::Up, Some(0), &kinds, &all, None), InMenu::Move(3), "round the start");
        assert_eq!(menu_key(Key::Home, Some(2), &kinds, &all, None), InMenu::Move(0));
        assert_eq!(menu_key(Key::End, Some(0), &kinds, &all, None), InMenu::Move(3));
        let some = [true, false, true, true];
        assert_eq!(menu_key(Key::Down, Some(0), &kinds, &some, None), InMenu::Move(2), "the one that is off is skipped");
        assert_eq!(menu_key(Key::Up, Some(2), &kinds, &some, None), InMenu::Move(0));
        assert_eq!(menu_key(Key::Down, Some(0), &[Item], &[true], None), InMenu::Move(0), "alone: stays");
        assert_eq!(menu_key(Key::Down, None, &[Item, Item], &[false, false], None), InMenu::Hold);
    }

    #[test]
    fn right_opens_a_sub_menu_and_left_and_esc_close_one() {
        let kinds = [Item, Sub, Item];
        let all = [true; 3];
        // in a sub-menu (not one of the bar's)
        assert_eq!(menu_key(Key::Right, Some(1), &kinds, &all, None), InMenu::OpenSub(1));
        assert_eq!(menu_key(Key::Right, Some(0), &kinds, &all, None), InMenu::Hold, "kept from the row: a dot would pick the next");
        assert_eq!(menu_key(Key::Left, Some(0), &kinds, &all, None), InMenu::Close);
        assert_eq!(menu_key(Key::Escape, Some(2), &kinds, &all, None), InMenu::Close);
        assert_eq!(menu_key(Key::Right, Some(1), &kinds, &[true, false, true], None), InMenu::Hold, "an item that is off opens nothing");
    }

    #[test]
    fn in_one_of_the_bars_menus_left_and_right_switch_menus_but_right_opens_a_sub_menu() {
        let kinds = [Item, Sub, Item];
        let all = [true; 3];
        assert_eq!(menu_key(Key::Right, Some(0), &kinds, &all, Some((0, 3))), InMenu::Bar(Act::Switch(1)));
        assert_eq!(menu_key(Key::Left, Some(0), &kinds, &all, Some((0, 3))), InMenu::Bar(Act::Switch(2)));
        assert_eq!(menu_key(Key::Right, Some(2), &kinds, &all, Some((2, 3))), InMenu::Bar(Act::Switch(0)));
        assert_eq!(menu_key(Key::Right, None, &kinds, &all, Some((1, 3))), InMenu::Bar(Act::Switch(2)));
        // Right on "Show >" opens the sub-menu instead; Left on it still moves on
        assert_eq!(menu_key(Key::Right, Some(1), &kinds, &all, Some((1, 3))), InMenu::OpenSub(1));
        assert_eq!(menu_key(Key::Left, Some(1), &kinds, &all, Some((1, 3))), InMenu::Bar(Act::Switch(0)));
        assert_eq!(menu_key(Key::Escape, Some(1), &kinds, &all, Some((1, 3))), InMenu::Close);
        assert_eq!(menu_key(Key::Letter('S'), Some(1), &kinds, &all, Some((1, 3))), InMenu::None);
        assert_eq!(menu_key(Key::Other, Some(1), &kinds, &all, Some((1, 3))), InMenu::None, "Enter and Space are the row's own");
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
    fn a_sub_menus_row_looks_open_whatever_the_pointer_does() {
        assert_eq!(sub_look(true, false, false, false), "SubNormal");
        assert_eq!(sub_look(true, false, false, true), "SubPointerOver");
        assert_eq!(sub_look(true, false, true, true), "SubPressed");
        assert_eq!(sub_look(true, true, false, false), "SubMenuOpened");
        assert_eq!(sub_look(true, true, true, true), "SubMenuOpened");
        assert_eq!(sub_look(false, true, true, true), "SubDisabled");
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

    fn menus() -> [Vec<Row>; 3] {
        let item = |n: &str, k: &str, m: &str| Row::Item { name: n.into(), text: n.into(), key: k.into(), mods: m.into() };
        let toggle = |n: &str, on: bool| Row::Toggle { name: n.into(), text: n.into(), checked: on, key: String::new(), mods: String::new() };
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
        assert_eq!(x.matches(r#"AreOpenCloseAnimationsEnabled="False" FlyoutPresenterStyle="{StaticResource SprawlBarMenu}""#).count(), 5);
        assert!(x.contains(r#"<Setter Property="Tag" Value="menu"/>"#));
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
        // drawn with WinUI's own resources
        for key in ["MenuBarItemMargin", "MenuBarItemButtonPadding", "MenuBarItemBackgroundSelected", "MenuFlyoutPresenterThemePadding", "MenuFlyoutItemThemePaddingNarrow", "MenuFlyoutItemPlaceholderThemeThickness", "MenuFlyoutSubItemBackgroundSubMenuOpened", "MenuFlyoutSeparatorThemePadding", "ToggleMenuFlyoutItemKeyboardAcceleratorTextForeground"] {
            assert!(x.contains(key), "{key}");
        }
    }

    #[test]
    fn the_menus_shape_knows_each_rows_menu_and_each_sub_menus_row() {
        let _ = xaml(menus());
        let shapes = SHAPE.with(|s| s.borrow().clone());
        assert_eq!(shapes.len(), 5);
        assert_eq!(shapes[1].rows.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![Kind::Item, Kind::Sub]);
        assert_eq!(shapes[1].rows[1].sub, Some(3));
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
