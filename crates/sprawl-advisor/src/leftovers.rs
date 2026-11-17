//! Folders left behind by applications that are no longer installed: one level below the
//! user's `AppData\Local`, `AppData\Roaming`, `AppData\LocalLow` and below `ProgramData`, and
//! nothing deeper. A folder is offered only when every check says the application is gone:
//!
//! - no installed application (Windows' list, the Store, Steam, Epic, GOG) whose name,
//!   publisher or install folder matches the folder's name, or the name of a folder in it, by
//!   whole words ("Go" never matches "Google", either way);
//! - no installed application's folder inside it;
//! - no program of a matching name, nor any program inside it, started in the last
//!   [`IDLE_DAYS`] (Prefetch, and the Start menu history);
//! - nothing in it changed in the last [`IDLE_DAYS`];
//! - nothing in it found by a rule, a cleaner or the Chromium recogniser, nothing in it on the
//!   never-list, and no Chromium folder in it.
//!
//! Application data can hold settings, saved games and licences, and Sprawl can't tell: these
//! are always orange (the user's own call, one by one), and never picked or scripted.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use sprawl_core::Tree;

use crate::facts::{month_year, Facts};
use crate::finding::{Action, Finding, Remedy, Risk, Source};

/// The rule id of what this finds.
pub const ID: &str = "app.leftover";

/// How long an application's folder must have been still, and its programs not started.
pub const IDLE_DAYS: u64 = 180;

/// Smallest folder offered.
pub const MIN_BYTES: u64 = 50 << 20;

/// Folders Windows itself makes there (or that hold what installers need), by lower-case name:
/// never applications' leftovers. Names starting with "microsoft" or "windows" are skipped too.
pub const NOT_APPS: &[&str] = &[
    "packages",
    "temp",
    "tmp",
    "programs",
    "publishers",
    "crashdumps",
    "d3dscache",
    "connecteddevicesplatform",
    "comms",
    "history",
    "virtualstore",
    "application data",
    "elevateddiagnostics",
    "peerdistrepub",
    "placeholdertilelogofolder",
    "iconcache",
    "fontcache",
    "squirreltemp",
    "temporary internet files",
    "package cache",
    "packagemanagement",
    "ssh",
    "usoprivate",
    "usoshared",
    "softwaredistribution",
    "templates",
    "start menu",
    "desktop",
    "documents",
    "favorites",
    "regid.1991-06.com.microsoft",
    "sun",
    "oracle",
    "identities",
    "deliveryoptimization",
    "assembly",
    "gameanalytics",
    // package managers: what they installed lives inside, with no Uninstall key of its own
    "chocolatey",
    "scoop",
    "winget",
    "chocolateyhttpcache",
];

/// A folder name that is never an application's leftover.
pub fn not_an_app(name: &str) -> bool {
    let n = name.to_lowercase();
    NOT_APPS.contains(&n.as_str()) || n.starts_with("microsoft") || n.starts_with("windows") || n.starts_with('.') || n.starts_with("regid.")
}

/// The words of a name, lower case: letters and digits, split at anything else and where a
/// lower-case letter meets an upper-case one ("GitHubDesktop" is git, hub, desktop).
pub fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && prev_lower && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = c.is_lowercase();
            cur.extend(c.to_lowercase());
        } else {
            prev_lower = false;
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Two names match when the words of one appear, in order and together, in the other, or
/// when both are the same once their words are joined ("Git Hub" and "github"). Whole words
/// only: "Go" doesn't match "Google".
pub fn names_match(a: &str, b: &str) -> bool {
    let (x, y) = (words(a), words(b));
    if x.is_empty() || y.is_empty() {
        return false;
    }
    let inside = |s: &[String], l: &[String]| s.len() <= l.len() && l.windows(s.len()).any(|w| w == s);
    inside(&x, &y) || inside(&y, &x) || x.concat() == y.concat()
}

/// Names an installed application goes by: its name, its publisher, and the last two folders
/// of each place it is installed in (`Blender Foundation\Blender 4.2`).
fn app_names(facts: &Facts) -> Vec<String> {
    let mut out = Vec::new();
    for a in &facts.apps {
        out.push(a.name.clone());
        out.extend(a.publisher.clone());
        for l in &a.locations {
            let segs: Vec<String> = l.to_string_lossy().split(['\\', '/']).filter(|s| !s.is_empty() && !s.ends_with(':')).map(str::to_string).collect();
            for s in segs.iter().rev().take(2) {
                let g = s.to_lowercase();
                if !["program files", "program files (x86)", "programs", "common files", "steamapps", "common", "appdata", "local", "roaming", "users"].contains(&g.as_str()) {
                    out.push(s.clone());
                }
            }
        }
    }
    out
}

/// The newest change of anything in folder `n`, files and folders alike.
fn newest_inside(tree: &Tree, n: usize) -> Option<SystemTime> {
    let mut best = None;
    let mut stack = vec![n];
    while let Some(k) = stack.pop() {
        let node = &tree.nodes[k];
        best = best.max(node.modified);
        if node.is_dir && (k == n || !node.is_reparse_point()) {
            stack.extend_from_slice(&node.children);
        }
    }
    best
}

/// A Chromium folder in the first levels of `n`.
fn holds_chromium(tree: &Tree, n: usize) -> bool {
    let mut stack = vec![(n, 0)];
    while let Some((k, d)) = stack.pop() {
        if crate::chromium::recognise(tree, k).is_some() {
            return true;
        }
        if d < 3 {
            stack.extend(tree.nodes[k].children.iter().filter(|&&c| tree.nodes[c].is_dir && !tree.nodes[c].is_reparse_point()).map(|&c| (c, d + 1)));
        }
    }
    false
}

/// Folder names that hold the user's own work, not an application's.
pub const WORK_FOLDERS: &[&str] = &[
    "sketches", "projects", "documents", "worlds", "maps", "profiles", "screenshots", "recordings", "replays", "backups", "exports", "workspace",
];

/// What in the first levels of `n` is the user's own work: saved games (a folder whose name
/// says save, but Unreal's `Saved` alone, which holds settings, logs and caches), a folder
/// [`WORK_FOLDERS`] names, or a document, image project or source file
/// ([`sprawl_core::kinds::user_work`]). Its path inside `n`, and whether it is saved games.
fn work_in(tree: &Tree, n: usize) -> Option<(String, bool)> {
    let mut stack = vec![(n, 0, String::new())];
    let mut file = None;
    while let Some((k, d, rel)) = stack.pop() {
        for &c in &tree.nodes[k].children {
            let node = &tree.nodes[c];
            let name = node.name.to_lowercase();
            let rel = if rel.is_empty() { node.name.clone() } else { format!(r"{rel}\{}", node.name) };
            if !node.is_dir {
                if file.is_none() && name.rsplit_once('.').is_some_and(|(_, e)| sprawl_core::kinds::user_work(e)) {
                    file = Some(rel);
                }
                continue;
            }
            if node.is_reparse_point() {
                continue;
            }
            if name.contains("save") && name != "saved" {
                return Some((rel, true));
            }
            if WORK_FOLDERS.contains(&name.as_str()) {
                return Some((rel, false));
            }
            if d < 2 {
                stack.push((c, d + 1, rel));
            }
        }
    }
    file.map(|f| (f, false))
}

/// Lower-case program names (`.exe`) in the first levels of `n`.
fn programs_in(tree: &Tree, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(n, 0)];
    while let Some((k, d)) = stack.pop() {
        let node = &tree.nodes[k];
        if !node.is_dir {
            if node.name.to_lowercase().ends_with(".exe") {
                out.push(node.name.to_lowercase());
            }
        } else if d < 6 && (k == n || !node.is_reparse_point()) {
            stack.extend(node.children.iter().map(|&c| (c, d + 1)));
        }
    }
    out
}

/// The folders whose children are looked at: the user's AppData parts and ProgramData.
fn roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        for p in ["Local", "Roaming", "LocalLow"] {
            out.push(home.join("AppData").join(p));
        }
    }
    if let Some(pd) = std::env::var_os("ProgramData").map(PathBuf::from) {
        out.push(pd);
    }
    out
}

/// What applications that are gone left behind (see the module's notes). `seen` holds the
/// lower-case paths already found by anything else.
pub fn find(tree: &Tree, facts: &Facts, seen: &HashSet<String>) -> Vec<Finding> {
    find_in(tree, facts, &roots(), seen, SystemTime::now())
}

pub(crate) fn find_in(tree: &Tree, facts: &Facts, roots: &[PathBuf], seen: &HashSet<String>, now: SystemTime) -> Vec<Finding> {
    let never = crate::safety::Never::default_list();
    let names = app_names(facts);
    // the user's thresholds (Settings > Advisor), never below their floors
    let idle_days = crate::tuning::get(&crate::tuning::LEFTOVER_IDLE_DAYS);
    let min_bytes = crate::tuning::get(&crate::tuning::LEFTOVER_MIN_MB) << 20;
    let since = now.checked_sub(Duration::from_secs(idle_days * 86_400)).unwrap_or(SystemTime::UNIX_EPOCH);
    let mut out = Vec::new();
    for root in roots {
        let Some(r) = crate::find_path(tree, root) else { continue };
        for &n in &tree.nodes[r].children {
            let node = &tree.nodes[n];
            if !node.is_dir || node.is_reparse_point() || node.size < min_bytes || not_an_app(&node.name) {
                continue;
            }
            // the folder's name, and the names of the folders in it (LocalLow\IronGate\Valheim)
            let mut own: Vec<&str> = vec![node.name.as_str()];
            own.extend(node.children.iter().filter(|&&c| tree.nodes[c].is_dir).map(|&c| tree.nodes[c].name.as_str()));
            if own.iter().any(|o| names.iter().any(|a| names_match(o, a))) {
                continue;
            }
            let path = tree.path_of(n);
            let lc = path.to_string_lossy().to_lowercase();
            // an installed application's folder in it, or it inside one
            if facts.apps.iter().any(|a| a.locations.iter().any(|l| crate::finding::within(l, &path) || crate::finding::within(&path, l))) {
                continue;
            }
            if seen.iter().any(|s| s == &lc || s.starts_with(&format!("{lc}\\")) || lc.starts_with(&format!("{s}\\"))) {
                continue;
            }
            if never.hit(&path.to_string_lossy()).is_some() || !never.hits_inside(tree, n).is_empty() || holds_chromium(tree, n) {
                continue;
            }
            let Some(newest) = newest_inside(tree, n) else { continue };
            if newest >= since {
                continue;
            }
            // programs of a matching name, or in it, started lately
            let inside = programs_in(tree, n);
            let last_run = facts.last_run(|exe| inside.contains(&exe.to_string()) || own.iter().any(|o| names_match(o, exe.trim_end_matches(".exe"))));
            if last_run.is_some_and(|(t, _)| t >= since) {
                continue;
            }
            let runs = match (facts.runs_complete, last_run) {
                (true, Some((t, from))) => format!("No program by this name started since {} ({})", month_year(t), from.label()),
                (true, None) => "No program by this name, or in it, started as far as Windows remembers".to_string(),
                (false, _) => format!(
                    "Windows Prefetch couldn't be read (Sprawl isn't running as administrator), so only starts from the Start menu were checked: it is offered because nothing in it changed for {idle_days} days"
                ),
            };
            // the user's own work in it: shown, never deleted by Sprawl
            let work = work_in(tree, n);
            let mut f = Finding {
                path,
                owner: None,
                bytes: node.size,
                risk: Risk::Orange,
                remedy: if work.is_some() { Remedy::Info } else { Remedy::Delete },
                command: None,
                title: match &work {
                    Some((_, true)) => format!("{} (left behind, holds your saved games)", node.name),
                    Some(_) => format!("{} (left behind, holds your files)", node.name),
                    None => format!("{} (left behind)", node.name),
                },
                evidence: vec![
                    format!("No installed application by this name ({} checked: Windows' list, the Store, Steam, Epic, GOG)", facts.apps.len()),
                    format!("Nothing in it changed since {}", month_year(newest)),
                    runs,
                    match &work {
                        Some((rel, true)) => format!("It holds your saved games ({rel}): look before removing it; Sprawl won't"),
                        Some((rel, false)) => format!("It holds your own files ({rel}): look before removing it; Sprawl won't"),
                        None => "It may hold settings or saved games of an application you might install again".into(),
                    },
                    format!("{} files in {} folders", node.files, node.folders),
                ],
                confidence: 0.6,
                source: Source::Rule(ID.into()),
                allow_protected: false,
                trusted: true,
                builtin: true,
                on_path: true,
                action: None,
                admin: false,
                carved: Vec::new(),
                gain: None,
            };
            if work.is_none() {
                f.set_action(Action::Remove, false);
            }
            out.push(f);
        }
    }
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{App, AppKind};

    #[test]
    fn names_match_by_whole_words_only() {
        assert!(names_match("Discord", "Discord"));
        assert!(names_match("Mozilla", "Mozilla Firefox (x64 en-US)"));
        assert!(names_match("GitHubDesktop", "GitHub Desktop"));
        assert!(names_match("github", "Git Hub"));
        assert!(names_match("JetBrains", "JetBrains s.r.o."));
        // substrings are not words, either way
        assert!(!names_match("Go", "Google"));
        assert!(!names_match("Google", "Go"));
        assert!(!names_match("Go", "Google Chrome"));
        // joined words are split where the case changes: SteamVR is steam, vr (matching more
        // only keeps more)
        assert!(names_match("Steam", "SteamVR"));
        assert!(!names_match("Steam", "Steamworks"));
        assert!(!names_match("", "x"));
    }

    #[test]
    fn what_windows_makes_is_never_a_leftover() {
        for n in ["Packages", "Temp", "Programs", "Publishers", "CrashDumps", "D3DSCache", "ConnectedDevicesPlatform", "Microsoft", "MicrosoftEdge", "Windows", "Package Cache", "chocolatey", ".vscode"] {
            assert!(not_an_app(n), "{n}");
        }
        assert!(!not_an_app("Discord"));
        // one list, lower case, no repeats
        let mut l = NOT_APPS.to_vec();
        assert!(l.iter().all(|n| *n == n.to_lowercase()));
        l.sort();
        l.dedup();
        assert_eq!(l.len(), NOT_APPS.len());
    }

    fn app(name: &str, publisher: Option<&str>, loc: &std::path::Path) -> App {
        App {
            name: name.into(),
            publisher: publisher.map(str::to_string),
            kind: AppKind::Installed,
            locations: vec![loc.to_path_buf()],
            last_used: None,
            how_to_remove: String::new(),
        }
    }

    #[test]
    fn only_what_gone_applications_left_is_offered() {
        let base = std::env::temp_dir().join(format!("sprawl-leftovers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let roaming = base.join("Roaming");
        let big = |p: &std::path::Path, old: bool| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            let f = std::fs::File::create(p).unwrap();
            f.set_len(60 << 20).unwrap();
            if old {
                f.set_modified(SystemTime::now() - Duration::from_secs(400 * 86_400)).unwrap();
            }
        };
        big(&roaming.join("GoneApp").join("data.bin"), true);
        // installed: by name, by publisher (a vendor folder), by a folder in it
        big(&roaming.join("Contoso Paint").join("data.bin"), true);
        big(&roaming.join("Contoso").join("Other").join("data.bin"), true);
        big(&roaming.join("IronGate").join("Valheim").join("save.bin"), true);
        // changed last month
        big(&roaming.join("Fresh").join("data.bin"), false);
        // a name that only contains an installed one ("Go" in "Google")
        big(&roaming.join("Google").join("Saved").join("SaveGames").join("s.sav"), true);
        // Windows' own
        big(&roaming.join("CrashDumps").join("x.dmp"), true);
        // something protected inside
        big(&roaming.join("Mailer").join("store.mbox"), true);
        // the user's sketches
        big(&roaming.join("Tinker").join("sketches").join("blink").join("blink.bin"), true);
        // small
        std::fs::create_dir_all(roaming.join("Tiny")).unwrap();
        std::fs::write(roaming.join("Tiny").join("a"), b"x").unwrap();
        // folders change when something is made in them: make them old too, but Fresh
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let old = SystemTime::now() - Duration::from_secs(400 * 86_400);
            let mut stack = vec![roaming.clone()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).unwrap().flatten() {
                    if e.path().is_dir() {
                        stack.push(e.path());
                    }
                }
                if !d.ends_with("Fresh") {
                    // FILE_FLAG_BACKUP_SEMANTICS opens a folder
                    let f = std::fs::OpenOptions::new().write(true).custom_flags(0x0200_0000).open(&d).unwrap();
                    f.set_modified(old).unwrap();
                }
            }
        }
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let progs = base.join("Program Files");
        let facts = Facts::from_apps(vec![
            app("Contoso Paint", Some("Contoso Ltd"), &progs.join("Contoso Paint")),
            app("Valheim", None, &progs.join("Valheim")),
            app("Go Programming Language", None, &progs.join("Go")),
        ]);
        let found = find_in(&tree, &facts, &[roaming.clone()], &HashSet::new(), SystemTime::now());
        let names: Vec<String> = found.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["GoneApp", "Google", "Tinker"], "{names:?}");
        for f in &found {
            assert_eq!(f.risk, Risk::Orange);
            assert!(f.heuristic(), "never picked for the user nor scripted");
            assert!(f.leftover() && f.group() == crate::finding::Group::Leftover);
            assert!(f.evidence.iter().any(|e| e.starts_with("Nothing in it changed since")));
        }
        // without the user's work: deletable through the review
        assert_eq!(found[0].action, Some(Action::Remove));
        assert_eq!(found[0].remedy, Remedy::Delete);
        assert!(found[0].evidence[3].starts_with("It may hold settings or saved games"));
        // saved games, or sketches: shown only, nothing to run
        for (f, what) in [(&found[1], r"saved games (Saved\SaveGames)"), (&found[2], r"own files (sketches)")] {
            assert_eq!(f.remedy, Remedy::Info, "{}", f.title);
            assert!(f.action.is_none() && f.command.is_none());
            assert!(f.evidence[3].contains(what), "{:?}", f.evidence);
        }
        assert!(found[1].title.contains("holds your saved games") && found[2].title.contains("holds your files"));
        // already found by something else: not again
        let seen: HashSet<String> = [roaming.join("GoneApp").join("cache").to_string_lossy().to_lowercase()].into();
        assert_eq!(find_in(&tree, &facts, &[roaming.clone()], &seen, SystemTime::now()).len(), 2);
        // a program of its name started lately keeps it
        let mut facts = facts;
        facts.set_run("goneapp.exe", SystemTime::now());
        assert_eq!(find_in(&tree, &facts, &[roaming.clone()], &HashSet::new(), SystemTime::now()).len(), 2);
        // never planned: orange, and a guess
        let n = found.len();
        assert!(crate::plan::plan(&found, &vec![true; n], 1 << 40).picked.is_empty());
        // a document inside is the user's work too; a settings file isn't
        let doc = roaming.join("Writer");
        big(&doc.join("notes.docx"), true);
        big(&doc.join("settings.json"), true);
        let t2 = sprawl_core::scan(&base, |_| {}).unwrap();
        assert!(work_in(&t2, crate::find_path(&t2, &doc).unwrap()).is_some_and(|(r, s)| r == "notes.docx" && !s));
        assert!(work_in(&t2, crate::find_path(&t2, &roaming.join("GoneApp")).unwrap()).is_none());
        // "library" is where many applications keep what they ship: not the user's work
        big(&roaming.join("Shipped").join("library").join("presets.bin"), true);
        let t3 = sprawl_core::scan(&base, |_| {}).unwrap();
        assert!(work_in(&t3, crate::find_path(&t3, &roaming.join("Shipped")).unwrap()).is_none());
        let _ = std::fs::remove_dir_all(&base);
    }
}
