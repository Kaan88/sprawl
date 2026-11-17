//! Instant name search over a scanned tree (the Search tab).
//!
//! [`Index::build`] keeps every name once, lower-case, in one block of bytes (about 25 bytes a
//! name), so a search is a plain scan over that block split across the cores: no second tree,
//! nothing to keep in step. [`Query::parse`] reads the search box:
//!
//! * `report 2024`: both words, anywhere in the name, any case (a space means "and");
//! * `"my file"`: quotes keep a space;
//! * `*.iso`, `img_????.jpg`: `*` and `?`, matched against the whole name as in the filter box;
//! * `\steam\ *.vdf`: a word with `\` is matched against the full path (folders end in `\`);
//! * `a|b` (or `a;b`): either; `!tmp`: not;
//! * `>1gb`, `<10kb`, `>=5mb`: size (1 kb = 1024 bytes; a folder's size is its total);
//! * `folder:`, `file:`: one kind only (also `folder:cache`).
//!
//! The rules for `*`, `?`, `|` and `\` are the filter box's (see [`crate::filter`]), so a
//! pattern means the same in both places.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::filter::wildcard;
use crate::{Tree, ROOT};

/// Every name of a tree, lower-case, back to back.
#[derive(Debug, Default)]
pub struct Index {
    blob: Vec<u8>,
    /// Where node `i`'s name starts; `starts[i + 1]` is where it ends.
    starts: Vec<u32>,
    /// Reachable from the root: a deleted branch stays in the node list, cut off from its parent.
    live: Vec<bool>,
}

impl Index {
    /// Build the index of `tree` (about 1 s for 3.4 million names; run it off the UI thread).
    pub fn build(tree: &Tree) -> Index {
        let n = tree.nodes.len();
        let mut blob = Vec::with_capacity(n * 24);
        let mut starts = Vec::with_capacity(n + 1);
        for node in &tree.nodes {
            starts.push(blob.len() as u32);
            // a "Select Multiple" child is named by its full path: its own name is the last part
            let name = node.name.trim_end_matches('\\');
            let name = name.rsplit('\\').next().unwrap_or(name);
            if name.is_ascii() {
                blob.extend(name.bytes().map(|b| b.to_ascii_lowercase()));
            } else {
                blob.extend_from_slice(name.to_lowercase().as_bytes());
            }
        }
        starts.push(blob.len() as u32);
        blob.shrink_to_fit();
        let mut live = vec![false; n];
        if n > 0 {
            let mut stack = vec![ROOT];
            while let Some(i) = stack.pop() {
                if i < n && !live[i] {
                    live[i] = true;
                    stack.extend_from_slice(&tree.nodes[i].children);
                }
            }
        }
        Index { blob, starts, live }
    }

    pub fn len(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes the index holds.
    pub fn memory(&self) -> usize {
        self.blob.capacity() + self.starts.capacity() * 4 + self.live.capacity()
    }

    /// Node `i`'s lower-case name.
    fn name(&self, i: usize) -> &[u8] {
        &self.blob[self.starts[i] as usize..self.starts[i + 1] as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Any,
    Files,
    Folders,
}

#[derive(Debug, Clone, PartialEq)]
enum Pat {
    /// Anywhere in the name.
    Sub(Vec<u8>),
    /// The whole name, with `*` and `?`.
    Glob(Vec<char>),
    /// Anywhere in the full path.
    PathSub(Vec<u8>),
    /// The whole path, with `*` and `?`.
    PathGlob(Vec<char>),
}

impl Pat {
    fn on_path(&self) -> bool {
        matches!(self, Pat::PathSub(_) | Pat::PathGlob(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Term {
    not: bool,
    alts: Vec<Pat>,
}

impl Term {
    fn on_path(&self) -> bool {
        self.alts.iter().any(Pat::on_path)
    }

    /// `name` lower-case; `path` lower-case with folders ending in `\` (only for path terms).
    fn matches(&self, name: &[u8], path: &[u8]) -> bool {
        let hit = self.alts.iter().any(|p| match p {
            Pat::Sub(s) => contains(name, s),
            Pat::Glob(g) => std::str::from_utf8(name).is_ok_and(|n| wildcard(g, n)),
            Pat::PathSub(s) => contains(path, s),
            Pat::PathGlob(g) => std::str::from_utf8(path).is_ok_and(|n| wildcard(g, n)),
        });
        hit != self.not
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > hay.len() {
        return false;
    }
    let first = needle[0];
    let last = hay.len() - needle.len();
    let mut i = 0;
    while i <= last {
        match hay[i..=last].iter().position(|&b| b == first) {
            None => return false,
            Some(k) => {
                i += k;
                if &hay[i..i + needle.len()] == needle {
                    return true;
                }
                i += 1;
            }
        }
    }
    false
}

/// A parsed search.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    terms: Vec<Term>,
    pub kind: Kind,
    /// Bigger than this many bytes.
    pub min: Option<u64>,
    /// Smaller than this many bytes.
    pub max: Option<u64>,
}

/// `1gb`, `10 kb`, `1.5m`, `500` (bytes).
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let v: f64 = num.parse().ok()?;
    let mul = match unit.trim() {
        "" | "b" => 1u64,
        "k" | "kb" | "kib" => 1 << 10,
        "m" | "mb" | "mib" => 1 << 20,
        "g" | "gb" | "gib" => 1 << 30,
        "t" | "tb" | "tib" => 1 << 40,
        _ => return None,
    };
    Some((v * mul as f64) as u64)
}

fn tokens(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in q.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

impl Query {
    pub fn parse(q: &str) -> Query {
        let mut query = Query::default();
        for tok in tokens(q) {
            let mut t = tok.as_str();
            let lower = t.to_ascii_lowercase();
            for (prefix, kind) in [("folder:", Kind::Folders), ("file:", Kind::Files)] {
                if lower.starts_with(prefix) {
                    query.kind = kind;
                    t = &t[prefix.len()..];
                    break;
                }
            }
            if t.is_empty() {
                continue;
            }
            // sizes
            let size = if let Some(r) = t.strip_prefix(">=").or_else(|| t.strip_prefix('>')) {
                parse_size(r).map(|v| (true, if t.starts_with(">=") { v.saturating_sub(1) } else { v }))
            } else if let Some(r) = t.strip_prefix("<=").or_else(|| t.strip_prefix('<')) {
                parse_size(r).map(|v| (false, if t.starts_with("<=") { v.saturating_add(1) } else { v }))
            } else {
                None
            };
            if let Some((bigger, v)) = size {
                if bigger {
                    query.min = Some(query.min.map_or(v, |m| m.max(v)));
                } else {
                    query.max = Some(query.max.map_or(v, |m| m.min(v)));
                }
                continue;
            }
            let (not, rest) = match t.strip_prefix('!') {
                Some(r) => (true, r),
                None => (false, t),
            };
            let alts: Vec<Pat> = rest
                .split(['|', ';'])
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let low = p.to_lowercase();
                    let glob = low.contains(['*', '?']);
                    match (low.contains('\\'), glob) {
                        (false, false) => Pat::Sub(low.into_bytes()),
                        (false, true) => Pat::Glob(low.chars().collect()),
                        (true, false) => Pat::PathSub(low.into_bytes()),
                        (true, true) => Pat::PathGlob(low.chars().collect()),
                    }
                })
                .collect();
            if !alts.is_empty() {
                query.terms.push(Term { not, alts });
            }
        }
        query
    }

    /// Nothing to look for: no words, no size, no kind.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.min.is_none() && self.max.is_none() && self.kind == Kind::Any
    }

    /// Some words need the full path (slower: the paths are built as the tree is walked).
    pub fn uses_paths(&self) -> bool {
        self.terms.iter().any(Term::on_path)
    }

    /// Where the first plain word is in `name` (byte range), to show it in bold.
    pub fn highlight(&self, name: &str) -> Option<(usize, usize)> {
        let lower = name.to_lowercase();
        if lower.len() != name.len() {
            return None;
        }
        self.terms.iter().filter(|t| !t.not).flat_map(|t| &t.alts).find_map(|p| match p {
            Pat::Sub(s) => {
                let s = std::str::from_utf8(s).ok()?;
                lower.find(s).map(|i| (i, i + s.len()))
            }
            _ => None,
        })
    }
}

/// The nodes of `tree` that match `q` (never the root), in tree order. `None` when `cancel`
/// was set on the way (a newer search started).
pub fn search(tree: &Tree, index: &Index, q: &Query, cancel: &AtomicBool) -> Option<Vec<usize>> {
    let n = tree.nodes.len().min(index.len());
    if n <= 1 || q.is_empty() {
        return Some(Vec::new());
    }
    let (name_terms, path_terms): (Vec<&Term>, Vec<&Term>) = q.terms.iter().partition(|t| !t.on_path());
    let cheap = |i: usize| -> bool {
        if !index.live[i] {
            return false;
        }
        let node = &tree.nodes[i];
        match q.kind {
            Kind::Files if node.is_dir => return false,
            Kind::Folders if !node.is_dir => return false,
            _ => {}
        }
        if q.min.is_some_and(|m| node.size <= m) || q.max.is_some_and(|m| node.size >= m) {
            return false;
        }
        let name = index.name(i);
        name_terms.iter().all(|t| t.matches(name, &[]))
    };

    // the cheap part on every core
    let threads = std::thread::available_parallelism().map_or(4, |c| c.get()).clamp(1, 16);
    let chunk = n.div_ceil(threads);
    let parts: Vec<Option<Vec<usize>>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|k| {
                let cheap = &cheap;
                s.spawn(move || {
                    let (a, b) = ((k * chunk).max(1), ((k + 1) * chunk).min(n));
                    let mut out = Vec::new();
                    for i in a..b {
                        if (i & 0xFFFF) == 0 && cancel.load(Ordering::Relaxed) {
                            return None;
                        }
                        if cheap(i) {
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
    if path_terms.is_empty() {
        return Some(hits);
    }

    // the path words: walk the tree with a path buffer, checking only the candidates; the
    // top of the tree is split into pieces that the cores walk side by side
    let mut candidate = vec![false; n];
    for &i in &hits {
        candidate[i] = true;
    }
    let walker = PathWalk { tree, index, terms: &path_terms, candidate: &candidate, n };
    let mut root: Vec<u8> = tree.root_path.to_string_lossy().to_lowercase().into_bytes();
    while root.last() == Some(&b'\\') {
        root.pop();
    }
    let mut keep = Vec::new();
    let mut items: Vec<(usize, Vec<u8>)> = vec![(ROOT, root)];
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

struct PathWalk<'a> {
    tree: &'a Tree,
    index: &'a Index,
    terms: &'a [&'a Term],
    candidate: &'a [bool],
    n: usize,
}

impl PathWalk<'_> {
    /// Add node `i`'s name to `path` (its parent's path).
    fn push_name(&self, i: usize, path: &mut Vec<u8>) {
        let name = &self.tree.nodes[i].name;
        if std::path::Path::new(name).is_absolute() {
            // a "Select Multiple" child is named by its full path
            path.clear();
            path.extend_from_slice(name.trim_end_matches('\\').to_lowercase().as_bytes());
        } else {
            path.push(b'\\');
            path.extend_from_slice(self.index.name(i));
        }
    }

    /// `path` is node `i`'s: keep `i` if it is a candidate and the path words match.
    fn check(&self, i: usize, path: &mut Vec<u8>, keep: &mut Vec<usize>) {
        if i == ROOT || !self.candidate[i] {
            return;
        }
        let dir = self.tree.nodes[i].is_dir;
        if dir {
            path.push(b'\\');
        }
        if self.terms.iter().all(|t| t.matches(self.index.name(i), path)) {
            keep.push(i);
        }
        if dir {
            path.pop();
        }
    }

    /// Everything from `start` down (`path` is `start`'s).
    fn walk(&self, start: usize, path: Vec<u8>, keep: &mut Vec<usize>, cancel: &AtomicBool) -> Option<()> {
        let mut buf = path;
        let mut stack: Vec<(usize, usize)> = Vec::new();
        let base = buf.len();
        self.check(start, &mut buf, keep);
        for &c in self.tree.nodes[start].children.iter().rev() {
            if c < self.n {
                stack.push((c, base));
            }
        }
        let mut seen = 0usize;
        while let Some((i, len)) = stack.pop() {
            seen += 1;
            if (seen & 0x3FFF) == 0 && cancel.load(Ordering::Relaxed) {
                return None;
            }
            buf.truncate(len);
            self.push_name(i, &mut buf);
            self.check(i, &mut buf, keep);
            let here = buf.len();
            for &c in self.tree.nodes[i].children.iter().rev() {
                if c < self.n {
                    stack.push((c, here));
                }
            }
        }
        Some(())
    }
}

/// The place of every folder in the order of its full path, compared case-insensitively (ASCII)
/// as the Folder column sorts: `rank[folder] < rank[other]` when its path sorts first. Each path
/// is built once, by a walk from the root; a sort by folder is then a sort of numbers (a few
/// hundred ms for millions of rows, not half a minute of paths built per comparison). Files
/// are `u32::MAX`.
pub fn folder_ranks(tree: &Tree) -> Vec<u32> {
    let mut keys: Vec<(Box<[u8]>, u32)> = Vec::new();
    let root: Vec<u8> = tree.root_path.to_string_lossy().bytes().map(|c| c.to_ascii_lowercase()).collect();
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
            p.extend(child.name.bytes().map(|c| c.to_ascii_lowercase()));
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
    ///   Games\Steam\config.vdf (10), Games\Steam\Report 2024.PDF (3000)
    ///   cache\ (folder) with temp.tmp (5)
    ///   setup.exe (2 GB)
    fn sample() -> Tree {
        let nodes = vec![
            node("C:\\Data", true, 0, None, vec![1, 5, 7]),
            node("Games", true, 0, Some(0), vec![2]),
            node("Steam", true, 0, Some(1), vec![3, 4]),
            node("config.vdf", false, 10, Some(2), vec![]),
            node("Report 2024.PDF", false, 3000, Some(2), vec![]),
            node("cache", true, 0, Some(0), vec![6]),
            node("temp.tmp", false, 5, Some(5), vec![]),
            node("setup.exe", false, 2 << 30, Some(0), vec![]),
        ];
        let mut t = Tree { root_path: PathBuf::from("C:\\Data"), nodes, errors: 0, method: Default::default() };
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

    #[test]
    fn sizes_and_empty() {
        assert_eq!(parse_size("1gb"), Some(1 << 30));
        assert_eq!(parse_size("1.5k"), Some(1536));
        assert_eq!(parse_size("12"), Some(12));
        assert_eq!(parse_size("1xb"), None);
        assert!(Query::parse("  ").is_empty());
        assert!(Query::parse("!").is_empty());
        // `>` with no number is a word
        assert!(!Query::parse(">abc").terms.is_empty());
    }

    #[test]
    fn highlight() {
        let q = Query::parse("!x 2024");
        assert_eq!(q.highlight("Report 2024.PDF"), Some((7, 11)));
        assert_eq!(Query::parse("*.pdf").highlight("a.pdf"), None);
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

    #[test]
    fn deleted_branches_are_skipped() {
        let mut t = sample();
        t.nodes[0].children.retain(|&c| c != 1); // Games cut off, as a delete leaves it
        assert!(find(&t, "*.vdf").is_empty());
        assert_eq!(find(&t, "setup"), ["setup.exe"]);
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
        let t = Tree { root_path: PathBuf::from("C:\\"), nodes, errors: 0, method: Default::default() };
        let start = std::time::Instant::now();
        let ix = Index::build(&t);
        println!("index: {:?}, {} MB", start.elapsed(), ix.memory() >> 20);
        for q in ["v42", "file name 12345", "*.dll", "\\folder_000123_program data\\", ">90kb v1"] {
            let start = std::time::Instant::now();
            let r = search(&t, &ix, &Query::parse(q), &AtomicBool::new(false)).unwrap();
            println!("{q:>34}: {:>8} hits in {:?}", r.len(), start.elapsed());
        }
    }
}
