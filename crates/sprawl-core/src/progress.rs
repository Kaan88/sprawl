//! Live scan progress, shared between the scanning thread(s) and the UI.
//!
//! The scanner updates plain atomics (counts, bytes, units done) and, in batches, a small
//! mutex-protected [`Live`] part (current path, per-extension totals, sizes of the root's direct
//! children). The UI calls [`Progress::snapshot`] as often as it likes (60x/s is fine): it takes
//! the lock once and copies at most 24 + 24 short entries.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

/// In a list of exclusions: the list of folders to skip couldn't be read, so every path counts
/// as skipped, network paths included (drive letters alone don't cover `\\server\share`).
pub const UNREADABLE_POLICY: &str = "<the list of folders to skip could not be read>";

/// In a list of exclusions: a folder to skip is on a network drive whose share Windows couldn't
/// name, so it can't be told which `\\server\share` paths reach it: every such path counts as
/// skipped (the network drives' letters are in the list as well).
pub const REMOTE_UNKNOWN: &str = "<the share of a network drive with a folder to skip could not be read>";

fn exclusion_key(path: &std::path::Path) -> String {
    let s = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let s = if let Some(unc) = s.strip_prefix(r"\\?\unc\") { format!(r"\\{unc}") } else { s.strip_prefix(r"\\?\").unwrap_or(&s).to_string() };
    s.trim_end_matches('\\').to_string()
}

/// How a tree was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanMethod {
    /// Directory listing (FindFirstFileExW, multi-threaded). Works without admin rights.
    #[default]
    Walk,
    /// Reading the NTFS master file table directly (admin, NTFS volumes only).
    Mft,
    /// Loaded from a WizTree/Sprawl CSV export.
    Csv,
}

impl ScanMethod {
    pub fn label(self) -> &'static str {
        match self {
            ScanMethod::Walk => "Folder scan",
            ScanMethod::Mft => "MFT",
            ScanMethod::Csv => "CSV",
        }
    }
    fn from_u8(v: u8) -> Self {
        match v {
            1 => ScanMethod::Mft,
            2 => ScanMethod::Csv,
            _ => ScanMethod::Walk,
        }
    }
    fn to_u8(self) -> u8 {
        match self {
            ScanMethod::Walk => 0,
            ScanMethod::Mft => 1,
            ScanMethod::Csv => 2,
        }
    }
}

/// How many extensions / top-level children a snapshot carries.
pub const SNAPSHOT_TOP: usize = 24;

/// The part of the progress that needs a lock. Only touched in batches.
#[derive(Debug, Default)]
struct Live {
    current: String,
    phase: &'static str,
    /// extension -> (bytes, files)
    by_ext: HashMap<String, (u64, u64)>,
    /// Root's direct children: (name, bytes so far).
    top: Vec<(String, u64)>,
    /// `scan_many`: every top-level contribution goes into this slot, and scanners can't replace
    /// the list.
    pinned: Option<usize>,
}

/// Shared, thread-safe scan progress. Create with [`Progress::new`], hand a reference to a scan
/// function running on a background thread, and poll [`Progress::snapshot`] from the UI.
#[derive(Debug, Default)]
pub struct Progress {
    #[cfg(test)]
    pub(crate) wof_probes: AtomicU64,
    excluded: Vec<String>,
    files: AtomicU64,
    folders: AtomicU64,
    bytes: AtomicU64,
    errors: AtomicU64,
    done_units: AtomicU64,
    /// 0 = unknown (indeterminate progress).
    total_units: AtomicU64,
    cancel: AtomicBool,
    method: AtomicU8,
    live: Mutex<Live>,
    /// Each root this scan opened, and whether it was on the network (see [`Progress::note_root`]).
    roots: Mutex<Vec<(std::path::PathBuf, Option<bool>)>>,
}

/// A copy of the progress at one moment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProgressSnapshot {
    /// Files seen so far.
    pub files: u64,
    /// Folders seen so far.
    pub folders: u64,
    /// File bytes (logical size) seen so far.
    pub bytes: u64,
    /// Folders/entries that couldn't be read.
    pub errors: u64,
    /// 0.0..=1.0 when the total amount of work is known (MFT bytes, CSV bytes), else None.
    pub fraction: Option<f64>,
    /// The folder or file being worked on (for display only).
    pub current: String,
    pub method: ScanMethod,
    /// "Reading MFT", "Scanning folders", "Building tree", "Reading CSV", "Done", ...
    pub phase: &'static str,
    /// (extension, bytes, files), biggest first, at most 24.
    pub by_ext: Vec<(String, u64, u64)>,
    /// Sizes so far of the scanned folder's direct children, biggest first, at most 24.
    /// Empty when not known (yet).
    pub top: Vec<(String, u64)>,
}

impl Progress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Exact absolute folder scopes, omitted before enumeration. Immutable for this scan.
    pub fn with_exclusions(paths: &[std::path::PathBuf]) -> Arc<Self> {
        Arc::new(Self { excluded: paths.iter().map(|p| exclusion_key(p)).collect(), ..Self::default() })
    }

    pub fn has_exclusions(&self) -> bool { !self.excluded.is_empty() }

    /// Record, from the handle the scan opened `root` with, whether it is on the network:
    /// `Some(true)` a share or a network drive, `Some(false)` a drive of this PC, `None` it
    /// couldn't be told. Told when the root was opened, so a letter mapped later in the scan
    /// doesn't change it; noted again for the same root (a walk after a failed MFT read), the
    /// later word stands.
    pub fn note_root(&self, root: &std::path::Path, network: Option<bool>) {
        let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        match roots.iter_mut().find(|(r, _)| r == root) {
            Some(r) => r.1 = network,
            None => roots.push((root.to_path_buf(), network)),
        }
    }

    /// What [`Progress::note_root`] recorded, in the order the roots were opened.
    pub fn roots(&self) -> Vec<(std::path::PathBuf, Option<bool>)> {
        self.roots.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn excludes(&self, path: &std::path::Path) -> bool {
        if self.excluded.is_empty() { return false; }
        if self.excluded.iter().any(|p| p == UNREADABLE_POLICY) { return true; }
        let key = exclusion_key(path);
        if key.starts_with(r"\\") && self.excluded.iter().any(|p| p == REMOTE_UNKNOWN) { return true; }
        self.excluded.iter().any(|p| key == *p || key.strip_prefix(p).is_some_and(|s| s.starts_with('\\')))
    }

    /// Some excluded scope lies strictly inside `path`: its children need checking.
    pub(crate) fn exclusion_inside(&self, path: &std::path::Path) -> bool {
        if self.excluded.is_empty() { return false; }
        let key = exclusion_key(path);
        self.excluded.iter().any(|p| p.strip_prefix(&key).is_some_and(|s| s.starts_with('\\') && s.len() > 1))
    }

    /// A consistent-enough copy for drawing. Cheap: one short lock.
    pub fn snapshot(&self) -> ProgressSnapshot {
        let total = self.total_units.load(Relaxed);
        let fraction = if total == 0 { None } else { Some((self.done_units.load(Relaxed) as f64 / total as f64).clamp(0.0, 1.0)) };
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let mut by_ext: Vec<(String, u64, u64)> = Vec::with_capacity(SNAPSHOT_TOP);
        // Partial selection of the biggest entries without sorting the whole map.
        let mut all: Vec<(&String, &(u64, u64))> = live.by_ext.iter().collect();
        let k = SNAPSHOT_TOP.min(all.len());
        if k > 0 && all.len() > k {
            all.select_nth_unstable_by(k - 1, |a, b| b.1 .0.cmp(&a.1 .0));
        }
        all.truncate(k);
        all.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(b.0)));
        for (e, (b, f)) in all {
            by_ext.push((e.clone(), *b, *f));
        }
        let mut refs: Vec<&(String, u64)> = live.top.iter().collect();
        let k = SNAPSHOT_TOP.min(refs.len());
        if k > 0 && refs.len() > k {
            refs.select_nth_unstable_by(k - 1, |a, b| b.1.cmp(&a.1));
        }
        refs.truncate(k);
        refs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let top: Vec<(String, u64)> = refs.into_iter().cloned().collect();
        ProgressSnapshot {
            files: self.files.load(Relaxed),
            folders: self.folders.load(Relaxed),
            bytes: self.bytes.load(Relaxed),
            errors: self.errors.load(Relaxed),
            fraction,
            current: live.current.clone(),
            method: ScanMethod::from_u8(self.method.load(Relaxed)),
            phase: live.phase,
            by_ext,
            top,
        }
    }

    /// Ask the running scan to stop. It returns `Err(ErrorKind::Interrupted)` soon after.
    pub fn cancel(&self) {
        self.cancel.store(true, Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }

    /// Items (files + folders) seen so far.
    pub fn items(&self) -> u64 {
        self.files.load(Relaxed) + self.folders.load(Relaxed)
    }

    // ---------------------------------------------------------------- scanner side

    pub(crate) fn set_method(&self, m: ScanMethod) {
        self.method.store(m.to_u8(), Relaxed);
    }

    pub fn set_phase(&self, phase: &'static str) {
        self.lock().phase = phase;
    }

    pub(crate) fn add_counts(&self, files: u64, folders: u64, bytes: u64) {
        if files != 0 {
            self.files.fetch_add(files, Relaxed);
        }
        if folders != 0 {
            self.folders.fetch_add(folders, Relaxed);
        }
        if bytes != 0 {
            self.bytes.fetch_add(bytes, Relaxed);
        }
    }

    pub(crate) fn set_counts(&self, files: u64, folders: u64, bytes: u64) {
        self.files.store(files, Relaxed);
        self.folders.store(folders, Relaxed);
        self.bytes.store(bytes, Relaxed);
    }

    pub(crate) fn add_errors(&self, n: u64) {
        self.errors.fetch_add(n, Relaxed);
    }

    pub(crate) fn errors(&self) -> u64 {
        self.errors.load(Relaxed)
    }

    pub(crate) fn set_units(&self, done: u64, total: u64) {
        self.total_units.store(total, Relaxed);
        self.done_units.store(done, Relaxed);
    }

    pub(crate) fn add_done_units(&self, n: u64) {
        self.done_units.fetch_add(n, Relaxed);
    }

    /// Back to zero before a (fallback) scan; keeps the cancel flag and a pinned top list.
    pub(crate) fn reset_counts(&self) {
        self.set_counts(0, 0, 0);
        self.errors.store(0, Relaxed);
        self.set_units(0, 0);
        let mut l = self.lock();
        l.by_ext.clear();
        if l.pinned.is_none() {
            l.top.clear();
        }
    }

    /// Replace the list of the root's direct children (ignored while pinned).
    pub(crate) fn top_reset(&self, top: Vec<(String, u64)>) {
        let mut l = self.lock();
        if l.pinned.is_none() {
            l.top = top;
        }
    }

    /// Add one direct child of the root; returns its slot for [`LiveBatch::add_top`].
    pub(crate) fn top_push(&self, name: String, bytes: u64) -> usize {
        let mut l = self.lock();
        if let Some(p) = l.pinned {
            if let Some(t) = l.top.get_mut(p) {
                t.1 += bytes;
            }
            return p;
        }
        l.top.push((name, bytes));
        l.top.len() - 1
    }

    /// `scan_many`: fixed top list, and route everything into slot `slot`.
    pub(crate) fn pin_top(&self, names: Option<Vec<String>>, slot: Option<usize>) {
        let mut l = self.lock();
        if let Some(names) = names {
            l.top = names.into_iter().map(|n| (n, 0)).collect();
        }
        l.pinned = slot;
    }

    /// Fold a scanner's local batch into the shared state (one lock).
    pub(crate) fn merge(&self, batch: &mut LiveBatch) {
        if batch.is_empty() {
            return;
        }
        let mut l = self.lock();
        for (ext, (b, f)) in batch.by_ext.drain() {
            let e = l.by_ext.entry(ext).or_insert((0, 0));
            e.0 += b;
            e.1 += f;
        }
        let pinned = l.pinned;
        for (slot, bytes) in batch.top.drain(..) {
            let s = pinned.unwrap_or(slot);
            if let Some(t) = l.top.get_mut(s) {
                t.1 += bytes;
            }
        }
        if let Some(c) = batch.current.take() {
            l.current = c;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A scanner-local accumulation of [`Live`] updates, merged with [`Progress::merge`].
#[derive(Debug, Default)]
pub(crate) struct LiveBatch {
    pub by_ext: HashMap<String, (u64, u64)>,
    pub top: Vec<(usize, u64)>,
    pub current: Option<String>,
}

impl LiveBatch {
    pub fn add_file(&mut self, name: &str, bytes: u64) {
        let ext = crate::extension_of(name);
        let e = self.by_ext.entry(ext).or_insert((0, 0));
        e.0 += bytes;
        e.1 += 1;
    }

    pub fn add_top(&mut self, slot: usize, bytes: u64) {
        if bytes == 0 {
            return;
        }
        match self.top.last_mut() {
            Some(last) if last.0 == slot => last.1 += bytes,
            _ => self.top.push((slot, bytes)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.by_ext.is_empty() && self.top.is_empty() && self.current.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_orders_and_limits() {
        let p = Progress::new();
        p.set_method(ScanMethod::Mft);
        p.set_units(25, 100);
        p.add_counts(3, 1, 300);
        let a = p.top_push("a".into(), 0);
        let b = p.top_push("b".into(), 10);
        let mut batch = LiveBatch::default();
        for i in 0..40 {
            batch.add_file(&format!("f.e{i}"), i as u64 * 10);
        }
        batch.add_top(a, 50);
        batch.add_top(b, 5);
        batch.current = Some("C:\\x".into());
        p.merge(&mut batch);
        let s = p.snapshot();
        assert_eq!(s.method, ScanMethod::Mft);
        assert_eq!(s.fraction, Some(0.25));
        assert_eq!((s.files, s.folders, s.bytes), (3, 1, 300));
        assert_eq!(s.by_ext.len(), SNAPSHOT_TOP);
        assert_eq!(s.by_ext[0], ("e39".to_string(), 390, 1));
        assert_eq!(s.top, vec![("a".to_string(), 50), ("b".to_string(), 15)]);
        assert_eq!(s.current, "C:\\x");
        p.cancel();
        assert!(p.is_cancelled());
    }

    #[test]
    fn pinned_top_collects_everything() {
        let p = Progress::new();
        p.pin_top(Some(vec!["one".into(), "two".into()]), Some(1));
        let slot = p.top_push("ignored".into(), 7);
        assert_eq!(slot, 1);
        p.top_reset(vec![("x".into(), 1)]);
        let mut batch = LiveBatch::default();
        batch.add_top(0, 3);
        p.merge(&mut batch);
        let s = p.snapshot();
        assert_eq!(s.top, vec![("two".to_string(), 10), ("one".to_string(), 0)]);
    }
}
