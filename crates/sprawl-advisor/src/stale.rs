//! Big things nothing has changed in for a long time: the folders and files a person would
//! look at first on a drive of games, media or backups, where no rule knows a cache.
//!
//! These are information only. Sprawl doesn't know whether old data is unwanted: it says how
//! big it is and when anything in it last changed, and the user decides. Nothing here has an
//! action, can be ticked, or counts as space Sprawl can clean.
//!
//! "Changed", not "used": Windows doesn't record reliably when a file was last opened, and a
//! game played yesterday may hold files last written years ago. Every finding says so.

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use sprawl_core::{Tree, ROOT};

use crate::facts::{month_year, Facts};
use crate::finding::{Finding, Remedy, Risk, Source};

/// The rule id of what this finds.
pub const ID: &str = "stale.big";

/// At least this big.
pub const MIN_BYTES: u64 = 2 << 30;
/// Nothing in it changed for this long.
pub const IDLE_DAYS: u64 = 365;
/// The most that are listed, biggest first.
pub const MOST: usize = 25;
/// How deep below the scan's root a folder is still looked into.
const DEPTH: usize = 5;

/// The newest change at or below every node (links not followed), None where no date is known
/// anywhere below.
fn newest(tree: &Tree) -> Vec<Option<SystemTime>> {
    let mut out: Vec<Option<SystemTime>> = tree.nodes.iter().map(|n| n.modified).collect();
    // children come after their parents in a walk from the root, so the reverse of that order
    // has every child before its parent
    let mut order = Vec::with_capacity(tree.nodes.len());
    let mut stack = vec![ROOT];
    while let Some(n) = stack.pop() {
        order.push(n);
        let node = &tree.nodes[n];
        if node.is_dir && (n == ROOT || !node.is_reparse_point()) {
            stack.extend_from_slice(&node.children);
        }
    }
    for &n in order.iter().rev() {
        let node = &tree.nodes[n];
        if node.is_dir && (n == ROOT || !node.is_reparse_point()) {
            // a folder's own date changes when a name in it does, which is a change too
            let mut best = out[n];
            for &c in &node.children {
                best = best.max(out[c]);
            }
            out[n] = best;
        }
    }
    out
}

/// The biggest folders and files of at least [`MIN_BYTES`] in which nothing changed for
/// [`IDLE_DAYS`]: the highest such folder is named, not what is inside it. Left out: what is
/// on the never-list or inside it (Windows, program installations, profiles' own folders),
/// what another finding already covers (`seen`, lower-case paths, either way round), links,
/// and anything without a date.
pub fn find(tree: &Tree, facts: &Facts, seen: &HashSet<String>) -> Vec<Finding> {
    find_at(tree, facts, seen, SystemTime::now())
}

pub(crate) fn find_at(tree: &Tree, facts: &Facts, seen: &HashSet<String>, now: SystemTime) -> Vec<Finding> {
    let never = crate::safety::Never::default_list();
    let since = now.checked_sub(Duration::from_secs(IDLE_DAYS * 86_400)).unwrap_or(SystemTime::UNIX_EPOCH);
    let newest = newest(tree);
    let mut out = Vec::new();
    let mut stack: Vec<(usize, usize)> = tree.nodes[ROOT].children.iter().map(|&c| (c, 1)).collect();
    while let Some((n, depth)) = stack.pop() {
        let node = &tree.nodes[n];
        if node.size < MIN_BYTES || node.is_reparse_point() {
            continue;
        }
        let path = tree.path_of(n);
        let s = path.to_string_lossy().to_string();
        let lc = s.to_lowercase();
        // Windows' own, program installations and the like: not the user's old data
        if never.hit(&s).is_some() {
            // a protected folder may still hold the user's folders (C:\Users\name): look inside
            if node.is_dir && depth < DEPTH {
                stack.extend(node.children.iter().map(|&c| (c, depth + 1)));
            }
            continue;
        }
        if seen.iter().any(|x| x == &lc || x.starts_with(&format!("{lc}\\")) || lc.starts_with(&format!("{x}\\"))) {
            continue;
        }
        let idle = newest[n].is_some_and(|t| t < since);
        if !idle {
            // something in it changed lately: its parts may still be old
            if node.is_dir && depth < DEPTH {
                stack.extend(node.children.iter().map(|&c| (c, depth + 1)));
            }
            continue;
        }
        let changed = newest[n].unwrap_or(SystemTime::UNIX_EPOCH);
        let owner = facts.owner_of(&path).map(|a| a.name.clone());
        let mut evidence = vec![
            format!("Nothing in it changed since {}", month_year(changed)),
            "That is when it last changed, not when it was last opened: Windows doesn't keep that reliably, and a game or film you still use may not have changed in years".to_string(),
        ];
        if let Some(o) = &owner {
            evidence.push(format!("It belongs to {o}, which is installed"));
        }
        if node.is_dir {
            evidence.push(format!("{} files in {} folders", node.files, node.folders));
        }
        evidence.push("Sprawl doesn't remove this: it is your data. Look at it in the tree or in Explorer, then move it to another drive or delete it yourself if you no longer need it".to_string());
        out.push(Finding {
            path,
            owner,
            bytes: node.size,
            risk: Risk::Orange,
            remedy: Remedy::Info,
            command: None,
            title: format!("{}: not changed since {}", node.name, month_year(changed)),
            evidence,
            confidence: 0.5,
            source: Source::Rule(ID.into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: false,
            action: None,
            admin: false,
            carved: Vec::new(),
            gain: None,
        });
    }
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out.truncate(MOST);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sprawl_core::Node;

    fn node(name: &str, dir: bool, size: u64, days_ago: Option<u64>, parent: Option<usize>, now: SystemTime) -> Node {
        Node { name: name.into(), is_dir: dir, size, allocated: size, files: u64::from(!dir), folders: 0, modified: days_ago.map(|d| now - Duration::from_secs(d * 86_400)), attributes: 0, parent, children: Vec::new() }
    }

    /// D:\ with an old game, a folder that is partly old, a small old folder, and an old big file.
    fn tree(now: SystemTime) -> Tree {
        const GB: u64 = 1 << 30;
        let mut t = sprawl_core::scan(&std::env::temp_dir().join("sprawl-stale-none"), |_| {}).unwrap_or_else(|_| {
            let dir = std::env::temp_dir().join(format!("sprawl-stale-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            let t = sprawl_core::scan(&dir, |_| {}).unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            t
        });
        t.root_path = r"D:\".into();
        t.nodes.truncate(1);
        t.nodes[ROOT] = node(r"D:\", true, 0, Some(1), None, now);
        let add = |t: &mut Tree, n: Node| -> usize {
            let i = t.nodes.len();
            let p = n.parent.unwrap();
            t.nodes.push(n);
            t.nodes[p].children.push(i);
            i
        };
        let games = add(&mut t, node("Games", true, 60 * GB, Some(900), Some(ROOT), now));
        let old = add(&mut t, node("OldGame", true, 40 * GB, Some(800), Some(games), now));
        add(&mut t, node("data.pak", false, 40 * GB, Some(800), Some(old), now));
        let fresh = add(&mut t, node("NewGame", true, 20 * GB, Some(700), Some(games), now));
        add(&mut t, node("save.dat", false, 20 * GB, Some(3), Some(fresh), now));
        let small = add(&mut t, node("Notes", true, GB, Some(2000), Some(ROOT), now));
        add(&mut t, node("a.txt", false, GB, Some(2000), Some(small), now));
        add(&mut t, node("backup-2019.iso", false, 8 * GB, Some(2500), Some(ROOT), now));
        add(&mut t, node("undated.bin", false, 9 * GB, None, Some(ROOT), now));
        t
    }

    #[test]
    fn old_big_things_are_named_and_never_actionable() {
        let now = SystemTime::now();
        let t = tree(now);
        let facts = Facts::from_apps(Vec::new());
        let found = find_at(&t, &facts, &HashSet::new(), now);
        let names: Vec<String> = found.iter().map(|f| f.path.display().to_string()).collect();
        // Games as a whole changed three days ago (a save in NewGame), so its old part is named;
        // NewGame is not; the small folder and the undated file are not; the old image is
        assert_eq!(names, [r"D:\Games\OldGame", r"D:\backup-2019.iso"], "{names:?}");
        for f in &found {
            assert_eq!(f.remedy, Remedy::Info);
            assert!(f.action.is_none() && f.command.is_none());
            assert!(crate::clean::action(f).is_none(), "nothing Sprawl can run");
            assert_eq!(f.group(), crate::finding::Group::Yourself);
            assert!(f.evidence.iter().any(|e| e.contains("not when it was last opened")));
        }
        // what another finding covers isn't said twice, whichever is inside the other
        let seen: HashSet<String> = [r"d:\games\oldgame\data.pak".to_string()].into();
        assert_eq!(find_at(&t, &facts, &seen, now).len(), 1);
        let seen: HashSet<String> = [r"d:\games".to_string()].into();
        assert_eq!(find_at(&t, &facts, &seen, now).len(), 1);
    }
}
