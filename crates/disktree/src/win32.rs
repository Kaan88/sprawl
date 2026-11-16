//! The few plain Win32 calls the app needs (drives, disk space, dialogs, clipboard,
//! window capture), declared by hand.
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::path::Path;

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

#[repr(C)]
struct SHFILEOPSTRUCTW {
    hwnd: HWND,
    wFunc: u32,
    pFrom: *const u16,
    pTo: *const u16,
    fFlags: u16,
    fAnyOperationsAborted: i32,
    hNameMappings: *mut c_void,
    lpszProgressTitle: *const u16,
}

#[link(name = "user32")]
extern "system" {
    fn SetWindowPos(h: HWND, after: HWND, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    fn ShowWindow(h: HWND, cmd: i32) -> i32;
    fn PrintWindow(h: HWND, hdc: isize, flags: u32) -> i32;
    fn GetWindowRect(h: HWND, r: *mut RECT) -> i32;
    fn GetDC(h: HWND) -> isize;
    fn ReleaseDC(h: HWND, dc: isize) -> i32;
    fn MessageBoxW(h: HWND, text: *const u16, caption: *const u16, kind: u32) -> i32;
    fn OpenClipboard(h: HWND) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, mem: isize) -> isize;
    fn CloseClipboard() -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(dc: isize) -> isize;
    fn CreateCompatibleBitmap(dc: isize, w: i32, h: i32) -> isize;
    fn SelectObject(dc: isize, obj: isize) -> isize;
    fn GetDIBits(dc: isize, bmp: isize, start: u32, lines: u32, bits: *mut c_void, info: *mut BITMAPINFOHEADER, usage: u32) -> i32;
    fn DeleteObject(obj: isize) -> i32;
    fn DeleteDC(dc: isize) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLogicalDrives() -> u32;
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetVolumeInformationW(root: *const u16, name: *mut u16, name_len: u32, serial: *mut u32, max: *mut u32, flags: *mut u32, fs: *mut u16, fs_len: u32) -> i32;
    fn GetDiskFreeSpaceExW(root: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    fn GlobalAlloc(flags: u32, bytes: usize) -> isize;
    fn GlobalLock(mem: isize) -> *mut c_void;
    fn GlobalUnlock(mem: isize) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn SHFileOperationW(op: *mut SHFILEOPSTRUCTW) -> i32;
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
        out.push(Drive { root, label });
    }
    out
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

pub fn confirm(owner: HWND, text: &str, caption: &str) -> bool {
    let (t, c) = (wide(text), wide(caption));
    unsafe { MessageBoxW(owner, t.as_ptr(), c.as_ptr(), 0x4 | 0x30) == 6 }
}

pub fn info(owner: HWND, text: &str, caption: &str) {
    let (t, c) = (wide(text), wide(caption));
    unsafe {
        MessageBoxW(owner, t.as_ptr(), c.as_ptr(), 0x40);
    }
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

/// Send a file or folder to the Recycle Bin (`permanent` deletes it outright).
pub fn delete_path(owner: HWND, path: &Path, permanent: bool) -> bool {
    let mut from: Vec<u16> = path.display().to_string().encode_utf16().collect();
    from.extend([0, 0]);
    let mut op = SHFILEOPSTRUCTW {
        hwnd: owner,
        wFunc: 3,
        pFrom: from.as_ptr(),
        pTo: std::ptr::null(),
        fFlags: if permanent { 0x10 } else { 0x40 | 0x10 },
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };
    unsafe { SHFileOperationW(&mut op) == 0 && op.fAnyOperationsAborted == 0 }
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
