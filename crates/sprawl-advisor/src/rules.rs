//! Rule packs: TOML files that say where reclaimable space usually lives and what to do about
//! it, and the engine that matches them against a scanned tree.
//!
//! ```toml
//! [pack]
//! name = "builtin"
//! version = 1
//!
//! [[rule]]
//! id = "dev.npm-cache"
//! title = "npm cache"
//! owner = "npm"
//! paths = ['%LOCALAPPDATA%\npm-cache', '%SystemDrive%\Users\*\AppData\Local\npm-cache']
//! kind = "dir"            # dir (default) | file | any
//! risk = "green"          # green | amber | orange | red
//! remedy = "command"      # delete | command | uninstall | move | info
//! command = "npm cache clean --force"
//! why = "Packages npm downloaded before; it fetches them again when they are needed."
//! min_bytes = 52428800    # findings smaller than this are skipped (default 50 MB)
//! min_age_days = 90       # only things not modified for this long
//! ```
//!
//! A delete says what it does as data, never as command text:
//!
//! ```toml
//! remedy = "delete"
//! action = "match"        # remove | clear (default for a folder) | match
//! pattern = "*.dmp"       # for match: file names, anywhere below the folder
//! keep_days = 7           # for clear: leave what changed in the last 7 days
//! admin = true            # needs administrator rights
//! ```
//!
//! Paths take `%VAR%` environment variables, `*` and `?` inside one segment, and `**` for any
//! number of segments. Matching ignores case. `{path}` in a command becomes the matched path.

use std::path::PathBuf;

use sprawl_core::{Tree, MULTIPLE, ROOT};
use serde::Deserialize;

use crate::finding::{Action, Finding, Remedy, Risk, Source};
use crate::segments;

pub const BUILTIN: &str = include_str!("../rules/builtin.toml");

#[cfg(test)]
mod one_program_tests {
    /// `elevated_tool_ok` checks the first program a command starts. That holds only while every
    /// built-in command is one program: a `;`, pipe, `&` or `$(` would start more, unchecked.
    #[test]
    fn every_built_in_command_is_one_program() {
        let pack = super::RulePack::builtin();
        let commands: Vec<&str> = pack.rules.iter().filter_map(|r| r.command.as_deref()).collect();
        assert!(commands.len() >= 11, "the built-in commands were not found");
        for c in commands {
            let body = c.split('#').next().unwrap_or("");
            for bad in [";", "|", "&", "$(", "`", "\n"] {
                assert!(!body.contains(bad), "built-in command {c:?} has {bad:?}: it could start a second program the elevation check doesn't see");
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
pub struct PackInfo {
    pub name: String,
    #[serde(default)]
    pub version: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Dir,
    File,
    Any,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub owner: Option<String>,
    pub paths: Vec<String>,
    #[serde(default)]
    pub kind: Kind,
    pub risk: Risk,
    pub remedy: Remedy,
    #[serde(default)]
    pub command: Option<String>,
    pub why: String,
    #[serde(default)]
    pub min_bytes: Option<u64>,
    /// Only matches not modified for at least this many days (old installers, not today's).
    #[serde(default)]
    pub min_age_days: Option<u32>,
    /// Built-in pack only: the command may act on a never-list path (an official clean-up tool).
    #[serde(default)]
    pub allow_protected: bool,
    /// What a delete does to the matched path (in place of command text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Act>,
    /// For `action = "match"`: the file names that go.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// For `action = "clear"`: what changed in the last this many days stays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_days: Option<u32>,
    /// The action needs administrator rights.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub admin: bool,
    /// What cleaning it costs, in one plain sentence (shown in Cleaners).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<String>,
}

/// A rule's action, as written in a pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Act {
    Remove,
    Clear,
    Match,
}

impl Rule {
    /// A rule with the fields every rule needs; the rest are left out.
    pub fn new(id: &str, title: &str, paths: Vec<String>, risk: Risk, remedy: Remedy, why: &str) -> Rule {
        Rule {
            id: id.into(),
            title: title.into(),
            owner: None,
            paths,
            kind: Kind::Dir,
            risk,
            remedy,
            command: None,
            why: why.into(),
            min_bytes: None,
            min_age_days: None,
            allow_protected: false,
            action: None,
            pattern: None,
            keep_days: None,
            admin: false,
            cost: None,
        }
    }

    /// What this rule does to a matched path (a folder or not) when cleaned up, if anything.
    pub(crate) fn typed_action(&self, path: &str, is_dir: bool) -> Option<Action> {
        if let Some(c) = &self.command {
            return Some(Action::Tool { command: c.replace("{path}", &ps_quote(path)) });
        }
        if !matches!(self.remedy, Remedy::Delete | Remedy::Command) {
            return None;
        }
        Some(match self.action {
            Some(Act::Remove) => Action::Remove,
            Some(Act::Clear) => Action::Clear { days: self.keep_days.unwrap_or(0) },
            Some(Act::Match) => Action::Match { pattern: self.pattern.clone()? },
            None if self.remedy == Remedy::Delete => {
                if is_dir {
                    Action::Clear { days: 0 }
                } else {
                    Action::Remove
                }
            }
            None => return None,
        })
    }

    /// The checks every rule must pass, from any pack.
    pub(crate) fn check(&self) -> Result<(), String> {
        let id = &self.id;
        if self.paths.is_empty() {
            return Err(format!("rule {id} has no paths"));
        }
        if self.command.is_some() && self.action.is_some() {
            return Err(format!("rule {id} has both a command and an action"));
        }
        if self.remedy == Remedy::Command && self.command.is_none() && self.action.is_none() {
            return Err(format!("rule {id} has remedy = \"command\" but no command or action"));
        }
        if self.action.is_some() && !matches!(self.remedy, Remedy::Delete | Remedy::Command) {
            return Err(format!("rule {id}: an action needs remedy = \"delete\""));
        }
        match (self.action, &self.pattern) {
            (Some(Act::Match), Some(p)) if !p.is_empty() && !p.contains(['\\', '/']) && p.len() < 100 => {}
            (Some(Act::Match), _) => return Err(format!("rule {id}: action = \"match\" needs a pattern of file names (no folders)")),
            (_, Some(_)) => return Err(format!("rule {id}: a pattern is only for action = \"match\"")),
            _ => {}
        }
        if self.keep_days.is_some() && !matches!(self.action, Some(Act::Clear)) {
            return Err(format!("rule {id}: keep_days is only for action = \"clear\""));
        }
        if self.keep_days.is_some_and(|d| d > 3650) {
            return Err(format!("rule {id}: keep_days is at most 3650"));
        }
        if self.command.as_deref().is_some_and(|c| c.contains("sprawl-")) {
            return Err(format!("rule {id}: say what a delete does with action, not with Sprawl's own functions"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
pub struct RulePack {
    pub pack: PackInfo,
    #[serde(default, rename = "rule")]
    pub rules: Vec<Rule>,
    /// Applications and their parts (see [`crate::cleaners`]).
    #[serde(default, rename = "cleaner", skip_serializing_if = "Vec::is_empty")]
    pub cleaners: Vec<crate::cleaners::Cleaner>,
    /// The cleaners' parts as rules, made when the pack is parsed.
    #[serde(skip)]
    pub part_rules: Vec<Rule>,
    /// Only the pack compiled into the program. Local and proposed packs are never trusted:
    /// their commands and `allow_protected` are refused by the safety checks.
    #[serde(skip)]
    pub trusted: bool,
}

impl RulePack {
    /// A pack with nothing in it.
    pub fn empty(name: &str) -> RulePack {
        RulePack { pack: PackInfo { name: name.into(), version: 1 }, rules: Vec::new(), cleaners: Vec::new(), part_rules: Vec::new(), trusted: false }
    }

    /// The rules, then the cleaners' parts.
    pub fn all_rules(&self) -> impl Iterator<Item = &Rule> {
        self.rules.iter().chain(self.part_rules.iter())
    }

    pub fn parse(text: &str) -> Result<RulePack, String> {
        let mut p: RulePack = toml::from_str(text).map_err(|e| e.to_string())?;
        p.part_rules = crate::cleaners::expand(&p)?;
        let mut ids = std::collections::HashSet::new();
        for r in &p.rules {
            if !ids.insert(&r.id) {
                return Err(format!("rule id {} appears twice", r.id));
            }
            r.check()?;
        }
        for r in &p.part_rules {
            if !ids.insert(&r.id) {
                return Err(format!("rule id {} appears twice", r.id));
            }
        }
        Ok(p)
    }

    pub fn builtin() -> RulePack {
        let mut p = RulePack::parse(BUILTIN).expect("built-in rule pack");
        p.trusted = true;
        p
    }
}

/// `%NAME%` replaced by the environment variable (left as it is when unset).
pub fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('%') {
            Some(b) => {
                let name = &after[..b];
                match std::env::var(name) {
                    Ok(v) if !name.is_empty() => out.push_str(&v),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[b + 1..];
            }
            None => {
                out.push_str(&rest[a..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// `*` and `?` within one segment.
pub(crate) fn seg_match(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut i, mut j) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

/// A compiled path pattern: a small NFA over segments.
#[derive(Clone, Debug)]
pub(crate) struct Pattern {
    segs: Vec<String>,
}

impl Pattern {
    pub(crate) fn new(p: &str) -> Pattern {
        Pattern { segs: segments(&expand(p)) }
    }

    fn close(&self, mut s: Vec<usize>) -> Vec<usize> {
        let mut i = 0;
        while i < s.len() {
            let p = s[i];
            if p < self.segs.len() && self.segs[p] == "**" && !s.contains(&(p + 1)) {
                s.push(p + 1);
            }
            i += 1;
        }
        s.sort_unstable();
        s.dedup();
        s
    }

    pub(crate) fn start(&self) -> Vec<usize> {
        self.close(vec![0])
    }

    /// States after one more path segment (lower-case).
    pub(crate) fn step(&self, states: &[usize], name: &str) -> Vec<usize> {
        let mut out = Vec::new();
        for &p in states {
            if p >= self.segs.len() {
                continue;
            }
            if self.segs[p] == "**" {
                out.push(p);
            } else if seg_match(&self.segs[p], name) {
                out.push(p + 1);
            }
        }
        self.close(out)
    }

    pub(crate) fn accepts(&self, states: &[usize]) -> bool {
        states.contains(&self.segs.len())
    }

    /// Whether `path` is a folder above something the pattern names, so deleting it would take
    /// that with it: `C:\Users\sam\AppData\Local\Google` for `...\Google\Chrome\User Data\**`.
    /// Only the fixed part before the first `**` counts (`?:\**\*.vhdx` contains nothing fixed).
    pub(crate) fn is_above(&self, path: &str) -> bool {
        let fixed = self.segs.iter().position(|s| s == "**").unwrap_or(self.segs.len());
        let parts = segments(path);
        parts.len() < fixed && parts.iter().zip(&self.segs).all(|(p, s)| seg_match(s, p))
    }

    /// For `?:\**\<tail>`: the tail's segments (`*.vhdx`, or `thunderbird`, `profiles`), lower
    /// case, looked for at the end of any path.
    pub(crate) fn any_depth_tail(&self) -> Option<&[String]> {
        match self.segs.as_slice() {
            [d, s, rest @ ..] if d == "?:" && s == "**" && !rest.is_empty() && rest.iter().all(|r| r != "**") => Some(rest),
            _ => None,
        }
    }

    /// Whether a whole absolute path matches.
    pub(crate) fn matches(&self, path: &str) -> bool {
        let mut s = self.start();
        for part in segments(path) {
            s = self.step(&s, &part);
            if s.is_empty() {
                return false;
            }
        }
        self.accepts(&s)
    }
}

pub(crate) const DEFAULT_MIN: u64 = 50 * 1024 * 1024;

/// Not modified for `days` (no date known counts as not old enough).
fn old_enough(node: &sprawl_core::Node, days: Option<u32>) -> bool {
    let Some(days) = days else { return true };
    let Some(m) = node.modified else { return false };
    std::time::SystemTime::now().duration_since(m).is_ok_and(|age| age.as_secs() >= u64::from(days) * 86_400)
}

/// Every place in the tree a rule matches, as findings (not yet checked for safety). A path
/// matched by one rule isn't searched further by that rule, and a path is reported once (by the
/// first rule that matched it).
pub fn run(tree: &Tree, packs: &[RulePack]) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let mut taken = std::collections::HashSet::new();
    let starts: Vec<(usize, String)> = if tree.nodes[ROOT].name == MULTIPLE {
        tree.nodes[ROOT].children.iter().map(|&c| (c, tree.nodes[c].name.clone())).collect()
    } else {
        vec![(ROOT, tree.path_of(ROOT).to_string_lossy().into_owned())]
    };
    // The built-in pack first, so a local pack can't shadow its findings by matching first.
    let ordered: Vec<&RulePack> = packs.iter().filter(|p| p.trusted).chain(packs.iter().filter(|p| !p.trusted)).collect();
    for pack in ordered {
        for rule in pack.all_rules() {
            // a rule grouped into a cleaner shows there from a megabyte; the Findings list
            // keeps its own threshold
            let min = rule.min_bytes.unwrap_or(DEFAULT_MIN);
            let min = if crate::cleaners::grouped(pack, &rule.id) { min.min(crate::cleaners::PART_MIN) } else { min };
            for pat in rule.paths.iter().map(|p| Pattern::new(p)) {
                for (start, base) in &starts {
                    let mut s = pat.start();
                    for part in segments(base) {
                        s = pat.step(&s, &part);
                    }
                    if s.is_empty() {
                        continue;
                    }
                    let mut stack = vec![(*start, s)];
                    while let Some((n, s)) = stack.pop() {
                        let node = &tree.nodes[n];
                        let kind_ok = match rule.kind {
                            Kind::Dir => node.is_dir,
                            Kind::File => !node.is_dir,
                            Kind::Any => true,
                        };
                        if pat.accepts(&s) && kind_ok {
                            if node.size >= min && old_enough(node, rule.min_age_days) && taken.insert(n) {
                                out.push(finding(tree, n, rule, pack));
                            }
                            continue;
                        }
                        // junctions and links are never followed
                        if node.is_reparse_point() && n != *start {
                            continue;
                        }
                        for &c in &node.children {
                            let next = pat.step(&s, &tree.nodes[c].name.to_lowercase());
                            if !next.is_empty() {
                                stack.push((c, next));
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// Every node any of `patterns` matches (the first match on a path; links never followed).
pub(crate) fn matching_nodes(tree: &Tree, patterns: &[Pattern]) -> Vec<usize> {
    let starts: Vec<(usize, String)> = if tree.nodes[ROOT].name == MULTIPLE {
        tree.nodes[ROOT].children.iter().map(|&c| (c, tree.nodes[c].name.clone())).collect()
    } else {
        vec![(ROOT, tree.path_of(ROOT).to_string_lossy().into_owned())]
    };
    let mut out = Vec::new();
    for pat in patterns {
        for (start, base) in &starts {
            let mut s = pat.start();
            for part in segments(base) {
                s = pat.step(&s, &part);
            }
            if s.is_empty() {
                continue;
            }
            let mut stack = vec![(*start, s)];
            while let Some((n, s)) = stack.pop() {
                if pat.accepts(&s) {
                    if !out.contains(&n) {
                        out.push(n);
                    }
                    continue;
                }
                if tree.nodes[n].is_reparse_point() && n != *start {
                    continue;
                }
                for &c in &tree.nodes[n].children {
                    let next = pat.step(&s, &tree.nodes[c].name.to_lowercase());
                    if !next.is_empty() {
                        stack.push((c, next));
                    }
                }
            }
        }
    }
    out
}

/// A PowerShell single-quoted literal. Besides `'`, PowerShell ends a single-quoted string at
/// the typographic quotes U+2018..U+201B, so each of those is doubled too.
pub fn ps_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// PowerShell functions every delete command starts with, tested in Windows PowerShell 5.1:
/// - `sprawl-rm item`: a junction or symbolic link is removed itself, never followed (even one
///   made after the scan); a folder is emptied the same way first; read-only files and folders
///   go too. A folder's read-only bit is cleared only once it is empty, so one that stays (a
///   file in it was in use) keeps its attributes, OneDrive's pin included; the mask leaves out
///   bits .NET Framework has no names for, or the cast would fail.
///   A name ending in a dot or space is skipped: Windows would act on its namesake without one.
/// - `sprawl-clear path days`: what is inside `path`, leaving anything created or changed
///   within `days` (a folder by its newest item, found without following links; a folder that
///   can't be read counts as new); nothing at all if `path` is no longer a plain folder.
/// - `sprawl-match path pattern`: files matching `pattern` anywhere below `path`, without
///   following links.
/// `Remove-Item -Recurse` and .NET Framework's `Directory.Delete(path, true)` can't be trusted
/// with links in 5.1. Built-in rules repeat this word for word (tested).
pub const RM_FN: &str = "function sprawl-rm($i) { if (-not $i) { return }; if ($i.Name -match '[. ]$') { Write-Warning ('Skipped, odd name: ' + $i.FullName); return }; if ($i.Attributes -band 1024) { if ($i.PSIsContainer) { [IO.Directory]::Delete($i.FullName) } else { $i.Delete() } } elseif ($i.PSIsContainer) { Get-ChildItem -LiteralPath $i.FullName -Force | ForEach-Object { sprawl-rm $_ }; if (($i.Attributes -band 1) -and -not (Get-ChildItem -LiteralPath $i.FullName -Force)) { $i.Attributes = [IO.FileAttributes]($i.Attributes -band 0x2FFF6) }; [IO.Directory]::Delete($i.FullName) } else { $i.IsReadOnly = $false; $i.Delete() } }; function sprawl-new($i, $cut) { if ($i.LastWriteTime -gt $cut -or $i.CreationTime -gt $cut) { return $true }; if (-not $i.PSIsContainer -or ($i.Attributes -band 1024)) { return $false }; $e = $null; $kids = Get-ChildItem -LiteralPath $i.FullName -Force -ErrorAction SilentlyContinue -ErrorVariable e; if ($e) { return $true }; foreach ($k in $kids) { if (sprawl-new $k $cut) { return $true } }; return $false }; function sprawl-dir($p) { $d = Get-Item -LiteralPath $p -Force; if (-not $d -or -not $d.PSIsContainer -or ($d.Attributes -band 1024)) { Write-Warning ('Not a plain folder: ' + $p); return $null }; $d }; function sprawl-clear($p, $days) { $d = sprawl-dir $p; if (-not $d) { return }; $cut = (Get-Date).AddDays(-$days); Get-ChildItem -LiteralPath $d.FullName -Force | Where-Object { $days -le 0 -or -not (sprawl-new $_ $cut) } | ForEach-Object { sprawl-rm $_ } }; function sprawl-match($p, $like) { $d = sprawl-dir $p; if (-not $d) { return }; Get-ChildItem -LiteralPath $d.FullName -Force | ForEach-Object { if ($_.Attributes -band 1024) { } elseif ($_.PSIsContainer) { sprawl-match $_.FullName $like } elseif ($_.Name -like $like) { sprawl-rm $_ } } }";

fn finding(tree: &Tree, n: usize, rule: &Rule, pack: &RulePack) -> Finding {
    let path: PathBuf = tree.path_of(n);
    let node = &tree.nodes[n];
    let action = rule.typed_action(&path.display().to_string(), node.is_dir);
    let mut evidence = vec![rule.why.clone()];
    if node.is_dir {
        evidence.push(format!("{} files in {} folders", node.files, node.folders));
    }
    // provenance from where the pack was loaded, never from the name it gives itself
    if pack.trusted {
        evidence.push(format!("Matched built-in rule {}", rule.id));
    } else {
        evidence.push(format!("Matched rule {} from the local pack \"{}\"", rule.id, pack.pack.name));
    }
    let on_path = rule.command.as_deref().map_or(action.is_some(), |c| c.contains("{path}"));
    let mut f = Finding {
        path,
        owner: rule.owner.clone(),
        bytes: node.size,
        risk: rule.risk,
        remedy: rule.remedy,
        command: None,
        title: rule.title.clone(),
        evidence,
        confidence: 0.95,
        source: Source::Rule(rule.id.clone()),
        allow_protected: pack.trusted && rule.allow_protected,
        // the program's own delete command, or the built-in pack's
        trusted: pack.trusted || rule.command.is_none(),
        builtin: pack.trusted,
        on_path,
        action: None,
        admin: false,
        carved: Vec::new(),
        gain: None,
    };
    if let Some(a) = action {
        let admin = rule.admin || matches!(&a, Action::Tool { command } if crate::export::needs_admin(command));
        f.set_action(a, admin);
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every built-in rule: the typed action a finding gets and the script text written for it
    /// say the same thing, on a folder and on a file, with an awkward name.
    #[test]
    fn typed_actions_match_the_script_for_every_builtin_rule() {
        let pack = RulePack::builtin();
        let mut acting = 0;
        for rule in pack.all_rules() {
            for is_dir in [true, false] {
                let path = r"C:\Users\sam\it's ‘odd’";
                let Some(a) = rule.typed_action(path, is_dir) else {
                    assert!(matches!(rule.remedy, Remedy::Info | Remedy::Uninstall | Remedy::Move), "{} has no action", rule.id);
                    continue;
                };
                acting += 1;
                let mut f = Finding {
                    path: path.into(),
                    owner: None,
                    bytes: 0,
                    risk: rule.risk,
                    remedy: rule.remedy,
                    command: None,
                    title: rule.title.clone(),
                    evidence: Vec::new(),
                    confidence: 1.0,
                    source: Source::Rule(rule.id.clone()),
                    allow_protected: rule.allow_protected,
                    trusted: true,
                    builtin: true,
                    on_path: true,
                    action: None,
                    admin: false,
                    carved: Vec::new(),
                    gain: None,
                };
                let admin = rule.admin || matches!(&a, Action::Tool { command } if crate::export::needs_admin(command));
                f.set_action(a.clone(), admin);
                assert_eq!(crate::clean::action(&f), Some((a.clone(), admin)), "{}", rule.id);
                assert_eq!(crate::clean::parse_command(&f), Some((a, admin)), "{}: {:?}", rule.id, f.command);
            }
        }
        assert!(acting > 20);
    }

    #[test]
    fn rule_actions_are_checked() {
        let parse = |extra: &str| RulePack::parse(&format!("[pack]\nname = 'x'\n[[rule]]\nid = 'a'\ntitle = 'A'\nwhy = 'w'\nrisk = 'green'\npaths = ['C:\\x\\y']\n{extra}\n"));
        assert!(parse("remedy = 'delete'\naction = 'match'\npattern = '*.log'").is_ok());
        assert!(parse("remedy = 'delete'\naction = 'clear'\nkeep_days = 7").is_ok());
        assert!(parse("remedy = 'delete'\naction = 'match'").is_err(), "match needs a pattern");
        assert!(parse("remedy = 'delete'\naction = 'match'\npattern = '..\\*.log'").is_err(), "file names only");
        assert!(parse("remedy = 'delete'\naction = 'remove'\nkeep_days = 3").is_err());
        assert!(parse("remedy = 'delete'\npattern = '*.log'").is_err());
        assert!(parse("remedy = 'info'\naction = 'remove'").is_err());
        assert!(parse("remedy = 'command'\ncommand = 'x'\naction = 'remove'").is_err());
        assert!(parse("remedy = 'command'\ncommand = 'function sprawl-rm($i) {}; sprawl-rm x'").is_err());
    }

    #[test]
    fn powershell_quoting() {
        assert_eq!(ps_quote(r"C:\a"), r"'C:\a'");
        assert_eq!(ps_quote("it's"), "'it''s'");
        // typographic quotes also end a single-quoted string in PowerShell
        assert_eq!(ps_quote("a\u{2019}b"), "'a\u{2019}\u{2019}b'");
        assert_eq!(ps_quote("\u{2018}\u{201A}\u{201B}"), "'\u{2018}\u{2018}\u{201A}\u{201A}\u{201B}\u{201B}'");
        // inert inside single quotes: left alone
        assert_eq!(ps_quote(r"$x`; & a\"), r"'$x`; & a\'");
    }

    #[test]
    fn segment_globs() {
        assert!(seg_match("*.tmp", "a.tmp"));
        assert!(seg_match("np?-cache", "npm-cache"));
        assert!(!seg_match("*.tmp", "a.tmpx"));
        assert!(seg_match("*", ""));
    }

    #[test]
    fn path_patterns() {
        let p = Pattern::new(r"C:\Users\*\AppData\Local\npm-cache");
        assert!(p.matches(r"c:\users\sam\appdata\local\NPM-CACHE"));
        assert!(!p.matches(r"c:\users\sam\appdata\local\npm-cache\x"));
        let p = Pattern::new(r"C:\**\node_modules");
        assert!(p.matches(r"C:\code\app\node_modules"));
        assert!(p.matches(r"C:\node_modules"));
        assert!(!p.matches(r"C:\code\app"));
    }

    #[test]
    fn env_expansion() {
        std::env::set_var("DT_TEST_VAR", r"C:\x");
        assert_eq!(expand(r"%DT_TEST_VAR%\y"), r"C:\x\y");
        assert_eq!(expand(r"%DT_UNSET_VAR_1%\y"), r"%DT_UNSET_VAR_1%\y");
        assert_eq!(expand("100%"), "100%");
    }

    #[test]
    fn builtin_pack_parses() {
        let p = RulePack::builtin();
        assert!(p.rules.len() >= 30);
        for r in &p.rules {
            if let Some(c) = &r.command {
                // a command never hard-codes a path or an unexpanded variable: it names {path}
                assert!(!c.contains(":\\") && !c.contains('%'), "{} hard-codes a path: {c}", r.id);
                if c.contains("Remove-Item") || c.contains("Get-ChildItem") {
                    assert!(c.contains("{path}"), "{} deletes without naming {{path}}", r.id);
                }
                assert_eq!(r.remedy, Remedy::Command, "{} has a command but isn't a command remedy", r.id);
                // recursive deletes follow links planted later; sprawl-rm doesn't
                let recursive_remove = c.split(['|', ';']).any(|part| part.contains("Remove-Item") && part.contains("-Recurse"));
                assert!(!recursive_remove && !c.contains(", $true)"), "{} deletes recursively: {c}", r.id);
                if c.contains("sprawl-rm") {
                    assert!(c.starts_with(&format!("{RM_FN}; ")), "{}: sprawl-rm differs from RM_FN", r.id);
                }
            }
            if matches!(r.remedy, Remedy::Info) {
                assert!(r.command.is_none(), "{} is info only", r.id);
            }
            if r.allow_protected {
                assert_eq!(r.remedy, Remedy::Command, "{}: allow_protected only for official commands", r.id);
            }
            assert!(r.risk != Risk::Red, "{}: red rules are pointless", r.id);
        }
    }

    #[test]
    fn bad_packs_are_refused() {
        let dup = "[pack]\nname='x'\n[[rule]]\nid='a'\ntitle='t'\npaths=['C:\\\\a']\nrisk='green'\nremedy='delete'\nwhy='w'\n[[rule]]\nid='a'\ntitle='t'\npaths=['C:\\\\b']\nrisk='green'\nremedy='delete'\nwhy='w'\n";
        assert!(RulePack::parse(dup).unwrap_err().contains("twice"));
        let cmd = "[pack]\nname='x'\n[[rule]]\nid='a'\ntitle='t'\npaths=['C:\\\\a']\nrisk='green'\nremedy='command'\nwhy='w'\n";
        assert!(RulePack::parse(cmd).unwrap_err().contains("no command"));
    }
}
