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
mod app;
mod commands;
mod format;
mod gpu;
mod hud;
mod paint;
mod scene;
mod state;
mod tables;
mod treemap;
mod ui;
mod win32;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

use disktree_core::ROOT;
use windows_core::{IInspectable, Interface, HSTRING};

use app::{App, APP};
use bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, Selector};
use bindings::Windows::Foundation::Point;
use tables::Which;
use bindings::Microsoft::UI::Xaml::Controls::{MenuFlyoutItem, ScrollViewer, SwapChainPanel, ToggleMenuFlyoutItem};
use bindings::Microsoft::UI::Xaml::Media::{CompositionTarget, DesktopAcrylicBackdrop, SystemBackdrop};
use bindings::Microsoft::UI::Xaml::{Application, ApplicationInitializationCallback, ApplicationTheme, FrameworkElement, UIElement, Window};
use bindings::Windows::Graphics::SizeInt32;
use state::{find, keep, load, with, State, Units, View, STATE};

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
    "uMB", "uGB", "uTB", "oAuto", "oHeader", "oTypes", "oTreemap", "oAlloc", "oCushion", "oNames", "oTilt",
];

/// Right-click menu on the tree: (item name, command).
const ROW_MENU: &[(&str, &str)] = &[
    ("cOpen", "mOpen"),
    ("cExplore", "mExplore"),
    ("cCmd", "mCmd"),
    ("cCopyPath", "mCopyPath"),
    ("cMapHere", "mMapHere"),
    ("cExpandAll", "mExpandAll"),
    ("cCopyInfo", "mCopyInfo"),
    ("cDelete", "mDelete"),
    ("cDeleteForever", "mDeleteForever"),
];

pub(crate) fn launch() -> windows_core::Result<()> {
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
    // Frosted glass: the desktop shows through the whole window, blurred.
    if let Ok(b) = DesktopAcrylicBackdrop::new().and_then(|b| b.cast::<SystemBackdrop>()) {
        window.SetSystemBackdrop(&b)?;
    }

    let hwnd = app::hwnd_of(&window)?;

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
    // The 3D treemap panel.
    let panel: SwapChainPanel = find(&root, "MapPanel")?;
    let map: UIElement = panel.cast()?;
    let pos = |s: &Option<IInspectable>, e: &bindings::Microsoft::UI::Xaml::Input::PointerRoutedEventArgs| -> Option<(f64, f64)> {
        let el = s.as_ref()?.cast::<UIElement>().ok()?;
        let p = e.GetCurrentPoint(&el).ok()?.Position().ok()?;
        Some((p.X as f64, p.Y as f64))
    };
    keep(map.PointerMoved(move |s, e| {
        let Some(e) = e.as_ref() else { return };
        if let Some((x, y)) = pos(&s.cloned(), e) {
            with(|st| st.map_hover(x, y));
        }
    })?);
    keep(map.PointerExited(|_, _| {
        with(|st| st.map_leave());
    })?);
    keep(map.PointerWheelChanged(move |s, e| {
        let Some(e) = e.as_ref() else { return };
        let Some(el) = s.as_ref().and_then(|s| s.cast::<UIElement>().ok()) else { return };
        let Ok(pt) = e.GetCurrentPoint(&el) else { return };
        let delta = pt.Properties().and_then(|p| p.MouseWheelDelta()).unwrap_or(0);
        if let Ok(p) = pt.Position() {
            with(|st| st.map_wheel(p.X as f64, p.Y as f64, delta));
        }
        let _ = e.SetHandled(true);
    })?);
    keep(map.Tapped(|s, e| {
        let (Some(s), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
        let Ok(el) = s.cast::<UIElement>() else { return };
        if let Ok(p) = e.GetPosition(&el) {
            with(|st| st.map_click(p.X as f64, p.Y as f64));
        }
    })?);
    keep(map.DoubleTapped(|s, e| {
        let (Some(s), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
        let Ok(el) = s.cast::<UIElement>() else { return };
        if let Ok(p) = e.GetPosition(&el) {
            with(|st| st.map_zoom_at(p.X as f64, p.Y as f64));
        }
    })?);

    // The two tables. Positions are taken relative to the scrolled content, so row = y / ROW_H.
    let content_pos = |sv: &IInspectable, get: &dyn Fn(&UIElement) -> windows_core::Result<Point>| -> Option<(f64, f64)> {
        let c = sv.cast::<ScrollViewer>().ok()?.Content().ok()?.cast::<UIElement>().ok()?;
        let p = get(&c).ok()?;
        Some((p.X as f64, p.Y as f64))
    };
    for (name, which) in [("TreeScroll", Which::Main), ("ExtScroll", Which::Exts)] {
        let sv: ScrollViewer = find(&root, name)?;
        keep(sv.Tapped(move |s, e| {
            let (Some(s), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
            if let Some((x, y)) = content_pos(s, &|c| e.GetPosition(c)) {
                with(|st| match which {
                    Which::Main => st.row_tapped(x, y, false),
                    Which::Exts => st.ext_tapped(y),
                });
            }
        })?);
        keep(sv.ViewChanged(move |_, _| {
            with(|st| st.fill(which));
        })?);
        keep(sv.cast::<FrameworkElement>()?.SizeChanged(move |_, _| {
            with(|st| st.fill(which));
        })?);
        let header: FrameworkElement = find(&root, if which == Which::Main { "TreeHeader" } else { "ExtHeader" })?;
        keep(header.Tapped(move |s, e| {
            let (Some(s), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
            let Ok(el) = s.cast::<UIElement>() else { return };
            let w = s.cast::<FrameworkElement>().and_then(|f| f.ActualWidth()).unwrap_or(0.0);
            if let Ok(p) = e.GetPosition(&el) {
                with(|st| st.header_clicked(which, p.X as f64, w));
            }
        })?);
    }
    let tree_sv: UIElement = find(&root, "TreeScroll")?;
    keep(tree_sv.DoubleTapped(move |s, e| {
        let (Some(s), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
        if let Some((x, y)) = content_pos(s, &|c| e.GetPosition(c)) {
            with(|st| st.row_tapped(x, y, true));
        }
    })?);
    keep(tree_sv.RightTapped(move |s, e| {
        let (Some(s), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
        let Some((_, y)) = content_pos(s, &|c| e.GetPosition(c)) else { return };
        let Ok(at) = s.cast::<UIElement>().and_then(|el| e.GetPosition(&el)) else { return };
        with(|st| st.row_menu(y, at));
    })?);
    keep(tree_sv.KeyDown(|_, e| {
        let Some(e) = e.as_ref() else { return };
        let Ok(k) = e.Key() else { return };
        if with(|st| st.table_key(k.0)).unwrap_or(false) {
            let _ = e.SetHandled(true);
        }
    })?);
    // The row menu's items reuse the File menu's commands.
    for (item, cmd) in ROW_MENU {
        if let Ok(i) = find::<MenuFlyoutItem>(&root, item) {
            keep(i.Click(move |_, _| {
                with(|s| s.command(cmd));
            })?);
        }
    }

    // Everything that moves (scan polling, the overlay, the treemap) runs once per frame.
    keep(CompositionTarget::Rendering(|_, _| {
        with(|s| s.frame());
    })?);

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
        tables: tables::Tables::default(),
        map_root: ROOT,
        map: treemap::MapView::new(panel),
        hud: hud::Hud::default(),
        units: Units::Dynamic,
        view: View::Tree,
        job: None,
        scan_secs: 0.0,
        shot,
        shot_ticks: 0,
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
    if let Err(hr) = app::bootstrap::start() {
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
