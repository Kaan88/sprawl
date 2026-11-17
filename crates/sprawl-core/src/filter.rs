//! WizTree-style name/path filters.
//!
//! * `*` matches any run of characters, `?` exactly one; matching is case-insensitive.
//! * Several patterns separated by `|` (or `;`, as the filter box suggests) match when any of
//!   them does: `*.mp4|*.mkv`.
//! * A pattern containing `\` is matched against the full path (`C:\Users\*\Downloads\*`),
//!   otherwise against the name only (`*.log`).
//! * A pattern without `*` or `?` matches anywhere inside the name/path (`setup` finds
//!   `Setup.exe` and `old-setup.msi`).
//!
//! [`filter_tree`] keeps the files that match plus the folders leading to them. A folder whose
//! full path (with a trailing `\`) matches a path pattern is kept with everything below it, so
//! `*\node_modules\` shows every `node_modules` folder in full.

use crate::{Node, Tree, ROOT};

#[derive(Debug, Clone)]
struct Alt {
    /// Lower-case pattern characters.
    pat: Vec<char>,
    /// Match against the full path rather than the name.
    path: bool,
}

/// A compiled filter; build once, match many times.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    alts: Vec<Alt>,
}

impl Filter {
    pub fn new(pattern: &str) -> Self {
        let alts = pattern
            .split(['|', ';'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| {
                let mut pat: Vec<char> = p.chars().flat_map(char::to_lowercase).collect();
                if !pat.iter().any(|&c| c == '*' || c == '?') {
                    pat.insert(0, '*');
                    pat.push('*');
                }
                Alt { path: p.contains('\\'), pat }
            })
            .collect();
        Filter { alts }
    }

    /// No patterns at all (an empty or all-blank filter string).
    pub fn is_empty(&self) -> bool {
        self.alts.is_empty()
    }

    /// Does `name_or_path` match? Name patterns look at the part after the last `\` (a trailing
    /// `\` is ignored for that), path patterns at the whole string.
    pub fn matches(&self, name_or_path: &str) -> bool {
        let trimmed = name_or_path.strip_suffix('\\').unwrap_or(name_or_path);
        let name = match trimmed.rfind('\\') {
            Some(i) => &trimmed[i + 1..],
            None => trimmed,
        };
        self.alts.iter().any(|a| wildcard(&a.pat, if a.path { name_or_path } else { name }))
    }

    /// Does a folder path (ending in `\`) match one of the path patterns?
    fn matches_folder(&self, folder_path: &str) -> bool {
        self.alts.iter().any(|a| a.path && wildcard(&a.pat, folder_path))
    }
}

/// `pattern` (see the module docs) against a name or a full path.
pub fn matches(pattern: &str, name_or_path: &str) -> bool {
    Filter::new(pattern).matches(name_or_path)
}

fn lower_eq(p: char, c: char) -> bool {
    p == c || (!c.is_ascii() || c.is_ascii_uppercase()) && c.to_lowercase().eq(std::iter::once(p))
}

/// Glob match with `*` and `?`, `pat` already lower-case. Linear-ish (backtracks to the last
/// star only).
pub(crate) fn wildcard(pat: &[char], s: &str) -> bool {
    wildcard_by(pat, s, lower_eq)
}

/// [`wildcard`] with upper and lower case told apart (`pat` as typed).
pub(crate) fn wildcard_case(pat: &[char], s: &str) -> bool {
    wildcard_by(pat, s, |p, c| p == c)
}

#[inline]
fn wildcard_by(pat: &[char], s: &str, eq: impl Fn(char, char) -> bool) -> bool {
    let (mut p, mut si) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None; // (pattern index after '*', string index)
    let bytes = s.len();
    while si < bytes {
        let c = s[si..].chars().next().unwrap_or('\0');
        if p < pat.len() && pat[p] == '*' {
            p += 1;
            star = Some((p, si));
            continue;
        }
        if p < pat.len() && (pat[p] == '?' || eq(pat[p], c)) {
            p += 1;
            si += c.len_utf8();
            continue;
        }
        match star {
            Some((sp, ss)) => {
                let skip = s[ss..].chars().next().map_or(1, char::len_utf8);
                star = Some((sp, ss + skip));
                p = sp;
                si = ss + skip;
            }
            None => return false,
        }
    }
    pat[p..].iter().all(|&c| c == '*')
}

/// A new tree with only the files matching `pattern` and the folders above them (sizes and
/// counts recomputed). The root is always kept. An empty pattern keeps everything.
pub fn filter_tree(tree: &Tree, pattern: &str) -> Tree {
    let f = Filter::new(pattern);
    let n = tree.nodes.len();
    if n == 0 || f.is_empty() {
        return tree.clone();
    }
    let mut keep = vec![false; n];
    let mut whole = vec![false; n];
    keep[ROOT] = true;
    // Depth-first with one path buffer.
    let mut buf = tree.root_path.display().to_string();
    let mut stack: Vec<(usize, usize)> = vec![(ROOT, buf.len())];
    while let Some((i, len)) = stack.pop() {
        buf.truncate(len);
        if i != ROOT {
            let name = &tree.nodes[i].name;
            if std::path::Path::new(name).is_absolute() {
                buf.clear(); // a scan_many child: its name is its full path
            } else if !buf.ends_with('\\') {
                buf.push('\\');
            }
            buf.push_str(name);
        }
        let node = &tree.nodes[i];
        let parent_whole = node.parent.is_some_and(|p| whole[p]);
        if parent_whole {
            whole[i] = true;
            keep[i] = true;
        } else if node.is_dir {
            if i != ROOT {
                let l = buf.len();
                if !buf.ends_with('\\') {
                    buf.push('\\');
                }
                if f.matches_folder(&buf) {
                    whole[i] = true;
                    keep[i] = true;
                }
                buf.truncate(l);
            }
        } else if f.matches(&buf) {
            keep[i] = true;
        }
        let here = buf.len();
        for &c in node.children.iter().rev() {
            stack.push((c, here));
        }
    }
    for i in (1..n).rev() {
        if keep[i] {
            if let Some(p) = tree.nodes[i].parent {
                keep[p] = true;
            }
        }
    }
    // Copy kept nodes in index order (parents before children), remapping indices.
    let mut map = vec![usize::MAX; n];
    let mut nodes: Vec<Node> = Vec::with_capacity(keep.iter().filter(|&&k| k).count());
    for i in 0..n {
        if !keep[i] {
            continue;
        }
        map[i] = nodes.len();
        let o = &tree.nodes[i];
        nodes.push(Node {
            name: o.name.clone(),
            is_dir: o.is_dir,
            size: o.size,
            allocated: o.allocated,
            files: o.files,
            folders: o.folders,
            modified: o.modified,
            attributes: o.attributes,
            parent: o.parent.map(|p| map[p]),
            children: Vec::new(),
        });
    }
    for i in 0..n {
        if keep[i] {
            let kids: Vec<usize> = tree.nodes[i].children.iter().filter(|&&c| keep[c]).map(|&c| map[c]).collect();
            nodes[map[i]].children = kids;
        }
    }
    let mut out = Tree { root_path: tree.root_path.clone(), nodes, errors: tree.errors, method: tree.method };
    // Folders that lost all their children would otherwise keep their old totals.
    for n in out.nodes.iter_mut().filter(|n| n.is_dir && n.children.is_empty()) {
        n.size = 0;
        n.allocated = 0;
        n.files = 0;
        n.folders = 0;
    }
    out.compute_totals();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn wildcards() {
        assert!(matches("*.mp4", "Movie.MP4"));
        assert!(!matches("*.mp4", "movie.mp4.part"));
        assert!(matches("*.mp4|*.mkv", "a.mkv"));
        assert!(matches(" *.mp4 | *.mkv ", "a.mkv"));
        assert!(matches("file?.txt", "file1.txt"));
        assert!(!matches("file?.txt", "file10.txt"));
        assert!(matches("*a*b*c", "xxaYYbZZc"));
        assert!(!matches("*a*b*c", "xxaYYbZZ"));
        assert!(matches("setup", "Old-Setup.exe")); // no wildcard: substring
        assert!(matches("ÄBC*", "äbcd"));
        assert!(matches("*", ""));
        assert!(!matches("?", ""));
        assert!(!matches("", "x")); // empty filter matches nothing on its own
        // Name patterns look at the last component only.
        assert!(matches("*.txt", r"C:\dir.txt\file.txt"));
        assert!(!matches("dir*", r"C:\dir\file.txt"));
        assert!(matches("dir*", r"C:\x\dir\"));
        // Path patterns look at the whole path.
        assert!(matches(r"C:\Users\*\Downloads\*", r"c:\users\bob\downloads\x.zip"));
        assert!(!matches(r"C:\Users\*\Downloads\*", r"C:\Users\bob\Documents\x.zip"));
        assert!(matches(r"*\temp\*.tmp", r"D:\a\Temp\b.TMP"));
        assert!(matches(r"\cache\", r"C:\x\Cache\y.bin"));
    }

    fn node(name: &str, is_dir: bool, size: u64, parent: Option<usize>, children: Vec<usize>) -> Node {
        Node { name: name.into(), is_dir, size, allocated: size, files: u64::from(!is_dir), parent, children, ..Default::default() }
    }

    fn sample() -> Tree {
        // C:\data\ { a.mp4 (100), b.txt (5), sub\ { c.mp4 (50), d.log (7) }, node_modules\ { e.js (3), deep\ { f.json (2) } } }
        let nodes = vec![
            node("C:\\data", true, 0, None, vec![1, 2, 3, 4]),
            node("a.mp4", false, 100, Some(0), vec![]),
            node("b.txt", false, 5, Some(0), vec![]),
            node("sub", true, 0, Some(0), vec![5, 6]),
            node("node_modules", true, 0, Some(0), vec![7, 8]),
            node("c.mp4", false, 50, Some(3), vec![]),
            node("d.log", false, 7, Some(3), vec![]),
            node("e.js", false, 3, Some(4), vec![]),
            node("deep", true, 0, Some(4), vec![9]),
            node("f.json", false, 2, Some(8), vec![]),
        ];
        let mut t = Tree { root_path: PathBuf::from("C:\\data"), nodes, errors: 0, method: Default::default() };
        t.compute_totals();
        t
    }

    #[test]
    fn filter_tree_keeps_matches_and_ancestors() {
        let t = sample();
        assert_eq!(t.nodes[ROOT].size, 167);

        let f = filter_tree(&t, "*.mp4");
        let r = &f.nodes[ROOT];
        assert_eq!((r.size, r.files, r.folders), (150, 2, 1));
        assert_eq!(f.nodes.len(), 4);
        let sub = f.nodes.iter().position(|n| n.name == "sub").unwrap();
        assert_eq!(f.path_of(f.nodes[sub].children[0]), PathBuf::from(r"C:\data\sub\c.mp4"));
        for (i, n) in f.nodes.iter().enumerate() {
            for &c in &n.children {
                assert!(c > i);
                assert_eq!(f.nodes[c].parent, Some(i));
            }
        }

        // A folder path pattern keeps the whole folder.
        let f = filter_tree(&t, r"*\node_modules\");
        let r = &f.nodes[ROOT];
        assert_eq!((r.size, r.files, r.folders), (5, 2, 2));

        // Full-path file pattern.
        let f = filter_tree(&t, r"C:\data\sub\*");
        assert_eq!(f.nodes[ROOT].size, 57);

        // Nothing matches: just the root, empty.
        let f = filter_tree(&t, "*.iso");
        assert_eq!(f.nodes.len(), 1);
        assert_eq!(f.nodes[ROOT].size, 0);
        assert_eq!(f.nodes[ROOT].files, 0);

        // Empty pattern: unchanged.
        assert_eq!(filter_tree(&t, "  ").nodes.len(), t.nodes.len());
    }
}
