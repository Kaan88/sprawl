//! The compact report a model starts from: totals, the biggest folders a few levels down, file
//! types, what the rules already found, and hidden space. Tens of kilobytes at most, never the
//! whole tree; the model asks for more through the tools.

use std::collections::HashMap;
use std::fmt::Write;

use sprawl_core::{format_size, Tree, ROOT};

use crate::facts::{month_year, Facts};
use crate::finding::Finding;
use crate::hidden::HiddenSpace;

/// Size, item count, newest change and owner of node `n` on one line.
pub(crate) fn node_line(tree: &Tree, facts: &Facts, n: usize) -> String {
    let node = &tree.nodes[n];
    let mut s = format!("{} ({} bytes)", format_size(node.size), node.size);
    if node.is_dir {
        let _ = write!(s, ", {} files, {} folders", node.files, node.folders);
    }
    if let Some(t) = node.modified {
        let _ = write!(s, ", modified {}", month_year(t));
    }
    if node.is_reparse_point() {
        s.push_str(", junction or link");
    }
    if let Some(app) = facts.owner_of(&tree.path_of(n)) {
        let _ = write!(s, ", belongs to {}", app.name);
    }
    s
}

/// Extension totals (bytes, files) below node `n`, biggest first.
pub(crate) fn types_below(tree: &Tree, n: usize, limit: usize) -> Vec<(String, u64, u64)> {
    let mut by: HashMap<String, (u64, u64)> = HashMap::new();
    let mut stack = vec![n];
    while let Some(k) = stack.pop() {
        let node = &tree.nodes[k];
        if node.is_dir {
            stack.extend_from_slice(&node.children);
        } else {
            let e = by.entry(node.extension()).or_default();
            e.0 += node.size;
            e.1 += 1;
        }
    }
    let mut v: Vec<(String, u64, u64)> = by.into_iter().map(|(k, (b, c))| (k, b, c)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.truncate(limit);
    v
}

pub fn build(tree: &Tree, facts: &Facts, found: &[Finding], hidden: Option<&HiddenSpace>, goal: Option<u64>) -> String {
    let root = &tree.nodes[ROOT];
    let mut s = String::new();
    let _ = writeln!(s, "# Scan of {}", tree.path_of(ROOT).display());
    let _ = writeln!(s, "{} in {} files and {} folders.", format_size(root.size), root.files, root.folders);
    if tree.errors > 0 {
        let _ = writeln!(s, "{} folders couldn't be read (no administrator rights), so their contents are missing.", tree.errors);
    }
    if let Some(g) = goal {
        let _ = writeln!(s, "\nThe user needs to free {} ({g} bytes).", format_size(g));
    }

    // the biggest folders, a few levels down: anything over 1% of the scan or 512 MB
    let floor = (root.size / 100).max(512 << 20);
    let _ = writeln!(s, "\n## Biggest folders (over {}), up to 5 levels down", format_size(floor));
    let mut lines = 0;
    let mut stack = vec![(ROOT, 0usize)];
    while let Some((n, depth)) = stack.pop() {
        if lines >= 120 {
            s.push_str("[more folders left out]\n");
            break;
        }
        if n != ROOT {
            let _ = writeln!(s, "{}{}  {}", "  ".repeat(depth - 1), tree.path_of(n).display(), node_line(tree, facts, n));
            lines += 1;
        }
        let node = &tree.nodes[n];
        if depth < 5 && !(n != ROOT && node.is_reparse_point()) {
            let mut kids: Vec<usize> = node.children.iter().copied().filter(|&c| tree.nodes[c].is_dir && tree.nodes[c].size >= floor).collect();
            kids.sort_by(|a, b| tree.nodes[*a].size.cmp(&tree.nodes[*b].size));
            // pushed smallest first, so the biggest comes off the stack first
            stack.extend(kids.into_iter().map(|c| (c, depth + 1)));
        }
    }

    s.push_str("\n## Biggest file types\n");
    for (ext, bytes, count) in types_below(tree, ROOT, 15) {
        let name = if ext.is_empty() { "(no extension)".to_string() } else { format!(".{ext}") };
        let _ = writeln!(s, "{name}: {} in {count} files", format_size(bytes));
    }

    s.push_str("\n## Already found by Sprawl's rules (don't report these again; you may comment on them)\n");
    if found.is_empty() {
        s.push_str("(nothing)\n");
    }
    for f in found.iter().take(60) {
        let _ = writeln!(s, "- {} ({:?}, {:?}): {} {}", f.title, f.risk, f.remedy, f.path.display(), format_size(f.bytes));
    }

    if let Some(h) = hidden.filter(|h| h.notable) {
        let _ = writeln!(s, "\n## Hidden space\nThe drive reports {} used; the scan accounts for {}.", format_size(h.used), format_size(h.scanned));
        for p in &h.parts {
            let _ = writeln!(s, "- {}: {}", p.label, p.note);
        }
    }

    let unused: Vec<&crate::facts::App> = facts.apps.iter().filter(|a| a.last_used.is_some()).collect();
    if !unused.is_empty() {
        s.push_str("\n## Apps with a known last use\n");
        let mut v = unused;
        v.sort_by_key(|a| a.last_used.map(|(t, _)| t));
        for a in v.into_iter().take(25) {
            if let Some((t, from)) = a.last_used {
                let _ = writeln!(s, "- {}: last started {} (from {})", a.name, month_year(t), from.label());
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_lists_big_folders_and_types() {
        let dir = std::env::temp_dir().join(format!("sprawl-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("big").join("inner")).unwrap();
        std::fs::write(dir.join("big").join("inner").join("a.bin"), vec![0u8; 4000]).unwrap();
        std::fs::write(dir.join("small.txt"), "x").unwrap();
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let facts = Facts::from_apps(Vec::new());
        let r = build(&tree, &facts, &[], None, Some(5 << 30));
        assert!(r.contains("# Scan of"));
        assert!(r.contains("The user needs to free 5.0 GB"));
        assert!(r.contains(".bin: "));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
