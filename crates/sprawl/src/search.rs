//! The Search tab: find files and folders by name in the scan, see them on the map, act on them
//! with the same commands as everywhere else. The index (`sprawl_core::search`) is built on
//! another thread the first time the tab is used after a scan; each keystroke starts a new
//! search on other threads and cancels the one before. The tree, the selection and the map's
//! layout are left as they were.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use sprawl_core::search::{search, Index, Kind, Query};

use crate::format::{fmt_time, thousands};
use crate::state::{find, text, State, View};
use crate::tables::{cell, Col, FILES_LIKE};
use crate::ui;

/// The map colour code of a match (see `advisor::lens_colour`).
pub(crate) const LENS_MATCH: u8 = 20;

/// The row menu's items that need the scan on screen: (item, its tip on the scan's row, its tip
/// on a row from another drive or a saved scan).
pub(crate) const SCAN_ONLY: &[(&str, &str, &str)] = &[
    ("cInTree", "Show it in the tree", "Only a result of the scan on screen is in the tree."),
    ("cMapHere", "Show where it sits on the treemap", "Only a result of the scan on screen is on the treemap."),
    ("cExpandAll", "Expand everything inside it", "Only a result of the scan on screen can be expanded."),
    ("cCopyInfo", "Copy the rows on screen as text", "Copy Name and Size copies the scan on screen's rows."),
];

/// The row menu's "Overwrite and delete": Tree View and File View only, off in Search.
pub(crate) const OVERWRITE_ONLY: &str = "Overwrite and delete works in Tree View and File View.";

/// The most results the row menu opens at once: Explorer starts a program for each result and a
/// window for each folder holding picked files or picked itself.
pub(crate) const MAX_OPEN: usize = 10;

/// Whether Open (or Show in Explorer) may go ahead for `n` results or windows at once.
pub(crate) fn can_open(n: usize) -> bool {
    n <= MAX_OPEN
}

/// Whether the row menu's `item` can act on a Search result: `scan` is true for a row of the scan
/// on screen, and the items that need the scan are off for the rows that come from elsewhere.
pub(crate) fn item_applies(item: &str, scan: bool) -> bool {
    if item == "cOverwrite" {
        return false;
    }
    scan || !SCAN_ONLY.iter().any(|&(name, _, _)| name == item)
}

/// (tree generation, query, sort, matches sorted, the folder ranks it built)
type Found = (u64, String, (usize, bool), Option<Vec<usize>>, Option<Arc<Vec<u32>>>);
/// (tree generation, sort, the matches sorted again, the folder ranks it built)
type Sorted = (u64, (usize, bool), Vec<usize>, Option<Arc<Vec<u32>>>);

/// How long typing pauses before the search runs: short enough to feel immediate, long enough
/// that a word typed quickly doesn't start (and cancel) a search per letter.
const TYPING_PAUSE_MS: u128 = 50;

/// More matches than this are sorted again on a worker (a header click), not on the UI thread.
const SORT_HERE: usize = 20_000;

#[derive(Default)]
pub(crate) struct Search {
    /// What is typed.
    pub(crate) query: String,
    /// The map is shown under the results (the Map button; off at first: results take the room).
    pub(crate) map: bool,
    /// What the no-match message says now ("" hidden; `None` not drawn yet).
    pub(crate) none: Option<String>,
    /// The last tap on a result: (row, when, already taken as a double-click).
    last_tap: Option<((usize, usize), std::time::Instant, bool)>,
    /// The index and the `tree_gen` it was built for.
    index: Option<(u64, Arc<Index>)>,
    pub(crate) building: Option<(u64, Receiver<Index>)>,
    cancel: Arc<AtomicBool>,
    running: Option<Receiver<Found>>,
    /// The search to run once the index is ready.
    pending: bool,
    /// Matches, sorted, for `results_gen`.
    pub(crate) results: Vec<usize>,
    pub(crate) results_gen: u64,
    pub(crate) sort: (usize, bool),
    /// Matches lit on the map.
    pub(crate) lens: Option<Arc<Vec<u8>>>,
    /// Nodes where the watcher saw a change since the scan, and the `tree_gen` they are for.
    changed: (u64, std::collections::HashSet<usize>),
    /// Saved scans and other drives.
    pub(crate) more: crate::search_more::More,
    /// When the box last changed: the search runs once typing pauses.
    typed: Option<std::time::Instant>,
    /// Bumped whenever the scan's matches or the other drives' matches change or are re-sorted.
    pub(crate) stamp: u64,
    /// Every folder's place in path order (`folder_ranks`), for the `tree_gen` it was built for.
    ranks: Option<(u64, Arc<Vec<u32>>)>,
    /// A sort of many matches on a worker.
    pub(crate) sorting: Option<Receiver<Sorted>>,
    /// The two lists merged into the table's rows, for (`stamp`, `tree_gen`).
    merged: std::cell::RefCell<Option<((u64, u64), Vec<(usize, usize)>)>>,
    /// The help and the empty state are made (once, when the tab first opens).
    pub(crate) ui_built: bool,
    /// The filter chips as made (their names are looked up in here).
    pub(crate) chips: Option<crate::bindings::Microsoft::UI::Xaml::FrameworkElement>,
    /// The suggestions under the box and the highlighted one.
    pub(crate) suggest: Option<crate::search_query::Suggestions>,
    pub(crate) highlight: Option<usize>,
    /// Saved and recent searches (read when first needed), and what their menu was made for.
    pub(crate) history: Option<crate::search_query::History>,
    pub(crate) saved_shape: String,
}

pub(crate) const COLS: &[Col] = FILES_LIKE;

impl Search {
    /// The watcher saw a change at node `n` (the nearest part of the tree that still exists).
    pub(crate) fn changed(&mut self, gen: u64, n: usize) {
        if self.changed.0 != gen {
            self.changed = (gen, Default::default());
        }
        if self.changed.1.len() < 100_000 {
            self.changed.1.insert(n);
        }
    }

    /// Node `n`, or the folder it is in, changed since the scan.
    fn is_changed(&self, gen: u64, n: usize, parent: Option<usize>) -> bool {
        self.changed.0 == gen && (self.changed.1.contains(&n) || parent.is_some_and(|p| self.changed.1.contains(&p)))
    }

    /// Biggest first.
    pub(crate) fn new() -> Search {
        Search { sort: (1, true), ..Default::default() }
    }
}

impl State {
    /// The text in the search box changed.
    pub(crate) fn search_changed(&mut self, q: String) {
        if q == self.search.query && self.search.results_gen == self.tree_gen {
            return;
        }
        self.search.query = q;
        self.search.typed = Some(std::time::Instant::now());
        self.search_box_state();
    }

    /// Start a search for what is typed (or wait for the index).
    pub(crate) fn search_run(&mut self) {
        // whatever is running is about something older
        self.search.cancel.store(true, Ordering::Relaxed);
        self.search.running = None;
        self.search.sorting = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.search.cancel = cancel.clone();
        // outside the scan: saved scans and other drives
        self.more_forget_covered();
        self.more_run(cancel.clone());
        let Some(tree) = self.tree.clone() else {
            let m = &self.search.more;
            let note = if m.is_reading() {
                "Reading your drives; results come in as each one is ready."
            } else if !m.has_places() {
                "Scan a folder, or turn on Options > Search Every Drive When Search Opens."
            } else if self.search.query.trim().is_empty() {
                "Type to search every drive."
            } else {
                ""
            };
            let note = if note.is_empty() { self.search_count() } else { note.to_string() };
            self.search_show(Vec::new(), note, false);
            return;
        };
        let q = Query::parse(&self.search.query);
        // a word that can't be understood: nothing is searched (the hint line says why)
        if q.is_empty() || !q.errors().is_empty() {
            self.search.pending = false;
            self.search_show(Vec::new(), String::new(), false);
            return;
        }
        let gen = self.tree_gen;
        let index = match &self.search.index {
            Some((g, ix)) if *g == gen => ix.clone(),
            _ => {
                // build it (once) and search when it is ready
                self.search.pending = true;
                if self.search.building.as_ref().is_none_or(|(g, _)| *g != gen) {
                    self.search.index = None;
                    let (tx, rx) = channel();
                    let t = tree.clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(Index::build(&t));
                    });
                    self.search.building = Some((gen, rx));
                }
                text(&self.root, "SearchInfo", "Getting the names ready...");
                return;
            }
        };
        self.search.pending = false;
        let (tx, rx) = channel();
        let text_q = self.search.query.clone();
        // sorted on the worker too: a million matches sort slowly
        let (c, desc) = self.search.sort;
        let key = COLS[c.min(COLS.len() - 1)].key();
        let alloc = if key == crate::tables::Key::Alloc { self.alloc.clone() } else { Vec::new() };
        let ranks = self.ranks_for(gen);
        std::thread::spawn(move || {
            let mut r = search(&tree, &index, &q, &cancel);
            let mut built = None;
            if let Some(v) = &mut r {
                let ranks = folder_ranks_if(&tree, key, v.len(), ranks, &mut built);
                crate::tables::sort_nodes(&tree, &alloc, key, desc, v, ranks.as_deref().map_or(&[], |r| &r[..]));
            }
            let _ = tx.send((gen, text_q, (c, desc), r, built));
        });
        self.search.running = Some(rx);
    }

    /// Each frame: the index, the results, and a tree that changed under the results.
    /// A search that found nothing says so where the results would be, with what was searched
    /// and the ways to widen it (the query stays in the box).
    fn search_none_state(&mut self) {
        let m = &self.search.more;
        let q = self.search.query.trim();
        let settled = self.search.running.is_none() && self.search.sorting.is_none() && !self.search.pending && self.search.typed.is_none() && !m.is_reading();
        let none = self.view == View::Search
            && !q.is_empty()
            && settled
            && Query::parse(q).errors().is_empty()
            && self.search.results_gen == self.tree_gen
            && self.search.results.is_empty()
            && m.current_hits().is_empty();
        let msg = if none {
            let drives = m.on_list();
            let place = match (self.tree.is_some(), drives.is_empty()) {
                (true, true) => "this scan".to_string(),
                (true, false) => format!("this scan and {}", drives.join(", ")),
                _ => drives.join(", "),
            };
            let mut ways = vec!["check the words", "take a filter off (a lit button under the box, or its words in the box)"];
            if drives.is_empty() {
                ways.push("add a drive with Search in");
            }
            let mut s = format!("Nothing matches \u{201c}{q}\u{201d} in {place}.\nTo widen it: {}.", ways.join("; "));
            if self.full_tree.is_some() {
                s += &format!("\nThe scan is filtered by \u{201c}{}\u{201d}: only what matches it is searched. Empty the Filter box on the Tree tab to search everything.", self.filter);
            }
            s
        } else {
            String::new()
        };
        if self.search.none.as_deref() == Some(msg.as_str()) {
            return;
        }
        text(&self.root, "SearchNoneText", &msg);
        if let Ok(e) = find::<crate::bindings::Microsoft::UI::Xaml::UIElement>(&self.root, "SearchNone") {
            let _ = e.SetVisibility(if msg.is_empty() { crate::bindings::Microsoft::UI::Xaml::Visibility::Collapsed } else { crate::bindings::Microsoft::UI::Xaml::Visibility::Visible });
        }
        self.search.none = Some(msg);
    }

    pub(crate) fn search_poll(&mut self) {
        self.search_none_state();
        if self.search.typed.is_some_and(|t| t.elapsed().as_millis() >= TYPING_PAUSE_MS) {
            self.search.typed = None;
            self.search_run();
        }
        self.more_poll();
        if let Some((g, rx)) = &self.search.building {
            if let Ok(ix) = rx.try_recv() {
                let g = *g;
                self.search.building = None;
                if g == self.tree_gen {
                    self.search.index = Some((g, Arc::new(ix)));
                    if self.search.pending {
                        self.search_run();
                    }
                }
            }
        }
        if let Some(rx) = &self.search.sorting {
            if let Ok((gen, sort, v, built)) = rx.try_recv() {
                self.search.sorting = None;
                self.keep_ranks(gen, built);
                if gen == self.tree_gen && self.search.results_gen == gen && sort == self.search.sort {
                    self.search.results = v;
                    self.search.stamp += 1;
                    // other drives' rows may still be sorting
                    let info = if self.search.more.is_resorting() { "Sorting...".to_string() } else { self.search_count() };
                    text(&self.root, "SearchInfo", &info);
                    if self.view == View::Search {
                        self.refresh_rows();
                    }
                }
            }
        }
        if let Some(rx) = &self.search.running {
            if let Ok((gen, q, sort, r, built)) = rx.try_recv() {
                self.search.running = None;
                self.keep_ranks(gen, built);
                if let Some(found) = r.filter(|_| gen == self.tree_gen && q == self.search.query) {
                    let resort = sort != self.search.sort;
                    self.search_show(found, String::new(), resort);
                }
            }
        }
        // a delete or a branch rescan changed the tree: search it again
        if self.view == View::Search
            && self.search.results_gen != self.tree_gen
            && !self.search.query.trim().is_empty()
            && self.search.running.is_none()
            && !self.search.pending
        {
            self.search_run();
        }
    }

    /// Show `found` (sorted here), or `note` when there is nothing to show.
    fn search_show(&mut self, found: Vec<usize>, note: String, sort: bool) {
        self.search.results = found;
        self.search.results_gen = self.tree_gen;
        if sort {
            self.search_sort();
        }
        let t = self.tree.as_ref();
        self.search.stamp += 1;
        let info = if !note.is_empty() { note } else { self.search_count() };
        text(&self.root, "SearchInfo", &info);
        // the map: matches lit, the rest dimmed
        self.search.lens = match t {
            Some(t) if !self.search.results.is_empty() => {
                let mut v = vec![0u8; t.nodes.len()];
                for &n in &self.search.results {
                    v[n] = LENS_MATCH;
                }
                Some(Arc::new(v))
            }
            _ => None,
        };
        if self.view == View::Search {
            self.refresh_rows();
            self.map.drawn_once = false;
        }
        if crate::bench::on() {
            crate::bench::note(&format!("search results shown ({})", self.search.results.len()));
        }
    }

    pub(crate) fn search_sort(&mut self) {
        let (c, desc) = self.search.sort;
        let key = COLS[c.min(COLS.len() - 1)].key();
        let gen = self.tree_gen;
        let ranks = self.ranks_for(gen);
        if let Some(t) = self.tree.clone() {
            if self.search.results.len() <= SORT_HERE {
                let mut v = std::mem::take(&mut self.search.results);
                crate::tables::sort_nodes(&t, &self.alloc, key, desc, &mut v, ranks.as_deref().map_or(&[], |r| &r[..]));
                self.search.results = v;
            } else {
                // many: on a worker; the rows keep their order until it is done
                let v = self.search.results.clone();
                let alloc = if key == crate::tables::Key::Alloc { self.alloc.clone() } else { Vec::new() };
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    let mut v = v;
                    let mut built = None;
                    let ranks = folder_ranks_if(&t, key, v.len(), ranks, &mut built);
                    crate::tables::sort_nodes(&t, &alloc, key, desc, &mut v, ranks.as_deref().map_or(&[], |r| &r[..]));
                    let _ = tx.send((gen, (c, desc), v, built));
                });
                self.search.sorting = Some(rx);
                text(&self.root, "SearchInfo", "Sorting...");
            }
        }
        self.more_sort();
        self.search.stamp += 1;
    }

    /// The folder ranks of tree `gen`, if they were built.
    fn ranks_for(&self, gen: u64) -> Option<Arc<Vec<u32>>> {
        self.search.ranks.as_ref().filter(|(g, _)| *g == gen).map(|(_, r)| r.clone())
    }

    /// Keep folder ranks a worker built, if they are for the tree on screen.
    fn keep_ranks(&mut self, gen: u64, built: Option<Arc<Vec<u32>>>) {
        if let Some(r) = built.filter(|_| gen == self.tree_gen) {
            self.search.ranks = Some((gen, r));
        }
    }

    /// "12 files (3.1 GB), 4 folders": the scan's matches and the other drives' together.
    pub(crate) fn search_count(&self) -> String {
        if self.search.query.trim().is_empty() {
            return String::new();
        }
        if !Query::parse(&self.search.query).errors().is_empty() {
            return "Nothing searched: a word can't be understood (see below the box)".to_string();
        }
        let (mut files, mut folders, mut bytes) = (0u64, 0u64, 0u64);
        let mut add = |node: &sprawl_core::Node| {
            if node.is_dir {
                folders += 1;
            } else {
                files += 1;
                bytes += node.size;
            }
        };
        if let Some(t) = self.tree.as_ref().filter(|_| self.search.results_gen == self.tree_gen) {
            for &n in &self.search.results {
                add(&t.nodes[n]);
            }
        }
        let m = &self.search.more;
        let others = m.current_hits();
        for &(k, n) in others {
            if let Some((ix, i)) = m.item(k, n) {
                if ix.is_dir(i) {
                    folders += 1;
                } else {
                    files += 1;
                    bytes += ix.size(i);
                }
            }
        }
        let mut s = match (files, folders) {
            (0, 0) if m.is_reading() => "Nothing yet; still reading drives".to_string(),
            (0, 0) => "Nothing found".to_string(),
            (f, 0) => format!("{} file{}, {}", thousands(f), if f == 1 { "" } else { "s" }, self.size(bytes)),
            (0, d) => format!("{} folder{}", thousands(d), if d == 1 { "" } else { "s" }),
            (f, d) => format!("{} file{} ({}), {} folder{}", thousands(f), if f == 1 { "" } else { "s" }, self.size(bytes), thousands(d), if d == 1 { "" } else { "s" }),
        };
        if !others.is_empty() && m.total > others.len() {
            s += &format!("; the biggest {} of {} on other drives", thousands(others.len() as u64), thousands(m.total as u64));
        }
        if self.full_tree.is_some() {
            s += "; the scan's part is the filtered view";
        }
        s
    }

    /// The rows of the Search tab (none while the tree has changed under the results).
    /// The scan's matches are `(node, 0)`; another drive's are `(node, place + 1)`.
    pub(crate) fn search_rows(&self) -> Vec<(usize, usize)> {
        let at = (self.search.stamp, self.tree_gen);
        if let Some((s, rows)) = self.search.merged.borrow().as_ref() {
            if *s == at {
                return rows.clone();
            }
        }
        let len = self.tree.as_ref().map_or(0, |t| t.nodes.len());
        let mine: &[usize] = if self.search.results_gen == self.tree_gen { &self.search.results } else { &[] };
        let mine: Vec<usize> = mine.iter().copied().filter(|&n| n < len).collect();
        let m = &self.search.more;
        let others = m.current_hits();
        let mut rows = Vec::with_capacity(mine.len() + others.len());
        match &self.tree {
            Some(t) if !others.is_empty() && !mine.is_empty() => {
                let (c, desc) = self.search.sort;
                let by = crate::search_more::sort_by(c);
                let (mut i, mut j) = (0, 0);
                // by folder: both lists are in folder order, so a folder's path is built once
                // for its run of rows, not for every comparison
                let mut mine_dir: (Option<usize>, String) = (None, String::new());
                let mut other_dir: (Option<(usize, u32)>, String) = (None, String::new());
                while i < mine.len() && j < others.len() {
                    let (k, n) = others[j];
                    let Some((ix, id)) = m.item(k, n) else {
                        j += 1;
                        continue;
                    };
                    let a = &t.nodes[mine[i]];
                    let o = if by == sprawl_core::compact::SortBy::Folder {
                        if mine_dir.0 != a.parent || mine_dir.1.is_empty() {
                            mine_dir = (a.parent, a.parent.map(|p| t.path_of(p).to_string_lossy().into_owned()).unwrap_or_default());
                        }
                        let here = ix.parent(id).map(|p| (k, p));
                        if other_dir.0 != here || other_dir.1.is_empty() {
                            other_dir = (here, ix.parent(id).map(|p| ix.path_of(p).to_string_lossy().into_owned()).unwrap_or_default());
                        }
                        // the one comparison both sorts use
                        crate::tables::cmp_names(&mine_dir.1, &other_dir.1).then_with(|| crate::tables::cmp_names(&a.name, &ix.name(id)))
                    } else if by == sprawl_core::compact::SortBy::Name {
                        // folders first (the same key the lists were sorted by), then the name
                        let group = |dir| crate::search_more::name_group(dir, desc);
                        group(a.is_dir).cmp(&group(ix.is_dir(id))).then_with(|| crate::tables::cmp_names(&a.name, &ix.name(id)))
                    } else {
                        crate::search_more::scan_val(t, mine[i], by, desc).cmp(&crate::search_more::compact_val(ix, id, by, desc))
                    };
                    let mine_first = if desc { o != std::cmp::Ordering::Less } else { o != std::cmp::Ordering::Greater };
                    if mine_first {
                        rows.push((mine[i], 0));
                        i += 1;
                    } else {
                        rows.push((n, k + 1));
                        j += 1;
                    }
                }
                rows.extend(mine[i..].iter().map(|&n| (n, 0)));
                rows.extend(others[j..].iter().filter(|&&(k, n)| m.item(k, n).is_some()).map(|&(k, n)| (n, k + 1)));
            }
            _ => {
                rows.extend(mine.iter().map(|&n| (n, 0)));
                rows.extend(others.iter().filter(|&&(k, n)| m.item(k, n).is_some()).map(|&(k, n)| (n, k + 1)));
            }
        }
        *self.search.merged.borrow_mut() = Some((at, rows.clone()));
        rows
    }

    /// The matched part of `name` in bold.
    pub(crate) fn name_runs(&self, name: &str) -> String {
        let q = Query::parse(&self.search.query);
        // (a cut inside a character would panic on the window's thread: plain text instead)
        match q.highlight(name).filter(|&(a, b)| a <= b && name.is_char_boundary(a) && name.is_char_boundary(b)) {
            Some((a, b)) => format!(
                r##"<Run Text="{}"/><Run Text="{}" FontWeight="Bold" Foreground="{{$match}}"/><Run Text="{}"/>"##,
                ui::esc(&name[..a]),
                ui::esc(&name[a..b]),
                ui::esc(&name[b..])
            ),
            None => format!(r##"<Run Text="{}"/>"##, ui::esc(name)),
        }
    }

    pub(crate) fn search_row(&self, k: usize) -> String {
        let (n, d) = self.rows[k];
        if d > 0 {
            return self.more_row(k, d - 1, n);
        }
        let t = self.tree.as_ref().unwrap();
        let node = &t.nodes[n];
        let sel = Some(n) == self.selected || self.marked.contains(&n);
        let (bg, sel_bar) = (self.row_bg(k, sel), crate::tables::row_bar(sel));
        let modified = node.modified.map(fmt_time).unwrap_or_default();
        let folder = node.parent.map(|p| t.path_of(p).display().to_string()).unwrap_or_default();
        let (glyph, gc) = if node.is_dir { ("&#xE8B7;", crate::tokens::refer("folder")) } else { ("&#xE7C3;", self.ext_colour_hex(&node.extension())) };
        let runs = self.name_runs(&node.name);
        let full = ui::esc(&t.path_of(n).display().to_string());
        let changed = if self.search.is_changed(self.tree_gen, n, node.parent) {
            r##"<FontIcon Glyph="&#xE895;" FontSize="{$fs11}" Foreground="{$warn}" ToolTipService.ToolTip="Changed since the scan: check it is still there"/>"##
        } else {
            ""
        };
        format!(
            r##"<Grid Height="24" Background="{bg}">{cols}{sel_bar}<StackPanel Orientation="Horizontal" Spacing="8" Margin="10,0,0,0" ToolTipService.ToolTip="{full}"><FontIcon Glyph="{glyph}" FontSize="{{$fs13}}" Foreground="{gc}"/><TextBlock VerticalAlignment="Center" FontSize="{{$fs12}}" TextTrimming="CharacterEllipsis">{runs}</TextBlock>{changed}</StackPanel>{c1}{c2}{c3}<TextBlock Grid.Column="4" Text="{short}" FontSize="{{$fs12}}" Foreground="{{$text3}}" VerticalAlignment="Center" Margin="10,0" TextTrimming="CharacterEllipsis" ToolTipService.ToolTip="{folder}"/></Grid>"##,
            short = ui::esc(&crate::format::mid_path(&folder, self.tables.folder_chars)),
            cols = crate::tables::col_defs(COLS),
            c1 = cell(1, &self.size(node.size)),
            c2 = cell(2, &self.size(self.alloc[n])),
            c3 = cell(3, &modified),
            folder = ui::esc(&folder),
        )
    }

    /// Esc in the results: back to the search box.
    pub(crate) fn search_box_focus(&mut self) {
        self.search_ui_build();
        self.search_box_take_focus();
    }

    /// The focus to the Search box, now if it can take it, else on the next frames
    /// ([`State::search_focus_frame`]).
    pub(crate) fn search_box_take_focus(&mut self) {
        if self.search_box_focus_now() {
            crate::bench::note("search box focused");
        }
        // watched for about half a second: the box may not be laid out yet, and the tab button
        // a click came from may take the focus back after its Click
        self.search_focus_wait = 30;
    }

    /// Whether the Search box has the focus after asking for it (false while it is hidden or
    /// not yet laid out).
    fn search_box_focus_now(&self) -> bool {
        use crate::bindings::Microsoft::UI::Xaml::{Controls::TextBox, FocusState};
        let Ok(b) = find::<TextBox>(&self.root, "SearchBox") else { return false };
        b.Focus(FocusState::Programmatic).unwrap_or(false)
    }

    /// Each frame, for a moment after the Search tab opened: the Search box takes the focus if
    /// nothing has it yet or a tab button still does (never from anything the user went to).
    pub(crate) fn search_focus_frame(&mut self) {
        if self.search_focus_wait == 0 {
            return;
        }
        if self.view != View::Search {
            self.search_focus_wait = 0;
            return;
        }
        self.search_focus_wait -= 1;
        use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement};
        use windows_core::Interface;
        let Ok(root) = self.root.cast::<UIElement>() else { return };
        let name = crate::menu::focused(&root).and_then(|f| f.cast::<FrameworkElement>().ok()).and_then(|f| f.Name().ok()).map(|n| n.to_string_lossy());
        // a menu title holds it when the window first opens (WinUI's first focus), as long as no
        // menu is open
        let free = match name.as_deref() {
            None => true,
            Some(n) => n.starts_with("Tab") || n.starts_with("Menu") && !crate::menu::any_open(),
        };
        if free && self.search_box_focus_now() {
            crate::bench::note(&format!("search box focused (from {})", name.as_deref().unwrap_or("nothing")));
        }
    }

    /// Ctrl+F: the Search tab, its box focused.
    pub(crate) fn search_focus(&mut self) {
        if self.view != View::Search {
            self.set_view(View::Search);
        }
        self.search_ui_build();
        self.search_box_state();
        self.more_open();
        self.search_box_take_focus();
    }

    /// Enter or a double-click on a result: a file opens with its program, a folder in
    /// Explorer, if it is still there.
    pub(crate) fn search_open(&mut self, path: &std::path::Path) {
        let Ok(m) = std::fs::metadata(path) else {
            text(&self.root, "StatusText", &format!("{} isn't there any more. Scan again to update the results.", path.display()));
            return;
        };
        self.search_note();
        let explorer = std::env::var_os("SystemRoot").map(|w| std::path::PathBuf::from(w).join("explorer.exe")).unwrap_or_else(|| "explorer.exe".into());
        // Explorer opens a folder in a window and a file with its default program
        let _ = std::process::Command::new(explorer).arg(path).spawn();
        text(&self.root, "StatusText", &format!("{} {}", if m.is_dir() { "Opened" } else { "Opening" }, path.display()));
    }

    /// Whether this tap on `row` is a double-click. WinUI's own double-tap is lost when the
    /// results are redrawn between the two clicks (other drives arriving every half second), so
    /// two taps on the same row within the double-click time count too; `gesture` is WinUI's,
    /// which is not counted again when the second tap already was.
    pub(crate) fn tap_is_double(&mut self, row: (usize, usize), gesture: bool) -> bool {
        let window = std::time::Duration::from_millis(crate::win32::double_click_ms());
        let (double, last) = second_tap(self.search.last_tap, row, std::time::Instant::now(), window, gesture);
        self.search.last_tap = Some(last);
        double
    }

    /// Ctrl+Enter or "Show in the Tree" on a result: show it in the tree.
    pub(crate) fn search_reveal(&mut self, n: usize) {
        self.search_note();
        self.set_view(View::Tree);
        self.reveal(n);
    }

    /// The full paths of the results the row menu acts on, in the order shown: the scan's picked
    /// rows, or the rows from another drive or a saved scan that are picked (or were right-clicked).
    /// Elsewhere, the selected row.
    pub(crate) fn menu_paths(&self) -> Vec<std::path::PathBuf> {
        if self.view != View::Search {
            return self.selected_path().into_iter().collect();
        }
        let more = &self.search.more;
        if more.sel.is_none() && more.picks.is_empty() {
            let t = self.tree.as_ref();
            return picked_in_order(&self.rows, &self.marked, self.selected).iter().filter_map(|&n| t.map(|t| t.path_of(n))).collect();
        }
        self.rows
            .iter()
            .filter_map(|&(n, d)| {
                if d == 0 {
                    return None;
                }
                let (k, id) = (d - 1, n);
                let (ix, i) = more.item(k, id)?;
                let p = ix.path_of(i);
                (more.sel == Some((k, id)) || more.picks.contains_key(&crate::search_delete::key(&p))).then_some(p)
            })
            .collect()
    }

    /// "Use as filter": the words go to the filter box (which narrows the whole analysis).
    pub(crate) fn search_to_filter(&mut self) {
        let q = Query::parse(&self.search.query);
        let words: Vec<&str> = self.search.query.split_whitespace().filter(|w| !w.starts_with(['>', '<', '!']) && !w.contains(':')).collect();
        if words.is_empty() || q.kind == Kind::Folders {
            text(&self.root, "SearchInfo", "Only names and patterns can become a filter (not sizes, ! or folder:).");
            return;
        }
        // a search's space means "and", which a filter can't say
        if words.len() > 1 {
            text(&self.root, "SearchInfo", "A filter takes one pattern (or a|b); it can't say \"this and that\".");
            return;
        }
        let f = words[0].to_string();
        if let Ok(b) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::TextBox>(&self.root, "FilterBox") {
            let _ = b.SetText(&f.into());
        }
        self.set_view(View::Files);
    }
}

type Tap = ((usize, usize), std::time::Instant, bool);

/// Whether a tap on `row` at `now` is the second of a double-click, and the tap to remember.
fn second_tap(last: Option<Tap>, row: (usize, usize), now: std::time::Instant, window: std::time::Duration, gesture: bool) -> (bool, Tap) {
    let recent = last.filter(|(r, t, _)| *r == row && now.duration_since(*t) <= window);
    let taken = recent.is_some_and(|(_, _, taken)| taken);
    let double = if gesture { !taken } else { recent.is_some() && !taken };
    (double, (row, now, double || (gesture && taken)))
}

#[cfg(test)]
mod tap_tests {
    use super::second_tap;
    use std::time::{Duration, Instant};

    #[test]
    fn two_taps_on_a_row_are_one_double_click() {
        let w = Duration::from_millis(500);
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        // WinUI lost the gesture: two plain taps
        let (d, last) = second_tap(None, (3, 0), t0, w, false);
        assert!(!d);
        let (d, last) = second_tap(Some(last), (3, 0), ms(200), w, false);
        assert!(d);
        // a third tap starts over, not a second double-click
        let (d, _) = second_tap(Some(last), (3, 0), ms(300), w, false);
        assert!(!d);
        // WinUI's own double-tap after one tap: once
        let (_, last) = second_tap(None, (3, 0), t0, w, false);
        let (d, last) = second_tap(Some(last), (3, 0), ms(200), w, true);
        assert!(d);
        let (d, _) = second_tap(Some(last), (3, 0), ms(210), w, true);
        assert!(!d, "the same double-click is not taken twice");
        // another row, or too slow: not a double-click
        let (_, last) = second_tap(None, (3, 0), t0, w, false);
        assert!(!second_tap(Some(last), (4, 0), ms(100), w, false).0);
        assert!(!second_tap(Some(last), (3, 0), ms(900), w, false).0);
    }
}

/// The folder ranks to sort `rows` matches by `key`: the ones given, or (by folder, for many
/// matches) built now and put in `built` to keep. Few matches don't need them.
fn folder_ranks_if(tree: &sprawl_core::Tree, key: crate::tables::Key, rows: usize, given: Option<Arc<Vec<u32>>>, built: &mut Option<Arc<Vec<u32>>>) -> Option<Arc<Vec<u32>>> {
    if key != crate::tables::Key::Folder || given.is_some() || rows <= SORT_HERE {
        return given;
    }
    let r = Arc::new(sprawl_core::search::folder_ranks(tree));
    *built = Some(r.clone());
    Some(r)
}

/// The scan's rows that are picked (`marked` and `selected`), in the order shown. A row from
/// another drive or a saved scan is never one of them, whatever its number.
pub(crate) fn picked_in_order(rows: &[(usize, usize)], marked: &std::collections::BTreeSet<usize>, selected: Option<usize>) -> Vec<usize> {
    let mut seen = std::collections::BTreeSet::new();
    rows.iter().filter(|r| r.1 == 0 && (marked.contains(&r.0) || selected == Some(r.0)) && seen.insert(r.0)).map(|r| r.0).collect()
}

#[cfg(test)]
mod picked_tests {
    use super::picked_in_order;

    #[test]
    fn only_the_scans_rows_in_the_order_shown() {
        // node 5 of the scan and item 5 of another drive (place 0) are different things
        let rows = [(9, 0), (5, 1), (5, 0), (2, 0), (7, 2)];
        let marked = [2, 5, 7].into_iter().collect();
        assert_eq!(picked_in_order(&rows, &marked, Some(9)), [9, 5, 2]);
        assert_eq!(picked_in_order(&rows, &Default::default(), Some(7)), Vec::<usize>::new(), "7 is only another drive's row");
        assert_eq!(picked_in_order(&rows, &Default::default(), None), Vec::<usize>::new());
    }
}

#[cfg(test)]
mod menu_tests {
    use super::{can_open, item_applies, MAX_OPEN, SCAN_ONLY};

    /// Open starts a program or an Explorer window for each result, so only up to the limit at
    /// once; one more is refused.
    #[test]
    fn open_goes_up_to_the_limit_and_no_further() {
        assert!(can_open(0) && can_open(1) && can_open(MAX_OPEN));
        assert!(!can_open(MAX_OPEN + 1));
        assert!(!can_open(MAX_OPEN * 2));
    }

    /// The items that go by path stay on for a row from another drive or a saved scan; the ones
    /// that need the scan on screen are off there, and every one of them says why.
    #[test]
    fn what_the_row_menu_can_do_with_another_drives_row() {
        for &(name, on, off) in SCAN_ONLY {
            assert!(item_applies(name, true), "{name}");
            assert!(!item_applies(name, false), "{name}");
            assert!(!on.is_empty() && !off.is_empty(), "{name}");
        }
        for name in ["cOpen", "cExplore", "cCmd", "cCopyPath", "cDelete", "cDeleteForever", "cShell", "cScan"] {
            assert!(item_applies(name, true), "{name}");
            assert!(item_applies(name, false), "{name}");
        }
        assert!(!item_applies("cOverwrite", true) && !item_applies("cOverwrite", false), "Search offers no overwrite");
    }

    /// Every row of the right-click menu is in one list or the other: a new item has to choose
    /// whether it works by path or needs the scan on screen.
    #[test]
    fn every_row_menu_item_is_known_here() {
        for &(item, _) in crate::ROW_MENU {
            match item {
                "cOverwrite" => assert!(!item_applies(item, true)),
                "cOpen" | "cExplore" | "cCmd" | "cCopyPath" | "cDelete" | "cDeleteForever" | "cShell" | "cScan" => assert!(item_applies(item, false), "{item}"),
                _ => assert!(SCAN_ONLY.iter().any(|&(n, _, _)| n == item), "{item} is neither by path nor listed as scan-only"),
            }
        }
    }
}
