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
use sprawl_advisor::llm::agent::{AiRun, ChatEvent};
use sprawl_advisor::memory::{self, Memory, Verdict};
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
    /// Hidden by the user's decisions ("Keep it", "Not now").
    kept: Vec<(Finding, Verdict)>,
    hidden: Option<HiddenSpace>,
    /// How this folder grew, from History's snapshots (arrives after the report).
    growth: Option<sprawl_advisor::growth::Growth>,
    /// Owning app of every node (index into `apps`).
    owners: Vec<Option<u32>>,
    apps: Vec<String>,
    /// How each app is uninstalled, when Sprawl can take the user there.
    uninstall: Vec<Option<sprawl_advisor::facts::Uninstall>>,
    /// Risk code of every node inside a finding: 0 none, 1 green, 2 amber, 3 orange.
    pub(crate) lens: Arc<Vec<u8>>,
    secs: f64,
}

enum Msg {
    Report(Box<Report>),
    Hidden(u64, Option<HiddenSpace>),
    Growth(u64, Option<sprawl_advisor::growth::Growth>),
    /// (tree generation, finding, answer)
    InUse(u64, usize, InUse),
    /// A status line from the model run.
    AiProgress(u64, String),
    AiDone(u64, Result<AiRun, String>),
    /// The script measured, and (path, size planned, size now) of every item in it.
    Verified(PathBuf, Vec<(PathBuf, u64, u64)>),
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
    /// (path, size when exported, title)
    items: Vec<(PathBuf, u64, String)>,
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
    /// Preferences, decisions, labels and proposed rules (loaded on the first analysis).
    pub(crate) memory: Memory,
    memory_loaded: bool,
    show_kept: bool,
    show_memory: bool,
    chat: Chat,
}

/// "Ask your disk": its own thread and channel, so it outlives filter changes; a new scan ends it.
#[derive(Default)]
struct Chat {
    /// Questions to the chat thread.
    tx: Option<Sender<String>>,
    rx: Option<Receiver<ChatMsg>>,
    /// (from the user, text)
    log: Vec<(bool, String)>,
    status: Option<String>,
    /// What is typed in the question box, kept across redraws.
    text: String,
    /// A web search waiting for the user's yes or no.
    consent: Option<(String, Sender<bool>)>,
    /// "Allow for this session" was chosen.
    web_ok: Arc<std::sync::atomic::AtomicBool>,
}

enum ChatMsg {
    Event(ChatEvent),
    Consent(String, Sender<bool>),
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

/// `f` is below another finding in `all`.
fn inside_another(all: &[Finding], f: &Finding) -> bool {
    let p = f.path.to_string_lossy().to_lowercase();
    let segs: Vec<&str> = p.split('\\').filter(|s| !s.is_empty()).collect();
    all.iter().any(|o| {
        let q = o.path.to_string_lossy().to_lowercase();
        let k: Vec<&str> = q.split('\\').filter(|s| !s.is_empty()).collect();
        k.len() < segs.len() && segs[..k.len()] == k[..]
    })
}

/// The Advisor's findings as text, for Claude Code.
fn findings_text(r: &Report) -> String {
    let mut s = String::new();
    for f in &r.findings {
        s += &format!("- [{:?}] {} ({}): {}\n", f.risk, f.title, sprawl_core::format_size(f.bytes), f.path.display());
        for e in &f.evidence {
            s += &format!("    {e}\n");
        }
        if let Some(c) = &f.command {
            // the functions are long and the same each time
            let shown = c.strip_prefix(RM_FN).map(|r| format!("(Sprawl's safe-delete functions){r}")).unwrap_or_else(|| c.clone());
            s += &format!("    PowerShell: {shown}\n");
        }
    }
    s
}

/// What the goal box shows when empty, and what an empty box means.
const GOAL_EXAMPLE: &str = "20";

/// Questions offered before the first one.
const EXAMPLES: [&str; 4] = ["What is the biggest thing here?", "Show me videos over 1 GB", "What did I download and forget?", "What can I safely clean up?"];

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
fn analyse(tree: &Arc<Tree>, gen: u64, memory: &Memory) -> (Report, Facts) {
    let started = std::time::Instant::now();
    let mut facts = Facts::gather();
    facts.attach_usage(tree);
    // the built-in rules, then the user's own (approved proposals): never trusted
    let mut packs = vec![RulePack::builtin()];
    packs.extend(memory::local_packs());
    let (findings, held) = sprawl_advisor::advise_all(tree, &packs, &facts);
    let (findings, kept) = memory.filter(findings);
    let nodes: Vec<Option<usize>> = findings.iter().map(|f| sprawl_advisor::find_path(tree, &f.path)).collect();
    let mut lens = vec![0u8; tree.nodes.len()];
    // inside another finding: shown, but its bytes are already in that one's
    let counted: Vec<bool> = findings.iter().map(|f| !inside_another(&findings, f)).collect();
    for (f, n) in findings.iter().zip(&nodes) {
        let Some(n) = *n else { continue };
        let code = risk_code(f.risk);
        let mut stack = vec![n];
        while let Some(k) = stack.pop() {
            lens[k] = lens[k].max(code);
            stack.extend_from_slice(&tree.nodes[k].children);
        }
    }
    let r = Report {
        tree: gen,
        nodes,
        counted,
        held: held.into_iter().map(|(f, why)| (f, why.to_string())).collect(),
        kept,
        hidden: None,
        growth: None,
        owners: facts.owner_map(tree),
        apps: facts.apps.iter().map(|a| a.name.clone()).collect(),
        uninstall: facts.apps.iter().map(|a| a.uninstall()).collect(),
        lens: Arc::new(lens),
        findings,
        secs: started.elapsed().as_secs_f64(),
    };
    (r, facts)
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
        if !self.advisor.memory_loaded {
            self.advisor.memory = Memory::load();
            self.advisor.memory_loaded = true;
            crate::bridge::publish_memory(&self.advisor.memory);
        }
        // Claude Code sees the whole scan, also while a filter is on
        crate::bridge::publish_tree(self.full_tree.clone().or_else(|| self.tree.clone()));
        crate::bridge::publish_findings(String::new());
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
        let memory = self.advisor.memory.clone();
        std::thread::spawn(move || {
            let (r, facts) = analyse(&t, gen, &memory);
            let (root, allocated, errors) = (t.path_of(sprawl_core::ROOT), t.nodes[sprawl_core::ROOT].allocated, t.errors);
            if tx.send(Msg::Report(Box::new(r))).is_err() {
                return;
            }
            // growth reads a few snapshots: after the report, so that shows first
            let free = sprawl_advisor::hidden::free_space(&root).map(|(_, f)| f);
            let growth = sprawl_advisor::history::base_dir().and_then(|b| sprawl_advisor::growth::growth(&t, &b, &facts, free, std::time::SystemTime::now()));
            if tx.send(Msg::Growth(gen, growth)).is_err() {
                return;
            }
            // let go of the tree before the slow part, so the app can change it without a copy
            drop(t);
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
        self.chat_poll();
        for f in crate::bridge::take_filters() {
            if let Ok(t) = find::<TextBox>(&self.root, "FilterBox") {
                let _ = t.SetText(&f.as_str().into());
            }
        }
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
                Msg::Growth(key, g) => {
                    if let Some(r) = self.advisor.report.as_mut().filter(|r| r.tree == key) {
                        r.growth = g;
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
                Msg::Verified(file, v) => {
                    if let Some(e) = self.advisor.exported.as_mut().filter(|e| e.file == file) {
                        // what really shrank counts as done: the model learns the user is fine with it
                        for (p, before, after) in &v {
                            if after < before {
                                let title = e.items.iter().find(|(q, _, _)| q == p).map(|(_, _, t)| t.clone()).unwrap_or_default();
                                self.advisor.memory.decide(p, &title, Verdict::Done);
                            }
                        }
                        e.verify = Some(Some(v));
                    }
                    self.save_memory();
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
        if let Some(r) = self.advice() {
            crate::bridge::publish_findings(findings_text(r));
        }
        if self.view == View::Advisor {
            self.render_advisor();
        }
    }

    /// The owner of `n` for the Owner column, whether its parent has the same one, and whether
    /// Sprawl knows where to uninstall it.
    pub(crate) fn owner_of(&self, n: usize) -> Option<(&str, bool, bool)> {
        let r = self.advice()?;
        let Some(o) = r.owners.get(n).copied().flatten() else {
            // no app owns it: what the user or the model said it is
            let t = self.tree.as_ref()?;
            let label = self.advisor.memory.label_for(&t.path_of(n))?;
            let parent_same = t.nodes[n].parent.is_some_and(|p| self.advisor.memory.label_for(&t.path_of(p)).is_some_and(|l| l.label == label.label));
            return Some((label.label.as_str(), parent_same, false));
        };
        let inherited = self.tree.as_ref().and_then(|t| t.nodes[n].parent).and_then(|p| r.owners.get(p).copied().flatten()) == Some(o);
        let can_uninstall = r.uninstall.get(o as usize).is_some_and(|u| u.is_some());
        Some((r.apps.get(o as usize)?.as_str(), inherited, can_uninstall))
    }

    pub(crate) fn advisor_command(&mut self, name: &str) {
        match name {
            "AdvLensOpp" => self.advisor.lens_mode = Lens::Opportunities,
            "AdvLensRisk" => self.advisor.lens_mode = Lens::Risk,
            "AdvLensOff" => self.advisor.lens_mode = Lens::Off,
            "AdvRefresh" => return self.advisor_start(),
            "AdvHeld" => self.advisor.show_held = !self.advisor.show_held,
            "AdvRejected" => self.advisor.show_rejected = !self.advisor.show_rejected,
            "AdvKeptShow" => self.advisor.show_kept = !self.advisor.show_kept,
            "AdvUninstall" => {
                let Some(owner) = self.advisor.open.and_then(|i| self.advice()?.findings.get(i)?.owner.clone()) else { return };
                let Some(how) = self.advice().and_then(|r| r.apps.iter().position(|a| *a == owner).and_then(|k| r.uninstall.get(k).cloned().flatten())) else { return };
                use std::os::windows::process::CommandExt;
                let _ = std::process::Command::new("explorer.exe").raw_arg(how.address()).spawn();
                let msg = match how {
                    sprawl_advisor::facts::Uninstall::Settings => {
                        win32::copy_text(self.hwnd, &owner);
                        format!("Installed apps is open: search for {owner} (the name is copied).")
                    }
                    sprawl_advisor::facts::Uninstall::Steam(_) => format!("Steam asks before uninstalling {owner}."),
                };
                crate::state::text(&self.root, "StatusText", &msg);
                return;
            }
            "AdvChatSend" => return self.chat_send(),
            "AdvMcpCopy" => {
                win32::copy_text(self.hwnd, &crate::bridge::setup_command());
                crate::state::text(&self.root, "StatusText", "Copied. Run it once in a terminal, then ask Claude Code about your disk while Sprawl is open.");
                return;
            }
            "AdvEx0" | "AdvEx1" | "AdvEx2" | "AdvEx3" => {
                let i = name[5..].parse::<usize>().unwrap_or(0);
                return self.chat_ask(EXAMPLES.get(i).copied().unwrap_or_default().to_string());
            }
            "AdvChatClear" => {
                self.chat_reset();
            }
            "AdvWebOnce" | "AdvWebSession" | "AdvWebDeny" => {
                if let Some((_, reply)) = self.advisor.chat.consent.take() {
                    if name == "AdvWebSession" {
                        self.advisor.chat.web_ok.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    let _ = reply.send(name != "AdvWebDeny");
                }
            }
            "AdvMemShow" => self.advisor.show_memory = !self.advisor.show_memory,
            "AdvKeep" | "AdvLater" => return self.advisor_decide(if name == "AdvKeep" { Verdict::Keep } else { Verdict::Later }),
            "AdvPrefAdd" => {
                let text = self.advisor.panel.as_ref().and_then(|p| find::<TextBox>(p, "AdvPrefText").ok()).and_then(|t| t.Text().ok()).map(|h| h.to_string_lossy()).unwrap_or_default();
                if let Err(e) = self.advisor.memory.add_pref(&text, memory::From::You) {
                    crate::state::text(&self.root, "StatusText", &format!("Not added: {e}."));
                }
                self.save_memory();
            }
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
    pub(crate) fn advisor_open(&mut self, i: usize) {
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
        // keep what was typed in the goal and question boxes across the redraw
        if let Some(t) = self.advisor.panel.as_ref().and_then(|p| find::<TextBox>(p, "AdvGoal").ok()) {
            if let Ok(h) = t.Text() {
                self.advisor.goal_text = h.to_string_lossy();
            }
        }
        if let Some(t) = self.advisor.panel.as_ref().and_then(|p| find::<TextBox>(p, "AdvChatText").ok()) {
            if let Ok(h) = t.Text() {
                self.advisor.chat.text = h.to_string_lossy();
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
        if let Ok(t) = find::<TextBox>(&fe, "AdvChatText") {
            let _ = t.SetText(&self.advisor.chat.text.as_str().into());
        }
        // Enter in the goal box makes the plan
        if let Ok(el) = find::<UIElement>(&fe, "AdvGoal") {
            if let Ok(t) = el.KeyDown(|_, e| {
                let Some(e) = e.as_ref() else { return };
                if e.Key().is_ok_and(|k| k.0 == 13) {
                    let _ = e.SetHandled(true);
                    with(|s| s.advisor_command("AdvPlan"));
                }
            }) {
                keep(t);
            }
        }
        // Enter in the question box asks
        if let Ok(el) = find::<UIElement>(&fe, "AdvChatText") {
            if let Ok(t) = el.KeyDown(|_, e| {
                let Some(e) = e.as_ref() else { return };
                if e.Key().is_ok_and(|k| k.0 == 13) {
                    let _ = e.SetHandled(true);
                    with(|s| s.advisor_command("AdvChatSend"));
                }
            }) {
                keep(t);
            }
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
        let m = &self.advisor.memory;
        let counts = [
            ("AdvUndo", self.advice().map_or(0, |r| r.kept.len())),
            ("AdvApprove", m.proposals.len()),
            ("AdvDismiss", m.proposals.len()),
            ("AdvForgetP", m.prefs.len()),
            ("AdvForgetL", m.labels.len()),
            ("AdvGrow", self.advice().and_then(|r| r.growth.as_ref()).map_or(0, |g| g.grew.len())),
        ];
        for (kind, n) in counts {
            for i in 0..n {
                if let Ok(b) = find::<ButtonBase>(&fe, &format!("{kind}{i}")) {
                    if let Ok(t) = b.Click(move |_, _| {
                        with(|s| s.advisor_indexed(kind, i));
                    }) {
                        keep(t);
                    }
                }
            }
        }
        for name in [
            "AdvKeep", "AdvLater", "AdvUninstall", "AdvKeptShow", "AdvMemShow", "AdvPrefAdd", "AdvChatSend", "AdvMcpCopy", "AdvChatClear", "AdvEx0", "AdvEx1", "AdvEx2", "AdvEx3", "AdvWebOnce", "AdvWebSession", "AdvWebDeny",
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
            if self.tree.is_some() {
                x += &self.chat_card();
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
        x += &self.growth_card(r);
        x += &self.ai_card(r);
        x += &self.chat_card();
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
        x += &self.memory_cards(r);
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
        let t = self.advisor.goal_text.trim().trim_end_matches(['G', 'g', 'B', 'b', ' ']).replace(' ', "");
        // "1,000" is a thousand; "2,5" is two and a half
        let t = match t.split_once(',') {
            Some((_, after)) if t.contains('.') || (after.len() == 3 && !after.contains(',')) => t.replace(',', ""),
            _ => t.replace(',', "."),
        };
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
        // an empty box means the example it shows
        if self.advisor.goal_text.trim().is_empty() {
            self.advisor_set_goal(GOAL_EXAMPLE);
        }
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
        let notes = r.growth.as_ref().zip(self.tree.as_ref()).map(|(g, t)| g.text(t)).unwrap_or_default();
        let goal = self.goal_bytes();
        let gen = self.tree_gen;
        let memory = self.advisor.memory.clone();
        self.advisor.ai = Ai::Running("Starting...".into());
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let mut progress = move |line: String| {
                let _ = progress_tx.send(Msg::AiProgress(gen, line));
            };
            // a panic in there still ends the run
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sprawl_advisor::llm::agent::run(&t, &cfg, &already, hidden.as_ref(), goal, &memory, &notes, &mut progress)))
                .unwrap_or_else(|_| Err("the analysis stopped with an internal error".into()));
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
        // indexes past the rules' findings change: what pointed at them goes
        self.advisor.plan = None;
        self.advisor.preview = false;
        self.advisor.open = None;
        self.advisor.in_use = None;
        let Some(r) = self.advisor.report.as_mut().filter(|r| r.tree == gen) else { return Ai::Idle };
        let lens = Arc::make_mut(&mut r.lens);
        for f in run.findings {
            let n = sprawl_advisor::find_path(&t, &f.path);
            r.counted.push(n.is_none_or(|n| lens.get(n).copied().unwrap_or(0) == 0) && !inside_another(&r.findings, &f));
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
        // what the model asked to remember, and its proposed rules (checked again here)
        let m = &mut self.advisor.memory;
        for p in &run.prefs {
            let _ = m.add_pref(&p.text, memory::From::Model);
        }
        for l in &run.labels {
            let _ = m.add_label(&l.pattern, &l.label, memory::From::Model);
        }
        for p in run.proposals {
            let _ = m.propose(p);
        }
        self.save_memory();
        Ai::Done { summary: run.summary, model: run.model, found, rejected: run.rejected, with_tools: run.with_tools, turns: run.turns }
    }

    fn save_memory(&self) {
        crate::bridge::publish_memory(&self.advisor.memory);
        if let Err(e) = self.advisor.memory.save() {
            crate::state::text(&self.root, "StatusText", &format!("Couldn't save what the advisor remembers: {e}"));
        }
    }

    /// "Keep it" / "Not now" on the open finding: remembered, and taken out of the list.
    fn advisor_decide(&mut self, verdict: Verdict) {
        let gen = self.tree_gen;
        let Some(i) = self.advisor.open else { return };
        let (Some(t), Some(r)) = (self.tree.clone(), self.advisor.report.as_mut().filter(|r| r.tree == gen)) else { return };
        if i >= r.findings.len() {
            return;
        }
        let f = r.findings.remove(i);
        r.nodes.remove(i);
        r.counted.remove(i);
        self.advisor.memory.decide(&f.path, &f.title, verdict);
        r.kept.push((f, verdict));
        // the lens without it
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
        // indexes moved: the open finding and any plan go
        self.advisor.open = None;
        self.advisor.in_use = None;
        self.advisor.plan = None;
        self.advisor.preview = false;
        self.save_memory();
        self.advisor_changed();
    }

    /// A numbered button: undo a decision, approve or dismiss a proposal, forget a memory.
    fn advisor_indexed(&mut self, kind: &str, i: usize) {
        if kind == "AdvGrow" {
            let n = self.advice().and_then(|r| r.growth.as_ref()).and_then(|g| g.grew.get(i)).map(|g| g.node);
            if let Some(n) = n.filter(|&n| self.tree.as_ref().is_some_and(|t| n < t.nodes.len())) {
                self.set_view(View::Tree);
                self.reveal(n);
            }
            return;
        }
        let m = &mut self.advisor.memory;
        match kind {
            "AdvUndo" => {
                let Some(path) = self.advice().and_then(|r| r.kept.get(i)).map(|(f, _)| f.path.clone()) else { return };
                self.advisor.memory.undo(&path);
                self.save_memory();
                // found again from scratch, so it comes back where it belongs
                return self.advisor_start();
            }
            "AdvApprove" => {
                match m.approve(i) {
                    Ok(()) => {
                        self.save_memory();
                        crate::state::text(&self.root, "StatusText", "Rule added to your rules. Checking again with it...");
                        return self.advisor_start();
                    }
                    Err(e) => crate::state::text(&self.root, "StatusText", &format!("Couldn't add the rule: {e}")),
                }
            }
            "AdvDismiss" if i < m.proposals.len() => {
                m.proposals.remove(i);
            }
            "AdvForgetP" if i < m.prefs.len() => {
                m.prefs.remove(i);
            }
            "AdvForgetL" if i < m.labels.len() => {
                m.labels.remove(i);
            }
            _ => return,
        }
        self.save_memory();
        self.advisor_changed();
    }

    /// Write the plan (or every finding with something to run) as a PowerShell script.
    fn advisor_export(&mut self) {
        let Some(r) = self.advice() else { return };
        // a plan, or everything that comes back or costs little: orange is the user's call, one by one
        let picked: Vec<usize> = match self.current_plan() {
            Some(p) => p.plan.picked.iter().copied().filter(|&i| i < r.findings.len()).collect(),
            None => (0..r.findings.len()).filter(|&i| r.counted[i] && r.findings[i].command.is_some() && r.findings[i].risk <= Risk::Amber).collect(),
        };
        if picked.is_empty() {
            crate::state::text(&self.root, "StatusText", "Nothing to export: no finding has a command to run.");
            return;
        }
        let list: Vec<&Finding> = picked.iter().map(|&i| &r.findings[i]).collect();
        let root = self.tree.as_ref().map(|t| t.path_of(sprawl_core::ROOT).display().to_string()).unwrap_or_default();
        let text = sprawl_advisor::export::script(&list, &root, std::time::SystemTime::now());
        let items: Vec<(PathBuf, u64, String)> = list.iter().filter(|f| f.command.is_some()).map(|f| (f.path.clone(), f.bytes, f.title.clone())).collect();
        // never over a script the user may just have read
        let dir = crate::format::user_dir("Documents");
        let base = format!("Sprawl clean-up {}", crate::format::stamp());
        let file = (1..100).map(|k| dir.join(if k == 1 { format!("{base}.ps1") } else { format!("{base} ({k}).ps1") })).find(|p| !p.exists()).unwrap_or_else(|| dir.join(format!("{base}.ps1")));
        match sprawl_advisor::export::write_script(&file, &text) {
            Ok(()) => {
                use std::os::windows::process::CommandExt;
                // quoted for Explorer itself: a comma in the path would split it
                let _ = std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", file.display())).spawn();
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
        let file = e.file.clone();
        std::thread::spawn(move || {
            let out = items
                .into_iter()
                .map(|(p, before, _)| {
                    let now = if std::fs::symlink_metadata(&p).is_err() {
                        0
                    } else {
                        sprawl_core::scan(&p, |_| {}).map(|t| t.nodes[sprawl_core::ROOT].size).unwrap_or(before)
                    };
                    (p, before, now)
                })
                .collect();
            let _ = tx.send(Msg::Verified(file, out));
        });
    }

    /// A new scan: the chat's copy of the old one is no use any more.
    pub(crate) fn chat_reset(&mut self) {
        let web_ok = self.advisor.chat.web_ok.clone();
        web_ok.store(false, std::sync::atomic::Ordering::Relaxed);
        // dropping the senders ends the thread; a pending consent answers no
        self.advisor.chat = Chat { web_ok, ..Chat::default() };
    }

    /// A sample exchange with Markdown in the answer (screenshot runs).
    pub(crate) fn advisor_chat_demo(&mut self) {
        self.advisor.chat.log.push((true, "what is the biggest thing?".into()));
        self.advisor.chat.log.push((
            false,
            "The biggest thing here is **SOLIDWORKS Corp**, at **7.9 GB**.\n\nNext come:\n- `Microsoft Visual Studio` (7.2 GB)\n- *Autodesk* (6.9 GB)\n\n```\nC:\\Program Files\\SOLIDWORKS Corp\n```".into(),
        ));
        self.advisor_changed();
    }

    /// Type a question and send it (screenshot runs).
    pub(crate) fn advisor_chat_dev(&mut self, q: &str) {
        if let Some(t) = self.advisor.panel.as_ref().and_then(|p| find::<TextBox>(p, "AdvChatText").ok()) {
            let _ = t.SetText(&q.into());
        }
        self.chat_send();
    }

    fn chat_send(&mut self) {
        let q = self.advisor.panel.as_ref().and_then(|p| find::<TextBox>(p, "AdvChatText").ok()).and_then(|t| t.Text().ok()).map(|h| h.to_string_lossy()).unwrap_or_default();
        self.chat_ask(q);
    }

    /// Send a question. While one is being answered the next waits its turn.
    fn chat_ask(&mut self, q: String) {
        let q = q.trim().to_string();
        if q.is_empty() {
            return;
        }
        // empty the box now, or the redraw would read the old question back into it
        if let Some(t) = self.advisor.panel.as_ref().and_then(|p| find::<TextBox>(p, "AdvChatText").ok()) {
            let _ = t.SetText(&"".into());
        }
        if self.advisor.chat.tx.is_none() {
            // the whole scan, also while a filter is on
            let Some(t) = self.full_tree.clone().or_else(|| self.tree.clone()) else { return };
            let cfg = AiConfig::load();
            let memory = self.advisor.memory.clone();
            let notes = self.advice().and_then(|r| r.growth.as_ref()).zip(self.tree.as_ref()).map(|(g, t)| g.text(t)).unwrap_or_default();
            let (qtx, qrx) = channel::<String>();
            let (etx, erx) = channel::<ChatMsg>();
            let web_ok = self.advisor.chat.web_ok.clone();
            std::thread::spawn(move || {
                let consent_tx = etx.clone();
                let consent = move |query: &str| -> bool {
                    if web_ok.load(std::sync::atomic::Ordering::Relaxed) {
                        return true;
                    }
                    let (rtx, rrx) = channel();
                    if consent_tx.send(ChatMsg::Consent(query.to_string(), rtx)).is_err() {
                        return false;
                    }
                    // no answer in ten minutes, or the chat ended: no
                    rrx.recv_timeout(std::time::Duration::from_secs(600)).unwrap_or(false)
                };
                let mut events = |e: ChatEvent| {
                    let _ = etx.send(ChatMsg::Event(e));
                };
                let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sprawl_advisor::llm::agent::chat(&t, &cfg, &memory, &notes, qrx, &mut events, &consent)));
                if ended.is_err() {
                    let _ = etx.send(ChatMsg::Event(ChatEvent::Failed("the chat stopped with an internal error".into())));
                }
            });
            self.advisor.chat.tx = Some(qtx);
            self.advisor.chat.rx = Some(erx);
        }
        if let Some(tx) = &self.advisor.chat.tx {
            if tx.send(q.clone()).is_ok() {
                self.advisor.chat.log.push((true, q));
                self.advisor.chat.status = Some("Thinking...".into());
                self.advisor.chat.text.clear();
            }
        }
        self.advisor_changed();
    }

    fn chat_poll(&mut self) {
        let Some(rx) = &self.advisor.chat.rx else { return };
        let msgs: Vec<ChatMsg> = rx.try_iter().collect();
        if msgs.is_empty() {
            return;
        }
        let mut redraw = false;
        for m in msgs {
            match m {
                ChatMsg::Event(ChatEvent::Progress(s)) => {
                    if let Some(p) = &self.advisor.panel {
                        crate::state::text(p, "AdvChatStatus", &s);
                    }
                    self.advisor.chat.status = Some(s);
                }
                ChatMsg::Event(ChatEvent::Reply(s)) => {
                    self.advisor.chat.log.push((false, s));
                    self.advisor.chat.status = None;
                    redraw = true;
                }
                ChatMsg::Event(ChatEvent::Failed(e)) => {
                    self.advisor.chat.log.push((false, format!("(failed: {e})")));
                    self.advisor.chat.status = None;
                    redraw = true;
                }
                ChatMsg::Event(ChatEvent::Filter(f)) => {
                    // through the filter box, as if typed: it applies a moment later
                    if let Ok(t) = find::<TextBox>(&self.root, "FilterBox") {
                        let _ = t.SetText(&f.as_str().into());
                    }
                }
                ChatMsg::Event(ChatEvent::Remember { prefs, labels }) => {
                    for p in &prefs {
                        let _ = self.advisor.memory.add_pref(&p.text, memory::From::Model);
                    }
                    for l in &labels {
                        let _ = self.advisor.memory.add_label(&l.pattern, &l.label, memory::From::Model);
                    }
                    self.save_memory();
                }
                ChatMsg::Consent(q, reply) => {
                    self.advisor.chat.consent = Some((q, reply));
                    redraw = true;
                }
            }
        }
        if redraw && self.view == View::Advisor {
            self.render_advisor();
        }
    }

    fn chat_card(&self) -> String {
        let c = &self.advisor.chat;
        let who = match AiConfig::load().provider {
            Provider::ClaudeCode => "Claude Code",
            Provider::Anthropic => "Claude",
            _ => "AI",
        };
        let mut x = String::from(r##"<Border Padding="12,10" CornerRadius="7" Background="#0CFFFFFF"><StackPanel Spacing="6"><TextBlock Text="Ask your disk" FontSize="13" FontWeight="SemiBold"/>"##);
        for (user, text) in c.log.iter().rev().take(12).rev() {
            if *user {
                x += &format!(
                    r##"<TextBlock TextWrapping="Wrap" FontSize="12" IsTextSelectionEnabled="True" Margin="0,4,0,0"><Run Text="You: " FontWeight="SemiBold" Foreground="#FF8FB4FF"/><Run Text="{}" Foreground="#FFD5DBE8"/></TextBlock>"##,
                    ui::esc(text)
                );
            } else {
                x += &format!(r##"<TextBlock Text="{who}" FontSize="12" FontWeight="SemiBold" Foreground="#FFB9C2D6"/>"##);
                x += &ui::markdown(text, 12, "#FFD5DBE8");
            }
        }
        if c.log.is_empty() {
            // ready-made questions: one click asks
            x += r##"<StackPanel Orientation="Horizontal" Spacing="6">"##;
            for (i, q) in EXAMPLES.iter().enumerate() {
                x += &format!(r##"<Button x:Name="AdvEx{i}" Content="{}" Padding="10,3" FontSize="11" CornerRadius="12" Background="#142F7BF6"/>"##, ui::esc(q));
            }
            x += "</StackPanel>";
        }
        if let Some((q, _)) = &c.consent {
            x += &format!(
                r##"<Border Padding="10,8" CornerRadius="6" Background="#22F5B342"><StackPanel Spacing="6"><TextBlock TextWrapping="Wrap" FontSize="12"><Run Text="The AI wants to search the web for: "/><Run Text="{}" FontWeight="SemiBold"/></TextBlock><TextBlock Text="The search goes to DuckDuckGo. Nothing else is sent." FontSize="11" Foreground="#FF8E97AA"/><StackPanel Orientation="Horizontal" Spacing="6"><Button x:Name="AdvWebOnce" Content="Allow once" Padding="10,3" FontSize="12" CornerRadius="5"/><Button x:Name="AdvWebSession" Content="Allow for this session" Padding="10,3" FontSize="12" CornerRadius="5"/><Button x:Name="AdvWebDeny" Content="Don't search" Padding="10,3" FontSize="12" CornerRadius="5"/></StackPanel></StackPanel></Border>"##,
                ui::esc(q)
            );
        }
        if let Some(s) = &c.status {
            x += &format!(r##"<TextBlock x:Name="AdvChatStatus" Text="{}" TextWrapping="Wrap" FontSize="11" Foreground="#FF8E97AA"/>"##, ui::esc(s));
        }
        x += r##"<StackPanel Orientation="Horizontal" Spacing="6"><TextBox x:Name="AdvChatText" Width="430" PlaceholderText="Ask about this scan, then press Enter"/><Button x:Name="AdvChatSend" Content="Ask" Padding="12,3" FontSize="12" CornerRadius="5"/><Button x:Name="AdvChatClear" Content="New chat" Padding="10,3" FontSize="12" CornerRadius="5" BorderThickness="0" Background="Transparent"/></StackPanel>"##;
        x += "</StackPanel></Border>";
        x
    }

    fn memory_cards(&self, r: &Report) -> String {
        let m = &self.advisor.memory;
        let small = |name: String, label: &str| format!(r##"<Button x:Name="{name}" Content="{label}" Padding="8,1" FontSize="11" CornerRadius="4"/>"##);
        let mut x = String::new();
        if !r.kept.is_empty() {
            x += &format!(
                r##"<Button x:Name="AdvKeptShow" Padding="10,4" FontSize="12" CornerRadius="5" BorderThickness="0" Background="Transparent" Content="{} hidden by your choices ({})"/>"##,
                r.kept.len(),
                if self.advisor.show_kept { "hide" } else { "show" }
            );
            if self.advisor.show_kept {
                for (i, (f, v)) in r.kept.iter().enumerate() {
                    let why = match v {
                        Verdict::Keep => "kept",
                        Verdict::Later => "not now",
                        Verdict::Done => "done",
                    };
                    x += &format!(
                        r##"<StackPanel Orientation="Horizontal" Spacing="8" Margin="10,0,0,0"><TextBlock FontSize="12" Foreground="#FFB9C2D6" VerticalAlignment="Center" Text="{} ({}, {why})"/>{}</StackPanel>"##,
                        ui::esc(&f.title),
                        self.size(f.bytes),
                        small(format!("AdvUndo{i}"), "Undo")
                    );
                }
            }
        }
        if !m.proposals.is_empty() {
            x += r##"<Border Padding="12,10" CornerRadius="7" Background="#0CFFFFFF"><StackPanel Spacing="6"><TextBlock Text="Rules the AI proposes" FontSize="13" FontWeight="SemiBold"/><TextBlock Text="Approved rules join your own rule pack: they find these folders by themselves next time. They never run commands, and their deletes are at least amber." TextWrapping="Wrap" FontSize="11" Foreground="#FF8E97AA"/>"##;
            for (i, p) in m.proposals.iter().enumerate() {
                x += &format!(
                    r##"<StackPanel Spacing="2" Margin="0,4,0,0"><StackPanel Orientation="Horizontal" Spacing="8"><Ellipse Width="8" Height="8" Fill="{}" VerticalAlignment="Center"/><TextBlock Text="{}" FontSize="12" FontWeight="SemiBold" VerticalAlignment="Center"/><TextBlock Text="{}" FontSize="11" Foreground="#FF8E97AA" VerticalAlignment="Center"/>{}{}</StackPanel><TextBlock Text="{}" TextWrapping="Wrap" FontSize="11" Foreground="#FFB9C2D6"/><TextBlock Text="{}" TextWrapping="Wrap" FontSize="11" FontFamily="Consolas" Foreground="#FF8E97AA"/></StackPanel>"##,
                    risk_hex(p.risk),
                    ui::esc(&p.title),
                    remedy_label(p.remedy),
                    small(format!("AdvApprove{i}"), "Approve"),
                    small(format!("AdvDismiss{i}"), "Dismiss"),
                    ui::esc(&p.why),
                    ui::esc(&p.paths.join("   ")),
                );
            }
            x += "</StackPanel></Border>";
        }
        x += &format!(
            r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="Use Sprawl from your own Claude Code:" FontSize="11" Foreground="#FF8E97AA" VerticalAlignment="Center"/><TextBlock Text="{}" FontFamily="Consolas" FontSize="11" Foreground="#FFB9C2D6" VerticalAlignment="Center" IsTextSelectionEnabled="True" TextTrimming="CharacterEllipsis" MaxWidth="460"/><Button x:Name="AdvMcpCopy" Content="Copy" Padding="8,1" FontSize="11" CornerRadius="4"/></StackPanel>"##,
            ui::esc(&crate::bridge::setup_command())
        );
        x += &format!(
            r##"<Button x:Name="AdvMemShow" Padding="10,4" FontSize="12" CornerRadius="5" BorderThickness="0" Background="Transparent" Content="What the advisor remembers: {} preferences, {} folder labels ({})"/>"##,
            m.prefs.len(),
            m.labels.len(),
            if self.advisor.show_memory { "hide" } else { "show" }
        );
        if self.advisor.show_memory {
            x += r##"<StackPanel Spacing="4" Margin="10,0,0,0">"##;
            x += r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBox x:Name="AdvPrefText" Width="360" PlaceholderText="A preference, like: I still play my Steam games"/><Button x:Name="AdvPrefAdd" Content="Remember" Padding="10,3" FontSize="12" CornerRadius="5"/></StackPanel>"##;
            for (i, p) in m.prefs.iter().enumerate() {
                let who = if p.from == memory::From::Model { " (from the AI)" } else { "" };
                x += &format!(r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock FontSize="12" Foreground="#FFB9C2D6" VerticalAlignment="Center" Text="{}{who}"/>{}</StackPanel>"##, ui::esc(&p.text), small(format!("AdvForgetP{i}"), "Forget"));
            }
            for (i, l) in m.labels.iter().enumerate() {
                x += &format!(
                    r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock FontSize="12" Foreground="#FFB9C2D6" VerticalAlignment="Center" Text="{}: {}"/>{}</StackPanel>"##,
                    ui::esc(&l.pattern),
                    ui::esc(&l.label),
                    small(format!("AdvForgetL{i}"), "Forget")
                );
            }
            x += "</StackPanel>";
        }
        x
    }

    fn growth_card(&self, r: &Report) -> String {
        use sprawl_advisor::growth::signed;
        let not_yet = r##"<TextBlock Text="Growth over time shows here once History has scans of this folder at least a day apart." FontSize="11" Foreground="#FF6F788C" TextWrapping="Wrap"/>"##;
        let Some(t) = self.tree.as_ref() else { return String::new() };
        let Some(g) = &r.growth else { return not_yet.to_string() };
        let mut x = String::from(r##"<Border Padding="12,10" CornerRadius="7" Background="#0CFFFFFF"><StackPanel Spacing="4">"##);
        let mut head = String::new();
        if let Some(since) = g.since {
            head += &format!("{} since {}", signed(g.change), sprawl_advisor::facts::month_year(since));
        }
        if let Some(rate) = g.per_day {
            head += &format!("{}about {} a day", if head.is_empty() { "" } else { ", " }, signed(rate as i64));
        }
        if head.is_empty() {
            return not_yet.to_string();
        }
        x += &format!(r##"<TextBlock Text="Growth: {}" FontSize="13" FontWeight="SemiBold"/>"##, ui::esc(&head));
        if let Some(d) = g.full_in_days {
            let (colour, text) = if d < 30.0 {
                ("#FFFF7A45", format!("At this rate the drive is full in about {d:.0} days."))
            } else if d < 365.0 {
                ("#FFF5B342", format!("At this rate the drive is full in about {:.0} weeks.", d / 7.0))
            } else {
                ("#FF8E97AA", "At this rate the drive has room for over a year.".to_string())
            };
            x += &format!(r##"<TextBlock Text="{text}" FontSize="12" Foreground="{colour}"/>"##);
        }
        for (i, gr) in g.grew.iter().enumerate() {
            if gr.node >= t.nodes.len() {
                continue;
            }
            x += &format!(
                r##"<Grid ColumnSpacing="8" Margin="0,2,0,0"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><StackPanel><TextBlock Text="{}" FontSize="12" TextTrimming="CharacterEllipsis"/><TextBlock Text="{}" FontSize="11" Foreground="#FF8E97AA" TextWrapping="Wrap"/></StackPanel><TextBlock Grid.Column="1" Text="{}" FontSize="12" VerticalAlignment="Center" Foreground="#FFF5B342"/><Button Grid.Column="2" x:Name="AdvGrow{i}" Content="Show" Padding="8,1" FontSize="11" CornerRadius="4" VerticalAlignment="Center"/></Grid>"##,
                ui::esc(&t.path_of(gr.node).display().to_string()),
                ui::esc(&gr.why.join("; ")),
                ui::esc(&signed(gr.delta)),
            );
        }
        x += "</StackPanel></Border>";
        x
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
            r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="I need" VerticalAlignment="Center" FontSize="13"/><TextBox x:Name="AdvGoal" Width="70" PlaceholderText="{GOAL_EXAMPLE}"/><TextBlock Text="GB" VerticalAlignment="Center" FontSize="13"/>{}{}{}</StackPanel>"##,
            btn("AdvPlan", "Make a plan"),
            ask,
            btn("AdvExport", if self.current_plan().is_some() { "Export plan as script" } else { "Export script" }),
        );
        if let Some(p) = self.current_plan() {
            let n = p.plan.picked.len();
            let worst = p.plan.picked.iter().filter_map(|&i| r.findings.get(i)).map(|f| f.risk).max();
            let msg = if n == 0 {
                "No finding has something to run, so there is no plan yet.".to_string()
            } else if p.plan.enough() {
                let items = if n == 1 { "1 item frees".to_string() } else { format!("{n} items free") };
                format!("{items} {} of the {} you need, at worst {}.", self.size(p.plan.bytes), self.size(p.plan.need), worst.map(risk_words).unwrap_or(""))
            } else {
                format!("Green and amber findings free {}, short of the {} you need. Orange ones are your own files: decide those one by one, or ask the AI to look further.", self.size(p.plan.bytes), self.size(p.plan.need))
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
                    x += &ui::markdown(summary, 13, "#FFD5DBE8");
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
            let planned: u64 = e.items.iter().map(|(_, b, _)| *b).sum();
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
        let how = f.owner.as_ref().and_then(|o| r.apps.iter().position(|a| a == o)).and_then(|k| r.uninstall.get(k).cloned().flatten());
        if let (Some(how), Some(o)) = (how, &f.owner) {
            let tip = match how {
                sprawl_advisor::facts::Uninstall::Settings => "Opens Windows' Installed apps",
                sprawl_advisor::facts::Uninstall::Steam(_) => "Opens Steam's uninstall, which asks first",
            };
            x += &format!(r##"<Button x:Name="AdvUninstall" Content="Uninstall {}..." ToolTipService.ToolTip="{tip}" Padding="10,3" FontSize="12" CornerRadius="5"/>"##, ui::esc(o));
        }
        x += r##"<Button x:Name="AdvKeep" Content="Keep it" ToolTipService.ToolTip="Never suggest this again" Padding="10,3" FontSize="12" CornerRadius="5"/><Button x:Name="AdvLater" Content="Not now" ToolTipService.ToolTip="Hide it for 30 days" Padding="10,3" FontSize="12" CornerRadius="5"/>"##;
        x += "</StackPanel></StackPanel></Border>";
        x
    }
}
