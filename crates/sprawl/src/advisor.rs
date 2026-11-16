//! The Advisor tab: what can be won back, with evidence, the command to run and a treemap lens.
//!
//! The analysis (rules, app facts, projects, safety checks) runs on a background thread after
//! every scan; the report is tied to the exact tree it was made from and ignored once that tree
//! changes. Nothing here deletes or runs anything: commands are shown for the user to copy.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use sprawl_advisor::facts::Facts;
use sprawl_advisor::hidden::HiddenSpace;
use sprawl_advisor::{Finding, Remedy, Risk, RulePack};
use sprawl_core::Tree;
use windows_core::Interface;

use crate::bindings::Microsoft::UI::Xaml::Controls::Border;
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::ButtonBase;
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement};
use crate::state::{find, keep, load, with, State, View};
use crate::{ui, win32};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Lens {
    /// Findings keep their colours, everything else dims.
    #[default]
    Opportunities,
    /// Findings coloured by risk.
    Risk,
    Off,
}

pub(crate) struct Report {
    /// `State::tree_gen` of the tree this was made from; stale once that changes.
    tree: u64,
    pub(crate) findings: Vec<Finding>,
    /// The node of each finding.
    nodes: Vec<Option<usize>>,
    /// Not inside another finding, so its bytes count toward the total.
    counted: Vec<bool>,
    held: Vec<(Finding, String)>,
    hidden: Option<HiddenSpace>,
    /// Owning app of every node (index into `apps`).
    owners: Vec<Option<u32>>,
    apps: Vec<String>,
    /// Risk code of every node inside a finding: 0 none, 1 green, 2 amber, 3 orange.
    pub(crate) lens: Arc<Vec<u8>>,
    secs: f64,
}

enum Msg {
    Report(Box<Report>),
    Hidden(u64, Option<HiddenSpace>),
    /// (tree generation, finding, answer)
    InUse(u64, usize, InUse),
}

/// The in-use check of an opened finding.
#[derive(Clone)]
enum InUse {
    Checking,
    /// `None`: Windows couldn't say.
    Done(Option<sprawl_advisor::inuse::Checked>),
}

#[derive(Default)]
pub(crate) struct Advisor {
    pub(crate) report: Option<Report>,
    rx: Option<Receiver<Msg>>,
    /// For follow-up work (the in-use check) on the same channel.
    tx: Option<Sender<Msg>>,
    running: bool,
    /// Why there is no report (no scan, a CSV, a filter).
    note: Option<&'static str>,
    open: Option<usize>,
    /// (finding, programs that have its files open); `None` inside while checking.
    in_use: Option<(usize, InUse)>,
    pub(crate) lens_mode: Lens,
    show_held: bool,
}

fn risk_code(r: Risk) -> u8 {
    match r {
        Risk::Green => 1,
        Risk::Amber => 2,
        Risk::Orange => 3,
        Risk::Red => 4,
    }
}

fn risk_hex(r: Risk) -> &'static str {
    match r {
        Risk::Green => "#FF4CC38A",
        Risk::Amber => "#FFF5B342",
        Risk::Orange => "#FFFF7A45",
        Risk::Red => "#FFE5484D",
    }
}

/// Lens tile colours by risk code (linear-ish, like the palette).
pub(crate) fn lens_colour(code: u8) -> [f32; 3] {
    match code {
        1 => [0.20, 0.72, 0.42],
        2 => [0.95, 0.62, 0.16],
        3 => [0.98, 0.36, 0.18],
        _ => [0.85, 0.20, 0.22],
    }
}

fn remedy_label(r: Remedy) -> &'static str {
    match r {
        Remedy::Delete => "Delete",
        Remedy::Command => "Run a command",
        Remedy::Uninstall => "Uninstall",
        Remedy::Move => "Move",
        Remedy::Info => "How to",
    }
}

/// Everything the advisor works out for one tree. Runs off the UI thread.
fn analyse(tree: &Arc<Tree>, gen: u64) -> Report {
    let started = std::time::Instant::now();
    let mut facts = Facts::gather();
    facts.attach_usage(tree);
    let (findings, held) = sprawl_advisor::advise_all(tree, &[RulePack::builtin()], &facts);
    let nodes: Vec<Option<usize>> = findings.iter().map(|f| sprawl_advisor::find_path(tree, &f.path)).collect();
    let mut lens = vec![0u8; tree.nodes.len()];
    let mut counted = vec![true; findings.len()];
    for (i, (f, n)) in findings.iter().zip(&nodes).enumerate() {
        let Some(n) = *n else { continue };
        // already inside an earlier (bigger) finding: shown, but not counted twice
        if lens[n] != 0 {
            counted[i] = false;
        }
        let code = risk_code(f.risk);
        let mut stack = vec![n];
        while let Some(k) = stack.pop() {
            lens[k] = lens[k].max(code);
            stack.extend_from_slice(&tree.nodes[k].children);
        }
    }
    Report {
        tree: gen,
        nodes,
        counted,
        held: held.into_iter().map(|(f, why)| (f, why.to_string())).collect(),
        hidden: None,
        owners: facts.owner_map(tree),
        apps: facts.apps.iter().map(|a| a.name.clone()).collect(),
        lens: Arc::new(lens),
        findings,
        secs: started.elapsed().as_secs_f64(),
    }
}

impl State {
    /// The report for the tree on screen, if it is current.
    pub(crate) fn advice(&self) -> Option<&Report> {
        self.tree.as_ref()?;
        self.advisor.report.as_ref().filter(|r| r.tree == self.tree_gen)
    }

    /// Analyse the tree in the background (after a scan, a delete, or on Refresh).
    pub(crate) fn advisor_start(&mut self) {
        // whatever was shown is for an earlier tree, and earlier jobs' answers are dropped
        self.advisor.report = None;
        self.advisor.rx = None;
        self.advisor.tx = None;
        self.advisor.running = false;
        self.advisor.open = None;
        self.advisor.in_use = None;
        self.advisor.note = None;
        let Some(t) = self.tree.clone() else {
            self.advisor.note = Some("Scan a drive or folder, and the advisor shows what can be won back.");
            return self.advisor_changed();
        };
        if t.method == sprawl_core::ScanMethod::Csv {
            self.advisor.note = Some("This view came from a CSV file, which may not match the disk. Scan the folder to get advice.");
            return self.advisor_changed();
        }
        if self.full_tree.is_some() {
            self.advisor.note = Some("A filter is on. Clear it to get advice on the whole scan.");
            return self.advisor_changed();
        }
        let (tx, rx) = channel();
        self.advisor.rx = Some(rx);
        self.advisor.tx = Some(tx.clone());
        self.advisor.running = true;
        let gen = self.tree_gen;
        std::thread::spawn(move || {
            let r = analyse(&t, gen);
            let (root, allocated, errors) = (t.path_of(sprawl_core::ROOT), t.nodes[sprawl_core::ROOT].allocated, t.errors);
            // let go of the tree before the slow part, so the app can change it without a copy
            drop(t);
            if tx.send(Msg::Report(Box::new(r))).is_err() {
                return;
            }
            // starts PowerShell, so it follows on its own
            let shadow = sprawl_advisor::hidden::shadow_storage(&root);
            let _ = tx.send(Msg::Hidden(gen, sprawl_advisor::hidden::account_parts(&root, allocated, errors, shadow)));
        });
        self.advisor_changed();
    }

    /// After the tree on screen changed without a new scan (a filter on or off).
    pub(crate) fn advisor_refresh(&mut self) {
        self.advisor_start();
    }

    /// Called every frame: pick up finished work.
    pub(crate) fn advisor_poll(&mut self) {
        let Some(rx) = &self.advisor.rx else { return };
        let mut changed = false;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Report(r) => {
                    self.advisor.report = Some(*r);
                    self.advisor.running = false;
                    self.advisor.open = None;
                    self.advisor.in_use = None;
                }
                Msg::Hidden(key, h) => {
                    if let Some(r) = self.advisor.report.as_mut().filter(|r| r.tree == key) {
                        r.hidden = h;
                    }
                }
                Msg::InUse(gen, i, answer) => {
                    if gen == self.tree_gen && self.advisor.in_use.as_ref().is_some_and(|(j, _)| *j == i) {
                        self.advisor.in_use = Some((i, answer));
                    }
                }
            }
            changed = true;
        }
        if changed {
            self.advisor_changed();
            self.refresh_rows(); // the Owner column
        }
    }

    fn advisor_changed(&mut self) {
        self.map.drawn_once = false;
        if self.view == View::Advisor {
            self.render_advisor();
        }
    }

    /// The owner of `n` for the Owner column, and whether its parent has the same one.
    pub(crate) fn owner_of(&self, n: usize) -> Option<(&str, bool)> {
        let r = self.advice()?;
        let o = (*r.owners.get(n)?)?;
        let inherited = self.tree.as_ref().and_then(|t| t.nodes[n].parent).and_then(|p| r.owners.get(p).copied().flatten()) == Some(o);
        Some((r.apps.get(o as usize)?.as_str(), inherited))
    }

    pub(crate) fn advisor_command(&mut self, name: &str) {
        match name {
            "AdvLensOpp" => self.advisor.lens_mode = Lens::Opportunities,
            "AdvLensRisk" => self.advisor.lens_mode = Lens::Risk,
            "AdvLensOff" => self.advisor.lens_mode = Lens::Off,
            "AdvRefresh" => return self.advisor_start(),
            "AdvHeld" => self.advisor.show_held = !self.advisor.show_held,
            "AdvCopy" => {
                if let Some(c) = self.advisor.open.and_then(|i| self.advice()?.findings.get(i)?.command.clone()) {
                    win32::copy_text(self.hwnd, &c);
                    crate::state::text(&self.root, "StatusText", "Command copied. Paste it into PowerShell.");
                }
                return;
            }
            "AdvShow" => {
                if let Some(n) = self.advisor.open.and_then(|i| *self.advice()?.nodes.get(i)?) {
                    self.set_view(View::Tree);
                    self.reveal(n);
                }
                return;
            }
            "AdvExplore" => {
                if let Some(n) = self.advisor.open.and_then(|i| *self.advice()?.nodes.get(i)?) {
                    self.selected = Some(n);
                    self.command("mExplore");
                }
                return;
            }
            _ => {}
        }
        self.advisor_changed();
    }

    /// A finding was clicked: open (or close) its details and light it up on the map.
    fn advisor_open(&mut self, i: usize) {
        if self.advisor.open == Some(i) {
            self.advisor.open = None;
            self.advisor.in_use = None;
            return self.advisor_changed();
        }
        self.advisor.open = Some(i);
        self.advisor.in_use = None;
        let node = self.advice().and_then(|r| r.nodes.get(i).copied().flatten());
        if let Some(n) = node {
            self.selected = Some(n);
            let removes = self.advice().is_some_and(|r| matches!(r.findings[i].remedy, Remedy::Delete | Remedy::Command));
            if let (true, Some(t), Some(tx)) = (removes, self.tree.clone(), self.advisor.tx.clone()) {
                self.advisor.in_use = Some((i, InUse::Checking));
                let gen = self.tree_gen;
                std::thread::spawn(move || {
                    let _ = tx.send(Msg::InUse(gen, i, InUse::Done(sprawl_advisor::inuse::in_use_under(&t, n))));
                });
            }
        }
        self.advisor_changed();
    }

    pub(crate) fn render_advisor(&mut self) {
        let Ok(host) = find::<Border>(&self.root, "AdvHost") else { return };
        let xaml = self.advisor_xaml();
        let Ok(el) = load::<UIElement>(&xaml) else {
            eprintln!("advisor panel failed to load");
            return;
        };
        let _ = host.SetChild(&el);
        let Ok(fe) = el.cast::<FrameworkElement>() else { return };
        let count = self.advice().map_or(0, |r| r.findings.len());
        for i in 0..count {
            if let Ok(b) = find::<ButtonBase>(&fe, &format!("AdvF{i}")) {
                if let Ok(t) = b.Click(move |_, _| {
                    with(|s| s.advisor_open(i));
                }) {
                    keep(t);
                }
            }
        }
        for name in ["AdvLensOpp", "AdvLensRisk", "AdvLensOff", "AdvRefresh", "AdvHeld", "AdvCopy", "AdvShow", "AdvExplore"] {
            if let Ok(b) = find::<ButtonBase>(&fe, name) {
                if let Ok(t) = b.Click(move |_, _| {
                    with(|s| s.advisor_command(name));
                }) {
                    keep(t);
                }
            }
        }
    }

    fn advisor_xaml(&self) -> String {
        let mut x = format!(r##"<StackPanel {} Spacing="10" Padding="18,16,18,18">"##, ui::NS);
        let Some(r) = self.advice() else {
            let msg = if self.advisor.running { "Looking for space to win back..." } else { self.advisor.note.unwrap_or("Scan a drive or folder, and the advisor shows what can be won back.") };
            x += &format!(r##"<TextBlock Text="{}" TextWrapping="Wrap" Foreground="#FFB9C2D6" FontSize="13"/>"##, ui::esc(msg));
            if !self.advisor.running && self.tree.is_some() {
                x += r##"<Button x:Name="AdvRefresh" Padding="10,3" FontSize="12" CornerRadius="5" Content="Check again"/>"##;
            }
            x += "</StackPanel>";
            return x;
        };
        let sum = |pred: &dyn Fn(&Finding) -> bool| -> u64 { r.findings.iter().zip(&r.counted).filter(|(f, c)| **c && pred(f)).map(|(f, _)| f.bytes).sum() };
        // what deleting or a command wins back; uninstalling is the user's call, shown below
        let actionable = |f: &Finding| matches!(f.remedy, Remedy::Delete | Remedy::Command);
        let total = sum(&|f| actionable(f));
        let green = sum(&|f| actionable(f) && f.risk == Risk::Green);
        let amber = sum(&|f| actionable(f) && f.risk == Risk::Amber);
        let orange = sum(&|f| actionable(f) && f.risk >= Risk::Orange);
        let lens_btn = |name: &str, label: &str, on: bool| {
            format!(r##"<Button x:Name="{name}" Content="{label}" Padding="10,3" FontSize="12" CornerRadius="5" BorderThickness="0" Background="{}"/>"##, if on { "#FF2C3B63" } else { "Transparent" })
        };
        let lm = self.advisor.lens_mode;
        x += &format!(
            r##"<Grid ColumnSpacing="24"><Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
<StackPanel><TextBlock Text="{total}" FontSize="26" FontWeight="SemiBold"/><TextBlock Text="can be won back" FontSize="12" Foreground="#FF8E97AA"/></StackPanel>
<StackPanel Grid.Column="1"><TextBlock Text="{green}" FontSize="26" FontWeight="SemiBold" Foreground="#FF4CC38A"/><TextBlock Text="of it comes back by itself" FontSize="12" Foreground="#FF8E97AA"/></StackPanel>
<StackPanel Grid.Column="2" HorizontalAlignment="Right" VerticalAlignment="Top" Spacing="6">
<StackPanel Orientation="Horizontal" Spacing="2" Padding="2" CornerRadius="7" Background="#14FFFFFF">{b1}{b2}{b3}</StackPanel>
<Button x:Name="AdvRefresh" HorizontalAlignment="Right" Padding="10,3" FontSize="12" CornerRadius="5" BorderThickness="0" Background="Transparent" Content="Check again"/>
</StackPanel></Grid>"##,
            total = self.size(total),
            green = self.size(green),
            b1 = lens_btn("AdvLensOpp", "Opportunities", lm == Lens::Opportunities),
            b2 = lens_btn("AdvLensRisk", "By risk", lm == Lens::Risk),
            b3 = lens_btn("AdvLensOff", "Normal", lm == Lens::Off),
        );
        if total > 0 {
            let share = |v: u64| (v as f64 * 1000.0 / total as f64).max(if v > 0 { 4.0 } else { 0.0 });
            x += &format!(
                r##"<Grid Height="8" CornerRadius="4" ColumnSpacing="2"><Grid.ColumnDefinitions><ColumnDefinition Width="{}*"/><ColumnDefinition Width="{}*"/><ColumnDefinition Width="{}*"/></Grid.ColumnDefinitions><Rectangle Fill="#FF4CC38A"/><Rectangle Grid.Column="1" Fill="#FFF5B342"/><Rectangle Grid.Column="2" Fill="#FFFF7A45"/></Grid>"##,
                share(green),
                share(amber),
                share(orange)
            );
        }
        x += r##"<StackPanel Orientation="Horizontal" Spacing="16"><StackPanel Orientation="Horizontal" Spacing="6"><Ellipse Width="8" Height="8" Fill="#FF4CC38A"/><TextBlock Text="Comes back by itself" FontSize="11" Foreground="#FF8E97AA"/></StackPanel><StackPanel Orientation="Horizontal" Spacing="6"><Ellipse Width="8" Height="8" Fill="#FFF5B342"/><TextBlock Text="Costs something" FontSize="11" Foreground="#FF8E97AA"/></StackPanel><StackPanel Orientation="Horizontal" Spacing="6"><Ellipse Width="8" Height="8" Fill="#FFFF7A45"/><TextBlock Text="Your own files" FontSize="11" Foreground="#FF8E97AA"/></StackPanel></StackPanel>"##;
        if r.findings.is_empty() {
            x += r##"<TextBlock Text="Nothing to suggest: no known caches, leftovers or idle build outputs worth the trouble." TextWrapping="Wrap" Foreground="#FFB9C2D6"/>"##;
        }
        for (i, f) in r.findings.iter().enumerate() {
            let open = self.advisor.open == Some(i);
            let sub = match &f.owner {
                Some(o) if !f.title.contains(o.as_str()) => format!("{o} · {}", f.path.display()),
                _ => f.path.display().to_string(),
            };
            x += &format!(
                r##"<Button x:Name="AdvF{i}" HorizontalAlignment="Stretch" HorizontalContentAlignment="Stretch" Padding="10,7" CornerRadius="7" BorderThickness="0" Background="{bg}">
<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="10"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="86"/></Grid.ColumnDefinitions>
<Ellipse Width="9" Height="9" Fill="{dot}" VerticalAlignment="Center"/>
<StackPanel Grid.Column="1" Spacing="1"><TextBlock Text="{title}" FontSize="13" TextTrimming="CharacterEllipsis"/><TextBlock Text="{sub}" FontSize="11" Foreground="#FF8E97AA" TextTrimming="CharacterEllipsis"/></StackPanel>
<Border Grid.Column="2" Padding="7,1" CornerRadius="4" Background="#1AFFFFFF" VerticalAlignment="Center"><TextBlock Text="{remedy}" FontSize="11" Foreground="#FFB9C2D6"/></Border>
<TextBlock Grid.Column="3" Text="{size}" FontSize="13" HorizontalAlignment="Right" VerticalAlignment="Center" Opacity="{op}"/>
</Grid></Button>"##,
                bg = if open { "#FF262C3D" } else { "#0CFFFFFF" },
                dot = risk_hex(f.risk),
                title = ui::esc(&f.title),
                sub = ui::esc(&sub),
                remedy = remedy_label(f.remedy),
                size = self.size(f.bytes),
                op = if r.counted[i] { "1" } else { "0.55" },
            );
            if open {
                x += &self.finding_detail(r, i);
            }
        }
        if let Some(h) = r.hidden.as_ref().filter(|h| h.notable) {
            x += &format!(
                r##"<Border Padding="12,10" CornerRadius="7" Background="#0CFFFFFF"><StackPanel Spacing="6"><TextBlock Text="{} used on the drive that the scan can't see" FontSize="13" FontWeight="SemiBold"/>"##,
                self.size(h.gap.unsigned_abs())
            );
            for p in &h.parts {
                let size = p.bytes.map(|b| format!(" ({})", self.size(b))).unwrap_or_default();
                x += &format!(r##"<TextBlock TextWrapping="Wrap" FontSize="12" Foreground="#FFB9C2D6"><Run Text="{}{size}: " FontWeight="SemiBold"/><Run Text="{}"/></TextBlock>"##, ui::esc(p.label), ui::esc(&p.note));
            }
            x += "</StackPanel></Border>";
        }
        if !r.held.is_empty() {
            x += &format!(
                r##"<Button x:Name="AdvHeld" Padding="10,4" FontSize="12" CornerRadius="5" BorderThickness="0" Background="Transparent" Content="{} held back by the safety checks ({})"/>"##,
                r.held.len(),
                if self.advisor.show_held { "hide" } else { "show" }
            );
            if self.advisor.show_held {
                for (f, why) in &r.held {
                    x += &format!(
                        r##"<TextBlock TextWrapping="Wrap" FontSize="12" Foreground="#FF8E97AA" Margin="10,0,0,0"><Run Text="{} ({})" Foreground="#FFB9C2D6"/><Run Text=": {}"/></TextBlock>"##,
                        ui::esc(&f.title),
                        self.size(f.bytes),
                        ui::esc(why)
                    );
                }
            }
        }
        x += &format!(
            r##"<TextBlock TextWrapping="Wrap" FontSize="11" Foreground="#FF6F788C" Text="Sprawl only suggests: nothing is deleted or run from this tab. Commands are for PowerShell. Checked in {:.1} s."/>"##,
            r.secs
        );
        x += "</StackPanel>";
        x
    }

    fn finding_detail(&self, r: &Report, i: usize) -> String {
        let f = &r.findings[i];
        let mut x = String::from(r##"<Border Margin="20,0,0,4" Padding="12,10" CornerRadius="7" Background="#0AFFFFFF"><StackPanel Spacing="6">"##);
        for e in &f.evidence {
            x += &format!(r##"<TextBlock TextWrapping="Wrap" FontSize="12" Foreground="#FFB9C2D6" Text="• {}"/>"##, ui::esc(e));
        }
        x += &format!(r##"<TextBlock TextWrapping="Wrap" FontSize="11" Foreground="#FF8E97AA" IsTextSelectionEnabled="True" Text="{}"/>"##, ui::esc(&f.path.display().to_string()));
        let line = |colour: &str, text: &str| format!(r##"<TextBlock TextWrapping="Wrap" FontSize="12" Foreground="{colour}" Text="{}"/>"##, ui::esc(text));
        match &self.advisor.in_use {
            Some((j, state)) if *j == i => {
                x += &match state {
                    InUse::Checking => line("#FF8E97AA", "Checking which programs use it..."),
                    InUse::Done(None) => line("#FF8E97AA", "Couldn't check which programs use it."),
                    InUse::Done(Some(c)) if !c.names.is_empty() => {
                        line("#FFF5B342", &format!("In use by {}: close it first, or files still open are skipped.", c.names.join(", ")))
                    }
                    InUse::Done(Some(c)) if c.all_files => line("#FF8E97AA", "No program has its files open."),
                    InUse::Done(Some(_)) => line("#FF8E97AA", "No program has the first 10,000 of its files open."),
                }
            }
            _ => {}
        }
        if let Some(c) = &f.command {
            x += &format!(
                r##"<TextBlock Text="PowerShell" FontSize="11" Foreground="#FF8E97AA"/><Border Padding="10,8" CornerRadius="5" Background="#33000000"><TextBlock FontFamily="Consolas" FontSize="12" TextWrapping="Wrap" IsTextSelectionEnabled="True" Text="{}"/></Border>"##,
                ui::esc(c)
            );
        }
        x += r##"<StackPanel Orientation="Horizontal" Spacing="6">"##;
        if f.command.is_some() {
            x += r##"<Button x:Name="AdvCopy" Content="Copy command" Padding="10,3" FontSize="12" CornerRadius="5"/>"##;
        }
        if r.nodes[i].is_some() {
            x += r##"<Button x:Name="AdvShow" Content="Show in tree" Padding="10,3" FontSize="12" CornerRadius="5"/><Button x:Name="AdvExplore" Content="Show in Explorer" Padding="10,3" FontSize="12" CornerRadius="5"/>"##;
        }
        x += "</StackPanel></StackPanel></Border>";
        x
    }
}
