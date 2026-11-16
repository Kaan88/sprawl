//! disktree-advisor: finds space that can be won back in a scanned tree, and explains it.
//!
//! See `docs/plans/ai-advisor.md`. Rust finds, a model (later) judges; nothing here deletes
//! anything: the output is [`Finding`]s with a remedy and a command for the user to run.
//!
//! * [`finding`]: the schema every finding follows, from rules and (later) from a model.
//! * [`rules`]: TOML rule packs and the engine that matches them against a tree.
//! * [`safety`]: the never-touch list and the checks every finding must pass.
//! * [`snapshot`]: a compact, compressed copy of a scan, for history and diffs.

pub mod finding;
pub mod rules;
pub mod safety;
pub mod snapshot;

pub use finding::{Finding, Remedy, Risk, Source};
pub use rules::{Rule, RulePack};

use std::path::Path;

use disktree_core::{Tree, MULTIPLE, ROOT};

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
        let tree = disktree_core::scan(&dir, |_| {}).unwrap();
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
        assert!(f.command.as_deref().unwrap().contains("Remove-Item"));
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
        let windows = disktree_core::scan(Path::new(r"C:\Windows\Web"), |_| {}).unwrap();
        let model = Finding { path: windows.path_of(ROOT), bytes: windows.nodes[ROOT].size, remedy: Remedy::Command, source: Source::Model("m".into()), ..good.clone() };
        assert!(matches!(safety::check(&model, &windows, &never), Err(safety::Rejection::Never(_))));
        let rule = Finding { source: Source::Rule("r".into()), ..model.clone() };
        assert!(safety::check(&rule, &windows, &never).is_ok(), "curated command on a protected path is allowed");
        let delete = Finding { remedy: Remedy::Delete, ..rule };
        assert!(matches!(safety::check(&delete, &windows, &never), Err(safety::Rejection::Never(_))));

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
