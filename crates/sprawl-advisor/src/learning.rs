//! A6: local preference evidence. This never changes risk, permissions or actions. Repeated
//! successful cleanups can modestly raise confidence; skips lower presentation priority.
//! Fast refill and explicitly managed game libraries leave automatic recommendations alone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use crate::finding::{Finding, Source};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Learning {
    #[serde(default)]
    pub rules: BTreeMap<String, Counts>,
    #[serde(default)]
    pub cleaned: Vec<Cleaned>,
    #[serde(default)]
    pub managed_libraries: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Counts { pub cleaned: u32, pub skipped: u32 }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cleaned {
    pub path: PathBuf, pub rule: String, pub when: u64,
    pub freed: u64, pub remaining: u64,
    #[serde(default)]
    pub refilled: bool,
}

fn id(f: &Finding) -> Option<&str> {
    if f.privacy() { return None; }
    match &f.source { Source::Rule(id) => Some(id), _ => None }
}

pub fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) }

fn cache(id: &str) -> bool {
    matches!(id, "dev.npm-cache" | "dev.pip-cache" | "dev.cargo-registry" | "dev.gradle-cache" | "dev.nuget-packages"
        | "dev.pnpm-store" | "dev.yarn-cache" | "dev.go-build-cache" | "games.steam-shadercache" | "web.firefox-cache"
        | "gfx.shader-caches") || id.starts_with("app.") && matches!(id.split('/').nth(1), Some("cache" | "code-cache" | "gpu-cache"))
}

pub fn steam_library(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|p| p.file_name().is_some_and(|s| s.eq_ignore_ascii_case("common"))
        && p.parent().and_then(|p| p.file_name()).is_some_and(|s| s.eq_ignore_ascii_case("steamapps"))).map(Path::to_path_buf)
}

impl Learning {
    pub fn skip(&mut self, f: &Finding) {
        let Some(id) = id(f) else { return };
        if self.rules.len() >= 500 && !self.rules.contains_key(id) { return; }
        let c = self.rules.entry(id.into()).or_default();
        c.skipped = c.skipped.saturating_add(1);
    }

    pub fn success(&mut self, path: &Path, rule: &str, freed: u64, remaining: Option<u64>, when: u64) {
        if freed == 0 { return; }
        if self.rules.len() < 500 || self.rules.contains_key(rule) {
            let c = self.rules.entry(rule.into()).or_default();
            c.cleaned = c.cleaned.saturating_add(1);
        }
        let Some(remaining) = remaining.filter(|_| cache(rule)) else { return };
        let refilled = self.cleaned.iter().any(|r| r.path.as_os_str().eq_ignore_ascii_case(path.as_os_str()) && r.refilled);
        self.cleaned.retain(|r| !r.path.as_os_str().eq_ignore_ascii_case(path.as_os_str()));
        self.cleaned.push(Cleaned { path: path.into(), rule: rule.into(), freed, remaining, when, refilled });
        if self.cleaned.len() > 1000 { self.cleaned.drain(..self.cleaned.len() - 1000); }
    }

    /// A finding is a measurement from a new scan. Require real new growth above the measured
    /// post-cleanup remainder, not just a large folder after a partial cleanup.
    pub fn refills(&self, findings: &[Finding], when: u64) -> Vec<Cleaned> {
        self.cleaned.iter().filter(|r| !r.refilled && when > r.when && when - r.when <= 3 * 86_400
            && r.freed >= 1 << 20 && findings.iter().any(|f| id(f) == Some(r.rule.as_str())
                && f.path.as_os_str().eq_ignore_ascii_case(r.path.as_os_str())
                && f.bytes.saturating_sub(r.remaining) >= r.freed / 2)).cloned().collect()
    }

    pub fn mark_refills(&mut self, observed: &[Cleaned]) -> bool {
        let mut changed = false;
        for old in observed {
            if let Some(r) = self.cleaned.iter_mut().find(|r| r.path == old.path && r.rule == old.rule && r.when == old.when) {
                changed |= !r.refilled;
                r.refilled = true;
            }
        }
        changed
    }

    pub fn quiet_reason(&self, f: &Finding) -> Option<&'static str> {
        if self.managed_libraries.iter().any(|p| crate::finding::within(&f.path, p)) {
            return Some("You manage games in this Steam library yourself");
        }
        let rule = id(f);
        self.cleaned.iter().any(|r| r.refilled && rule.is_none_or(|id| r.rule == id) && r.path.as_os_str().eq_ignore_ascii_case(f.path.as_os_str()))
            .then_some("This cache regained at least half the cleaned space within three days; repeated cleanup is unlikely to help for long")
    }

    pub fn priority(&self, f: &Finding) -> u8 {
        if self.quiet_reason(f).is_some() { return 2; }
        if id(f).and_then(|id| self.rules.get(id)).is_some_and(|s| s.skipped >= 2 && s.skipped >= s.cleaned) { 1 } else { 0 }
    }

    pub fn apply(&self, findings: &mut [Finding]) {
        for f in findings {
            if let Some(reason) = self.quiet_reason(f) { f.evidence.push(format!("Not recommended again: {reason}. You can still choose it manually; Forget learning restores recommendations.")); }
            if let Some(c) = id(f).and_then(|id| self.rules.get(id)) {
                if c.cleaned >= 2 && c.skipped == 0 {
                    f.confidence = f.confidence.max((f.confidence + 0.05).min(0.98));
                    f.evidence.push(format!("You successfully cleaned this rule {} times. That records your past choices, not a lower safety risk", c.cleaned));
                } else if c.skipped >= 2 {
                    f.evidence.push(format!("You kept or postponed this kind {} times; ranked later in Recommended order", c.skipped));
                }
            }
        }
    }

    pub fn manage_library(&mut self, path: &Path) -> bool {
        let Some(lib) = steam_library(path) else { return false };
        if self.managed_libraries.len() < 100 && !self.managed_libraries.contains(&lib) { self.managed_libraries.push(lib); }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::{Action, Remedy, Risk};
    fn f() -> Finding { Finding {
        path: r"C:\fixture\npm".into(), owner: None, bytes: 20 << 20, risk: Risk::Green,
        remedy: Remedy::Delete, command: None, title: "npm".into(), evidence: vec![], confidence: 0.8,
        source: Source::Rule("dev.npm-cache".into()), allow_protected: false, trusted: true,
        builtin: true, on_path: true, action: Some(Action::Remove), admin: false, carved: vec![], gain: None, forced_from: None,
    } }
    #[test]
    fn learning_changes_advice_not_actions_or_safety() {
        let mut learning = Learning::default();
        let original = f();
        learning.success(&original.path, "dev.npm-cache", 20 << 20, Some(8 << 20), 100);
        assert!(learning.refills(&[original.clone()], 100).is_empty(), "a scan at or before the cleanup cannot prove refill");
        assert!(learning.refills(&[original.clone()], 99).is_empty());
        assert!(learning.refills(&[original.clone()], 100 + 4 * 86_400).is_empty());
        let mut partial = original.clone(); partial.bytes = 9 << 20;
        assert!(learning.refills(&[partial], 200).is_empty(), "the old remainder is not growth");
        let marks = learning.refills(&[original.clone()], 200);
        assert_eq!(marks.len(), 1);
        assert!(learning.mark_refills(&marks));
        assert!(learning.quiet_reason(&original).is_some());
        learning.success(&original.path, "dev.npm-cache", 20 << 20, Some(0), 300);
        assert!(!learning.mark_refills(&marks), "old observations can't flag a newer cleanup");
        let mut fs = vec![original.clone()]; learning.apply(&mut fs);
        assert_eq!((fs[0].risk, fs[0].remedy, fs[0].action.clone()), (original.risk, original.remedy, original.action.clone()));
        learning.skip(&original); learning.skip(&original);
        assert!(learning.priority(&original) > 0);
        let mut local = original.clone(); local.source = Source::Rule("local.owned-rule".into()); local.builtin = false; local.trusted = false;
        learning.skip(&local); learning.skip(&local);
        assert_eq!(learning.priority(&local), 1, "explicit user-rule choices can be learned without trusting their actions");
        let mut model = local.clone(); model.source = Source::Model("model".into());
        let rules_before = learning.rules.len();
        learning.skip(&model);
        assert_eq!(learning.rules.len(), rules_before, "model claims do not create rule history");
        assert_eq!(learning.priority(&model), 2, "a model cannot bypass a quiet path");
        model.path = r"C:\fixture\unrelated".into();
        assert_eq!(learning.priority(&model), 0);
        assert!(!learning.manage_library(Path::new(r"C:\Users\Example")));
        assert!(learning.manage_library(Path::new(r"D:\SteamLibrary\steamapps\common\Game")));
        assert_eq!(learning.managed_libraries[0], PathBuf::from(r"D:\SteamLibrary\steamapps\common"));
        let encoded = toml::to_string(&learning).unwrap();
        assert_eq!(toml::from_str::<Learning>(&encoded).unwrap(), learning);
    }
}
