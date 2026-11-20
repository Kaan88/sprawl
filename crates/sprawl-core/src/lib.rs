//! sprawl-core: the parts of a WizTree-style disk analyser that don't need a UI.
//!
//! * [`scan_auto`] reads the NTFS master file table directly when it can ([`mft`], needs admin)
//!   and otherwise lists folders with a multi-threaded `FindFirstFileExW` walk ([`walk`]).
//!   Either way it reports live [`Progress`] that a UI can poll while the scan runs.
//! * [`scan_many`] scans several paths under one synthetic root ("Select Multiple").
//! * [`csv`] writes and reads WizTree-compatible CSV files; [`filter`] does WizTree-style
//!   `*.mp4|*.mkv` filters.
//! * Every folder gets its total size, allocated size, item, file and folder counts.
//! * `extension_stats` groups files by extension; `largest_files` lists the biggest files.
//! * `squarify` and `treemap` lay sizes out as rectangles (the squarified treemap).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub mod compact;
pub mod csv;
pub mod filter;
pub mod kinds;
pub mod mft;
pub mod progress;
pub mod local;
pub mod query;
pub mod identity;
pub mod search;
pub mod walk;
pub mod wof;

pub use csv::{export_csv, load_csv};
pub use filter::{filter_tree, matches, Filter};
pub use progress::{Progress, ProgressSnapshot, ScanMethod};

/// `FILE_ATTRIBUTE_*` bits as found in [`Node::attributes`].
pub mod attr {
    pub const READONLY: u32 = 0x1;
    pub const HIDDEN: u32 = 0x2;
    pub const SYSTEM: u32 = 0x4;
    pub const DIRECTORY: u32 = 0x10;
    pub const ARCHIVE: u32 = 0x20;
    pub const NORMAL: u32 = 0x80;
    pub const TEMPORARY: u32 = 0x100;
    pub const SPARSE_FILE: u32 = 0x200;
    pub const REPARSE_POINT: u32 = 0x400;
    pub const COMPRESSED: u32 = 0x800;
    /// Sprawl's scan-only marker for IO_REPARSE_TAG_WOF; not a Windows attribute.
    pub const WOF: u32 = 0x8000_0000;
    pub const OFFLINE: u32 = 0x1000;
    pub const NOT_CONTENT_INDEXED: u32 = 0x2000;
    pub const ENCRYPTED: u32 = 0x4000;
}

/// One file or folder.
///
/// Memory: a full system drive is ~3.4M nodes, so a node is kept to plain fields (120 bytes
/// plus the name's heap allocation). `children` is empty (no heap) for files and shrunk to fit for folders.
/// Index types stay `usize` so `tree.nodes[i]` indexing reads naturally.
#[derive(Debug, Clone, Default)]
pub struct Node {
    pub name: String,
    pub is_dir: bool,
    /// Bytes: the file's length, or the total of everything below a folder.
    pub size: u64,
    /// Bytes on disk. From the MFT: the real allocation (0 for small files stored inside the MFT
    /// record, the compressed size for compressed/sparse files). From a folder walk: the length
    /// rounded up to the volume's cluster size. Folders: the total below.
    pub allocated: u64,
    /// Files below a folder (1 for a file).
    pub files: u64,
    /// Folders below a folder, not counting itself.
    pub folders: u64,
    pub modified: Option<SystemTime>,
    /// `FILE_ATTRIBUTE_*` bits (see [`attr`]): hidden, system, reparse point, compressed, ...
    pub attributes: u32,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Which file this is on its volume, as Windows gives it for an open handle (the NTFS file
    /// reference, `nFileIndexHigh:Low`): what was seen at this path, so a clean-up can tell
    /// the same item from another one put in its place. 0 when not known (a share, FAT, a
    /// detailed walk, a tree from History or a CSV).
    pub id: u64,
}

impl Node {
    /// Files plus folders below this one.
    pub fn items(&self) -> u64 {
        self.files + self.folders
    }

    /// Lower-case extension without the dot, or "(none)".
    pub fn extension(&self) -> String {
        extension_of(&self.name)
    }

    pub fn is_hidden(&self) -> bool {
        self.attributes & attr::HIDDEN != 0
    }

    /// Filesystem compression evidence, including WOF (compact /exe), never inferred from size.
    pub fn is_compressed(&self) -> bool { self.attributes & (attr::COMPRESSED | attr::WOF) != 0 }

    pub fn is_system(&self) -> bool {
        self.attributes & attr::SYSTEM != 0
    }

    /// Junctions, symbolic links, mount points, cloud placeholders...: counted as themselves,
    /// never followed.
    pub fn is_reparse_point(&self) -> bool {
        self.attributes & attr::REPARSE_POINT != 0
    }
}

/// A scanned folder. Children always have higher indices than their parent.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub root_path: PathBuf,
    pub nodes: Vec<Node>,
    /// Entries that couldn't be read (access denied and similar).
    pub errors: u64,
    /// How the tree was produced.
    pub method: ScanMethod,
    /// The volume serial of each real scan root (its path as scanned), read when it was
    /// scanned: node ids are only meaningful on that volume. Empty when not known (a tree
    /// read from a CSV file or a saved snapshot).
    pub volumes: Vec<(PathBuf, u64)>,
}

/// The serial in [`Tree::volumes`] that stands for "not known here": an entry with it hides the
/// serial of a root further out (a branch rescanned without one, or overlapping roots that
/// disagree).
pub const UNKNOWN_VOLUME: u64 = 0;

/// A path's parts for comparing scan roots: whole names, lower case, no separators.
fn root_parts(p: &Path) -> Vec<String> {
    p.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).filter(|c| c != "\\").collect()
}

impl Tree {
    /// The volume serial `p` was scanned on: that of the deepest scan root holding it (names
    /// compared whole, ignoring case), or None when not known.
    pub fn volume_of(&self, p: &Path) -> Option<u64> {
        let want = root_parts(p);
        self.volumes.iter()
            .map(|(r, v)| (root_parts(r), *v))
            .filter(|(r, _)| want.starts_with(r))
            .max_by_key(|(r, _)| r.len())
            .map(|(_, v)| v)
            .filter(|&v| v != UNKNOWN_VOLUME)
    }

    /// The branch at `at` was replaced by a fresh scan of it whose roots were on `incoming`:
    /// the entries at or under `at` go and the fresh scan's come in. When those don't cover
    /// `at` itself, `at` is marked unknown, so a root further out doesn't lend its serial to
    /// the fresh ids. Other branches keep theirs.
    pub fn replace_volumes(&mut self, at: &Path, incoming: &[(PathBuf, u64)]) {
        let base = root_parts(at);
        self.volumes.retain(|(r, _)| !root_parts(r).starts_with(&base));
        let fresh: Vec<(PathBuf, u64)> = incoming.iter().filter(|(r, _)| root_parts(r).starts_with(&base)).cloned().collect();
        if !fresh.iter().any(|(r, _)| root_parts(r) == base) {
            self.volumes.push((at.to_path_buf(), UNKNOWN_VOLUME));
        }
        self.volumes.extend(fresh);
    }
}

/// Overlapping roots that were on different volumes: the inner one (or both, for the same
/// path) becomes unknown, since a node below it may come from either scan.
fn settle_overlaps(volumes: &mut [(PathBuf, u64)]) {
    let parts: Vec<Vec<String>> = volumes.iter().map(|(r, _)| root_parts(r)).collect();
    let clash: Vec<bool> = (0..volumes.len())
        .map(|i| (0..volumes.len()).any(|j| j != i && parts[i].starts_with(&parts[j]) && volumes[j].1 != UNKNOWN_VOLUME && volumes[j].1 != volumes[i].1))
        .collect();
    for (v, c) in volumes.iter_mut().zip(clash) {
        if c {
            v.1 = UNKNOWN_VOLUME;
        }
    }
}

/// dev: with `SPRAWL_TIMING` set, one line per call in `%TEMP%\sprawl-timing.log`: what took
/// how long since `since`. Off (one env lookup, cached) otherwise.
pub fn timing(what: &str, since: std::time::Instant) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    // never as administrator: TEMP and the switch are the user's, so an elevated write there
    // could be sent anywhere (Fable core review P2-1)
    if !*ON.get_or_init(|| std::env::var_os("SPRAWL_TIMING").is_some() && elevation().ok() == Some(false)) {
        return;
    }
    use std::io::Write;
    let ms = since.elapsed().as_secs_f64() * 1000.0;
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("sprawl-timing.log")) {
        let _ = writeln!(f, "{what}: {ms:.1} ms");
    }
}

/// The volume serial of the item at `path` (not following a link), read now (see
/// [`local::volume_id`]).
pub fn volume_serial(path: &Path) -> Option<u64> {
    use std::os::windows::fs::OpenOptionsExt;
    // FILE_READ_ATTRIBUTES; FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
    let f = fs::OpenOptions::new().access_mode(0x80).share_mode(7).custom_flags(0x0200_0000 | 0x0020_0000).open(path).ok()?;
    local::volume_id(&f)
}

pub const ROOT: usize = 0;

/// Name of the synthetic root made by [`scan_many`] (and by CSV files holding several roots).
pub const MULTIPLE: &str = "Multiple";

pub fn extension_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => "(none)".to_string(),
    }
}

// -------------------------------------------------------------------- scanning entry points

/// The one spelling of a scan root that paths, the never-touch list and deletes all agree on:
/// absolute (a relative root would resolve against whatever folder a later step runs in),
/// without the `\\?\` prefix, and with 8.3 short names (`PROGRA~1`) expanded to long ones.
pub fn canonical_root(path: &Path) -> PathBuf {
    let s = root_spelling(path);
    long_name(&s).unwrap_or(s).into()
}

/// [`canonical_root`] without the long-name lookup (which may reach a server).
pub(crate) fn root_spelling(path: &Path) -> String {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let s = abs.display().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\").or_else(|| s.strip_prefix(r"\\.\")).filter(|r| {
        // only `\\?\C:\...`: a `\\?\Volume{guid}\` stays as it is (stripped it would be relative)
        let b = r.as_bytes();
        b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
    }) {
        rest.to_string()
    } else {
        s
    }
}

#[cfg(windows)]
fn long_name(s: &str) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::GetLongPathNameW;
    let w: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
    let mut buf = vec![0u16; 32_768];
    let n = unsafe { GetLongPathNameW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| String::from_utf16_lossy(&buf[..n]))
}

#[cfg(not(windows))]
fn long_name(_: &str) -> Option<String> {
    None
}

/// Scan `path` (see [`scan_auto`]) and call `progress` with the number of items seen so far,
/// about every 30 ms. Kept for callers that only need a count; new code should use
/// [`scan_auto`] with a shared [`Progress`].
pub fn scan(path: &Path, mut progress: impl FnMut(u64)) -> io::Result<Tree> {
    let p = Progress::new();
    std::thread::scope(|s| {
        let job = s.spawn(|| scan_auto(path, &p));
        while !job.is_finished() {
            progress(p.items());
            std::thread::sleep(Duration::from_millis(30));
        }
        let res = job.join().unwrap_or_else(|_| Err(io::Error::other("scan thread panicked")));
        progress(p.items());
        res
    })
}

/// Scan `path` the fastest way available: the NTFS MFT when the process is elevated and the
/// path is on an NTFS volume, otherwise the multi-threaded folder walk. The method used is in
/// `progress.snapshot().method` and in [`Tree::method`]. Cancelling via [`Progress::cancel`]
/// returns `ErrorKind::Interrupted`.
/// Folder inventories retain listing compression attributes/tags, but do not open every
/// plain file to discover hidden WOF backing. `walk::walk` provides that detailed check.
pub fn scan_auto(path: &Path, progress: &Progress) -> io::Result<Tree> {
    let t0 = std::time::Instant::now();
    let root = local::scan_root(path, false)?;
    timing("scan: pin the root", t0);
    let path = &root.path;
    if root.pinned.is_some() && is_elevated() {
        let t = std::time::Instant::now();
        match mft::scan(path, progress) {
            Ok(tree) => { timing("scan: MFT scan, all of it", t); progress.note_root(path, root.on_network()); return Ok(tree) }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => return Err(e),
            Err(e) => {
                timing(&format!("scan: MFT scan FAILED ({e}), walking instead"), t);
                progress.reset_counts() // not NTFS, not a folder, no access: walk instead
            }
        }
    }
    let t = std::time::Instant::now();
    let r = walk::inventory(path, progress);
    timing("scan: walk", t);
    r
}

/// "Select Multiple": scan each path and put them under one synthetic root named
/// [`MULTIPLE`] (`root_path` is also "Multiple"). The children's names are their full paths, so
/// [`Tree::path_of`] still works. Paths on the same NTFS volume share one MFT read when
/// elevated. During the scan, `snapshot().top` lists the paths themselves. A single path is
/// just [`scan_auto`].
pub fn scan_many(paths: &[PathBuf], progress: &Progress) -> io::Result<Tree> {
    scan_many_mode(paths, progress, true)
}

/// Inventory only the requested folders, without reading a whole volume's MFT for small
/// Cleaner scopes. Uses the same exclusions, pinned paths and cancellation as a normal scan.
pub fn scan_many_inventory(paths: &[PathBuf], progress: &Progress) -> io::Result<Tree> {
    scan_many_mode(paths, progress, false)
}

fn scan_many_mode(paths: &[PathBuf], progress: &Progress, use_mft: bool) -> io::Result<Tree> {
    if paths.len() == 1 {
        return if use_mft { scan_auto(&paths[0], progress) } else { walk::inventory(&paths[0], progress) };
    }
    let pinned: Vec<_> = paths.iter().map(|p| local::scan_root(p, false)).collect::<io::Result<_>>()?;
    let abs: Vec<PathBuf> = pinned.iter().map(|p| p.path.clone()).collect();
    // every root checked before any name is shown or anything is read
    if abs.iter().any(|p| progress.excludes(p)) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "This folder is excluded in Settings > Scanning"));
    }
    progress.pin_top(Some(abs.iter().map(|p| p.display().to_string()).collect()), None);
    let mut trees: Vec<Option<Tree>> = (0..abs.len()).map(|_| None).collect();
    if use_mft && is_elevated() {
        // Group by volume so each MFT is read once.
        let mut groups: Vec<(PathBuf, Vec<usize>)> = Vec::new();
        for (i, p) in abs.iter().enumerate() {
            // only pinned local roots: a share is listed, never asked for its volume or MFT
            if pinned[i].pinned.is_none() { continue; }
            if let Some(vol) = mft::volume_of(p) {
                match groups.iter_mut().find(|g| g.0 == vol) {
                    Some(g) => g.1.push(i),
                    None => groups.push((vol, vec![i])),
                }
            }
        }
        for (_, idx) in groups {
            let group: Vec<PathBuf> = idx.iter().map(|&i| abs[i].clone()).collect();
            if idx.len() == 1 {
                progress.pin_top(None, Some(idx[0]));
            } else {
                progress.pin_top(None, Some(usize::MAX)); // several roots at once: no live top
            }
            match mft::scan_multi(&group, progress) {
                Ok(ts) => {
                    for (&i, t) in idx.iter().zip(ts) {
                        progress.note_root(&abs[i], pinned[i].on_network());
                        trees[i] = Some(t);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => return Err(e),
                Err(_) => {}
            }
            // The MFT reader counts whole volumes; show what was kept so far instead.
            let kept = trees.iter().flatten().map(|t| &t.nodes[ROOT]);
            let (f, d, b) = kept.fold((0, 0, 0), |a, r| (a.0 + r.files, a.1 + r.folders, a.2 + r.size));
            progress.set_counts(f, d, b);
        }
    }
    let (mut files, mut folders, mut bytes, mut errors) = (0u64, 0u64, 0u64, 0u64);
    for t in trees.iter().flatten() {
        let r = &t.nodes[ROOT];
        files += r.files;
        folders += r.folders;
        bytes += r.size;
        errors += t.errors;
    }
    for (i, slot) in trees.iter_mut().enumerate() {
        if slot.is_none() {
            // Walk the rest; counts keep adding up from the MFT results.
            progress.set_counts(files, folders, bytes);
            progress.pin_top(None, Some(i));
            let t = walk::inventory(&abs[i], progress)?;
            let r = &t.nodes[ROOT];
            files += r.files;
            folders += r.folders;
            bytes += r.size;
            errors += t.errors;
            *slot = Some(t);
        }
    }
    progress.pin_top(None, None);
    let trees: Vec<Tree> = trees.into_iter().flatten().collect();
    let mut out = combine(trees, abs.iter().map(|p| p.display().to_string()).collect());
    out.errors = errors;
    let r = &out.nodes[ROOT];
    progress.set_counts(r.files, r.folders, r.size);
    progress.top_reset(top_of(&out));
    progress.set_phase("Done");
    Ok(out)
}

/// Put `trees` under a synthetic [`MULTIPLE`] root; `names` become the children's names.
pub(crate) fn combine(trees: Vec<Tree>, names: Vec<String>) -> Tree {
    let method = if trees.iter().all(|t| t.method == ScanMethod::Mft) { ScanMethod::Mft } else { ScanMethod::Walk };
    let mut volumes: Vec<(PathBuf, u64)> = trees.iter().flat_map(|t| t.volumes.iter().cloned()).collect();
    settle_overlaps(&mut volumes);
    let total: usize = 1 + trees.iter().map(|t| t.nodes.len()).sum::<usize>();
    let mut nodes: Vec<Node> = Vec::with_capacity(total);
    nodes.push(Node { name: MULTIPLE.to_string(), is_dir: true, attributes: attr::DIRECTORY, ..Default::default() });
    for (t, name) in trees.into_iter().zip(names) {
        let off = nodes.len();
        nodes[ROOT].children.push(off);
        for (i, mut n) in t.nodes.into_iter().enumerate() {
            n.parent = Some(n.parent.map_or(ROOT, |p| p + off));
            for c in &mut n.children {
                *c += off;
            }
            if i == ROOT {
                n.name = name.clone();
            }
            nodes.push(n);
        }
    }
    let mut tree = Tree { volumes, root_path: PathBuf::from(MULTIPLE), nodes, errors: 0, method };
    tree.compute_totals();
    tree
}

/// The root's direct children with their sizes, for [`ProgressSnapshot::top`].
pub(crate) fn top_of(tree: &Tree) -> Vec<(String, u64)> {
    match tree.nodes.first() {
        Some(r) => r.children.iter().map(|&c| (tree.nodes[c].name.clone(), tree.nodes[c].size)).collect(),
        None => Vec::new(),
    }
}

/// True when the process runs elevated (as administrator), which reading the MFT needs.
pub fn is_elevated() -> bool {
    elevation().unwrap_or(false)
}

/// Checked elevation state. Callers guarding per-user writes must refuse an unknown state.
pub fn elevation() -> io::Result<bool> {
    // SAFETY: the pseudo-handle of this process is always valid.
    unsafe { identity::token_elevated(windows_sys::Win32::System::Threading::GetCurrentProcess()) }
}

/// The original single-threaded `std::fs` walk (one metadata call per entry). Slow; kept as a
/// reference for tests and benchmarks.
pub fn scan_std(path: &Path, mut progress: impl FnMut(u64)) -> io::Result<Tree> {
    use std::os::windows::fs::MetadataExt;
    let meta = fs::symlink_metadata(path)?;
    let mut tree = Tree { volumes: Vec::new(), root_path: path.to_path_buf(), nodes: Vec::new(), errors: 0, method: ScanMethod::Walk };
    tree.nodes.push(Node {
        name: path.display().to_string(),
        is_dir: meta.is_dir(),
        size: if meta.is_dir() { 0 } else { meta.len() },
        files: if meta.is_dir() { 0 } else { 1 },
        modified: meta.modified().ok(),
        attributes: meta.file_attributes(),
        ..Default::default()
    });

    let mut stack: Vec<(usize, PathBuf)> = Vec::new();
    if meta.is_dir() {
        stack.push((ROOT, path.to_path_buf()));
    }
    let mut seen: u64 = 0;
    while let Some((dir_index, dir_path)) = stack.pop() {
        let entries = match fs::read_dir(&dir_path) {
            Ok(e) => e,
            Err(_) => {
                tree.errors += 1;
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    tree.errors += 1;
                    continue;
                }
            };
            // symlink_metadata: links and junctions are counted as themselves, never followed.
            let m = match fs::symlink_metadata(entry.path()) {
                Ok(m) => m,
                Err(_) => {
                    tree.errors += 1;
                    continue;
                }
            };
            let is_dir = m.is_dir();
            let index = tree.nodes.len();
            tree.nodes.push(Node {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir,
                size: if is_dir { 0 } else { m.len() },
                files: if is_dir { 0 } else { 1 },
                modified: m.modified().ok(),
                attributes: m.file_attributes(),
                parent: Some(dir_index),
                ..Default::default()
            });
            tree.nodes[dir_index].children.push(index);
            if is_dir {
                stack.push((index, entry.path()));
            }
            seen += 1;
            if seen.is_multiple_of(1000) {
                progress(seen);
            }
        }
    }
    tree.compute_totals();
    progress(seen);
    Ok(tree)
}

// -------------------------------------------------------------------- times

/// FILETIME ticks (100 ns since 1601) to `SystemTime`; 0 means unknown.
pub(crate) fn filetime_to_system(ft: u64) -> Option<SystemTime> {
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    if ft == 0 {
        return None;
    }
    if ft >= EPOCH_DIFF {
        let d = ft - EPOCH_DIFF;
        // a damaged or foreign timestamp must not panic the scan
        UNIX_EPOCH.checked_add(Duration::new(d / 10_000_000, (d % 10_000_000) as u32 * 100))
    } else {
        let d = EPOCH_DIFF - ft;
        UNIX_EPOCH.checked_sub(Duration::new(d / 10_000_000, (d % 10_000_000) as u32 * 100))
    }
}

/// `SystemTime` to FILETIME ticks.
pub(crate) fn system_to_filetime(t: SystemTime) -> u64 {
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => EPOCH_DIFF + d.as_secs() * 10_000_000 + u64::from(d.subsec_nanos() / 100),
        Err(e) => {
            let d = e.duration();
            EPOCH_DIFF.saturating_sub(d.as_secs() * 10_000_000 + u64::from(d.subsec_nanos() / 100))
        }
    }
}

// -------------------------------------------------------------------- totals

impl Tree {
    /// Add every node's size, allocated size and counts into its parent (children come after
    /// parents), then sort every folder's children biggest first.
    ///
    /// Folders without children keep their own values (a CSV import of a folders-only export
    /// carries sizes on folder rows; scanned empty folders are 0 anyway).
    pub fn compute_totals(&mut self) {
        for n in self.nodes.iter_mut().filter(|n| n.is_dir && !n.children.is_empty()) {
            n.size = 0;
            n.allocated = 0;
            n.files = 0;
            n.folders = 0;
        }
        for i in (1..self.nodes.len()).rev() {
            let (size, allocated, files, folders, is_dir) = {
                let n = &self.nodes[i];
                (n.size, n.allocated, n.files, n.folders, n.is_dir)
            };
            if let Some(p) = self.nodes[i].parent {
                let parent = &mut self.nodes[p];
                parent.size = parent.size.saturating_add(size);
                parent.allocated = parent.allocated.saturating_add(allocated);
                parent.files = parent.files.saturating_add(files);
                parent.folders = parent.folders.saturating_add(folders + u64::from(is_dir));
            }
        }
        for i in 0..self.nodes.len() {
            if self.nodes[i].children.is_empty() {
                continue;
            }
            let mut c = std::mem::take(&mut self.nodes[i].children);
            let nodes = &self.nodes;
            c.sort_by(|a, b| nodes[*b].size.cmp(&nodes[*a].size).then_with(|| nodes[*a].name.cmp(&nodes[*b].name)));
            c.shrink_to_fit();
            self.nodes[i].children = c;
        }
    }

    /// Share of the parent's size, 0..=100 (100 for the root).
    pub fn percent_of_parent(&self, index: usize) -> f64 {
        match self.nodes[index].parent {
            None => 100.0,
            Some(p) if self.nodes[p].size == 0 => 0.0,
            Some(p) => self.nodes[index].size as f64 * 100.0 / self.nodes[p].size as f64,
        }
    }

    /// Full path of a node.
    pub fn path_of(&self, index: usize) -> PathBuf {
        let mut parts = Vec::new();
        let mut i = index;
        while let Some(p) = self.nodes[i].parent {
            parts.push(self.nodes[i].name.as_str());
            i = p;
        }
        let mut path = self.root_path.clone();
        for part in parts.iter().rev() {
            path.push(part);
        }
        path
    }

    /// A copy of the part of the tree below `index` (which becomes the root), with
    /// `root_path` set to that node's path.
    pub fn subtree(&self, index: usize) -> Tree {
        let root_path = self.path_of(index);
        let mut nodes: Vec<Node> = Vec::new();
        let mut queue = std::collections::VecDeque::from([(index, None::<usize>)]);
        while let Some((old, parent)) = queue.pop_front() {
            let n = &self.nodes[old];
            let new = nodes.len();
            nodes.push(Node { parent, children: Vec::with_capacity(n.children.len()), ..n.clone() });
            if let Some(p) = parent {
                nodes[p].children.push(new);
            }
            for &c in &n.children {
                queue.push_back((c, Some(new)));
            }
        }
        nodes[ROOT].name = root_path.display().to_string();
        Tree { volumes: self.volumes.clone(), root_path, nodes, errors: self.errors, method: self.method }
    }
}

/// Totals for one file extension.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionStat {
    pub extension: String,
    pub size: u64,
    pub files: u64,
    /// Share of all file bytes, 0..=100.
    pub percent: f64,
}

/// Every extension, biggest total first.
pub fn extension_stats(tree: &Tree) -> Vec<ExtensionStat> {
    let mut map: HashMap<String, (u64, u64)> = HashMap::new();
    let mut total: u64 = 0;
    for n in tree.nodes.iter().filter(|n| !n.is_dir) {
        let e = map.entry(n.extension()).or_insert((0, 0));
        e.0 += n.size;
        e.1 += 1;
        total += n.size;
    }
    let mut v: Vec<ExtensionStat> = map
        .into_iter()
        .map(|(extension, (size, files))| ExtensionStat {
            extension,
            size,
            files,
            percent: if total == 0 { 0.0 } else { size as f64 * 100.0 / total as f64 },
        })
        .collect();
    v.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.extension.cmp(&b.extension)));
    v
}

/// Indices of the `limit` biggest files, biggest first.
pub fn largest_files(tree: &Tree, limit: usize) -> Vec<usize> {
    let mut v: Vec<usize> = (0..tree.nodes.len()).filter(|&i| !tree.nodes[i].is_dir).collect();
    v.sort_by(|a, b| tree.nodes[*b].size.cmp(&tree.nodes[*a].size).then_with(|| a.cmp(b)));
    v.truncate(limit);
    v
}

/// A colour for an extension, stable across runs (hue from a hash, like WizTree's per-type colours).
pub fn extension_colour(extension: &str) -> (u8, u8, u8) {
    // FNV-1a
    let mut h: u32 = 0x811c_9dc5;
    for b in extension.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    let hue = (h % 360) as f64;
    hsv_to_rgb(hue, 0.55, 0.85)
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |f: f64| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to(r), to(g), to(b))
}

/// Human size: "1.5 GB".
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["bytes", "KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.1} {}", UNITS[u])
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }
    pub fn area(&self) -> f64 {
        self.w * self.h
    }
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

fn worst(row: &[f64], side: f64) -> f64 {
    let s: f64 = row.iter().sum();
    if s <= 0.0 || side <= 0.0 {
        return f64::INFINITY;
    }
    let max = row.iter().cloned().fold(f64::MIN, f64::max);
    let min = row.iter().cloned().fold(f64::MAX, f64::min);
    let s2 = s * s;
    let side2 = side * side;
    f64::max(side2 * max / s2, s2 / (side2 * min))
}

/// Squarified treemap (Bruls, Huizing, van Wijk). Returns one rectangle per value, in input order;
/// values of zero or less get an empty rectangle.
pub fn squarify(values: &[f64], bounds: Rect) -> Vec<Rect> {
    let mut out = vec![Rect::new(bounds.x, bounds.y, 0.0, 0.0); values.len()];
    let total: f64 = values.iter().filter(|v| **v > 0.0).sum();
    if total <= 0.0 || bounds.area() <= 0.0 {
        return out;
    }
    let scale = bounds.area() / total;
    let mut order: Vec<usize> = (0..values.len()).filter(|&i| values[i] > 0.0).collect();
    order.sort_by(|a, b| values[*b].partial_cmp(&values[*a]).unwrap_or(std::cmp::Ordering::Equal));

    let mut free = bounds;
    let mut row: Vec<usize> = Vec::new();
    let mut row_areas: Vec<f64> = Vec::new();
    let mut i = 0;
    while i < order.len() {
        let a = values[order[i]] * scale;
        let side = free.w.min(free.h);
        let mut candidate = row_areas.clone();
        candidate.push(a);
        if row.is_empty() || worst(&candidate, side) <= worst(&row_areas, side) {
            row.push(order[i]);
            row_areas.push(a);
            i += 1;
        } else {
            place_row(&row, &row_areas, &mut free, &mut out);
            row.clear();
            row_areas.clear();
        }
    }
    if !row.is_empty() {
        place_row(&row, &row_areas, &mut free, &mut out);
    }
    out
}

fn place_row(row: &[usize], areas: &[f64], free: &mut Rect, out: &mut [Rect]) {
    let s: f64 = areas.iter().sum();
    if free.w >= free.h {
        // A column along the left edge.
        let thick = if free.h > 0.0 { (s / free.h).min(free.w) } else { 0.0 };
        let mut y = free.y;
        for (k, &idx) in row.iter().enumerate() {
            let h = if thick > 0.0 { areas[k] / thick } else { 0.0 };
            out[idx] = Rect::new(free.x, y, thick, h);
            y += h;
        }
        free.x += thick;
        free.w -= thick;
    } else {
        // A row along the top edge.
        let thick = if free.w > 0.0 { (s / free.w).min(free.h) } else { 0.0 };
        let mut x = free.x;
        for (k, &idx) in row.iter().enumerate() {
            let w = if thick > 0.0 { areas[k] / thick } else { 0.0 };
            out[idx] = Rect::new(x, free.y, w, thick);
            x += w;
        }
        free.y += thick;
        free.h -= thick;
    }
}

/// One file's rectangle in the whole-tree treemap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    pub node: usize,
    pub rect: Rect,
}

/// Nested treemap of the files below `start`: folders are split among their children,
/// and tiles smaller than `min_side` pixels are dropped. At most `max_tiles` are returned.
pub fn treemap(tree: &Tree, start: usize, bounds: Rect, min_side: f64, max_tiles: usize) -> Vec<Tile> {
    let mut tiles = Vec::new();
    let mut stack = vec![(start, bounds)];
    while let Some((index, rect)) = stack.pop() {
        if tiles.len() >= max_tiles || rect.w < min_side || rect.h < min_side {
            continue;
        }
        let node = &tree.nodes[index];
        if !node.is_dir {
            tiles.push(Tile { node: index, rect });
            continue;
        }
        let sizes: Vec<f64> = node.children.iter().map(|&c| tree.nodes[c].size as f64).collect();
        let rects = squarify(&sizes, rect);
        for (&child, r) in node.children.iter().zip(rects).rev() {
            stack.push((child, r));
        }
    }
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vol_tree(volumes: &[(&str, u64)]) -> Tree {
        let volumes = volumes.iter().map(|(p, v)| (PathBuf::from(p), *v)).collect();
        Tree { volumes, root_path: PathBuf::from(r"C:\"), nodes: Vec::new(), errors: 0, method: ScanMethod::Walk }
    }

    #[test]
    fn a_rescanned_branch_takes_its_own_volume() {
        // the drive was reassigned and C:\A rescanned: its fresh ids are on volume 7
        let mut t = vol_tree(&[(r"C:\", 5), (r"D:\x", 9)]);
        t.replace_volumes(Path::new(r"C:\A"), &[(PathBuf::from(r"c:\a"), 7)]);
        assert_eq!(t.volume_of(Path::new(r"C:\A\f.txt")), Some(7));
        assert_eq!(t.volume_of(Path::new(r"C:\B\f.txt")), Some(5));
        assert_eq!(t.volume_of(Path::new(r"D:\x\y")), Some(9));
        // rescanned again: the old entry under it goes
        t.replace_volumes(Path::new(r"C:\"), &[(PathBuf::from(r"C:\"), 8)]);
        assert_eq!(t.volume_of(Path::new(r"C:\A\f.txt")), Some(8));
        assert_eq!(t.volumes.len(), 2, "{:?}", t.volumes);
    }

    #[test]
    fn a_rescan_with_no_volume_does_not_borrow_the_old_one() {
        let mut t = vol_tree(&[(r"C:\", 5)]);
        t.replace_volumes(Path::new(r"C:\A"), &[]);
        assert_eq!(t.volume_of(Path::new(r"C:\A\f.txt")), None);
        assert_eq!(t.volume_of(Path::new(r"C:\B")), Some(5));
    }

    #[test]
    fn a_snapshot_branch_refreshed_live_gets_a_volume() {
        // a tree from a snapshot knows no volumes; the refreshed branch does
        let mut t = vol_tree(&[]);
        t.replace_volumes(Path::new(r"C:\A"), &[(PathBuf::from(r"C:\A"), 7)]);
        assert_eq!(t.volume_of(Path::new(r"C:\A\f.txt")), Some(7));
        assert_eq!(t.volume_of(Path::new(r"C:\B")), None);
    }

    #[test]
    fn overlapping_roots_that_disagree_are_unknown() {
        let mut v: Vec<(PathBuf, u64)> = [(r"C:\A", 5), (r"C:\A\B", 6), (r"C:\Z", 5), (r"C:\Z\Y", 5)].iter().map(|(p, s)| (PathBuf::from(p), *s)).collect();
        settle_overlaps(&mut v);
        let t = Tree { volumes: v, ..vol_tree(&[]) };
        assert_eq!(t.volume_of(Path::new(r"C:\A\B\f")), None);
        assert_eq!(t.volume_of(Path::new(r"C:\A\f")), Some(5));
        assert_eq!(t.volume_of(Path::new(r"C:\Z\Y\f")), Some(5));
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn squarify_covers_area_and_stays_inside() {
        let values = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let b = Rect::new(0.0, 0.0, 6.0, 4.0);
        let rects = squarify(&values, b);
        let total: f64 = rects.iter().map(Rect::area).sum();
        assert!(close(total, 24.0), "total {total}");
        for (r, v) in rects.iter().zip(values) {
            assert!(close(r.area(), v), "area {} for {v}", r.area());
            assert!(r.x >= -1e-9 && r.y >= -1e-9);
            assert!(r.x + r.w <= 6.0 + 1e-9 && r.y + r.h <= 4.0 + 1e-9);
        }
    }

    #[test]
    fn squarify_handles_zero_and_empty() {
        assert!(squarify(&[], Rect::new(0.0, 0.0, 10.0, 10.0)).is_empty());
        let r = squarify(&[0.0, 5.0], Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(close(r[0].area(), 0.0));
        assert!(close(r[1].area(), 100.0));
    }

    #[test]
    fn extensions_and_formatting() {
        assert_eq!(extension_of("a.TXT"), "txt");
        assert_eq!(extension_of(".gitignore"), "(none)");
        assert_eq!(extension_of("README"), "(none)");
        assert_eq!(format_size(512), "512 bytes");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(extension_colour("exe"), extension_colour("exe"));
    }

    #[test]
    fn scan_totals_and_stats() {
        let dir = std::env::temp_dir().join(format!("sprawl-core-test-{}", std::process::id()));
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(dir.join("a.txt"), vec![0u8; 100]).unwrap();
        fs::write(sub.join("b.bin"), vec![0u8; 300]).unwrap();
        fs::write(sub.join("c.txt"), vec![0u8; 50]).unwrap();

        let tree = scan(&dir, |_| {}).unwrap();
        let root = &tree.nodes[ROOT];
        assert_eq!(root.size, 450);
        assert_eq!(root.files, 3);
        assert_eq!(root.folders, 1);
        assert_eq!(root.items(), 4);
        // Biggest child first: sub (350) before a.txt (100).
        assert_eq!(tree.nodes[root.children[0]].name, "sub");
        assert!(close(tree.percent_of_parent(root.children[0]), 350.0 * 100.0 / 450.0));

        let stats = extension_stats(&tree);
        assert_eq!(stats[0].extension, "bin");
        assert_eq!(stats[0].size, 300);
        assert_eq!(stats[1].extension, "txt");
        assert_eq!(stats[1].files, 2);

        let big = largest_files(&tree, 1);
        assert_eq!(tree.nodes[big[0]].name, "b.bin");
        assert_eq!(tree.path_of(big[0]), sub.join("b.bin"));

        let tiles = treemap(&tree, ROOT, Rect::new(0.0, 0.0, 90.0, 50.0), 0.5, 100);
        assert_eq!(tiles.len(), 3);
        let area: f64 = tiles.iter().map(|t| t.rect.area()).sum();
        assert!(close(area, 4500.0));

        // Leaves the temp folder in place: deleting is left to the system's temp cleanup.
    }

    #[test]
    fn scan_many_and_subtree() {
        let base = std::env::temp_dir().join(format!("sprawl-core-many-{}", std::process::id()));
        let (a, b) = (base.join("a"), base.join("b"));
        fs::create_dir_all(a.join("inner")).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("inner").join("x.bin"), vec![0u8; 700]).unwrap();
        fs::write(b.join("y.txt"), vec![0u8; 300]).unwrap();

        let p = Progress::new();
        let t = scan_many(&[a.clone(), b.clone()], &p).unwrap();
        let r = &t.nodes[ROOT];
        assert_eq!(r.name, MULTIPLE);
        assert_eq!((r.size, r.files, r.folders), (1000, 2, 3));
        assert_eq!(t.nodes[r.children[0]].name, a.display().to_string());
        let x = t.nodes.iter().position(|n| n.name == "x.bin").unwrap();
        assert_eq!(t.path_of(x), a.join("inner").join("x.bin"));
        for (i, n) in t.nodes.iter().enumerate() {
            for &c in &n.children {
                assert!(c > i);
                assert_eq!(t.nodes[c].parent, Some(i));
            }
        }
        let s = p.snapshot();
        assert_eq!(s.top, vec![(a.display().to_string(), 700), (b.display().to_string(), 300)]);
        assert_eq!((s.files, s.folders, s.bytes), (2, 3, 1000));

        // one root skipped: refused before any root's name is shown (Astra's case)
        let skip = Progress::with_exclusions(std::slice::from_ref(&b));
        let e = scan_many(&[a.clone(), b.clone()], &skip).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        let s = skip.snapshot();
        assert!(s.top.is_empty() && !s.current.contains(&*b.display().to_string()), "{:?} {}", s.top, s.current);

        let sub = t.subtree(t.nodes[ROOT].children[0]);
        assert_eq!(sub.root_path, a);
        assert_eq!(sub.nodes[ROOT].size, 700);
        assert_eq!(sub.path_of(sub.nodes.len() - 1), a.join("inner").join("x.bin"));

        // The compatibility wrapper still reports a count.
        let mut last = 0;
        let t = scan(&base, |n| last = n).unwrap();
        assert_eq!(t.nodes[ROOT].size, 1000);
        assert_eq!(last, 5);
        let _ = fs::remove_dir_all(&base);
    }
}

/// A time as local date and time text, e.g. `2026/11/16 21:04:05`.
pub fn local_time(t: SystemTime) -> String {
    csv::local_time_string(system_to_filetime(t))
}

/// A time as local [year, month, day, hour, minute, second].
pub fn local_parts(t: SystemTime) -> Option<[u16; 6]> {
    csv::local_parts(system_to_filetime(t))
}
