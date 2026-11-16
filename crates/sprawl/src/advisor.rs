//! The Advisor tab: what can be won back, with evidence, the command to run and a treemap lens.
//!
//! The analysis (rules, app facts, projects, safety checks) runs on a background thread after
//! every scan; the report is tied to the exact tree it was made from and ignored once that tree
//! changes. Nothing here deletes or runs anything: commands are shown for the user to copy, or
//! written into a PowerShell script the user runs (a dry run unless given `-Run`).
//!
//! Phase 2 adds the model ("Ask Claude"), goal mode ("I need N GB" gives the lowest-risk plan,
//! previewed on the map as tiles shrinking away), the script export and a verify step that
//! measures what was really freed.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use sprawl_advisor::config::{AiConfig, Provider};
use sprawl_advisor::facts::Facts;
use sprawl_advisor::hidden::HiddenSpace;
use sprawl_advisor::llm::agent::AiRun;
use sprawl_advisor::plan::Plan;
use sprawl_advisor::rules::RM_FN;
use sprawl_advisor::{Finding, Remedy, Risk, RulePack, Source};
use sprawl_core::Tree;
use windows_core::Interface;

use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, TextBox};
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
    /// A status line from the model run.
    AiProgress(u64, String),
    AiDone(u64, Result<AiRun, String>),
    /// (path, size planned, size now) of every exported item.
    Verified(Vec<(PathBuf, u64, u64)>),
}

/// The model run for the current report.
#[derive(Default)]
enum Ai {
    #[default]
    Idle,
    Running(String),
    Done { summary: String, model: String, found: usize, rejected: Vec<(Finding, String)>, with_tools: bool, turns: usize },
    Failed(String),
}

/// A goal plan for the current report, and its map preview.
struct Planned {
    gen: u64,
    plan: Plan,
    /// Risk code of every node the plan removes (0 elsewhere): the preview's lens and what shrinks.
    lens: Arc<Vec<u8>>,
}

/// The last script written, and what verifying it found.
struct Exported {
    file: PathBuf,
    /// (path, size when exported)
    items: Vec<(PathBuf, u64)>,
    verify: Option<Option<Vec<(PathBuf, u64, u64)>>>,
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
    /// The panel as loaded, for its named parts (its own namescope).
    panel: Option<FrameworkElement>,
    /// What is in the "I need __ GB" box, kept across redraws.
    goal_text: String,
    ai: Ai,
    plan: Option<Planned>,
    /// The plan's tiles shrink away on the map.
    pub(crate) preview: bool,
    exported: Option<Exported>,
    show_rejected: bool,
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

fn risk_words(r: Risk) -> &'static str {
    match r {
        Risk::Green => "things that come back by themselves",
        Risk::Amber => "things that cost something to get back",
        Risk::Orange => "your own files: check each one",
        Risk::Red => "never",
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
        self.advisor.ai = Ai::Idle;
        self.advisor.plan = None;
        self.advisor.preview = false;
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
        let msgs: Vec<Msg> = rx.try_iter().collect();
        let mut changed = false;
        for m in msgs {
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
                Msg::AiProgress(gen, line) => {
                    // only the status line changes, so typing in the panel isn't disturbed
                    if gen == self.tree_gen && matches!(self.advisor.ai, Ai::Running(_)) {
                        if let Some(p) = &self.advisor.panel {
                            crate::state::text(p, "AdvAiStatus", &line);
                        }
                        self.advisor.ai = Ai::Running(line);
                    }
                    continue;
                }
                Msg::AiDone(gen, result) => {
                    if gen == self.tree_gen && self.advice().is_some() {
                        self.advisor.ai = match result {
                            Ok(run) => self.merge_ai(run),
                            Err(e) => Ai::Failed(e),
                        };
                    }
                }
                Msg::Verified(v) => {
                    if let Some(e) = &mut self.advisor.exported {
                        e.verify = Some(Some(v));
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
            "AdvRejected" => self.advisor.show_rejected = !self.advisor.show_rejected,
            "AdvAsk" => self.advisor_ask(),
            "AdvSetup" => return self.command("mWizard"),
            "AdvPlan" => self.advisor_plan(),
            "AdvPreview" => self.advisor.preview = !self.advisor.preview,
            "AdvClearPlan" => {
                self.advisor.plan = None;
                self.advisor.preview = false;
            }
            "AdvExport" => self.advisor_export(),
            "AdvVerify" => self.advisor_verify(),
            "AdvLog" => {
                match sprawl_advisor::llm::request_log_path().filter(|p| p.is_file()) {
                    Some(p) => {
                        let _ = std::process::Command::new("notepad.exe").arg(p).spawn();
                    }
                    None => crate::state::text(&self.root, "StatusText", "Nothing has been sent to an AI yet."),
                }
                return;
            }
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
            if let (true, Some(t), Some(tx)) = (removes, self.tree.as_ref(), self.advisor.tx.clone()) {
                use sprawl_advisor::inuse;
                self.advisor.in_use = Some((i, InUse::Checking));
                let gen = self.tree_gen;
                // the paths here, so the thread doesn't hold the tree
                let files = inuse::files_under(t, n, inuse::MAX_FILES + 1);
                std::thread::spawn(move || {
                    let _ = tx.send(Msg::InUse(gen, i, InUse::Done(inuse::check_files(files))));
                });
            }
        }
        self.advisor_changed();
    }

    pub(crate) fn render_advisor(&mut self) {
        // keep what was typed in the goal box across the redraw
        if let Some(t) = self.advisor.panel.as_ref().and_then(|p| find::<TextBox>(p, "AdvGoal").ok()) {
            if let Ok(h) = t.Text() {
                self.advisor.goal_text = h.to_string_lossy();
            }
        }
        let Ok(host) = find::<Border>(&self.root, "AdvHost") else { return };
        let xaml = self.advisor_xaml();
        let Ok(el) = load::<UIElement>(&xaml) else {
            eprintln!("advisor panel failed to load");
            return;
        };
        let _ = host.SetChild(&el);
        let Ok(fe) = el.cast::<FrameworkElement>() else { return };
        self.advisor.panel = Some(fe.clone());
        if let Ok(t) = find::<TextBox>(&fe, "AdvGoal") {
            let _ = t.SetText(&self.advisor.goal_text.as_str().into());
        }
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
        for name in [
            "AdvLensOpp", "AdvLensRisk", "AdvLensOff", "AdvRefresh", "AdvHeld", "AdvCopy", "AdvShow", "AdvExplore", "AdvAsk", "AdvSetup", "AdvPlan", "AdvPreview", "AdvClearPlan",
            "AdvExport", "AdvVerify", "AdvLog", "AdvRejected",
        ] {
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
        x += &self.ai_card(r);
        if r.findings.is_empty() {
            x += r##"<TextBlock Text="Nothing to suggest: no known caches, leftovers or idle build outputs worth the trouble." TextWrapping="Wrap" Foreground="#FFB9C2D6"/>"##;
        }
        let in_plan = |i: usize| self.current_plan().is_some_and(|p| p.plan.picked.contains(&i));
        let mut model_header = false;
        for (i, f) in r.findings.iter().enumerate() {
            if let (Source::Model(m), false) = (&f.source, model_header) {
                model_header = true;
                x += &format!(r##"<TextBlock Text="Found by {}" FontSize="12" FontWeight="SemiBold" Foreground="#FFB9C2D6" Margin="0,6,0,0"/>"##, ui::esc(m));
            }
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
<StackPanel Grid.Column="2" Orientation="Horizontal" Spacing="4" VerticalAlignment="Center">{plan}<Border Padding="7,1" CornerRadius="4" Background="#1AFFFFFF"><TextBlock Text="{remedy}" FontSize="11" Foreground="#FFB9C2D6"/></Border></StackPanel>
<TextBlock Grid.Column="3" Text="{size}" FontSize="13" HorizontalAlignment="Right" VerticalAlignment="Center" Opacity="{op}"/>
</Grid></Button>"##,
                bg = if open { "#FF262C3D" } else { "#0CFFFFFF" },
                dot = risk_hex(f.risk),
                title = ui::esc(&f.title),
                sub = ui::esc(&sub),
                remedy = remedy_label(f.remedy),
                plan = if in_plan(i) { r##"<Border Padding="7,1" CornerRadius="4" Background="#332F7BF6"><TextBlock Text="In plan" FontSize="11" Foreground="#FF8FB4FF"/></Border>"## } else { "" },
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

    /// The goal plan, if it is for the report on screen.
    fn current_plan(&self) -> Option<&Planned> {
        self.advisor.plan.as_ref().filter(|p| p.gen == self.tree_gen && self.advice().is_some())
    }

    /// The preview's lens, while it is on.
    pub(crate) fn plan_preview(&self) -> Option<Arc<Vec<u8>>> {
        self.advisor.preview.then(|| self.current_plan().map(|p| p.lens.clone())).flatten()
    }

    fn goal_bytes(&self) -> Option<u64> {
        let t = self.advisor.goal_text.trim().trim_end_matches(['G', 'g', 'B', 'b', ' ']).replace(',', ".");
        let gb: f64 = t.parse().ok()?;
        (gb > 0.0 && gb < 1e6).then(|| (gb * (1u64 << 30) as f64) as u64)
    }

    /// Put `v` in the goal box (screenshot runs).
    pub(crate) fn advisor_set_goal(&mut self, v: &str) {
        self.advisor.goal_text = v.to_string();
        if let Some(t) = self.advisor.panel.as_ref().and_then(|p| find::<TextBox>(p, "AdvGoal").ok()) {
            let _ = t.SetText(&v.into());
        }
    }

    fn read_goal(&mut self) {
        if let Some(t) = self.advisor.panel.as_ref().and_then(|p| find::<TextBox>(p, "AdvGoal").ok()) {
            if let Ok(h) = t.Text() {
                self.advisor.goal_text = h.to_string_lossy();
            }
        }
    }

    /// "Make a plan": the lowest-risk findings that free the goal, previewed on the map.
    fn advisor_plan(&mut self) {
        self.read_goal();
        let Some(need) = self.goal_bytes() else {
            crate::state::text(&self.root, "StatusText", "Type how many GB you need, like 20.");
            return;
        };
        let (Some(t), Some(r)) = (self.tree.as_ref(), self.advice()) else { return };
        let plan = sprawl_advisor::plan::plan(&r.findings, &r.counted, need);
        let mut lens = vec![0u8; t.nodes.len()];
        for &i in &plan.picked {
            let Some(n) = r.nodes[i] else { continue };
            let code = risk_code(r.findings[i].risk);
            let mut stack = vec![n];
            while let Some(k) = stack.pop() {
                lens[k] = code;
                stack.extend_from_slice(&t.nodes[k].children);
            }
        }
        self.advisor.plan = Some(Planned { gen: self.tree_gen, plan, lens: Arc::new(lens) });
        self.advisor.preview = true;
        self.map.scene.shrink_restart = true;
    }

    /// "Ask Claude": the model looks at the report and the tree, in the background.
    fn advisor_ask(&mut self) {
        if matches!(self.advisor.ai, Ai::Running(_)) {
            return;
        }
        self.read_goal();
        let cfg = AiConfig::load();
        if cfg.provider == Provider::Off {
            self.advisor.ai = Ai::Failed("AI is off. Choose a provider with Set up AI.".into());
            return;
        }
        let (Some(t), Some(r), Some(tx)) = (self.tree.clone(), self.advice(), self.advisor.tx.clone()) else { return };
        // only what the rules found: the model's own earlier findings are found again if still true
        let already: Vec<Finding> = r.findings.iter().filter(|f| !matches!(f.source, Source::Model(_))).cloned().collect();
        let hidden = r.hidden.clone();
        let goal = self.goal_bytes();
        let gen = self.tree_gen;
        self.advisor.ai = Ai::Running("Starting...".into());
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let mut progress = move |line: String| {
                let _ = progress_tx.send(Msg::AiProgress(gen, line));
            };
            let result = sprawl_advisor::llm::agent::run(&t, &cfg, &already, hidden.as_ref(), goal, &mut progress);
            let _ = tx.send(Msg::AiDone(gen, result));
        });
    }

    /// Add the model's findings to the report (after the rules' ones, so indexes stay).
    fn merge_ai(&mut self, run: AiRun) -> Ai {
        let gen = self.tree_gen;
        let (Some(t), Some(r)) = (self.tree.clone(), self.advisor.report.as_mut().filter(|r| r.tree == gen)) else { return Ai::Idle };
        // the model's findings from an earlier ask go; this answer replaces them
        let keep = r.findings.iter().position(|f| matches!(f.source, Source::Model(_))).unwrap_or(r.findings.len());
        if keep < r.findings.len() {
            r.findings.truncate(keep);
            r.nodes.truncate(keep);
            r.counted.truncate(keep);
            // rebuild the lens from the rules' findings alone
            let mut lens = vec![0u8; t.nodes.len()];
            for (f, n) in r.findings.iter().zip(&r.nodes) {
                let Some(n) = *n else { continue };
                let code = risk_code(f.risk);
                let mut stack = vec![n];
                while let Some(k) = stack.pop() {
                    lens[k] = lens[k].max(code);
                    stack.extend_from_slice(&t.nodes[k].children);
                }
            }
            r.lens = Arc::new(lens);
        }
        let found = run.findings.len();
        let lens = Arc::make_mut(&mut r.lens);
        for f in run.findings {
            let n = sprawl_advisor::find_path(&t, &f.path);
            r.counted.push(n.is_none_or(|n| lens.get(n).copied().unwrap_or(0) == 0));
            if let Some(n) = n {
                let code = risk_code(f.risk);
                let mut stack = vec![n];
                while let Some(k) = stack.pop() {
                    lens[k] = lens[k].max(code);
                    stack.extend_from_slice(&t.nodes[k].children);
                }
            }
            r.nodes.push(n);
            r.findings.push(f);
        }
        Ai::Done { summary: run.summary, model: run.model, found, rejected: run.rejected, with_tools: run.with_tools, turns: run.turns }
    }

    /// Write the plan (or every finding with something to run) as a PowerShell script.
    fn advisor_export(&mut self) {
        let Some(r) = self.advice() else { return };
        let picked: Vec<usize> = match self.current_plan() {
            Some(p) => p.plan.picked.clone(),
            None => (0..r.findings.len()).filter(|&i| r.counted[i] && r.findings[i].command.is_some()).collect(),
        };
        if picked.is_empty() {
            crate::state::text(&self.root, "StatusText", "Nothing to export: no finding has a command to run.");
            return;
        }
        let list: Vec<&Finding> = picked.iter().map(|&i| &r.findings[i]).collect();
        let root = self.tree.as_ref().map(|t| t.path_of(sprawl_core::ROOT).display().to_string()).unwrap_or_default();
        let text = sprawl_advisor::export::script(&list, &root, std::time::SystemTime::now());
        let items: Vec<(PathBuf, u64)> = list.iter().filter(|f| f.command.is_some()).map(|f| (f.path.clone(), f.bytes)).collect();
        let file = crate::format::user_dir("Documents").join(format!("Sprawl clean-up {}.ps1", crate::format::stamp()));
        match sprawl_advisor::export::write_script(&file, &text) {
            Ok(()) => {
                let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", file.display())).spawn();
                crate::state::text(&self.root, "StatusText", "Script saved. Read it, then run it in PowerShell: as it is, it only measures.");
                self.advisor.exported = Some(Exported { file, items, verify: None });
            }
            Err(e) => crate::state::text(&self.root, "StatusText", &format!("Couldn't save the script: {e}")),
        }
    }

    /// Measure what the exported items hold now, to see what running the script freed.
    fn advisor_verify(&mut self) {
        let Some(e) = &mut self.advisor.exported else { return };
        let Some(tx) = self.advisor.tx.clone() else { return };
        e.verify = Some(None);
        let items = e.items.clone();
        std::thread::spawn(move || {
            let out = items
                .into_iter()
                .map(|(p, before)| {
                    let now = if std::fs::symlink_metadata(&p).is_err() {
                        0
                    } else {
                        sprawl_core::scan(&p, |_| {}).map(|t| t.nodes[sprawl_core::ROOT].size).unwrap_or(before)
                    };
                    (p, before, now)
                })
                .collect();
            let _ = tx.send(Msg::Verified(out));
        });
    }

    fn ai_card(&self, r: &Report) -> String {
        let cfg = AiConfig::load();
        let btn = |name: &str, label: &str| format!(r##"<Button x:Name="{name}" Content="{label}" Padding="10,3" FontSize="12" CornerRadius="5"/>"##);
        let ask = match cfg.provider {
            Provider::Off => btn("AdvSetup", "Set up AI"),
            Provider::Anthropic => btn("AdvAsk", "Ask Claude"),
            Provider::ClaudeCode => btn("AdvAsk", "Ask Claude Code"),
            Provider::OpenAi => btn("AdvAsk", &format!("Ask {}", ui::esc(if cfg.model.is_empty() { "the model" } else { &cfg.model }))),
        };
        let mut x = String::from(r##"<Border Padding="12,10" CornerRadius="7" Background="#0CFFFFFF"><StackPanel Spacing="8">"##);
        x += &format!(
            r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="I need" VerticalAlignment="Center" FontSize="13"/><TextBox x:Name="AdvGoal" Width="70" PlaceholderText="20"/><TextBlock Text="GB" VerticalAlignment="Center" FontSize="13"/>{}{}{}</StackPanel>"##,
            btn("AdvPlan", "Make a plan"),
            ask,
            btn("AdvExport", if self.current_plan().is_some() { "Export plan as script" } else { "Export script" }),
        );
        if let Some(p) = self.current_plan() {
            let n = p.plan.picked.len();
            let worst = p.plan.picked.iter().map(|&i| r.findings[i].risk).max();
            let msg = if n == 0 {
                "No finding has something to run, so there is no plan yet.".to_string()
            } else if p.plan.enough() {
                let items = if n == 1 { "1 item frees".to_string() } else { format!("{n} items free") };
                format!("{items} {} of the {} you need, at worst {}.", self.size(p.plan.bytes), self.size(p.plan.need), worst.map(risk_words).unwrap_or(""))
            } else {
                format!("Everything found frees {}, short of the {} you need. Ask the AI to look further.", self.size(p.plan.bytes), self.size(p.plan.need))
            };
            x += &format!(
                r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" TextWrapping="Wrap" FontSize="12" Foreground="#FF8FB4FF" VerticalAlignment="Center"/>{}{}</StackPanel>"##,
                ui::esc(&msg),
                btn("AdvPreview", if self.advisor.preview { "Stop preview" } else { "Preview on map" }),
                btn("AdvClearPlan", "Clear"),
            );
        }
        match &self.advisor.ai {
            Ai::Idle => {}
            Ai::Running(s) => x += &format!(r##"<TextBlock x:Name="AdvAiStatus" Text="{}" TextWrapping="Wrap" FontSize="12" Foreground="#FF8E97AA"/>"##, ui::esc(s)),
            Ai::Failed(e) => x += &format!(r##"<TextBlock Text="{}" TextWrapping="Wrap" FontSize="12" Foreground="#FFFF7A45"/>"##, ui::esc(e)),
            Ai::Done { summary, model, found, rejected, with_tools, turns } => {
                if !summary.is_empty() {
                    x += &format!(r##"<TextBlock Text="{}" TextWrapping="Wrap" FontSize="13" Foreground="#FFD5DBE8"/>"##, ui::esc(summary));
                }
                let how = if *with_tools { format!("{turns} turns, looking closer with tools") } else { "one answer from the report".into() };
                x += &format!(
                    r##"<TextBlock Text="{} by {} ({how}). Its deletes are at least orange: check them yourself." TextWrapping="Wrap" FontSize="11" Foreground="#FF8E97AA"/>"##,
                    match found {
                        0 => "Nothing more found".to_string(),
                        1 => "1 more finding".to_string(),
                        n => format!("{n} more findings"),
                    },
                    ui::esc(model)
                );
                if !rejected.is_empty() {
                    x += &format!(
                        r##"<Button x:Name="AdvRejected" Padding="10,2" FontSize="11" CornerRadius="5" BorderThickness="0" Background="Transparent" Content="{} claims didn't pass the checks ({})"/>"##,
                        rejected.len(),
                        if self.advisor.show_rejected { "hide" } else { "show" }
                    );
                    if self.advisor.show_rejected {
                        for (f, why) in rejected {
                            x += &format!(r##"<TextBlock TextWrapping="Wrap" FontSize="11" Foreground="#FF8E97AA" Margin="10,0,0,0" Text="{}: {}"/>"##, ui::esc(&f.path.display().to_string()), ui::esc(why));
                        }
                    }
                }
            }
        }
        if let Some(e) = &self.advisor.exported {
            let planned: u64 = e.items.iter().map(|(_, b)| *b).sum();
            let line = match &e.verify {
                None => format!("Saved {} ({} items, {}). After running it with -Run, check what it freed.", e.file.display(), e.items.len(), self.size(planned)),
                Some(None) => "Measuring the exported items...".to_string(),
                Some(Some(v)) => {
                    let freed: u64 = v.iter().map(|(_, b, a)| b.saturating_sub(*a)).sum();
                    let done = v.iter().filter(|(_, b, a)| a < b).count();
                    format!("Freed {} of the {} planned: {done} of {} items are gone or smaller. Scan again for the full picture.", self.size(freed), self.size(planned), v.len())
                }
            };
            x += &format!(
                r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" TextWrapping="Wrap" FontSize="12" Foreground="#FFB9C2D6" VerticalAlignment="Center" MaxWidth="560"/>{}</StackPanel>"##,
                ui::esc(&line),
                btn("AdvVerify", "Check what it freed")
            );
        }
        if cfg.provider != Provider::Off {
            x += r##"<Button x:Name="AdvLog" Padding="0" FontSize="11" BorderThickness="0" Background="Transparent" Foreground="#FF6F788C" Content="See what was sent to the AI"/>"##;
        }
        x += "</StackPanel></Border>";
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
            // the safe-delete functions are long and the same every time: shown folded, copied whole
            let (shown, label) = match c.strip_prefix(RM_FN) {
                Some(rest) => (format!("(Sprawl's safe-delete functions){rest}"), "PowerShell. Copy includes Sprawl's safe-delete functions, which never follow links."),
                None => (c.clone(), "PowerShell"),
            };
            x += &format!(
                r##"<TextBlock Text="{}" FontSize="11" Foreground="#FF8E97AA" TextWrapping="Wrap"/><Border Padding="10,8" CornerRadius="5" Background="#33000000"><TextBlock FontFamily="Consolas" FontSize="12" TextWrapping="Wrap" IsTextSelectionEnabled="True" Text="{}"/></Border>"##,
                label,
                ui::esc(&shown)
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
