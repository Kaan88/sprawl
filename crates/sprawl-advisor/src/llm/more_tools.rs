//! `search` and `growth`, for Claude Code through `sprawl --mcp` (next round, Phase 1): the
//! Search tab's language over a scan, and a folder compared with History. Read-only.

use std::fmt::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use serde_json::{json, Value};
use sprawl_core::{format_size, Tree, ROOT};

use super::report::node_line_at;
use super::ToolSpec;
use crate::facts::Facts;

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "search",
            description: "Find files and folders by name, size, date or type in a scan, with the language of Sprawl's Search tab: words (all must match, \
                any case), *.iso and img_????.jpg patterns, a|b (either), !tmp (not), >1gb <10kb (size; a folder's size is its total), ext:mp4,mkv, \
                dm:2024 dm:>2025-06-01 (modified), dc:2024 (created), da:2024 da:>2025-06-01 (last opened: the files' Windows last-access times, read for each match, \
                so give it with another narrowing word), folder: or file:, path:steam (in the path), depth:<3, regex:^IMG_\\d+$, parent:C:\\Games (directly \
                inside it), child:*.tmp (folders holding one), dupe:, sizedupe:, attrib:hs, len:>260, audio: video: pic: doc: exe: zip:, and groups <a b>|c. \
                With path, only inside that folder (and the scan holding it is the one searched). Biggest first. \
                Each result says when it was last opened and changed, and, for a program, when one with its name last ran (by program name, any folder). \n                A folder's last-opened date comes from its listing: files open right now, or used under another hard-link name, can show an older date.",
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

pub fn search(tree: &Tree, facts: &Facts, input: &Value, access: &mut crate::used::Accessed) -> Result<String, String> {
    search_cancellable(tree, facts, input, access, &AtomicBool::new(false))
}

const SEARCH_CANCELLED: &str = "Sprawl cancelled the request.";

pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) { Err(SEARCH_CANCELLED.into()) } else { Ok(()) }
}

pub(crate) fn search_cancellable(tree: &Tree, facts: &Facts, input: &Value, access: &mut crate::used::Accessed, cancel: &AtomicBool) -> Result<String, String> {
    check_cancel(cancel)?;
    let text = input["query"].as_str().map(str::trim).filter(|q| !q.is_empty()).ok_or("give a `query`, e.g. ext:iso >1gb")?;
    // `dc:` and `da:` aren't in the scan: taken out here, and only the matches the other words
    // leave are read (a time is read per path, never scanned)
    let (plain, dates) = sprawl_core::query::split_read_dates(text, SystemTime::now());
    let dates = match dates {
        Ok(d) => d,
        Err(e) => return Err(format!("The search has mistakes: {e}.\n\nThe language:\n{}", sprawl_core::query::HELP)),
    };
    if !dates.is_empty() && plain.trim().is_empty() {
        return Err("dc: and da: read each match's own time, so give them another word to narrow the search first (a name, ext:, size: or a folder).".into());
    }
    let q = sprawl_core::query::Query::parse(&plain);
    if !q.errors().is_empty() {
        return Err(format!("The search has mistakes: {}.\n\nThe language:\n{}", q.errors().join("; "), sprawl_core::query::HELP));
    }
    let top = node_or_root(tree, input)?;
    let max = input["limit"].as_u64().unwrap_or(30).clamp(1, 100) as usize;
    let index = sprawl_core::search::Index::build_until(tree, cancel).ok_or(SEARCH_CANCELLED)?;
    let matches = sprawl_core::search::search(tree, &index, &q, cancel).ok_or(SEARCH_CANCELLED)?;
    let mut hits = Vec::new();
    for n in matches {
        check_cancel(cancel)?;
        if n != top && within(tree, n, top) { hits.push(n); }
    }
    // `dc:`: each match's creation time, read for the matches the other words left
    let mut created_notes: Vec<String> = Vec::new();
    // matches the other words left, and how many of them no time could be read for (times-20)
    let asked = hits.len();
    let mut created_missed = 0usize;
    if let Some((from, to)) = dates.created {
        // a view that holds no disk times reads nothing, and says so instead of looking
        if let Some(unseen) = access.unseen() {
            return Err(unseen_created(unseen));
        }
        if hits.len() > ACCESS_MOST {
            return Err(format!(
                "dc: would read the creation time of {} matches; add a word that narrows the search first (a name, ext:, size: or a folder).",
                hits.len()
            ));
        }
        let mut born = crate::born::Created::new();
        let mut unread = 0usize;
        let mut unknown_made = 0usize;
        hits.retain(|&n| !cancel.load(Ordering::Relaxed) && match born.of(&tree.path_of(n)) {
            crate::born::Born::At(t) => {
                let s = sprawl_core::query::unix_secs(t);
                from <= s && s < to
            }
            crate::born::Born::Unread => {
                unread += 1;
                false
            }
            // counted with its reason inside `born`
            crate::born::Born::Unknown => {
                unknown_made += 1;
                false
            }
        });
        check_cancel(cancel)?;
        created_missed = unread + unknown_made;
        if unread > 0 {
            created_notes.push(format!("{unread} matches not checked for created: the reading limit ran out."));
        }
        created_notes.extend(born.notes("match", "matches"));
    }
    // `da:`: each match's time is read once and used again for the list; what the reading limit
    // left out is counted and said, never taken for a non-match
    let mut by_time = false;
    let mut timed: Vec<(usize, (SystemTime, bool))> = Vec::new();
    let (mut unread, mut unknown, mut partial) = (0usize, 0usize, 0usize);
    if let Some((from, to)) = dates.accessed {
        by_time = true;
        let (kept, left_out, not_full, no_times) = accessed_hits(tree, hits, from, to, access, cancel)?;
        timed = kept;
        unread = left_out;
        partial = not_full;
        unknown = no_times;
        hits = timed.iter().map(|&(n, _)| n).collect();
    }
    let partial_note = crate::used::partial_note(partial, "match", "matches");
    if hits.is_empty() {
        let missed = unread + unknown + partial + created_missed;
        let decided = asked.saturating_sub(missed);
        let mut s = if missed > 0 && decided > 0 {
            // some were read and none is in the range: not "nothing could be checked"
            format!(
                "None of the {decided} {} read for {text} in {} is in the range; {missed} couldn't be checked.",
                if decided == 1 { "match" } else { "matches" },
                tree.path_of(top).display()
            )
        } else if missed > 0 {
            format!("Nothing in {} could be checked for {text}: their times weren't read.", tree.path_of(top).display())
        } else {
            format!("Nothing in {} matches {text}.", tree.path_of(top).display())
        };
        for note in &created_notes {
            let _ = write!(s, "\n({note})");
        }
        for note in partial_note.iter().chain(crate::used::notes(unread, unknown, "match", "matches", access).iter()) {
            let _ = write!(s, "\n({note})");
        }
        return Ok(s);
    }
    hits.sort_by_key(|&n| std::cmp::Reverse(tree.nodes[n].size));
    timed.sort_by_key(|&(n, _)| std::cmp::Reverse(tree.nodes[n].size));
    // a folder's total holds its matching children's: the total counts outermost matches
    let hit_set: std::collections::HashSet<usize> = hits.iter().copied().collect();
    let mut total = 0;
    for &n in &hits {
        check_cancel(cancel)?;
        if !has_hit_ancestor(&hit_set, tree, n) { total += tree.nodes[n].size; }
    }
    let mut s = format!("{} matches for {text} in {}, {} in all. Biggest first:\n", hits.len(), tree.path_of(top).display(), format_size(total));
    if by_time {
        for &(n, a) in timed.iter().take(max) {
            let kind = if tree.nodes[n].is_dir { "folder" } else { "file" };
            let _ = writeln!(s, "- {} [{kind}] {}", tree.path_of(n).display(), node_line_at(tree, facts, n, Some(a)));
        }
    } else {
        for &n in hits.iter().take(max) {
            check_cancel(cancel)?;
            let kind = if tree.nodes[n].is_dir { "folder" } else { "file" };
            let a = access.time(&tree.path_of(n), &mut unread, &mut unknown);
            let _ = writeln!(s, "- {} [{kind}] {}", tree.path_of(n).display(), node_line_at(tree, facts, n, a));
        }
    }
    if hits.len() > max {
        let _ = writeln!(s, "({} more, smaller)", hits.len() - max);
    }
    for note in &created_notes {
        let _ = writeln!(s, "({note})");
    }
    for note in partial_note.iter().chain(crate::used::notes(unread, unknown, "match", "matches", access).iter()) {
        let _ = writeln!(s, "({note})");
    }
    Ok(s)
}

/// The most matches a `da:` search reads an access time for: the same as one answer's reading
/// budget, since past it every further match would only be counted as not read.
const ACCESS_MOST: usize = crate::used::ANSWER_BUDGET;

/// Why `dc:` can't read anything for this view.
fn unseen_created(unseen: crate::used::Unseen) -> String {
    let held = match unseen {
        crate::used::Unseen::ScanOnly => "This view may only use the scan",
        crate::used::Unseen::Csv => "This view came from a CSV file",
    };
    format!("{held}, which holds no creation times, so dc: can't search by when files were created.")
}

/// Why `da:` can't read anything for this view.
fn unseen_accessed(unseen: crate::used::Unseen) -> String {
    let held = match unseen {
        crate::used::Unseen::ScanOnly => "This view may only use the scan",
        crate::used::Unseen::Csv => "This view came from a CSV file",
    };
    format!("{held}, which holds no last-access times, so da: can't search by when files were last opened.")
}

/// `hits` whose last-access time is inside `[from, to)` (seconds since 1970), as (node, time and
/// whether all of it was read); and how many were left out by the reading limit, how many were
/// read only in part (a skipped or unreadable folder inside) and how many have no time here. A
/// partly read folder is not a "no": it counts as not fully read.
fn accessed_hits(
    tree: &Tree,
    hits: Vec<usize>,
    from: i64,
    to: i64,
    access: &mut crate::used::Accessed,
    cancel: &AtomicBool,
) -> Result<(Vec<(usize, (SystemTime, bool))>, usize, usize, usize), String> {
    use crate::used::LastOpened;
    if let Some(unseen) = access.unseen() {
        return Err(unseen_accessed(unseen));
    }
    if !access.kept() {
        return Err("This search asks when files were last opened, but this PC has Windows' last-access updates turned off, so Sprawl has no such times to check.".into());
    }
    if hits.len() > ACCESS_MOST {
        return Err(format!("da: would read the last-access time of {} matches; add a word that narrows the search first (a name, ext:, size: or a folder).", hits.len()));
    }
    let (mut kept, mut unread, mut partial, mut unknown) = (Vec::new(), 0, 0, 0);
    for n in hits {
        check_cancel(cancel)?;
        match access.of(&tree.path_of(n)) {
            LastOpened::At(t, true) => {
                let s = sprawl_core::query::unix_secs(t);
                if from <= s && s < to {
                    kept.push((n, (t, true)));
                }
            }
            LastOpened::At(_, false) => partial += 1,
            // the reading limit ran out before the end of it: never decided either way
            LastOpened::Limited(_) | LastOpened::Unread => unread += 1,
            LastOpened::Unknown => unknown += 1,
        }
    }
    Ok((kept, unread, partial, unknown))
}

/// Whether a match sits above `n` (a match inside a matching folder isn't counted twice). The
/// matches are a set, so the total is linear in the number of matches, not quadratic.
fn has_hit_ancestor(hits: &std::collections::HashSet<usize>, tree: &Tree, n: usize) -> bool {
    let mut cur = tree.nodes[n].parent;
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
    let n = match crate::mcp_scan::path_of(input) {
        Some(path) => crate::history::find_path(tree, &path).ok_or_else(|| format!("{} {}", path.display(), crate::mcp_scan::NOT_IN_SCAN))?,
        None => ROOT,
    };
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
        let mut access = crate::used::Accessed::new();
        let r = search(&t, &facts, &json!({"query": "ext:iso"}), &mut access).unwrap();
        assert!(r.starts_with("2 matches") && r.find("big.iso") < r.find("small.iso"), "{r}");
        let r = search(&t, &facts, &json!({"query": "*.iso >10kb"}), &mut crate::used::Accessed::new()).unwrap();
        assert!(r.contains("big.iso") && !r.contains("small.iso"), "{r}");
        let inside = search(&t, &facts, &json!({"query": "iso", "path": dir.join("a").to_string_lossy()}), &mut crate::used::Accessed::new()).unwrap();
        assert!(inside.starts_with("1 matches"), "{inside}");
        assert!(search(&t, &facts, &json!({"query": "size:>>"}), &mut crate::used::Accessed::new()).is_err());
        assert!(search(&t, &facts, &json!({"query": "zzz-none"}), &mut crate::used::Accessed::new()).unwrap().starts_with("Nothing"));
        // `da:` needs another word, and a value that isn't a date is said
        assert!(search(&t, &facts, &json!({"query": "da:2026-01"}), &mut crate::used::Accessed::new()).is_err());
        assert!(search(&t, &facts, &json!({"query": "ext:iso da:2026-13"}), &mut crate::used::Accessed::new()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_by_last_opened_reads_the_files_time() {
        let dir = std::env::temp_dir().join(format!("sprawl-more-da-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.txt"), b"x").unwrap();
        let t = tree_of(&dir);
        let facts = Facts::from_apps(Vec::new());
        let mut access = crate::used::Accessed::new();
        if access.kept() {
            let today = search(&t, &facts, &json!({"query": "*.txt da:today"}), &mut access).unwrap();
            assert!(today.contains("note.txt") && today.contains("last opened"), "{today}");
            let future = search(&t, &facts, &json!({"query": "*.txt da:>2030-01-01"}), &mut crate::used::Accessed::new()).unwrap();
            assert!(future.starts_with("Nothing"), "{future}");
        }
        // a reading limit that runs out is said, never turned into "nothing matches"
        let tiny = search(&t, &facts, &json!({"query": "*.txt da:today"}), &mut crate::used::Accessed::with_budget(0)).unwrap();
        assert!(tiny.contains("could be checked"), "{tiny}");
        assert!(tiny.contains("1 match not checked for a last-opened time: the reading limit ran out"), "{tiny}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Matches that were read and are out of the range say so; only matches no time could be read
    /// for are "not checked" (times-20).
    #[test]
    fn an_empty_range_is_not_reported_as_nothing_checked() {
        let dir = std::env::temp_dir().join(format!("sprawl-more-decided-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["a.txt", "b.txt", "c.txt", "gone.txt"] {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        let t = tree_of(&dir);
        // gone since the scan: no time to read for this one
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        let facts = Facts::from_apps(Vec::new());
        let r = search(&t, &facts, &json!({"query": "*.txt dc:<2000"}), &mut crate::used::Accessed::new()).unwrap();
        assert!(r.starts_with("None of the 3 matches read for *.txt dc:<2000"), "{r}");
        assert!(r.contains("1 couldn't be checked") && !r.contains("Nothing"), "{r}");
        assert!(r.contains("gone"), "the reason is still said: {r}");
        // all unread: still "could be checked"
        let tiny = search(&t, &facts, &json!({"query": "*.txt da:today"}), &mut crate::used::Accessed::with_budget(0)).unwrap();
        assert!(tiny.contains("could be checked"), "{tiny}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A client allowed only the scan reads nothing and is told so for both words; a CSV view
    /// says where it came from.
    #[test]
    fn a_scan_only_or_csv_view_cannot_search_by_a_time() {
        use crate::used::{Accessed, Unseen};
        let dir = std::env::temp_dir().join(format!("sprawl-more-times-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.txt"), b"x").unwrap();
        let t = tree_of(&dir);
        let facts = Facts::from_apps(Vec::new());
        let e = search(&t, &facts, &json!({"query": "*.txt dc:today"}), &mut Accessed::off()).unwrap_err();
        assert!(e.contains("may only use the scan") && e.contains("dc:"), "{e}");
        let e = search(&t, &facts, &json!({"query": "*.txt da:today"}), &mut Accessed::off()).unwrap_err();
        assert!(e.contains("may only use the scan") && e.contains("da:"), "{e}");
        let e = search(&t, &facts, &json!({"query": "*.txt dc:today"}), &mut Accessed::off_for(Unseen::Csv)).unwrap_err();
        assert!(e.contains("CSV"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder and a matching file inside it count once in the total, not twice.
    #[test]
    fn a_folder_and_its_matching_child_count_once() {
        let dir = std::env::temp_dir().join(format!("sprawl-more-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("iso")).unwrap();
        std::fs::write(dir.join("iso").join("big.iso"), vec![0u8; 20_000]).unwrap();
        let t = tree_of(&dir);
        let facts = Facts::from_apps(Vec::new());
        let r = search(&t, &facts, &json!({"query": "iso"}), &mut crate::used::Accessed::new()).unwrap();
        assert!(r.starts_with("2 matches"), "{r}");
        let once = format!("{} in all", sprawl_core::format_size(20_000));
        assert!(r.contains(&once), "{r}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn growth_selects_case_distinct_current_siblings_exactly() {
        use sprawl_core::{Node, ScanMethod};
        let dir = std::env::temp_dir().join(format!("sprawl-growth-case-{}", std::process::id()));
        let root = dir.join("Synthetic");
        let mut old = Tree { volumes: Vec::new(), root_path: root.clone(), nodes: vec![
            Node { name: root.display().to_string(), is_dir: true, children: vec![1, 2], ..Default::default() },
            Node { name: "Data".into(), is_dir: true, parent: Some(0), children: vec![3], ..Default::default() },
            Node { name: "data".into(), is_dir: true, parent: Some(0), children: vec![4], ..Default::default() },
            Node { name: "file".into(), size: 10, files: 1, parent: Some(1), ..Default::default() },
            Node { name: "file".into(), size: 100, files: 1, parent: Some(2), ..Default::default() },
        ], errors: 0, method: ScanMethod::Walk };
        old.compute_totals();
        let now = SystemTime::now();
        crate::history::prepare(&old, now - Duration::from_secs(86_400), true).save(&dir, u64::MAX).unwrap();
        let mut current = old.clone();
        current.nodes[3].size = 20;
        current.nodes[4].size = 300;
        current.compute_totals();
        let answer = growth_in(&current, &json!({"path": root.join("data")}), Some(&dir), now).unwrap();
        assert!(answer.contains(&format!("{} now; {} in History", format_size(300), format_size(100))), "{answer}");
        std::fs::remove_dir_all(dir).unwrap();
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
