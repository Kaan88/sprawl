//! First-run setup wizard (also Options > Setup Wizard): the AI advisor, scanning, the look of
//! the map and scan history. Every page only fills in choices that exist elsewhere too, so the
//! wizard can be skipped and everything changed later.

use sprawl_advisor::config::{self, AiConfig, Detected, Provider};

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::Selector;
use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, Control, PasswordBox, TextBox, ToggleMenuFlyoutItem, ToggleSwitch};
use crate::bindings::Microsoft::UI::Xaml::{UIElement, Visibility};
use crate::scene::Mode;
use crate::state::{find, text, State};

pub const PAGES: usize = 6;
const TITLES: [&str; PAGES] = ["Welcome", "AI advisor", "Scanning", "Look", "History", "Advisor"];
pub const CAPS_MB: [u32; 4] = [500, 1024, 2048, 5120];

/// Every control the wizard wires up: buttons, then lists.
pub const BUTTONS: [&str; 4] = ["WizBack", "WizNext", "WizSkip", "WizMcpCopy"];

fn switch(name: &str, header: &str, note: &str) -> String {
    format!(
        r##"<StackPanel Spacing="0"><ToggleSwitch x:Name="{name}" Header="{header}" FontSize="13"/><TextBlock Text="{note}" TextWrapping="Wrap" FontSize="12" Foreground="#FF8E97AA" Margin="0,-4,0,6"/></StackPanel>"##
    )
}

fn label(t: &str) -> String {
    format!(r##"<TextBlock Text="{t}" FontSize="12" Foreground="#FF9AA3B8" Margin="0,8,0,2"/>"##)
}

fn page_title(glyph: &str, title: &str, sub: &str) -> String {
    format!(
        r##"<StackPanel Orientation="Horizontal" Spacing="12" Margin="0,0,0,8"><Border Width="40" Height="40" CornerRadius="10" Background="#FF2C3B63"><FontIcon Glyph="{glyph}" FontSize="18" Foreground="#FFBFD4FF"/></Border><StackPanel VerticalAlignment="Center"><TextBlock Text="{title}" FontSize="20" FontWeight="SemiBold"/><TextBlock Text="{sub}" FontSize="12" Foreground="#FF9AA3B8" TextWrapping="Wrap"/></StackPanel></StackPanel>"##
    )
}

fn bullet(glyph: &str, title: &str, t: &str) -> String {
    format!(
        r##"<Grid ColumnSpacing="12" Margin="0,6,0,0"><Grid.ColumnDefinitions><ColumnDefinition Width="28"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><FontIcon Glyph="{glyph}" FontSize="16" Foreground="#FF8FB4FF" VerticalAlignment="Top" Margin="0,2,0,0"/><StackPanel Grid.Column="1"><TextBlock Text="{title}" FontWeight="SemiBold" FontSize="13"/><TextBlock Text="{t}" TextWrapping="Wrap" FontSize="12" Foreground="#FF9AA3B8"/></StackPanel></Grid>"##
    )
}

fn combo(name: &str, items: &[&str]) -> String {
    let items: String = items.iter().map(|i| format!(r##"<ComboBoxItem Content="{i}"/>"##)).collect();
    format!(r##"<ComboBox x:Name="{name}" HorizontalAlignment="Stretch" SelectedIndex="0">{items}</ComboBox>"##)
}

/// The overlay, placed over the whole window.
pub fn xaml() -> String {
    let providers: Vec<&str> = Provider::ALL.iter().map(|p| p.label()).collect();
    let palettes: Vec<&str> = crate::paint::PALETTES.to_vec();
    let p0 = format!(
        r##"<StackPanel x:Name="WizP0" Spacing="2">{}
        <TextBlock TextWrapping="Wrap" FontSize="13" Foreground="#FFC9D1E2" Margin="0,4,0,6" Text="A few choices before you start, all optional: each one can be changed later under Options &gt; Settings."/>
        {}{}{}{}</StackPanel>"##,
        page_title("&#xE7BE;", "Welcome to Sprawl", "Let's set it up the way you like it"),
        bullet("&#xE945;", "AI advisor", "Pick Claude, Claude Code or a local model to explain what is using your space. Optional."),
        bullet("&#xE721;", "Scanning", "Faster scans as administrator, and a right-click entry in Explorer."),
        bullet("&#xE790;", "Look", "Treemap, city or galaxy, colours, the mirror floor and live pulses."),
        bullet("&#xE81C;", "History", "Keep compact snapshots so you can see what grew over time."),
    );
    let p1 = format!(
        r##"<StackPanel x:Name="WizP1" Spacing="2" Visibility="Collapsed">{}
        <TextBlock TextWrapping="Wrap" FontSize="12.5" Foreground="#FFC9D1E2" Text="The built-in rules already find caches and leftovers. An AI can judge the rest: name unknown folders, add what it finds to your list, and answer questions. It only suggests: nothing goes until you tick it and press Clean up."/>
        <Border Background="#1A8FB4FF" CornerRadius="6" Padding="10,7" Margin="0,8,0,2"><StackPanel Orientation="Horizontal" Spacing="8"><FontIcon Glyph="&#xE721;" FontSize="13" Foreground="#FF8FB4FF"/><TextBlock x:Name="WizDetected" Text=" " FontSize="12" TextWrapping="Wrap" Foreground="#FFBFD4FF"/></StackPanel></Border>
        {}{}
        <StackPanel x:Name="WizKeyRow">{}<PasswordBox x:Name="WizKey" PlaceholderText="Paste your API key"/></StackPanel>
        <StackPanel x:Name="WizUrlRow">{}<TextBox x:Name="WizUrl" PlaceholderText="http://localhost:11434/v1"/></StackPanel>
        <StackPanel x:Name="WizModelRow">{}<Border x:Name="WizModelHost"/><TextBox x:Name="WizModel" PlaceholderText="Model name"/><TextBlock x:Name="WizModelNote" FontSize="12" Foreground="#FF8E97AA" TextWrapping="Wrap" Visibility="Collapsed"/></StackPanel>
        <TextBlock TextWrapping="Wrap" FontSize="12" Foreground="#FF8E97AA" Margin="0,10,0,0" Text="When the advisor runs, folder and file paths from your scan are sent to the provider as they are. Every request is listed in the advisor's log. Web lookups always ask first."/>
        </StackPanel>"##,
        page_title("&#xE945;", "AI advisor", "Optional: who should explain your disk?"),
        label("Provider"),
        combo("WizProvider", &providers),
        label("API key (kept in Windows Credential Manager, not in a file)"),
        label("Base URL"),
        label("Model"),
    );
    let p2 = format!(
        r##"<StackPanel x:Name="WizP2" Spacing="4" Visibility="Collapsed">{}{}{}</StackPanel>"##,
        page_title("&#xE721;", "Scanning", "How Sprawl reads your drives"),
        switch("WizAdmin", "Always run as administrator", "Reads the NTFS master file table directly: a whole drive in seconds instead of a minute or more. Windows asks for permission each time Sprawl starts."),
        switch("WizExplorer", "Add &quot;Scan with Sprawl&quot; to Explorer", "Right-click any folder or drive in Explorer to scan it. Only for your account; no administrator needed."),
    );
    let p3 = format!(
        r##"<StackPanel x:Name="WizP3" Spacing="2" Visibility="Collapsed">{}{}{}{}{}
        <Grid ColumnSpacing="16" Margin="0,8,0,0"><Grid.ColumnDefinitions><ColumnDefinition/><ColumnDefinition/></Grid.ColumnDefinitions>
          <StackPanel>{}{}</StackPanel>
          <StackPanel Grid.Column="1">{}</StackPanel>
        </Grid></StackPanel>"##,
        page_title("&#xE790;", "Look", "How the map is drawn"),
        label("Start in"),
        combo("WizView", &["Treemap: lit 3D tiles", "City: folders as buildings", "Galaxy: folders as stars"]),
        label("Colours"),
        combo("WizPalette", &palettes),
        switch("WizMirror", "Mirror floor", "A glossy reflection under the map."),
        switch("WizPulses", "Live change pulses", "Tiles pulse when their files change."),
        switch("WizAnim", "Animations", "Turn off for a calmer, lighter map."),
    );
    let p4 = format!(
        r##"<StackPanel x:Name="WizP4" Spacing="2" Visibility="Collapsed">{}{}{}{}
        <TextBlock TextWrapping="Wrap" FontSize="12" Foreground="#FF8E97AA" Margin="0,10,0,0" Text="Snapshots are kept in %LOCALAPPDATA%\Sprawl\snapshots on this PC only. The oldest are thinned out first when the limit is reached."/>
        </StackPanel>"##,
        page_title("&#xE81C;", "History", "See how your disk changes over time"),
        switch("WizSnapshots", "Keep scan history", "After each scan Sprawl saves a compact snapshot: about 5 MB for a whole system drive, and the last 3 in full detail (about 27 MB each). It powers growth over time, &quot;what grew since last week&quot;, the time-lapse and scan comparisons."),
        label("Space for history"),
        combo("WizCap", &["500 MB", "1 GB", "2 GB", "5 GB"]),
    );
    let p5 = format!(
        r##"<StackPanel x:Name="WizP5" Spacing="2" Visibility="Collapsed">{}{}
        <TextBox x:Name="WizUninstaller" PlaceholderText="A full path to its .exe (optional)"/>
        <TextBlock TextWrapping="Wrap" FontSize="12" Foreground="#FF8E97AA" Margin="0,4,0,0" Text="Uninstall... in the Advisor hands an app to it (Revo, Bulk Crap Uninstaller, and the like) instead of Windows' own uninstaller."/>
        {}<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock x:Name="WizMcp" FontFamily="Consolas" FontSize="12" Foreground="#FFB9C2D6" VerticalAlignment="Center" IsTextSelectionEnabled="True" TextTrimming="CharacterEllipsis" MaxWidth="440"/><Button x:Name="WizMcpCopy" Content="Copy" Padding="8,1" FontSize="12" CornerRadius="4"/></StackPanel>
        <TextBlock TextWrapping="Wrap" FontSize="12" Foreground="#FF8E97AA" Margin="0,4,0,0" Text="Run it once in a terminal; then your own Claude Code can ask Sprawl about your disk while Sprawl is open."/>
        </StackPanel>"##,
        page_title("&#xE90F;", "Advisor", "Your uninstaller, and Claude Code"),
        label("Uninstaller"),
        label("Use Sprawl from your own Claude Code"),
    );
    format!(
        r##"
  <Grid x:Name="Wizard" Grid.RowSpan="8" Visibility="Collapsed" Background="#B8080A10">
    <Border Width="620" Height="600" HorizontalAlignment="Center" VerticalAlignment="Center" CornerRadius="14" BorderBrush="#30FFFFFF" BorderThickness="1" Padding="28,22,28,20">
      <Border.Background><AcrylicBrush TintColor="#FF141824" TintOpacity="0.62" TintLuminosityOpacity="0.75" FallbackColor="#F2161A26"/></Border.Background>
      <Grid RowSpacing="14">
        <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
        <Grid>
          <TextBlock x:Name="WizStep" Text="Step 1 of 5" FontSize="12" Foreground="#FF8E97AA"/>
          <TextBlock x:Name="WizDots" Text="" FontSize="12" Foreground="#FF8FB4FF" HorizontalAlignment="Right"/>
        </Grid>
        <ScrollViewer Grid.Row="1" HorizontalScrollBarVisibility="Disabled" VerticalScrollBarVisibility="Auto" Padding="0,0,8,0">
          <StackPanel Spacing="18">{p0}{p1}{p2}{p3}{p4}{p5}</StackPanel>
        </ScrollViewer>
        <Grid Grid.Row="2">
          <Button x:Name="WizSkip" Background="Transparent" BorderThickness="0" Foreground="#FF9AA3B8"><TextBlock x:Name="WizSkipText" Text="Skip setup"/></Button>
          <StackPanel Orientation="Horizontal" Spacing="8" HorizontalAlignment="Right">
            <Button x:Name="WizBack" Content="Back" Padding="18,6" MinWidth="90"/>
            <Button x:Name="WizNext" Padding="18,6" MinWidth="110" Background="#FF3A6FE0" Foreground="White"><TextBlock x:Name="WizNextText" Text="Next"/></Button>
          </StackPanel>
        </Grid>
      </Grid>
    </Border>
  </Grid>"##
    )
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

fn set_switch(s: &State, name: &str, on: bool) {
    if let Ok(t) = find::<ToggleSwitch>(&s.root, name) {
        let _ = t.SetIsOn(on);
    }
}

fn switch_on(s: &State, name: &str) -> bool {
    find::<ToggleSwitch>(&s.root, name).and_then(|t| t.IsOn()).unwrap_or(false)
}

fn checked(s: &State, item: &str) -> bool {
    find::<ToggleMenuFlyoutItem>(&s.root, item).and_then(|t| t.IsChecked()).unwrap_or(false)
}

fn field(s: &State, name: &str) -> String {
    find::<TextBox>(&s.root, name).and_then(|t| t.Text()).map(|h| h.to_string_lossy()).unwrap_or_default()
}

fn set_field(s: &State, name: &str, v: &str) {
    if let Ok(t) = find::<TextBox>(&s.root, name) {
        let _ = t.SetText(&v.into());
    }
}

impl State {
    pub(crate) fn wizard_open(&mut self) {
        // AI: what was saved, or a suggestion from what is installed
        let det = Detected::probe();
        text(&self.root, "WizDetected", &det.summary());
        let saved = AiConfig::load();
        let first = self.settings.bool("wizard_done") != Some(true);
        // the wizard suggests what is installed; Settings shows what is saved, never more
        let ai = if first && !self.wizard_all && saved.provider == Provider::Off { det.suggest() } else { saved };
        let pi = Provider::ALL.iter().position(|&p| p == ai.provider).unwrap_or(0);
        select(self, "WizProvider", pi);
        set_field(self, "WizUrl", &ai.base_url);
        set_field(self, "WizModel", &ai.model);
        self.wizard_provider();

        set_switch(self, "WizAdmin", checked(self, "oAlwaysAdmin"));
        set_switch(self, "WizExplorer", checked(self, "oExplorer"));
        let view = match self.map.scene.mode {
            Mode::Map => 0,
            Mode::City => 1,
            Mode::Galaxy => 2,
        };
        select(self, "WizView", view);
        select(self, "WizPalette", self.settings.num("palette").unwrap_or(0.0) as usize);
        set_switch(self, "WizMirror", checked(self, "oMirror"));
        set_switch(self, "WizPulses", checked(self, "oPulses"));
        set_switch(self, "WizAnim", self.map.scene.animate);
        set_switch(self, "WizSnapshots", self.settings.bool("snapshots").unwrap_or(true));
        let cap = self.settings.num("snapshot_cap_mb").unwrap_or(1024.0) as u32;
        select(self, "WizCap", CAPS_MB.iter().position(|&c| c == cap).unwrap_or(1));
        set_field(self, "WizUninstaller", self.settings.get("uninstaller").unwrap_or("").to_string().as_str());
        text(&self.root, "WizMcp", &crate::bridge::setup_command());

        self.wizard_page = 0;
        self.wizard_show();
        shown(self, "Wizard", true);
    }

    /// Options > Settings: the wizard's choices on one page (without its welcome), saved at once.
    pub(crate) fn settings_open(&mut self) {
        self.wizard_all = true;
        self.wizard_open();
    }

    pub(crate) fn wizard_show(&self) {
        if self.wizard_all {
            for i in 0..PAGES {
                shown(self, &format!("WizP{i}"), i > 0);
            }
            text(&self.root, "WizStep", "Settings");
            text(&self.root, "WizDots", "");
            text(&self.root, "WizNextText", "Save");
            shown(self, "WizBack", false);
            text(&self.root, "WizSkipText", "Cancel");
            return;
        }
        shown(self, "WizBack", true);
        text(&self.root, "WizSkipText", "Skip setup");
        for i in 0..PAGES {
            shown(self, &format!("WizP{i}"), i == self.wizard_page);
        }
        let p = self.wizard_page;
        text(&self.root, "WizStep", &format!("Step {} of {PAGES}: {}", p + 1, TITLES[p]));
        text(&self.root, "WizDots", &(0..PAGES).map(|i| if i == p { "●" } else { "○" }).collect::<Vec<_>>().join(" "));
        text(&self.root, "WizNextText", if p + 1 == PAGES { "Finish" } else { "Next" });
        if let Ok(b) = find::<Control>(&self.root, "WizBack") {
            let _ = b.SetIsEnabled(p > 0);
        }
    }

    /// The provider list changed: show the fields that provider needs.
    pub(crate) fn wizard_provider(&mut self) {
        let p = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
        shown(self, "WizKeyRow", p.needs_key());
        shown(self, "WizUrlRow", p == Provider::OpenAi);
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
        // the list of models for this provider; a text box until (and unless) it arrives
        self.wiz_models.want(p, &field(self, "WizUrl"));
        self.wizard_models();
    }

    pub(crate) fn wizard_command(&mut self, name: &str) {
        match name {
            "WizBack" => {
                self.wizard_page = self.wizard_page.saturating_sub(1);
                self.wizard_show();
            }
            "WizMcpCopy" => {
                crate::win32::copy_text(self.hwnd, &crate::bridge::setup_command());
                text(&self.root, "FootLeft", "Copied the Claude Code setup command.");
            }
            "WizNext" if self.wizard_all => self.wizard_finish(),
            "WizSkip" if self.wizard_all => shown(self, "Wizard", false),
            "WizNext" if self.wizard_page + 1 < PAGES => {
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

    /// Apply every page through the same paths the menus use.
    fn wizard_finish(&mut self) {
        let mut notes = Vec::new();
        let p = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
        let ai = AiConfig { provider: p, model: self.wizard_model(), base_url: field(self, "WizUrl").trim().to_string() };
        if ai.save().is_err() {
            notes.push("couldn't save the AI settings");
        }
        let key = find::<PasswordBox>(&self.root, "WizKey").and_then(|k| k.Password()).map(|h| h.to_string_lossy()).unwrap_or_default();
        if p.needs_key() && !key.trim().is_empty() {
            if !config::set_key(p, &key) {
                notes.push("couldn't store the API key");
            }
            if let Ok(k) = find::<PasswordBox>(&self.root, "WizKey") {
                let _ = k.SetPassword(&"".into());
            }
        }

        for (sw, item) in [("WizAdmin", "oAlwaysAdmin"), ("WizExplorer", "oExplorer"), ("WizMirror", "oMirror"), ("WizPulses", "oPulses")] {
            let on = switch_on(self, sw);
            if checked(self, item) != on {
                if let Ok(t) = find::<ToggleMenuFlyoutItem>(&self.root, item) {
                    let _ = t.SetIsChecked(on);
                }
                self.command(item);
            }
        }
        // the start view, only when it changed (saving Settings mustn't leave a More view)
        let pick = selected(self, "WizView").min(2);
        let now = match self.map.scene.mode {
            Mode::Map => 0,
            Mode::City => 1,
            Mode::Galaxy => 2,
        };
        if !self.wizard_all || pick != now {
            self.command(["ModeMap", "ModeCity", "ModeGalaxy"][pick]);
        }
        select(self, "PaletteBox", selected(self, "WizPalette"));
        if let Ok(t) = find::<ToggleSwitch>(&self.root, "AnimSwitch") {
            let _ = t.SetIsOn(switch_on(self, "WizAnim"));
        }
        self.settings.set_bool("snapshots", switch_on(self, "WizSnapshots"));
        self.settings.set("snapshot_cap_mb", CAPS_MB[selected(self, "WizCap").min(CAPS_MB.len() - 1)]);
        self.settings.set("uninstaller", field(self, "WizUninstaller").trim().trim_matches('"'));
        self.settings.set_bool("wizard_done", true);
        self.settings.save();
        shown(self, "Wizard", false);
        let msg = if notes.is_empty() {
            "Settings saved. Change them any time under Options > Settings (Ctrl+Shift+S).".to_string()
        } else {
            format!("Setup saved, but {}.", notes.join(" and "))
        };
        text(&self.root, "FootLeft", &msg);
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

impl State {
    /// Each frame: when the typed address stops changing, fetch that server's list.
    pub(crate) fn wizard_url_poll(&mut self) {
        let open = find::<UIElement>(&self.root, "Wizard").and_then(|w| w.Visibility()).is_ok_and(|v| v == Visibility::Visible);
        if !open {
            return;
        }
        let p = Provider::ALL[selected(self, "WizProvider").min(Provider::ALL.len() - 1)];
        if p != Provider::OpenAi {
            return;
        }
        let url = field(self, "WizUrl");
        if self.wiz_url.settled(&url) {
            self.wiz_models.want(p, &url);
            self.wizard_models();
        }
    }
}
