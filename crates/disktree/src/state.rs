//! The window's state: what was scanned, what is shown, and the helpers every part uses.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use disktree_core::{extension_stats, format_size, largest_files, Tree, ROOT};
use windows_core::{Interface, HSTRING};

use crate::bindings::Microsoft::UI::Dispatching::DispatcherQueueTimer;
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::RangeBase;
use crate::bindings::Microsoft::UI::Xaml::Controls::{ProgressBar, TextBlock};
use crate::bindings::Microsoft::UI::Xaml::Markup::XamlReader;
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, Window};
use crate::format::thousands;
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
    About,
}

pub(crate) struct ExtRow {
    pub(crate) ext: String,
    pub(crate) size: u64,
    pub(crate) alloc: u64,
    pub(crate) files: u64,
    pub(crate) percent: f64,
}

pub(crate) struct Job {
    pub(crate) path: PathBuf,
    pub(crate) count: Arc<AtomicU64>,
    pub(crate) result: Arc<Mutex<Option<std::io::Result<Tree>>>>,
    pub(crate) started: Instant,
}

pub(crate) const ROW_H: f64 = 24.0;
/// Border thickness plus padding of `MapBorder` in ui.rs, per side.
pub(crate) const MAP_INSET: f64 = 3.0;
pub(crate) const MAX_ROWS: usize = 900;

pub(crate) struct State {
    pub(crate) window: Window,
    pub(crate) hwnd: isize,
    pub(crate) root: FrameworkElement,
    pub(crate) drives: Vec<String>,
    pub(crate) tree: Option<Tree>,
    pub(crate) alloc: Vec<u64>,
    pub(crate) expanded: HashSet<usize>,
    pub(crate) selected: Option<usize>,
    pub(crate) rows: Vec<(usize, usize)>, // (node, depth)
    pub(crate) files: Vec<usize>,
    pub(crate) exts: Vec<ExtRow>,
    pub(crate) colours: HashMap<String, usize>,
    pub(crate) ext_selected: Option<usize>,
    pub(crate) map_root: usize,
    pub(crate) layout: Option<paint::Layout>,
    pub(crate) map_px: (usize, usize),
    pub(crate) map_pixels: Vec<u8>,
    pub(crate) hover: Option<usize>,
    pub(crate) units: Units,
    pub(crate) view: View,
    pub(crate) job: Option<Job>,
    pub(crate) scan_secs: f64,
    pub(crate) shot: Option<PathBuf>,
    pub(crate) shot_ticks: u32,
    /// The poll timer; it stops when dropped.
    pub(crate) _timer: DispatcherQueueTimer,
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
        let f = |d: f64, u: &str| format!("{:.1} {u}", v as f64 / d);
        match self.units {
            Units::Dynamic => format_size(v),
            Units::Bytes => thousands(v),
            Units::KB => f(1024.0, "KB"),
            Units::MB => f(1024.0 * 1024.0, "MB"),
            Units::GB => f(1024.0 * 1024.0 * 1024.0, "GB"),
            Units::TB => f(1024.0f64.powi(4), "TB"),
        }
    }

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
        if self.job.is_some() {
            return;
        }
        let count = Arc::new(AtomicU64::new(0));
        let result = Arc::new(Mutex::new(None));
        let (c2, r2, p2) = (count.clone(), result.clone(), path.clone());
        std::thread::spawn(move || {
            let tree = disktree_core::scan(&p2, |n| c2.store(n, Ordering::Relaxed));
            *r2.lock().unwrap() = Some(tree);
        });
        self.job = Some(Job { path: path.clone(), count, result, started: Instant::now() });
        text(&self.root, "StatusText", &format!("Scanning {} ...", path.display()));
        if let Ok(p) = find::<ProgressBar>(&self.root, "Progress") {
            let _ = p.SetIsIndeterminate(true);
        }
        let _ = self.window.SetTitle(&HSTRING::from(format!("[{}] - DiskTree", path.display())));
        text(&self.root, "TitleText", &format!("[{}]  -  DiskTree", path.display()));
    }

    pub(crate) fn poll(&mut self) {
        let Some(job) = &self.job else { return };
        let done = job.result.lock().unwrap().take();
        match done {
            None => {
                let n = job.count.load(Ordering::Relaxed);
                text(&self.root, "StatusText", &format!("Scanning... {} items ({:.1} s)", thousands(n), job.started.elapsed().as_secs_f64()));
            }
            Some(res) => {
                let job = self.job.take().unwrap();
                self.scan_secs = job.started.elapsed().as_secs_f64();
                if let Ok(p) = find::<ProgressBar>(&self.root, "Progress") {
                    let _ = p.SetIsIndeterminate(false);
                    let _ = p.cast::<RangeBase>().and_then(|r| r.SetValue(100.0));
                }
                match res {
                    Ok(tree) => self.set_tree(tree, &job.path),
                    Err(e) => text(&self.root, "StatusText", &format!("Scan failed: {e}")),
                }
            }
        }
    }

    pub(crate) fn set_tree(&mut self, tree: Tree, path: &std::path::Path) {
        // Allocated size: each file rounded up to a 4 KB cluster, folders add up their contents.
        let mut alloc: Vec<u64> = tree.nodes.iter().map(|n| if n.is_dir { 0 } else { n.size.div_ceil(4096) * 4096 }).collect();
        for i in (1..tree.nodes.len()).rev() {
            if let Some(p) = tree.nodes[i].parent {
                alloc[p] += alloc[i];
            }
        }
        self.alloc = alloc;
        self.tree = Some(tree);
        self.expanded = HashSet::from([ROOT]);
        self.selected = Some(ROOT);
        self.map_root = ROOT;
        self.ext_selected = None;
        self.rebuild_exts();
        let t = self.tree.as_ref().unwrap();
        self.files = largest_files(t, 1000);
        text(&self.root, "StatusText", &format!("Scan complete in {:.2} seconds", self.scan_secs));
        text(
            &self.root,
            "ScanInfo",
            &format!("{} files, {} folders, {} unreadable", thousands(t.nodes[ROOT].files), thousands(t.nodes[ROOT].folders), thousands(t.errors)),
        );
        self.update_header(path);
        self.refresh_rows();
        self.refresh_exts();
        self.render_map();
    }

    pub(crate) fn rebuild_exts(&mut self) {
        let t = self.tree.as_ref().unwrap();
        let mut alloc_by: HashMap<String, u64> = HashMap::new();
        for (i, n) in t.nodes.iter().enumerate() {
            if !n.is_dir {
                *alloc_by.entry(n.extension()).or_default() += self.alloc[i];
            }
        }
        self.exts = extension_stats(t)
            .into_iter()
            .map(|s| ExtRow { alloc: alloc_by.get(&s.extension).copied().unwrap_or(0), ext: s.extension, size: s.size, files: s.files, percent: s.percent })
            .collect();
        self.colours = self.exts.iter().take(paint::PALETTE.len()).enumerate().map(|(i, e)| (e.ext.clone(), i)).collect();
    }

    pub(crate) fn update_header(&self, path: &std::path::Path) {
        text(&self.root, "SelText", &path.display().to_string());
        if let Some((total, free)) = win32::disk_space(path) {
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

    // ------------------------------------------------------------ screenshot mode

    pub(crate) fn shot_tick(&mut self) {
        let Some(path) = self.shot.clone() else { return };
        if self.job.is_some() {
            return;
        }
        self.shot_ticks += 1;
        if self.shot_ticks == 20 {
            self.render_map();
        }
        if self.shot_ticks < 120 {
            return;
        }
        if let Some((w, h, px)) = win32::capture(self.hwnd) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, paint::png(w, h, &px));
            std::process::exit(0);
        }
        if self.shot_ticks > 600 {
            eprintln!("could not capture the window");
            std::process::exit(3);
        }
    }
}
