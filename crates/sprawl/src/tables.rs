//! The folder tree, the file list and the file-type table. Columns sort when their header is
//! clicked, only the rows on screen (plus some either side) exist as XAML, rows have a
//! right-click menu, and the tree can be driven from the keyboard.

use std::cmp::Ordering;

use sprawl_core::{Tree, ROOT};
use windows_core::Interface;

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{FlyoutBase, FlyoutShowOptions};
use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, ScrollViewer};
use crate::bindings::Microsoft::UI::Xaml::{DependencyObject, FocusState, FrameworkElement, UIElement};
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
    /// (column, descending)
    pub(crate) tree_sort: (usize, bool),
    pub(crate) files_sort: (usize, bool),
    pub(crate) ext_sort: (usize, bool),
    main_body: Option<Border>,
    ext_body: Option<Border>,
    main_span: (usize, usize, usize),
    ext_span: (usize, usize, usize),
    /// The rows' XAML as last loaded (main table, file types): the same text is not loaded
    /// again, so rows that didn't change aren't replaced (other drives' results arrive every
    /// half second; replacing the rows lost a double-click and the scroll's smoothness).
    drawn: [String; 2],
}

impl Default for Tables {
    fn default() -> Self {
        Tables {
            tree_sort: (2, true),
            files_sort: (1, true),
            ext_sort: (4, true),
            main_body: None,
            ext_body: None,
            main_span: (0, 0, usize::MAX),
            ext_span: (0, 0, usize::MAX),
            drawn: Default::default(),
        }
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
    let mut s = format!(r##"<Grid {} Height="30" Background="#30000000">{}"##, ui::NS, col_defs(cols));
    for (i, c) in cols.iter().enumerate() {
        let align = if c.right { "Right" } else { "Left" };
        let arrow = if i == sort.0 { if sort.1 { "&#xE70D;" } else { "&#xE70E;" } } else { "" };
        let colour = if i == sort.0 { "#FFE6EBF5" } else { "#FFB9C2D6" };
        s += &format!(
            r##"<StackPanel Grid.Column="{i}" Orientation="Horizontal" Spacing="5" HorizontalAlignment="{align}" Margin="10,0,10,0"><TextBlock Text="{}" FontSize="12" FontWeight="SemiBold" Foreground="{colour}" VerticalAlignment="Center"/><FontIcon Glyph="{arrow}" FontSize="12" Foreground="#FF4CC2FF" VerticalAlignment="Center"/></StackPanel>"##,
            ui::esc(c.title)
        );
    }
    s + r##"<Border Grid.ColumnSpan="9" Height="1" VerticalAlignment="Bottom" Background="#22FFFFFF"/></Grid>"##
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
    format!(r##"<TextBlock Grid.Column="{col}" Text="{}" {} FontSize="12" HorizontalAlignment="Right" VerticalAlignment="Center" Margin="0,0,10,0"/>"##, ui::esc(s), crate::look::NUM)
}

pub(crate) fn pct_bar(col: usize, pct: f64, a: &str, b: &str) -> String {
    let w = pct.clamp(0.0, 100.0);
    let num = crate::look::NUM;
    format!(
        r##"<Grid Grid.Column="{col}" Margin="6,4,6,4"><Border CornerRadius="3" Background="#18FFFFFF"/><Border CornerRadius="3" HorizontalAlignment="Left" Width="{w:.1}"><Border.Background><LinearGradientBrush StartPoint="0,0" EndPoint="1,0"><GradientStop Color="{a}" Offset="0"/><GradientStop Color="{b}" Offset="1"/></LinearGradientBrush></Border.Background></Border><TextBlock Text="{pct:.1} %" {num} FontSize="12" HorizontalAlignment="Right" VerticalAlignment="Center" Margin="0,0,6,0"/></Grid>"##
    )
}

impl State {
    // ------------------------------------------------------------ sorting

    fn main_cols(&self) -> (&'static [Col], (usize, bool)) {
        if self.view == View::Files {
            (FILES, self.tables.files_sort)
        } else if self.view == View::Search {
            (FILES_LIKE, self.search.sort)
        } else {
            (TREE, self.tables.tree_sort)
        }
    }

    /// Order every folder's children by the tree view's sort column.
    pub(crate) fn sort_tree(&mut self) {
        let sort = self.tables.tree_sort;
        let Some(t) = self.tree.as_mut().map(std::sync::Arc::make_mut) else { return };
        sort_children(t, &self.alloc, sort);
    }

    pub(crate) fn sort_files(&mut self) {
        let Some(t) = &self.tree else { return };
        sort_file_list(t, &self.alloc, self.tables.files_sort, &mut self.files);
    }

    pub(crate) fn sort_exts(&mut self) {
        sort_ext_rows(&mut self.exts, self.tables.ext_sort);
    }

    /// A click on a table header at `x` (header coordinates).
    pub(crate) fn header_clicked(&mut self, which: Which, x: f64, width: f64) {
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
                flip(&mut self.tables.files_sort);
                self.sort_files();
                self.refresh_rows();
            }
            Which::Main if self.view == View::Search => {
                flip(&mut self.search.sort);
                self.search_sort();
                self.refresh_rows();
            }
            Which::Main => {
                flip(&mut self.tables.tree_sort);
                self.sort_tree();
                self.refresh_rows();
            }
        }
    }

    // ------------------------------------------------------------ rows

    pub(crate) fn visible_rows(&self) -> Vec<(usize, usize)> {
        let t = self.tree.as_ref().unwrap();
        let mut out = Vec::new();
        let mut stack = vec![(ROOT, 0usize)];
        while let Some((n, d)) = stack.pop() {
            out.push((n, d));
            if self.expanded.contains(&n) {
                for &c in t.nodes[n].children.iter().rev() {
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
        self.rows = match self.view {
            View::Files => self.files.iter().map(|&f| (f, 0)).collect(),
            View::Search => self.search_rows(),
            _ => self.visible_rows(),
        };
        let (cols, sort) = self.main_cols();
        if let (Ok(h), Ok(head)) = (find::<Border>(&self.root, "TreeHeader"), load::<UIElement>(&header(cols, sort))) {
            let _ = h.SetChild(&head);
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
        self.fill(Which::Main);
    }

    pub(crate) fn refresh_exts(&mut self) {
        if let (Ok(h), Ok(head)) = (find::<Border>(&self.root, "ExtHeader"), load::<UIElement>(&header(EXTS, self.tables.ext_sort))) {
            let _ = h.SetChild(&head);
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
                let body = if which == Which::Main { &self.tables.main_body } else { &self.tables.ext_body };
                if let Some(fe) = body.as_ref().and_then(|b| b.Child().ok()).and_then(|c| c.cast::<FrameworkElement>().ok()) {
                    let _ = fe.SetMargin(crate::bindings::Microsoft::UI::Xaml::Thickness { Left: 0.0, Top: g.y_of(span.0), Right: 0.0, Bottom: 0.0 });
                }
            }
            return;
        }
        let a = first.saturating_sub(OVERSCAN).min(count);
        let b = (last + OVERSCAN).min(count);

        // The tall, empty content that gives the scroll bar its range; rows sit inside it.
        let body = {
            let slot = if which == Which::Main { &mut self.tables.main_body } else { &mut self.tables.ext_body };
            if slot.is_none() {
                if let Ok(b) = load::<Border>(&format!(r##"<Border {} VerticalAlignment="Top" Background="Transparent"/>"##, ui::NS)) {
                    if let Ok(u) = b.cast::<UIElement>() {
                        let _ = sv.SetContent(&u);
                    }
                    *slot = Some(b);
                }
            }
            slot.clone()
        };
        let Some(body) = body else { return };
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
        let slot = if which == Which::Main { 0 } else { 1 };
        let span = (a, b, count);
        if self.tables.drawn[slot] == xaml && body.Child().is_ok() {
            if which == Which::Main {
                self.tables.main_span = span;
            } else {
                self.tables.ext_span = span;
            }
            return;
        }
        match load::<UIElement>(&xaml) {
            Ok(chunk) => {
                let _ = body.SetChild(&chunk);
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

    fn tree_row(&self, k: usize) -> String {
        let t = self.tree.as_ref().unwrap();
        let (n, d) = self.rows[k];
        let node = &t.nodes[n];
        let bg = self.row_bg(k, Some(n) == self.selected || self.marked.contains(&n));
        let pct = t.percent_of_parent(n);
        let has_kids = node.is_dir && !node.children.is_empty();
        let chevron = if !has_kids { "" } else if self.expanded.contains(&n) { "&#xE70D;" } else { "&#xE76C;" };
        let (glyph, gc) = if n == ROOT {
            ("&#xEDA2;", "#FF9FB4FF".to_string())
        } else if node.is_dir {
            ("&#xE8B7;", "#FFFFC83D".to_string())
        } else {
            ("&#xE7C3;", self.ext_colour_hex(&node.extension()))
        };
        let items = if node.is_dir { thousands(node.items()) } else { String::new() };
        let files = if node.is_dir { thousands(node.files) } else { String::new() };
        let hidden = n != ROOT && (node.is_hidden() || node.is_system());
        let fg = if hidden { "#FF7D8494" } else { "#FFFFFFFF" };
        format!(
            r##"<Grid Height="24" Background="{bg}">{cols}<StackPanel Orientation="Horizontal" Spacing="6" Margin="{indent},0,0,0"><FontIcon Glyph="{chevron}" FontSize="12" Width="12" Foreground="#FF8E97AA"/><FontIcon Glyph="{glyph}" FontSize="14" Foreground="{gc}" Opacity="{op}"/><TextBlock Text="{name}" VerticalAlignment="Center" FontSize="12" Foreground="{fg}" TextTrimming="CharacterEllipsis"/></StackPanel>{bar}{c2}{c3}{c4}{c5}{c6}</Grid>"##,
            cols = col_defs(TREE),
            indent = 8 + d * 16,
            op = if hidden { "0.55" } else { "1" },
            name = ui::esc(&self.label(n)),
            bar = pct_bar(1, pct, "#FF3A7BD5", "#FF00D2FF"),
            c2 = cell(2, &self.size(node.size)),
            c3 = cell(3, &self.size(self.alloc[n])),
            c4 = cell(4, &items),
            c5 = cell(5, &files),
            // the app that owns it; dimmed where it just continues from the parent
            c6 = match self.owner_of(n) {
                // an uninstall mark where the app starts, when Sprawl knows where it is uninstalled
                Some((name, inherited, can_uninstall)) => format!(
                    r##"<TextBlock Grid.Column="6" FontSize="12" Margin="12,0,6,0" VerticalAlignment="Center" TextTrimming="CharacterEllipsis" Foreground="{}">{}<Run Text="{}"/></TextBlock>"##,
                    if inherited { "#FF8E97AA" } else { "#FFB9C2D6" },
                    if can_uninstall && !inherited { r##"<Run Text="&#xE74D; " FontFamily="Segoe Fluent Icons,Segoe MDL2 Assets" FontSize="12" Foreground="#FF8E97AA"/>"## } else { "" },
                    ui::esc(name),
                ),
                None => String::new(),
            },
        )
    }

    fn file_row(&self, k: usize) -> String {
        let t = self.tree.as_ref().unwrap();
        let n = self.rows[k].0;
        let node = &t.nodes[n];
        let bg = self.row_bg(k, Some(n) == self.selected || self.marked.contains(&n));
        let modified = node.modified.map(fmt_time).unwrap_or_default();
        let folder = node.parent.map(|p| t.path_of(p).display().to_string()).unwrap_or_default();
        format!(
            r##"<Grid Height="24" Background="{bg}">{cols}<StackPanel Orientation="Horizontal" Spacing="8" Margin="10,0,0,0"><FontIcon Glyph="&#xE7C3;" FontSize="13" Foreground="{fc}"/><TextBlock Text="{name}" VerticalAlignment="Center" FontSize="12" TextTrimming="CharacterEllipsis"/></StackPanel>{c1}{c2}{c3}<TextBlock Grid.Column="4" Text="{folder}" FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center" Margin="10,0" TextTrimming="CharacterEllipsis"/></Grid>"##,
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
        let bg = self.row_bg(k, self.ext_selected.as_deref() == Some(e.ext.as_str()));
        let shown = if e.ext == "(none)" { String::new() } else { format!(".{}", e.ext) };
        format!(
            r##"<Grid Height="24" Background="{bg}">{cols}<Border Width="12" Height="12" CornerRadius="3" Background="{c}" Margin="8,0,0,0"/><TextBlock Grid.Column="1" Text="{ext}" FontSize="12" VerticalAlignment="Center" Margin="8,0,0,0" TextTrimming="CharacterEllipsis"/><TextBlock Grid.Column="2" Text="{kind}" FontSize="12" Foreground="#FFB9C2D6" VerticalAlignment="Center" Margin="10,0" TextTrimming="CharacterEllipsis"/>{bar}{c4}{c5}{c6}</Grid>"##,
            cols = col_defs(EXTS),
            c = self.ext_colour_hex(&e.ext),
            ext = ui::esc(&shown),
            kind = ui::esc(&file_type(&e.ext)),
            bar = pct_bar(3, e.percent, "#FF8E54E9", "#FFFF5E9C"),
            c4 = cell(4, &self.size(e.size)),
            c5 = cell(5, &self.size(e.alloc)),
            c6 = cell(6, &thousands(e.files)),
        )
    }

    pub(crate) fn row_bg(&self, k: usize, selected: bool) -> &'static str {
        if selected {
            "#FF2C3B63"
        } else if k % 2 == 1 {
            "#08FFFFFF"
        } else {
            "Transparent"
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
        let k = self.rows_now(Which::Main).at(y);
        let Some(&(n, d)) = self.rows.get(k) else { return };
        let double = if self.view == View::Search { self.tap_is_double((n, d), double) } else { double };
        if self.view == View::Search {
            if d > 0 {
                self.more_tapped(d - 1, n, double);
                return;
            }
            self.search.more.sel = None;
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

    /// Right-click on a row: select it and open the row menu at the pointer.
    pub(crate) fn row_menu(&mut self, y_content: f64, at: Point) {
        let k = self.rows_now(Which::Main).at(y_content);
        let Some(&(n, d)) = self.rows.get(k) else { return };
        if self.view == View::Search {
            if d > 0 {
                if crate::win32::ctrl_shift_down().1 {
                    if let Some((ix, i)) = self.search.more.item(d - 1, n) {
                        let p = ix.path_of(i);
                        self.search.more.sel = Some((d - 1, n));
                        self.refresh_rows();
                        self.shell_menu(p, false);
                        return;
                    }
                }
                self.more_menu(d - 1, n, at);
                return;
            }
            self.search.more.sel = None;
        }
        // right-click outside the picked rows picks just that row
        if !self.marked.contains(&n) {
            self.marked.clear();
        }
        self.selected = Some(n);
        self.refresh_rows();
        self.show_selection();
        // with several rows picked the Windows menu is off: it would act on one of them only
        if let Ok(i) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::Control>(&self.root, "cShell") {
            // ...nor during a clean-up, nor as administrator unless Settings > Scanning allows it
            let admin_ok = !sprawl_core::is_elevated() || self.admin_on(&crate::admin::SHELL_MENU);
            let _ = i.SetIsEnabled(self.picked_nodes().len() <= 1 && self.advisor.cleanup.running.is_none() && admin_ok);
        }
        // Shift+right-click: straight to the Windows menu
        if crate::win32::ctrl_shift_down().1 && self.picked_nodes().len() <= 1 {
            if let Some(p) = self.selected_path() {
                self.shell_menu(p, true);
                return;
            }
        }
        if let Ok(i) = find::<UIElement>(&self.root, "cInTree") {
            let _ = i.SetVisibility(if self.view == View::Search { crate::bindings::Microsoft::UI::Xaml::Visibility::Visible } else { crate::bindings::Microsoft::UI::Xaml::Visibility::Collapsed });
        }
        let shown = || -> windows_core::Result<()> {
            let menu: FlyoutBase = find(&self.root, "RowMenu")?;
            let target: DependencyObject = find(&self.root, "TreeScroll")?;
            let opts = FlyoutShowOptions::new()?;
            opts.SetPosition(Some(at))?;
            menu.ShowAtWithOptions(&target, &opts)
        };
        if let Err(e) = shown() {
            eprintln!("row menu: {e:?}");
        }
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
                self.selected = Some(first.0);
            } else {
                self.search.more.sel = Some((first.1 - 1, first.0));
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
            27 if !self.marked.is_empty() => {
                self.marked.clear();
                self.refresh_rows();
                self.show_selection();
                return true;
            }
            // Esc with nothing picked: in Search, back to the box
            27 if self.view == View::Search => {
                self.search_box_focus();
                return true;
            }
            65 if ctrl => {
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
            } else if !(self.view == View::Tree && matches!(key, 37 | 39)) {
                self.marked.clear();
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
            } else {
                self.search.more.sel = None;
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

#[cfg(test)]
mod tests {
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
        let mut t = Tree { root_path: r"C:\T".into(), nodes, errors: 0, method: Default::default() };
        t.compute_totals();
        t
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
    pub(crate) files: Vec<usize>,
    pub(crate) exts: Vec<crate::state::ExtRow>,
    /// Each extension's colour, by size (before the table's own sort).
    pub(crate) colours: std::collections::HashMap<String, usize>,
    pub(crate) sorts: Sorts,
}

pub(crate) fn prepare(mut tree: Tree, sorts: Sorts) -> Prepared {
    let alloc: Vec<u64> = tree.nodes.iter().map(|n| n.allocated).collect();
    sort_children(&mut tree, &alloc, sorts.tree);
    let mut files = sprawl_core::largest_files(&tree, usize::MAX);
    sort_file_list(&tree, &alloc, sorts.files, &mut files);
    let mut exts = ext_rows(&tree, &alloc);
    let colours = exts.iter().take(paint::COUNT).enumerate().map(|(i, e)| (e.ext.clone(), i)).collect();
    sort_ext_rows(&mut exts, sorts.exts);
    Prepared { tree: std::sync::Arc::new(tree), alloc, files, exts, colours, sorts }
}

/// What the window worked out for the full tree while a filter shows part of it: kept, so
/// clearing the filter shows the full tree again at once, without a copy or a worker.
pub(crate) struct Parts {
    pub(crate) alloc: Vec<u64>,
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
        self.files = sprawl_core::largest_files(t, usize::MAX);
        sort_file_list(t, &self.alloc, self.sorts.files, &mut self.files);
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

pub(crate) fn sort_children(t: &mut Tree, alloc: &[u64], (c, desc): (usize, bool)) {
    let key = TREE[c].key;
    for i in 0..t.nodes.len() {
        if t.nodes[i].children.len() < 2 {
            continue;
        }
        let mut kids = std::mem::take(&mut t.nodes[i].children);
        kids.sort_by(|&a, &b| {
            let o = if key == Key::Name { cmp_names(&t.nodes[a].name, &t.nodes[b].name) } else { cmp_nodes(t, alloc, key, a, b) };
            if desc { o.reverse() } else { o }
        });
        t.nodes[i].children = kids;
    }
}

pub(crate) fn sort_file_list(t: &Tree, alloc: &[u64], (c, desc): (usize, bool), files: &mut [usize]) {
    let key = FILES[c].key;
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
