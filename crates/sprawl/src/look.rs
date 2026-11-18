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

/// The background of the selected item, for every kind.
pub(crate) const SELECTED: &str = "#FF2C3B63";

/// The families of selectable items: they differ only in their background at rest.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Pick {
    /// The tab row: nothing at rest.
    Tab,
    /// The Advisor's and Search's chips: a faint pill at rest.
    Chip,
    /// The map's mode buttons, on their dark strip.
    Mode,
}

impl Pick {
    pub(crate) fn rest(self) -> &'static str {
        match self {
            Pick::Tab => "Transparent",
            Pick::Chip => "#14FFFFFF",
            Pick::Mode => "#FF1E212C",
        }
    }

    /// Its background, selected or not.
    pub(crate) fn bg(self, on: bool) -> &'static str {
        if on {
            SELECTED
        } else {
            self.rest()
        }
    }
}

/// The attributes for a `<Button>` drawn as a selectable item.
pub(crate) fn pick(kind: Pick, on: bool) -> String {
    format!(r##"Style="{{StaticResource SprawlPick}}" Background="{}""##, kind.bg(on))
}

/// The same for a `<DropDownButton>` (its arrow is part of the look).
pub(crate) fn pick_drop(kind: Pick, on: bool) -> String {
    format!(r##"Style="{{StaticResource SprawlPickDrop}}" Background="{}""##, kind.bg(on))
}

/// Light or unlight the selectable item `name` (a tab, a mode) that is already on screen.
pub(crate) fn paint(root: &FrameworkElement, name: &str, kind: Pick, on: bool) {
    let Ok(c) = find::<Control>(root, name) else { return };
    if let Ok(br) = load::<Brush>(&format!(r##"<SolidColorBrush {} Color="{}"/>"##, ui::NS, kind.bg(on))) {
        let _ = c.SetBackground(&br);
    }
}

/// A tag on a row that says what kind of thing it is ("Delete", "How to"): plain words in the
/// row's quiet colour, with no background or border, so it does not read as a button.
pub(crate) fn label(text: &str) -> String {
    format!(r##"<TextBlock Text="{}" FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center" Margin="4,0,0,0"/>"##, ui::esc(text))
}

/// The template both selectable styles share: the item's own background (set by `paint` or
/// `pick`: at rest or selected), a white wash over it for the pointer (pointer over: faint,
/// pressed: a little more), and the system's focus rectangle, which shows for the keyboard
/// only. So selected-and-pointer-over is the selected background with the faint wash, and a
/// selected item the pointer has left is the selected background alone.
fn template(target: &str, arrow: bool) -> String {
    let (cols, arrow) = if arrow {
        (
            r##"<Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>"##,
            r##"<FontIcon x:Name="Arrow" Grid.Column="1" Glyph="&#xE70D;" FontSize="8" Margin="2,0,10,0" VerticalAlignment="Center" Foreground="{TemplateBinding Foreground}"/>"##,
        )
    } else {
        ("", "")
    };
    format!(
        r##"<ControlTemplate TargetType="{target}">
  <Grid x:Name="Root" Background="{{TemplateBinding Background}}" CornerRadius="{{TemplateBinding CornerRadius}}" BorderBrush="{{TemplateBinding BorderBrush}}" BorderThickness="{{TemplateBinding BorderThickness}}">
    {cols}
    <Border x:Name="Wash" Grid.ColumnSpan="2" Background="#FFFFFFFF" Opacity="0" CornerRadius="{{TemplateBinding CornerRadius}}"/>
    <ContentPresenter x:Name="Content" Content="{{TemplateBinding Content}}" ContentTemplate="{{TemplateBinding ContentTemplate}}" Padding="{{TemplateBinding Padding}}" HorizontalContentAlignment="{{TemplateBinding HorizontalContentAlignment}}" VerticalContentAlignment="{{TemplateBinding VerticalContentAlignment}}" Foreground="{{TemplateBinding Foreground}}" FontSize="{{TemplateBinding FontSize}}" FontFamily="{{TemplateBinding FontFamily}}" FontWeight="{{TemplateBinding FontWeight}}"/>
    {arrow}
    <VisualStateManager.VisualStateGroups>
      <VisualStateGroup x:Name="CommonStates">
        <VisualState x:Name="Normal"/>
        <VisualState x:Name="PointerOver"><VisualState.Setters><Setter Target="Wash.Opacity" Value="0.07"/></VisualState.Setters></VisualState>
        <VisualState x:Name="Pressed"><VisualState.Setters><Setter Target="Wash.Opacity" Value="0.12"/><Setter Target="Content.Opacity" Value="0.8"/></VisualState.Setters></VisualState>
        <VisualState x:Name="Disabled"><VisualState.Setters><Setter Target="Content.Opacity" Value="0.4"/></VisualState.Setters></VisualState>
      </VisualStateGroup>
    </VisualStateManager.VisualStateGroups>
  </Grid>
</ControlTemplate>"##
    )
}

fn style(key: &str, target: &str, arrow: bool) -> String {
    format!(
        r##"<Style x:Key="{key}" TargetType="{target}">
  <Setter Property="Background" Value="Transparent"/>
  <Setter Property="Foreground" Value="#FFE6EAF2"/>
  <Setter Property="BorderThickness" Value="0"/>
  <Setter Property="Padding" Value="10,3"/>
  <Setter Property="CornerRadius" Value="6"/>
  <Setter Property="FontSize" Value="12"/>
  <Setter Property="HorizontalContentAlignment" Value="Center"/>
  <Setter Property="VerticalContentAlignment" Value="Center"/>
  <Setter Property="UseSystemFocusVisuals" Value="True"/>
  <Setter Property="FocusVisualMargin" Value="-2"/>
  <Setter Property="Template"><Setter.Value>{}</Setter.Value></Setter>
</Style>"##,
        template(target, arrow)
    )
}

/// The styles, as a dictionary merged into the application's resources at start (so any
/// XAML loaded later, the Advisor's or Search's included, can name them).
pub(crate) fn resources() -> String {
    format!(
        r##"<ResourceDictionary {}>{}{}</ResourceDictionary>"##,
        ui::NS,
        style("SprawlPick", "Button", false),
        style("SprawlPickDrop", "DropDownButton", true)
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
        let mut x = format!(r##"<StackPanel {} Spacing="2"><TextBlock Text="Running now" FontSize="12" Foreground="#FF8E97AA" Margin="10,0,10,4"/>"##, ui::NS);
        for (i, j) in listed.iter().enumerate() {
            x += &format!(
                r##"<Button x:Name="BusyGo{i}" {} HorizontalAlignment="Stretch" HorizontalContentAlignment="Stretch" Padding="10,5" ToolTipService.ToolTip="Open {tab}"><Grid ColumnSpacing="16"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBlock Text="{}" TextTrimming="CharacterEllipsis"/><TextBlock Grid.Column="1" Text="{tab}  &#x203A;" Foreground="#FF8E97AA"/></Grid></Button>"##,
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
    fn one_selected_look_for_every_kind() {
        for k in [Pick::Tab, Pick::Chip, Pick::Mode] {
            assert_eq!(k.bg(true), SELECTED);
            assert_ne!(k.bg(false), SELECTED);
        }
        assert!(pick(Pick::Tab, true).contains("SprawlPick") && pick(Pick::Tab, true).contains(SELECTED));
        assert!(pick_drop(Pick::Mode, false).contains("SprawlPickDrop"));
    }

    #[test]
    fn the_styles_define_every_state_once() {
        let r = resources();
        for s in ["\"Normal\"", "\"PointerOver\"", "\"Pressed\"", "\"Disabled\""] {
            assert_eq!(r.matches(s).count(), 2, "{s}");
        }
        assert_eq!(r.matches("UseSystemFocusVisuals").count(), 2);
    }
}
