//! Scan history: snapshots saved after each scan, the retention policy, and projecting an old
//! snapshot onto the current tree (for the time-lapse and "what changed").
//!
//! Layout: `%LOCALAPPDATA%\Sprawl\snapshots\<key>\` per scanned root, holding `root.txt`
//! (the root path) and `<unix seconds>.<unique>.fo.dtsn` (folders only), `.df.dtsn` (folders
//! only, stored as changes against the file it names) and `.full.dtsn` (the whole tree).
//! New buckets use a SHA-256 root key; validated legacy buckets and filenames remain readable.
//!
//! Every scan's folders-only snapshot is a delta against the previous one of the same root,
//! with a keyframe (a whole snapshot, `.fo.dtsn`) at least every eight snapshots and whenever
//! the delta would be more than half a whole one. Old whole snapshots (version 1 files) are
//! keyframes too. A chain is only ever read from its keyframe, so a missing or damaged file
//! costs the snapshots built on it, never the rest.
//!
//! Retention (see docs/plans/ai-advisor.md): every folders-only snapshot from the last 30
//! days, then the newest per week for 6 months, then the newest per month; the newest 3 full
//! snapshots per root; and a total cap that removes the oldest folders-only snapshots first,
//! never a file a kept delta is built on.
//!
//! A scan of a folder the History already shows, from a scan of a folder above it made within
//! the last day and with no later point of its own, is not saved at all: the slider can show
//! the drive's snapshot projected onto it.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::io::{Read, Write};
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sprawl_core::{Tree, ROOT};
use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, GetFileInformationByHandleEx, FileIdInfo, BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO};

use crate::snapshot::{self, FILES_HERE};
use crate::store::{ensure_dir, nonce, delete, write_atomic, lock};

pub const KEEP_FULL: usize = 3;
const DAY: u64 = 86_400;
/// A keyframe at least every eight snapshots of a root: at most seven deltas in a row, so a
/// lost file costs at most a short run of the slider.
pub const MAX_DELTAS: usize = 7;
/// How far a chain may be followed before it is treated as damaged (a hand-made loop).
const MAX_CHAIN: usize = 64;

pub fn base_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|a| PathBuf::from(a).join("Sprawl").join("snapshots"))
}

/// Collision-resistant identity of the root spelling, without probing that root on disk.
pub fn key(root: &Path) -> String {
    use sha2::{Digest, Sha256};
    let identity = root_id(root).unwrap_or_else(|| root.to_string_lossy().into_owned());
    format!("v3-{:x}", Sha256::digest(identity.as_bytes()))
}

fn folded_key(root: &Path) -> String {
    use sha2::{Digest, Sha256};
    format!("v2-{:x}", Sha256::digest(root_id(root).unwrap_or_default().to_lowercase().as_bytes()))
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
    let parts = parts.split('\\').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("\\");
    if s.starts_with(r"\\") { Some(format!(r"\\{parts}")) }
    else { Some(format!("{}:\\{parts}", b[0].to_ascii_uppercase() as char).trim_end_matches('\\').to_string()) }
}

/// Observation identity preserves component case. Live scans already supply canonical names.
pub fn covers(root: &Path, path: &Path) -> bool {
    root_id(root).zip(root_id(path)).is_some_and(|(root, path)| path == root || path.strip_prefix(&root).is_some_and(|tail| tail.starts_with('\\')))
}

pub fn find_path(tree: &Tree, path: &Path) -> Option<usize> {
    let wanted = root_id(path)?;
    let starts = if tree.nodes.first()?.name == sprawl_core::MULTIPLE { tree.nodes[ROOT].children.clone() } else { vec![ROOT] };
    for start in starts {
        let base = root_id(&tree.path_of(start))?;
        if wanted != base && !wanted.strip_prefix(&base).is_some_and(|tail| tail.starts_with('\\')) { continue; }
        let mut node = start;
        for part in wanted[base.len()..].split('\\').filter(|part| !part.is_empty()) {
            node = tree.nodes[node].children.iter().copied().find(|&child| tree.nodes[child].name == part)?;
        }
        return Some(node);
    }
    None
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
    /// A folders-only snapshot stored as changes against `delta_base`.
    pub delta: bool,
    pub bytes: u64,
    /// The file this delta continues, as its own header names it (never followed when missing).
    delta_base: Option<PathBuf>,
    base: PathBuf,
    identity: Identity,
    modified: Option<SystemTime>,
}

#[derive(Clone, Debug, PartialEq)]
enum Identity { Full(u64, [u8; 16]), Legacy(u32, u64) }

fn bad() -> io::Error { io::Error::other("History's path, identity or metadata changed") }

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
    if name != key(&root) && name != folded_key(&root) && name != legacy_key(&root) { return Err(bad()); }
    Ok((held, root))
}

fn stamp(name: &str) -> Option<(SystemTime, bool, bool)> {
    let (stem, full, delta) = if let Some(s) = name.strip_suffix(".full.dtsn") {
        (s, true, false)
    } else if let Some(s) = name.strip_suffix(".df.dtsn") {
        (s, false, true)
    } else if let Some(s) = name.strip_suffix(".fo.dtsn") {
        (s, false, false)
    } else {
        return None;
    };
    let (t, unique) = stem.split_once('.').map_or((stem, None), |(t, n)| (t, Some(n)));
    if unique.is_some_and(|n| n.is_empty() || n.len() > 128 || !n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) { return None; }
    Some((UNIX_EPOCH.checked_add(Duration::from_secs(t.parse().ok()?))?, full, delta))
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

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Bodies are encoded by the caller (on its thread); this compresses and writes them.
pub struct Pending {
    /// The scanned folder's size, kept beside the snapshots (the welcome screen shows it).
    pub size: u64,
    pub root: PathBuf,
    pub taken: SystemTime,
    /// The folders-only tree, kept so [`Pending::save`] can diff it against the previous
    /// snapshot on its thread.
    folders: Tree,
    /// Its whole-snapshot body: a keyframe, or the fallback when a delta is not worth it.
    folders_body: Vec<u8>,
    full: Option<Vec<u8>>,
}

/// Encode the snapshots for a finished scan (fast; the slow part is [`Pending::save`]).
pub fn prepare(tree: &Tree, taken: SystemTime, full: bool) -> Pending {
    let folders = snapshot::folders_only(tree);
    let folders_body = snapshot::encode_ordered(&folders, taken);
    let full = full.then(|| snapshot::encode(tree, taken));
    Pending { size: tree.nodes[sprawl_core::ROOT].size, root: tree.root_path.clone(), taken, folders, folders_body, full }
}

impl Pending {
    /// Write into `base` and apply the retention policy with a total cap.
    pub fn save(self, base: &Path, cap_bytes: u64) -> io::Result<()> {
        let identity = root_id(&self.root).ok_or_else(bad)?;
        // A scan of a folder a fresh scan above it already shows adds a duplicate point: see
        // [`covered_by_recent`]. Nothing is written, not even a bucket.
        if covered_by_recent(base, &self.root, self.taken) { return Ok(()); }
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
        let write = |name: &str, whole: bool, body: &[u8], base_name: Option<&str>| -> io::Result<()> {
            write_atomic(dir, name, false, |file| {
                let mut buffered = io::BufWriter::new(file);
                match base_name {
                    Some(b) => snapshot::write_delta_body(body, b, 9, &mut buffered)?,
                    None if whole => snapshot::write_body(body, 9, &mut buffered)?,
                    None => return Err(io::Error::new(io::ErrorKind::InvalidInput, "a snapshot is either whole or a delta")),
                }
                buffered.flush()?;
                if buffered.get_ref().metadata()?.len() > snapshot::MAX_FILE { return Err(io::Error::new(io::ErrorKind::InvalidData, "snapshot exceeds the supported file size")); }
                Ok(())
            })
        };
        if let Some(full) = &self.full {
            write(&format!("{t}.{unique}.full.dtsn"), true, full, None)?;
        }
        let entries = list(&base.path);
        match self.delta_for(&entries, dir) {
            Some((body, base_name)) => write(&format!("{t}.{unique}.df.dtsn"), false, &body, Some(&base_name))?,
            None => write(&format!("{t}.{unique}.fo.dtsn"), true, &self.folders_body, None)?,
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

    /// The delta that stands for this scan's folders-only tree, when the previous snapshot is
    /// readable and the chain short enough and the delta under half a whole one.
    fn delta_for(&self, entries: &[Entry], dir: &Path) -> Option<(Vec<u8>, String)> {
        let prev = previous(entries, dir, self.taken)?;
        if depth(entries, prev).is_none_or(|d| d >= MAX_DELTAS) { return None; }
        // reading the base also proves its own chain: a gap in it writes a keyframe
        let old = load(prev).ok()?;
        let body = snapshot::encode_delta(&old, &self.folders, self.taken)?;
        if body.len().saturating_mul(2) > self.folders_body.len() { return None; }
        Some((body, prev.file.file_name()?.to_str()?.to_string()))
    }
}

/// The snapshot of `dir` taken at or before `taken` (the one to build a delta on).
fn previous<'a>(entries: &'a [Entry], dir: &Path, taken: SystemTime) -> Option<&'a Entry> {
    entries.iter().filter(|e| !e.full && e.file.parent() == Some(dir) && e.taken <= taken).max_by(|a, b| (a.taken, &a.file).cmp(&(b.taken, &b.file)))
}

/// The snapshot a delta continues, by the file name its header stores: `None` when that file
/// is gone or belongs to another moment.
fn base_of<'a>(entries: &'a [Entry], e: &'a Entry) -> Option<&'a Entry> {
    let name = e.delta_base.as_ref()?.file_name()?;
    entries.iter().find(|x| !x.full && x.file.parent() == e.file.parent() && x.file.file_name() == Some(name) && x.taken <= e.taken)
}

/// How many deltas deep `e` is (0 for a keyframe); `None` when its chain is broken.
fn depth(entries: &[Entry], e: &Entry) -> Option<usize> {
    let (mut n, mut cur) = (0, e);
    while cur.delta {
        if n > MAX_CHAIN { return None; }
        cur = base_of(entries, cur)?;
        n += 1;
    }
    Some(n)
}

/// The files `e` is built from, oldest first: the keyframe and the deltas after it, each
/// named by the base its own header stores. A missing or misnamed base breaks the chain.
fn chain<'a>(entries: &'a [Entry], e: &'a Entry) -> Option<Vec<&'a Entry>> {
    let mut out = vec![e];
    let mut cur = e;
    while cur.delta {
        if out.len() > MAX_CHAIN { return None; }
        cur = base_of(entries, cur)?;
        out.push(cur);
    }
    out.reverse();
    Some(out)
}

/// True when a snapshot of a folder above `root` taken within the last day already shows it,
/// and `root` has no later snapshot of its own. Scanning the folder again would add a point the
/// slider can already show (the drive's snapshot, projected onto the folder), so nothing is
/// saved; once the covering snapshot is a day old, or the folder has a newer point of its own,
/// its history resumes.
fn covered_by_recent(base: &Path, root: &Path, taken: SystemTime) -> bool {
    let Some(me) = root_id(root) else { return false; };
    let scans = last_scans(base);
    let own = scans.iter().filter(|(r, _, _)| root_id(r).as_deref() == Some(me.as_str())).map(|(_, when, _)| *when).max();
    scans.iter().any(|(r, when, _)| {
        root_id(r).is_some_and(|id| id != me && covers(r, root))
            && taken.duration_since(*when).is_ok_and(|age| age <= Duration::from_secs(DAY))
            && own.is_none_or(|own| own <= *when)
    })
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
    // A root saved before 1.0 has an older folder beside its new one: name it once, newest.
    let mut named = std::collections::HashSet::new();
    out.retain(|(root, _, _)| named.insert(root_id(root).unwrap_or_else(|| root.display().to_string())));
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
            let Some((taken, full, delta)) = stamp(&name) else { continue; };
            let Ok(mut file) = sprawl_core::local::file(&f.path(), snapshot::MAX_FILE) else { continue; };
            let (Ok(meta), Ok(identity)) = (file.file.metadata(), file_id(&file.file)) else { continue; };
            // a delta names the file it continues in its own header; a damaged one names
            // nothing and is never shown (its followers are hidden with it)
            let delta_base = if delta {
                snapshot::read_header(&mut io::BufReader::new(&mut file.file)).ok()
                    .and_then(|(is_delta, base)| if is_delta { base } else { None })
                    .map(PathBuf::from)
            } else {
                None
            };
            out.push(Entry { root: root.clone(), taken, file: f.path(), full, delta, bytes: meta.len(), delta_base, base: base.path.clone(), identity, modified: meta.modified().ok() });
        }
    }
    out.sort_by(|a, b| a.taken.cmp(&b.taken).then_with(|| a.file.cmp(&b.file)));
    out
}

/// The entries of one bucket folder, as [`list`] makes them (used to follow a delta's chain
/// without reading every other root's files). `base` is the snapshots folder that holds it.
fn bucket_entries(base: &Path, dir: &Path) -> Vec<Entry> {
    let Ok((held, root)) = bucket(dir) else { return Vec::new(); };
    let Ok(files) = fs::read_dir(&held.path) else { return Vec::new(); };
    let mut out = Vec::new();
    for f in files.flatten() {
        let name = f.file_name().to_string_lossy().into_owned();
        let Some((taken, full, delta)) = stamp(&name) else { continue; };
        let Ok(mut file) = sprawl_core::local::file(&f.path(), snapshot::MAX_FILE) else { continue; };
        let (Ok(meta), Ok(identity)) = (file.file.metadata(), file_id(&file.file)) else { continue; };
        let delta_base = if delta {
            snapshot::read_header(&mut io::BufReader::new(&mut file.file)).ok()
                .and_then(|(is_delta, base)| if is_delta { base } else { None })
                .map(PathBuf::from)
        } else {
            None
        };
        out.push(Entry { root: root.clone(), taken, file: f.path(), full, delta, bytes: meta.len(), delta_base, base: base.to_path_buf(), identity, modified: meta.modified().ok() });
    }
    out.sort_by(|a, b| a.taken.cmp(&b.taken).then_with(|| a.file.cmp(&b.file)));
    out
}

/// Snapshots that cover `root` (taken of it or of a folder above it), one per moment (the
/// full one when both exist), oldest first. A delta whose chain is broken (a file lost in the
/// middle) is left out, so the slider never offers a snapshot that can't be read.
pub fn covering(base: &Path, root: &Path) -> Vec<Entry> {
    let all = list(base);
    let mut memo: HashMap<PathBuf, bool> = HashMap::new();
    let mut v: Vec<Entry> = all
        .iter()
        .filter(|e| covers(&e.root, root) && (!e.delta || chain_ok(&all, e, &mut memo)))
        .cloned()
        .collect();
    // prefer the full snapshot of the same moment and root; then the closest root
    v.sort_by(|a, b| a.taken.cmp(&b.taken).then(b.full.cmp(&a.full)).then(b.root.as_os_str().len().cmp(&a.root.as_os_str().len())));
    v.dedup_by(|b, a| a.taken == b.taken);
    v
}

fn chain_ok(all: &[Entry], e: &Entry, memo: &mut HashMap<PathBuf, bool>) -> bool {
    let mut seen = Vec::new();
    let mut cur = e;
    let ok = loop {
        if let Some(&ok) = memo.get(&cur.file) { break ok; }
        if seen.contains(&cur.file) || seen.len() > MAX_CHAIN { break false; }
        seen.push(cur.file.clone());
        if !cur.delta { break true; }
        match base_of(all, cur) {
            Some(b) => cur = b,
            None => break false,
        }
    };
    for file in seen { memo.insert(file, ok); }
    ok
}

/// Snapshot times have one-second precision. Exclude the scan's entire start second as
/// well as all later entries, even when the question is asked hours after the scan.
pub fn earlier(base: &Path, root: &Path, scan_before: SystemTime) -> Vec<Entry> {
    let Ok(age) = scan_before.duration_since(UNIX_EPOCH) else { return Vec::new(); };
    let Some(bound) = UNIX_EPOCH.checked_add(Duration::from_secs(age.as_secs())) else { return Vec::new(); };
    covering(base, root).into_iter().filter(|entry| entry.taken < bound).collect()
}

/// Which files the policy removes, given every entry and the time now.
pub fn doomed(entries: &[Entry], cap_bytes: u64, now: SystemTime) -> Vec<PathBuf> {
    let mut kill: Vec<PathBuf> = Vec::new();
    let mut roots: Vec<&PathBuf> = entries.iter().map(|e| &e.root).collect();
    roots.sort();
    roots.dedup();
    let now_s = secs(now);
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
                _ => {}
            }
        }
    }
    // The cap: folders-only snapshots first, then full ones, oldest first within each, but
    // never a root's newest snapshot. Full snapshots were never removed for the cap before
    // 1.0.2, so three per root could keep History well over it. Exactly one snapshot per root
    // is protected: a full and a folders-only snapshot saved together share a second, and the
    // full one is kept then (it holds everything the other does).
    let kept: Vec<&Entry> = entries.iter().filter(|e| !kill.contains(&e.file)).collect();
    let mut total: u64 = kept.iter().map(|e| e.bytes).sum();
    if total > cap_bytes {
        let mut newest: std::collections::HashMap<&PathBuf, &Entry> = std::collections::HashMap::new();
        for &e in &kept {
            let best = newest.entry(&e.root).or_insert(e);
            if (e.taken, e.full, &e.file) > (best.taken, best.full, &best.file) {
                *best = e;
            }
        }
        let mut order = kept;
        order.sort_by_key(|e| (e.full, e.taken));
        for e in order {
            if total <= cap_bytes {
                break;
            }
            if newest.get(&e.root).is_some_and(|best| best.file == e.file) {
                continue;
            }
            total = total.saturating_sub(e.bytes);
            kill.push(e.file.clone());
        }
    }
    // A delta is unreadable without the snapshot it continues, so a base the cap chose is kept
    // after all: the cap may then stay a little over, the same way one snapshot per root can.
    protect_bases(entries, &mut kill);
    // With bases kept, whole chains (a keyframe and the deltas built on it) are what the cap
    // can still remove: oldest first, never the chain holding a root's newest snapshot.
    thin_chains(entries, &mut kill, cap_bytes);
    kill
}

fn protect_bases(entries: &[Entry], kill: &mut Vec<PathBuf>) {
    let live: Vec<&Entry> = entries.iter().filter(|e| e.delta && !kill.contains(&e.file)).collect();
    for e in live {
        let mut cur = e;
        while let Some(b) = base_of(entries, cur) {
            kill.retain(|k| k != &b.file);
            cur = b;
            if !cur.delta { break; }
        }
    }
}

/// The keyframe a folders-only snapshot is built from (or the top of a broken chain, which is
/// one group until it is removed).
fn chain_root(entries: &[Entry], e: &Entry) -> PathBuf {
    let (mut cur, mut n) = (e, 0);
    while cur.delta {
        match base_of(entries, cur) {
            Some(b) if n < MAX_CHAIN => { cur = b; n += 1; }
            _ => break,
        }
    }
    cur.file.clone()
}

fn thin_chains(entries: &[Entry], kill: &mut Vec<PathBuf>, cap_bytes: u64) {
    let mut total: u64 = entries.iter().filter(|e| !kill.contains(&e.file)).map(|e| e.bytes).sum();
    if total <= cap_bytes { return; }
    // the one snapshot per root the cap never removes: a chain holding it stays whole
    let mut newest: HashMap<&PathBuf, (SystemTime, bool, &PathBuf)> = HashMap::new();
    for e in entries.iter().filter(|e| !kill.contains(&e.file)) {
        let cur = newest.entry(&e.root).or_insert((e.taken, e.full, &e.file));
        if (e.taken, e.full, &e.file) > *cur { *cur = (e.taken, e.full, &e.file); }
    }
    let mut groups: HashMap<PathBuf, Vec<&Entry>> = HashMap::new();
    for e in entries.iter().filter(|e| !e.full && !kill.contains(&e.file)) {
        groups.entry(chain_root(entries, e)).or_default().push(e);
    }
    let mut order: Vec<(SystemTime, PathBuf)> = groups.iter()
        .filter(|(_, g)| !g.iter().any(|e| newest.get(&e.root).is_some_and(|n| n.2 == &e.file)))
        .map(|(key, g)| (g.iter().map(|e| e.taken).max().unwrap_or(UNIX_EPOCH), key.clone()))
        .collect();
    order.sort();
    for (_, key) in order {
        if total <= cap_bytes { break; }
        let Some(g) = groups.get(&key) else { continue };
        // a chain the policy already partly removed stays as it is: its surviving delta needs
        // its base, which protection has just kept
        if g.iter().any(|e| kill.contains(&e.file)) { continue; }
        for e in g {
            kill.push(e.file.clone());
            total = total.saturating_sub(e.bytes);
        }
    }
}

pub fn prune(base: &Path, cap_bytes: u64, now: SystemTime) -> io::Result<()> {
    let entries = list(base);
    let doomed = doomed(&entries, cap_bytes, now);
    for entry in entries.iter().filter(|entry| doomed.contains(&entry.file)) {
        let _ = remove_entry(entry);
    }
    Ok(())
}

/// Rebuilt trees kept for the slider: a chain is rebuilt once and moving the slider back and
/// forth over it does not read it again. Bounded by node count (a node is about 120 bytes), so
/// a big drive keeps its newest point and little else.
#[derive(Default)]
struct Cache {
    nodes: usize,
    order: Vec<PathBuf>,
    kept: HashMap<PathBuf, Kept>,
}

struct Kept {
    identity: Identity,
    modified: Option<SystemTime>,
    bytes: u64,
    tree: Tree,
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
const CACHE_NODES: usize = 1_000_000;

/// The tree read from `e`'s file before, when that file is still the very same one: the same
/// bytes, time and identity, and the same root. A path never stands for another snapshot.
fn cached(e: &Entry) -> Option<Tree> {
    let c = CACHE.get()?.lock().ok()?;
    let kept = c.kept.get(&e.file)?;
    if kept.identity != e.identity || kept.modified != e.modified || kept.bytes != e.bytes { return None; }
    if root_id(&kept.tree.root_path) != root_id(&e.root) { return None; }
    Some(kept.tree.clone())
}

fn remember(e: &Entry, tree: &Tree) {
    if tree.nodes.len() > CACHE_NODES { return; }
    let cache = CACHE.get_or_init(|| Mutex::new(Cache::default()));
    let Ok(mut c) = cache.lock() else { return; };
    if c.kept.contains_key(&e.file) { return; }
    c.order.retain(|p| p != &e.file);
    c.order.push(e.file.clone());
    c.nodes += tree.nodes.len();
    c.kept.insert(e.file.clone(), Kept { identity: e.identity.clone(), modified: e.modified, bytes: e.bytes, tree: tree.clone() });
    while c.nodes > CACHE_NODES && c.order.len() > 1 {
        let old = c.order.remove(0);
        if let Some(kept) = c.kept.remove(&old) {
            c.nodes = c.nodes.saturating_sub(kept.tree.nodes.len());
        }
    }
}

pub fn load(e: &Entry) -> io::Result<Tree> {
    if !e.delta { return read_one(e, None); }
    let dir = e.file.parent().ok_or_else(bad)?;
    let entries = bucket_entries(&e.base, dir);
    let links = chain(&entries, e).ok_or_else(bad)?;
    let mut tree: Option<Tree> = None;
    for link in links {
        tree = Some(read_one(link, tree.as_ref())?);
    }
    tree.ok_or_else(bad)
}

/// Read one snapshot of a chain: a whole one by itself, a delta on top of its base.
fn read_one(e: &Entry, base: Option<&Tree>) -> io::Result<Tree> {
    let (_bucket, path) = entry_path(e)?;
    let mut held = sprawl_core::local::file(&path, snapshot::MAX_FILE)?;
    check_entry(e, &held.file)?;
    if let Some(tree) = cached(e) { return Ok(tree); }
    let opened = snapshot::open(&mut io::BufReader::new(&mut held.file))?;
    let tree = if opened.delta {
        if !e.delta { return Err(bad()); }
        let old = base.ok_or_else(bad)?;
        let name = opened.base.as_deref().ok_or_else(bad)?;
        if e.delta_base.as_deref().and_then(Path::file_name) != Some(std::ffi::OsStr::new(name)) { return Err(bad()); }
        let (tree, meta) = snapshot::decode_delta(old, &opened.body)?;
        if root_id(&meta.root_path) != root_id(&e.root) || meta.taken != e.taken { return Err(bad()); }
        tree
    } else {
        if e.delta { return Err(bad()); }
        let (tree, meta) = snapshot::decode(&opened.body)?;
        if root_id(&meta.root_path) != root_id(&e.root) || meta.taken != e.taken { return Err(bad()); }
        tree
    };
    remember(e, &tree);
    Ok(tree)
}

/// The indexes of `entries[i]` and every snapshot that would be unreadable without it: the
/// deltas whose chain runs through it. The app leaves them off the slider.
pub fn dependents(entries: &[Entry], i: usize) -> Vec<usize> {
    let mut out = vec![i];
    let Some(broken) = entries.get(i) else { return out; };
    for (j, e) in entries.iter().enumerate() {
        if j == i || !e.delta || e.file.parent() != broken.file.parent() { continue; }
        let (mut cur, mut n) = (e, 0);
        while let Some(b) = base_of(entries, cur) {
            if b.file == broken.file { out.push(j); break; }
            if n > MAX_CHAIN { break; }
            cur = b;
            n += 1;
        }
    }
    out
}

fn entry_path(e: &Entry) -> io::Result<(sprawl_core::local::Directory, PathBuf)> {
    let base = sprawl_core::local::directory(&e.base)?;
    let path = sprawl_core::local::lexical(&e.file)?;
    let parent = path.parent().ok_or_else(bad)?;
    if parent.parent() != Some(base.path.as_path()) { return Err(bad()); }
    let (held, root) = bucket(parent)?;
    if root_id(&root) != root_id(&e.root) { return Err(bad()); }
    let Some((taken, full, delta)) = path.file_name().and_then(|n| n.to_str()).and_then(stamp) else { return Err(bad()); };
    if taken != e.taken || full != e.full || delta != e.delta { return Err(bad()); }
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
    let Some(start) = find_path(old, &cur.path_of(ROOT)) else { return w };
    let mut stack = vec![(ROOT, start)];
    while let Some((c, o)) = stack.pop() {
        let cn = &cur.nodes[c];
        if !cn.is_dir {
            if !old.nodes[o].is_dir { w[c] = old.nodes[o].size; }
            continue;
        }
        let on = &old.nodes[o];
        let mut by_name: std::collections::HashMap<String, usize> = std::collections::HashMap::with_capacity(on.children.len());
        let mut files_here = None;
        for &k in &on.children {
            if old.nodes[k].name == FILES_HERE && !old.nodes[k].is_dir {
                files_here = Some(old.nodes[k].size);
            } else {
                by_name.insert(old.nodes[k].name.clone(), k);
            }
        }
        let direct_now: u64 = cn.children.iter().filter(|&&k| !cur.nodes[k].is_dir).map(|&k| cur.nodes[k].size).sum();
        let mut shared = Vec::new();
        for &k in &cn.children {
            let kn = &cur.nodes[k];
            match by_name.get(&kn.name) {
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
    fn comparisons_exclude_the_current_second_and_newer_scans() {
        let (dir, base, tree) = fixture("time-bound");
        let current = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 3600);
        for taken in [current - Duration::from_secs(60), current, current + Duration::from_secs(60)] {
            prepare(&tree, taken, false).save(&base, u64::MAX).unwrap();
        }
        let earlier = earlier(&base, &tree.root_path, current + Duration::from_millis(900));
        assert_eq!(earlier.len(), 1);
        assert_eq!(earlier[0].taken, current - Duration::from_secs(60));
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
    fn case_distinct_roots_and_single_file_snapshots_remain_separate() {
        let (dir, base, mut upper) = fixture("case-identity");
        upper.root_path = PathBuf::from(r"C:\CaseSensitive\Project");
        upper.nodes[ROOT].name = upper.root_path.display().to_string();
        let mut lower = upper.clone();
        lower.root_path = PathBuf::from(r"C:\CaseSensitive\project");
        lower.nodes[ROOT].name = lower.root_path.display().to_string();
        let taken = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 60);
        prepare(&upper, taken, true).save(&base, u64::MAX).unwrap();
        prepare(&lower, taken, true).save(&base, u64::MAX).unwrap();
        assert_ne!(key(&upper.root_path), key(&lower.root_path));
        for tree in [&upper, &lower] {
            let entries = covering(&base, &tree.root_path);
            assert_eq!(entries.len(), 1);
            assert_eq!(load(&entries[0]).unwrap().root_path, tree.root_path);
        }
        assert!(project(&upper, &lower).iter().all(|bytes| *bytes == 0));
        assert!(find_path(&upper, &upper.root_path.join("A.bin")).is_none(), "descendant names also preserve case");
        let legacy = base.join(folded_key(&upper.root_path));
        fs::create_dir(&legacy).unwrap();
        fs::write(legacy.join("root.txt"), upper.root_path.display().to_string()).unwrap();
        let legacy_file = legacy.join(format!("{}.full.dtsn", secs(taken)));
        snapshot::write(&lower, taken, 3, &mut fs::File::create(&legacy_file).unwrap()).unwrap();
        let entry = list(&base).into_iter().find(|entry| entry.file == legacy_file).unwrap();
        assert!(load(&entry).is_err(), "a legacy folded bucket cannot relabel a case-distinct snapshot");
        let file = Tree { volumes: Vec::new(), root_path: r"C:\Files\one.bin".into(), nodes: vec![sprawl_core::Node { name: r"C:\Files\one.bin".into(), size: 40, allocated: 4096, files: 1, ..Default::default() }], errors: 0, method: sprawl_core::ScanMethod::Walk };
        prepare(&file, taken, false).save(&base, u64::MAX).unwrap();
        let entry = covering(&base, &file.root_path).pop().unwrap();
        let back = load(&entry).unwrap();
        assert_eq!(back.nodes.len(), 1);
        assert!(!back.nodes[ROOT].is_dir);
        assert_eq!(project(&file, &back), vec![40]);
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
        Entry { root: root.into(), taken: t, file: PathBuf::from(format!("{root}-{days_ago}-{full}")), full, delta: false, bytes, delta_base: None, base: PathBuf::new(), identity: Identity::Legacy(0, 0), modified: None }
    }

    #[test]
    fn keys() {
        assert_eq!(legacy_key(Path::new(r"C:\")), "C");
        assert_eq!(legacy_key(Path::new(r"C:\Program Files")), "C~Program Files");
        assert_ne!(key(Path::new(r"C:\a~b")), key(Path::new(r"C:\a\b")));
        assert_eq!(key(Path::new(r"C:\Program Files\")), key(Path::new(r"c:/Program Files")));
        assert_ne!(key(Path::new(r"C:\Program Files")), key(Path::new(r"C:\program files")));
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
        // the newest is a full and a folders-only snapshot from the same second: the full one
        // is the one protected
        assert!(left.iter().any(|x| x.full && x.taken == now));
        assert!(left.iter().map(|x| x.bytes).sum::<u64>() <= 100);
        // Under the cap's pressure a tie keeps just one snapshot per root, and it is the full one.
        let tie = vec![e("C", 0, false, 50, now), e("C", 0, true, 80, now), e("D", 0, true, 80, now), e("D", 0, false, 50, now)];
        let kill = doomed(&tie, 160, now);
        let left: Vec<&Entry> = tie.iter().filter(|x| !kill.contains(&x.file)).collect();
        assert_eq!(left.len(), 2, "{left:?}");
        assert!(left.iter().all(|x| x.full));
        // the cap can stay exceeded when one snapshot per root is already over it
        let kill = doomed(&tie, 100, now);
        assert_eq!(tie.iter().filter(|x| !kill.contains(&x.file)).count(), 2);
        // Full snapshots count toward the cap too: many roots with three each must not stay over it.
        let mut many = Vec::new();
        for r in ["C", "D", "E", "F"] {
            for d in 1..=3 { many.push(e(r, d, true, 30, now)); }
        }
        let kill = doomed(&many, 200, now);
        let left: Vec<&Entry> = many.iter().filter(|x| !kill.contains(&x.file)).collect();
        assert!(left.iter().map(|x| x.bytes).sum::<u64>() <= 200, "full snapshots are thinned for the cap");
        for r in ["C", "D", "E", "F"] {
            assert!(left.iter().any(|x| x.root == Path::new(r) && x.taken == now - Duration::from_secs(DAY)), "each root keeps its newest");
        }
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

    // ------------------------------------------------------------ deltas

    /// A synthetic tree: `root` with one folder per entry, each holding its (name, size) files.
    fn build(root: &str, folders: &[(&str, &[(&str, u64)])]) -> Tree {
        let mut nodes = vec![sprawl_core::Node { name: root.into(), is_dir: true, ..Default::default() }];
        for (folder, files) in folders {
            let d = nodes.len();
            nodes.push(sprawl_core::Node { name: (*folder).into(), is_dir: true, parent: Some(0), ..Default::default() });
            nodes[0].children.push(d);
            for (name, size) in *files {
                let f = nodes.len();
                nodes.push(sprawl_core::Node { name: (*name).into(), size: *size, allocated: size.div_ceil(4096) * 4096, files: 1, parent: Some(d), modified: Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000 + *size)), ..Default::default() });
                nodes[d].children.push(f);
            }
        }
        let mut t = Tree { volumes: Vec::new(), root_path: root.into(), nodes, errors: 0, method: sprawl_core::ScanMethod::Walk };
        t.compute_totals();
        t
    }

    fn assert_same(a: &Tree, b: &Tree) {
        assert_eq!(a.root_path, b.root_path);
        assert_eq!(a.nodes.len(), b.nodes.len());
        for i in 0..a.nodes.len() {
            let at = a.path_of(i);
            assert_eq!(at, b.path_of(i), "node {i}");
            assert_eq!(a.nodes[i].is_dir, b.nodes[i].is_dir, "{}", at.display());
            assert_eq!(a.nodes[i].size, b.nodes[i].size, "{}", at.display());
            assert_eq!(a.nodes[i].allocated, b.nodes[i].allocated, "{}", at.display());
            assert_eq!(a.nodes[i].files, b.nodes[i].files, "{}", at.display());
            assert_eq!(a.nodes[i].folders, b.nodes[i].folders, "{}", at.display());
            assert_eq!(a.nodes[i].modified, b.nodes[i].modified, "{}", at.display());
            assert_eq!(a.nodes[i].attributes, b.nodes[i].attributes, "{}", at.display());
            assert_eq!(a.nodes[i].children, b.nodes[i].children, "{}", at.display());
        }
    }

    /// The tree of the i-th round: adds a folder, drops one, renames one, changes sizes.
    fn round(root: &str, i: u64) -> Tree {
        let mut folders: Vec<(String, Vec<(String, u64)>)> =
            (0..300).map(|f| (format!("folder{f}"), vec![("a".into(), 10 + f), ("b".into(), 20 + f)])).collect();
        for k in 0..3usize {
            folders[(k * 97 + i as usize * 13) % 300].1[0].1 += 1 + i;
        }
        match i % 5 {
            0 => folders.push((format!("new{i}"), vec![("f".into(), 40)])),
            1 => { folders.remove(7); }
            2 => folders[9].0 = format!("renamed{i}"),
            _ => {}
        }
        build_owned(root, &folders)
    }

    fn build_owned(root: &str, folders: &[(String, Vec<(String, u64)>)]) -> Tree {
        let files: Vec<(&str, Vec<(&str, u64)>)> = folders.iter().map(|(n, f)| (n.as_str(), f.iter().map(|(a, b)| (a.as_str(), *b)).collect())).collect();
        let view: Vec<(&str, &[(&str, u64)])> = files.iter().map(|(n, f)| (*n, f.as_slice())).collect();
        build(root, &view)
    }

    fn scratch(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sprawl-history-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let base = dir.join("history");
        (dir, base)
    }

    #[test]
    fn a_chain_of_deltas_rebuilds_every_snapshot_and_keeps_a_keyframe_every_eight() {
        let (dir, base) = scratch("chain");
        let root = dir.join("data").display().to_string();
        let t0 = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 20 * 3600);
        let trees: Vec<Tree> = (0..12).map(|i| round(&root, i)).collect();
        for (i, tree) in trees.iter().enumerate() {
            prepare(tree, t0 + Duration::from_secs(60 * i as u64), false).save(&base, u64::MAX).unwrap();
        }
        let entries = list(&base);
        assert_eq!(entries.len(), 12);
        let deltas = entries.iter().filter(|e| e.delta).count();
        assert!(deltas >= 8, "most snapshots are deltas, {deltas} were not");
        let mut run = 0;
        for e in &entries {
            if e.delta {
                run += 1;
                assert!(run <= MAX_DELTAS, "a keyframe at least every eight snapshots");
            } else {
                run = 0;
            }
        }
        for (i, e) in entries.iter().enumerate() {
            let back = load(e).unwrap();
            let taken = t0 + Duration::from_secs(60 * i as u64);
            let (whole, meta) = snapshot::decode(&snapshot::encode_ordered(&snapshot::folders_only(&trees[i]), taken)).unwrap();
            assert_eq!(meta.taken, taken);
            assert_same(&back, &whole);
            assert_eq!(e.taken, taken, "whole-second times");
        }
        // and the trees agree node for node in either direction (back to front too)
        for (i, e) in entries.iter().enumerate().rev() {
            assert_same(&load(e).unwrap(), &snapshot::decode(&snapshot::encode_ordered(&snapshot::folders_only(&trees[i]), e.taken)).unwrap().0);
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_delta_more_than_half_a_whole_one_is_written_as_a_keyframe() {
        let (dir, base) = scratch("half");
        let root = dir.join("data").display().to_string();
        let names: Vec<String> = (0..300).map(|i| format!("folder{i}")).collect();
        let renames: Vec<String> = (0..300).map(|i| format!("renamed-folder-{i}")).collect();
        let folders = |names: &[String]| -> Vec<(String, Vec<(&str, u64)>)> {
            names.iter().map(|n| (n.clone(), vec![("f", 500)])).collect()
        };
        let a = { let f = folders(&names); build(&root, &f.iter().map(|(n, s)| (n.as_str(), s.as_slice())).collect::<Vec<_>>()) };
        let b = { let f = folders(&renames); build(&root, &f.iter().map(|(n, s)| (n.as_str(), s.as_slice())).collect::<Vec<_>>()) };
        let t0 = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 3600);
        prepare(&a, t0, false).save(&base, u64::MAX).unwrap();
        prepare(&b, t0 + Duration::from_secs(60), false).save(&base, u64::MAX).unwrap();
        let entries = list(&base);
        assert_eq!(entries.len(), 2);
        assert!(!entries[1].delta, "every name changed: that is a keyframe");
        // the rule itself: this delta is over half a whole snapshot
        let delta = snapshot::encode_delta(&a, &b, t0).unwrap();
        let whole = snapshot::encode_ordered(&b, t0);
        assert!(delta.len() * 2 > whole.len(), "{} vs {}", delta.len(), whole.len());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_version_one_file_is_a_keyframe_a_delta_can_continue() {
        let (dir, base) = scratch("v1");
        let root = dir.join("data").display().to_string();
        let t0 = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 3600);
        let old = build(&root, &[("a", &[("x", 1)]), ("b", &[("y", 2)])]);
        // what 1.0.7 wrote: the magic, version 1 and the frame, no kind byte
        let held = ensure_dir(&base.join(key(&old.root_path))).unwrap();
        fs::write(held.path.join("root.txt"), old.root_path.display().to_string()).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"DTSN");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        {
            let mut z = zstd::stream::Encoder::new(&mut bytes, 3).unwrap();
            z.write_all(&snapshot::encode(&snapshot::folders_only(&old), t0)).unwrap();
            z.finish().unwrap();
        }
        fs::write(held.path.join(format!("{}.fo.dtsn", secs(t0))), bytes).unwrap();
        assert_eq!(load(&list(&base)[0]).unwrap().nodes[0].size, old.nodes[0].size);
        let next = build(&root, &[("a", &[("x", 9)]), ("b", &[("y", 2)])]);
        prepare(&next, t0 + Duration::from_secs(60), false).save(&base, u64::MAX).unwrap();
        let entries = list(&base);
        assert_eq!(entries.len(), 2);
        assert!(entries[1].delta, "the old file serves as the keyframe");
        let (whole, _) = snapshot::decode(&snapshot::encode_ordered(&snapshot::folders_only(&next), t0 + Duration::from_secs(60))).unwrap();
        assert_same(&load(&entries[1]).unwrap(), &whole);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_lost_or_damaged_file_hides_only_what_is_built_on_it() {
        let (dir, base) = scratch("lost");
        let root = dir.join("data").display().to_string();
        let t0 = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 20 * 3600);
        let trees: Vec<Tree> = (0..10).map(|i| round(&root, i)).collect();
        for (i, tree) in trees.iter().enumerate() {
            prepare(tree, t0 + Duration::from_secs(60 * i as u64), false).save(&base, u64::MAX).unwrap();
        }
        let all = list(&base);
        let middle: Vec<Entry> = all.iter().filter(|e| e.delta).cloned().collect();
        assert!(middle.len() >= 5);
        // one file lost from the middle of the chain
        fs::remove_file(&middle[1].file).unwrap();
        let visible = covering(&base, Path::new(&root));
        assert!(visible.len() < all.len(), "the snapshots built on it are not offered");
        assert!(!visible.iter().any(|e| e.file == middle[1].file));
        for e in &visible {
            assert!(load(e).is_ok(), "everything else still loads: {}", e.file.display());
        }
        // with the newest snapshot unreadable, the next save cannot build on it: a keyframe
        let last = all.last().unwrap().clone();
        let mut bytes = fs::read(&last.file).unwrap();
        bytes.truncate(bytes.len() - 8);
        fs::write(&last.file, &bytes).unwrap();
        prepare(&trees[0], t0 + Duration::from_secs(600), false).save(&base, u64::MAX).unwrap();
        let newest = list(&base).pop().unwrap();
        assert!(!newest.delta, "a gap in the chain is closed with a keyframe");
        assert!(load(&newest).is_ok());

        // damage inside a file rather than its loss: the reader refuses it, and the app drops
        // it and the deltas built on it from the slider (see `dependents`)
        let (dir2, base2) = scratch("damaged");
        for (i, tree) in trees.iter().enumerate() {
            prepare(tree, t0 + Duration::from_secs(60 * i as u64), false).save(&base2, u64::MAX).unwrap();
        }
        let all = list(&base2);
        let deltas: Vec<usize> = (0..all.len()).filter(|&i| all[i].delta).collect();
        let broken = deltas[1];
        let mut bytes = fs::read(&all[broken].file).unwrap();
        bytes.truncate(bytes.len() - 8);
        fs::write(&all[broken].file, &bytes).unwrap();
        // the entry's own listing still matches (same file), so the app reads it and is refused
        let e = list(&base2).into_iter().find(|e| e.file == all[broken].file).unwrap();
        assert!(load(&e).is_err());
        let doomed = dependents(&all, broken);
        assert!(doomed.contains(&broken) && doomed.len() > 1, "the deltas built on it go too: {doomed:?}");
        // with the newest snapshot unreadable, the next save cannot build on it: a keyframe
        let last = all.last().unwrap().clone();
        assert!(last.delta);
        let mut bytes = fs::read(&last.file).unwrap();
        bytes.truncate(bytes.len() - 8);
        fs::write(&last.file, &bytes).unwrap();
        let last_now = list(&base2).into_iter().find(|e| e.file == last.file).unwrap();
        assert!(load(&last_now).is_err());
        prepare(&trees[0], t0 + Duration::from_secs(600), false).save(&base2, u64::MAX).unwrap();
        let newest = list(&base2).pop().unwrap();
        assert!(!newest.delta, "the previous snapshot can't be read: a keyframe is written");
        assert!(load(&newest).is_ok());
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(dir2);
    }

    fn chain_entry(root: &str, at: SystemTime, name: &str, base: Option<&str>, bytes: u64) -> Entry {
        Entry { root: root.into(), taken: at, file: PathBuf::from(name), full: false, delta: base.is_some(), bytes, delta_base: base.map(PathBuf::from), base: PathBuf::new(), identity: Identity::Legacy(0, 0), modified: None }
    }

    #[test]
    fn the_cap_never_leaves_a_kept_delta_without_its_base() {
        let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let mut v = vec![
            chain_entry("C", now - Duration::from_secs(60 * DAY), "k1", None, 10),
            chain_entry("C", now - Duration::from_secs(59 * DAY), "d1", Some("k1"), 10),
            chain_entry("C", now - Duration::from_secs(2 * DAY), "k2", None, 10),
            chain_entry("C", now - Duration::from_secs(DAY), "d2", Some("k2"), 10),
        ];
        v.sort_by_key(|e| e.taken);
        let kill = doomed(&v, 35, now);
        let kept: Vec<&Entry> = v.iter().filter(|e| !kill.contains(&e.file)).collect();
        assert!(kept.iter().any(|e| e.file == Path::new("d2")), "a root's newest is never removed");
        for e in &kept {
            if let Some(name) = &e.delta_base {
                assert!(kept.iter().any(|b| b.file.file_name() == name.file_name()), "{} keeps its base", e.file.display());
            }
        }
        assert!(kept.iter().all(|e| e.file != Path::new("k1") && e.file != Path::new("d1")), "the old chain went whole: {kept:?}");
        // the newest chain cannot shrink: its base is kept even when that alone is over the cap
        let tight = doomed(&v, 5, now);
        let kept: Vec<&Entry> = v.iter().filter(|e| !tight.contains(&e.file)).collect();
        assert!(kept.iter().any(|e| e.file == Path::new("k2")), "the newest delta's base stays");
        assert!(kept.iter().any(|e| e.file == Path::new("d2")));
    }

    #[test]
    fn a_subfolder_already_shown_by_a_fresh_scan_above_it_is_not_saved() {
        let (dir, base) = scratch("covered");
        let parent = dir.display().to_string();
        let sub = dir.join("data").display().to_string();
        let t0 = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 3600);
        prepare(&build(&parent, &[("data", &[("x", 1)])]), t0, false).save(&base, u64::MAX).unwrap();
        let count_sub = |base: &Path| list(base).iter().filter(|e| root_id(&e.root) == root_id(Path::new(&sub))).count();
        prepare(&build(&sub, &[("x", &[("f", 1)])]), t0 + Duration::from_secs(60), false).save(&base, u64::MAX).unwrap();
        assert_eq!(count_sub(&base), 0, "the drive's snapshot already shows it");
        // a day later the drive's snapshot is old news: the folder's own history starts
        prepare(&build(&sub, &[("x", &[("f", 2)])]), t0 + Duration::from_secs(25 * 3600), false).save(&base, u64::MAX).unwrap();
        assert_eq!(count_sub(&base), 1);
        // ...and once it has a later point of its own, later scans are saved again
        prepare(&build(&sub, &[("x", &[("f", 3)])]), t0 + Duration::from_secs(26 * 3600), false).save(&base, u64::MAX).unwrap();
        assert_eq!(count_sub(&base), 2);
        let _ = fs::remove_dir_all(dir);
    }

    /// F7's number: a chain of ten snapshots against ten whole ones, on a synthetic tree of
    /// 50,000 folders-only nodes where about 1% changes between snapshots. Run with
    /// `--nocapture` to see the sizes.
    #[test]
    fn ten_snapshots_with_deltas_take_a_fraction_of_ten_whole_ones() {
        let (dir, base) = scratch("size");
        let root = dir.join("drive").display().to_string();
        let mut cur = big_tree(&root, 250, 100, 4);
        assert!(snapshot::folders_only(&cur).nodes.len() >= 50_000);
        let t0 = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 20 * 3600);
        let mut whole = 0u64;
        for i in 0..10 {
            let taken = t0 + Duration::from_secs(60 * i);
            prepare(&cur, taken, false).save(&base, u64::MAX).unwrap();
            // one whole snapshot of the same tree, for comparison
            let body = snapshot::encode_ordered(&snapshot::folders_only(&cur), taken);
            let mut file = Vec::new();
            snapshot::write_body(&body, 9, &mut file).unwrap();
            whole += file.len() as u64;
            if i < 9 {
                churn(&mut cur, i as usize);
            }
        }
        let entries = list(&base);
        let chain: u64 = entries.iter().map(|e| e.bytes).sum();
        let deltas: Vec<&Entry> = entries.iter().filter(|e| e.delta).collect();
        let average = deltas.iter().map(|e| e.bytes).sum::<u64>() / deltas.len().max(1) as u64;
        eprintln!(
            "history size: ten snapshots as a delta chain {chain} bytes ({} keyframes, {} deltas averaging {average} bytes), ten whole ones {whole} bytes ({:.1}x)",
            entries.len() - deltas.len(),
            deltas.len(),
            whole as f64 / chain as f64
        );
        assert!(chain * 3 < whole, "a day of scans is many times smaller: {chain} against {whole}");
        assert!(entries.iter().filter(|e| e.delta).count() >= 8);
        assert_same(&load(&entries[9]).unwrap(), &snapshot::decode(&snapshot::encode_ordered(&snapshot::folders_only(&cur), entries[9].taken)).unwrap().0);
        let _ = fs::remove_dir_all(dir);
    }

    /// `dirs` folders of `subs` folders of `files` files each: about 50,000 nodes folders-only.
    /// Names carry a spread-out tag, as real folder names do, so the whole snapshot's name
    /// table is the bulk of it (a synthetic tree of `d0`, `s0` would compress unrealistically).
    fn big_tree(root: &str, dirs: usize, subs: usize, files: usize) -> Tree {
        let mut nodes = vec![sprawl_core::Node { name: root.into(), is_dir: true, ..Default::default() }];
        for d in 0..dirs {
            let di = nodes.len();
            nodes.push(sprawl_core::Node { name: format!("d{d}-{}", tag(d as u64)), is_dir: true, parent: Some(0), ..Default::default() });
            nodes[0].children.push(di);
            for s in 0..subs {
                let si = nodes.len();
                nodes.push(sprawl_core::Node { name: format!("s{s}-{}", tag((d * 1000 + s) as u64)), is_dir: true, parent: Some(di), ..Default::default() });
                nodes[di].children.push(si);
                for f in 0..files {
                    let fi = nodes.len();
                    let size = 100 + (d * 31 + s * 7 + f * 13) as u64 % 900;
                    nodes.push(sprawl_core::Node { name: format!("f{f}"), size, allocated: size.div_ceil(4096) * 4096, files: 1, parent: Some(si), modified: Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000 + f as u64)), ..Default::default() });
                    nodes[si].children.push(fi);
                }
            }
        }
        let mut t = Tree { volumes: Vec::new(), root_path: root.into(), nodes, errors: 0, method: sprawl_core::ScanMethod::Walk };
        t.compute_totals();
        t
    }

    fn tag(seed: u64) -> String {
        format!("{:08x}{:04x}", seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(17), (seed.wrapping_mul(0x85eb_ca6b) >> 40) as u16)
    }

    /// About 1% of the tree differs from the last snapshot: one file in fifty changed, and a
    /// folder in a few hundred renamed.
    fn churn(t: &mut Tree, round: usize) {
        let mut renamed = 0;
        for (i, n) in t.nodes.iter_mut().enumerate() {
            if !n.is_dir && i % 50 == 0 {
                n.size += 37;
                n.allocated = n.size.div_ceil(4096) * 4096;
                n.modified = Some(UNIX_EPOCH + Duration::from_secs(1_750_000_000 + round as u64));
            }
            if n.is_dir && n.parent.is_some() && n.parent != Some(0) && renamed < 10 && i % 20_000 == 0 {
                n.name = format!("moved-{round}-{i}");
                renamed += 1;
            }
        }
        t.compute_totals();
    }
}
