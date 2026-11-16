//! The folder tree, the file list and the file-type table (rows are XAML built as text).

use disktree_core::ROOT;

use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, ScrollViewer};
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement};
use crate::format::{file_type, fmt_time, thousands};
use crate::state::{find, load, text, State, View, MAX_ROWS, ROW_H};
use crate::{paint, ui};

impl State {
    // ------------------------------------------------------------ tables

    pub(crate) fn visible_rows(&self) -> Vec<(usize, usize)> {
        let t = self.tree.as_ref().unwrap();
        let mut out = Vec::new();
        let mut stack = vec![(ROOT, 0usize)];
        while let Some((n, d)) = stack.pop() {
            out.push((n, d));
            if out.len() >= MAX_ROWS {
                break;
            }
            if self.expanded.contains(&n) {
                for &c in t.nodes[n].children.iter().rev() {
                    stack.push((c, d + 1));
                }
            }
        }
        out
    }

    pub(crate) fn refresh_rows(&mut self) {
        let Some(t) = &self.tree else { return };
        let header_cols: Vec<(&str, bool)>;
        let mut body = format!(r##"<StackPanel {} Background="Transparent">"##, ui::NS);
        match self.view {
            View::Files => {
                header_cols = vec![("File Name", false), ("Size", true), ("Allocated", true), ("Modified", true), ("Folder", false)];
                self.rows = self.files.iter().map(|&f| (f, 0)).collect();
                for (k, &(n, _)) in self.rows.iter().enumerate() {
                    let node = &t.nodes[n];
                    let bg = self.row_bg(k, Some(n) == self.selected);
                    let modified = node.modified.map(fmt_time).unwrap_or_default();
                    let folder = node.parent.map(|p| t.path_of(p).display().to_string()).unwrap_or_default();
                    body += &format!(
                        r##"<Grid Height="24" Background="{bg}">{cols}<StackPanel Orientation="Horizontal" Spacing="8" Margin="10,0,0,0"><FontIcon Glyph="&#xE7C3;" FontSize="13" Foreground="{fc}"/><TextBlock Text="{name}" VerticalAlignment="Center" FontSize="12" TextTrimming="CharacterEllipsis"/></StackPanel>{c1}{c2}{c3}<TextBlock Grid.Column="4" Text="{folder}" FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center" Margin="10,0" TextTrimming="CharacterEllipsis"/></Grid>"##,
                        cols = ui::FILE_COLS,
                        fc = self.ext_colour_hex(&node.extension()),
                        name = ui::esc(&node.name),
                        c1 = cell(1, &self.size(node.size)),
                        c2 = cell(2, &self.size(self.alloc[n])),
                        c3 = cell(3, &modified),
                        folder = ui::esc(&folder),
                    );
                }
            }
            _ => {
                header_cols = vec![("Folder", false), ("% of Parent", true), ("Size", true), ("Allocated", true), ("Items", true), ("Files", true)];
                self.rows = self.visible_rows();
                for (k, &(n, d)) in self.rows.iter().enumerate() {
                    let node = &t.nodes[n];
                    let bg = self.row_bg(k, Some(n) == self.selected);
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
                    body += &format!(
                        r##"<Grid Height="24" Background="{bg}">{cols}<StackPanel Orientation="Horizontal" Spacing="6" Margin="{indent},0,0,0"><FontIcon Glyph="{chevron}" FontSize="9" Width="12" Foreground="#FF8E97AA"/><FontIcon Glyph="{glyph}" FontSize="14" Foreground="{gc}"/><TextBlock Text="{name}" VerticalAlignment="Center" FontSize="12" TextTrimming="CharacterEllipsis"/></StackPanel>{bar}{c2}{c3}{c4}{c5}</Grid>"##,
                        cols = ui::TREE_COLS,
                        indent = 8 + d * 16,
                        name = ui::esc(&self.label(n)),
                        bar = pct_bar(1, pct, "#FF3A7BD5", "#FF00D2FF"),
                        c2 = cell(2, &self.size(node.size)),
                        c3 = cell(3, &self.size(self.alloc[n])),
                        c4 = cell(4, &items),
                        c5 = cell(5, &files),
                    );
                }
            }
        }
        body += "</StackPanel>";
        let cols = if self.view == View::Files { ui::FILE_COLS } else { ui::TREE_COLS };
        if let (Ok(h), Ok(head)) = (find::<Border>(&self.root, "TreeHeader"), load::<UIElement>(&ui::header_cells(cols, &header_cols))) {
            let _ = h.SetChild(&head);
        }
        match load::<FrameworkElement>(&body) {
            Ok(panel) => {
                if let Ok(sv) = find::<ScrollViewer>(&self.root, "TreeScroll") {
                    let off = sv.VerticalOffset().unwrap_or(0.0);
                    let _ = sv.SetContent(&panel);
                    let _ = sv.ChangeView(None, Some(off), None);
                }
            }
            Err(e) => eprintln!("rows: {e:?}"),
        }
        let total = t.nodes[ROOT].size;
        text(&self.root, "FootRight", &format!("{} items  |  {}", thousands(t.nodes[ROOT].items()), self.size(total)));
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
        let (r, g, b) = self.colours.get(ext).map(|&i| paint::PALETTE[i]).unwrap_or(paint::GREY);
        format!("#FF{r:02X}{g:02X}{b:02X}")
    }

    pub(crate) fn row_tapped(&mut self, x: f64, y: f64, double: bool) {
        let k = (y / ROW_H) as usize;
        let Some(&(n, d)) = self.rows.get(k) else { return };
        let on_chevron = self.view == View::Tree && x < (8 + d * 16 + 20) as f64;
        self.selected = Some(n);
        if self.view == View::Tree && (double || on_chevron) {
            let is_dir = self.tree.as_ref().is_some_and(|t| t.nodes[n].is_dir);
            if is_dir && !self.expanded.remove(&n) {
                self.expanded.insert(n);
            }
        }
        self.refresh_rows();
        self.render_map();
        self.show_selection();
    }

    pub(crate) fn show_selection(&self) {
        if let (Some(t), Some(n)) = (&self.tree, self.selected) {
            text(&self.root, "FootLeft", &format!("{}   -   {}", t.path_of(n).display(), self.size(t.nodes[n].size)));
        }
    }

    pub(crate) fn refresh_exts(&mut self) {
        let header = ui::header_cells(
            ui::EXT_COLS,
            &[("", false), ("Extension", false), ("File Type", false), ("Percent", true), ("Size", true), ("Allocated", true), ("Files", true)],
        );
        if let (Ok(h), Ok(head)) = (find::<Border>(&self.root, "ExtHeader"), load::<UIElement>(&header)) {
            let _ = h.SetChild(&head);
        }
        let mut body = format!(r##"<StackPanel {} Background="Transparent">"##, ui::NS);
        for (k, e) in self.exts.iter().take(MAX_ROWS).enumerate() {
            let bg = self.row_bg(k, Some(k) == self.ext_selected);
            let shown = if e.ext == "(none)" { String::new() } else { format!(".{}", e.ext) };
            body += &format!(
                r##"<Grid Height="24" Background="{bg}">{cols}<Border Width="12" Height="12" CornerRadius="3" Background="{c}" Margin="8,0,0,0"/><TextBlock Grid.Column="1" Text="{ext}" FontSize="12" VerticalAlignment="Center" Margin="8,0,0,0" TextTrimming="CharacterEllipsis"/><TextBlock Grid.Column="2" Text="{kind}" FontSize="12" Foreground="#FFB9C2D6" VerticalAlignment="Center" Margin="10,0" TextTrimming="CharacterEllipsis"/>{bar}{c4}{c5}{c6}</Grid>"##,
                cols = ui::EXT_COLS,
                c = self.ext_colour_hex(&e.ext),
                ext = ui::esc(&shown),
                kind = ui::esc(&file_type(&e.ext)),
                bar = pct_bar(3, e.percent, "#FF8E54E9", "#FFFF5E9C"),
                c4 = cell(4, &self.size(e.size)),
                c5 = cell(5, &self.size(e.alloc)),
                c6 = cell(6, &thousands(e.files)),
            );
        }
        body += "</StackPanel>";
        if let Ok(panel) = load::<FrameworkElement>(&body) {
            if let Ok(sv) = find::<ScrollViewer>(&self.root, "ExtScroll") {
                let off = sv.VerticalOffset().unwrap_or(0.0);
                let _ = sv.SetContent(&panel);
                let _ = sv.ChangeView(None, Some(off), None);
            }
        }
    }
}

pub(crate) fn cell(col: usize, s: &str) -> String {
    format!(r##"<TextBlock Grid.Column="{col}" Text="{}" FontSize="12" HorizontalAlignment="Right" VerticalAlignment="Center" Margin="0,0,10,0"/>"##, ui::esc(s))
}

pub(crate) fn pct_bar(col: usize, pct: f64, a: &str, b: &str) -> String {
    let w = (pct.clamp(0.0, 100.0)).max(0.0);
    format!(
        r##"<Grid Grid.Column="{col}" Margin="6,4,6,4"><Border CornerRadius="3" Background="#FF262A36"/><Border CornerRadius="3" HorizontalAlignment="Left" Width="{w:.1}"><Border.Background><LinearGradientBrush StartPoint="0,0" EndPoint="1,0"><GradientStop Color="{a}" Offset="0"/><GradientStop Color="{b}" Offset="1"/></LinearGradientBrush></Border.Background></Border><TextBlock Text="{pct:.1} %" FontSize="11" HorizontalAlignment="Right" VerticalAlignment="Center" Margin="0,0,6,0"/></Grid>"##
    )
}
