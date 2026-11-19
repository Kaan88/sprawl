//! sprawl-advisor: finds space that can be won back in a scanned tree, and explains it.
//!
//! See `docs/plans/ai-advisor.md`. Rust finds, a model (later) judges; nothing here deletes
//! anything: the output is [`Finding`]s with a remedy and a command for the user to run.
//!
//! * [`facts`]: which app owns a folder and when it was last used (registry, manifests).
//! * [`hidden`]: used space the scan can't see (shadow copies, reserved storage, ...).
//! * [`inuse`]: which programs have files open (Restart Manager).
//! * [`projects`]: code projects, their build outputs and last activity.
//! * [`finding`]: the schema every finding follows, from rules and (later) from a model.
//! * [`rules`]: TOML rule packs and the engine that matches them against a tree.
//! * [`safety`]: the never-touch list and the checks every finding must pass.
//! * [`snapshot`]: a compact, compressed copy of a scan, for history and diffs.

pub mod anomaly;
pub mod chromium;
pub mod leftovers;
pub mod clean;
pub mod cleaners;
pub mod community;
pub mod config;
pub mod confidence;
pub mod dupes;
pub mod export;
pub mod exclusions;
pub mod exe_installers;
pub mod facts;
pub mod kinds;
pub mod learning;
pub mod finding;
pub mod growth;
pub mod game_leftovers;
pub mod goal;
pub mod hidden;
pub mod history;
pub mod inuse;
pub mod installers;
pub mod llm;
pub mod mcp;
pub mod mcp_scan;
pub mod memory;
pub mod pipe;
pub mod plan;
pub mod prevention;
pub mod privacy;
pub mod procs;
pub mod projects;
pub mod remedies;
pub mod rules;
pub mod safety;
pub mod snapshot;
pub mod stale;
mod store;
pub mod tuning;
pub mod used;
pub mod versions;
pub mod recycle;

/// "Work offline" (Settings > Network): while set, nothing leaves this PC. Every HTTP request
/// (update check, community cleaners, AI providers, web lookups) and Claude Code are refused.
pub static OFFLINE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What a refused request says while working offline.
pub const OFFLINE_MSG: &str = "Sprawl is working offline (Settings > Network): nothing is sent.";

/// Whether "Work offline" is on.
/// Running as administrator, the user allowed Claude Code (Settings > Scanning, kept where only
/// an administrator can write). Off: Claude Code isn't started from an elevated Sprawl.
pub static ELEVATED_CLAUDE_CODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The same for built-in tool commands whose program sits in a folder the account can change.
pub static ELEVATED_USER_TOOLS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn offline() -> bool {
    OFFLINE.load(std::sync::atomic::Ordering::Relaxed)
}
#[cfg(windows)]
pub(crate) mod reg;

pub use finding::{Finding, Remedy, Risk, Source};
pub use rules::{Rule, RulePack};

use std::path::Path;

use sprawl_core::{Tree, MULTIPLE, ROOT};

/// Write one of this account's files (settings, notes, exports) safely from an app that may be
/// elevated: every folder on the way is opened as itself (no link), the bytes go to a new
/// temporary file of its own, and that is renamed to `path` through its handle. A link planted
/// at the temporary name or at `path` is never followed. `replace: false` refuses a name that
/// already exists.
pub fn write_local(path: &Path, bytes: &[u8], replace: bool) -> std::io::Result<()> {
    use std::io::Write;
    let parent = store::ensure_dir(path.parent().ok_or_else(|| std::io::Error::other("No folder to write in"))?)?;
    let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| std::io::Error::other("Invalid file name"))?;
    store::write_atomic(&parent.path, name, replace, |f| f.write_all(bytes))
}

/// Remove a file of Sprawl's own through its handle: the folders on the way held, the file
/// opened as itself (a link at its name, or a file with other names, is left and refused).
/// Nothing there is not an error.
pub fn remove_local(path: &Path) -> std::io::Result<()> {
    let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| std::io::Error::other("Invalid file name"))?;
    store::remove_checked(path.parent().ok_or_else(|| std::io::Error::other("No folder"))?, name)
}

/// [`find_path`] that never takes one of several names differing only in case (in a
/// case-sensitive folder) by chance: at such a level only the name as spelled in `path`
/// matches. Elsewhere case is ignored, as Windows does.
pub(crate) fn find_path_unambiguous(tree: &Tree, path: &Path) -> Option<usize> {
    let s = path.to_string_lossy();
    let spelled: Vec<&str> = s.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    let want = segments(&s);
    // (node, its path's segments lower-cased, and as spelled)
    let root = |n: usize, p: &str| (n, segments(p), p.split(['\\', '/']).filter(|s| !s.is_empty()).map(str::to_string).collect::<Vec<_>>());
    let starts: Vec<(usize, Vec<String>, Vec<String>)> = if tree.nodes[ROOT].name == MULTIPLE {
        tree.nodes[ROOT].children.iter().map(|&c| root(c, &tree.nodes[c].name)).collect()
    } else {
        vec![root(ROOT, &tree.path_of(ROOT).to_string_lossy())]
    };
    let starts: Vec<_> = starts.into_iter().filter(|(_, base, _)| want.len() >= base.len() && want[..base.len()] == base[..]).collect();
    'start: for (start, base, as_spelled) in &starts {
        // scan roots differing only in case (`Game` and `game`): only the one spelled as in
        // `path` matches, and none when neither is
        // (a drive letter's case never counts)
        let same = |a: &&str, b: &String| *a == b.as_str() || a.ends_with(':') && a.eq_ignore_ascii_case(b);
        if starts.iter().filter(|s| s.1 == *base).count() > 1 && !spelled[..base.len()].iter().zip(as_spelled).all(|(a, b)| same(a, b)) {
            continue;
        }
        let (start, base) = (*start, base);
        let mut n = start;
        for (i, part) in want.iter().enumerate().skip(base.len()) {
            let alike: Vec<usize> = tree.nodes[n].children.iter().copied().filter(|&c| tree.nodes[c].name.to_lowercase() == *part).collect();
            n = match alike.as_slice() {
                [] => continue 'start,
                [one] => *one,
                many => many.iter().copied().find(|&c| tree.nodes[c].name == spelled[i])?,
            };
        }
        return Some(n);
    }
    None
}

/// Lower-case path segments, without empty ones: `C:\Users\` -> `["c:", "users"]`.
pub(crate) fn segments(path: &str) -> Vec<String> {
    path.split(['\\', '/']).filter(|s| !s.is_empty()).map(|s| s.to_lowercase()).collect()
}

/// A file's current time against a scan's record, to the whole second. A History or bridge
/// tree is read from a snapshot, which keeps whole seconds only, so an exact comparison
/// (1.0) called every such file changed and refused it.
pub(crate) fn same_time(now: Option<std::time::SystemTime>, scanned: Option<std::time::SystemTime>) -> bool {
    // signed whole seconds, cut toward 1970 as a snapshot stores them (before 1970 too)
    let secs = |t: std::time::SystemTime| match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i128,
        Err(e) => -(e.duration().as_secs() as i128),
    };
    match (now, scanned) {
        (Some(a), Some(b)) => secs(a) == secs(b),
        _ => false,
    }
}

#[cfg(test)]
#[test]
fn same_time_compares_whole_seconds_before_1970_too() {
    use std::time::{Duration, UNIX_EPOCH};
    let before = UNIX_EPOCH - Duration::from_millis(86_400_250);
    let stored = UNIX_EPOCH - Duration::from_secs(86_400);
    assert!(same_time(Some(stored), Some(stored)), "identical pre-1970 times (Astra's case)");
    assert!(same_time(Some(before), Some(stored)), "a snapshot keeps whole seconds");
    assert!(!same_time(Some(UNIX_EPOCH - Duration::from_secs(86_401)), Some(stored)));
    let after = UNIX_EPOCH + Duration::from_millis(1_700_000_000_900);
    assert!(same_time(Some(after), Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000))));
    assert!(!same_time(None, None));
}

/// Only a live multiple-root scan's direct children may carry absolute names.
/// Ordinary descendants must remain plain components, even below a synthetic root.
pub(crate) fn scan_name_ok(tree: &Tree, n: usize) -> bool {
    let node = &tree.nodes[n];
    if n == ROOT { return true; }
    if !node.name.is_empty() && !matches!(node.name.as_str(), "." | "..")
        && !node.name.contains(['\\', '/', ':']) { return true; }
    tree.method != sprawl_core::ScanMethod::Csv && tree.root_path == Path::new(MULTIPLE)
        && node.parent == Some(ROOT) && tree.nodes[ROOT].children.contains(&n)
        && Path::new(&node.name).is_absolute()
        && sprawl_core::local::lexical(Path::new(&node.name)).is_ok()
}

/// The node at an absolute `path`, if the tree has it.
pub fn find_path(tree: &Tree, path: &Path) -> Option<usize> {
    let want = segments(&path.to_string_lossy());
    let starts: Vec<(usize, Vec<String>)> = if tree.nodes[ROOT].name == MULTIPLE {
        tree.nodes[ROOT].children.iter().map(|&c| (c, segments(&tree.nodes[c].name))).collect()
    } else {
        vec![(ROOT, segments(&tree.path_of(ROOT).to_string_lossy()))]
    };
    for (start, base) in starts {
        if want.len() < base.len() || want[..base.len()] != base[..] {
            continue;
        }
        let mut n = start;
        let mut ok = true;
        for part in &want[base.len()..] {
            match tree.nodes[n].children.iter().copied().find(|&c| tree.nodes[c].name.to_lowercase() == *part) {
                Some(c) => n = c,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return Some(n);
        }
    }
    None
}

/// Run every pack over the tree, then drop findings that fail the safety checks. The rejected ones
/// come back too, with the reason, so the UI can show what was held back.
pub fn advise(tree: &Tree, packs: &[RulePack]) -> (Vec<Finding>, Vec<(Finding, safety::Rejection)>) {
    let never = safety::Never::default_list();
    let mut ok = Vec::new();
    let mut held = Vec::new();
    for f in rules::run(tree, packs) {
        match safety::check(&f, tree, &never) {
            Ok(f) => ok.push(f),
            Err(r) => held.push((f, r)),
        }
    }
    ok.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    (ok, held)
}

/// Apps unused for this long are suggested for removal.
pub const UNUSED_DAYS: u64 = 180;

/// Build outputs of projects untouched for this long are suggested.
pub const PROJECT_IDLE_DAYS: u64 = 30;

/// Everything Rust can say without a model: rule findings (with their owner app filled in) and
/// apps not used for [`UNUSED_DAYS`], all through the safety checks. A path is reported once.
pub fn advise_all(tree: &Tree, packs: &[RulePack], facts: &facts::Facts) -> (Vec<Finding>, Vec<(Finding, safety::Rejection)>) {
    let never = safety::Never::default_list();
    let mut found = rules::run(tree, packs);
    for f in &mut found {
        // F3: WSL's and Docker's disk files become a shrink Sprawl runs when ticked
        remedies::shrinkable(f);
        if f.owner.is_none() {
            f.owner = facts.owner_of(&f.path).map(|a| a.name.clone());
        }
    }
    let mut seen: std::collections::HashSet<String> = found.iter().map(|f| f.path.to_string_lossy().to_lowercase()).collect();
    let projects = projects::find(tree);
    let extra = facts::unused_apps(tree, facts, tuning::get(&tuning::UNUSED_APP_DAYS), 1 << 30)
        .into_iter()
        .chain(projects::build_outputs(tree, &projects, facts, tuning::get(&tuning::PROJECT_IDLE_DAYS), 100 << 20))
        .chain(anomaly::find(tree))
        .chain(versions::find(tree, &projects, &facts.apps))
        // A3: copies the scan alone shows (same name and size; whole folders), nothing read
        .chain(dupes::likely(tree, tuning::get(&tuning::DUPES_MIN_MB) << 20, 1 << 30))
        // A2: what things are, by their layout (an archive already extracted beside it)
        .chain(kinds::archives(tree, 50 << 20))
        .chain(installers::find(tree, facts, 10 << 20))
        .chain(exe_installers::find(tree, facts, 10 << 20))
        .chain(game_leftovers::find(tree, facts, 50 << 20))
        .chain(remedies::compact(tree))
        .chain(remedies::onedrive(tree))
        .chain(remedies::moves(tree, &hidden::fixed_drives()));
    for f in extra {
        if seen.insert(f.path.to_string_lossy().to_lowercase()) {
            found.push(f);
        }
    }
    // caches of programs built on Chromium, known by their folder; then caches of programs no
    // cleaner knows, last, so a known one names them first
    for f in chromium::find(tree, facts, &seen, cleaners::part_min()) {
        if seen.insert(f.path.to_string_lossy().to_lowercase()) {
            found.push(f);
        }
    }
    for f in cleaners::by_shape(tree, &seen) {
        if seen.insert(f.path.to_string_lossy().to_lowercase()) {
            found.push(f);
        }
    }
    // folders left by applications that are gone: last, with everything else already known
    for f in leftovers::find(tree, facts, &seen) {
        if seen.insert(f.path.to_string_lossy().to_lowercase()) {
            found.push(f);
        }
    }
    // big things nothing changed in for a year, as information: what a person looks at first on
    // a drive of games, media or backups, where no rule knows a cache. After everything else,
    // so nothing is said twice.
    for f in stale::find(tree, facts, &seen) {
        if seen.insert(f.path.to_string_lossy().to_lowercase()) {
            found.push(f);
        }
    }
    // how sure Sprawl is of each, from the signals above; before the checks, which may only
    // raise a risk
    confidence::apply(&mut found, tree, facts, std::time::SystemTime::now());
    let mut ok = Vec::new();
    let mut held = Vec::new();
    for f in found {
        match safety::check(&f, tree, &never) {
            Ok(f) => ok.push(f),
            Err(r) => held.push((f, r)),
        }
    }
    ok.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    (ok, held)
}

/// What the Advisor shows for one scan.
#[derive(Clone, Debug, Default)]
pub struct Advice {
    /// [`advise_all`]'s findings: everything that frees space. Never a privacy part.
    pub findings: Vec<Finding>,
    pub held: Vec<(Finding, safety::Rejection)>,
    /// Privacy parts (cookies, history), checked: empty unless the setting was on. Kept apart so
    /// no sum of what can be freed, no tick, plan or script takes them by accident.
    pub privacy: Vec<Finding>,
    pub privacy_held: Vec<(Finding, safety::Rejection)>,
}

/// [`advise_all`], and with `privacy_cleaners` on (the setting [`privacy::SETTING`],
/// `privacyCleaners`, off by default) the privacy parts in this account's browser profiles, in
/// a list of their own. With it off, nothing about them is read or computed.
pub fn advise_report(tree: &Tree, packs: &[RulePack], facts: &facts::Facts, privacy_cleaners: bool) -> Advice {
    let places = if privacy_cleaners { privacy::Places::current() } else { None };
    advise_report_in(tree, packs, facts, privacy_cleaners, places.as_ref(), std::env::var_os("USERPROFILE").as_deref().map(Path::new))
}

pub(crate) fn advise_report_in(tree: &Tree, packs: &[RulePack], facts: &facts::Facts, privacy_cleaners: bool, places: Option<&privacy::Places>, home: Option<&Path>) -> Advice {
    let (findings, held) = advise_all(tree, packs, facts);
    let mut out = Advice { findings, held, ..Advice::default() };
    if !privacy_cleaners {
        return out;
    }
    let Some(places) = places else { return out };
    let never = safety::Never::default_list();
    for f in privacy::find_in(tree, places) {
        match safety::check_in(&f, tree, &never, safety::CACHE_CARVE_OUTS, home, Some(places)) {
            Ok(f) => out.privacy.push(f),
            Err(r) => out.privacy_held.push((f, r)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn local_writes_replace_the_name_never_what_it_links_to() {
        let base = std::env::temp_dir().join(format!("sprawl-write-local-{}", store::nonce()));
        std::fs::create_dir_all(&base).unwrap();
        let other = base.join("other.txt");
        std::fs::write(&other, b"keep me").unwrap();
        // the settings name made a second name of another file: only the name is replaced
        std::fs::hard_link(&other, base.join("settings.ini")).unwrap();
        write_local(&base.join("settings.ini"), b"a=1", true).unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), b"keep me");
        assert_eq!(std::fs::read(base.join("settings.ini")).unwrap(), b"a=1");
        // a missing folder on the way is made
        write_local(&base.join("new").join("x.txt"), b"x", true).unwrap();
        // never over an existing name when asked not to
        assert!(write_local(&other, b"no", false).is_err());
        assert_eq!(std::fs::read(&other).unwrap(), b"keep me");
        // no temporary file is left behind
        let names: Vec<String> = std::fs::read_dir(&base).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    fn temp_tree(tag: &str) -> (PathBuf, Tree) {
        let dir = std::env::temp_dir().join(format!("dt-adv-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cache").join("deep")).unwrap();
        std::fs::create_dir_all(dir.join("keep")).unwrap();
        std::fs::write(dir.join("cache").join("deep").join("blob.bin"), vec![7u8; 300_000]).unwrap();
        std::fs::write(dir.join("keep").join("doc.txt"), b"mine").unwrap();
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        (dir, tree)
    }

    fn pack(dir: &Path, remedy: &str) -> RulePack {
        let p = dir.join("cache").display().to_string().replace('\'', "''");
        RulePack::parse(&format!(
            "[pack]\nname = 'test'\n[[rule]]\nid = 't.cache'\ntitle = 'Test cache'\npaths = ['{p}']\nrisk = 'green'\nremedy = '{remedy}'\nwhy = 'test'\nmin_bytes = 1000\n"
        ))
        .unwrap()
    }

    #[test]
    fn rules_find_and_safety_passes() {
        let (dir, tree) = temp_tree("ok");
        let (ok, held) = advise(&tree, &[pack(&dir, "delete")]);
        assert!(held.is_empty(), "{held:?}");
        assert_eq!(ok.len(), 1);
        let f = &ok[0];
        assert_eq!(f.bytes, 300_000);
        assert_eq!(f.source, Source::Rule("t.cache".into()));
        assert!(f.command.as_deref().unwrap().contains("; sprawl-clear '"));
        assert_eq!(find_path(&tree, &dir.join("keep").join("doc.txt")).map(|n| tree.nodes[n].size), Some(4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn safety_holds_back_bad_findings() {
        let (dir, tree) = temp_tree("bad");
        let never = safety::Never::default_list();
        let (ok, _) = advise(&tree, &[pack(&dir, "delete")]);
        let good = ok[0].clone();

        let mut inflated = good.clone();
        inflated.bytes = 50_000_000;
        assert!(matches!(safety::check(&inflated, &tree, &never), Err(safety::Rejection::SizeMismatch { .. })));

        let mut invented = good.clone();
        invented.path = dir.join("nope");
        assert_eq!(safety::check(&invented, &tree, &never), Err(safety::Rejection::NotInTree));

        std::env::set_var("SystemDrive", "C:");
        std::env::set_var("SystemRoot", r"C:\Windows");
        let windows = sprawl_core::scan(Path::new(r"C:\Windows\Web"), |_| {}).unwrap();
        let model = Finding { path: windows.path_of(ROOT), bytes: windows.nodes[ROOT].size, remedy: Remedy::Command, source: Source::Model("m".into()), ..good.clone() };
        assert!(matches!(safety::check(&model, &windows, &never), Err(safety::Rejection::Never(_))));
        let rule = Finding {
            source: Source::Rule("r".into()),
            command: Some("DISM /Online /Cleanup-Image /StartComponentCleanup".into()),
            allow_protected: true,
            trusted: true,
            builtin: true,
            on_path: false,
            ..model.clone()
        };
        assert!(safety::check(&rule, &windows, &never).is_ok(), "curated command on a protected path is allowed");
        // the same command from a local pack or a model is not
        let local = Finding { builtin: false, ..rule.clone() };
        assert!(matches!(safety::check(&local, &windows, &never), Err(safety::Rejection::Never(_))));
        let delete = Finding { remedy: Remedy::Delete, ..rule };
        assert!(matches!(safety::check(&delete, &windows, &never), Err(safety::Rejection::Never(_))));

        // command text from a model or a local pack is never accepted
        let sneaky = Finding { command: Some(format!("Remove-Item -Recurse {}", dir.join("keep").display())), trusted: false, ..good.clone() };
        assert_eq!(safety::check(&sneaky, &tree, &never), Err(safety::Rejection::UntrustedCommand));
        // red is information only
        let red = Finding { risk: Risk::Red, ..good.clone() };
        assert_eq!(safety::check(&red, &tree, &never), Err(safety::Rejection::RedWithAction));
        // the scan root itself is never deleted
        let root = Finding { path: tree.path_of(ROOT), bytes: tree.nodes[ROOT].size, ..good.clone() };
        assert_eq!(safety::check(&root, &tree, &never), Err(safety::Rejection::ScanRoot));
        // any drive's system folders are protected, not only the system drive's
        assert!(never.hit(r"D:\").is_some());
        assert!(never.hit(r"D:\System Volume Information\x").is_some());
        assert!(never.hit(r"E:\Windows\System32").is_some());
        assert!(never.hit(r"D:\Users\sam\.ssh\id_ed25519").is_some());
        assert!(never.hit(r"D:\vm\disk.vhdx").is_some());
        assert!(never.hit(r"D:\games\cache").is_none());
        assert!(never.is_user_data(r"D:\Users\sam\Downloads\a.iso"));
        // folders that hold protected things are protected
        assert!(never.hit(r"C:\Users\sam\AppData\Local\Google").is_some());
        assert!(never.hit(r"C:\Users\sam\AppData\Roaming\Mozilla\Firefox").is_some());
        assert!(never.hit(r"C:\Users\sam\AppData\Local\Google\DriveFS").is_none());
        // other spellings of the same places
        assert!(never.hit(r"\\?\C:\Windows\System32").is_some());
        assert!(never.hit(r"\\.\C:\Windows").is_some());
        assert!(never.hit(r"\\localhost\C$\Windows\System32").is_some());
        assert!(never.hit(r"\\?\UNC\pc\share\Users\sam\.ssh").is_some());
        assert!(never.hit("C:/Windows/System32").is_some());
        assert!(never.hit(r"\\?\Volume{1234}\x").is_some(), "unknown forms fail closed");
        assert!(never.hit(r"\\nas\media\films").is_none());
        assert!(never.hit(r"\\pc\ADMIN$\System32").is_some());
        assert!(never.hit(r"\\pc\Users\sam\.ssh").is_some());
        assert!(never.hit(r"\\pc\IPC$\x").is_some());
        assert!(never.hit(r"C:\foo\..\Windows").is_some(), ".. fails closed");
        assert!(never.hit(r"C:\Windows.\System32").is_some(), "trailing dot fails closed");
        assert!(never.hit(r"C:foo").is_some());
        // an exempt built-in rule whose command names the path keeps its link checks
        let exempt_on_path = Finding {
            command: Some(format!("Remove-Item -LiteralPath {} -Recurse -Force", rules::ps_quote(&tree.path_of(ROOT).display().to_string()))),
            remedy: Remedy::Command,
            allow_protected: true,
            trusted: true,
            builtin: true,
            on_path: true,
            action: None,
            admin: false,
            carved: Vec::new(),
            gain: None,
            forced_from: None,
            path: tree.path_of(ROOT),
            bytes: tree.nodes[ROOT].size,
            ..good.clone()
        };
        assert_eq!(safety::check(&exempt_on_path, &tree, &never), Err(safety::Rejection::ScanRoot));
        // a recursive remedy is refused when a disk image sits inside
        std::fs::write(dir.join("cache").join("deep").join("vm.vhdx"), b"x").unwrap();
        let t2 = sprawl_core::scan(&dir, |_| {}).unwrap();
        let n = find_path(&t2, &dir.join("cache")).unwrap();
        assert!(never.hit_inside(&t2, n).is_some());
        // Sprawl's own clean-up skips it: the finding stays, the image carved out of it
        let with_image = Finding { bytes: t2.nodes[n].size, ..good.clone() };
        let carved = safety::check(&with_image, &t2, &never).expect("carved out, not held back");
        assert_eq!(carved.carved.len(), 1);
        assert_eq!(carved.carved[0].path, dir.join("cache").join("deep").join("vm.vhdx"));
        assert_eq!(carved.bytes, t2.nodes[n].size - 1);
        assert_eq!(carved.carved_note().as_deref(), Some("1 protected item kept"));
        // a typed clear under a "command" remedy is Sprawl's own run: carved too
        let typed = Finding { remedy: Remedy::Command, ..with_image.clone() };
        assert_eq!(safety::check(&typed, &t2, &never).map(|f| f.carved.len()), Ok(1));
        // a tool's command, a move or a model's finding can't skip it: still held back
        let tool = Finding { remedy: Remedy::Command, action: Some(crate::finding::Action::Tool { command: "x".into() }), trusted: true, builtin: true, ..with_image.clone() };
        assert!(matches!(safety::check(&tool, &t2, &never), Err(safety::Rejection::Never(_))));
        let moved = Finding { remedy: Remedy::Move, ..with_image.clone() };
        assert!(matches!(safety::check(&moved, &t2, &never), Err(safety::Rejection::Never(_))));
        let from_model = Finding { source: Source::Model("m".into()), ..with_image.clone() };
        assert!(matches!(safety::check(&from_model, &t2, &never), Err(safety::Rejection::Never(_))));
        let no_action = Finding { action: None, ..with_image.clone() };
        assert!(matches!(safety::check(&no_action, &t2, &never), Err(safety::Rejection::Never(_))));
        // a local pack's delete is never shown as green
        assert!(good.risk >= Risk::Amber);

        std::fs::remove_dir_all(dir.join("cache")).unwrap();
        assert_eq!(safety::check(&good, &tree, &never), Err(safety::Rejection::Missing));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folders_only_keeps_totals() {
        let (dir, tree) = temp_tree("fo");
        let fo = snapshot::folders_only(&tree);
        assert!(fo.nodes.iter().all(|n| n.is_dir || n.name == snapshot::FILES_HERE));
        assert_eq!(fo.nodes[ROOT].size, tree.nodes[ROOT].size);
        assert_eq!(fo.nodes[ROOT].allocated, tree.nodes[ROOT].allocated);
        assert_eq!(fo.nodes[ROOT].files, tree.nodes[ROOT].files);
        let mut buf = Vec::new();
        snapshot::write(&fo, std::time::SystemTime::now(), 9, &mut buf).unwrap();
        let (back, _) = snapshot::read(&mut buf.as_slice()).unwrap();
        assert_eq!(back.nodes[ROOT].size, tree.nodes[ROOT].size);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
