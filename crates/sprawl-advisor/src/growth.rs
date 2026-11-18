//! Time: how fast the scanned folder or drive grows, when the drive would be full at that rate,
//! and what grew since about a week ago, explained (which app, which file types, how many files
//! changed). Built from the snapshots History saves after every scan.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime};

use sprawl_core::{format_size, Tree, ROOT};

use crate::facts::{month_year, Facts};
use crate::history::{self, Entry};

const DAY: f64 = 86_400.0;

#[derive(Clone, Debug)]
pub struct Grower {
    pub node: usize,
    /// Bytes more (or less) than at `Growth::since`.
    pub delta: i64,
    /// Why, one point per line.
    pub why: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Growth {
    /// (when, size of the scanned folder), oldest first, ending with now.
    pub points: Vec<(SystemTime, u64)>,
    /// Bytes a day, from a straight line through the points of the last 60 days.
    pub per_day: Option<f64>,
    /// At that rate, when the drive's free space runs out (only when it grows).
    pub full_in_days: Option<f64>,
    /// The snapshot "what grew" compares with.
    pub since: Option<SystemTime>,
    /// Total change since then.
    pub change: i64,
    pub grew: Vec<Grower>,
}

impl Growth {
    /// A few sentences for the model's report.
    pub fn text(&self, tree: &Tree) -> String {
        let mut s = String::new();
        if let Some(r) = self.per_day {
            s.push_str(&format!("\n## Growth\nThe scanned folder changes by about {} a day", signed(r as i64)));
            if let Some(d) = self.full_in_days {
                s.push_str(&format!("; at that rate the drive is full in about {d:.0} days"));
            }
            s.push_str(".\n");
        }
        if let Some(t) = self.since {
            s.push_str(&format!("Since {} it changed by {}. What grew most:\n", month_year(t), signed(self.change)));
            for g in &self.grew {
                s.push_str(&format!("- {} {} ({})\n", tree.path_of(g.node).display(), signed(g.delta), g.why.join("; ")));
            }
        }
        s
    }
}

/// The rule id of a growth finding.
pub const ID: &str = "growth.folder";

/// A folder that grew at least this much since the baseline is a finding of its own.
pub const FINDING_MIN: i64 = 1 << 30;

/// Folders that grew by [`FINDING_MIN`] or more since the snapshot `g` compares with, as
/// information: by how much, from what to what, and why. From the user's own history, which
/// says more than any usual size. `now` dates the baseline in days.
pub fn findings(g: &Growth, tree: &Tree, now: SystemTime) -> Vec<crate::finding::Finding> {
    findings_over(g, tree, now, FINDING_MIN)
}

fn findings_over(g: &Growth, tree: &Tree, now: SystemTime, min: i64) -> Vec<crate::finding::Finding> {
    let Some(since) = g.since else { return Vec::new() };
    let days = now.duration_since(since).map(|d| d.as_secs() / 86_400).unwrap_or(0).max(1);
    g.grew
        .iter()
        .filter(|w| w.delta >= min && w.node < tree.nodes.len())
        .map(|w| {
            let node = &tree.nodes[w.node];
            let was = (node.size as i64 - w.delta).max(0) as u64;
            let mut evidence = vec![format!(
                "Grew by {} in the last {days} days: {} then, {} now (from the scans History kept)",
                format_size(w.delta as u64),
                format_size(was),
                format_size(node.size)
            )];
            evidence.extend(w.why.iter().map(|s| {
                let mut c = s.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
            }));
            evidence.push("Information only: the Growth section shows the rate and what else grew".into());
            crate::finding::Finding {
                path: tree.path_of(w.node),
                owner: None,
                bytes: node.size,
                risk: crate::finding::Risk::Amber,
                remedy: crate::finding::Remedy::Info,
                command: None,
                title: format!("{} grew by {} in {days} days", node.name, format_size(w.delta as u64)),
                evidence,
                confidence: 0.8,
                source: crate::finding::Source::Rule(ID.into()),
                allow_protected: false,
                trusted: true,
                builtin: true,
                on_path: true,
                action: None,
                admin: false,
                carved: Vec::new(),
                gain: None,
                forced_from: None,
            }
        })
        .collect()
}

pub fn signed(v: i64) -> String {
    if v < 0 {
        format!("-{}", format_size(v.unsigned_abs()))
    } else {
        format!("+{}", format_size(v as u64))
    }
}

/// Least squares slope of (seconds, bytes), in bytes a day.
fn slope(points: &[(f64, f64)]) -> Option<f64> {
    if points.len() < 2 {
        return None;
    }
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    // the points must span a day at least, or the rate means nothing
    (sxx > 0.0 && points.last()?.0 - points.first()?.0 >= DAY).then(|| sxy / sxx * DAY)
}

/// Snapshots to read: the newest, and the ones nearest 1, 3, 7, 14, 30 and 60 days ago.
fn sample(entries: &[Entry], now: SystemTime) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for days in [0u64, 1, 3, 7, 14, 30, 60] {
        let want = now.checked_sub(Duration::from_secs(days * 86_400)).unwrap_or(now);
        let near = entries.iter().min_by_key(|e| if e.taken > want { e.taken.duration_since(want) } else { want.duration_since(e.taken) }.unwrap_or_default());
        if let Some(e) = near {
            if !out.iter().any(|o| o.taken == e.taken) {
                out.push(e.clone());
            }
        }
    }
    out.sort_by_key(|e| e.taken);
    out
}

/// Growth of `cur` from the snapshots in `base` (History's folder). `free` is the drive's free
/// space, for "full in". `now` is the captured scan time, never the question's wall-clock
/// time. `None` when there is no earlier snapshot of this folder.
pub fn growth(cur: &Tree, base: &Path, facts: &Facts, free: Option<u64>, scan_time: SystemTime) -> Option<Growth> {
    let now = scan_time;
    let root = cur.path_of(ROOT);
    // older than this scan: History may already hold this one
    let entries: Vec<Entry> = history::earlier(base, &root, now);
    if entries.is_empty() {
        return None;
    }
    let mut g = Growth::default();
    let mut baseline: Option<(Entry, Tree)> = None;
    let week = now.checked_sub(Duration::from_secs(7 * 86_400)).unwrap_or(now);
    let chosen = sample(&entries, now);
    // the baseline: the snapshot nearest a week ago that is at least a day old
    let base_entry = entries
        .iter()
        .filter(|e| now.duration_since(e.taken).is_ok_and(|d| d.as_secs() >= 86_400))
        .min_by_key(|e| if e.taken > week { e.taken.duration_since(week) } else { week.duration_since(e.taken) }.unwrap_or_default())
        .cloned();
    for e in &chosen {
        let Ok(old) = history::load(e) else { continue };
        let Some(n) = history::find_path(&old, &root) else { continue };
        g.points.push((e.taken, old.nodes[n].size));
        if base_entry.as_ref().is_some_and(|b| b.taken == e.taken) {
            baseline = Some((e.clone(), old));
        }
    }
    if baseline.is_none() {
        if let Some(b) = base_entry {
            if let Ok(old) = history::load(&b) {
                baseline = Some((b, old));
            }
        }
    }
    g.points.push((now, cur.nodes[ROOT].size));
    g.points.sort_by_key(|p| p.0);
    let first = g.points.first()?.0;
    let recent: Vec<(f64, f64)> = g
        .points
        .iter()
        .filter(|p| now.duration_since(p.0).is_ok_and(|d| d.as_secs() <= 60 * 86_400))
        .map(|p| (p.0.duration_since(first).unwrap_or_default().as_secs_f64(), p.1 as f64))
        .collect();
    g.per_day = slope(&recent);
    if let (Some(r), Some(f)) = (g.per_day, free) {
        if r > 1.0 {
            g.full_in_days = Some(f as f64 / r);
        }
    }
    if let Some((e, old)) = baseline {
        let was = history::project(cur, &old);
        g.since = Some(e.taken);
        g.change = cur.nodes[ROOT].size as i64 - was[ROOT] as i64;
        g.grew = growers(cur, &was, e.taken, facts);
    }
    Some(g)
}

/// The folders that grew most, each the one that holds the growth (not a parent that only
/// passes it on): at least 256 MB, at most 8.
fn growers(cur: &Tree, was: &[u64], since: SystemTime, facts: &Facts) -> Vec<Grower> {
    let delta = |n: usize| cur.nodes[n].size as i64 - was[n] as i64;
    let min = 256i64 << 20;
    let mut out = Vec::new();
    let mut stack = vec![ROOT];
    while let Some(n) = stack.pop() {
        let d = delta(n);
        if d < min || !cur.nodes[n].is_dir {
            continue;
        }
        // a child that holds most of it explains it better
        let big: Vec<usize> = cur.nodes[n].children.iter().copied().filter(|&c| cur.nodes[c].is_dir && delta(c) >= min).collect();
        let main = big.iter().copied().find(|&c| delta(c) * 10 >= d * 7);
        match main {
            Some(_) => stack.extend(big),
            None if n != ROOT => out.push(n),
            None => stack.extend(big),
        }
    }
    out.sort_by_key(|&n| std::cmp::Reverse(delta(n)));
    out.truncate(8);
    out.into_iter().map(|n| Grower { node: n, delta: delta(n), why: explain(cur, n, since, facts) }).collect()
}

/// What changed inside `n`: its app, how many files are new or changed, and their main types.
fn explain(cur: &Tree, n: usize, since: SystemTime, facts: &Facts) -> Vec<String> {
    let mut why = Vec::new();
    if let Some(app) = facts.owner_of(&cur.path_of(n)) {
        why.push(format!("belongs to {}", app.name));
    }
    let mut count = 0u64;
    let mut by_type: HashMap<String, u64> = HashMap::new();
    let mut stack = vec![n];
    while let Some(k) = stack.pop() {
        let node = &cur.nodes[k];
        if node.is_dir {
            if k == n || !node.is_reparse_point() {
                stack.extend_from_slice(&node.children);
            }
        } else if node.modified.is_some_and(|m| m > since) {
            count += 1;
            *by_type.entry(node.extension()).or_default() += node.size;
        }
    }
    if count > 0 {
        let mut types: Vec<(String, u64)> = by_type.into_iter().collect();
        types.sort_by_key(|t| std::cmp::Reverse(t.1));
        let top: Vec<String> = types.iter().take(3).map(|(e, b)| format!("{} {}", if e.is_empty() { "no-extension".into() } else { format!(".{e}") }, format_size(*b))).collect();
        why.push(format!("{count} files new or changed since {}, mostly {}", month_year(since), top.join(", ")));
    }
    why
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_needs_a_day() {
        assert_eq!(slope(&[(0.0, 100.0)]), None);
        assert_eq!(slope(&[(0.0, 100.0), (3600.0, 200.0)]), None, "an hour isn't a rate");
        let r = slope(&[(0.0, 0.0), (DAY, 1000.0), (2.0 * DAY, 2000.0)]).unwrap();
        assert!((r - 1000.0).abs() < 1e-6);
        assert_eq!(signed(-(1 << 30)), "-1.0 GB");
    }

    #[test]
    fn grows_against_an_old_snapshot() {
        let dir = std::env::temp_dir().join(format!("sprawl-growth-{}", std::process::id()));
        let snaps = dir.join("snaps");
        let data = dir.join("data");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(data.join("steady")).unwrap();
        std::fs::create_dir_all(data.join("logs").join("app")).unwrap();
        std::fs::write(data.join("steady").join("a.bin"), vec![0u8; 1000]).unwrap();
        let old = sprawl_core::scan(&data, |_| {}).unwrap();
        let week_ago = SystemTime::now() - Duration::from_secs(8 * 86_400);
        history::prepare(&old, week_ago, true).save(&snaps, 1 << 30).unwrap();
        // the logs grow by 300 MB (sparse, so nothing is really written)
        let f = std::fs::File::create(data.join("logs").join("app").join("big.log")).unwrap();
        f.set_len(300 << 20).unwrap();
        drop(f);
        let cur = sprawl_core::scan(&data, |_| {}).unwrap();
        let g = growth(&cur, &snaps, &Facts::from_apps(Vec::new()), Some(10 << 30), SystemTime::now()).unwrap();
        assert!(g.change >= 300 << 20);
        assert_eq!(g.grew.len(), 1);
        assert_eq!(cur.nodes[g.grew[0].node].name, "app", "the folder that holds the growth, not its parent");
        assert!(g.grew[0].why.iter().any(|w| w.contains(".log")), "{:?}", g.grew[0].why);
        assert!(g.per_day.unwrap() > 0.0 && g.full_in_days.is_some());
        assert!(g.text(&cur).contains("What grew most"));
        // as a finding: information, from the history, with the why
        assert!(findings(&g, &cur, SystemTime::now()).is_empty(), "300 MB is under the 1 GB floor");
        let f = findings_over(&g, &cur, SystemTime::now(), 256 << 20);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].group(), crate::finding::Group::Know);
        assert!(f[0].title.starts_with("app grew by 300.0 MB in 8 days"), "{}", f[0].title);
        assert!(f[0].evidence[0].contains("then") && f[0].evidence.iter().any(|e| e.contains(".log")), "{:?}", f[0].evidence);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
