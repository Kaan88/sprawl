//! Pinned local directory creation, cross-process metadata locks and owned-file publication.
use std::{fs, io};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};

fn bad() -> io::Error { io::Error::other("Local storage path or metadata changed") }

pub(crate) fn ensure_dir(path: &Path) -> io::Result<sprawl_core::local::Directory> {
    let mut path = sprawl_core::local::lexical(path)?;
    let mut missing = Vec::new();
    let mut held = loop {
        match sprawl_core::local::directory(&path) {
            Ok(held) => break held,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                missing.push(path.file_name().ok_or_else(bad)?.to_os_string());
                path = path.parent().ok_or_else(bad)?.to_path_buf();
            }
            Err(e) => return Err(e),
        }
    };
    for name in missing.into_iter().rev() {
        let next = held.path.join(name);
        match fs::create_dir(&next) { Ok(()) => {}, Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}, Err(e) => return Err(e) }
        held = sprawl_core::local::directory(&next)?;
    }
    Ok(held)
}


pub(crate) fn nonce() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!("{:x}-{:x}-{:x}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed), SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos())
}


#[link(name = "kernel32")]
extern "system" {
    fn SetFileInformationByHandle(file: *mut core::ffi::c_void, class: i32, info: *const core::ffi::c_void, size: u32) -> i32;
}


fn rename(file: &fs::File, to: &Path, replace: bool) -> io::Result<()> {
    let name: Vec<u16> = std::ffi::OsStr::new(&format!(r"\??\{}", to.display())).encode_wide().collect();
    let head = 8 + std::mem::size_of::<usize>() + 4;
    let size = head + name.len() * 2 + 2;
    let mut buffer = vec![0u64; size.div_ceil(8)];
    let bytes = unsafe { std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), buffer.len() * 8) };
    bytes[0] = u8::from(replace);
    bytes[8 + std::mem::size_of::<usize>()..head].copy_from_slice(&((name.len() * 2) as u32).to_le_bytes());
    for (i, c) in name.iter().enumerate() { bytes[head + i * 2..head + i * 2 + 2].copy_from_slice(&c.to_le_bytes()); }
    if unsafe { SetFileInformationByHandle(file.as_raw_handle().cast(), 3, buffer.as_ptr().cast(), size as u32) } != 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

pub(crate) fn delete(file: &fs::File) -> io::Result<()> {
    let flags = 3u32;
    if unsafe { SetFileInformationByHandle(file.as_raw_handle().cast(), 21, (&flags as *const u32).cast(), 4) } != 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

pub(crate) fn write_atomic(dir: &Path, name: &str, replace: bool, write: impl FnOnce(&mut fs::File) -> io::Result<()>) -> io::Result<()> {
    let tmp = dir.join(format!("{}.tmp", nonce()));
    let mut file = fs::OpenOptions::new().read(true).write(true).access_mode(0x8000_0000 | 0x4000_0000 | 0x0001_0000)
        .share_mode(1 | 4).create_new(true).custom_flags(0x0020_0000 | 0x0010_0000).open(&tmp)?;
    let result = write(&mut file).and_then(|_| file.sync_all()).and_then(|_| {
        for _ in 0..20 {
            match rename(&file, &dir.join(name), replace) {
                Ok(()) => return Ok(()),
                Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => std::thread::sleep(Duration::from_millis(5)),
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::WouldBlock, "Local metadata is being read"))
    });
    if result.is_err() { let _ = delete(&file); }
    result
}

pub(crate) fn lock(dir: &Path) -> io::Result<fs::File> {
    use std::os::windows::fs::MetadataExt;
    // up to about 10 s: History saves run off the window's thread, and several at once (each
    // flushing to disk) took longer than 2 s on a busy PC, losing a snapshot
    let start = std::time::Instant::now();
    for attempt in 0u64.. {
        if start.elapsed() > Duration::from_secs(10) { break; }
        match fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).share_mode(0)
            .custom_flags(0x0020_0000 | 0x0010_0000).open(dir.join(".save.lock")) {
            Ok(file) => {
                let m = file.metadata()?;
                if !m.is_file() || m.len() != 0 || m.file_attributes() & (0x400 | 0x1000 | 0x0004_0000 | 0x0040_0000) != 0 { return Err(bad()); }
                return Ok(file);
            }
            Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => std::thread::sleep(Duration::from_millis(10 + 10 * attempt.min(9))),
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::WouldBlock, "Local metadata is busy"))
}

