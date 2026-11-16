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
//! Paths take `%VAR%` environment variables, `*` and `?` inside one segment, and `**` for any
//! number of segments. Matching ignores case. `{path}` in a command becomes the matched path.

use std::path::PathBuf;

use sprawl_core::{Tree, MULTIPLE, ROOT};
use serde::Deserialize;

use crate::finding::{Finding, Remedy, Risk, Source};
use crate::segments;

pub const BUILTIN: &str = include_str!("../rules/builtin.toml");

#[derive(Clone, Debug, Deserialize)]
pub struct PackInfo {
    pub name: String,
    #[serde(default)]
    pub version: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Dir,
    File,
    Any,
}

#[derive(Clone, Debug, Deserialize)]
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
}

#[derive(Clone, Debug, Deserialize)]
pub struct RulePack {
    pub pack: PackInfo,
    #[serde(default, rename = "rule")]
    pub rules: Vec<Rule>,
    /// Only the pack compiled into the program. Local and proposed packs are never trusted:
    /// their commands and `allow_protected` are refused by the safety checks.
    #[serde(skip)]
    pub trusted: bool,
}

impl RulePack {
    pub fn parse(text: &str) -> Result<RulePack, String> {
        let p: RulePack = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut ids = std::collections::HashSet::new();
        for r in &p.rules {
            if !ids.insert(&r.id) {
                return Err(format!("rule id {} appears twice", r.id));
            }
            if r.paths.is_empty() {
                return Err(format!("rule {} has no paths", r.id));
            }
            if r.remedy == Remedy::Command && r.command.is_none() {
                return Err(format!("rule {} has remedy = \"command\" but no command", r.id));
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

    /// For `?:\**\<name>`: the name pattern (`*.vhdx`), checked against every name in a tree.
    pub(crate) fn any_depth_name(&self) -> Option<&str> {
        match self.segs.as_slice() {
            [d, s, n] if d == "?:" && s == "**" && !n.contains('\\') => Some(n.as_str()),
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

const DEFAULT_MIN: u64 = 50 * 1024 * 1024;

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
        for rule in &pack.rules {
            let min = rule.min_bytes.unwrap_or(DEFAULT_MIN);
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
///   made after the scan); a folder is emptied the same way first; a read-only file goes too.
///   A name ending in a dot or space is skipped: Windows would act on its namesake without one.
/// - `sprawl-clear path days`: what is inside `path`, leaving anything (a folder by its newest
///   file) changed within `days`; nothing at all if `path` itself is now a link.
/// `Remove-Item -Recurse` and .NET Framework's `Directory.Delete(path, true)` can't be trusted
/// with links in 5.1. Built-in rules repeat this word for word (tested).
pub const RM_FN: &str = "function sprawl-rm($i) { if (-not $i) { return }; if ($i.Name -match '[. ]$') { Write-Warning ('Skipped, odd name: ' + $i.FullName); return }; if ($i.Attributes -band 1024) { if ($i.PSIsContainer) { [IO.Directory]::Delete($i.FullName) } else { $i.Delete() } } elseif ($i.PSIsContainer) { Get-ChildItem -LiteralPath $i.FullName -Force | ForEach-Object { sprawl-rm $_ }; [IO.Directory]::Delete($i.FullName) } else { $i.IsReadOnly = $false; $i.Delete() } }; function sprawl-new($i, $cut) { $i.LastWriteTime -gt $cut -or ($i.PSIsContainer -and -not ($i.Attributes -band 1024) -and $null -ne (Get-ChildItem -LiteralPath $i.FullName -Recurse -Force -ErrorAction SilentlyContinue | Where-Object LastWriteTime -gt $cut | Select-Object -First 1)) }; function sprawl-clear($p, $days) { $d = Get-Item -LiteralPath $p -Force; if (-not $d -or ($d.Attributes -band 1024)) { Write-Warning ('Not a plain folder: ' + $p); return }; $cut = (Get-Date).AddDays(-$days); Get-ChildItem -LiteralPath $d.FullName -Force | Where-Object { -not (sprawl-new $_ $cut) } | ForEach-Object { sprawl-rm $_ } }";

fn finding(tree: &Tree, n: usize, rule: &Rule, pack: &RulePack) -> Finding {
    let path: PathBuf = tree.path_of(n);
    let quoted = ps_quote(&path.display().to_string());
    let command = match (&rule.command, rule.remedy) {
        (Some(c), _) => Some(c.replace("{path}", &quoted)),
        (None, Remedy::Delete) => Some(if tree.nodes[n].is_dir {
            format!("{RM_FN}; sprawl-clear {quoted} 0")
        } else {
            format!("{RM_FN}; sprawl-rm (Get-Item -LiteralPath {quoted} -Force)")
        }),
        _ => None,
    };
    let node = &tree.nodes[n];
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
    let on_path = rule.command.as_deref().map_or(command.is_some(), |c| c.contains("{path}"));
    Finding {
        path,
        owner: rule.owner.clone(),
        bytes: node.size,
        risk: rule.risk,
        remedy: rule.remedy,
        command,
        title: rule.title.clone(),
        evidence,
        confidence: 0.95,
        source: Source::Rule(rule.id.clone()),
        allow_protected: pack.trusted && rule.allow_protected,
        // the program's own delete command, or the built-in pack's
        trusted: pack.trusted || rule.command.is_none(),
        builtin: pack.trusted,
        on_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
