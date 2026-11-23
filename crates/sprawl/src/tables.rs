//! The folder tree, the file list and the file-type table. Columns sort when their header is
//! clicked, only the rows on screen (plus some either side) exist as XAML, rows have a
//! right-click menu, and the tree can be driven from the keyboard.

use std::cmp::Ordering;

use sprawl_core::{Tree, ROOT};
use windows_core::Interface;

use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, ScrollViewer};
use crate::bindings::Microsoft::UI::Xaml::{FocusState, FrameworkElement, UIElement, Visibility};
use crate::bindings::Windows::Foundation::Point;
use crate::format::{file_type, fmt_time, thousands};
use crate::state::{find, load, text, State, View, ROW_H};
use crate::{paint, ui};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Key {
    Name,
    Size,
    Alloc,
    Items,
    Files,
    Modified,
    Folder,
    Type,
    None,
}

pub(crate) struct Col {
    title: &'static str,
    /// Pixels, or a negative number for a share (`-2.0` is `2*`).
    width: f64,
    right: bool,
    key: Key,
}

const fn col(title: &'static str, width: f64, right: bool, key: Key) -> Col {
    Col { title, width, right, key }
}

pub(crate) const TREE: &[Col] = &[
    col("Folder", -1.0, false, Key::Name),
    col("% of Parent", 118.0, true, Key::Size),
    col("Size", 92.0, true, Key::Size),
    col("Allocated", 92.0, true, Key::Alloc),
    col("Items", 84.0, true, Key::Items),
    col("Files", 84.0, true, Key::Files),
    col("Owner", 150.0, false, Key::None),
];
const TREE_NARROW: &[Col] = &[
    col("Folder", -1.0, false, Key::Name),
    col("% of Parent", 100.0, true, Key::Size),
    col("Size", 92.0, true, Key::Size),
    col("Allocated", 92.0, true, Key::Alloc),
    col("Items", 0.0, true, Key::Items),
    col("Files", 0.0, true, Key::Files),
    col("Owner", 0.0, false, Key::None),
];
pub(crate) const FILES: &[Col] = &[
    col("File Name", -2.0, false, Key::Name),
    col("Size", 92.0, true, Key::Size),
    col("Allocated", 92.0, true, Key::Alloc),
    col("Modified", 130.0, true, Key::Modified),
    col("Folder", -3.0, false, Key::Folder),
];
/// The Search tab: files and folders together.
pub(crate) const FILES_LIKE: &[Col] = &[
    col("Name", -2.0, false, Key::Name),
    col("Size", 92.0, true, Key::Size),
    col("Allocated", 92.0, true, Key::Alloc),
    col("Modified", 130.0, true, Key::Modified),
    col("Folder", -3.0, false, Key::Folder),
];
/// The Folder column's width in characters for a table `width` wide: what the fixed columns
/// leave, its 3 parts of 5, at about 6.5 pixels a character of 12 px text (0: not measured).
pub(crate) fn folder_chars(width: f64) -> usize {
    if width <= 0.0 {
        return 0;
    }
    let fixed: f64 = FILES.iter().map(|c| c.width).filter(|w| *w > 0.0).sum::<f64>() + 40.0;
    (((width - fixed).max(0.0) * 3.0 / 5.0) / 6.5).max(16.0) as usize
}

pub(crate) const EXTS: &[Col] = &[
    col("", 22.0, false, Key::None),
    col("Extension", 78.0, false, Key::Name),
    col("File Type", -1.0, false, Key::Type),
    col("Percent", 96.0, true, Key::Size),
    col("Size", 80.0, true, Key::Size),
    col("Allocated", 80.0, true, Key::Alloc),
    col("Files", 70.0, true, Key::Files),
];

const OVERSCAN: usize = 40;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Which {
    Main,
    Exts,
}

/// Sort order and what part of each table currently exists.
pub(crate) struct Tables {
    compact_tree: bool,
    /// About how many characters the Folder column of File view and Search holds (0: not
    /// measured yet); longer paths are shortened in the middle to it.
    pub(crate) folder_chars: usize,
    /// (column, descending)
    pub(crate) tree_sort: (usize, bool),
    pub(crate) files_sort: (usize, bool),
    pub(crate) ext_sort: (usize, bool),
    /// The tall content of each table's scroll (Tree, Files, Search, file types): each view has
    /// its own, so going back to a tab shows its rows as they were, not loaded again.
    bodies: [Option<Border>; 4],
    main_span: (usize, usize, usize),
    ext_span: (usize, usize, usize),
    /// The rows' XAML as last loaded (main table, file types): the same text is not loaded
    /// again, so rows that didn't change aren't replaced (other drives' results arrive every
    /// half second; replacing the rows lost a double-click and the scroll's smoothness).
    drawn: [String; 4],
    /// The rows on screen (as `bodies`): a new set of rows changes only the rows that differ
    /// (a selection, a folder opened, a sort), so the others aren't laid out again.
    slots: [crate::reconcile::Slot; 4],
    /// The headers' XAML as last loaded: loaded again only when it changed.
    heads: [String; 2],
}

impl Default for Tables {
    fn default() -> Self {
        Tables {
            compact_tree: false,
            folder_chars: 0,
            tree_sort: (2, true),
            files_sort: (1, true),
            ext_sort: (4, true),
            bodies: Default::default(),
            main_span: (0, 0, usize::MAX),
            ext_span: (0, 0, usize::MAX),
            drawn: Default::default(),
            slots: Default::default(),
            heads: Default::default(),
        }
    }
}

impl Tables {
    /// The page was made again (the theme changed): what was drawn is drawn again, into new
    /// hosts. The sorts and the measured widths stay.
    pub(crate) fn restyle(&mut self) {
        *self = Tables { compact_tree: self.compact_tree, folder_chars: self.folder_chars, tree_sort: self.tree_sort, files_sort: self.files_sort, ext_sort: self.ext_sort, ..Default::default() };
    }
}

impl Col {
    pub(crate) fn key(&self) -> Key {
        self.key
    }
}

/// Sort `v` (node indices) by `key`, the way the tables do. By folder, `ranks` (from
/// `folder_ranks`, or empty) saves building paths: without it each row's folder path is built
/// once, not once per comparison.
pub(crate) fn sort_nodes(t: &Tree, alloc: &[u64], key: Key, desc: bool, v: &mut [usize], ranks: &[u32]) {
    let dir = |o: Ordering| if desc { o.reverse() } else { o };
    if key == Key::Folder {
        let name = |a: usize, b: usize| cmp_names(&t.nodes[a].name, &t.nodes[b].name);
        if ranks.len() == t.nodes.len() {
            let rank = |n: usize| t.nodes[n].parent.map_or(0, |p| ranks[p] as u64 + 1);
            v.sort_by(|&a, &b| dir(rank(a).cmp(&rank(b)).then_with(|| name(a, b))));
        } else {
            let mut keyed: Vec<(Vec<u8>, usize)> = v.iter().map(|&n| (folder_key(t, n), n)).collect();
            keyed.sort_by(|(ka, a), (kb, b)| dir(ka.cmp(kb).then_with(|| name(*a, *b))));
            for (slot, (_, n)) in v.iter_mut().zip(keyed) {
                *slot = n;
            }
        }
        return;
    }
    v.sort_by(|&a, &b| {
        let o = if key == Key::Name { cmp_names(&t.nodes[a].name, &t.nodes[b].name) } else { cmp_nodes(t, alloc, key, a, b) };
        dir(o)
    });
}

/// The folder a row is in, as the Folder column compares it: its path, lowercased.
pub(crate) fn folder_key(t: &Tree, n: usize) -> Vec<u8> {
    let mut k = Vec::new();
    if let Some(p) = t.nodes[n].parent {
        sprawl_core::search::lower_bytes(&t.path_of(p).to_string_lossy(), &mut k);
    }
    k
}

/// Compare two nodes that may come from different trees (Search mixes the scan with other
/// drives). A tree without allocated sizes (`&[]`) uses the size. Kept as the tests' measure
/// of the folder sort.
#[cfg(test)]
pub(crate) fn cmp_two(ta: &Tree, aa: &[u64], a: usize, tb: &Tree, ab: &[u64], b: usize, key: Key) -> Ordering {
    let (x, y) = (&ta.nodes[a], &tb.nodes[b]);
    let o = match key {
        Key::Name => cmp_names(&x.name, &y.name),
        Key::Size => x.size.cmp(&y.size),
        Key::Alloc => aa.get(a).copied().unwrap_or(x.size).cmp(&ab.get(b).copied().unwrap_or(y.size)),
        Key::Items => x.items().cmp(&y.items()),
        Key::Files => x.files.cmp(&y.files),
        Key::Modified => x.modified.cmp(&y.modified),
        Key::Folder => match (x.parent, y.parent) {
            (Some(p), Some(q)) => cmp_names(&ta.path_of(p).to_string_lossy(), &tb.path_of(q).to_string_lossy()),
            _ => Ordering::Equal,
        },
        Key::Type => cmp_names(&file_type(&x.extension()), &file_type(&y.extension())),
        Key::None => Ordering::Equal,
    };
    o.then_with(|| cmp_names(&x.name, &y.name))
}

pub(crate) fn col_defs(cols: &[Col]) -> String {
    let mut s = String::from("<Grid.ColumnDefinitions>");
    for c in cols {
        if c.width < 0.0 {
            s += &format!(r##"<ColumnDefinition Width="{}*"/>"##, -c.width);
        } else {
            s += &format!(r##"<ColumnDefinition Width="{}"/>"##, c.width);
        }
    }
    s + "</Grid.ColumnDefinitions>"
}

fn header(cols: &[Col], sort: (usize, bool)) -> String {
    let mut s = format!(r##"<Grid {} Height="30" Background="{{$shade}}">{}"##, ui::NS, col_defs(cols));
    for (i, c) in cols.iter().enumerate() {
        if c.width == 0.0 { continue; }
        let align = if c.right { "Right" } else { "Left" };
        let arrow = if i == sort.0 { if sort.1 { "&#xE70D;" } else { "&#xE70E;" } } else { "" };
        let colour = if i == sort.0 { crate::tokens::refer("rest") } else { crate::tokens::refer("text2") };
        s += &format!(
            r##"<StackPanel Grid.Column="{i}" Orientation="Horizontal" Spacing="5" HorizontalAlignment="{align}" Margin="10,0,10,0"><TextBlock Text="{}" FontSize="{{$fs12}}" FontWeight="SemiBold" Foreground="{colour}" VerticalAlignment="Center"/><FontIcon Glyph="{arrow}" FontSize="{{$fs12}}" Foreground="{{$accenttext}}" VerticalAlignment="Center"/></StackPanel>"##,
            ui::esc(c.title)
        );
    }
    s + r##"<Border Grid.ColumnSpan="9" Height="1" VerticalAlignment="Bottom" Background="{$pressed}"/></Grid>"##
}

/// The column under `x` in a header `width` wide.
fn col_at(cols: &[Col], width: f64, x: f64) -> Option<usize> {
    let fixed: f64 = cols.iter().filter(|c| c.width > 0.0).map(|c| c.width).sum();
    let stars: f64 = cols.iter().filter(|c| c.width < 0.0).map(|c| -c.width).sum();
    let per_star = ((width - fixed) / stars.max(1.0)).max(0.0);
    let mut left = 0.0;
    for (i, c) in cols.iter().enumerate() {
        left += if c.width > 0.0 { c.width } else { -c.width * per_star };
        if x < left {
            return Some(i);
        }
    }
    None
}

/// Case-insensitive comparison without allocating: the order of the names lowercased (as
/// Search's index of other drives keeps them), so the scan's rows and other drives' rows sort
/// alike and merge at one seam. ASCII names take the quick path.
pub(crate) fn cmp_names(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        return a.bytes().map(|c| c.to_ascii_lowercase()).cmp(b.bytes().map(|c| c.to_ascii_lowercase()));
    }
    a.chars().flat_map(char::to_lowercase).cmp(b.chars().flat_map(char::to_lowercase))
}

fn cmp_nodes(t: &Tree, alloc: &[u64], key: Key, a: usize, b: usize) -> Ordering {
    let (x, y) = (&t.nodes[a], &t.nodes[b]);
    let o = match key {
        Key::Size => x.size.cmp(&y.size),
        Key::Alloc => alloc.get(a).cmp(&alloc.get(b)),
        Key::Items => x.items().cmp(&y.items()),
        Key::Files => x.files.cmp(&y.files),
        Key::Modified => x.modified.cmp(&y.modified),
        Key::Folder => match (x.parent, y.parent) {
            (Some(p), Some(q)) if p != q => cmp_names(&t.path_of(p).to_string_lossy(), &t.path_of(q).to_string_lossy()),
            _ => Ordering::Equal,
        },
        Key::Type => cmp_names(&file_type(&x.extension()), &file_type(&y.extension())),
        Key::Name | Key::None => Ordering::Equal,
    };
    o.then_with(|| cmp_names(&x.name, &y.name))
}

pub(crate) fn cell(col: usize, s: &str) -> String {
    format!(r##"<TextBlock Grid.Column="{col}" Text="{}" {} FontSize="{{$fs12}}" HorizontalAlignment="Right" VerticalAlignment="Center" Margin="0,0,10,0"/>"##, ui::esc(s), crate::look::NUM)
}

/// A percentage bar: the track, then the share in the accent, washed so the figure on it reads
/// whichever accent the look uses. The look's `bars` option decides whether the fill is one
/// colour, a gradient to a lighter accent, or a 4 px line along the bottom.
/// The selected row's accent line (the Selection option's Accent bar and Both): 3 px, rounded,
/// at the row's left edge. Every row has one, shown or collapsed, so a row picked or unpicked is
/// changed in place (an element that came or went would load the row again: see reconcile.rs).
/// Nothing at all when the option is the tint alone.
pub(crate) fn row_bar(selected: bool) -> String {
    if !crate::tokens::current().selection.bar() {
        return String::new();
    }
    let shown = if selected { "Visible" } else { "Collapsed" };
    format!(r##"<Border Width="3" Margin="1,4,0,4" HorizontalAlignment="Left" CornerRadius="{{$rc2}}" Background="{{$accent}}" Visibility="{shown}" IsHitTestVisible="False"/>"##)
}

pub(crate) fn pct_bar(col: usize, pct: f64) -> String {
    let w = pct.clamp(0.0, 100.0);
    let num = crate::look::NUM;
    let fill = crate::look::accent().fill;
    let bars = crate::tokens::current().bars;
    // the lit top edge of the fill (the Soft finish)
    let lit = crate::look::top_light("hlbar", &crate::tokens::rc(3.0), true);
    let (track, fill_border) = match bars {
        crate::tokens::Bars::Thin => (
            r##"<Border CornerRadius="{$rc3}" Background="{$line1}" VerticalAlignment="Bottom" Height="4"/>"##.to_string(),
            format!(r##"<Border CornerRadius="{{$rc3}}" HorizontalAlignment="Left" Width="{w:.1}" Background="{fill}" Opacity="0.45" VerticalAlignment="Bottom" Height="4"/>"##),
        ),
        crate::tokens::Bars::Gradient => (
            r##"<Border CornerRadius="{$rc3}" Background="{$line1}"/>"##.to_string(),
            format!(
                r##"<Border CornerRadius="{{$rc3}}" HorizontalAlignment="Left" Width="{w:.1}" Opacity="0.45"><Border.Background>{}</Border.Background>{lit}</Border>"##,
                crate::look::accent_gradient()
            ),
        ),
        crate::tokens::Bars::Solid => (
            r##"<Border CornerRadius="{$rc3}" Background="{$line1}"/>"##.to_string(),
            format!(r##"<Border CornerRadius="{{$rc3}}" HorizontalAlignment="Left" Width="{w:.1}" Background="{fill}" Opacity="0.45">{lit}</Border>"##),
        ),
    };
    format!(
        r##"<Grid Grid.Column="{col}" Margin="6,4,6,4">{track}{fill_border}<TextBlock Text="{pct:.1} %" {num} FontSize="{{$fs12}}" Foreground="{{$text1}}" HorizontalAlignment="Right" VerticalAlignment="Center" Margin="0,0,6,0"/></Grid>"##
    )
}

impl State {
    // ------------------------------------------------------------ sorting

    /// dev: the middle of the main table's column `k` at this width (the benchmark's sort step).
    pub(crate) fn main_col_middle(&self, k: usize, width: f64) -> f64 {
        let cols = self.main_cols().0;
        let fixed: f64 = cols.iter().filter(|c| c.width > 0.0).map(|c| c.width).sum();
        let stars: f64 = cols.iter().filter(|c| c.width < 0.0).map(|c| -c.width).sum();
        let per_star = ((width - fixed) / stars.max(1.0)).max(0.0);
        let w = |c: &Col| if c.width > 0.0 { c.width } else { -c.width * per_star };
        cols.iter().take(k).map(w).sum::<f64>() + cols.get(k).map_or(0.0, |c| w(c) / 2.0)
    }

    fn main_cols(&self) -> (&'static [Col], (usize, bool)) {
        if self.view == View::Files {
            (FILES, self.tables.files_sort)
        } else if self.view == View::Search {
            (FILES_LIKE, self.search.sort)
        } else {
            (if self.tables.compact_tree { TREE_NARROW } else { TREE }, self.tables.tree_sort)
        }
    }

    /// Order every folder's children by the tree view's sort column.
    pub(crate) fn sort_tree(&mut self) {
        self.order_gen += 1;
        let sort = self.tables.tree_sort;
        let Some(t) = self.tree.as_mut().map(std::sync::Arc::make_mut) else { return };
        sort_children(t, &self.alloc, sort);
    }

    /// Make the name and folder places for the tree on screen on a worker, unless the ones
    /// held fit it already (a filter cleared: the full tree's are kept with it).
    pub(crate) fn ranks_start(&mut self) {
        self.ranks_job = None;
        let Some(t) = self.tree.clone() else { return };
        if self.ranks.fits(&t) {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(Ranks::of(&t));
        });
        self.ranks_job = Some((self.tree_set, rx));
    }

    /// Each frame: places made on the worker are kept if they are for the tree still shown.
    /// With `wait`, a File View click waits for them (no longer than making them here).
    pub(crate) fn ranks_poll(&mut self, wait: bool) {
        let Some((set, rx)) = &self.ranks_job else { return };
        let got = if wait { rx.recv().ok() } else {
            match rx.try_recv() {
                Ok(r) => Some(r),
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(_) => None,
            }
        };
        let same = *set == self.tree_set;
        self.ranks_job = None;
        if let (Some(r), true, Some(t)) = (got, same, self.tree.as_ref()) {
            if r.fits(t) {
                self.ranks = std::sync::Arc::new(r);
            }
        }
    }

    pub(crate) fn sort_files(&mut self) {
        self.ranks_poll(true);
        let Some(t) = &self.tree else { return };
        // a rescan added nodes since: made again here, once, not the slow sort every click
        if !self.ranks.fits(t) {
            let t0 = std::time::Instant::now();
            self.ranks = std::sync::Arc::new(Ranks::of(t));
            crate::state::timing("name and folder places made again", t0);
        }
        let t0 = std::time::Instant::now();
        sort_file_list(t, &self.alloc, &self.ranks, self.tables.files_sort, &mut self.files);
        crate::state::timing(&format!("File View sort ({} files)", self.files.len()), t0);
    }

    pub(crate) fn sort_exts(&mut self) {
        sort_ext_rows(&mut self.exts, self.tables.ext_sort);
    }

    /// A click on a table header at `x` (header coordinates).
    pub(crate) fn header_clicked(&mut self, which: Which, x: f64, width: f64) {
        let t0 = std::time::Instant::now();
        self.header_clicked_inner(which, x, width);
        crate::state::timing("header click (sort)", t0);
    }

    fn header_clicked_inner(&mut self, which: Which, x: f64, width: f64) {
        let cols = match which {
            Which::Exts => EXTS,
            Which::Main => self.main_cols().0,
        };
        let Some(c) = col_at(cols, width, x) else { return };
        if cols[c].key == Key::None {
            return;
        }
        let numeric = !matches!(cols[c].key, Key::Name | Key::Folder | Key::Type);
        let flip = |s: &mut (usize, bool)| *s = if s.0 == c { (c, !s.1) } else { (c, numeric) };
        match which {
            Which::Exts => {
                flip(&mut self.tables.ext_sort);
                self.sort_exts();
                self.refresh_exts();
            }
            Which::Main if self.view == View::Files => {
                let before = self.tables.files_sort;
                flip(&mut self.tables.files_sort);
                // the same column again: the list turned around, exactly as sorting it would
                // (a few ms, where sorting millions of files takes about a tenth of a second)
                let turned = before.0 == self.tables.files_sort.0 && before.1 != self.tables.files_sort.1 && self.tree.as_ref().is_some_and(|t| reverse_file_list(t, &self.alloc, &self.ranks, before.0, &mut self.files));
                if !turned {
                    self.sort_files();
                }
                self.refresh_rows();
            }
            Which::Main if self.view == View::Search => {
                flip(&mut self.search.sort);
                self.search_sort();
                self.refresh_rows();
            }
            Which::Main => {
                // only the open folders are put in the new order, as they are shown
                // (visible_rows); the tree itself is left as it is
                flip(&mut self.tables.tree_sort);
                self.order_gen += 1;
                self.refresh_rows();
            }
        }
    }

    // ------------------------------------------------------------ rows

    pub(crate) fn visible_rows(&self) -> Vec<(usize, usize)> {
        let t = self.tree.as_ref().unwrap();
        let sort = self.tables.tree_sort;
        let key = (std::sync::Arc::as_ptr(t) as usize, self.tree_set, sort, self.order_gen);
        let mut cache = self.kid_order.borrow_mut();
        if cache.0 != Some(key) {
            *cache = (Some(key), std::collections::HashMap::new());
        }
        let mut out = Vec::new();
        let mut stack = vec![(ROOT, 0usize)];
        while let Some((n, d)) = stack.pop() {
            out.push((n, d));
            if self.expanded.contains(&n) {
                let kids = cache.1.entry(n).or_insert_with(|| {
                    let mut kids = t.nodes[n].children.clone();
                    kids.sort_by(child_order(t, &self.alloc, sort));
                    kids
                });
                for &c in kids.iter().rev() {
                    stack.push((c, d + 1));
                }
            }
        }
        out
    }

    /// Rebuild the main table (tree or file list) after the data, selection or view changed.
    pub(crate) fn refresh_rows(&mut self) {
        if self.tree.is_none() && self.view != View::Search {
            return;
        }
        let t0 = std::time::Instant::now();
        self.rows = match self.view {
            View::Files => self.files.iter().map(|&f| (f, 0)).collect(),
            View::Search => self.search_rows(),
            _ => self.visible_rows(),
        };
        let (cols, sort) = self.main_cols();
        let head = header(cols, sort);
        if head != self.tables.heads[0] {
            if let (Ok(h), Ok(el)) = (find::<Border>(&self.root, "TreeHeader"), load::<UIElement>(&head)) {
                let _ = h.SetChild(&el);
                self.tables.heads[0] = head;
            }
        }
        let total = self.tree.as_ref().map_or(0, |t| t.nodes[ROOT].size);
        let items = self.tree.as_ref().map_or(0, |t| t.nodes[ROOT].items());
        if self.view == View::Search {
            let more = self.search.more.items_searched();
            text(&self.root, "FootRight", &format!("{} found  |  {} items searched", thousands(self.rows.len() as u64), thousands(items + more)));
            crate::welcome::tree_hint_shown(self, false);
        } else {
            text(&self.root, "FootRight", &format!("{} items  |  {}", thousands(items), self.size(total)));
        }
        self.tables.main_span = (0, 0, usize::MAX);
        crate::state::timing("refresh_rows: rows and header", t0);
        self.fill(Which::Main);
        crate::state::timing(&format!("refresh_rows ({} rows)", self.rows.len()), t0);
    }

    pub(crate) fn refresh_exts(&mut self) {
        let head = header(EXTS, self.tables.ext_sort);
        if head != self.tables.heads[1] {
            if let (Ok(h), Ok(el)) = (find::<Border>(&self.root, "ExtHeader"), load::<UIElement>(&head)) {
                let _ = h.SetChild(&el);
                self.tables.heads[1] = head;
            }
        }
        self.tables.ext_span = (0, 0, usize::MAX);
        self.fill(Which::Exts);
    }

    /// Make sure the rows on screen exist (called after scrolling and resizing too).
    pub(crate) fn fill(&mut self, which: Which) {
        let (scroll, count) = match which {
            Which::Main => ("TreeScroll", if self.tree.is_some() || self.view == View::Search { self.rows.len() } else { 0 }),
            Which::Exts => ("ExtScroll", self.exts.len()),
        };
        let Ok(sv) = find::<ScrollViewer>(&self.root, scroll) else { return };
        if which == Which::Main && !matches!(self.view, View::Files | View::Search) {
            let width = sv.ViewportWidth().unwrap_or(0.0);
            let compact = width > 0.0 && width < 900.0;
            if compact != self.tables.compact_tree {
                self.tables.compact_tree = compact;
                self.refresh_rows();
                return;
            }
        }
        if which == Which::Main && matches!(self.view, View::Files | View::Search) {
            let chars = folder_chars(sv.ViewportWidth().unwrap_or(0.0));
            if chars.abs_diff(self.tables.folder_chars) > 3 {
                self.tables.folder_chars = chars;
                self.refresh_rows();
                return;
            }
        }
        let off = sv.VerticalOffset().unwrap_or(0.0);
        let vh = sv.ViewportHeight().unwrap_or(0.0);
        let vh = if vh > 0.0 { vh } else { 900.0 };
        let g = Rows { count, off, vh };
        let first = (g.top().floor().max(0.0) as usize).min(count);
        let last = ((g.top() + vh / ROW_H).ceil().max(0.0) as usize).min(count);
        let span = if which == Which::Main { self.tables.main_span } else { self.tables.ext_span };
        if span.2 == count && span.0 <= first && last <= span.1 {
            // a long list's rows move with the scroll (see Rows): the loaded ones are moved
            if g.scaled() {
                let ix = self.body_ix(which);
                if let Some(fe) = self.tables.bodies[ix].as_ref().and_then(|b| b.Child().ok()).and_then(|c| c.cast::<FrameworkElement>().ok()) {
                    let _ = fe.SetMargin(crate::bindings::Microsoft::UI::Xaml::Thickness { Left: 0.0, Top: g.y_of(span.0), Right: 0.0, Bottom: 0.0 });
                    // the screen no longer shows the string's place: the next rows load whole
                    self.tables.slots[ix].forget();
                }
            }
            return;
        }
        let a = first.saturating_sub(OVERSCAN).min(count);
        let b = (last + OVERSCAN).min(count);

        // The tall, empty content that gives the scroll bar its range; rows sit inside it.
        let slot = self.body_ix(which);
        if self.tables.bodies[slot].is_none() {
            self.tables.bodies[slot] = load::<Border>(&format!(r##"<Border {} VerticalAlignment="Top" Background="Transparent"/>"##, ui::NS)).ok();
        }
        let Some(body) = self.tables.bodies[slot].clone() else { return };
        // this view's rows in the scroll (another tab's stay as they are, for coming back)
        let shown = sv.Content().ok().and_then(|c| c.cast::<Border>().ok()).is_some_and(|c| c == body);
        if !shown {
            if let Ok(u) = body.cast::<UIElement>() {
                let _ = sv.SetContent(&u);
            }
        }
        if let Ok(fe) = body.cast::<FrameworkElement>() {
            let _ = fe.SetHeight(g.body());
        }
        let mut xaml = format!(r##"<StackPanel {} VerticalAlignment="Top" Margin="0,{},0,0">"##, ui::NS, g.y_of(a));
        for k in a..b {
            xaml += &match which {
                Which::Main if self.view == View::Files => self.file_row(k),
                Which::Main if self.view == View::Search => self.search_row(k),
                Which::Main => self.tree_row(k),
                Which::Exts => self.ext_row(k),
            };
        }
        xaml += "</StackPanel>";
        let span = (a, b, count);
        if self.tables.drawn[slot] == xaml && body.Child().is_ok() {
            if which == Which::Main {
                self.tables.main_span = span;
            } else {
                self.tables.ext_span = span;
            }
            return;
        }
        let t0 = std::time::Instant::now();
        let shown = self.tables.slots[slot].show(&body, &xaml, true);
        crate::state::timing(&format!("fill: {} {} rows", if shown.as_ref().is_ok_and(|s| s.whole) { "load" } else { "change" }, b - a), t0);
        match shown {
            Ok(_) => {
                self.tables.drawn[slot] = xaml;
            }
            Err(e) => {
                // leave the span stale so the next scroll tries again
                eprintln!("rows: {e:?}");
                return;
            }
        }
        let span = (a, b, count);
        if which == Which::Main {
            self.tables.main_span = span;
        } else {
            self.tables.ext_span = span;
        }
    }

    /// Which of `bodies` (and `slots`, `drawn`) a table shows now.
    fn body_ix(&self, which: Which) -> usize {
        match which {
            Which::Exts => 3,
            Which::Main => match self.view {
                View::Files => 1,
                View::Search => 2,
                _ => 0,
            },
        }
    }

    fn tree_row(&self, k: usize) -> String {
        let t = self.tree.as_ref().unwrap();
        let (n, d) = self.rows[k];
        let node = &t.nodes[n];
        let sel = Some(n) == self.selected || self.marked.contains(&n);
        let (bg, sel_bar) = (self.row_bg(k, sel), row_bar(sel));
        let pct = t.percent_of_parent(n);
        let has_kids = node.is_dir && !node.children.is_empty();
        let chevron = if !has_kids { "" } else if self.expanded.contains(&n) { "&#xE70D;" } else { "&#xE76C;" };
        let (glyph, gc) = if n == ROOT {
            ("&#xEDA2;", crate::tokens::refer("link"))
        } else if node.is_dir {
            ("&#xE8B7;", crate::tokens::refer("folder"))
        } else {
            ("&#xE7C3;", self.ext_colour_hex(&node.extension()))
        };
        let items = if node.is_dir { thousands(node.items()) } else { String::new() };
        let files = if node.is_dir { thousands(node.files) } else { String::new() };
        let hidden = n != ROOT && (node.is_hidden() || node.is_system());
        let fg = if hidden { crate::tokens::refer("faint") } else { crate::tokens::refer("text0") };
        let label = |n: usize| {
            let l = self.label(n);
            // the scanned folder's own name first: its long path is cut from the end
            if n != ROOT {
                return l;
            }
            let p = std::path::Path::new(&l);
            match (p.file_name(), p.parent()) {
                (Some(base), Some(parent)) if !parent.as_os_str().is_empty() => format!("{}   ·   {}", base.to_string_lossy(), parent.display()),
                _ => l,
            }
        };
        let name = if node.is_compressed() { format!("{} · compressed", label(n)) } else if self.advisor.memory.compressed.iter().any(|c| crate::cleanup::key(std::path::Path::new(&c.path)) == crate::cleanup::key(&t.path_of(n))) { format!("{} · compressed by Sprawl (record)", label(n)) } else { label(n) };
        format!(
            r##"<Grid x:Name="TRow{n}" Height="24" Background="{bg}">{cols}{sel_bar}<Grid ColumnSpacing="6" Margin="{indent},0,6,0" ToolTipService.ToolTip="{name}"><Grid.ColumnDefinitions><ColumnDefinition Width="12"/><ColumnDefinition Width="16"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><FontIcon Glyph="{chevron}" FontSize="{{$fs12}}" Width="12" Foreground="{{$text3}}"/><FontIcon Grid.Column="1" Glyph="{glyph}" FontSize="{{$fs14}}" Foreground="{gc}" Opacity="{op}"/><TextBlock Grid.Column="2" Text="{name}" VerticalAlignment="Center" FontSize="{{$fs12}}" Foreground="{fg}" TextTrimming="CharacterEllipsis"/></Grid>{bar}{c2}{c3}{c4}{c5}{c6}</Grid>"##,
            cols = col_defs(self.main_cols().0),
            indent = 8 + d * 16,
            op = if hidden { "0.55" } else { "1" },
            name = ui::esc(&name),
            bar = pct_bar(1, pct),
            c2 = cell(2, &self.size(node.size)),
            c3 = cell(3, &self.size(self.alloc[n])),
            c4 = if self.tables.compact_tree { String::new() } else { cell(4, &items) },
            c5 = if self.tables.compact_tree { String::new() } else { cell(5, &files) },
            // the app that owns it; dimmed where it just continues from the parent
            c6 = match self.owner_of(n).filter(|_| !self.tables.compact_tree) {
                // an uninstall mark where the app starts, when Sprawl knows where it is uninstalled
                Some((name, inherited, can_uninstall)) => format!(
                    r##"<TextBlock Grid.Column="6" FontSize="{{$fs12}}" Margin="12,0,6,0" VerticalAlignment="Center" TextTrimming="CharacterEllipsis" Foreground="{}">{}<Run Text="{}"/></TextBlock>"##,
                    if inherited { crate::tokens::refer("text3") } else { crate::tokens::refer("text2") },
                    if can_uninstall && !inherited { r##"<Run Text="&#xE74D; " FontFamily="Segoe Fluent Icons,Segoe MDL2 Assets" FontSize="{$fs12}" Foreground="{$text3}"/>"## } else { "" },
                    ui::esc(&name),
                ),
                None => String::new(),
            },
        )
    }

    fn file_row(&self, k: usize) -> String {
        let t = self.tree.as_ref().unwrap();
        let n = self.rows[k].0;
        let node = &t.nodes[n];
        let sel = Some(n) == self.selected || self.marked.contains(&n);
        let (bg, sel_bar) = (self.row_bg(k, sel), row_bar(sel));
        let modified = node.modified.map(fmt_time).unwrap_or_default();
        let folder = node.parent.map(|p| t.path_of(p).display().to_string()).unwrap_or_default();
        format!(
            r##"<Grid x:Name="FRow{n}" Height="24" Background="{bg}">{cols}{sel_bar}<StackPanel Orientation="Horizontal" Spacing="8" Margin="10,0,0,0"><FontIcon Glyph="&#xE7C3;" FontSize="{{$fs13}}" Foreground="{fc}"/><TextBlock Text="{name}" VerticalAlignment="Center" FontSize="{{$fs12}}" TextTrimming="CharacterEllipsis"/></StackPanel>{c1}{c2}{c3}<TextBlock Grid.Column="4" Text="{short}" FontSize="{{$fs12}}" Foreground="{{$text3}}" VerticalAlignment="Center" Margin="10,0" TextTrimming="CharacterEllipsis" ToolTipService.ToolTip="{folder}"/></Grid>"##,
            short = ui::esc(&crate::format::mid_path(&folder, self.tables.folder_chars)),
            cols = col_defs(FILES),
            fc = self.ext_colour_hex(&node.extension()),
            name = ui::esc(&node.name),
            c1 = cell(1, &self.size(node.size)),
            c2 = cell(2, &self.size(self.alloc[n])),
            c3 = cell(3, &modified),
            folder = ui::esc(&folder),
        )
    }

    fn ext_row(&self, k: usize) -> String {
        let e = &self.exts[k];
        let sel = self.ext_selected.as_deref() == Some(e.ext.as_str());
        let (bg, sel_bar) = (self.row_bg(k, sel), row_bar(sel));
        let shown = if e.ext == "(none)" { String::new() } else { format!(".{}", e.ext) };
        format!(
            r##"<Grid Height="24" Background="{bg}">{cols}{sel_bar}<Border Width="12" Height="12" CornerRadius="{{$rc3}}" Background="{c}" Margin="8,0,0,0"/><TextBlock Grid.Column="1" Text="{ext}" FontSize="{{$fs12}}" VerticalAlignment="Center" Margin="8,0,0,0" TextTrimming="CharacterEllipsis"/><TextBlock Grid.Column="2" Text="{kind}" FontSize="{{$fs12}}" Foreground="{{$text2}}" VerticalAlignment="Center" Margin="10,0" TextTrimming="CharacterEllipsis"/>{bar}{c4}{c5}{c6}</Grid>"##,
            cols = col_defs(EXTS),
            c = self.ext_colour_hex(&e.ext),
            ext = ui::esc(&shown),
            kind = ui::esc(&file_type(&e.ext)),
            bar = pct_bar(3, e.percent),
            c4 = cell(4, &self.size(e.size)),
            c5 = cell(5, &self.size(e.alloc)),
            c6 = cell(6, &thousands(e.files)),
        )
    }

    pub(crate) fn row_bg(&self, k: usize, selected: bool) -> String {
        if selected && crate::tokens::current().selection.fill() {
            crate::tokens::refer("selected")
        } else if k % 2 == 1 {
            crate::tokens::refer("stripe")
        } else {
            "Transparent".to_string()
        }
    }

    pub(crate) fn ext_colour_hex(&self, ext: &str) -> String {
        let (r, g, b) = self.colours.get(ext).map(|&i| paint::colour(i)).unwrap_or(paint::GREY);
        format!("#FF{r:02X}{g:02X}{b:02X}")
    }

    // ------------------------------------------------------------ mouse and keyboard

    /// Where the rows of a table sit now.
    pub(crate) fn rows_now(&self, which: Which) -> Rows {
        let (scroll, count) = match which {
            Which::Main => ("TreeScroll", if self.tree.is_some() || self.view == View::Search { self.rows.len() } else { 0 }),
            Which::Exts => ("ExtScroll", self.exts.len()),
        };
        let (off, vh) = find::<ScrollViewer>(&self.root, scroll).map_or((0.0, 0.0), |sv| (sv.VerticalOffset().unwrap_or(0.0), sv.ViewportHeight().unwrap_or(0.0)));
        Rows { count, off, vh: if vh > 0.0 { vh } else { 900.0 } }
    }

    pub(crate) fn row_tapped(&mut self, x: f64, y: f64, double: bool) {
        let t0 = std::time::Instant::now();
        self.row_tapped_inner(x, y, double);
        crate::state::timing("row tap", t0);
    }

    fn row_tapped_inner(&mut self, x: f64, y: f64, double: bool) {
        let k = self.rows_now(Which::Main).at(y);
        let Some(&(n, d)) = self.rows.get(k) else { return };
        let double = if self.view == View::Search { self.tap_is_double((n, d), double) } else { double };
        if self.view == View::Search {
            if d > 0 {
                self.more_tapped(k, d - 1, n, double);
                return;
            }
            self.search.more.sel = None;
            self.search.more.picks.clear();
        }
        let on_chevron = self.view == View::Tree && x < (8 + d * 16 + 20) as f64;
        // Ctrl adds or takes away a row, Shift picks the run from the selected row
        let (ctrl, shift) = if double || on_chevron { (false, false) } else { crate::win32::ctrl_shift_down() };
        if shift {
            let from = self.selected.and_then(|s| self.rows.iter().position(|r| r.0 == s && self.scan_row(r))).unwrap_or(k);
            let (a, b) = (from.min(k), from.max(k));
            self.marked = self.rows[a..=b].iter().filter(|r| self.scan_row(r)).map(|r| r.0).collect();
            self.refresh_rows();
            self.show_selection();
            return;
        }
        if ctrl {
            if let Some(s) = self.selected {
                self.marked.insert(s);
            }
            if !self.marked.remove(&n) {
                self.marked.insert(n);
            }
            self.selected = Some(n);
            self.refresh_rows();
            self.show_selection();
            return;
        }
        if double && self.view == View::Search {
            self.marked.clear();
            self.selected = Some(n);
            self.refresh_rows();
            if let Some(p) = self.tree.as_ref().map(|t| t.path_of(n)) {
                self.search_open(&p);
            }
            return;
        }
        self.marked.clear();
        self.selected = Some(n);
        if self.view == View::Tree && (double || on_chevron) {
            self.toggle(n);
        }
        self.focus_main();
        self.refresh_rows();
        self.show_selection();
    }

    fn toggle(&mut self, n: usize) {
        let is_dir = self.tree.as_ref().is_some_and(|t| t.nodes[n].is_dir);
        if is_dir && !self.expanded.remove(&n) {
            self.expanded.insert(n);
        }
    }

    pub(crate) fn focus_main(&self) {
        if let Ok(sv) = find::<UIElement>(&self.root, "TreeScroll") {
            let _ = sv.Focus(FocusState::Pointer);
        }
    }

    pub(crate) fn ext_tapped(&mut self, y: f64) {
        let k = self.rows_now(Which::Exts).at(y);
        let Some(e) = self.exts.get(k) else { return };
        self.ext_selected = if self.ext_selected.as_deref() == Some(e.ext.as_str()) { None } else { Some(e.ext.clone()) };
        self.refresh_exts();
        // so the arrow keys work here next
        if let Ok(sv) = find::<UIElement>(&self.root, "ExtScroll") {
            let _ = sv.Focus(FocusState::Pointer);
        }
    }

    /// Arrow keys, Page Up/Down, Home/End and Escape in the types table. True when used.
    pub(crate) fn ext_key(&mut self, key: i32) -> bool {
        if self.exts.is_empty() {
            return false;
        }
        let last = self.exts.len() - 1;
        let k = self.ext_selected.as_ref().and_then(|s| self.exts.iter().position(|e| &e.ext == s));
        let target = match (key, k) {
            (27, _) => {
                self.ext_selected = None;
                self.refresh_exts();
                return true;
            }
            (38, Some(k)) => k.saturating_sub(1),
            (40, Some(k)) => (k + 1).min(last),
            (33, Some(k)) => k.saturating_sub(20),
            (34, Some(k)) => (k + 20).min(last),
            (38 | 40 | 33 | 34, None) | (36, _) => 0,
            (35, _) => last,
            _ => return false,
        };
        self.ext_selected = Some(self.exts[target].ext.clone());
        self.refresh_exts();
        if let Some(y) = self.rows_now(Which::Exts).off_to_show(target) {
            if let Ok(sv) = find::<ScrollViewer>(&self.root, "ExtScroll") {
                let _ = sv.ChangeView(None, Some(y), None);
            }
        }
        true
    }

    /// The scan's node `n` is about to get the row menu (a right-click on its row or its tile on
    /// the map): the menu's commands act on it, as [`menu_pick`] leaves the picks, and the
    /// Windows menu is on only when it would act on what the others do.
    pub(crate) fn menu_on_node(&mut self, n: usize) {
        let search = self.view == View::Search;
        menu_pick(&mut self.marked, &mut self.search.more.sel, &mut self.search.more.picks, search, n);
        self.selected = Some(n);
        self.refresh_rows();
        self.show_selection();
        self.row_menu_state(true);
    }

    /// The row menu's rows as they are for what was right-clicked: a row (or tile) of the scan on
    /// screen (`scan`), or a Search result from another drive or a saved scan. The items that need
    /// the scan are shown, off, saying why; those that go by path stay on, on every pick.
    pub(crate) fn row_menu_state(&mut self, scan: bool) {
        let search = self.view == View::Search;
        use crate::bindings::Microsoft::UI::Xaml::Controls::Control;
        for &(name, on, off) in crate::search::SCAN_ONLY {
            if let Ok(i) = find::<Control>(&self.root, name) {
                let _ = i.SetIsEnabled(crate::search::item_applies(name, scan));
            }
            crate::menu::set_tip(&self.root, name, if scan { on } else { off });
        }
        // overwrite runs on the scan in Tree View and File View, never in Search
        if let Ok(i) = find::<Control>(&self.root, "cOverwrite") {
            let _ = i.SetIsEnabled(!search);
        }
        crate::menu::set_tip(&self.root, "cOverwrite", if search { crate::search::OVERWRITE_ONLY } else { "Write over every file in it once with random data, then delete it" });
        // the tree's own item and the folder a row is in are Search's
        for name in ["cInTree", "cScan"] {
            if let Ok(i) = find::<UIElement>(&self.root, name) {
                let _ = i.SetVisibility(if search { Visibility::Visible } else { Visibility::Collapsed });
            }
        }
        // with several rows picked the Windows menu is off: it would act on one of them only
        if let Ok(i) = find::<Control>(&self.root, "cShell") {
            // ...nor during a clean-up, nor as administrator unless Settings > Scanning allows it
            let admin_ok = !crate::admin::elevated() || self.admin_on(&crate::admin::SHELL_MENU);
            let _ = i.SetIsEnabled(shell_allowed(self.menu_pick_count(scan), self.advisor.cleanup.running.is_some(), admin_ok));
        }
    }

    /// How many results the row menu acts on: the scan's picked nodes, or another drive's picks
    /// with the row right-clicked.
    fn menu_pick_count(&self, scan: bool) -> usize {
        if scan {
            return self.picked_nodes().len();
        }
        let m = &self.search.more;
        let mut n = m.picks.len();
        if let Some((k, id)) = m.sel {
            if let Some((ix, i)) = m.item(k, id) {
                if !m.picks.contains_key(&crate::search_delete::key(&ix.path_of(i))) {
                    n += 1;
                }
            }
        }
        n
    }

    /// Right-click on a row: select it and open the row menu at the pointer (`by_key`: the menu
    /// key or Shift+F10, at the row: [`State::row_menu_by_key`]).
    pub(crate) fn row_menu(&mut self, y_content: f64, at: Point, by_key: bool) {
        let k = self.rows_now(Which::Main).at(y_content);
        let Some(&(n, d)) = self.rows.get(k) else { return };
        if self.view == View::Search && d > 0 {
            let Some((ix, i)) = self.search.more.item(d - 1, n) else { return };
            let path = ix.path_of(i);
            // a row among the picks keeps them (the menu acts on them all); another is alone
            crate::search_delete::menu_keeps_picks(&mut self.search.more.picks, &path);
            self.search.more.sel = Some((d - 1, n));
            // as more_tapped does: a Delete afterwards must not act on a scan row
            self.selected = None;
            self.marked.clear();
            self.refresh_rows();
            self.row_menu_state(false);
            // Shift+right-click: straight to the Windows menu, with one result
            if !by_key && crate::win32::ctrl_shift_down().1 && self.menu_pick_count(false) <= 1 {
                self.shell_menu(path, false);
                return;
            }
            if let Ok(target) = find::<UIElement>(&self.root, "TreeScroll") {
                if !by_key {
                    let _ = target.Focus(FocusState::Pointer);
                }
                crate::menu::show_at("RowMenu", &target, at, by_key);
            }
            return;
        }
        self.menu_on_node(n);
        // Shift+right-click: straight to the Windows menu (Shift+F10 is the keyboard's menu key)
        if !by_key && crate::win32::ctrl_shift_down().1 && self.picked_nodes().len() <= 1 {
            if let Some(p) = self.selected_path() {
                self.shell_menu(p, true);
                return;
            }
        }
        // the table takes the focus, as a list does under a right-click: it is where the focus
        // goes back to when the menu closes
        if let Ok(target) = find::<UIElement>(&self.root, "TreeScroll") {
            if !by_key {
                let _ = target.Focus(FocusState::Pointer);
            }
            crate::menu::show_at("RowMenu", &target, at, by_key);
        }
    }

    /// The menu key or Shift+F10 on the table: the menu of the keyboard's row, its top left
    /// corner under the row, in from its left edge (where the name starts).
    pub(crate) fn row_menu_by_key(&mut self) {
        if self.rows.is_empty() || (self.tree.is_none() && self.view != View::Search) {
            return;
        }
        let k = self.key_row().min(self.rows.len() - 1);
        let r = self.rows_now(Which::Main);
        // the row in view first, as the arrow keys leave it
        if let Some(off) = r.off_to_show(k) {
            if let Ok(sv) = find::<ScrollViewer>(&self.root, "TreeScroll") {
                let _ = sv.ChangeView(None, Some(off), None);
            }
        }
        let r = Rows { off: r.off_to_show(k).unwrap_or(r.off), ..r };
        let y = r.y_of(k);
        let at = Point { X: 32.0, Y: (y - r.off + ROW_H) as f32 };
        self.row_menu(y + ROW_H / 2.0, at, true);
    }

    /// Arrow keys and friends in the main table. Returns true when the key was used.
    /// A row of the scan on screen (in Search, a row can be from another drive).
    pub(crate) fn scan_row(&self, r: &(usize, usize)) -> bool {
        self.view != View::Search || r.1 == 0
    }

    /// Enter or Down in the search box: keep the row the keyboard is on if it is still listed,
    /// otherwise take the first one. A row from another drive is only looked at (`more.sel`),
    /// never put in `selected`/`marked`, which index the scan's tree.
    pub(crate) fn search_box_down(&mut self) {
        let Some(&first) = self.rows.first() else { return };
        let on_listed = match self.search.more.sel.filter(|_| self.view == View::Search) {
            Some((p, n)) => self.rows.iter().any(|r| r.1 == p + 1 && r.0 == n),
            None => self.selected.is_some_and(|s| self.rows.iter().any(|r| r.0 == s && self.scan_row(r))),
        };
        if !on_listed {
            if self.scan_row(&first) {
                self.search.more.sel = None;
                self.search.more.picks.clear();
                self.selected = Some(first.0);
            } else {
                self.search.more.sel = Some((first.1 - 1, first.0));
                self.search.more.picks.clear();
                self.selected = None;
                self.marked.clear();
            }
        }
        self.refresh_rows();
        self.scroll_to_selected();
        self.show_selection();
        self.focus_main();
    }

    /// The row the keyboard is on.
    fn key_row(&self) -> usize {
        if let Some((p, n)) = self.search.more.sel.filter(|_| self.view == View::Search) {
            if let Some(k) = self.rows.iter().position(|r| r.1 == p + 1 && r.0 == n) {
                return k;
            }
        }
        self.selected.and_then(|n| self.rows.iter().position(|r| r.0 == n && self.scan_row(r))).unwrap_or(0)
    }

    /// The name on row `k`, whichever tree it is from.
    fn row_name(&self, k: usize) -> String {
        let Some(&(n, d)) = self.rows.get(k) else { return String::new() };
        if !self.scan_row(&(n, d)) {
            return self.search.more.item(d - 1, n).map(|(ix, i)| ix.name(i).to_lowercase()).unwrap_or_default();
        }
        self.tree.as_ref().map(|t| t.nodes[n].name.to_lowercase()).unwrap_or_default()
    }

    pub(crate) fn table_key(&mut self, key: i32) -> bool {
        if self.tree.is_none() && self.view != View::Search {
            return false;
        }
        let k = self.key_row();
        let last = self.rows.len().saturating_sub(1);
        let page = 20;
        let mut target = k;
        let (ctrl, shift) = crate::win32::ctrl_shift_down();
        match key {
            // Esc drops the picks; Ctrl+A picks every row
            27 if !self.marked.is_empty() || (self.view == View::Search && !self.search.more.picks.is_empty()) => {
                self.marked.clear();
                self.search.more.picks.clear();
                self.refresh_rows();
                self.show_selection();
                return true;
            }
            // Esc with nothing picked: in Search, back to the box
            27 if self.view == View::Search => {
                self.search_box_focus();
                return true;
            }
            // (in Search, on a row from another drive: every such row, up to the most that
            // can be picked; never together with the scan's rows)
            65 if ctrl && self.view == View::Search && self.rows.get(k).is_some_and(|r| !self.scan_row(r)) => {
                self.selected = None;
                self.marked.clear();
                self.search.more.picks.clear();
                let last = self.rows.len().saturating_sub(1);
                self.more_pick_run(0, last);
                self.more_picked_note();
                self.refresh_rows();
                return true;
            }
            65 if ctrl => {
                self.search.more.picks.clear();
                self.marked = self.rows.iter().filter(|r| self.scan_row(r)).map(|r| r.0).collect();
                self.refresh_rows();
                self.show_selection();
                return true;
            }
            _ => {}
        }
        // moving: Shift extends the picks, otherwise they go
        if matches!(key, 33..=40) {
            if shift {
                self.marked.extend(self.selected);
                // a row from another drive the keyboard is on starts the run of picks there
                if let Some((p, n)) = self.search.more.sel.filter(|_| self.view == View::Search) {
                    if self.search.more.picks.is_empty() {
                        self.more_pick(p, n);
                    }
                }
            } else if !(self.view == View::Tree && matches!(key, 37 | 39)) {
                self.marked.clear();
                self.search.more.picks.clear();
            }
        }
        match key {
            38 => target = k.saturating_sub(1),     // up
            40 => target = (k + 1).min(last),       // down
            33 => target = k.saturating_sub(page),  // page up
            34 => target = (k + page).min(last),    // page down
            36 => target = 0,                       // home
            35 => target = last,                    // end
            // Enter opens; Ctrl+Enter shows it in the tree (another drive's row: in Explorer)
            13 if self.view == View::Search => {
                match self.rows.get(k) {
                    Some(&(n, 0)) if ctrl => self.search_reveal(n),
                    Some(&(n, 0)) => {
                        if let Some(p) = self.tree.as_ref().map(|t| t.path_of(n)) {
                            self.search_open(&p);
                        }
                    }
                    Some(&(n, d)) if ctrl => self.more_command('E', d - 1, n),
                    Some(&(n, d)) => self.more_command('O', d - 1, n),
                    None => {}
                }
                return true;
            }
            39 | 37 | 13 if self.view == View::Tree => {
                let Some(&(n, _)) = self.rows.get(k) else { return false };
                let Some(t) = self.tree.clone() else { return false };
                let node = &t.nodes[n];
                let open = self.expanded.contains(&n);
                match key {
                    39 if node.is_dir && !open => {
                        self.expanded.insert(n);
                    }
                    39 if node.is_dir => target = (k + 1).min(last),
                    37 if open => {
                        self.expanded.remove(&n);
                    }
                    37 => {
                        if let Some(p) = node.parent {
                            self.selected = Some(p);
                            self.refresh_rows();
                            self.scroll_to_selected();
                            self.show_selection();
                            return true;
                        }
                    }
                    13 => self.toggle(n),
                    _ => return false,
                }
            }
            // type-to-find: letters and digits typed within a second jump to the next row whose
            // name starts with them
            48..=57 | 65..=90 if !crate::win32::ctrl_or_alt_down() => {
                thread_local! {
                    static TYPED: std::cell::RefCell<(String, Option<std::time::Instant>)> = const { std::cell::RefCell::new((String::new(), None)) };
                }
                let typed = TYPED.with(|b| {
                    let mut b = b.borrow_mut();
                    if b.1.is_none_or(|at| at.elapsed().as_millis() > 1000) {
                        b.0.clear();
                    }
                    b.0.push((key as u8 as char).to_ascii_lowercase());
                    b.1 = Some(std::time::Instant::now());
                    b.0.clone()
                });
                // one letter moves on to the next match; more letters refine from here
                let start = if typed.chars().count() == 1 { k + 1 } else { k };
                let n = self.rows.len();
                let hit = (0..n).map(|i| (start + i) % n.max(1)).find(|&i| self.row_name(i).starts_with(&typed));
                match hit {
                    Some(i) => target = i,
                    None => return true,
                }
            }
            _ => return false,
        }
        if let Some(&(n, d)) = self.rows.get(target) {
            if !self.scan_row(&(n, d)) {
                // a row from another drive: only looked at, never part of the scan's picks
                self.search.more.sel = Some((d - 1, n));
                self.selected = None;
                self.marked.clear();
                if shift && matches!(key, 33..=40) {
                    // the whole run of rows from other drives from where it started
                    self.more_pick_run(k.min(target), k.max(target));
                    self.more_picked_note();
                }
            } else {
                self.search.more.sel = None;
                self.search.more.picks.clear();
                self.selected = Some(n);
                if shift && matches!(key, 33..=40) {
                    // the whole run from where it started
                    let (a, b) = (k.min(target), k.max(target));
                    let run: Vec<usize> = self.rows[a..=b].iter().filter(|r| self.scan_row(r)).map(|r| r.0).collect();
                    self.marked.extend(run);
                }
            }
        }
        self.refresh_rows();
        self.scroll_to_selected();
        self.show_selection();
        true
    }

    /// Scroll the main table just enough to show the selected row.
    pub(crate) fn scroll_to_selected(&mut self) {
        let k = self.key_row();
        if self.rows.is_empty() {
            return;
        }
        let Ok(sv) = find::<ScrollViewer>(&self.root, "TreeScroll") else { return };
        if let Some(y) = self.rows_now(Which::Main).off_to_show(k) {
            let _ = sv.ChangeView(None, Some(y), None);
        }
    }

    pub(crate) fn show_selection(&self) {
        if let (Some(t), false) = (&self.tree, self.marked.is_empty()) {
            // what is inside another picked folder counts once, as Delete will
            let top = self.picked_nodes();
            let total: u64 = top.iter().map(|&n| t.nodes[n].size).sum();
            text(&self.root, "FootLeft", &format!("{} picked   -   {}   (Delete takes them all; Esc drops the picks)", top.len(), self.size(total)));
            return;
        }
        if let (Some(t), Some(n)) = (&self.tree, self.selected) {
            text(&self.root, "FootLeft", &format!("{}   -   {}", t.path_of(n).display(), self.size(t.nodes[n].size)));
        }
    }
}

/// Whether the row menu's Windows item may open: one result picked (it acts on one), no clean-up
/// running, and the administrator evidence allowed when Sprawl is elevated.
pub(crate) fn shell_allowed(picked: usize, cleanup: bool, admin_ok: bool) -> bool {
    picked <= 1 && !cleanup && admin_ok
}

/// What the row menu's commands (Delete among them) act on after a right-click on the scan's node
/// `n`, by its row or its tile on the map: in Search, never a row from another drive picked
/// before (`more_sel`, `picks`); and `n` alone unless it is one of the rows picked already.
pub(crate) fn menu_pick(marked: &mut std::collections::BTreeSet<usize>, more_sel: &mut Option<(usize, usize)>, picks: &mut crate::search_delete::Picks, search: bool, n: usize) {
    if search {
        *more_sel = None;
        picks.clear();
    }
    if !marked.contains(&n) {
        marked.clear();
    }
}

#[cfg(test)]
mod menu_pick_tests {
    use super::{menu_pick, shell_allowed};
    use crate::search_delete::{Pick, Picks};
    use std::collections::BTreeSet;

    #[test]
    fn the_windows_menu_needs_one_result_no_clean_up_and_the_administrator_switch() {
        assert!(shell_allowed(1, false, true), "one row, nothing running, the switch allows it");
        assert!(!shell_allowed(2, false, true), "it acts on one result only");
        assert!(!shell_allowed(1, true, true), "a clean-up is running");
        assert!(!shell_allowed(1, false, false), "elevated without Settings > Scanning allowing it");
    }

    fn picks() -> Picks {
        let mut p = Picks::default();
        p.add(Pick { path: r"D:\other\a.bin".into(), is_dir: false, size: 1, secs: None, place: 0, id: 1, gen: 0 });
        p
    }

    #[test]
    fn a_node_outside_the_picked_rows_is_acted_on_alone() {
        let (mut marked, mut sel, mut p) = (BTreeSet::from([3, 4]), None, Picks::default());
        menu_pick(&mut marked, &mut sel, &mut p, false, 7);
        assert!(marked.is_empty());
    }

    #[test]
    fn a_node_among_the_picked_rows_keeps_them() {
        let (mut marked, mut sel, mut p) = (BTreeSet::from([3, 4]), None, Picks::default());
        menu_pick(&mut marked, &mut sel, &mut p, false, 4);
        assert_eq!(marked, BTreeSet::from([3, 4]));
    }

    #[test]
    fn in_search_another_drives_pick_goes() {
        let (mut marked, mut sel, mut p) = (BTreeSet::new(), Some((0, 9)), picks());
        menu_pick(&mut marked, &mut sel, &mut p, true, 2);
        assert_eq!(sel, None);
        assert!(p.is_empty());
    }

    #[test]
    fn outside_search_another_drives_pick_is_left_alone() {
        // (not on screen there; Search's own right-click clears it when it opens)
        let (mut marked, mut sel, mut p) = (BTreeSet::new(), Some((0, 9)), picks());
        menu_pick(&mut marked, &mut sel, &mut p, false, 2);
        assert_eq!(sel, Some((0, 9)));
        assert_eq!(p.len(), 1);
    }
}

#[cfg(test)]
mod tests {
    /// File View's header clicks timed on History's newest full scan (B5, next-round.md): each
    /// column, both ways, as `header_clicked` sorts them. Ignored (needs a saved scan, and a
    /// release build to mean anything):
    ///
    ///     cargo test --release -p sprawl bench_files_sort -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_files_sort() {
        let base = sprawl_advisor::history::base_dir().expect("history folder");
        let list = sprawl_advisor::history::list(&base);
        let e = list.iter().rev().find(|e| e.full).expect("a full scan in History");
        let t0 = std::time::Instant::now();
        let tree = sprawl_advisor::history::load(e).expect("load");
        println!("loaded {} nodes in {:.0} ms", tree.nodes.len(), t0.elapsed().as_secs_f64() * 1000.0);
        let p = prepare(tree, Sorts { tree: (2, true), files: (1, true), exts: (4, true) });
        let t1 = std::time::Instant::now();
        let ranks = std::sync::Arc::new(Ranks::of(&p.tree));
        println!("name and folder places made in {:.0} ms (on a worker after the tree shows, once per tree)", t1.elapsed().as_secs_f64() * 1000.0);
        let p = Prepared { ranks, ..p };
        println!("{} files", p.files.len());
        for (c, col) in FILES.iter().enumerate() {
            for desc in [true, false] {
                let mut v = p.files.clone();
                let t = std::time::Instant::now();
                sort_file_list(&p.tree, &p.alloc, &p.ranks, (c, desc), &mut v);
                let took = t.elapsed().as_secs_f64() * 1000.0;
                // in the order the old comparison gives (one comparison per row)
                assert!(in_old_order(&p.tree, &p.alloc, c, desc, &v), "{} desc={desc}", col.title);
                println!("sort by {:<10} {}: {took:>8.0} ms", col.title, if desc { "desc" } else { "asc " });
            }
        }
    }

    /// Names compare as their lowercase forms do, non-ASCII ones too: the scan's sort, other
    /// drives' sort and the seam between them agree.
    #[test]
    fn names_compare_as_lowercase() {
        let names = ["Äpfel", "apfel", "Zebra", "éclair", "Eclair", "ÉCOLE", "b", "Ω", "ω1", "_x", "Ab"];
        for a in names {
            for b in names {
                assert_eq!(super::cmp_names(a, b), a.to_lowercase().cmp(&b.to_lowercase()), "{a} vs {b}");
            }
        }
    }

    use super::*;
    use sprawl_core::Node;

    /// The percentage bar's figure sits on a washed accent, so it reads on whatever accent
    /// the look uses: the fill at 45% over the track, the text in the page's brightest.
    #[test]
    fn a_percentage_reads_on_a_washed_accent() {
        let b = pct_bar(1, 42.0);
        assert!(b.contains(&format!(r##"Background="{}" Opacity="0.45""##, crate::look::accent().fill)), "{b}");
        assert!(b.contains(r#"Foreground="{$text1}""#), "{b}");
        assert!(b.contains(r#"Background="{$line1}""#), "{b}");
        // the track still fills the cell, the fill its share of it
        assert!(b.contains(r##"Width="42.0""##) && b.contains("42.0 %"), "{b}");
        // and the bar loads as XAML with the tokens filled in
        let x = crate::tokens::expand(&b);
        let open = format!("{}{}", '{', '$');
        assert!(!x.contains(&open), "{x}");
    }

    /// The bars option: today's solid accent, the accent's gradient, or a 4 px line at the
    /// bottom; the figure stays where it is in each.
    #[test]
    fn the_percentage_bar_follows_the_bars_option() {
        use crate::tokens::{Bars, Look};
        let fill = crate::look::accent().fill;

        let solid = pct_bar(1, 42.0);
        assert!(solid.contains(&format!(r##"Background="{fill}" Opacity="0.45""##)), "{solid}");

        crate::tokens::set(&Look { bars: Bars::Gradient, ..Look::classic() });
        let g = pct_bar(1, 42.0);
        assert!(g.contains("<LinearGradientBrush StartPoint=\"0,0.5\" EndPoint=\"1,0.5\">"), "{g}");
        assert!(g.contains(&format!(r##"GradientStop Color="{fill}" Offset="0""##)), "{g}");
        assert!(g.contains(&format!(r##"GradientStop Color="{}" Offset="1""##, crate::tokens::lighter(&fill))), "{g}");
        assert!(g.contains(r##"Opacity="0.45""##) && g.contains("42.0 %"), "{g}");

        crate::tokens::set(&Look { bars: Bars::Thin, ..Look::classic() });
        let t = pct_bar(1, 42.0);
        assert_eq!(t.matches(r#"VerticalAlignment="Bottom" Height="4""#).count(), 2, "{t}");
        // the figure keeps its place in the cell
        assert!(t.contains(r#"Text="42.0 %""#) && t.contains(r#"VerticalAlignment="Center""#), "{t}");
        // and every shape loads as XAML with the tokens filled in
        for b in [solid, g, t] {
            let x = crate::tokens::expand(&b);
            assert!(!x.contains(&format!("{}{}", '{', '$')), "{x}");
        }
        crate::tokens::set(&Look::classic());
    }

    /// C:\T with folders b, A, a b, a (and a\x), each holding a file or two.
    fn sample() -> Tree {
        let mut nodes = vec![Node { name: r"C:\T".into(), is_dir: true, ..Default::default() }];
        let mut add = |name: &str, dir: bool, parent: usize, size: u64| {
            let i = nodes.len();
            nodes.push(Node { name: name.into(), is_dir: dir, size, allocated: size, files: u64::from(!dir), parent: Some(parent), ..Default::default() });
            nodes[parent].children.push(i);
            i
        };
        let b = add("b", true, 0, 0);
        let big_a = add("A", true, 0, 0);
        let ab = add("a b", true, 0, 0);
        let a = add("a", true, 0, 0);
        let x = add("x", true, a, 0);
        add("z.log", false, b, 5);
        add("y.txt", false, big_a, 700);
        add("m.log", false, ab, 30);
        add("k.dll", false, a, 1);
        add("j.log", false, x, 90);
        add("i.TXT", false, big_a, 2);
        let mut t = Tree { volumes: Vec::new(), root_path: r"C:\T".into(), nodes, errors: 0, method: Default::default() };
        t.compute_totals();
        t
    }

    /// `v` is in the order the old comparison (`cmp_nodes`, then the name) gives: every row
    /// compares not after the next one.
    pub(super) fn in_old_order(t: &Tree, alloc: &[u64], c: usize, desc: bool, v: &[usize]) -> bool {
        let key = FILES[c].key;
        let cmp = |a: usize, b: usize| {
            let o = if key == Key::Name { cmp_names(&t.nodes[a].name, &t.nodes[b].name) } else { cmp_nodes(t, alloc, key, a, b) };
            if desc { o.reverse() } else { o }
        };
        v.windows(2).all(|w| cmp(w[0], w[1]) != Ordering::Greater)
    }

    #[test]
    fn file_view_sorts_by_ranks_as_it_compared() {
        // ties on purpose: names that differ only in case, equal sizes, the same folder name
        // in two places, a file name that is also a folder name
        let mut nodes = vec![Node { name: r"C:\T".into(), is_dir: true, ..Default::default() }];
        let mut add = |name: &str, dir: bool, parent: usize, size: u64, secs: u64| {
            let i = nodes.len();
            let modified = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs));
            nodes.push(Node { name: name.into(), is_dir: dir, size, allocated: size.div_ceil(4096) * 4096, files: u64::from(!dir), parent: Some(parent), modified, ..Default::default() });
            nodes[parent].children.push(i);
            i
        };
        let a = add("a", true, 0, 0, 0);
        let big_a = add("A b", true, 0, 0, 0);
        let sub = add("x", true, a, 0, 0);
        let sub2 = add("X", true, big_a, 0, 0);
        let names = ["k.dll", "K.dll", "x", "Äpfel.txt", "apfel.txt", "z", "b.log", "B.LOG", "é.txt", "e.txt"];
        let mut k = 0;
        for (j, p) in [0, a, big_a, sub, sub2].into_iter().enumerate() {
            for n in names {
                k += 1;
                add(n, false, p, (k % 4) * 1000 + j as u64, k % 3);
            }
        }
        let mut t = Tree { volumes: Vec::new(), root_path: r"C:\T".into(), nodes, errors: 0, method: Default::default() };
        t.compute_totals();
        let alloc: Vec<u64> = t.nodes.iter().map(|n| n.allocated).collect();
        let ranks = Ranks::of(&t);
        assert!(ranks.fits(&t));
        let files: Vec<usize> = (0..t.nodes.len()).filter(|&i| !t.nodes[i].is_dir).collect();
        for c in 0..FILES.len() {
            for desc in [false, true] {
                let mut fast = files.clone();
                sort_file_list(&t, &alloc, &ranks, (c, desc), &mut fast);
                assert!(in_old_order(&t, &alloc, c, desc, &fast), "{} desc={desc}", FILES[c].title);
                let mut slow = files.clone();
                sort_file_list(&t, &alloc, &Ranks::default(), (c, desc), &mut slow);
                assert!(in_old_order(&t, &alloc, c, desc, &slow));
                let mut fs = fast.clone();
                fs.sort_unstable();
                assert_eq!(fs, files, "every file once");
                // the same column clicked again: turned around, exactly the other way's sort
                let mut turned = fast.clone();
                if reverse_file_list(&t, &alloc, &ranks, c, &mut turned) {
                    let mut sorted = files.clone();
                    sort_file_list(&t, &alloc, &ranks, (c, !desc), &mut sorted);
                    assert_eq!(turned, sorted, "{} turned from desc={desc}", FILES[c].title);
                }
            }
        }
        // a tree with nodes the places weren't made for: not trusted
        assert!(!Ranks::default().fits(&t));
    }

    #[test]
    fn a_folder_sort_by_ranks_is_the_folder_sort() {
        let t = sample();
        let files: Vec<usize> = (0..t.nodes.len()).filter(|&i| !t.nodes[i].is_dir).collect();
        let ranks = sprawl_core::search::folder_ranks(&t);
        for desc in [false, true] {
            // the old way: two paths built for every comparison
            let mut old = files.clone();
            old.sort_by(|&a, &b| {
                let o = cmp_two(&t, &[], a, &t, &[], b, Key::Folder);
                if desc { o.reverse() } else { o }
            });
            let mut keyed = files.clone();
            sort_nodes(&t, &[], Key::Folder, desc, &mut keyed, &[]);
            let mut ranked = files.clone();
            sort_nodes(&t, &[], Key::Folder, desc, &mut ranked, &ranks);
            assert_eq!(keyed, old, "keyed, desc {desc}");
            assert_eq!(ranked, old, "ranked, desc {desc}");
        }
    }

    #[test]
    fn a_prepared_tree_is_what_the_window_worked_out() {
        let t = sample();
        let sorts = Sorts { tree: (1, true), files: (1, true), exts: (4, true) };
        let p = prepare(t.clone(), sorts);
        let p = Prepared { tree: p.tree.clone(), ..p };
        // every file, biggest first
        let names: Vec<&str> = p.files.iter().map(|&i| p.tree.nodes[i].name.as_str()).collect();
        assert_eq!(names, ["y.txt", "j.log", "m.log", "z.log", "i.TXT", "k.dll"]);
        // children by size, biggest first
        let kids: Vec<&str> = p.tree.nodes[ROOT].children.iter().map(|&i| p.tree.nodes[i].name.as_str()).collect();
        assert_eq!(kids, ["A", "a", "a b", "b"]);
        // the file types and their colours (by size: txt, log, dll)
        let rows = ext_rows(&t, &p.alloc);
        assert_eq!(p.exts.len(), rows.len());
        assert_eq!(p.colours.get("txt"), Some(&0));
        assert_eq!(p.colours.get("log"), Some(&1));
        assert_eq!(p.alloc.len(), p.tree.nodes.len());
    }

    #[test]
    fn a_long_list_maps_its_scroll_onto_the_rows() {
        // short: the content is the rows
        let g = Rows { count: 1000, off: 240.0, vh: 300.0 };
        assert!(!g.scaled());
        assert_eq!(g.body(), 24_000.0);
        assert_eq!(g.top(), 10.0);
        assert_eq!(g.y_of(12), 288.0);
        assert_eq!(g.at(289.0), 12);
        assert_eq!(g.off_to_show(3), Some(72.0));
        assert_eq!(g.off_to_show(11), None);
        assert_eq!(g.off_to_show(30), Some(30.0 * 24.0 + 24.0 - 300.0));
        // long: the content stays short enough for XAML to clip, and every row is reachable
        let n = 500_000;
        let top = Rows { count: n, off: 0.0, vh: 300.0 };
        assert!(top.scaled() && top.body() <= MAX_BODY);
        assert_eq!(top.at(5.0), 0);
        let end = Rows { count: n, off: top.body() - 300.0, vh: 300.0 };
        let last_seen = end.at(end.off + 299.0);
        assert_eq!(last_seen, n - 1, "the last row is at the bottom of the full scroll");
        // rows stay a row apart on screen, and a tap finds the row drawn there
        let mid = Rows { count: n, off: 400_000.0, vh: 300.0 };
        let k = mid.at(mid.off + 100.0);
        assert!((mid.y_of(k + 1) - mid.y_of(k) - ROW_H).abs() < 1e-6);
        assert_eq!(mid.at(mid.y_of(k) + 1.0), k);
        // scrolling to a row puts it in view
        let to = mid.off_to_show(123_456).unwrap();
        let there = Rows { off: to, ..mid };
        let y = there.y_of(123_456);
        assert!(y >= there.off - 1e-6 && y + ROW_H <= there.off + there.vh + 1e-6, "{y} in {}..{}", there.off, there.off + there.vh);
    }

    #[test]
    fn the_filter_box_and_what_is_on_screen() {
        use FilterStep::*;
        // shown already, or being made: nothing
        assert_eq!(filter_step("*.dll", Some("*.dll"), None, false), Nothing);
        assert_eq!(filter_step("*.dll", None, Some("*.dll"), false), Nothing);
        // the review's case: a filter on, the box cleared, the tree changed meanwhile (the
        // pattern applied was forgotten): the full tree comes back, it doesn't stay filtered
        assert_eq!(filter_step("", Some("*.dll"), None, true), Unfilter);
        assert_eq!(filter_step("", Some(""), None, false), Unfilter);
        assert_eq!(filter_step("", None, None, true), Nothing);
        // the tree under a filter changed: made again, same pattern or not
        assert_eq!(filter_step("*.dll", Some("*.dll"), None, true), Filter);
        assert_eq!(filter_step("*.dll", Some("*.dll"), Some("*.dll"), true), Filter);
        // typed on
        assert_eq!(filter_step("*.exe", Some("*.dll"), Some("*.dll"), false), Filter);
    }

    #[test]
    fn header_hit_test() {
        // fixed 118+92+92+84+84+150 = 620; a 1000 wide header leaves 380 for the name column
        assert_eq!(col_at(TREE, 1000.0, 10.0), Some(0));
        assert_eq!(col_at(TREE, 1000.0, 379.0), Some(0));
        assert_eq!(col_at(TREE, 1000.0, 381.0), Some(1));
        assert_eq!(col_at(TREE, 1000.0, 849.0), Some(5));
        assert_eq!(col_at(TREE, 1000.0, 999.0), Some(6));
        assert_eq!(col_at(TREE, 1000.0, 1001.0), None);
    }

    #[test]
    fn names_compare_without_case() {
        assert_eq!(cmp_names("apple", "Banana"), Ordering::Less);
        assert_eq!(cmp_names("ABC", "abc"), Ordering::Equal);
    }
}

/// The tables' sort columns, as a worker preparing a tree needs them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Sorts {
    pub(crate) tree: (usize, bool),
    pub(crate) files: (usize, bool),
    pub(crate) exts: (usize, bool),
}

/// Everything the window shows for a tree that takes long to work out on a big scan: the
/// children in order, the allocated sizes, every file biggest first and the file-type table.
/// Made on a worker (a scan's own thread, or the filter's), so the window doesn't stop for
/// seconds when a scan ends.
pub(crate) struct Prepared {
    pub(crate) tree: std::sync::Arc<Tree>,
    pub(crate) alloc: Vec<u64>,
    pub(crate) ranks: std::sync::Arc<Ranks>,
    pub(crate) files: Vec<usize>,
    pub(crate) exts: Vec<crate::state::ExtRow>,
    /// Each extension's colour, by size (before the table's own sort).
    pub(crate) colours: std::collections::HashMap<String, usize>,
    pub(crate) sorts: Sorts,
}

pub(crate) fn prepare(mut tree: Tree, sorts: Sorts) -> Prepared {
    let alloc: Vec<u64> = tree.nodes.iter().map(|n| n.allocated).collect();
    sort_children(&mut tree, &alloc, sorts.tree);
    // the name and folder places are made after the tree is on screen (State::ranks_start):
    // here they would hold a finished scan back by seconds
    let ranks = std::sync::Arc::new(Ranks::default());
    let mut files = sprawl_core::largest_files(&tree, usize::MAX);
    sort_file_list(&tree, &alloc, &ranks, sorts.files, &mut files);
    let mut exts = ext_rows(&tree, &alloc);
    let colours = exts.iter().take(paint::COUNT).enumerate().map(|(i, e)| (e.ext.clone(), i)).collect();
    sort_ext_rows(&mut exts, sorts.exts);
    Prepared { tree: std::sync::Arc::new(tree), alloc, ranks, files, exts, colours, sorts }
}

/// What the window worked out for the full tree while a filter shows part of it: kept, so
/// clearing the filter shows the full tree again at once, without a copy or a worker.
pub(crate) struct Parts {
    pub(crate) alloc: Vec<u64>,
    pub(crate) ranks: std::sync::Arc<Ranks>,
    pub(crate) files: Vec<usize>,
    pub(crate) exts: Vec<crate::state::ExtRow>,
    pub(crate) colours: std::collections::HashMap<String, usize>,
    pub(crate) sorts: Sorts,
}

impl Parts {
    /// The full tree changed (a clean-up's rescans, `alloc` already follows): the rest is
    /// worked out again, as the window does for the tree on screen.
    pub(crate) fn redo(&mut self, t: &mut Tree) {
        sort_children(t, &self.alloc, self.sorts.tree);
        // places that no longer fit (the rescan added nodes) sort the slow way here, and are
        // made again when this tree is shown
        self.files = sprawl_core::largest_files(t, usize::MAX);
        sort_file_list(t, &self.alloc, &self.ranks, self.sorts.files, &mut self.files);
        self.exts = ext_rows(t, &self.alloc);
        sort_ext_rows(&mut self.exts, self.sorts.exts);
    }
}

/// The tallest the scrolled rows of a table are made. Past about two million pixels XAML stops
/// clipping them to the view, and the last rows paint over what is below the table (a search
/// of 100,000 results was 2.5 million).
pub(crate) const MAX_BODY: f64 = 1_000_000.0;

/// Where the rows of a table of `count` rows sit, scrolled to `off` in a view `vh` high. Up to
/// [`MAX_BODY`] the scrolled content is the rows, one under the other. Past it the content is
/// [`MAX_BODY`] high, its scroll range maps onto the rows evenly, and the rows near the view are
/// drawn a row apart around it. Every position on the content is in content coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rows {
    pub(crate) count: usize,
    pub(crate) off: f64,
    pub(crate) vh: f64,
}

impl Rows {
    pub(crate) fn scaled(&self) -> bool {
        self.count as f64 * ROW_H > MAX_BODY
    }

    /// The height of the scrolled content.
    pub(crate) fn body(&self) -> f64 {
        (self.count as f64 * ROW_H).clamp(1.0, MAX_BODY)
    }

    /// Rows per pixel of scrolling.
    fn ratio(&self) -> f64 {
        if self.scaled() {
            (self.count as f64 - self.vh / ROW_H) / (MAX_BODY - self.vh).max(1.0)
        } else {
            1.0 / ROW_H
        }
    }

    /// The row at the top of the view (with the part of it above).
    pub(crate) fn top(&self) -> f64 {
        self.off * self.ratio()
    }

    /// Where row `k` is drawn.
    pub(crate) fn y_of(&self, k: usize) -> f64 {
        self.off + (k as f64 - self.top()) * ROW_H
    }

    /// The row drawn at `y`.
    pub(crate) fn at(&self, y: f64) -> usize {
        (self.top() + (y - self.off) / ROW_H).floor().max(0.0) as usize
    }

    /// The scroll that shows row `k` with the least movement; None when it is in view.
    pub(crate) fn off_to_show(&self, k: usize) -> Option<f64> {
        let rows_in_view = self.vh / ROW_H;
        let top = self.top();
        let want = if (k as f64) < top {
            k as f64
        } else if (k + 1) as f64 > top + rows_in_view {
            (k + 1) as f64 - rows_in_view
        } else {
            return None;
        };
        Some((want / self.ratio()).clamp(0.0, (self.body() - self.vh).max(0.0)))
    }

    /// The scroll that puts row `k` near the top, `above` rows down from it.
    pub(crate) fn off_near(&self, k: usize, above: f64) -> f64 {
        ((k as f64 - above).max(0.0) / self.ratio()).clamp(0.0, (self.body() - self.vh).max(0.0))
    }
}

/// What the filter box asks for, given what is on screen.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FilterStep {
    /// It is shown already, or being made.
    Nothing,
    /// Show the full tree again.
    Unfilter,
    /// Make this filtered view.
    Filter,
}

/// `shown`: the pattern of the filtered view on screen (None: the full tree); `busy`: the one
/// being made; `again`: the full tree changed, so a filtered view is made again even if it is
/// the same pattern.
pub(crate) fn filter_step(wanted: &str, shown: Option<&str>, busy: Option<&str>, again: bool) -> FilterStep {
    if wanted.is_empty() {
        return if shown.is_some() { FilterStep::Unfilter } else { FilterStep::Nothing };
    }
    if !again && (busy == Some(wanted) || (busy.is_none() && shown == Some(wanted))) {
        return FilterStep::Nothing;
    }
    FilterStep::Filter
}

/// The file-type table, biggest first.
pub(crate) fn ext_rows(t: &Tree, alloc: &[u64]) -> Vec<crate::state::ExtRow> {
    let mut alloc_by: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for (i, n) in t.nodes.iter().enumerate() {
        if !n.is_dir {
            *alloc_by.entry(n.extension()).or_default() += alloc.get(i).copied().unwrap_or(0);
        }
    }
    sprawl_core::extension_stats(t)
        .into_iter()
        .map(|s| crate::state::ExtRow { alloc: alloc_by.get(&s.extension).copied().unwrap_or(0), ext: s.extension, size: s.size, files: s.files, percent: s.percent })
        .collect()
}

pub(crate) fn sort_children(t: &mut Tree, alloc: &[u64], sort: (usize, bool)) {
    for i in 0..t.nodes.len() {
        if t.nodes[i].children.len() < 2 {
            continue;
        }
        let mut kids = std::mem::take(&mut t.nodes[i].children);
        kids.sort_by(child_order(t, alloc, sort));
        t.nodes[i].children = kids;
    }
}

/// How Tree View orders a folder's children for the column `c` (reversed with `desc`).
fn child_order<'a>(t: &'a Tree, alloc: &'a [u64], (c, desc): (usize, bool)) -> impl Fn(&usize, &usize) -> Ordering + 'a {
    let key = TREE[c].key;
    move |&a, &b| {
        let o = if key == Key::Name { cmp_names(&t.nodes[a].name, &t.nodes[b].name) } else { cmp_nodes(t, alloc, key, a, b) };
        if desc { o.reverse() } else { o }
    }
}

/// Every node's place in name order and every folder's place in path order, both as the
/// tables compare them (`cmp_names`, the folder's full path lowercased). Worked out once per
/// tree, on the worker that prepares it, so a File View header click sorts numbers: a few
/// hundred ms for millions of files, where comparing names and building paths took seconds
/// (and a minute and a half by folder on a full C: drive).
#[derive(Default)]
pub(crate) struct Ranks {
    names: Vec<u32>,
    folders: Vec<u32>,
}

impl Ranks {
    pub(crate) fn of(t: &Tree) -> Ranks {
        // the two are independent: the folders' on a second thread
        std::thread::scope(|s| {
            let folders = s.spawn(|| sprawl_core::search::folder_ranks(t));
            let names = Self::names(t);
            Ranks { names, folders: folders.join().unwrap_or_default() }
        })
    }

    fn names(t: &Tree) -> Vec<u32> {
        let mut idx: Vec<u32> = (0..t.nodes.len() as u32).collect();
        idx.sort_unstable_by(|&a, &b| cmp_names(&t.nodes[a as usize].name, &t.nodes[b as usize].name));
        let mut names = vec![0u32; t.nodes.len()];
        // names that compare equal (differ only in case) share a place
        let mut place = 0u32;
        for (i, &n) in idx.iter().enumerate() {
            if i > 0 && cmp_names(&t.nodes[idx[i - 1] as usize].name, &t.nodes[n as usize].name) != Ordering::Equal {
                place = i as u32;
            }
            names[n as usize] = place;
        }
        names
    }

    /// Made for this tree (a rescan that added nodes leaves them short: they are made again).
    pub(crate) fn fits(&self, t: &Tree) -> bool {
        self.names.len() == t.nodes.len() && self.folders.len() == t.nodes.len()
    }
}

/// Sort `files` by `key(n)`, then name, then node number, from a key made once per row.
fn sort_keyed<K: Ord + Copy>(files: &mut [usize], desc: bool, names: &[u32], key: impl Fn(usize) -> K) {
    let mut v: Vec<(K, u32, u32)> = files.iter().map(|&n| (key(n), names[n], n as u32)).collect();
    if desc {
        v.sort_unstable_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)).then(a.2.cmp(&b.2)));
    } else {
        v.sort_unstable();
    }
    for (slot, (_, _, n)) in files.iter_mut().zip(v) {
        *slot = n as usize;
    }
}

/// Turn `files`, sorted by `sort_keyed` on one key, into its sort the other way round: reversed,
/// then each run that sorts equal (same key and name place) turned back, since both ways break
/// those ties by node index, lowest first. The same order sorting would give, in linear time.
fn reverse_keyed<K: Eq>(files: &mut [usize], names: &[u32], key: impl Fn(usize) -> K) {
    files.reverse();
    let mut i = 0;
    while i < files.len() {
        let (k, name) = (key(files[i]), names[files[i]]);
        let mut j = i + 1;
        while j < files.len() && names[files[j]] == name && key(files[j]) == k {
            j += 1;
        }
        files[i..j].reverse();
        i = j;
    }
}

/// File View's list, sorted on column `c` one way, turned to the other way without sorting it
/// again ([`reverse_keyed`]). False when that can't be done exactly (no name places for this
/// tree, or a column sorted by comparing): then it is sorted as usual.
pub(crate) fn reverse_file_list(t: &Tree, alloc: &[u64], ranks: &Ranks, c: usize, files: &mut [usize]) -> bool {
    if !ranks.fits(t) {
        return false;
    }
    let names = &ranks.names;
    match FILES[c].key {
        Key::Name => reverse_keyed(files, names, |_| ()),
        Key::Size => reverse_keyed(files, names, |n| t.nodes[n].size),
        Key::Alloc => reverse_keyed(files, names, |n| alloc.get(n).copied()),
        Key::Modified => reverse_keyed(files, names, |n| t.nodes[n].modified),
        Key::Folder => reverse_keyed(files, names, |n| t.nodes[n].parent.map_or(0, |p| u64::from(ranks.folders[p]) + 1)),
        _ => return false,
    }
    true
}

/// File View's sort. With `ranks` made for `t` every column sorts numbers; without (they
/// don't fit) it compares as before, which is right but slow on millions of files.
pub(crate) fn sort_file_list(t: &Tree, alloc: &[u64], ranks: &Ranks, (c, desc): (usize, bool), files: &mut [usize]) {
    let key = FILES[c].key;
    if ranks.fits(t) {
        let names = &ranks.names;
        match key {
            Key::Name => return sort_keyed(files, desc, names, |_| ()),
            Key::Size => return sort_keyed(files, desc, names, |n| t.nodes[n].size),
            Key::Alloc => return sort_keyed(files, desc, names, |n| alloc.get(n).copied()),
            Key::Modified => return sort_keyed(files, desc, names, |n| t.nodes[n].modified),
            // a file's folder by its place; in the same folder, by name (as comparing paths did)
            Key::Folder => return sort_keyed(files, desc, names, |n| t.nodes[n].parent.map_or(0, |p| u64::from(ranks.folders[p]) + 1)),
            _ => {}
        }
    }
    files.sort_by(|&a, &b| {
        let o = if key == Key::Name { cmp_names(&t.nodes[a].name, &t.nodes[b].name) } else { cmp_nodes(t, alloc, key, a, b) };
        if desc { o.reverse() } else { o }
    });
}

pub(crate) fn sort_ext_rows(exts: &mut [crate::state::ExtRow], (c, desc): (usize, bool)) {
    let key = EXTS[c].key;
    exts.sort_by(|a, b| {
        let o = match key {
            Key::Size => a.size.cmp(&b.size),
            Key::Alloc => a.alloc.cmp(&b.alloc),
            Key::Files => a.files.cmp(&b.files),
            Key::Type => cmp_names(&file_type(&a.ext), &file_type(&b.ext)),
            _ => Ordering::Equal,
        }
        .then_with(|| cmp_names(&a.ext, &b.ext));
        if desc { o.reverse() } else { o }
    });
}
