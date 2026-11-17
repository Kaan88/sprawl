// Sprawl: a disk space analyser in Rust + WinUI 3 (Windows App SDK 1.8).
//
// The window is WinUI 3 XAML loaded at run time; the scanning, totals and treemap layout
// live in `sprawl-core`.
//
// Usage: sprawl [folder]                     open and scan `folder` (or wait for a pick)
//        sprawl --screenshot <file.png> [folder]   scan, save a picture of the window, exit
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)]
#[allow(dead_code, unused_imports, non_snake_case, non_camel_case_types, non_upper_case_globals, clippy::all)]
mod bindings;
mod advisor;
mod bridge;
mod app;
mod cleanup;
mod commands;
mod format;
mod dialogs;
mod galaxy;
mod gpu;
mod history;
mod hud;
mod paint;
mod scene;
mod settings;
mod state;
mod tables;
mod views;
mod watch;
mod treemap;
mod ui;
mod look;
mod win32;
mod welcome;
mod wizard;
mod prefs;
mod update;
mod cleaners;
mod models;
mod search;
mod search_more;
mod search_query;
mod search_box;
mod shellmenu;
mod admin;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

use sprawl_core::ROOT;
use windows_core::{IInspectable, Interface, HSTRING};

use app::{App, APP};
use bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, Selector};
use bindings::Windows::Foundation::Point;
use tables::Which;
use bindings::Microsoft::UI::Xaml::Controls::Primitives::RangeBase;
use bindings::Microsoft::UI::Xaml::Controls::{MenuFlyoutItem, ScrollViewer, SwapChainPanel, TextBox, ToggleMenuFlyoutItem, ToggleSwitch};
use bindings::Microsoft::UI::Xaml::Media::{CompositionTarget, DesktopAcrylicBackdrop, SystemBackdrop};
use bindings::Microsoft::UI::Xaml::{Application, ApplicationInitializationCallback, ApplicationTheme, FrameworkElement, UIElement, Window};
use bindings::Windows::Graphics::SizeInt32;
use state::{find, keep, load, with, State, Units, View, STATE};

// ---------------------------------------------------------------- start-up

struct Args {
    shot: Option<PathBuf>,
    folder: Option<PathBuf>,
    /// Restarted as administrator from the Advisor: open it again, with these ticks.
    advisor: bool,
    ticks: Vec<PathBuf>,
}

fn args() -> Args {
    let mut a = Args { shot: None, folder: None, advisor: false, ticks: Vec::new() };
    let mut it = std::env::args_os().skip(1);
    while let Some(x) = it.next() {
        if x == "--screenshot" {
            a.shot = it.next().map(PathBuf::from);
        } else if x == "--advisor" {
            a.advisor = true;
        } else if x == "--tick" {
            a.ticks.extend(it.next().map(PathBuf::from));
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
    "mSelectMultiple", "mSelectCsv", "mSettings", "mShortcuts", "mAbout", "mUpdate", "oUpdates", "mStop", "mReselectChild", "mFilter", "mSearch", "mTweaks", "mAdmin", "oFree", "oMirror", "oPulses", "oAlwaysAdmin", "mWizard", "oExplorer", "aaOff", "aaLow", "aaMedium", "aaHigh",
];

/// Right-click menu on the tree: (item name, command).
const ROW_MENU: &[(&str, &str)] = &[
    ("cOpen", "mOpen"),
    ("cInTree", "mInTree"),
    ("cExplore", "mExplore"),
    ("cCmd", "mCmd"),
    ("cCopyPath", "mCopyPath"),
    ("cMapHere", "mMapHere"),
    ("cExpandAll", "mExpandAll"),
    ("cCopyInfo", "mCopyInfo"),
    ("cDelete", "mDelete"),
    ("cDeleteForever", "mDeleteForever"),
    ("cShell", "mShell"),
];

pub(crate) fn launch() -> windows_core::Result<()> {
    let a = args();
    let mut settings = settings::Settings::load();
    settings.keep_old_defaults();
    // "Always run as administrator": come back elevated (the UAC prompt shows) and quit this copy.
    if a.shot.is_none() && settings.bool("oAlwaysAdmin") == Some(true) && !sprawl_core::is_elevated() {
        let args: Vec<String> = std::env::args_os().skip(1).map(|x| dialogs::quote(std::path::Path::new(&x))).collect();
        if dialogs::restart_elevated(0, &args.join(" ")) {
            std::process::exit(0);
        }
    }
    let drives = win32::drives();
    let labels: Vec<String> = drives.iter().map(|d| format!("[{}] {}", d.root.trim_end_matches('\\'), d.label)).collect();

    let window = Window::new()?;
    let root: FrameworkElement = load(&ui::page(&labels))?;
    window.SetContent(&root.cast::<UIElement>()?)?;
    window.SetTitle(&HSTRING::from("Sprawl"))?;
    window.SetExtendsContentIntoTitleBar(true)?;
    window.SetTitleBar(&find::<UIElement>(&root, "TitleBar")?)?;
    // dev: SPRAWL_SIZE=1280x800 sets the window's size for a screenshot
    let (w, h) = std::env::var("SPRAWL_SIZE").ok().filter(|_| a.shot.is_some()).and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).unwrap_or((1680, 1080));
    window.AppWindow()?.Resize(SizeInt32 { Width: w, Height: h })?;
    // Frosted glass: the desktop shows through the whole window, blurred.
    if let Ok(b) = DesktopAcrylicBackdrop::new().and_then(|b| b.cast::<SystemBackdrop>()) {
        window.SetSystemBackdrop(&b)?;
    }

    let hwnd = app::hwnd_of(&window)?;
    win32::set_app_icon(hwnd);

    // closing mid clean-up asks first (what is gone stays gone; the rest is left as it was)
    win32::on_close(hwnd, || {
        let ok = match with(|s| (s.advisor.cleanup.running.is_some(), s.hwnd)) {
            Some((true, h)) => win32::confirm(h, "A clean-up is still running. Close Sprawl anyway?

What is already deleted stays deleted; the rest is left as it was.", "Sprawl"),
            _ => true,
        };
        if ok {
            // the window's place and size, for next time
            with(|s| {
                if let Some(p) = win32::window_place(s.hwnd) {
                    s.settings.set("window", p);
                    s.settings.save();
                }
            });
        }
        ok
    });
    // the window where it was left, or most of the screen (screenshots keep their fixed size)
    if a.shot.is_none() {
        let remembered = crate::settings::Settings::load().get("window").is_some_and(|v| win32::set_window_place(hwnd, v));
        if !remembered {
            win32::fit_work_area(hwnd);
        }
    }
    if a.shot.is_some() {
        crate::settings::READ_ONLY.store(true, std::sync::atomic::Ordering::Relaxed);
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
    // the other views and their colour switch
    for name in crate::views::ALL.iter().map(|v| v.1).chain(crate::views::PAINTS.iter().map(|p| p.1)) {
        if let Ok(item) = find::<MenuFlyoutItem>(&root, name) {
            keep(item.Click(move |_, _| {
                with(|s| s.command(name));
            })?);
        }
    }
    for name in wizard::BUTTONS {
        keep(find::<ButtonBase>(&root, name)?.Click(move |_, _| {
            with(|s| s.wizard_command(name));
        })?);
    }
    keep(find::<Selector>(&root, "WizProvider")?.SelectionChanged(|_, _| {
        with(|s| s.wizard_provider());
    })?);
    keep(find::<ButtonBase>(&root, "HistGrowth")?.Click(|_, _| {
        with(|s| {
            s.command("TabAdvisor");
            s.advisor_command("AdvGrowOpen");
        });
    })?);
    for name in ["HistClose", "HistPlay"] {
        keep(find::<ButtonBase>(&root, name)?.Click(move |_, _| {
            with(|s| if name == "HistPlay" { s.hist_play() } else { s.hist_close() });
        })?);
    }
    keep(find::<RangeBase>(&root, "HistSlider")?.ValueChanged(|_, e| {
        let Some(v) = e.as_ref().and_then(|e| e.NewValue().ok()) else { return };
        with(|st| {
            if st.hist.as_ref().is_some_and(|h| h.pos != v.round() as usize) {
                st.hist_seek(v.round().max(0.0) as usize);
            }
        });
    })?);
    for name in ["ScanButton", "TabTree", "TabFiles", "TabSearch", "TabAdvisor", "TabCleaners", "SearchToFilter", "TabAbout", "ModeMap", "ModeCity", "ModeGalaxy", "ModeHist", "ViewOnly", "SetupYes", "SetupNo", "UpdateNow", "UpdateLater", "UpdateSkip"] {
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
    keep(map.PointerMoved(move |s, e| {
        let Some(e) = e.as_ref() else { return };
        let Some(el) = s.as_ref().and_then(|s| s.cast::<UIElement>().ok()) else { return };
        let Ok(pt) = e.GetCurrentPoint(&el) else { return };
        let props = pt.Properties();
        let left = props.as_ref().is_ok_and(|p| p.IsLeftButtonPressed().unwrap_or(false));
        let pan = props.as_ref().is_ok_and(|p| p.IsMiddleButtonPressed().unwrap_or(false) || p.IsRightButtonPressed().unwrap_or(false));
        if let Ok(p) = pt.Position() {
            with(|st| st.map_hover(p.X as f64, p.Y as f64, left, pan));
        }
    })?);
    keep(map.PointerPressed(move |s, e| {
        let Some(e) = e.as_ref() else { return };
        let Some(el) = s.as_ref().and_then(|s| s.cast::<UIElement>().ok()) else { return };
        let Ok(pt) = e.GetCurrentPoint(&el) else { return };
        if let Ok(p) = pt.Position() {
            with(|st| st.map_press(p.X as f64, p.Y as f64));
        }
    })?);
    keep(map.RightTapped(move |s, e| {
        let (Some(s), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
        let Ok(el) = s.cast::<UIElement>() else { return };
        if let Ok(p) = e.GetPosition(&el) {
            with(|st| st.map_menu(p.X as f64, p.Y as f64));
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

    // The two tables. Positions are taken relative to the scrolled content (see tables::Rows).
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
    keep(find::<UIElement>(&root, "ExtScroll")?.KeyDown(|_, e| {
        let Some(e) = e.as_ref() else { return };
        let Ok(k) = e.Key() else { return };
        if with(|st| st.ext_key(k.0)).unwrap_or(false) {
            let _ = e.SetHandled(true);
        }
    })?);
    // a narrow window gives the tree more of the width beside the file types
    keep(root.SizeChanged(|s, _| {
        let Some(f) = s.as_ref().and_then(|s| s.cast::<FrameworkElement>().ok()) else { return };
        let (Ok(w), Ok(h)) = (f.ActualWidth(), f.ActualHeight()) else { return };
        with(|st| st.fit_size(w, h));
    })?);
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
    keep(find::<TextBox>(&root, "SearchBox")?.TextChanged(|s, _| {
        let Some(t) = s.as_ref().and_then(|s| s.cast::<TextBox>().ok()) else { return };
        let v = t.Text().map(|h| h.to_string_lossy()).unwrap_or_default();
        let caret = t.SelectionStart().unwrap_or(i32::MAX).max(0) as usize;
        with(|st| st.search_typed(v, caret));
    })?);
    // Enter or Down in the search box goes to the results; with suggestions showing, Up, Down,
    // Enter, Tab and Esc may be theirs (search_query::box_key)
    keep(find::<UIElement>(&root, "SearchBox")?.PreviewKeyDown(|_, e| {
        let Some(e) = e.as_ref() else { return };
        let Ok(k) = e.Key() else { return };
        if with(|st| st.search_box_key(k.0)).unwrap_or(false) {
            let _ = e.SetHandled(true);
        }
    })?);
    // Filter box and the Colours and Tweaks panel.
    keep(find::<TextBox>(&root, "FilterBox")?.TextChanged(|s, _| {
        let Some(t) = s.as_ref().and_then(|s| s.cast::<TextBox>().ok()) else { return };
        let v = t.Text().map(|h| h.to_string_lossy()).unwrap_or_default();
        with(|st| st.filter_changed(v));
    })?);
    for name in ["LiftSlider", "CushionSlider", "TiltSlider"] {
        keep(find::<RangeBase>(&root, name)?.ValueChanged(move |_, e| {
            let Some(v) = e.as_ref().and_then(|e| e.NewValue().ok()) else { return };
            with(|st| st.tweak(name, v));
        })?);
    }
    keep(find::<ToggleSwitch>(&root, "AnimSwitch")?.Toggled(|s, _| {
        let on = s.as_ref().and_then(|s| s.cast::<ToggleSwitch>().ok()).and_then(|t| t.IsOn().ok()).unwrap_or(true);
        with(|st| st.tweak("AnimSwitch", if on { 1.0 } else { 0.0 }));
    })?);
    keep(find::<Selector>(&root, "PaletteBox")?.SelectionChanged(|s, _| {
        let i = s.as_ref().and_then(|s| s.cast::<Selector>().ok()).and_then(|b| b.SelectedIndex().ok()).unwrap_or(0);
        with(|st| st.tweak("PaletteBox", i.max(0) as f64));
    })?);
    keep(find::<ButtonBase>(&root, "TweaksClose")?.Click(|_, _| {
        with(|st| st.command("mTweaks"));
    })?);

    // The menu of a search row from another drive.
    for (item, kind) in [("sxExplore", 'E'), ("sxCopy", 'C'), ("sxScan", 'S'), ("sxShell", 'W')] {
        if let Ok(i) = find::<MenuFlyoutItem>(&root, item) {
            keep(i.Click(move |_, _| {
                with(|s| s.more_menu_pick(kind));
            })?);
        }
    }

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
    // dev: SPRAWL_EMPTY=1 pictures the window as it opens, before any scan
    let empty = std::env::var_os("SPRAWL_EMPTY").is_some();
    let first = a.folder.clone().or_else(|| shot.as_ref().filter(|_| !empty).map(|_| PathBuf::from(r"C:\Program Files")));
    let state = State {
        window,
        hwnd,
        root,
        drives: drives.iter().map(|d| d.root.clone()).collect(),
        tree: None,
        alloc: Vec::new(),
        expanded: HashSet::new(),
        selected: None,
        marked: Default::default(),
        rows: Vec::new(),
        files: Vec::new(),
        exts: Vec::new(),
        colours: HashMap::new(),
        ext_selected: None,
        tables: tables::Tables::default(),
        full_tree: None,
        full_parts: None,
        advisor: Default::default(),
        tree_gen: 0,
        scan_id: 0,
        filter: String::new(),
        filter_text: String::new(),
        filter_pending: None,
        filtering: None,
        child_stack: Vec::new(),
        wizard_page: 0,
        wizard_all: false,
        wizard_offered: false,
        update: Default::default(),
        wiz_models: Default::default(),
        prefs: Default::default(),
        search: crate::search::Search::new(),
        hist: None,
        scan_taken: std::time::SystemTime::now(),
        scan_began: std::time::SystemTime::now(),
        settings,
        watcher: None,
        changes: 0,
        own_deletes: Vec::new(),
        shell_rescans: None,
        win_h: 0.0,
        admin_asked: Default::default(),
        pulses_on: true,
        map_root: ROOT,
        map: treemap::MapView::new(panel),
        hud: hud::Hud::default(),
        units: Units::Dynamic,
        view: View::Tree,
        job: None,
        scan_secs: 0.0,
        shot,
        shot_ticks: 0,
        welcome: None,
        welcome_drives: Vec::new(),
        welcome_recent: Vec::new(),
    };
    STATE.with(|s| *s.borrow_mut() = Some(state));
    with(|s| {
        s.apply_settings();
        // the welcome screen comes first; the wizard is offered when the first scan is done
        // dev: --screenshot with SPRAWL_WIZARD_PAGE=n pictures the wizard on page n
        if let Some(p) = s.shot.as_ref().and(std::env::var("SPRAWL_WIZARD_PAGE").ok()).and_then(|p| p.parse::<usize>().ok()) {
            s.wizard_open();
            s.wizard_page = p.min(wizard::PAGES - 1);
            s.wizard_show();
        }
        let restart = a.advisor && first.is_some();
        let tab = look::start_tab(look::start_choice(s.settings.num("startTab")), s.settings.get("lastTab"), first.is_some(), restart);
        if restart {
            // ticks only become ticks for findings of the new report; the review still asks
            s.advisor.cleanup.pending = Some(a.ticks.iter().take(500).map(|p| crate::cleanup::key(p)).collect());
            s.command("TabAdvisor");
        } else {
            s.open_on(tab);
        }
        if let Some(f) = first {
            s.start_scan(f);
        } else if let Some(c) = s.drives.first().cloned() {
            s.update_header(std::path::Path::new(&c));
        }
    });
    Ok(())
}

fn main() {
    // `sprawl --mcp`: an MCP server on stdin and stdout for Claude Code, no window
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    if args.first().map(String::as_str) == Some("--mcp") {
        sprawl_advisor::mcp::main(&args);
        return;
    }
    // `sprawl --uninstall`: from Settings > Apps
    if args.first().map(String::as_str) == Some("--uninstall") {
        sprawl_install::setup::uninstall(args.iter().any(|a| a == "/S"));
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        sprawl_advisor::llm::set_mcp_exe(exe);
    }
    bridge::start();
    sprawl_advisor::config::migrate_old_folders();
    // A panic inside a XAML callback aborts the process with no message; keep a note of it.
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("{info}
{}
", std::backtrace::Backtrace::force_capture());
        eprintln!("{msg}");
        if let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("Sprawl")) {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("crash.log"), msg);
        }
    }));
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
