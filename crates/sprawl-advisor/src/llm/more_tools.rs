//! `search` and `growth`, for Claude Code through `sprawl --mcp` (next round, Phase 1): the
//! Search tab's language over a scan, and a folder compared with History. Read-only.

use std::fmt::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use serde_json::{json, Value};
use sprawl_core::{format_size, Tree, ROOT};

use super::report::node_line;
use super::ToolSpec;
use crate::facts::Facts;

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "search",
            description: "Find files and folders by name, size, date or type in a scan, with the language of Sprawl's Search tab: words (all must match, \
                any case), *.iso and img_????.jpg patterns, a|b (either), !tmp (not), >1gb <10kb (size; a folder's size is its total), ext:mp4,mkv, \
                dm:2024 dm:>2025-06-01 (modified), folder: or file:, path:steam (in the path), depth:<3, regex:^IMG_\\d+$. \
                With path, only inside that folder (and the scan holding it is the one searched). Biggest first.",
            schema: json!({"type": "object", "properties": {
                "query": {"type": "string", "description": "The search, as typed in the Search tab, e.g. ext:iso >1gb or node_modules folder:"},
                "path": {"type": "string", "description": "Only inside this folder."},
                "limit": {"type": "integer", "description": "At most this many (default 30, max 100)."}
            }, "required": ["query"]}),
        },
        ToolSpec {
            name: "growth",
            description: "How a folder changed since an earlier scan in Sprawl's History: its size then and now, and the folders inside it that grew \
                or shrank most (new and gone ones too). Default: the scan nearest a week ago.",
            schema: json!({"type": "object", "properties": {
                "path": {"type": "string", "description": "The folder (default: the scan's root)."},
                "days": {"type": "integer", "description": "Compare with the scan nearest this many days ago (default 7)."}
            }}),
        },
    ]
}

/// The node a call's optional `path` names, or the root.
fn node_or_root(tree: &Tree, input: &Value) -> Result<usize, String> {
    match input["path"].as_str().map(|p| p.trim().trim_matches('"').replace('/', "\\")).filter(|p| !p.is_empty()) {
        None => Ok(ROOT),
        Some(p) => crate::find_path(tree, Path::new(&p)).ok_or_else(|| format!("{p} {}", crate::mcp_scan::NOT_IN_SCAN)),
    }
}

/// `n` is `top` or inside it.
fn within(tree: &Tree, mut n: usize, top: usize) -> bool {
    loop {
        if n == top {
            return true;
        }
        match tree.nodes[n].parent {
            Some(p) => n = p,
            None => return false,
        }
    }
}

pub fn search(tree: &Tree, facts: &Facts, input: &Value) -> Result<String, String> {
    let text = input["query"].as_str().map(str::trim).filter(|q| !q.is_empty()).ok_or("give a `query`, e.g. ext:iso >1gb")?;
    let q = sprawl_core::query::Query::parse(text);
    if !q.errors().is_empty() {
        return Err(format!("The search has mistakes: {}.\n\nThe language:\n{}", q.errors().join("; "), sprawl_core::query::HELP));
    }
    let top = node_or_root(tree, input)?;
    let max = input["limit"].as_u64().unwrap_or(30).clamp(1, 100) as usize;
    let index = sprawl_core::search::Index::build(tree);
    let hits = sprawl_core::search::search(tree, &index, &q, &AtomicBool::new(false)).unwrap_or_default();
    let mut hits: Vec<usize> = hits.into_iter().filter(|&n| n != top && within(tree, n, top)).collect();
    if hits.is_empty() {
        return Ok(format!("Nothing in {} matches {text}.", tree.path_of(top).display()));
    }
    hits.sort_by_key(|&n| std::cmp::Reverse(tree.nodes[n].size));
    // a folder's total holds its matching children's: the total counts outermost matches
    let total: u64 = hits.iter().filter(|&&n| !tree.nodes[n].parent.is_some_and(|p| hits_contains(&hits, tree, p))).map(|&n| tree.nodes[n].size).sum();
    let mut s = format!("{} matches for {text} in {}, {} in all. Biggest first:\n", hits.len(), tree.path_of(top).display(), format_size(total));
    for &n in hits.iter().take(max) {
        let kind = if tree.nodes[n].is_dir { "folder" } else { "file" };
        let _ = writeln!(s, "- {} [{kind}] {}", tree.path_of(n).display(), node_line(tree, facts, n));
    }
    if hits.len() > max {
        let _ = writeln!(s, "({} more, smaller)", hits.len() - max);
    }
    Ok(s)
}

/// Whether a match sits above `n` (a match inside a matching folder isn't counted twice).
fn hits_contains(hits: &[usize], tree: &Tree, n: usize) -> bool {
    let mut cur = Some(n);
    while let Some(c) = cur {
        if hits.contains(&c) {
            return true;
        }
        cur = tree.nodes[c].parent;
    }
    false
}

pub fn growth(tree: &Tree, input: &Value, scan_before: SystemTime) -> Result<String, String> {
    growth_in(tree, input, crate::history::base_dir().as_deref(), scan_before)
}

fn growth_in(tree: &Tree, input: &Value, base: Option<&Path>, now: SystemTime) -> Result<String, String> {
    let n = node_or_root(tree, input)?;
    let target = tree.path_of(n);
    if !tree.nodes[n].is_dir {
        return Err(format!("{} is a file: growth compares folders.", target.display()));
    }
    let base = base.ok_or("Sprawl has no History folder here.")?;
    // older than this scan: History may already hold the scan itself
    let entries: Vec<crate::history::Entry> =
        crate::history::earlier(base, &target, now);
    if entries.is_empty() {
        return Ok(format!("History has no earlier scan of {}: growth needs one. Sprawl keeps one after every scan while History is on.", target.display()));
    }
    let days = input["days"].as_u64().unwrap_or(7);
    let want = now.checked_sub(Duration::from_secs(days.saturating_mul(86_400))).unwrap_or(SystemTime::UNIX_EPOCH);
    let gap = |e: &crate::history::Entry| if e.taken > want { e.taken.duration_since(want) } else { want.duration_since(e.taken) }.unwrap_or_default();
    let e = entries.iter().min_by_key(|e| gap(e)).expect("not empty");
    let old = crate::history::load(e).map_err(|err| format!("History's scan of {} couldn't be read: {err}", e.root.display()))?;
    let o = crate::history::find_path(&old, &target).ok_or_else(|| format!("{} wasn't in History's scan of {} then.", target.display(), e.root.display()))?;
    let (now_size, was) = (tree.nodes[n].size, old.nodes[o].size);
    let mut s = format!(
        "{}: {} now; {} in History's scan of {} {} ({}).\n",
        target.display(),
        format_size(now_size),
        format_size(was),
        e.root.display(),
        crate::mcp_scan::age(e.taken),
        crate::growth::signed(now_size as i64 - was as i64)
    );
    // its folders by name, then the files directly in it as one line
    let mut old_dirs: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut old_files = 0u64;
    for &k in &old.nodes[o].children {
        let kn = &old.nodes[k];
        if kn.is_dir {
            old_dirs.insert(kn.name.clone(), kn.size);
        } else {
            old_files += kn.size;
        }
    }
    let mut rows: Vec<(i64, String)> = Vec::new();
    let mut new_files = 0u64;
    for &k in &tree.nodes[n].children {
        let kn = &tree.nodes[k];
        if !kn.is_dir {
            new_files += kn.size;
            continue;
        }
        match old_dirs.remove(&kn.name) {
            Some(b) => rows.push((kn.size as i64 - b as i64, format!("{} (now {}, was {})", kn.name, format_size(kn.size), format_size(b)))),
            None => rows.push((kn.size as i64, format!("{} (new, {})", kn.name, format_size(kn.size)))),
        }
    }
    for (name, b) in old_dirs {
        rows.push((-(b as i64), format!("{name} (gone, was {})", format_size(b))));
    }
    if new_files != old_files {
        rows.push((new_files as i64 - old_files as i64, format!("the files directly in it (now {}, was {})", format_size(new_files), format_size(old_files))));
    }
    rows.retain(|r| r.0 != 0);
    rows.sort_by_key(|r| std::cmp::Reverse(r.0.unsigned_abs()));
    if rows.is_empty() {
        s += "Nothing inside it changed size.\n";
    } else {
        s += "What changed most inside it:\n";
        for (d, what) in rows.iter().take(15) {
            let _ = writeln!(s, "- {} {what}", crate::growth::signed(*d));
        }
        s += "(Call growth on one of these folders to look inside it.)\n";
    }
    let others: Vec<String> = entries.iter().rev().take(8).map(|x| crate::mcp_scan::age(x.taken)).collect();
    let _ = writeln!(s, "Earlier scans in History that hold it: {}.", others.join(", "));
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_of(dir: &Path) -> Tree {
        sprawl_core::scan(dir, |_| {}).unwrap()
    }

    #[test]
    fn search_uses_the_search_tabs_language() {
        let dir = std::env::temp_dir().join(format!("sprawl-more-search-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::write(dir.join("a").join("big.iso"), vec![0u8; 20_000]).unwrap();
        std::fs::write(dir.join("small.iso"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join("note.txt"), b"x").unwrap();
        let t = tree_of(&dir);
        let facts = Facts::from_apps(Vec::new());
        let r = search(&t, &facts, &json!({"query": "ext:iso"})).unwrap();
        assert!(r.starts_with("2 matches") && r.find("big.iso") < r.find("small.iso"), "{r}");
        let r = search(&t, &facts, &json!({"query": "*.iso >10kb"})).unwrap();
        assert!(r.contains("big.iso") && !r.contains("small.iso"), "{r}");
        let inside = search(&t, &facts, &json!({"query": "iso", "path": dir.join("a").to_string_lossy()})).unwrap();
        assert!(inside.starts_with("1 matches"), "{inside}");
        assert!(search(&t, &facts, &json!({"query": "size:>>"})).is_err());
        assert!(search(&t, &facts, &json!({"query": "zzz-none"})).unwrap().starts_with("Nothing"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn growth_compares_with_history() {
        let dir = std::env::temp_dir().join(format!("sprawl-more-growth-{}", std::process::id()));
        let base = dir.join("history");
        let root = dir.join("data");
        std::fs::create_dir_all(root.join("keeps")).unwrap();
        std::fs::create_dir_all(root.join("goes")).unwrap();
        std::fs::write(root.join("keeps").join("f"), vec![0u8; 1000]).unwrap();
        std::fs::write(root.join("goes").join("f"), vec![0u8; 500]).unwrap();
        let old = tree_of(&root);
        let then = SystemTime::now() - Duration::from_secs(6 * 86_400);
        crate::history::prepare(&old, then, true).save(&base, u64::MAX).unwrap();
        std::fs::remove_dir_all(root.join("goes")).unwrap();
        std::fs::create_dir_all(root.join("new")).unwrap();
        std::fs::write(root.join("new").join("f"), vec![0u8; 3000]).unwrap();
        std::fs::write(root.join("keeps").join("g"), vec![0u8; 2000]).unwrap();
        let cur = tree_of(&root);
        let r = growth_in(&cur, &json!({}), Some(&base), SystemTime::now()).unwrap();
        assert!(r.contains("6 days ago"), "{r}");
        assert!(r.contains("new (new"), "{r}");
        assert!(r.contains("goes (gone"), "{r}");
        assert!(r.contains("keeps (now"), "{r}");
        let captured = SystemTime::UNIX_EPOCH + Duration::from_secs(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() - 3600);
        crate::history::prepare(&cur, captured, true).save(&base, u64::MAX).unwrap();
        crate::history::prepare(&cur, captured + Duration::from_secs(1800), true).save(&base, u64::MAX).unwrap();
        let later_question = growth_in(&cur, &json!({"days": 0}), Some(&base), captured + Duration::from_millis(900)).unwrap();
        assert!(later_question.contains("6 days ago"), "the old scan cannot compare with itself or a newer scan: {later_question}");
        assert!(growth_in(&cur, &json!({"days": u64::MAX}), Some(&base), captured).is_ok());
        let facts = crate::facts::Facts::default();
        let mut unknown = super::super::tools::Tools::new(&cur, &facts, "fixture", &[], &crate::memory::Memory::default());
        assert!(unknown.call("growth", &json!({})).unwrap_err().contains("capture time"));
        let none = growth_in(&cur, &json!({}), Some(&dir.join("empty")), SystemTime::now()).unwrap();
        assert!(none.contains("no earlier scan"), "{none}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
