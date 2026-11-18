//! The low-space alert (F6). `sprawl --check`, run every few hours by a scheduled task while
//! the setting is on, looks at the free space of each fixed drive and, when one is below the
//! limit, shows one Windows notice. A click on the notice opens Sprawl on that drive; nothing is
//! scanned, cleaned or sent by the check itself. One notice per drive per week at most.
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;

use crate::win32::{wide, HWND};

/// The name of the scheduled task, as Task Scheduler shows it.
fn task_name() -> Option<String> { sprawl_install::setup::alert_task_name().ok() }
/// Hours between two checks.
const EVERY_HOURS: u32 = 4;
/// A drive warned about is not warned about again for this long, unless it recovers first.
const QUIET_SECS: u64 = 7 * 24 * 3600;
const GB: u64 = 1 << 30;
const NO_WINDOW: u32 = 0x0800_0000;
/// The daily light scan runs when the last one is this old (a little under a day, as the
/// check runs every 4 hours).
const DAILY_SECS: u64 = 20 * 3600;
const PROCESS_MODE_BACKGROUND_BEGIN: u32 = 0x0010_0000;

fn state_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Sprawl").join("alerts.ini"))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn load_state() -> BTreeMap<String, u64> {
    let text = state_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    text.lines().filter_map(|l| l.rsplit_once('=')).filter_map(|(k, v)| Some((k.trim().to_string(), v.trim().parse().ok()?))).collect()
}

fn save_state(s: &BTreeMap<String, u64>) {
    let Some(p) = state_path() else { return };
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, s.iter().map(|(k, v)| format!("{k}={v}\r\n")).collect::<String>());
}

/// Which drives to warn about now, and the state to keep. `drives` is (root, free bytes); a
/// drive back above the limit is forgotten, so its next drop is said at once.
pub fn due(drives: &[(String, u64)], limit: u64, mut state: BTreeMap<String, u64>, now: u64) -> (Vec<String>, BTreeMap<String, u64>) {
    let mut warn = Vec::new();
    for (root, free) in drives {
        if *free >= limit {
            state.remove(root);
        } else if state.get(root).is_none_or(|&t| now.saturating_sub(t) >= QUIET_SECS) {
            state.insert(root.clone(), now);
            warn.push(root.clone());
        }
    }
    (warn, state)
}

/// The limit the setting holds, in bytes.
pub fn limit(settings: &crate::settings::Settings) -> u64 {
    let i = settings.num("alertFreeGb").unwrap_or(1.0) as usize;
    crate::prefs::ALERT_GB[i.min(crate::prefs::ALERT_GB.len() - 1)] * GB
}

/// `sprawl --check`: the check the scheduled task runs.
pub fn check() {
    let settings = crate::settings::Settings::load();
    // dev: SPRAWL_ALERT_TEST=1 shows the notice for every drive and every folder that grew, keeps
    // no state, scans nothing, needs no setting
    // (SPRAWL_ALERT_TEST=grow: the growth part only)
    let test_as = std::env::var("SPRAWL_ALERT_TEST").ok();
    let test = test_as.is_some();
    let low_on = test_as.as_deref().is_some_and(|v| v != "grow") || settings.bool("alertLow") == Some(true);
    let grow_on = test || settings.bool("alertGrowth") == Some(true);
    // both switched off while a task was left behind: say nothing
    if !low_on && !grow_on {
        return;
    }
    let mut state = if test { BTreeMap::new() } else { load_state() };
    let now = now();
    let mut lines: Vec<String> = Vec::new();
    let mut title = String::new();
    let mut open = String::new();
    if low_on {
        let drives: Vec<(String, u64, u64)> = crate::win32::drives()
            .into_iter()
            .filter(|d| d.kind == 3)
            .filter_map(|d| crate::win32::disk_space(std::path::Path::new(&d.root)).map(|(total, free)| (d.root, total, free)))
            .collect();
        let free: Vec<(String, u64)> = drives.iter().map(|(r, _, f)| (r.clone(), *f)).collect();
        let (warn, st) = due(&free, if test { u64::MAX } else { limit(&settings) }, state, now);
        state = st;
        if !warn.is_empty() {
            lines.extend(
                warn.iter()
                    .filter_map(|r| drives.iter().find(|d| &d.0 == r))
                    .map(|(r, total, f)| format!("{} has {} free of {}.", r.trim_end_matches('\\'), gb(*f), gb(*total))),
            );
            title = if warn.len() == 1 { format!("Drive {} is nearly full", warn[0].trim_end_matches('\\')) } else { "Drives are nearly full".to_string() };
            // the fullest drive is the one a click opens
            open = warn.iter().min_by_key(|r| free.iter().find(|d| &d.0 == *r).map_or(0, |d| d.1)).cloned().unwrap_or_default();
        }
    }
    if grow_on {
        let grew = growth_alerts(&settings, &mut state, now, test);
        if !grew.is_empty() {
            lines.extend(grew.iter().map(|(_, folder, delta, days)| format!("{} grew {} in {} days.", folder.display(), gb(*delta), days)));
            if title.is_empty() {
                title = if grew.len() == 1 { "A folder grew fast".to_string() } else { "Folders grew fast".to_string() };
                open = grew[0].0.display().to_string();
            }
        }
    }
    if !test {
        save_state(&state);
    }
    if lines.is_empty() {
        return;
    }
    lines.truncate(4);
    let body = format!("{}\nClick to see what takes the space.", lines.join("\n"));
    // dev: with SPRAWL_ALERT_TEST, SPRAWL_ALERT_DRY prints the notice instead of showing it
    if test && std::env::var_os("SPRAWL_ALERT_DRY").is_some() {
        println!("{title}\n{body}\nopens: {open}");
        return;
    }
    if notify(&title, &body) {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).arg(&open).spawn();
        }
    }
}

/// The growth part: (the scanned root to open, the folder that grew, by how much, over how many
/// days). From History's snapshots of the user's own scans, plus, when the user allowed it, a
/// daily folders-only scan of their user folder made here. Each scan is looked at once; a folder
/// is named at most once a week.
fn growth_alerts(settings: &crate::settings::Settings, state: &mut BTreeMap<String, u64>, now: u64, test: bool) -> Vec<(PathBuf, PathBuf, u64, u64)> {
    use sprawl_advisor::history;
    let Some(base) = history::base_dir() else { return Vec::new() };
    let i = settings.num("alertGrowGb").unwrap_or(1.0) as usize;
    let limit = if test { 0 } else { crate::prefs::GROW_GB[i.min(crate::prefs::GROW_GB.len() - 1)] * GB };
    // the daily light scan: only with History on, as it is saved there
    if !test && settings.bool("alertDailyScan") == Some(true) && settings.bool("snapshots") == Some(true) && now.saturating_sub(state.get("scan").copied().unwrap_or(0)) >= DAILY_SECS {
        state.insert("scan".into(), now);
        if let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
            // low priority for CPU and disk while it reads
            unsafe { SetPriorityClass(-1, PROCESS_MODE_BACKGROUND_BEGIN) };
            let progress = sprawl_advisor::exclusions::progress();
            if let Ok(tree) = sprawl_core::walk::walk_with(&profile, &progress, 2) {
                let cap = (settings.num("snapshot_cap_mb").unwrap_or(1024.0) * 1024.0 * 1024.0) as u64;
                let _ = history::prepare(&tree, std::time::SystemTime::now(), false).save(&base, cap);
            }
        }
    }
    let entries = history::list(&base);
    let mut out = Vec::new();
    for (root, when, _) in history::last_scans(&base) {
        let t = secs(when);
        // a scan looked at already, or too old to be news
        let seen = format!("seen|{}", root.display());
        if !test && (state.get(&seen) == Some(&t) || now.saturating_sub(t) > 2 * 86_400) {
            continue;
        }
        state.insert(seen, t);
        let Some(e) = entries.iter().filter(|e| e.root == root && secs(e.taken) == t).max_by_key(|e| e.full) else { continue };
        let Ok(cur) = history::load(e) else { continue };
        let facts = sprawl_advisor::facts::Facts::default();
        let Some(g) = sprawl_advisor::growth::growth(&cur, &base, &facts, None, e.taken) else { continue };
        let Some(since) = g.since else { continue };
        let days = e.taken.duration_since(since).map(|d| d.as_secs().div_ceil(86_400)).unwrap_or(1).max(1);
        for gr in g.grew.iter().filter(|gr| gr.delta > 0 && gr.delta as u64 >= limit) {
            let folder = cur.path_of(gr.node);
            let key = format!("grew|{}", folder.display());
            if !test && state.get(&key).is_some_and(|&w| now.saturating_sub(w) < QUIET_SECS) {
                continue;
            }
            state.insert(key, now);
            out.push((root.clone(), folder, gr.delta as u64, days));
        }
    }
    out.sort_by_key(|g| std::cmp::Reverse(g.2));
    out
}

fn secs(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn gb(b: u64) -> String {
    let g = b as f64 / GB as f64;
    if g >= 100.0 {
        format!("{g:.0} GB")
    } else {
        format!("{g:.1} GB")
    }
}

/// The command the scheduled task runs.
fn task_command(exe: &std::path::Path) -> String {
    format!("\"{}\" --check", exe.display())
}

/// The arguments for schtasks: create (or replace) the task, or delete it.
pub fn schtasks_args(on: bool, exe: &std::path::Path) -> Vec<String> {
    let Some(task) = task_name() else { return Vec::new(); };
    if on {
        ["/Create", "/TN", &task, "/TR", &task_command(exe), "/SC", "HOURLY", "/MO", &EVERY_HOURS.to_string(), "/RL", "LIMITED", "/F"].iter().map(|s| s.to_string()).collect()
    } else {
        ["/Delete", "/TN", &task, "/F"].iter().map(|s| s.to_string()).collect()
    }
}

/// Create or remove the scheduled task, for this account only (no administrator). Never for a
/// screenshot run. True when schtasks did it.
pub fn schedule(on: bool) -> bool {
    if crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    if sprawl_install::setup::per_user_only().is_err() { return false; }
    let Ok(exe) = std::env::current_exe() else { return false };
    let Ok(sys) = sprawl_install::setup::system_dir() else { return false; };
    let mut c = std::process::Command::new(sys.join("schtasks.exe"));
    c.current_dir(&sys).creation_flags(NO_WINDOW).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // Rust quotes each argument the way schtasks reads its command line (the quotes in /TR too)
    let args = schtasks_args(on, &exe);
    if args.is_empty() { return false; }
    c.args(args);
    let done = c.status().is_ok_and(|s| s.success());
    if done { sprawl_install::setup::migrate_alert_task(); }
    done
}

// ---- the notice: a tray icon with a balloon (Windows 10 and 11 show it as a notification) ----

#[repr(C)]
struct NOTIFYICONDATAW {
    cbSize: u32,
    hWnd: HWND,
    uID: u32,
    uFlags: u32,
    uCallbackMessage: u32,
    hIcon: isize,
    szTip: [u16; 128],
    dwState: u32,
    dwStateMask: u32,
    szInfo: [u16; 256],
    uVersion: u32,
    szInfoTitle: [u16; 64],
    dwInfoFlags: u32,
    guidItem: [u32; 4],
    hBalloonIcon: isize,
}

#[repr(C)]
struct WNDCLASSW {
    style: u32,
    lpfnWndProc: unsafe extern "system" fn(HWND, u32, usize, isize) -> isize,
    cbClsExtra: i32,
    cbWndExtra: i32,
    hInstance: isize,
    hIcon: isize,
    hCursor: isize,
    hbrBackground: isize,
    lpszMenuName: *const u16,
    lpszClassName: *const u16,
}

#[repr(C)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wParam: usize,
    lParam: isize,
    time: u32,
    pt: [i32; 2],
    private: u32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(c: *const WNDCLASSW) -> u16;
    fn CreateWindowExW(ex: u32, class: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: isize, inst: isize, param: *mut c_void) -> HWND;
    fn DefWindowProcW(h: HWND, msg: u32, w: usize, l: isize) -> isize;
    fn GetMessageW(m: *mut MSG, h: HWND, min: u32, max: u32) -> i32;
    fn TranslateMessage(m: *const MSG) -> i32;
    fn DispatchMessageW(m: *const MSG) -> isize;
    fn PostQuitMessage(code: i32);
    fn SetTimer(h: HWND, id: usize, ms: u32, f: *const c_void) -> usize;
    fn DestroyWindow(h: HWND) -> i32;
    fn LoadImageW(inst: isize, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> isize;
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> isize;
    fn SetPriorityClass(process: isize, class: u32) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn Shell_NotifyIconW(msg: u32, data: *const NOTIFYICONDATAW) -> i32;
}

const WM_TIMER: u32 = 0x0113;
const WM_TRAY: u32 = 0x8000 + 1;
const NIN_BALLOONHIDE: isize = 0x403;
const NIN_BALLOONTIMEOUT: isize = 0x404;
const NIN_BALLOONUSERCLICK: isize = 0x405;
static CLICKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

unsafe extern "system" fn proc(h: HWND, msg: u32, w: usize, l: isize) -> isize {
    match msg {
        WM_TRAY => match l & 0xFFFF {
            NIN_BALLOONUSERCLICK => {
                CLICKED.store(true, std::sync::atomic::Ordering::Relaxed);
                PostQuitMessage(0);
                0
            }
            NIN_BALLOONTIMEOUT | NIN_BALLOONHIDE => {
                PostQuitMessage(0);
                0
            }
            _ => 0,
        },
        WM_TIMER => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(h, msg, w, l),
    }
}

fn copy(dst: &mut [u16], s: &str) {
    let room = dst.len() - 1;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16().take(room)) {
        *d = c;
    }
}

/// Show the notice and wait (two minutes at most) for it to be clicked or to go. True when it
/// was clicked.
fn notify(title: &str, body: &str) -> bool {
    unsafe {
        let inst = GetModuleHandleW(std::ptr::null());
        let class = wide("SprawlAlert");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: proc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: inst,
            hIcon: 0,
            hCursor: 0,
            hbrBackground: 0,
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let name = wide("Sprawl");
        let h = CreateWindowExW(0, class.as_ptr(), name.as_ptr(), 0, 0, 0, 0, 0, 0, 0, inst, std::ptr::null_mut());
        if h == 0 {
            return false;
        }
        // the app's icon (resource 1), small size
        let icon = LoadImageW(inst, 1 as *const u16, 1, GetSystemMetrics(49), GetSystemMetrics(50), 0);
        let mut d: NOTIFYICONDATAW = std::mem::zeroed();
        d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        d.hWnd = h;
        d.uID = 1;
        // message, icon, tip, info
        d.uFlags = 0x1 | 0x2 | 0x4 | 0x10;
        d.uCallbackMessage = WM_TRAY;
        d.hIcon = icon;
        copy(&mut d.szTip, "Sprawl");
        copy(&mut d.szInfoTitle, title);
        copy(&mut d.szInfo, body);
        // NIIF_WARNING
        d.dwInfoFlags = 0x2;
        let shown = Shell_NotifyIconW(0, &d) != 0;
        if shown {
            d.uVersion = 4;
            Shell_NotifyIconW(4, &d);
            SetTimer(h, 1, 120_000, std::ptr::null());
            let mut m: MSG = std::mem::zeroed();
            while GetMessageW(&mut m, 0, 0, 0) > 0 {
                TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            Shell_NotifyIconW(2, &d);
        }
        DestroyWindow(h);
        shown && CLICKED.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_a_week_and_again_after_recovering() {
        let limit = 10 * GB;
        let low = vec![("C:\\".to_string(), 3 * GB), ("D:\\".to_string(), 50 * GB)];
        let (w, s) = due(&low, limit, BTreeMap::new(), 1000);
        assert_eq!(w, vec!["C:\\".to_string()]);
        // a few hours later: quiet
        let (w, s) = due(&low, limit, s, 1000 + 4 * 3600);
        assert!(w.is_empty());
        // a week later: said again
        let (w, s) = due(&low, limit, s, 1000 + QUIET_SECS);
        assert_eq!(w.len(), 1);
        // freed, then low again: said at once
        let ok = vec![("C:\\".to_string(), 30 * GB)];
        let (w, s) = due(&ok, limit, s, 1000 + QUIET_SECS + 10);
        assert!(w.is_empty() && s.is_empty());
        let (w, _) = due(&low, limit, s, 1000 + QUIET_SECS + 20);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn the_task_runs_this_exe_with_check() {
        let exe = std::path::Path::new(r"C:\Program Files\Sprawl\sprawl.exe");
        let a = schtasks_args(true, exe);
        let task = task_name().unwrap();
        assert_eq!(a[..3], ["/Create", "/TN", &task]);
        assert_eq!(a[4], "\"C:\\Program Files\\Sprawl\\sprawl.exe\" --check");
        assert!(a.ends_with(&["/F".to_string()]));
        assert!(a.windows(2).any(|pair| pair == ["/RL", "LIMITED"]));
        assert_eq!(schtasks_args(false, exe), ["/Delete", "/TN", &task, "/F"]);
    }
}
