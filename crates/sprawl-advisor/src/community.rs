//! Community cleaners: winapp2.ini and BleachBit's CleanerML, downloaded from their GitHub
//! repositories (never bundled with Sprawl, so their licences stay with their files) and turned
//! into Sprawl's own cleaner format entry by entry.
//!
//! Only what Sprawl's format can say survives: folders under the fixed `%VAR%` list, file
//! patterns, the process check. Registry keys, excludes, database edits, files of a folder's top
//! level only, and anything the never-list or the local-pack checks refuse are left out, each with
//! a reason the user can read. What is kept stays untrusted for good: at least amber, never
//! ticked by Sprawl, never privacy, and unusable until the user turns each cleaner on (the app
//! keeps that choice only while the cleaner's definition is unchanged).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::cleaners::{Cleaner, Closed, Part};
use crate::finding::Risk;
use crate::rules::{Act, Kind, RulePack};

pub const WINAPP2_URL: &str = "https://raw.githubusercontent.com/MoscaDotTo/Winapp2/master/Winapp2.ini";
pub const CLEANERML_LIST: &str = "https://api.github.com/repos/bleachbit/cleanerml/contents/release";
const CLEANERML_RAW: &str = "https://raw.githubusercontent.com/bleachbit/cleanerml/master/release/";
pub const WINAPP2: &str = "winapp2.";
pub const BLEACHBIT: &str = "bleachbit.";
/// How often the files are fetched again by themselves.
pub const EVERY: u64 = 7 * 86_400;
const MAX_INI: usize = 8 << 20;
const MAX_XML: usize = 512 << 10;
const MAX_FILES: usize = 200;

/// Whether a cleaner or rule id comes from a community file.
pub fn is_community(id: &str) -> bool {
    [WINAPP2, BLEACHBIT].iter().any(|prefix| id.get(..prefix.len()).is_some_and(|s| s.eq_ignore_ascii_case(prefix)))
}

/// An entry, or a part of one, that wasn't taken over, and why.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LeftOut {
    pub entry: String,
    pub why: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceStatus {
    pub name: String,
    pub licence: String,
    pub url: String,
    pub imported: usize,
    #[serde(default)]
    pub left_out: Vec<LeftOut>,
    /// The last download's problem, if it had one (the earlier import is kept then).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// When the files were last fetched (seconds since 1970); 0: never.
    #[serde(default)]
    pub fetched: u64,
    #[serde(default, rename = "source")]
    pub sources: Vec<SourceStatus>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Where the converted packs and the status are kept.
pub fn dir() -> Option<PathBuf> {
    crate::memory::dir().map(|d| d.join("community"))
}

pub fn status() -> Status {
    dir().and_then(|d| std::fs::read_to_string(d.join("status.toml")).ok()).and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

/// Whether it is time to fetch the files again.
pub fn due(s: &Status) -> bool {
    now().saturating_sub(s.fetched) >= crate::tuning::get(&crate::tuning::COMMUNITY_DAYS) * 86_400
}

/// The converted packs, each checked whole as it is loaded (a file changed by hand that no
/// longer passes is left out). `taken`: the ids Sprawl's and the user's packs use.
pub fn load(taken: &mut HashSet<String>) -> Vec<RulePack> {
    let Some(d) = dir() else { return Vec::new() };
    load_in(&d, taken)
}

fn load_in(d: &Path, taken: &mut HashSet<String>) -> Vec<RulePack> {
    let never = crate::safety::Never::default_list();
    let mut out = Vec::new();
    for (file, prefix) in [("winapp2.toml", WINAPP2), ("bleachbit.toml", BLEACHBIT)] {
        let p = d.join(file);
        let Ok(m) = std::fs::metadata(&p) else { continue };
        if !m.is_file() || m.len() > 16 << 20 {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        // against a copy: a check that fails part-way has already taken the ids it passed
        let mut t = taken.clone();
        match crate::memory::check_import_text(&text, prefix, &never, &mut t) {
            Ok(pack) => {
                *taken = t;
                out.push(pack);
            }
            // a file converted under older checks (a part a newer Sprawl refuses): its cleaners
            // one by one, as a fetch checks them, so one refused part doesn't take every other
            // community cleaner with it. Only cleaners are kept, each passing the checks whole.
            Err(_) => {
                // read without the parse's own checks: a newer refusal there would lose the pack
                let Ok(all) = toml::from_str::<RulePack>(text.trim_start_matches('\u{feff}')) else { continue };
                let mut pack = RulePack::empty(&all.pack.name);
                for c in all.cleaners {
                    let mut one = RulePack::empty(&all.pack.name);
                    one.cleaners.push(c.clone());
                    // its ids are taken only when it passes
                    let mut t = taken.clone();
                    let ok = toml::to_string(&one).map_err(|e| e.to_string()).and_then(|x| crate::memory::check_import_text(&x, prefix, &never, &mut t));
                    if ok.is_ok() {
                        *taken = t;
                        pack.cleaners.push(c);
                    }
                }
                if !pack.cleaners.is_empty() {
                    if let Ok(p) = toml::to_string(&pack).map_err(|e| e.to_string()).and_then(|t| RulePack::parse(&t)) {
                        out.push(RulePack { trusted: false, ..p });
                    }
                }
            }
        }
    }
    out
}

/// Bind approval to this exact definition. Legacy short fingerprints intentionally expire.
pub fn fingerprint(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(text.as_bytes()))
}

// ------------------------------------------------------------------ fetching

fn fetch(url: &str, max: usize) -> Result<String, String> {
    let r = crate::llm::http::get(url, &[("User-Agent", "Sprawl".into()), ("Accept", "*/*".into())], Duration::from_secs(60))?;
    if r.status != 200 {
        return Err(format!("{url} answered HTTP {}", r.status));
    }
    if r.body.len() > max {
        return Err(format!("{url} is bigger than expected"));
    }
    Ok(r.body)
}

/// Fetch both sources, convert them and keep the result. A source that can't be fetched keeps
/// its earlier import. Nothing about this PC is sent: two plain GETs of public files (and one
/// per CleanerML file).
pub fn update() -> Status {
    let d = dir();
    let mut st = status();
    let mut sources = Vec::new();
    let taken = taken_ids();
    // winapp2.ini
    let mut s = SourceStatus {
        name: "winapp2.ini".into(),
        licence: "CC BY-SA 4.0".into(),
        url: "https://github.com/MoscaDotTo/Winapp2".into(),
        ..Default::default()
    };
    match fetch(WINAPP2_URL, MAX_INI) {
        Ok(text) => {
            let (cleaners, mut left) = convert_winapp2(&text);
            let (pack, more, n) = finish("winapp2.ini (CC BY-SA 4.0, github.com/MoscaDotTo/Winapp2)", WINAPP2, cleaners, &taken);
            left.extend(more);
            s.imported = n;
            s.left_out = left;
            if let Err(e) = keep(d.as_deref(), "winapp2.toml", &pack) {
                s.error = e;
            }
        }
        Err(e) => {
            s.error = e;
            keep_old(&st, &mut s);
        }
    }
    sources.push(s);
    // BleachBit's CleanerML, the release folder
    let mut s = SourceStatus {
        name: "BleachBit CleanerML".into(),
        licence: "GPL-3.0-or-later".into(),
        url: "https://github.com/bleachbit/cleanerml".into(),
        ..Default::default()
    };
    match fetch_cleanerml() {
        Ok(files) => {
            let mut cleaners = Vec::new();
            let mut left = Vec::new();
            for (name, text) in files {
                let (c, l) = convert_cleanerml(&name, &text);
                cleaners.extend(c);
                left.extend(l);
            }
            let (pack, more, n) = finish("BleachBit CleanerML (GPL-3.0-or-later, github.com/bleachbit/cleanerml)", BLEACHBIT, cleaners, &taken);
            left.extend(more);
            s.imported = n;
            s.left_out = left;
            if let Err(e) = keep(d.as_deref(), "bleachbit.toml", &pack) {
                s.error = e;
            }
        }
        Err(e) => {
            s.error = e;
            keep_old(&st, &mut s);
        }
    }
    sources.push(s);
    st.fetched = now();
    st.sources = sources;
    if let (Some(d), Ok(t)) = (d, toml::to_string(&st)) {
        let _ = std::fs::create_dir_all(&d);
        let _ = crate::memory::write_atomic(&d.join("status.toml"), &t);
    }
    st
}

fn keep_old(old: &Status, s: &mut SourceStatus) {
    if let Some(o) = old.sources.iter().find(|o| o.name == s.name) {
        s.imported = o.imported;
        s.left_out = o.left_out.clone();
    }
}

fn keep(d: Option<&Path>, file: &str, text: &str) -> Result<(), String> {
    let d = d.ok_or("APPDATA is not set")?;
    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    crate::memory::write_atomic(&d.join(file), text).map_err(|e| e.to_string())
}

fn fetch_cleanerml() -> Result<Vec<(String, String)>, String> {
    let list: serde_json::Value = serde_json::from_str(&fetch(CLEANERML_LIST, 1 << 20)?).map_err(|e| format!("the CleanerML list: {e}"))?;
    let names: Vec<String> = list
        .as_array()
        .ok_or("the CleanerML list isn't a list")?
        .iter()
        .filter_map(|v| v["name"].as_str())
        // a plain file name: the address is Sprawl's own, never one from the answer
        .filter(|n| n.ends_with(".xml") && n.len() <= 80 && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
        .take(MAX_FILES)
        .map(String::from)
        .collect();
    let mut out = Vec::new();
    for n in names {
        if let Ok(t) = fetch(&format!("{CLEANERML_RAW}{n}"), MAX_XML) {
            out.push((n, t));
        }
    }
    if out.is_empty() {
        return Err("no CleanerML file could be fetched".into());
    }
    Ok(out)
}

/// The ids Sprawl's own and the user's packs use (community ids can't take them over).
fn taken_ids() -> HashSet<String> {
    let builtin = RulePack::builtin();
    let mut taken: HashSet<String> = builtin.all_rules().map(|r| r.id.to_lowercase()).collect();
    taken.extend(builtin.cleaners.iter().map(|c| format!("cleaner:{}", c.id.to_lowercase())));
    for p in crate::memory::local_packs() {
        taken.extend(p.all_rules().map(|r| r.id.to_lowercase()));
        taken.extend(p.cleaners.iter().map(|c| format!("cleaner:{}", c.id.to_lowercase())));
    }
    taken
}

/// Check each converted cleaner on its own (one that fails is left out with the reason) and
/// write the rest as one pack. Returns (the pack's text, what was left out, how many were kept).
fn finish(pack_name: &str, prefix: &str, cleaners: Vec<Cleaner>, taken: &HashSet<String>) -> (String, Vec<LeftOut>, usize) {
    let never = crate::safety::Never::default_list();
    let mut taken = taken.clone();
    let mut pack = RulePack::empty(pack_name);
    let mut left = Vec::new();
    for c in cleaners {
        let mut one = RulePack::empty(pack_name);
        one.cleaners.push(c.clone());
        let checked = toml::to_string(&one).map_err(|e| e.to_string()).and_then(|t| crate::memory::check_import_text(&t, prefix, &never, &mut taken));
        match checked {
            Ok(_) => pack.cleaners.push(c),
            Err(e) => left.push(LeftOut { entry: c.name.clone(), why: e }),
        }
    }
    let n = pack.cleaners.len();
    (toml::to_string(&pack).unwrap_or_default(), left, n)
}

// ------------------------------------------------------------------ shared conversion

/// An id made of the letters and digits of `name`, with `prefix`, at most 60 characters.
fn slug(prefix: &str, name: &str, seen: &mut HashSet<String>) -> String {
    let mut s = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s: String = s.trim_end_matches('-').chars().take(50).collect();
    let base = format!("{prefix}{}", if s.is_empty() { "x" } else { &s });
    let mut id = base.clone();
    let mut n = 2;
    while !seen.insert(id.clone()) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Words in an entry's name that say it is about what the user did, not about space. Those are
/// privacy cleaning, which comes only with Sprawl itself.
const PRIVATE: &[&str] = &[
    "history", "cookie", "autofill", "auto-fill", "session", "password", "recent", "mru", "login", "logon", "form data", "search", "typed", "bookmark",
    "jump list", "jumplist", "preference", "settings", "config", "saved", "favorites", "favourites", "credential", "token", "account", "chat", "message",
    "storage", "sync", "extension", "wallet", "certificate", "web app", "sandbox", "drm", "key", "signin", "sign-in", "privacy",
    "backup", "back-up", "autosave", "autoback", "save", "recover", "snapshot", "archive", "project", "document",
];

/// A path inside a browser profile (`User Data`, `Profiles`): the user's own; even its caches
/// are cleaned only by Sprawl's own cleaners.
fn in_profile(path: &str) -> bool {
    path.split('\\').any(|s| s.eq_ignore_ascii_case("User Data") || s.eq_ignore_ascii_case("Profiles"))
}

fn private_name(name: &str) -> bool {
    let l = name.to_lowercase();
    PRIVATE.iter().any(|w| l.contains(w))
}

/// A path with a source's variable turned into one of Sprawl's, or None when it uses one Sprawl's
/// rules can't name.
fn map_var(path: &str, vars: &[(&str, &str)]) -> Option<String> {
    let p = path.trim().replace('/', "\\");
    let p = p.trim_end_matches('\\');
    for (from, to) in vars {
        if p.get(..from.len()).is_some_and(|head| head.eq_ignore_ascii_case(from)) {
            let rest = &p[from.len()..];
            if rest.is_empty() || rest.starts_with('\\') {
                return Some(format!("{to}{rest}"));
            }
        }
    }
    None
}

fn part(id: String, title: String, risk: Risk, paths: Vec<String>, action: Act, pattern: Option<String>, kind: Kind, why: String) -> Part {
    Part {
        id,
        title: title.chars().take(80).collect(),
        class: Default::default(),
        risk,
        paths,
        kind,
        action: Some(action),
        pattern,
        keep_days: None,
        closed: Closed::Warn,
        cost: String::new(),
        why: why.chars().take(400).collect(),
        admin: false,
        min_bytes: None,
    }
}

fn last_name(path: &str) -> String {
    path.rsplit('\\').find(|s| !s.is_empty() && !s.contains(['*', '?'])).unwrap_or("Files").to_string()
}

// ------------------------------------------------------------------ winapp2.ini

const WINAPP2_VARS: &[(&str, &str)] = &[
    ("%LocalLowAppData%", "%USERPROFILE%\\AppData\\LocalLow"),
    ("%LocalAppData%", "%LOCALAPPDATA%"),
    ("%AppData%", "%APPDATA%"),
    ("%UserProfile%", "%USERPROFILE%"),
    ("%ProgramData%", "%ProgramData%"),
    ("%CommonAppData%", "%ProgramData%"),
    ("%SystemDrive%", "%SystemDrive%"),
    ("%Public%", "%PUBLIC%"),
    ("%Temp%", "%TEMP%"),
];

fn winapp2_group(section: &str, lang: &str) -> &'static str {
    let s = section.to_lowercase();
    if s.contains("browser") || s.contains("chrome") || s.contains("firefox") || s.contains("edge") || matches!(lang, "3026" | "3027" | "3029" | "3030") {
        "Browsers"
    } else if s == "games" {
        "Games"
    } else if lang == "3023" {
        "Media"
    } else if lang == "3025" {
        "Windows"
    } else {
        "Other"
    }
}

/// The values of key `k`, or of `k` with a number after it (FileKey1, FileKey2...).
fn ini_keys<'a>(keys: &'a [(String, String)], k: &'a str) -> impl Iterator<Item = &'a (String, String)> + 'a {
    keys.iter().filter(move |(n, _)| n.eq_ignore_ascii_case(k) || (n.len() > k.len() && n.is_char_boundary(k.len()) && n[..k.len()].eq_ignore_ascii_case(k) && n[k.len()..].chars().all(|c| c.is_ascii_digit())))
}

/// winapp2.ini as cleaners (unchecked yet), and the entries left out.
pub fn convert_winapp2(text: &str) -> (Vec<Cleaner>, Vec<LeftOut>) {
    let mut out = Vec::new();
    let mut left = Vec::new();
    let mut seen = HashSet::new();
    let mut entries: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines() {
        let l = line.trim().trim_start_matches('\u{feff}');
        if l.is_empty() || l.starts_with(';') {
            continue;
        }
        if let Some(name) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            entries.push((name.trim().trim_end_matches('*').trim().to_string(), Vec::new()));
        } else if let (Some((k, v)), Some(e)) = (l.split_once('='), entries.last_mut()) {
            e.1.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    for (name, keys) in entries {
        let mut lo = |why: &str| left.push(LeftOut { entry: name.clone(), why: why.into() });
        if name.is_empty() {
            continue;
        }
        if private_name(&name) {
            lo("about what you did (history, sign-ins, settings): privacy cleaning comes only with Sprawl itself");
            continue;
        }
        if ini_keys(&keys, "ExcludeKey").next().is_some() {
            lo("it keeps some files inside (ExcludeKey), which Sprawl can't express");
            continue;
        }
        let files: Vec<&String> = ini_keys(&keys, "FileKey").map(|(_, v)| v).collect();
        if files.is_empty() {
            if ini_keys(&keys, "RegKey").next().is_some() {
                lo("it only changes the registry, which Sprawl doesn't do");
            }
            continue;
        }
        let warning: Option<String> = ini_keys(&keys, "Warning").map(|(_, v)| v.clone()).next();
        let risk = if warning.is_some() { Risk::Orange } else { Risk::Amber };
        let why = match &warning {
            Some(w) => format!("From winapp2.ini ({name}). Its warning: {w}"),
            None => format!("From winapp2.ini ({name})."),
        };
        let mut parts: Vec<Part> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        for f in files {
            let mut bits = f.split('|');
            let raw = bits.next().unwrap_or("");
            let patterns = bits.next().unwrap_or("*");
            let flags: Vec<String> = bits.map(|b| b.trim().to_uppercase()).collect();
            let Some(path) = map_var(raw, WINAPP2_VARS) else {
                notes.push(format!("{raw}: a folder Sprawl's rules can't name"));
                continue;
            };
            if in_profile(&path) {
                notes.push(format!("{raw}: inside a browser profile, which only Sprawl's own cleaners touch"));
                continue;
            }
            if private_name(&last_name(&path)) {
                notes.push(format!("{raw}: looks like your own data (backups, history, settings)"));
                continue;
            }
            for pat in patterns.split(';').map(str::trim).filter(|p| !p.is_empty()) {
                if pat.contains(['\\', '/']) {
                    notes.push(format!("{raw}|{pat}: a pattern with a folder in it"));
                    continue;
                }
                let all = pat == "*" || pat == "*.*";
                let (act, pattern) = if flags.iter().any(|f| f == "REMOVESELF") {
                    if all {
                        (Act::Remove, None)
                    } else {
                        (Act::Match, Some(pat.to_string()))
                    }
                } else if flags.iter().any(|f| f == "RECURSE") {
                    (Act::Match, Some(if all { "*".to_string() } else { pat.to_string() }))
                } else {
                    notes.push(format!("{raw}|{pat}: only the folder's own files, not its subfolders, which Sprawl can't express"));
                    continue;
                };
                let title = if all { last_name(&path) } else { format!("{} ({pat})", last_name(&path)) };
                parts.push(part(format!("k{}", parts.len() + 1), title, risk, vec![path.clone()], act, pattern, Kind::Dir, why.clone()));
            }
        }
        if parts.is_empty() {
            lo(&notes.first().cloned().unwrap_or_else(|| "nothing Sprawl can express".into()));
            continue;
        }
        if parts.len() > 40 {
            lo("more than 40 parts");
            continue;
        }
        for n in notes {
            left.push(LeftOut { entry: name.clone(), why: format!("part left out: {n}") });
        }
        let section = ini_keys(&keys, "Section").map(|(_, v)| v.as_str()).next().unwrap_or("");
        let lang = ini_keys(&keys, "LangSecRef").map(|(_, v)| v.as_str()).next().unwrap_or("");
        let present: Vec<String> = ini_keys(&keys, "DetectFile").filter_map(|(_, v)| map_var(v, WINAPP2_VARS)).take(8).collect();
        out.push(Cleaner {
            id: slug(WINAPP2, &name, &mut seen),
            name: name.chars().take(60).collect(),
            group: winapp2_group(section, lang).into(),
            present,
            running: Vec::new(),
            rules: Vec::new(),
            parts,
        });
    }
    (out, left)
}

// ------------------------------------------------------------------ CleanerML

/// One XML element: enough of XML for CleanerML (no namespaces, no DTD).
#[derive(Debug, Default)]
struct El {
    name: String,
    attrs: Vec<(String, String)>,
    kids: Vec<El>,
    text: String,
}

impl El {
    fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
    fn kids<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.kids.iter().filter(move |k| k.name == name)
    }
    fn kid_text(&self, name: &str) -> String {
        self.kids(name).next().map(|k| k.text.trim().to_string()).unwrap_or_default()
    }
    /// Meant for Windows (no `os`, or `windows`).
    fn windows(&self) -> bool {
        self.attr("os").is_none_or(|o| o.eq_ignore_ascii_case("windows"))
    }
}

fn unescape(s: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let end = rest[i..].find(';').ok_or("a broken entity")? + i;
        let ent = &rest[i + 1..end];
        let c = match ent {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32).ok_or("a broken entity")?,
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32).ok_or("a broken entity")?,
            _ => return Err(format!("an unknown entity &{ent};")),
        };
        out.push(c);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn parse_xml(s: &str) -> Result<El, String> {
    let s = s.trim_start_matches('\u{feff}');
    let mut stack: Vec<El> = vec![El::default()];
    let mut i = 0;
    let b = s.as_bytes();
    while i < s.len() {
        if b[i] != b'<' {
            let end = s[i..].find('<').map_or(s.len(), |j| i + j);
            let t = unescape(&s[i..end])?;
            stack.last_mut().ok_or("broken")?.text.push_str(&t);
            i = end;
            continue;
        }
        let r = &s[i..];
        if r.starts_with("<!--") {
            i += r.find("-->").ok_or("an unclosed comment")? + 3;
        } else if r.starts_with("<![CDATA[") {
            let end = r.find("]]>").ok_or("an unclosed CDATA")?;
            stack.last_mut().ok_or("broken")?.text.push_str(&r[9..end]);
            i += end + 3;
        } else if r.starts_with("<?") || r.starts_with("<!") {
            i += r.find('>').ok_or("an unclosed declaration")? + 1;
        } else if let Some(r) = r.strip_prefix("</") {
            let end = r.find('>').ok_or("an unclosed tag")?;
            let name = r[..end].trim();
            let el = stack.pop().ok_or("broken")?;
            if el.name != name || stack.is_empty() {
                return Err(format!("</{name}> doesn't close <{}>", el.name));
            }
            stack.last_mut().ok_or("broken")?.kids.push(el);
            i += end + 3;
        } else {
            // a start tag: the name, then attributes up to > or />
            let mut j = 1;
            let rb = r.as_bytes();
            while j < r.len() && !rb[j].is_ascii_whitespace() && rb[j] != b'>' && rb[j] != b'/' {
                j += 1;
            }
            let mut el = El { name: r[1..j].to_string(), ..Default::default() };
            if el.name.is_empty() {
                return Err("an empty tag".into());
            }
            loop {
                while j < r.len() && rb[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= r.len() {
                    return Err("an unclosed tag".into());
                }
                if r[j..].starts_with("/>") {
                    stack.last_mut().ok_or("broken")?.kids.push(el);
                    j += 2;
                    break;
                }
                if rb[j] == b'>' {
                    if stack.len() > 32 {
                        return Err("nested too deep".into());
                    }
                    stack.push(el);
                    j += 1;
                    break;
                }
                let eq = r[j..].find('=').ok_or("an attribute without a value")? + j;
                let name = r[j..eq].trim().to_string();
                let mut k = eq + 1;
                while k < r.len() && rb[k].is_ascii_whitespace() {
                    k += 1;
                }
                let q = *rb.get(k).ok_or("an unclosed tag")?;
                if q != b'"' && q != b'\'' {
                    return Err("an attribute value without quotes".into());
                }
                let close = r[k + 1..].find(q as char).ok_or("an unclosed attribute")? + k + 1;
                el.attrs.push((name, unescape(&r[k + 1..close])?));
                j = close + 1;
            }
            i += j;
        }
    }
    let mut root = stack.pop().ok_or("broken")?;
    if !stack.is_empty() {
        return Err("an unclosed element".into());
    }
    root.kids.pop().filter(|_| root.kids.is_empty()).ok_or_else(|| "not one root element".into())
}

const CLEANERML_VARS: &[(&str, &str)] = &[
    ("$localappdatalow", "%USERPROFILE%\\AppData\\LocalLow"),
    ("$localappdata", "%LOCALAPPDATA%"),
    ("$appdata", "%APPDATA%"),
    ("$userprofile", "%USERPROFILE%"),
    ("$programdata", "%ProgramData%"),
    ("$allusersprofile", "%ProgramData%"),
    ("$systemdrive", "%SystemDrive%"),
    ("$public", "%PUBLIC%"),
    ("$temp", "%TEMP%"),
    ("$tmp", "%TEMP%"),
    ("${localappdata}", "%LOCALAPPDATA%"),
    ("${appdata}", "%APPDATA%"),
    ("${userprofile}", "%USERPROFILE%"),
    ("%localappdata%", "%LOCALAPPDATA%"),
    ("%appdata%", "%APPDATA%"),
    ("%userprofile%", "%USERPROFILE%"),
    ("%programdata%", "%ProgramData%"),
    ("%temp%", "%TEMP%"),
];

/// One CleanerML file as a cleaner (unchecked yet), and what was left out.
pub fn convert_cleanerml(file: &str, text: &str) -> (Option<Cleaner>, Vec<LeftOut>) {
    let mut left = Vec::new();
    let root = match parse_xml(text) {
        Ok(r) if r.name == "cleaner" => r,
        Ok(_) => return (None, vec![LeftOut { entry: file.into(), why: "not a cleaner".into() }]),
        Err(e) => return (None, vec![LeftOut { entry: file.into(), why: format!("doesn't read as XML: {e}") }]),
    };
    let name: String = root.kid_text("label").chars().take(60).collect();
    let name = if name.is_empty() { file.trim_end_matches(".xml").to_string() } else { name };
    if !root.windows() {
        // a cleaner for another system: nothing to say
        return (None, left);
    }
    let id_src = root.attr("id").unwrap_or(file.trim_end_matches(".xml"));
    // the variables, with their Windows values
    let vars: Vec<(String, Vec<String>)> = root.kids("var").filter_map(|v| Some((v.attr("name")?.to_string(), v.kids("value").filter(|x| x.windows()).map(|x| x.text.trim().to_string()).collect()))).collect();
    let running: Vec<String> = root
        .kids("running")
        .filter(|r| r.windows() && r.attr("type").is_none_or(|t| t == "exe"))
        .map(|r| r.text.trim().to_string())
        .filter(|r| r.to_lowercase().ends_with(".exe") && !r.contains(['\\', '/', '*', '?', ':']) && r.len() <= 64)
        .take(8)
        .collect();
    let mut parts: Vec<Part> = Vec::new();
    for opt in root.kids("option").filter(|o| o.windows()) {
        let label = opt.kid_text("label");
        let oid: String = opt.attr("id").unwrap_or("part").chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(30).collect();
        let what = format!("{name}: {label}");
        if private_name(&label) || private_name(&oid) {
            left.push(LeftOut { entry: what, why: "about what you did (history, sign-ins, settings): privacy cleaning comes only with Sprawl itself".into() });
            continue;
        }
        let warning = opt.kid_text("warning");
        let risk = if warning.is_empty() { Risk::Amber } else { Risk::Orange };
        let mut why = format!("From BleachBit's CleanerML ({name}). {}", opt.kid_text("description"));
        if !warning.is_empty() {
            why += &format!(" Its warning: {warning}");
        }
        // actions with the same effect share a part
        let mut groups: Vec<(Act, Option<String>, Kind, Vec<String>)> = Vec::new();
        for a in opt.kids("action").filter(|a| a.windows()) {
            let cmd = a.attr("command").unwrap_or("");
            if cmd != "delete" {
                left.push(LeftOut { entry: what.clone(), why: format!("a \"{cmd}\" action, which Sprawl doesn't do") });
                continue;
            }
            if a.attrs.iter().any(|(k, _)| matches!(k.as_str(), "regex" | "nregex" | "wholeregex" | "nwholeregex" | "type")) {
                left.push(LeftOut { entry: what.clone(), why: "a filter by regular expression or type, which Sprawl can't express".into() });
                continue;
            }
            let Some(raw) = a.attr("path") else { continue };
            // a path for another system
            if raw.starts_with('~') || raw.starts_with('/') {
                continue;
            }
            let search = a.attr("search").unwrap_or("file");
            // BleachBit reads `*` and `?` literally outside a glob; Sprawl would match with them
            if search != "glob" && raw.contains(['*', '?']) {
                left.push(LeftOut { entry: what.clone(), why: "a path with * or ? that BleachBit reads as a plain name".into() });
                continue;
            }
            let (act, pattern, kind) = match search {
                "walk.files" => (Act::Match, Some("*".to_string()), Kind::Dir),
                "walk.all" => (Act::Clear, None, Kind::Dir),
                "walk.top" => (Act::Remove, None, Kind::Dir),
                // BleachBit removes a file there, and a folder only when it is empty: never a
                // whole folder with what is in it
                "file" => (Act::Remove, None, Kind::File),
                "glob" if !raw.contains(['*', '?']) => (Act::Remove, None, Kind::File),
                other => {
                    left.push(LeftOut { entry: what.clone(), why: format!("a \"{other}\" search, which Sprawl can't match the same way") });
                    continue;
                }
            };
            // $$var$$ stands for each of its values
            let mut paths = vec![raw.to_string()];
            let mut too_many = false;
            for (n, vals) in &vars {
                let tag = format!("$${n}$$");
                // many-valued variables multiply: a hostile file could fill the memory, so the
                // count is checked before the paths are made
                let tagged = paths.iter().filter(|p| p.contains(&tag)).count();
                if (paths.len() - tagged).saturating_add(tagged.saturating_mul(vals.len())) > 64 {
                    too_many = true;
                    break;
                }
                paths = paths.into_iter().flat_map(|p| if p.contains(&tag) { vals.iter().map(|v| p.replace(&tag, v)).collect() } else { vec![p] }).collect();
            }
            if too_many {
                left.push(LeftOut { entry: what.clone(), why: "more than 64 folders from one action".into() });
                continue;
            }
            for p in paths {
                if p.contains("$$") {
                    continue;
                }
                let Some(mapped) = map_var(&p, CLEANERML_VARS) else {
                    left.push(LeftOut { entry: what.clone(), why: format!("{p}: a folder Sprawl's rules can't name") });
                    continue;
                };
                if in_profile(&mapped) {
                    left.push(LeftOut { entry: what.clone(), why: format!("{p}: inside a browser profile, which only Sprawl's own cleaners touch") });
                    continue;
                }
                if private_name(&last_name(&mapped)) {
                    left.push(LeftOut { entry: what.clone(), why: format!("{p}: looks like your own data (backups, history, settings)") });
                    continue;
                }
                match groups.iter_mut().find(|g| g.0 == act && g.1 == pattern && g.2 == kind && g.3.len() < 8) {
                    Some(g) => g.3.push(mapped),
                    None => groups.push((act, pattern.clone(), kind, vec![mapped])),
                }
            }
        }
        for (k, (act, pattern, kind, paths)) in groups.into_iter().enumerate() {
            let id = if k == 0 { oid.clone() } else { format!("{oid}-{}", k + 1) };
            let id = if id.is_empty() { format!("p{}", parts.len() + 1) } else { id };
            parts.push(part(id, if label.is_empty() { last_name(&paths[0]) } else { label.clone() }, risk, paths, act, pattern, kind, why.clone()));
        }
    }
    if parts.is_empty() {
        return (None, left);
    }
    parts.truncate(40);
    let mut seen = HashSet::new();
    let c = Cleaner { id: slug(BLEACHBIT, id_src, &mut seen), name, group: "Other".into(), present: Vec::new(), running, rules: Vec::new(), parts };
    (Some(c), left)
}

#[cfg(test)]
mod tests {
    #[test]
    fn approvals_use_sha256_and_expire_legacy_short_hashes() {
        assert_eq!(super::fingerprint("abc"), "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_ne!(super::fingerprint("abc"), super::fingerprint("abcd"));
        assert_ne!(super::fingerprint("abc"), "e71fa2190541574b");
        assert!(super::is_community("WINAPP2.changed/part"));
        assert!(super::is_community("BleachBit.changed"));
    }
    use super::*;

    fn env() {
        std::env::set_var("SystemDrive", "C:");
        std::env::set_var("SystemRoot", r"C:\Windows");
        std::env::set_var("LOCALAPPDATA", r"C:\Users\sam\AppData\Local");
        std::env::set_var("APPDATA", r"C:\Users\sam\AppData\Roaming");
        std::env::set_var("USERPROFILE", r"C:\Users\sam");
        std::env::set_var("ProgramData", r"C:\ProgramData");
    }

    /// Entries written for these tests in winapp2's format (not copied from the file).
    const INI: &str = "; a comment\r\n\
[Tool A Cache *]\r\nLangSecRef=3021\r\nDetectFile=%LocalAppData%\\ToolA\r\nFileKey1=%LocalAppData%\\ToolA\\Cache|*.*|REMOVESELF\r\nFileKey2=%LocalAppData%\\ToolA\\logs|*.log;*.txt|RECURSE\r\nFileKey3=%LocalAppData%\\ToolA|*.tmp\r\nFileKey4=%WinDir%\\ToolA|*.*|RECURSE\r\n\r\n\
[Tool B History *]\r\nFileKey1=%AppData%\\ToolB\\History|*.*|REMOVESELF\r\n\r\n\
[Tool C Registry *]\r\nRegKey1=HKCU\\Software\\ToolC\\MRU\r\n\r\n\
[Tool D Logs *]\r\nExcludeKey1=FILE|%AppData%\\ToolD\\|keep.log\r\nFileKey1=%AppData%\\ToolD|*.log|RECURSE\r\n\r\n\
[Tool E Temp *]\r\nSection=Games\r\nWarning=Saved replays live here too.\r\nFileKey1=%AppData%\\ToolE\\temp|*.*|RECURSE\r\n\r\n\
[Chromium Bits *]\r\nFileKey1=%LocalAppData%\\Chromium\\User Data\\*\\GPUCache|*.*|REMOVESELF\r\n\r\n\
[Bad Paths *]\r\nFileKey1=%UserProfile%\\Documents|*.*|RECURSE\r\nFileKey2=%LocalAppData%\\..\\..\\x|*.*|RECURSE\r\nFileKey3=%LocalAppData%\\Bad*|*.*|REMOVESELF\r\n";

    #[test]
    fn winapp2_converts_what_sprawl_can_say() {
        env();
        let (cleaners, left) = convert_winapp2(INI);
        let a = cleaners.iter().find(|c| c.name == "Tool A Cache").expect("Tool A");
        assert_eq!(a.id, "winapp2.tool-a-cache");
        assert_eq!(a.present, vec![r"%LOCALAPPDATA%\ToolA"]);
        assert_eq!(a.parts.len(), 3, "{:?}", a.parts);
        assert_eq!((a.parts[0].action, a.parts[0].pattern.as_deref()), (Some(Act::Remove), None));
        assert_eq!((a.parts[1].action, a.parts[1].pattern.as_deref()), (Some(Act::Match), Some("*.log")));
        assert_eq!(a.parts[2].pattern.as_deref(), Some("*.txt"));
        assert!(a.parts.iter().all(|p| p.risk == Risk::Amber));
        assert!(left.iter().any(|l| l.entry == "Tool A Cache" && l.why.contains("only the folder's own files")));
        assert!(left.iter().any(|l| l.entry == "Tool A Cache" && l.why.contains("%WinDir%")));
        assert!(left.iter().any(|l| l.entry == "Tool B History" && l.why.contains("privacy")));
        assert!(left.iter().any(|l| l.entry == "Tool C Registry" && l.why.contains("registry")));
        assert!(left.iter().any(|l| l.entry == "Tool D Logs" && l.why.contains("ExcludeKey")));
        assert!(left.iter().any(|l| l.entry == "Chromium Bits" && l.why.contains("browser profile")), "{left:?}");
        let e = cleaners.iter().find(|c| c.name == "Tool E Temp").unwrap();
        assert_eq!(e.group, "Games");
        assert_eq!(e.parts[0].risk, Risk::Orange, "a warning makes it orange");
        assert!(e.parts[0].why.contains("Saved replays"));
        // the checks take out what the converter let through
        let (text, more, n) = finish("x", WINAPP2, cleaners, &HashSet::new());
        assert_eq!(n, 2, "{more:?}");
        assert!(more.iter().any(|l| l.entry == "Bad Paths"), "{more:?}");
        let pack = crate::memory::check_import_text(&text, WINAPP2, &crate::safety::Never::default_list(), &mut HashSet::new()).unwrap();
        assert_eq!(pack.cleaners.len(), 2);
        assert!(pack.part_rules.iter().all(|r| r.id.starts_with(WINAPP2)));
    }

    /// A CleanerML file written for this test in BleachBit's format.
    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- a comment with <tags> -->
<cleaner id="toolx">
  <label>Tool &amp; X</label>
  <running type="exe" os="windows">ToolX.exe</running>
  <running type="exe" os="linux">toolx</running>
  <var name="base"><value os="windows">%LocalAppData%\ToolX</value><value os="linux">~/.toolx</value></var>
  <option id="cache">
    <label>Cache</label>
    <description>Pictures it downloads again</description>
    <action command="delete" search="walk.all" path="$$base$$\cache\"/>
    <action command="delete" search="walk.files" path="~/.toolx/cache/"/>
    <action command="delete" search="walk.all" path="$appdata\ToolX\cache"/>
  </option>
  <option id="logs">
    <label>Logs</label>
    <description>Old logs</description>
    <action command="delete" search="walk.files" path="$$base$$\logs"/>
    <action command="delete" search="glob" path="$$base$$\*.log"/>
    <action command="winreg" path="HKCU\Software\ToolX" name="Last"/>
  </option>
  <option id="history">
    <label>History</label>
    <description>What you opened</description>
    <action command="delete" search="file" path="$$base$$\history.db"/>
  </option>
</cleaner>"#;

    #[test]
    fn cleanerml_converts_what_sprawl_can_say() {
        env();
        let (c, left) = convert_cleanerml("toolx.xml", XML);
        let c = c.expect("a cleaner");
        assert_eq!(c.id, "bleachbit.toolx");
        assert_eq!(c.name, "Tool & X");
        assert_eq!(c.running, vec!["ToolX.exe"]);
        assert_eq!(c.parts.len(), 2, "{:?}", c.parts);
        assert_eq!(c.parts[0].action, Some(Act::Clear));
        assert_eq!(c.parts[0].paths, vec![r"%LOCALAPPDATA%\ToolX\cache", r"%APPDATA%\ToolX\cache"]);
        assert_eq!((c.parts[1].action, c.parts[1].pattern.as_deref()), (Some(Act::Match), Some("*")));
        assert!(left.iter().any(|l| l.why.contains("\"glob\"")), "{left:?}");
        assert!(left.iter().any(|l| l.why.contains("winreg")));
        assert!(left.iter().any(|l| l.entry.ends_with("History") && l.why.contains("privacy")));
        let (_, more, n) = finish("x", BLEACHBIT, vec![c], &HashSet::new());
        assert_eq!(n, 1, "{more:?}");
        // other systems, and broken files
        assert!(convert_cleanerml("l.xml", "<cleaner id='l' os='linux'><label>L</label></cleaner>").0.is_none());
        for bad in ["<cleaner>", "<cleaner id='x'><label>a</lab></cleaner>", "<a/><b/>", "<cleaner id='x'>&bogus;</cleaner>", "<cleaner id=x/>"] {
            let (c, l) = convert_cleanerml("b.xml", bad);
            assert!(c.is_none() && !l.is_empty(), "{bad}");
        }
    }

    /// The real files, fetched and converted (APPDATA should point at a scratch profile).
    #[test]
    #[ignore]
    fn fetch_live() {
        let t = std::time::Instant::now();
        let st = update();
        for s in &st.sources {
            eprintln!("{}: {} imported, {} left out, error {:?}", s.name, s.imported, s.left_out.len(), s.error);
        }
        eprintln!("took {:?}", t.elapsed());
        let mut taken = HashSet::new();
        let packs = load(&mut taken);
        eprintln!("loaded {} packs, {} cleaners", packs.len(), packs.iter().map(|p| p.cleaners.len()).sum::<usize>());
        assert!(st.sources.iter().all(|s| s.error.is_empty()));
    }

    /// A stored pack converted under older checks: a cleaner a newer Sprawl refuses is left out
    /// alone, the others still load.
    #[test]
    fn a_stored_pack_loses_only_its_refused_cleaner() {
        env();
        let d = std::env::temp_dir().join(format!("sprawl-comm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let one = |id: &str, path: &str| format!("[[cleaner]]\nid = '{id}'\nname = 'A'\n[[cleaner.part]]\nid = 'k1'\ntitle = 'Cache'\nrisk = 'amber'\npaths = ['{path}']\nwhy = 'w'\n");
        let text = format!("[pack]\nname = 'x'\n{}{}", one("winapp2.ok", r"%LOCALAPPDATA%\A\Cache"), one("winapp2.bad", r"%USERPROFILE%\Documents\Work"));
        std::fs::write(d.join("winapp2.toml"), text).unwrap();
        let packs = load_in(&d, &mut HashSet::new());
        let ids: Vec<&str> = packs.iter().flat_map(|p| p.cleaners.iter().map(|c| c.id.as_str())).collect();
        assert_eq!(ids, ["winapp2.ok"]);
        assert!(packs.iter().all(|p| !p.trusted && p.rules.is_empty() && !p.part_rules.is_empty()));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A pack that claims another source's prefix, a green part, a privacy part or a plain rule
    /// is refused whole when loaded.
    #[test]
    fn hand_edited_imports_are_refused() {
        env();
        let never = crate::safety::Never::default_list();
        let good = "[pack]\nname = 'x'\n[[cleaner]]\nid = 'winapp2.a'\nname = 'A'\n[[cleaner.part]]\nid = 'k1'\ntitle = 'Cache'\nrisk = 'amber'\npaths = ['%LOCALAPPDATA%\\A\\Cache']\nwhy = 'w'\n";
        assert!(crate::memory::check_import_text(good, WINAPP2, &never, &mut HashSet::new()).is_ok());
        for bad in [
            good.replace("winapp2.a", "bleachbit.a"),
            good.replace("'amber'", "'green'"),
            good.replace("why = 'w'", "why = 'w'\nclass = 'privacy'\nclosed = 'must'"),
            format!("{good}[[rule]]\nid = 'winapp2.r'\ntitle = 'R'\nwhy = 'w'\nrisk = 'amber'\nremedy = 'delete'\npaths = ['%LOCALAPPDATA%\\A\\B']\n"),
            // the user's own folders
            good.replace(r"%LOCALAPPDATA%\A\Cache", r"%USERPROFILE%\Documents\Work"),
            good.replace(r"%LOCALAPPDATA%\A\Cache", r"%USERPROFILE%\Pictures\2024"),
            good.replace(r"%LOCALAPPDATA%\A\Cache", r"%USERPROFILE%\D?cuments\Work\x"),
            good.replace(r"%LOCALAPPDATA%\A\Cache", r"%USERPROFILE%\Doc*\A\B"),
            good.replace(r"%LOCALAPPDATA%\A\Cache", r"C:\Users\**\Work\x"),
            // a part that runs as administrator
            good.replace("why = 'w'", "why = 'w'\nadmin = true"),
        ] {
            assert!(crate::memory::check_import_text(&bad, WINAPP2, &never, &mut HashSet::new()).is_err(), "{bad}");
        }
        // a local pack can't use a community prefix
        assert!(crate::memory::check_pack_text(good, &mut HashSet::new()).is_err());
    }
}
