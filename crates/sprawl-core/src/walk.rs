//! Fast folder walk that needs no admin rights.
//!
//! * `FindFirstFileExW(FindExInfoBasic, FIND_FIRST_EX_LARGE_FETCH)`: size, attributes and
//!   modified time come straight from the directory listing. Plain nonempty files also get
//!   an optional pinned metadata-only WOF query: Windows can hide compression from that
//!   listing. Interactive inventories omit that per-file probe; explicit detailed walks keep it.
//! * Directories are spread over a pool of worker threads (a shared LIFO work queue, so the walk
//!   stays roughly depth-first and memory stays low).
//! * Reparse points (junctions, symbolic links, mount points) are listed as themselves and
//!   never entered - except the scanned path itself.
//! * Paths are passed as `\\?\` long paths, so deep trees beyond MAX_PATH work.
//! * Allocated size is the length rounded up to the volume's cluster size. (Whether a small
//!   file lives inside its MFT record, or how much a compressed file really takes, can't be seen
//!   from a listing; the MFT scanner knows.)

use std::collections::VecDeque;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{GetLastError, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, GetDiskFreeSpaceW, GetVolumePathNameW,
    FIND_FIRST_EX_LARGE_FETCH, WIN32_FIND_DATAW,
};

use crate::progress::{LiveBatch, Progress, ScanMethod};
use crate::{attr, filetime_to_system, top_of, Node, Tree, ROOT};

const NO_JOB: u32 = u32::MAX;

/// One directory entry as listed.
struct Entry {
    name: String,
    size: u64,
    attrs: u32,
    modified: u64,
    /// The job that lists this folder, or NO_JOB.
    job: u32,
}

struct Job {
    id: u32,
    /// `\\?\` path, no terminating NUL.
    path: Vec<u16>,
    /// Slot of the root's direct child this folder is under (NO_JOB for the root itself).
    top: u32,
}

struct Queue {
    jobs: Vec<Job>,
    /// Queued plus running jobs; 0 means the walk is finished.
    pending: usize,
}

struct Shared<'a> {
    queue: Mutex<Queue>,
    cv: Condvar,
    next_id: AtomicU32,
    progress: &'a Progress,
    detect_wof: bool,
    /// Local inventories: the root's real path on its volume (lower case, no trailing `\`)
    /// and the length of the root job's path. Each folder is opened (links not followed),
    /// checked to be at this path plus its own relative path, and listed through that handle,
    /// so a folder or ancestor swapped for a junction after it was queued is never listed.
    verify: Option<(String, usize)>,
}

/// How many threads list folders. Listing is I/O- and kernel-bound; a handful more threads than
/// cores doesn't help.
fn worker_count() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(4, 16)
}

/// Walk `path` into a tree, reporting into `progress`. Returns `ErrorKind::Interrupted` when
/// cancelled.
pub fn walk(path: &Path, progress: &Progress) -> io::Result<Tree> {
    walk_with(path, progress, worker_count())
}

/// Fast inventory for interactive scans and Cleaner discovery. Directory entries already
/// contain the size, attributes and time; do not open every plain file just to discover
/// WOF compression hidden by Windows. Visible compression tags are still recorded.
/// Use `walk` when an explicit storage operation needs complete WOF metadata.
pub fn inventory(path: &Path, progress: &Progress) -> io::Result<Tree> {
    walk_mode(path, progress, worker_count(), false)
}

/// [`inventory`] with an explicit number of worker threads (a quiet background scan).
pub fn inventory_with(path: &Path, progress: &Progress, threads: usize) -> io::Result<Tree> {
    walk_mode(path, progress, threads, false)
}

/// [`walk`] with an explicit number of worker threads (for benchmarks).
pub fn walk_with(path: &Path, progress: &Progress, threads: usize) -> io::Result<Tree> {
    walk_mode(path, progress, threads, true)
}

fn walk_mode(path: &Path, progress: &Progress, threads: usize, detect_wof: bool) -> io::Result<Tree> {
    let scan = crate::local::scan_root(path, true)?;
    let path = &scan.path;
    // WOF is local NTFS: a network share is listed only, never probed.
    let detect_wof = detect_wof && scan.pinned.is_some();
    if progress.excludes(path) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "This folder is excluded in Settings > Scanning"));
    }
    progress.set_method(ScanMethod::Walk);
    progress.set_phase("Scanning folders");
    let meta = std::fs::symlink_metadata(path)?;
    let abs = std::path::absolute(path)?;
    let cluster = cluster_size(&abs).unwrap_or(4096);
    let mut root = Node {
        name: path.display().to_string(),
        is_dir: meta.is_dir(),
        modified: meta.modified().ok(),
        attributes: meta.file_attributes(),
        ..Default::default()
    };
    if !meta.is_dir() {
        let mut errors = 0;
        if root.attributes & attr::COMPRESSED == 0 && meta.len() > 0 {
            match scan.pinned.as_ref().map_or(Ok(false), |p| p.externally_backed()) {
                Ok(true) => root.attributes |= attr::WOF,
                Ok(false) => {},
                Err(_) => { errors += 1; progress.add_errors(1); }
            }
        }
        root.size = meta.len();
        root.allocated = round_up(root.size, cluster);
        root.files = 1;
        progress.add_counts(1, 0, root.size);
        progress.set_phase("Done");
        return Ok(Tree { root_path: path.to_path_buf(), nodes: vec![root], errors, method: ScanMethod::Walk });
    }
    let errors_before = progress.errors();

    let root_job = long_path(&abs);
    // the detailed walk pins every folder's whole chain instead; a share is listed as it is
    let verify = if scan.pinned.is_some() && !detect_wof {
        Some((dir::open(&root_job).and_then(|h| dir::real_path(&h))?, root_job.len()))
    } else {
        None
    };
    let shared = Shared {
        queue: Mutex::new(Queue { jobs: vec![Job { id: 0, path: root_job, top: NO_JOB }], pending: 1 }),
        cv: Condvar::new(),
        next_id: AtomicU32::new(1),
        progress,
        detect_wof,
        verify,
    };
    let results: Vec<Vec<(u32, Vec<Entry>)>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads.max(1)).map(|_| s.spawn(|| worker(&shared))).collect();
        handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
    });
    if progress.is_cancelled() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "scan cancelled"));
    }
    progress.set_phase("Building tree");

    // Lists by job id, then breadth-first so parents get lower indices than their children.
    let jobs = shared.next_id.load(Ordering::Relaxed) as usize;
    let mut lists: Vec<Option<Vec<Entry>>> = (0..jobs).map(|_| None).collect();
    let mut total = 1usize;
    for part in results {
        for (id, entries) in part {
            total += entries.len();
            lists[id as usize] = Some(entries);
        }
    }
    let mut nodes: Vec<Node> = Vec::with_capacity(total);
    nodes.push(root);
    let mut queue: VecDeque<(usize, u32)> = VecDeque::from([(ROOT, 0u32)]);
    while let Some((ni, jid)) = queue.pop_front() {
        let entries = lists[jid as usize].take().unwrap_or_default();
        let mut children = Vec::with_capacity(entries.len());
        for e in entries {
            let idx = nodes.len();
            children.push(idx);
            let is_dir = e.attrs & attr::DIRECTORY != 0;
            nodes.push(Node {
                name: e.name,
                is_dir,
                size: if is_dir { 0 } else { e.size },
                allocated: if is_dir { 0 } else { round_up(e.size, cluster) },
                files: u64::from(!is_dir),
                folders: 0,
                modified: filetime_to_system(e.modified),
                attributes: e.attrs,
                parent: Some(ni),
                children: Vec::new(),
            });
            if e.job != NO_JOB {
                queue.push_back((idx, e.job));
            }
        }
        nodes[ni].children = children;
    }
    let mut tree = Tree { root_path: path.to_path_buf(), nodes, errors: progress.errors() - errors_before, method: ScanMethod::Walk };
    tree.compute_totals();
    progress.top_reset(top_of(&tree));
    progress.set_phase("Done");
    Ok(tree)
}

fn worker(sh: &Shared) -> Vec<(u32, Vec<Entry>)> {
    let mut out = Vec::new();
    let mut batch = LiveBatch::default();
    let mut last_flush = Instant::now();
    // SAFETY: WIN32_FIND_DATAW is plain old data; all-zero is a valid value.
    let mut find: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let mut pattern: Vec<u16> = Vec::with_capacity(1024);
    let mut buf: Vec<u64> = Vec::new();
    loop {
        let job = {
            let mut q = sh.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if sh.progress.is_cancelled() {
                    return out;
                }
                if let Some(j) = q.jobs.pop() {
                    break j;
                }
                if q.pending == 0 {
                    sh.progress.merge(&mut batch);
                    return out;
                }
                q = sh.cv.wait_timeout(q, Duration::from_millis(50)).unwrap_or_else(|e| e.into_inner()).0;
            }
        };
        let mut subjobs = Vec::new();
        let entries = list_dir(sh, &job, &mut find, &mut pattern, &mut buf, &mut subjobs, &mut batch);
        out.push((job.id, entries));
        if last_flush.elapsed() > Duration::from_millis(15) {
            batch.current = Some(display_path(&job.path));
            sh.progress.merge(&mut batch);
            last_flush = Instant::now();
        }
        let n = subjobs.len();
        let finished = {
            let mut q = sh.queue.lock().unwrap_or_else(|e| e.into_inner());
            q.jobs.extend(subjobs);
            q.pending = q.pending + n - 1;
            q.pending == 0
        };
        if finished || n > 1 {
            sh.cv.notify_all();
        } else if n == 1 {
            sh.cv.notify_one();
        }
    }
}

/// List one folder. Subfolders to enter are appended to `subjobs`.
fn list_dir(
    sh: &Shared,
    job: &Job,
    find: &mut WIN32_FIND_DATAW,
    pattern: &mut Vec<u16>,
    buf: &mut Vec<u64>,
    subjobs: &mut Vec<Job>,
    batch: &mut LiveBatch,
) -> Vec<Entry> {
    let mut entries = Vec::new();
    // Only the detailed walk opens files (the WOF probe), so only it pins the folder's whole
    // chain first. An inventory opens the folder itself once and checks where it really is
    // (`Shared::verify`): pinning every ancestor of every folder made scans 15-55x slower (1.0).
    let pinned = if sh.detect_wof {
        match crate::local::directory(Path::new(&display_path(&job.path))) {
            Ok(p) => Some(p),
            Err(_) => { sh.progress.add_errors(1); return entries; }
        }
    } else {
        None
    };
    let is_root = job.top == NO_JOB;
    let mut top_names: Vec<(String, u64)> = Vec::new();
    let (mut files, mut folders, mut bytes) = (0u64, 0u64, 0u64);
    let mut visit = |wname: &[u16], raw_attrs: u32, tag: u32, size: u64, modified: u64| -> bool {
        if sh.progress.is_cancelled() { return false; }
        let dot = wname == [b'.' as u16] || wname == [b'.' as u16, b'.' as u16];
        let excluded = !dot && sh.progress.has_exclusions() && sh.progress.excludes(&Path::new(&display_path(&job.path)).join(String::from_utf16_lossy(wname)));
        if !dot && !excluded {
            let name = String::from_utf16_lossy(wname);
            let mut attrs = raw_attrs | if raw_attrs & attr::REPARSE_POINT != 0 && tag == 0x8000_0017 { attr::WOF } else { 0 };
            if let Some(pinned) = pinned.as_ref().filter(|_| size > 0 && attrs & (attr::DIRECTORY | attr::REPARSE_POINT | attr::COMPRESSED | 0x1000 | 0x0004_0000 | 0x0040_0000) == 0) {
                #[cfg(test)]
                sh.progress.wof_probes.fetch_add(1, Ordering::Relaxed);
                match pinned.child_externally_backed(&name) {
                    Ok(true) => attrs |= attr::WOF,
                    Ok(false) => {},
                    Err(_) => sh.progress.add_errors(1),
                }
            }
            let is_dir = attrs & attr::DIRECTORY != 0;
            let slot = if is_root { entries.len() as u32 } else { job.top };
            let mut child_job = NO_JOB;
            if is_dir {
                folders += 1;
                if attrs & attr::REPARSE_POINT == 0 {
                    child_job = sh.next_id.fetch_add(1, Ordering::Relaxed);
                    let mut path = Vec::with_capacity(job.path.len() + 1 + wname.len());
                    path.extend_from_slice(&job.path);
                    if path.last() != Some(&(b'\\' as u16)) {
                        path.push(b'\\' as u16);
                    }
                    path.extend_from_slice(wname);
                    subjobs.push(Job { id: child_job, path, top: slot });
                }
            } else {
                files += 1;
                bytes += size;
                batch.add_file(&name, size);
                if is_root {
                    batch.add_top(slot as usize, size);
                }
            }
            if is_root {
                top_names.push((name.clone(), 0));
            }
            entries.push(Entry { name, size, attrs, modified, job: child_job });
        }
        true
    };
    let listed = match &sh.verify {
        Some((root, root_len)) => list_by_handle(job, root, *root_len, buf, &mut visit),
        None => list_by_name(job, find, pattern, &mut visit),
    };
    if !listed {
        sh.progress.add_errors(1);
    }
    if is_root {
        sh.progress.top_reset(top_names);
    } else {
        batch.add_top(job.top as usize, bytes);
    }
    sh.progress.add_counts(files, folders, bytes);
    entries
}

/// Each entry of `job`'s folder as (name, attributes, reparse tag, size, modified), by name
/// (`FindFirstFileExW`). False when the listing failed (a folder that is gone is not a failure).
fn list_by_name(job: &Job, find: &mut WIN32_FIND_DATAW, pattern: &mut Vec<u16>, visit: &mut dyn FnMut(&[u16], u32, u32, u64, u64) -> bool) -> bool {
    pattern.clear();
    pattern.extend_from_slice(&job.path);
    if pattern.last() != Some(&(b'\\' as u16)) {
        pattern.push(b'\\' as u16);
    }
    pattern.push(b'*' as u16);
    pattern.push(0);
    // SAFETY: `pattern` is NUL-terminated, `find` is a valid WIN32_FIND_DATAW.
    let h = unsafe {
        FindFirstFileExW(
            pattern.as_ptr(),
            FindExInfoBasic,
            find as *mut WIN32_FIND_DATAW as *mut _,
            FindExSearchNameMatch,
            std::ptr::null(),
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        // SAFETY: trivially safe.
        return unsafe { GetLastError() } == ERROR_FILE_NOT_FOUND;
    }
    let mut ok = true;
    loop {
        let len = find.cFileName.iter().position(|&c| c == 0).unwrap_or(find.cFileName.len());
        let size = (u64::from(find.nFileSizeHigh) << 32) | u64::from(find.nFileSizeLow);
        let modified = (u64::from(find.ftLastWriteTime.dwHighDateTime) << 32) | u64::from(find.ftLastWriteTime.dwLowDateTime);
        if !visit(&find.cFileName[..len], find.dwFileAttributes, find.dwReserved0, size, modified) { break; }
        // SAFETY: `h` is a valid find handle.
        if unsafe { FindNextFileW(h, find) } == 0 {
            // SAFETY: trivially safe.
            ok = unsafe { GetLastError() } == ERROR_NO_MORE_FILES;
            break;
        }
    }
    // SAFETY: `h` is a valid find handle, closed once.
    unsafe { FindClose(h) };
    ok
}

/// [`list_by_name`] through a handle to the folder itself, after checking that it is a plain
/// folder at `root` plus the job's own relative path. A folder that has become a link, or whose
/// path now leads elsewhere, is not listed (and counts as unreadable).
fn list_by_handle(job: &Job, root: &str, root_len: usize, buf: &mut Vec<u64>, visit: &mut dyn FnMut(&[u16], u32, u32, u64, u64) -> bool) -> bool {
    let h = match dir::open(&job.path) {
        Ok(h) => h,
        Err(e) => return e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32),
    };
    let expected = format!("{root}{}", String::from_utf16_lossy(&job.path[root_len.min(job.path.len())..]).trim_end_matches('\\')).to_lowercase();
    if !dir::is_plain_dir(&h) || dir::real_path(&h).ok().as_deref() != Some(expected.as_str()) {
        return false;
    }
    dir::list(&h, buf, visit)
}

/// Folders opened as themselves and listed through their handle.
mod dir {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, GetFileInformationByHandleEx, GetFinalPathNameByHandleW};

    pub(super) fn open(long: &[u16]) -> std::io::Result<OwnedHandle> {
        const LIST_DIRECTORY: u32 = 0x1;
        const SYNCHRONIZE: u32 = 0x0010_0000;
        const OPEN_EXISTING: u32 = 3;
        const BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        let mut w = long.to_vec();
        w.push(0);
        // SAFETY: `w` is NUL-terminated; the handle is owned below.
        let h = unsafe { CreateFileW(w.as_ptr(), LIST_DIRECTORY | SYNCHRONIZE, 0x1 | 0x2 | 0x4, std::ptr::null(), OPEN_EXISTING, BACKUP_SEMANTICS | OPEN_REPARSE_POINT, std::ptr::null_mut()) };
        if h as isize == -1 { return Err(std::io::Error::last_os_error()); }
        // SAFETY: a fresh handle from CreateFileW.
        Ok(unsafe { OwnedHandle::from_raw_handle(h) })
    }

    /// A folder, not a link or other reparse point.
    pub(super) fn is_plain_dir(h: &OwnedHandle) -> bool {
        let mut tag = [0u32; 2]; // FILE_ATTRIBUTE_TAG_INFO: attributes, reparse tag
        // SAFETY: the buffer is the size given; class 9 is FileAttributeTagInfo.
        let ok = unsafe { GetFileInformationByHandleEx(h.as_raw_handle(), 9, tag.as_mut_ptr().cast(), 8) } != 0;
        ok && tag[0] & crate::attr::DIRECTORY != 0 && tag[0] & crate::attr::REPARSE_POINT == 0
    }

    /// Where the opened folder really is on its volume, lower case, without a trailing `\`
    /// (`\` for a volume's root becomes the empty string).
    pub(super) fn real_path(h: &OwnedHandle) -> std::io::Result<String> {
        const VOLUME_NAME_NONE: u32 = 0x4;
        let mut out = vec![0u16; 512];
        loop {
            // SAFETY: `out` has the capacity given.
            let n = unsafe { GetFinalPathNameByHandleW(h.as_raw_handle(), out.as_mut_ptr(), out.len() as u32, VOLUME_NAME_NONE) } as usize;
            if n == 0 { return Err(std::io::Error::last_os_error()); }
            if n < out.len() { return Ok(String::from_utf16_lossy(&out[..n]).trim_end_matches('\\').to_lowercase()); }
            if n > 65_536 { return Err(std::io::Error::other("The final path is too long")); }
            out.resize(n + 1, 0);
        }
    }

    /// Every entry, as FindFirstFileExW's basic listing gives them (FILE_FULL_DIR_INFO).
    pub(super) fn list(h: &OwnedHandle, buf: &mut Vec<u64>, visit: &mut dyn FnMut(&[u16], u32, u32, u64, u64) -> bool) -> bool {
        const FULL_DIRECTORY_INFO: i32 = 14;
        const FULL_DIRECTORY_RESTART_INFO: i32 = 15;
        const NO_MORE_FILES: i32 = 18;
        if buf.len() < 8192 { buf.resize(8192, 0); } // 64 KiB, 8-byte aligned
        let bytes = buf.len() * 8;
        let mut class = FULL_DIRECTORY_RESTART_INFO;
        loop {
            // SAFETY: `buf` holds `bytes` bytes.
            if unsafe { GetFileInformationByHandleEx(h.as_raw_handle(), class, buf.as_mut_ptr().cast(), bytes as u32) } == 0 {
                return std::io::Error::last_os_error().raw_os_error() == Some(NO_MORE_FILES);
            }
            class = FULL_DIRECTORY_INFO;
            // SAFETY: the reads below stay inside `buf` (checked against `bytes`).
            let base = buf.as_ptr() as *const u8;
            let mut off = 0usize;
            loop {
                if off + 68 > bytes { return false; }
                let rd = |at: usize| unsafe { std::ptr::read_unaligned(base.add(off + at) as *const u32) };
                let rd64 = |at: usize| unsafe { std::ptr::read_unaligned(base.add(off + at) as *const u64) };
                let (next, written, size, attrs, name_len, ea) = (rd(0) as usize, rd64(24), rd64(40), rd(56), rd(60) as usize, rd(64));
                if off + 68 + name_len > bytes || name_len % 2 != 0 { return false; }
                // SAFETY: in bounds (checked above); u16 reads unaligned-safe via a copy.
                let name: Vec<u16> = (0..name_len / 2).map(|i| unsafe { std::ptr::read_unaligned(base.add(off + 68 + 2 * i) as *const u16) }).collect();
                // the reparse tag is in EaSize for a reparse point, as FindFirstFile's dwReserved0
                if !visit(&name, attrs, if attrs & crate::attr::REPARSE_POINT != 0 { ea } else { 0 }, size, written) { return true; }
                if next == 0 { break; }
                off += next;
            }
        }
    }
}

pub(crate) fn round_up(size: u64, cluster: u64) -> u64 {
    if cluster == 0 {
        size
    } else {
        size.div_ceil(cluster) * cluster
    }
}

/// `C:\x` -> `\\?\C:\x`, `\\server\share` -> `\\?\UNC\server\share`. `abs` must be absolute.
pub(crate) fn long_path(abs: &Path) -> Vec<u16> {
    let s: Vec<u16> = abs.as_os_str().encode_wide().collect();
    let w = |t: &str| t.encode_utf16().collect::<Vec<u16>>();
    if s.starts_with(&w(r"\\?\")) || s.starts_with(&w(r"\\.\")) {
        s
    } else if s.starts_with(&w(r"\\")) {
        let mut out = w(r"\\?\UNC\");
        out.extend_from_slice(&s[2..]);
        out
    } else {
        let mut out = w(r"\\?\");
        out.extend_from_slice(&s);
        out
    }
}

/// A `\\?\` path back to the usual form, for display.
pub(crate) fn display_path(p: &[u16]) -> String {
    let s = String::from_utf16_lossy(p);
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s
    }
}

pub(crate) fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// The volume root (`C:\`, or a mount folder) that `path` lives on, NUL-terminated.
pub(crate) fn volume_path_name(path: &Path) -> Option<Vec<u16>> {
    let p = wide(path.as_os_str());
    let mut buf = vec![0u16; 1024];
    // SAFETY: `p` is NUL-terminated, `buf` has the given capacity.
    if unsafe { GetVolumePathNameW(p.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } == 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0)?;
    buf.truncate(n + 1);
    Some(buf)
}

/// Bytes per cluster of the volume holding `path`.
pub(crate) fn cluster_size(path: &Path) -> Option<u64> {
    let root = volume_path_name(path)?;
    let (mut spc, mut bps, mut free, mut total) = (0u32, 0u32, 0u32, 0u32);
    // SAFETY: `root` is NUL-terminated; out-pointers are valid.
    if unsafe { GetDiskFreeSpaceW(root.as_ptr(), &mut spc, &mut bps, &mut free, &mut total) } == 0 {
        return None;
    }
    let c = u64::from(spc) * u64::from(bps);
    (c > 0).then_some(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sprawl-walk-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn long_paths() {
        let s = |v: Vec<u16>| String::from_utf16(&v).unwrap();
        assert_eq!(s(long_path(Path::new(r"C:\x\y"))), r"\\?\C:\x\y");
        assert_eq!(s(long_path(Path::new(r"\\srv\share\x"))), r"\\?\UNC\srv\share\x");
        assert_eq!(s(long_path(Path::new(r"\\?\C:\x"))), r"\\?\C:\x");
        assert_eq!(display_path(&long_path(Path::new(r"\\srv\share\x"))), r"\\srv\share\x");
        assert_eq!(round_up(1, 4096), 4096);
        assert_eq!(round_up(0, 4096), 0);
        assert_eq!(round_up(8192, 4096), 8192);
    }

    #[test]
    fn interactive_inventory_does_not_probe_every_plain_file() {
        let dir = temp_dir("inventory-probes");
        let path = dir.join("busy.bin");
        fs::write(&path, vec![7u8; 8192]).unwrap();
        let progress = Progress::new();
        let fast = inventory(&dir, &progress).unwrap();
        assert_eq!(fast.nodes[ROOT].size, 8192);
        assert_eq!(fast.nodes[ROOT].files, 1);
        assert_eq!(fast.errors, 0, "listing must not probe each ordinary file's WOF backing");
        assert_eq!(progress.wof_probes.load(Ordering::Relaxed), 0);
        let detailed_progress = Progress::new();
        let detailed = walk(&dir, &detailed_progress).unwrap();
        assert_eq!(detailed.errors, 0);
        assert_eq!(detailed_progress.wof_probes.load(Ordering::Relaxed), 1, "explicit detailed walks retain WOF discovery");
        let many = crate::scan_many_inventory(&[dir.clone()], &Progress::new()).unwrap();
        assert_eq!(many.method, ScanMethod::Walk);
        assert_eq!(many.errors, 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_folder_reached_through_a_junction_is_not_listed() {
        let base = std::env::temp_dir().join(format!("sprawl-walk-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("real").join("inner")).unwrap();
        std::fs::write(base.join("real").join("inner").join("secret.txt"), b"x").unwrap();
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(base.join("link")).arg(base.join("real")).output().is_ok_and(|o| o.status.success());
        if !made { let _ = std::fs::remove_dir_all(&base); return; }
        let abs = std::path::absolute(&base).unwrap();
        let root_job = long_path(&abs);
        let root = dir::real_path(&dir::open(&root_job).unwrap()).unwrap();
        let job = |p: &Path| Job { id: 1, path: long_path(p), top: 0 };
        let mut buf = Vec::new();
        let mut names = Vec::new();
        let mut visit = |w: &[u16], _: u32, _: u32, _: u64, _: u64| { names.push(String::from_utf16_lossy(w)); true };
        // the folder where it really is: listed
        assert!(list_by_handle(&job(&abs.join("real").join("inner")), &root, root_job.len(), &mut buf, &mut visit));
        // the same folder through a junction above it (a queued folder's parent swapped for a
        // link, Astra's case): refused, nothing listed
        assert!(!list_by_handle(&job(&abs.join("link").join("inner")), &root, root_job.len(), &mut buf, &mut visit));
        // the junction itself in place of a queued folder: refused
        assert!(!list_by_handle(&job(&abs.join("link")), &root, root_job.len(), &mut buf, &mut visit));
        assert_eq!(names.iter().filter(|n| *n == "secret.txt").count(), 1, "{names:?}");
        // and an inventory lists the link as a link, never what is behind it
        let t = inventory(&base, &Progress::new()).unwrap();
        assert_eq!((t.nodes[ROOT].files, t.errors), (1, 0));
        let _ = std::fs::remove_dir(base.join("link"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn excluded_scopes_are_not_enumerated_or_counted() {
        let dir = temp_dir("excluded");
        fs::create_dir_all(dir.join("skip/deep")).unwrap();
        fs::create_dir_all(dir.join("skip-other")).unwrap();
        fs::write(dir.join("skip/deep/secret"), vec![0u8; 100]).unwrap();
        fs::write(dir.join("skip-other/kept"), vec![0u8; 7]).unwrap();
        let excluded = dir.join("skip");
        let p = Progress::with_exclusions(&[excluded.clone()]);
        let tree = crate::scan_auto(&dir, &p).unwrap();
        assert_eq!(tree.method, ScanMethod::Walk);
        assert_eq!(tree.nodes[ROOT].size, 7);
        assert_eq!(tree.nodes[ROOT].files, 1);
        assert!(!tree.nodes.iter().any(|n| n.name == "skip" || n.name == "secret"));
        assert_eq!(walk(&excluded, &p).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        let many = crate::scan_many(&[dir.clone(), dir.join("skip-other")], &Progress::with_exclusions(&[excluded])).unwrap();
        assert_eq!(many.nodes[ROOT].size, 14);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn walk_matches_std_scan() {
        let dir = temp_dir("agree");
        // A few levels, many files, some empty folders.
        for a in 0..5 {
            let da = dir.join(format!("dir{a}"));
            fs::create_dir_all(da.join("empty")).unwrap();
            for b in 0..4 {
                let db = da.join(format!("sub{b}"));
                fs::create_dir_all(&db).unwrap();
                for f in 0..7 {
                    fs::write(db.join(format!("f{f}.{}", ["txt", "bin", "dat"][f % 3])), vec![1u8; a * 1000 + b * 100 + f]).unwrap();
                }
            }
            fs::write(da.join("top.log"), vec![0u8; 12345]).unwrap();
        }
        fs::write(dir.join("root.txt"), b"hello").unwrap();

        let p = Progress::new();
        let fast = walk_with(&dir, &p, 3).unwrap();
        let slow = crate::scan_std(&dir, |_| {}).unwrap();
        assert_eq!(fast.nodes.len(), slow.nodes.len());
        let (fr, sr) = (&fast.nodes[ROOT], &slow.nodes[ROOT]);
        assert_eq!((fr.size, fr.files, fr.folders), (sr.size, sr.files, sr.folders));
        assert_eq!(fast.errors, 0);
        // Same paths with the same sizes and times. (A folder's time in its parent's listing
        // can lag behind the folder itself on NTFS, so only file times are compared.)
        let collect = |t: &Tree| {
            let mut v: Vec<(String, bool, u64, u64, Option<std::time::SystemTime>)> = (0..t.nodes.len())
                .map(|i| {
                    let n = &t.nodes[i];
                    (t.path_of(i).display().to_string(), n.is_dir, n.size, n.files, if n.is_dir { None } else { n.modified })
                })
                .collect();
            v.sort_by(|a, b| a.0.cmp(&b.0));
            v
        };
        assert_eq!(collect(&fast), collect(&slow));
        // Children have higher indices than parents.
        for (i, n) in fast.nodes.iter().enumerate() {
            for &c in &n.children {
                assert!(c > i);
                assert_eq!(fast.nodes[c].parent, Some(i));
            }
        }
        // Allocated is cluster-rounded and summed.
        let cluster = cluster_size(&dir).unwrap();
        let alloc: u64 = fast.nodes.iter().filter(|n| !n.is_dir).map(|n| round_up(n.size, cluster)).sum();
        assert_eq!(fr.allocated, alloc);
        // Progress agrees with the tree.
        let s = p.snapshot();
        assert_eq!((s.files, s.folders, s.bytes), (fr.files, fr.folders, fr.size));
        assert_eq!(s.method, ScanMethod::Walk);
        assert_eq!(s.top.len(), 6);
        assert_eq!(s.top.iter().map(|t| t.1).sum::<u64>(), fr.size);
        assert!(s.by_ext.iter().any(|e| e.0 == "log" && e.2 == 5));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn junctions_are_not_followed() {
        let dir = temp_dir("junction");
        let target = dir.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("big.bin"), vec![0u8; 5000]).unwrap();
        let link = dir.join("link");
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("mklink /J unavailable; skipping");
            return;
        }
        let tree = walk(&dir, &Progress::new()).unwrap();
        let root = &tree.nodes[ROOT];
        assert_eq!(root.size, 5000);
        assert_eq!(root.files, 1);
        let l = root.children.iter().map(|&c| &tree.nodes[c]).find(|n| n.name == "link").unwrap();
        assert!(l.is_reparse_point());
        assert!(l.children.is_empty());
        assert!(walk(&link, &Progress::new()).is_err(), "an explicit junction root must not be entered");
        assert!(crate::scan_auto(&link.join("big.bin"), &Progress::new()).is_err(), "a junction ancestor must not be followed either");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deep_paths_beyond_max_path() {
        let dir = temp_dir("deep");
        let mut p = long_path(&dir);
        let seg: Vec<u16> = "\\abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz".encode_utf16().collect();
        for _ in 0..6 {
            p.extend_from_slice(&seg);
        }
        let deep = std::path::PathBuf::from(String::from_utf16(&p).unwrap());
        assert!(fs::create_dir_all(&deep).is_ok());
        fs::write(deep.join("x.txt"), b"12345").unwrap();
        let tree = walk(&dir, &Progress::new()).unwrap();
        assert_eq!(tree.nodes[ROOT].size, 5);
        assert_eq!(tree.nodes[ROOT].folders, 6);
        assert_eq!(tree.errors, 0);
        let _ = fs::remove_dir_all(String::from_utf16(&long_path(&dir)).unwrap());
    }

    #[test]
    fn single_file_and_cancel() {
        let dir = temp_dir("single");
        let f = dir.join("one.txt");
        fs::write(&f, b"abc").unwrap();
        let t = walk(&f, &Progress::new()).unwrap();
        assert_eq!(t.nodes.len(), 1);
        assert_eq!(t.nodes[ROOT].size, 3);
        let p = Progress::new();
        p.cancel();
        assert_eq!(walk(&dir, &p).unwrap_err().kind(), io::ErrorKind::Interrupted);
        let _ = fs::remove_dir_all(&dir);
    }
}
