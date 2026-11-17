//! Cleaners: a BleachBit-like list of applications, each with parts that can be cleaned.
//!
//! A cleaner lives in a rule pack beside the rules. Its parts are rules with a few more fields,
//! so there is one matching engine and one clean-up path: a part's matches are findings, ticked,
//! reviewed and removed like any other. A cleaner can also group rules of its pack by id.
//!
//! ```toml
//! [[cleaner]]
//! id = "app.discord"
//! name = "Discord"
//! group = "Chat"                      # Browsers | Chat | Games | Media | Developer tools | Windows | Other
//! present = ['%APPDATA%\discord']     # listed when one of these is in the scan (or on disk)
//! running = ["Discord.exe"]           # process names that mean the app is open
//! rules = ["some.rule"]               # rules of this pack shown as parts
//!
//!   [[cleaner.part]]
//!   id = "cache"
//!   title = "Cache"
//!   class = "space"                   # space | privacy (privacy: Sprawl's own cleaners only)
//!   risk = "green"
//!   paths = ['%APPDATA%\discord\Cache', '%APPDATA%\discord\Code Cache']
//!   action = "clear"                  # clear (default for a folder) | remove | match
//!   keep_days = 0                     # for clear: leave what changed in the last N days
//!   closed = "warn"                   # must | warn | no: what an open app means for this part
//!   cost = "Images and pages load again the first time."
//!   why = "Copies of things Discord downloaded. It fetches them again."
//! ```

use serde::{Deserialize, Serialize};

use crate::finding::{Remedy, Risk};
use crate::rules::{Act, Kind, Rule, RulePack};

/// Space cleaning removes copies; privacy cleaning removes the only record of something.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    #[default]
    Space,
    Privacy,
}

/// What it means for a part when its application is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Closed {
    /// It can't be cleaned while the app is open.
    Must,
    /// Files the app holds are skipped; the review says so.
    #[default]
    Warn,
    /// The app being open doesn't matter.
    No,
}

pub const GROUPS: &[&str] = &["Browsers", "Chat", "Games", "Media", "Developer tools", "Windows", "Other"];

/// The smallest part a cleaner shows (the Findings list keeps its own, larger thresholds).
pub const PART_MIN: u64 = 1 << 20;

fn other() -> String {
    "Other".into()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cleaner {
    pub id: String,
    pub name: String,
    #[serde(default = "other")]
    pub group: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub present: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub running: Vec<String>,
    /// Rules of the same pack, shown as parts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,
    #[serde(default, rename = "part", skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "is_space")]
    pub class: Class,
    pub risk: Risk,
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "is_dir_kind")]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Act>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_days: Option<u32>,
    #[serde(default)]
    pub closed: Closed,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cost: String,
    pub why: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub admin: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_bytes: Option<u64>,
}

fn is_space(c: &Class) -> bool {
    *c == Class::Space
}

fn is_dir_kind(k: &Kind) -> bool {
    *k == Kind::Dir
}

/// The rule id of a part.
pub fn part_rule_id(cleaner: &str, part: &str) -> String {
    format!("{cleaner}/{part}")
}

fn plain_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 60 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

impl Part {
    /// The rule that finds this part.
    pub fn rule(&self, cleaner: &Cleaner) -> Rule {
        // named after its application, so the review and the log say whose cache it is
        let title = format!("{}: {}", cleaner.name, self.title);
        let mut r = Rule::new(&part_rule_id(&cleaner.id, &self.id), &title, self.paths.clone(), self.risk, Remedy::Delete, &self.why);
        r.owner = Some(cleaner.name.clone());
        r.kind = self.kind;
        // keep_days says clear
        r.action = self.action.or(self.keep_days.map(|_| Act::Clear));
        r.pattern = self.pattern.clone();
        r.keep_days = self.keep_days;
        r.admin = self.admin;
        r.min_bytes = Some(self.min_bytes.unwrap_or(PART_MIN));
        r.cost = (!self.cost.is_empty()).then(|| self.cost.clone());
        r
    }
}

/// Check a pack's cleaners and turn their parts into rules (`RulePack::parse` calls this).
pub(crate) fn expand(pack: &RulePack) -> Result<Vec<Rule>, String> {
    let mut out: Vec<Rule> = Vec::new();
    let mut ids = std::collections::HashSet::new();
    let rule_ids: std::collections::HashSet<&str> = pack.rules.iter().map(|r| r.id.as_str()).collect();
    let mut grouped = std::collections::HashSet::new();
    for c in &pack.cleaners {
        if !plain_id(&c.id) || !ids.insert(c.id.to_lowercase()) {
            return Err(format!("cleaner id \"{}\" is empty, not plain, or used twice", c.id));
        }
        if c.name.trim().is_empty() || c.name.len() > 60 {
            return Err(format!("cleaner {} needs a name (60 characters at most)", c.id));
        }
        if !GROUPS.contains(&c.group.as_str()) {
            return Err(format!("cleaner {}: group must be one of {}", c.id, GROUPS.join(", ")));
        }
        if c.parts.is_empty() && c.rules.is_empty() {
            return Err(format!("cleaner {} has no parts", c.id));
        }
        if c.parts.len() + c.rules.len() > 40 || c.present.len() > 8 || c.running.len() > 8 {
            return Err(format!("cleaner {}: at most 40 parts, 8 present paths and 8 process names", c.id));
        }
        // a process name, not a path or a pattern
        if c.running.iter().any(|p| p.is_empty() || p.len() > 64 || p.contains(['\\', '/', '*', '?', ':']) || !p.to_lowercase().ends_with(".exe")) {
            return Err(format!("cleaner {}: running names a program file, like Discord.exe", c.id));
        }
        for r in &c.rules {
            if !rule_ids.contains(r.as_str()) {
                return Err(format!("cleaner {} groups rule {r}, which isn't in this pack", c.id));
            }
            if !grouped.insert(r.clone()) {
                return Err(format!("rule {r} is in two cleaners"));
            }
        }
        let mut parts = std::collections::HashSet::new();
        for p in &c.parts {
            if !plain_id(&p.id) || p.id.contains('.') || !parts.insert(p.id.to_lowercase()) {
                return Err(format!("cleaner {}: part id \"{}\" is empty, not plain, or used twice", c.id, p.id));
            }
            if p.title.trim().is_empty() || p.why.trim().is_empty() {
                return Err(format!("cleaner {}: part {} needs a title and a why", c.id, p.id));
            }
            if p.risk == Risk::Red {
                return Err(format!("cleaner {}: part {} is red; a part is something that can go", c.id, p.id));
            }
            if p.class == Class::Privacy && (p.risk < Risk::Amber || p.closed != Closed::Must) {
                return Err(format!("cleaner {}: privacy part {} must be at least amber and closed = \"must\"", c.id, p.id));
            }
            if p.action == Some(Act::Remove) && p.keep_days.is_some() {
                return Err(format!("cleaner {}: part {}: keep_days is only for clear", c.id, p.id));
            }
            let r = p.rule(c);
            if rule_ids.contains(r.id.as_str()) {
                return Err(format!("part {} has the id of a rule", r.id));
            }
            r.check()?;
            out.push(r);
        }
    }
    Ok(out)
}

/// One cleaner as the Cleaners section shows it.
#[derive(Clone, Debug)]
pub struct CleanerDef {
    pub id: String,
    pub name: String,
    pub group: String,
    pub present: Vec<String>,
    pub running: Vec<String>,
    /// From the build (not a file on disk).
    pub builtin: bool,
    pub pack: String,
    pub parts: Vec<PartDef>,
    /// The definition as written (TOML), for "Show the definition".
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct PartDef {
    /// The rule whose findings are this part's.
    pub rule: String,
    pub title: String,
    pub class: Class,
    pub risk: Risk,
    pub closed: Closed,
    pub cost: String,
    pub why: String,
    /// The definition's "keep the last N days" (a clear only).
    pub keep_days: Option<u32>,
    pub clears: bool,
    /// A tip only: nothing to tick.
    pub info: bool,
    /// The paths it looks at.
    pub paths: Vec<String>,
}

/// The cleaner and part a rule's findings belong to.
pub fn part_of<'a>(catalogue: &'a [CleanerDef], rule: &str) -> Option<(&'a CleanerDef, &'a PartDef)> {
    catalogue.iter().find_map(|d| d.parts.iter().find(|p| p.rule == rule).map(|p| (d, p)))
}

/// The cleaners as text, for the read-only MCP tool: each application found, its parts and
/// sizes. `parts`: (cleaner name, part title, risk, bytes, paths).
pub fn as_text(rows: &[(String, Vec<(String, Risk, u64, Vec<String>)>)]) -> String {
    let mut s = String::new();
    for (name, parts) in rows {
        let total: u64 = parts.iter().map(|p| p.2).sum();
        s += &format!("## {name} ({})\n", sprawl_core::format_size(total));
        for (title, risk, bytes, paths) in parts {
            s += &format!("- {title}: {} [{risk:?}]", sprawl_core::format_size(*bytes));
            if !paths.is_empty() {
                s += &format!(" at {}", paths.join(", "));
            }
            s += "\n";
        }
    }
    if s.is_empty() {
        s = "No cleaner has anything in this scan.".into();
    }
    s + "\nRead-only: the user ticks parts in Sprawl's Advisor (Cleaners) and presses Clean up.\n"
}

/// The id of the "found by shape" cleaner, and of its findings' rule.
pub const SHAPE: &str = "shape.cache";

/// Every cleaner of `packs`, the built-in pack first. Local cleaners whose id is taken are left
/// out (a local pack can't stand in for a built-in cleaner).
pub fn catalogue(packs: &[RulePack]) -> Vec<CleanerDef> {
    let mut out: Vec<CleanerDef> = Vec::new();
    let ordered = packs.iter().filter(|p| p.trusted).chain(packs.iter().filter(|p| !p.trusted));
    for pack in ordered {
        for c in &pack.cleaners {
            if out.iter().any(|d| d.id.eq_ignore_ascii_case(&c.id)) {
                continue;
            }
            let mut parts: Vec<PartDef> = Vec::new();
            for id in &c.rules {
                let Some(r) = pack.rules.iter().find(|r| &r.id == id) else { continue };
                parts.push(PartDef {
                    rule: r.id.clone(),
                    title: r.title.clone(),
                    class: Class::Space,
                    risk: r.risk,
                    closed: Closed::Warn,
                    cost: r.cost.clone().unwrap_or_default(),
                    why: r.why.clone(),
                    keep_days: r.keep_days,
                    clears: r.action == Some(Act::Clear) || (r.action.is_none() && r.remedy == Remedy::Delete && r.kind != Kind::File),
                    info: !matches!(r.remedy, Remedy::Delete | Remedy::Command),
                    paths: r.paths.clone(),
                });
            }
            for p in &c.parts {
                let r = p.rule(c);
                parts.push(PartDef {
                    rule: r.id.clone(),
                    title: p.title.clone(),
                    class: p.class,
                    risk: p.risk,
                    closed: if p.class == Class::Privacy { Closed::Must } else { p.closed },
                    cost: p.cost.clone(),
                    why: p.why.clone(),
                    keep_days: p.keep_days,
                    clears: r.action.unwrap_or(if p.kind == Kind::File { Act::Remove } else { Act::Clear }) == Act::Clear,
                    info: false,
                    paths: p.paths.clone(),
                });
            }
            let mut text = toml::to_string(&Wrap { cleaner: vec![c.clone()] }).unwrap_or_default();
            // the rules it groups, written out too
            for id in &c.rules {
                if let Some(r) = pack.rules.iter().find(|r| &r.id == id) {
                    text += &toml::to_string(&RuleWrap { rule: vec![r.clone()] }).unwrap_or_default();
                }
            }
            out.push(CleanerDef {
                id: c.id.clone(),
                name: c.name.clone(),
                group: c.group.clone(),
                present: c.present.clone(),
                running: c.running.clone(),
                builtin: pack.trusted,
                pack: if pack.trusted { "Sprawl's built-in rules".into() } else { pack.pack.name.clone() },
                parts,
                text,
            });
        }
    }
    out
}

#[derive(Serialize)]
struct Wrap {
    cleaner: Vec<Cleaner>,
}

#[derive(Serialize)]
struct RuleWrap {
    rule: Vec<Rule>,
}

/// Where a cleaner's application is, for this scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presence {
    /// In the scan (one of its `present` paths, or something to clean).
    InScan,
    /// On this PC, outside the scan: the drive to scan.
    Elsewhere(std::path::PathBuf),
    Absent,
}

/// Whether `def`'s application is in the scan, elsewhere on this PC, or not here. `found`: the
/// cleaner has findings in this scan.
pub fn presence(tree: &sprawl_core::Tree, def: &CleanerDef, found: bool) -> Presence {
    if found {
        return Presence::InScan;
    }
    let pats: Vec<crate::rules::Pattern> = def.present.iter().map(|p| crate::rules::Pattern::new(p)).collect();
    if !pats.is_empty() && !crate::rules::matching_nodes(tree, &pats).is_empty() {
        return Presence::InScan;
    }
    for p in &def.present {
        let e = crate::rules::expand(p);
        if e.contains(['*', '?', '%']) {
            continue;
        }
        let path = std::path::Path::new(&e);
        if path.is_dir() {
            if let Some(std::path::Component::Prefix(pre)) = path.components().next() {
                return Presence::Elsewhere(std::path::PathBuf::from(format!("{}\\", pre.as_os_str().to_string_lossy())));
            }
        }
    }
    Presence::Absent
}

/// The smallest finding of each grouped rule that the Findings list shows (its own threshold);
/// smaller ones show in Cleaners only.
pub fn list_min(packs: &[RulePack], rule: &str) -> Option<u64> {
    if rule == SHAPE {
        return Some(crate::rules::DEFAULT_MIN);
    }
    if rule.contains('/') {
        // a part: the Findings list shows it from the usual size
        return packs.iter().flat_map(|p| p.part_rules.iter()).find(|r| r.id == rule).map(|_| crate::rules::DEFAULT_MIN);
    }
    let grouped = packs.iter().any(|p| p.cleaners.iter().any(|c| c.rules.iter().any(|r| r == rule)));
    if !grouped {
        return None;
    }
    packs.iter().flat_map(|p| p.rules.iter()).find(|r| r.id == rule).map(|r| r.min_bytes.unwrap_or(crate::rules::DEFAULT_MIN))
}

/// Whether a rule is grouped into a cleaner (its findings are kept from [`PART_MIN`]).
pub(crate) fn grouped(pack: &RulePack, rule: &str) -> bool {
    pack.cleaners.iter().any(|c| c.rules.iter().any(|r| r == rule))
}

/// Folders called `Cache`, `Code Cache` or `GPUCache` directly inside an application's folder in
/// AppData that no rule or cleaner knows: "Other apps (found by shape)". Amber, never ticked by
/// Sprawl, named after the application's folder. `seen`: lower-case paths already found.
pub fn by_shape(tree: &sprawl_core::Tree, seen: &std::collections::HashSet<String>) -> Vec<crate::Finding> {
    use crate::finding::{Action, Finding, Source};
    let names = ["Cache", "Code Cache", "GPUCache"];
    // a folder here that isn't an application's
    let skip = ["temp", "microsoft", "packages", "crashdumps", "programs", "google", "mozilla", "bravesoftware", "nvidia", "amd", "d3dscache", "npm-cache", "pip", "yarn", "pnpm"];
    let mut pats = Vec::new();
    for base in ["%LOCALAPPDATA%", "%APPDATA%"] {
        if crate::rules::expand(base).contains('%') {
            continue;
        }
        for n in names {
            pats.push(crate::rules::Pattern::new(&format!(r"{base}\*\{n}")));
        }
    }
    let mut out = Vec::new();
    for n in crate::rules::matching_nodes(tree, &pats) {
        let node = &tree.nodes[n];
        if !node.is_dir || node.size < PART_MIN || node.is_reparse_point() {
            continue;
        }
        let path = tree.path_of(n);
        let key = path.to_string_lossy().to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        let Some(app) = node.parent.map(|p| tree.nodes[p].name.clone()) else { continue };
        if skip.contains(&app.to_lowercase().as_str()) || app.starts_with('.') {
            continue;
        }
        let mut f = Finding {
            path,
            owner: Some(app.clone()),
            bytes: node.size,
            risk: Risk::Amber,
            remedy: Remedy::Delete,
            command: None,
            title: format!("{app}: {}", node.name),
            evidence: vec![
                format!("A folder named {} directly inside {app}'s folder in AppData. Programs keep copies of downloaded things there and fetch them again.", node.name),
                "Found by its shape: Sprawl has no cleaner for this program, so check it yourself.".into(),
                format!("{} files in {} folders", node.files, node.folders),
            ],
            confidence: 0.7,
            source: Source::Rule(SHAPE.into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: true,
            action: None,
            admin: false,
        };
        f.set_action(Action::Clear { days: 0 }, false);
        out.push(f);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(cleaner: &str) -> Result<RulePack, String> {
        RulePack::parse(&format!("[pack]\nname = 'x'\n[[rule]]\nid = 'r.one'\ntitle = 'One'\nwhy = 'w'\nrisk = 'green'\nremedy = 'delete'\npaths = ['C:\\a\\b']\n{cleaner}\n"))
    }

    const GOOD: &str = "[[cleaner]]\nid = 'app.x'\nname = 'X'\ngroup = 'Chat'\nrunning = ['X.exe']\nrules = ['r.one']\n[[cleaner.part]]\nid = 'cache'\ntitle = 'Cache'\nrisk = 'green'\npaths = ['C:\\x\\Cache']\nwhy = 'copies'\ncost = 'Slower once.'\n[[cleaner.part]]\nid = 'logs'\ntitle = 'Logs'\nrisk = 'green'\npaths = ['C:\\x\\logs']\naction = 'match'\npattern = '*.log'\nwhy = 'logs'\n";

    #[test]
    fn cleaners_parse_into_rules() {
        let p = pack(GOOD).unwrap();
        assert_eq!(p.part_rules.len(), 2);
        assert_eq!(p.part_rules[0].id, "app.x/cache");
        assert_eq!(p.part_rules[0].min_bytes, Some(PART_MIN));
        assert_eq!(p.part_rules[1].action, Some(Act::Match));
        let c = catalogue(&[p.clone()]);
        assert_eq!(c[0].parts.len(), 3, "the grouped rule and two parts");
        assert_eq!(c[0].parts[0].rule, "r.one");
        assert!(c[0].text.contains("[[cleaner]]") && c[0].text.contains("[[rule]]"));
        // written back, it reads the same
        let again = RulePack::parse(&toml::to_string(&p).unwrap()).unwrap();
        assert_eq!(again.part_rules.len(), 2);
        assert_eq!(again.rules.len(), 1, "parts aren't written out as rules");
    }

    /// Every built-in part that deletes names a place the never-list allows: a part on it would
    /// only ever be held back. (Browser caches inside protected profiles wait for the carve-outs.)
    #[test]
    fn builtin_parts_are_outside_the_never_list() {
        std::env::set_var("SystemDrive", "C:");
        std::env::set_var("SystemRoot", r"C:\Windows");
        std::env::set_var("LOCALAPPDATA", r"C:\Users\sam\AppData\Local");
        std::env::set_var("APPDATA", r"C:\Users\sam\AppData\Roaming");
        std::env::set_var("USERPROFILE", r"C:\Users\sam");
        std::env::set_var("ProgramData", r"C:\ProgramData");
        let never = crate::safety::Never::default_list();
        let pack = RulePack::builtin();
        assert!(pack.cleaners.len() >= 12);
        for r in &pack.part_rules {
            for p in &r.paths {
                let sample = crate::memory::sample_path(p);
                assert!(never.hit(&sample).is_none(), "{} {p} -> {sample}", r.id);
                assert!(!crate::rules::expand(p).contains('%'), "{p}");
            }
            assert!(r.command.is_none() && !r.allow_protected, "{}", r.id);
        }
        // every rule is in at most one cleaner, and the cleaners' text loads again
        for d in catalogue(&[pack.clone()]) {
            let text = format!("[pack]\nname = 'x'\n{}", d.text);
            assert!(RulePack::parse(&text).is_ok(), "{}: {text}", d.id);
        }
    }

    /// A cleaner on a real folder: its parts become findings with the right action and title,
    /// and the catalogue finds them by rule id.
    #[test]
    fn a_cleaner_finds_its_parts() {
        let dir = std::env::temp_dir().join(format!("sprawl-cl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("App").join("Cache")).unwrap();
        std::fs::create_dir_all(dir.join("App").join("logs")).unwrap();
        std::fs::write(dir.join("App").join("Cache").join("a.bin"), vec![1u8; 2 << 20]).unwrap();
        std::fs::write(dir.join("App").join("logs").join("old.log"), vec![1u8; 3 << 20]).unwrap();
        std::fs::write(dir.join("App").join("settings.json"), b"{}").unwrap();
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let base = dir.display().to_string().replace('\'', "''");
        let text = format!(
            "[pack]\nname = 'x'\n[[cleaner]]\nid = 'app.t'\nname = 'Test app'\npresent = ['{base}\\App']\n[[cleaner.part]]\nid = 'cache'\ntitle = 'Cache'\nrisk = 'green'\npaths = ['{base}\\App\\Cache']\nwhy = 'w'\n[[cleaner.part]]\nid = 'logs'\ntitle = 'Logs'\nrisk = 'green'\npaths = ['{base}\\App\\logs']\naction = 'match'\npattern = '*.log'\nwhy = 'w'\n"
        );
        let mut pack = RulePack::parse(&text).unwrap();
        pack.trusted = true;
        let found = crate::rules::run(&tree, &[pack.clone()]);
        assert_eq!(found.len(), 2, "{found:?}");
        let cache = found.iter().find(|f| f.source == crate::Source::Rule("app.t/cache".into())).unwrap();
        assert_eq!(cache.title, "Test app: Cache");
        assert_eq!(cache.action, Some(crate::finding::Action::Clear { days: 0 }));
        let logs = found.iter().find(|f| f.source == crate::Source::Rule("app.t/logs".into())).unwrap();
        assert_eq!(logs.action, Some(crate::finding::Action::Match { pattern: "*.log".into() }));
        let cat = catalogue(&[pack]);
        assert_eq!(cat[0].parts.iter().map(|p| p.rule.as_str()).collect::<Vec<_>>(), ["app.t/cache", "app.t/logs"]);
        assert_eq!(presence(&tree, &cat[0], false), Presence::InScan);
        // found by shape: a Cache folder of an unknown program
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_cleaners_are_refused() {
        for bad in [
            GOOD.replace("group = 'Chat'", "group = 'Stuff'"),
            GOOD.replace("rules = ['r.one']", "rules = ['r.two']"),
            GOOD.replace("running = ['X.exe']", "running = ['C:\\\\x\\\\X.exe']"),
            GOOD.replace("id = 'logs'", "id = 'cache'"),
            GOOD.replace("risk = 'green'\npaths = ['C:\\x\\logs']", "risk = 'red'\npaths = ['C:\\x\\logs']"),
            GOOD.replace("pattern = '*.log'", "pattern = 'a\\\\*.log'"),
            GOOD.replace("why = 'logs'", "why = 'logs'\nclass = 'privacy'"),
            GOOD.replace("why = 'logs'", "why = 'logs'\ncommand = 'x'"),
            format!("{GOOD}[[cleaner]]\nid = 'app.x'\nname = 'Y'\nrules = []\n"),
        ] {
            assert!(pack(&bad).is_err(), "{bad}");
        }
    }
}
