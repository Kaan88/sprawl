//! Facts Rust can gather without a model: which app owns a folder, and when it was last used.
//!
//! Everything here only reads: the registry (Uninstall keys, Store packages, GOG, WSL,
//! UserAssist), small manifest files (Steam `.acf`/`.vdf`, Epic `.item`) and file times
//! (Prefetch). Nothing is run, and no path found here is trusted on its own: findings built from
//! these facts still go through [`crate::safety::check`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sprawl_core::{Tree, ROOT};

use crate::finding::{Finding, Remedy, Risk, Source};
use crate::{find_path, segments};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppKind {
    /// Has an entry in Settings > Apps (an Uninstall key).
    Installed,
    /// A Microsoft Store (AppX/MSIX) package.
    Store,
    Steam,
    Epic,
    Gog,
    /// A WSL Linux distribution.
    Wsl,
}

/// How a "last used" time was learned. Shown with the date so the user can judge it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsedFrom {
    /// Steam's own LastPlayed.
    Steam,
    /// `C:\Windows\Prefetch` (written when a program starts; needs admin to read).
    Prefetch,
    /// Explorer's UserAssist record of programs started from the Start menu or desktop. Shows
    /// recent use, but its absence proves nothing (apps started at sign-in or by other apps
    /// aren't recorded), so it never makes an app "unused".
    UserAssist,
}

impl UsedFrom {
    pub fn label(self) -> &'static str {
        match self {
            UsedFrom::Steam => "Steam",
            UsedFrom::Prefetch => "Windows Prefetch",
            UsedFrom::UserAssist => "the Start menu history",
        }
    }
}

#[derive(Clone, Debug)]
pub struct App {
    pub name: String,
    pub publisher: Option<String>,
    pub kind: AppKind,
    /// Folders the app keeps its files in: the install folder first, then data folders.
    pub locations: Vec<PathBuf>,
    pub last_used: Option<(SystemTime, UsedFrom)>,
    /// How to remove it, in words. Sprawl never runs an uninstaller.
    pub how_to_remove: String,
}

/// The apps found on this machine, and a lookup from folder to owner.
#[derive(Default)]
pub struct Facts {
    pub apps: Vec<App>,
    /// (lower-case segments of a location, app index), longest first.
    index: Vec<(Vec<String>, usize)>,
}

impl Facts {
    /// Read everything this machine says about its apps. Takes a fraction of a second; missing
    /// sources (no Steam, no admin for Prefetch) are skipped.
    pub fn gather() -> Facts {
        let mut apps = Vec::new();
        #[cfg(windows)]
        {
            apps.extend(sources::uninstall_keys());
            apps.extend(sources::store_packages());
            apps.extend(sources::gog());
            apps.extend(sources::wsl());
        }
        apps.extend(steam::apps());
        apps.extend(epic());
        Facts::from_apps(apps)
    }

    pub fn from_apps(apps: Vec<App>) -> Facts {
        let mut index: Vec<(Vec<String>, usize)> = Vec::new();
        for (i, a) in apps.iter().enumerate() {
            for l in &a.locations {
                let segs = segments(&l.to_string_lossy());
                // A drive or a top-level folder is never one app's (an installer that says its
                // location is C:\Program Files must not claim all of it).
                if segs.len() >= 3 || (segs.len() == 2 && !TOP_LEVEL.contains(&segs[1].as_str())) {
                    index.push((segs, i));
                }
            }
        }
        // longest first; for equal locations the first source wins (Steam over its Uninstall key)
        index.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        Facts { apps, index }
    }

    /// The app whose folder holds `path`, if any (the most specific one).
    pub fn owner_of(&self, path: &Path) -> Option<&App> {
        let segs = segments(&path.to_string_lossy());
        self.index.iter().find(|(loc, _)| segs.len() >= loc.len() && segs[..loc.len()] == loc[..]).map(|&(_, i)| &self.apps[i])
    }

    /// The owning app of every node (index into `apps`), for an Owner column.
    pub fn owner_map(&self, tree: &Tree) -> Vec<Option<u32>> {
        let mut own: Vec<Option<u32>> = vec![None; tree.nodes.len()];
        // most specific last, so it overwrites the broader one
        for (loc, i) in self.index.iter().rev() {
            if let Some(n) = find_path(tree, Path::new(&loc.join("\\"))) {
                own[n] = Some(*i as u32);
            }
        }
        let mut stack = vec![(ROOT, None)];
        while let Some((n, inherited)) = stack.pop() {
            let mine = own[n].or(inherited);
            own[n] = mine;
            for &c in &tree.nodes[n].children {
                stack.push((c, mine));
            }
        }
        own
    }

    /// Fill in `last_used` from program starts (Prefetch, UserAssist), matched by the names of
    /// the programs found in each app's folder.
    pub fn attach_usage(&mut self, tree: &Tree) {
        let runs = program_runs();
        if runs.is_empty() {
            return;
        }
        for app in &mut self.apps {
            if app.last_used.is_some() {
                continue;
            }
            let mut best: Option<(SystemTime, UsedFrom)> = None;
            for l in &app.locations {
                let Some(n) = find_path(tree, l) else { continue };
                for exe in programs_in(tree, n, 4) {
                    if let Some(&(t, from)) = runs.get(&exe) {
                        if best.is_none_or(|(b, _)| t > b) {
                            best = Some((t, from));
                        }
                    }
                }
            }
            app.last_used = best;
        }
    }
}

/// Top-level folders that hold many apps, never claimed by one.
const TOP_LEVEL: &[&str] = &["program files", "program files (x86)", "programdata", "users", "windows", "games"];

/// Lower-case `.exe` names in the first `depth` levels below node `n`.
fn programs_in(tree: &Tree, n: usize, depth: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(n, 0)];
    while let Some((k, d)) = stack.pop() {
        let node = &tree.nodes[k];
        if !node.is_dir {
            if node.name.to_lowercase().ends_with(".exe") {
                out.push(node.name.to_lowercase());
            }
            continue;
        }
        if d < depth && !node.is_reparse_point() {
            stack.extend(node.children.iter().map(|&c| (c, d + 1)));
        }
    }
    out
}

/// When each program (by lower-case file name) last started, from every source available.
fn program_runs() -> HashMap<String, (SystemTime, UsedFrom)> {
    let mut out: HashMap<String, (SystemTime, UsedFrom)> = HashMap::new();
    let mut add = |name: String, t: SystemTime, from: UsedFrom| {
        let e = out.entry(name).or_insert((t, from));
        if t > e.0 {
            *e = (t, from);
        }
    };
    // Prefetch: NAME.EXE-1A2B3C4D.pf, modified when the program last started
    if let Some(dir) = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("Prefetch")) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_lowercase();
                let Some(stem) = name.strip_suffix(".pf") else { continue };
                let Some((exe, _)) = stem.rsplit_once('-') else { continue };
                if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                    add(exe.to_string(), t, UsedFrom::Prefetch);
                }
            }
        }
    }
    #[cfg(windows)]
    for (exe, t) in sources::user_assist() {
        add(exe, t, UsedFrom::UserAssist);
    }
    out
}

pub(crate) fn filetime(ft: u64) -> Option<SystemTime> {
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    let d = ft.checked_sub(EPOCH_DIFF)?;
    if d == 0 {
        return None;
    }
    UNIX_EPOCH.checked_add(Duration::new(d / 10_000_000, (d % 10_000_000) as u32 * 100))
}

/// "March 2025".
pub fn month_year(t: SystemTime) -> String {
    let days = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    format!("{} {y}", MONTHS[(m - 1) as usize])
}

/// Strip `\\?\`, quotes and a trailing backslash from a path written by an installer.
fn clean_path(s: &str) -> Option<PathBuf> {
    let s = s.trim().trim_matches('"').trim();
    let s = s.strip_prefix(r"\\?\").unwrap_or(s);
    let s = s.trim_end_matches('\\');
    let b = s.as_bytes();
    // only absolute drive paths
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\').then(|| PathBuf::from(s))
}

// ---------------------------------------------------------------------- registry sources

#[cfg(windows)]
mod sources {
    use super::*;
    use crate::reg::{Key, View, HKCU, HKLM};

    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

    pub(super) fn uninstall_keys() -> Vec<App> {
        let mut out: Vec<App> = Vec::new();
        let roots = [(HKLM, View::Native), (HKLM, View::Bit32), (HKCU, View::Native)];
        for (root, view) in roots {
            let Some(k) = Key::open(root, UNINSTALL, view) else { continue };
            for id in k.subkeys() {
                let Some(e) = k.sub(&id) else { continue };
                let Some(name) = e.string("DisplayName") else { continue };
                // components, updates and patches aren't apps
                if e.dword("SystemComponent") == Some(1) || e.string("ParentKeyName").is_some() || e.string("ReleaseType").is_some() {
                    continue;
                }
                let loc = e.string("InstallLocation").and_then(|l| clean_path(&l)).or_else(|| {
                    // no location: the folder of its icon, if that is an ordinary app folder
                    let icon = e.string("DisplayIcon")?;
                    let icon = icon.split(',').next()?;
                    let p = clean_path(icon)?;
                    let dir = p.parent()?.to_path_buf();
                    let segs = segments(&dir.to_string_lossy());
                    (segs.len() >= 3 && segs[1] != "windows" && segs[1] != "programdata").then_some(dir)
                });
                let Some(loc) = loc else { continue };
                if out.iter().any(|a| a.name == name && a.locations.first() == Some(&loc)) {
                    continue;
                }
                out.push(App {
                    how_to_remove: format!("Settings > Apps > Installed apps > {name} > Uninstall"),
                    name,
                    publisher: e.string("Publisher"),
                    kind: AppKind::Installed,
                    locations: vec![loc],
                    last_used: None,
                });
            }
        }
        out
    }

    /// Store packages: the install folder (usually under WindowsApps, unreadable without admin)
    /// and the per-user data folder `%LOCALAPPDATA%\Packages\<family>`.
    pub(super) fn store_packages() -> Vec<App> {
        let mut out: Vec<App> = Vec::new();
        let path = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
        let Some(k) = Key::open(HKCU, path, View::Native) else { return out };
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        for full in k.subkeys() {
            // Name_Version_Arch_ResourceId_PublisherId
            let parts: Vec<&str> = full.split('_').collect();
            if parts.len() < 5 {
                continue;
            }
            let (pkg, publisher_id) = (parts[0], parts[parts.len() - 1]);
            let lower = pkg.to_lowercase();
            if ["vclibs", "framework", "net.native", "ui.xaml", "windowsappruntime", "directx", "winappruntime"].iter().any(|f| lower.contains(f)) {
                continue;
            }
            let family = format!("{pkg}_{publisher_id}");
            if out.iter().any(|a| a.name == pkg || a.locations.iter().any(|l| l.ends_with(&family))) {
                continue;
            }
            let Some(e) = k.sub(&full) else { continue };
            let shown = e.string("DisplayName").filter(|d| !d.starts_with('@') && !d.starts_with("ms-resource"));
            let name = shown.unwrap_or_else(|| pkg.rsplit('.').next().unwrap_or(pkg).to_string());
            let mut locations = Vec::new();
            if let Some(root) = e.string("PackageRootFolder").and_then(|r| clean_path(&r)) {
                locations.push(root);
            }
            if let Some(l) = &local {
                locations.push(l.join("Packages").join(&family));
            }
            out.push(App {
                how_to_remove: format!("Settings > Apps > Installed apps > {name} > Uninstall"),
                name,
                publisher: None,
                kind: AppKind::Store,
                locations,
                last_used: None,
            });
        }
        out
    }

    pub(super) fn gog() -> Vec<App> {
        let mut out = Vec::new();
        let Some(k) = Key::open(HKLM, r"SOFTWARE\GOG.com\Games", View::Bit32) else { return out };
        for id in k.subkeys() {
            let Some(e) = k.sub(&id) else { continue };
            let (Some(name), Some(loc)) = (e.string("gameName"), e.string("path").and_then(|p| clean_path(&p))) else { continue };
            out.push(App {
                how_to_remove: format!("GOG Galaxy: right-click {name} > Manage installation > Uninstall (or Settings > Apps)"),
                name,
                publisher: Some("GOG".into()),
                kind: AppKind::Gog,
                locations: vec![loc],
                last_used: None,
            });
        }
        out
    }

    pub(super) fn wsl() -> Vec<App> {
        let mut out = Vec::new();
        let Some(k) = Key::open(HKCU, r"Software\Microsoft\Windows\CurrentVersion\Lxss", View::Native) else { return out };
        for id in k.subkeys() {
            let Some(e) = k.sub(&id) else { continue };
            let (Some(name), Some(loc)) = (e.string("DistributionName"), e.string("BasePath").and_then(|p| clean_path(&p))) else { continue };
            out.push(App {
                how_to_remove: format!("'wsl --unregister {name}' deletes the distribution and every file in it; back it up first with 'wsl --export {name} <file>'"),
                name: format!("{name} (WSL)"),
                publisher: None,
                kind: AppKind::Wsl,
                locations: vec![loc],
                last_used: None,
            });
        }
        out
    }

    /// UserAssist: ROT13 names of programs started from Explorer, with the last start time at
    /// byte 60 of each 72-byte record.
    pub(super) fn user_assist() -> Vec<(String, SystemTime)> {
        let mut out = Vec::new();
        let Some(k) = Key::open(HKCU, r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist", View::Native) else { return out };
        for guid in k.subkeys() {
            let Some(count) = k.sub(&guid).and_then(|g| g.sub("Count")) else { continue };
            for (name, data) in count.values() {
                if data.len() < 68 {
                    continue;
                }
                let plain: String = name.chars().map(rot13).collect();
                let file = plain.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
                if !file.ends_with(".exe") {
                    continue;
                }
                let ft = u64::from_le_bytes(data[60..68].try_into().unwrap());
                if let Some(t) = filetime(ft) {
                    out.push((file, t));
                }
            }
        }
        out
    }

    fn rot13(c: char) -> char {
        match c {
            'a'..='z' => (((c as u8 - b'a') + 13) % 26 + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A') + 13) % 26 + b'A') as char,
            _ => c,
        }
    }
}

// ---------------------------------------------------------------------- Steam

pub(crate) mod steam {
    use super::*;

    /// Quoted strings and braces of a Valve KeyValues (`.vdf`/`.acf`) file.
    pub(crate) fn tokens(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' => {
                    let mut s = String::new();
                    while let Some(c) = chars.next() {
                        match c {
                            '\\' => {
                                if let Some(n) = chars.next() {
                                    s.push(match n {
                                        'n' => '\n',
                                        't' => '\t',
                                        other => other,
                                    });
                                }
                            }
                            '"' => break,
                            c => s.push(c),
                        }
                    }
                    out.push(s);
                }
                '{' | '}' => out.push(c.to_string()),
                _ => {}
            }
            if out.len() > 1_000_000 {
                break;
            }
        }
        out
    }

    /// Every `"key" "value"` pair, keys lower-cased, first one wins.
    pub(crate) fn pairs(text: &str) -> HashMap<String, String> {
        let t = tokens(text);
        let mut out = HashMap::new();
        for w in t.windows(2) {
            if w[0] != "{" && w[0] != "}" && w[1] != "{" && w[1] != "}" {
                out.entry(w[0].to_lowercase()).or_insert_with(|| w[1].clone());
            }
        }
        out
    }

    /// The `"path"` of every Steam library.
    pub(crate) fn library_paths(text: &str) -> Vec<PathBuf> {
        let t = tokens(text);
        t.windows(2).filter(|w| w[0].eq_ignore_ascii_case("path")).filter_map(|w| clean_path(&w[1].replace('/', "\\"))).collect()
    }

    fn steam_root() -> Option<PathBuf> {
        #[cfg(windows)]
        {
            use crate::reg::{Key, View, HKCU, HKLM};
            if let Some(p) = Key::open(HKCU, r"Software\Valve\Steam", View::Native).and_then(|k| k.string("SteamPath")) {
                return clean_path(&p.replace('/', "\\"));
            }
            if let Some(p) = Key::open(HKLM, r"SOFTWARE\Valve\Steam", View::Bit32).and_then(|k| k.string("InstallPath")) {
                return clean_path(&p);
            }
        }
        None
    }

    pub(crate) fn apps() -> Vec<App> {
        let Some(root) = steam_root() else { return Vec::new() };
        let mut libs = vec![root.clone()];
        if let Ok(text) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) {
            for p in library_paths(&text) {
                if !libs.iter().any(|l| l.as_os_str().eq_ignore_ascii_case(p.as_os_str())) {
                    libs.push(p);
                }
            }
        }
        let mut out = Vec::new();
        for lib in libs {
            let apps = lib.join("steamapps");
            let Ok(rd) = std::fs::read_dir(&apps) else { continue };
            for e in rd.flatten() {
                let f = e.file_name().to_string_lossy().to_lowercase();
                if !(f.starts_with("appmanifest_") && f.ends_with(".acf")) || e.metadata().map_or(true, |m| m.len() > 1 << 20) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
                if let Some(app) = from_manifest(&apps, &text) {
                    out.push(app);
                }
            }
        }
        out
    }

    /// One game from its `appmanifest_<id>.acf`, with its folders in `steamapps`.
    pub(crate) fn from_manifest(steamapps: &Path, text: &str) -> Option<App> {
        let kv = pairs(text);
        let id = kv.get("appid")?.clone();
        let name = kv.get("name")?.clone();
        let dir = kv.get("installdir")?;
        // a folder name, never a path
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) || dir.is_empty() || dir.contains(['\\', '/', ':']) || dir == "." || dir == ".." {
            return None;
        }
        let last = kv.get("lastplayed").and_then(|s| s.parse::<u64>().ok()).filter(|&s| s > 0).and_then(|s| UNIX_EPOCH.checked_add(Duration::from_secs(s)));
        let locations = vec![
            steamapps.join("common").join(dir),
            steamapps.join("shadercache").join(&id),
            steamapps.join("compatdata").join(&id),
            steamapps.join("workshop").join("content").join(&id),
        ];
        Some(App {
            how_to_remove: format!("Steam: Library > right-click {name} > Manage > Uninstall"),
            name,
            publisher: None,
            kind: AppKind::Steam,
            locations,
            last_used: last.map(|t| (t, UsedFrom::Steam)),
        })
    }
}

// ---------------------------------------------------------------------- Epic

fn epic() -> Vec<App> {
    let Some(pd) = std::env::var_os("ProgramData") else { return Vec::new() };
    let dir = PathBuf::from(pd).join(r"Epic\EpicGamesLauncher\Data\Manifests");
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        if !e.file_name().to_string_lossy().to_lowercase().ends_with(".item") || e.metadata().map_or(true, |m| m.len() > 1 << 20) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let (Some(name), Some(loc)) = (v["DisplayName"].as_str(), v["InstallLocation"].as_str().and_then(|l| clean_path(&l.replace('/', "\\")))) else { continue };
        out.push(App {
            how_to_remove: format!("Epic Games Launcher: Library > {name} > ... > Uninstall"),
            name: name.to_string(),
            publisher: None,
            kind: AppKind::Epic,
            locations: vec![loc],
            last_used: None,
        });
    }
    out
}

// ---------------------------------------------------------------------- findings

/// Apps not used for `days`: one finding per app, on its install folder, with the size of all
/// its folders in the evidence. Only a date from Steam or Prefetch counts: an unknown date, or
/// one only from UserAssist, is never taken to mean "unused". A launcher whose folder holds
/// other apps (Steam and its games) is never suggested.
pub fn unused_apps(tree: &Tree, facts: &Facts, days: u64, min_bytes: u64) -> Vec<Finding> {
    let now = SystemTime::now();
    let mut out = Vec::new();
    for app in &facts.apps {
        let Some((when, from)) = app.last_used else { continue };
        let Ok(age) = now.duration_since(when) else { continue };
        if age.as_secs() < days * 86_400 || app.kind == AppKind::Wsl || from == UsedFrom::UserAssist {
            continue;
        }
        let holds_others = facts.apps.iter().any(|other| {
            !std::ptr::eq(other, app)
                && other.locations.iter().any(|o| app.locations.iter().any(|l| o != l && o.starts_with(l)))
        });
        if holds_others {
            continue;
        }
        let found: Vec<(usize, &PathBuf)> = app.locations.iter().filter_map(|l| find_path(tree, l).map(|n| (n, l))).collect();
        let Some(&(main, path)) = found.first() else { continue };
        let total: u64 = found.iter().map(|&(n, _)| tree.nodes[n].size).sum();
        if total < min_bytes {
            continue;
        }
        let mut evidence = vec![format!("Last used in {} (from {})", month_year(when), from.label())];
        if found.len() > 1 {
            evidence.push(format!("{} folders in all; uninstalling removes them together", found.len()));
        }
        evidence.push(format!("To remove it: {}", app.how_to_remove));
        out.push(Finding {
            path: path.clone(),
            owner: Some(app.name.clone()),
            bytes: tree.nodes[main].size,
            risk: Risk::Amber,
            remedy: Remedy::Uninstall,
            command: None,
            title: format!("Unused: {}", app.name),
            evidence,
            confidence: if from == UsedFrom::Steam { 0.9 } else { 0.7 },
            source: Source::Rule("app.unused".into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_manifest() {
        let acf = r#""AppState"
{
	"appid"		"620"
	"name"		"Portal 2"
	"installdir"		"Portal 2"
	"LastPlayed"		"1700000000"
	"UserConfig" { "language" "english" }
}"#;
        let app = steam::from_manifest(Path::new(r"D:\SteamLibrary\steamapps"), acf).unwrap();
        assert_eq!(app.name, "Portal 2");
        assert_eq!(app.locations[0], PathBuf::from(r"D:\SteamLibrary\steamapps\common\Portal 2"));
        assert_eq!(app.locations[1], PathBuf::from(r"D:\SteamLibrary\steamapps\shadercache\620"));
        assert_eq!(app.last_used.unwrap().1, UsedFrom::Steam);
        // a manifest can't point outside its library
        for bad in [r#""..\\..\\Windows""#, r#"".."""#, r#""C:\\Windows""#, r#""a/b""#] {
            let evil = acf.replace(r#""Portal 2"
	"LastPlayed""#, &format!("{bad}\n\t\"LastPlayed\""));
            assert_ne!(evil, acf);
            assert!(steam::from_manifest(Path::new(r"D:\SteamLibrary\steamapps"), &evil).is_none(), "{bad}");
        }
        let vdf = r#""libraryfolders" { "0" { "path" "C:\\Program Files (x86)\\Steam" } "1" { "path" "D:\\SteamLibrary" } }"#;
        assert_eq!(steam::library_paths(vdf), vec![PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r"D:\SteamLibrary")]);
    }

    #[test]
    fn owners_and_dates() {
        let app = |name: &str, loc: &str| App {
            name: name.into(),
            publisher: None,
            kind: AppKind::Installed,
            locations: vec![PathBuf::from(loc)],
            last_used: None,
            how_to_remove: String::new(),
        };
        let f = Facts::from_apps(vec![app("Suite", r"C:\Program Files\Suite"), app("Suite Tool", r"C:\Program Files\Suite\Tool"), app("Greedy", r"C:\Program Files")]);
        assert_eq!(f.owner_of(Path::new(r"C:\Program Files\Suite\Tool\x.exe")).unwrap().name, "Suite Tool");
        assert_eq!(f.owner_of(Path::new(r"c:\program files\suite\data")).unwrap().name, "Suite");
        assert!(f.owner_of(Path::new(r"C:\Program Files\Other")).is_none(), "a top-level folder is no one's");
        assert_eq!(month_year(UNIX_EPOCH + Duration::from_secs(1_700_000_000)), "November 2023");
        assert_eq!(month_year(UNIX_EPOCH), "January 1970");
        assert_eq!(clean_path(r#""C:\Games\X\""#), Some(PathBuf::from(r"C:\Games\X")));
        assert_eq!(clean_path(r"relative\x"), None);
    }
}
