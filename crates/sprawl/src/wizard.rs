//! The first-run setup wizard and Settings: one overlay, both built from [`crate::prefs`]. The
//! wizard asks only what matters on the first day; Settings has everything, in sections, in
//! the same words. Both keep a draft of the values and apply it on Finish or Save, through the
//! same paths the menus use.

use std::collections::HashMap;

use windows_core::Interface;

use sprawl_advisor::config::{self, AiConfig, Detected, Provider};

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, RangeBase, Selector};
use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, Control, PasswordBox, TextBox, ToggleSwitch};
use crate::bindings::Microsoft::UI::Xaml::Media::VisualTreeHelper;
use crate::bindings::Microsoft::UI::Xaml::{FocusState, FrameworkElement, UIElement, Visibility};
use crate::prefs::{self, Kind, Pref, Sec, PREFS, WIZARD};
use crate::state::{find, keep, load, text, with, State};
use crate::ui;

pub const PAGES: usize = WIZARD.len();

/// Every control of the overlay that is wired once: buttons, then the sections.
pub const BUTTONS: [&str; 16] = ["WizBack", "WizNext", "WizSkip", "WizLoadModels", "WizUseDetected", "WizForgetKey", "WizRemoveAi", "PfSec0", "PfSec1", "PfSec2", "PfSec3", "PfSec4", "PfSec5", "PfSec6", "PfSec7", "PfSec8"];

const LIST_ROWS: usize = 4;

fn page_range(n: usize, page: usize) -> (std::ops::Range<usize>, usize) {
    let start = page.min(n.saturating_sub(1) / LIST_ROWS) * LIST_ROWS;
    (start..(start + LIST_ROWS).min(n), n.div_ceil(LIST_ROWS).max(1))
}

/// The overlay's state while it is open.
#[derive(Default)]
pub(crate) struct PrefUi {
    /// The Settings section shown.
    pub(crate) sec: usize,
    group: usize,
    list_page: usize,
    /// The values as chosen so far (key: value as text), and as they were when it opened.
    pub(crate) draft: HashMap<&'static str, String>,
    orig: HashMap<&'static str, String>,
    /// The settings on the page shown, read back into the draft before it changes.
    shown: Vec<&'static str>,
    /// The page's loaded content (its controls are found in it).
    pub(crate) fe: Option<FrameworkElement>,
    /// The fixed drives offered for Search, in the order shown.
    drives: Vec<(usize, String)>,
    focus_name: Option<String>,
    ai_orig: AiConfig,
    ai_load_error: Option<String>,
    ai_custom: String,
    ai_customs: Vec<config::CustomAi>,
    ai_efforts: std::collections::BTreeMap<String, String>,
    ai_edited: bool,
    ai_detected: Option<AiConfig>,
    ai_forget_key: Option<config::CredentialTarget>,
    /// "Clear history" was pressed once: the next press clears.
    clear_armed: bool,
    /// What the last action said, shown at the top of the page.
    pub(crate) note: Option<String>,
    /// The Theme page's picks (themepage.rs).
    pub(crate) theme: crate::themepage::ThemeUi,
    /// The focus the overlay gives its page once it is on screen (whether it was opened from
    /// the keyboard), and how many frames are left to try: a control cannot take the focus
    /// before it is loaded and laid out, and that is not on the frame the overlay opens.
    focus_in: Option<(FocusState, u8)>,
    /// The focus it was opened with, for when the window comes to the front again.
    pub(crate) open_state: FocusState,
    /// What had the keyboard's focus before it opened, and in which state: where it goes back.
    focus_back: Option<(UIElement, FocusState)>,
    /// The frames left to give that focus back (the overlay may still be going).
    back_tries: u8,
    /// The window was in front at the last frame (a focus given while it was behind does not
    /// hold, so a focus still outside the overlay is tried again when it comes to the front).
    front: bool,
    /// The registry backups, read once when the Cleaning section opens (and after a put back);
    /// None until then, so a render never reads the folder.
    registry_backups: Option<Vec<sprawl_advisor::regclean::BackupInfo>>,
}

fn labelled_row(name: &str, title: &str, field: &str) -> String {
    format!(r##"<Grid ColumnSpacing="12" Margin="0,2"><Grid.ColumnDefinitions><ColumnDefinition Width="205"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><TextBlock x:Name="{name}_Label" Text="{}" FontSize="{{$fs13}}" TextWrapping="Wrap" VerticalAlignment="Center"/><Border Grid.Column="1">{field}</Border></Grid>"##, ui::esc(title))
}

fn label(t: &str) -> String {
    format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$label}}" Margin="0,8,0,2"/>"##, ui::esc(t))
}

fn help(t: &str) -> String {
    if t.is_empty() {
        return String::new();
    }
    ui::note_text(r#"Margin="0,0,0,6""#, t)
}

fn combo(name: &str, items: &[&str], pick: usize) -> String {
    let items: String = items.iter().map(|i| format!(r##"<ComboBoxItem Content="{}"/>"##, ui::esc(i))).collect();
    format!(r##"<ComboBox x:Name="{name}" AutomationProperties.LabeledBy="{{Binding ElementName={name}_Label}}" HorizontalAlignment="Stretch" MaxDropDownHeight="300" SelectedIndex="{pick}">{items}</ComboBox>"##)
}

fn page_controls(prefix: &str, n: usize, range: std::ops::Range<usize>, pages: usize, buttons: &mut Vec<String>) -> String {
    if pages == 1 { return String::new(); }
    let page = range.start / LIST_ROWS;
    let mut nav = String::new();
    for (step, title, enabled) in [("Prev", "Previous", page > 0), ("Next", "Next", page + 1 < pages)] {
        let name = format!("PfB_page{step}{prefix}");
        if enabled { buttons.push(name.clone()); }
        nav += &format!(r##"<Button x:Name="{name}" IsEnabled="{enabled}" Content="{title}"/>"##);
    }
    format!(r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}–{} of {n}" VerticalAlignment="Center"/>{nav}</StackPanel>"##, range.start + 1, range.end)
}

fn button(name: &str, t: &str) -> String {
    format!(r##"<Button x:Name="{name}" Content="{}"/>"##, ui::esc(t))
}

/// Check a newly picked Program folder the way a scan opens one: a plain local folder, held
/// while checked, returned in its long, canonical spelling. Existing entries are never read
/// again here, so a disconnected drive keeps the spelling it was saved with.
fn checked_program_folder(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if !sprawl_advisor::program_folders::storable(path) {
        return Err("A folder name with ';' can't be stored here: semicolons separate the folders. Rename it or choose another folder.".into());
    }
    if path.parent().is_none() {
        return Err("A drive itself can't be a program folder. Choose a folder on it.".into());
    }
    let held = sprawl_core::local::directory(path).map_err(|e| format!("That folder can't be used ({e}). Choose a plain local folder."))?;
    let path = held.path.clone();
    if path.parent().is_none() {
        return Err("A drive itself can't be a program folder. Choose a folder on it.".into());
    }
    if let Some(home) = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from) {
        let home = sprawl_core::canonical_root(&home);
        if crate::cleanup::key(&path).trim_end_matches('\\') == crate::cleanup::key(&home).trim_end_matches('\\') {
            return Err("Your user profile folder itself can't be a program folder. Choose a folder inside it.".into());
        }
    }
    sprawl_advisor::program_folders::validate_library(&path)?;
    Ok(path)
}

fn page_title(glyph: &str, title: &str, sub: &str) -> String {
    format!(
        r##"<StackPanel {} Orientation="Horizontal" Spacing="12" Margin="0,0,0,4"><Border Width="40" Height="40" CornerRadius="{{$rc10}}" Background="{{$selected}}"><FontIcon Glyph="{glyph}" FontSize="{{$fs18}}" Foreground="{{$seltext}}"/></Border><StackPanel VerticalAlignment="Center"><TextBlock Text="{}" FontSize="{{$fs20}}" FontWeight="SemiBold"/><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$label}}" TextWrapping="Wrap"/></StackPanel></StackPanel>"##,
        ui::NS,
        ui::esc(title),
        ui::esc(sub)
    )
}

fn bullet(glyph: &str, title: &str, t: &str) -> String {
    format!(
        r##"<Grid ColumnSpacing="12" Margin="0,6,0,0"><Grid.ColumnDefinitions><ColumnDefinition Width="28"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><FontIcon Glyph="{glyph}" FontSize="{{$fs16}}" Foreground="{{$link}}" VerticalAlignment="Top" Margin="0,2,0,0"/><StackPanel Grid.Column="1"><TextBlock Text="{}" FontWeight="SemiBold" FontSize="{{$fs13}}"/><TextBlock Text="{}" TextWrapping="Wrap" FontSize="{{$fs12}}" Foreground="{{$label}}"/></StackPanel></Grid>"##,
        ui::esc(title),
        ui::esc(t)
    )
}

/// The overlay, placed over the whole window. The AI provider's controls are fixed (they have
/// their own events); everything else is built per page.
pub fn xaml() -> String {
    let providers: Vec<&str> = Provider::ALL.iter().map(|p| p.label()).collect();
    let ai_privacy = ui::note_with("", "AI is optional.", "The provider receives file and folder paths and may read small text files. It cannot approve a cleanup. See Logs and integrations for the request log.");
    let ai = format!(
        r##"<StackPanel x:Name="WizAi" Spacing="2" Visibility="Collapsed">
        <Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBlock x:Name="WizDetected" Text="" FontSize="{{$fs12}}" TextWrapping="Wrap" Foreground="{{$text2}}" VerticalAlignment="Center"/><Button x:Name="WizUseDetected" Grid.Column="1" Content="Use detected" Visibility="Collapsed"/></Grid>
        {}
        <Border x:Name="WizSavedHost"/>
        <TextBlock x:Name="WizCodexRisk" Visibility="Collapsed" TextWrapping="Wrap" MaxWidth="520" HorizontalAlignment="Left" FontSize="{{$fs12}}" Foreground="{{$warn}}" Margin="0,4,0,2" Text="{}"/>
        <StackPanel x:Name="WizUrlRow">{url}{nickname}</StackPanel>
        <StackPanel x:Name="WizKeyRow">{key}</StackPanel>
        <StackPanel x:Name="WizModelRow">{model}<TextBlock x:Name="WizModelNote" FontSize="{{$fs12}}" Foreground="{{$text3}}" TextWrapping="Wrap" Visibility="Collapsed"/></StackPanel>
        <Border x:Name="WizEffortHost"/>
        {ai_privacy}
        </StackPanel>"##,
        labelled_row("WizProvider", "Provider", &combo("WizProvider", &providers, 0)),
        crate::ui::esc(sprawl_advisor::llm::codex::RISK),
        url = labelled_row("WizUrl", "Server address", &{
            let names = sprawl_advisor::llm::models::preset_names();
            let preset = crate::models::combo("WizUrlPreset", &names, &names[0], 136).replacen("<ComboBox ", r##"<ComboBox Grid.Column="1" AutomationProperties.Name="Server preset" "##, 1);
            format!(r##"<Grid ColumnSpacing="6"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBox x:Name="WizUrl" AutomationProperties.LabeledBy="{{Binding ElementName=WizUrl_Label}}" PlaceholderText="https://api.openai.com/v1"/>{preset}</Grid>"##)
        }),
        nickname = labelled_row("WizAiName", "Saved provider name", r##"<Grid ColumnSpacing="6"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBox x:Name="WizAiName" AutomationProperties.LabeledBy="{Binding ElementName=WizAiName_Label}" PlaceholderText="Optional" MaxLength="40"/><Button x:Name="WizRemoveAi" Grid.Column="1" Content="Remove" ToolTipService.ToolTip="Remove this name from Saved providers on Save. This leaves its server key and current connection alone."/></Grid>"##),
        key = labelled_row("WizKey", "API key", r##"<Grid ColumnSpacing="6"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><PasswordBox x:Name="WizKey" AutomationProperties.LabeledBy="{Binding ElementName=WizKey_Label}" PlaceholderText="Paste your API key" ToolTipService.ToolTip="Kept in Windows Credential Manager for this server only."/><Button x:Name="WizForgetKey" Grid.Column="1" Content="Forget key"/></Grid>"##),
        model = labelled_row("WizModel", "Model", r##"<Grid ColumnSpacing="6"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><Border x:Name="WizModelHost"/><TextBox x:Name="WizModel" AutomationProperties.LabeledBy="{Binding ElementName=WizModel_Label}" PlaceholderText="Model name"/><Button x:Name="WizLoadModels" Grid.Column="1" Content="Load models" VerticalAlignment="Top" ToolTipService.ToolTip="Ask the selected provider for its list of models, using the typed or saved key."/></Grid>"##),
    );
    let nav: String = Sec::ALL
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                r##"<Button x:Name="PfSec{i}" {} HorizontalAlignment="Stretch" HorizontalContentAlignment="Left" Padding="10,6" CornerRadius="{{$rc6}}"><StackPanel Orientation="Horizontal" Spacing="10"><FontIcon Glyph="{}" FontSize="{{$fs14}}"/><TextBlock Text="{}" FontSize="{{$fs13}}"/></StackPanel></Button>"##,
                crate::look::pick(crate::look::Pick::Tab, false),
                s.glyph(),
                ui::esc(s.title())
            )
        })
        .collect();
    // the card's soft shadow, in a cell of its own with the card (the card's size is set by name)
    let shadow = crate::look::shadow(true);
    let lit = crate::look::top_light("hlcard", &crate::tokens::rc(14.0), false);
    format!(
        r##"
  <Grid x:Name="Wizard" Grid.RowSpan="8" Visibility="Collapsed" Background="{{$scrim}}" TabFocusNavigation="Cycle">
    <Grid HorizontalAlignment="Center" VerticalAlignment="Center">{shadow}<Border x:Name="WizCard" Width="640" Height="610" CornerRadius="{{$rc14}}" BorderBrush="{{$edge}}" BorderThickness="1" Background="{{$popup}}" Padding="28,22,28,20">
      <Grid RowSpacing="14">
        <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
        <Grid>
          <TextBlock x:Name="WizStep" Text="Step 1" FontSize="{{$fs12}}" Foreground="{{$text3}}"/>
          <TextBlock x:Name="WizDots" Text="" FontSize="{{$fs12}}" Foreground="{{$link}}" HorizontalAlignment="Right"/>
        </Grid>
        <Grid Grid.Row="1" ColumnSpacing="18">
          <Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
          <StackPanel x:Name="PfNav" Width="200" Spacing="2" Visibility="Collapsed">{nav}</StackPanel>
          <StackPanel Grid.Column="1" Spacing="4"><Border x:Name="PfTop"/><Border x:Name="PfGroupHost"/>{ai}<Border x:Name="PfHost"/></StackPanel>
        </Grid>
        <Grid Grid.Row="2">
          <Button x:Name="WizSkip" Background="Transparent" BorderThickness="0" Foreground="{{$label}}"><TextBlock x:Name="WizSkipText" Text="Skip setup"/></Button>
          <StackPanel Orientation="Horizontal" Spacing="8" HorizontalAlignment="Right">
            <Button x:Name="WizBack" Content="Back" MinWidth="90"/>
            <Button x:Name="WizNext" MinWidth="110" Style="{{StaticResource SprawlPrimary}}"><TextBlock x:Name="WizNextText" Text="Next"/></Button>
          </StackPanel>
        </Grid>
      </Grid>
    </Border>{lit}</Grid>
  </Grid>"##
    )
}

/// The card's size: 940x680 for Settings and 640x610 for the wizard, but never more than the
/// window less a margin each side (0 = the window isn't laid out yet: the usual size).
pub(crate) fn card_size(settings: bool, win_w: f64, win_h: f64) -> (f64, f64) {
    const MARGIN: f64 = 24.0;
    let (w, h) = if settings { (940.0, 680.0) } else { (640.0, 610.0) };
    let cap = |want: f64, win: f64| if win > 0.0 { want.min((win - 2.0 * MARGIN).max(240.0)) } else { want };
    (cap(w, win_w), cap(h, win_h))
}

fn shown(s: &State, name: &str, on: bool) {
    if let Ok(e) = find::<UIElement>(&s.root, name) {
        let _ = e.SetVisibility(if on { Visibility::Visible } else { Visibility::Collapsed });
    }
}

fn select(s: &State, name: &str, i: usize) {
    if let Ok(b) = find::<Selector>(&s.root, name) {
        let _ = b.SetSelectedIndex(i as i32);
    }
}

fn selected(s: &State, name: &str) -> usize {
    find::<Selector>(&s.root, name).and_then(|b| b.SelectedIndex()).unwrap_or(0).max(0) as usize
}

fn checked(s: &State, item: &str) -> bool {
    crate::menubar::checked(&s.root, item).unwrap_or(false)
}

fn field(s: &State, name: &str) -> String {
    find::<TextBox>(&s.root, name).and_then(|t| t.Text()).map(|h| h.to_string_lossy()).unwrap_or_default()
}

/// The API key typed in the wizard and not saved yet (empty: none typed).
fn typed_key(s: &State) -> String {
    find::<PasswordBox>(&s.root, "WizKey").and_then(|k| k.Password()).map(|h| h.to_string_lossy()).unwrap_or_default()
}

fn set_field(s: &State, name: &str, v: &str) {
    if let Ok(t) = find::<TextBox>(&s.root, name) {
        let _ = t.SetText(&v.into());
    }
}

fn on(v: &str) -> &'static str {
    if v == "1" {
        "True"
    } else {
        "False"
    }
}

fn choice_number(s: &str, min: u64, max: u64) -> Option<u64> {
    let v = s.trim();
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) { return None; }
    v.parse().ok().filter(|n| (min..=max).contains(n))
}

fn switch_draft(draft: &HashMap<&'static str, String>, key: &str) -> bool {
    draft.get(key).is_some_and(|v| v == "1")
}

fn request_allowed(draft_offline: bool, saved_offline: bool) -> bool {
    !draft_offline && !saved_offline
}

fn ai_needs_save(edited: bool, old_web: bool, web: bool, secret: &str) -> bool {
    edited || old_web != web || !secret.trim().is_empty()
}

fn uninstaller_path(value: &str) -> Result<Option<std::path::PathBuf>, String> {
    let v = value.trim().trim_matches('"');
    if v.is_empty() { return Ok(None); }
    let path = std::path::PathBuf::from(v);
    if path.is_absolute() && path.extension().is_some_and(|s| s.eq_ignore_ascii_case("exe")) && path.is_file() {
        Ok(Some(path))
    } else {
        Err("Choose an existing .exe file or leave Uninstaller empty.".into())
    }
}

fn map_animation_choice(saved: Option<bool>) -> usize {
    match saved { None => 0, Some(true) => 1, Some(false) => 2 }
}

fn reset_draft(draft: &mut HashMap<&'static str, String>, sec: Option<Sec>, elevated: bool) {
    for p in PREFS.iter().filter(|p| sec.is_none_or(|s| s == p.sec) && (elevated || !crate::admin::is_switch(p.key))) {
        if let Some(v) = p.default_value() {
            draft.insert(p.key, v);
        } else if matches!(p.key, "searchDrivesOn" | "searchColumns") {
            draft.insert(p.key, String::new());
        }
    }
}

/// The focus the overlay gives its page: the keyboard's with a ring when it was opened from the
/// keyboard, the pointer's (no ring) when a click opened it.
fn focus_state(keyboard: bool) -> FocusState {
    if keyboard {
        FocusState::Keyboard
    } else {
        FocusState::Pointer
    }
}

fn state_name(state: FocusState) -> &'static str {
    match state.0 {
        1 => "pointer",
        2 => "keyboard",
        3 => "programmatic",
        _ => "unfocused",
    }
}

/// How a timing line names an element: its class and x:Name.
fn focus_label(el: &UIElement) -> String {
    let name = el.cast::<FrameworkElement>().and_then(|f| f.Name()).map(|n| n.to_string_lossy()).unwrap_or_default();
    let class = el.cast::<windows_core::IInspectable>().and_then(|i| i.GetRuntimeClassName()).map(|c| c.to_string_lossy()).unwrap_or_default();
    let class = class.rsplit('.').next().unwrap_or("").to_string();
    format!("{class} {name:?}")
}

/// Whether `el` is a control on screen that could take the focus now: enabled, visible and laid
/// out. (A hidden or not yet laid out control's `Focus` does nothing. Whether a Tab would reach
/// it is WinUI's tab stop, which only `Focus` itself can answer.)
fn laid_out_control(el: &UIElement) -> bool {
    let enabled = el.cast::<Control>().is_ok_and(|c| c.IsEnabled().unwrap_or(true));
    let sized = el.cast::<FrameworkElement>().is_ok_and(|f| f.ActualWidth().unwrap_or(0.0) > 0.0 && f.ActualHeight().unwrap_or(0.0) > 0.0);
    enabled && sized && el.Visibility().is_ok_and(|v| v == Visibility::Visible)
}

/// Every control in `el`'s visual tree, in the order a Tab would meet them (the panel itself
/// excluded). The focus is given to the first that takes it, so Settings' first box or button
/// does when the section list's entry cannot.
fn controls_in(el: &UIElement, out: &mut Vec<UIElement>) {
    if out.len() >= 400 {
        return;
    }
    if el.cast::<Control>().is_ok() && laid_out_control(el) {
        out.push(el.clone());
    }
    for i in 0..VisualTreeHelper::GetChildrenCount(el).unwrap_or(0) {
        if let Ok(child) = VisualTreeHelper::GetChild(el, i).and_then(|c| c.cast::<UIElement>()) {
            controls_in(&child, out);
        }
    }
}

impl State {
    /// The overlay (the setup wizard or Settings) is on screen.
    pub(crate) fn wizard_shown(&self) -> bool {
        find::<UIElement>(&self.root, "Wizard").and_then(|w| w.Visibility()).is_ok_and(|v| v == Visibility::Visible)
    }

    /// Whether `el` is the overlay, or inside it.
    fn in_overlay(&self, el: &UIElement) -> bool {
        let Ok(overlay) = find::<UIElement>(&self.root, "Wizard") else { return false };
        let mut now = el.clone();
        for _ in 0..64 {
            if now == overlay {
                return true;
            }
            let Ok(parent) = VisualTreeHelper::GetParent(&now).and_then(|p| p.cast::<UIElement>()) else { return false };
            now = parent;
        }
        false
    }

    /// The controls the overlay may give the focus to when it opens, in order: the selected
    /// section's entry in the list (Settings), then the controls of the open section (the
    /// wizard's pages have no list), then the wizard's own button. Empty until it is laid out.
    fn wizard_focus_targets(&self) -> Vec<UIElement> {
        let mut out: Vec<UIElement> = Vec::new();
        if let Some(name) = self.prefs.focus_name.as_deref() {
            if let Some(el) = self.prefs.fe.as_ref().and_then(|f| find::<UIElement>(f, name).ok()).or_else(|| find::<UIElement>(&self.root, name).ok()) {
                out.push(el);
            }
        }
        if self.wizard_all {
            let sec = self.prefs.sec.min(Sec::ALL.len().saturating_sub(1));
            if let Ok(entry) = find::<UIElement>(&self.root, &format!("PfSec{sec}")) {
                if entry.Visibility().is_ok_and(|v| v == Visibility::Visible) {
                    out.push(entry);
                }
            }
        }
        if let Some(page) = self.prefs.fe.as_ref().and_then(|f| f.cast::<UIElement>().ok()) {
            controls_in(&page, &mut out);
        }
        for name in ["WizNext", "WizSkip", "WizBack"] {
            if let Ok(b) = find::<UIElement>(&self.root, name) {
                if laid_out_control(&b) {
                    out.push(b);
                }
            }
        }
        out
    }

    /// The overlay opened (from the keyboard, or from a click): note what had the focus, and
    /// give it the page once the page is on screen. A click leaves no focus ring.
    pub(crate) fn wizard_focus_arm(&mut self, keyboard: bool) {
        let state = focus_state(keyboard);
        if self.prefs.focus_back.is_none() {
            // from a menu, the focus is one of its rows: the element the menu took it from is
            // the one to return to (menu.rs keeps it)
            let page = self.root.cast::<UIElement>().ok();
            let before = crate::menu::outside().or_else(|| page.as_ref().and_then(crate::menu::focused));
            if let Some(el) = before {
                let had = el.FocusState().unwrap_or(FocusState::Unfocused);
                let had = if had == FocusState::Unfocused { FocusState::Programmatic } else { had };
                self.prefs.focus_back = Some((el, had));
            }
        }
        self.prefs.open_state = state;
        self.prefs.focus_in = Some((state, 12));
        self.prefs.back_tries = 0;
    }

    pub(crate) fn wizard_focus_control(&mut self, name: &str) {
        let state = if crate::menu::key_recent() { FocusState::Keyboard } else { self.prefs.open_state };
        self.prefs.focus_name = Some(name.to_string());
        self.prefs.focus_in = Some((state, 12));
    }

    pub(crate) fn wizard_before_restyle(&mut self) {
        let focused = self.root.cast::<UIElement>().ok().and_then(|r| crate::menu::focused(&r));
        if let Some(f) = focused.filter(|f| self.in_overlay(f)) {
            if let Ok(name) = f.cast::<FrameworkElement>().and_then(|f| f.Name()) {
                let name = name.to_string_lossy();
                if !name.is_empty() { self.wizard_focus_control(&name); }
            }
        }
        // The dynamic model dropdown is not in the window's XAML namescope. Carry its value
        // rather than its index: the service's list may have changed in the meantime.
        let model = self.wizard_model();
        set_field(self, "WizModel", &model);
        self.prefs_harvest();
    }

    /// Each frame while the overlay is open: the focus goes into it once it is laid out. The
    /// window in front, a focus still outside it is tried again (a focus given while the window
    /// was behind does not hold).
    pub(crate) fn wizard_frame(&mut self) {
        if !self.wizard_shown() {
            self.prefs.focus_in = None;
            self.prefs.front = false;
            if self.prefs.back_tries > 0 {
                self.wizard_focus_back();
            }
            return;
        }
        let front = crate::win32::is_foreground(self.hwnd);
        let came_front = front && !self.prefs.front;
        self.prefs.front = front;
        // a question is over the overlay: its button is the one to have the focus
        if self.dialogs.open() {
            return;
        }
        if came_front && self.prefs.focus_in.is_none() {
            let inside = self.root.cast::<UIElement>().ok().and_then(|r| crate::menu::focused(&r)).is_some_and(|f| self.in_overlay(&f));
            if !inside {
                self.prefs.focus_in = Some((self.prefs.open_state, 12));
            }
        }
        let Some((state, tries)) = self.prefs.focus_in else { return };
        let took = self.wizard_focus_targets().into_iter().find(|t| t.Focus(state).unwrap_or(false));
        if let Some(target) = took {
            self.prefs.focus_in = None;
            self.prefs.focus_name = None;
            let el = target.FocusState().unwrap_or(state);
            crate::state::timing(&format!("{} focus: {} ({})", if self.wizard_all { "Settings" } else { "Setup" }, focus_label(&target), state_name(el)), std::time::Instant::now());
        } else if tries > 1 {
            self.prefs.focus_in = Some((state, tries - 1));
        } else {
            self.prefs.focus_in = None;
            crate::state::timing(&format!("{} focus: none (not laid out)", if self.wizard_all { "Settings" } else { "Setup" }), std::time::Instant::now());
        }
    }

    /// The overlay closed: the focus goes back where it was before it opened, or to the tree
    /// view when that element is gone (or there was none: a click on the tree leaves none).
    fn wizard_focus_back(&mut self) {
        if self.wizard_shown() {
            self.prefs.back_tries = 0;
            return;
        }
        // a question that opened as the overlay closed is the one to have the focus
        if self.dialogs.open() {
            return;
        }
        let Some((el, state)) = self.prefs.focus_back.clone() else {
            self.prefs.back_tries = 0;
            self.focus_tree();
            return;
        };
        // the page may have been made again (a theme change while Settings was open): the same
        // control by its x:Name is the one to go back to
        let same_name = el.cast::<FrameworkElement>().ok().and_then(|f| f.Name().ok()).map(|n| n.to_string_lossy()).unwrap_or_default();
        let by_name = (!same_name.is_empty()).then(|| find::<UIElement>(&self.root, &same_name).ok()).flatten();
        let back = [Some(el.clone()), by_name].into_iter().flatten().find(|e| e.XamlRoot().is_ok() && e.Focus(state).unwrap_or(false));
        if let Some(back) = back {
            self.prefs.focus_back = None;
            self.prefs.back_tries = 0;
            crate::state::timing(&format!("focus back to {}", focus_label(&back)), std::time::Instant::now());
            return;
        }
        self.prefs.back_tries -= 1;
        if self.prefs.back_tries == 0 {
            self.prefs.focus_back = None;
            self.focus_tree();
            crate::state::timing("focus back on the tree", std::time::Instant::now());
        }
    }

    fn focus_tree(&self) {
        if let Ok(tree) = find::<UIElement>(&self.root, "TreeScroll") {
            let _ = tree.Focus(FocusState::Programmatic);
        }
    }

    /// The overlay closed by Cancel, Esc or its close button: the focus goes back (at the next
    /// frames, once it is gone), and the theme tried on its Theme page goes back too.
    fn wizard_hide(&mut self) {
        self.wizard_discard_secret();
        self.prefs.ai_forget_key = None;
        self.prefs.focus_name = None;
        shown(self, "Wizard", false);
        self.prefs.fe = None;
        self.prefs.focus_in = None;
        self.prefs.back_tries = 3;
    }

    /// Esc while the overlay is open closes it, as Cancel does. Not when a menu, a dropdown or a
    /// question is open over it (Esc closes that: main.rs gives the key to the question first).
    pub(crate) fn wizard_key(&mut self, key: i32) -> bool {
        if key != 27 || !self.wizard_shown() || self.dialogs.open() || crate::menu::any_open() || self.popup_open() {
            return false;
        }
        self.wizard_cancel();
        true
    }

    /// A dropdown list or a flyout is open (one of WinUI's popups, not the overlay itself).
    fn popup_open(&self) -> bool {
        self.root
            .XamlRoot()
            .ok()
            .and_then(|x| VisualTreeHelper::GetOpenPopupsForXamlRoot(&x).ok())
            .is_some_and(|popups| popups.Size().unwrap_or(0) > 0)
    }

    fn wizard_cancel(&mut self) {
        if self.wizard_all {
            self.theme_cancel();
        } else {
            // skip setup: don't ask again
            self.settings.set_bool("wizard_done", true);
            self.persist_settings();
        }
        self.wizard_hide();
    }

    /// The setup wizard, from its first page.
    pub(crate) fn wizard_open(&mut self) {
        // the page saw a key just now: it was opened from the keyboard, and shows the focus ring
        let keyboard = crate::menu::key_recent();
        self.wizard_all = false;
        self.prefs_begin();
        self.wizard_page = 0;
        self.wizard_show();
        shown(self, "Wizard", true);
        self.wizard_focus_arm(keyboard);
    }

    /// Options > Settings, at its first section.
    pub(crate) fn settings_open(&mut self) {
        self.settings_open_at("general");
    }

    /// Settings, at one section (`"ai"`, `"history"`...).
    pub(crate) fn settings_open_at(&mut self, section: &str) {
        self.settings_open_group(section, 0);
    }

    pub(crate) fn settings_open_group(&mut self, section: &str, group: usize) {
        // the page saw a key just now: opened from the keyboard (Ctrl+Shift+S), it shows the
        // focus ring; a click opens it without one
        let keyboard = crate::menu::key_recent();
        let sec = Sec::by_id(section).and_then(|s| Sec::ALL.iter().position(|x| *x == s)).unwrap_or(0);
        // already open (the Advisor's buttons open it at a section): its unsaved changes stay,
        // and only the section moves
        if self.wizard_shown() {
            self.prefs_harvest();
            self.prefs.sec = sec;
            self.prefs.group = group;
            self.prefs.list_page = 0;
            self.wizard_show();
            self.wizard_focus_arm(keyboard);
            return;
        }
        self.wizard_all = true;
        self.prefs_begin();
        self.prefs.sec = sec;
        self.prefs.group = group;
        self.prefs.list_page = 0;
        self.wizard_show();
        shown(self, "Wizard", true);
        self.wizard_focus_arm(keyboard);
    }

    /// Fill the draft from what is in use now, and the AI provider's controls.
    fn prefs_begin(&mut self) {
        self.wizard_discard_secret();
        self.prefs.ai_forget_key = None;
        let mut draft = HashMap::new();
        for p in PREFS.iter() {
            if let Some(v) = self.pref_value(p) {
                draft.insert(p.key, v);
            }
        }
        self.advisor.memory_ready();
        draft.insert("communityFetch", if self.advisor.memory.community_off { "0" } else { "1" }.to_string());
        self.prefs.orig = draft.clone();
        self.prefs.draft = draft;
        self.prefs.note = None;
        self.prefs.clear_armed = false;
        self.prefs.registry_backups = None;
        self.theme_begin();
        // AI: what was saved (Off at first); what is installed is only offered, one click away
        self.prefs.ai_detected = None;
        let loaded = AiConfig::load_checked();
        self.prefs.ai_load_error = loaded.as_ref().err().map(ToString::to_string);
        let ai = loaded.unwrap_or_default();
        let web = if ai.web_lookup { "1" } else { "0" }.to_string();
        self.prefs.orig.insert("aiWebLookup", web.clone());
        self.prefs.draft.insert("aiWebLookup", web);
        self.prefs.ai_orig = ai.clone();
        self.prefs.ai_custom = ai.custom.clone();
        self.prefs.ai_customs = ai.customs.clone();
        self.prefs.ai_efforts = ai.efforts.clone();
        let pi = Provider::ALL.iter().position(|&p| p == ai.provider).unwrap_or(0);
        select(self, "WizProvider", pi);
        set_field(self, "WizUrl", &ai.base_url);
        set_field(self, "WizModel", &ai.model);
        set_field(self, "WizAiName", &ai.custom);
        self.wizard_provider();
        self.prefs.ai_edited = false;
    }

    /// The value of `p` in use now, as the draft holds it.
    fn pref_value(&self, p: &Pref) -> Option<String> {
        let b = |v: bool| if v { "1".to_string() } else { "0".to_string() };
        let dflt = p.default_value();
        Some(match p.key {
            k if prefs::MENU.contains(&k) => b(checked(self, k)),
            "units" => prefs::UNIT_ITEMS.iter().position(|u| checked(self, u)).unwrap_or(0).to_string(),
            "aa" => self.settings.get("aa").and_then(|a| prefs::AA_ITEMS.iter().position(|x| *x == a)).unwrap_or(2).to_string(),
            "palette" => crate::paint::palette().to_string(),
            "animate" => map_animation_choice(self.settings.bool("animate")).to_string(),
            "lift" => (self.map.scene.lift_scale * 100.0).round().to_string(),
            "cushion" => (self.map.cushion_strength * 100.0).round().to_string(),
            "tilt" => (self.map.scene.tilt_amount / 0.8 * 100.0).round().to_string(),
            "aiWebLookup" => b(AiConfig::load().web_lookup),
            "afterScan" => self.settings.num("afterScan").unwrap_or(0.0).clamp(0.0, 1.0).to_string(),
            "theme" => (self.settings.num("theme").unwrap_or(0.0).clamp(0.0, (crate::theme::CHOICES.len() - 1) as f64) as usize).to_string(),
            "alertFreeGb" | "alertGrowGb" => self.settings.num(p.key).unwrap_or(1.0).clamp(0.0, 3.0).to_string(),
            "startTab" => crate::look::start_choice(self.settings.num("startTab")).to_string(),
            "snapshot_cap_mb" => {
                let cap = self.settings.num("snapshot_cap_mb").unwrap_or(1024.0) as u32;
                prefs::CAPS_MB.iter().position(|&c| c == cap).unwrap_or(1).to_string()
            }
            "uninstaller" => self.settings.get("uninstaller").unwrap_or("").to_string(),
            "scanExclude" => self.settings.get("scanExclude").unwrap_or("").to_string(),
            "programFolders" => self.settings.get("programFolders").unwrap_or("").to_string(),
            "searchDrivesOn" => self.settings.get("searchDrivesOn").unwrap_or("").to_string(),
            "searchColumns" => self.settings.get("searchColsHidden").unwrap_or("").to_string(),
            // the administrator switches: in the store only an administrator can write
            k if crate::admin::is_switch(k) => b(crate::admin::store::get(k)),
            k => match &p.kind {
                Kind::Number(knob, _) => sprawl_advisor::tuning::get(knob).to_string(),
                Kind::Switch(d) => b(self.settings.bool(k).unwrap_or(*d)),
                _ => return dflt,
            },
        })
    }

    /// Translate the validated draft to the same persistent keys as the contextual controls.
    fn stage_pref(s: &mut crate::settings::Settings, p: &Pref, v: &str) {
        let idx = |n: usize| v.parse::<usize>().unwrap_or(0).min(n.saturating_sub(1));
        match p.key {
            "units" => s.set("units", prefs::UNIT_ITEMS[idx(prefs::UNIT_ITEMS.len())]),
            "aa" => s.set("aa", prefs::AA_ITEMS[idx(prefs::AA_ITEMS.len())]),
            "animate" if v == "0" => s.remove("animate"),
            "animate" => s.set_bool("animate", v == "1"),
            "lift" | "cushion" | "tilt" => s.set(p.key, v.parse::<f32>().unwrap_or(0.0) * if p.key == "tilt" { 0.008 } else { 0.01 }),
            "snapshot_cap_mb" => s.set(p.key, prefs::CAPS_MB[idx(prefs::CAPS_MB.len())]),
            "uninstaller" => s.set(p.key, v.trim().trim_matches('"')),
            "scanExclude" | "programFolders" | "palette" | "theme" | "afterScan" | "startTab" | "alertFreeGb" | "alertGrowGb" => s.set(p.key, v.trim()),
            "searchColumns" => s.set("searchColsHidden", v),
            "searchDrivesOn" => { s.set(p.key, v); s.set("searchDrivesOff", ""); s.set_bool("oSearchDrives", false); }
            "aiWebLookup" | "oExplorer" | "communityFetch" => {}
            k if crate::admin::is_switch(k) => {}
            _ => match p.kind {
                Kind::Switch(_) => s.set_bool(p.key, v == "1"),
                Kind::Number(k, _) => s.set(p.key, choice_number(v, k.floor, k.max).unwrap_or(k.default)),
                _ => {}
            },
        }
    }

    /// Put the committed choice into use. Settings read by the operation need no side effect.
    fn pref_apply(&mut self, p: &Pref, v: &str) {
        let yes = v == "1";
        let idx = |n: usize| v.parse::<usize>().unwrap_or(0).min(n.saturating_sub(1));
        match p.key {
            k if prefs::MENU.contains(&k) => {
                if checked(self, k) != yes {
                    let _ = crate::menubar::set_checked(&self.root, k, yes);
                    self.command(k);
                }
            }
            "units" | "aa" => {
                let items: &[&str] = if p.key == "units" { &prefs::UNIT_ITEMS } else { &prefs::AA_ITEMS };
                let item = items[idx(items.len())];
                let _ = crate::menubar::set_checked(&self.root, item, true);
                self.command(item);
            }
            "palette" => {
                let i = idx(crate::paint::PALETTES.len());
                self.apply_map_setting("palette", i as f64);
            }
            "animate" => {
                let yes = match v { "1" => true, "2" => false, _ => crate::win32::animations_on() };
                self.apply_map_setting("animate", if yes { 1.0 } else { 0.0 });
            }
            "lift" | "cushion" | "tilt" => {
                let value = v.parse::<f64>().unwrap_or(0.0);
                self.apply_map_setting(p.key, value);
            }
            // the page is made again for the new theme at the next frame (restyle.rs)
            "theme" => {
                crate::restyle::request();
            }
            "decimal" => {
                crate::format::DECIMAL.store(yes, std::sync::atomic::Ordering::Relaxed);
                self.refresh_rows();
                self.refresh_exts();
                if let Some(t) = &self.tree { let root = t.root_path.clone(); self.update_header(&root); }
                if self.view == crate::state::View::Advisor { self.render_advisor(); }
                self.map.dirty = true;
                self.map.views.stamp = 0;
            }
            "programFolders" => {
                let paths = sprawl_advisor::program_folders::parse(v).unwrap_or_default();
                sprawl_advisor::program_folders::replace(paths);
            }
            "searchHistory" => self.search_history_changed(),
            "searchColumns" => self.apply_search_columns(crate::search::hidden_from(v)),
            "communityFetch" => {
                self.advisor.memory_ready();
                self.advisor.memory.community_off = !yes;
                self.save_memory();
                self.advisor_start();
            }
            "offline" => {
                sprawl_advisor::OFFLINE.store(yes, std::sync::atomic::Ordering::Relaxed);
                if yes {
                    self.wiz_models.reset();
                    self.advisor_command("AdvStop");
                    self.chat_reset();
                }
            }
            // written by the save itself (wizard_finish), which says when a write fails
            k if crate::admin::is_switch(k) => {}
            _ => match &p.kind {
                Kind::Number(knob, _) => {
                    sprawl_advisor::tuning::set(knob, choice_number(v, knob.floor, knob.max).unwrap_or(knob.default));
                }
                _ => {}
            },
        }
    }

    /// Size the card: its usual size when the window has room, else the window less a margin.
    pub(crate) fn wizard_fit(&self) {
        let (w, h) = card_size(self.wizard_all, self.win_w, self.win_h);
        if let Ok(c) = find::<FrameworkElement>(&self.root, "WizCard") {
            let _ = c.SetWidth(w);
            let _ = c.SetHeight(h);
        }
    }

    /// Show the page or section the overlay is on.
    pub(crate) fn wizard_show(&mut self) {
        let settings = self.wizard_all;
        shown(self, "PfNav", settings);
        self.wizard_fit();
        if settings {
            text(&self.root, "WizStep", "Settings");
            text(&self.root, "WizDots", "Changes apply when you press Save; the buttons in a section act at once.");
            text(&self.root, "WizNextText", "Save");
            shown(self, "WizBack", false);
            text(&self.root, "WizSkipText", "Cancel");
            for i in 0..Sec::ALL.len() {
                {
                    crate::look::paint(&self.root, &format!("PfSec{i}"), crate::look::Pick::Tab, i == self.prefs.sec);
                }
            }
        } else {
            shown(self, "WizBack", true);
            text(&self.root, "WizSkipText", "Skip setup");
            let p = self.wizard_page;
            text(&self.root, "WizStep", &format!("Step {} of {PAGES}: {}", p + 1, WIZARD[p].0));
            text(&self.root, "WizDots", &(0..PAGES).map(|i| if i == p { "●" } else { "○" }).collect::<Vec<_>>().join(" "));
            text(&self.root, "WizNextText", if p + 1 == PAGES { "Finish" } else { "Next" });
            if let Ok(b) = find::<Control>(&self.root, "WizBack") {
                let _ = b.SetIsEnabled(p > 0);
            }
        }
        self.prefs_render();
    }

    /// Build the rows of the page shown.
    pub(crate) fn prefs_render(&mut self) {
        let (top, keys): (String, Vec<&'static str>) = if self.wizard_all {
            let sec = Sec::ALL[self.prefs.sec.min(Sec::ALL.len() - 1)];
            let groups = sec.groups();
            let keys = groups.get(self.prefs.group.min(groups.len().saturating_sub(1))).map_or_else(Vec::new, |g| g.1.to_vec());
            (page_title(sec.glyph(), sec.title(), ""), keys)
        } else {
            let (title, glyph, sub, keys) = WIZARD[self.wizard_page.min(PAGES - 1)];
            let title = if self.wizard_page == 0 { "Welcome to Sprawl" } else { title };
            (page_title(glyph, title, sub), keys.to_vec())
        };
        if let (Ok(host), Ok(el)) = (find::<Border>(&self.root, "PfTop"), load::<UIElement>(&top)) {
            let _ = host.SetChild(&el);
        }
        if let Ok(host) = find::<Border>(&self.root, "PfGroupHost") {
            let sec = Sec::ALL[self.prefs.sec.min(Sec::ALL.len() - 1)];
            let groups: Vec<_> = sec.groups().iter().map(|g| g.0).collect();
            let group = if self.wizard_all && groups.len() > 1 {
                let x = labelled_row("PfGroup", "Section", &combo("PfGroup", &groups, self.prefs.group.min(groups.len() - 1)));
                load::<FrameworkElement>(&x.replacen("<Grid ", &format!("<Grid {} ", ui::NS), 1)).ok()
            } else { None };
            let child = group.as_ref().and_then(|g| g.cast::<UIElement>().ok());
            let _ = host.SetChild(child.as_ref());
            if let Some(s) = group.and_then(|g| find::<Selector>(&g, "PfGroup").ok()) {
                if let Ok(t) = s.SelectionChanged(|sel, _| {
                    if let Some(i) = sel.as_ref().and_then(|s| s.cast::<Selector>().ok()).and_then(|c| c.SelectedIndex().ok()).and_then(|i| usize::try_from(i).ok()) {
                        with(move |s| s.prefs_group(i));
                    }
                }) { keep(t); }
            }
        }
        // the Cleaning section's backup list, read once here, never per row or per render
        if keys.contains(&"registryBackups") && self.prefs.registry_backups.is_none() && !crate::settings::read_only() {
            self.prefs.registry_backups = Some(sprawl_advisor::regclean::backups());
        }
        shown(self, "WizAi", keys.contains(&"ai"));
        if keys.contains(&"ai") {
            self.wizard_detect();
            self.wizard_saved_providers();
            self.wizard_local_models();
        }
        let mut x = format!(r##"<StackPanel {} Spacing="4">"##, ui::NS);
        if let Some(n) = &self.prefs.note {
            x += &format!(r##"<Border Padding="10,6" CornerRadius="{{$rc6}}" Background="{{$goodwash}}"><TextBlock Text="{}" TextWrapping="Wrap" FontSize="{{$fs12}}"/></Border>"##, ui::esc(n));
        }
        if keys.contains(&"alertLow") || keys.contains(&"alertGrowth") {
            let wanted = self.settings.bool("alertLow") == Some(true) || self.settings.bool("alertGrowth") == Some(true);
            x += &help(crate::alert::task_status(wanted));
        }
        if !self.wizard_all && self.wizard_page == 0 {
            x += r##"<TextBlock TextWrapping="Wrap" FontSize="{$fs13}" Foreground="{$text1}" Margin="0,4,0,6" Text="A few choices before you start, all optional: nothing that sends or keeps data is on unless you choose it, and each one is in Settings (Ctrl+Shift+S) too."/>"##;
            for (t, g, s, _) in WIZARD.iter().skip(1) {
                x += &bullet(g, t, s);
            }
        }
        let mut buttons = Vec::new();
        for k in &keys {
            let Some(p) = prefs::get(k) else { continue };
            if p.key == "ai" {
                continue;
            }
            x += &self.pref_row(p, &mut buttons);
        }
        let theme_page = self.wizard_all && Sec::ALL[self.prefs.sec.min(Sec::ALL.len() - 1)] == Sec::Theme;
        if theme_page {
            let (tx, tb) = self.prefs.theme.page();
            x += &tx;
            buttons.extend(tb);
        } else if self.wizard_all && !keys.contains(&"ai") {
            x += &format!(r##"<StackPanel Orientation="Horizontal" Spacing="8" Margin="0,16,0,0">{}</StackPanel>"##, button("PfB_reset", "Reset this section"));
            buttons.push("PfB_reset".to_string());
        }
        x += "</StackPanel>";
        self.prefs.shown = keys;
        let fe = load::<FrameworkElement>(&x).ok();
        if let (Ok(host), Some(el)) = (find::<Border>(&self.root, "PfHost"), fe.as_ref()) {
            let _ = host.SetChild(el);
        }
        if let Some(fe) = &fe {
            for name in buttons {
                if let Ok(b) = find::<ButtonBase>(fe, &name) {
                    if let Ok(t) = b.Click(move |_, _| {
                        let n = name.clone();
                        with(move |s| s.prefs_button(&n));
                    }) {
                        keep(t);
                    }
                }
            }
            if theme_page {
                self.theme_wire(fe);
            }
            // Update dependent controls as soon as their switch moves.
            for name in ["Pf_offline", "Pf_alertGrowth"] {
                if let Ok(t) = find::<ToggleSwitch>(fe, name) {
                    if let Ok(tok) = t.Toggled(|_, _| { with(|s| s.prefs_offline_shown()); }) {
                        keep(tok);
                    }
                }
            }
        }
        self.prefs.fe = fe;
        self.prefs_offline_shown();
        self.wizard_provider_rows();
    }

    /// Dev screenshot step: select a zero-based group in the open Settings section.
    pub(crate) fn prefs_group_for_shot(&mut self, group: usize) {
        if self.shot.is_none() || !self.wizard_all || !self.wizard_shown() { return; }
        match Sec::ALL.get(self.prefs.sec).copied() {
            Some(Sec::Theme) if group < crate::themepage::GROUPS.len() => self.prefs_button(&format!("TmTab{group}")),
            Some(sec) if group < sec.groups().len() => self.prefs_group(group),
            _ => {}
        }
    }

    fn prefs_group(&mut self, group: usize) {
        if self.prefs.group == group { return; }
        self.prefs_harvest();
        self.prefs.group = group;
        self.prefs.list_page = 0;
        self.prefs.note = None;
        self.prefs.clear_armed = false;
        self.prefs_render();
        if let Ok(host) = find::<Border>(&self.root, "PfGroupHost") {
            if let Ok(group) = host.Child().and_then(|p| p.cast::<FrameworkElement>()).and_then(|p| find::<UIElement>(&p, "PfGroup")) {
                let _ = group.Focus(self.prefs.open_state);
            }
        }
    }

    /// One setting's row.
    fn pref_row(&mut self, p: &Pref, buttons: &mut Vec<String>) -> String {
        let v = self.prefs.draft.get(p.key).cloned().unwrap_or_default();
        let name = format!("Pf_{}", p.key);
        let row = |inner: String| format!(r##"<StackPanel x:Name="PfRow_{}" Spacing="0" Margin="0,4,0,0">{inner}</StackPanel>"##, p.key);
        // the four administrator switches share one warning, in a box above the first
        let admin_note = if p.key == crate::admin::SHELL_MENU.key {
            format!(
                r##"<Border Margin="0,10,0,2" Padding="12,10" CornerRadius="{{$rc6}}" Background="{{$warnwash}}" BorderBrush="{{$warnline}}" BorderThickness="1" MaxWidth="640" HorizontalAlignment="Left"><StackPanel Spacing="4"><TextBlock Text="While Sprawl runs as administrator, also allow" FontSize="{{$fs13}}" FontWeight="SemiBold"/><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text1}}" TextWrapping="Wrap"/>{}</StackPanel></Border>"##,
                ui::esc(crate::admin::WARNING),
                if sprawl_core::is_elevated() {
                    String::new()
                } else {
                    ui::note("They can be changed only in Sprawl running as administrator,", "so that no other program can turn them on for you.")
                }
            )
        } else {
            String::new()
        };
        // greyed out unless elevated: the store they are kept in takes no other writer
        let enabled = if crate::admin::is_switch(p.key) && !sprawl_core::is_elevated() { r#" IsEnabled="False""# } else { "" };
        match &p.kind {
            Kind::Switch(_) => {
                let note = if matches!(p.key, "oAlwaysAdmin" | "alertDailyScan" | "privacyCleaners" | "offline" | "searchHistory") { help(p.help) } else { String::new() };
                format!("{admin_note}{}", row(format!("{}{note}", labelled_row(&name, p.label, &format!(r##"<ToggleSwitch x:Name="{name}" AutomationProperties.LabeledBy="{{Binding ElementName={name}_Label}}" ToolTipService.ToolTip="{}" OnContent="" OffContent="" IsOn="{}"{enabled}/>"##, ui::esc(p.help), on(&v))))))
            }
            Kind::Number(k, unit) => row(labelled_row(&name, p.label, &format!(
                r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBox x:Name="{name}" AutomationProperties.LabeledBy="{{Binding ElementName={name}_Label}}" ToolTipService.ToolTip="{}" Width="96" MaxLength="15" Text="{}" InputScope="Number"/><TextBlock Text="{}" VerticalAlignment="Center" FontSize="{{$fs12}}" Foreground="{{$text2}}"/></StackPanel>"##,
                ui::esc(&format!("{}. Range: {}–{} {unit}", p.help_line(), k.floor, k.max)), ui::esc(&v), ui::esc(unit)
            ))),
            Kind::Choice(items, _) => {
                let help = if matches!(p.key, "startTab" | "snapshot_cap_mb") { help(p.help) } else { String::new() };
                let c = combo(&name, items, v.parse().unwrap_or(0)).replacen("<ComboBox ", &format!(r#"<ComboBox ToolTipService.ToolTip="{}" "#, ui::esc(p.help)), 1);
                row(format!("{}{help}", labelled_row(&name, p.label, &c)))
            }
            Kind::Text(ph) => {
                let field = format!(r##"<TextBox x:Name="{name}" AutomationProperties.LabeledBy="{{Binding ElementName={name}_Label}}" Text="{}" PlaceholderText="{}"/>"##, ui::esc(&v), ui::esc(ph));
                let field = if p.key == "uninstaller" {
                    buttons.push("PfB_uninstaller".into());
                    format!(r##"<Grid ColumnSpacing="6"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>{field}<Button Grid.Column="1" x:Name="PfB_uninstaller" Content="Browse…"/></Grid>"##)
                } else { field };
                row(format!("{}{}", labelled_row(&name, p.label, &field), help(p.help)))
            }
            Kind::Range(max, default) => row(labelled_row(&name, p.label, &format!(r##"<Slider x:Name="{name}" AutomationProperties.LabeledBy="{{Binding ElementName={name}_Label}}" Minimum="0" Maximum="{max}" StepFrequency="1" Value="{}"/>"##, v.parse::<u32>().unwrap_or(*default).min(*max)))),
            Kind::Custom => row(self.custom_row(p, &v, buttons)),
        }
    }

    /// The rows built by their own code.
    fn custom_row(&mut self, p: &Pref, v: &str, buttons: &mut Vec<String>) -> String {
        let mut b = |name: &str, t: &str| {
            buttons.push(name.to_string());
            button(name, t)
        };
        let mut x = label(p.label);
        match p.key {
            "dataFolder" => {
                x += &help(p.help);
                x += &format!(r##"<StackPanel Orientation="Horizontal" Spacing="8">{}{}</StackPanel>"##, b("PfB_data", "Open Roaming data"), b("PfB_local", "Open Local data"));
                x += &help("Restoring preferences preserves scan history, saved searches, Keep decisions, program folders, AI keys and saved providers. Administrator allowances can only be reset when elevated.");
                x += &b("PfB_resetAll", "Restore preference defaults");
            }
            "aiLog" => {
                x += &help(p.help);
                let size = sprawl_advisor::llm::request_log_path().and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len());
                x += &format!(
                    r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text2}}" VerticalAlignment="Center"/>{}{}</StackPanel>"##,
                    ui::esc(&if size == 0 { "Empty".to_string() } else { self.size(size) }),
                    b("PfB_logOpen", "Open"),
                    b("PfB_logClear", "Clear")
                );
                x += &format!(r##"<StackPanel Orientation="Horizontal" Spacing="8" Margin="0,4,0,0">{}</StackPanel>"##, b("PfB_aiForget", "Forget what the AI said"));
            }
            "searchDrivesOn" => {
                x += &help(p.help);
                let enabled_drives: Vec<String> = v.split(',').map(str::trim).filter(|d| !d.is_empty()).map(str::to_lowercase).collect();
                let drives = State::fixed_drives_pub();
                let (range, pages) = page_range(drives.len(), self.prefs.list_page);
                self.prefs.drives = drives.iter().enumerate().skip(range.start).take(range.len()).map(|(i, d)| (i, d.clone())).collect();
                for (i, d) in &self.prefs.drives {
                    x += &format!(r##"<ToggleSwitch x:Name="PfDrv{i}" AutomationProperties.Name="{0}" OnContent="{0}" OffContent="{0}" IsOn="{1}" FontSize="{{$fs13}}"/>"##, ui::esc(d), on(if enabled_drives.contains(&d.to_lowercase()) { "1" } else { "0" }));
                }
                if drives.is_empty() {
                    x += &help("No fixed drive found.");
                }
                drop(b);
                x += &page_controls("Drive", drives.len(), range, pages, buttons);
            }
            "programFolders" => {
                x += &help(p.help);
                let paths = match sprawl_advisor::program_folders::parse(v) {
                    Ok(paths) => paths,
                    Err(e) => {
                        x += &help(&e);
                        Vec::new()
                    }
                };
                if paths.is_empty() && v.trim().is_empty() {
                    x += &help("Nothing added yet. Program Files is always checked.");
                }
                let add = b("PfB_progAdd", "Add folder...");
                let (range, pages) = page_range(paths.len(), self.prefs.list_page);
                for (i, path) in paths.iter().enumerate().skip(range.start).take(range.len()) {
                    x += &format!(
                        r##"<Grid ColumnSpacing="8" Margin="0,4,0,0"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBlock Text="{0}" ToolTipService.ToolTip="{0}" FontFamily="Consolas" FontSize="{{$fs12}}" Foreground="{{$text2}}" TextTrimming="CharacterEllipsis" VerticalAlignment="Center" IsTextSelectionEnabled="True"/><Button x:Name="PfB_progDel{i}" Grid.Column="1" AutomationProperties.Name="Remove {0}" Content="Remove" Padding="8,1" FontSize="{{$fs12}}" CornerRadius="{{$rc4}}" VerticalAlignment="Top"/></Grid>"##,
                        ui::esc(&path.display().to_string())
                    );
                    buttons.push(format!("PfB_progDel{i}"));
                }
                x += &format!(r##"<StackPanel Orientation="Horizontal" Spacing="8" Margin="0,6,0,0">{add}</StackPanel>"##);
                x += &page_controls("Prog", paths.len(), range, pages, buttons);
            }
            "neverTouch" => {
                x += &help(p.help);
                self.advisor.memory_ready();
                let kept = self.advisor.memory.kept_paths();
                if kept.is_empty() {
                    x += &help("Nothing yet. Keep on a finding adds it here.");
                }
                let (range, pages) = page_range(kept.len(), self.prefs.list_page);
                for (i, k) in kept.iter().enumerate().skip(range.start).take(range.len()) {
                    x += &format!(
                        r##"<Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBlock Text="{0}" ToolTipService.ToolTip="{0}" FontFamily="Consolas" FontSize="{{$fs12}}" Foreground="{{$text2}}" TextTrimming="CharacterEllipsis" VerticalAlignment="Center"/><Button x:Name="PfB_unkeep{i}" Grid.Column="1" AutomationProperties.Name="Remove {0}" Content="Remove" Padding="8,1" FontSize="{{$fs12}}" CornerRadius="{{$rc4}}"/></Grid>"##,
                        ui::esc(&k.display().to_string())
                    );
                    buttons.push(format!("PfB_unkeep{i}"));
                }
                x += &format!(
                    r##"<Grid ColumnSpacing="8" Margin="0,4,0,0" MaxWidth="560" HorizontalAlignment="Left"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBox x:Name="PfKeepPath" PlaceholderText="A folder or file to keep, like D:\Photos"/><Button x:Name="PfB_keepAdd" Grid.Column="1" Content="Add"/></Grid>"##
                );
                buttons.push("PfB_keepAdd".into());
                x += &page_controls("Keep", kept.len(), range, pages, buttons);
            }
            "recentSearches" => {
                x += &help(p.help);
                x += &b("PfB_recentClear", "Clear recent searches");
            }
            "searchColumns" => {
                x += &help(p.help);
                let hidden = crate::search::hidden_from(v);
                for (_, caption, i) in crate::search::COL_MENU {
                    x += &format!(r##"<ToggleSwitch x:Name="PfCol{i}" Header="{}" IsOn="{}" OnContent="" OffContent=""/>"##, ui::esc(caption), if hidden & (1 << i) == 0 { "True" } else { "False" });
                }
            }
            "mcp" => {
                x += &help(p.help);
                x += &format!(
                    r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" FontFamily="Consolas" FontSize="{{$fs12}}" Foreground="{{$text2}}" VerticalAlignment="Center" IsTextSelectionEnabled="True" TextTrimming="CharacterEllipsis" MaxWidth="520"/>{}</StackPanel>"##,
                    ui::esc(&crate::bridge::setup_command()),
                    b("PfB_mcpCopy", "Copy")
                );
            }
            "historyDisk" => {
                x += &help(p.help);
                let (n, bytes) = sprawl_advisor::history::base_dir().map(|d| sprawl_advisor::history::list(&d)).map_or((0, 0), |l| (l.len(), l.iter().map(|e| e.bytes).sum::<u64>()));
                let armed = self.prefs.clear_armed;
                x += &format!(
                    r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text2}}" VerticalAlignment="Center"/>{}</StackPanel>"##,
                    ui::esc(&format!("{n} snapshots, {}", self.size(bytes))),
                    if n > 0 { b("PfB_histClear", if armed { "Press again to delete them all" } else { "Clear" }) } else { String::new() }
                );
            }
            "registryBackups" => {
                x += &help(p.help);
                let list = self.prefs.registry_backups.clone().unwrap_or_default();
                if list.is_empty() {
                    x += &help("No backups yet. They are made when you clean a registry list in Cleaners.");
                }
                let (range, pages) = page_range(list.len(), self.prefs.list_page);
                for (i, bk) in list.iter().enumerate().skip(range.start).take(range.len()) {
                    // the date comes from the file name's own stamp; a stamp too far out is
                    // skipped, never made to panic
                    let Some(when) = std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(bk.when)).map(sprawl_core::local_time) else { continue };
                    x += &format!(
                        r##"<Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text2}}" VerticalAlignment="Center" TextTrimming="CharacterEllipsis"/><StackPanel Grid.Column="1" Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" VerticalAlignment="Center"/>{}</StackPanel></Grid>"##,
                        ui::esc(&bk.title),
                        ui::esc(&format!("{when} · {}", if bk.values == 1 { "1 value".to_string() } else { format!("{} values", bk.values) })),
                        b(&format!("PfB_regBack{i}"), "Put back")
                    );
                }
                x += &b("PfB_browserBackups", "Open browser backup folder");
                x += &page_controls("Reg", list.len(), range, pages, buttons);
            }
            _ => {}
        }
        x
    }

    /// Read the page's controls back into the draft (before the page changes, and on Save).
    pub(crate) fn prefs_harvest(&mut self) {
        let Some(fe) = self.prefs.fe.clone() else { return };
        for k in self.prefs.shown.clone() {
            let Some(p) = prefs::get(k) else { continue };
            let name = format!("Pf_{k}");
            let v = match &p.kind {
                Kind::Switch(_) => find::<ToggleSwitch>(&fe, &name).and_then(|t| t.IsOn()).ok().map(|b| if b { "1".to_string() } else { "0".to_string() }),
                Kind::Number(..) | Kind::Text(_) => find::<TextBox>(&fe, &name).and_then(|t| t.Text()).ok().map(|h| h.to_string_lossy()),
                Kind::Range(..) => find::<RangeBase>(&fe, &name).and_then(|r| r.Value()).ok().map(|v| v.round().to_string()),
                Kind::Choice(..) => find::<Selector>(&fe, &name).and_then(|s| s.SelectedIndex()).ok().map(|i| i.max(0).to_string()),
                Kind::Custom if k == "searchDrivesOn" => {
                    let mut on: std::collections::BTreeSet<_> = self.prefs.draft.get(k).into_iter().flat_map(|s| s.split(',')).map(str::trim).filter(|d| !d.is_empty()).map(str::to_ascii_uppercase).collect();
                    for (i, d) in &self.prefs.drives {
                        if let Ok(value) = find::<ToggleSwitch>(&fe, &format!("PfDrv{i}")).and_then(|t| t.IsOn()) {
                            if value { on.insert(d.clone()); } else { on.remove(d); }
                        }
                    }
                    Some(on.into_iter().collect::<Vec<_>>().join(","))
                }
                Kind::Custom if k == "searchColumns" => {
                    let mut hidden = 0;
                    for (_, _, i) in crate::search::COL_MENU {
                        if find::<ToggleSwitch>(&fe, &format!("PfCol{i}")).and_then(|t| t.IsOn()) == Ok(false) { hidden |= 1 << i; }
                    }
                    Some(crate::search::hidden_to(hidden))
                }
                Kind::Custom => None,
            };
            if let Some(v) = v {
                self.prefs.draft.insert(k, v);
            }
        }
    }

    fn draft_offline(&self) -> bool {
        let on_screen = self
            .prefs
            .fe
            .as_ref()
            .and_then(|fe| find::<ToggleSwitch>(fe, "Pf_offline").and_then(|t| t.IsOn()).ok());
        on_screen.unwrap_or_else(|| switch_draft(&self.prefs.draft, "offline"))
    }

    /// Read-only, the saved gate and the draft each veto a request from the editor.
    fn prefs_offline_shown(&self) {
        let off = self.draft_offline();
        if let Some(fe) = &self.prefs.fe {
            for n in ["Pf_oUpdates", "Pf_communityFetch", "Pf_aiWebLookup"] {
                if let Ok(c) = find::<Control>(fe, n) {
                    let _ = c.SetIsEnabled(!off);
                }
            }
        }
        for n in ["WizLoadModels", "WizUseDetected"] {
            if let Ok(c) = find::<Control>(&self.root, n) {
                let _ = c.SetIsEnabled(self.wizard_request_allowed());
            }
        }
        if let Some(fe) = &self.prefs.fe {
            let daily = find::<ToggleSwitch>(fe, "Pf_alertGrowth").and_then(|t| t.IsOn()).unwrap_or(false) && switch_draft(&self.prefs.draft, "snapshots");
            if let Ok(c) = find::<Control>(fe, "Pf_alertDailyScan") {
                let _ = c.SetIsEnabled(daily);
            }
        }
    }

    /// A button inside a page.
    pub(crate) fn prefs_button(&mut self, name: &str) {
        self.prefs_harvest();
        self.prefs.note = None;
        if name != "PfB_histClear" {
            self.prefs.clear_armed = false;
        }
        match name {
            n if n.starts_with("Tm") => self.theme_button(n),
            n if n.starts_with("PfB_pageNext") => self.prefs.list_page += 1,
            n if n.starts_with("PfB_pagePrev") => self.prefs.list_page = self.prefs.list_page.saturating_sub(1),
            n if crate::settings::read_only() && matches!(n, "PfB_data" | "PfB_local" | "PfB_browserBackups" | "PfB_logOpen" | "PfB_logClear" | "PfB_aiForget" | "PfB_histClear" | "PfB_keepAdd" | "PfB_recentClear")
                || crate::settings::read_only() && (n.starts_with("PfB_regBack") || n.starts_with("PfB_unkeep")) => {
                self.prefs.note = Some("Screenshot mode cannot change or open profile data.".into());
            }
            "PfB_reset" | "PfB_resetAll" => {
                let sec = Sec::ALL[self.prefs.sec.min(Sec::ALL.len() - 1)];
                let elevated = sprawl_core::is_elevated();
                reset_draft(&mut self.prefs.draft, (name != "PfB_resetAll").then_some(sec), elevated);
                if name == "PfB_resetAll" {
                    self.theme_reset();
                }
                if name == "PfB_resetAll" || sec == Sec::Ai {
                    select(self, "WizProvider", 0);
                    self.wizard_discard_secret();
                    self.prefs.ai_forget_key = None;
                    self.prefs.ai_edited = true;
                    self.wizard_provider();
                }
                let admin = if !elevated && (name == "PfB_resetAll" || sec == Sec::Scanning) {
                    " Administrator permissions are unchanged unless Sprawl runs as administrator."
                } else { "" };
                self.prefs.note = Some(format!("Defaults are staged. Save applies them; Cancel keeps your preferences.{admin}"));
            }
            "PfB_data" | "PfB_local" | "PfB_browserBackups" => {
                let var = if name == "PfB_data" { "APPDATA" } else { "LOCALAPPDATA" };
                let mut path = std::env::var_os(var).map(|a| std::path::PathBuf::from(a).join("Sprawl"));
                if name == "PfB_browserBackups" {
                    path = path.map(|p| p.join("backups").join("privacy"));
                }
                if let Some(d) = path.filter(|p| p.is_dir()) {
                    crate::win32::reveal(&d);
                } else { self.prefs.note = Some("That folder does not exist yet.".into()); }
            }
            "PfB_recentClear" => self.clear_recent_searches(),
            "PfB_uninstaller" => {
                if let Some(p) = crate::dialogs::pick_uninstaller(self.hwnd) {
                    self.prefs.draft.insert("uninstaller", p.to_string_lossy().into_owned());
                }
            }
            "PfB_logOpen" => match sprawl_advisor::llm::request_log_path().filter(|p| p.is_file()) {
                Some(p) => {
                    let _ = std::process::Command::new("notepad.exe").arg(p).spawn();
                }
                None => self.prefs.note = Some("The AI request log is empty.".into()),
            },
            "PfB_logClear" => {
                self.prefs.note = Some(match sprawl_advisor::llm::clear_request_log() {
                    Ok(()) => "The AI request log is cleared.".into(),
                    Err(e) => format!("The AI request log couldn't be cleared ({e})."),
                });
            }
            "PfB_aiForget" => {
                self.advisor.memory_ready();
                let m = &mut self.advisor.memory;
                let n = m.labels.iter().filter(|l| l.from == sprawl_advisor::memory::From::Model).count() + m.proposals.len() + m.cleaner_proposals.len();
                m.labels.retain(|l| l.from != sprawl_advisor::memory::From::Model);
                m.proposals.clear();
                m.cleaner_proposals.clear();
                self.advisor.cl.part_answers.clear();
                self.save_memory();
                self.prefs.note = Some(format!("Forgot {n} things the AI said. Rules and cleaners you approved stay."));
            }
            "PfB_mcpCopy" => {
                crate::win32::copy_text(self.hwnd, &crate::bridge::setup_command());
                self.prefs.note = Some("Copied the Claude Code setup command.".into());
            }
            "PfB_histClear" if !self.prefs.clear_armed => self.prefs.clear_armed = true,
            "PfB_histClear" => {
                self.prefs.clear_armed = false;
                let cleared = sprawl_advisor::history::base_dir().map_or(Ok((0, 0)), |base| sprawl_advisor::history::clear(&base));
                self.prefs.note = Some(match cleared {
                    Ok((n, 0)) => format!("Deleted {n} snapshots of History."),
                    Ok((n, left)) => format!("Deleted {n} snapshots of History. Could not clear {left}."),
                    Err(e) => format!("No snapshots were cleared: {e}"),
                });
            }
            "PfB_progAdd" => {
                if let Some(picked) = crate::dialogs::pick_setting_folder(self.hwnd, "Select a program folder") {
                    match checked_program_folder(&picked) {
                        Err(e) => self.prefs.note = Some(e),
                        Ok(path) => {
                            let mut paths = sprawl_advisor::program_folders::parse(self.prefs.draft.get("programFolders").map_or("", String::as_str)).unwrap_or_default();
                            let key = path.to_string_lossy();
                            if paths.iter().any(|p| p.to_string_lossy().eq_ignore_ascii_case(&key)) {
                                self.prefs.note = Some(format!("{} is already in the list.", path.display()));
                            } else {
                                paths.push(path);
                                match sprawl_advisor::program_folders::parse(&sprawl_advisor::program_folders::value(&paths)) {
                                    Ok(paths) => {
                                        self.prefs.draft.insert("programFolders".into(), sprawl_advisor::program_folders::value(&paths));
                                        self.prefs.note = Some("Program folder added. Press Save to keep it.".into());
                                    }
                                    Err(e) => self.prefs.note = Some(e),
                                }
                            }
                        }
                    }
                }
            }
            n if n.starts_with("PfB_progDel") => {
                let i: usize = n["PfB_progDel".len()..].parse().unwrap_or(usize::MAX);
                let mut paths = sprawl_advisor::program_folders::parse(self.prefs.draft.get("programFolders").map_or("", String::as_str)).unwrap_or_default();
                if i < paths.len() {
                    let removed = paths.remove(i);
                    self.prefs.draft.insert("programFolders".into(), sprawl_advisor::program_folders::value(&paths));
                    self.prefs.note = Some(format!("{} removed. Press Save to keep that.", removed.display()));
                }
            }
            "PfB_keepAdd" => {
                let fe = self.prefs.fe.clone();
                let typed = fe.as_ref().and_then(|fe| find::<TextBox>(fe, "PfKeepPath").and_then(|t| t.Text()).ok()).map(|h| h.to_string_lossy()).unwrap_or_default();
                let path = std::path::PathBuf::from(typed.trim().trim_matches('"'));
                match sprawl_advisor::memory::keep_path(&path) {
                    Ok(path) => {
                        self.advisor.memory_ready();
                        let before = self.advisor.memory.clone();
                        let title = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                        self.advisor.memory.decide(&path, &title, sprawl_advisor::memory::Verdict::Keep);
                        self.prefs.note = Some(if self.save_keep_edit(before) { format!("Added {} to the Keep list.", path.display()) } else {
                            "Couldn't save the Keep list. Nothing was added.".into()
                        });
                    }
                    Err(e) => self.prefs.note = Some(format!("Use the full path of an existing local file or folder. Sprawl couldn't check it ({e}).")),
                }
            }
            n if n.starts_with("PfB_regBack") => {
                let i: usize = n["PfB_regBack".len()..].parse().unwrap_or(usize::MAX);
                // the row is an index into the cached list the page was rendered from, never a
                // fresh listing; the file's name is what the put back then uses
                match self.prefs.registry_backups.as_ref().and_then(|l| l.get(i)).cloned() {
                    None => self.prefs.note = Some("That backup is no longer there.".into()),
                    Some(bk) => match self.registry_put_back_refusal() {
                        Some(why) => self.prefs.note = Some(why),
                        None => {
                            let name = bk.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            let (title, values, part) = (bk.title.clone(), bk.values, bk.part.clone());
                            self.ask(
                                "Put registry values back?",
                                &format!("Write back {values} {} to {title}? Values written since stay.", if values == 1 { "value" } else { "values" }),
                                "Put back",
                                false,
                                move |s| s.registry_put_back(&name, &part, &title),
                            );
                        }
                    },
                }
            }
            n if n.starts_with("PfB_unkeep") => {
                let i: usize = n["PfB_unkeep".len()..].parse().unwrap_or(usize::MAX);
                self.advisor.memory_ready();
                if let Some(p) = self.advisor.memory.kept_paths().get(i).cloned() {
                    let before = self.advisor.memory.clone();
                    self.advisor.memory.undo(&p);
                    self.prefs.note = Some(if !self.save_keep_edit(before) {
                        "Couldn't save the Keep list. Nothing was removed.".into()
                    } else if self.advisor.memory.kept_paths().iter().any(|k| crate::cleanup::key(k) == crate::cleanup::key(&p)) {
                        "Another window changed this Keep. Review the updated list before removing it.".into()
                    } else {
                        format!("{} removed from the Keep list.", p.display())
                    });
                }
            }
            _ => {}
        }
        self.prefs_render();
    }

    /// Why a registry backup can't be put back right now, if it can't: a clean-up, another file
    /// operation, a scan or an update is running.
    fn registry_put_back_refusal(&self) -> Option<String> {
        if self.advisor.cleanup.running.is_some() {
            return Some("A clean-up is running. Put back can wait until it ends.".into());
        }
        if self.advisor.file_op() {
            return Some("Another file operation is running. Put back can wait until it ends.".into());
        }
        if self.job.is_some() {
            return Some("A scan is running. Put back can wait until it ends.".into());
        }
        if self.update.applying {
            return Some("An update is being applied. Put back can wait until it ends.".into());
        }
        None
    }

    /// Put one backup back, named by its file (the file Sprawl itself listed), then say what
    /// was put back and read the list and the counts again.
    fn registry_put_back(&mut self, name: &str, part: &str, title: &str) {
        // a clean-up, scan or update may have started while the question was open
        if let Some(why) = self.registry_put_back_refusal() {
            self.prefs.note = Some(why);
            self.prefs_render();
            return;
        }
        let Some(dir) = sprawl_advisor::regclean::backup_dir() else {
            self.prefs.note = Some("Sprawl's data folder couldn't be found, so nothing was put back.".into());
            return;
        };
        let path = dir.join(name);
        match sprawl_advisor::regclean::restore(&path, Some(part)) {
            Ok(v) => {
                self.prefs.note = Some(format!("Put back {v} {} of {title}. Values written since stay.", if v == 1 { "value" } else { "values" }));
                self.prefs.registry_backups = None;
                self.refresh_registry_advice();
            }
            Err(e) => self.prefs.note = Some(format!("The backup couldn't be put back: {e}")),
        }
        self.prefs_render();
    }

    /// The provider list changed: show the fields that provider needs.
    pub(crate) fn wizard_provider(&mut self) {
        let p = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
        // a key typed for one provider never goes to another (Fable 1.0.4-1.0.5 P2-4)
        thread_local! { static TYPED_FOR: std::cell::Cell<Option<Provider>> = const { std::cell::Cell::new(None) }; }
        if TYPED_FOR.with(|t| t.replace(Some(p))).is_some_and(|q| q != p) {
            self.wizard_discard_secret();
        }
        shown(self, "WizKeyRow", p.needs_key());
        shown(self, "WizUrlRow", p == Provider::OpenAi);
        shown(self, "WizLoadModels", matches!(p, Provider::OpenAi | Provider::Anthropic));
        if p == Provider::OpenAi {
            let i = sprawl_advisor::llm::models::preset_index(&field(self, "WizUrl"));
            if i != selected(self, "WizUrlPreset") {
                select(self, "WizUrlPreset", i);
            }
        }
        shown(self, "WizCodexRisk", p == Provider::Codex);
        shown(self, "WizModelRow", p != Provider::Off);
        if p != Provider::OpenAi { shown(self, "WizRemoveAi", false); }
        // an empty model, or another provider's default, becomes this provider's default
        let m = field(self, "WizModel").trim().to_string();
        if p != Provider::Off && (m.is_empty() || Provider::ALL.iter().any(|q| q.default_model() == m)) {
            set_field(self, "WizModel", p.default_model());
        }
        self.wizard_key_hint(p);
        // the list of models for this provider; a text box until (and unless) it arrives. An
        // OpenAI-compatible address is asked only on "Load models", never while it is typed;
        // nothing is asked while working offline
        if self.prefs.shown.contains(&"ai") { self.wizard_local_models(); }
        self.wizard_models();
        self.wizard_provider_rows();
    }

    fn wizard_detect(&mut self) {
        if !self.wizard_request_allowed() {
            shown(self, "WizDetected", false);
            shown(self, "WizUseDetected", false);
            return;
        }
        shown(self, "WizDetected", true);
        if self.prefs.ai_detected.is_none() {
            let found = Detected::probe();
            text(&self.root, "WizDetected", &found.summary());
            self.prefs.ai_detected = Some(found.suggest());
        }
        shown(self, "WizUseDetected", self.prefs.ai_detected.as_ref().is_some_and(|a| a.provider != Provider::Off));
    }

    fn wizard_request_allowed(&self) -> bool {
        !crate::settings::read_only() && request_allowed(self.draft_offline(), sprawl_advisor::offline())
    }

    fn wizard_local_models(&mut self) {
        let p = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
        if self.wizard_request_allowed() && matches!(p, Provider::ClaudeCode | Provider::Codex) {
            self.wiz_models.want(p, "", "");
        }
    }

    pub(crate) fn wizard_discard_secret(&mut self) {
        if let Ok(k) = find::<PasswordBox>(&self.root, "WizKey") {
            let _ = k.SetPassword(&"".into());
        }
        self.wiz_models.reset();
    }

    pub(crate) fn wizard_ai_changed(&mut self, address: bool) {
        self.prefs.ai_edited = true;
        if address {
            self.wizard_discard_secret();
            let provider = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
            self.wizard_key_hint(provider);
            self.wizard_models();
        }
    }

    fn wizard_key_hint(&self, p: Provider) {
        let url = field(self, "WizUrl");
        let has = if p == Provider::OpenAi { sprawl_advisor::llm::agent::saved_key_for(&url) } else { p.needs_key() && config::has_key(p) };
        let target = config::CredentialTarget::for_provider(p, &url);
        let forget = target.is_some() && target.as_ref() == self.prefs.ai_forget_key.as_ref();
        if let Ok(k) = find::<PasswordBox>(&self.root, "WizKey") {
            let hint = if forget { "Key will be removed on Save" } else if has { "Saved key; leave empty to keep it" } else { "Paste an API key" };
            let _ = k.SetPlaceholderText(&hint.into());
        }
        shown(self, "WizForgetKey", has && !forget);
        shown(self, "WizRemoveAi", p == Provider::OpenAi && !self.prefs.ai_custom.is_empty());
    }

    fn wizard_saved_providers(&self) {
        let Ok(host) = find::<Border>(&self.root, "WizSavedHost") else { return };
        if self.prefs.ai_customs.is_empty() {
            let _ = host.SetChild(None::<&UIElement>);
            return;
        }
        let mut names = vec!["Choose a saved provider…"];
        names.extend(self.prefs.ai_customs.iter().map(|a| a.name.as_str()));
        let i = names.iter().position(|n| *n == self.prefs.ai_custom).unwrap_or(0);
        let x = labelled_row("WizSavedAi", "Saved providers", &combo("WizSavedAi", &names, i));
        let Ok(fe) = load::<FrameworkElement>(&x.replacen("<Grid ", &format!("<Grid {} ", ui::NS), 1)) else { return };
        let _ = host.SetChild(&fe);
        if let Ok(s) = find::<Selector>(&fe, "WizSavedAi") {
            if let Ok(t) = s.SelectionChanged(|sel, _| {
                if let Some(i) = sel.as_ref().and_then(|p| p.cast::<Selector>().ok()).and_then(|p| p.SelectedIndex().ok()).and_then(|i| usize::try_from(i).ok()) {
                    with(move |st| st.wizard_saved_provider(i));
                }
            }) { keep(t); }
        }
    }

    fn wizard_saved_provider(&mut self, index: usize) {
        let Some(a) = index.checked_sub(1).and_then(|i| self.prefs.ai_customs.get(i)).cloned() else { return };
        self.wizard_ai_changed(true);
        self.prefs.ai_custom = a.name.clone();
        set_field(self, "WizAiName", &a.name);
        set_field(self, "WizUrl", &a.base_url);
        set_field(self, "WizModel", &a.model);
        select(self, "WizProvider", Provider::ALL.iter().position(|p| *p == Provider::OpenAi).unwrap_or(0));
        self.wizard_provider();
    }

    /// A well-known service picked beside Base URL: its address in the box. Its models are asked
    /// for on Load models, as for a typed address.
    pub(crate) fn wizard_preset(&mut self) {
        let Some(url) = sprawl_advisor::llm::models::preset_url(selected(self, "WizUrlPreset")) else { return };
        if field(self, "WizUrl").trim() != url {
            self.wizard_ai_changed(true);
            set_field(self, "WizUrl", url);
            self.wiz_models.reset();
            self.wizard_models();
        }
    }

    /// The web-lookup switch shows only once a provider is chosen.
    fn wizard_provider_rows(&self) {
        let p = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
        if let Some(fe) = &self.prefs.fe {
            if let Ok(e) = find::<UIElement>(fe, "PfRow_aiWebLookup") {
                let _ = e.SetVisibility(if p == Provider::Off { Visibility::Collapsed } else { Visibility::Visible });
            }
        }
    }

    pub(crate) fn wizard_command(&mut self, name: &str) {
        match name {
            // dev (screenshot runs): WizProvider=1 picks the provider by its place in the list
            // (1 is Anthropic, which asks for a key), as choosing it from the dropdown does
            p if p.starts_with("WizProvider=") => {
                select(self, "WizProvider", p["WizProvider=".len()..].parse().unwrap_or(0));
                self.wizard_provider();
            }
            "WizBack" => {
                self.prefs_harvest();
                self.wizard_page = self.wizard_page.saturating_sub(1);
                self.wizard_show();
            }
            "WizUseDetected" => {
                if !self.wizard_request_allowed() { return; }
                let Some(ai) = self.prefs.ai_detected.clone() else { return; };
                self.wizard_ai_changed(true);
                self.prefs.ai_custom.clear();
                set_field(self, "WizAiName", "");
                let pi = Provider::ALL.iter().position(|&p| p == ai.provider).unwrap_or(0);
                select(self, "WizProvider", pi);
                set_field(self, "WizUrl", &ai.base_url);
                set_field(self, "WizModel", &ai.model);
                self.wizard_provider();
            }
            "WizLoadModels" => {
                if !self.wizard_request_allowed() {
                    return;
                }
                let p = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
                self.wiz_models.reset();
                self.wiz_models.want(p, &field(self, "WizUrl"), &typed_key(self));
                self.wizard_models();
            }
            "WizForgetKey" if !crate::settings::read_only() => {
                let p = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
                if p.needs_key() {
                    let Some(target) = config::CredentialTarget::for_provider(p, &field(self, "WizUrl")) else {
                        self.pref_error("ai", "Enter a valid server address before removing its key.".into());
                        return;
                    };
                    self.wizard_discard_secret();
                    self.prefs.ai_forget_key = Some(target);
                    self.wizard_key_hint(p);
                }
            }
            "WizRemoveAi" => {
                let name = std::mem::take(&mut self.prefs.ai_custom);
                self.prefs.ai_customs.retain(|a| a.name != name);
                self.prefs.ai_edited = true;
                set_field(self, "WizAiName", "");
                shown(self, "WizRemoveAi", false);
                self.wizard_saved_providers();
            }
            n if n.starts_with("PfSec") => {
                self.prefs_harvest();
                self.prefs.note = None;
                self.prefs.clear_armed = false;
                self.prefs.sec = n[5..].parse().unwrap_or(0);
                self.prefs.group = 0;
                self.prefs.list_page = 0;
                self.wizard_show();
            }
            "WizNext" if self.wizard_all => self.wizard_finish(),
            // Cancel: nothing is saved, and the look tried on the Theme page goes back
            "WizSkip" if self.wizard_all => self.wizard_cancel(),
            "WizNext" if self.wizard_page + 1 < PAGES => {
                self.prefs_harvest();
                self.wizard_page += 1;
                self.wizard_show();
            }
            "WizNext" => self.wizard_finish(),
            // skip setup (or the wizard's close button): don't ask again
            _ => self.wizard_cancel(),
        }
    }

    /// Apply what changed, through the same paths the menus use.
    fn wizard_finish(&mut self) {
        self.prefs_harvest();
        if crate::settings::read_only() {
            self.prefs.note = Some("Screenshot mode does not save preferences.".into());
            self.wizard_show();
            return;
        }
        let mut notes: Vec<String> = Vec::new();
        let changed: Vec<(&'static Pref, String)> = PREFS
            .iter()
            .filter(|p| p.key != "aiWebLookup")
            .filter_map(|p| {
                let v = self.prefs.draft.get(p.key)?;
                (self.prefs.orig.get(p.key) != Some(v)).then(|| (p, v.clone()))
            })
            .collect();
        for (p, v) in &changed {
            let error = match p.key {
                "scanExclude" => sprawl_advisor::exclusions::parse(v).err(),
                "programFolders" => sprawl_advisor::program_folders::parse(v).err(),
                "uninstaller" => uninstaller_path(v).err(),
                _ => match p.kind {
                    Kind::Number(k, unit) if choice_number(v, k.floor, k.max).is_none() => Some(format!("{}: enter a whole number from {} to {} {unit}.", p.label, k.floor, k.max)),
                    _ => None,
                },
            };
            if let Some(e) = error {
                self.pref_error(p.key, e);
                return;
            }
        }
        let web = switch_draft(&self.prefs.draft, "aiWebLookup");
        let key = typed_key(self);
        let name: String = field(self, "WizAiName").trim().chars().filter(|c| !c.is_control()).take(40).collect();
        let ai_edit = ai_needs_save(self.prefs.ai_edited || name != self.prefs.ai_orig.custom, self.prefs.ai_orig.web_lookup, web, &key);
        let mut ai = self.prefs.ai_orig.clone();
        if ai_edit {
            ai.provider = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
            ai.web_lookup = web;
            ai.model = self.wizard_model();
            ai.base_url = field(self, "WizUrl").trim().to_string();
            ai.custom = if ai.provider == Provider::OpenAi { name } else { String::new() };
            ai.customs = self.prefs.ai_customs.clone();
            ai.efforts = self.prefs.ai_efforts.clone();
            if ai.provider == Provider::OpenAi && !ai.base_url.is_empty() && sprawl_advisor::llm::agent::host_of(&ai.base_url).is_none() {
                self.pref_error("ai", "Enter a valid server address, or choose a service from the dropdown.".into());
                return;
            }
            if !ai.keep_custom(&self.prefs.ai_custom) {
                self.pref_error("ai", "Another saved provider already has that name.".into());
                return;
            }
        }
        let mut staged = self.settings.clone();
        for (p, v) in &changed {
            Self::stage_pref(&mut staged, p, v);
        }
        self.theme_stage(&mut staged);
        staged.set_bool("wizard_done", true);
        if let Err(e) = staged.save() {
            self.prefs.note = Some(format!("Couldn't save Settings: {e}. Your draft is still open."));
            self.wizard_show();
            return;
        }
        self.settings = staged;
        let putting = ai_edit && ai.provider.needs_key() && !key.trim().is_empty();
        let forget = self.prefs.ai_forget_key.take();
        let mut ai_changed = false;
        if (ai_edit && ai != self.prefs.ai_orig) || putting || forget.is_some() {
            if let Some(e) = &self.prefs.ai_load_error {
                notes.push(format!("AI settings and keys were not changed: the opening config couldn't be read ({e}). Reopen Settings after fixing it."));
            } else {
                match ai.save_edited(&self.prefs.ai_orig, putting, |saved| {
                    notes.extend(config::save_key_changes(saved, putting.then_some(key.as_str()), forget.as_ref()).into_iter().map(str::to_owned));
                }) {
                    Ok((changed, ())) => ai_changed = changed || putting || forget.is_some(),
                    Err(e) => notes.push(format!("AI settings and keys were not changed: {e}")),
                }
            }
        }
        // the administrator switches as they are before this save, to tell a real change
        let lists_changed = changed.iter().any(|(p, _)| p.key == "scanExclude" || p.key == "programFolders");
        let privacy_changed = changed.iter().any(|(p, _)| p.key == "privacyCleaners");
        let search_scope_changed = changed.iter().any(|(p, _)| matches!(p.key, "searchDrivesOn" | "searchSnapshots" | "snapshots"));
        let alert_changed = changed.iter().any(|(p, _)| matches!(p.key, "alertLow" | "alertGrowth"));
        if let Some((p, v)) = changed.iter().find(|(p, _)| p.key == "offline") { self.pref_apply(p, v); }
        let admin_before: Vec<bool> = [&crate::admin::CLAUDE_CODE, &crate::admin::USER_TOOLS].iter().map(|l| crate::admin::store::get(l.key)).collect();
        for (p, v) in changed {
            // into their store; a write that fails is said in the message, never "saved"
            if crate::admin::is_switch(p.key) {
                if let Err(e) = crate::admin::store::set(p.key, v == "1") {
                    notes.push(format!("{} is unchanged ({e})", p.label.to_lowercase()));
                }
                continue;
            }
            if p.key == "oExplorer" {
                if !crate::dialogs::set_explorer_menu(v == "1") {
                    notes.push("couldn't change Explorer's menu".into());
                }
                let _ = crate::menubar::set_checked(&self.root, p.key, crate::dialogs::explorer_menu_installed());
                continue;
            }
            if p.key != "offline" { self.pref_apply(p, &v); }
        }
        self.theme_commit();
        if alert_changed {
            let want = self.settings.bool("alertLow") == Some(true) || self.settings.bool("alertGrowth") == Some(true);
            if !crate::alert::schedule(want) {
                notes.push(if want { "Windows couldn't schedule the alert task; it will be retried next launch".into() } else { "Windows couldn't remove the alert task; the check exits with both alerts off".into() });
            }
        }
        if search_scope_changed { self.more_settings_changed(); }
        if privacy_changed { self.privacy_settings_changed(); } else if lists_changed { self.advisor_refresh(); }
        // the administrator switches take effect now, not at the next start
        let admin_after: Vec<bool> = [&crate::admin::CLAUDE_CODE, &crate::admin::USER_TOOLS].iter().map(|l| crate::admin::store::get(l.key)).collect();
        let admin_changed = admin_before != admin_after;
        self.admin_flags();
        if ai_changed || admin_changed {
            // an "Ask Claude" run under the old settings ends too, not only the chat
            self.advisor_command("AdvStop");
            self.chat_reset();
            if self.view == crate::state::View::Advisor {
                self.render_advisor();
            }
        }
        self.wizard_hide();
        let msg = if notes.is_empty() { "Settings saved.".to_string() } else { format!("Not all settings could be applied: {}.", notes.join("; ")) };
        text(&self.root, "FootLeft", &msg);
    }

    fn pref_error(&mut self, key: &str, message: String) {
        if self.wizard_all {
            for (sec, s) in Sec::ALL.into_iter().enumerate() {
                if let Some(group) = s.groups().iter().position(|g| g.1.contains(&key)) {
                    self.prefs.sec = sec;
                    self.prefs.group = group;
                    break;
                }
            }
        } else if let Some(i) = WIZARD.iter().position(|p| p.3.contains(&key)) {
            self.wizard_page = i;
        }
        self.prefs.note = Some(message);
        self.prefs.focus_name = Some(if key == "ai" { "WizUrl".into() } else { format!("Pf_{key}") });
        self.wizard_show();
        self.wizard_focus_arm(true);
    }
}

impl State {
    /// Show the Model dropdown if the list is here, else the text box (and why).
    pub(crate) fn wizard_models(&mut self) {
        let current = field(self, "WizModel");
        let choices = self.wizard_choices(&current);
        if let Ok(host) = find::<Border>(&self.root, "WizModelHost") {
            let display = |s: &str| if s.is_empty() { "Provider default".to_string() } else { s.to_string() };
            let combo = choices.as_ref().and_then(|list| {
                let list: Vec<_> = list.iter().map(|s| display(s)).collect();
                load::<UIElement>(&crate::models::combo("WizModelPick", &list, &display(&current), 0)
                    .replacen("<ComboBox ", &format!(r#"<ComboBox {} AutomationProperties.Name="Model" HorizontalAlignment="Stretch" "#, ui::NS), 1)
                    .replace(" Width=\"0\"", "")).ok()
            });
            let _ = host.SetChild(combo.as_ref());
            if let Some(sel) = combo.and_then(|c| c.cast::<Selector>().ok()) {
                if let Ok(t) = sel.SelectionChanged(|_, _| { with(|s| { s.prefs.ai_edited = true; s.wizard_effort(); }); }) { keep(t); }
            }
        }
        shown(self, "WizModel", choices.is_none());
        let note = self.wiz_models.note();
        text(&self.root, "WizModelNote", note.as_deref().unwrap_or(""));
        shown(self, "WizModelNote", note.is_some());
        self.wizard_effort();
    }

    fn wizard_choices(&self, current: &str) -> Option<Vec<String>> {
        let mut list = self.wiz_models.choices(current)?;
        if current.is_empty() { list.insert(0, String::new()); }
        Some(list)
    }

    fn wizard_effort(&self) {
        let Ok(host) = find::<Border>(&self.root, "WizEffortHost") else { return };
        let provider = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
        let model = self.wizard_model();
        let levels = sprawl_advisor::llm::effort::levels(provider, &field(self, "WizUrl"), &model);
        if levels.is_empty() {
            let _ = host.SetChild(None::<&UIElement>);
            return;
        }
        let default = sprawl_advisor::llm::effort::sprawl_default(provider, &levels)
            .map_or_else(|| "Default (provider)".to_string(), |d| format!("Default ({d})"));
        let mut names = vec![default.as_str()];
        names.extend(levels.iter().map(String::as_str));
        let pick = self.prefs.ai_efforts.get(&model).and_then(|v| levels.iter().position(|l| l == v)).map_or(0, |i| i + 1);
        let x = labelled_row("WizEffort", "Reasoning effort", &combo("WizEffort", &names, pick));
        let Ok(fe) = load::<FrameworkElement>(&x.replacen("<Grid ", &format!("<Grid {} ", ui::NS), 1)) else { return };
        let _ = host.SetChild(&fe);
        if let Ok(sel) = find::<Selector>(&fe, "WizEffort") {
            if let Ok(t) = sel.SelectionChanged(move |s, _| {
                if let Some(i) = s.as_ref().and_then(|s| s.cast::<Selector>().ok()).and_then(|s| s.SelectedIndex().ok()).and_then(|i| usize::try_from(i).ok()) {
                    let model = model.clone();
                    let level = i.checked_sub(1).and_then(|i| levels.get(i)).cloned();
                    with(move |st| {
                        st.prefs.ai_edited = true;
                        if let Some(level) = level { st.prefs.ai_efforts.insert(model, level); } else { st.prefs.ai_efforts.remove(&model); }
                    });
                }
            }) { keep(t); }
        }
    }

    /// The model chosen: the dropdown's entry, or what is typed.
    fn wizard_model(&self) -> String {
        let typed = field(self, "WizModel").trim().to_string();
        let pick = find::<Border>(&self.root, "WizModelHost").ok().and_then(|h| h.Child().ok()).and_then(|c| windows_core::Interface::cast::<Selector>(&c).ok()).and_then(|s| s.SelectedIndex().ok());
        match (pick, self.wizard_choices(&typed)) {
            (Some(i), Some(list)) => usize::try_from(i).ok().and_then(|i| list.get(i).cloned()).unwrap_or(typed),
            _ => typed,
        }
    }
}

#[cfg(test)]
mod card_tests {
    use super::{card_size, focus_state, xaml};
    use crate::bindings::Microsoft::UI::Xaml::FocusState;

    #[test]
    fn numeric_edits_reject_punctuation_overflow_and_out_of_range_input() {
        use super::choice_number as read;
        assert_eq!(read(" 15 ", 1, 30), Some(15));
        for invalid in ["1.5", "1,5", "1e1", "-1", " ", "0", "31", "18446744073709551616"] {
            assert_eq!(read(invalid, 1, 30), None, "{invalid}");
        }
    }

    #[test]
    fn unrelated_settings_do_not_write_ai_but_key_and_web_changes_do() {
        use super::ai_needs_save as needs;
        assert!(!needs(false, false, false, " "));
        assert!(!needs(false, true, true, ""));
        assert!(needs(true, false, false, ""));
        assert!(needs(false, false, true, ""));
        assert!(needs(false, false, false, "fixture key"));
    }

    #[test]
    fn either_saved_or_draft_offline_blocks_a_request() {
        use super::request_allowed as allowed;
        assert!(allowed(false, false));
        assert!(!allowed(true, false));
        assert!(!allowed(false, true));
        assert!(!allowed(true, true));
    }

    #[test]
    fn reset_all_restores_windows_motion_and_respects_admin_permissions() {
        use super::{map_animation_choice, reset_draft};
        let draft = [("searchDrivesOn", "D:\\"), ("searchColumns", "size"), ("oHeader", "0"),
            ("animate", "1"), ("adminMcpDisk", "1"), ("adminUserTools", "1"), ("neverTouch", "C:\\Fixture")];
        for elevated in [false, true] {
            let mut d = draft.into_iter().map(|(k, v)| (k, v.to_string())).collect();
            reset_draft(&mut d, None, elevated);
            assert_eq!((&*d["searchDrivesOn"], &*d["searchColumns"], &*d["oHeader"]), ("", "", "1"));
            assert_eq!(d["animate"], map_animation_choice(None).to_string());
            assert_eq!(d["adminMcpDisk"], if elevated { "0" } else { "1" });
            assert_eq!(d["adminUserTools"], if elevated { "0" } else { "1" });
            assert_eq!(d["neverTouch"], "C:\\Fixture", "Reset does not clear protected folders");
        }
    }

    #[test]
    fn the_last_keep_page_still_contains_every_entry() {
        let n = 213;
        let (first, pages) = super::page_range(n, 0);
        assert!(pages > 1 && first.start == 0);
        let all: Vec<_> = (0..pages).flat_map(|i| super::page_range(n, i).0).collect();
        assert_eq!(all, (0..n).collect::<Vec<_>>());
        assert_eq!(super::page_range(n, pages).0.end, n);
        assert!(!xaml().contains("<ScrollViewer"));
    }

    #[test]
    fn the_card_keeps_its_size_with_room_and_shrinks_without() {
        assert_eq!(card_size(true, 1680.0, 1080.0), (940.0, 680.0));
        assert_eq!(card_size(false, 1680.0, 1080.0), (640.0, 610.0));
        assert_eq!(card_size(true, 0.0, 0.0), (940.0, 680.0));
        assert_eq!(card_size(true, 1000.0, 560.0), (940.0, 512.0));
        assert_eq!(card_size(true, 760.0, 520.0), (712.0, 472.0));
        assert_eq!(card_size(false, 500.0, 300.0), (452.0, 252.0));
        assert_eq!(card_size(true, 100.0, 100.0), (240.0, 240.0));
    }

    /// The overlay covers the window, so Tab must stay in it: the root cycles the tab order, and
    /// the section list's entries are there for the focus to land on.
    #[test]
    fn tab_stays_inside_the_overlay_and_the_sections_can_take_the_focus() {
        let x = xaml();
        assert!(x.contains(r#"x:Name="Wizard""#));
        assert!(x.contains(r#"TabFocusNavigation="Cycle""#));
        for i in 0..super::Sec::ALL.len() {
            assert!(x.contains(&format!(r#"x:Name="PfSec{i}""#)));
        }
    }

    /// A click opens Settings without a focus ring; a key (Ctrl+Shift+S, or the menu's row with
    /// Enter) shows it.
    #[test]
    fn a_click_opens_without_a_focus_ring_and_the_keyboard_with_one() {
        assert_eq!(focus_state(false), FocusState::Pointer);
        assert_eq!(focus_state(true), FocusState::Keyboard);
    }
}
