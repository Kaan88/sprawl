//! Fast folder walk that needs no admin rights.
//!
//! * `FindFirstFileExW(FindExInfoBasic, FIND_FIRST_EX_LARGE_FETCH)`: size, attributes and
//!   modified time come straight from the directory listing - no per-file metadata call.
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

/// [`walk`] with an explicit number of worker threads (for benchmarks).
pub fn walk_with(path: &Path, progress: &Progress, threads: usize) -> io::Result<Tree> {
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
        root.size = meta.len();
        root.allocated = round_up(root.size, cluster);
        root.files = 1;
        progress.add_counts(1, 0, root.size);
        progress.set_phase("Done");
        return Ok(Tree { root_path: path.to_path_buf(), nodes: vec![root], errors: 0, method: ScanMethod::Walk });
    }
    let errors_before = progress.errors();

    let shared = Shared {
        queue: Mutex::new(Queue { jobs: vec![Job { id: 0, path: long_path(&abs), top: NO_JOB }], pending: 1 }),
        cv: Condvar::new(),
        next_id: AtomicU32::new(1),
        progress,
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
        let entries = list_dir(sh, &job, &mut find, &mut pattern, &mut subjobs, &mut batch);
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
    subjobs: &mut Vec<Job>,
    batch: &mut LiveBatch,
) -> Vec<Entry> {
    let mut entries = Vec::new();
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
        if unsafe { GetLastError() } != ERROR_FILE_NOT_FOUND {
            sh.progress.add_errors(1);
        }
        return entries;
    }
    let is_root = job.top == NO_JOB;
    let mut top_names: Vec<(String, u64)> = Vec::new();
    let (mut files, mut folders, mut bytes) = (0u64, 0u64, 0u64);
    loop {
        let len = find.cFileName.iter().position(|&c| c == 0).unwrap_or(find.cFileName.len());
        let wname = &find.cFileName[..len];
        let dot = wname == [b'.' as u16] || wname == [b'.' as u16, b'.' as u16];
        if !dot {
            let name = String::from_utf16_lossy(wname);
            let attrs = find.dwFileAttributes;
            let size = (u64::from(find.nFileSizeHigh) << 32) | u64::from(find.nFileSizeLow);
            let modified = (u64::from(find.ftLastWriteTime.dwHighDateTime) << 32) | u64::from(find.ftLastWriteTime.dwLowDateTime);
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
        // SAFETY: `h` is a valid find handle.
        if unsafe { FindNextFileW(h, find) } == 0 {
            // SAFETY: trivially safe.
            if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
                sh.progress.add_errors(1);
            }
            break;
        }
    }
    // SAFETY: `h` is a valid find handle, closed once.
    unsafe { FindClose(h) };
    if is_root {
        sh.progress.top_reset(top_names);
    } else {
        batch.add_top(job.top as usize, bytes);
    }
    sh.progress.add_counts(files, folders, bytes);
    entries
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
        let dir = std::env::temp_dir().join(format!("disktree-walk-{tag}-{}", std::process::id()));
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
