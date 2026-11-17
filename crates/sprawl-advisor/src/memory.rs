//! What the advisor remembers between runs: the user's preferences, what they decided about
//! findings, labels for folders (so nobody has to work out twice what a folder is), and rules
//! the model proposed, waiting for the user's approval.
//!
//! Kept in `%APPDATA%\Sprawl\advisor\memory.toml`. Approved rules go to
//! `%APPDATA%\Sprawl\advisor\rules\local.toml`, a local pack: never trusted, so it can't carry
//! command text, can't touch protected folders, and its deletes are at least Amber.
//!
//! Memory only ever hides findings or adds context for the model. It can't loosen a safety
//! check.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::facts::clean_text;
use crate::finding::{Finding, Remedy, Risk};
use crate::rules::{Kind, PackInfo, Pattern, Rule, RulePack};
use crate::segments;

/// How long "Not now" hides a finding.
pub const LATER_DAYS: u64 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// Never suggest this (or anything inside it) again.
    Keep,
    /// Hide it for [`LATER_DAYS`].
    Later,
    /// The user cleaned it up (measured after running the script).
    Done,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub path: PathBuf,
    pub title: String,
    pub verdict: Verdict,
    /// Unix seconds.
    pub when: u64,
    /// A Keep of this one suggestion only: what is inside may still come up (information
    /// findings about a whole folder: moving it, compressing it). A separate flag, so an older
    /// Sprawl reads it as a full Keep, which hides more.
    #[serde(default)]
    pub exact: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum From {
    You,
    Model,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pref {
    pub text: String,
    pub from: From,
}

/// What a folder (by pattern) is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub pattern: String,
    pub label: String,
    pub from: From,
}

/// A rule the model proposed. No command: only Sprawl's built-in rules carry command text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub title: String,
    pub paths: Vec<String>,
    pub risk: Risk,
    pub remedy: Remedy,
    pub why: String,
    #[serde(default)]
    pub min_bytes: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    #[serde(default)]
    pub prefs: Vec<Pref>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub proposals: Vec<Proposal>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn dir() -> Option<PathBuf> {
    crate::llm::advisor_dir()
}

fn file() -> Option<PathBuf> {
    dir().map(|d| d.join("memory.toml"))
}

pub fn rules_dir() -> Option<PathBuf> {
    dir().map(|d| d.join("rules"))
}

/// Written next to the target and renamed over it, so a crash never leaves half a file.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// A path or pattern the model may name in a rule or a label: absolute (a drive or a
/// `%VARIABLE%`), no `..`, nothing that isn't a plain path, and narrow: at least two segments
/// after the root that aren't wildcards (`?:\\**\\*.jpg` or `C:\\*` would reach everything).
fn plain_pattern(p: &str) -> bool {
    let b = p.as_bytes();
    let rooted = (b.len() >= 3 && (b[0].is_ascii_alphabetic() || b[0] == b'?') && b[1] == b':' && b[2] == b'\\') || p.starts_with('%');
    if !rooted || p.len() >= 300 || p.contains("..") || p.contains(['"', '\'', '`', '|', '<', '>', ';', '\n', '\r', '{', '}', '$']) {
        return false;
    }
    let literal = p.split('\\').skip(1).filter(|s| !s.is_empty() && !s.contains(['*', '?']) && !s.starts_with('%')).count();
    literal >= 2
}

/// What a proposed rule must be, checked when proposed and again when approved (the file may
/// have been changed in between).
fn check_proposal(p: &Proposal) -> Result<(), String> {
    if p.title.is_empty() || p.why.is_empty() {
        return Err("give a title and a reason".into());
    }
    if p.paths.is_empty() || p.paths.len() > 8 || !p.paths.iter().all(|x| plain_pattern(x)) {
        return Err("give 1 to 8 absolute, narrow path patterns (C:\\... or %VARIABLE%\\..., at least two named folders)".into());
    }
    // a delete names what it deletes: the last part is a name, not a wildcard
    if p.remedy == Remedy::Delete && p.paths.iter().any(|x| x.rsplit('\\').next().is_none_or(|last| last.contains(['*', '?']) || last.is_empty())) {
        return Err("a delete rule must end in a folder or file name, not a wildcard".into());
    }
    if matches!(p.remedy, Remedy::Command | Remedy::Move) {
        return Err("a proposed rule can delete, suggest uninstalling, or explain (info): commands come only from Sprawl itself".into());
    }
    if p.risk == Risk::Red && p.remedy != Remedy::Info {
        return Err("red is information only".into());
    }
    Ok(())
}

impl Memory {
    /// Load without touching the file, even if it doesn't parse.
    pub fn load_readonly() -> Memory {
        file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| toml::from_str(t.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
    }

    pub fn load() -> Memory {
        let Some(f) = file() else { return Memory::default() };
        let Ok(t) = std::fs::read_to_string(&f) else { return Memory::default() };
        match toml::from_str(t.trim_start_matches('\u{feff}')) {
            Ok(m) => m,
            Err(_) => {
                // never overwrite a file that didn't load: keep it beside, start afresh
                let _ = std::fs::rename(&f, f.with_extension("toml.unreadable"));
                Memory::default()
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let f = file().ok_or_else(|| std::io::Error::other("APPDATA is not set"))?;
        write_atomic(&f, &toml::to_string(self).map_err(std::io::Error::other)?)
    }

    /// Record what the user decided about `f` (replacing an earlier decision on the same path).
    pub fn decide(&mut self, path: &Path, title: &str, verdict: Verdict) {
        let key = path.to_string_lossy().to_lowercase();
        self.decisions.retain(|d| d.path.to_string_lossy().to_lowercase() != key);
        self.decisions.push(Decision { path: path.to_path_buf(), title: clean_text(title), verdict, when: now(), exact: false });
        // the oldest go first past 2000
        if self.decisions.len() > 2000 {
            let extra = self.decisions.len() - 2000;
            self.decisions.drain(..extra);
        }
    }

    /// Keep this one suggestion only (see [`Decision::exact`]).
    pub fn keep_exact(&mut self, path: &Path, title: &str) {
        self.decide(path, title, Verdict::Keep);
        if let Some(d) = self.decisions.last_mut() {
            d.exact = true;
        }
    }

    /// Remove whatever decision hides `path` (its own, or a Keep on a folder above it).
    pub fn undo_hiding(&mut self, path: &Path) {
        if let Some(d) = self.hides(path).map(|d| d.path.clone()) {
            self.undo(&d);
        }
    }

    pub fn undo(&mut self, path: &Path) {
        let key = path.to_string_lossy().to_lowercase();
        self.decisions.retain(|d| d.path.to_string_lossy().to_lowercase() != key);
    }

    /// The decision that hides `path`: kept (it or a folder above it), or "not now" lately.
    pub fn hides(&self, path: &Path) -> Option<&Decision> {
        let p = segments(&path.to_string_lossy());
        let t = now();
        self.decisions.iter().find(|d| {
            let k = segments(&d.path.to_string_lossy());
            match d.verdict {
                Verdict::Keep if d.exact => p == k,
                Verdict::Keep => p.len() >= k.len() && p[..k.len()] == k[..],
                Verdict::Later => p == k && t.saturating_sub(d.when) < LATER_DAYS * 86_400,
                Verdict::Done => false,
            }
        })
    }

    /// Split findings into those to show and those the user's decisions hide.
    pub fn filter(&self, findings: Vec<Finding>) -> (Vec<Finding>, Vec<(Finding, Verdict)>) {
        let mut shown = Vec::new();
        let mut hidden = Vec::new();
        for f in findings {
            match self.hides(&f.path) {
                Some(d) => hidden.push((f, d.verdict)),
                None => shown.push(f),
            }
        }
        (shown, hidden)
    }

    pub fn label_for(&self, path: &Path) -> Option<&Label> {
        let p = path.to_string_lossy();
        self.labels.iter().rev().find(|l| Pattern::new(&l.pattern).matches(&p))
    }

    pub fn add_pref(&mut self, text: &str, from: From) -> Result<(), String> {
        let text = clean_text(text);
        if text.len() < 3 {
            return Err("too short".into());
        }
        if self.prefs.iter().any(|p| p.text.eq_ignore_ascii_case(&text)) {
            return Ok(());
        }
        if self.prefs.len() >= 100 {
            return Err("there are already 100 preferences; remove some first".into());
        }
        self.prefs.push(Pref { text, from });
        Ok(())
    }

    pub fn add_label(&mut self, pattern: &str, label: &str, from: From) -> Result<(), String> {
        let pattern = pattern.trim().replace('/', "\\");
        if !plain_pattern(&pattern) {
            return Err("the pattern must be an absolute path (C:\\... or %VARIABLE%\\...), optionally with * and **".into());
        }
        let label = clean_text(label);
        if label.is_empty() {
            return Err("give a label".into());
        }
        self.labels.retain(|l| !l.pattern.eq_ignore_ascii_case(&pattern));
        self.labels.push(Label { pattern, label, from });
        if self.labels.len() > 500 {
            self.labels.remove(0);
        }
        Ok(())
    }

    /// Check a rule the model proposes and keep it for the user to approve.
    pub fn propose(&mut self, mut p: Proposal) -> Result<(), String> {
        p.id = format!("local.{}", p.id.trim_start_matches("local.").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(40).collect::<String>());
        if p.id == "local." {
            return Err("give the rule an id".into());
        }
        p.title = clean_text(&p.title);
        p.why = clean_text(&p.why);
        check_proposal(&p)?;
        if self.proposals.iter().any(|q| q.id == p.id) || local_ids().contains(&p.id) {
            return Err(format!("there is already a rule {}", p.id));
        }
        if self.proposals.len() >= 50 {
            return Err("50 proposals are already waiting".into());
        }
        self.proposals.push(p);
        Ok(())
    }

    /// Move proposal `i` into the local rule pack.
    pub fn approve(&mut self, i: usize) -> Result<(), String> {
        let p = self.proposals.get(i).cloned().ok_or("no such proposal")?;
        check_proposal(&p)?;
        let path = rules_dir().ok_or("APPDATA is not set")?.join("local.toml");
        let mut pack = match std::fs::read_to_string(&path) {
            // a file that doesn't load is the user's to fix, not to overwrite
            Ok(t) => RulePack::parse(&t).map_err(|e| format!("{} doesn't load ({e}); fix or remove it first", path.display()))?,
            Err(_) => RulePack { pack: PackInfo { name: "Your rules".into(), version: 1 }, rules: Vec::new(), trusted: false },
        };
        pack.rules.retain(|r| r.id != p.id);
        pack.rules.push(Rule {
            id: p.id,
            title: p.title,
            owner: None,
            paths: p.paths,
            kind: Kind::Any,
            risk: p.risk,
            remedy: p.remedy,
            command: None,
            why: p.why,
            min_bytes: p.min_bytes,
            min_age_days: None,
            allow_protected: false,
        });
        let text = toml::to_string(&pack).map_err(|e| e.to_string())?;
        // what is written must load again
        RulePack::parse(&text)?;
        write_atomic(&path, &text).map_err(|e| e.to_string())?;
        self.proposals.remove(i);
        Ok(())
    }

    /// What the model should know: preferences, what was kept or done, folder labels.
    pub fn context(&self) -> String {
        let mut s = String::new();
        if !self.prefs.is_empty() {
            s.push_str("\n## The user's preferences\n");
            for p in &self.prefs {
                s.push_str(&format!("- {}\n", p.text));
            }
        }
        let kept: Vec<&Decision> = self.decisions.iter().filter(|d| d.verdict == Verdict::Keep).collect();
        if !kept.is_empty() {
            s.push_str("\n## The user chose to keep (never suggest these or anything inside them)\n");
            for d in kept.iter().rev().take(60) {
                s.push_str(&format!("- {} ({})\n", d.path.display(), d.title));
            }
        }
        let done: Vec<&Decision> = self.decisions.iter().filter(|d| d.verdict == Verdict::Done).collect();
        if !done.is_empty() {
            s.push_str("\n## The user cleaned these up before (they are fine with this kind of suggestion)\n");
            for d in done.iter().rev().take(30) {
                s.push_str(&format!("- {} ({})\n", d.title, d.path.display()));
            }
        }
        if !self.labels.is_empty() {
            s.push_str("\n## Known folders\n");
            for l in self.labels.iter().rev().take(80) {
                s.push_str(&format!("- {}: {}\n", l.pattern, l.label));
            }
        }
        s
    }
}

fn local_ids() -> Vec<String> {
    local_packs().into_iter().flat_map(|p| p.rules.into_iter().map(|r| r.id)).collect()
}

/// Every `*.toml` in the local rules folder, untrusted. Broken files are skipped.
pub fn local_packs() -> Vec<RulePack> {
    let Some(d) = rules_dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(d) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")) && e.metadata().is_ok_and(|m| m.len() < 1 << 20) {
            if let Some(mut pack) = std::fs::read_to_string(&p).ok().and_then(|t| RulePack::parse(&t).ok()) {
                pack.trusted = false;
                out.push(pack);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_hide_findings() {
        let mut m = Memory::default();
        m.decide(Path::new(r"C:\Games\Old"), "Old game", Verdict::Keep);
        m.decide(Path::new(r"C:\Cache"), "Cache", Verdict::Later);
        assert!(m.hides(Path::new(r"c:\games\old\saves")).is_some(), "inside a kept folder");
        assert!(m.hides(Path::new(r"C:\Cache")).is_some());
        assert!(m.hides(Path::new(r"C:\Cache\sub")).is_none(), "not now is for that finding only");
        m.decisions[1].when -= (LATER_DAYS + 1) * 86_400;
        assert!(m.hides(Path::new(r"C:\Cache")).is_none(), "not now expires");
        m.undo(Path::new(r"c:\GAMES\old"));
        assert!(m.hides(Path::new(r"C:\Games\Old")).is_none());
        let t = toml::to_string(&m).unwrap();
        assert_eq!(toml::from_str::<Memory>(&t).unwrap(), m);
    }

    #[test]
    fn model_input_is_checked() {
        let mut m = Memory::default();
        assert!(m.add_label(r"C:\Tools\*\cache", "Tool caches", From::Model).is_ok());
        // too broad to be a label or a rule
        for broad in [r"?:\", r"?:\**\*.jpg", r"C:\*\**", r"%USERPROFILE%\*"] {
            assert!(m.add_label(broad, "x", From::Model).is_err(), "{broad}");
        }
        assert_eq!(m.label_for(Path::new(r"C:\Tools\x\cache")).unwrap().label, "Tool caches");
        assert!(m.add_label(r"..\x", "no", From::Model).is_err());
        assert!(m.add_label(r"C:\x'; Remove-Item", "no", From::Model).is_err());
        let p = |paths: Vec<&str>, remedy: Remedy| Proposal {
            id: "My Cache!".into(),
            title: "Cache".into(),
            paths: paths.into_iter().map(String::from).collect(),
            risk: Risk::Green,
            remedy,
            why: "regenerates".into(),
            min_bytes: None,
        };
        assert!(m.propose(p(vec![r"%LOCALAPPDATA%\Tool\cache"], Remedy::Command)).is_err(), "no commands");
        assert!(m.propose(p(vec!["cache"], Remedy::Delete)).is_err(), "absolute only");
        assert!(m.propose(p(vec![r"%LOCALAPPDATA%\Tool\*"], Remedy::Delete)).is_err(), "a delete ends in a name");
        assert!(m.propose(p(vec![r"%LOCALAPPDATA%\Tool\cache"], Remedy::Delete)).is_ok());
        assert_eq!(m.proposals[0].id, "local.MyCache");
        assert!(m.propose(p(vec![r"%LOCALAPPDATA%\Tool\cache"], Remedy::Delete)).is_err(), "not twice");
        // an approved rule is a pack that loads, untrusted, with no command
        let pack = RulePack {
            pack: PackInfo { name: "Your rules".into(), version: 1 },
            rules: vec![Rule {
                id: "local.x".into(),
                title: "x".into(),
                owner: None,
                paths: vec![r"C:\x".into()],
                kind: Kind::Any,
                risk: Risk::Green,
                remedy: Remedy::Delete,
                command: None,
                why: "w".into(),
                min_bytes: None,
                min_age_days: None,
                allow_protected: false,
            }],
            trusted: false,
        };
        let back = RulePack::parse(&toml::to_string(&pack).unwrap()).unwrap();
        assert!(!back.trusted && back.rules[0].command.is_none());
        assert!(m.context().is_empty() || m.context().contains("Known folders"));
    }
}
