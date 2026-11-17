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
pub mod clean;
pub mod config;
pub mod dupes;
pub mod export;
pub mod facts;
pub mod finding;
pub mod growth;
pub mod hidden;
pub mod history;
pub mod inuse;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod pipe;
pub mod plan;
pub mod projects;
pub mod remedies;
pub mod rules;
pub mod safety;
pub mod snapshot;
#[cfg(windows)]
pub(crate) mod reg;

pub use finding::{Finding, Remedy, Risk, Source};
pub use rules::{Rule, RulePack};

use std::path::Path;

use sprawl_core::{Tree, MULTIPLE, ROOT};

/// Lower-case path segments, without empty ones: `C:\Users\` -> `["c:", "users"]`.
pub(crate) fn segments(path: &str) -> Vec<String> {
    path.split(['\\', '/']).filter(|s| !s.is_empty()).map(|s| s.to_lowercase()).collect()
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
        if f.owner.is_none() {
            f.owner = facts.owner_of(&f.path).map(|a| a.name.clone());
        }
    }
    let mut seen: std::collections::HashSet<String> = found.iter().map(|f| f.path.to_string_lossy().to_lowercase()).collect();
    let projects = projects::find(tree);
    let extra = facts::unused_apps(tree, facts, UNUSED_DAYS, 1 << 30)
        .into_iter()
        .chain(projects::build_outputs(tree, &projects, facts, PROJECT_IDLE_DAYS, 100 << 20))
        .chain(anomaly::find(tree))
        .chain(remedies::compact(tree))
        .chain(remedies::onedrive(tree))
        .chain(remedies::moves(tree, &hidden::fixed_drives()));
    for f in extra {
        if seen.insert(f.path.to_string_lossy().to_lowercase()) {
            found.push(f);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
        let with_image = Finding { bytes: t2.nodes[n].size, ..good.clone() };
        assert!(matches!(safety::check(&with_image, &t2, &never), Err(safety::Rejection::Never(_))));
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
