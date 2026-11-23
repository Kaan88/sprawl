//! Overwrite and delete (F9): overwrite a file's data once, then delete it; a folder's files
//! that way, then its empty folders.
//!
//! What it promises, and what it does not:
//! - One pass of random data over each file, from its start to the end of the space it takes on
//!   disk (its allocated length, so a sparse file's holes are not written); the data is flushed,
//!   the file's length set to 0, the file renamed to a random name in its own folder, and only
//!   then deleted. Nothing goes to the Recycle Bin.
//! - Nothing here claims the data is unrecoverable. On an SSD the drive moves writes around; a
//!   compressed, sparse, encrypted or WOF file and a copy-on-write volume may put the new data
//!   somewhere else; a cloud-synced folder may keep older versions online; and System Restore,
//!   shadow copies and the Recycle Bin may hold older copies. The window says each of these
//!   before anything runs ([`volume`], [`marks`]).
//! - It is at least as careful as a delete: every item is opened through a handle with links not
//!   followed, a path that leads elsewhere is left alone, links, reparse points, cloud or offline
//!   files and the never-list are refused, and anything created or changed after the scan (or
//!   the user's Keep list) stays. A file someone else is writing can't be opened for writing, so
//!   it stays too.
//! - Stop finishes the file it is on (a half-overwritten file would be neither kept nor gone),
//!   then stops.
//!
//! Free-space wiping, which is Windows' own `cipher /w:` and not an overwrite of any file, lives
//! at the end of this module: [`cipher_command`], [`wipe_free_space`] and the volume facts both
//! features' questions use ([`filesystem`], [`solid_state`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::clean::{same_path, win, Spare};

const REPARSE: u32 = 0x400;
const DIRECTORY: u32 = 0x10;
const READONLY: u32 = 0x1;
/// Offline, recall-on-open (cloud placeholder) and sparse cloud bits: a file that isn't here.
pub const NOT_LOCAL: u32 = 0x1000 | 0x0004_0000 | 0x0040_0000;

/// What one overwrite left, in the same shape as a clean-up's outcome.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Bytes written over (the progress shown while it runs).
    pub overwritten: u64,
    /// Files overwritten and deleted.
    pub files: u64,
    /// Folders removed (their files having gone).
    pub folders: u64,
    /// The files' own lengths, for the window's "what this frees" figure.
    pub bytes: u64,
    /// What was left, and why (the first few).
    pub problems: Vec<String>,
    pub more_problems: u64,
    /// Items left because they were created or changed after the scan.
    pub kept_new: u64,
    /// Items left because the user chose to keep them.
    pub kept_chosen: u64,
    /// Items left because they are on the never-list.
    pub kept_protected: u64,
    pub stopped: bool,
}

impl Outcome {
    fn problem(&mut self, p: &Path, why: impl std::fmt::Display) {
        if self.problems.len() < 8 {
            self.problems.push(format!("{}: {why}", p.display()));
        } else {
            self.more_problems += 1;
        }
    }
}

fn never() -> &'static crate::safety::Never {
    static N: std::sync::OnceLock<crate::safety::Never> = std::sync::OnceLock::new();
    N.get_or_init(crate::safety::Never::default_list)
}

/// Overwrite and delete `path` (a file or a folder), leaving what `spare` names. `progress` gets
/// the bytes written over so far. See the module comment for what it promises.
pub fn run(path: &Path, spare: &Spare, stop: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Outcome {
    let mut o = Outcome::default();
    if stop.load(Ordering::Relaxed) {
        o.stopped = true;
        return o;
    }
    // the never-list must be known in full: with a part of it unread nothing may go
    let never = never();
    if !never.complete() {
        o.problem(path, "left alone: Sprawl couldn't work out where Windows keeps its own folders");
        return o;
    }
    let path_buf = match sprawl_core::local::lexical(path) {
        Ok(p) => p,
        Err(e) => {
            o.problem(path, e);
            return o;
        }
    };
    let path = path_buf.as_path();
    // a drive's root as a whole is never offered, and its bookkeeping files stay
    if path.parent().is_none() {
        o.problem(path, "a whole drive cannot be overwritten");
        return o;
    }
    if let Some(p) = never.hit(&path.display().to_string()) {
        o.kept_protected += 1;
        o.problem(path, format!("left alone: it is protected ({p})"));
        return o;
    }
    if let Some(p) = never.path_hit(path) {
        o.kept_protected += 1;
        o.problem(path, format!("left alone: it is protected ({p})"));
        return o;
    }
    if odd_name(path) {
        o.problem(path, "skipped, its name ends in a dot or space");
        return o;
    }
    if spare.keeps(path) {
        o.kept_chosen += 1;
        return o;
    }
    // a file is opened for writing at once; a folder is opened to be emptied and removed
    let attrs = match win::open_attrs(path) {
        Ok((_, info)) => info,
        Err(e) => {
            o.problem(path, e);
            return o;
        }
    };
    if attrs.attrs & DIRECTORY == 0 {
        scrub_file(path, spare, &mut o, progress);
        return o;
    }
    let (h, info) = match win::open(path) {
        Ok(x) => x,
        Err(e) => {
            o.problem(path, e);
            return o;
        }
    };
    if spare.is_new(path, &info) {
        o.kept_new += 1;
        return o;
    }
    if info.attrs & REPARSE != 0 || info.attrs & DIRECTORY == 0 {
        o.problem(path, "left alone: it is not a plain folder any more");
        return o;
    }
    if info.attrs & NOT_LOCAL != 0 {
        o.problem(path, "left alone: a cloud or offline folder");
        return o;
    }
    match win::final_path(&h) {
        Some(real) if same_path(&real, path) => {}
        Some(real) => {
            o.problem(path, format!("now leads to {} instead; left alone", real.display()));
            return o;
        }
        None => {
            o.problem(path, "its real location couldn't be checked; left alone");
            return o;
        }
    }
    let kids = match list(path) {
        Ok((k, 0)) => k,
        Ok((k, n)) => {
            o.problem(path, unreadable(n));
            k
        }
        Err(e) => {
            o.problem(path, e);
            return o;
        }
    };
    let mut stack = vec![Frame { h, path: path.to_path_buf(), kids: kids.into_iter() }];
    while let Some(top) = stack.last_mut() {
        if stop.load(Ordering::Relaxed) {
            o.stopped = true;
            return o;
        }
        let Some(k) = top.kids.next() else {
            let f = stack.pop().unwrap();
            match win::delete(&f.h) {
                Ok(()) => o.folders += 1,
                // what must stay inside is already noted: the folder stays with it
                Err(e) if e.raw_os_error() == Some(145) => {}
                Err(e) => o.problem(&f.path, e),
            }
            continue;
        };
        if odd_name(&k) {
            o.problem(&k, "skipped, its name ends in a dot or space");
            continue;
        }
        if let Some(p) = never.path_hit(&k) {
            o.kept_protected += 1;
            o.problem(&k, format!("left alone: it is protected ({p})"));
            continue;
        }
        if spare.keeps(&k) {
            o.kept_chosen += 1;
            continue;
        }
        // read its marks without following a link and without downloading a cloud file
        let (ah, ainfo) = match win::open_attrs(&k) {
            Ok(x) => x,
            Err(e) => {
                o.problem(&k, e);
                continue;
            }
        };
        drop(ah);
        if spare.is_new(&k, &ainfo) {
            o.kept_new += 1;
            continue;
        }
        if ainfo.attrs & REPARSE != 0 {
            // a link is never followed, and a WOF-compressed file is data behind one: both stay
            o.problem(&k, "left alone: a link, junction or Windows-compressed (WOF) file");
            continue;
        }
        if ainfo.attrs & DIRECTORY == 0 {
            if ainfo.attrs & NOT_LOCAL != 0 {
                o.problem(&k, "left alone: a cloud or offline file");
                continue;
            }
            scrub_file(&k, spare, &mut o, progress);
            continue;
        }
        if ainfo.attrs & NOT_LOCAL != 0 {
            o.problem(&k, "left alone: a cloud or offline folder");
            continue;
        }
        // open it to be emptied and removed; it stays open (and so can't be swapped for a link)
        // while its contents go
        let (h, info) = match win::open(&k) {
            Ok(x) => x,
            Err(e) => {
                o.problem(&k, e);
                continue;
            }
        };
        if info.attrs & REPARSE != 0 || info.attrs & DIRECTORY == 0 || info.attrs & NOT_LOCAL != 0 {
            o.problem(&k, "left alone: it changed while the overwrite ran");
            continue;
        }
        match win::final_path(&h) {
            Some(real) if same_path(&real, &k) => {}
            Some(real) => {
                o.problem(&k, format!("now leads to {} instead; left alone", real.display()));
                continue;
            }
            None => {
                o.problem(&k, "its real location couldn't be checked; left alone");
                continue;
            }
        }
        match list(&k) {
            Ok((kids, unread)) => {
                if unread > 0 {
                    o.problem(&k, unreadable(unread));
                }
                stack.push(Frame { h, path: k, kids: kids.into_iter() });
            }
            Err(e) => o.problem(&k, e),
        }
    }
    o
}

/// One open folder and what is left to do in it.
struct Frame {
    h: win::Handle,
    path: PathBuf,
    kids: std::vec::IntoIter<PathBuf>,
}

/// One file, end to end: opened for writing (nobody else holds it for writing), checked, then
/// overwritten once, set to 0 bytes, given a random name and deleted. Left as it is with the
/// reason when anything refuses it.
fn scrub_file(p: &Path, spare: &Spare, o: &mut Outcome, progress: &mut dyn FnMut(u64)) {
    let (h, info) = match win::open_overwrite(p) {
        Ok(x) => x,
        Err(e) => {
            o.problem(p, format!("left alone: it couldn't be opened for writing, so it may be in use ({e})"));
            return;
        }
    };
    if info.attrs & DIRECTORY != 0 {
        o.problem(p, "left alone: it became a folder");
        return;
    }
    if info.attrs & REPARSE != 0 {
        o.problem(p, "left alone: a link, junction or Windows-compressed (WOF) file");
        return;
    }
    if info.attrs & NOT_LOCAL != 0 {
        o.problem(p, "left alone: a cloud or offline file");
        return;
    }
    if spare.is_new(p, &info) {
        o.kept_new += 1;
        return;
    }
    match win::final_path(&h) {
        Some(real) if same_path(&real, p) => {}
        Some(real) => {
            o.problem(p, format!("now leads to {} instead; left alone", real.display()));
            return;
        }
        None => {
            o.problem(p, "its real location couldn't be checked; left alone");
            return;
        }
    }
    let base = o.overwritten;
    let mut written = 0u64;
    let result = scrub_open(p, &h, &info, &mut written, &mut |n| progress(base + n));
    o.overwritten = base + written;
    match result {
        Ok(to) => match win::delete(&h) {
            Ok(()) => {
                o.files += 1;
                o.bytes += info.size;
                progress(o.overwritten);
            }
            // the name went with the delete as it was asked; nothing to report
            Err(e) if e.raw_os_error() == Some(145) => {}
            Err(e) => o.problem(&to, format!("overwritten once and set to 0 bytes, but it couldn't be deleted ({e}); it stays under this name")),
        },
        Err(why) => o.problem(p, why),
    }
}

/// The data step of one opened file: one pass of random data from its start to the end of its
/// allocated length, flushed, then its length set to 0 and a random name in the same folder.
/// Returns that name. `written` gets the bytes written, also when a later step fails, so the
/// caller can say how far it got.
fn scrub_open(p: &Path, h: &win::Handle, info: &win::Info, written: &mut u64, progress: &mut dyn FnMut(u64)) -> Result<PathBuf, String> {
    if info.attrs & READONLY != 0 {
        win::clear_readonly(h, info.attrs).map_err(|e| format!("left as it is: its read-only mark couldn't be cleared ({e})"))?;
    }
    let alloc = win::allocation(h, info.size);
    let mut state = seed(p);
    overwrite_once(h, alloc, &mut state, written, progress).map_err(|e| format!("left partly overwritten: the random data couldn't be written ({e})"))?;
    win::set_end(h, 0).map_err(|e| format!("overwritten once, but its length couldn't be set to 0 ({e}); it stays where it is"))?;
    win::flush(h).map_err(|e| format!("overwritten once and set to 0 bytes, but that couldn't be flushed to the disk ({e}); it stays where it is"))?;
    let Some(folder) = p.parent() else { return Err("it has no folder to be given a random name in".into()) };
    for _ in 0..8 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let to = folder.join(format!("sprawl-overwrite-{state:016x}"));
        match win::rename(h, &to) {
            Ok(()) => return Ok(to),
            Err(e) if matches!(e.raw_os_error(), Some(80) | Some(183)) => continue,
            Err(e) => return Err(format!("overwritten once and set to 0 bytes, but it couldn't be given a random name ({e}); it stays at this path")),
        }
    }
    Err("overwritten once and set to 0 bytes, but no free random name was found; it stays at this path".into())
}

/// Write one pass of pseudo-random bytes over the open file, from its start to `alloc` bytes,
/// and flush them. `written` counts them; progress is told as it goes.
fn overwrite_once(h: &win::Handle, alloc: u64, state: &mut u64, written: &mut u64, progress: &mut dyn FnMut(u64)) -> std::io::Result<()> {
    win::seek_start(h)?;
    let mut buf = vec![0u8; 1 << 20];
    while *written < alloc {
        let n = usize::try_from((alloc - *written).min(buf.len() as u64)).unwrap_or(buf.len());
        fill(&mut buf[..n], state);
        win::write(h, &buf[..n])?;
        *written += n as u64;
        progress(*written);
    }
    win::flush(h)
}

/// A xorshift64* stream into `buf`; the same bytes are never written by two files (each starts
/// from its own seed: the time, this process, the path and a counter).
fn fill(buf: &mut [u8], state: &mut u64) {
    if *state == 0 {
        *state = 0x9E37_79B9_7F4A_7C15;
    }
    for chunk in buf.chunks_mut(8) {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        chunk.copy_from_slice(&state.to_le_bytes()[..chunk.len()]);
    }
}

fn seed(p: &Path) -> u64 {
    static N: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for b in p.to_string_lossy().bytes() {
        hash = (hash ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    now ^ (std::process::id() as u64) << 32 ^ hash ^ N.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
}

fn odd_name(p: &Path) -> bool {
    p.file_name().map(|n| n.to_string_lossy()).is_some_and(|n| n.ends_with('.') || n.ends_with(' '))
}

/// The entries of folder `p`, and how many of them couldn't be read.
fn list(p: &Path) -> std::io::Result<(Vec<PathBuf>, usize)> {
    let (mut got, mut unread) = (Vec::new(), 0);
    for e in std::fs::read_dir(p)? {
        match e {
            Ok(e) => got.push(e.path()),
            Err(_) => unread += 1,
        }
    }
    Ok((got, unread))
}

fn unreadable(n: usize) -> String {
    format!("{n} item{} in it couldn't be read; left as {}", if n == 1 { "" } else { "s" }, if n == 1 { "it is" } else { "they are" })
}

// ---------------------------------------------------------------- volume facts and free space

/// The drive root (`C:\`) holding `path`, when `path` is a plain local drive path.
pub fn drive_root(path: &Path) -> Option<PathBuf> {
    match path.components().next()? {
        std::path::Component::Prefix(p) if matches!(p.kind(), std::path::Prefix::Disk(_)) => {
            Some(PathBuf::from(format!("{}\\", p.as_os_str().to_string_lossy())))
        }
        _ => None,
    }
}

/// The volume's file system name as Windows reports it ("NTFS", "ReFS"), from a path on it.
pub fn filesystem(path: &Path) -> Option<String> {
    let root = drive_root(path)?;
    let mut label = [0u16; 64];
    let mut fs = [0u16; 64];
    // SAFETY: both buffers are 64 units; the wide root is NUL-terminated.
    let ok = unsafe {
        GetVolumeInformationW(
            wide(&root).as_ptr(),
            label.as_mut_ptr(),
            64,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            64,
        )
    };
    (ok != 0).then(|| from_wide(&fs)).filter(|s| !s.is_empty())
}

/// Whether Windows says the volume takes no seek penalty (a solid-state drive). None when it
/// can't tell: an old drive, a volume that doesn't answer, or one this account can't query.
pub fn solid_state(path: &Path) -> Option<bool> {
    let root = drive_root(path)?;
    let letter = root.to_string_lossy();
    let device = format!(r"\\.\{}", letter.trim_end_matches('\\'));
    // no access at all, shared with everything: a query, not a read of the drive
    // SAFETY: the device name is NUL-terminated.
    let h = unsafe { CreateFileW(wide(Path::new(&device)).as_ptr(), 0, 0x1 | 0x2 | 0x4, std::ptr::null_mut(), 3, 0, std::ptr::null_mut()) };
    if h as isize == -1 {
        return None;
    }
    // STORAGE_PROPERTY_QUERY { PropertyId: 7 (StorageDeviceSeekPenaltyProperty), QueryType: 0 }
    let mut query = [0u8; 12];
    query[0..4].copy_from_slice(&7u32.to_le_bytes());
    // DEVICE_SEEK_PENALTY_DESCRIPTOR { Version u32, Size u32, IncursSeekPenalty u8 }
    let mut out = [0u8; 32];
    let mut returned = 0u32;
    // SAFETY: the buffers are the sizes given; only `returned` and `out` are written.
    let ok = unsafe {
        DeviceIoControl(h, 0x002D_1400, query.as_ptr().cast(), 12, out.as_mut_ptr().cast(), 32, &mut returned, std::ptr::null_mut())
    };
    // SAFETY: the handle came from CreateFileW above and is closed once.
    unsafe { CloseHandle(h as isize) };
    (ok != 0 && returned >= 9).then(|| out[8] == 0)
}

/// Windows' own free-space wipe for the drive `root` stands on, as the tool runner runs it:
/// `cipher /w:<root>` writes zeros, then ones, then random data over the drive's unused space.
pub fn cipher_command(root: &Path) -> String {
    format!("cipher {}", crate::rules::ps_quote(&format!("/w:{}", root.display())))
}

/// What a free-space wipe left. `ok` is whether cipher itself finished without a problem; what
/// it wrote stays written either way.
pub struct Wiped {
    pub outcome: crate::clean::Outcome,
    pub ok: bool,
}

/// Wipe the free space of the drive holding `root` through Windows' cipher and the clean-up's
/// tool runner (hidden, in a job, Stop ending it and what it started). Its working folder,
/// `EFSTMPWP`, is removed when cipher leaves it behind and it wasn't there before. A big drive
/// can take many hours, so this call's own limit is a day.
pub fn wipe_free_space(root: &Path, stop: &AtomicBool) -> Wiped {
    let Some(root) = drive_root(root) else {
        let mut outcome = crate::clean::Outcome::default();
        outcome.problems.push("not a local drive path".to_string());
        return Wiped { outcome, ok: false };
    };
    let scratch = root.join("EFSTMPWP");
    let existed = std::fs::symlink_metadata(&scratch).is_ok();
    let mut outcome = crate::clean::run_tool(&root, &cipher_command(&root), stop, Duration::from_secs(24 * 3600), &mut |_| {});
    let ok = !outcome.stopped && outcome.problems.is_empty();
    remove_leftover(&scratch, existed, &mut outcome);
    Wiped { outcome, ok }
}

/// Remove cipher's working folder when it left one behind (after a stop), and say so when it
/// couldn't go: it is a plain folder Sprawl saw cipher make, never one that was there before.
fn remove_leftover(scratch: &Path, existed: bool, o: &mut crate::clean::Outcome) {
    if existed {
        // it was there before this run: not this run's to remove
        return;
    }
    use std::os::windows::fs::MetadataExt;
    let Ok(m) = std::fs::symlink_metadata(scratch) else { return };
    if !m.is_dir() || m.file_attributes() & REPARSE != 0 {
        return;
    }
    if let Err(e) = std::fs::remove_dir_all(scratch) {
        if o.problems.len() < 8 {
            o.problems.push(format!("{}: cipher's working folder couldn't be removed ({e}); it can be deleted once the drive is free", scratch.display()));
        }
    }
}

fn wide(p: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

#[link(name = "kernel32")]
extern "system" {
    fn GetVolumeInformationW(root: *const u16, label: *mut u16, label_len: u32, serial: *mut u32, max_len: *mut u32, flags: *mut u32, fs: *mut u16, fs_len: u32) -> i32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut std::ffi::c_void, disposition: u32, flags: u32, template: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn DeviceIoControl(h: *mut std::ffi::c_void, code: u32, inbuf: *const std::ffi::c_void, inlen: u32, outbuf: *mut std::ffi::c_void, outlen: u32, returned: *mut u32, overlapped: *mut std::ffi::c_void) -> i32;
    #[link_name = "CloseHandle"]
    fn CloseHandle(h: isize) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("sprawl-overwrite-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn zeros(dir: &Path) -> usize {
        std::fs::read_dir(dir).unwrap().flatten().count()
    }

    #[test]
    fn overwriting_replaces_the_content() {
        let base = temp("content");
        let f = base.join("notes.bin");
        let pattern = b"SPRAWL-KNOWN-PATTERN-0123456789".repeat(2000);
        std::fs::write(&f, &pattern).unwrap();
        let (h, info) = win::open_overwrite(&f).unwrap();
        let alloc = win::allocation(&h, info.size);
        let mut state = seed(&f);
        let mut written = 0u64;
        overwrite_once(&h, alloc, &mut state, &mut written, &mut |_| {}).unwrap();
        assert_eq!(written, alloc.max(info.size), "the whole allocated length was written");
        drop(h);
        let after = std::fs::read(&f).unwrap();
        assert!(after.len() >= pattern.len(), "the file still has its bytes to read back");
        let hit = after.windows(pattern.len()).any(|w| w == pattern.as_slice());
        assert!(!hit, "none of the pattern is left after one pass");
        assert!(after.iter().any(|&b| b != 0), "the pass wrote data, not just zeros");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_file_is_set_to_zero_and_renamed_before_it_is_deleted() {
        let base = temp("rename");
        let f = base.join("old-name.txt");
        std::fs::write(&f, vec![7u8; 4096]).unwrap();
        let (h, info) = win::open_overwrite(&f).unwrap();
        let mut written = 0u64;
        let to = scrub_open(&f, &h, &info, &mut written, &mut |_| {}).unwrap();
        assert!(written >= info.size);
        assert!(!f.exists(), "the old name is gone");
        assert!(to.exists(), "the file is there under its random name");
        assert_eq!(to.parent(), Some(base.as_path()));
        let name = to.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("sprawl-overwrite-") && name.len() == "sprawl-overwrite-".len() + 16, "{name}");
        assert_eq!(std::fs::metadata(&to).unwrap().len(), 0, "its length is 0 before the delete");
        win::delete(&h).unwrap();
        drop(h);
        assert!(!to.exists(), "then it is gone");
        assert_eq!(zeros(&base), 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn links_and_reparse_points_are_left_alone() {
        let base = temp("links");
        let keep = base.join("keep");
        std::fs::create_dir_all(keep.join("inner")).unwrap();
        std::fs::write(keep.join("inner").join("precious.txt"), b"keep me").unwrap();
        let folder = base.join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("gone.bin"), vec![3u8; 900]).unwrap();
        let link = folder.join("link");
        let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&keep).output().is_ok_and(|o| o.status.success());
        let stop = AtomicBool::new(false);
        let o = run(&folder, &Spare::default(), &stop, &mut |_| {});
        assert!(o.problems.iter().any(|p| p.contains("link") || p.contains("reparse")), "{:?}", o.problems);
        assert!(keep.join("inner").join("precious.txt").exists(), "the junction's target stays");
        assert_eq!(std::fs::read(keep.join("inner").join("precious.txt")).unwrap(), b"keep me");
        assert!(!folder.join("gone.bin").exists(), "the plain file went");
        if ok {
            assert!(link.exists(), "the junction itself stays: it is not followed and not overwritten");
            assert!(folder.exists(), "the folder stays, holding the link it must not remove");
        } else {
            assert!(!folder.exists(), "with no link made, the folder went too");
        }

        // the junction as the whole item: refused, and its target untouched
        let as_root = run(&link, &Spare::default(), &stop, &mut |_| {});
        assert!(!as_root.problems.is_empty() && as_root.files == 0, "{:?}", as_root.problems);
        assert!(keep.join("inner").join("precious.txt").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_file_changed_after_the_scan_is_left_alone() {
        let base = temp("changed");
        let f = base.join("changed.bin");
        std::fs::write(&f, b"before").unwrap();
        let cut = SystemTime::now();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&f, b"after the scan began").unwrap();
        let spare = Spare { since: Some(cut), keep: Vec::new(), seen: None };
        let stop = AtomicBool::new(false);
        let o = run(&f, &spare, &stop, &mut |_| {});
        assert_eq!((o.files, o.kept_new), (0, 1), "{:?}", o.problems);
        assert_eq!(std::fs::read(&f).unwrap(), b"after the scan began");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn stop_finishes_the_file_it_is_on_and_leaves_the_rest() {
        let base = temp("stop");
        std::fs::write(base.join("a.bin"), vec![1u8; 2 << 20]).unwrap();
        std::fs::write(base.join("b.bin"), b"second").unwrap();
        let stop = AtomicBool::new(false);
        let o = run(&base, &Spare::default(), &stop, &mut |_| {
            stop.store(true, Ordering::Relaxed);
        });
        assert!(o.stopped, "it stopped");
        assert_eq!(o.files, 1, "the file it was on finished: {:?}", o.problems);
        let left: Vec<String> = std::fs::read_dir(&base).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec!["b.bin".to_string()], "the rest stayed, and no random name was left behind");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_folder_goes_file_by_file_and_then_its_empty_folders() {
        let base = temp("folder");
        let root = base.join("target");
        std::fs::create_dir_all(root.join("sub").join("deeper")).unwrap();
        std::fs::write(root.join("one.bin"), vec![9u8; 700]).unwrap();
        std::fs::write(root.join("sub").join("two.bin"), vec![9u8; 1300]).unwrap();
        std::fs::write(root.join("sub").join("deeper").join("three.bin"), vec![9u8; 100]).unwrap();
        let stop = AtomicBool::new(false);
        let o = run(&root, &Spare::default(), &stop, &mut |_| {});
        assert!(o.problems.is_empty(), "{:?}", o.problems);
        assert_eq!((o.files, o.bytes), (3, 2100));
        assert_eq!(o.folders, 3, "the folder, its sub and the folder inside it");
        assert!(!root.exists(), "the folder picked went too");
        assert_eq!(zeros(&base), 0, "nothing was left behind");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_kept_file_keeps_its_folder() {
        let base = temp("kept");
        let root = base.join("target");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("go.bin"), vec![1u8; 10]).unwrap();
        let stay = root.join("stay.txt");
        std::fs::write(&stay, b"mine").unwrap();
        let spare = Spare { since: None, keep: vec![stay.clone()], seen: None };
        let stop = AtomicBool::new(false);
        let o = run(&root, &spare, &stop, &mut |_| {});
        assert_eq!((o.files, o.kept_chosen), (1, 1), "{:?}", o.problems);
        assert_eq!(std::fs::read(&stay).unwrap(), b"mine");
        assert!(root.exists(), "the folder stays, holding the kept file");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_read_only_file_goes_too() {
        let base = temp("readonly");
        let f = base.join("read-only.bin");
        std::fs::write(&f, b"not for changing").unwrap();
        let mut perm = std::fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&f, perm).unwrap();
        let stop = AtomicBool::new(false);
        let o = run(&f, &Spare::default(), &stop, &mut |_| {});
        assert_eq!(o.files, 1, "{:?}", o.problems);
        assert!(!f.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cipher_leaves_its_working_folder_removed_only_when_it_made_it() {
        let base = temp("cipher-leftover");
        let scratch = base.join("EFSTMPWP");
        std::fs::create_dir_all(scratch.join("inner")).unwrap();
        std::fs::write(scratch.join("inner").join("part"), b"x").unwrap();
        let mut o = crate::clean::Outcome::default();
        remove_leftover(&scratch, false, &mut o);
        assert!(!scratch.exists() && o.problems.is_empty());

        // one that was there before this run is not this run's to remove
        std::fs::create_dir_all(scratch.join("inner")).unwrap();
        std::fs::write(scratch.join("inner").join("part"), b"x").unwrap();
        let mut o = crate::clean::Outcome::default();
        remove_leftover(&scratch, true, &mut o);
        assert!(scratch.join("inner").join("part").exists(), "a folder that was there before stays");
        assert!(o.problems.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_cipher_command_names_the_drive_root() {
        assert_eq!(cipher_command(Path::new(r"C:\")), r"cipher '/w:C:\'");
        assert_eq!(cipher_command(Path::new(r"D:\")), r"cipher '/w:D:\'");
        assert_eq!(drive_root(Path::new(r"C:\Users\x")).as_deref(), Some(Path::new(r"C:\")));
        assert_eq!(drive_root(Path::new(r"C:\")), Some(PathBuf::from(r"C:\")));
        assert!(drive_root(Path::new(r"\\server\share\a")).is_none());
        assert!(filesystem(&std::env::temp_dir()).is_some_and(|fs| !fs.is_empty()), "the temp folder's file system is known");
        // the answer may be None (an old drive, a VM, no rights); it must never panic
        let _ = solid_state(&std::env::temp_dir());
    }
}
