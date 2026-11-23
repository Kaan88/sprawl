//! Instant name search over a scanned tree (the Search tab).
//!
//! [`Index::build`] keeps every name once, lower-case, in one block of bytes (about 25 bytes a
//! name), so a search is a plain scan over that block split across the cores: no second tree,
//! nothing to keep in step. [`Query::parse`] reads the search box; the language is described
//! in [`crate::query`] and listed for users in [`HELP`]. In short:
//!
//! * `report 2024`: both words, anywhere in the name, any case (a space means "and");
//! * `"my file"`: quotes keep a space;
//! * `*.iso`, `img_????.jpg`: `*` and `?`, matched against the whole name as in the filter box;
//! * `\steam\ *.vdf`: a word with `\` (or `/`) is matched against the full path (folders end
//!   in `\`);
//! * `a|b` (or `a;b`): either; `!tmp`: not;
//! * `>1gb`, `<10kb`, `>=5mb`: size (1 kb = 1024 bytes; a folder's size is its total);
//! * `folder:`, `file:`: one kind only (also `folder:cache`);
//! * `ext:`, `dm:`, `path:`, `size:`, `regex:`, `case:`, `exact:`, `depth:`: see [`HELP`].
//!
//! The rules for `*`, `?`, `|` and `\` are the filter box's (see [`crate::filter`]), so a
//! pattern means the same in both places.
//!
//! Drives read only to be searched can use [`crate::compact::CompactIndex`] instead of a tree
//! and this index: the same queries, in a fraction of the memory.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::query::{unix_secs, Item, Term, NEED_CASE_PATH};
pub use crate::query::{Kind, Query, HELP};
use crate::{Tree, ROOT};

/// Every name of a tree, lower-case, back to back.
#[derive(Debug, Default)]
pub struct Index {
    blob: Vec<u8>,
    /// Where node `i`'s name starts; `starts[i + 1]` is where it ends.
    starts: Vec<u32>,
    /// 0: not reachable from the root (a deleted branch stays in the node list, cut off from
    /// its parent); otherwise one more than the levels below the root (the root is 1), at
    /// most 255.
    depth: Vec<u8>,
}

/// The last part of a node's name: a "Select Multiple" child is named by its full path.
pub(crate) fn last_name(name: &str) -> &str {
    let name = name.trim_end_matches('\\');
    name.rsplit('\\').next().unwrap_or(name)
}

impl Index {
    /// Build the index of `tree` (about 0.2 s for 3.7 million names; run it off the UI thread).
    pub fn build(tree: &Tree) -> Index {
        let n = tree.nodes.len();
        let mut blob = Vec::with_capacity(n * 24);
        let mut starts = Vec::with_capacity(n + 1);
        for node in &tree.nodes {
            starts.push(blob.len() as u32);
            let name = last_name(&node.name);
            if name.is_ascii() {
                blob.extend(name.bytes().map(|b| b.to_ascii_lowercase()));
            } else {
                blob.extend_from_slice(name.to_lowercase().as_bytes());
            }
        }
        starts.push(blob.len() as u32);
        blob.shrink_to_fit();
        let mut depth = vec![0u8; n];
        if n > 0 {
            let mut stack = vec![(ROOT, 1u8)];
            while let Some((i, d)) = stack.pop() {
                if i < n && depth[i] == 0 {
                    depth[i] = d;
                    let next = d.saturating_add(1);
                    stack.extend(tree.nodes[i].children.iter().map(|&c| (c, next)));
                }
            }
        }
        Index { blob, starts, depth }
    }

    pub fn len(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes the index holds.
    pub fn memory(&self) -> usize {
        self.blob.capacity() + self.starts.capacity() * 4 + self.depth.capacity()
    }

    /// Node `i`'s lower-case name.
    fn name(&self, i: usize) -> &[u8] {
        &self.blob[self.starts[i] as usize..self.starts[i + 1] as usize]
    }
}

/// A node as the query sees it.
struct NodeItem<'a> {
    tree: &'a Tree,
    index: &'a Index,
    i: usize,
    path: &'a [u8],
    opath: &'a [u8],
}

impl Item for NodeItem<'_> {
    #[inline]
    fn name(&self) -> &[u8] {
        self.index.name(self.i)
    }
    fn orig_name(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(last_name(&self.tree.nodes[self.i].name).as_bytes())
    }
    fn path(&self) -> &[u8] {
        self.path
    }
    fn orig_path(&self) -> &[u8] {
        self.opath
    }
    fn is_dir(&self) -> bool {
        self.tree.nodes[self.i].is_dir
    }
    fn compressed(&self) -> bool { self.tree.nodes[self.i].is_compressed() }
    fn size(&self) -> u64 {
        self.tree.nodes[self.i].size
    }
    fn modified(&self) -> Option<i64> {
        self.tree.nodes[self.i].modified.map(unix_secs)
    }
    fn depth(&self) -> u32 {
        u32::from(self.index.depth[self.i]).saturating_sub(1)
    }
    fn attributes(&self) -> u32 {
        self.tree.nodes[self.i].attributes
    }
    fn children(&self) -> u32 {
        u32::try_from(self.tree.nodes[self.i].children.len()).unwrap_or(u32::MAX)
    }
    fn child_files(&self) -> u32 {
        let t = self.tree;
        u32::try_from(t.nodes[self.i].children.iter().filter(|&&c| c < t.nodes.len() && !t.nodes[c].is_dir).count()).unwrap_or(u32::MAX)
    }
    fn child_matches(&self, ts: &[Term]) -> bool {
        let tree = self.tree;
        tree.nodes[self.i].children.iter().any(|&c| {
            // the index can be shorter than the tree (`search` works on the shorter): a child
            // past its end has no name to test
            if c >= tree.nodes.len() || c >= self.index.len() {
                return false;
            }
            let it = NodeItem { tree, index: self.index, i: c, path: &[], opath: &[] };
            ts.iter().all(|t| t.matches(&it))
        })
    }
}

/// How many threads a search uses.
pub(crate) fn threads() -> usize {
    std::thread::available_parallelism().map_or(4, |c| c.get()).clamp(1, 16)
}

/// The items `1..n` for which `keep` holds, in order, split across the cores. `None` when
/// `cancel` was set on the way.
pub(crate) fn par_filter(n: usize, cancel: &AtomicBool, keep: impl Fn(usize) -> bool + Sync) -> Option<Vec<usize>> {
    let threads = threads();
    let chunk = n.div_ceil(threads);
    let parts: Vec<Option<Vec<usize>>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|k| {
                let keep = &keep;
                s.spawn(move || {
                    let (a, b) = ((k * chunk).max(1), ((k + 1) * chunk).min(n));
                    let mut out = Vec::new();
                    for i in a..b {
                        if (i & 0xFFFF) == 0 && cancel.load(Ordering::Relaxed) {
                            return None;
                        }
                        if keep(i) {
                            out.push(i);
                        }
                    }
                    Some(out)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or(None)).collect()
    });
    let mut hits = Vec::new();
    for p in parts {
        hits.extend(p?);
    }
    Some(hits)
}

/// The nodes of `tree` that match `q` (never the root), in tree order. `None` when `cancel`
/// was set on the way (a newer search started). A query with [`Query::errors`] finds nothing.
pub fn search(tree: &Tree, index: &Index, q: &Query, cancel: &AtomicBool) -> Option<Vec<usize>> {
    let n = tree.nodes.len().min(index.len());
    if n <= 1 || q.is_empty() || !q.errors().is_empty() {
        return Some(Vec::new());
    }
    // sizes, dates and depths, then names; the path words after
    let (early, path_terms) = q.split_terms();
    // a node is a 176-byte record: read it only when the query asks about it (a plain word
    // looks at the index alone, as fast as before the keywords were added)
    let sized = q.min.is_some() || q.max.is_some();
    let cheap = |i: usize| -> bool {
        if index.depth[i] == 0 {
            return false;
        }
        let node = &tree.nodes[i];
        match q.kind {
            Kind::Any => {}
            Kind::Files if node.is_dir => return false,
            Kind::Folders if !node.is_dir => return false,
            _ => {}
        }
        if sized && !q.size_ok(node.size) {
            return false;
        }
        let it = NodeItem { tree, index, i, path: &[], opath: &[] };
        early.iter().all(|t| t.matches(&it))
    };

    // the cheap part on every core, then the path words, then `dupe:`/`sizedupe:`: the copies
    // only.
    let hits = par_filter(n, cancel, cheap)?;
    let mut hits = if path_terms.is_empty() { hits } else { path_matches(tree, index, &path_terms, hits, n, cancel)? };
    // `dupe:` and `sizedupe:`: among the matches of everything else
    if q.dupe {
        let keep = crate::query::keep_shared(hits.len(), |k| Some(index.name(hits[k])));
        hits = keep.into_iter().map(|k| hits[k]).collect();
    }
    if q.size_dupe {
        let keep = crate::query::keep_shared(hits.len(), |k| {
            let i = hits[k];
            (!tree.nodes[i].is_dir).then_some(tree.nodes[i].size)
        });
        hits = keep.into_iter().map(|k| hits[k]).collect();
    }
    Some(hits)
}

/// The key that puts copies next to each other in a `dupe:`/`sizedupe:` query (`None` for other
/// queries, and for a folder under `sizedupe:`): a sort can use it before its own column.
pub fn group_key(tree: &Tree, index: &Index, q: &Query, n: usize) -> Option<crate::query::GroupKey> {
    match q.group_by()? {
        crate::query::GroupBy::Name => Some(crate::query::GroupKey::Name(index.name(n).to_vec())),
        crate::query::GroupBy::Size => (!tree.nodes[n].is_dir).then(|| crate::query::GroupKey::Size(tree.nodes[n].size)),
    }
}

/// The path words over the tree, for the candidates in `hits`.
fn path_matches(
    tree: &Tree,
    index: &Index,
    path_terms: &[&Term],
    hits: Vec<usize>,
    n: usize,
    cancel: &AtomicBool,
) -> Option<Vec<usize>> {
    let threads = threads();

    // the path words: walk the tree with a path buffer, checking only the candidates; the
    // top of the tree is split into pieces that the cores walk side by side
    let mut candidate = vec![false; n];
    for &i in &hits {
        candidate[i] = true;
    }
    let orig = path_terms.iter().any(|t| t.needs & NEED_CASE_PATH != 0);
    let walker = PathWalk { tree, index, terms: path_terms, candidate: &candidate, n, orig };
    let root = tree.root_path.to_string_lossy();
    let root = root.trim_end_matches('\\');
    let root = Bufs { low: root.to_lowercase().into_bytes(), orig: if orig { root.as_bytes().to_vec() } else { Vec::new() } };
    let mut keep = Vec::new();
    let mut items: Vec<(usize, Bufs)> = vec![(ROOT, root)];
    while items.len() < threads * 16 {
        let mut next = Vec::new();
        for (i, mut path) in std::mem::take(&mut items) {
            walker.check(i, &mut path, &mut keep);
            for &c in &tree.nodes[i].children {
                if c >= n {
                    continue;
                }
                let mut p = path.clone();
                walker.push_name(c, &mut p);
                if tree.nodes[c].children.is_empty() {
                    walker.check(c, &mut p, &mut keep);
                } else {
                    next.push((c, p));
                }
            }
        }
        if next.is_empty() {
            break;
        }
        items = next;
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let parts: Vec<Option<Vec<usize>>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let (walker, items, next) = (&walker, &items, &next);
                s.spawn(move || {
                    let mut out = Vec::new();
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some((i, path)) = items.get(k) else { return Some(out) };
                        walker.walk(*i, path.clone(), &mut out, cancel)?;
                    }
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or(None)).collect()
    });
    for p in parts {
        keep.extend(p?);
    }
    keep.sort_unstable();
    Some(keep)
}

/// A path being built: lower-case, and as on disk when a word needs that.
#[derive(Clone)]
struct Bufs {
    low: Vec<u8>,
    orig: Vec<u8>,
}

struct PathWalk<'a> {
    tree: &'a Tree,
    index: &'a Index,
    terms: &'a [&'a Term],
    candidate: &'a [bool],
    n: usize,
    /// Build the path as on disk too (for a `case:` path word).
    orig: bool,
}

impl PathWalk<'_> {
    /// Add node `i`'s name to `path` (its parent's path).
    fn push_name(&self, i: usize, path: &mut Bufs) {
        let name = &self.tree.nodes[i].name;
        if std::path::Path::new(name).is_absolute() {
            // a "Select Multiple" child is named by its full path
            let full = name.trim_end_matches('\\');
            path.low.clear();
            path.low.extend_from_slice(full.to_lowercase().as_bytes());
            if self.orig {
                path.orig.clear();
                path.orig.extend_from_slice(full.as_bytes());
            }
        } else {
            path.low.push(b'\\');
            path.low.extend_from_slice(self.index.name(i));
            if self.orig {
                path.orig.push(b'\\');
                path.orig.extend_from_slice(last_name(name).as_bytes());
            }
        }
    }

    /// `path` is node `i`'s: keep `i` if it is a candidate and the path words match.
    fn check(&self, i: usize, path: &mut Bufs, keep: &mut Vec<usize>) {
        if i == ROOT || !self.candidate[i] {
            return;
        }
        let dir = self.tree.nodes[i].is_dir;
        if dir {
            path.low.push(b'\\');
            if self.orig {
                path.orig.push(b'\\');
            }
        }
        let it = NodeItem { tree: self.tree, index: self.index, i, path: &path.low, opath: &path.orig };
        if self.terms.iter().all(|t| t.matches(&it)) {
            keep.push(i);
        }
        if dir {
            path.low.pop();
            if self.orig {
                path.orig.pop();
            }
        }
    }

    /// Everything from `start` down (`path` is `start`'s).
    fn walk(&self, start: usize, path: Bufs, keep: &mut Vec<usize>, cancel: &AtomicBool) -> Option<()> {
        let mut buf = path;
        let mut stack: Vec<(usize, usize, usize)> = Vec::new();
        let base = (buf.low.len(), buf.orig.len());
        self.check(start, &mut buf, keep);
        for &c in self.tree.nodes[start].children.iter().rev() {
            if c < self.n {
                stack.push((c, base.0, base.1));
            }
        }
        let mut seen = 0usize;
        while let Some((i, len, olen)) = stack.pop() {
            seen += 1;
            if (seen & 0x3FFF) == 0 && cancel.load(Ordering::Relaxed) {
                return None;
            }
            buf.low.truncate(len);
            buf.orig.truncate(olen);
            self.push_name(i, &mut buf);
            self.check(i, &mut buf, keep);
            let here = (buf.low.len(), buf.orig.len());
            for &c in self.tree.nodes[i].children.iter().rev() {
                if c < self.n {
                    stack.push((c, here.0, here.1));
                }
            }
        }
        Some(())
    }
}

/// `s` lowercased, as bytes (ASCII quickly; other names as `str::to_lowercase`).
pub fn lower_bytes(s: &str, out: &mut Vec<u8>) {
    if s.is_ascii() {
        out.extend(s.bytes().map(|c| c.to_ascii_lowercase()));
    } else {
        out.extend_from_slice(s.to_lowercase().as_bytes());
    }
}

/// The place of every folder in the order of its full path, compared case-insensitively (as
/// lowercase) as the Folder column sorts: `rank[folder] < rank[other]` when its path sorts first. Each path
/// is built once, by a walk from the root; a sort by folder is then a sort of numbers (a few
/// hundred ms for millions of rows, not half a minute of paths built per comparison). Files
/// are `u32::MAX`.
pub fn folder_ranks(tree: &Tree) -> Vec<u32> {
    let mut keys: Vec<(Box<[u8]>, u32)> = Vec::new();
    let mut root: Vec<u8> = Vec::new();
    lower_bytes(&tree.root_path.to_string_lossy(), &mut root);
    // (node, its lowercased path)
    let mut stack: Vec<(usize, Vec<u8>)> = vec![(crate::ROOT, root)];
    while let Some((n, path)) = stack.pop() {
        for &c in &tree.nodes[n].children {
            let child = &tree.nodes[c];
            if !child.is_dir {
                continue;
            }
            let mut p = Vec::with_capacity(path.len() + 1 + child.name.len());
            p.extend_from_slice(&path);
            // as PathBuf::push joins them
            if !p.ends_with(b"\\") && !p.ends_with(b"/") {
                p.push(b'\\');
            }
            lower_bytes(&child.name, &mut p);
            stack.push((c, p));
        }
        keys.push((path.into_boxed_slice(), n as u32));
    }
    keys.sort_unstable();
    let mut rank = vec![u32::MAX; tree.nodes.len()];
    // folders whose paths differ only in case (a case-sensitive folder) share a place, as the
    // comparison of their paths would have it
    let mut place = 0u32;
    for (i, (k, n)) in keys.iter().enumerate() {
        if i > 0 && keys[i - 1].0 != *k {
            place = i as u32;
        }
        rank[*n as usize] = place;
    }
    rank
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{filter_tree, Node};

    #[test]
    fn folders_rank_in_path_order() {
        let t = sample();
        let r = folder_ranks(&t);
        // C:\Data, C:\Data\cache, C:\Data\Games, C:\Data\Games\Steam (case ignored)
        let mut dirs: Vec<usize> = (0..t.nodes.len()).filter(|&i| t.nodes[i].is_dir).collect();
        dirs.sort_by_key(|&i| r[i]);
        let names: Vec<&str> = dirs.iter().map(|&i| t.nodes[i].name.as_str()).collect();
        assert_eq!(names, ["C:\\Data", "cache", "Games", "Steam"]);
        // the same order as comparing the paths themselves
        let mut by_path = dirs.clone();
        by_path.sort_by_key(|&i| t.path_of(i).to_string_lossy().to_ascii_lowercase());
        assert_eq!(dirs, by_path);
        assert_eq!(r[3], u32::MAX, "a file has no folder rank");
    }
    use std::path::PathBuf;

    fn node(name: &str, is_dir: bool, size: u64, parent: Option<usize>, children: Vec<usize>) -> Node {
        Node { name: name.into(), is_dir, size, parent, children, ..Default::default() }
    }

    /// C:\Data
    ///   Games\Steam\config.vdf (10, hidden and system), Games\Steam\Report 2024.PDF (3000, read-only)
    ///   cache\ (folder, hidden and system) with temp.tmp (5, a link)
    ///   setup.exe (2 GB)
    fn sample() -> Tree {
        let mut nodes = vec![
            node("C:\\Data", true, 0, None, vec![1, 5, 7]),
            node("Games", true, 0, Some(0), vec![2]),
            node("Steam", true, 0, Some(1), vec![3, 4]),
            node("config.vdf", false, 10, Some(2), vec![]),
            node("Report 2024.PDF", false, 3000, Some(2), vec![]),
            node("cache", true, 0, Some(0), vec![6]),
            node("temp.tmp", false, 5, Some(5), vec![]),
            node("setup.exe", false, 2 << 30, Some(0), vec![]),
        ];
        // the flags `attrib:` asks about, as a scan keeps them
        nodes[3].attributes = crate::attr::HIDDEN | crate::attr::SYSTEM;
        nodes[4].attributes = crate::attr::READONLY;
        nodes[5].attributes = crate::attr::HIDDEN | crate::attr::SYSTEM;
        nodes[6].attributes = crate::attr::REPARSE_POINT;
        let mut t = Tree { volumes: Vec::new(), root_path: PathBuf::from("C:\\Data"), nodes, errors: 0, method: Default::default() };
        t.compute_totals();
        t
    }

    fn find(t: &Tree, q: &str) -> Vec<String> {
        let ix = Index::build(t);
        search(t, &ix, &Query::parse(q), &AtomicBool::new(false)).unwrap().into_iter().map(|i| t.nodes[i].name.clone()).collect()
    }

    #[test]
    fn syntax() {
        let t = sample();
        assert_eq!(find(&t, "report 2024"), ["Report 2024.PDF"]);
        assert_eq!(find(&t, "\"report 2024\""), ["Report 2024.PDF"]);
        assert!(find(&t, "\"report  2024\"").is_empty());
        assert_eq!(find(&t, "*.vdf"), ["config.vdf"]);
        assert_eq!(find(&t, "c?nfig.*"), ["config.vdf"]);
        assert_eq!(find(&t, "\\steam\\ *.vdf"), ["config.vdf"]);
        let mut both = find(&t, "*.vdf|*.exe");
        both.sort();
        assert_eq!(both, ["config.vdf", "setup.exe"]);
        assert_eq!(find(&t, "*.vdf;*.exe").len(), 2);
        assert_eq!(find(&t, "f !report"), ["config.vdf"]);
        assert_eq!(find(&t, ">1gb"), ["setup.exe"]);
        assert_eq!(find(&t, "file: <=10"), ["config.vdf", "temp.tmp"]);
        assert_eq!(find(&t, "folder:cache"), ["cache"]);
        assert_eq!(find(&t, "file: cache"), Vec::<String>::new());
        // folders end in \ for path words: the folder itself and what is inside
        let mut steam = find(&t, "\\steam\\");
        steam.sort();
        assert_eq!(steam, ["Report 2024.PDF", "Steam", "config.vdf"]);
        assert_eq!(find(&t, "folder: *\\games\\*"), ["Games", "Steam"]);
        // the root is never a result
        assert!(!find(&t, "data").contains(&"C:\\Data".to_string()));
    }

    /// The shared patterns find the same files as the filter box.
    #[test]
    fn same_as_filter() {
        let t = sample();
        for pat in ["*.vdf", "setup", "*\\games\\*", "*.vdf|*.tmp", "REPORT"] {
            let f = filter_tree(&t, pat);
            let mut a: Vec<String> = f.nodes.iter().filter(|n| !n.is_dir).map(|n| n.name.clone()).collect();
            let mut b: Vec<String> = find(&t, &format!("file: {pat}"));
            a.sort();
            b.sort();
            assert_eq!(a, b, "{pat}");
        }
    }

    /// `parent:`, `child:` and the counts over a real tree: the direct children, a folder
    /// holding a match, and how many are directly inside.
    #[test]
    fn folder_shape() {
        let t = sample();
        assert_eq!(find(&t, "parent:c:\\data"), ["Games", "cache", "setup.exe"]);
        let mut steam = find(&t, "parent:steam");
        steam.sort();
        assert_eq!(steam, ["Report 2024.PDF", "config.vdf"]);
        assert_eq!(find(&t, "parent:c:\\data file:"), ["setup.exe"]);
        assert_eq!(find(&t, "parent:c:\\data"), find(&t, "parent:C:/Data/"));
        assert!(find(&t, "parent:c:\\").is_empty(), "the root is never a result");
        assert_eq!(find(&t, "child:*.vdf"), ["Steam"]);
        assert_eq!(find(&t, "child:steam"), ["Games"]);
        assert!(find(&t, "child:*.iso").is_empty());
        assert_eq!(find(&t, "childcount:2"), ["Steam"]);
        assert_eq!(find(&t, "childfilecount:2"), ["Steam"]);
        assert_eq!(find(&t, "folder: childfilecount:1"), ["cache"]);
        assert_eq!(find(&t, "childcount:>1 folder:"), ["Steam"]);
        // a group keeps them words of their own
        assert_eq!(find(&t, "<parent:c:\\data *.exe>|zzz"), ["setup.exe"]);
    }

    #[test]
    fn deleted_branches_are_skipped() {
        let mut t = sample();
        t.nodes[0].children.retain(|&c| c != 1); // Games cut off, as a delete leaves it
        assert!(find(&t, "*.vdf").is_empty());
        assert_eq!(find(&t, "setup"), ["setup.exe"]);
    }

    /// `attrib:` on the scan on screen reads the node's flags: the tree's own searcher used to
    /// answer nothing for all but `attrib:c`, so `!attrib:h` listed the hidden files too.
    #[test]
    fn attributes_match_the_scan() {
        let t = sample();
        let mut hidden = find(&t, "attrib:h");
        hidden.sort();
        assert_eq!(hidden, ["cache", "config.vdf"]);
        assert_eq!(find(&t, "attrib:hs"), find(&t, "attrib:h"));
        assert_eq!(find(&t, "attrib:r"), ["Report 2024.PDF"]);
        assert_eq!(find(&t, "attrib:l"), ["temp.tmp"]);
        let mut not_r = find(&t, "!attrib:r");
        not_r.sort();
        assert_eq!(not_r, ["Games", "Steam", "cache", "config.vdf", "setup.exe", "temp.tmp"]);
        assert_eq!(find(&t, "!attrib:h").len(), 5, "the hidden ones are left out");
    }

    /// A child past the end of the index is not asked its name: `search` works on the shorter
    /// of the tree and the index, and a worker's panic would look like a cancelled search.
    #[test]
    fn a_child_past_the_index_is_not_read() {
        let t = sample();
        let mut ix = Index::build(&t);
        ix.starts.truncate(3);
        ix.depth.truncate(2);
        let hits = search(&t, &ix, &Query::parse("child:config.vdf"), &AtomicBool::new(false)).unwrap();
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn cancelled() {
        let t = sample();
        let ix = Index::build(&t);
        // small trees finish before the first check; a cancel only matters for big ones
        assert!(search(&t, &ix, &Query::parse("e"), &AtomicBool::new(true)).is_some());
    }

    /// `cargo test --release -p sprawl-core bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench() {
        let n = 3_400_000usize;
        let mut nodes = Vec::with_capacity(n);
        nodes.push(node("C:\\", true, 0, None, Vec::new()));
        let dirs = 200_000usize;
        for d in 1..=dirs {
            nodes.push(node(&format!("folder_{d:06}_Program Data"), true, 0, Some(0), Vec::new()));
            nodes[0].children.push(d);
        }
        for f in dirs + 1..n {
            let p = 1 + (f * 7919) % dirs;
            nodes.push(node(&format!("Some File Name {f} v{}.dll", f % 97), false, (f as u64 * 31) % 100_000, Some(p), Vec::new()));
            nodes[p].children.push(f);
        }
        let t = Tree { volumes: Vec::new(), root_path: PathBuf::from("C:\\"), nodes, errors: 0, method: Default::default() };
        let start = std::time::Instant::now();
        let ix = Index::build(&t);
        println!("index: {:?}, {} MB", start.elapsed(), ix.memory() >> 20);
        for q in ["v42", "file name 12345", "*.dll", "\\folder_000123_program data\\", ">90kb v1", "childcount:>5", "len:>80", "dupe:"] {
            let start = std::time::Instant::now();
            let r = search(&t, &ix, &Query::parse(q), &AtomicBool::new(false)).unwrap();
            println!("{q:>34}: {:>8} hits in {:?}", r.len(), start.elapsed());
        }
    }
}
