//! Which programs have files open: the Restart Manager, the same thing Windows Installer uses
//! to say "close these apps first". Read-only: a session is opened, asked, and closed; nothing
//! is shut down or restarted.

use std::path::{Path, PathBuf};

use sprawl_core::Tree;

/// Files under node `n`, at most `cap` of them (the Restart Manager takes files, not folders).
pub fn files_under(tree: &Tree, n: usize, cap: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![n];
    while let Some(k) = stack.pop() {
        if out.len() >= cap {
            break;
        }
        let node = &tree.nodes[k];
        if node.is_dir {
            if k == n || !node.is_reparse_point() {
                stack.extend_from_slice(&node.children);
            }
        } else {
            out.push(tree.path_of(k));
        }
    }
    out
}

/// Names of the programs that have any of `files` open (empty when none), or `None` when
/// Windows couldn't say.
pub fn in_use(files: &[PathBuf]) -> Option<Vec<String>> {
    #[cfg(windows)]
    {
        rm::list(files)
    }
    #[cfg(not(windows))]
    {
        let _ = files;
        None
    }
}

/// The answer for a folder.
#[derive(Clone, Debug)]
pub struct Checked {
    pub names: Vec<String>,
    /// Every file was asked about (there are at most [`MAX_FILES`]).
    pub all_files: bool,
}

pub const MAX_FILES: usize = 10_000;

/// [`in_use`] for everything under node `n` (the first [`MAX_FILES`] files).
pub fn in_use_under(tree: &Tree, n: usize) -> Option<Checked> {
    check_files(files_under(tree, n, MAX_FILES + 1))
}

/// [`in_use`] for files from [`files_under`] with a cap of `MAX_FILES + 1`, so the tree isn't
/// needed (and kept) while Windows is asked.
pub fn check_files(mut files: Vec<PathBuf>) -> Option<Checked> {
    let all_files = files.len() <= MAX_FILES;
    files.truncate(MAX_FILES);
    in_use(&files).map(|names| Checked { names, all_files })
}

#[cfg(windows)]
mod rm {
    use super::*;

    #[repr(C)]
    struct UniqueProcess {
        pid: u32,
        start_low: u32,
        start_high: u32,
    }

    #[repr(C)]
    struct ProcessInfo {
        process: UniqueProcess,
        app_name: [u16; 256],
        service_short_name: [u16; 64],
        app_type: u32,
        app_status: u32,
        ts_session_id: u32,
        restartable: i32,
    }

    #[link(name = "rstrtmgr")]
    extern "system" {
        fn RmStartSession(session: *mut u32, flags: u32, key: *mut u16) -> u32;
        fn RmEndSession(session: u32) -> u32;
        fn RmRegisterResources(session: u32, nfiles: u32, files: *const *const u16, napps: u32, apps: *const UniqueProcess, nservices: u32, services: *const *const u16) -> u32;
        fn RmGetList(session: u32, needed: *mut u32, count: *mut u32, info: *mut ProcessInfo, reasons: *mut u32) -> u32;
    }

    struct Session(u32);

    impl Drop for Session {
        fn drop(&mut self) {
            unsafe {
                RmEndSession(self.0);
            }
        }
    }

    pub(super) fn list(files: &[PathBuf]) -> Option<Vec<String>> {
        if files.is_empty() {
            return Some(Vec::new());
        }
        let mut handle = 0u32;
        let mut key = [0u16; 33];
        if unsafe { RmStartSession(&mut handle, 0, key.as_mut_ptr()) } != 0 {
            return None;
        }
        let session = Session(handle);
        let wide: Vec<Vec<u16>> = files.iter().map(|p| wide(p)).collect();
        for chunk in wide.chunks(1000) {
            let ptrs: Vec<*const u16> = chunk.iter().map(|w| w.as_ptr()).collect();
            if unsafe { RmRegisterResources(session.0, ptrs.len() as u32, ptrs.as_ptr(), 0, std::ptr::null(), 0, std::ptr::null()) } != 0 {
                return None;
            }
        }
        let mut infos: Vec<ProcessInfo> = Vec::new();
        let mut answered = false;
        for _ in 0..3 {
            let mut needed = 0u32;
            let mut count = infos.capacity() as u32;
            let mut reasons = 0u32;
            let r = unsafe { RmGetList(session.0, &mut needed, &mut count, infos.as_mut_ptr(), &mut reasons) };
            match r {
                0 => {
                    unsafe { infos.set_len(count.min(infos.capacity() as u32) as usize) };
                    answered = true;
                    break;
                }
                234 => infos = Vec::with_capacity(needed as usize + 4), // ERROR_MORE_DATA
                _ => return None,
            }
        }
        if !answered {
            return None;
        }
        let mut names: Vec<String> = infos
            .iter()
            .map(|i| {
                let n = i.app_name.iter().position(|&c| c == 0).unwrap_or(i.app_name.len());
                String::from_utf16_lossy(&i.app_name[..n])
            })
            .filter(|s| !s.is_empty())
            .collect();
        names.sort();
        names.dedup();
        Some(names)
    }

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_file_is_reported() {
        let p = std::env::temp_dir().join(format!("sprawl-inuse-{}.txt", std::process::id()));
        std::fs::write(&p, b"x").unwrap();
        assert!(in_use(&[p.clone()]).unwrap().is_empty(), "nobody has it open");
        let _keep = std::fs::File::open(&p).unwrap();
        // the test process itself holds it now
        assert!(!in_use(&[p.clone()]).unwrap().is_empty());
        drop(_keep);
        let _ = std::fs::remove_file(&p);
        let _ = Path::new("");
    }
}
