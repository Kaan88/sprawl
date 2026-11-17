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

type Found = (u64, String, Option<Vec<usize>>);

#[derive(Default)]
pub(crate) struct Search {
    /// What is typed.
    pub(crate) query: String,
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
        self.search_run();
    }

    /// Start a search for what is typed (or wait for the index).
    pub(crate) fn search_run(&mut self) {
        // whatever is running is about something older
        self.search.cancel.store(true, Ordering::Relaxed);
        self.search.running = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.search.cancel = cancel.clone();
        // outside the scan: saved scans and other drives
        self.more_forget_covered();
        self.more_run(cancel.clone());
        let Some(tree) = self.tree.clone() else {
            let note = if self.search.more.places.is_empty() && self.search.more.loading.is_empty() { "Scan a folder first, or run Sprawl as administrator to search every drive." } else { "No scan yet: searching your drives and saved scans." };
            self.search_show(Vec::new(), note.into());
            return;
        };
        let q = Query::parse(&self.search.query);
        if q.is_empty() {
            self.search.pending = false;
            self.search_show(Vec::new(), String::new());
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
        std::thread::spawn(move || {
            let r = search(&tree, &index, &q, &cancel);
            let _ = tx.send((gen, text_q, r));
        });
        self.search.running = Some(rx);
    }

    /// Each frame: the index, the results, and a tree that changed under the results.
    pub(crate) fn search_poll(&mut self) {
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
        if let Some(rx) = &self.search.running {
            if let Ok((gen, q, r)) = rx.try_recv() {
                self.search.running = None;
                if let Some(found) = r.filter(|_| gen == self.tree_gen && q == self.search.query) {
                    self.search_show(found, String::new());
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
    fn search_show(&mut self, found: Vec<usize>, note: String) {
        self.search.results = found;
        self.search.results_gen = self.tree_gen;
        self.search_sort();
        let t = self.tree.as_ref();
        let info = if !note.is_empty() {
            note
        } else if self.search.query.trim().is_empty() {
            String::new()
        } else if let Some(t) = t {
            let (mut files, mut folders, mut bytes) = (0u64, 0u64, 0u64);
            for &n in &self.search.results {
                if t.nodes[n].is_dir {
                    folders += 1;
                } else {
                    files += 1;
                    bytes += t.nodes[n].size;
                }
            }
            let mut s = match (files, folders) {
                (0, 0) => "Nothing found".to_string(),
                (f, 0) => format!("{} file{}, {}", thousands(f), if f == 1 { "" } else { "s" }, self.size(bytes)),
                (0, d) => format!("{} folder{}", thousands(d), if d == 1 { "" } else { "s" }),
                (f, d) => format!("{} file{} ({}), {} folder{}", thousands(f), if f == 1 { "" } else { "s" }, self.size(bytes), thousands(d), if d == 1 { "" } else { "s" }),
            };
            if self.full_tree.is_some() {
                s += " in the filtered view";
            }
            s
        } else {
            String::new()
        };
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
    }

    pub(crate) fn search_sort(&mut self) {
        let (c, desc) = self.search.sort;
        let Some(t) = &self.tree else { return };
        let mut v = std::mem::take(&mut self.search.results);
        crate::tables::sort_nodes(t, &self.alloc, COLS[c.min(COLS.len() - 1)].key(), desc, &mut v);
        self.search.results = v;
    }

    /// The rows of the Search tab (none while the tree has changed under the results).
    pub(crate) fn search_rows(&self) -> Vec<(usize, usize)> {
        let len = self.tree.as_ref().map_or(0, |t| t.nodes.len());
        if self.search.results_gen != self.tree_gen {
            return Vec::new();
        }
        self.search.results.iter().filter(|&&n| n < len).map(|&n| (n, 0)).collect()
    }

    pub(crate) fn search_row(&self, k: usize) -> String {
        let t = self.tree.as_ref().unwrap();
        let n = self.rows[k].0;
        let node = &t.nodes[n];
        let bg = self.row_bg(k, Some(n) == self.selected || self.marked.contains(&n));
        let modified = node.modified.map(fmt_time).unwrap_or_default();
        let folder = node.parent.map(|p| t.path_of(p).display().to_string()).unwrap_or_default();
        let (glyph, gc) = if node.is_dir { ("&#xE8B7;", "#FFFFC83D".to_string()) } else { ("&#xE7C3;", self.ext_colour_hex(&node.extension())) };
        // the matched part in bold
        let q = Query::parse(&self.search.query);
        let name = &node.name;
        let runs = match q.highlight(name) {
            Some((a, b)) => format!(
                r##"<Run Text="{}"/><Run Text="{}" FontWeight="Bold" Foreground="#FF7FD4FF"/><Run Text="{}"/>"##,
                ui::esc(&name[..a]),
                ui::esc(&name[a..b]),
                ui::esc(&name[b..])
            ),
            None => format!(r##"<Run Text="{}"/>"##, ui::esc(name)),
        };
        let changed = if self.search.is_changed(self.tree_gen, n, node.parent) {
            r##"<FontIcon Glyph="&#xE895;" FontSize="11" Foreground="#FFFFB547" ToolTipService.ToolTip="Changed since the scan: check it is still there"/>"##
        } else {
            ""
        };
        format!(
            r##"<Grid Height="24" Background="{bg}">{cols}<StackPanel Orientation="Horizontal" Spacing="8" Margin="10,0,0,0"><FontIcon Glyph="{glyph}" FontSize="13" Foreground="{gc}"/><TextBlock VerticalAlignment="Center" FontSize="12" TextTrimming="CharacterEllipsis">{runs}</TextBlock>{changed}</StackPanel>{c1}{c2}{c3}<TextBlock Grid.Column="4" Text="{folder}" FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center" Margin="10,0" TextTrimming="CharacterEllipsis"/></Grid>"##,
            cols = crate::tables::col_defs(COLS),
            c1 = cell(1, &self.size(node.size)),
            c2 = cell(2, &self.size(self.alloc[n])),
            c3 = cell(3, &modified),
            folder = ui::esc(&folder),
        )
    }

    /// Ctrl+F: the Search tab, its box focused.
    pub(crate) fn search_focus(&mut self) {
        if self.view != View::Search {
            self.set_view(View::Search);
        }
        self.more_open();
        if let Ok(b) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::TextBox>(&self.root, "SearchBox") {
            let _ = b.Focus(crate::bindings::Microsoft::UI::Xaml::FocusState::Programmatic);
        }
    }

    /// Enter or a double-click on a result: show it in the tree.
    pub(crate) fn search_reveal(&mut self, n: usize) {
        self.set_view(View::Tree);
        self.reveal(n);
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
