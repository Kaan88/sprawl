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
mod bench;
mod ask;
mod bridge;
mod app;
mod cleanup;
mod commands;
mod coverage;
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
mod wiping;
mod views;
mod watch;
mod treemap;
mod ui;
mod look;
mod popups;
mod menu;
mod menubar;
mod reconcile;
mod restyle;
mod theme;
mod tokens;
mod themepage;
mod win32;
mod welcome;
mod wizard;
mod prefs;
mod update;
mod cleaners;
mod models;
mod search;
mod search_more;
mod search_delete;
mod search_rename;
mod search_query;
mod search_box;
mod shellmenu;
mod admin;
mod alert;
mod shade;
mod tabline;

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
use bindings::Microsoft::UI::Xaml::Controls::{ScrollViewer, SwapChainPanel, TextBox};
use bindings::Microsoft::UI::Xaml::Media::CompositionTarget;
use bindings::Microsoft::UI::Xaml::{Application, ApplicationInitializationCallback, ApplicationTheme, FrameworkElement, UIElement, Window};
use bindings::Windows::Graphics::SizeInt32;
use state::{find, keep, load, with, State, Units, View, STATE};

// ---------------------------------------------------------------- start-up

struct Args {
    shot: Option<PathBuf>,
    /// What to scan. Several, when a scan of several folders is opened again (a restart as
    /// administrator passes each real root, not the "Multiple" the tree is named after).
    folders: Vec<PathBuf>,
    /// Restarted as administrator from the Advisor: open it again, with these ticks.
    advisor: bool,
    ticks: Vec<PathBuf>,
    /// Ticks that did not fit on that restart's command line, to say once.
    ticks_left: usize,
    /// Started by an elevated copy to install this version without administrator rights.
    resume: Option<String>,
    /// Started by Setup when it finishes (Setup runs as administrator).
    from_setup: bool,
    /// Started by an elevated copy in Program Files, as the user, only to remove the per-user copy.
    remove_per_user: bool,
}

/// Setup's [Run] entries pass this (installer/sprawl.iss).
const FROM_SETUP_ARG: &str = "--from-setup";
/// The helper an elevated copy starts without administrator rights to remove the per-user copy.
const REMOVE_PER_USER_ARG: &str = "--remove-per-user";
/// How many ticks a restart as administrator had to leave behind (the command line is finite).
pub(crate) const TICKS_LEFT_ARG: &str = "--ticks-left";

fn args() -> Args {
    let mut a = Args { shot: None, folders: Vec::new(), advisor: false, ticks: Vec::new(), ticks_left: 0, resume: None, from_setup: false, remove_per_user: false };
    let mut it = std::env::args_os().skip(1);
    while let Some(x) = it.next() {
        if x == "--screenshot" {
            a.shot = it.next().map(PathBuf::from);
        } else if x == "--advisor" {
            a.advisor = true;
        } else if x == update::RESUME_ARG {
            a.resume = Some(it.next().map(|v| v.to_string_lossy().into_owned()).unwrap_or_default());
        } else if x == "--tick" {
            a.ticks.extend(it.next().map(PathBuf::from));
        } else if x == TICKS_LEFT_ARG {
            a.ticks_left = it.next().and_then(|v| v.to_string_lossy().parse().ok()).unwrap_or(0);
        } else if x == FROM_SETUP_ARG {
            a.from_setup = true;
        } else if x == REMOVE_PER_USER_ARG {
            a.remove_per_user = true;
        } else {
            a.folders.push(PathBuf::from(x));
        }
    }
    a
}

const COMMANDS: &[&str] = &[
    "mScan", "mSelectFolder", "mNewInstance", "mReset", "mExpandAll", "mParent", "mZoomIn", "mZoomOut", "mSavePng", "mExplore",
    "mCmd", "mCopyPath", "mDelete", "mDeleteForever", "mExportCsv", "mCopyInfo", "mExportTypes", "mExit", "uDyn", "uB", "uKB",
    "uMB", "uGB", "uTB", "oAuto", "oHeader", "oTypes", "oTreemap", "oAlloc", "oCushion", "oNames", "oTilt",
    "mSelectMultiple", "mSelectCsv", "mSettings", "mShortcuts", "mAbout", "mUpdate", "oUpdates", "mStop", "mReselectChild", "mFilter", "mSearch", "mAdmin", "oFree", "oMirror", "oPulses", "oAlwaysAdmin", "mWizard", "oExplorer", "aaOff", "aaLow", "aaMedium", "aaHigh", "mWipeFree",
];

/// Right-click menu on the tree: (item name, command).
const ROW_MENU: &[(&str, &str)] = &[
    ("cOpen", "mOpen"),
    ("cInTree", "mInTree"),
    ("cExplore", "mExplore"),
    ("cScan", "mScanHere"),
    ("cCmd", "mCmd"),
    ("cCopyPath", "mCopyPath"),
    ("cMapHere", "mMapHere"),
    ("cExpandAll", "mExpandAll"),
    ("cCopyInfo", "mCopyInfo"),
    ("cDelete", "mDelete"),
    ("cDeleteForever", "mDeleteForever"),
    ("cOverwrite", "mOverwrite"),
    ("cShell", "mShell"),
];

/// Installed for all users now: the per-user copy of 1.0.4 and before goes, once, by its own
/// uninstaller run as the user. "Scan with Sprawl" in Explorer, which it removes, comes back
/// pointing here.
fn remove_per_user_copy() {
    let menu = dialogs::explorer_menu_installed();
    if let Err(e) = sprawl_install::machine::remove_per_user_install(std::time::Duration::from_secs(60)) {
        eprintln!("per-user copy: {e}");
    }
    if menu {
        dialogs::set_explorer_menu(true);
    }
}

pub(crate) fn launch() -> windows_core::Result<()> {
    bench::mark("launch entered (OnLaunched)");
    let a = args();
    // The helper: remove the per-user copy as the user, then end, with no window. Only from the
    // copy in Program Files, and only without administrator rights (the per-user uninstaller is
    // in a folder the user can change, so it never runs elevated).
    if a.remove_per_user {
        if a.shot.is_none() && sprawl_core::elevation().ok() == Some(false) && sprawl_install::machine::is_protected_copy() {
            remove_per_user_copy();
        }
        std::process::exit(0);
    }
    // The per-user copy, once Sprawl is installed for all users (its old shortcut, or "Scan with
    // Sprawl" still pointing here): open the copy in Program Files instead, which removes this
    // one as it starts. Offering to install for all users again would only repeat the install.
    if a.shot.is_none() && a.resume.is_none() {
        if let Some(exe) = sprawl_install::machine::all_users_copy() {
            let started = if sprawl_core::is_elevated() {
                // never hand administrator rights on from this copy
                sprawl_core::identity::launch_unelevated(&exe, &[]).map(drop)
            } else {
                std::process::Command::new(&exe).args(std::env::args_os().skip(1)).spawn().map(drop)
            };
            if started.is_ok() {
                std::process::exit(0);
            }
        }
    }
    let settings = settings::Settings::load();
    // once, before any rule runs: the compression rule reads this snapshot, never the file
    sprawl_advisor::program_folders::load(settings.get(sprawl_advisor::program_folders::SETTING).unwrap_or(""));
    // Setup runs as administrator, and when Sprawl itself started it (an update, or Install for
    // all users), Setup has no other user to start the new Sprawl as: it comes up elevated. Open
    // again as the user, unless "Always run as administrator" is on.
    if a.from_setup && a.shot.is_none() && sprawl_core::is_elevated() && settings.bool("oAlwaysAdmin") != Some(true) {
        if let Ok(exe) = std::env::current_exe() {
            if sprawl_core::identity::launch_unelevated(&exe, &[]).is_ok() {
                std::process::exit(0);
            }
        }
    }
    // An elevated copy handing its update over: this copy stays without administrator rights.
    let update_resume = if a.shot.is_none() { update::resume_request(a.resume.clone()) } else { None };
    // Installed for all users now: remove the per-user copy. Elevated (started by Setup, or with
    // "Always run as administrator" on), a helper started as the user does it, so this copy never
    // runs the per-user uninstaller with administrator rights.
    if a.shot.is_none() && sprawl_install::machine::is_protected_copy() && sprawl_install::machine::per_user_install().is_some() {
        if !sprawl_core::is_elevated() {
            remove_per_user_copy();
        } else if let Ok(exe) = std::env::current_exe() {
            if let Err(e) = sprawl_core::identity::launch_unelevated(&exe, &[REMOVE_PER_USER_ARG]) {
                eprintln!("per-user copy: {e}");
            }
        }
    }
    // "Always run as administrator": come back elevated (the UAC prompt shows) and quit this copy.
    // Never for a hand-off, valid or not: that would loop back to an elevated copy. Only a copy
    // installed for all users: this one says why not at start.
    if a.shot.is_none() && a.resume.is_none() && settings.bool("oAlwaysAdmin") == Some(true) && !sprawl_core::is_elevated() && !sprawl_install::machine::may_elevate() {
        update::ADMIN_REFUSED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if a.shot.is_none() && a.resume.is_none() && settings.bool("oAlwaysAdmin") == Some(true) && !sprawl_core::is_elevated() && sprawl_install::machine::may_elevate() {
        let args: Vec<String> = std::env::args_os().skip(1).map(|x| dialogs::quote(std::path::Path::new(&x))).collect();
        if dialogs::restart_elevated(0, &args.join(" ")) {
            std::process::exit(0);
        }
    }
    // the low-space check's task names this exe: renew it, so an update or a moved copy keeps it right
    if a.shot.is_none() && (settings.bool("alertLow") == Some(true) || settings.bool("alertGrowth") == Some(true)) {
        std::thread::spawn(|| alert::schedule(true));
    }
    bench::mark("launch: settings read");
    // the map's device and shaders, made while the window and its page are set up
    gpu::prewarm();
    let drives = win32::drives();
    let labels: Vec<String> = drives.iter().map(|d| format!("[{}] {}", d.root.trim_end_matches('\\'), d.label)).collect();
    bench::mark("launch: drives listed");

    let window = Window::new()?;
    bench::mark("window created");
    let page = ui::page(&labels);
    bench::mark("page XAML written");
    let root: FrameworkElement = load(&page)?;
    bench::mark("page XAML loaded (first XAML load done)");
    bench::watch_root(&root);
    window.SetContent(&root.cast::<UIElement>()?)?;
    bench::mark("window: content set");
    window.SetTitle(&HSTRING::from("Sprawl"))?;
    look::apply_backdrop(&window);
    bench::mark("window: backdrop set");
    window.SetExtendsContentIntoTitleBar(true)?;
    bench::mark("window: content extended into the title bar");
    window.SetTitleBar(&find::<UIElement>(&root, "TitleBar")?)?;
    bench::mark("window: title bar set");
    // dev: SPRAWL_SIZE=1280x800 sets the window's size for a screenshot
    let (w, h) = std::env::var("SPRAWL_SIZE").ok().filter(|_| a.shot.is_some()).and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).unwrap_or((1680, 1080));
    window.AppWindow()?.Resize(SizeInt32 { Width: w, Height: h })?;
    // the minimise, maximise and close buttons in the theme's colours
    {
        use crate::bindings::Microsoft::UI::Windowing::TitleBarTheme;
        let theme = if crate::theme::light() { TitleBarTheme::Light } else { TitleBarTheme::Dark };
        let _ = window.AppWindow().and_then(|a| a.TitleBar()).and_then(|t| t.SetPreferredTheme(theme));
    }
    bench::mark("window: sized and themed");
    let hwnd = app::hwnd_of(&window)?;
    win32::set_app_icon(hwnd);
    bench::mark("window: icon set");

    // closing mid clean-up asks first (what is gone stays gone; the rest is left as it was)
    // (Sprawl's own box doesn't block: the close is called off, and "Close anyway" exits)
    win32::on_close(hwnd, || {
        if with(|s| s.advisor.cleanup.running.is_some() || s.advisor.deleting.is_some() || s.advisor.wiping.is_some()) == Some(true) {
            with(|s| s.ask_close());
            return false;
        }
        with(|s| s.save_window_place());
        true
    });
    // the window where it was left, or most of the screen (screenshots keep their fixed size)
    if a.shot.is_none() {
        let remembered = crate::settings::Settings::load().get("window").is_some_and(|v| win32::set_window_place(hwnd, v));
        if !remembered {
            win32::fit_work_area(hwnd);
        }
    }
    if a.shot.is_some() {
        // dev: SPRAWL_FRONT=1 shows the window in front, to click through a change by hand
        // (screenshot mode doesn't elevate, so it can be driven where a normal start can't)
        if std::env::var_os("SPRAWL_FRONT").is_some() {
            window.Activate()?;
        } else {
            win32::show_behind(hwnd);
        }
    } else {
        window.Activate()?;
    }
    bench::mark("window shown");

    let panel = bind(&root)?;
    bench::mark("page bound (bind)");

    // Everything that moves (scan polling, the overlay, the treemap) runs once per frame.
    keep(CompositionTarget::Rendering(|_, _| {
        crate::bench::frame_begin();
        // a menu that has just opened plays its opening before it is drawn (popups.rs)
        crate::popups::watch();
        // the menus: the bar's titles, the pointer moving over them, a menu's first item (menu.rs)
        crate::menu::tick();
        with(|s| s.frame());
        // a menu opened by this frame's own work (the bar's menu reopened where it belongs, a
        // sub-menu after the pointer rested, a step of a screenshot run): caught before the
        // frame is laid out and drawn, invisible and with its shadow from its first frame
        crate::popups::settle();
        if crate::bench::on() {
            crate::bench::frame_end(with(|s| s.view == View::Search).unwrap_or(false));
        }
    })?);

    let shot = a.shot.clone();
    // dev: SPRAWL_EMPTY=1 pictures the window as it opens, before any scan
    let empty = std::env::var_os("SPRAWL_EMPTY").is_some();
    let first = a.folders.first().cloned().or_else(|| shot.as_ref().filter(|_| !empty).map(|_| PathBuf::from(r"C:\Program Files")));
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
        update: update::Updater::resuming(update_resume),
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
        shell_rescans: Vec::new(),
        win_h: 0.0,
        win_w: 0.0,
        pulses_on: true,
        map_root: ROOT,
        map: treemap::MapView::new(panel),
        hud: hud::Hud::default(),
        units: Units::Dynamic,
        view: View::Tree,
        job: None,
        pending_scan: None,
        scan_secs: 0.0,
        coverage: None,
        shot,
        shot_ticks: 0,
        dialogs: Default::default(),
        ranks: Default::default(),
        ranks_job: None,
        tree_set: 0,
        kid_order: Default::default(),
        order_gen: 0,
        search_focus_wait: 0,
        welcome: None,
        welcome_drives: Vec::new(),
        welcome_recent: Vec::new(),
    };
    STATE.with(|s| *s.borrow_mut() = Some(state));
    with(|s| {
        s.apply_settings();
        // the line under the tabs is made now, under the tab the page starts on (a start on
        // another tab slides it there at its first layout, which is no slide: see tabline.rs)
        s.paint_tabs();
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
            s.advisor.cleanup.pending_left = a.ticks_left;
            s.command("TabAdvisor");
        } else {
            s.open_on(tab);
        }
        if a.folders.len() > 1 {
            s.start(state::Source::Folders(a.folders.clone()));
        } else if let Some(f) = first.clone().filter(|f| s.shot.is_some() && f.is_file() && f.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv"))) {
            // dev: a screenshot run given a .csv opens it as File > Open CSV would (the benchmark's
            // large synthetic scan, tools/bench.ps1)
            s.start(state::Source::Csv(f));
        } else if let Some(f) = first {
            s.start_scan(f);
        } else if let Some(c) = s.drives.first().cloned() {
            s.update_header(std::path::Path::new(&c));
        }
    });
    bench::mark("launch done (state made, start tab opened)");
    Ok(())
}

/// Everything the page's controls do, wired to the state by name. Run on the page made at start
/// and again on the one a theme change makes (`crate::restyle`); returns the map's panel.
pub(crate) fn bind(root: &FrameworkElement) -> windows_core::Result<SwapChainPanel> {
    // the menu bar: File, Options and Help, and what WinUI's MenuBar did on its own (the page's
    // menus start again here)
    crate::menubar::bind(root)?;
    // the page's other menus, drawn and run as the bar's are (menu.rs): the right-click menus
    // and the map bar's dropdowns
    for (set, how) in [("RowMenu", menu::How::Context), ("ColsMenu", menu::How::Context), ("ViewMoreFly", menu::How::Button), ("ViewPaintFly", menu::How::Button)] {
        crate::menu::bind(root, set, how)?;
    }
    // the Search list's columns: a check changes its column, however it was changed
    for (name, _, i) in crate::search::COL_MENU {
        crate::menu::on_mark(root, name, move |on| {
            with(|s| s.search_col_set(i, on));
        })?;
    }
    // The menu bar's rows all route to State::command by name (its checks and dots when their
    // mark changes, in menubar.rs).
    for &name in COMMANDS {
        if !crate::menubar::runs_on_click(name) {
            continue;
        }
        if let Ok(item) = find::<ButtonBase>(root, name) {
            keep(item.Click(move |_, _| {
                with(|s| s.command(name));
            })?);
        }
    }
    // the other views and their colour switch
    for name in crate::views::ALL.iter().map(|v| v.1).chain(crate::views::PAINTS.iter().map(|p| p.1)) {
        if let Ok(item) = find::<ButtonBase>(root, name) {
            keep(item.Click(move |_, _| {
                with(|s| s.command(name));
            })?);
        }
    }
    for name in wizard::BUTTONS {
        keep(find::<ButtonBase>(root, name)?.Click(move |_, _| {
            with(|s| s.wizard_command(name));
        })?);
    }
    keep(find::<Selector>(root, "WizProvider")?.SelectionChanged(|_, _| {
        with(|s| { s.wizard_ai_changed(false); s.wizard_provider(); });
    })?);
    keep(find::<Selector>(root, "WizUrlPreset")?.SelectionChanged(|_, _| {
        with(|s| s.wizard_preset());
    })?);
    for (name, address) in [("WizModel", false), ("WizUrl", true)] {
        keep(find::<TextBox>(root, name)?.TextChanged(move |_, _| {
            with(|s| s.wizard_ai_changed(address));
        })?);
    }
    keep(find::<ButtonBase>(root, "HistGrowth")?.Click(|_, _| {
        with(|s| {
            s.command("TabAdvisor");
            s.advisor_command("AdvGrowOpen");
        });
    })?);
    for name in ["HistClose", "HistPlay"] {
        keep(find::<ButtonBase>(root, name)?.Click(move |_, _| {
            with(|s| if name == "HistPlay" { s.hist_play() } else { s.hist_close() });
        })?);
    }
    keep(find::<RangeBase>(root, "HistSlider")?.ValueChanged(|_, e| {
        let Some(v) = e.as_ref().and_then(|e| e.NewValue().ok()) else { return };
        with(|st| {
            if st.hist.as_ref().is_some_and(|h| h.pos != v.round() as usize) {
                st.hist_seek(v.round().max(0.0) as usize);
            }
        });
    })?);
    for name in ["ScanButton", "CoverageDetails", "TabTree", "TabFiles", "TabSearch", "TabAdvisor", "TabCleaners", "SearchToFilter", "SearchDelete", "SearchMapBtn", "TabAbout", "ModeMap", "ModeCity", "ModeGalaxy", "ModeHist", "ViewOnly", "MapTweaks", "SetupYes", "SetupNo", "UpdateNow", "UpdateLater", "UpdateSkip", "DeleteStop"] {
        keep(find::<ButtonBase>(root, name)?.Click(move |_, _| {
            with(|s| s.command(name));
        })?);
    }
    keep(find::<Selector>(root, "DriveBox")?.SelectionChanged(|_, _| {
        with(|s| {
            let auto = crate::menubar::checked(&s.root, "oAuto").unwrap_or(false);
            if auto && s.shot.is_none() {
                s.command("ScanSelectedDrive");
            }
        });
    })?);
    // The 3D treemap panel.
    let panel: SwapChainPanel = find(root, "MapPanel")?;
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
        let sv: ScrollViewer = find(root, name)?;
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
        let header: FrameworkElement = find(root, if which == Which::Main { "TreeHeader" } else { "ExtHeader" })?;
        keep(header.Tapped(move |s, e| {
            let (Some(s), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
            let Ok(el) = s.cast::<UIElement>() else { return };
            let w = s.cast::<FrameworkElement>().and_then(|f| f.ActualWidth()).unwrap_or(0.0);
            if let Ok(p) = e.GetPosition(&el) {
                with(|st| st.header_clicked(which, p.X as f64, w));
            }
        })?);
        // a right-click on the Search list's header: which columns to show
        if which == Which::Main {
            keep(header.RightTapped(|s, e| {
                let (Some(s), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
                let Ok(el) = s.cast::<UIElement>() else { return };
                if let Ok(p) = e.GetPosition(&el) {
                    with(|st| st.search_cols_menu(p));
                }
            })?);
        }
    }
    keep(find::<UIElement>(root, "ExtScroll")?.KeyDown(|_, e| {
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
    let tree_sv: UIElement = find(root, "TreeScroll")?;
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
        with(|st| st.row_menu(y, at, false));
    })?);
    // the menu key or Shift+F10 on the table: the row menu of the keyboard's row (a right-click
    // raises this too, with a point: that one is RightTapped's)
    keep(tree_sv.ContextRequested(|s, e| {
        let (Some(s), Some(e)) = (s.as_ref(), e.as_ref()) else { return };
        let mut at = Point::default();
        if e.TryGetPosition(s, &mut at).unwrap_or(false) {
            return;
        }
        let _ = e.SetHandled(true);
        with(|st| st.row_menu_by_key());
    })?);
    keep(tree_sv.KeyDown(|_, e| {
        let Some(e) = e.as_ref() else { return };
        let Ok(k) = e.Key() else { return };
        if with(|st| st.table_key(k.0)).unwrap_or(false) {
            let _ = e.SetHandled(true);
        }
    })?);
    keep(find::<TextBox>(root, "SearchBox")?.TextChanged(|s, _| {
        let Some(t) = s.as_ref().and_then(|s| s.cast::<TextBox>().ok()) else { return };
        let v = t.Text().map(|h| h.to_string_lossy()).unwrap_or_default();
        let caret = t.SelectionStart().unwrap_or(i32::MAX).max(0) as usize;
        with(|st| st.search_typed(v, caret));
    })?);
    // Enter or Down in the search box goes to the results; with suggestions showing, Up, Down,
    // Enter, Tab and Esc may be theirs (search_query::box_key)
    keep(find::<UIElement>(root, "SearchBox")?.PreviewKeyDown(|_, e| {
        let Some(e) = e.as_ref() else { return };
        let Ok(k) = e.Key() else { return };
        if with(|st| st.search_box_key(k.0)).unwrap_or(false) {
            let _ = e.SetHandled(true);
        }
    })?);
    // a question open in Sprawl's own box has the keyboard (ask.rs dialog_key); with none, Esc
    // closes the setup wizard or Settings, as Cancel does (wizard.rs wizard_key)
    keep(root.cast::<UIElement>()?.PreviewKeyDown(|_, e| {
        let Some(e) = e.as_ref() else { return };
        let Ok(k) = e.Key() else { return };
        if with(|st| st.dialog_key(k.0) || st.wizard_key(k.0)).unwrap_or(false) {
            let _ = e.SetHandled(true);
        }
    })?);
    // Filter box.
    keep(find::<TextBox>(root, "FilterBox")?.TextChanged(|s, _| {
        let Some(t) = s.as_ref().and_then(|s| s.cast::<TextBox>().ok()) else { return };
        let v = t.Text().map(|h| h.to_string_lossy()).unwrap_or_default();
        with(|st| st.filter_changed(v));
    })?);

    // The row menu's items reuse the File menu's commands.
    for (item, cmd) in ROW_MENU {
        if let Ok(i) = find::<ButtonBase>(root, item) {
            keep(i.Click(move |_, _| {
                with(|s| s.command(cmd));
            })?);
        }
    }
    Ok(panel)
}

fn main() {
    let screenshot = crate::args().shot.is_some();
    settings::READ_ONLY.store(screenshot, std::sync::atomic::Ordering::Relaxed);
    sprawl_advisor::set_read_only(screenshot);
    // `sprawl --mcp`: an MCP server on stdin and stdout for Claude Code, no window
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    if !screenshot && args.first().map(String::as_str) == Some("--mcp") {
        sprawl_advisor::mcp::main(&args, || {
            let settings = settings::Settings::load();
            sprawl_advisor::program_folders::load(settings.get(sprawl_advisor::program_folders::SETTING).unwrap_or(""));
        });
        return;
    }
    // `sprawl --check`: the low-space check a scheduled task runs while the setting is on
    if !screenshot && args.first().map(String::as_str) == Some("--check") {
        alert::check();
        return;
    }
    // 1.0.1 started a copy with this to retire the old shared task; nothing is deleted now
    if args.first().map(String::as_str) == Some(sprawl_install::setup::MIGRATE_ARG) {
        return;
    }
    // `sprawl --uninstall`: from Settings > Apps
    if !screenshot && args.first().map(String::as_str) == Some("--uninstall") {
        sprawl_install::setup::uninstall(args.iter().any(|a| a == "/S"));
        return;
    }
    bench::mark("main entered");
    if let Ok(exe) = std::env::current_exe() {
        sprawl_advisor::llm::set_mcp_exe(exe);
    }
    if !screenshot {
        bridge::start();
        sprawl_advisor::config::migrate_old_folders();
        sprawl_advisor::overwrite::sweep_wipe_mounts();
        // a key from before hosts were recorded stays with the server it was saved for
        sprawl_advisor::config::record_key_host();
    }
    // A panic inside a XAML callback aborts the process with no message; keep a note of it.
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("{info}
{}
", std::backtrace::Backtrace::force_capture());
        eprintln!("{msg}");
        if let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("Sprawl")) {
            let _ = sprawl_advisor::write_local(&dir.join("crash.log"), msg.as_bytes(), true);
        }
    }));
    #[cfg(has_bootstrap)]
    if let Err(hr) = app::bootstrap::start() {
        eprintln!("Could not load the Windows App Runtime 1.8 (error 0x{:08X}).", hr as u32);
        std::process::exit(1);
    }
    bench::mark("Windows App Runtime loaded (bootstrap)");
    let result = Application::Start(&ApplicationInitializationCallback::new(|_| {
        bench::mark("WinUI started (Application::Start callback)");
        let app = Application::compose(App { provider: OnceLock::new() })?;
        let stored = crate::settings::Settings::load();
        crate::theme::init(stored.get("theme"));
        // the look (the Theme page's options) is set before any XAML loads
        crate::tokens::set(&crate::themepage::startup_look(&stored));
        app.SetRequestedTheme(if crate::theme::light() { ApplicationTheme::Light } else { ApplicationTheme::Dark })?;
        let _ = APP.set(app);
        Ok(())
    }));
    if let Err(e) = result {
        eprintln!("WinUI failed to start: {e:?}");
        std::process::exit(1);
    }
}
