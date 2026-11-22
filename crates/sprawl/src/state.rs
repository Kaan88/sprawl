//! The window's state: what was scanned, what is shown, and the helpers every part uses.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use sprawl_core::{Progress, Tree, ROOT};
use windows_core::{Interface, HSTRING};

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::RangeBase;
use crate::bindings::Microsoft::UI::Xaml::Controls::{ProgressBar, TextBlock};
use crate::bindings::Microsoft::UI::Xaml::Markup::XamlReader;
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, Window};
use crate::format::thousands;
use crate::hud::{Hud, Live};
use crate::settings::Settings;
use crate::treemap::{LiveMap, MapView};
use crate::{paint, win32};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Units {
    Dynamic,
    Bytes,
    KB,
    MB,
    GB,
    TB,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum View {
    Tree,
    Files,
    Search,
    Advisor,
    About,
}

pub(crate) struct ExtRow {
    pub(crate) ext: String,
    pub(crate) size: u64,
    pub(crate) alloc: u64,
    pub(crate) files: u64,
    pub(crate) percent: f64,
}

/// What to scan.
pub(crate) enum Source {
    Folders(Vec<PathBuf>),
    Csv(PathBuf),
}

/// The sample answer the `mddemo` screenshot step shows.
const MD_DEMO: &str = "## Biggest folders\nMost of it is in your profile:\nC:\\Users\\me\\\n├── AppData (40.2 GB)\n│   ├── Local\\npm-cache\\_cacache (6.1 GB)\n│   └── Roaming\n└── Videos (12.0 GB)\n\n| Folder | Size | Safe? |\n|---|---:|---|\n| `npm-cache` | 6.1 GB | yes, **rebuilt** on demand |\n| Videos | 12.0 GB | ~~no~~ your own files |\n\n- Caches\n  - npm: run `npm cache clean --force`\n- See [the npm docs](https://docs.npmjs.com/cli/cache)\n> Nothing is deleted until you review it.\n\nPath: C:\\Users\\me\\Videos\nSize: 12.0 GB\n\n---\n```\nnpm cache verify\n```";

/// Cancellation wins even when a worker finished successfully before the UI polled it.
fn scan_result<T>(result: std::io::Result<T>, cancelled: bool) -> std::io::Result<T> {
    if cancelled { Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "scan cancelled")) }
    else { result }
}

/// How a scan was read, as the header and the status line say it.
fn method_label(m: sprawl_core::ScanMethod) -> &'static str {
    match m {
        sprawl_core::ScanMethod::Mft => "read from the MFT",
        sprawl_core::ScanMethod::Csv => "loaded from CSV",
        sprawl_core::ScanMethod::Walk => "folder walk",
    }
}

/// The header's name for the volume a path is on: "Drive C:", "Share:" for a network path,
/// "Drives:" for a scan of several folders.
fn drive_label(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    let b = s.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        format!("Drive {}:", (b[0] as char).to_ascii_uppercase())
    } else if s.starts_with(r"\\") {
        "Share:".into()
    } else {
        "Drives:".into()
    }
}

#[cfg(test)]
mod scan_result_tests {
    use super::{drive_label, scan_result};

    #[test]
    fn the_header_names_the_drive_a_scan_is_on() {
        assert_eq!(drive_label(std::path::Path::new(r"c:\Users\x")), "Drive C:");
        assert_eq!(drive_label(std::path::Path::new(r"D:\")), "Drive D:");
        assert_eq!(drive_label(std::path::Path::new(r"\\server\share\x")), "Share:");
        assert_eq!(drive_label(std::path::Path::new(sprawl_core::MULTIPLE)), "Drives:");
    }

    #[test]
    fn cancellation_wins_over_a_completed_or_failed_worker() {
        assert_eq!(scan_result(Ok("new tree"), false).unwrap(), "new tree");
        assert_eq!(scan_result(Ok("new tree"), true).unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        let failure = || Err::<(), _>(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "unreadable source"));
        assert_eq!(scan_result(failure(), false).unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(scan_result(failure(), true).unwrap_err().kind(), std::io::ErrorKind::Interrupted);
    }
}

pub(crate) struct Job {
    pub(crate) path: PathBuf,
    pub(crate) csv: bool,
    pub(crate) progress: Arc<Progress>,
    pub(crate) result: Arc<Mutex<Option<std::io::Result<crate::tables::Prepared>>>>,
    pub(crate) started: Instant,
    pub(crate) last_layout: Instant,
    /// The folders scanned and the folders to skip in force, for its Details.
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) skips: Vec<PathBuf>,
}

/// A filter being made: its pattern, the full tree it filters and that tree's generation.
pub(crate) struct Filtering {
    pattern: String,
    base: Arc<Tree>,
    gen: u64,
    rx: std::sync::mpsc::Receiver<crate::tables::Prepared>,
}

impl Job {
    /// What the scanner has found so far.
    pub(crate) fn live(&self) -> Live {
        let s = self.progress.snapshot();
        Live { files: s.files, folders: s.folders, bytes: s.bytes, fraction: s.fraction, top: s.top, by_ext: s.by_ext, method: s.method.label(), phase: s.phase }
    }
}

pub(crate) const ROW_H: f64 = 24.0;

pub(crate) struct State {
    /// Sprawl's own question boxes waiting (the first is on screen).
    pub(crate) dialogs: crate::ask::Dialogs,
    pub(crate) window: Window,
    pub(crate) hwnd: isize,
    pub(crate) root: FrameworkElement,
    pub(crate) drives: Vec<String>,
    /// Shared so the advisor can read it on another thread; changed through `Arc::make_mut`.
    pub(crate) tree: Option<Arc<Tree>>,
    pub(crate) alloc: Vec<u64>,
    /// Name and folder places for the tree on screen (File View sorts by them).
    pub(crate) ranks: std::sync::Arc<crate::tables::Ranks>,
    /// The places being made on a worker, for the tree set as number `.0`.
    pub(crate) ranks_job: Option<(u64, std::sync::mpsc::Receiver<crate::tables::Ranks>)>,
    /// Counts trees put on screen (set_tree), not their edits: places made for one tree are
    /// never kept for another.
    pub(crate) tree_set: u64,
    pub(crate) expanded: HashSet<usize>,
    pub(crate) selected: Option<usize>,
    /// More rows picked with Ctrl or Shift, besides `selected` (Delete takes them all).
    pub(crate) marked: std::collections::BTreeSet<usize>,
    pub(crate) rows: Vec<(usize, usize)>, // (node, depth)
    pub(crate) files: Vec<usize>,
    pub(crate) exts: Vec<ExtRow>,
    pub(crate) colours: HashMap<String, usize>,
    pub(crate) ext_selected: Option<String>,
    pub(crate) tables: crate::tables::Tables,
    /// The unfiltered scan while a filter is on.
    pub(crate) full_tree: Option<Arc<Tree>>,
    /// What was worked out for `full_tree`, kept up to date with it while a filter is on.
    pub(crate) full_parts: Option<crate::tables::Parts>,
    pub(crate) advisor: crate::advisor::Advisor,
    /// Changes whenever the tree on screen is replaced or loses nodes, so work done for an
    /// earlier tree (the advisor's report) is never applied to this one. Sorting doesn't change
    /// node numbers and keeps it.
    pub(crate) tree_gen: u64,
    /// Changes with every finished scan (not with deletes or filters): growth is worked out
    /// once per scan.
    pub(crate) scan_id: u64,
    pub(crate) filter: String,
    pub(crate) filter_text: String,
    pub(crate) filter_pending: Option<Instant>,
    /// The filter being made on a worker.
    pub(crate) filtering: Option<Filtering>,
    /// Selections left by Select Parent, for Reselect Child.
    pub(crate) child_stack: Vec<usize>,
    pub(crate) wizard_page: usize,
    /// The wizard shown as the Settings page: every section at once.
    pub(crate) wizard_all: bool,
    pub(crate) wizard_offered: bool,
    pub(crate) update: crate::update::Updater,
    /// The Model dropdown on the setup / Settings page.
    pub(crate) wiz_models: crate::models::ModelFetch,
    /// The wizard and Settings while open.
    pub(crate) prefs: crate::wizard::PrefUi,
    /// The Search tab.
    pub(crate) search: crate::search::Search,
    pub(crate) hist: Option<crate::history::Hist>,
    /// When the current tree's scan finished (its snapshot carries this time).
    pub(crate) scan_taken: std::time::SystemTime,
    /// When the current tree's scan began (for a CSV, when the file was written, if earlier): a
    /// clean-up leaves anything created or changed after it.
    pub(crate) scan_began: std::time::SystemTime,
    pub(crate) settings: Settings,
    /// Live change pulses: the watcher on the scanned folder and how many changes it saw.
    pub(crate) watcher: Option<crate::watch::Watcher>,
    pub(crate) changes: u64,
    /// What Sprawl itself just deleted (lower case), and when: not counted as live changes.
    pub(crate) own_deletes: Vec<(String, std::time::Instant)>,
    /// Folders read again after something ran from the Windows menu.
    /// What the user allowed this session while elevated (crate::admin), by setting key.
    /// The window's height (0 until it is laid out), for the map's share on Search.
    pub(crate) win_h: f64,
    /// The window's width (0 until it is laid out), for the Settings card to fit.
    pub(crate) win_w: f64,
    pub(crate) shell_rescans: Option<std::sync::mpsc::Receiver<(std::path::PathBuf, Option<sprawl_core::Tree>)>>,
    pub(crate) pulses_on: bool,
    pub(crate) map_root: usize,
    pub(crate) map: MapView,
    pub(crate) hud: Hud,
    pub(crate) units: Units,
    pub(crate) view: View,
    pub(crate) job: Option<Job>,
    /// Latest requested source while the current scanner is stopping.
    pub(crate) pending_scan: Option<Source>,
    pub(crate) scan_secs: f64,
    /// What the scan on screen covered (its Details link).
    pub(crate) coverage: Option<crate::coverage::Coverage>,
    pub(crate) shot: Option<PathBuf>,
    pub(crate) shot_ticks: u32,
    /// Whether the welcome is shown (None before the first frame).
    pub(crate) welcome: Option<bool>,
    /// The drives and folders its cards scan.
    pub(crate) welcome_drives: Vec<String>,
    pub(crate) welcome_recent: Vec<PathBuf>,
}

thread_local! {
    pub(crate) static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Run `f` on the state unless it's already in use (an event raised while we change the UI).
pub(crate) fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| match s.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

/// Event subscriptions end when their handle is dropped; these live as long as the window.
pub(crate) fn keep<T>(handle: T) {
    std::mem::forget(handle);
}

pub(crate) fn find<T: Interface>(root: &FrameworkElement, name: &str) -> windows_core::Result<T> {
    root.FindName(&HSTRING::from(name))?.cast()
}

pub(crate) fn text(root: &FrameworkElement, name: &str, value: &str) {
    if let Ok(t) = find::<TextBlock>(root, name) {
        let _ = t.SetText(&HSTRING::from(value));
    }
}

thread_local! {
    /// dev: a screenshot run's window being dragged to a size: from, to, frames gone (of 20).
    static SHOT_DRAG: std::cell::Cell<Option<((i32, i32), (i32, i32), u32)>> = const { std::cell::Cell::new(None) };
}

/// dev: with `SPRAWL_TIMING` set, one line per call in `%TEMP%\sprawl-timing.log`: what took
/// how long since `since`. Off (one env lookup, cached) otherwise.
pub(crate) fn timing(what: &str, since: Instant) {
    sprawl_core::timing(what, since);
}

/// Load a piece of XAML: the page's own colours are turned around first, then the look's tokens
/// are put in, each already holding the value for the theme in use. Expanding here covers every
/// string the app loads, `reconcile`'s one-attribute loads included; `reconcile` compares the
/// strings as they are written and lets `set` resolve a token for a value it reads itself.
pub(crate) fn load<T: Interface>(xaml: &str) -> windows_core::Result<T> {
    let made = XamlReader::Load(&HSTRING::from(&*crate::popups::menus(&crate::tokens::expand(&crate::theme::themed(xaml)))))?;
    // the drop shadows it marks are hung on once it is laid out (shade.rs)
    if let Ok(el) = made.cast::<crate::bindings::Microsoft::UI::Xaml::UIElement>() {
        crate::shade::watch(&el);
    }
    made.cast()
}

impl State {
    pub(crate) fn size(&self, v: u64) -> String {
        size_with(self.units, v)
    }
}

/// `v` bytes in `units`.
pub(crate) fn size_with(units: Units, v: u64) -> String {
    {
        let (base, labels) = crate::format::unit_base();
        let f = |power: i32| format!("{:.1} {}", v as f64 / base.powi(power), labels[power as usize]);
        match units {
            Units::Dynamic => crate::format::size(v),
            Units::Bytes => thousands(v),
            Units::KB => f(1),
            Units::MB => f(2),
            Units::GB => f(3),
            Units::TB => f(4),
        }
    }
}

impl State {
    pub(crate) fn label(&self, node: usize) -> String {
        let t = self.tree.as_ref().unwrap();
        if node == ROOT {
            t.root_path.display().to_string()
        } else {
            t.nodes[node].name.clone()
        }
    }

    // ------------------------------------------------------------ scanning

    pub(crate) fn start_scan(&mut self, path: PathBuf) {
        self.watcher = None;
        self.start(Source::Folders(vec![path]));
    }

    pub(crate) fn start(&mut self, source: Source) {
        if let Some(job) = &self.job {
            self.pending_scan = Some(source);
            job.progress.cancel();
            text(&self.root, "StatusText", "Stopping the previous scan before switching source...");
            return;
        }
        // a new scan would replace the tree the clean-up is checking and rescanning
        if self.advisor.cleanup.running.is_some() || self.advisor.file_op() || self.update.applying {
            text(&self.root, "FootLeft", "A file operation is running: scan again when it has finished.");
            return;
        }
        // rereads after a Windows menu command were for the tree this scan replaces
        self.shell_rescans = None;
        let (path, csv) = match &source {
            Source::Folders(v) if v.len() == 1 => (v[0].clone(), false),
            Source::Folders(v) if v.is_empty() => return,
            Source::Folders(_) => (PathBuf::from(sprawl_core::MULTIPLE), false),
            Source::Csv(p) => (p.clone(), true),
        };
        // the same list the scan uses, kept for its Details
        let skips = sprawl_advisor::exclusions::configured();
        let progress = sprawl_core::Progress::with_exclusions(&skips);
        let roots = match &source {
            Source::Folders(v) => v.clone(),
            Source::Csv(_) => Vec::new(),
        };
        let result = Arc::new(Mutex::new(None));
        let (g2, r2) = (progress.clone(), result.clone());
        let sorts = self.sorts();
        std::thread::spawn(move || {
            let t = Instant::now();
            let tree = match &source {
                Source::Folders(v) if v.len() == 1 => sprawl_core::scan_auto(&v[0], &g2),
                Source::Folders(v) => sprawl_core::scan_many(v, &g2),
                Source::Csv(p) => sprawl_core::load_csv(p, &g2),
            };
            timing(&format!("scan: the scan itself ({} nodes)", tree.as_ref().map_or(0, |t| t.nodes.len())), t);
            // sorted and tallied here, not on the window's thread
            if tree.is_ok() {
                g2.set_phase("Sorting");
            }
            let t = Instant::now();
            let prepared = tree.map(|t| crate::tables::prepare(t, sorts));
            timing("scan: sort and tally (tables::prepare)", t);
            *r2.lock().unwrap() = Some(prepared);
        });
        self.job = Some(Job { path: path.clone(), csv, progress, result, started: Instant::now(), last_layout: Instant::now(), roots, skips });
        if let Some((progress, _)) = self.advisor.cl.own_reading.take() { progress.cancel(); }
        self.advisor_scanning();
        self.scan_button(true);
        // The map shows the scan growing; the header's used space lets the overlay estimate progress.
        self.map.scene.clear();
        self.map.live = Some(LiveMap::new());
        self.map.dirty = true;
        let s = path.display().to_string();
        let is_drive_root = s.len() <= 3 && s.as_bytes().get(1) == Some(&b':');
        let expected = if is_drive_root { win32::disk_space(&path).map(|(t, f)| t.saturating_sub(f)).unwrap_or(0) } else { 0 };
        self.hud.reset(expected);
        text(&self.root, "StatusText", &format!("Scanning {} ...", path.display()));
        if let Ok(p) = find::<ProgressBar>(&self.root, "Progress") {
            let _ = p.SetIsIndeterminate(true);
        }
        self.scan_title(&path);
    }

    fn scan_title(&self, path: &std::path::Path) {
        let _ = self.window.SetTitle(&HSTRING::from(format!("[{}] - Sprawl", path.display())));
        text(&self.root, "TitleText", &format!("[{}]  -  Sprawl", path.display()));
    }

    pub(crate) fn poll(&mut self) {
        // the first start: the setup wizard opens by itself (Skip remembers)
        if !self.wizard_offered && self.shot.is_none() && self.settings.bool("wizard_done") != Some(true) {
            self.wizard_offered = true;
            self.wizard_all = false;
            self.wizard_open();
        }
        self.advisor_poll();
        self.cleanup_poll();
        self.update_poll();
        self.search_poll();
        self.filter_poll();
        if self.wiz_models.poll() {
            self.wizard_models();
        }
        let Some(job) = &self.job else { return };
        let done = job.result.lock().unwrap().take();
        match done {
            None => {
                let live = job.live();
                text(
                    &self.root,
                    "StatusText",
                    &format!("Scanning... {} items ({:.1} s)", thousands(live.files + live.folders), job.started.elapsed().as_secs_f64()),
                );
                if !live.top.is_empty() && job.last_layout.elapsed().as_millis() > 120 {
                    if let Some(m) = &mut self.map.live {
                        m.update(&live.top);
                        self.map.dirty = true;
                    }
                    if let Some(j) = &mut self.job {
                        j.last_layout = Instant::now();
                    }
                }
            }
            Some(res) => {
                let job = self.job.take().unwrap();
                timing("scan: from Scan to the result on the window's thread", job.started);
                let t_ui = Instant::now();
                // A drive selected while scanning supersedes this result, even if it finished
                // just before cancellation. Only the latest requested source is published.
                if let Some(source) = self.pending_scan.take() {
                    self.map.live = None;
                    self.start(source);
                    return;
                }
                self.scan_button(false);
                let res = scan_result(res, job.progress.is_cancelled());
                // the first scan that worked: a small offer to set things up (Not now remembers)
                if res.is_ok() && !job.progress.is_cancelled() && self.shot.is_none() && self.settings.bool("wizard_done") != Some(true) && !self.wizard_offered {
                    self.wizard_offered = true;
                    if let Ok(e) = find::<crate::bindings::Microsoft::UI::Xaml::UIElement>(&self.root, "SetupOffer") {
                        let _ = e.SetVisibility(crate::bindings::Microsoft::UI::Xaml::Visibility::Visible);
                    }
                }
                self.scan_secs = job.started.elapsed().as_secs_f64();
                if let Ok(p) = find::<ProgressBar>(&self.root, "Progress") {
                    let _ = p.SetIsIndeterminate(false);
                    let _ = p.cast::<RangeBase>().and_then(|r| r.SetValue(100.0));
                }
                let live = self.map.live.take();
                match res {
                    Ok(prepared) => {
                        let tree = &prepared.tree;
                        // Let the live boxes open up into the real folders.
                        if let Some(live) = live.filter(|l| !l.index.is_empty()) {
                            let real: HashMap<&str, usize> = tree.nodes[ROOT].children.iter().map(|&c| (tree.nodes[c].name.as_str(), c)).collect();
                            let lt = &live.tree;
                            self.map.scene.rekey(&|old| if old == ROOT { Some(ROOT) } else { real.get(lt.nodes[old].name.as_str()).copied() });
                        } else {
                            self.map.scene.clear();
                        }
                        let header = if job.csv { tree.root_path.clone() } else { job.path.clone() };
                        // A new scan starts unfiltered; the filter box is applied again right away.
                        self.full_tree = None;
                        self.full_parts = None;
                        self.filtering = None;
                        self.filter.clear();
                        self.child_stack.clear();
                        self.scan_taken = std::time::SystemTime::now();
                        let began = self.scan_taken.checked_sub(job.started.elapsed()).unwrap_or(self.scan_taken);
                        self.scan_began = if job.csv { std::fs::metadata(&job.path).and_then(|m| m.modified()).map_or(began, |t| t.min(began)) } else { began };
                        let (network, unknown) = crate::coverage::network_roots(&job.roots, &job.progress.roots());
                        self.coverage = Some(crate::coverage::Coverage { network, unknown, roots: job.roots.clone(), skips: job.skips.clone(), elevated: sprawl_core::is_elevated() });
                        let t = Instant::now();
                        self.set_tree(prepared, &header);
                        timing("scan: set_tree", t);
                        self.map.scene.start_wave();
                        let t = Instant::now();
                        self.watch();
                        timing("scan: watch", t);
                        let t = Instant::now();
                        self.save_snapshot();
                        timing("scan: save_snapshot", t);
                        if !self.filter_text.trim().is_empty() {
                            self.apply_filter();
                        }
                        self.scan_id += 1;
                        self.chat_reset();
                        let t = Instant::now();
                        self.advisor_start();
                        timing("scan: advisor_start", t);
                        let t = Instant::now();
                        self.own_on_open();
                        timing("scan: own_on_open", t);
                        timing("scan: the window's work on the result", t_ui);
                        timing("scan: from Scan to done", job.started);
                        // Settings > Scanning, "After a scan": open the Advisor (not for a
                        // stopped scan, nor in a screenshot run)
                        if self.settings.num("afterScan") == Some(1.0) && !job.progress.is_cancelled() && self.shot.is_none() && self.view != View::Advisor {
                            self.set_view(View::Advisor);
                        }
                    }
                    Err(e) => {
                        // The map was drawing the live preview, whose node numbers aren't the
                        // previous tree's: start it afresh from whatever tree is still loaded.
                        self.map.scene.clear();
                        self.map.galaxy.clear();
                        self.render_map();
                        let mut msg = if e.kind() == std::io::ErrorKind::Interrupted { "Scan stopped.".to_string() } else { format!("Scan failed: {e}") };
                        if let Some(root) = self.full_tree.as_ref().or(self.tree.as_ref()).map(|t| t.root_path.clone()) {
                            self.scan_title(&root);
                            self.update_header(&root);
                            msg += &format!(" Showing previous scan: {}.", root.display());
                        } else {
                            let _ = self.window.SetTitle(&HSTRING::from("Sprawl"));
                            text(&self.root, "TitleText", "Sprawl");
                        }
                        text(&self.root, "StatusText", &msg);
                        self.advisor_start();
                        self.own_on_open();
                    }
                }
            }
        }
    }

    /// Start (or stop) watching the scanned folder for live change pulses.
    pub(crate) fn watch(&mut self) {
        self.watcher = None;
        self.changes = 0;
        if !self.pulses_on {
            return;
        }
        let Some(t) = &self.tree else { return };
        if t.method == sprawl_core::ScanMethod::Csv {
            return;
        }
        let root = t.path_of(ROOT);
        self.watcher = crate::watch::Watcher::start(&root);
    }

    /// Pulse the map where files changed since the last frame.
    pub(crate) fn drain_changes(&mut self) {
        self.drain_shell_rescans();
        let Some(w) = &self.watcher else { return };
        let mut seen = HashSet::new();
        while let Ok(p) = w.rx.try_recv() {
            seen.insert(p);
            if seen.len() > 400 {
                break;
            }
        }
        // Sprawl's own deletes (a clean-up, or Delete in the tree) aren't changes from outside
        let running = self.advisor.cleanup.running.is_some();
        self.own_deletes.retain(|(_, at)| running || at.elapsed().as_secs() < 120);
        if !self.own_deletes.is_empty() {
            seen.retain(|p| {
                let l = p.to_string_lossy().to_lowercase();
                !self.own_deletes.iter().any(|(o, _)| l == *o || l.strip_prefix(o.as_str()).is_some_and(|r| r.starts_with('\\')))
            });
        }
        if seen.is_empty() || self.map.live.is_some() {
            return;
        }
        let Some(t) = &self.tree else { return };
        for p in &seen {
            let n = crate::watch::find(t, p);
            self.search.changed(self.tree_gen, n);
            self.map.scene.pulse(t, n);
            self.map.galaxy.pulse(t, n);
        }
        self.changes += seen.len() as u64;
        let c = self.changes;
        text(&self.root, "FootLeft", &format!("Live: {} change{} in this folder since the scan", thousands(c), if c == 1 { "" } else { "s" }));
    }

    /// The sort columns now, for a worker preparing a tree.
    pub(crate) fn sorts(&self) -> crate::tables::Sorts {
        crate::tables::Sorts { tree: self.tables.tree_sort, files: self.tables.files_sort, exts: self.tables.ext_sort }
    }

    /// Show a tree prepared on a worker ([`crate::tables::prepare`]).
    pub(crate) fn set_tree(&mut self, p: crate::tables::Prepared, path: &std::path::Path) {
        self.hist_close();
        let crate::tables::Prepared { tree, alloc, ranks, files, exts, colours, sorts } = p;
        self.alloc = alloc;
        self.ranks = ranks;
        self.tree = Some(tree);
        self.tree_gen += 1;
        self.tree_set += 1;
        self.ranks_start();
        self.expanded = HashSet::from([ROOT]);
        self.selected = Some(ROOT);
        self.marked.clear();
        self.map_root = ROOT;
        self.ext_selected = None;
        self.exts = exts;
        self.colours = colours;
        self.files = files;
        // a column clicked while it was being prepared: sorted again here
        if sorts.tree != self.tables.tree_sort {
            self.sort_tree();
        }
        if sorts.files != self.tables.files_sort {
            self.sort_files();
        }
        if sorts.exts != self.tables.ext_sort {
            self.sort_exts();
        }
        let t = self.tree.as_ref().unwrap();
        text(&self.root, "StatusText", &format!("Scan complete in {:.2} seconds ({})", self.scan_secs, method_label(t.method)));
        if t.method == sprawl_core::ScanMethod::Walk && !sprawl_core::is_elevated() {
            text(&self.root, "FootLeft", "Tip: Options > Restart as Administrator reads the MFT directly, which is much faster on big drives.");
        }
        self.update_header(path);
        self.scan_title(path);
        // The free-space tile only makes sense when a whole drive was scanned.
        let s = path.display().to_string();
        let drive_root = s.len() <= 3 && s.as_bytes().get(1) == Some(&b':');
        self.map.free = if drive_root { win32::disk_space(path).map(|(_, f)| f) } else { None };
        self.refresh_rows();
        self.refresh_exts();
        self.render_map();
    }

    pub(crate) fn rebuild_exts(&mut self) {
        let t = self.tree.as_ref().unwrap();
        self.exts = crate::tables::ext_rows(t, &self.alloc);
        self.colours = self.exts.iter().take(paint::COUNT).enumerate().map(|(i, e)| (e.ext.clone(), i)).collect();
    }

    /// The header card: what was scanned (or is picked, before a scan) on one line, what the
    /// scan holds on the next, and the drive it lives on on its own line, so a folder's scan
    /// isn't read as the drive's numbers.
    pub(crate) fn update_header(&self, path: &std::path::Path) {
        let scanned = self.tree.as_ref().filter(|t| t.root_path == path);
        text(&self.root, "HeadLabel", if scanned.is_some() { "Scanned:" } else { "Selected:" });
        text(&self.root, "SelText", &path.display().to_string());
        let info = match scanned {
            Some(t) => {
                let n = &t.nodes[ROOT];
                // a CSV wasn't read from the disk: it has no scan time to show
                let when = if self.scan_secs > 0.0 && t.method != sprawl_core::ScanMethod::Csv {
                    format!("read in {:.2} s ({})", self.scan_secs, method_label(t.method))
                } else {
                    method_label(t.method).to_string()
                };
                format!(
                    "{}   ·   {} files, {} folders   ·   {} unreadable   ·   {when}",
                    self.size(n.size),
                    thousands(n.files),
                    thousands(n.folders),
                    thousands(t.errors)
                )
            }
            None => "Not scanned yet".to_string(),
        };
        text(&self.root, "ScanInfo", &info);
        if let Ok(e) = find::<crate::bindings::Microsoft::UI::Xaml::UIElement>(&self.root, "CoverageDetails") {
            let shown = scanned.is_some() && self.coverage.is_some();
            let _ = e.SetVisibility(if shown { crate::bindings::Microsoft::UI::Xaml::Visibility::Visible } else { crate::bindings::Microsoft::UI::Xaml::Visibility::Collapsed });
        }
        text(&self.root, "DriveLabel", &drive_label(path));
        // A CSV file names any path it likes: asking a network path for its free space would
        // connect to that server just because the file was opened.
        let from_csv = self.tree.as_ref().is_some_and(|t| t.method == sprawl_core::ScanMethod::Csv);
        let remote = path.to_string_lossy().starts_with(r"\\");
        let space = if from_csv && remote { None } else { win32::disk_space(path) };
        let bar = (find::<FrameworkElement>(&self.root, "UsageTrack"), find::<FrameworkElement>(&self.root, "UsageBar"));
        if let Some((total, free)) = space {
            let used = total.saturating_sub(free);
            let pct = |v: u64| if total == 0 { 0.0 } else { v as f64 * 100.0 / total as f64 };
            text(
                &self.root,
                "DriveText",
                &format!("{}   ·   {} used ({:.1}%)   ·   {} free ({:.1}%)", self.size(total), self.size(used), pct(used), self.size(free), pct(free)),
            );
            if let (Ok(track), Ok(bar)) = bar {
                let w = track.ActualWidth().unwrap_or(200.0);
                let _ = bar.SetWidth((w * pct(used) / 100.0).max(14.0));
            }
        } else {
            text(&self.root, "DriveText", "-");
            if let (_, Ok(bar)) = bar {
                let _ = bar.SetWidth(0.0);
            }
        }
    }

    /// The Scan button turns into Stop while a scan runs.
    fn scan_button(&self, scanning: bool) {
        text(&self.root, "ScanButtonText", if scanning { "Stop" } else { "Scan" });
        if let Ok(i) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::FontIcon>(&self.root, "ScanButtonIcon") {
            let glyph = char::from_u32(if scanning { 0xE71A } else { 0xE721 }).unwrap_or(' ').to_string();
            let _ = i.SetGlyph(&glyph.as_str().into());
        }
    }

    pub(crate) fn stop_scan(&mut self) {
        self.pending_scan = None;
        if let Some(j) = &self.job {
            j.progress.cancel();
            text(&self.root, "StatusText", "Stopping...");
        }
    }

    // ------------------------------------------------------------ filter

    /// The filter box changed; it's applied once typing pauses.
    pub(crate) fn filter_changed(&mut self, text: String) {
        self.filter_text = text;
        self.filter_pending = Some(Instant::now());
    }

    /// Filter the whole tree by the filter box, on a worker; [`State::filter_poll`] shows it.
    pub(crate) fn apply_filter(&mut self) {
        self.filter_again(false);
    }

    /// The same, also when the filtered view on screen has the pattern already (`again`: the
    /// full tree under it changed).
    pub(crate) fn filter_again(&mut self, again: bool) {
        use crate::tables::FilterStep;
        self.filter_pending = None;
        let pattern = self.filter_text.trim().to_string();
        let shown = self.full_tree.is_some().then_some(self.filter.as_str());
        let busy = self.filtering.as_ref().map(|f| f.pattern.as_str());
        match crate::tables::filter_step(&pattern, shown, busy, again) {
            FilterStep::Nothing => {
                if pattern.is_empty() {
                    self.filtering = None;
                }
                return;
            }
            FilterStep::Unfilter => return self.unfilter(),
            FilterStep::Filter => {}
        }
        let Some(base) = self.full_tree.clone().or_else(|| self.tree.clone()) else { return };
        let sorts = self.sorts();
        let (tx, rx) = std::sync::mpsc::channel();
        let (p, b) = (pattern.clone(), base.clone());
        std::thread::spawn(move || {
            let _ = tx.send(crate::tables::prepare(sprawl_core::filter_tree(&b, &p), sorts));
        });
        if !pattern.is_empty() {
            text(&self.root, "StatusText", &format!("Filtering by \"{pattern}\"..."));
        }
        self.filtering = Some(Filtering { pattern, base, gen: self.tree_gen, rx });
    }

    /// A filtered (or unfiltered again) tree is ready: show it, unless the tree it came from
    /// changed meanwhile (then it is made again).
    pub(crate) fn filter_poll(&mut self) {
        let Some(f) = &self.filtering else { return };
        let Ok(prepared) = f.rx.try_recv() else { return };
        let Filtering { pattern, base, gen, .. } = self.filtering.take().unwrap();
        if gen != self.tree_gen {
            return self.filter_again(true);
        }
        // the full tree's own parts are kept for when the filter is cleared
        if self.full_tree.is_none() {
            self.full_parts = Some(crate::tables::Parts {
                alloc: std::mem::take(&mut self.alloc),
                ranks: std::mem::take(&mut self.ranks),
                files: std::mem::take(&mut self.files),
                exts: std::mem::take(&mut self.exts),
                colours: std::mem::take(&mut self.colours),
                sorts: self.sorts(),
            });
        }
        self.filter = pattern.clone();
        let path = base.root_path.clone();
        // Node numbers change, so the treemap grows afresh rather than morphing, and the
        // back/forward stack of nodes no longer means anything.
        self.map.scene.clear();
        self.child_stack.clear();
        let (found, bytes) = (prepared.tree.nodes[ROOT].files, prepared.tree.nodes[ROOT].size);
        self.full_tree = Some(base);
        self.set_tree(prepared, &path);
        self.advisor_refresh();
        text(&self.root, "StatusText", &format!("Filter \"{pattern}\": {} files, {}", thousands(found), self.size(bytes)));
        // typed on while it was made
        if self.filter_text.trim() != self.filter && self.filter_pending.is_none() {
            self.apply_filter();
        }
    }

    /// The filter box was cleared: the full tree again, as it was worked out (kept up to date
    /// with deletes and clean-ups), with no copy.
    fn unfilter(&mut self) {
        self.filtering = None;
        let Some(tree) = self.full_tree.take() else { return };
        let parts = match self.full_parts.take() {
            Some(p) => p,
            // never left without them; should it happen, worked out here
            None => {
                let mut tree = tree;
                let sorts = self.sorts();
                let alloc: Vec<u64> = tree.nodes.iter().map(|n| n.allocated).collect();
                let mut p = crate::tables::Parts { alloc, ranks: Default::default(), files: Vec::new(), exts: Vec::new(), colours: HashMap::new(), sorts };
                p.redo(Arc::make_mut(&mut tree));
                p.colours = p.exts.iter().take(paint::COUNT).enumerate().map(|(i, e)| (e.ext.clone(), i)).collect();
                self.full_tree = Some(tree);
                return self.unfilter_with(p);
            }
        };
        self.full_tree = Some(tree);
        self.unfilter_with(parts);
    }

    fn unfilter_with(&mut self, parts: crate::tables::Parts) {
        let Some(tree) = self.full_tree.take() else { return };
        let crate::tables::Parts { alloc, ranks, files, exts, colours, sorts } = parts;
        self.filter.clear();
        let path = tree.root_path.clone();
        self.map.scene.clear();
        self.child_stack.clear();
        self.set_tree(crate::tables::Prepared { tree, alloc, ranks, files, exts, colours, sorts }, &path);
        self.advisor_refresh();
    }

    // ------------------------------------------------------------ screenshot mode

    pub(crate) fn shot_tick(&mut self) {
        let Some(path) = self.shot.clone() else { return };
        if self.job.is_some() {
            return;
        }
        if self.tree.is_none() && std::env::var_os("SPRAWL_EMPTY").is_none() {
            return;
        }
        self.shot_ticks += 1;
        // dev: SPRAWL_HISTORY=n opens the History panel at slider position n for the picture
        if self.shot_ticks == 1 {
            if let Some(p) = std::env::var("SPRAWL_HISTORY").ok().and_then(|v| v.parse::<usize>().ok()) {
                self.hist_toggle();
                self.hist_seek(p);
            }
        }
        // dev: a `kb=` step's Alt is let go of a few frames after its keys were read
        if self.shot_ticks % 30 == 10 && std::env::var("SPRAWL_SWITCH").is_ok_and(|v| v.contains("kb=")) {
            crate::win32::alt_state(false);
        }
        // dev: a `drag=` step's window, one size further each frame
        if let Some((from, to, k)) = SHOT_DRAG.with(|d| d.get()) {
            let k = k + 1;
            let at = |a: i32, b: i32| a + (b - a) * k as i32 / 20;
            let _ = self.window.AppWindow().and_then(|a| a.Resize(crate::bindings::Windows::Graphics::SizeInt32 { Width: at(from.0, to.0), Height: at(from.1, to.1) }));
            SHOT_DRAG.with(|d| d.set((k < 20).then_some((from, to, k))));
        }
        // dev: SPRAWL_SWITCH=ModeCity,ModeGalaxy,... runs those commands 30 frames apart,
        // hovering the map in between
        if self.shot_ticks % 30 == 0 {
            if let Some(c) = std::env::var("SPRAWL_SWITCH").ok().and_then(|v| v.split(',').nth(self.shot_ticks as usize / 30 - 1).map(str::to_string)) {
                let (w, h) = (self.map.scene.w as f64, self.map.scene.h as f64);
                for k in 0..40 {
                    self.map_hover(w * (0.3 + k as f64 * 0.01), h * 0.5, false, false);
                }
                match c.as_str() {
                    // dev: wheel-zoom in at the middle, or drag the view to the right
                    "zoom" => self.map_wheel(w * 0.5, h * 0.5, 360),
                    "pan" => {
                        self.map_press(w * 0.5, h * 0.5);
                        for k in 1..=20 {
                            self.map_hover(w * 0.5 + k as f64 * 6.0, h * 0.5, false, true);
                        }
                    }
                    // dev: type a goal, or press an Advisor button
                    g if g.starts_with("goal=") => self.advisor_set_goal(&g[5..]),
                    o if o.starts_with("open=") => self.advisor_open(o[5..].parse().unwrap_or(0)),
                    t if t.starts_with("tick=") => {
                        self.cleanup_toggle(t[5..].parse().unwrap_or(0));
                        self.render_advisor();
                    }
                    "wait" => {}
                    // dev: the window resized mid-run (size=2000x1100), as dragging its edge,
                    // maximising or restoring it would
                    s if s.starts_with("size=") => {
                        if let Some((sw, sh)) = s[5..].split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))) {
                            let _ = self.window.AppWindow().and_then(|a| a.Resize(crate::bindings::Windows::Graphics::SizeInt32 { Width: sw, Height: sh }));
                        }
                    }
                    // dev: the window's edge dragged to this size over the next 20 frames, a new
                    // size each frame (drag=2000x1100)
                    s if s.starts_with("drag=") => {
                        if let Some((sw, sh)) = s[5..].split_once('x').and_then(|(a, b)| Some((a.parse::<i32>().ok()?, b.parse::<i32>().ok()?))) {
                            let now = crate::win32::window_size(self.hwnd).unwrap_or((sw, sh));
                            SHOT_DRAG.with(|d| d.set(Some((now, (sw, sh), 0))));
                        }
                    }
                    // dev: give the control the keyboard's focus (focus=FilterBox), for the focus ring's picture
                    // (Keyboard, not Programmatic: a combo box only draws its ring for the first)
                    f if f.starts_with("focus=") => {
                        if let Ok(e) = find::<crate::bindings::Microsoft::UI::Xaml::UIElement>(&self.root, &f[6..]) {
                            let _ = e.Focus(crate::bindings::Microsoft::UI::Xaml::FocusState::Keyboard);
                        }
                    }
                    // dev: open a dropdown by its x:Name (drop=PaletteBox), or a menu or any
                    // expandable thing by its name (menu=File), as a screen reader would; the
                    // open popup is in the picture with SPRAWL_SHOT_POPUPS
                    d if d.starts_with("drop=") => {
                        crate::win32::expand_by(self.hwnd, &d[5..], true);
                    }
                    d if d.starts_with("menu=") => {
                        crate::win32::expand_by(self.hwnd, &d[5..], false);
                    }
                    // dev: keys pressed in this window only, as its own input (kb=alt.right,
                    // kb=alt+f): nothing outside the process sees them
                    k if k.starts_with("kb=") => {
                        let presses: Vec<&str> = k[3..].split('.').collect();
                        crate::win32::post_keys(self.hwnd, &presses);
                    }
                    // dev: what has the focus, and which menus are open, in the timing log
                    "barlog" => crate::menu::dev_report(),
                    // dev: a right-click on row k of the main table (ctx=3), the menu key on the
                    // keyboard's row (ctxkey), a right-click on the map at x,y (mapctx=300x200)
                    c if c.starts_with("ctx=") => {
                        let k: usize = c[4..].parse().unwrap_or(0);
                        let r = self.rows_now(crate::tables::Which::Main);
                        let y = r.y_of(k);
                        let at = crate::bindings::Windows::Foundation::Point { X: 60.0, Y: (y - r.off + ROW_H / 2.0) as f32 };
                        self.row_menu(y + ROW_H / 2.0, at, false);
                    }
                    "ctxkey" => self.row_menu_by_key(),
                    m if m.starts_with("mapctx=") => {
                        if let Some((x, y)) = m[7..].split_once('x') {
                            self.map_menu(x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0));
                        }
                    }
                    // dev: a control in one of its visual states, as the pointer or a press would
                    // put it (pose=ScanBtn=PointerOver), for the shadows' pictures
                    p if p.starts_with("pose=") => {
                        if let Some((n, s)) = p[5..].split_once('=') {
                            if let Ok(c) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::Control>(&self.root, n) {
                                let _ = crate::bindings::Microsoft::UI::Xaml::VisualStateManager::GoToState(&c, &s.into(), false);
                            }
                        }
                    }
                    // dev: the pointer onto the menu bar's title k while a menu is open (hover=1)
                    h if h.starts_with("hover=") => crate::menubar::dev_hover(h[6..].parse().unwrap_or(0)),
                    // dev: the pointer said to rest on a menu's row by its x:Name (point=BarSub3),
                    // for the bar to find there as it finds the real pointer
                    p if p.starts_with("point=") => crate::menu::dev_point(&p[6..]),
                    // dev: a screen reader's Toggle or Select on a menu's check or dot (uiamark=oAuto)
                    u if u.starts_with("uiamark=") => crate::win32::toggle_or_select_by(self.hwnd, &u[8..]),
                    // dev: the menu bar as UI Automation sees it, in the timing log
                    "uia" => crate::win32::uia_report(self.hwnd),
                    // dev: the other theme, as saving it in Settings would (in memory: a screenshot run saves nothing)
                    "theme" => {
                        self.settings.set("theme", if crate::theme::light() { 2 } else { 1 });
                        crate::restyle::request();
                    }
                    // dev: the Delete card as it looks part way; nothing is deleted
                    "deletedemo" => {
                        self.advisor.delete_total = (3 << 30, true, 2);
                        self.delete_card(Some(1 << 30));
                    }
                    // dev: render a synthetic successful result; never run a cleanup.
                    "preventiondemo" => {
                        self.advisor.cleanup.done = Some(crate::cleanup::Done {
                            freed: 128 << 20, items: 2,
                            prevention: ["dev.npm-cache", "win.user-temp"].iter()
                                .filter_map(|id| sprawl_advisor::prevention::for_rule(id)).collect(),
                            ..Default::default()
                        });
                        self.render_advisor();
                    }
                    // dev: a sample AI answer in the chat, to see the Markdown drawn; nothing is sent
                    // (mddemo2: only its second half, which a screenshot can't otherwise reach)
                    "mddemo" | "mddemo2" => {
                        let half = if c == "mddemo2" { MD_DEMO.find("- Caches").unwrap_or(0) } else { 0 };
                        self.advisor.chat.log.push((true, "Where did the space go?".into()));
                        self.advisor.chat.log.push((false, MD_DEMO[half..].into()));
                        self.render_advisor();
                    }
                    // dev: Enter in the search box, or a key in the main table (key=40 is Down)
                    "boxenter" => self.search_box_down(),
                    // dev: a double-click on row k of the main table
                    d if d.starts_with("dbl=") => {
                        let k: f64 = d[4..].parse().unwrap_or(0.0);
                        self.row_tapped(40.0, self.rows_now(crate::tables::Which::Main).y_of(k as usize) + 5.0, false);
                        self.row_tapped(40.0, self.rows_now(crate::tables::Which::Main).y_of(k as usize) + 5.0, true);
                    }
                    // dev: two plain taps on row k (WinUI lost the double-tap)
                    d if d.starts_with("tap2=") => {
                        let k: f64 = d[5..].parse().unwrap_or(0.0);
                        self.row_tapped(40.0, self.rows_now(crate::tables::Which::Main).y_of(k as usize) + 5.0, false);
                        self.row_tapped(40.0, self.rows_now(crate::tables::Which::Main).y_of(k as usize) + 5.0, false);
                    }
                    k if k.starts_with("key=") => {
                        let t0 = std::time::Instant::now();
                        self.table_key(k[4..].parse().unwrap_or(0));
                        crate::state::timing(&format!("{k}"), t0);
                    }
                    // dev: a click on the main table's header at x (sorts by that column)
                    h if h.starts_with("hdr=") => {
                        let width = find::<crate::bindings::Microsoft::UI::Xaml::Controls::ScrollViewer>(&self.root, "TreeScroll").ok().and_then(|s| s.ViewportWidth().ok()).unwrap_or(1200.0);
                        self.header_clicked(crate::tables::Which::Main, h[4..].parse().unwrap_or(0.0), width);
                    }
                    // dev: type into the search box
                    q if q.starts_with("find=") => {
                        let t = q[5..].to_string();
                        self.search_set_box(&t, t.len());
                    }
                    // dev: the saved searches menu open
                    "saved" => self.search_saved_open(),
                    // dev: open a dropdown or a menu bar's menu as a screen reader would
                    // (expand=File, expand=TmPick_accent)
                    e if e.starts_with("expand=") => {
                        crate::win32::expand_dropdown(self.hwnd, &e[7..]);
                    }
                    // dev: close a named menu or flyout
                    h if h.starts_with("hide=") => {
                        if let Ok(f) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::FlyoutBase>(&self.root, &h[5..]) {
                            let _ = f.Hide();
                        }
                    }
                    // dev: a named flyout shown by an element (flyout=SearchHelpFly@SearchHelp), or at
                    // a point in it as a right-click shows it (flyout=RowMenu@TreeScroll@80x40: not a
                    // comma, the steps are split on those)
                    m if m.starts_with("flyout=") => {
                        let mut parts = m[7..].split('@');
                        let (menu, target, at) = (parts.next().unwrap_or(""), parts.next().unwrap_or("TreeScroll"), parts.next());
                        let shown = || -> windows_core::Result<()> {
                            use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{FlyoutBase, FlyoutShowOptions};
                            let f: FlyoutBase = find(&self.root, menu)?;
                            let t: crate::bindings::Microsoft::UI::Xaml::DependencyObject = find(&self.root, target)?;
                            let opts = FlyoutShowOptions::new()?;
                            if let Some((x, y)) = at.and_then(|a| a.split_once('x')) {
                                opts.SetPosition(Some(crate::bindings::Windows::Foundation::Point { X: x.parse().unwrap_or(0.0), Y: y.parse().unwrap_or(0.0) }))?;
                            }
                            f.ShowAtWithOptions(&t, &opts)
                        };
                        if let Err(e) = shown() {
                            eprintln!("menu {menu}: {e:?}");
                        }
                    }
                    // dev: a key in the search box (boxkey=40 is Down)
                    k if k.starts_with("boxkey=") => {
                        self.search_box_key(k[7..].parse().unwrap_or(0));
                    }
                    // dev: Search's filter chips: chip=k_i picks option i of chip k, chipclick=k
                    // clicks chip k (lit: takes its words out)
                    p if p.starts_with("chip=") => {
                        if let Some((k, i)) = p[5..].split_once('_') {
                            self.search_chip_pick(k.parse().unwrap_or(0), i.parse().unwrap_or(0));
                        }
                    }
                    p if p.starts_with("chipclick=") => self.search_chip_click(p[10..].parse().unwrap_or(0), false),
                    // dev: type into a text box, field=Name=text
                    f if f.starts_with("field=") => {
                        if let Some((n, v)) = f[6..].split_once('=') {
                            if let Ok(t) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::TextBox>(&self.root, n) {
                                let _ = t.SetText(&v.into());
                            }
                        }
                    }
                    q if q.starts_with("chat=") => self.advisor_chat_dev(&q[5..]),
                    "chatdemo" => self.advisor_chat_demo(),
                    // dev: an AI suggestion of every part in the scan
                    "suggestdemo" => self.suggest_demo(),
                    // dev: click the map a third of the way across
                    "mapclick" => self.map_click(w * 0.3, h * 0.5),
                    // dev: write where the Cleaners tab's list is scrolled to into the timing log
                    "cllog" => {
                        if let Some(sv) = self.advisor.pfind::<crate::bindings::Microsoft::UI::Xaml::Controls::ScrollViewer>("ClScroll") {
                            crate::state::timing(&format!("ClScroll is at {:.0}", sv.VerticalOffset().unwrap_or(-1.0)), std::time::Instant::now());
                        }
                    }
                    // dev: scroll the Cleaners tab's list down to y
                    y if y.starts_with("clscroll=") => {
                        if let Some(sv) = self.advisor.pfind::<crate::bindings::Microsoft::UI::Xaml::Controls::ScrollViewer>("ClScroll") {
                            let _ = sv.ChangeViewWithOptionalAnimation(None, y[9..].parse().ok(), None, true);
                        }
                    }
                    // dev: a Cleaners button, AdvClO0 (open), AdvClT2 (tick), AdvClS2 (files)
                    a if a.starts_with("AdvCl") && a.ends_with(|c: char| c.is_ascii_digit()) && !a.starts_with("AdvClean") => {
                        let cut = a.trim_end_matches(|c: char| c.is_ascii_digit()).len();
                        self.cleaner_command(&a[..cut], a[cut..].parse().unwrap_or(0));
                    }
                    // dev: two compressed folders in the record (not saved), and the list shown (F1)
                    "compdemo" => {
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                        let pf = std::env::var("ProgramFiles").unwrap_or_default();
                        self.advisor.memory.compressed.push(sprawl_advisor::memory::Compressed { path: format!(r"{pf}\dotnet"), alg: "xpress8k".into(), when: now - 86_400 * 3, saved: 1_300_000_000, size: 2_900_000_000, partial: false });
                        self.advisor.memory.compressed.push(sprawl_advisor::memory::Compressed { path: format!(r"{pf}\Gone Game"), alg: "xpress16k".into(), when: now - 86_400 * 40, saved: 4_200_000_000, size: 9_100_000_000, partial: false });
                        self.advisor_command("AdvOnlyComp");
                    }
                    // dev: an AI suggestion of the first three findings it may suggest (R9)
                    "fnsugdemo" => {
                        let keys: Vec<String> = self.advice().map(|r| self.suggestable(r).into_iter().take(3).map(|i| crate::cleanup::key(&r.findings[i].path)).collect()).unwrap_or_default();
                        self.advisor.fn_suggest = Some((keys, "These are caches the apps make again.".into()));
                        self.render_advisor();
                    }
                    // dev: click the first row that ticks on a second click (R7c)
                    "secondtry" => {
                        let first = self.advice().and_then(|r| (0..r.findings.len()).find(|&i| crate::cleanup::second_try(&r.findings[i], r.counted[i]) || r.findings[i].forced_from.is_some()));
                        if let Some(i) = first {
                            self.second_try_click(i);
                            self.render_advisor();
                        }
                    }
                    // dev: tick finding N as its box does (tick=0)
                    t if t.starts_with("tick=") => {
                        if let Ok(i) = t[5..].parse() {
                            self.cleanup_toggle(i);
                            self.render_advisor();
                        }
                    }
                    // dev: a privacy part's tick, AdvPvT0
                    a if a.starts_with("AdvPvT") => {
                        self.privacy_toggle(a[6..].parse().unwrap_or(0));
                        self.advisor_redraw();
                    }
                    a if a.starts_with("Adv") => self.advisor_command(a),
                    // dev: the Theme page in a state (ThemeMore, ThemeTab=1, ThemePre=2, ThemeDrop=accent)
                    t if t.starts_with("Theme") => self.theme_dev(t),
                    // dev: a Settings section or a wizard button
                    w if w.starts_with("PfSec") || w.starts_with("Wiz") => self.wizard_command(w),
                    _ => self.command(&c),
                }
            }
        }
        // Let the finish wave and morphs play out first (about 3 s at 60 frames a second).
        // ...and every SPRAWL_SWITCH step its 30 frames, plus a second for the last one to settle
        let steps = std::env::var("SPRAWL_SWITCH").map_or(0, |v| v.split(',').count() as u32);
        // dev: SPRAWL_SHOT_EARLY=n takes the picture n frames after the last step instead, to
        // catch a menu or a dropdown part way through opening
        let early = std::env::var("SPRAWL_SHOT_EARLY").ok().and_then(|v| v.parse::<u32>().ok());
        let due = early.map_or(180u32.max(steps * 30 + 60), |n| steps * 30 + n);
        if self.shot_ticks < due {
            return;
        }
        // dev: SPRAWL_SHOT_SEQ=n pictures a menu opening frame by frame: from then on each frame
        // is taken, and from the first one with a popup open, that frame, the two before it and
        // the n-3 after are saved (out-f00.png, ...), each with the open popups in the timing log
        if let Some(n) = std::env::var("SPRAWL_SHOT_SEQ").ok().and_then(|v| v.parse::<u32>().ok()) {
            thread_local! {
                /// The last frames taken (picture, log line), and how many have been saved.
                static SEQ: std::cell::RefCell<(std::collections::VecDeque<(Option<(u32, u32, Vec<u8>)>, String)>, Option<u32>)> = std::cell::RefCell::new((std::collections::VecDeque::new(), None));
            }
            let t0 = std::time::Instant::now();
            let report = self.root.cast::<crate::bindings::Microsoft::UI::Xaml::UIElement>().map(|r| crate::popups::dev_report(&r)).unwrap_or_default();
            let shot = win32::capture(self.hwnd);
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let save = |i: u32, shot: Option<(u32, u32, Vec<u8>)>, report: &str| {
                if let Some((w, h, px)) = shot {
                    let out = path.with_file_name(format!("{stem}-f{i:02}.png"));
                    if let Some(dir) = out.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = std::fs::write(&out, paint::png(w, h, &px));
                }
                crate::state::timing(&format!("frame {i}: {report}"), t0);
            };
            let done = SEQ.with(|q| {
                let mut q = q.borrow_mut();
                match q.1 {
                    None if report != "no popup" => {
                        let before: Vec<_> = q.0.drain(..).collect();
                        let mut i = 0;
                        for (s, r) in before {
                            save(i, s, &r);
                            i += 1;
                        }
                        save(i, shot, &report);
                        q.1 = Some(i + 1);
                    }
                    None => {
                        q.0.push_back((shot, report));
                        if q.0.len() > 2 {
                            q.0.pop_front();
                        }
                    }
                    Some(i) => {
                        save(i, shot, &report);
                        q.1 = Some(i + 1);
                    }
                }
                q.1.is_some_and(|i| i >= n) || self.shot_ticks > due + 600
            });
            if done {
                std::process::exit(0);
            }
            return;
        }
        if let Some((w, h, px)) = win32::capture(self.hwnd) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, paint::png(w, h, &px));
            std::process::exit(0);
        }
        if self.shot_ticks > 900 {
            eprintln!("could not capture the window");
            std::process::exit(3);
        }
    }
}
