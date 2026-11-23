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
    fn PostMessageW(h: HWND, msg: u32, w: usize, l: isize) -> i32;
    fn EnumChildWindows(parent: HWND, f: unsafe extern "system" fn(HWND, isize) -> i32, l: isize) -> i32;
    fn GetClassNameW(h: HWND, name: *mut u16, len: i32) -> i32;
    fn GetKeyboardState(state: *mut u8) -> i32;
    fn SetKeyboardState(state: *const u8) -> i32;
    fn MapVirtualKeyW(code: u32, kind: u32) -> u32;
    fn GetCursorPos(p: *mut POINT) -> i32;
    fn ScreenToClient(h: HWND, p: *mut POINT) -> i32;
    fn WindowFromPoint(p: POINT) -> HWND;
    fn GetAncestor(h: HWND, flags: u32) -> HWND;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct POINT {
    x: i32,
    y: i32,
}

/// Where the pointer is, in the client pixels of window `h`, when it is over that window (not
/// over another one in front of it).
pub fn cursor_in(h: HWND) -> Option<(i32, i32)> {
    unsafe {
        let mut p = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut p) == 0 {
            return None;
        }
        let under = WindowFromPoint(p);
        if under == 0 || GetAncestor(under, 2) != h {
            return None; // GA_ROOT
        }
        ScreenToClient(h, &mut p);
        Some((p.x, p.y))
    }
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
    fn GlobalFree(mem: isize) -> isize;
    fn MoveFileW(from: *const u16, to: *const u16) -> i32;
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

/// How long the pointer rests on a menu's item before its sub-menu opens (Windows' own
/// setting, SPI_GETMENUSHOWDELAY), in milliseconds: 400 when Windows can't say.
pub fn menu_show_delay() -> u32 {
    const SPI_GETMENUSHOWDELAY: u32 = 0x006A;
    let mut ms = 400u32;
    if unsafe { SystemParametersInfoW(SPI_GETMENUSHOWDELAY, 0, (&mut ms as *mut u32).cast(), 0) } == 0 {
        return 400;
    }
    ms
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

/// The first window's size, in DIPs: a normal app window, not the whole of a wide screen.
pub const FIRST_WINDOW: (f64, f64) = (1440.0, 920.0);

/// The first window's size in pixels on a work area of `ww` by `wh` pixels at `scale`:
/// [`FIRST_WINDOW`], never more than 90% of the work area, never less than the smallest
/// window (look.rs `MIN_WINDOW`) unless the work area itself is smaller.
pub fn first_size(ww: i32, wh: i32, scale: f64) -> (i32, i32) {
    let fit = |want: f64, least: f64, room: i32| {
        let most = room as f64 * 0.9;
        (want * scale).min(most).max((least * scale).min(room as f64)).round() as i32
    };
    let (min_w, min_h) = crate::look::MIN_WINDOW;
    (fit(FIRST_WINDOW.0, min_w, ww), fit(FIRST_WINDOW.1, min_h, wh))
}

#[link(name = "user32")]
extern "system" {
    fn GetDpiForWindow(h: HWND) -> u32;
}

/// Give a window with no remembered place its first size ([`first_size`]), centred on the
/// primary screen's work area.
pub fn fit_work_area(h: HWND) {
    let mut work = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    if unsafe { SystemParametersInfoW(0x30, 0, (&mut work as *mut RECT).cast(), 0) } == 0 {
        return;
    }
    let (ww, wh) = (work.right - work.left, work.bottom - work.top);
    let dpi = unsafe { GetDpiForWindow(h) };
    let scale = if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 };
    let (w, hh) = first_size(ww, wh, scale);
    unsafe {
        SetWindowPos(h, 0, work.left + (ww - w) / 2, work.top + (wh - hh) / 2, w, hh, 0x0004 | 0x0010); // no z-order, no activate
    }
}

#[cfg(test)]
mod first_size_tests {
    use super::first_size;

    #[test]
    fn a_wide_screen_gets_a_normal_window_and_a_small_one_still_fits() {
        // 3440 x 1400 at 100%: the normal size, not 90% of the width
        assert_eq!(first_size(3440, 1400, 1.0), (1440, 920));
        // a 4K screen at 150%: the same size in DIPs
        assert_eq!(first_size(3840, 2100, 1.5), (2160, 1380));
        // a 1366 x 728 laptop: 90% of it, but never under the smallest window
        assert_eq!(first_size(1366, 728, 1.0), (1229, 655));
        // a screen smaller than the smallest window: the whole work area
        assert_eq!(first_size(900, 560, 1.0), (900, 560));
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

/// A mouse button held down now (as this thread's window last saw it).
pub fn mouse_button_down() -> bool {
    // VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2
    [0x01, 0x02, 0x04, 0x05, 0x06].iter().any(|&vk| unsafe { GetKeyState(vk) } < 0)
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

/// A DROPFILES block (CF_HDROP): the 20-byte header (offset of the names, a point, "not client
/// area", "wide names") and every path as UTF-16 ending in NUL, then one more NUL.
pub fn hdrop_bytes(paths: &[std::path::PathBuf]) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    let mut b = Vec::new();
    for v in [20u32, 0, 0, 0, 1] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for p in paths {
        for c in p.as_os_str().encode_wide().chain(std::iter::once(0)) {
            b.extend_from_slice(&c.to_le_bytes());
        }
    }
    b.extend_from_slice(&0u16.to_le_bytes());
    b
}

/// A block of global memory (GMEM_MOVEABLE | GMEM_ZEROINIT) holding `bytes`, for the clipboard;
/// 0 when it can't be made (nothing is left allocated then).
unsafe fn global_with(bytes: &[u8]) -> isize {
    let mem = GlobalAlloc(0x0002 | 0x0040, bytes.len());
    if mem == 0 {
        return 0;
    }
    let p = GlobalLock(mem) as *mut u8;
    if p.is_null() {
        GlobalFree(mem);
        return 0;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    GlobalUnlock(mem);
    mem
}

/// Put the block `mem` on the clipboard as `format`. The system owns the block only once
/// SetClipboardData has taken it; on any failure it is freed here. The clipboard is opened (and
/// so emptied) only now that there is something to put on it.
unsafe fn clipboard_take(owner: HWND, format: u32, mem: isize) -> bool {
    if mem == 0 {
        return false;
    }
    if OpenClipboard(owner) == 0 {
        GlobalFree(mem);
        return false;
    }
    EmptyClipboard();
    let ok = SetClipboardData(format, mem) != 0;
    if !ok {
        GlobalFree(mem);
    }
    CloseClipboard();
    ok
}

/// Put `paths` on the clipboard as files (CF_HDROP, copy), so Paste in Explorer copies them.
/// The block is made before the clipboard is touched, so a failure leaves what was on it.
pub fn copy_files(owner: HWND, paths: &[std::path::PathBuf]) -> bool {
    let bytes = hdrop_bytes(paths);
    unsafe {
        let mem = global_with(&bytes);
        // CF_HDROP
        clipboard_take(owner, 15, mem)
    }
}

/// Rename `from` to `to` (in the same folder) without ever replacing something already at `to`:
/// MoveFileW fails when the name is taken, where `std::fs::rename` would overwrite a file.
pub fn move_no_replace(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let w = |p: &std::path::Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(std::iter::once(0)).collect() };
    let (a, b) = (w(from), w(to));
    if unsafe { MoveFileW(a.as_ptr(), b.as_ptr()) } != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
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

/// The Explorer windows [`reveal_all`] opens for `paths`: one for each folder picked, and one for
/// each folder holding picked files (a folder picked and holding picked files is one window).
/// Counting stops once it is past `limit` (the number returned is then `limit + 1`, "more than"),
/// so a huge pick is not read from the disk path by path.
pub fn reveal_windows(paths: &[PathBuf], limit: usize) -> usize {
    windows_for(paths, |p| p.is_dir(), limit)
}

/// [`reveal_windows`] with the folder-or-not answer given by `is_dir`.
fn windows_for(paths: &[PathBuf], is_dir: impl Fn(&Path) -> bool, limit: usize) -> usize {
    use std::collections::BTreeSet;
    let mut windows: BTreeSet<PathBuf> = BTreeSet::new();
    for p in paths {
        if is_dir(p) {
            windows.insert(p.clone());
        } else if let Some(parent) = p.parent() {
            windows.insert(parent.to_path_buf());
        }
        if windows.len() > limit {
            break;
        }
    }
    windows.len()
}

/// Show every path in `paths` in Explorer: the files of one folder are selected together, in one
/// window, and each folder picked is opened. At most `max_windows` windows; false, opening
/// nothing, when that would take more. True when Explorer was started for at least one.
pub fn reveal_all(paths: &[PathBuf], max_windows: usize) -> bool {
    use std::collections::BTreeMap;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};
    // each path is asked of the disk once: for the count and for the grouping below
    let dirs: std::collections::HashMap<&Path, bool> = paths.iter().map(|p| (p.as_path(), p.is_dir())).collect();
    let is_dir = |p: &Path| dirs.get(p).copied().unwrap_or(false);
    if windows_for(paths, is_dir, max_windows) > max_windows {
        return false;
    }
    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(std::iter::once(0)).collect() };
    let mut by_parent: BTreeMap<PathBuf, Vec<&Path>> = BTreeMap::new();
    let mut folders = Vec::new();
    for p in paths {
        match p.parent() {
            Some(parent) if !is_dir(p) => by_parent.entry(parent.to_path_buf()).or_default().push(p),
            _ => folders.push(p),
        }
    }
    let mut ok = false;
    for (parent, items) in &by_parent {
        let w = wide(parent);
        let holds: Vec<_> = items
            .iter()
            .filter_map(|p| {
                let wi = wide(p);
                let pidl = unsafe { ILCreateFromPathW(windows::core::PCWSTR(wi.as_ptr())) };
                (!pidl.is_null()).then_some(pidl as *const _)
            })
            .collect();
        let mut shown = false;
        unsafe {
            let folder = ILCreateFromPathW(windows::core::PCWSTR(w.as_ptr()));
            if !folder.is_null() && !holds.is_empty() {
                shown = SHOpenFolderAndSelectItems(folder, Some(&holds), 0).is_ok();
            }
            if !folder.is_null() {
                ILFree(Some(folder));
            }
            for p in &holds {
                ILFree(Some(*p));
            }
        }
        // a pidl Explorer would not take: the first item, the way one item is shown
        ok |= shown || reveal(items[0]);
    }
    for f in folders {
        // a picked folder whose files were shown together above is already open
        if by_parent.contains_key(f) {
            continue;
        }
        ok |= explorer().arg(f).spawn().is_ok();
    }
    ok
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

/// The window's outer size in pixels.
pub fn window_size(h: HWND) -> Option<(i32, i32)> {
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    (unsafe { GetWindowRect(h, &mut r) } != 0).then_some((r.right - r.left, r.bottom - r.top))
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

/// dev (screenshot runs): EnumChildWindows callback gathering every child window.
unsafe extern "system" fn collect_children(h: HWND, l: isize) -> i32 {
    (*(l as *mut Vec<HWND>)).push(h);
    1
}

/// dev (screenshot runs): the window class of `h`.
fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// dev (screenshot runs): the window WinUI reads the keyboard from: the content's input site
/// (a child of the window), or the window itself when there is none.
fn key_target(h: HWND) -> HWND {
    let mut kids: Vec<HWND> = Vec::new();
    unsafe { EnumChildWindows(h, collect_children, &mut kids as *mut Vec<HWND> as isize) };
    let classes: Vec<String> = kids.iter().map(|&k| class_of(k)).collect();
    crate::state::timing(&format!("child windows: {classes:?}"), std::time::Instant::now());
    kids.iter().zip(&classes).find(|(_, c)| c.contains("InputSite")).or_else(|| kids.iter().zip(&classes).last()).map_or(h, |(&k, _)| k)
}

/// dev (screenshot runs): key presses posted to this window's own input, in this process only
/// (nothing reaches the rest of the desktop, and the window is not brought forward). Each
/// press is `alt`, `right`, `f`, ... or `alt+f` (Alt held round the key); the keyboard state of
/// this thread says Alt is down for an Alt press, until [`keys_release`].
pub fn post_keys(h: HWND, presses: &[&str]) {
    const VK_MENU: u16 = 0x12;
    let target = key_target(h);
    let post = |msg: u32, vk: u16, up: bool, alt: bool| {
        let scan = unsafe { MapVirtualKeyW(u32::from(vk), 0) } as isize;
        let mut l: isize = 1 | (scan << 16);
        if alt {
            l |= 1 << 29;
        }
        if up {
            l |= (1 << 30) | (1 << 31);
        }
        unsafe { PostMessageW(target, msg, usize::from(vk), l) };
    };
    for press in presses {
        let (alt, key) = match press.strip_prefix("alt+") {
            Some(k) => (true, k),
            None => (false, *press),
        };
        let Some(vk) = vk_of(key) else { continue };
        if vk == VK_MENU {
            alt_state(true);
            post(0x104, VK_MENU, false, true); // WM_SYSKEYDOWN
            post(0x105, VK_MENU, true, false); // WM_SYSKEYUP
        } else if alt {
            alt_state(true);
            post(0x104, VK_MENU, false, true);
            post(0x104, vk, false, true);
            post(0x105, vk, true, true);
            post(0x105, VK_MENU, true, false);
        } else {
            post(0x100, vk, false, false); // WM_KEYDOWN
            post(0x101, vk, true, false); // WM_KEYUP
        }
    }
}

/// dev (screenshot runs): this thread's keyboard state with Alt down (or up again).
pub fn alt_state(down: bool) {
    let mut s = [0u8; 256];
    unsafe {
        GetKeyboardState(s.as_mut_ptr());
        for k in [0x12usize, 0xA4] {
            s[k] = if down { 0x80 } else { 0 };
        }
        SetKeyboardState(s.as_ptr());
    }
}

/// dev: a key's virtual key code by a short name.
fn vk_of(name: &str) -> Option<u16> {
    Some(match name {
        "alt" => 0x12,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "enter" => 0x0D,
        "esc" => 0x1B,
        "space" => 0x20,
        "tab" => 0x09,
        "home" => 0x24,
        // the menu key (a context menu for the keyboard)
        "apps" => 0x5D,
        "end" => 0x23,
        f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u16>().is_ok_and(|n| (1..=12).contains(&n)) => 0x6F + f[1..].parse::<u16>().unwrap_or(1),
        k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => u16::from(k.as_bytes()[0].to_ascii_uppercase()),
        _ => return None,
    })
}

/// dev (screenshot runs): open the dropdown whose AutomationId (its x:Name) is `id`, through
/// UI Automation, as a screen reader would; the popup is part of the picture then.
pub fn expand_dropdown(h: HWND, id: &str) -> bool {
    expand_by_here(h, id, true)
}

/// [`expand_dropdown`] for anything that expands: by AutomationId (`by_id`, a dropdown's or a
/// button's x:Name) or by its name as a screen reader reads it (the File menu's "File").
pub fn expand_by(h: HWND, key: &str, by_id: bool) -> bool {
    // a menu keeps the call until it closes, and the window's own thread is the one that
    // would have to answer it: from another thread, the window carries on
    let key = key.to_string();
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        expand_by_here(h, &key, by_id);
    });
    true
}

fn expand_by_here(h: HWND, key: &str, by_id: bool) -> bool {
    use windows::core::BSTR;
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomationExpandCollapsePattern, TreeScope_Descendants, UIA_AutomationIdPropertyId, UIA_ExpandCollapsePatternId, UIA_NamePropertyId};
    let id = key;
    let run = || -> windows::core::Result<()> {
        unsafe {
            let ua: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            let root = ua.ElementFromHandle(windows::Win32::Foundation::HWND(h as *mut _))?;
            let cond = ua.CreatePropertyCondition(if by_id { UIA_AutomationIdPropertyId } else { UIA_NamePropertyId }, &VARIANT::from(BSTR::from(id)))?;
            let found = root.FindFirst(TreeScope_Descendants, &cond)?;
            let pattern: IUIAutomationExpandCollapsePattern = windows::core::Interface::cast(&found.GetCurrentPattern(UIA_ExpandCollapsePatternId)?)?;
            pattern.Expand()
        }
    };
    run().is_ok()
}

/// dev (screenshot runs): a screen reader's Toggle (a check) or Select (a dot) on the element
/// whose AutomationId is `id`, from another thread as [`expand_by`] does.
pub fn toggle_or_select_by(h: HWND, id: &str) {
    let id = id.to_string();
    std::thread::spawn(move || {
        use windows::core::BSTR;
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
        use windows::Win32::System::Variant::VARIANT;
        use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern, TreeScope_Descendants, UIA_AutomationIdPropertyId, UIA_SelectionItemPatternId, UIA_TogglePatternId};
        let run = || -> windows::core::Result<&'static str> {
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
                let ua: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
                let root = ua.ElementFromHandle(windows::Win32::Foundation::HWND(h as *mut _))?;
                let cond = ua.CreatePropertyCondition(UIA_AutomationIdPropertyId, &VARIANT::from(BSTR::from(id.as_str())))?;
                let found = root.FindFirst(TreeScope_Descendants, &cond)?;
                if let Ok(p) = found.GetCurrentPattern(UIA_SelectionItemPatternId).and_then(|p| windows::core::Interface::cast::<IUIAutomationSelectionItemPattern>(&p)) {
                    p.Select()?;
                    return Ok("select");
                }
                let p: IUIAutomationTogglePattern = windows::core::Interface::cast(&found.GetCurrentPattern(UIA_TogglePatternId)?)?;
                p.Toggle()?;
                Ok("toggle")
            }
        };
        crate::state::timing(&format!("uia: {id}: {:?}", run()), std::time::Instant::now());
    });
}

/// dev (screenshot runs): the menu bar as UI Automation (a screen reader) sees it, in the timing
/// log: each menu bar and menu item, its name, AutomationId, access key, parent, and whether it
/// expands and collapses (and is expanded now).
pub fn uia_report(h: HWND) {
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        if let Err(e) = uia_report_here(h) {
            crate::state::timing(&format!("uia: {e:?}"), std::time::Instant::now());
        }
    });
}

fn uia_report_here(h: HWND) -> windows::core::Result<()> {
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationExpandCollapsePattern, UIA_ExpandCollapsePatternId, UIA_InvokePatternId};
    unsafe {
        let ua: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let root = ua.ElementFromHandle(windows::Win32::Foundation::HWND(h as *mut _))?;
        let walker = ua.ControlViewWalker()?;
        fn walk(ua: &IUIAutomation, walker: &windows::Win32::UI::Accessibility::IUIAutomationTreeWalker, e: &IUIAutomationElement, depth: u32, parent: &str) -> windows::core::Result<()> {
            if depth > 16 {
                return Ok(());
            }
            let mut c = unsafe { walker.GetFirstChildElement(e) }.ok();
            while let Some(el) = c {
                let (ct, name, id, ak) = unsafe { (el.CurrentControlType()?.0, el.CurrentName()?.to_string(), el.CurrentAutomationId()?.to_string(), el.CurrentAccessKey()?.to_string()) };
                // 50010 menu bar, 50011 menu item, 50009 menu
                if matches!(ct, 50009 | 50010 | 50011) || id.starts_with("Menu") {
                    let expand: Option<IUIAutomationExpandCollapsePattern> = unsafe { el.GetCurrentPattern(UIA_ExpandCollapsePatternId) }.ok().and_then(|p| windows::core::Interface::cast(&p).ok());
                    let state = expand.as_ref().and_then(|p| unsafe { p.CurrentExpandCollapseState() }.ok()).map(|s| s.0);
                    let invoke = unsafe { el.GetCurrentPattern(UIA_InvokePatternId) }.is_ok();
                    crate::state::timing(&format!("uia: {}type {ct} name {name:?} id {id:?} accesskey {ak:?} parent {parent:?} expandcollapse {state:?} invoke {invoke}", "  ".repeat(depth as usize)), std::time::Instant::now());
                }
                let label = format!("{ct}:{name}");
                walk(ua, walker, &el, depth + 1, &label)?;
                c = unsafe { walker.GetNextSiblingElement(&el) }.ok();
            }
            Ok(())
        }
        walk(&ua, &walker, &root, 0, "window")
    }
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
    use super::{reveal_windows, select_arg, windows_for};
    use std::path::Path;

    fn unquoted(arg: &str) -> &str {
        arg.strip_prefix("/select,\"").and_then(|a| a.strip_suffix('"')).expect("the switch, then one quoted path")
    }

    #[test]
    fn the_windows_reveal_all_opens_are_counted() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir().join(format!("sprawl-reveal-{}-{nanos}", std::process::id()));
        let (a, b) = (base.join("a"), base.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let (a1, a2, b1) = (a.join("one.txt"), a.join("two.txt"), b.join("three.txt"));
        for f in [&a1, &a2, &b1] {
            std::fs::write(f, b"x").unwrap();
        }
        let p = |f: &Path| f.to_path_buf();
        // one folder's files: one window; two folders: two
        assert_eq!(reveal_windows(&[p(&a1), p(&a2)], 10), 1);
        assert_eq!(reveal_windows(&[p(&a1), p(&b1)], 10), 2);
        // a picked folder is a window of its own; with its picked files shown in it, one
        assert_eq!(reveal_windows(&[p(&a), p(&b1)], 10), 2);
        assert_eq!(reveal_windows(&[p(&a), p(&a1)], 10), 1);
        assert_eq!(reveal_windows(&[], 10), 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A huge pick is not asked of the disk path by path: counting stops once past the limit.
    #[test]
    fn counting_windows_stops_past_the_limit() {
        let paths: Vec<std::path::PathBuf> = (0..1000).map(|i| std::path::PathBuf::from(format!(r"Z:/d{i}/file.txt"))).collect();
        let asked = std::cell::Cell::new(0usize);
        let n = windows_for(&paths, |_| { asked.set(asked.get() + 1); false }, 10);
        assert_eq!(n, 11, "more than the limit, no more exact than that");
        assert_eq!(asked.get(), 11, "the rest were never read");
        assert_eq!(windows_for(&paths[..3], |_| false, 10), 3);
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
