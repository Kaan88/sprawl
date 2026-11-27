//! Pinned local directory creation, cross-process metadata locks and owned-file publication.
use std::{fs, io};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};

fn bad() -> io::Error { io::Error::other("Local storage path or metadata changed") }

/// The folder `path`, made where missing, held (see [`sprawl_core::local::Directory`]): the
/// folders that are there are pinned from the drive down, and each missing one is made and
/// opened through its parent's handle as itself, never through a link.
pub(crate) fn ensure_dir(path: &Path) -> io::Result<sprawl_core::local::Directory> {
    let mut path = sprawl_core::local::lexical(path)?;
    let mut missing = Vec::new();
    let mut held = loop {
        match sprawl_core::local::directory(&path) {
            Ok(held) => break held,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                missing.push(path.file_name().and_then(|n| n.to_str()).ok_or_else(bad)?.to_string());
                path = path.parent().ok_or_else(bad)?.to_path_buf();
            }
            Err(e) => return Err(e),
        }
    };
    for name in missing.into_iter().rev() {
        held = held.child(&name, true)?;
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
    write_atomic_until(dir, name, replace, &|| Ok(()), write)
}

pub(crate) fn write_atomic_until(dir: &Path, name: &str, replace: bool, check: &dyn Fn() -> io::Result<()>, write: impl FnOnce(&mut fs::File) -> io::Result<()>) -> io::Result<()> {
    check()?;
    let tmp = dir.join(format!("{}.tmp", nonce()));
    let mut file = fs::OpenOptions::new().read(true).write(true).access_mode(0x8000_0000 | 0x4000_0000 | 0x0001_0000)
        .share_mode(1 | 4).create_new(true).custom_flags(0x0020_0000 | 0x0010_0000).open(&tmp)?;
    let result = write(&mut file).and_then(|_| check()).and_then(|_| file.sync_all()).and_then(|_| {
        for _ in 0..20 {
            check()?;
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

/// Open `name` in the held folder `dir` through its handle, as itself: a plain file with one
/// name only (a link at its name is refused, never followed). `create`: made when missing.
/// With delete access, so it can be renamed or removed through the handle, and shared for
/// reading only: while it is open no one else can write, rename or remove it.
fn open_own(dir: &sprawl_core::local::Directory, name: &str, create: bool) -> io::Result<fs::File> {
    use sprawl_core::local::{Disposition, Kind, APPEND_DATA, DELETE, SHARE_READ};
    let how = if create { Disposition::OpenOrCreate } else { Disposition::Open };
    // append only: what is written goes at the end, never over what is there
    dir.open_in(name, APPEND_DATA | DELETE, SHARE_READ, how, Kind::File, true)
}

/// Append to `dir\name` as [`open_own`] opens it, the folders on the way held. Past
/// `rotate_at` bytes the file is first renamed (through its handle, within the held folder) to
/// `rotated`, replacing what has that name, and a new one is begun. Appends from this process
/// take turns; one from another Sprawl (its MCP server) is waited for, briefly.
pub(crate) fn append_checked(dir: &Path, name: &str, rotate_at: u64, rotated: &str, write: impl FnOnce(&mut fs::File) -> io::Result<()>) -> io::Result<()> {
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let held = ensure_dir(dir)?;
    let open = || {
        let mut tries = 0;
        loop {
            match open_own(&held, name, true) {
                Err(e) if e.raw_os_error() == Some(32) && tries < 100 => { tries += 1; std::thread::sleep(Duration::from_millis(20)); }
                r => return r,
            }
        }
    };
    let mut file = open()?;
    if file.metadata()?.len() > rotate_at {
        held.rename_in(&file, rotated, true)?;
        drop(file);
        file = open()?;
    }
    write(&mut file)
}

/// Remove `dir\name` through its handle, as [`open_own`] opens it. Nothing there is not an
/// error; a link or a file with other names is left and is one.
pub(crate) fn remove_checked(dir: &Path, name: &str) -> io::Result<()> {
    let held = match sprawl_core::local::directory(dir) {
        Ok(held) => held,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    match open_own(&held, name, false) {
        Ok(file) => sprawl_core::local::dispose(&file),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
#[test]
fn the_log_is_appended_and_removed_only_as_itself() {
    // Astra 1.0.2 re-review, other per-user writes: the request log and searches.txt
    use std::io::Write;
    let owned = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-append").unwrap();
    let dir = owned.path().join("advisor");
    append_checked(&dir, "requests.log", 10, "requests.log.old", |f| f.write_all(b"first")).unwrap();
    append_checked(&dir, "requests.log", 10, "requests.log.old", |f| f.write_all(b" second")).unwrap();
    assert_eq!(fs::read(dir.join("requests.log")).unwrap(), b"first second");
    // past the limit: the old one moves aside and a new one begins
    append_checked(&dir, "requests.log", 10, "requests.log.old", |f| f.write_all(b"third")).unwrap();
    assert_eq!(fs::read(dir.join("requests.log.old")).unwrap(), b"first second");
    assert_eq!(fs::read(dir.join("requests.log")).unwrap(), b"third");
    // a second name for the log (a hard link to another file): refused, nothing written
    let other = owned.path().join("other.txt");
    fs::write(&other, b"not the log").unwrap();
    fs::remove_file(dir.join("requests.log")).unwrap();
    fs::hard_link(&other, dir.join("requests.log")).unwrap();
    assert!(append_checked(&dir, "requests.log", 1 << 20, "requests.log.old", |f| f.write_all(b"x")).is_err());
    assert!(remove_checked(&dir, "requests.log").is_err());
    assert_eq!(fs::read(&other).unwrap(), b"not the log");
    fs::remove_file(dir.join("requests.log")).unwrap();
    // a link at its name: refused, never followed
    if std::os::windows::fs::symlink_file(&other, dir.join("requests.log")).is_ok() {
        assert!(append_checked(&dir, "requests.log", 1 << 20, "requests.log.old", |f| f.write_all(b"x")).is_err());
        assert!(remove_checked(&dir, "requests.log").is_err());
        assert!(other.exists() && fs::read(&other).unwrap() == b"not the log");
        fs::remove_file(dir.join("requests.log")).unwrap();
    }
    // removal of its own file, and of nothing, are fine
    remove_checked(&dir, "requests.log.old").unwrap();
    assert!(!dir.join("requests.log.old").exists());
    remove_checked(&dir, "requests.log").unwrap();
    remove_checked(&owned.path().join("nowhere"), "requests.log").unwrap();
    // appends that overlap all get written (Astra 1.0.2 round 3: one could fail on sharing)
    let threads: Vec<_> = (0..8).map(|t| {
        let dir = dir.clone();
        std::thread::spawn(move || (0..20).all(|i| append_checked(&dir, "busy.log", 1 << 30, "busy.log.old", |f| f.write_all(format!("{t}-{i}\n").as_bytes())).is_ok()))
    }).collect();
    assert!(threads.into_iter().all(|t| t.join().unwrap()));
    assert_eq!(fs::read_to_string(dir.join("busy.log")).unwrap().lines().count(), 160);
    // while one is open, its name can't be renamed or removed by anyone else
    let held = ensure_dir(&dir).unwrap();
    let open = open_own(&held, "busy.log", false).unwrap();
    assert!(fs::rename(dir.join("busy.log"), dir.join("away.log")).is_err() && fs::remove_file(dir.join("busy.log")).is_err());
    drop((open, held));
    drop(owned);
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
