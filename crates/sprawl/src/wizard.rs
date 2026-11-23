//! The first-run setup wizard and Settings: one overlay, both built from [`crate::prefs`]. The
//! wizard asks only what matters on the first day; Settings has everything, in sections, in
//! the same words. Both keep a draft of the values and apply it on Finish or Save, through the
//! same paths the menus use.

use std::collections::HashMap;

use sprawl_advisor::config::{self, AiConfig, Detected, Provider};

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, Selector};
use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, Control, PasswordBox, TextBox, ToggleSwitch};
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement, Visibility};
use crate::prefs::{self, Kind, Pref, Sec, PREFS, WIZARD};
use crate::state::{find, keep, load, text, with, State};
use crate::ui;

pub const PAGES: usize = WIZARD.len();

/// Every control of the overlay that is wired once: buttons, then the sections.
pub const BUTTONS: [&str; 15] = ["WizBack", "WizNext", "WizSkip", "WizLoadModels", "WizUseDetected", "PfSec0", "PfSec1", "PfSec2", "PfSec3", "PfSec4", "PfSec5", "PfSec6", "PfSec7", "PfSec8", "PfSec9"];

/// The overlay's state while it is open.
#[derive(Default)]
pub(crate) struct PrefUi {
    /// The Settings section shown.
    pub(crate) sec: usize,
    /// The values as chosen so far (key: value as text), and as they were when it opened.
    pub(crate) draft: HashMap<&'static str, String>,
    orig: HashMap<&'static str, String>,
    /// The settings on the page shown, read back into the draft before it changes.
    shown: Vec<&'static str>,
    /// The page's loaded content (its controls are found in it).
    pub(crate) fe: Option<FrameworkElement>,
    /// The fixed drives offered for Search, in the order shown.
    drives: Vec<String>,
    /// "Clear history" was pressed once: the next press clears.
    clear_armed: bool,
    /// What the last action said, shown at the top of the page.
    pub(crate) note: Option<String>,
    /// The Theme page's picks (themepage.rs).
    pub(crate) theme: crate::themepage::ThemeUi,
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
    format!(r##"<ComboBox x:Name="{name}" HorizontalAlignment="Left" Width="420" SelectedIndex="{pick}">{items}</ComboBox>"##)
}

fn button(name: &str, t: &str) -> String {
    format!(r##"<Button x:Name="{name}" Content="{}"/>"##, ui::esc(t))
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
    let ai_intro = ui::note_with("", "The built-in rules already find caches and leftovers.", "An AI can judge the rest: name unknown folders, suggest what to clean, and answer questions. It only suggests: nothing goes until you tick it, review it and confirm.");
    let ai_privacy = ui::note_with(r#"Margin="0,10,0,0""#, "When the AI works,", "folder and file paths from your scan go to the provider as they are, and it can read small text files (readme, licence, manifest) to tell what a folder is. Each request is in the AI request log. With Claude Code, what it looks at afterwards runs in Sprawl itself, on this PC. The AI searches the web only if you switch that on, and asks each time.");
    let ai = format!(
        r##"<StackPanel x:Name="WizAi" Spacing="2" Visibility="Collapsed">
        {ai_intro}
        <Border Background="{{$accentwash}}" CornerRadius="{{$rc6}}" Padding="10,7" Margin="0,8,0,2"><StackPanel Orientation="Horizontal" Spacing="8"><FontIcon Glyph="&#xE721;" FontSize="{{$fs13}}" Foreground="{{$link}}"/><TextBlock x:Name="WizDetected" Text=" " FontSize="{{$fs12}}" TextWrapping="Wrap" Foreground="{{$seltext}}" VerticalAlignment="Center" MaxWidth="460"/><Button x:Name="WizUseDetected" Content="Use it" Padding="8,1" FontSize="{{$fs12}}" CornerRadius="{{$rc4}}" Visibility="Collapsed"/></StackPanel></Border>
        {}{}
        <TextBlock x:Name="WizCodexRisk" Visibility="Collapsed" TextWrapping="Wrap" MaxWidth="520" HorizontalAlignment="Left" FontSize="{{$fs12}}" Foreground="{{$warn}}" Margin="0,4,0,2" Text="{}"/>
        <StackPanel x:Name="WizKeyRow">{}<PasswordBox x:Name="WizKey" PlaceholderText="Paste your API key" MaxWidth="420" HorizontalAlignment="Left" Width="420"/></StackPanel>
        <StackPanel x:Name="WizUrlRow">{}<Grid ColumnSpacing="8" MaxWidth="520" HorizontalAlignment="Left"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBox x:Name="WizUrl" PlaceholderText="https://api.openai.com/v1"/>{}</Grid></StackPanel>
        <StackPanel x:Name="WizModelRow" MaxWidth="520" HorizontalAlignment="Left" Width="520">{}<Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><Border x:Name="WizModelHost"/><TextBox x:Name="WizModel" PlaceholderText="Model name"/><Button x:Name="WizLoadModels" Grid.Column="1" Content="Load models" VerticalAlignment="Top" ToolTipService.ToolTip="Ask for the list of models, with the key typed above (or the saved one). A key goes only to the server it is for, and only over https (or to this PC)."/></Grid><TextBlock x:Name="WizModelNote" FontSize="{{$fs12}}" Foreground="{{$text3}}" TextWrapping="Wrap" Visibility="Collapsed"/></StackPanel>
        {ai_privacy}
        </StackPanel>"##,
        label("Provider"),
        format!(r##"<ComboBox x:Name="WizProvider" HorizontalAlignment="Left" Width="420" SelectedIndex="0">{}</ComboBox>"##, providers.iter().map(|i| format!(r##"<ComboBoxItem Content="{i}"/>"##)).collect::<String>()),
        crate::ui::esc(sprawl_advisor::llm::codex::RISK),
        label("API key (kept in Windows Credential Manager, not in a file)"),
        label("Base URL"),
        {
            let names = sprawl_advisor::llm::models::preset_names();
            crate::models::combo("WizUrlPreset", &names, &names[0], 170).replacen("<ComboBox ", r##"<ComboBox Grid.Column="1" ToolTipService.ToolTip="Fill in the address of a well-known service. Then Load models." "##, 1)
        },
        label("Model"),
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
  <Grid x:Name="Wizard" Grid.RowSpan="8" Visibility="Collapsed" Background="{{$scrim}}">
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
          <ScrollViewer x:Name="PfScroll" Grid.Column="1" HorizontalScrollBarVisibility="Disabled" VerticalScrollBarVisibility="Auto" Padding="0,0,12,0">
            <StackPanel Spacing="8"><Border x:Name="PfTop"/>{ai}<Border x:Name="PfHost"/></StackPanel>
          </ScrollViewer>
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

impl State {
    /// The setup wizard, from its first page.
    pub(crate) fn wizard_open(&mut self) {
        self.wizard_all = false;
        self.prefs_begin();
        self.wizard_page = 0;
        self.wizard_show();
        shown(self, "Wizard", true);
    }

    /// Options > Settings, at its first section.
    pub(crate) fn settings_open(&mut self) {
        self.settings_open_at("general");
    }

    /// Settings, at one section (`"ai"`, `"history"`...).
    pub(crate) fn settings_open_at(&mut self, section: &str) {
        self.wizard_all = true;
        self.prefs_begin();
        self.prefs.sec = Sec::by_id(section).and_then(|s| Sec::ALL.iter().position(|x| *x == s)).unwrap_or(0);
        self.wizard_show();
        shown(self, "Wizard", true);
    }

    /// Fill the draft from what is in use now, and the AI provider's controls.
    fn prefs_begin(&mut self) {
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
        self.theme_begin();
        // AI: what was saved (Off at first); what is installed is only offered, one click away
        let det = Detected::probe();
        let offer = det.suggest();
        let summary = match offer.provider {
            Provider::Off => det.summary(),
            p => format!("{}. Suggested: {}. The AI stays off unless you choose it.", det.summary(), p.label()),
        };
        text(&self.root, "WizDetected", &summary);
        shown(self, "WizUseDetected", offer.provider != Provider::Off);
        let ai = AiConfig::load();
        let pi = Provider::ALL.iter().position(|&p| p == ai.provider).unwrap_or(0);
        select(self, "WizProvider", pi);
        set_field(self, "WizUrl", &ai.base_url);
        set_field(self, "WizModel", &ai.model);
        self.wizard_provider();
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
            "animate" => b(self.map.scene.animate),
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
            "searchDrivesOn" => self.settings.get("searchDrivesOn").unwrap_or("").to_string(),
            // the administrator switches: in the store only an administrator can write
            k if crate::admin::is_switch(k) => b(crate::admin::store::get(k)),
            k => match &p.kind {
                Kind::Number(knob, _) => sprawl_advisor::tuning::get(knob).to_string(),
                Kind::Switch(d) => b(self.settings.bool(k).unwrap_or(*d)),
                _ => return dflt,
            },
        })
    }

    /// Put `v` (as the draft holds it) into use for `p`.
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
                select(self, "PaletteBox", i);
                self.tweak("PaletteBox", i as f64);
            }
            "animate" => {
                if let Ok(t) = find::<ToggleSwitch>(&self.root, "AnimSwitch") {
                    let _ = t.SetIsOn(yes);
                }
                self.tweak("AnimSwitch", if yes { 1.0 } else { 0.0 });
            }
            "aiWebLookup" => {
                let mut c = AiConfig::load();
                c.web_lookup = yes;
                let _ = c.save();
            }
            "afterScan" => self.settings.set("afterScan", idx(prefs::AFTER_SCAN.len())),
            // the page is made again for the new theme at the next frame (restyle.rs)
            "theme" => {
                self.settings.set("theme", idx(crate::theme::CHOICES.len()));
                crate::restyle::request();
            }
            "decimal" => {
                self.settings.set_bool("decimal", yes);
                crate::format::DECIMAL.store(yes, std::sync::atomic::Ordering::Relaxed);
                self.refresh_rows();
                self.refresh_exts();
                if let Some(t) = &self.tree { let root = t.root_path.clone(); self.update_header(&root); }
                if self.view == crate::state::View::Advisor { self.render_advisor(); }
                self.map.dirty = true;
                self.map.views.stamp = 0;
            }
            "startTab" => self.settings.set("startTab", idx(prefs::OPEN_ON.len())),
            "alertFreeGb" => self.settings.set("alertFreeGb", idx(prefs::ALERT_AT.len())),
            "alertGrowGb" => self.settings.set("alertGrowGb", idx(prefs::GROW_AT.len())),
            "alertLow" | "alertGrowth" => {
                self.settings.set_bool(p.key, yes);
                // one task serves both; it goes when both are off
                let want = self.settings.bool("alertLow") == Some(true) || self.settings.bool("alertGrowth") == Some(true);
                if !crate::alert::schedule(want) && yes {
                    self.settings.set_bool(p.key, false);
                }
            }
            "snapshot_cap_mb" => self.settings.set("snapshot_cap_mb", prefs::CAPS_MB[idx(prefs::CAPS_MB.len())]),
            "uninstaller" => self.settings.set("uninstaller", v.trim().trim_matches('"')),
            "scanExclude" => self.settings.set("scanExclude", v.trim()),
            "searchDrivesOn" => {
                self.settings.set("searchDrivesOn", v);
                self.settings.set("searchDrivesOff", "");
                self.more_settings_changed();
            }
            "communityFetch" => {
                self.advisor.memory_ready();
                self.advisor.memory.community_off = !yes;
                self.save_memory();
                self.advisor_start();
            }
            "offline" => {
                self.settings.set_bool("offline", yes);
                sprawl_advisor::OFFLINE.store(yes, std::sync::atomic::Ordering::Relaxed);
            }
            // written by the save itself (wizard_finish), which says when a write fails
            k if crate::admin::is_switch(k) => {}
            k => match &p.kind {
                Kind::Number(knob, _) => {
                    let n = v.trim().replace([',', '.', ' '], "").parse::<u64>().unwrap_or(knob.default);
                    let used = sprawl_advisor::tuning::set(knob, n);
                    self.settings.set(k, used);
                }
                Kind::Switch(_) => self.settings.set_bool(k, yes),
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
            let mut keys: Vec<&'static str> = PREFS.iter().filter(|p| p.sec == sec && !p.advanced).chain(PREFS.iter().filter(|p| p.sec == sec && p.advanced)).map(|p| p.key).collect();
            // the Theme page is built by themepage.rs, with its own controls
            if sec == Sec::Theme {
                keys.clear();
            }
            (page_title(sec.glyph(), sec.title(), ""), keys)
        } else {
            let (title, glyph, sub, keys) = WIZARD[self.wizard_page.min(PAGES - 1)];
            let title = if self.wizard_page == 0 { "Welcome to Sprawl" } else { title };
            (page_title(glyph, title, sub), keys.to_vec())
        };
        if let (Ok(host), Ok(el)) = (find::<Border>(&self.root, "PfTop"), load::<UIElement>(&top)) {
            let _ = host.SetChild(&el);
        }
        shown(self, "WizAi", keys.contains(&"ai"));
        let mut x = format!(r##"<StackPanel {} Spacing="4">"##, ui::NS);
        if let Some(n) = &self.prefs.note {
            x += &format!(r##"<Border Padding="10,6" CornerRadius="{{$rc6}}" Background="{{$goodwash}}"><TextBlock Text="{}" TextWrapping="Wrap" FontSize="{{$fs12}}"/></Border>"##, ui::esc(n));
        }
        if !self.wizard_all && self.wizard_page == 0 {
            x += r##"<TextBlock TextWrapping="Wrap" FontSize="{$fs13}" Foreground="{$text1}" Margin="0,4,0,6" Text="A few choices before you start, all optional: nothing that sends or keeps data is on unless you choose it, and each one is in Settings (Ctrl+Shift+S) too."/>"##;
            for (t, g, s, _) in WIZARD.iter().skip(1) {
                x += &bullet(g, t, s);
            }
        }
        let mut advanced_said = false;
        let mut buttons = Vec::new();
        for k in &keys {
            let Some(p) = prefs::get(k) else { continue };
            if p.key == "ai" {
                continue;
            }
            if p.advanced && !advanced_said {
                advanced_said = true;
                x += r##"<TextBlock Text="Advanced" FontSize="{$fs14}" FontWeight="SemiBold" Margin="0,14,0,0"/>"##;
                x += &ui::note("Thresholds with a floor:", "none can make something look safer or be ticked for you.");
            }
            x += &self.pref_row(p, &mut buttons);
        }
        let theme_page = self.wizard_all && Sec::ALL[self.prefs.sec.min(Sec::ALL.len() - 1)] == Sec::Theme;
        if theme_page {
            let (tx, tb) = self.theme_page();
            x += &tx;
            buttons.extend(tb);
        } else if self.wizard_all {
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
            // "Work offline" greys what it stops, at once
            if let Ok(t) = find::<ToggleSwitch>(fe, "Pf_offline") {
                if let Ok(tok) = t.Toggled(|_, _| {
                    with(|s| s.prefs_offline_shown());
                }) {
                    keep(tok);
                }
            }
        }
        self.prefs.fe = fe;
        self.prefs_offline_shown();
        self.wizard_provider_rows();
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
            Kind::Switch(_) => format!("{admin_note}{}", row(format!(r##"<ToggleSwitch x:Name="{name}" Header="{}" IsOn="{}" FontSize="{{$fs13}}"{enabled}/>{}"##, ui::esc(p.label), on(&v), if p.help_line().is_empty() { String::new() } else { help(&p.help_line()) }))),
            Kind::Number(_, unit) => row(format!(
                r##"{}<StackPanel Orientation="Horizontal" Spacing="8"><TextBox x:Name="{name}" Width="130" Text="{}" InputScope="Number"/><TextBlock Text="{}" VerticalAlignment="Center" FontSize="{{$fs12}}" Foreground="{{$text2}}"/></StackPanel>{}"##,
                label(p.label),
                ui::esc(&v),
                ui::esc(unit),
                help(&p.help_line())
            )),
            Kind::Choice(items, _) => row(format!("{}{}{}", label(p.label), combo(&name, items, v.parse().unwrap_or(0)), help(p.help))),
            Kind::Text(ph) => row(format!(r##"{}<TextBox x:Name="{name}" Text="{}" PlaceholderText="{}" Width="520" HorizontalAlignment="Left"/>{}"##, label(p.label), ui::esc(&v), ui::esc(ph), help(p.help))),
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
                x += &format!(r##"<StackPanel Orientation="Horizontal" Spacing="8">{}{}</StackPanel>"##, b("PfB_data", "Open the data folder"), b("PfB_resetAll", "Reset everything"));
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
                let on: Vec<String> = v.split(',').map(str::trim).filter(|d| !d.is_empty()).map(str::to_lowercase).collect();
                self.prefs.drives = State::fixed_drives_pub();
                for (i, d) in self.prefs.drives.iter().enumerate() {
                    x += &format!(r##"<ToggleSwitch x:Name="PfDrv{i}" OnContent="{0}" OffContent="{0}" IsOn="{1}" FontSize="{{$fs13}}"/>"##, ui::esc(d), if on.contains(&d.to_lowercase()) { "True" } else { "False" });
                }
                if self.prefs.drives.is_empty() {
                    x += &help("No fixed drive found.");
                }
            }
            "neverTouch" => {
                x += &help(p.help);
                self.advisor.memory_ready();
                let kept = self.advisor.memory.kept_paths();
                if kept.is_empty() {
                    x += &help("Nothing yet. Keep on a finding adds it here.");
                }
                for (i, k) in kept.iter().enumerate().take(200) {
                    x += &format!(
                        r##"<Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBlock Text="{}" FontFamily="Consolas" FontSize="{{$fs12}}" Foreground="{{$text2}}" TextTrimming="CharacterEllipsis" VerticalAlignment="Center"/><Button x:Name="PfB_unkeep{i}" Grid.Column="1" Content="Remove" Padding="8,1" FontSize="{{$fs12}}" CornerRadius="{{$rc4}}"/></Grid>"##,
                        ui::esc(&k.display().to_string())
                    );
                    buttons.push(format!("PfB_unkeep{i}"));
                }
                x += &format!(
                    r##"<Grid ColumnSpacing="8" Margin="0,4,0,0" MaxWidth="560" HorizontalAlignment="Left"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBox x:Name="PfKeepPath" PlaceholderText="A folder or file to keep, like D:\Photos"/><Button x:Name="PfB_keepAdd" Grid.Column="1" Content="Add"/></Grid>"##
                );
                buttons.push("PfB_keepAdd".into());
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
                let list = sprawl_advisor::regclean::backups();
                if list.is_empty() {
                    x += &help("No backups yet. They are made when you clean a registry list in Cleaners.");
                }
                for (i, bk) in list.iter().enumerate() {
                    let when = sprawl_core::local_time(std::time::UNIX_EPOCH + std::time::Duration::from_secs(bk.when));
                    x += &format!(
                        r##"<Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text2}}" VerticalAlignment="Center" TextTrimming="CharacterEllipsis"/><StackPanel Grid.Column="1" Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" VerticalAlignment="Center"/>{}</StackPanel></Grid>"##,
                        ui::esc(&bk.title),
                        ui::esc(&format!("{when} · {}", if bk.values == 1 { "1 value".to_string() } else { format!("{} values", bk.values) })),
                        b(&format!("PfB_regBack{i}"), "Put back")
                    );
                }
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
                Kind::Choice(..) => find::<Selector>(&fe, &name).and_then(|s| s.SelectedIndex()).ok().map(|i| i.max(0).to_string()),
                Kind::Custom if k == "searchDrivesOn" => {
                    let on: Vec<String> = self.prefs.drives.iter().enumerate().filter(|(i, _)| find::<ToggleSwitch>(&fe, &format!("PfDrv{i}")).and_then(|t| t.IsOn()).unwrap_or(false)).map(|(_, d)| d.clone()).collect();
                    Some(on.join(","))
                }
                Kind::Custom => None,
            };
            if let Some(v) = v {
                self.prefs.draft.insert(k, v);
            }
        }
    }

    /// "Work offline" stops the network switches and the AI provider: shown greyed.
    fn prefs_offline_shown(&self) {
        let off = self
            .prefs
            .fe
            .as_ref()
            .and_then(|fe| find::<ToggleSwitch>(fe, "Pf_offline").and_then(|t| t.IsOn()).ok())
            .unwrap_or_else(|| self.prefs.draft.get("offline").is_some_and(|v| v == "1"));
        if let Some(fe) = &self.prefs.fe {
            for n in ["Pf_oUpdates", "Pf_communityFetch", "Pf_aiWebLookup"] {
                if let Ok(c) = find::<Control>(fe, n) {
                    let _ = c.SetIsEnabled(!off);
                }
            }
        }
        if let Ok(c) = find::<Control>(&self.root, "WizProvider") {
            let _ = c.SetIsEnabled(!off);
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
            "PfB_reset" | "PfB_resetAll" => {
                let sec = Sec::ALL[self.prefs.sec.min(Sec::ALL.len() - 1)];
                // the administrator switches only change where they can be saved: elevated
                for p in PREFS.iter().filter(|p| name == "PfB_resetAll" || p.sec == sec).filter(|p| !crate::admin::is_switch(p.key) || sprawl_core::is_elevated()) {
                    if let Some(d) = p.default_value() {
                        self.prefs.draft.insert(p.key, d);
                    }
                }
                if name == "PfB_resetAll" {
                    self.theme_reset();
                }
                if name == "PfB_resetAll" || sec == Sec::Ai {
                    select(self, "WizProvider", 0);
                    self.wizard_provider();
                }
                self.prefs.note = Some(if name == "PfB_resetAll" { "Everything is back to its default. Press Save to keep that, or Cancel." } else { "This section is back to its defaults. Press Save to keep that, or Cancel." }.into());
            }
            "PfB_data" => {
                if let Some(d) = std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("Sprawl")) {
                    let _ = std::fs::create_dir_all(&d);
                    crate::win32::reveal(&d);
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
                let mut n = 0;
                if let Some(base) = sprawl_advisor::history::base_dir() {
                    for e in sprawl_advisor::history::list(&base) {
                        // only History's own files, in its own folder
                        if sprawl_advisor::history::remove_entry(&e).is_ok() {
                            n += 1;
                        }
                    }
                }
                self.prefs.note = Some(format!("Deleted {n} snapshots of History."));
            }
            "PfB_keepAdd" => {
                let fe = self.prefs.fe.clone();
                let typed = fe.as_ref().and_then(|fe| find::<TextBox>(fe, "PfKeepPath").and_then(|t| t.Text()).ok()).map(|h| h.to_string_lossy()).unwrap_or_default();
                let path = std::path::PathBuf::from(typed.trim().trim_matches('"'));
                if path.is_absolute() && path.exists() {
                    self.advisor.memory_ready();
                    let title = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                    self.advisor.memory.decide(&path, &title, sprawl_advisor::memory::Verdict::Keep);
                    self.save_memory();
                    self.prefs.note = Some(format!("Sprawl will never offer {}.", path.display()));
                } else {
                    self.prefs.note = Some("Type the full path of a folder or file that exists, like D:\\Photos.".into());
                }
            }
            n if n.starts_with("PfB_regBack") => {
                let i: usize = n["PfB_regBack".len()..].parse().unwrap_or(usize::MAX);
                match sprawl_advisor::regclean::backups().get(i) {
                    Some(bk) => match sprawl_advisor::regclean::restore(&bk.path, Some(&bk.part)) {
                        Ok(v) => {
                            self.refresh_registry_counts();
                            self.prefs.note = Some(format!("Put back {v} {} of {}. Values written since stay.", if v == 1 { "value" } else { "values" }, bk.title));
                        }
                        Err(e) => self.prefs.note = Some(format!("The backup couldn't be put back: {e}")),
                    },
                    None => self.prefs.note = Some("That backup is no longer there.".into()),
                }
            }
            n if n.starts_with("PfB_unkeep") => {
                let i: usize = n["PfB_unkeep".len()..].parse().unwrap_or(usize::MAX);
                self.advisor.memory_ready();
                if let Some(p) = self.advisor.memory.kept_paths().get(i).cloned() {
                    self.advisor.memory.undo(&p);
                    self.save_memory();
                    self.prefs.note = Some(format!("{} can be offered again (never ticked for you).", p.display()));
                }
            }
            _ => {}
        }
        self.prefs_render();
    }

    /// The provider list changed: show the fields that provider needs.
    pub(crate) fn wizard_provider(&mut self) {
        let p = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
        // a key typed for one provider never goes to another (Fable 1.0.4-1.0.5 P2-4)
        thread_local! { static TYPED_FOR: std::cell::Cell<Option<Provider>> = const { std::cell::Cell::new(None) }; }
        if TYPED_FOR.with(|t| t.replace(Some(p))).is_some_and(|q| q != p) {
            if let Ok(k) = find::<PasswordBox>(&self.root, "WizKey") {
                let _ = k.SetPassword(&"".into());
            }
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
        // an empty model, or another provider's default, becomes this provider's default
        let m = field(self, "WizModel").trim().to_string();
        if p != Provider::Off && (m.is_empty() || Provider::ALL.iter().any(|q| q.default_model() == m)) {
            set_field(self, "WizModel", p.default_model());
        }
        if let Ok(k) = find::<PasswordBox>(&self.root, "WizKey") {
            let hint = if p.needs_key() && config::has_key(p) { "A key is saved. Leave empty to keep it." } else { "Paste your API key" };
            let _ = k.SetPlaceholderText(&hint.into());
        }
        // the list of models for this provider; a text box until (and unless) it arrives. An
        // OpenAI-compatible address is asked only on "Load models", never while it is typed;
        // nothing is asked while working offline
        if p != Provider::OpenAi && !sprawl_advisor::offline() {
            self.wiz_models.want(p, &field(self, "WizUrl"), &typed_key(self));
        }
        self.wizard_models();
        self.wizard_provider_rows();
    }

    /// A well-known service picked beside Base URL: its address in the box. Its models are asked
    /// for on Load models, as for a typed address.
    pub(crate) fn wizard_preset(&mut self) {
        let Some(url) = sprawl_advisor::llm::models::preset_url(selected(self, "WizUrlPreset")) else { return };
        if field(self, "WizUrl").trim() != url {
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
                let ai = Detected::probe().suggest();
                let pi = Provider::ALL.iter().position(|&p| p == ai.provider).unwrap_or(0);
                select(self, "WizProvider", pi);
                set_field(self, "WizUrl", &ai.base_url);
                set_field(self, "WizModel", &ai.model);
                self.wizard_provider();
            }
            "WizLoadModels" => {
                if sprawl_advisor::offline() {
                    return;
                }
                let p = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
                self.wiz_models.reset();
                self.wiz_models.want(p, &field(self, "WizUrl"), &typed_key(self));
                self.wizard_models();
            }
            n if n.starts_with("PfSec") => {
                self.prefs_harvest();
                self.prefs.note = None;
                self.prefs.clear_armed = false;
                self.prefs.sec = n[5..].parse().unwrap_or(0);
                self.wizard_show();
            }
            "WizNext" if self.wizard_all => self.wizard_finish(),
            "WizSkip" if self.wizard_all => {
                // the look tried on the Theme page goes back to the one in use
                self.theme_cancel();
                shown(self, "Wizard", false);
            }
            "WizNext" if self.wizard_page + 1 < PAGES => {
                self.prefs_harvest();
                self.wizard_page += 1;
                self.wizard_show();
            }
            "WizNext" => self.wizard_finish(),
            _ => {
                // skip: don't ask again
                self.settings.set_bool("wizard_done", true);
                self.settings.save();
                shown(self, "Wizard", false);
            }
        }
    }

    /// Apply what changed, through the same paths the menus use.
    fn wizard_finish(&mut self) {
        self.prefs_harvest();
        let mut notes: Vec<String> = Vec::new();
        if let Err(e) = sprawl_advisor::exclusions::parse(self.prefs.draft.get("scanExclude").map_or("", String::as_str)) {
            self.prefs.note = Some(e);
            self.wizard_show();
            return;
        }
        let p = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
        let web = self.prefs.draft.get("aiWebLookup").is_some_and(|v| v == "1");
        // the effort picked per model is set in the Advisor's AI settings, and kept here
        let old = AiConfig::load();
        let base_url = field(self, "WizUrl").trim().to_string();
        // the custom AIs are named in the Advisor's AI settings; the one in use keeps its name
        // while its address stays
        let custom = if p == Provider::OpenAi && base_url == old.base_url { old.custom.clone() } else { String::new() };
        let ai = AiConfig { provider: p, model: self.wizard_model(), base_url, web_lookup: web, efforts: old.efforts.clone(), custom, customs: old.customs.clone() };
        // a chat under way was started with the old AI settings (provider, model, web lookup):
        // the next question starts a new one, so a change takes effect at once, without a restart
        let ai_changed = AiConfig::load() != ai;
        if ai.save().is_err() {
            notes.push("couldn't save the AI settings".into());
        }
        let key = find::<PasswordBox>(&self.root, "WizKey").and_then(|k| k.Password()).map(|h| h.to_string_lossy()).unwrap_or_default();
        if let Some(w) = (p == Provider::OpenAi && key.trim().is_empty()).then(|| sprawl_advisor::llm::agent::key_elsewhere(&ai.base_url)).flatten() {
            notes.push(w);
        }
        if p.needs_key() && !key.trim().is_empty() {
            if !config::set_key(p, &key, &ai.base_url) {
                notes.push("couldn't store the API key".into());
            }
            if let Ok(k) = find::<PasswordBox>(&self.root, "WizKey") {
                let _ = k.SetPassword(&"".into());
            }
        }
        // what changed, and only that ("aiWebLookup" went with the AI settings above)
        let changed: Vec<(&'static Pref, String)> = PREFS
            .iter()
            .filter(|p| p.key != "aiWebLookup")
            .filter_map(|p| {
                let v = self.prefs.draft.get(p.key)?;
                (self.prefs.orig.get(p.key) != Some(v)).then(|| (p, v.clone()))
            })
            .collect();
        // the administrator switches as they are before this save, to tell a real change
        let exclusions_changed = changed.iter().any(|(p, _)| p.key == "scanExclude");
        let admin_before: Vec<bool> = [&crate::admin::CLAUDE_CODE, &crate::admin::USER_TOOLS].iter().map(|l| crate::admin::store::get(l.key)).collect();
        for (p, v) in changed {
            // into their store; a write that fails is said in the message, never "saved"
            if crate::admin::is_switch(p.key) {
                if let Err(e) = crate::admin::store::set(p.key, v == "1") {
                    notes.push(format!("{} is unchanged ({e})", p.label.to_lowercase()));
                }
                continue;
            }
            if let Kind::Number(knob, unit) = &p.kind {
                let n = v.trim().replace([',', '.', ' '], "").parse::<u64>();
                match n {
                    Ok(n) if knob.clamp(n) != n => notes.push(format!("{} was held at {} {unit}", p.label.to_lowercase(), knob.clamp(n))),
                    Err(_) => notes.push(format!("{} wasn't a number, so it stays {} {unit}", p.label.to_lowercase(), knob.default)),
                    _ => {}
                }
            }
            self.pref_apply(p, &v);
        }
        self.theme_commit();
        self.settings.set_bool("wizard_done", true);
        self.settings.save();
        if exclusions_changed { self.advisor_refresh(); }
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
        shown(self, "Wizard", false);
        let msg = if notes.is_empty() { "Settings saved. Change them any time under Options > Settings (Ctrl+Shift+S).".to_string() } else { format!("Settings saved; {}.", notes.join("; ")) };
        text(&self.root, "FootLeft", &msg);
        self.prefs.fe = None;
    }
}

impl State {
    /// Show the Model dropdown if the list is here, else the text box (and why).
    pub(crate) fn wizard_models(&mut self) {
        let current = field(self, "WizModel");
        let choices = self.wiz_models.choices(&current);
        if let Ok(host) = find::<Border>(&self.root, "WizModelHost") {
            let combo = choices.as_ref().and_then(|list| crate::state::load::<UIElement>(&crate::models::combo("WizModelPick", list, &current, 0).replacen("<ComboBox ", &format!("<ComboBox {} HorizontalAlignment=\"Stretch\" ", crate::ui::NS), 1).replace(" Width=\"0\"", "")).ok());
            let _ = host.SetChild(combo.as_ref());
        }
        shown(self, "WizModel", choices.is_none());
        let note = self.wiz_models.note();
        text(&self.root, "WizModelNote", note.as_deref().unwrap_or(""));
        shown(self, "WizModelNote", note.is_some());
    }

    /// The model chosen: the dropdown's entry, or what is typed.
    fn wizard_model(&self) -> String {
        let typed = field(self, "WizModel").trim().to_string();
        let pick = find::<Border>(&self.root, "WizModelHost").ok().and_then(|h| h.Child().ok()).and_then(|c| windows_core::Interface::cast::<Selector>(&c).ok()).and_then(|s| s.SelectedIndex().ok());
        match (pick, self.wiz_models.choices(&typed)) {
            (Some(i), Some(list)) => usize::try_from(i).ok().and_then(|i| list.get(i).cloned()).unwrap_or(typed),
            _ => typed,
        }
    }
}

#[cfg(test)]
mod card_tests {
    use super::card_size;

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
}
