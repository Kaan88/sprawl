//! The Theme page of Settings: a sample that follows each pick, the presets, and the options in four groups. A preset is only a set of option values; Custom is the
//! user's own picks, kept apart so a preset tried and left does not lose them. The options are
//! kept in `settings.ini` as one `look.<option>` key each (a missing key is Sprawl's own value),
//! read before any XAML loads, and applied live on Save through [`crate::restyle::apply_look`].
//! Copy theme and Paste theme use the same one-line text; a pasted line is outside text, so
//! only known options with known values are taken from it.

use windows_core::{Interface, HSTRING};

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, Selector};
use crate::bindings::Microsoft::UI::Xaml::Controls::Border;
use crate::bindings::Microsoft::UI::Xaml::Markup::XamlReader;
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement};
use crate::settings::Settings;
use crate::state::{find, keep, with, State};
use crate::tokens::{self, Bars, Look, SmallText, Ticks};
use crate::ui;

/// One option on the page: its key in a copied theme, its label, and its choices as (name in a
/// copied theme, words on the page), Sprawl's own first. `mode` (light or dark) is not in
/// [`Look`]: it is the Theme setting that was there before this page.
pub(crate) struct Opt {
    pub key: &'static str,
    pub label: &'static str,
    pub group: usize,
    pub choices: &'static [(&'static str, &'static str)],
}

pub(crate) const GROUPS: [&str; 4] = ["Colour", "Shape", "Feel", "Text and space"];

/// Only options that exist are here; each group holds at most eight.
pub(crate) const OPTS: [Opt; 10] = [
    Opt { key: "mode", label: "Light or dark", group: 0, choices: &[("like", "Like Windows"), ("light", "Light"), ("dark", "Dark")] },
    Opt { key: "accent", label: "Accent", group: 0, choices: &[("windows", "Windows accent"), ("azure", "Azure"), ("iris", "Iris"), ("violet", "Violet"), ("sunset", "Sunset")] },
    Opt { key: "surfaces", label: "Surfaces", group: 0, choices: &[("ink", "Ink"), ("graphite", "Graphite"), ("tinted", "Tinted"), ("black", "Black"), ("slate", "Slate"), ("warmgrey", "Warm grey")] },
    Opt { key: "contrast", label: "Contrast", group: 0, choices: &[("normal", "Normal"), ("strong", "Strong")] },
    Opt { key: "folders", label: "Folder icons", group: 0, choices: &[("classic", "Classic yellow"), ("accent", "Accent"), ("grey", "Grey")] },
    Opt { key: "corners", label: "Corners", group: 1, choices: &[("four8", "4 / 8"), ("six10", "6 / 10"), ("eight14", "8 / 14"), ("pills", "Pills"), ("square", "Square")] },
    Opt { key: "ticks", label: "Tick boxes", group: 1, choices: &[("square", "Square"), ("round", "Round")] },
    Opt { key: "bars", label: "Bars", group: 1, choices: &[("solid", "Solid"), ("gradient", "Gradient"), ("thin", "Thin line")] },
    Opt { key: "motion", label: "Motion", group: 2, choices: &[("subtle", "Subtle"), ("lively", "Lively"), ("none", "None")] },
    Opt { key: "smalltext", label: "Small text", group: 3, choices: &[("leadwords", "13 px, lead words"), ("plain", "13 px"), ("grey12", "12 px, grey")] },
];

pub(crate) const PRESETS: [&str; 5] = ["Sprawl", "Windows", "Midnight", "Paper", "Custom"];
/// The index of Custom in [`PRESETS`].
const CUSTOM: usize = 4;

fn opt(key: &str) -> Option<&'static Opt> {
    OPTS.iter().find(|o| o.key == key)
}

// ------------------------------------------------------------------ the options as values

/// The name of the choice `look` has for option `key` (not `mode`).
fn name_of(look: &Look, key: &str) -> Option<String> {
    look.to_kv().split(';').find_map(|p| p.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v.to_string()))
}

/// The index of the choice picked for `key`.
pub(crate) fn index(look: &Look, mode: usize, key: &str) -> usize {
    let Some(o) = opt(key) else { return 0 };
    if key == "mode" {
        return mode.min(o.choices.len() - 1);
    }
    let name = name_of(look, key).unwrap_or_default();
    o.choices.iter().position(|c| c.0 == name).unwrap_or(0)
}

/// `look` and `mode` with option `key` set to choice `i`.
pub(crate) fn with_pick(look: &Look, mode: usize, key: &str, i: usize) -> (Look, usize) {
    let Some(o) = opt(key) else { return (look.clone(), mode) };
    let i = i.min(o.choices.len() - 1);
    if key == "mode" {
        return (look.clone(), i);
    }
    (Look::from_kv(&format!("{};{key}={}", look.to_kv(), o.choices[i].0)), mode)
}

/// The values of preset `i`, and the light/dark choice it sets (None: leaves it as it is).
/// Custom (the last) has none: it is the user's own.
pub(crate) fn preset(i: usize) -> Option<(Look, Option<usize>)> {
    use tokens::{Accent, Motion, Surfaces};
    Some(match i {
        0 => (Look::default(), Some(0)),
        1 => (Look { surfaces: Surfaces::Graphite, motion: Motion::None, small_text: SmallText::Plain, ..Look::default() }, None),
        2 => (Look { surfaces: Surfaces::Black, accent: Accent::Iris, ..Look::default() }, Some(2)),
        3 => (Look { surfaces: Surfaces::Graphite, motion: Motion::None, ..Look::default() }, Some(1)),
        _ => return None,
    })
}

/// Which preset the picks are: the first one they match, else Custom. Sprawl's own look is
/// Sprawl's own in light or dark; Midnight and Paper are a light or dark of their own.
pub(crate) fn which(look: &Look, mode: usize) -> usize {
    (0..CUSTOM).find(|&i| preset(i).is_some_and(|(l, m)| &l == look && (i == 0 || m.is_none_or(|m| m == mode)))).unwrap_or(CUSTOM)
}

// ------------------------------------------------------------------ a theme as one line

/// The options as one line, with the light/dark choice: `accent=iris;...;mode=dark`.
pub(crate) fn line(look: &Look, mode: usize) -> String {
    format!("{};mode={}", look.to_kv(), OPTS[0].choices[mode.min(2)].0)
}

/// How many parts of `s` are a known option with a known value.
fn known(s: &str) -> usize {
    s.split(';').filter_map(|p| p.split_once('=')).filter(|(k, v)| opt(k.trim()).is_some_and(|o| o.choices.iter().any(|c| c.0 == v.trim()))).count()
}

/// A line as Copy theme wrote it, or outside text claiming to be one: only known options with
/// known values are taken, the rest is ignored. None when nothing in it is known.
pub(crate) fn parse_line(s: &str) -> Option<(Look, Option<usize>)> {
    if s.len() > 2000 || known(s) == 0 {
        return None;
    }
    let mode = s.split(';').filter_map(|p| p.split_once('=')).filter(|(k, _)| k.trim() == "mode").find_map(|(_, v)| OPTS[0].choices.iter().position(|c| c.0 == v.trim()));
    Some((Look::from_kv(s), mode))
}

// ------------------------------------------------------------------ settings.ini

/// The look kept in the settings: one `look.<option>` key each; a missing key (or one holding
/// anything unknown) is Sprawl's own value.
pub(crate) fn read(s: &Settings) -> Look {
    let mut line = String::new();
    for o in OPTS.iter().filter(|o| o.key != "mode") {
        if let Some(v) = s.get(&format!("look.{}", o.key)) {
            line += &format!("{}={v};", o.key);
        }
    }
    Look::from_kv(&line)
}

pub(crate) fn write(s: &mut Settings, look: &Look) {
    for o in OPTS.iter().filter(|o| o.key != "mode") {
        if let Some(v) = name_of(look, o.key) {
            s.set(&format!("look.{}", o.key), v);
        }
    }
}

/// The user's own picks, kept apart from the options in use.
pub(crate) fn read_custom(s: &Settings) -> Option<(Look, usize)> {
    let (look, mode) = parse_line(s.get("look.custom")?)?;
    Some((look, mode.unwrap_or(0)))
}

pub(crate) fn write_custom(s: &mut Settings, custom: Option<&(Look, usize)>) {
    s.set("look.custom", custom.map(|(l, m)| line(l, *m)).unwrap_or_default());
}

/// `look` with the options of a `key=value;key=value` line put over it (a dev override).
pub(crate) fn overlaid(look: &Look, over: &str) -> Look {
    Look::from_kv(&format!("{};{over}", look.to_kv()))
}

/// The look to start with: the settings', and over it `SPRAWL_LOOK` (a dev override for
/// screenshot runs, never written back).
pub(crate) fn startup_look(s: &Settings) -> Look {
    let look = read(s);
    match std::env::var("SPRAWL_LOOK") {
        Ok(over) => overlaid(&look, &over),
        Err(_) => look,
    }
}

// ------------------------------------------------------------------ the sample

/// A small mock of the page under `look`, drawn with that look's own table of tokens (and the
/// light or dark page `mode` asks for), so each pick shows at once.
pub(crate) fn sample(look: &Look, mode: usize) -> String {
    let light = crate::theme::wanted_light(Some(&mode.to_string()));
    let t = tokens::table(look, light);
    let accent = t["accent"].clone();
    let bar_end = tokens::lighter(&format!("#{}", &accent[3..]));
    let (size, colour, lead) = match look.small_text {
        SmallText::LeadWords => (13.0, "text2", true),
        SmallText::Plain => (13.0, "text2", false),
        SmallText::Grey12 => (12.0, "text3", false),
    };
    let (size, colour) = (tokens::fs(size), tokens::refer(colour));
    let lead_style = if lead { r#" FontWeight="SemiBold" Foreground="{$text1}""# } else { "" };
    let chip = |text: &str, on: bool| {
        let (bg, fg) = if on { (tokens::refer("accent"), tokens::refer("onaccent")) } else { (tokens::refer("surface3"), tokens::refer("text2")) };
        format!(r##"<Border CornerRadius="{{$rcpill}}" Padding="10,2" Background="{bg}" BorderBrush="{{$line1}}" BorderThickness="1"><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{fg}"/></Border>"##, ui::esc(text))
    };
    let tick_radius = t[if look.ticks == Ticks::Round { "rcpill" } else { "rc4" }].clone();
    // a 190 px bar, 62 % full, as the tables draw it: washed, so the figure on it reads
    let w = 118;
    let (track, fill) = match look.bars {
        Bars::Thin => (
            r##"<Border CornerRadius="{$rc3}" Background="{$line1}" VerticalAlignment="Bottom" Height="4"/>"##.to_string(),
            format!(r##"<Border CornerRadius="{{$rc3}}" HorizontalAlignment="Left" Width="{w}" Background="{{$accent}}" Opacity="0.45" VerticalAlignment="Bottom" Height="4"/>"##),
        ),
        Bars::Gradient => (
            r##"<Border CornerRadius="{$rc3}" Background="{$line1}"/>"##.to_string(),
            format!(r##"<Border CornerRadius="{{$rc3}}" HorizontalAlignment="Left" Width="{w}" Opacity="0.45" Background="{{$accent}}"><Border.Background><LinearGradientBrush StartPoint="0,0.5" EndPoint="1,0.5"><GradientStop Color="{{$accent}}" Offset="0"/><GradientStop Color="{bar_end}" Offset="1"/></LinearGradientBrush></Border.Background></Border>"##),
        ),
        Bars::Solid => (
            r##"<Border CornerRadius="{$rc3}" Background="{$line1}"/>"##.to_string(),
            format!(r##"<Border CornerRadius="{{$rc3}}" HorizontalAlignment="Left" Width="{w}" Background="{{$accent}}" Opacity="0.45"/>"##),
        ),
    };
    let xaml = format!(
        r##"<Border {ns} Background="{{$ground}}" CornerRadius="{{$rc10}}" BorderBrush="{{$line1}}" BorderThickness="1" Padding="10">
  <Border Background="{{$card}}" CornerRadius="{{$rc8}}" BorderBrush="{{$line1}}" BorderThickness="1" Padding="14,10">
    <Grid ColumnSpacing="16">
      <Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="190"/></Grid.ColumnDefinitions>
      <StackPanel Spacing="6">
        <StackPanel Orientation="Horizontal" Spacing="8">
          <FontIcon Glyph="&#xE8B7;" FontSize="{{$fs16}}" Foreground="{{$folder}}" VerticalAlignment="Center"/>
          <TextBlock Text="Downloads" FontSize="{{$fs14}}" FontWeight="SemiBold" Foreground="{{$text1}}"/>
          <TextBlock Text="12.4 GB" FontSize="{{$fs14}}" Foreground="{{$accenttext}}"/>
        </StackPanel>
        <TextBlock FontSize="{size}" Foreground="{colour}" TextWrapping="Wrap"><Run Text="Installers you ran once." {lead_style}/><Run Text=" This is how an explanation reads."/></TextBlock>
        <StackPanel Orientation="Horizontal" Spacing="6">{c1}{c2}{c3}</StackPanel>
      </StackPanel>
      <StackPanel Grid.Column="1" Spacing="8">
        <StackPanel Orientation="Horizontal" Spacing="6">
          <Border Padding="14,4" CornerRadius="{{$rc4}}" Background="{{$accent}}"><TextBlock Text="Scan" FontSize="{{$fs13}}" FontWeight="SemiBold" Foreground="{{$onaccent}}"/></Border>
          <Border Padding="14,4" CornerRadius="{{$rc4}}" Background="{{$surface3}}" BorderBrush="{{$line2}}" BorderThickness="1"><TextBlock Text="Cancel" FontSize="{{$fs13}}" Foreground="{{$text1}}"/></Border>
        </StackPanel>
        <StackPanel Orientation="Horizontal" Spacing="8">
          <Border Width="18" Height="18" CornerRadius="{tick_radius}" Background="{{$accent}}"><FontIcon Glyph="&#xE73E;" FontSize="{{$fs12}}" Foreground="{{$onaccent}}" HorizontalAlignment="Center" VerticalAlignment="Center"/></Border>
          <TextBlock Text="Keep a copy" FontSize="{{$fs13}}" Foreground="{{$text1}}" VerticalAlignment="Center"/>
        </StackPanel>
        <Grid Width="190" Height="20" HorizontalAlignment="Left">{track}{fill}<TextBlock Text="62.0 %" FontSize="{{$fs12}}" Foreground="{{$text1}}" HorizontalAlignment="Right" VerticalAlignment="Center" Margin="0,0,6,0"/></Grid>
      </StackPanel>
    </Grid>
  </Border>
</Border>"##,
        ns = ui::NS,
        c1 = chip("Folders", false),
        c2 = chip("Files", true),
        c3 = chip("Caches", false),
    );
    tokens::expand_in(&xaml, &t)
}

/// The sample, made into an element.
fn sample_element(look: &Look, mode: usize) -> Option<UIElement> {
    XamlReader::Load(&HSTRING::from(sample(look, mode))).ok()?.cast().ok()
}

// ------------------------------------------------------------------ the page

/// What the page holds between one render and the next.
#[derive(Default)]
pub(crate) struct ThemeUi {
    /// The picks, as the page shows them (the options in use when Settings opened, changed).
    pub(crate) look: Look,
    /// Light or dark: an index of [`crate::theme::CHOICES`].
    pub(crate) mode: usize,
    /// The user's own set (Custom), kept when a preset is tried.
    pub(crate) custom: Option<(Look, usize)>,
    /// The options when Settings opened, and the custom set then, to tell what changed.
    pub(crate) orig: Look,
    orig_custom: Option<(Look, usize)>,
    /// The group shown.
    tab: usize,
}

fn button(name: &str, t: &str, primary: bool) -> String {
    let style = if primary { r#" Style="{StaticResource SprawlPrimary}""# } else { "" };
    format!(r##"<Button x:Name="{name}" Content="{}"{style}/>"##, ui::esc(t))
}

impl State {
    /// Settings opened: the page starts from what is in use.
    pub(crate) fn theme_begin(&mut self) {
        let mode = self.prefs.draft.get("theme").and_then(|v| v.parse().ok()).unwrap_or(0usize).min(2);
        let custom = read_custom(&self.settings);
        let t = &mut self.prefs.theme;
        t.look = tokens::current();
        t.orig = t.look.clone();
        t.mode = mode;
        t.custom = custom.clone();
        t.orig_custom = custom;
        t.tab = 0;
    }

    /// The page's XAML, and the names of its buttons.
    pub(crate) fn theme_page(&self) -> (String, Vec<String>) {
        let t = &self.prefs.theme;
        let mut buttons: Vec<String> = Vec::new();
        let mut x = String::new();
        x += &format!(r##"<Border x:Name="TmSample"/>"##);
        x += &ui::note_text(r#"Margin="0,2,0,0""#, "The sample follows each pick. Press Save to use it in Sprawl.");
        x += r##"<Border x:Name="TmPresets"/>"##;
        x += &format!(
            r##"<StackPanel Orientation="Horizontal" Spacing="6">{}{}{}</StackPanel>"##,
            button("TmCopy", "Copy theme", false),
            button("TmPaste", "Paste theme", false),
            button("TmBack", "Back to Sprawl's own", false)
        );
        buttons.extend(["TmCopy", "TmPaste", "TmBack"].map(String::from));
        {
            x += r##"<StackPanel Orientation="Horizontal" Spacing="2" Margin="0,6,0,0">"##;
            for (g, name) in GROUPS.iter().enumerate() {
                x += &format!(
                    r##"<Button x:Name="TmTab{g}" {}><TextBlock Text="{}" FontSize="{{$fs13}}"/></Button>"##,
                    crate::look::pick(crate::look::Pick::Page, g == t.tab),
                    ui::esc(name)
                );
                buttons.push(format!("TmTab{g}"));
            }
            x += "</StackPanel>";
            let in_group: Vec<&Opt> = OPTS.iter().filter(|o| o.group == t.tab).collect();
            let rows = r##"<RowDefinition Height="Auto"/>"##.repeat(in_group.len().div_ceil(2));
            x += &format!(
                r##"<Grid ColumnSpacing="16" RowSpacing="8" Margin="0,4,0,0"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><Grid.RowDefinitions>{rows}</Grid.RowDefinitions>"##
            );
            for (n, o) in in_group.iter().enumerate() {
                let items: String = o.choices.iter().map(|(_, l)| format!(r##"<ComboBoxItem Content="{}"/>"##, ui::esc(l))).collect();
                x += &format!(
                    r##"<StackPanel Grid.Row="{}" Grid.Column="{}"><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$label}}" Margin="0,0,0,2"/><ComboBox x:Name="TmPick_{}" HorizontalAlignment="Stretch" SelectedIndex="{}">{items}</ComboBox></StackPanel>"##,
                    n / 2,
                    n % 2,
                    ui::esc(o.label),
                    o.key,
                    index(&t.look, t.mode, o.key)
                );
            }
            x += "</Grid>";
        }
        (x, buttons)
    }

    /// The preset buttons, the one the picks match drawn as the main one.
    fn theme_presets(&self) -> String {
        let t = &self.prefs.theme;
        let on = which(&t.look, t.mode);
        let b: String = PRESETS.iter().enumerate().map(|(i, name)| button(&format!("TmPre{i}"), name, i == on)).collect();
        format!(r##"<StackPanel {} Orientation="Horizontal" Spacing="6">{b}</StackPanel>"##, ui::NS)
    }

    /// The page was loaded: its dropdowns are heard, its sample and presets put in.
    pub(crate) fn theme_wire(&self, fe: &FrameworkElement) {
        for o in OPTS.iter() {
            let key = o.key;
            if let Ok(s) = find::<Selector>(fe, &format!("TmPick_{key}")) {
                if let Ok(tok) = s.SelectionChanged(move |sel, _| {
                    let i = sel.as_ref().and_then(|o| o.cast::<Selector>().ok()).and_then(|s| s.SelectedIndex().ok()).and_then(|i| usize::try_from(i).ok());
                    if let Some(i) = i {
                        with(move |st| st.theme_pick(key, i));
                    }
                }) {
                    keep(tok);
                }
            }
        }
        self.theme_refresh(fe);
    }

    /// The sample and the presets, made again for the picks (the dropdowns stay as they are).
    fn theme_refresh(&self, fe: &FrameworkElement) {
        let t = &self.prefs.theme;
        if let (Ok(host), Some(el)) = (find::<Border>(fe, "TmSample"), sample_element(&t.look, t.mode)) {
            let _ = host.SetChild(&el);
        }
        if let (Ok(host), Ok(el)) = (find::<Border>(fe, "TmPresets"), crate::state::load::<FrameworkElement>(&self.theme_presets())) {
            let _ = host.SetChild(&el);
            // the row was loaded on its own, so its names are found in it, not in the page
            for i in 0..PRESETS.len() {
                let name = format!("TmPre{i}");
                if let Ok(b) = find::<ButtonBase>(&el, &name) {
                    if let Ok(tok) = b.Click(move |_, _| {
                        let n = name.clone();
                        with(move |s| s.prefs_button(&n));
                    }) {
                        keep(tok);
                    }
                }
            }
        }
    }

    /// A dropdown's pick: the picks change, and it is the user's own set now.
    fn theme_pick(&mut self, key: &str, i: usize) {
        let t = &mut self.prefs.theme;
        if index(&t.look, t.mode, key) == i {
            return;
        }
        let (look, mode) = with_pick(&t.look, t.mode, key, i);
        t.look = look;
        t.mode = mode;
        t.custom = Some((t.look.clone(), t.mode));
        self.prefs.draft.insert("theme", self.prefs.theme.mode.to_string());
        if let Some(fe) = self.prefs.fe.clone() {
            self.theme_refresh(&fe);
        }
    }

    /// A button of the page; the page is made again after it.
    pub(crate) fn theme_button(&mut self, name: &str) {
        let t = &mut self.prefs.theme;
        match name {
            "TmCopy" => {
                let text = line(&t.look, t.mode);
                crate::win32::copy_text(self.hwnd, &text);
                self.prefs.note = Some("Copied this theme. Paste it into Sprawl on another PC with Paste theme.".into());
            }
            "TmPaste" => {
                let text = crate::win32::paste_text(self.hwnd).unwrap_or_default();
                match parse_line(text.trim()) {
                    Some((look, mode)) => {
                        let t = &mut self.prefs.theme;
                        t.look = look;
                        if let Some(m) = mode {
                            t.mode = m;
                        }
                        t.custom = Some((t.look.clone(), t.mode));
                        self.prefs.note = Some("Pasted the theme; only the options Sprawl knows were taken. Press Save to use it.".into());
                    }
                    None => self.prefs.note = Some("The clipboard doesn't hold a theme from Copy theme, so nothing changed.".into()),
                }
                self.prefs.draft.insert("theme", self.prefs.theme.mode.to_string());
            }
            "TmBack" => {
                t.look = Look::default();
                t.mode = 0;
                t.custom = None;
                self.prefs.draft.insert("theme", "0".into());
                self.prefs.note = Some("Back to Sprawl's own look. Press Save to keep that, or Cancel.".into());
            }
            n if n.starts_with("TmTab") => t.tab = n[5..].parse().unwrap_or(0).min(GROUPS.len() - 1),
            n if n.starts_with("TmPre") => {
                let i: usize = n[5..].parse().unwrap_or(0);
                if let Some((look, mode)) = preset(i) {
                    t.look = look;
                    if let Some(m) = mode {
                        t.mode = m;
                    }
                } else if let Some((look, mode)) = t.custom.clone() {
                    t.look = look;
                    t.mode = mode;
                } else {
                    // nothing of the user's own yet: what is picked now is it
                    t.custom = Some((t.look.clone(), t.mode));
                }
                self.prefs.draft.insert("theme", self.prefs.theme.mode.to_string());
            }
            _ => {}
        }
    }

    /// Reset everything: the picks are Sprawl's own too.
    pub(crate) fn theme_reset(&mut self) {
        let t = &mut self.prefs.theme;
        t.look = Look::default();
        t.mode = 0;
        t.custom = None;
    }

    /// Save: the picks are kept in the settings, and a changed look is applied now. The light
    /// or dark choice is the `theme` setting, applied by the page's own path.
    pub(crate) fn theme_commit(&mut self) {
        let t = &self.prefs.theme;
        if t.custom != t.orig_custom {
            write_custom(&mut self.settings, t.custom.as_ref());
        }
        if t.look != t.orig {
            let look = t.look.clone();
            write(&mut self.settings, &look);
            crate::restyle::apply_look(look.clone());
            self.prefs.theme.orig = look;
        }
        self.prefs.theme.orig_custom = self.prefs.theme.custom.clone();
    }

    /// dev: a step of a screenshot run that shows the page in a state (`ThemeMore`,
    /// `ThemeTab=1`, `ThemePre=2`, `ThemeDrop=accent` then `ThemeOpen=accent`).
    pub(crate) fn theme_dev(&mut self, step: &str) {
        let (cmd, arg) = step.split_once('=').unwrap_or((step, ""));
        match cmd {
            "ThemeMore" => {}
            "ThemeTab" => {
                self.prefs.theme.tab = arg.parse().unwrap_or(0).min(GROUPS.len() - 1);
            }
            "ThemePre" => self.theme_button(&format!("TmPre{arg}")),
            "ThemeDrop" => {
                if let Some(o) = opt(arg) {
                    self.prefs.theme.tab = o.group;
                }
            }
            // opened as a screen reader would, once the page above has been laid out
            "ThemeOpen" => {
                crate::win32::expand_dropdown(self.hwnd, &format!("TmPick_{arg}"));
                return;
            }
            _ => {}
        }
        self.prefs_render();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_choice_round_trips_and_every_group_fits() {
        for o in OPTS.iter() {
            assert!(OPTS.iter().filter(|p| p.group == o.group).count() <= 8, "{}", GROUPS[o.group]);
            for i in 0..o.choices.len() {
                let (look, mode) = with_pick(&Look::default(), 0, o.key, i);
                assert_eq!(index(&look, mode, o.key), i, "{} {i}", o.key);
                if o.key != "mode" {
                    assert_eq!(look == Look::default(), i == 0, "the first choice of {} is Sprawl's own", o.key);
                }
            }
        }
        // the page's choices are the whole of each option: to_kv names nothing the page lacks
        let all = Look::default().to_kv();
        for part in all.split(';') {
            let (k, v) = part.split_once('=').unwrap();
            if let Some(o) = opt(k) {
                assert_eq!(o.choices[0].0, v, "{k}: the default is the first choice");
            }
        }
    }

    #[test]
    fn presets_are_only_option_values() {
        use tokens::{Accent, Motion, Surfaces};
        assert_eq!(preset(0), Some((Look::default(), Some(0))));
        let (w, m) = preset(1).unwrap();
        assert_eq!((w.surfaces, w.corners, w.motion, w.bars, w.small_text), (Surfaces::Graphite, tokens::Corners::Four8, Motion::None, Bars::Solid, SmallText::Plain));
        assert_eq!((w.accent, w.contrast, w.ticks), (Accent::Windows, tokens::Contrast::Normal, Ticks::Square));
        assert_eq!(m, None);
        let (mid, m) = preset(2).unwrap();
        assert_eq!((mid.surfaces, mid.accent, m), (Surfaces::Black, Accent::Iris, Some(2)));
        let (paper, m) = preset(3).unwrap();
        assert_eq!((paper.surfaces, paper.motion, m), (Surfaces::Graphite, Motion::None, Some(1)));
        assert_eq!(preset(CUSTOM), None);
        // the picks name the preset they match; anything else is Custom
        assert_eq!(which(&Look::default(), 0), 0);
        assert_eq!(which(&Look::default(), 2), 0, "Sprawl's own look in dark is still Sprawl's own");
        assert_eq!(which(&w, 1), 1);
        assert_eq!(which(&w, 2), 1, "Windows leaves light or dark alone");
        assert_eq!(which(&mid, 2), 2);
        assert_eq!(which(&mid, 1), CUSTOM, "Midnight is dark");
        assert_eq!(which(&paper, 1), 3);
        assert_eq!(which(&Look { ticks: Ticks::Round, ..Look::default() }, 0), CUSTOM);
    }

    #[test]
    fn the_settings_round_trip() {
        let look = Look::from_kv("accent=sunset;surfaces=slate;contrast=strong;folders=grey;corners=pills;motion=lively;bars=thin;smalltext=grey12;ticks=round");
        let mut s = Settings::default();
        // a missing key is Sprawl's own
        assert_eq!(read(&s), Look::default());
        write(&mut s, &look);
        assert_eq!(s.get("look.surfaces"), Some("slate"));
        assert_eq!(s.get("look.smalltext"), Some("grey12"));
        assert_eq!(read(&s), look);
        // one key missing or junk: only that option is Sprawl's own
        s.set("look.corners", "wobbly");
        let got = read(&s);
        assert_eq!(got.corners, tokens::Corners::Four8);
        assert_eq!(got.surfaces, tokens::Surfaces::Slate);
        // Custom is kept apart, with its light or dark choice
        write_custom(&mut s, Some(&(look.clone(), 2)));
        assert_eq!(read_custom(&s), Some((look, 2)));
        write_custom(&mut s, None);
        assert_eq!(read_custom(&s), None);
    }

    #[test]
    fn a_pasted_line_takes_only_what_it_knows() {
        let (look, mode) = parse_line("accent=iris;surfaces=black;mode=dark").unwrap();
        assert_eq!((look.accent, look.surfaces, mode), (tokens::Accent::Iris, tokens::Surfaces::Black, Some(2)));
        // junk keys and values are ignored, the known ones kept; the rest is Sprawl's own
        let (look, mode) = parse_line("evil=1;accent=<script>;corners=square;mode=sideways;;=;surfaces").unwrap();
        assert_eq!((look.accent, look.corners, mode), (tokens::Accent::Windows, tokens::Corners::Square, None));
        // text with nothing known in it is not a theme
        assert_eq!(parse_line("hello world"), None);
        assert_eq!(parse_line("accent=nothing;mode=never"), None);
        assert_eq!(parse_line(&"accent=iris;".repeat(500)), None, "far too long to be a theme");
        // what Copy theme writes comes back whole
        let look = Look::from_kv("accent=violet;ticks=round;bars=gradient");
        assert_eq!(parse_line(&line(&look, 1)), Some((look, Some(1))));
    }

    #[test]
    fn a_dev_override_goes_over_the_stored_look() {
        let stored = Look::from_kv("surfaces=black;bars=thin");
        let over = overlaid(&stored, "surfaces=slate;accent=sunset;junk=1");
        assert_eq!((over.surfaces, over.accent, over.bars), (tokens::Surfaces::Slate, tokens::Accent::Sunset, Bars::Thin));
    }

    #[test]
    fn the_sample_loads_for_every_pick() {
        // every token the sample writes is in the table: none is left unfilled
        for look in [Look::default(), Look::from_kv("bars=thin;ticks=round;smalltext=grey12"), Look::from_kv("bars=gradient;corners=square;surfaces=slate")] {
            for mode in [1, 2] {
                let x = sample(&look, mode);
                assert!(!x.contains('$'), "a token is left in the sample: {x}");
            }
        }
    }
}
