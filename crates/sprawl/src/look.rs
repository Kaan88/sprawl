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

/// The background of a selected row or tab (a list's line, the Settings sections). Chips and
/// the map's modes take the accent instead ([`Pick::bg`]).
pub(crate) fn selected() -> String {
    crate::tokens::refer("selected")
}

/// A selectable item's text at rest, as the styles set it.
fn rest_text() -> String {
    crate::tokens::refer("rest")
}

/// The Windows accent colour (Settings > Personalisation > Colours) the way WinUI's own accent
/// buttons draw it: `fill` behind `on` text, and `text` for accent-coloured words on the page.
/// Written "#RRGGBB" (six digits) on purpose: the light-mode rewrite in theme.rs turns only
/// "#AARRGGBB" values around, and these are already the ones for the theme in use.
pub(crate) struct Accent {
    pub fill: String,
    pub on: &'static str,
    pub text: String,
}

/// The accent for the theme in use, read once per theme (the theme can change while Sprawl runs).
pub(crate) fn accent() -> &'static Accent {
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

/// The accent for a light or dark page, from the palette (Windows' own blue without one):
/// dark pages fill with Light2 and write accent text in Light3, light pages Dark1 and Dark2.
fn accent_from(p: Option<[[u8; 3]; 7]>, light: bool) -> Accent {
    let p = p.unwrap_or([[0x99, 0xEB, 0xFF], [0x4C, 0xC2, 0xFF], [0x00, 0x91, 0xF8], [0x00, 0x78, 0xD4], [0x00, 0x67, 0xC0], [0x00, 0x3E, 0x92], [0x00, 0x1A, 0x68]]);
    let hex = |c: [u8; 3]| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
    if light {
        Accent { fill: hex(p[4]), on: "#FFFFFF", text: hex(p[5]) }
    } else {
        Accent { fill: hex(p[1]), on: "#000000", text: hex(p[0]) }
    }
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

/// The underline under the open page tab: the accent.
pub(crate) fn underline() -> &'static str {
    &accent().fill
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
            (Pick::Chip | Pick::Mode, true) => accent().fill.clone(),
            (Pick::Tab, true) => selected(),
            _ => self.rest(),
        }
    }

    /// Its text colour when it differs from the style's: on the accent fill.
    pub(crate) fn fg(self, on: bool) -> Option<String> {
        (on && matches!(self, Pick::Chip | Pick::Mode)).then(|| accent().on.to_string())
    }

    fn style(self) -> &'static str {
        match self {
            Pick::Chip => "SprawlChip",
            Pick::Page => "SprawlPage",
            _ => "SprawlPick",
        }
    }
}

/// The attributes for a `<Button>` drawn as a selectable item.
pub(crate) fn pick(kind: Pick, on: bool) -> String {
    if kind == Pick::Page {
        return format!(r##"Style="{{StaticResource SprawlPage}}" Background="Transparent" BorderBrush="{}""##, if on { underline() } else { "Transparent" });
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
    if kind == Pick::Page {
        // the underline is the template's line, in the tab's BorderBrush
        if let Ok(br) = load::<Brush>(&format!(r##"<SolidColorBrush {} Color="{}"/>"##, ui::NS, if on { underline() } else { "#00000000" })) {
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

/// How long a state takes to arrive: 140 ms, or none when Windows' "Animation effects" is off
/// (read when the styles are built, at start; the states still change, without the travel).
fn state_time() -> &'static str {
    if crate::win32::animations_on() { "0:0:0.14" } else { "0:0:0" }
}

/// One state's storyboard, shared by every button: the wash over the item, and the content's
/// lift (negative Y is up) and scale. Ease-out, ExponentialEase with exponent 6, as close as
/// WinUI gets to cubic-bezier(.2,.8,.2,1).
fn states(wash: f64, lift: f64, scale: f64) -> String {
    let d = state_time();
    let ease = r##"<DoubleAnimation.EasingFunction><ExponentialEase EasingMode="EaseOut" Exponent="6"/></DoubleAnimation.EasingFunction>"##;
    let one = |target: &str, prop: &str, to: f64| {
        format!(r##"<DoubleAnimation Storyboard.TargetName="{target}" Storyboard.TargetProperty="{prop}" To="{to}" Duration="{d}">{ease}</DoubleAnimation>"##)
    };
    let t = "(UIElement.RenderTransform).(CompositeTransform.";
    format!(
        "<Storyboard>{}{}{}{}</Storyboard>",
        one("Wash", "Opacity", wash),
        one("Content", &format!("{t}TranslateY)"), lift),
        one("Content", &format!("{t}ScaleX)"), scale),
        one("Content", &format!("{t}ScaleY)"), scale),
    )
}

/// The template both selectable styles share: the item's own background (set by `paint` or
/// `pick`: at rest or selected), a white wash over it for the pointer (pointer over: 10%,
/// pressed: 20%, with the content up 1 px, then down 1 px and at 0.98), and the system's focus rectangle, which shows for the keyboard
/// only. So selected-and-pointer-over is the selected background with the faint wash, and a
/// selected item the pointer has left is the selected background alone. With `line`: a 3 px
/// line under the content in the BorderBrush (the page tabs; they have no border of their own).
fn template_with(target: &str, arrow: bool, line: bool, grey_off: bool) -> String {
    let (cols, arrow) = if arrow {
        (
            r##"<Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>"##,
            r##"<FontIcon x:Name="Arrow" Grid.Column="1" Glyph="&#xE70D;" FontSize="{$fs8}" Margin="2,0,10,0" VerticalAlignment="Center" Foreground="{TemplateBinding Foreground}"/>"##,
        )
    } else {
        ("", "")
    };
    let (border, underline) = if line {
        ("", r##"<Border x:Name="Line" Grid.ColumnSpan="2" VerticalAlignment="Bottom" Height="3" Margin="12,0,12,0" CornerRadius="{$rc2}" Background="{TemplateBinding BorderBrush}" IsHitTestVisible="False"/>"##)
    } else {
        (r##"BorderBrush="{TemplateBinding BorderBrush}" BorderThickness="{TemplateBinding BorderThickness}""##, "")
    };
    // a filled button (the red one) turns grey when it can't be pressed, as WinUI's do; the
    // rest just fade
    let disabled = if grey_off {
        r##"<Setter Target="Root.Background" Value="{$hover}"/><Setter Target="Content.Foreground" Value="{$disabled}"/>"##
    } else {
        r##"<Setter Target="Content.Opacity" Value="0.4"/>"##
    };
    let (normal, over, pressed) = (states(0.0, 0.0, 1.0), states(0.10, -1.0, 1.0), states(0.20, 1.0, 0.98));
    format!(
        r##"<ControlTemplate TargetType="{target}">
  <Grid x:Name="Root" Background="{{TemplateBinding Background}}" CornerRadius="{{TemplateBinding CornerRadius}}" {border}>
    {cols}
    <Border x:Name="Wash" Grid.ColumnSpan="2" Background="{{$wash}}" Opacity="0" CornerRadius="{{TemplateBinding CornerRadius}}"/>
    <ContentPresenter x:Name="Content" RenderTransformOrigin="0.5,0.5" Content="{{TemplateBinding Content}}" ContentTemplate="{{TemplateBinding ContentTemplate}}" Padding="{{TemplateBinding Padding}}" HorizontalContentAlignment="{{TemplateBinding HorizontalContentAlignment}}" VerticalContentAlignment="{{TemplateBinding VerticalContentAlignment}}" Foreground="{{TemplateBinding Foreground}}" FontSize="{{TemplateBinding FontSize}}" FontFamily="{{TemplateBinding FontFamily}}" FontWeight="{{TemplateBinding FontWeight}}"><ContentPresenter.RenderTransform><CompositeTransform/></ContentPresenter.RenderTransform></ContentPresenter>
    {arrow}
    {underline}
    <VisualStateManager.VisualStateGroups>
      <VisualStateGroup x:Name="CommonStates">
        <VisualState x:Name="Normal"><VisualState.Storyboard>{normal}</VisualState.Storyboard></VisualState>
        <VisualState x:Name="PointerOver"><VisualState.Storyboard>{over}</VisualState.Storyboard></VisualState>
        <VisualState x:Name="Pressed"><VisualState.Storyboard>{pressed}</VisualState.Storyboard></VisualState>
        <VisualState x:Name="Disabled"><VisualState.Setters>{disabled}</VisualState.Setters></VisualState>
      </VisualStateGroup>
    </VisualStateManager.VisualStateGroups>
  </Grid>
</ControlTemplate>"##
    )
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
}

impl Geo {
    /// The selectable items as they were: small, no border.
    fn small() -> Self {
        Geo { padding: "10,3", corner: crate::tokens::rc(6.0), size: crate::tokens::fs(12.0), weight: "Normal", border: ("0", "Transparent".to_string()), background: "Transparent".to_string(), foreground: rest_text() }
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
        template_with(target, arrow, line, grey_off)
    )
}

/// A stock control's themed brush, replaced by one of the look's colours. WinUI draws the control
/// from these keys; a key of the same name in a dictionary merged after `XamlControlsResources`
/// wins over both of its pages, so one value is enough and no ThemeDictionaries is needed.
fn brush(key: &str, token: &str) -> String {
    format!(r#"<SolidColorBrush x:Key="{key}" Color="{}"/>"#, crate::tokens::refer(token))
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
    // a text box: the page's ground under it, a hairline that firms up under the pointer and
    // takes the accent with the focus; its placeholder is the dimmest text
    for (key, token) in [
        ("TextControlBackground", "ground"),
        ("TextControlBackgroundPointerOver", "ground"),
        ("TextControlBackgroundFocused", "ground"),
        ("TextControlBorderBrush", "line2"),
        ("TextControlBorderBrushPointerOver", "edge"),
        ("TextControlBorderBrushFocused", "accent"),
        ("TextControlForeground", "text1"),
        ("TextControlForegroundPointerOver", "text1"),
        ("TextControlForegroundFocused", "text1"),
        ("TextControlPlaceholderForeground", "text3"),
        ("TextControlPlaceholderForegroundPointerOver", "text3"),
        ("TextControlPlaceholderForegroundFocused", "text3"),
    ] {
        x += &brush(key, token);
    }
    // and it keeps a 1 px line whether or not it has the focus (WinUI draws 2 px under it)
    for key in ["TextControlBorderThemeThickness", "TextControlBorderThemeThicknessFocused"] {
        x += &thickness(key);
    }
    // a combo box: the field on the ground, its list on the popup's
    for (key, token) in [
        ("ComboBoxBackground", "ground"),
        ("ComboBoxBackgroundPointerOver", "row"),
        ("ComboBoxBackgroundPressed", "card"),
        ("ComboBoxBorderBrush", "line2"),
        ("ComboBoxBorderBrushPointerOver", "edge"),
        ("ComboBoxForeground", "text1"),
        ("ComboBoxDropDownBackground", "popup"),
        ("ComboBoxDropDownBorderBrush", "line1"),
    ] {
        x += &brush(key, token);
    }
    // flyouts, menus and tooltips: the popup's ground inside a hairline
    for (key, token) in [
        ("FlyoutPresenterBackground", "popup"),
        ("FlyoutBorderThemeBrush", "line1"),
        ("MenuFlyoutPresenterBackground", "popup"),
        ("MenuFlyoutPresenterBorderBrush", "line1"),
        ("MenuFlyoutItemBackgroundPointerOver", "hover"),
        ("MenuFlyoutItemBackgroundPressed", "pressed"),
        ("ToolTipBackground", "popup"),
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
    x
}

/// The styles, as a dictionary merged into the application's resources at start (so any
/// XAML loaded later, the Advisor's or Search's included, can name them). The stock controls'
/// own keys come first, so a text box, a menu or a check box matches the look; then the buttons,
/// which follow WinUI: a plain `<Button>` with no size of its own is the standard one;
/// `SprawlPrimary` is the one main action of a place, in the Windows accent; `SprawlDanger`
/// deletes for good; `SprawlSubtle` is a text action with no fill until the pointer is over it.
pub(crate) fn resources() -> String {
    format!(
        r##"<ResourceDictionary {}>{}{}{}{}{}{}{}
<Style x:Key="SprawlPrimary" TargetType="Button" BasedOn="{{StaticResource AccentButtonStyle}}"><Setter Property="FontWeight" Value="SemiBold"/></Style></ResourceDictionary>"##,
        ui::NS,
        stock(),
        style("SprawlPick", "Button", false),
        style("SprawlPickDrop", "DropDownButton", true),
        style_with("SprawlPage", "Button", false, true, false, Geo::small()),
        // chips: pills (the corner is clamped to half the height), a WinUI button's size with its
        // faint edge, filled with the accent when picked
        style_with("SprawlChip", "Button", false, false, false, Geo { corner: crate::tokens::get("rcpill"), ..Geo::button().with_border("1", crate::tokens::refer("line1")) }),
        style_with("SprawlSubtle", "Button", false, false, false, Geo { foreground: accent().text.clone(), ..Geo::button() }),
        style_with("SprawlDanger", "Button", false, false, true, Geo { background: danger(), foreground: "White".into(), weight: "SemiBold", ..Geo::button() }),
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
        for k in [Pick::Chip, Pick::Mode] {
            assert_eq!(k.bg(true), accent().fill);
            assert_eq!(k.fg(true), Some(accent().on.to_string()));
            assert_ne!(k.bg(false), accent().fill);
            assert_eq!(k.fg(false), None);
            assert!(pick(k, true).contains(&format!(r#"Foreground="{}""#, accent().on)));
        }
        // a selected row keeps its grey second line readable: no accent fill, no text colour
        assert_eq!(Pick::Tab.bg(true), selected());
        assert_eq!(Pick::Tab.fg(true), None);
        assert!(pick(Pick::Tab, true).contains("SprawlPick") && !pick(Pick::Tab, true).contains("Foreground"));
        assert!(pick(Pick::Chip, false).contains("SprawlChip"));
        assert!(pick_drop(Pick::Mode, false).contains("SprawlPickDrop"));
    }

    #[test]
    fn the_accent_follows_the_palette_and_skips_the_light_rewrite() {
        let p = [[1, 2, 3], [0x97, 0xC8, 0xCE], [5, 6, 7], [8, 9, 10], [0x37, 0x63, 0x72], [0x23, 0x44, 0x52], [0, 0, 0]];
        let dark = accent_from(Some(p), false);
        assert_eq!((dark.fill.as_str(), dark.on, dark.text.as_str()), ("#97C8CE", "#000000", "#010203"));
        let light = accent_from(Some(p), true);
        assert_eq!((light.fill.as_str(), light.on, light.text.as_str()), ("#376372", "#FFFFFF", "#234452"));
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
        // and the check box's tick is drawn on the accent
        assert!(expanded.contains(&format!(r##"<SolidColorBrush x:Key="CheckBoxCheckGlyphForegroundChecked" Color="{}"/>"##, crate::tokens::get("onaccent"))));
    }

    #[test]
    fn the_styles_define_every_state_once() {
        let r = resources();
        // the templated styles; SprawlPrimary builds on WinUI's own accent button
        for s in ["Normal", "PointerOver", "Pressed", "Disabled"] {
            assert_eq!(r.matches(&format!(r#"x:Name="{s}""#)).count(), 6, "{s}");
        }
        assert_eq!(r.matches("UseSystemFocusVisuals").count(), 6);
        for k in ["SprawlPick", "SprawlPickDrop", "SprawlPage", "SprawlChip", "SprawlSubtle", "SprawlDanger", "SprawlPrimary"] {
            assert_eq!(r.matches(&format!(r#"x:Key="{k}""#)).count(), 1, "{k}");
        }
        assert!(r.contains(r#"BasedOn="{StaticResource AccentButtonStyle}""#));
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
        assert!(pick(Pick::Page, true).contains(underline()) && !pick(Pick::Page, false).contains(underline()));
    }
}
