//! The Search tab, outside the scan: the last full snapshot of folders scanned before (searchable
//! before any scan of this session) and, as administrator, other drives read from their MFT.
//! These rows are not part of the scan on screen, so they offer Open, Show in Explorer, Copy
//! path and "Scan this folder" — never a delete, which only goes through a scanned tree.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::SystemTime;

use sprawl_core::search::{search, Index, Query};
use sprawl_core::Tree;

use crate::bindings::Microsoft::UI::Xaml::Controls::Border;
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::ButtonBase;
use crate::bindings::Microsoft::UI::Xaml::{UIElement, Visibility};
use crate::format::{fmt_time, thousands};
use crate::state::{find, keep, load, with, State, View};
use crate::ui;

/// Rows shown, biggest first.
const SHOWN: usize = 200;
/// Saved scans loaded at most (each costs about what its scan cost in memory).
const MAX_SNAPSHOTS: usize = 3;

/// A tree that isn't the scan on screen.
pub(crate) struct Place {
    /// "Saved scan, 12 Nov" or "Read now".
    pub(crate) label: String,
    pub(crate) tree: Arc<Tree>,
    index: Arc<Index>,
    /// From a saved scan (its date), or read just now.
    pub(crate) as_of: Option<SystemTime>,
}

type Hits = (u64, String, Option<(Vec<(usize, usize)>, usize)>);

#[derive(Default)]
pub(crate) struct More {
    pub(crate) places: Vec<Arc<Place>>,
    /// Bumped when `places` changes.
    gen: u64,
    snapshots_tried: bool,
    arriving: Option<Receiver<Option<Place>>>,
    /// What is being read ("C:\ (saved 12 Nov)", "D:").
    pub(crate) loading: Vec<String>,
    drives_on: HashSet<String>,
    /// Drives the user switched off this session: not read again by themselves.
    drives_off: HashSet<String>,
    running: Option<Receiver<Hits>>,
    /// (place, node), biggest first, at most SHOWN; and how many there were.
    hits: Vec<(usize, usize)>,
    total: usize,
    hits_gen: u64,
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
            newest.push(e);
        }
        newest.truncate(MAX_SNAPSHOTS);
        if newest.is_empty() {
            self.render_more();
            return;
        }
        self.search.more.loading = newest.iter().map(|e| format!("{} (saved {})", e.root.display(), fmt_time(e.taken))).collect();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for e in newest {
                let place = sprawl_advisor::history::load(&e).ok().map(|t| {
                    let index = Arc::new(Index::build(&t));
                    Place { label: format!("Saved scan, {}", fmt_time(e.taken)), tree: Arc::new(t), index, as_of: Some(e.taken) }
                });
                if tx.send(place).is_err() {
                    return;
                }
            }
        });
        self.search.more.arriving = Some(rx);
        self.render_more();
    }

    /// Every fixed drive the scan on screen isn't, read by itself (as administrator, from the
    /// file table), like Everything: search finds things on all drives without asking.
    fn more_all_drives(&mut self) {
        if !sprawl_core::is_elevated() || self.settings.bool("search_all_drives") == Some(false) {
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
        m.loading.push(format!("{root} (reading)"));
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let progress = sprawl_core::Progress::new();
            let place = sprawl_core::scan_auto(Path::new(&root), &progress).ok().map(|t| {
                let index = Arc::new(Index::build(&t));
                Place { label: "Read now".into(), tree: Arc::new(t), index, as_of: None }
            });
            let _ = tx.send(place);
        });
        // one reader at a time is plenty; a second chip waits behind the first
        if self.search.more.arriving.is_none() {
            self.search.more.arriving = Some(rx);
        } else {
            let prev = self.search.more.arriving.take().unwrap();
            let (tx2, rx2) = channel();
            std::thread::spawn(move || {
                while let Ok(p) = prev.recv() {
                    if tx2.send(p).is_err() {
                        return;
                    }
                }
                if let Ok(p) = rx.recv() {
                    let _ = tx2.send(p);
                }
            });
            self.search.more.arriving = Some(rx2);
        }
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
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut all: Vec<(usize, usize)> = Vec::new();
            for (k, p) in places.iter().enumerate() {
                match search(&p.tree, &p.index, &q, &cancel) {
                    Some(v) => all.extend(v.into_iter().map(|n| (k, n))),
                    None => {
                        let _ = tx.send((gen, text_q, None));
                        return;
                    }
                }
            }
            all.sort_by(|a, b| places[b.0].tree.nodes[b.1].size.cmp(&places[a.0].tree.nodes[a.1].size));
            // what the scan on screen already shows is left out
            let mut shown = Vec::new();
            let mut total = 0usize;
            for (k, n) in all {
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send((gen, text_q, None));
                    return;
                }
                if let Some(c) = &cur {
                    if within(&norm(&places[k].tree.path_of(n)), c) {
                        continue;
                    }
                }
                total += 1;
                if shown.len() < SHOWN {
                    shown.push((k, n));
                }
            }
            let _ = tx.send((gen, text_q, Some((shown, total))));
        });
        m.running = Some(rx);
    }

    /// Each frame: places that arrived, results that arrived.
    pub(crate) fn more_poll(&mut self) {
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
        }
        if let Some(rx) = &self.search.more.running {
            if let Ok((gen, q, r)) = rx.try_recv() {
                self.search.more.running = None;
                if let Some((hits, total)) = r.filter(|_| gen == self.search.more.gen && q == self.search.query) {
                    let m = &mut self.search.more;
                    m.hits = hits;
                    m.total = total;
                    m.hits_gen = gen;
                    self.render_more();
                }
            }
        }
    }

    fn more_path(&self, i: usize) -> Option<PathBuf> {
        let &(k, n) = self.search.more.hits.get(i)?;
        let p = self.search.more.places.get(k)?;
        Some(p.tree.path_of(n))
    }

    fn more_command(&mut self, kind: char, i: usize) {
        let Some(path) = self.more_path(i) else { return };
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
        let mut x = format!(r##"<StackPanel {} Spacing="4" Padding="8,6,8,6">"##, ui::NS);
        // the head: what is searched besides the scan
        x += r##"<StackPanel Orientation="Horizontal" Spacing="8">"##;
        let head = if self.search.query.trim().is_empty() || m.places.is_empty() {
            "Outside this scan".to_string()
        } else if m.total == 0 {
            "Outside this scan: nothing".to_string()
        } else if m.total > m.hits.len() {
            format!("Outside this scan: {} found, the biggest {} shown", thousands(m.total as u64), m.hits.len())
        } else {
            format!("Outside this scan: {} found", thousands(m.total as u64))
        };
        x += &format!(r##"<TextBlock Text="{}" FontSize="12" FontWeight="SemiBold" Foreground="#FFB9C2D6" VerticalAlignment="Center"/>"##, ui::esc(&head));
        let mut places: Vec<String> = m.places.iter().map(|p| format!("{} ({})", p.tree.root_path.display(), p.label.to_lowercase())).collect();
        places.extend(m.loading.iter().map(|l| format!("{l}...")));
        if !places.is_empty() {
            x += &format!(r##"<TextBlock Text="{}" FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center" TextTrimming="CharacterEllipsis" MaxWidth="520"/>"##, ui::esc(&places.join(",  ")));
        }
        if elevated {
            for d in &offer {
                let letter = d.root.chars().next().unwrap_or('C');
                let on = m.drives_on.contains(&d.root);
                x += &format!(
                    r##"<ToggleButton x:Name="SxD{letter}" IsChecked="{on}" FontSize="11" Padding="8,2" ToolTipService.ToolTip="Search {root} too (read from its file table, kept in memory until Sprawl closes). Press to leave it out." Content="{letter}:"/>"##,
                    on = if on { "True" } else { "False" },
                    root = ui::esc(&d.root),
                );
            }
        } else if !offer.is_empty() {
            x += r##"<TextBlock Text="Other drives are searched too when Sprawl runs as administrator." FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center"/>"##;
        }
        x += "</StackPanel>";
        if !m.hits.is_empty() && m.hits_gen == m.gen {
            x += r##"<ScrollViewer MaxHeight="210" VerticalScrollBarVisibility="Auto"><StackPanel>"##;
            for (i, &(k, n)) in m.hits.iter().enumerate() {
                let p = &m.places[k];
                let node = &p.tree.nodes[n];
                let folder = node.parent.map(|q| p.tree.path_of(q).display().to_string()).unwrap_or_default();
                let glyph = if node.is_dir { "&#xE8B7;" } else { "&#xE7C3;" };
                let when = match p.as_of {
                    Some(t) => format!("as of {}", fmt_time(t)),
                    None => "read now".into(),
                };
                x += &format!(
                    r##"<Grid Height="26" ColumnSpacing="8" Background="{bg}"><Grid.ColumnDefinitions><ColumnDefinition Width="2*"/><ColumnDefinition Width="86"/><ColumnDefinition Width="150"/><ColumnDefinition Width="3*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><StackPanel Orientation="Horizontal" Spacing="8" Margin="4,0,0,0"><FontIcon Glyph="{glyph}" FontSize="13" Foreground="#FF8E97AA"/><TextBlock Text="{name}" FontSize="12" VerticalAlignment="Center" TextTrimming="CharacterEllipsis"/></StackPanel><TextBlock Grid.Column="1" Text="{size}" FontSize="12" HorizontalAlignment="Right" VerticalAlignment="Center"/><TextBlock Grid.Column="2" Text="{when}" FontSize="12" Foreground="#FFFFB547" VerticalAlignment="Center"/><TextBlock Grid.Column="3" Text="{folder}" FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center" TextTrimming="CharacterEllipsis"/><StackPanel Grid.Column="4" Orientation="Horizontal" Spacing="2">{b1}{b2}{b3}</StackPanel></Grid>"##,
                    bg = if i % 2 == 1 { "#08FFFFFF" } else { "Transparent" },
                    name = ui::esc(&node.name),
                    size = ui::esc(&self.size(node.size)),
                    when = ui::esc(&when),
                    folder = ui::esc(&folder),
                    b1 = small_button(&format!("SxE{i}"), "&#xEC50;", "Show in Explorer"),
                    b2 = small_button(&format!("SxC{i}"), "&#xE8C8;", "Copy path"),
                    b3 = small_button(&format!("SxS{i}"), "&#xE721;", "Scan this folder"),
                );
            }
            x += "</StackPanel></ScrollViewer>";
        }
        x += "</StackPanel>";
        let Ok(content) = load::<UIElement>(&x) else { return };
        let _ = host.SetChild(&content);
        let _ = host.cast::<UIElement>().and_then(|u| u.SetVisibility(Visibility::Visible));
        // the clicks
        let Ok(fe) = content.cast::<crate::bindings::Microsoft::UI::Xaml::FrameworkElement>() else { return };
        for i in 0..self.search.more.hits.len() {
            for kind in ['E', 'C', 'S'] {
                if let Ok(b) = find::<ButtonBase>(&fe, &format!("Sx{kind}{i}")) {
                    if let Ok(t) = b.Click(move |_, _| {
                        with(|s| s.more_command(kind, i));
                    }) {
                        keep(t);
                    }
                }
            }
        }
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

fn small_button(name: &str, glyph: &str, tip: &str) -> String {
    format!(r##"<Button x:Name="{name}" Padding="6,2" MinWidth="0" Background="Transparent" BorderThickness="0" ToolTipService.ToolTip="{tip}"><FontIcon Glyph="{glyph}" FontSize="12"/></Button>"##)
}

use windows_core::Interface;
