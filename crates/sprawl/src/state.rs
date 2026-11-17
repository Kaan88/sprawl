//! The window's state: what was scanned, what is shown, and the helpers every part uses.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use sprawl_core::{format_size, Progress, Tree, ROOT};
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

#[derive(Clone, Copy, PartialEq)]
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

pub(crate) struct Job {
    pub(crate) path: PathBuf,
    pub(crate) csv: bool,
    pub(crate) progress: Arc<Progress>,
    pub(crate) result: Arc<Mutex<Option<std::io::Result<crate::tables::Prepared>>>>,
    pub(crate) started: Instant,
    pub(crate) last_layout: Instant,
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
    pub(crate) window: Window,
    pub(crate) hwnd: isize,
    pub(crate) root: FrameworkElement,
    pub(crate) drives: Vec<String>,
    /// Shared so the advisor can read it on another thread; changed through `Arc::make_mut`.
    pub(crate) tree: Option<Arc<Tree>>,
    pub(crate) alloc: Vec<u64>,
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
    pub(crate) pulses_on: bool,
    pub(crate) map_root: usize,
    pub(crate) map: MapView,
    pub(crate) hud: Hud,
    pub(crate) units: Units,
    pub(crate) view: View,
    pub(crate) job: Option<Job>,
    pub(crate) scan_secs: f64,
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

pub(crate) fn load<T: Interface>(xaml: &str) -> windows_core::Result<T> {
    XamlReader::Load(&HSTRING::from(xaml))?.cast()
}

impl State {
    pub(crate) fn size(&self, v: u64) -> String {
        size_with(self.units, v)
    }
}

/// `v` bytes in `units`.
pub(crate) fn size_with(units: Units, v: u64) -> String {
    {
        let f = |d: f64, u: &str| format!("{:.1} {u}", v as f64 / d);
        match units {
            Units::Dynamic => format_size(v),
            Units::Bytes => thousands(v),
            Units::KB => f(1024.0, "KB"),
            Units::MB => f(1024.0 * 1024.0, "MB"),
            Units::GB => f(1024.0 * 1024.0 * 1024.0, "GB"),
            Units::TB => f(1024.0f64.powi(4), "TB"),
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
        if self.job.is_some() {
            return;
        }
        // a new scan would replace the tree the clean-up is checking and rescanning
        if self.advisor.cleanup.running.is_some() {
            text(&self.root, "FootLeft", "A clean-up is running: scan again when it has finished.");
            return;
        }
        let (path, csv) = match &source {
            Source::Folders(v) if v.len() == 1 => (v[0].clone(), false),
            Source::Folders(v) if v.is_empty() => return,
            Source::Folders(_) => (PathBuf::from(sprawl_core::MULTIPLE), false),
            Source::Csv(p) => (p.clone(), true),
        };
        let progress = Progress::new();
        let result = Arc::new(Mutex::new(None));
        let (g2, r2) = (progress.clone(), result.clone());
        let sorts = self.sorts();
        std::thread::spawn(move || {
            let tree = match &source {
                Source::Folders(v) if v.len() == 1 => sprawl_core::scan_auto(&v[0], &g2),
                Source::Folders(v) => sprawl_core::scan_many(v, &g2),
                Source::Csv(p) => sprawl_core::load_csv(p, &g2),
            };
            // sorted and tallied here, not on the window's thread
            if tree.is_ok() {
                g2.set_phase("Sorting");
            }
            *r2.lock().unwrap() = Some(tree.map(|t| crate::tables::prepare(t, sorts)));
        });
        self.job = Some(Job { path: path.clone(), csv, progress, result, started: Instant::now(), last_layout: Instant::now() });
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
                self.scan_button(false);
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
                        self.set_tree(prepared, &header);
                        self.map.scene.start_wave();
                        self.watch();
                        self.save_snapshot();
                        if !self.filter_text.trim().is_empty() {
                            self.apply_filter();
                        }
                        self.scan_id += 1;
                        self.chat_reset();
                        self.advisor_start();
                    }
                    Err(e) => {
                        // The map was drawing the live preview, whose node numbers aren't the
                        // previous tree's: start it afresh from whatever tree is still loaded.
                        self.map.scene.clear();
                        self.map.galaxy.clear();
                        self.render_map();
                        let msg = if e.kind() == std::io::ErrorKind::Interrupted { "Scan stopped.".to_string() } else { format!("Scan failed: {e}") };
                        text(&self.root, "StatusText", &msg);
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
        let crate::tables::Prepared { tree, alloc, files, exts, colours, sorts } = p;
        self.alloc = alloc;
        self.tree = Some(tree);
        self.tree_gen += 1;
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
        let how = match t.method {
            sprawl_core::ScanMethod::Mft => "read from the MFT",
            sprawl_core::ScanMethod::Csv => "loaded from CSV",
            sprawl_core::ScanMethod::Walk => "folder walk",
        };
        text(&self.root, "StatusText", &format!("Scan complete in {:.2} seconds ({how})", self.scan_secs));
        if t.method == sprawl_core::ScanMethod::Walk && !sprawl_core::is_elevated() {
            text(&self.root, "FootLeft", "Tip: Options > Restart as Administrator reads the MFT directly, which is much faster on big drives.");
        }
        text(
            &self.root,
            "ScanInfo",
            &format!("{} files, {} folders, {} unreadable", thousands(t.nodes[ROOT].files), thousands(t.nodes[ROOT].folders), thousands(t.errors)),
        );
        self.update_header(path);
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

    pub(crate) fn update_header(&self, path: &std::path::Path) {
        text(&self.root, "SelText", &path.display().to_string());
        // A CSV file names any path it likes: asking a network path for its free space would
        // connect to that server just because the file was opened.
        let from_csv = self.tree.as_ref().is_some_and(|t| t.method == sprawl_core::ScanMethod::Csv);
        let remote = path.to_string_lossy().starts_with(r"\\");
        let space = if from_csv && remote { None } else { win32::disk_space(path) };
        if let Some((total, free)) = space {
            let used = total.saturating_sub(free);
            let pct = |v: u64| if total == 0 { 0.0 } else { v as f64 * 100.0 / total as f64 };
            text(&self.root, "TotalText", &self.size(total));
            text(&self.root, "UsedText", &format!("{}   ({:.1}%)", self.size(used), pct(used)));
            text(&self.root, "FreeText", &format!("{}   ({:.1}%)", self.size(free), pct(free)));
            if let (Ok(track), Ok(bar)) = (find::<FrameworkElement>(&self.root, "UsageTrack"), find::<FrameworkElement>(&self.root, "UsageBar")) {
                let w = track.ActualWidth().unwrap_or(200.0);
                let _ = bar.SetWidth((w * pct(used) / 100.0).max(14.0));
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
                let mut p = crate::tables::Parts { alloc, files: Vec::new(), exts: Vec::new(), colours: HashMap::new(), sorts };
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
        let crate::tables::Parts { alloc, files, exts, colours, sorts } = parts;
        self.filter.clear();
        let path = tree.root_path.clone();
        self.map.scene.clear();
        self.child_stack.clear();
        self.set_tree(crate::tables::Prepared { tree, alloc, files, exts, colours, sorts }, &path);
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
                        self.table_key(k[4..].parse().unwrap_or(0));
                    }
                    // dev: type into the search box
                    q if q.starts_with("find=") => {
                        if let Ok(b) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::TextBox>(&self.root, "SearchBox") {
                            let _ = b.SetText(&q[5..].into());
                        }
                    }
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
                    // dev: a Cleaners button, AdvClO0 (open), AdvClT2 (tick), AdvClS2 (files)
                    a if a.starts_with("AdvCl") && a.ends_with(|c: char| c.is_ascii_digit()) && !a.starts_with("AdvClean") => {
                        let cut = a.trim_end_matches(|c: char| c.is_ascii_digit()).len();
                        self.cleaner_command(&a[..cut], a[cut..].parse().unwrap_or(0));
                    }
                    a if a.starts_with("Adv") => self.advisor_command(a),
                    _ => self.command(&c),
                }
            }
        }
        // Let the finish wave and morphs play out first (about 3 s at 60 frames a second).
        // ...and every SPRAWL_SWITCH step its 30 frames, plus a second for the last one to settle
        let steps = std::env::var("SPRAWL_SWITCH").map_or(0, |v| v.split(',').count() as u32);
        if self.shot_ticks < 180u32.max(steps * 30 + 60) {
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
