//! The Search tab beyond the scan: every other fixed drive, read when the tab opens (from the
//! MFT as administrator, else folder by folder), and saved scans of places no drive read covers.
//! Their matches sit in the same list as the scan's. They are not part of the scan on screen,
//! so they offer Show in Explorer, Copy path and "Scan this folder" — never a delete, which only
//! goes through a scanned tree.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use sprawl_core::search::{search, Index, Query};
use sprawl_core::Tree;

use crate::bindings::Microsoft::UI::Xaml::Controls::Border;
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::ButtonBase;
use crate::bindings::Microsoft::UI::Xaml::{UIElement, Visibility};
use crate::format::{fmt_time, thousands};
use crate::state::{find, keep, load, with, State, View};
use crate::ui;

/// Matches from other drives kept at most (the first ones in the sort order).
const MAX_HITS: usize = 100_000;
/// Saved scans loaded at most (each costs about what its scan cost in memory).
const MAX_SNAPSHOTS: usize = 3;

/// A tree that isn't the scan on screen.
pub(crate) struct Place {
    pub(crate) tree: Arc<Tree>,
    index: Arc<Index>,
    /// From a saved scan (its date), or read just now.
    pub(crate) as_of: Option<SystemTime>,
}

type Hits = (u64, String, (usize, bool), Option<(Vec<(usize, usize)>, usize)>);

#[derive(Default)]
pub(crate) struct More {
    pub(crate) places: Vec<Arc<Place>>,
    /// Bumped when `places` changes.
    gen: u64,
    snapshots_tried: bool,
    arriving: Option<Receiver<Option<Place>>>,
    /// What is being read ("C:\ (saved 12 Nov)", "D:\"), with a drive read's progress.
    pub(crate) loading: Vec<(String, Option<Arc<sprawl_core::Progress>>)>,
    drives_on: HashSet<String>,
    /// Drives the user switched off this session: not read again by themselves.
    drives_off: HashSet<String>,
    running: Option<Receiver<Hits>>,
    /// (place, node) in the Search sort order, at most MAX_HITS; and how many there were.
    hits: Vec<(usize, usize)>,
    pub(crate) total: usize,
    hits_gen: u64,
    /// The row from another drive that is picked (place, node).
    pub(crate) sel: Option<(usize, usize)>,
    /// When the line under the results last showed the reading progress.
    drawn: Option<Instant>,
    /// What the indexing note in the main window says now ("" hidden).
    busy_shown: String,
}

impl More {
    /// The matches for the places as they are now (none while they changed under them).
    pub(crate) fn current_hits(&self) -> &[(usize, usize)] {
        if self.hits_gen == self.gen {
            &self.hits
        } else {
            &[]
        }
    }
}

/// "D:\ 1,234,567 items", or "C:\ (saved 12 Nov)".
fn loading_label((l, p): &(String, Option<Arc<sprawl_core::Progress>>)) -> String {
    match p {
        Some(p) if p.items() > 0 => format!("{l} {} items", thousands(p.items())),
        Some(_) => format!("{l} starting"),
        None => l.clone(),
    }
}

fn norm(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches('\\').to_lowercase()
}

/// `a` is `b` or inside it.
fn within(a: &str, b: &str) -> bool {
    a == b || a.strip_prefix(b).is_some_and(|r| r.starts_with('\\')) || (b.len() == 2 && a.starts_with(b))
}

impl State {
    fn current_root(&self) -> Option<String> {
        self.tree.as_ref().filter(|t| t.root_path.as_os_str() != sprawl_core::MULTIPLE).map(|t| norm(&t.root_path))
    }

    /// The tab opened: load the saved scans (once a session) that the current scan doesn't cover.
    pub(crate) fn more_open(&mut self) {
        self.more_forget_covered();
        self.more_all_drives();
        if self.search.more.snapshots_tried {
            self.render_more();
            return;
        }
        self.search.more.snapshots_tried = true;
        if self.settings.bool("snapshots") == Some(false) {
            self.render_more();
            return;
        }
        let Some(base) = sprawl_advisor::history::base_dir() else { return };
        let cur = self.current_root();
        // the newest full snapshot of each root, newest roots first
        let mut newest: Vec<sprawl_advisor::history::Entry> = Vec::new();
        for e in sprawl_advisor::history::list(&base).into_iter().rev().filter(|e| e.full) {
            if newest.iter().any(|n| norm(&n.root) == norm(&e.root)) {
                continue;
            }
            let r = norm(&e.root);
            if cur.as_ref().is_some_and(|c| within(c, &r) || within(&r, c)) {
                continue;
            }
            // a drive being read now is newer than any saved scan on it
            if self.search.more.drives_on.iter().any(|d| within(&r, &norm(Path::new(d)))) {
                continue;
            }
            newest.push(e);
        }
        newest.truncate(MAX_SNAPSHOTS);
        if newest.is_empty() {
            self.render_more();
            return;
        }
        self.search.more.loading.extend(newest.iter().map(|e| (format!("{} (saved {})", e.root.display(), fmt_time(e.taken)), None)));
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for e in newest {
                let place = sprawl_advisor::history::load(&e).ok().map(|t| {
                    let index = Arc::new(Index::build(&t));
                    Place { tree: Arc::new(t), index, as_of: Some(e.taken) }
                });
                if tx.send(place).is_err() {
                    return;
                }
            }
        });
        self.more_queue(rx);
        self.render_more();
    }

    /// Places arrive in the order `loading` lists them: a new reader waits behind the ones before.
    fn more_queue(&mut self, rx: Receiver<Option<Place>>) {
        let Some(prev) = self.search.more.arriving.take() else {
            self.search.more.arriving = Some(rx);
            return;
        };
        let (tx2, rx2) = channel();
        std::thread::spawn(move || {
            for r in [prev, rx] {
                while let Ok(p) = r.recv() {
                    if tx2.send(p).is_err() {
                        return;
                    }
                }
            }
        });
        self.search.more.arriving = Some(rx2);
    }

    /// Every fixed drive the scan on screen isn't, read by itself like Everything: from the file
    /// table as administrator (seconds), else by walking its folders (slower).
    fn more_all_drives(&mut self) {
        // off unless switched on (on for settings from before 0.2.6, which had it on)
        if self.settings.bool("oSearchDrives") != Some(true) {
            return;
        }
        let cur = self.current_root();
        for d in crate::win32::drives().into_iter().filter(|d| d.kind == 3) {
            let m = &self.search.more;
            if cur.as_ref().is_some_and(|c| norm(Path::new(&d.root)) == *c) || m.drives_on.contains(&d.root) || m.drives_off.contains(&d.root) {
                continue;
            }
            if let Some(letter) = d.root.chars().next() {
                self.more_drive(letter);
            }
        }
    }

    /// A drive chip: read that drive (administrator, MFT) or forget it.
    pub(crate) fn more_drive(&mut self, letter: char) {
        let root = format!("{letter}:\\");
        let m = &mut self.search.more;
        m.drives_off.remove(&root);
        if m.drives_on.remove(&root) {
            m.drives_off.insert(root.clone());
            m.places.retain(|p| norm(&p.tree.root_path) != norm(Path::new(&root)));
            m.gen += 1;
            self.search_run();
            self.render_more();
            return;
        }
        m.drives_on.insert(root.clone());
        let progress = sprawl_core::Progress::new();
        m.loading.push((root.clone(), Some(progress.clone())));
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let place = sprawl_core::scan_auto(Path::new(&root), &progress).ok().map(|t| {
                let index = Arc::new(Index::build(&t));
                Place { tree: Arc::new(t), index, as_of: None }
            });
            let _ = tx.send(place);
        });
        self.more_queue(rx);
        self.render_more();
    }

    /// A scan replaced the tree: places it now covers go (they would only repeat it, older).
    pub(crate) fn more_forget_covered(&mut self) {
        let Some(cur) = self.current_root() else { return };
        let m = &mut self.search.more;
        let before = m.places.len();
        m.places.retain(|p| {
            let r = norm(&p.tree.root_path);
            // a whole drive read now still adds the rest of the volume; a saved scan of the same
            // folders is just older
            p.as_of.is_none() || !(within(&cur, &r) || within(&r, &cur))
        });
        if m.places.len() != before {
            m.gen += 1;
        }
    }

    /// Search the places for the current words (called with the main search).
    pub(crate) fn more_run(&mut self, cancel: Arc<AtomicBool>) {
        let cur = self.current_root();
        let m = &mut self.search.more;
        m.running = None;
        let q = Query::parse(&self.search.query);
        if q.is_empty() || m.places.is_empty() {
            m.hits.clear();
            m.total = 0;
            m.hits_gen = m.gen;
            self.render_more();
            return;
        }
        let places = m.places.clone();
        let gen = m.gen;
        let text_q = self.search.query.clone();
        let sort = self.search.sort;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut all: Vec<(usize, usize)> = Vec::new();
            for (k, p) in places.iter().enumerate() {
                match search(&p.tree, &p.index, &q, &cancel) {
                    Some(v) => all.extend(v.into_iter().map(|n| (k, n))),
                    None => {
                        let _ = tx.send((gen, text_q, sort, None));
                        return;
                    }
                }
            }
            sort_hits(&places, sort, &mut all);
            // what the scan on screen already shows is left out
            let mut shown = Vec::new();
            let mut total = 0usize;
            for (k, n) in all {
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send((gen, text_q, sort, None));
                    return;
                }
                if let Some(c) = &cur {
                    if within(&norm(&places[k].tree.path_of(n)), c) {
                        continue;
                    }
                }
                total += 1;
                if shown.len() < MAX_HITS {
                    shown.push((k, n));
                }
            }
            let _ = tx.send((gen, text_q, sort, Some((shown, total))));
        });
        m.running = Some(rx);
    }

    /// Each frame: places that arrived, results that arrived.
    /// The note in the main window while names are being indexed: the drives being read, or the
    /// scan's names, on any tab.
    fn index_note(&mut self) {
        let m = &self.search.more;
        let mut what: Vec<String> = m.loading.iter().map(loading_label).collect();
        if self.search.building.is_some() {
            what.insert(0, "this scan".into());
        }
        let note = if what.is_empty() { String::new() } else { format!("Indexing {} for search...", what.join(", ")) };
        if note == self.search.more.busy_shown {
            return;
        }
        self.search.more.busy_shown = note.clone();
        crate::state::text(&self.root, "IndexText", &note);
        if let Ok(b) = find::<Border>(&self.root, "IndexBusy") {
            let _ = b.cast::<UIElement>().and_then(|u| u.SetVisibility(if note.is_empty() { Visibility::Collapsed } else { Visibility::Visible }));
        }
    }

    pub(crate) fn more_poll(&mut self) {
        self.index_note();
        let mut changed = false;
        if let Some(rx) = self.search.more.arriving.take() {
            let mut open = true;
            loop {
                match rx.try_recv() {
                    Ok(place) => {
                        let m = &mut self.search.more;
                        if !m.loading.is_empty() {
                            m.loading.remove(0);
                        }
                        if let Some(p) = place {
                            m.places.push(Arc::new(p));
                            m.gen += 1;
                            changed = true;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        open = false;
                        self.search.more.loading.clear();
                        changed = true;
                        break;
                    }
                }
            }
            if open && self.search.more.arriving.is_none() {
                self.search.more.arriving = Some(rx);
            }
        }
        if changed {
            self.more_forget_covered();
            if self.view == View::Search {
                self.search_run();
                self.render_more();
            }
        } else if !self.search.more.loading.is_empty() && self.view == View::Search && self.search.more.drawn.is_none_or(|t| t.elapsed().as_millis() > 500) {
            // the counts going up
            self.render_more();
        }
        if let Some(rx) = &self.search.more.running {
            if let Ok((gen, q, sort, r)) = rx.try_recv() {
                self.search.more.running = None;
                if let Some((hits, total)) = r.filter(|_| gen == self.search.more.gen && q == self.search.query) {
                    let m = &mut self.search.more;
                    m.hits = hits;
                    m.total = total;
                    m.hits_gen = gen;
                    m.sel = None;
                    if sort != self.search.sort {
                        self.more_sort();
                    }
                    self.search.stamp += 1;
                    if self.view == View::Search {
                        self.refresh_rows();
                        let info = self.search_count();
                        crate::state::text(&self.root, "SearchInfo", &info);
                    }
                }
            }
        }
    }

    /// The matches from other drives in the current sort order (after a click on a column).
    pub(crate) fn more_sort(&mut self) {
        let m = &mut self.search.more;
        let mut v = std::mem::take(&mut m.hits);
        sort_hits(&m.places, self.search.sort, &mut v);
        self.search.more.hits = v;
    }

    /// A click on a row from another drive: picked (never part of the scan's picks); a
    /// double-click opens it (a file with its program, a folder in Explorer).
    pub(crate) fn more_tapped(&mut self, k: usize, n: usize, double: bool) {
        self.search.more.sel = Some((k, n));
        self.selected = None;
        self.marked.clear();
        if double {
            self.more_command('O', k, n);
        }
        self.focus_main();
        self.refresh_rows();
    }

    /// Right-click on a row from another drive: what can be done with it.
    pub(crate) fn more_menu(&mut self, k: usize, n: usize, at: crate::bindings::Windows::Foundation::Point) {
        self.search.more.sel = Some((k, n));
        self.selected = None;
        self.marked.clear();
        self.refresh_rows();
        let shown = || -> windows_core::Result<()> {
            use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{FlyoutBase, FlyoutShowOptions};
            let menu: FlyoutBase = find(&self.root, "MoreMenu")?;
            let target: crate::bindings::Microsoft::UI::Xaml::DependencyObject = find(&self.root, "TreeScroll")?;
            let opts = FlyoutShowOptions::new()?;
            opts.SetPosition(Some(at))?;
            menu.ShowAtWithOptions(&target, &opts)
        };
        if let Err(e) = shown() {
            eprintln!("search menu: {e:?}");
        }
    }

    /// An item of that menu, for the picked row.
    pub(crate) fn more_menu_pick(&mut self, kind: char) {
        if let Some((k, n)) = self.search.more.sel {
            self.more_command(kind, k, n);
        }
    }

    /// A row from another drive, in the same columns as the scan's.
    pub(crate) fn more_row(&self, row: usize, k: usize, n: usize) -> String {
        let Some(p) = self.search.more.places.get(k) else { return String::new() };
        let node = &p.tree.nodes[n];
        let bg = self.row_bg(row, self.search.more.sel == Some((k, n)));
        let folder = node.parent.map(|q| p.tree.path_of(q).display().to_string()).unwrap_or_default();
        let (glyph, gc) = if node.is_dir { ("&#xE8B7;", "#FFFFC83D".to_string()) } else { ("&#xE7C3;", self.ext_colour_hex(&node.extension())) };
        let saved = match p.as_of {
            Some(t) => format!(
                r##"<Border Background="#33FFB547" CornerRadius="3" Padding="5,0" VerticalAlignment="Center" ToolTipService.ToolTip="From a saved scan: how it was then, not checked now"><TextBlock Text="saved {}" FontSize="11" Foreground="#FFFFB547"/></Border>"##,
                ui::esc(&fmt_time(t))
            ),
            None => String::new(),
        };
        format!(
            r##"<Grid Height="24" Background="{bg}">{cols}<StackPanel Orientation="Horizontal" Spacing="8" Margin="10,0,0,0"><FontIcon Glyph="{glyph}" FontSize="13" Foreground="{gc}"/><TextBlock VerticalAlignment="Center" FontSize="12" TextTrimming="CharacterEllipsis">{runs}</TextBlock>{saved}</StackPanel>{c1}{c2}{c3}<TextBlock Grid.Column="4" Text="{folder}" FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center" Margin="10,0" TextTrimming="CharacterEllipsis"/></Grid>"##,
            cols = crate::tables::col_defs(crate::search::COLS),
            runs = self.name_runs(&node.name),
            c1 = crate::tables::cell(1, &self.size(node.size)),
            c2 = crate::tables::cell(2, ""),
            c3 = crate::tables::cell(3, &node.modified.map(fmt_time).unwrap_or_default()),
            folder = ui::esc(&folder),
        )
    }

    pub(crate) fn more_command(&mut self, kind: char, k: usize, n: usize) {
        let Some(path) = self.search.more.places.get(k).map(|p| p.tree.path_of(n)) else { return };
        match kind {
            'E' => {
                // show it in Explorer if it is still there, else its nearest folder that is
                let target = path.ancestors().find(|p| p.exists()).map(Path::to_path_buf);
                match target {
                    Some(t) if t == path => {
                        let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", t.display())).spawn();
                    }
                    Some(t) => {
                        let _ = std::process::Command::new("explorer.exe").arg(&t).spawn();
                        crate::state::text(&self.root, "SearchInfo", "That is gone now; its nearest folder is open.");
                    }
                    None => crate::state::text(&self.root, "SearchInfo", "That path is gone."),
                }
            }
            'O' => self.search_open(&path),
            'C' => {
                crate::win32::copy_text(self.hwnd, &path.display().to_string());
                crate::state::text(&self.root, "SearchInfo", "Path copied.");
            }
            'S' => {
                let folder = if path.is_dir() { path } else { path.parent().map(Path::to_path_buf).unwrap_or(path) };
                if !folder.exists() {
                    crate::state::text(&self.root, "SearchInfo", "That folder is gone.");
                    return;
                }
                self.set_view(View::Tree);
                self.start_scan(folder);
            }
            _ => {}
        }
    }

    /// The panel under the results: places, chips for other drives, and the rows.
    pub(crate) fn render_more(&mut self) {
        let Ok(host) = find::<Border>(&self.root, "SearchMore") else { return };
        let m = &self.search.more;
        let elevated = sprawl_core::is_elevated();
        let cur = self.current_root();
        let drives: Vec<crate::win32::Drive> = crate::win32::drives().into_iter().filter(|d| d.kind == 3).collect();
        let offer: Vec<&crate::win32::Drive> = drives.iter().filter(|d| cur.as_ref().is_none_or(|c| norm(Path::new(&d.root)) != *c)).collect();
        if m.places.is_empty() && m.loading.is_empty() && offer.is_empty() {
            let _ = host.cast::<UIElement>().and_then(|u| u.SetVisibility(Visibility::Collapsed));
            return;
        }
        self.search.more.drawn = Some(Instant::now());
        let m = &self.search.more;
        let mut x = format!(r##"<StackPanel {} Spacing="4" Padding="8,6,8,6">"##, ui::NS);
        x += r##"<StackPanel Orientation="Horizontal" Spacing="8">"##;
        let busy = !m.loading.is_empty();
        if busy {
            x += r##"<ProgressRing IsActive="True" Width="14" Height="14" MinWidth="0" MinHeight="0" VerticalAlignment="Center"/>"##;
        }
        x += &format!(r##"<TextBlock Text="{}" FontSize="12" FontWeight="SemiBold" Foreground="#FFB9C2D6" VerticalAlignment="Center"/>"##, if busy { "Reading" } else { "Searching" });
        let mut places: Vec<String> = Vec::new();
        if self.tree.is_some() {
            places.push("this scan".into());
        }
        places.extend(m.places.iter().map(|p| match p.as_of {
            Some(t) => format!("{} (saved {})", p.tree.root_path.display(), fmt_time(t)),
            None => p.tree.root_path.display().to_string(),
        }));
        let reading: Vec<String> = m.loading.iter().map(loading_label).collect();
        let line = if busy {
            let mut l = reading.join(",  ");
            if !places.is_empty() {
                l += &format!("   ·   searching {} meanwhile", places.join(", "));
            }
            l
        } else {
            places.join(",  ")
        };
        if !line.is_empty() {
            x += &format!(r##"<TextBlock Text="{}" FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center" TextTrimming="CharacterEllipsis" MaxWidth="640"/>"##, ui::esc(&line));
        }
        for d in &offer {
            let letter = d.root.chars().next().unwrap_or('C');
            let on = m.drives_on.contains(&d.root);
            x += &format!(
                r##"<ToggleButton x:Name="SxD{letter}" IsChecked="{on}" FontSize="11" Padding="8,2" ToolTipService.ToolTip="Search {root} too (kept in memory until Sprawl closes). Press to leave it out." Content="{letter}:"/>"##,
                on = if on { "True" } else { "False" },
                root = ui::esc(&d.root),
            );
        }
        if !elevated && busy {
            x += r##"<TextBlock Text="Folder by folder; as administrator it takes seconds." FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center"/>"##;
        }
        x += "</StackPanel>";
        x += "</StackPanel>";
        let Ok(content) = load::<UIElement>(&x) else { return };
        let _ = host.SetChild(&content);
        let _ = host.cast::<UIElement>().and_then(|u| u.SetVisibility(Visibility::Visible));
        // the clicks
        let Ok(fe) = content.cast::<crate::bindings::Microsoft::UI::Xaml::FrameworkElement>() else { return };
        for d in &offer {
            let letter = d.root.chars().next().unwrap_or('C');
            if let Ok(b) = find::<ButtonBase>(&fe, &format!("SxD{letter}")) {
                if let Ok(t) = b.Click(move |_, _| {
                    with(|s| s.more_drive(letter));
                }) {
                    keep(t);
                }
            }
        }
    }
}

/// (place, node) pairs in the Search sort order.
fn sort_hits(places: &[Arc<Place>], (c, desc): (usize, bool), v: &mut [(usize, usize)]) {
    let key = crate::search::COLS[c.min(crate::search::COLS.len() - 1)].key();
    v.sort_by(|a, b| {
        let o = crate::tables::cmp_two(&places[a.0].tree, &[], a.1, &places[b.0].tree, &[], b.1, key);
        if desc {
            o.reverse()
        } else {
            o
        }
    });
}

use windows_core::Interface;
