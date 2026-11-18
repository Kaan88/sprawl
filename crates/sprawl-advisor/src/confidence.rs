//! How sure Sprawl is of a finding, from the signals it already gathered: what found it (a rule
//! naming the exact place, a tool's marker files, a guess by shape), whether its application is
//! known, and how long nothing in it changed. Used to order findings and to decide what may be
//! green: a green finding Sprawl isn't sure of is amber. Never raises a risk's grade.

use std::time::{Duration, SystemTime};

use sprawl_core::Tree;

use crate::facts::Facts;
use crate::finding::{Finding, Risk, Source};

/// Below this a green finding is shown amber.
pub const GREEN_FLOOR: f32 = 0.6;

/// What found it, and how sure that alone makes Sprawl.
fn base(f: &Finding) -> (f32, &'static str) {
    let Source::Rule(id) = &f.source else {
        // a model's suggestion: what it said, at most middling
        return (f.confidence.min(0.6), "suggested by the AI");
    };
    if !f.builtin {
        return (0.5, "a rule you added names this place");
    }
    match id.as_str() {
        crate::cleaners::SHAPE => (0.45, "found by its shape (a cache folder of an app Sprawl doesn't know)"),
        crate::leftovers::ID => (0.4, "no installed application by its name"),
        crate::chromium::ID => (0.75, "Chromium's own layout"),
        "app.unused" => (f.confidence, "not started for a long time"),
        _ if id.starts_with("project.") => (0.7, "the marker files of the tool that makes it"),
        _ if id.starts_with("remedy.") || id.starts_with("anomaly.") || id.starts_with("growth.") || id.starts_with("stale.") => (f.confidence, "information"),
        _ => (0.75, "a rule names this exact place"),
    }
}

/// The newest change of anything in `n`.
fn newest(tree: &Tree, n: usize) -> Option<SystemTime> {
    let mut best = None;
    let mut stack = vec![n];
    while let Some(k) = stack.pop() {
        let node = &tree.nodes[k];
        if !node.is_dir {
            best = best.max(node.modified);
        } else if k == n || !node.is_reparse_point() {
            stack.extend_from_slice(&node.children);
        }
    }
    best
}

/// How sure Sprawl is of `f` (0 to 1), and the signals that say so, in words.
pub fn assess(f: &Finding, tree: &Tree, facts: &Facts, now: SystemTime) -> (f32, Vec<String>) {
    let (mut c, why) = base(f);
    let mut signals = vec![why.to_string()];
    if let Some(app) = facts.owner_of(&f.path).map(|a| a.name.clone()).or_else(|| f.owner.clone()) {
        c += 0.1;
        signals.push(format!("its application is known ({app})"));
    }
    if let Some(t) = crate::find_path(tree, &f.path).and_then(|n| newest(tree, n)) {
        let age = now.duration_since(t).unwrap_or(Duration::ZERO).as_secs() / 86_400;
        if age >= 30 {
            c += 0.1;
            signals.push(format!("nothing in it changed for {age} days"));
        } else if age < 1 {
            c -= 0.1;
            signals.push("something in it changed today: it may be in use".into());
        }
    }
    (c.clamp(0.05, 0.98), signals)
}

/// "high", "fair" or "low".
pub fn words(c: f32) -> &'static str {
    if c >= 0.8 {
        "high"
    } else if c >= GREEN_FLOOR {
        "fair"
    } else {
        "low"
    }
}

/// Set each finding's confidence from its signals, say why in its evidence, and show a green
/// one Sprawl isn't sure of as amber.
pub fn apply(found: &mut [Finding], tree: &Tree, facts: &Facts, now: SystemTime) {
    for f in found {
        let (c, signals) = assess(f, tree, facts, now);
        f.confidence = c;
        f.evidence.push(format!("How sure Sprawl is: {} ({})", words(c), signals.join("; ")));
        if f.risk == Risk::Green && c < GREEN_FLOOR {
            f.risk = Risk::Amber;
            f.evidence.push("Shown as worth a check rather than safe: Sprawl isn't sure enough of it".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::Remedy;

    fn finding(path: &std::path::Path, id: &str, risk: Risk) -> Finding {
        Finding {
            path: path.to_path_buf(),
            owner: None,
            bytes: 1,
            risk,
            remedy: Remedy::Delete,
            command: None,
            title: "t".into(),
            evidence: vec![],
            confidence: 0.9,
            source: Source::Rule(id.into()),
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
    }

    #[test]
    fn confidence_comes_from_the_signals() {
        let base = std::env::temp_dir().join(format!("sprawl-conf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let old = base.join("old");
        let fresh = base.join("fresh");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&fresh).unwrap();
        let f = std::fs::File::create(old.join("a.bin")).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(90 * 86_400)).unwrap();
        std::fs::write(fresh.join("b.bin"), b"x").unwrap();
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let facts = Facts::from_apps(Vec::new());
        let now = SystemTime::now();
        // a rule naming the place, its app known, nothing changed for months: high
        let mut rule = finding(&old, "dev.cache", Risk::Green);
        rule.owner = Some("Tool".into());
        let (c, why) = assess(&rule, &tree, &facts, now);
        assert!(c >= 0.8, "{c} {why:?}");
        assert!(why.iter().any(|w| w.contains("90 days")) && why.iter().any(|w| w.contains("Tool")));
        // a guess by shape with something changed today: low
        let shape = finding(&fresh, crate::cleaners::SHAPE, Risk::Amber);
        let (c, why) = assess(&shape, &tree, &facts, now);
        assert!(c < GREEN_FLOOR, "{c} {why:?}");
        assert!(why.iter().any(|w| w.contains("may be in use")));
        // a green finding Sprawl isn't sure of is amber; never the other way
        let mut v = vec![finding(&fresh, "local.thing", Risk::Green), finding(&old, "dev.cache", Risk::Green), shape];
        v[0].builtin = false;
        apply(&mut v, &tree, &facts, now);
        assert_eq!(v[0].risk, Risk::Amber);
        assert!(v[0].evidence.last().unwrap().contains("isn't sure enough"));
        assert_eq!(v[1].risk, Risk::Green);
        assert_eq!(v[2].risk, Risk::Amber);
        assert!(v[1].evidence[0].starts_with("How sure Sprawl is: high"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
