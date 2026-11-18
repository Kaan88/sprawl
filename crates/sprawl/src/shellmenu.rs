//! "Windows menu...": Explorer's own right-click menu for one row, shown by Sprawl.
//!
//! The menu's extensions (7-Zip, cloud drives, antivirus...) are other programs' code, loaded
//! into Sprawl: it is never offered when Sprawl runs as administrator (they would run elevated
//! too), and a badly written one can crash Sprawl. What is done through it (delete, rename,
//! move...) bypasses Sprawl's checks; the row's folder is read again afterwards.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use windows::core::{Interface, HSTRING, PCSTR, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{DefSubclassProc, IContextMenu, IContextMenu2, IContextMenu3, IShellFolder, RemoveWindowSubclass, SHBindToParent, SHParseDisplayName, SetWindowSubclass, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX};
use windows::Win32::UI::WindowsAndMessaging::{CreatePopupMenu, DestroyMenu, GetCursorPos, TrackPopupMenuEx, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON};

/// Why the item is not offered now, or None when it is.
/// (As administrator it also needs Settings > Scanning: crate::admin::SHELL_MENU.)
pub(crate) fn unavailable(picked: usize) -> Option<&'static str> {
    (picked > 1).then_some("one row at a time")
}

/// How the menu went.
#[derive(Debug, PartialEq)]
pub(crate) enum Shown {
    /// Closed without a choice.
    Closed,
    /// A command was run (its verb, when the menu says it).
    Ran(String),
}

thread_local! {
    /// The open menu's handler, for the window messages its submenus need (Send to, Open with).
    static OPEN: RefCell<Option<(Option<IContextMenu3>, Option<IContextMenu2>)>> = const { RefCell::new(None) };
}

unsafe extern "system" fn menu_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM, _id: usize, _data: usize) -> LRESULT {
    // WM_INITMENUPOPUP, WM_DRAWITEM, WM_MEASUREITEM, WM_MENUCHAR
    if matches!(m, 0x0117 | 0x002B | 0x002C | 0x0120) {
        let handled = OPEN.with(|o| {
            let o = o.borrow();
            let (c3, c2) = o.as_ref()?;
            if let Some(c3) = c3 {
                let mut r = LRESULT(0);
                return unsafe { c3.HandleMenuMsg2(m, w, l, Some(&mut r)) }.ok().map(|_| r);
            }
            unsafe { c2.as_ref()?.HandleMenuMsg(m, w, l) }.ok().map(|_| LRESULT(0))
        });
        if let Some(r) = handled {
            return r;
        }
    }
    unsafe { DefSubclassProc(h, m, w, l) }
}

/// Show Explorer's menu for `path` at the mouse pointer, and run what is chosen. `allowed`: the
/// user allowed it as administrator (crate::admin); an elevated Sprawl refuses without it.
pub(crate) fn show(owner: isize, path: &Path, allowed: bool) -> Result<Shown, String> {
    if sprawl_core::is_elevated() && !allowed {
        return Err(crate::admin::refused(&crate::admin::SHELL_MENU));
    }
    if std::fs::symlink_metadata(path).is_err() {
        return Err(format!("{} is gone", path.display()));
    }
    let hwnd = HWND(owner as *mut _);
    unsafe {
        // the UI thread is already a single-threaded apartment; this only makes sure
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        SHParseDisplayName(&HSTRING::from(path.as_os_str()), None, &mut pidl, 0, None).map_err(|e| format!("Windows couldn't find it ({e})"))?;
        let r = (|| -> Result<Shown, String> {
            let mut last: *mut ITEMIDLIST = std::ptr::null_mut();
            let folder: IShellFolder = SHBindToParent(pidl, Some(&mut last)).map_err(|e| e.to_string())?;
            let cm: IContextMenu = folder.GetUIObjectOf(hwnd, &[last as *const ITEMIDLIST], None).map_err(|e| e.to_string())?;
            let menu = CreatePopupMenu().map_err(|e| e.to_string())?;
            const FIRST: u32 = 1;
            // CMF_NORMAL; Shift held adds the extended verbs, as in Explorer
            let flags = if crate::win32::ctrl_shift_down().1 { 0x100 } else { 0 };
            let q = cm.QueryContextMenu(menu, 0, FIRST, 0x7FFF, flags);
            if q.is_err() {
                let _ = DestroyMenu(menu);
                return Err(q.to_string());
            }
            OPEN.with(|o| *o.borrow_mut() = Some((cm.cast::<IContextMenu3>().ok(), cm.cast::<IContextMenu2>().ok())));
            let _ = SetWindowSubclass(hwnd, Some(menu_proc), 0x5345_4C4C, 0);
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let id = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as u32;
            let _ = RemoveWindowSubclass(hwnd, Some(menu_proc), 0x5345_4C4C);
            // the ids the menu's handlers took: QueryContextMenu's code holds how many
            let taken = (q.0 as u32) & 0xFFFF;
            let result = if id >= FIRST && id - FIRST < taken {
                let off = (id - FIRST) as usize;
                let verb = verb_of(&cm, off).unwrap_or_default();
                let info = CMINVOKECOMMANDINFOEX {
                    cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                    // CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE
                    fMask: 0x4000 | 0x2000_0000,
                    hwnd,
                    lpVerb: PCSTR(off as *const u8),
                    lpVerbW: PCWSTR(off as *const u16),
                    nShow: SW_SHOWNORMAL.0,
                    ptInvoke: pt,
                    ..Default::default()
                };
                cm.InvokeCommand(&info as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO).map(|_| Shown::Ran(verb)).map_err(|e| e.to_string())
            } else {
                Ok(Shown::Closed)
            };
            OPEN.with(|o| *o.borrow_mut() = None);
            let _ = DestroyMenu(menu);
            result
        })();
        CoTaskMemFree(Some(pidl as *const _));
        r
    }
}

/// The verb of menu item `off` ("delete", "rename"...), when it has one.
unsafe fn verb_of(cm: &IContextMenu, off: usize) -> Option<String> {
    // MAX_PATH and more: some handlers write past the size they are given
    let mut buf = [0u16; 1024];
    // GCS_VERBW
    unsafe { cm.GetCommandString(off, 0x4, None, windows::core::PSTR(buf.as_mut_ptr().cast()), 260) }.ok()?;
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    (n > 0).then(|| String::from_utf16_lossy(&buf[..n]))
}

/// What a rescan after the menu reads again: the row's folder (a delete, rename or move changes
/// it, and "Extract here" adds beside the row). When that folder is the scan's own root, only the
/// row itself, not the whole scan again.
pub(crate) fn rescan_target(row: &Path, scan_root: &Path) -> PathBuf {
    match row.parent() {
        Some(p) if !same(p, scan_root) && p.starts_with(scan_root) => p.to_path_buf(),
        _ => row.to_path_buf(),
    }
}

fn same(a: &Path, b: &Path) -> bool {
    let k = |p: &Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
    k(a) == k(b)
}

/// What is at `p` now, for `apply_rescans`: Some(None) when it is gone, None when it can't be read.
fn read_now(p: &Path) -> Option<Option<sprawl_core::Tree>> {
    match std::fs::symlink_metadata(p) {
        Ok(_) => crate::cleanup::rescan(p).ok().map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}

impl crate::state::State {
    /// "Windows menu..." for `path`. `in_scan`: a row of the scan on screen, read again afterwards.
    pub(crate) fn shell_menu(&mut self, path: PathBuf, in_scan: bool) {
        // a delete from that menu in the middle of a clean-up would change the tree it is checking
        if self.advisor.cleanup.running.is_some() {
            crate::state::text(&self.root, "FootLeft", "A clean-up is running: the Windows menu is back when it has finished.");
            return;
        }
        let picked = if in_scan { self.picked_nodes().len() } else { 1 };
        if let Some(why) = unavailable(picked) {
            crate::state::text(&self.root, "FootLeft", &format!("Windows menu: {why}"));
            return;
        }
        if let Err(why) = self.admin_allows(&crate::admin::SHELL_MENU) {
            crate::state::text(&self.root, "FootLeft", &format!("Windows menu: {why}"));
            return;
        }
        match show(self.hwnd, &path, true) {
            Ok(Shown::Closed) => {}
            Ok(Shown::Ran(verb)) => {
                let Some(root) = self.tree.as_ref().map(|t| t.path_of(sprawl_core::ROOT)).filter(|_| in_scan) else { return };
                let target = rescan_target(&path, &root);
                let what = if verb.is_empty() { "That".to_string() } else { format!("\u{201C}{verb}\u{201D}") };
                crate::state::text(&self.root, "FootLeft", &format!("{what} ran outside Sprawl's checks; reading {} again...", target.display()));
                // the menu's command may still be working (a copy, a delete with its own progress):
                // read now and once more a little later
                let (tx, rx) = std::sync::mpsc::channel();
                self.shell_rescans = Some(rx);
                std::thread::spawn(move || {
                    for wait in [1500, 8000] {
                        std::thread::sleep(std::time::Duration::from_millis(wait));
                        if let Some(now) = read_now(&target) {
                            if tx.send((target.clone(), now)).is_err() {
                                return;
                            }
                        }
                    }
                });
            }
            Err(e) => crate::state::text(&self.root, "FootLeft", &format!("Windows menu: {e}")),
        }
    }

    /// A rescan after the Windows menu, when one has come in.
    pub(crate) fn drain_shell_rescans(&mut self) {
        let Some(rx) = &self.shell_rescans else { return };
        let got: Vec<_> = rx.try_iter().collect();
        if !got.is_empty() {
            self.apply_rescans(got);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    #[test]
    fn the_rows_folder_is_read_again_but_not_the_whole_scan() {
        let t = super::rescan_target;
        assert_eq!(t(Path::new(r"C:\Data\Photos\a.jpg"), Path::new(r"C:\Data")), PathBuf::from(r"C:\Data\Photos"));
        assert_eq!(t(Path::new(r"C:\Data\a.jpg"), Path::new(r"C:\Data")), PathBuf::from(r"C:\Data\a.jpg"));
        assert_eq!(t(Path::new(r"C:\Data\a.jpg"), Path::new(r"c:\data\")), PathBuf::from(r"C:\Data\a.jpg"));
        assert_eq!(t(Path::new(r"C:\Windows"), Path::new(r"C:\")), PathBuf::from(r"C:\Windows"));
    }

    #[test]
    fn one_row_at_a_time() {
        assert!(super::unavailable(2).is_some_and(|w| w.contains("one row")));
        assert_eq!(super::unavailable(1), None);
        assert_eq!(super::unavailable(0), None);
        // as administrator without the setting, show() itself refuses before touching the shell
        if sprawl_core::is_elevated() {
            assert!(super::show(0, std::path::Path::new(r"C:\Windows"), false).is_err_and(|e| e.contains("Settings > Scanning")));
        }
    }
}
