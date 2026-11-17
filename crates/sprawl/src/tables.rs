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
        }
    }
}

impl Col {
    pub(crate) fn key(&self) -> Key {
        self.key
    }
}

/// Sort `v` (node indices) by `key`, the way the tables do.
pub(crate) fn sort_nodes(t: &Tree, alloc: &[u64], key: Key, desc: bool, v: &mut [usize]) {
    v.sort_by(|&a, &b| {
        let o = if key == Key::Name { cmp_names(&t.nodes[a].name, &t.nodes[b].name) } else { cmp_nodes(t, alloc, key, a, b) };
        if desc { o.reverse() } else { o }
    });
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

/// Case-insensitive comparison without allocating.
fn cmp_names(a: &str, b: &str) -> Ordering {
    a.bytes().map(|c| c.to_ascii_lowercase()).cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
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
    format!(r##"<TextBlock Grid.Column="{col}" Text="{}" FontSize="12" HorizontalAlignment="Right" VerticalAlignment="Center" Margin="0,0,10,0"/>"##, ui::esc(s))
}

pub(crate) fn pct_bar(col: usize, pct: f64, a: &str, b: &str) -> String {
    let w = pct.clamp(0.0, 100.0);
    format!(
        r##"<Grid Grid.Column="{col}" Margin="6,4,6,4"><Border CornerRadius="3" Background="#18FFFFFF"/><Border CornerRadius="3" HorizontalAlignment="Left" Width="{w:.1}"><Border.Background><LinearGradientBrush StartPoint="0,0" EndPoint="1,0"><GradientStop Color="{a}" Offset="0"/><GradientStop Color="{b}" Offset="1"/></LinearGradientBrush></Border.Background></Border><TextBlock Text="{pct:.1} %" FontSize="12" HorizontalAlignment="Right" VerticalAlignment="Center" Margin="0,0,6,0"/></Grid>"##
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
        let (c, desc) = self.tables.tree_sort;
        let key = TREE[c].key;
        let Some(t) = self.tree.as_mut().map(std::sync::Arc::make_mut) else { return };
        let alloc = &self.alloc;
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

    pub(crate) fn sort_files(&mut self) {
        let (c, desc) = self.tables.files_sort;
        let key = FILES[c].key;
        let Some(t) = &self.tree else { return };
        let alloc = &self.alloc;
        self.files.sort_by(|&a, &b| {
            let o = if key == Key::Name { cmp_names(&t.nodes[a].name, &t.nodes[b].name) } else { cmp_nodes(t, alloc, key, a, b) };
            if desc { o.reverse() } else { o }
        });
    }

    pub(crate) fn sort_exts(&mut self) {
        let (c, desc) = self.tables.ext_sort;
        let key = EXTS[c].key;
        self.exts.sort_by(|a, b| {
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
        let Some(t) = &self.tree else { return };
        self.rows = match self.view {
            View::Files => self.files.iter().map(|&f| (f, 0)).collect(),
            View::Search => self.search_rows(),
            _ => self.visible_rows(),
        };
        let (cols, sort) = self.main_cols();
        if let (Ok(h), Ok(head)) = (find::<Border>(&self.root, "TreeHeader"), load::<UIElement>(&header(cols, sort))) {
            let _ = h.SetChild(&head);
        }
        let total = t.nodes[ROOT].size;
        let items = t.nodes[ROOT].items();
        if self.view == View::Search {
            text(&self.root, "FootRight", &format!("{} found  |  {} items scanned", thousands(self.rows.len() as u64), thousands(items)));
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
            Which::Main => ("TreeScroll", if self.tree.is_some() { self.rows.len() } else { 0 }),
            Which::Exts => ("ExtScroll", self.exts.len()),
        };
        let Ok(sv) = find::<ScrollViewer>(&self.root, scroll) else { return };
        let off = sv.VerticalOffset().unwrap_or(0.0);
        let vh = sv.ViewportHeight().unwrap_or(0.0);
        let vh = if vh > 0.0 { vh } else { 900.0 };
        let first = (off / ROW_H).floor() as usize;
        let last = (((off + vh) / ROW_H).ceil() as usize).min(count);
        let span = if which == Which::Main { self.tables.main_span } else { self.tables.ext_span };
        if span.2 == count && span.0 <= first && last <= span.1 {
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
            let _ = fe.SetHeight((count as f64 * ROW_H).max(1.0));
        }
        let mut xaml = format!(r##"<StackPanel {} VerticalAlignment="Top" Margin="0,{},0,0">"##, ui::NS, a as f64 * ROW_H);
        for k in a..b {
            xaml += &match which {
                Which::Main if self.view == View::Files => self.file_row(k),
                Which::Main if self.view == View::Search => self.search_row(k),
                Which::Main => self.tree_row(k),
                Which::Exts => self.ext_row(k),
            };
        }
        xaml += "</StackPanel>";
        match load::<UIElement>(&xaml) {
            Ok(chunk) => {
                let _ = body.SetChild(&chunk);
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

    pub(crate) fn row_tapped(&mut self, x: f64, y: f64, double: bool) {
        let k = (y / ROW_H) as usize;
        let Some(&(n, d)) = self.rows.get(k) else { return };
        let on_chevron = self.view == View::Tree && x < (8 + d * 16 + 20) as f64;
        // Ctrl adds or takes away a row, Shift picks the run from the selected row
        let (ctrl, shift) = if double || on_chevron { (false, false) } else { crate::win32::ctrl_shift_down() };
        if shift {
            let from = self.selected.and_then(|s| self.rows.iter().position(|r| r.0 == s)).unwrap_or(k);
            let (a, b) = (from.min(k), from.max(k));
            self.marked = self.rows[a..=b].iter().map(|r| r.0).collect();
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
            self.search_reveal(n);
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
        let k = (y / ROW_H) as usize;
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
        if let Ok(sv) = find::<ScrollViewer>(&self.root, "ExtScroll") {
            let off = sv.VerticalOffset().unwrap_or(0.0);
            let vh = sv.ViewportHeight().unwrap_or(0.0);
            let (top, bottom) = (target as f64 * ROW_H, (target + 1) as f64 * ROW_H);
            let to = if top < off { Some(top) } else if bottom > off + vh { Some(bottom - vh) } else { None };
            if let Some(y) = to {
                let _ = sv.ChangeView(None, Some(y), None);
            }
        }
        true
    }

    /// Right-click on a row: select it and open the row menu at the pointer.
    pub(crate) fn row_menu(&mut self, y_content: f64, at: Point) {
        let k = (y_content / ROW_H) as usize;
        let Some(&(n, _)) = self.rows.get(k) else { return };
        // right-click outside the picked rows picks just that row
        if !self.marked.contains(&n) {
            self.marked.clear();
        }
        self.selected = Some(n);
        self.refresh_rows();
        self.show_selection();
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
    pub(crate) fn table_key(&mut self, key: i32) -> bool {
        let Some(t) = &self.tree else { return false };
        let k = self.selected.and_then(|n| self.rows.iter().position(|r| r.0 == n)).unwrap_or(0);
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
            65 if ctrl => {
                self.marked = self.rows.iter().map(|r| r.0).collect();
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
            13 if self.view == View::Search => {
                if let Some(&(n, _)) = self.rows.get(k) {
                    self.search_reveal(n);
                }
                return true;
            }
            39 | 37 | 13 if self.view == View::Tree => {
                let Some(&(n, _)) = self.rows.get(k) else { return false };
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
                let hit = (0..n).map(|i| (start + i) % n.max(1)).find(|&i| t.nodes[self.rows[i].0].name.to_lowercase().starts_with(&typed));
                match hit {
                    Some(i) => target = i,
                    None => return true,
                }
            }
            _ => return false,
        }
        if let Some(&(n, _)) = self.rows.get(target) {
            self.selected = Some(n);
            if shift && matches!(key, 33..=40) {
                // the whole run from where it started
                let (a, b) = (k.min(target), k.max(target));
                self.marked.extend(self.rows[a..=b].iter().map(|r| r.0));
            }
        }
        self.refresh_rows();
        self.scroll_to_selected();
        self.show_selection();
        true
    }

    /// Scroll the main table just enough to show the selected row.
    pub(crate) fn scroll_to_selected(&mut self) {
        let Some(k) = self.selected.and_then(|n| self.rows.iter().position(|r| r.0 == n)) else { return };
        let Ok(sv) = find::<ScrollViewer>(&self.root, "TreeScroll") else { return };
        let off = sv.VerticalOffset().unwrap_or(0.0);
        let vh = sv.ViewportHeight().unwrap_or(0.0);
        let (top, bottom) = (k as f64 * ROW_H, (k + 1) as f64 * ROW_H);
        let to = if top < off {
            Some(top)
        } else if bottom > off + vh {
            Some(bottom - vh)
        } else {
            None
        };
        if let Some(y) = to {
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
    use super::*;

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
