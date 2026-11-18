//! A4: alternatives for a space goal. This builds plans, never performs them. Cleanup and
//! compression use existing checked findings; game moves are guided through Steam itself.

use std::path::PathBuf;
use crate::facts::{AppKind, Facts};
use crate::finding::{Finding, Risk};

#[derive(Clone, Debug)]
pub struct Game {
    pub name: String,
    pub path: PathBuf,
    /// Space on the source disk; may differ from logical size when compressed/sparse.
    pub bytes: u64,
    pub destination_need: u64,
}

#[derive(Clone, Debug)]
pub struct GameMove {
    pub game: Game,
    pub destination: char,
    pub free_before: u64,
    pub free_after: u64,
}

#[derive(Clone, Debug)]
pub struct OptionPlan {
    pub kind: Kind,
    pub title: &'static str,
    pub cost: &'static str,
    pub findings: Vec<usize>,
    pub moves: Vec<GameMove>,
    pub bytes: u64,
    pub estimated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Green, Cleanup, Compression, Moves }

/// Drive-letter aliases must not count as another volume, or reserve its free space twice.
pub fn destinations(source: char, drives: &[(char, u64, u64)]) -> Vec<(char, u64, u64)> {
    destinations_with(source, drives, |c| crate::hidden::volume_guid(std::path::Path::new(&format!("{c}:\\"))))
}

fn destinations_with(source: char, drives: &[(char, u64, u64)], mut identify: impl FnMut(char) -> Option<String>) -> Vec<(char, u64, u64)> {
    let Some(source_id) = identify(source.to_ascii_uppercase()) else { return Vec::new() };
    let mut seen = std::collections::HashSet::from([source_id.to_ascii_lowercase()]);
    drives.iter().copied().filter(|d| identify(d.0.to_ascii_uppercase()).is_some_and(|id| seen.insert(id.to_ascii_lowercase()))).collect()
}

pub fn games(tree: &sprawl_core::Tree, facts: &Facts) -> Vec<Game> {
    if tree.method == sprawl_core::ScanMethod::Csv { return Vec::new(); }
    let mut out = Vec::new();
    for app in facts.apps.iter().filter(|a| a.kind == AppKind::Steam) {
        let Some(path) = app.locations.first() else { continue };
        let Some(parent) = path.parent().filter(|p| p.file_name().is_some_and(|s| s.eq_ignore_ascii_case("common"))) else { continue };
        if !parent.parent().and_then(|p| p.file_name()).is_some_and(|s| s.eq_ignore_ascii_case("steamapps")) { continue; }
        let Some(n) = crate::find_path(tree, path) else { continue };
        let node = &tree.nodes[n];
        if !node.is_dir || node.is_reparse_point() || node.attributes & crate::dupes::NOT_LOCAL != 0 || node.size == 0 { continue; }
        if out.iter().any(|g: &Game| related(&g.path, path)) { continue; }
        out.push(Game { name: app.name.clone(), path: path.clone(), bytes: node.allocated.min(node.size), destination_need: node.size });
    }
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out
}

fn related(a: &std::path::Path, b: &std::path::Path) -> bool {
    crate::finding::within(a, b) || crate::finding::within(b, a)
}

fn drive(p: &std::path::Path) -> Option<char> {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    let b = s.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\').then(|| (b[0] as char).to_ascii_uppercase())
}

/// Ordered by cost: green cleanup, cleanup with rebuild/download cost, compression, moves.
/// Alternatives are separate plans, not totals to add together. `volume` is the goal's drive.
pub fn options(findings: &[Finding], counted: &[bool], games: &[Game], drives: &[(char, u64, u64)], volume: Option<char>, need: u64) -> Vec<OptionPlan> {
    if need == 0 { return Vec::new(); }
    let allowed: Vec<bool> = findings.iter().enumerate().map(|(i, f)| counted.get(i).copied().unwrap_or(true)
        && volume.is_none_or(|v| drive(&f.path) == Some(v.to_ascii_uppercase()))).collect();
    let green: Vec<bool> = allowed.iter().zip(findings).map(|(&ok, f)| ok && f.risk == Risk::Green).collect();
    let safe = crate::plan::plan(findings, &green, need);
    let all = crate::plan::plan(findings, &allowed, need);
    let cleanup = |kind, title, cost, picked: Vec<usize>, bytes| OptionPlan {
        kind, title, cost, estimated: picked.iter().any(|&i| findings[i].estimate()),
        findings: picked, moves: Vec::new(), bytes,
    };
    let mut out = vec![cleanup(Kind::Green, "Caches and safe cleanup", "Regenerable files; caches can refill and the first start may be slower.", safe.picked, safe.bytes)];
    if all.picked != out[0].findings {
        out.push(cleanup(Kind::Cleanup, "Include rebuilds and downloads", "Adds amber items: rebuilding or downloading them again costs time. Review each item.", all.picked.clone(), all.bytes));
    }
    let mut compressed = cleanup(Kind::Compression, "Cleanup plus compression", "Nothing in compressed folders is deleted. Compression takes CPU/time; reads may be slower. Savings are estimates.", all.picked.clone(), all.bytes);
    let mut candidates: Vec<usize> = (0..findings.len()).filter(|&i| allowed[i] && findings[i].builtin && findings[i].trusted
        && matches!(findings[i].source, crate::Source::Rule(_)) && !findings[i].heuristic() && !findings[i].privacy() && !findings[i].other_account()
        && !findings[i].in_active_project() && findings[i].risk <= Risk::Amber
        && crate::remedies::compresses(&findings[i]) && findings[i].gain.is_some_and(|b| b > 0)).collect();
    candidates.sort_by_key(|&i| std::cmp::Reverse(findings[i].expected_gain()));
    for i in candidates {
        if compressed.bytes >= need { break; }
        if compressed.findings.iter().any(|&k| related(&findings[k].path, &findings[i].path)) { continue; }
        compressed.bytes = compressed.bytes.saturating_add(findings[i].expected_gain());
        compressed.findings.push(i);
        compressed.estimated = true;
    }
    if compressed.findings.len() > all.picked.len() { out.push(compressed); }
    let mut moved = cleanup(Kind::Moves, "Cleanup plus Steam game moves", "Moves take time and need free space on another drive. Games may be unavailable during the move; use Steam's own storage manager.", all.picked, all.bytes);
    if let Some(volume) = volume.map(|c| c.to_ascii_uppercase()) {
        let mut room: std::collections::BTreeMap<char, u64> = drives.iter().filter(|d| d.0.to_ascii_uppercase() != volume)
            .map(|d| (d.0.to_ascii_uppercase(), d.2.min(d.1))).collect();
        let mut candidates: Vec<&Game> = games.iter().filter(|g| drive(&g.path) == Some(volume) && g.bytes > 0).collect();
        candidates.sort_by_key(|g| std::cmp::Reverse(g.bytes));
        for game in candidates {
            if moved.bytes >= need { break; }
            if moved.findings.iter().any(|&i| related(&findings[i].path, &game.path))
                || moved.moves.iter().any(|m| related(&m.game.path, &game.path)) { continue; }
            let Some(reserve) = game.destination_need.checked_add(1 << 30) else { continue };
            let Some((&destination, &free)) = room.iter().filter(|(_, r)| **r >= reserve).max_by_key(|(_, r)| **r) else { continue };
            let after = free - game.destination_need;
            room.insert(destination, after);
            moved.bytes = moved.bytes.saturating_add(game.bytes);
            moved.moves.push(GameMove { game: game.clone(), destination, free_before: free, free_after: after });
        }
    }
    if !moved.moves.is_empty() { moved.estimated = true; out.push(moved); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::{Action, Remedy, Source};
    fn f(path: &str, risk: Risk, bytes: u64) -> Finding { Finding {
        path: path.into(), owner: None, bytes, risk, remedy: Remedy::Delete, command: Some("fixture".into()),
        title: "fixture".into(), evidence: vec![], confidence: 0.8, source: Source::Rule("test".into()),
        allow_protected: false, trusted: true, builtin: true, on_path: true, action: None, admin: false,
        carved: vec![], gain: None, forced_from: None,
    } }
    const GB: u64 = 1 << 30;
    #[test]
    fn alternatives_respect_volume_cost_overlap_and_destination_capacity() {
        let mut compress = f(r"C:\Programs\Large", Risk::Amber, 50 * GB);
        compress.remedy = Remedy::Command;
        compress.action = Some(Action::Tool { command: format!("{}xpress8k", crate::remedies::COMPRESS) });
        compress.gain = Some(5 * GB);
        let fs = vec![f(r"C:\Cache", Risk::Green, 2 * GB), f(r"C:\Build", Risk::Amber, 3 * GB),
            f(r"D:\Cache", Risk::Green, 90 * GB), f(r"C:\Own", Risk::Orange, 100 * GB), compress];
        let game = |name: &str| Game { name: name.into(), path: format!(r"C:\Steam\steamapps\common\{name}").into(), bytes: 7 * GB, destination_need: 10 * GB };
        let gs = vec![game("A"), game("B"), game("A")];
        let opts = options(&fs, &[true; 5], &gs, &[('C', 200 * GB, 0), ('D', 100 * GB, 20 * GB)], Some('C'), 25 * GB);
        assert_eq!(opts[0].bytes, 2 * GB);
        assert_eq!(opts[1].bytes, 5 * GB);
        assert_eq!(opts[2].bytes, 10 * GB);
        assert!(opts.iter().all(|p| !p.findings.contains(&2) && !p.findings.contains(&3)));
        let moved = opts.last().unwrap();
        assert_eq!(moved.moves.len(), 1, "second move cannot consume the reserved 1 GB of headroom");
        assert_eq!(moved.bytes, 12 * GB, "source allocation, not logical size, is freed");
        assert_eq!(moved.moves[0].free_after, 10 * GB);
        assert!(options(&fs, &[true; 5], &gs, &[], Some('C'), 0).is_empty());
        assert!(options(&fs, &[true; 5], &gs, &[('D', 100 * GB, 50 * GB)], None, 100 * GB).iter().all(|p| p.moves.is_empty()));
        // Cleaning a game's child and moving the same game cannot be added together.
        let fs = vec![f(r"C:\Steam\steamapps\common\A\cache", Risk::Green, GB)];
        let opts = options(&fs, &[true], &[game("A")], &[('D', 100 * GB, 50 * GB)], Some('C'), 25 * GB);
        assert!(opts.iter().all(|p| p.moves.is_empty()));
        let drives = [('C', 100, 50), ('D', 100, 50), ('E', 100, 50), ('F', 100, 50), ('G', 100, 50)];
        let distinct = destinations_with('C', &drives, |c| match c { 'C' | 'D' => Some("source".into()), 'E' | 'F' => Some("other".into()), _ => None });
        assert_eq!(distinct, vec![('E', 100, 50)], "one capacity per distinct known destination volume");
    }
}
