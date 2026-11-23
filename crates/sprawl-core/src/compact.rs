//! A search-only copy of a scan, for drives read just to be searched.
//!
//! A [`Tree`] costs about 180 bytes an item and the search [`crate::search::Index`] keeps every
//! name a second time. [`CompactIndex`] keeps only what a search and its results list need:
//! the names once (lower-case, with one bit a letter to give back the capitals), the parent,
//! size, modified time, a folder flag and the depth, about 26 bytes an item plus the name.
//!
//! How the app should use it, for a drive that is only searched (not shown in the tree):
//!
//! 1. scan the drive as today, `let ix = CompactIndex::build(&tree)`, then drop the tree;
//! 2. on each query, `ix.search(&query, &cancel)` with the same [`Query`] the scan's search
//!    uses (show [`Query::errors`] the same way); results are item ids (`u32`), in path order
//!    of the scan (folders before what is inside them);
//! 3. for the rows: [`CompactIndex::name`], [`CompactIndex::path_of`], [`CompactIndex::size`],
//!    [`CompactIndex::modified`], [`CompactIndex::is_dir`];
//! 4. for a column header click: [`CompactIndex::sort`] (the folder order is built on the first
//!    folder sort, about 0.2 s for a full drive, and kept);
//! 5. ids are only valid for the index they came from: after a rescan, build a new one and
//!    search again. Nothing here follows changes on disk; check that a path exists before
//!    acting on it.
//!
//! Build it off the UI thread (it walks the tree once); searching is as fast as the tree's.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::query::{unix_secs, Item, Kind, Query, Term, NEED_CASE_PATH};
use crate::search::{last_name, par_filter, threads};
use crate::{Tree, ROOT};

const DIR: u8 = 1;
/// The name is a full path ("Select Multiple" children), kept in `abs`.
const ABS: u8 = 2;
/// The name is not ASCII: kept as it is in `odd`.
const ODD: u8 = 4;
const COMPRESSED: u8 = 8;
/// The `FILE_ATTRIBUTE_*` bits a search can ask about (`attrib:`).
const HIDDEN: u8 = 16;
const SYSTEM: u8 = 32;
const READONLY: u8 = 64;
const REPARSE: u8 = 128;
/// No modified time.
const NO_TIME: i64 = i64::MIN;

/// What a results list can be sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Size,
    Name,
    Modified,
    /// The folder the item is in, in path order; then the name.
    Folder,
}

/// See the module docs. Item 0 is the scanned root; items are in depth-first order, so a
/// folder comes before everything inside it.
#[derive(Debug, Default)]
pub struct CompactIndex {
    root: PathBuf,
    /// Every name, lower-case, back to back.
    blob: Vec<u8>,
    /// Where item `i`'s name starts; `starts[i + 1]` is where it ends.
    starts: Vec<u32>,
    /// One bit for each byte of `blob`: it was an upper-case ASCII letter.
    upper: Vec<u8>,
    /// Names as they are, for items flagged [`ODD`] (sorted by id).
    odd: Vec<(u32, Box<str>)>,
    /// Full names (paths), for items flagged [`ABS`] (sorted by id).
    abs: Vec<(u32, Box<str>)>,
    /// `u32::MAX` for the root.
    parent: Vec<u32>,
    size: Vec<u64>,
    /// Seconds since 1970 (UTC), or [`NO_TIME`].
    modified: Vec<i64>,
    flags: Vec<u8>,
    /// Levels below the root, at most 255.
    depth: Vec<u8>,
    /// Every folder's place in path order, made on the first folder sort.
    ranks: OnceLock<Vec<u32>>,
}

fn lookup(v: &[(u32, Box<str>)], i: usize) -> Option<&str> {
    v.binary_search_by_key(&(i as u32), |e| e.0).ok().map(|k| &*v[k].1)
}

impl CompactIndex {
    /// A compact copy of `tree`'s items that can be reached from its root (a deleted branch is
    /// left out). The tree can be dropped afterwards.
    pub fn build(tree: &Tree) -> CompactIndex {
        let n0 = tree.nodes.len();
        let mut ix = CompactIndex { root: tree.root_path.clone(), ..Default::default() };
        if n0 == 0 {
            ix.starts.push(0);
            return ix;
        }
        ix.blob.reserve(n0 * 20);
        for v in [&mut ix.starts, &mut ix.parent] {
            v.reserve(n0 + 1);
        }
        ix.size.reserve(n0);
        ix.modified.reserve(n0);
        ix.flags.reserve(n0);
        ix.depth.reserve(n0);
        let mut seen = vec![false; n0];
        let mut stack: Vec<(usize, u32, u8)> = vec![(ROOT, u32::MAX, 0)];
        while let Some((old, parent, depth)) = stack.pop() {
            if old >= n0 || seen[old] {
                continue;
            }
            seen[old] = true;
            let id = ix.parent.len() as u32;
            let node = &tree.nodes[old];
            let mut flags = if node.is_dir { DIR } else { 0 };
            if node.is_compressed() {
                flags |= COMPRESSED;
            }
            if node.attributes & crate::attr::HIDDEN != 0 {
                flags |= HIDDEN;
            }
            if node.attributes & crate::attr::SYSTEM != 0 {
                flags |= SYSTEM;
            }
            if node.attributes & crate::attr::READONLY != 0 {
                flags |= READONLY;
            }
            if node.attributes & crate::attr::REPARSE_POINT != 0 {
                flags |= REPARSE;
            }
            let at = ix.blob.len();
            ix.starts.push(at as u32);
            let name = last_name(&node.name);
            if name.is_ascii() {
                ix.blob.extend(name.bytes().map(|b| b.to_ascii_lowercase()));
                ix.upper.resize(ix.blob.len().div_ceil(8), 0);
                for (k, b) in name.bytes().enumerate() {
                    if b.is_ascii_uppercase() {
                        ix.upper[(at + k) / 8] |= 1 << ((at + k) % 8);
                    }
                }
            } else {
                ix.blob.extend_from_slice(name.to_lowercase().as_bytes());
                ix.upper.resize(ix.blob.len().div_ceil(8), 0);
                flags |= ODD;
                ix.odd.push((id, name.into()));
            }
            if old != ROOT && Path::new(&node.name).is_absolute() {
                flags |= ABS;
                ix.abs.push((id, node.name.as_str().into()));
            }
            ix.parent.push(parent);
            ix.size.push(node.size);
            ix.modified.push(node.modified.map_or(NO_TIME, unix_secs));
            ix.flags.push(flags);
            ix.depth.push(depth);
            let next = depth.saturating_add(1);
            for &c in node.children.iter().rev() {
                stack.push((c, id, next));
            }
        }
        ix.starts.push(ix.blob.len() as u32);
        ix.blob.shrink_to_fit();
        ix.upper.shrink_to_fit();
        ix.starts.shrink_to_fit();
        ix.parent.shrink_to_fit();
        ix.size.shrink_to_fit();
        ix.modified.shrink_to_fit();
        ix.flags.shrink_to_fit();
        ix.depth.shrink_to_fit();
        ix.odd.shrink_to_fit();
        ix.abs.shrink_to_fit();
        ix
    }

    /// Items, the root included.
    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    /// The scanned folder.
    pub fn root_path(&self) -> &Path {
        &self.root
    }

    /// Bytes the index holds (the folder order included once it is made).
    pub fn memory(&self) -> usize {
        let side = |v: &[(u32, Box<str>)]| v.iter().map(|e| e.1.len()).sum::<usize>();
        self.blob.capacity()
            + self.starts.capacity() * 4
            + self.upper.capacity()
            + self.odd.capacity() * std::mem::size_of::<(u32, Box<str>)>()
            + side(&self.odd)
            + self.abs.capacity() * std::mem::size_of::<(u32, Box<str>)>()
            + side(&self.abs)
            + self.parent.capacity() * 4
            + self.size.capacity() * 8
            + self.modified.capacity() * 8
            + self.flags.capacity()
            + self.depth.capacity()
            + self.ranks.get().map_or(0, |r| r.capacity() * 4)
    }

    /// Item `i`'s lower-case name.
    fn low(&self, i: usize) -> &[u8] {
        &self.blob[self.starts[i] as usize..self.starts[i + 1] as usize]
    }

    /// Item `i`'s name as it is on disk (the last part, for a "Select Multiple" child).
    pub fn name(&self, id: u32) -> String {
        match self.orig_name(id as usize) {
            Cow::Borrowed(b) => String::from_utf8_lossy(b).into_owned(),
            Cow::Owned(b) => String::from_utf8(b).unwrap_or_default(),
        }
    }

    fn orig_name(&self, i: usize) -> Cow<'_, [u8]> {
        if self.flags[i] & ODD != 0 {
            return Cow::Borrowed(lookup(&self.odd, i).unwrap_or("").as_bytes());
        }
        let at = self.starts[i] as usize;
        let mut v = self.low(i).to_vec();
        for (k, b) in v.iter_mut().enumerate() {
            if self.upper[(at + k) / 8] & (1 << ((at + k) % 8)) != 0 {
                *b = b.to_ascii_uppercase();
            }
        }
        Cow::Owned(v)
    }

    /// The full path, as [`Tree::path_of`] gives it.
    pub fn path_of(&self, id: u32) -> PathBuf {
        let mut parts: Vec<String> = Vec::new();
        let mut i = id as usize;
        while i < self.len() && self.parent[i] != u32::MAX {
            parts.push(match lookup(&self.abs, i).filter(|_| self.flags[i] & ABS != 0) {
                Some(full) => full.to_string(),
                None => self.name(i as u32),
            });
            i = self.parent[i] as usize;
        }
        let mut path = self.root.clone();
        for part in parts.iter().rev() {
            path.push(part);
        }
        path
    }

    /// Bytes (a folder's is its total).
    pub fn size(&self, id: u32) -> u64 {
        self.size[id as usize]
    }

    pub fn modified(&self, id: u32) -> Option<SystemTime> {
        let s = self.modified[id as usize];
        if s == NO_TIME {
            None
        } else if s >= 0 {
            UNIX_EPOCH.checked_add(Duration::from_secs(s as u64))
        } else {
            UNIX_EPOCH.checked_sub(Duration::from_secs(s.unsigned_abs()))
        }
    }

    pub fn is_dir(&self, id: u32) -> bool {
        self.flags[id as usize] & DIR != 0
    }

    /// The direct children of item `i`, in order (the ids of a folder and what is inside it are
    /// neighbours).
    fn kids(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let n = self.len();
        let d = self.depth[i];
        let mut j = i + 1;
        std::iter::from_fn(move || {
            while j < n && self.depth[j] > d {
                let k = j;
                j += 1;
                if self.parent[k] == i as u32 {
                    return Some(k);
                }
            }
            None
        })
    }

    /// The folder an item is in; `None` for the root.
    pub fn parent(&self, id: u32) -> Option<u32> {
        Some(self.parent[id as usize]).filter(|&p| p != u32::MAX)
    }

    /// Levels below the root (its children are 1).
    pub fn depth(&self, id: u32) -> u32 {
        u32::from(self.depth[id as usize])
    }

    /// The items that match `q` (never the root), in index order. `None` when `cancel` was
    /// set on the way. A query with [`Query::errors`] finds nothing.
    pub fn search(&self, q: &Query, cancel: &AtomicBool) -> Option<Vec<u32>> {
        let n = self.len();
        if n <= 1 || q.is_empty() || !q.errors().is_empty() {
            return Some(Vec::new());
        }
        let (early, path_terms) = q.split_terms();
        // the flags and sizes are read only when the query asks about them
        let sized = q.min.is_some() || q.max.is_some();
        let cheap = |i: usize| -> bool {
            match q.kind {
                Kind::Any => {}
                Kind::Files if self.flags[i] & DIR != 0 => return false,
                Kind::Folders if self.flags[i] & DIR == 0 => return false,
                _ => {}
            }
            if sized && !q.size_ok(self.size[i]) {
                return false;
            }
            let it = CItem { ix: self, i, path: &[], opath: &[] };
            early.iter().all(|t| t.matches(&it))
        };
        let hits = par_filter(n, cancel, cheap)?;
        let mut hits = hits;
        if path_terms.is_empty() {
            hits = self.dupes(q, hits);
            return Some(hits.into_iter().map(|i| i as u32).collect());
        }

        // the path words: each core takes a run of items and builds their paths as it goes
        // (a folder comes before what is inside it, so its path is at hand)
        let mut candidate = vec![false; n];
        for &i in &hits {
            candidate[i] = true;
        }
        let orig = path_terms.iter().any(|t| t.needs & NEED_CASE_PATH != 0);
        let threads = threads();
        let chunk = n.div_ceil(threads * 4).max(1);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let parts: Vec<Option<Vec<(usize, Vec<u32>)>>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    let (next, candidate, path_terms) = (&next, &candidate, &path_terms);
                    s.spawn(move || {
                        let mut out = Vec::new();
                        loop {
                            let k = next.fetch_add(1, Ordering::Relaxed);
                            let a = (k * chunk).max(1);
                            if a >= n {
                                return Some(out);
                            }
                            let b = ((k + 1) * chunk).min(n);
                            out.push((k, self.walk_run(a, b, candidate, path_terms, orig, cancel)?));
                        }
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or(None)).collect()
        });
        let mut runs = Vec::new();
        for p in parts {
            runs.extend(p?);
        }
        runs.sort_unstable_by_key(|r| r.0);
        let hits: Vec<usize> = runs.into_iter().flat_map(|r| r.1).map(|i| i as usize).collect();
        Some(self.dupes(q, hits).into_iter().map(|i| i as u32).collect())
    }

    /// `dupe:` and `sizedupe:`: among the matches of the rest of the query, the ones with a copy.
    fn dupes(&self, q: &Query, hits: Vec<usize>) -> Vec<usize> {
        let mut hits = hits;
        if q.dupe {
            let keep = crate::query::keep_shared(hits.len(), |k| Some(self.low(hits[k])));
            hits = keep.into_iter().map(|k| hits[k]).collect();
        }
        if q.size_dupe {
            let keep = crate::query::keep_shared(hits.len(), |k| {
                let i = hits[k];
                (self.flags[i] & DIR == 0).then_some(self.size[i])
            });
            hits = keep.into_iter().map(|k| hits[k]).collect();
        }
        hits
    }

    /// The key that puts copies next to each other in a `dupe:`/`sizedupe:` query (`None` for
    /// other queries): as [`crate::search::group_key`].
    pub fn group_key(&self, q: &Query, id: u32) -> Option<crate::query::GroupKey> {
        match q.group_by()? {
            crate::query::GroupBy::Name => Some(crate::query::GroupKey::Name(self.low(id as usize).to_vec())),
            crate::query::GroupBy::Size => (self.flags[id as usize] & DIR == 0).then(|| crate::query::GroupKey::Size(self.size[id as usize])),
        }
    }

    /// The root's path, lower-case and as it is, without a trailing `\`.
    fn root_bufs(&self, orig: bool) -> (Vec<u8>, Vec<u8>) {
        let root = self.root.to_string_lossy();
        let root = root.trim_end_matches('\\');
        (root.to_lowercase().into_bytes(), if orig { root.as_bytes().to_vec() } else { Vec::new() })
    }

    /// Add item `i`'s name to the paths of its parent.
    fn push_name(&self, i: usize, low: &mut Vec<u8>, opath: &mut Vec<u8>, orig: bool) {
        if self.flags[i] & ABS != 0 {
            let full = lookup(&self.abs, i).unwrap_or("").trim_end_matches('\\');
            low.clear();
            low.extend_from_slice(full.to_lowercase().as_bytes());
            if orig {
                opath.clear();
                opath.extend_from_slice(full.as_bytes());
            }
        } else {
            low.push(b'\\');
            low.extend_from_slice(self.low(i));
            if orig {
                opath.push(b'\\');
                opath.extend_from_slice(&self.orig_name(i));
            }
        }
    }

    /// The path words over items `a..b`.
    fn walk_run(&self, a: usize, b: usize, candidate: &[bool], terms: &[&Term], orig: bool, cancel: &AtomicBool) -> Option<Vec<u32>> {
        let (mut low, mut opath) = self.root_bufs(orig);
        // the folders above `a`, with the length of each one's path
        let mut chain = Vec::new();
        let mut p = self.parent[a];
        while p != u32::MAX && p != 0 {
            chain.push(p as usize);
            p = self.parent[p as usize];
        }
        let mut stack: Vec<(u32, usize, usize)> = vec![(0, low.len(), opath.len())];
        for &f in chain.iter().rev() {
            self.push_name(f, &mut low, &mut opath, orig);
            stack.push((f as u32, low.len(), opath.len()));
        }
        let mut keep = Vec::new();
        for i in a..b {
            if (i & 0x3FFF) == 0 && cancel.load(Ordering::Relaxed) {
                return None;
            }
            let p = self.parent[i];
            while stack.len() > 1 && stack.last().is_some_and(|s| s.0 != p) {
                stack.pop();
            }
            let &(_, len, olen) = stack.last()?;
            low.truncate(len);
            opath.truncate(olen);
            self.push_name(i, &mut low, &mut opath, orig);
            let dir = self.flags[i] & DIR != 0;
            if candidate[i] {
                if dir {
                    low.push(b'\\');
                    opath.push(b'\\');
                }
                let it = CItem { ix: self, i, path: &low, opath: &opath };
                if terms.iter().all(|t| t.matches(&it)) {
                    keep.push(i as u32);
                }
                if dir {
                    low.pop();
                    opath.pop();
                }
            }
            // the next item is inside this one when it is a folder (pushed whatever the flag
            // says: a file with children would otherwise give them the wrong path)
            if dir || self.parent.get(i + 1) == Some(&(i as u32)) {
                stack.push((i as u32, low.len(), opath.len()));
            }
        }
        Some(keep)
    }

    /// Every folder's place in the order of its full path, compared case-insensitively, as the
    /// Folder column sorts; files are `u32::MAX`.
    fn ranks(&self) -> &[u32] {
        self.ranks.get_or_init(|| {
            let n = self.len();
            let mut keys: Vec<(Box<[u8]>, u32)> = Vec::new();
            let mut slot = vec![u32::MAX; n];
            for i in 0..n {
                if self.flags[i] & DIR == 0 {
                    continue;
                }
                let key: Vec<u8> = if i == 0 {
                    self.root.to_string_lossy().to_lowercase().into_bytes()
                } else if self.flags[i] & ABS != 0 {
                    lookup(&self.abs, i).unwrap_or("").to_lowercase().into_bytes()
                } else {
                    let parent = slot[self.parent[i] as usize];
                    let mut p = keys.get(parent as usize).map_or(Vec::new(), |k| k.0.to_vec());
                    // as PathBuf::push joins them
                    if !p.ends_with(b"\\") && !p.ends_with(b"/") {
                        p.push(b'\\');
                    }
                    p.extend_from_slice(self.low(i));
                    p
                };
                slot[i] = keys.len() as u32;
                keys.push((key.into_boxed_slice(), i as u32));
            }
            drop(slot);
            keys.sort_unstable();
            let mut rank = vec![u32::MAX; n];
            // folders whose paths differ only in case share a place
            let mut place = 0u32;
            for (k, (key, i)) in keys.iter().enumerate() {
                if k > 0 && keys[k - 1].0 != *key {
                    place = k as u32;
                }
                rank[*i as usize] = place;
            }
            rank
        })
    }

    /// Sort `hits` (ids from [`CompactIndex::search`]). Equal keys keep their order.
    pub fn sort(&self, hits: &mut [u32], by: SortBy, descending: bool) {
        let dir = |o: std::cmp::Ordering| if descending { o.reverse() } else { o };
        match by {
            SortBy::Size => hits.sort_by(|&a, &b| dir(self.size[a as usize].cmp(&self.size[b as usize]))),
            SortBy::Modified => hits.sort_by(|&a, &b| dir(self.modified[a as usize].cmp(&self.modified[b as usize]))),
            SortBy::Name => hits.sort_by(|&a, &b| dir(self.low(a as usize).cmp(self.low(b as usize)))),
            SortBy::Folder => {
                let ranks = self.ranks();
                let rank = |i: u32| self.parent(i).map_or(0, |p| u64::from(ranks[p as usize]) + 1);
                hits.sort_by(|&a, &b| dir(rank(a).cmp(&rank(b)).then_with(|| self.low(a as usize).cmp(self.low(b as usize)))));
            }
        }
    }
}

/// An item as the query sees it.
struct CItem<'a> {
    ix: &'a CompactIndex,
    i: usize,
    path: &'a [u8],
    opath: &'a [u8],
}

impl Item for CItem<'_> {
    #[inline]
    fn name(&self) -> &[u8] {
        self.ix.low(self.i)
    }
    fn orig_name(&self) -> Cow<'_, [u8]> {
        self.ix.orig_name(self.i)
    }
    fn path(&self) -> &[u8] {
        self.path
    }
    fn orig_path(&self) -> &[u8] {
        self.opath
    }
    fn is_dir(&self) -> bool {
        self.ix.flags[self.i] & DIR != 0
    }
    fn compressed(&self) -> bool { self.ix.flags[self.i] & COMPRESSED != 0 }
    fn size(&self) -> u64 {
        self.ix.size[self.i]
    }
    fn modified(&self) -> Option<i64> {
        Some(self.ix.modified[self.i]).filter(|&m| m != NO_TIME)
    }
    fn depth(&self) -> u32 {
        // as the tree's index counts them
        u32::from(self.ix.depth[self.i]).min(254)
    }
    fn attributes(&self) -> u32 {
        let f = self.ix.flags[self.i];
        let mut a = 0;
        if f & HIDDEN != 0 {
            a |= crate::attr::HIDDEN;
        }
        if f & SYSTEM != 0 {
            a |= crate::attr::SYSTEM;
        }
        if f & READONLY != 0 {
            a |= crate::attr::READONLY;
        }
        if f & REPARSE != 0 {
            a |= crate::attr::REPARSE_POINT;
        }
        if f & COMPRESSED != 0 {
            a |= crate::attr::COMPRESSED;
        }
        a
    }
    fn children(&self) -> u32 {
        u32::try_from(self.ix.kids(self.i).count()).unwrap_or(u32::MAX)
    }
    fn child_files(&self) -> u32 {
        u32::try_from(self.ix.kids(self.i).filter(|&c| self.ix.flags[c] & DIR == 0).count()).unwrap_or(u32::MAX)
    }
    fn child_matches(&self, ts: &[Term]) -> bool {
        self.ix.kids(self.i).any(|c| {
            let it = CItem { ix: self.ix, i: c, path: &[], opath: &[] };
            ts.iter().all(|t| t.matches(&it))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{search, Index};
    use crate::Node;

    fn node(name: &str, is_dir: bool, size: u64, parent: Option<usize>, children: Vec<usize>, modified: Option<u64>) -> Node {
        let modified = modified.map(|s| UNIX_EPOCH + Duration::from_secs(s));
        Node { name: name.into(), is_dir, size, parent, children, modified, ..Default::default() }
    }

    const DAY: u64 = 86_400;
    const T0: u64 = 1_767_225_600; // 2026-01-01 UTC

    /// C:\Data with folders, mixed case, non-ASCII names, empty files, times, and a cut-off
    /// branch. Node order is not depth-first on purpose.
    fn sample() -> Tree {
        let nodes = vec![
            node("C:\\Data", true, 0, None, vec![9, 1, 5, 7, 10], Some(T0)),
            node("Games", true, 0, Some(0), vec![2], Some(T0 + 3 * DAY)),
            node("Steam", true, 0, Some(1), vec![3, 4, 11], Some(T0 + 30 * DAY)),
            node("config.vdf", false, 10, Some(2), vec![], Some(T0 + 40 * DAY)),
            node("Report 2024.PDF", false, 3000, Some(2), vec![], Some(T0 + 300 * DAY)),
            node("cache", true, 0, Some(0), vec![6, 12], None),
            node("temp.tmp", false, 5, Some(5), vec![], Some(T0 - 400 * DAY)),
            node("setup.EXE", false, 2 << 30, Some(0), vec![], Some(T0 + 100 * DAY)),
            node("cut off.txt", false, 7, None, vec![], Some(T0)),
            node("Über Ärger.TXT", false, 0, Some(0), vec![], Some(T0 + 200 * DAY)),
            node("Ünïcödé Földer", true, 0, Some(0), vec![13], Some(T0 + 5 * DAY)),
            node("README.md", false, 0, Some(2), vec![], None),
            node("Readme.MD", false, 1 << 20, Some(5), vec![], Some(T0 + 200 * DAY)),
            node("Photo.JPG", false, 500_000, Some(10), vec![], Some(T0 + 20 * DAY)),
        ];
        let mut t = Tree { volumes: Vec::new(), root_path: PathBuf::from("C:\\Data"), nodes, errors: 0, method: Default::default() };
        t.compute_totals();
        t
    }

    /// The Select Multiple shape: children named by their full paths.
    fn multiple() -> Tree {
        let nodes = vec![
            node(crate::MULTIPLE, true, 0, None, vec![1, 3], None),
            node("C:\\Users\\Me\\Downloads", true, 0, Some(0), vec![2], Some(T0)),
            node("Setup.ISO", false, 4 << 30, Some(1), vec![], Some(T0)),
            node("D:\\Backup\\", true, 0, Some(0), vec![4], Some(T0)),
            node("old.ZIP", false, 10, Some(3), vec![], Some(T0)),
        ];
        let mut t = Tree { volumes: Vec::new(), root_path: PathBuf::from(crate::MULTIPLE), nodes, errors: 0, method: Default::default() };
        t.compute_totals();
        t
    }

    const QUERIES: &[&str] = &[
        "e", "report 2024", "\"report 2024\"", "*.vdf", "c?nfig.*", "\\steam\\ *.vdf", "*.vdf|*.exe", "*.vdf;*.exe", "f !report",
        ">1gb", "file: <=10", "folder:cache", "file: cache", "\\steam\\", "folder: *\\games\\*", "data", ">=0", "<1", "readme",
        "case:README", "case:Readme", "exact:readme.md", "case:exact:README.md", "ext:md", "ext:txt;pdf", "ext:.exe|vdf",
        "path:steam", "path:games/steam", "/steam/ *.md", "case:\\Steam\\", "case:\\steam\\", "path:exact:c:\\data\\games\\steam",
        "regex:^re.*\\.md$", "case:regex:^README", "regex:\\\\steam\\\\.*\\.pdf", "case:regex:\\\\Steam\\\\", "size:empty",
        "size:1kb..1mb", "size:>1gb", "!size:empty", "depth:1", "depth:<=2", "depth:>=3", "dm:2026", "dm:<2026", "!dm:2026",
        "dm:2026-01..2026-02", "dm:>2026-06-01", "über", "ärger", "case:Über", "case:über", "ünïcödé", "ünïcödé\\photo",
        "case:Földer\\Photo", "folder:", "file:", "!e", "zzz", "*", "c:", "foo:bar", "\"!x\"", "dm:bad", "regex:(",
        "parent:c:\\data", "parent:steam", "parent:c:", "child:*.vdf", "child:cache", "child:ext:pdf;md",
        "childcount:2", "childcount:>1", "childfilecount:0", "childfilecount:>0", "folder: childcount:1",
        "<parent:c:\\data *.pdf>|*.exe", "!<parent:c:\\data file:>",
        "dupe:", "dupe: readme", "sizedupe:", "sizedupe: file:", "attrib:h", "attrib:c", "audio:", "doc:", "len:<20", "len:>20",
    ];

    fn tree_paths(t: &Tree, q: &Query) -> Vec<PathBuf> {
        let ix = Index::build(t);
        let mut v: Vec<PathBuf> = search(t, &ix, q, &AtomicBool::new(false)).unwrap().into_iter().map(|i| t.path_of(i)).collect();
        v.sort();
        v
    }

    fn compact_paths(c: &CompactIndex, q: &Query) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = c.search(q, &AtomicBool::new(false)).unwrap().into_iter().map(|i| c.path_of(i)).collect();
        v.sort();
        v
    }

    #[test]
    fn compressed_queries_agree_and_do_not_guess_from_links_or_sizes() {
        let mut t = sample();
        let files: Vec<usize> = t.nodes.iter().enumerate().filter(|(_, n)| !n.is_dir).map(|(i, _)| i).take(3).collect();
        t.nodes[files[0]].attributes = crate::attr::COMPRESSED;
        t.nodes[files[1]].attributes = crate::attr::REPARSE_POINT | crate::attr::WOF;
        t.nodes[files[2]].attributes = crate::attr::REPARSE_POINT;
        t.nodes[files[2]].allocated = 0;
        let c = CompactIndex::build(&t);
        for query in ["compressed:", "compressed:yes", "compressed:no", "!compressed:", "file: compressed:", "compressed:true|ext:vdf"] {
            let q = Query::parse(query);
            assert!(q.errors().is_empty(), "{query}");
            assert_eq!(compact_paths(&c, &q), tree_paths(&t, &q), "{query}");
        }
        assert_eq!(tree_paths(&t, &Query::parse("compressed:")).len(), 2);
        assert!(!Query::parse("compressed:unknown").errors().is_empty());
    }

    #[test]
    fn same_results_as_the_tree() {
        for (t, least) in [(sample(), 100), (multiple(), 20)] {
            let c = CompactIndex::build(&t);
            let now = UNIX_EPOCH + Duration::from_secs(T0 + 320 * DAY);
            let mut found = 0;
            for q in QUERIES {
                let q2 = Query::parse_at(q, now);
                let a = tree_paths(&t, &q2);
                assert_eq!(a, compact_paths(&c, &q2), "{q}");
                found += a.len();
            }
            assert!(found > least, "the queries find things: {found}");
        }
    }

    #[test]
    fn some_answers() {
        let t = sample();
        let c = CompactIndex::build(&t);
        let names = |q: &str| -> Vec<String> {
            let mut v: Vec<String> = c.search(&Query::parse(q), &AtomicBool::new(false)).unwrap().into_iter().map(|i| c.name(i)).collect();
            v.sort();
            v
        };
        assert_eq!(names("case:README"), ["README.md"]);
        assert_eq!(names("readme"), ["README.md", "Readme.MD"]);
        assert_eq!(names("ext:txt"), ["Über Ärger.TXT"]);
        assert_eq!(names("cut"), Vec::<String>::new(), "a cut-off branch is left out");
        assert_eq!(names("size:empty file:"), ["README.md", "Über Ärger.TXT"]);
        assert_eq!(names("depth:1 folder:"), ["Games", "cache", "Ünïcödé Földer"]);
    }

    #[test]
    fn duplicates_and_their_group_key() {
        let t = sample();
        let c = CompactIndex::build(&t);
        let names = |q: &str| -> Vec<String> {
            let mut v: Vec<String> = c.search(&Query::parse(q), &AtomicBool::new(false)).unwrap().into_iter().map(|i| c.name(i)).collect();
            v.sort();
            v
        };
        // README.md and Readme.MD: one name, any case
        assert_eq!(names("dupe:"), ["README.md", "Readme.MD"]);
        assert_eq!(names("dupe: readme"), ["README.md", "Readme.MD"]);
        assert!(names("dupe: config").is_empty(), "one of it is not a duplicate");
        // sizedupe: files only, and the two files of 0 bytes
        assert_eq!(names("sizedupe:"), ["README.md", "Über Ärger.TXT"]);
        assert!(names("sizedupe: ext:txt").is_empty(), "among the .txt files only one is 0 bytes: not a duplicate");
        // the group key puts copies together; folders never take part in sizedupe:
        let q = Query::parse("dupe:");
        let hits = c.search(&q, &AtomicBool::new(false)).unwrap();
        let keys: Vec<_> = hits.iter().filter_map(|&i| c.group_key(&q, i)).collect();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], keys[1]);
        let q = Query::parse("sizedupe:");
        let keys: Vec<_> = c.search(&q, &AtomicBool::new(false)).unwrap().iter().filter_map(|&i| c.group_key(&q, i)).collect();
        assert_eq!(keys, [crate::query::GroupKey::Size(0), crate::query::GroupKey::Size(0)]);
        assert_eq!(Query::parse("readme").group_by(), None);
    }

    #[test]
    fn accessors() {
        let t = sample();
        let c = CompactIndex::build(&t);
        assert_eq!(c.len(), 13, "the cut-off item is not kept");
        assert_eq!(c.root_path(), Path::new("C:\\Data"));
        for id in 0..c.len() as u32 {
            let p = c.path_of(id);
            let i = (0..t.nodes.len()).find(|&i| t.path_of(i) == p).expect("same path in the tree");
            let n = &t.nodes[i];
            assert_eq!(c.name(id), if i == ROOT { "Data" } else { n.name.as_str() });
            assert_eq!(c.size(id), n.size);
            assert_eq!(c.is_dir(id), n.is_dir);
            assert_eq!(c.modified(id), n.modified);
            assert_eq!(c.parent(id).map(|p| c.path_of(p)), n.parent.map(|p| t.path_of(p)));
        }
        assert_eq!(c.depth(0), 0);
        let m = multiple();
        let c = CompactIndex::build(&m);
        let mut all: Vec<PathBuf> = (1..c.len() as u32).map(|i| c.path_of(i)).collect();
        all.sort();
        let mut want: Vec<PathBuf> = (1..m.nodes.len()).map(|i| m.path_of(i)).collect();
        want.sort();
        assert_eq!(all, want);
        assert!(CompactIndex::build(&Tree::default()).search(&Query::parse("e"), &AtomicBool::new(false)).unwrap().is_empty());
    }

    #[test]
    fn sorts() {
        let t = sample();
        let c = CompactIndex::build(&t);
        let mut hits = c.search(&Query::parse("file:"), &AtomicBool::new(false)).unwrap();
        c.sort(&mut hits, SortBy::Size, true);
        assert_eq!(c.name(hits[0]), "setup.EXE");
        assert!(hits.windows(2).all(|w| c.size(w[0]) >= c.size(w[1])));
        c.sort(&mut hits, SortBy::Name, false);
        let names: Vec<String> = hits.iter().map(|&i| c.name(i).to_lowercase()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        c.sort(&mut hits, SortBy::Modified, false);
        assert!(hits.windows(2).all(|w| c.modified(w[0]) <= c.modified(w[1])));
        assert_eq!(c.modified(hits[0]), None, "no time sorts first");
        // by folder: the folders' paths in order, any case, then the name
        c.sort(&mut hits, SortBy::Folder, false);
        let keys: Vec<(String, String)> =
            hits.iter().map(|&i| (c.path_of(c.parent(i).unwrap()).to_string_lossy().to_lowercase(), c.name(i).to_lowercase())).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        c.sort(&mut hits, SortBy::Folder, true);
        let mut rev = keys.clone();
        rev.reverse();
        let keys2: Vec<(String, String)> =
            hits.iter().map(|&i| (c.path_of(c.parent(i).unwrap()).to_string_lossy().to_lowercase(), c.name(i).to_lowercase())).collect();
        assert_eq!(keys2, rev);
    }

    #[test]
    fn memory_is_small() {
        let t = sample();
        let c = CompactIndex::build(&t);
        let names: usize = (0..c.len()).map(|i| c.low(i).len()).sum();
        // 26 bytes an item plus the names and their capitals, plus the non-ASCII names again
        assert!(c.memory() <= c.len() * 30 + names * 2 + 3 * 64, "{} bytes", c.memory());
    }

    /// A node not marked as a folder that still has children (never from a scan): what is
    /// inside it gets the same path as in the tree.
    #[test]
    fn file_with_children() {
        let mut nodes = vec![
            node("C:\\Data", true, 0, None, vec![1], Some(T0)),
            node("odd", false, 0, Some(0), vec![], Some(T0)),
        ];
        // enough items that a run of the path walk holds many of them
        for k in 0..2000usize {
            let id = nodes.len();
            let p = if k < 1000 { 1 } else { 0 };
            nodes[p].children.push(id);
            nodes.push(node(&format!("{}{k}.txt", if p == 1 { "inner" } else { "after" }), false, 1, Some(p), vec![], Some(T0)));
        }
        let t = Tree { volumes: Vec::new(), root_path: PathBuf::from("C:\\Data"), nodes, errors: 0, method: Default::default() };
        let c = CompactIndex::build(&t);
        for q in ["\\odd\\", "path:inner", "\\data\\after", "path:txt", "\\odd\\inner99"] {
            let q = Query::parse(q);
            let a = tree_paths(&t, &q);
            assert!(!a.is_empty());
            assert_eq!(a, compact_paths(&c, &q));
        }
    }

    /// Many items and runs: the path words see the right paths where runs start mid-folder.
    #[test]
    fn long_runs() {
        let mut nodes = vec![node("C:\\", true, 0, None, vec![], Some(T0))];
        for d in 0..50usize {
            let id = nodes.len();
            nodes[0].children.push(id);
            nodes.push(node(&format!("Dir{d}"), true, 0, Some(0), vec![], Some(T0)));
            let sub = nodes.len();
            nodes[id].children.push(sub);
            nodes.push(node("Sub", true, 0, Some(id), vec![], Some(T0)));
            for f in 0..40usize {
                let fid = nodes.len();
                let (p, name) = if f % 2 == 0 { (id, format!("File{f}.Txt")) } else { (sub, format!("Deep{f}.LOG")) };
                nodes[p].children.push(fid);
                nodes.push(node(&name, false, f as u64, Some(p), vec![], Some(T0 + f as u64 * DAY)));
            }
        }
        let mut t = Tree { volumes: Vec::new(), root_path: PathBuf::from("C:\\"), nodes, errors: 0, method: Default::default() };
        t.compute_totals();
        let c = CompactIndex::build(&t);
        for q in ["\\dir1\\", "\\dir17\\sub\\ *.log", "case:\\Dir3\\Sub\\Deep", "path:dir4", "regex:\\\\dir4\\d\\\\", "\\sub\\"] {
            let q = Query::parse(q);
            let a = tree_paths(&t, &q);
            assert!(!a.is_empty());
            assert_eq!(a, compact_paths(&c, &q));
        }
    }
}
