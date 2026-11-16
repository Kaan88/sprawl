//! Watches the scanned folder for changes (ReadDirectoryChangesW on a background thread) so
//! the map can pulse where files are being written, created, renamed or deleted.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::CancelIoEx;

pub struct Watcher {
    handle: isize,
    /// Paths relative to the watched folder.
    pub rx: Receiver<PathBuf>,
}

impl Watcher {
    pub fn start(root: &Path) -> Option<Watcher> {
        let h = unsafe {
            CreateFileW(
                &HSTRING::from(root.as_os_str()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        }
        .ok()?;
        let raw = h.0 as isize;
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("watch".into())
            .spawn(move || {
                let h = HANDLE(raw as *mut _);
                // DWORD-aligned buffer for FILE_NOTIFY_INFORMATION records
                let mut buf = vec![0u32; 16 * 1024];
                loop {
                    let mut got = 0u32;
                    let ok = unsafe {
                        ReadDirectoryChangesW(
                            h,
                            buf.as_mut_ptr() as *mut _,
                            (buf.len() * 4) as u32,
                            true,
                            FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE,
                            Some(&mut got),
                            None,
                            None,
                        )
                    };
                    if ok.is_err() {
                        break; // cancelled, or the folder went away
                    }
                    if got == 0 {
                        continue; // overflow: too many changes at once; skip this batch
                    }
                    let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, got as usize) };
                    let mut off = 0usize;
                    loop {
                        if off + 12 > bytes.len() {
                            break;
                        }
                        let rd = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
                        let next = rd(off) as usize;
                        let len = rd(off + 8) as usize;
                        let start = off + 12;
                        if start + len > bytes.len() {
                            break;
                        }
                        let name: Vec<u16> = bytes[start..start + len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                        if tx.send(PathBuf::from(String::from_utf16_lossy(&name))).is_err() {
                            unsafe {
                                let _ = CloseHandle(h);
                            }
                            return;
                        }
                        if next == 0 {
                            break;
                        }
                        off += next;
                    }
                }
                unsafe {
                    let _ = CloseHandle(h);
                }
            })
            .ok()?;
        Some(Watcher { handle: raw, rx })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // Wakes the thread out of ReadDirectoryChangesW; it closes the handle and ends.
        unsafe {
            let _ = CancelIoEx(HANDLE(self.handle as *mut _), None);
        }
    }
}

/// The deepest node of `t` on the way to `rel` (relative to the root).
pub fn find(t: &sprawl_core::Tree, rel: &Path) -> usize {
    let mut n = sprawl_core::ROOT;
    for part in rel.components() {
        let name = part.as_os_str().to_string_lossy();
        match t.nodes[n].children.iter().copied().find(|&c| t.nodes[c].name.eq_ignore_ascii_case(&name)) {
            Some(c) => n = c,
            None => break,
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sees_a_new_file_and_finds_its_folder() {
        let dir = std::env::temp_dir().join(format!("dt-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let w = Watcher::start(&dir).expect("watch");
        std::thread::sleep(Duration::from_millis(100));
        std::fs::write(dir.join("sub").join("new.bin"), b"hello").unwrap();
        let got = w.rx.recv_timeout(Duration::from_secs(5)).expect("a change");
        assert!(got.starts_with("sub"), "{got:?}");

        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let n = find(&tree, Path::new(r"sub\new.bin"));
        assert_eq!(tree.nodes[n].name, "new.bin");
        let n = find(&tree, Path::new(r"sub\gone.bin"));
        assert_eq!(tree.nodes[n].name, "sub");
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
