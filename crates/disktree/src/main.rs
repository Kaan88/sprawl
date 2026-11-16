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
mod paint;
mod state;
mod tables;
mod treemap;
mod ui;
mod win32;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

use disktree_core::ROOT;
use windows_core::{IInspectable, Interface, Ref, HSTRING};

use app::{App, APP};
use bindings::Microsoft::UI::Dispatching::DispatcherQueue;
use bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, Selector};
use bindings::Microsoft::UI::Xaml::Controls::{MenuFlyoutItem, ScrollViewer, ToggleMenuFlyoutItem};
use bindings::Microsoft::UI::Xaml::{Application, ApplicationInitializationCallback, ApplicationTheme, FrameworkElement, UIElement, Window};
use bindings::Windows::Graphics::SizeInt32;
use state::{find, keep, load, with, State, Units, View, ROW_H, STATE};

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
