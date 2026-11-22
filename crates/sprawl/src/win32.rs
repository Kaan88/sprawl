//! The few plain Win32 calls the app needs (drives, disk space, dialogs, clipboard,
//! window capture), declared by hand.
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::path::{Path, PathBuf};

pub type HWND = isize;

#[repr(C)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct BITMAPINFOHEADER {
    biSize: u32,
    biWidth: i32,
    biHeight: i32,
    biPlanes: u16,
    biBitCount: u16,
    biCompression: u32,
    biSizeImage: u32,
    biXPelsPerMeter: i32,
    biYPelsPerMeter: i32,
    biClrUsed: u32,
    biClrImportant: u32,
}

#[link(name = "user32")]
extern "system" {
    fn LoadImageW(inst: isize, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> isize;
    fn GetSystemMetrics(index: i32) -> i32;
    fn SendMessageW(h: HWND, msg: u32, w: usize, l: isize) -> isize;
    fn SetWindowPos(h: HWND, after: HWND, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    fn ShowWindow(h: HWND, cmd: i32) -> i32;
    fn GetForegroundWindow() -> HWND;
    fn PrintWindow(h: HWND, hdc: isize, flags: u32) -> i32;
    fn GetWindowRect(h: HWND, r: *mut RECT) -> i32;
    fn GetDC(h: HWND) -> isize;
    fn ReleaseDC(h: HWND, dc: isize) -> i32;
    fn OpenClipboard(h: HWND) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, mem: isize) -> isize;
    fn CloseClipboard() -> i32;
    fn GetClipboardData(format: u32) -> isize;
    fn EnumWindows(f: unsafe extern "system" fn(HWND, isize) -> i32, l: isize) -> i32;
    fn IsWindowVisible(h: HWND) -> i32;
    fn GetWindowThreadProcessId(h: HWND, pid: *mut u32) -> u32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(dc: isize) -> isize;
    fn CreateCompatibleBitmap(dc: isize, w: i32, h: i32) -> isize;
    fn SelectObject(dc: isize, obj: isize) -> isize;
    fn GetDIBits(dc: isize, bmp: isize, start: u32, lines: u32, bits: *mut c_void, info: *mut BITMAPINFOHEADER, usage: u32) -> i32;
    fn DeleteObject(obj: isize) -> i32;
    fn DeleteDC(dc: isize) -> i32;
    fn BitBlt(dst: isize, x: i32, y: i32, w: i32, h: i32, src: isize, sx: i32, sy: i32, rop: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> isize;
    fn GetLogicalDrives() -> u32;
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetVolumeInformationW(root: *const u16, name: *mut u16, name_len: u32, serial: *mut u32, max: *mut u32, flags: *mut u32, fs: *mut u16, fs_len: u32) -> i32;
    fn GetDiskFreeSpaceExW(root: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    fn GlobalAlloc(flags: u32, bytes: usize) -> isize;
    fn GlobalLock(mem: isize) -> *mut c_void;
    fn GlobalUnlock(mem: isize) -> i32;
}

#[link(name = "ole32")]
extern "system" {
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// One drive letter with its label, like "[C:] Local Disk".
pub struct Drive {
    pub root: String,
    pub label: String,
    /// GetDriveType: 2 removable, 3 fixed, 4 network, 5 optical.
    pub kind: u32,
}

pub fn drives() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{letter}:\\");
        let w = wide(&root);
        let kind = unsafe { GetDriveTypeW(w.as_ptr()) };
        let mut name = [0u16; 261];
        let ok = unsafe {
            GetVolumeInformationW(w.as_ptr(), name.as_mut_ptr(), 261, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0)
        };
        let mut label = if ok != 0 { from_wide(&name) } else { String::new() };
        if label.is_empty() {
            label = match kind {
                2 => "Removable Disk",
                4 => "Network Drive",
                5 => "CD Drive",
                _ => "Local Disk",
            }
            .to_string();
        }
        out.push(Drive { root, label, kind });
    }
    out
}

/// The fixed drives' roots (`C:\` style), by kind alone: nothing else is asked of any drive.
pub fn fixed_drives() -> Vec<String> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u32).filter(|i| mask & (1 << i) != 0).map(|i| format!("{}:\\", (b'A' + i as u8) as char))
        .filter(|root| unsafe { GetDriveTypeW(wide(root).as_ptr()) } == 3).collect()
}

/// (total, free) bytes of the volume holding `path`.
pub fn disk_space(path: &Path) -> Option<(u64, u64)> {
    let s = path.display().to_string();
    let root = if s.len() >= 2 && s.as_bytes()[1] == b':' { format!("{}\\", &s[..2]) } else { s };
    let w = wide(&root);
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) };
    (ok != 0).then_some((total, free))
}

#[repr(C)]
struct Placement {
    length: u32,
    flags: u32,
    show: u32,
    min: [i32; 2],
    max: [i32; 2],
    normal: RECT,
    device: RECT,
}

#[link(name = "user32")]
extern "system" {
    fn GetWindowPlacement(h: HWND, p: *mut Placement) -> i32;
    fn SetWindowPlacement(h: HWND, p: *const Placement) -> i32;
    fn SystemParametersInfoW(action: u32, param: u32, out: *mut c_void, flags: u32) -> i32;
}

/// Windows' "Animation effects" (Settings > Accessibility > Visual effects): off asks apps
/// for less motion. On when Windows can't say.
pub fn animations_on() -> bool {
    const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
    let mut on = 1i32;
    if unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut on as *mut i32).cast(), 0) } == 0 {
        return true;
    }
    on != 0
}

fn placement(h: HWND) -> Option<Placement> {
    let mut p = Placement { length: std::mem::size_of::<Placement>() as u32, flags: 0, show: 0, min: [0; 2], max: [0; 2], normal: RECT { left: 0, top: 0, right: 0, bottom: 0 }, device: RECT { left: 0, top: 0, right: 0, bottom: 0 } };
    (unsafe { GetWindowPlacement(h, &mut p) } != 0).then_some(p)
}

/// The window's normal position and size, and whether it is maximised: "x,y,w,h,max".
pub fn window_place(h: HWND) -> Option<String> {
    let p = placement(h)?;
    let r = &p.normal;
    Some(format!("{},{},{},{},{}", r.left, r.top, r.right - r.left, r.bottom - r.top, (p.show == 3) as u8))
}

/// Put the window back where `window_place` found it; false (and nothing done) when the
/// remembered place is off every screen's work area or makes no sense.
pub fn set_window_place(h: HWND, v: &str) -> bool {
    let n: Vec<i32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    let [x, y, w, hh, max] = n[..] else { return false };
    let mut work = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { SystemParametersInfoW(0x30, 0, (&mut work as *mut RECT).cast(), 0) };
    // the primary work area only: a window left on a monitor since unplugged starts fresh
    if w < 640 || hh < 400 || x + 100 > work.right || y < work.top - 20 || x + w < work.left + 100 || y + 60 > work.bottom {
        return false;
    }
    let Some(mut p) = placement(h) else { return false };
    p.normal = RECT { left: x, top: y, right: x + w, bottom: y + hh };
    p.show = if max == 1 { 3 } else { 1 };
    unsafe { SetWindowPlacement(h, &p) != 0 }
}

/// Size the window to most of the primary screen's work area, centred.
pub fn fit_work_area(h: HWND) {
    let mut work = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    if unsafe { SystemParametersInfoW(0x30, 0, (&mut work as *mut RECT).cast(), 0) } == 0 {
        return;
    }
    let (ww, wh) = (work.right - work.left, work.bottom - work.top);
    let (w, hh) = ((ww as f32 * 0.9) as i32, (wh as f32 * 0.9) as i32);
    unsafe {
        SetWindowPos(h, 0, work.left + (ww - w) / 2, work.top + (wh - hh) / 2, w, hh, 0x0004 | 0x0010); // no z-order, no activate
    }
}

#[link(name = "user32")]
extern "system" {
    fn GetKeyState(vk: i32) -> i16;
    fn GetDoubleClickTime() -> u32;
}

/// The user's double-click time (Mouse settings).
pub fn double_click_ms() -> u64 {
    match unsafe { GetDoubleClickTime() } {
        0 => 500,
        t => t as u64,
    }
}

/// (Ctrl, Shift) held down now.
pub fn ctrl_shift_down() -> (bool, bool) {
    unsafe { (GetKeyState(0x11) < 0, GetKeyState(0x10) < 0) }
}

/// Ctrl or Alt held down now (type-to-find leaves shortcuts alone).
pub fn ctrl_or_alt_down() -> bool {
    unsafe { GetKeyState(0x11) < 0 || GetKeyState(0x12) < 0 }
}

#[link(name = "comctl32")]
extern "system" {
    fn SetWindowSubclass(h: HWND, f: unsafe extern "system" fn(HWND, u32, usize, isize, usize, usize) -> isize, id: usize, data: usize) -> i32;
    fn DefSubclassProc(h: HWND, m: u32, w: usize, l: isize) -> isize;
}

static MAY_CLOSE: std::sync::OnceLock<fn() -> bool> = std::sync::OnceLock::new();

unsafe extern "system" fn close_proc(h: HWND, m: u32, w: usize, l: isize, _id: usize, _data: usize) -> isize {
    // WM_CLOSE: the close button, Alt+F4 and the taskbar's Close all come here
    if m == 0x0010 && MAY_CLOSE.get().is_some_and(|f| !f()) {
        return 0;
    }
    unsafe { DefSubclassProc(h, m, w, l) }
}

/// Ask `may_close` before the window closes; false keeps it open.
pub fn on_close(h: HWND, may_close: fn() -> bool) {
    let _ = MAY_CLOSE.set(may_close);
    unsafe {
        SetWindowSubclass(h, close_proc, 1, 0);
    }
}

/// `h` is the window the keyboard goes to.
pub fn is_foreground(h: HWND) -> bool {
    unsafe { GetForegroundWindow() == h }
}

pub fn copy_text(owner: HWND, text: &str) -> bool {
    let w = wide(text);
    unsafe {
        if OpenClipboard(owner) == 0 {
            return false;
        }
        EmptyClipboard();
        let mem = GlobalAlloc(0x0002, w.len() * 2);
        let ok = if mem != 0 {
            let p = GlobalLock(mem) as *mut u16;
            std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
            GlobalUnlock(mem);
            SetClipboardData(13, mem) != 0
        } else {
            false
        };
        CloseClipboard();
        ok
    }
}

/// The text on the clipboard (Unicode text), if there is some. Outside text: a caller takes
/// only what it knows from it.
pub fn paste_text(owner: HWND) -> Option<String> {
    unsafe {
        if OpenClipboard(owner) == 0 {
            return None;
        }
        let mem = GetClipboardData(13);
        let text = if mem != 0 {
            let p = GlobalLock(mem) as *const u16;
            let t = if p.is_null() {
                None
            } else {
                // at most 64 KB of text is read, and it ends at the first NUL
                let mut n = 0;
                while n < 32768 && *p.add(n) != 0 {
                    n += 1;
                }
                Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)))
            };
            GlobalUnlock(mem);
            t
        } else {
            None
        };
        CloseClipboard();
        text
    }
}

/// The Explorer command line that shows `path` selected: `/select,"C:\a b\c.txt"`.
///
/// Explorer parses this line itself rather than argv, and it splits the path at the first
/// space unless the quotes start right after the comma. Rust's `arg` quotes the whole
/// switch instead (`"/select,C:\a b\c.txt"`), which Explorer reads as far as
/// `C:\a` — a path that isn't there — and then opens Documents.
pub fn select_arg(path: &Path) -> String {
    format!("/select,\"{}\"", path.display())
}

/// Show `path` in Explorer: a file is selected inside its folder, a folder opens itself.
///
/// The shell call takes the path as a wide string, so spaces, commas, `&`, `#` and
/// non-ASCII names reach Explorer intact. (`SHOpenFolderAndSelectItems` given a folder's
/// own pidl opens its *parent* with the folder selected, so a folder is opened as a path,
/// which needs no `/select,` parsing at all.) `select_arg` is only the fallback.
pub fn reveal(path: &Path) -> bool {
    if path.is_dir() {
        return explorer().arg(path).spawn().is_ok();
    }
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};
    let w: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let pidl = ILCreateFromPathW(windows::core::PCWSTR(w.as_ptr()));
        if !pidl.is_null() {
            // a file's pidl as the folder: the item is selected in the folder holding it
            let ok = SHOpenFolderAndSelectItems(pidl, None, 0).is_ok();
            ILFree(Some(pidl));
            if ok {
                return true;
            }
        }
    }
    use std::os::windows::process::CommandExt;
    explorer().raw_arg(select_arg(path)).spawn().is_ok()
}

fn explorer() -> std::process::Command {
    let exe = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("explorer.exe")).unwrap_or_else(|| PathBuf::from("explorer.exe"));
    std::process::Command::new(exe)
}

/// Show without taking focus and put it behind the other windows.
pub fn show_behind(h: HWND) {
    unsafe {
        ShowWindow(h, 4); // SW_SHOWNOACTIVATE
        SetWindowPos(h, 1, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010); // HWND_BOTTOM, no size/move/activate
    }
}

/// The window's picture as RGBA rows (width, height, pixels).
pub fn capture(h: HWND) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetWindowRect(h, &mut r);
        let (w, hgt) = (r.right - r.left, r.bottom - r.top);
        if w <= 0 || hgt <= 0 {
            return None;
        }
        let screen = GetDC(0);
        let dc = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, hgt);
        let old = SelectObject(dc, bmp);
        let ok = PrintWindow(h, dc, 2) != 0; // PW_RENDERFULLCONTENT
        // dev: SPRAWL_SHOT_POPUPS puts the app's own popup windows (an open dropdown) in the
        // picture: PrintWindow of the main window leaves them out
        if std::env::var_os("SPRAWL_SHOT_POPUPS").is_some() {
            let mut found: Vec<HWND> = Vec::new();
            EnumWindows(collect_own, &mut found as *mut Vec<HWND> as isize);
            for p in found.into_iter().filter(|&p| p != h) {
                let mut pr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                GetWindowRect(p, &mut pr);
                let (pw, ph) = (pr.right - pr.left, pr.bottom - pr.top);
                if pw <= 0 || ph <= 0 {
                    continue;
                }
                let pdc = CreateCompatibleDC(screen);
                let pbmp = CreateCompatibleBitmap(screen, pw, ph);
                let pold = SelectObject(pdc, pbmp);
                if PrintWindow(p, pdc, 2) != 0 {
                    BitBlt(dc, pr.left - r.left, pr.top - r.top, pw, ph, pdc, 0, 0, 0x00CC_0020);
                }
                SelectObject(pdc, pold);
                DeleteObject(pbmp);
                DeleteDC(pdc);
            }
        }
        let mut info = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -hgt,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        let mut px = vec![0u8; (w * hgt * 4) as usize];
        SelectObject(dc, old);
        let lines = GetDIBits(dc, bmp, 0, hgt as u32, px.as_mut_ptr() as *mut c_void, &mut info, 0);
        DeleteObject(bmp);
        DeleteDC(dc);
        ReleaseDC(0, screen);
        if !ok || lines == 0 {
            return None;
        }
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2);
            p[3] = 255;
        }
        Some((w as u32, hgt as u32, px))
    }
}

/// dev (screenshot runs): EnumWindows callback gathering the visible windows of this process.
unsafe extern "system" fn collect_own(h: HWND, l: isize) -> i32 {
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, &mut pid);
    if pid == std::process::id() && IsWindowVisible(h) != 0 {
        (*(l as *mut Vec<HWND>)).push(h);
    }
    1
}

/// dev (screenshot runs): open the dropdown whose AutomationId (its x:Name) is `id`, through
/// UI Automation, as a screen reader would; the popup is part of the picture then.
pub fn expand_dropdown(h: HWND, id: &str) -> bool {
    use windows::core::BSTR;
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomationExpandCollapsePattern, TreeScope_Descendants, UIA_AutomationIdPropertyId, UIA_ExpandCollapsePatternId, UIA_NamePropertyId};
    let run = || -> windows::core::Result<()> {
        unsafe {
            let ua: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            let root = ua.ElementFromHandle(windows::Win32::Foundation::HWND(h as *mut _))?;
            let cond = ua.CreatePropertyCondition(UIA_AutomationIdPropertyId, &VARIANT::from(BSTR::from(id)))?;
            // by its id (a control's x:Name), else by its name (a menu bar's "File")
            let found = match root.FindFirst(TreeScope_Descendants, &cond) {
                Ok(f) => f,
                Err(_) => root.FindFirst(TreeScope_Descendants, &ua.CreatePropertyCondition(UIA_NamePropertyId, &VARIANT::from(BSTR::from(id)))?)?,
            };
            let pattern: IUIAutomationExpandCollapsePattern = windows::core::Interface::cast(&found.GetCurrentPattern(UIA_ExpandCollapsePatternId)?)?;
            pattern.Expand()
        }
    };
    run().is_ok()
}

/// Give the window the app's icon (resource 1, embedded by build.rs) for the title bar,
/// taskbar and Alt+Tab.
pub fn set_app_icon(h: HWND) {
    const IMAGE_ICON: u32 = 1;
    const WM_SETICON: u32 = 0x0080;
    unsafe {
        let inst = GetModuleHandleW(std::ptr::null());
        for (which, cx, cy) in [(1usize, 11, 12), (0usize, 49, 50)] {
            // ICON_BIG uses SM_CXICON/SM_CYICON, ICON_SMALL SM_CXSMICON/SM_CYSMICON
            let icon = LoadImageW(inst, 1 as *const u16, IMAGE_ICON, GetSystemMetrics(cx), GetSystemMetrics(cy), 0);
            if icon != 0 {
                SendMessageW(h, WM_SETICON, which, icon);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::select_arg;
    use std::path::Path;

    fn unquoted(arg: &str) -> &str {
        arg.strip_prefix("/select,\"").and_then(|a| a.strip_suffix('"')).expect("the switch, then one quoted path")
    }

    #[test]
    fn select_arg_quotes_the_path_after_the_comma() {
        // the reported case: the old `arg(format!("/select,{}", …))` let Rust quote the
        // whole switch, and Explorer stopped at "Downloads" and opened Documents
        let p = Path::new(r"C:\Users\Someone\Downloads Archive\Old Files\Scripts and tools\tray tool.py");
        let arg = select_arg(p);
        assert_eq!(arg, r#"/select,"C:\Users\Someone\Downloads Archive\Old Files\Scripts and tools\tray tool.py""#);
        assert_eq!(unquoted(&arg), p.display().to_string());
        assert!(!arg.contains("\"/select"), "the switch itself is never quoted");
    }

    #[test]
    fn select_arg_keeps_commas_ampersands_hashes_and_unicode_inside_the_quotes() {
        for p in [r"C:\a, b\c, d.txt", r"C:\x&y\a & b #1.txt", r"C:\Ölçüm\şğü ı İ.txt", r"C:\plain\file.py", r"C:\"] {
            let arg = select_arg(Path::new(p));
            assert_eq!(arg, format!("/select,\"{p}\""));
            assert_eq!(unquoted(&arg), p);
        }
    }
}
