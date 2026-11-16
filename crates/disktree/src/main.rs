// DiskTree: a disk space analyser in Rust + WinUI 3 (Windows App SDK 1.8).
//
// The window is WinUI 3 XAML loaded at run time; the scanning, totals and treemap layout
// live in `disktree-core`.
//
// Usage: disktree [folder]                     open and scan `folder` (or wait for a pick)
//        disktree --screenshot <file.png> [folder]   scan, save a picture of the window, exit
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)]

#[allow(dead_code, unused_imports, non_snake_case, non_camel_case_types, non_upper_case_globals, clippy::all)]
mod bindings;
mod paint;
mod ui;
mod win32;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use bindings::Microsoft::UI::Dispatching::{DispatcherQueue, DispatcherQueueTimer};
use bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, RangeBase, Selector};
use bindings::Microsoft::UI::Xaml::Controls::{
    Border, ColumnDefinition, Image, MenuFlyoutItem, ProgressBar, RowDefinition, ScrollViewer, TextBlock,
    ToggleMenuFlyoutItem, XamlControlsResources,
};
use bindings::Microsoft::UI::Xaml::Markup::{IXamlMetadataProvider, IXamlMetadataProvider_Impl, IXamlType, XamlReader, XmlnsDefinition};
use bindings::Microsoft::UI::Xaml::Media::Imaging::WriteableBitmap;
use bindings::Microsoft::UI::Xaml::Media::ImageSource;
use bindings::Microsoft::UI::Xaml::XamlTypeInfo::XamlControlsXamlMetaDataProvider;
use bindings::Microsoft::UI::Xaml::{
    Application, ApplicationInitializationCallback, ApplicationTheme, FrameworkElement, GridLength, GridUnitType,
    IApplicationOverrides, IApplicationOverrides_Impl, LaunchActivatedEventArgs, ResourceDictionary, UIElement,
    Visibility, Window,
};
use bindings::Windows::Graphics::SizeInt32;
use bindings::Windows::UI::Xaml::Interop::TypeName;
use windows_core::{implement, IInspectable, Interface, Ref, HSTRING};
use disktree_core::{extension_stats, format_size, largest_files, Tree, ROOT};

// ---------------------------------------------------------------- Windows App Runtime start-up

#[cfg(has_bootstrap)]
mod bootstrap {
    #[link(name = "Microsoft.WindowsAppRuntime.Bootstrap")]
    extern "system" {
        fn MddBootstrapInitialize2(major_minor: u32, version_tag: *const u16, min_version: u64, options: u32) -> i32;
    }
    /// Windows App SDK 1.8; show the "install the runtime" prompt when it's missing.
    pub fn start() -> Result<(), i32> {
        let hr = unsafe { MddBootstrapInitialize2(0x0001_0008, std::ptr::null(), 0, 0x18) };
        if hr < 0 { Err(hr) } else { Ok(()) }
    }
}

#[windows_core::interface("EECDBF0E-BAE9-4CB6-A68E-9598E1CB57BB")]
unsafe trait IWindowNative: windows_core::IUnknown {
    fn WindowHandle(&self, hwnd: *mut isize) -> windows_core::HRESULT;
}

#[windows_core::interface("905a0fef-bc53-11df-8c49-001e4fc686da")]
unsafe trait IBufferByteAccess: windows_core::IUnknown {
    fn Buffer(&self, value: *mut *mut u8) -> windows_core::HRESULT;
}

// ---------------------------------------------------------------- the XAML Application

#[implement(IApplicationOverrides, IXamlMetadataProvider)]
struct App {
    provider: OnceLock<XamlControlsXamlMetaDataProvider>,
}

impl App_Impl {
    fn provider(&self) -> windows_core::Result<&XamlControlsXamlMetaDataProvider> {
        if let Some(p) = self.provider.get() {
            return Ok(p);
        }
        let p = XamlControlsXamlMetaDataProvider::new()?;
        Ok(self.provider.get_or_init(|| p))
    }
}

impl IXamlMetadataProvider_Impl for App_Impl {
    fn GetXamlType(&self, t: &TypeName) -> windows_core::Result<IXamlType> {
        self.provider()?.GetXamlType(t)
    }
    fn GetXamlTypeByFullName(&self, name: &HSTRING) -> windows_core::Result<IXamlType> {
        self.provider()?.GetXamlTypeByFullName(name)
    }
    fn GetXmlnsDefinitions(&self) -> windows_core::Result<windows_core::Array<XmlnsDefinition>> {
        self.provider()?.GetXmlnsDefinitions()
    }
}

impl IApplicationOverrides_Impl for App_Impl {
    fn OnLaunched(&self, _args: Ref<LaunchActivatedEventArgs>) -> windows_core::Result<()> {
        if let Some(app) = APP.get() {
            let xcr = XamlControlsResources::new()?;
            app.Resources()?.MergedDictionaries()?.Append(&xcr.cast::<ResourceDictionary>()?)?;
        }
        if let Err(e) = launch() {
            eprintln!("start-up failed: {e:?}");
            std::process::exit(2);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- state

#[derive(Clone, Copy, PartialEq)]
enum Units {
    Dynamic,
    Bytes,
    KB,
    MB,
    GB,
    TB,
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Tree,
    Files,
    About,
}

struct ExtRow {
    ext: String,
    size: u64,
    alloc: u64,
    files: u64,
    percent: f64,
}

struct Job {
    path: PathBuf,
    count: Arc<AtomicU64>,
    result: Arc<Mutex<Option<std::io::Result<Tree>>>>,
    started: Instant,
}

const ROW_H: f64 = 24.0;
/// Border thickness plus padding of `MapBorder` in ui.rs, per side.
const MAP_INSET: f64 = 3.0;
const MAX_ROWS: usize = 900;

struct State {
    window: Window,
    hwnd: isize,
    root: FrameworkElement,
    drives: Vec<String>,
    tree: Option<Tree>,
    alloc: Vec<u64>,
    expanded: HashSet<usize>,
    selected: Option<usize>,
    rows: Vec<(usize, usize)>, // (node, depth)
    files: Vec<usize>,
    exts: Vec<ExtRow>,
    colours: HashMap<String, usize>,
    ext_selected: Option<usize>,
    map_root: usize,
    layout: Option<paint::Layout>,
    map_px: (usize, usize),
    map_pixels: Vec<u8>,
    hover: Option<usize>,
    units: Units,
    view: View,
    job: Option<Job>,
    scan_secs: f64,
    shot: Option<PathBuf>,
    shot_ticks: u32,
    /// The poll timer; it stops when dropped.
    _timer: DispatcherQueueTimer,
}

static APP: OnceLock<Application> = OnceLock::new();

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Run `f` on the state unless it's already in use (an event raised while we change the UI).
fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| match s.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

/// Event subscriptions end when their handle is dropped; these live as long as the window.
fn keep<T>(handle: T) {
    std::mem::forget(handle);
}

fn find<T: Interface>(root: &FrameworkElement, name: &str) -> windows_core::Result<T> {
    root.FindName(&HSTRING::from(name))?.cast()
}

fn text(root: &FrameworkElement, name: &str, value: &str) {
    if let Ok(t) = find::<TextBlock>(root, name) {
        let _ = t.SetText(&HSTRING::from(value));
    }
}

fn load<T: Interface>(xaml: &str) -> windows_core::Result<T> {
    XamlReader::Load(&HSTRING::from(xaml))?.cast()
}

fn thousands(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl State {
    fn size(&self, v: u64) -> String {
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

    fn label(&self, node: usize) -> String {
        let t = self.tree.as_ref().unwrap();
        if node == ROOT {
            t.root_path.display().to_string()
        } else {
            t.nodes[node].name.clone()
        }
    }

    // ------------------------------------------------------------ scanning

    fn start_scan(&mut self, path: PathBuf) {
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

    fn poll(&mut self) {
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

    fn set_tree(&mut self, tree: Tree, path: &std::path::Path) {
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

    fn rebuild_exts(&mut self) {
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

    fn update_header(&self, path: &std::path::Path) {
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

    // ------------------------------------------------------------ tables

    fn visible_rows(&self) -> Vec<(usize, usize)> {
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

    fn refresh_rows(&mut self) {
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

    fn row_bg(&self, k: usize, selected: bool) -> &'static str {
        if selected {
            "#FF2C3B63"
        } else if k % 2 == 1 {
            "#08FFFFFF"
        } else {
            "Transparent"
        }
    }

    fn ext_colour_hex(&self, ext: &str) -> String {
        let (r, g, b) = self.colours.get(ext).map(|&i| paint::PALETTE[i]).unwrap_or(paint::GREY);
        format!("#FF{r:02X}{g:02X}{b:02X}")
    }

    fn row_tapped(&mut self, x: f64, y: f64, double: bool) {
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

    fn show_selection(&self) {
        if let (Some(t), Some(n)) = (&self.tree, self.selected) {
            text(&self.root, "FootLeft", &format!("{}   -   {}", t.path_of(n).display(), self.size(t.nodes[n].size)));
        }
    }

    fn refresh_exts(&mut self) {
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

    // ------------------------------------------------------------ treemap

    fn scale(&self) -> f64 {
        win32::dpi(self.hwnd) as f64 / 96.0
    }

    fn render_map(&mut self) {
        let Some(t) = &self.tree else { return };
        let Ok(img) = find::<FrameworkElement>(&self.root, "MapImage") else { return };
        // Measure the frame, not the Image: an Image without a picture yet has no size.
        let Ok(frame) = find::<FrameworkElement>(&self.root, "MapBorder") else { return };
        let s = self.scale();
        let inner = |v: f64| ((v - 2.0 * MAP_INSET) * s).round().max(0.0) as usize;
        let w = inner(frame.ActualWidth().unwrap_or(0.0));
        let h = inner(frame.ActualHeight().unwrap_or(0.0));
        if w < 8 || h < 8 {
            return;
        }
        let lay = paint::layout(t, self.map_root, w as f64, h as f64);
        let mut px = vec![0u8; w * h * 4];
        let hx = self.ext_selected.and_then(|k| self.exts.get(k)).map(|e| e.ext.as_str());
        paint::paint(t, &lay, &self.colours, hx, self.selected, w, h, &mut px);
        let shown = || -> windows_core::Result<()> {
            let bmp = WriteableBitmap::CreateInstanceWithDimensions(w as i32, h as i32)?;
            let buf = bmp.PixelBuffer()?;
            let access: IBufferByteAccess = buf.cast()?;
            let mut p: *mut u8 = std::ptr::null_mut();
            unsafe { access.Buffer(&mut p).ok()? };
            let len = buf.Length()? as usize;
            unsafe { std::ptr::copy_nonoverlapping(px.as_ptr(), p, len.min(px.len())) };
            bmp.Invalidate()?;
            img.cast::<Image>()?.SetSource(&bmp.cast::<ImageSource>()?)?;
            Ok(())
        };
        if let Err(e) = shown() {
            eprintln!("treemap: {e:?}");
        }
        self.layout = Some(lay);
        self.map_px = (w, h);
        self.map_pixels = px;
        text(&self.root, "MapPath", &format!("[{}]", t.path_of(self.map_root).display()));
    }

    fn map_node_at(&self, x: f64, y: f64) -> Option<usize> {
        let s = self.scale();
        self.layout.as_ref()?.hit(x * s, y * s)
    }

    fn map_hover(&mut self, x: f64, y: f64) {
        let n = self.map_node_at(x, y);
        if n == self.hover {
            return;
        }
        self.hover = n;
        if let (Some(t), Some(n)) = (&self.tree, n) {
            text(&self.root, "MapPath", &format!("{}   ({})", t.path_of(n).display(), self.size(t.nodes[n].size)));
        }
    }

    fn map_click(&mut self, x: f64, y: f64) {
        let Some(n) = self.map_node_at(x, y) else { return };
        self.reveal(n);
    }

    /// Select a node and open every folder above it, then scroll it into view.
    fn reveal(&mut self, n: usize) {
        let Some(t) = &self.tree else { return };
        let mut p = t.nodes[n].parent;
        while let Some(i) = p {
            self.expanded.insert(i);
            p = t.nodes[i].parent;
        }
        self.selected = Some(n);
        if self.view != View::Tree {
            self.set_view(View::Tree);
        }
        self.refresh_rows();
        self.render_map();
        self.show_selection();
        if let Some(k) = self.rows.iter().position(|r| r.0 == n) {
            if let Ok(sv) = find::<ScrollViewer>(&self.root, "TreeScroll") {
                let _ = sv.ChangeView(None, Some((k as f64 * ROW_H - 120.0).max(0.0)), None);
            }
        }
    }

    // ------------------------------------------------------------ commands

    fn set_view(&mut self, v: View) {
        self.view = v;
        let vis = |b: bool| if b { Visibility::Visible } else { Visibility::Collapsed };
        if let Ok(a) = find::<UIElement>(&self.root, "AboutPanel") {
            let _ = a.SetVisibility(vis(v == View::About));
        }
        for (name, on) in [("TreeScroll", v != View::About), ("TreeHeader", v != View::About)] {
            if let Ok(e) = find::<UIElement>(&self.root, name) {
                let _ = e.SetVisibility(vis(on));
            }
        }
        for (name, me) in [("TabTree", View::Tree), ("TabFiles", View::Files), ("TabAbout", View::About)] {
            if let Ok(b) = find::<bindings::Microsoft::UI::Xaml::Controls::Control>(&self.root, name) {
                let brush: windows_core::Result<bindings::Microsoft::UI::Xaml::Media::Brush> =
                    load(&format!(r##"<SolidColorBrush {} Color="{}"/>"##, ui::NS, if me == v { "#FF2C3550" } else { "Transparent" }));
                if let Ok(br) = brush {
                    let _ = b.SetBackground(&br);
                }
            }
        }
        if v != View::About {
            self.refresh_rows();
        }
    }

    fn selected_path(&self) -> Option<PathBuf> {
        Some(self.tree.as_ref()?.path_of(self.selected?))
    }

    fn command(&mut self, name: &str) {
        let has_tree = self.tree.is_some();
        match name {
            "mScan" | "ScanButton" => {
                let idx = find::<Selector>(&self.root, "DriveBox").and_then(|c| c.SelectedIndex()).unwrap_or(0);
                if let Some(d) = self.drives.get(idx.max(0) as usize) {
                    self.start_scan(PathBuf::from(d));
                }
            }
            "mSelectFolder" => {
                if let Some(p) = win32::pick_folder(self.hwnd) {
                    self.start_scan(PathBuf::from(p));
                }
            }
            "mNewInstance" => {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).spawn();
                }
            }
            "mReset" if has_tree => {
                self.expanded = HashSet::from([ROOT]);
                self.refresh_rows();
            }
            "mExpandAll" if has_tree => {
                let t = self.tree.as_ref().unwrap();
                let base = self.selected.unwrap_or(ROOT);
                let mut stack = vec![base];
                while let Some(n) = stack.pop() {
                    if t.nodes[n].is_dir {
                        self.expanded.insert(n);
                        stack.extend(&t.nodes[n].children);
                    }
                }
                self.refresh_rows();
            }
            "mParent" if has_tree => {
                if let Some(p) = self.selected.and_then(|s| self.tree.as_ref().unwrap().nodes[s].parent) {
                    self.reveal(p);
                }
            }
            "mZoomIn" if has_tree => {
                let t = self.tree.as_ref().unwrap();
                let mut n = self.selected.unwrap_or(ROOT);
                if !t.nodes[n].is_dir {
                    n = t.nodes[n].parent.unwrap_or(ROOT);
                }
                self.map_root = n;
                self.render_map();
            }
            "mZoomOut" if has_tree => {
                if let Some(p) = self.tree.as_ref().unwrap().nodes[self.map_root].parent {
                    self.map_root = p;
                    self.render_map();
                }
            }
            "mSavePng" if has_tree => {
                let (w, h) = self.map_px;
                let mut rgba = self.map_pixels.clone();
                for p in rgba.chunks_exact_mut(4) {
                    p.swap(0, 2);
                }
                let path = user_dir("Pictures").join(format!("DiskTree treemap {}.png", stamp()));
                let msg = match std::fs::write(&path, paint::png(w as u32, h as u32, &rgba)) {
                    Ok(()) => format!("Saved the treemap picture to {}", path.display()),
                    Err(e) => format!("Couldn't save the picture: {e}"),
                };
                text(&self.root, "FootLeft", &msg);
            }
            "mExplore" => {
                if let Some(p) = self.selected_path() {
                    let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", p.display())).spawn();
                }
            }
            "mCmd" => {
                if let Some(mut p) = self.selected_path() {
                    if !p.is_dir() {
                        p.pop();
                    }
                    use std::os::windows::process::CommandExt;
                    let _ = std::process::Command::new("cmd.exe").arg("/K").current_dir(p).creation_flags(0x10).spawn();
                }
            }
            "mCopyPath" => {
                if let Some(p) = self.selected_path() {
                    win32::copy_text(self.hwnd, &p.display().to_string());
                }
            }
            "mCopyInfo" if has_tree => {
                let t = self.tree.as_ref().unwrap();
                let mut s = String::from("Name\tSize\tAllocated\tItems\tFiles\r\n");
                for &(n, _) in &self.rows {
                    let node = &t.nodes[n];
                    s += &format!("{}\t{}\t{}\t{}\t{}\r\n", t.path_of(n).display(), self.size(node.size), self.size(self.alloc[n]), node.items(), node.files);
                }
                win32::copy_text(self.hwnd, &s);
                text(&self.root, "FootLeft", "Copied the visible rows to the clipboard");
            }
            "mDelete" | "mDeleteForever" => self.delete(name == "mDeleteForever"),
            "mExportCsv" if has_tree => {
                let t = self.tree.as_ref().unwrap();
                let mut s = String::from("File Name,Size,Allocated,Modified,Attributes,Files,Folders\r\n");
                for (i, n) in t.nodes.iter().enumerate() {
                    let mut name = t.path_of(i).display().to_string();
                    if n.is_dir && !name.ends_with('\\') {
                        name.push('\\');
                    }
                    s += &format!("\"{}\",{},{},{},{},{},{}\r\n", name.replace('"', "\"\""), n.size, self.alloc[i], n.modified.map(fmt_time).unwrap_or_default(), if n.is_dir { "D" } else { "" }, n.files, n.folders);
                }
                self.save_text("DiskTree export", &s);
            }
            "mExportTypes" if has_tree => {
                let mut s = String::from("Extension,File Type,Percent,Size,Allocated,Files\r\n");
                for e in &self.exts {
                    s += &format!("\"{}\",\"{}\",{:.1},{},{},{}\r\n", e.ext, file_type(&e.ext), e.percent, e.size, e.alloc, e.files);
                }
                self.save_text("DiskTree file types", &s);
            }
            "mExit" => std::process::exit(0),
            "uDyn" | "uB" | "uKB" | "uMB" | "uGB" | "uTB" => {
                self.units = match name {
                    "uB" => Units::Bytes,
                    "uKB" => Units::KB,
                    "uMB" => Units::MB,
                    "uGB" => Units::GB,
                    "uTB" => Units::TB,
                    _ => Units::Dynamic,
                };
                if has_tree {
                    self.refresh_rows();
                    self.refresh_exts();
                    let p = self.tree.as_ref().unwrap().root_path.clone();
                    self.update_header(&p);
                }
            }
            "oHeader" | "oTypes" | "oTreemap" => {
                let on = find::<ToggleMenuFlyoutItem>(&self.root, name).and_then(|t| t.IsChecked()).unwrap_or(true);
                let vis = if on { Visibility::Visible } else { Visibility::Collapsed };
                match name {
                    "oHeader" => {
                        if let Ok(e) = find::<UIElement>(&self.root, "Header") {
                            let _ = e.SetVisibility(vis);
                        }
                    }
                    "oTypes" => {
                        if let Ok(e) = find::<UIElement>(&self.root, "TypesPanel") {
                            let _ = e.SetVisibility(vis);
                        }
                        if let Ok(c) = find::<ColumnDefinition>(&self.root, "TypesCol") {
                            let _ = c.SetWidth(GridLength { Value: if on { 1.0 } else { 0.0 }, GridUnitType: GridUnitType::Star });
                        }
                    }
                    _ => {
                        for n in ["MapBorder", "MapPath"] {
                            if let Ok(e) = find::<UIElement>(&self.root, n) {
                                let _ = e.SetVisibility(vis);
                            }
                        }
                        if let Ok(r) = find::<RowDefinition>(&self.root, "MapRow") {
                            let _ = r.SetHeight(GridLength { Value: if on { 1.0 } else { 0.0 }, GridUnitType: GridUnitType::Star });
                        }
                    }
                }
            }
            "oAlloc" | "oCushion" => self.render_map(),
            "TabTree" => self.set_view(View::Tree),
            "TabFiles" => self.set_view(View::Files),
            "TabAbout" => self.set_view(View::About),
            _ => {}
        }
    }

    fn save_text(&self, base: &str, s: &str) {
        let path = user_dir("Documents").join(format!("{base} {}.csv", stamp()));
        let msg = match std::fs::write(&path, s) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Couldn't save: {e}"),
        };
        text(&self.root, "FootLeft", &msg);
    }

    fn delete(&mut self, permanent: bool) {
        let Some(n) = self.selected.filter(|&n| n != ROOT) else { return };
        let Some(path) = self.selected_path() else { return };
        let what = if permanent { "Delete this permanently? It can't be brought back." } else { "Move this to the Recycle Bin?" };
        if !win32::confirm(self.hwnd, &format!("{what}\n\n{}", path.display()), "DiskTree") {
            return;
        }
        if !win32::delete_path(self.hwnd, &path, permanent) {
            win32::info(self.hwnd, "It couldn't be deleted (it may be in use or protected).", "DiskTree");
            return;
        }
        let t = self.tree.as_mut().unwrap();
        let (size, files, folders, is_dir, alloc) = (t.nodes[n].size, t.nodes[n].files, t.nodes[n].folders, t.nodes[n].is_dir, self.alloc[n]);
        let parent = t.nodes[n].parent;
        if let Some(p) = parent {
            t.nodes[p].children.retain(|&c| c != n);
        }
        let mut p = parent;
        while let Some(i) = p {
            let node = &mut t.nodes[i];
            node.size -= size;
            node.files -= files;
            node.folders -= folders + u64::from(is_dir);
            self.alloc[i] -= alloc;
            p = node.parent;
        }
        // Detach the whole branch so file-type totals forget it.
        let mut stack = vec![n];
        while let Some(i) = stack.pop() {
            stack.extend(t.nodes[i].children.clone());
            t.nodes[i].is_dir = true;
            t.nodes[i].size = 0;
            t.nodes[i].children.clear();
            self.alloc[i] = 0;
        }
        self.selected = parent;
        self.files.retain(|&f| f != n && !self.tree.as_ref().unwrap().nodes[f].is_dir);
        self.rebuild_exts();
        self.refresh_rows();
        self.refresh_exts();
        self.render_map();
    }

    // ------------------------------------------------------------ screenshot mode

    fn shot_tick(&mut self) {
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

fn cell(col: usize, s: &str) -> String {
    format!(r##"<TextBlock Grid.Column="{col}" Text="{}" FontSize="12" HorizontalAlignment="Right" VerticalAlignment="Center" Margin="0,0,10,0"/>"##, ui::esc(s))
}

fn pct_bar(col: usize, pct: f64, a: &str, b: &str) -> String {
    let w = (pct.clamp(0.0, 100.0)).max(0.0);
    format!(
        r##"<Grid Grid.Column="{col}" Margin="6,4,6,4"><Border CornerRadius="3" Background="#FF262A36"/><Border CornerRadius="3" HorizontalAlignment="Left" Width="{w:.1}"><Border.Background><LinearGradientBrush StartPoint="0,0" EndPoint="1,0"><GradientStop Color="{a}" Offset="0"/><GradientStop Color="{b}" Offset="1"/></LinearGradientBrush></Border.Background></Border><TextBlock Text="{pct:.1} %" FontSize="11" HorizontalAlignment="Right" VerticalAlignment="Center" Margin="0,0,6,0"/></Grid>"##
    )
}

fn file_type(ext: &str) -> String {
    let known = match ext {
        "(none)" => "(No Extension)",
        "exe" => "Application",
        "dll" => "Application extension",
        "sys" => "System file",
        "msi" => "Windows Installer Package",
        "zip" => "ZIP archive",
        "7z" => "7-Zip archive",
        "rar" => "RAR archive",
        "gz" => "GZip archive",
        "iso" => "Disc Image File",
        "png" => "PNG image",
        "jpg" | "jpeg" => "JPEG image",
        "gif" => "GIF image",
        "txt" => "Text Document",
        "pdf" => "PDF Document",
        "mp4" => "MP4 Video",
        "mp3" => "MP3 Audio",
        "lib" => "Object File Library",
        "pdb" => "Program Debug Database",
        "json" => "JSON File",
        "xml" => "XML Document",
        "js" => "JavaScript File",
        "ttf" => "TrueType font file",
        _ => "",
    };
    if known.is_empty() { format!("{} File", ext.to_uppercase()) } else { known.to_string() }
}

fn fmt_time(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

fn stamp() -> String {
    fmt_time(std::time::SystemTime::now()).replace(':', ".")
}

fn user_dir(sub: &str) -> PathBuf {
    let base = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join(sub);
    if d.is_dir() { d } else { base }
}

// ---------------------------------------------------------------- start-up

struct Args {
    shot: Option<PathBuf>,
    folder: Option<PathBuf>,
}

fn args() -> Args {
    let mut a = Args { shot: None, folder: None };
    let mut it = std::env::args_os().skip(1);
    while let Some(x) = it.next() {
        if x == "--screenshot" {
            a.shot = it.next().map(PathBuf::from);
        } else {
            a.folder = Some(PathBuf::from(x));
        }
    }
    a
}

const COMMANDS: &[&str] = &[
    "mScan", "mSelectFolder", "mNewInstance", "mReset", "mExpandAll", "mParent", "mZoomIn", "mZoomOut", "mSavePng", "mExplore",
    "mCmd", "mCopyPath", "mDelete", "mDeleteForever", "mExportCsv", "mCopyInfo", "mExportTypes", "mExit", "uDyn", "uB", "uKB",
    "uMB", "uGB", "uTB", "oAuto", "oHeader", "oTypes", "oTreemap", "oAlloc", "oCushion",
];

fn launch() -> windows_core::Result<()> {
    let a = args();
    let drives = win32::drives();
    let labels: Vec<String> = drives.iter().map(|d| format!("[{}] {}", d.root.trim_end_matches('\\'), d.label)).collect();

    let window = Window::new()?;
    let root: FrameworkElement = load(&ui::page(&labels))?;
    window.SetContent(&root.cast::<UIElement>()?)?;
    window.SetTitle(&HSTRING::from("DiskTree"))?;
    window.SetExtendsContentIntoTitleBar(true)?;
    window.SetTitleBar(&find::<UIElement>(&root, "TitleBar")?)?;
    window.AppWindow()?.Resize(SizeInt32 { Width: 1680, Height: 1080 })?;

    let mut hwnd = 0isize;
    unsafe { window.cast::<IWindowNative>()?.WindowHandle(&mut hwnd).ok()? };

    if a.shot.is_some() {
        win32::show_behind(hwnd);
    } else {
        window.Activate()?;
    }

    // Menu items and buttons all route to State::command by name.
    for &name in COMMANDS {
        if let Ok(item) = find::<MenuFlyoutItem>(&root, name) {
            keep(item.Click(move |_, _| {
                with(|s| s.command(name));
            })?);
        }
    }
    for name in ["ScanButton", "TabTree", "TabFiles", "TabAbout"] {
        keep(find::<ButtonBase>(&root, name)?.Click(move |_, _| {
            with(|s| s.command(name));
        })?);
    }
    keep(find::<Selector>(&root, "DriveBox")?.SelectionChanged(|_, _| {
        with(|s| {
            let auto = find::<ToggleMenuFlyoutItem>(&s.root, "oAuto").and_then(|t| t.IsChecked()).unwrap_or(false);
            if auto && s.shot.is_none() {
                s.command("mScan");
            }
        });
    })?);
    let map: FrameworkElement = find(&root, "MapImage")?;
    keep(map.Tapped(|s, e| {
        let (Some(s), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
        let Ok(el) = s.cast::<UIElement>() else { return };
        if let Ok(p) = e.GetPosition(&el) {
            with(|st| st.map_click(p.X as f64, p.Y as f64));
        }
    })?);
    keep(map.PointerMoved(|s, e| {
        let (Some(s), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
        let Ok(el) = s.cast::<UIElement>() else { return };
        if let Ok(pos) = e.GetCurrentPoint(&el).and_then(|p| p.Position()) {
            with(|st| st.map_hover(pos.X as f64, pos.Y as f64));
        }
    })?);
    keep(find::<FrameworkElement>(&root, "MapBorder")?.SizeChanged(|_, _| {
        with(|s| s.render_map());
    })?);

    // Clicks on the two tables: the row comes from the position within the scrolled content.
    for (name, ext) in [("TreeScroll", false), ("ExtScroll", true)] {
        let sv: ScrollViewer = find(&root, name)?;
        let handler = move |s: Ref<IInspectable>, x: f64, y: f64, double: bool| {
            let _ = s;
            with(|st| {
                if ext {
                    let k = (y / ROW_H) as usize;
                    if k < st.exts.len() {
                        st.ext_selected = if st.ext_selected == Some(k) { None } else { Some(k) };
                        st.refresh_exts();
                        st.render_map();
                    }
                } else {
                    st.row_tapped(x, y, double);
                }
            });
        };
        keep(sv.Tapped(move |s, e| {
            let (Some(sv), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
            let Some(content) = sv.cast::<ScrollViewer>().ok().and_then(|v| v.Content().ok()).and_then(|c| c.cast::<UIElement>().ok()) else { return };
            if let Ok(p) = e.GetPosition(&content) {
                handler(s, p.X as f64, p.Y as f64, false);
            }
        })?);
        if !ext {
            keep(sv.DoubleTapped(move |s, e| {
                let (Some(sv), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
                let Some(content) = sv.cast::<ScrollViewer>().ok().and_then(|v| v.Content().ok()).and_then(|c| c.cast::<UIElement>().ok()) else { return };
                if let Ok(p) = e.GetPosition(&content) {
                    with(|st| st.row_tapped(p.X as f64, p.Y as f64, true));
                }
            })?);
        }
    }

    // Polls the background scan; in screenshot mode it also counts frames.
    let queue = DispatcherQueue::GetForCurrentThread()?;
    let timer = queue.CreateTimer()?;
    timer.SetInterval(windows_time::TimeSpan { duration: if a.shot.is_some() { 160_000 } else { 1_000_000 } })?;
    keep(timer.Tick(|_, _| {
        with(|s| {
            s.poll();
            s.shot_tick();
        });
    })?);
    timer.Start()?;

    let shot = a.shot.clone();
    let first = a.folder.clone().or_else(|| shot.as_ref().map(|_| PathBuf::from(r"C:\Program Files")));
    let state = State {
        window,
        hwnd,
        root,
        drives: drives.iter().map(|d| d.root.clone()).collect(),
        tree: None,
        alloc: Vec::new(),
        expanded: HashSet::new(),
        selected: None,
        rows: Vec::new(),
        files: Vec::new(),
        exts: Vec::new(),
        colours: HashMap::new(),
        ext_selected: None,
        map_root: ROOT,
        layout: None,
        map_px: (0, 0),
        map_pixels: Vec::new(),
        hover: None,
        units: Units::Dynamic,
        view: View::Tree,
        job: None,
        scan_secs: 0.0,
        shot,
        shot_ticks: 0,
        _timer: timer,
    };
    STATE.with(|s| *s.borrow_mut() = Some(state));
    with(|s| {
        if let Some(f) = first {
            s.start_scan(f);
        } else if let Some(c) = s.drives.first().cloned() {
            s.update_header(std::path::Path::new(&c));
        }
    });
    Ok(())
}

fn main() {
    #[cfg(has_bootstrap)]
    if let Err(hr) = bootstrap::start() {
        eprintln!("Could not load the Windows App Runtime 1.8 (error 0x{:08X}).", hr as u32);
        std::process::exit(1);
    }
    let result = Application::Start(&ApplicationInitializationCallback::new(|_| {
        let app = Application::compose(App { provider: OnceLock::new() })?;
        app.SetRequestedTheme(ApplicationTheme::Dark)?;
        let _ = APP.set(app);
        Ok(())
    }));
    if let Err(e) = result {
        eprintln!("WinUI failed to start: {e:?}");
        std::process::exit(1);
    }
}
