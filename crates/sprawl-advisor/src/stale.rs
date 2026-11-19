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
    // last-access times, only where this machine keeps them, on NTFS drives, within a budget
    let kept = crate::used::access_times_kept();
    let mut ntfs: std::collections::HashMap<char, bool> = Default::default();
    let mut budget = ACCESS_BUDGET;
    let mut access = |p: &std::path::Path| -> Option<(SystemTime, bool)> {
        let drive = p.to_string_lossy().chars().next()?.to_ascii_uppercase();
        if !kept || !drive.is_ascii_alphabetic() || !*ntfs.entry(drive).or_insert_with(|| crate::used::ntfs(drive)) {
            return None;
        }
        let (t, complete) = crate::used::accessed_in(p, &mut budget);
        t.map(|t| (t, complete))
    };
    find_at(tree, facts, seen, SystemTime::now(), &mut access)
}

/// The most folder entries read for last-access times in one look.
const ACCESS_BUDGET: usize = 200_000;
/// Where the last-access times come from, as the rows say.
const ACCESS: &str = "the files' last-access times, which this PC keeps";

/// `access`: the newest last-access time of the files in a path, and whether all of them were
/// read; None where these times aren't kept.
pub(crate) fn find_at(tree: &Tree, facts: &Facts, seen: &HashSet<String>, now: SystemTime, access: &mut dyn FnMut(&std::path::Path) -> Option<(SystemTime, bool)>) -> Vec<Finding> {
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
        // evidence of use (A1): the owning app's last start or play, and Explorer's Recent items
        let app = facts.owner_of(&path);
        let mut used = [
            app.and_then(|a| a.last_used).map(|(t, from)| (t, if from == crate::facts::UsedFrom::Steam { "played" } else { "used" }, from.label())),
            crate::used::opened_in(&facts.opened, &s, facts.exact).map(|t| (t, "opened", "Explorer's Recent items")),
        ]
        .into_iter()
        .flatten()
        .max_by_key(|u| u.0);
        let changed = newest[n].unwrap_or(SystemTime::UNIX_EPOCH);
        let mut idle = newest[n].is_some_and(|t| t < since) && used.is_none_or(|u| u.0 < since);
        // and the files' last-access times: opened lately, or not opened since they changed
        let mut not_opened = false;
        if idle {
            if let Some((t, complete)) = access(&path) {
                if t >= since {
                    idle = false;
                } else if complete && t > changed + Duration::from_secs(86_400) {
                    if used.is_none_or(|u| t > u.0) {
                        used = Some((t, "opened", ACCESS));
                    }
                } else if complete {
                    not_opened = true;
                }
            }
        }
        if !idle {
            // something in it changed or was used lately: its parts may still be old
            if node.is_dir && depth < DEPTH {
                stack.extend(node.children.iter().map(|&c| (c, depth + 1)));
            }
            continue;
        }
        let owner = app.map(|a| a.name.clone());
        let mut evidence = match used {
            Some((t, how, from)) if from == ACCESS => vec![format!("Last {how} {} ({from})", month_year(t)), format!("Nothing in it changed since {}", month_year(changed))],
            Some((t, how, from)) => vec![
                format!("Last {how} {} ({from})", month_year(t)),
                format!("Nothing in it changed since {}", month_year(changed)),
                format!("{from} shows use, not disuse: it may have been used another way since"),
            ],
            None if not_opened => vec![format!("Nothing in it changed or was opened since {} ({ACCESS})", month_year(changed))],
            None => vec![
                format!("Nothing in it changed since {}", month_year(changed)),
                "That is when it last changed, not when it was last opened: Windows doesn't keep that reliably, and a game or film you still use may not have changed in years".to_string(),
            ],
        };
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
            title: match used {
                Some((t, how, _)) => format!("{}: last {how} {}", node.name, month_year(t)),
                None => format!("{}: not changed since {}", node.name, month_year(changed)),
            },
            evidence,
            // a date of use is better evidence than one of change
            confidence: if used.is_some() { 0.6 } else { 0.5 },
            source: Source::Rule(ID.into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: false,
            action: None,
            admin: false,
            carved: Vec::new(),
            gain: None,
            forced_from: None,
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
        Node { name: name.into(), is_dir: dir, size, allocated: size, files: u64::from(!dir), folders: 0, modified: days_ago.map(|d| now - Duration::from_secs(d * 86_400)), attributes: 0, parent, children: Vec::new(), id: 0 }
    }

    /// D:\ with an old game, a folder that is partly old, a small old folder, and an old big file.
    fn tree(now: SystemTime) -> Tree {
        const GB: u64 = 1 << 30;
        let mut t = Tree { volumes: Vec::new(),
            root_path: r"D:\".into(),
            nodes: vec![node(r"D:\", true, 0, Some(1), None, now)],
            errors: 0,
            method: sprawl_core::ScanMethod::Walk,
        };
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
        let found = find_at(&t, &facts, &HashSet::new(), now, &mut |_| None);
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
        assert_eq!(find_at(&t, &facts, &seen, now, &mut |_| None).len(), 1);
        let seen: HashSet<String> = [r"d:\games".to_string()].into();
        assert_eq!(find_at(&t, &facts, &seen, now, &mut |_| None).len(), 1);
    }

    /// A1: something opened inside lately keeps a folder off the list (its old parts may still
    /// be named); use that is old too is named with its source.
    #[test]
    fn evidence_of_use_drops_or_names_a_row() {
        let now = SystemTime::now();
        let t = tree(now);
        let ago = |d: u64| now - Duration::from_secs(d * 86_400);
        let mut facts = Facts::from_apps(Vec::new());
        facts.opened = vec![(r"d:\games\oldgame\data.pak".into(), ago(20)), (r"d:\backup-2019.iso".into(), ago(500))];
        let found = find_at(&t, &facts, &HashSet::new(), now, &mut |_| None);
        let names: Vec<&str> = found.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(names, [format!("backup-2019.iso: last opened {}", month_year(ago(500)))], "{names:?}");
        assert!(found[0].evidence[0].contains("Explorer's Recent items"));
        // an installed game last played long ago, by Steam
        let mut app = crate::facts::App { name: "Old Game".into(), publisher: None, kind: crate::facts::AppKind::Steam, locations: vec![r"D:\Games\OldGame".into()], last_used: Some((ago(400), crate::facts::UsedFrom::Steam)), how_to_remove: String::new() };
        let facts = Facts::from_apps(vec![app.clone()]);
        let found = find_at(&t, &facts, &HashSet::new(), now, &mut |_| None);
        assert!(found.iter().any(|f| f.title == format!("OldGame: last played {}", month_year(ago(400)))), "{:?}", found.iter().map(|f| &f.title).collect::<Vec<_>>());
        // played last week: not listed
        app.last_used = Some((ago(7), crate::facts::UsedFrom::Steam));
        let found = find_at(&t, &Facts::from_apps(vec![app]), &HashSet::new(), now, &mut |_| None);
        assert!(!found.iter().any(|f| f.path.ends_with("OldGame")));
    }

    /// A1 step 2: last-access times hide what was opened lately, name what was opened after it
    /// last changed, and say so when nothing was opened since; a partial read proves nothing old.
    #[test]
    fn last_access_times_where_kept() {
        let now = SystemTime::now();
        let t = tree(now);
        let ago = |d: u64| now - Duration::from_secs(d * 86_400);
        let facts = Facts::from_apps(Vec::new());
        let run = |answer: &dyn Fn(&str) -> Option<(SystemTime, bool)>| {
            let found = find_at(&t, &facts, &HashSet::new(), now, &mut |p| answer(&p.to_string_lossy()));
            found.into_iter().map(|f| (f.title, f.evidence[0].clone())).collect::<Vec<_>>()
        };
        // the game opened last month: gone; the image opened long after it was written
        let found = run(&|p| if p.contains("OldGame") { Some((ago(30), true)) } else { Some((ago(600), true)) });
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, format!("backup-2019.iso: last opened {}", month_year(ago(600))));
        assert!(found[0].1.contains("last-access"));
        // read as written, never opened since
        let found = run(&|p| if p.ends_with("OldGame") { Some((ago(800), true)) } else { Some((ago(2500), true)) });
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|(_, e)| e.contains("changed or was opened since")), "{found:?}");
        // not all read: an old time says nothing
        let found = run(&|_| Some((ago(2500), false)));
        assert!(found.iter().all(|(_, e)| e.starts_with("Nothing in it changed since")), "{found:?}");
    }
}
