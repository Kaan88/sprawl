//! Scan history: snapshots saved after each scan, the retention policy, and projecting an old
//! snapshot onto the current tree (for the time-lapse and "what changed").
//!
//! Layout: `%LOCALAPPDATA%\Sprawl\snapshots\<key>\` per scanned root, holding `root.txt`
//! (the root path) and `<unix seconds>.<unique>.fo.dtsn` (folders only) and `.full.dtsn`.
//! New buckets use a SHA-256 root key; validated legacy buckets and filenames remain readable.
//!
//! Retention (see docs/plans/ai-advisor.md): every folders-only snapshot from the last 30
//! days, then the newest per week for 6 months, then the newest per month; the newest 3 full
//! snapshots per root; and a total cap that removes the oldest folders-only snapshots first.

use std::fs;
use std::io;
use std::io::{Read, Write};
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sprawl_core::{Tree, ROOT};
use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, GetFileInformationByHandleEx, FileIdInfo, BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO};

use crate::snapshot::{self, FILES_HERE};

pub const KEEP_FULL: usize = 3;
const DAY: u64 = 86_400;

pub fn base_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|a| PathBuf::from(a).join("Sprawl").join("snapshots"))
}

/// Collision-resistant identity of the root spelling, without probing that root on disk.
pub fn key(root: &Path) -> String {
    use sha2::{Digest, Sha256};
    let identity = root_id(root).unwrap_or_else(|| root.to_string_lossy().into_owned());
    format!("v2-{:x}", Sha256::digest(identity.as_bytes()))
}

fn root_id(root: &Path) -> Option<String> {
    let s = root.to_str()?.replace('/', "\\");
    let s = if let Some(rest) = s.strip_prefix(r"\\?\UNC\") { format!(r"\\{rest}") }
        else if let Some(rest) = s.strip_prefix(r"\\?\") { rest.to_string() } else { s };
    if s.len() > 128 << 10 || s.chars().any(char::is_control) { return None; }
    let b = s.as_bytes();
    let parts = if let Some(rest) = s.strip_prefix(r"\\") {
        if rest.split('\\').filter(|p| !p.is_empty()).count() < 2 { return None; }
        rest
    } else if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' { &s[3..] }
    else { return None; };
    if parts.split('\\').any(|p| matches!(p, "." | "..") || p.contains([':', '*', '?', '"', '<', '>', '|'])) { return None; }
    Some(s.trim_end_matches('\\').to_lowercase())
}

fn legacy_key(root: &Path) -> String {
    let s = root.to_string_lossy();
    let s = s.trim_end_matches(['\\', '/']);
    let k: String = s
        .chars()
        .map(|c| match c {
            '\\' | '/' => '~',
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    k.replace("_~", "~").trim_end_matches('_').to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub root: PathBuf,
    pub taken: SystemTime,
    pub file: PathBuf,
    pub full: bool,
    pub bytes: u64,
    base: PathBuf,
    identity: Identity,
    modified: Option<SystemTime>,
}

#[derive(Clone, Debug, PartialEq)]
enum Identity { Full(u64, [u8; 16]), Legacy(u32, u64) }

fn bad() -> io::Error { io::Error::other("History's path, identity or metadata changed") }

fn ensure_dir(path: &Path) -> io::Result<sprawl_core::local::Directory> {
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

fn text(path: &Path, max: u64) -> io::Result<String> {
    let mut file = sprawl_core::local::file(path, max)?;
    let mut text = String::new();
    file.file.read_to_string(&mut text)?;
    Ok(text)
}

fn bucket(path: &Path) -> io::Result<(sprawl_core::local::Directory, PathBuf)> {
    let held = sprawl_core::local::directory(path)?;
    let root = PathBuf::from(text(&held.path.join("root.txt"), 128 << 10)?);
    root_id(&root).ok_or_else(bad)?;
    let name = held.path.file_name().ok_or_else(bad)?.to_string_lossy();
    if name != key(&root) && name != legacy_key(&root) { return Err(bad()); }
    Ok((held, root))
}

fn nonce() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!("{:x}-{:x}-{:x}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed), SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos())
}

fn stamp(name: &str) -> Option<(SystemTime, bool)> {
    let (stem, full) = if let Some(s) = name.strip_suffix(".full.dtsn") { (s, true) } else { (name.strip_suffix(".fo.dtsn")?, false) };
    let (t, unique) = stem.split_once('.').map_or((stem, None), |(t, n)| (t, Some(n)));
    if unique.is_some_and(|n| n.is_empty() || n.len() > 128 || !n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) { return None; }
    Some((UNIX_EPOCH.checked_add(Duration::from_secs(t.parse().ok()?))?, full))
}

#[link(name = "kernel32")]
extern "system" {
    fn SetFileInformationByHandle(file: *mut core::ffi::c_void, class: i32, info: *const core::ffi::c_void, size: u32) -> i32;
}

fn file_id(file: &fs::File) -> io::Result<Identity> {
    let mut full: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandleEx(file.as_raw_handle().cast(), FileIdInfo, (&mut full as *mut FILE_ID_INFO).cast(), std::mem::size_of::<FILE_ID_INFO>() as u32) } != 0 {
        return Ok(Identity::Full(full.VolumeSerialNumber, full.FileId.Identifier));
    }
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 { return Err(io::Error::last_os_error()); }
    Ok(Identity::Legacy(info.dwVolumeSerialNumber, (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)))
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

fn delete(file: &fs::File) -> io::Result<()> {
    let flags = 3u32;
    if unsafe { SetFileInformationByHandle(file.as_raw_handle().cast(), 21, (&flags as *const u32).cast(), 4) } != 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

fn write_atomic(dir: &Path, name: &str, replace: bool, write: impl FnOnce(&mut fs::File) -> io::Result<()>) -> io::Result<()> {
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
        Err(io::Error::new(io::ErrorKind::WouldBlock, "History metadata is being read"))
    });
    if result.is_err() { let _ = delete(&file); }
    result
}

fn lock(dir: &Path) -> io::Result<fs::File> {
    use std::os::windows::fs::MetadataExt;
    for _ in 0..40 {
        match fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).share_mode(0)
            .custom_flags(0x0020_0000 | 0x0010_0000).open(dir.join(".save.lock")) {
            Ok(file) => {
                let m = file.metadata()?;
                if !m.is_file() || m.len() != 0 || m.file_attributes() & (0x400 | 0x1000 | 0x0004_0000 | 0x0040_0000) != 0 { return Err(bad()); }
                return Ok(file);
            }
            Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::WouldBlock, "History metadata is busy"))
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Bodies are encoded by the caller (on its thread); this compresses and writes them.
pub struct Pending {
    /// The scanned folder's size, kept beside the snapshots (the welcome screen shows it).
    pub size: u64,
    pub root: PathBuf,
    pub taken: SystemTime,
    pub folders: Vec<u8>,
    pub full: Option<Vec<u8>>,
}

/// Encode the snapshots for a finished scan (fast; the slow part is [`Pending::save`]).
pub fn prepare(tree: &Tree, taken: SystemTime, full: bool) -> Pending {
    let folders = snapshot::encode(&snapshot::folders_only(tree), taken);
    let full = full.then(|| snapshot::encode(tree, taken));
    Pending { size: tree.nodes[sprawl_core::ROOT].size, root: tree.root_path.clone(), taken, folders, full }
}

impl Pending {
    /// Write into `base` and apply the retention policy with a total cap.
    pub fn save(self, base: &Path, cap_bytes: u64) -> io::Result<()> {
        let identity = root_id(&self.root).ok_or_else(bad)?;
        let base = ensure_dir(base)?;
        let held = ensure_dir(&base.path.join(key(&self.root)))?;
        let dir = &held.path;
        {
            let _lock = lock(dir)?;
            match text(&dir.join("root.txt"), 128 << 10) {
                Ok(stored) if root_id(Path::new(&stored)).as_ref() == Some(&identity) => {},
                Ok(_) => return Err(bad()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => write_atomic(dir, "root.txt", false, |f| f.write_all(self.root.to_string_lossy().as_bytes()))?,
                Err(e) => return Err(e),
            }
        }
        let t = secs(self.taken);
        let unique = nonce();
        let write = |name: &str, body: &[u8]| -> io::Result<()> {
            write_atomic(dir, name, false, |file| {
                let mut buffered = io::BufWriter::new(file);
                snapshot::write_body(body, 9, &mut buffered)?;
                buffered.flush()?;
                if buffered.get_ref().metadata()?.len() > snapshot::MAX_FILE { return Err(io::Error::new(io::ErrorKind::InvalidData, "snapshot exceeds the supported file size")); }
                Ok(())
            })
        };
        write(&format!("{t}.{unique}.fo.dtsn"), &self.folders)?;
        if let Some(full) = &self.full {
            write(&format!("{t}.{unique}.full.dtsn"), full)?;
        }
        {
            let _lock = lock(dir)?;
            let previous = match text(&dir.join("last.txt"), 1024) {
                Ok(last) => last.split_whitespace().next().and_then(|t| t.parse::<u64>().ok()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(e),
            };
            if previous.is_none_or(|previous| t >= previous) {
                write_atomic(dir, "last.txt", true, |file| write!(file, "{t} {}", self.size))?;
            }
        }
        prune(&base.path, cap_bytes, SystemTime::now())
    }
}

/// Each scanned root's last scan: (root, when, the folder's size then), newest first. Roots
/// saved before the size was kept are left out.
pub fn last_scans(base: &Path) -> Vec<(PathBuf, SystemTime, u64)> {
    let mut out = Vec::new();
    let Ok(base) = sprawl_core::local::directory(base) else { return out; };
    let Ok(dirs) = fs::read_dir(&base.path) else { return out };
    for d in dirs.flatten() {
        let Ok((held, root)) = bucket(&d.path()) else { continue; };
        let Ok(last) = text(&held.path.join("last.txt"), 1024) else { continue; };
        let mut it = last.split_whitespace().map(|v| v.parse::<u64>());
        let (Some(Ok(t)), Some(Ok(size))) = (it.next(), it.next()) else { continue };
        let Some(when) = UNIX_EPOCH.checked_add(Duration::from_secs(t)) else { continue };
        out.push((root, when, size));
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// Every snapshot under `base`, oldest first.
pub fn list(base: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let Ok(base) = sprawl_core::local::directory(base) else { return out; };
    let Ok(dirs) = fs::read_dir(&base.path) else { return out };
    for d in dirs.flatten() {
        let Ok((held, root)) = bucket(&d.path()) else { continue; };
        let Ok(files) = fs::read_dir(&held.path) else { continue };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let Some((taken, full)) = stamp(&name) else { continue; };
            let Ok(file) = sprawl_core::local::file(&f.path(), snapshot::MAX_FILE) else { continue; };
            let (Ok(meta), Ok(identity)) = (file.file.metadata(), file_id(&file.file)) else { continue; };
            out.push(Entry {
                root: root.clone(),
                taken,
                file: f.path(),
                full,
                bytes: meta.len(),
                base: base.path.clone(), identity, modified: meta.modified().ok(),
            });
        }
    }
    out.sort_by(|a, b| a.taken.cmp(&b.taken).then(a.full.cmp(&b.full)));
    out
}

/// Snapshots that cover `root` (taken of it or of a folder above it), one per moment (the
/// full one when both exist), oldest first.
pub fn covering(base: &Path, root: &Path) -> Vec<Entry> {
    let want = crate::segments(&root.to_string_lossy());
    let mut v: Vec<Entry> = list(base)
        .into_iter()
        .filter(|e| {
            let r = crate::segments(&e.root.to_string_lossy());
            want.len() >= r.len() && want[..r.len()] == r[..]
        })
        .collect();
    // prefer the full snapshot of the same moment and root; then the closest root
    v.sort_by(|a, b| a.taken.cmp(&b.taken).then(b.full.cmp(&a.full)).then(b.root.as_os_str().len().cmp(&a.root.as_os_str().len())));
    v.dedup_by(|b, a| a.taken == b.taken);
    v
}

/// Which files the policy removes, given every entry and the time now.
pub fn doomed(entries: &[Entry], cap_bytes: u64, now: SystemTime) -> Vec<PathBuf> {
    let mut kill: Vec<PathBuf> = Vec::new();
    let mut roots: Vec<&PathBuf> = entries.iter().map(|e| &e.root).collect();
    roots.sort();
    roots.dedup();
    let now_s = secs(now);
    let mut kept_fo: Vec<&Entry> = Vec::new();
    for root in roots {
        let mine: Vec<&Entry> = entries.iter().filter(|e| &e.root == root).collect();
        // full: newest 3
        let mut full: Vec<&&Entry> = mine.iter().filter(|e| e.full).collect();
        full.sort_by(|a, b| b.taken.cmp(&a.taken));
        kill.extend(full.iter().skip(crate::tuning::get(&crate::tuning::KEEP_FULL) as usize).map(|e| e.file.clone()));
        // folders only: buckets by age
        let mut fo: Vec<&Entry> = mine.iter().copied().filter(|e| !e.full).collect();
        fo.sort_by(|a, b| b.taken.cmp(&a.taken)); // newest first
        let mut seen = std::collections::HashSet::new();
        for e in fo {
            let age = now_s.saturating_sub(secs(e.taken));
            let bucket = if age <= 30 * DAY {
                None
            } else if age <= 183 * DAY {
                Some(("w", secs(e.taken) / (7 * DAY)))
            } else {
                Some(("m", secs(e.taken) / (30 * DAY)))
            };
            match bucket {
                Some(b) if !seen.insert(b) => kill.push(e.file.clone()),
                _ => kept_fo.push(e),
            }
        }
    }
    // the cap: oldest folders-only first, but never a root's newest snapshot
    let mut total: u64 = entries.iter().filter(|e| !kill.contains(&e.file)).map(|e| e.bytes).sum();
    if total > cap_bytes {
        let newest: Vec<(&PathBuf, SystemTime)> = {
            let mut m: std::collections::HashMap<&PathBuf, SystemTime> = std::collections::HashMap::new();
            for e in &kept_fo {
                let t = m.entry(&e.root).or_insert(e.taken);
                *t = (*t).max(e.taken);
            }
            m.into_iter().collect()
        };
        kept_fo.sort_by(|a, b| a.taken.cmp(&b.taken));
        for e in kept_fo {
            if total <= cap_bytes {
                break;
            }
            if newest.iter().any(|(r, t)| *r == &e.root && *t == e.taken) {
                continue;
            }
            total = total.saturating_sub(e.bytes);
            kill.push(e.file.clone());
        }
    }
    kill
}

pub fn prune(base: &Path, cap_bytes: u64, now: SystemTime) -> io::Result<()> {
    let entries = list(base);
    let doomed = doomed(&entries, cap_bytes, now);
    for entry in entries.iter().filter(|entry| doomed.contains(&entry.file)) {
        let _ = remove_entry(entry);
    }
    Ok(())
}

pub fn load(e: &Entry) -> io::Result<Tree> {
    let (_bucket, path) = entry_path(e)?;
    let mut held = sprawl_core::local::file(&path, snapshot::MAX_FILE)?;
    check_entry(e, &held.file)?;
    let (tree, meta) = snapshot::read(&mut io::BufReader::new(&mut held.file))?;
    if root_id(&meta.root_path) != root_id(&e.root) || meta.taken != e.taken { return Err(bad()); }
    Ok(tree)
}

fn entry_path(e: &Entry) -> io::Result<(sprawl_core::local::Directory, PathBuf)> {
    let base = sprawl_core::local::directory(&e.base)?;
    let path = sprawl_core::local::lexical(&e.file)?;
    let parent = path.parent().ok_or_else(bad)?;
    if parent.parent() != Some(base.path.as_path()) { return Err(bad()); }
    let (held, root) = bucket(parent)?;
    if root_id(&root) != root_id(&e.root) || path.file_name().and_then(|n| n.to_str()).and_then(stamp) != Some((e.taken, e.full)) { return Err(bad()); }
    Ok((held, path))
}

fn check_entry(e: &Entry, file: &fs::File) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.file_attributes() & (0x400 | 0x1000 | 0x0004_0000 | 0x0040_0000) != 0
        || meta.len() != e.bytes || meta.modified().ok() != e.modified || file_id(file)? != e.identity { return Err(bad()); }
    Ok(())
}

/// Delete only this listed History file, after revalidating its pinned bucket and identity.
pub fn remove_entry(e: &Entry) -> io::Result<()> {
    let (_bucket, path) = entry_path(e)?;
    let file = fs::OpenOptions::new().access_mode(0x0001_0080).share_mode(1 | 2 | 4)
        .custom_flags(0x0020_0000 | 0x0010_0000).open(path)?;
    check_entry(e, &file)?;
    delete(&file)
}

/// The size every node of `cur` had in `old` (0 where it didn't exist), consistent so folders
/// are the sum of their children. Files a folders-only snapshot doesn't list individually get
/// a share of that folder's old `<files>` total in proportion to their size now.
pub fn project(cur: &Tree, old: &Tree) -> Vec<u64> {
    let mut w = vec![0u64; cur.nodes.len()];
    let Some(start) = crate::find_path(old, &cur.path_of(ROOT)) else { return w };
    let lower = |s: &str| s.to_lowercase();
    let mut stack = vec![(ROOT, start)];
    while let Some((c, o)) = stack.pop() {
        let cn = &cur.nodes[c];
        if !cn.is_dir {
            continue;
        }
        let on = &old.nodes[o];
        let mut by_name: std::collections::HashMap<String, usize> = std::collections::HashMap::with_capacity(on.children.len());
        let mut files_here = None;
        for &k in &on.children {
            if old.nodes[k].name == FILES_HERE && !old.nodes[k].is_dir {
                files_here = Some(old.nodes[k].size);
            } else {
                by_name.insert(lower(&old.nodes[k].name), k);
            }
        }
        let direct_now: u64 = cn.children.iter().filter(|&&k| !cur.nodes[k].is_dir).map(|&k| cur.nodes[k].size).sum();
        let mut shared = Vec::new();
        for &k in &cn.children {
            let kn = &cur.nodes[k];
            match by_name.get(&lower(&kn.name)) {
                Some(&ok) if old.nodes[ok].is_dir == kn.is_dir => {
                    if kn.is_dir {
                        stack.push((k, ok));
                    } else {
                        w[k] = old.nodes[ok].size;
                    }
                }
                _ if !kn.is_dir => shared.push(k),
                _ => {}
            }
        }
        // a folders-only snapshot: share the folder's old file total out by size now, exactly
        if let (Some(total), false) = (files_here, shared.is_empty()) {
            if direct_now > 0 {
                let mut given = 0u64;
                for &k in &shared {
                    w[k] = (total as u128 * cur.nodes[k].size as u128 / direct_now as u128) as u64;
                    given += w[k];
                }
                let big = *shared.iter().max_by_key(|&&k| cur.nodes[k].size).unwrap();
                w[big] += total - given;
            }
        }
    }
    // folders: sum of children (children come after parents)
    for i in (1..cur.nodes.len()).rev() {
        if cur.nodes[i].is_dir && cur.nodes[i].children.is_empty() {
            continue;
        }
        if let Some(p) = cur.nodes[i].parent {
            w[p] += w[i];
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> (PathBuf, PathBuf, Tree) {
        let dir = std::env::temp_dir().join(format!("sprawl-history-{}-{tag}", std::process::id()));
        let data = dir.join("data");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("a.bin"), b"data").unwrap();
        let tree = sprawl_core::scan(&data, |_| {}).unwrap();
        (dir.clone(), dir.join("history"), tree)
    }

    #[test]
    fn concurrent_same_second_saves_keep_every_snapshot() {
        let (dir, base, tree) = fixture("concurrent");
        let taken = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 60);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let mut jobs = Vec::new();
        for size in 1..=8 {
            let mut tree = tree.clone();
            let file = tree.nodes[ROOT].children[0];
            tree.nodes[file].size = size;
            tree.compute_totals();
            let pending = prepare(&tree, taken, false);
            let (barrier, base) = (barrier.clone(), base.clone());
            jobs.push(std::thread::spawn(move || { barrier.wait(); pending.save(&base, u64::MAX).unwrap(); }));
        }
        for job in jobs { job.join().unwrap(); }
        let entries = list(&base);
        assert_eq!(entries.len(), 8);
        let mut sizes: Vec<_> = entries.iter().map(|e| { assert_eq!(e.taken, taken); load(e).unwrap().nodes[ROOT].size }).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, (1..=8).collect::<Vec<_>>());
        assert_eq!(entries.iter().map(|e| e.file.clone()).collect::<std::collections::HashSet<_>>().len(), 8);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replaced_files_and_wrong_roots_are_refused() {
        let (dir, base, tree) = fixture("identity");
        let taken = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 60);
        prepare(&tree, taken, false).save(&base, u64::MAX).unwrap();
        let entry = list(&base).pop().unwrap();
        let preserved = entry.file.with_extension("keep");
        fs::rename(&entry.file, &preserved).unwrap();
        fs::copy(&preserved, &entry.file).unwrap();
        fs::File::options().write(true).open(&entry.file).unwrap().set_modified(entry.modified.unwrap()).unwrap();
        assert!(load(&entry).is_err() && remove_entry(&entry).is_err(), "identity matters even when bytes, size and time match");
        let current = list(&base).pop().unwrap();
        assert!(load(&current).is_ok());
        let root_file = current.file.parent().unwrap().join("root.txt");
        fs::write(&root_file, dir.join("another-root").display().to_string()).unwrap();
        assert!(list(&base).is_empty());
        assert!(load(&current).is_err() && remove_entry(&current).is_err());
        assert!(current.file.exists());
        fs::write(&root_file, tree.root_path.display().to_string()).unwrap();
        remove_entry(&current).unwrap();
        assert!(preserved.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_buckets_load_only_their_recorded_root() {
        let (dir, base, tree) = fixture("legacy");
        let bucket = base.join(legacy_key(&tree.root_path));
        fs::create_dir_all(&bucket).unwrap();
        fs::write(bucket.join("root.txt"), tree.root_path.display().to_string()).unwrap();
        let taken = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 60);
        let path = bucket.join(format!("{}.full.dtsn", secs(taken)));
        snapshot::write(&tree, taken, 3, &mut fs::File::create(&path).unwrap()).unwrap();
        assert!(load(&list(&base).pop().unwrap()).is_ok());
        let mut wrong = tree.clone();
        wrong.root_path = dir.join("another-root");
        snapshot::write(&wrong, taken, 3, &mut fs::File::create(&path).unwrap()).unwrap();
        assert!(load(&list(&base).pop().unwrap()).is_err(), "root.txt cannot relabel a different snapshot");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn redirected_buckets_and_base_never_escape_history() {
        let (dir, base, tree) = fixture("junction");
        let taken = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 60);
        prepare(&tree, taken, false).save(&base, u64::MAX).unwrap();
        let entry = list(&base).pop().unwrap();
        let original = entry.file.parent().unwrap();
        let outside = dir.join("outside-bucket");
        fs::rename(original, &outside).unwrap();
        let junction = |at: &Path, to: &Path| std::process::Command::new(crate::clean::system_dir().unwrap().join("cmd.exe"))
            .args(["/C", "mklink", "/J"]).arg(at).arg(to).output().unwrap().status.success();
        assert!(junction(original, &outside));
        assert!(list(&base).is_empty() && last_scans(&base).is_empty());
        assert!(load(&entry).is_err() && remove_entry(&entry).is_err());
        assert!(prepare(&tree, taken, false).save(&base, u64::MAX).is_err());
        prune(&base, 0, SystemTime::now()).unwrap();
        assert!(outside.join(entry.file.file_name().unwrap()).exists());
        fs::remove_dir(original).unwrap();
        let moved = dir.join("outside-base");
        fs::rename(&base, &moved).unwrap();
        assert!(junction(&base, &moved));
        assert!(list(&base).is_empty());
        assert!(prepare(&tree, taken, false).save(&base.join("must-not-create"), u64::MAX).is_err());
        assert!(!moved.join("must-not-create").exists());
        fs::remove_dir(&base).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_writes_only_remove_their_own_temporary_handle() {
        let (dir, base, _) = fixture("write-failure");
        let held = ensure_dir(&base).unwrap();
        fs::write(base.join("existing"), b"keep").unwrap();
        assert!(write_atomic(&base, "existing", false, |file| file.write_all(b"replacement")).is_err());
        assert_eq!(fs::read(base.join("existing")).unwrap(), b"keep");
        let moved = base.join("moved.tmp");
        let mut occupied = None;
        let result = write_atomic(&base, "never-published", false, |file| {
            file.write_all(b"partial output")?;
            let tmp = fs::read_dir(&base)?.filter_map(Result::ok).map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "tmp")).unwrap();
            fs::rename(&tmp, &moved)?;
            fs::write(&tmp, b"unrelated arrival")?;
            occupied = Some(tmp);
            Err(io::Error::other("simulated write failure"))
        });
        assert!(result.is_err());
        assert!(!moved.exists() && !base.join("never-published").exists());
        assert_eq!(fs::read(occupied.unwrap()).unwrap(), b"unrelated arrival");
        drop(held);
        fs::remove_dir_all(dir).unwrap();
    }

    fn e(root: &str, days_ago: u64, full: bool, bytes: u64, now: SystemTime) -> Entry {
        let t = now - Duration::from_secs(days_ago * DAY);
        Entry { root: root.into(), taken: t, file: PathBuf::from(format!("{root}-{days_ago}-{full}")), full, bytes, base: PathBuf::new(), identity: Identity::Legacy(0, 0), modified: None }
    }

    #[test]
    fn keys() {
        assert_eq!(legacy_key(Path::new(r"C:\")), "C");
        assert_eq!(legacy_key(Path::new(r"C:\Program Files")), "C~Program Files");
        assert_ne!(key(Path::new(r"C:\a~b")), key(Path::new(r"C:\a\b")));
        assert_eq!(key(Path::new(r"C:\Program Files\")), key(Path::new(r"c:/program files")));
        assert_eq!(key(Path::new(r"\\?\C:\Program Files")), key(Path::new(r"C:\Program Files")));
    }

    #[test]
    fn retention() {
        let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let mut v = Vec::new();
        for d in 0..400 {
            v.push(e("C", d, false, 5, now));
        }
        for d in 0..6 {
            v.push(e("C", d, true, 27, now));
        }
        let kill = doomed(&v, u64::MAX, now);
        let left: Vec<&Entry> = v.iter().filter(|x| !kill.contains(&x.file)).collect();
        assert_eq!(left.iter().filter(|x| x.full).count(), 3);
        assert!(left.iter().filter(|x| x.full).all(|x| x.taken >= now - Duration::from_secs(2 * DAY)));
        let fo = left.iter().filter(|x| !x.full).count();
        // 31 daily + about 22 weekly + about 7 monthly
        assert!((55..=65).contains(&fo), "{fo}");
        // a tight cap removes the oldest but keeps the newest
        let kill = doomed(&v, 100, now);
        let left: Vec<&Entry> = v.iter().filter(|x| !kill.contains(&x.file)).collect();
        assert!(left.iter().any(|x| !x.full && x.taken == now));
        assert!(left.iter().map(|x| x.bytes).sum::<u64>() <= 100 || left.iter().filter(|x| !x.full).count() == 1);
    }

    #[test]
    fn save_list_project() {
        let base = std::env::temp_dir().join(format!("dt-hist-{}", std::process::id()));
        let data = std::env::temp_dir().join(format!("dt-hist-data-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&data);
        fs::create_dir_all(data.join("sub")).unwrap();
        fs::write(data.join("sub").join("a.bin"), vec![1u8; 1000]).unwrap();
        fs::write(data.join("b.bin"), vec![1u8; 300]).unwrap();
        let old = sprawl_core::scan(&data, |_| {}).unwrap();
        // recent, so retention keeps both
        let t0 = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 3600);
        prepare(&old, t0, false).save(&base, u64::MAX).unwrap();

        fs::write(data.join("sub").join("a.bin"), vec![1u8; 4000]).unwrap();
        fs::write(data.join("new.bin"), vec![1u8; 50]).unwrap();
        let cur = sprawl_core::scan(&data, |_| {}).unwrap();
        prepare(&cur, t0 + Duration::from_secs(60), true).save(&base, u64::MAX).unwrap();

        let all = covering(&base, &data);
        assert_eq!(all.len(), 2);
        assert!(!all[0].full && all[1].full);
        let first = load(&all[0]).unwrap();
        let w = project(&cur, &first);
        let find = |t: &Tree, name: &str| t.nodes.iter().position(|n| n.name == name).unwrap();
        assert_eq!(w[find(&cur, "sub")], 1000);
        // b.bin and new.bin share the folder's old 300 bytes of direct files by size now
        assert_eq!(w[find(&cur, "b.bin")] + w[find(&cur, "new.bin")], 300);
        assert_eq!(w[ROOT], 1300);
        // a subfolder scan finds its part in the snapshot of the parent
        let sub = sprawl_core::scan(&data.join("sub"), |_| {}).unwrap();
        assert_eq!(covering(&base, &data.join("sub")).len(), 2);
        assert_eq!(project(&sub, &first)[ROOT], 1000);
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&data);
    }
}
