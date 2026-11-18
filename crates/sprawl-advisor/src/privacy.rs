//! Privacy cleaners (phase 5 of `docs/plans/cleaners.md`): parts that remove the only record of
//! something (cookies, history) rather than a copy. They free almost no space.
//!
//! What sets them apart from every other part, and where each rule is kept:
//! - **Compiled in, never written in a pack.** The parts are the list below. No rule pack (the
//!   built-in TOML included), community import or model can define one: `cleaners::expand`
//!   refuses `class = "privacy"`, and only this file makes an [`Action::Set`].
//! - **Off unless the user turns them on.** Nothing here runs unless the caller passes the
//!   setting [`SETTING`] (`advise_report`, `cleaners::catalogue_with`). Off, nothing about them is
//!   read, computed or listed.
//! - **Whole files, as a set.** A part names databases; each goes with its `-journal`, `-wal` and
//!   `-shm` files, all of them or none (`clean::run` with [`Action::Set`]). No database is opened
//!   to remove some rows.
//! - **This account's profiles only**, each by name: Chromium's `Default` and `Profile N`
//!   (named as the browser names them in `Local State`), Firefox's profile folders (named as
//!   `profiles.ini` names them). Found under this user's `%LOCALAPPDATA%` and `%APPDATA%`.
//! - **The browser closed**, by its process names and by the Restart Manager on the files,
//!   checked when the review opens (the app) and again by the executor just before the set
//!   goes; an answer Windows can't give counts as open.
//! - **Never ticked for the user, never planned, never in a script, never in a space total.**
//!   [`Finding::privacy`] says which findings are these; `plan` and `export` leave them out; the
//!   report keeps them in a list of their own.
//! - **Plain words.** Each part says what the user loses, not how; none says or implies that the
//!   files are beyond recovery (a test holds every string to a list of words).
//!
//! Left out on purpose (see the report of 2026-11-18 and `docs/plans/cleaners.md`): Firefox
//! history (in `places.sqlite` with the bookmarks), saved passwords, form data and autofill
//! (`Login Data`, `Web Data`, `formhistory.sqlite`), bookmarks, preferences, extensions, local
//! storage and IndexedDB, session and tab restore, Windows' Recent items and jump lists (kept by
//! Explorer, which is always open), and anything in the registry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sprawl_core::Tree;

use crate::cleaners::{CleanerDef, Class, Closed, PartDef};
use crate::finding::{Action, Finding, Remedy, Risk, Source};

/// The settings key the app stores the switch under, and passes in as `privacy_cleaners`.
/// Off by default: with it off, nothing in this module runs.
pub const SETTING: &str = "privacyCleaners";

/// The files that belong to a database and go with it or not at all: SQLite's rollback journal,
/// write-ahead log and shared-memory index.
pub const SIDE_FILES: &[&str] = &["-journal", "-wal", "-shm"];

/// Words no privacy text may use: each says or implies that the files are beyond recovery, or
/// that removing them is without cost. For the app's own wording as well as this file's.
pub const PROMISE_WORDS: &[&str] = &["wipe", "secure", "unrecoverable", "irrecoverable", "recoverable", "safe", "shred", "permanent", "forever", "for good", "erase", "scrub", "no trace", "untraceable", "gone completely", "cannot be recovered", "can't be recovered", "can not be recovered", "overwritten so", "clean slate", "harmless", "nothing to lose"];

/// Said on every part: Sprawl deletes; it doesn't overwrite.
pub const DELETED_NOTE: &str = "Deleted, not overwritten: recovery tools may still find it.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `<User Data>\Default`, `<User Data>\Profile N`, each with a `Preferences` file.
    Chromium,
    /// `<Profiles>\<folder>`, each with a `prefs.js` file.
    Firefox,
}

#[derive(Debug)]
pub struct Browser {
    /// The cleaner's id (`privacy.chrome`); a part's rule id is `<cleaner>/<part>`.
    pub cleaner: &'static str,
    pub name: &'static str,
    pub short: &'static str,
    pub kind: Kind,
    /// Where its profiles are: below `%LOCALAPPDATA%` for Chromium, `%APPDATA%` for Firefox.
    pub profiles: &'static str,
    /// Its program files: any of them running means it is open.
    pub running: &'static [&'static str],
}

pub const BROWSERS: &[Browser] = &[
    Browser { cleaner: "privacy.chrome", name: "Google Chrome", short: "Chrome", kind: Kind::Chromium, profiles: r"Google\Chrome\User Data", running: &["chrome.exe"] },
    Browser { cleaner: "privacy.edge", name: "Microsoft Edge", short: "Edge", kind: Kind::Chromium, profiles: r"Microsoft\Edge\User Data", running: &["msedge.exe"] },
    Browser { cleaner: "privacy.brave", name: "Brave", short: "Brave", kind: Kind::Chromium, profiles: r"BraveSoftware\Brave-Browser\User Data", running: &["brave.exe"] },
    Browser { cleaner: "privacy.firefox", name: "Firefox", short: "Firefox", kind: Kind::Firefox, profiles: r"Mozilla\Firefox\Profiles", running: &["firefox.exe"] },
];

#[derive(Debug)]
pub struct PrivacyPart {
    /// The [`Browser::cleaner`] it belongs to.
    pub browser: &'static str,
    pub id: &'static str,
    pub title: &'static str,
    /// What the user loses, in plain words (shown on the part and in the review).
    pub cost: &'static str,
    /// What it is and which files go.
    pub why: &'static str,
    /// At least amber.
    pub risk: Risk,
    /// The files that go, relative to the profile folder, each with its [`SIDE_FILES`]. The first
    /// must be there for the part to be offered; the others go when they are there.
    pub files: &'static [&'static str],
    /// Never-list entries that name these files on purpose (found by name anywhere), passed only
    /// for this part's own files. Entries that cover a whole folder (`...\User Data\**`) are
    /// passed for them too; nothing else is.
    pub waives: &'static [&'static str],
}

const CHROMIUM_COOKIES_WHY: &str = "Cookies are what websites leave to recognise you. The browser keeps them in one database, Network\\Cookies, with its journal; Sprawl deletes that file whole, never part of it.";
const CHROMIUM_HISTORY_WHY: &str = "The browser keeps the pages you visited and the list of downloads in one database, History, so they go together. Visited Links, Top Sites and Shortcuts are made from it and go with it. If sync is on, history may come back from your other devices.";
const CHROMIUM_COOKIE_FILES: &[&str] = &[r"Network\Cookies"];
const CHROMIUM_HISTORY_FILES: &[&str] = &["History", "Visited Links", "Top Sites", "Shortcuts"];
const CHROMIUM_COOKIE_WAIVES: &[&str] = &[r"?:\**\Network\Cookies"];

pub const PARTS: &[PrivacyPart] = &[
    PrivacyPart { browser: "privacy.chrome", id: "cookies", title: "Cookies", cost: "Signs you out of most websites in Chrome (this profile), and sites forget choices they kept in cookies.", why: CHROMIUM_COOKIES_WHY, risk: Risk::Amber, files: CHROMIUM_COOKIE_FILES, waives: CHROMIUM_COOKIE_WAIVES },
    PrivacyPart { browser: "privacy.chrome", id: "history", title: "History and download list", cost: "Chrome forgets the pages you visited and its list of downloads (this profile), with the most-visited tiles and address-bar suggestions made from them. The downloaded files stay.", why: CHROMIUM_HISTORY_WHY, risk: Risk::Orange, files: CHROMIUM_HISTORY_FILES, waives: &[] },
    PrivacyPart { browser: "privacy.edge", id: "cookies", title: "Cookies", cost: "Signs you out of most websites in Edge (this profile), and sites forget choices they kept in cookies.", why: CHROMIUM_COOKIES_WHY, risk: Risk::Amber, files: CHROMIUM_COOKIE_FILES, waives: CHROMIUM_COOKIE_WAIVES },
    PrivacyPart { browser: "privacy.edge", id: "history", title: "History and download list", cost: "Edge forgets the pages you visited and its list of downloads (this profile), with the most-visited tiles and address-bar suggestions made from them. The downloaded files stay.", why: CHROMIUM_HISTORY_WHY, risk: Risk::Orange, files: CHROMIUM_HISTORY_FILES, waives: &[] },
    PrivacyPart { browser: "privacy.brave", id: "cookies", title: "Cookies", cost: "Signs you out of most websites in Brave (this profile), and sites forget choices they kept in cookies.", why: CHROMIUM_COOKIES_WHY, risk: Risk::Amber, files: CHROMIUM_COOKIE_FILES, waives: CHROMIUM_COOKIE_WAIVES },
    PrivacyPart { browser: "privacy.brave", id: "history", title: "History and download list", cost: "Brave forgets the pages you visited and its list of downloads (this profile), with the most-visited tiles and address-bar suggestions made from them. The downloaded files stay.", why: CHROMIUM_HISTORY_WHY, risk: Risk::Orange, files: CHROMIUM_HISTORY_FILES, waives: &[] },
    PrivacyPart {
        browser: "privacy.firefox",
        id: "cookies",
        title: "Cookies",
        cost: "Signs you out of most websites in Firefox (this profile), and sites forget choices they kept in cookies.",
        why: "Cookies are what websites leave to recognise you. Firefox keeps them in one database, cookies.sqlite, with its journal files; Sprawl deletes them whole, never part of it.",
        risk: Risk::Amber,
        files: &["cookies.sqlite"],
        waives: &[],
    },
];

impl PrivacyPart {
    pub fn rule_id(&self) -> String {
        crate::cleaners::part_rule_id(self.browser, self.id)
    }

    pub fn browser(&self) -> &'static Browser {
        BROWSERS.iter().find(|b| b.cleaner == self.browser).expect("every part names a browser of BROWSERS")
    }
}

/// The part whose rule id is `id` (`privacy.chrome/cookies`).
pub fn part(id: &str) -> Option<&'static PrivacyPart> {
    PARTS.iter().find(|p| p.rule_id() == id)
}

/// A rule id that is a privacy part's.
pub fn is_privacy_id(id: &str) -> bool {
    part(id).is_some()
}

/// This account's AppData folders, where the profiles are looked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Places {
    pub local: PathBuf,
    pub roaming: PathBuf,
}

impl Places {
    /// From `%LOCALAPPDATA%` and `%APPDATA%`; None when either is unset or not a plain local
    /// folder (`X:\...`), and then nothing is offered or cleaned.
    pub fn current() -> Option<Places> {
        let get = |v: &str| std::env::var_os(v).map(PathBuf::from).filter(|p| plain_local(&p.to_string_lossy()));
        Some(Places { local: get("LOCALAPPDATA")?, roaming: get("APPDATA")? })
    }

    /// The folder `b` keeps its profiles in.
    pub fn profiles(&self, b: &Browser) -> PathBuf {
        let root = match b.kind {
            Kind::Chromium => &self.local,
            Kind::Firefox => &self.roaming,
        };
        root.join(b.profiles)
    }
}

/// `X:\...` with backslashes, no `.` or `..` segment and no name ending in a dot or space.
fn plain_local(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' && !p.contains('/') && p[3..].split('\\').filter(|s| !s.is_empty()).all(|s| s != "." && s != ".." && !s.ends_with('.') && !s.ends_with(' '))
}

/// A profile folder's name `kind` makes: Chromium's `Default` or `Profile N`; for Firefox, one
/// plain name (it makes `xxxxxxxx.default-release`, and the user may name one).
pub fn profile_name_ok(kind: Kind, name: &str) -> bool {
    match kind {
        Kind::Chromium => crate::safety::chromium_profile_name(name),
        Kind::Firefox => !name.is_empty() && name.len() <= 120 && name != "." && name != ".." && !name.ends_with('.') && !name.ends_with(' ') && !name.contains(['\\', '/', ':', '*', '?']),
    }
}

/// A file of a part, worked out from its path.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    /// The profile folder it is in.
    pub profile: PathBuf,
    /// Which of [`PrivacyPart::files`] it is.
    pub file: usize,
    /// One of [`SIDE_FILES`], when it is a database's side file.
    pub side: Option<&'static str>,
}

/// What `path` is to `part`, if it is one of its files (or their side files) in a profile of
/// this account (`places`). Exact: `<profiles>\<profile>\<file>[<side>]`, nothing else.
pub fn member(part: &PrivacyPart, path: &Path, places: &Places) -> Option<Member> {
    let s = path.to_string_lossy();
    if !plain_local(&s) {
        return None;
    }
    let b = part.browser();
    let base = places.profiles(b);
    let base_segs = crate::segments(&base.to_string_lossy());
    let raw: Vec<&str> = s.split('\\').filter(|x| !x.is_empty()).collect();
    if raw.len() < base_segs.len() + 2 || crate::segments(&s)[..base_segs.len()] != base_segs[..] {
        return None;
    }
    let profile_name = raw[base_segs.len()];
    if !profile_name_ok(b.kind, profile_name) {
        return None;
    }
    let rest = raw[base_segs.len() + 1..].join("\\").to_lowercase();
    for (i, f) in part.files.iter().enumerate() {
        let f = f.to_lowercase();
        if rest == f {
            return Some(Member { profile: base.join(profile_name), file: i, side: None });
        }
        if let Some(sfx) = SIDE_FILES.iter().find(|sfx| rest == format!("{f}{sfx}")) {
            return Some(Member { profile: base.join(profile_name), file: i, side: Some(sfx) });
        }
    }
    None
}

/// The parts of the browser whose cleaner id is `cleaner`.
pub fn parts_of(cleaner: &str) -> impl Iterator<Item = &'static PrivacyPart> + '_ {
    PARTS.iter().filter(move |p| p.browser == cleaner)
}

/// The privacy cleaners as the Cleaners section lists them (one per browser, its parts class
/// privacy, closed = must). For `cleaners::catalogue_with` when the setting is on.
pub fn catalogue() -> Vec<CleanerDef> {
    BROWSERS
        .iter()
        .map(|b| {
            let base = match b.kind {
                Kind::Chromium => format!(r"%LOCALAPPDATA%\{}", b.profiles),
                Kind::Firefox => format!(r"%APPDATA%\{}", b.profiles),
            };
            let parts: Vec<PartDef> = parts_of(b.cleaner)
                .map(|p| PartDef {
                    rule: p.rule_id(),
                    title: p.title.into(),
                    class: Class::Privacy,
                    risk: p.risk,
                    closed: Closed::Must,
                    cost: p.cost.into(),
                    why: format!("{} {DELETED_NOTE}", p.why),
                    keep_days: None,
                    clears: false,
                    info: false,
                    paths: p.files.iter().map(|f| format!(r"{base}\<profile>\{f}")).collect(),
                })
                .collect();
            let mut text = format!("# {}: privacy parts built into Sprawl (they can't be written in a rule pack).\n# Each file goes with its -journal, -wal and -shm files, all of them or none, only with {} closed.\n", b.name, b.short);
            for p in &parts {
                text += &format!("# {} [{}]: {}\n#   {}\n", p.title, p.rule, p.cost, p.paths.join(", "));
            }
            CleanerDef {
                id: b.cleaner.into(),
                name: b.name.into(),
                group: "Browsers".into(),
                present: vec![base],
                running: b.running.iter().map(|r| r.to_string()).collect(),
                builtin: true,
                pack: "Sprawl's built-in privacy cleaners".into(),
                parts,
                text,
            }
        })
        .collect()
}

/// The names the browser gives its profiles, by folder (lower case): Chromium's `Local State`
/// (`profile.info_cache.<folder>.name`), Firefox's `profiles.ini` (`Name=` of a relative `Path=
/// Profiles/<folder>`). Read only; a file that is missing, too big or not as expected gives none.
pub fn profile_names(b: &Browser, places: &Places) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let read = |p: &Path| -> Option<String> {
        let m = std::fs::symlink_metadata(p).ok()?;
        if !m.is_file() || m.len() > 16 << 20 {
            return None;
        }
        std::fs::read_to_string(p).ok()
    };
    match b.kind {
        Kind::Chromium => {
            let Some(text) = read(&places.profiles(b).join("Local State")) else { return out };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return out };
            if let Some(cache) = v.pointer("/profile/info_cache").and_then(|c| c.as_object()) {
                for (dir, info) in cache {
                    if let Some(name) = info.get("name").and_then(|n| n.as_str()) {
                        out.insert(dir.to_lowercase(), name.to_string());
                    }
                }
            }
        }
        Kind::Firefox => {
            let Some(text) = places.profiles(b).parent().and_then(|d| read(&d.join("profiles.ini"))) else { return out };
            let (mut name, mut path, mut relative) = (None::<String>, None::<String>, false);
            let flush = |name: &mut Option<String>, path: &mut Option<String>, relative: &mut bool, out: &mut HashMap<String, String>| {
                if let (Some(n), Some(p), true) = (name.take(), path.take(), *relative) {
                    let p = p.replace('/', "\\");
                    if let Some(dir) = p.strip_prefix("Profiles\\").filter(|d| !d.contains('\\')) {
                        out.insert(dir.to_lowercase(), n);
                    }
                }
                *relative = false;
            };
            for line in text.lines().map(str::trim) {
                if line.starts_with('[') {
                    flush(&mut name, &mut path, &mut relative, &mut out);
                } else if let Some(v) = line.strip_prefix("Name=") {
                    name = Some(v.to_string());
                } else if let Some(v) = line.strip_prefix("Path=") {
                    path = Some(v.to_string());
                } else if let Some(v) = line.strip_prefix("IsRelative=") {
                    relative = v == "1";
                }
            }
            flush(&mut name, &mut path, &mut relative, &mut out);
        }
    }
    // a name from a file is shown as text only: short, one line
    for v in out.values_mut() {
        *v = v.chars().filter(|c| !c.is_control()).take(40).collect::<String>().trim().to_string();
    }
    out.retain(|_, v| !v.is_empty());
    out
}

/// The child of `n` called `name` (any case).
fn child(tree: &Tree, n: usize, name: &str) -> Option<usize> {
    tree.nodes[n].children.iter().copied().find(|&c| tree.nodes[c].name.eq_ignore_ascii_case(name))
}

/// A plain file (not a folder, not a link).
fn plain_file(tree: &Tree, k: usize) -> bool {
    !tree.nodes[k].is_dir && !tree.nodes[k].is_reparse_point()
}

/// Every privacy part in this account's browser profiles that the scan has, as findings (not yet
/// checked for safety). Call only when the setting is on. Each finding's path is the part's
/// first file, its size that of every file of the set, its action the set.
pub fn find(tree: &Tree) -> Vec<Finding> {
    match Places::current() {
        Some(p) => find_in(tree, &p),
        None => Vec::new(),
    }
}

pub(crate) fn find_in(tree: &Tree, places: &Places) -> Vec<Finding> {
    let mut out = Vec::new();
    for b in BROWSERS {
        let base = places.profiles(b);
        let Some(n) = crate::find_path(tree, &base) else { continue };
        if !tree.nodes[n].is_dir || tree.nodes[n].is_reparse_point() {
            continue;
        }
        let names = profile_names(b, places);
        let marker = match b.kind {
            Kind::Chromium => "Preferences",
            Kind::Firefox => "prefs.js",
        };
        let mut profiles: Vec<usize> = tree.nodes[n].children.iter().copied().filter(|&c| tree.nodes[c].is_dir && !tree.nodes[c].is_reparse_point() && profile_name_ok(b.kind, &tree.nodes[c].name)).collect();
        profiles.sort_by_key(|&c| tree.nodes[c].name.to_lowercase());
        for prof in profiles {
            // a real profile: the browser's own settings file is in it
            if !child(tree, prof, marker).is_some_and(|k| plain_file(tree, k)) {
                continue;
            }
            let folder = tree.nodes[prof].name.clone();
            let shown = names.get(&folder.to_lowercase()).cloned().unwrap_or_else(|| folder.clone());
            'part: for part in parts_of(b.cleaner) {
                let mut files = Vec::new();
                let mut members: Vec<(String, u64)> = Vec::new();
                let mut bytes = 0u64;
                for (i, rel) in part.files.iter().enumerate() {
                    let mut k = Some(prof);
                    let segs: Vec<&str> = rel.split('\\').collect();
                    for (j, seg) in segs.iter().enumerate() {
                        k = k.and_then(|x| child(tree, x, seg)).filter(|&x| j + 1 == segs.len() || (tree.nodes[x].is_dir && !tree.nodes[x].is_reparse_point()));
                    }
                    let Some(k) = k else {
                        if i == 0 {
                            continue 'part;
                        }
                        continue;
                    };
                    if !plain_file(tree, k) {
                        // something odd where a database should be: not offered at all
                        continue 'part;
                    }
                    files.push(tree.path_of(k));
                    members.push((rel.to_string(), tree.nodes[k].size));
                    bytes += tree.nodes[k].size;
                    let Some(parent) = tree.nodes[k].parent else { continue 'part };
                    for sfx in SIDE_FILES {
                        let side = format!("{}{sfx}", tree.nodes[k].name);
                        if let Some(s) = child(tree, parent, &side) {
                            if !plain_file(tree, s) {
                                continue 'part;
                            }
                            members.push((format!("{rel}{sfx}"), tree.nodes[s].size));
                            bytes += tree.nodes[s].size;
                        }
                    }
                }
                let list: Vec<String> = members.iter().map(|(m, b)| format!("{m} ({})", sprawl_core::format_size(*b))).collect();
                out.push(Finding {
                    path: files[0].clone(),
                    owner: Some(b.name.into()),
                    bytes,
                    risk: part.risk.max(Risk::Amber),
                    remedy: Remedy::Delete,
                    command: None,
                    title: format!("{}: {} ({shown})", b.short, part.title),
                    evidence: vec![
                        part.cost.to_string(),
                        part.why.to_string(),
                        format!("Profile \"{shown}\", in {}", tree.path_of(prof).display()),
                        format!("Goes together, or not at all: {}", list.join(", ")),
                        format!("Only with {} closed: checked when you review it and again just before.", b.short),
                        DELETED_NOTE.to_string(),
                    ],
                    confidence: 1.0,
                    source: Source::Rule(part.rule_id()),
                    allow_protected: false,
                    trusted: true,
                    builtin: true,
                    on_path: true,
                    action: Some(Action::Set { part: part.rule_id(), files }),
                    admin: false,
                    carved: Vec::new(),
                    gain: None,
                    forced_from: None,
                });
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn write(p: &Path, n: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![7u8; n]).unwrap();
    }

    /// A fake AppData with Chrome (two profiles and a guest), Edge with no history, Firefox
    /// (a profile named in profiles.ini, and one without prefs.js), each with what must stay.
    pub(crate) fn fixture(tag: &str) -> (PathBuf, Places) {
        let base = std::env::temp_dir().join(format!("sprawl-privacy-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let places = Places { local: base.join("Local"), roaming: base.join("Roaming") };
        let chrome = places.local.join(r"Google\Chrome\User Data");
        std::fs::create_dir_all(&chrome).unwrap();
        std::fs::write(chrome.join("Local State"), r#"{"profile":{"info_cache":{"Default":{"name":"Person 1"},"Profile 2":{"name":"Work"}}}}"#).unwrap();
        for prof in ["Default", "Profile 2"] {
            let p = chrome.join(prof);
            for (f, n) in [("Preferences", 10), (r"Network\Cookies", 1000), (r"Network\Cookies-journal", 20), ("History", 2000), ("History-journal", 30), ("Visited Links", 40), ("Top Sites", 50), ("Shortcuts", 60), ("Login Data", 70), ("Bookmarks", 80), ("Web Data", 90), (r"Cache\Cache_Data\f_000001", 500), ("Favicons", 5)] {
                write(&p.join(f), n);
            }
        }
        // a profile Chromium didn't name so: never offered
        write(&chrome.join("Guest Profile").join("Preferences"), 10);
        write(&chrome.join("Guest Profile").join("Network").join("Cookies"), 100);
        // Edge: cookies only
        let edge = places.local.join(r"Microsoft\Edge\User Data\Default");
        write(&edge.join("Preferences"), 10);
        write(&edge.join("Network").join("Cookies"), 300);
        let ff = places.roaming.join(r"Mozilla\Firefox");
        std::fs::create_dir_all(&ff).unwrap();
        std::fs::write(ff.join("profiles.ini"), "[General]\nStartWithLastProfile=1\n\n[Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/abcd1234.default-release\nDefault=1\n").unwrap();
        let fp = ff.join(r"Profiles\abcd1234.default-release");
        for (f, n) in [("prefs.js", 10), ("cookies.sqlite", 4000), ("cookies.sqlite-wal", 300), ("cookies.sqlite-shm", 32), ("places.sqlite", 9000), ("formhistory.sqlite", 100), ("logins.json", 50), ("key4.db", 50)] {
            write(&fp.join(f), n);
        }
        // a folder in Profiles that isn't a profile (no prefs.js)
        write(&ff.join(r"Profiles\leftover\cookies.sqlite"), 100);
        (base, places)
    }

    #[test]
    fn profiles_are_found_by_name_and_only_real_ones() {
        let (base, places) = fixture("find");
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let found = find_in(&tree, &places);
        let mut titles: Vec<&str> = found.iter().map(|f| f.title.as_str()).collect();
        titles.sort();
        assert_eq!(
            titles,
            [
                "Chrome: Cookies (Person 1)",
                "Chrome: Cookies (Work)",
                "Chrome: History and download list (Person 1)",
                "Chrome: History and download list (Work)",
                "Edge: Cookies (Default)",
                "Firefox: Cookies (default-release)",
            ]
        );
        let ck = found.iter().find(|f| f.title == "Chrome: Cookies (Person 1)").unwrap();
        assert_eq!(ck.bytes, 1020, "the cookies and their journal");
        assert_eq!(ck.action, Some(Action::Set { part: "privacy.chrome/cookies".into(), files: vec![places.local.join(r"Google\Chrome\User Data\Default\Network\Cookies")] }));
        assert!(ck.command.is_none() && ck.privacy() && ck.risk >= Risk::Amber);
        let hist = found.iter().find(|f| f.title == "Chrome: History and download list (Work)").unwrap();
        assert_eq!(hist.bytes, 2000 + 30 + 40 + 50 + 60);
        assert_eq!(hist.path, places.local.join(r"Google\Chrome\User Data\Profile 2\History"));
        let ff = found.iter().find(|f| f.title.starts_with("Firefox")).unwrap();
        assert_eq!(ff.bytes, 4000 + 300 + 32);
        // nothing else of a profile is ever a member
        for f in &found {
            let Some(Action::Set { files, .. }) = &f.action else { panic!() };
            for p in files {
                let name = p.file_name().unwrap().to_string_lossy().to_lowercase();
                assert!(["cookies", "history", "visited links", "top sites", "shortcuts", "cookies.sqlite"].contains(&name.as_str()), "{}", p.display());
            }
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Only a part's own files, in a profile of this account, by the exact layout.
    #[test]
    fn members_are_exact() {
        let places = Places { local: r"C:\Users\sam\AppData\Local".into(), roaming: r"C:\Users\sam\AppData\Roaming".into() };
        let chrome = part("privacy.chrome/cookies").unwrap();
        let hist = part("privacy.chrome/history").unwrap();
        let ff = part("privacy.firefox/cookies").unwrap();
        let ud = r"C:\Users\sam\AppData\Local\Google\Chrome\User Data";
        let m = |p: &PrivacyPart, s: &str| member(p, Path::new(s), &places);
        assert_eq!(m(chrome, &format!(r"{ud}\Default\Network\Cookies")).map(|x| (x.file, x.side)), Some((0, None)));
        assert_eq!(m(chrome, &format!(r"{ud}\Profile 3\Network\Cookies-journal")).map(|x| (x.file, x.side)), Some((0, Some("-journal"))));
        assert_eq!(m(hist, &format!(r"{ud}\Default\Top Sites-wal")).map(|x| (x.file, x.side)), Some((2, Some("-wal"))));
        for bad in [
            format!(r"{ud}\Default\Login Data"),
            format!(r"{ud}\Default\Web Data"),
            format!(r"{ud}\Default\Bookmarks"),
            format!(r"{ud}\Default\Cookies"),
            format!(r"{ud}\Default\Network\Cookies.bak"),
            format!(r"{ud}\Default\Network\Cookies-journal-x"),
            format!(r"{ud}\Default\Network"),
            format!(r"{ud}\Default"),
            format!(r"{ud}\Guest Profile\Network\Cookies"),
            format!(r"{ud}\System Profile\Network\Cookies"),
            format!(r"{ud}\Network\Cookies"),
            format!(r"{ud}\Default\x\Network\Cookies"),
            r"C:\Users\other\AppData\Local\Google\Chrome\User Data\Default\Network\Cookies".to_string(),
            r"C:\Users\sam\AppData\Local\Microsoft\Edge\User Data\Default\Network\Cookies".to_string(),
            format!(r"{ud}\..\User Data\Default\Network\Cookies"),
            format!(r"\\?\{ud}\Default\Network\Cookies"),
            format!(r"{ud}/Default/Network/Cookies"),
        ] {
            assert!(m(chrome, &bad).is_none(), "{bad}");
        }
        let prof = r"C:\Users\sam\AppData\Roaming\Mozilla\Firefox\Profiles";
        assert!(m(ff, &format!(r"{prof}\abc.default-release\cookies.sqlite-shm")).is_some());
        for bad in ["places.sqlite", "formhistory.sqlite", "logins.json", "key4.db", r"storage\cookies.sqlite", "cookies.sqlite.bak"] {
            assert!(m(ff, &format!(r"{prof}\abc.default\{bad}")).is_none(), "{bad}");
        }
        assert!(m(ff, &format!(r"{prof}\cookies.sqlite")).is_none(), "not in a profile");
        assert!(m(ff, &format!(r"{prof}\..\cookies.sqlite")).is_none());
        // every part: the first file is the database that names it, and nothing of what must
        // stay is a member of any part, in any profile
        let keep = ["Login Data", "Login Data For Account", "Web Data", "Bookmarks", "Preferences", "Secure Preferences", "Local Storage", "IndexedDB", "Sessions", "Current Session", "Last Session", "Current Tabs", "Last Tabs", "Extensions", "Local State", "Favicons", "places.sqlite", "formhistory.sqlite", "logins.json", "key4.db", "cert9.db", "prefs.js", "sessionstore.jsonlz4"];
        for p in PARTS {
            assert!(p.risk >= Risk::Amber, "{}", p.rule_id());
            let base = places.profiles(p.browser());
            for prof in ["Default", "Profile 1", "abc.default-release"] {
                for k in keep {
                    for sfx in ["", "-journal", "-wal"] {
                        let path = base.join(prof).join(format!("{k}{sfx}"));
                        assert!(member(p, &path, &places).is_none(), "{} {}", p.rule_id(), path.display());
                    }
                }
            }
        }
    }

    /// No privacy part's words say or suggest that what goes is beyond recovery, or that
    /// cleaning it is without cost.
    #[test]
    fn the_words_promise_nothing() {
        const BANNED: &[&str] = PROMISE_WORDS;
        let mut texts: Vec<String> = Vec::new();
        for p in PARTS {
            texts.extend([p.title, p.cost, p.why].map(String::from));
        }
        for d in catalogue() {
            texts.push(d.text.clone());
            for p in &d.parts {
                texts.extend([p.title.clone(), p.cost.clone(), p.why.clone()]);
            }
        }
        let (base, places) = fixture("words");
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        for f in find_in(&tree, &places) {
            texts.push(f.title.clone());
            texts.extend(f.evidence.clone());
        }
        let _ = std::fs::remove_dir_all(&base);
        texts.push(DELETED_NOTE.into());
        assert!(texts.len() > 40);
        for t in &texts {
            let low = t.to_lowercase();
            for w in BANNED {
                // "recoverable" stands for itself only; "recovery tools may still find it" is the
                // honest note every part carries
                assert!(!low.contains(w), "{w:?} in {t:?}");
            }
        }
        // and each says what is lost, and that it is deleted rather than overwritten
        for p in PARTS {
            assert!(!p.cost.is_empty() && p.cost.len() < 240, "{}", p.rule_id());
        }
        for d in catalogue() {
            assert!(d.parts.iter().all(|p| p.why.ends_with(DELETED_NOTE) && p.class == Class::Privacy && p.closed == Closed::Must && p.risk >= Risk::Amber));
        }
    }

    /// In the real layout (`C:\Users\<name>\AppData\...`), each part's files are on the
    /// never-list only by the browser's whole folder and the names the part waives; what must
    /// stay beside them is held by a name no part waives.
    #[test]
    fn in_the_real_layout_only_covers_and_waives_are_passed() {
        let never = crate::safety::Never::default_list();
        let places = Places { local: r"C:\Users\sam\AppData\Local".into(), roaming: r"C:\Users\sam\AppData\Roaming".into() };
        let cover = |h: &str| h.ends_with(r"\**") && !h.starts_with(r"?:\**");
        for p in PARTS {
            let base = places.profiles(p.browser());
            for f in p.files {
                for sfx in std::iter::once("").chain(SIDE_FILES.iter().copied()) {
                    let path = base.join(if p.browser == "privacy.firefox" { "abc.default-release" } else { "Profile 1" }).join(format!("{f}{sfx}"));
                    let hits = never.all_hits(&path.to_string_lossy());
                    assert!(!hits.is_empty(), "{}: the browser's folder is protected", path.display());
                    assert!(hits.iter().all(|h| cover(h) || p.waives.contains(h)), "{} {}: {hits:?}", p.rule_id(), path.display());
                }
            }
            // every waive is a never-list entry, found by the part's own first file
            for w in p.waives {
                let path = base.join("Default").join(p.files[0]);
                assert!(never.all_hits(&path.to_string_lossy()).contains(w), "{w}");
            }
            for k in ["Login Data", "Web Data"] {
                let hits = never.all_hits(&base.join("Default").join(k).to_string_lossy());
                assert!(hits.iter().any(|h| !cover(h) && !p.waives.contains(h)), "{k} is held for {}", p.rule_id());
            }
        }
    }

    #[test]
    fn profile_names_come_from_the_browser() {
        let (base, places) = fixture("names");
        let chrome = profile_names(&BROWSERS[0], &places);
        assert_eq!(chrome.get("default").map(String::as_str), Some("Person 1"));
        assert_eq!(chrome.get("profile 2").map(String::as_str), Some("Work"));
        let ff = profile_names(&BROWSERS[3], &places);
        assert_eq!(ff.get("abcd1234.default-release").map(String::as_str), Some("default-release"));
        // a broken Local State gives none, not an error
        std::fs::write(places.profiles(&BROWSERS[0]).join("Local State"), "{not json").unwrap();
        assert!(profile_names(&BROWSERS[0], &places).is_empty());
        assert!(profile_names(&BROWSERS[1], &places).is_empty(), "no Local State");
        let _ = std::fs::remove_dir_all(&base);
    }

    use crate::clean::{Outcome, Spare};
    use std::collections::HashSet;
    use std::sync::atomic::AtomicBool;

    fn none_running() -> HashSet<String> {
        ["explorer.exe".to_string()].into()
    }

    fn nobody(_: &[PathBuf]) -> Option<Vec<String>> {
        Some(Vec::new())
    }

    /// Run a part's set on the fixture, as `clean::run` does but with the fixture's AppData, the
    /// given process list and the given Restart Manager answer.
    fn run_set(id: &str, files: &[PathBuf], spare: &Spare, places: &Places, now: Option<&HashSet<String>>, in_use: &dyn Fn(&[PathBuf]) -> Option<Vec<String>>) -> Outcome {
        let mut o = Outcome::default();
        crate::clean::set(part(id).unwrap(), files, spare, &AtomicBool::new(false), places, now, in_use, &mut o, &mut |_| {});
        o
    }

    /// The set goes whole: the database and its side files, nothing else of the profile.
    #[test]
    fn a_set_goes_together() {
        let (base, places) = fixture("set");
        let prof = places.local.join(r"Google\Chrome\User Data\Default");
        let cookies = vec![prof.join(r"Network\Cookies")];
        // a side file the scan didn't see goes with it too
        write(&prof.join(r"Network\Cookies-wal"), 15);
        let o = run_set("privacy.chrome/cookies", &cookies, &Spare::default(), &places, Some(&none_running()), &nobody);
        assert!(o.problems.is_empty(), "{:?}", o.problems);
        assert_eq!((o.files, o.freed), (3, 1000 + 20 + 15));
        for k in [r"Network\Cookies", r"Network\Cookies-journal", r"Network\Cookies-wal"] {
            assert!(!prof.join(k).exists(), "{k}");
        }
        for k in ["Login Data", "Bookmarks", "Web Data", "History", "Preferences", "Favicons", r"Cache\Cache_Data\f_000001"] {
            assert!(prof.join(k).exists(), "{k} must stay");
        }
        assert!(places.local.join(r"Google\Chrome\User Data\Profile 2\Network\Cookies").exists(), "another profile stays");
        // history: the four files and their journals, the downloaded list with them
        let files: Vec<PathBuf> = ["History", "Visited Links", "Top Sites", "Shortcuts"].iter().map(|f| prof.join(f)).collect();
        let o = run_set("privacy.chrome/history", &files, &Spare::default(), &places, Some(&none_running()), &nobody);
        assert_eq!((o.files, o.freed), (5, 2000 + 30 + 40 + 50 + 60), "{:?}", o.problems);
        assert!(prof.join("Login Data").exists() && prof.join("Bookmarks").exists() && prof.join("Favicons").exists());
        // Firefox: the database with its write-ahead log and index; places.sqlite stays
        let fp = places.roaming.join(r"Mozilla\Firefox\Profiles\abcd1234.default-release");
        let o = run_set("privacy.firefox/cookies", &[fp.join("cookies.sqlite")], &Spare::default(), &places, Some(&none_running()), &nobody);
        assert_eq!((o.files, o.freed), (3, 4000 + 300 + 32), "{:?}", o.problems);
        for k in ["places.sqlite", "formhistory.sqlite", "logins.json", "key4.db", "prefs.js"] {
            assert!(fp.join(k).exists(), "{k} must stay");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// One locked file holds the whole set, however the lock is found: by the Restart Manager,
    /// or (when it says nothing) by opening each file alone.
    #[test]
    fn a_locked_member_holds_the_whole_set() {
        let (base, places) = fixture("locked");
        let prof = places.local.join(r"Google\Chrome\User Data\Default");
        let cookies = vec![prof.join(r"Network\Cookies")];
        let all = [prof.join(r"Network\Cookies"), prof.join(r"Network\Cookies-journal")];
        let lock = std::fs::File::open(prof.join(r"Network\Cookies-journal")).unwrap();
        // the Restart Manager sees this test holding the journal
        let o = run_set("privacy.chrome/cookies", &cookies, &Spare::default(), &places, Some(&none_running()), &crate::inuse::in_use);
        assert!(o.problems.iter().any(|p| p.contains("in use by") && p.contains("nothing in this set was deleted")), "{:?}", o.problems);
        assert!(all.iter().all(|p| p.exists()) && o.files == 0);
        // a lock the Restart Manager doesn't report: the file can't be opened alone
        let o = run_set("privacy.chrome/cookies", &cookies, &Spare::default(), &places, Some(&none_running()), &nobody);
        assert!(o.problems.iter().any(|p| p.contains("opened alone")), "{:?}", o.problems);
        assert!(all.iter().all(|p| p.exists()) && o.files == 0, "the database stays with its journal");
        // the main file locked instead
        drop(lock);
        let lock = std::fs::OpenOptions::new().read(true).write(true).open(&all[0]).unwrap();
        let o = run_set("privacy.chrome/cookies", &cookies, &Spare::default(), &places, Some(&none_running()), &nobody);
        assert!(all.iter().all(|p| p.exists()) && o.files == 0, "{:?}", o.problems);
        drop(lock);
        // unlocked: it goes
        let o = run_set("privacy.chrome/cookies", &cookies, &Spare::default(), &places, Some(&none_running()), &nobody);
        assert!(all.iter().all(|p| !p.exists()) && o.files == 2, "{:?}", o.problems);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The browser open, or Windows not saying, holds it; so do a changed set, a kept file, a
    /// file that isn't the part's, and a link above it.
    #[test]
    fn a_set_waits_for_its_browser_and_its_checks() {
        let (base, places) = fixture("wait");
        let prof = places.local.join(r"Google\Chrome\User Data\Default");
        let cookies = vec![prof.join(r"Network\Cookies")];
        let stays = |o: &Outcome| prof.join(r"Network\Cookies").exists() && prof.join(r"Network\Cookies-journal").exists() && o.files == 0 && o.freed == 0;
        let open: HashSet<String> = ["chrome.exe".to_string()].into();
        let o = run_set("privacy.chrome/cookies", &cookies, &Spare::default(), &places, Some(&open), &nobody);
        assert!(stays(&o) && o.problems[0].contains("Chrome is open"), "{:?}", o.problems);
        let o = run_set("privacy.chrome/cookies", &cookies, &Spare::default(), &places, None, &nobody);
        assert!(stays(&o) && o.problems[0].contains("couldn't say whether Chrome is open"), "{:?}", o.problems);
        let o = run_set("privacy.chrome/cookies", &cookies, &Spare::default(), &places, Some(&none_running()), &|_| None);
        assert!(stays(&o) && o.problems[0].contains("couldn't say which programs"), "{:?}", o.problems);
        // Edge's part doesn't take Chrome's files, nor does Chrome's another profile's mix
        let o = run_set("privacy.edge/cookies", &cookies, &Spare::default(), &places, Some(&none_running()), &nobody);
        assert!(stays(&o), "{:?}", o.problems);
        let mixed = vec![prof.join("History"), places.local.join(r"Google\Chrome\User Data\Profile 2\Visited Links")];
        let o = run_set("privacy.chrome/history", &mixed, &Spare::default(), &places, Some(&none_running()), &nobody);
        assert!(o.files == 0 && prof.join("History").exists(), "{:?}", o.problems);
        // a set naming what must stay, or only a side file, or not its first file
        for files in [vec![prof.join(r"Network\Cookies"), prof.join("Login Data")], vec![prof.join(r"Network\Cookies-journal")], vec![prof.join("Visited Links")], vec![]] {
            let id = if files.iter().any(|f| f.ends_with("Visited Links")) { "privacy.chrome/history" } else { "privacy.chrome/cookies" };
            let o = run_set(id, &files, &Spare::default(), &places, Some(&none_running()), &nobody);
            assert!(o.files == 0 && !o.problems.is_empty(), "{files:?}");
        }
        assert!(prof.join("Login Data").exists() && prof.join("Visited Links").exists());
        // kept by the user, at any depth above
        let kept = Spare { since: None, keep: vec![prof.join("Network")] };
        let o = run_set("privacy.chrome/cookies", &cookies, &kept, &places, Some(&none_running()), &nobody);
        assert!(stays(&o) && o.kept_chosen == 1, "{:?}", o.problems);
        // a side file that appeared after the scan: the set changed, all of it stays
        let since = Spare { since: Some(std::time::SystemTime::now()), keep: Vec::new() };
        std::thread::sleep(std::time::Duration::from_millis(50));
        write(&prof.join(r"Network\Cookies-wal"), 5);
        let o = run_set("privacy.chrome/cookies", &cookies, &since, &places, Some(&none_running()), &nobody);
        assert!(stays(&o) && prof.join(r"Network\Cookies-wal").exists() && o.kept_new == 1, "{:?}", o.problems);
        // a profile folder swapped for a junction
        let real = base.join("elsewhere");
        let p2 = places.local.join(r"Google\Chrome\User Data\Profile 9");
        write(&real.join(r"Network\Cookies"), 10);
        if std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&p2).arg(&real).output().is_ok_and(|o| o.status.success()) {
            let o = run_set("privacy.chrome/cookies", &[p2.join(r"Network\Cookies")], &Spare::default(), &places, Some(&none_running()), &nobody);
            assert!(o.files == 0 && real.join(r"Network\Cookies").exists(), "{:?}", o.problems);
            let _ = std::fs::remove_dir(&p2);
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The preview is the set the run would take: every file, by name and size; nothing goes.
    #[test]
    fn the_preview_lists_the_exact_files() {
        let (base, places) = fixture("preview");
        let prof = places.local.join(r"Google\Chrome\User Data\Default");
        let p = part("privacy.chrome/history").unwrap();
        let files: Vec<PathBuf> = ["History", "Visited Links", "Top Sites", "Shortcuts"].iter().map(|f| prof.join(f)).collect();
        let got = crate::clean::set_preview(p, &files, &Spare::default(), &places);
        let names: Vec<(String, u64)> = got.biggest.iter().map(|(f, b)| (f.file_name().unwrap().to_string_lossy().into_owned(), *b)).collect();
        assert_eq!(names, [("History".to_string(), 2000), ("Shortcuts".to_string(), 60), ("Top Sites".to_string(), 50), ("Visited Links".to_string(), 40), ("History-journal".to_string(), 30)]);
        assert_eq!((got.bytes, got.files), (2180, 5));
        assert!(files.iter().all(|f| f.exists()));
        // the preview and the finding agree
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let f = find_in(&tree, &places).into_iter().find(|f| f.path == files[0]).unwrap();
        assert_eq!(f.bytes, got.bytes);
        // a kept file: the run would take nothing, so the preview shows nothing
        let kept = Spare { since: None, keep: vec![prof.join("Top Sites")] };
        assert_eq!(crate::clean::set_preview(p, &files, &kept, &places).files, 0);
        // not a part's own file: nothing
        assert_eq!(crate::clean::set_preview(p, &[prof.join("Login Data")], &Spare::default(), &places).files, 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The safety check takes a set only as this module makes it: Sprawl's own, every file the
    /// part's own, sizes as the scan has them. Nothing from a model or a pack gets through.
    #[test]
    fn only_sprawls_own_sets_pass_the_checks() {
        let (base, places) = fixture("check");
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let never = crate::safety::Never::default_list();
        let home = Some(base.as_path());
        let check = |f: &Finding| crate::safety::check_in(f, &tree, &never, crate::safety::CACHE_CARVE_OUTS, home, Some(&places));
        let found = find_in(&tree, &places);
        assert_eq!(found.len(), 6);
        for f in &found {
            let ok = check(f).unwrap_or_else(|e| panic!("{}: {e}", f.title));
            assert_eq!(ok.bytes, f.bytes);
            assert!(crate::clean::action(&ok).is_some(), "{}", f.title);
        }
        let good = found.iter().find(|f| f.title == "Chrome: Cookies (Person 1)").unwrap().clone();
        let prof = places.local.join(r"Google\Chrome\User Data\Default");
        let set = |files: Vec<PathBuf>| Some(Action::Set { part: "privacy.chrome/cookies".into(), files });
        let bad: Vec<(&str, Finding)> = vec![
            ("a model", Finding { source: Source::Model("m".into()), ..good.clone() }),
            ("a pack on disk", Finding { builtin: false, ..good.clone() }),
            ("untrusted", Finding { trusted: false, ..good.clone() }),
            ("with a command", Finding { command: Some("Remove-Item x".into()), ..good.clone() }),
            ("another rule's", Finding { source: Source::Rule("privacy.chrome/history".into()), ..good.clone() }),
            ("an unknown part", Finding { action: Some(Action::Set { part: "privacy.chrome/passwords".into(), files: vec![prof.join(r"Network\Cookies")] }), source: Source::Rule("privacy.chrome/passwords".into()), ..good.clone() }),
            ("passwords in the set", Finding { action: set(vec![prof.join(r"Network\Cookies"), prof.join("Login Data")]), ..good.clone() }),
            ("passwords as the set", Finding { path: prof.join("Login Data"), action: set(vec![prof.join("Login Data")]), ..good.clone() }),
            ("a guest profile", Finding { path: places.local.join(r"Google\Chrome\User Data\Guest Profile\Network\Cookies"), action: set(vec![places.local.join(r"Google\Chrome\User Data\Guest Profile\Network\Cookies")]), bytes: 100, ..good.clone() }),
            ("a side file alone", Finding { path: prof.join(r"Network\Cookies-journal"), action: set(vec![prof.join(r"Network\Cookies-journal")]), bytes: 20, ..good.clone() }),
            ("its path not the set's", Finding { path: prof.join("History"), ..good.clone() }),
            ("the same file twice", Finding { action: set(vec![prof.join(r"Network\Cookies"), prof.join(r"Network\Cookies")]), ..good.clone() }),
            ("a privacy rule without its set", Finding { action: Some(Action::Remove), ..good.clone() }),
            ("a privacy rule with no action", Finding { action: None, ..good.clone() }),
        ];
        for (why, f) in &bad {
            assert!(check(f).is_err(), "{why} passed");
        }
        // the executor's own gate, whatever the check said
        for (why, f) in &bad[..6] {
            assert!(crate::clean::action(f).is_none(), "{why}");
        }
        // another account's profile, and AppData not known: nothing passes
        assert!(crate::safety::check_in(&good, &tree, &never, crate::safety::CACHE_CARVE_OUTS, Some(Path::new(r"C:\Users\someone-else")), Some(&places)).is_err());
        assert!(crate::safety::check_in(&good, &tree, &never, crate::safety::CACHE_CARVE_OUTS, home, None).is_err());
        // a size that isn't the set's
        assert!(matches!(check(&Finding { bytes: good.bytes + (50 << 20), ..good.clone() }), Err(crate::safety::Rejection::SizeMismatch { .. })));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Off (the default), the report and the cleaners list have nothing of privacy cleaning;
    /// on, its parts come in a list of their own, never among what frees space.
    #[test]
    fn the_setting_off_shows_nothing() {
        let (base, places) = fixture("flag");
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let facts = crate::facts::Facts::from_apps(Vec::new());
        let packs = [crate::RulePack::builtin()];
        let off = crate::advise_report(&tree, &packs, &facts, false);
        assert!(off.privacy.is_empty() && off.privacy_held.is_empty());
        assert!(off.findings.iter().chain(off.held.iter().map(|(f, _)| f)).all(|f| !f.privacy()));
        let off_in = crate::advise_report_in(&tree, &packs, &facts, false, Some(&places), Some(&base));
        assert!(off_in.privacy.is_empty() && off_in.privacy_held.is_empty(), "off, even with the profiles in the scan");
        assert!(crate::cleaners::catalogue_with(&packs, false).iter().all(|d| d.parts.iter().all(|p| p.class == Class::Space) && !d.id.starts_with("privacy.")));
        assert!(crate::cleaners::catalogue(&packs).iter().all(|d| d.parts.iter().all(|p| p.class == Class::Space)));
        // on
        let on = crate::advise_report_in(&tree, &packs, &facts, true, Some(&places), Some(&base));
        assert_eq!(on.privacy.len(), 6, "{:?}", on.privacy_held.iter().map(|(f, r)| format!("{}: {r}", f.title)).collect::<Vec<_>>());
        assert!(on.findings.iter().all(|f| !f.privacy()), "never among what frees space");
        assert_eq!(on.findings.len(), off_in.findings.len());
        let cat = crate::cleaners::catalogue_with(&packs, true);
        let privacy: Vec<&CleanerDef> = cat.iter().filter(|d| d.parts.iter().any(|p| p.class == Class::Privacy)).collect();
        assert_eq!(privacy.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["privacy.chrome", "privacy.edge", "privacy.brave", "privacy.firefox"]);
        for f in &on.privacy {
            let (d, p) = crate::cleaners::part_of(&cat, match &f.source {
                Source::Rule(id) => id,
                _ => unreachable!(),
            })
            .expect("each finding has its part");
            assert!(d.builtin && p.class == Class::Privacy && p.closed == Closed::Must);
        }
        // never planned, never in a script
        let counted = vec![true; on.privacy.len()];
        assert!(crate::plan::plan(&on.privacy, &counted, 1).picked.is_empty());
        let refs: Vec<&Finding> = on.privacy.iter().collect();
        let script = crate::export::script(&refs, "C:\\", std::time::SystemTime::now());
        assert!(on.privacy.iter().all(|f| !script.contains(&f.path.display().to_string()) && !script.contains(&f.title)), "{script}");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// No pack (the built-in TOML included), community import or model proposal can make a
    /// privacy part, or reach a cache carve-out.
    #[test]
    fn packs_imports_and_models_cannot_make_privacy_parts_or_carve_outs() {
        std::env::set_var("SystemDrive", "C:");
        std::env::set_var("SystemRoot", r"C:\Windows");
        std::env::set_var("LOCALAPPDATA", r"C:\Users\sam\AppData\Local");
        std::env::set_var("APPDATA", r"C:\Users\sam\AppData\Roaming");
        std::env::set_var("USERPROFILE", r"C:\Users\sam");
        std::env::set_var("ProgramData", r"C:\ProgramData");
        let never = crate::safety::Never::default_list();
        let cleaner = |class: &str, path: &str, risk: &str| {
            format!("[pack]\nname = 'x'\n[[cleaner]]\nid = 'winapp2.x'\nname = 'X'\ngroup = 'Browsers'\nrunning = ['chrome.exe']\n[[cleaner.part]]\nid = 'p'\ntitle = 'P'\nrisk = '{risk}'\nclass = '{class}'\nclosed = 'must'\npaths = ['{path}']\nwhy = 'w'\n")
        };
        let cookie = r"%LOCALAPPDATA%\Google\Chrome\User Data\Default\Network\Cookies";
        let cache = r"%LOCALAPPDATA%\Google\Chrome\User Data\Default\Cache";
        // any pack, trusted or not: class privacy is refused when it is read
        for risk in ["amber", "orange"] {
            assert!(crate::RulePack::parse(&cleaner("privacy", r"%LOCALAPPDATA%\Tool\cookies", risk)).is_err());
        }
        // a pack on disk, an import: neither a privacy part nor a carve-out path
        for (class, path) in [("privacy", cookie), ("space", cookie), ("space", cache), ("privacy", cache), ("space", r"%LOCALAPPDATA%\Microsoft\Edge\User Data\Default\Code Cache"), ("space", r"%LOCALAPPDATA%\Google\Chrome\User Data\Profile 1\GPUCache")] {
            let text = cleaner(class, path, "amber");
            assert!(crate::memory::check_pack_text(&text.replace("winapp2.x", "local.x"), &mut Default::default()).is_err(), "{class} {path}");
            assert!(crate::memory::check_import_text(&text, "winapp2.", &never, &mut Default::default()).is_err(), "{class} {path}");
        }
        // nor take a privacy cleaner's id, so nothing can stand in for one by its name
        for text in [cleaner("space", r"%LOCALAPPDATA%\Tool\x", "amber").replace("winapp2.x", "privacy.chrome"), cleaner("space", r"%LOCALAPPDATA%\Tool\x", "amber").replace("winapp2.x", "Privacy.Edge")] {
            assert!(crate::RulePack::parse(&text).is_err(), "{text}");
        }
        assert!(crate::RulePack::parse("[pack]\nname = 'x'\n[[rule]]\nid = 'privacy.chrome/cookies'\ntitle = 'T'\nwhy = 'w'\nrisk = 'amber'\nremedy = 'delete'\npaths = ['C:\\\\Tool\\\\x']\n").is_err());
        // a model's proposal
        for (class, path) in [(Class::Privacy, cookie), (Class::Space, cookie), (Class::Space, cache)] {
            let c: crate::cleaners::Cleaner = serde_json::from_value(serde_json::json!({
                "id": "browser", "name": "Browser", "group": "Browsers", "running": ["chrome.exe"],
                "part": [{"id": "p", "title": "P", "risk": "amber", "closed": "must", "paths": [path], "why": "w"}]
            }))
            .unwrap();
            let mut c = c;
            c.parts[0].class = class;
            let mut m = crate::memory::Memory::default();
            assert!(m.propose_cleaner(crate::memory::CleanerProposal { cleaner: c, note: String::new() }).is_err(), "{class:?} {path}");
        }
        // what packs can't name, the built-in pack doesn't either: its cleaners are space only,
        // and every privacy part is in PARTS
        let b = crate::RulePack::builtin();
        assert!(b.cleaners.iter().all(|c| c.parts.iter().all(|p| p.class == Class::Space) && !c.id.starts_with("privacy.")));
        assert!(b.all_rules().all(|r| !is_privacy_id(&r.id)));
    }
}
