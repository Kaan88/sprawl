//! A2: folders in a Steam-style common directory absent from its current, readable manifests.
//! Absence does not prove uninstall (a manual/portable game may live here), so information
//! only. Mods and saves may remain; nothing is ticked, planned or scripted for the user.

use std::collections::HashSet;
use std::path::Path;
use crate::facts::Facts;
use crate::finding::{Finding, Remedy, Risk, Source, within};
use sprawl_core::{Tree, ROOT};

pub const ID: &str = "kind.game-leftover";

/// All manifests must be readable, plain and valid. At least one valid manifest is required
/// to identify the library's layout; a folder named steamapps alone proves nothing.
fn installed_dirs(root: &Path) -> Option<HashSet<String>> {
    let rd = std::fs::read_dir(root).ok()?;
    let mut dirs = HashSet::new();
    let mut manifests = 0;
    for (count, entry) in rd.enumerate() {
        if count >= 10_000 { return None; }
        let e = entry.ok()?;
        let name = e.file_name().to_string_lossy().to_lowercase();
        let Some(id) = name.strip_prefix("appmanifest_").and_then(|s| s.strip_suffix(".acf")) else { continue };
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) { return None; }
        let m = std::fs::symlink_metadata(e.path()).ok()?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & (0x400 | crate::dupes::NOT_LOCAL) != 0 { return None; }
        }
        if !m.is_file() || m.len() > 1 << 20 { return None; }
        let text = std::fs::read_to_string(e.path()).ok()?;
        let pairs = crate::facts::steam::pairs(&text);
        if pairs.get("appid").map(String::as_str) != Some(id) { return None; }
        let app = crate::facts::steam::from_manifest(root, &text)?;
        let folder = app.locations.first()?.file_name()?.to_str()?.to_lowercase();
        dirs.insert(folder);
        manifests += 1;
    }
    (manifests > 0).then_some(dirs)
}

pub fn find(tree: &Tree, facts: &Facts, min: u64) -> Vec<Finding> {
    if tree.method == sprawl_core::ScanMethod::Csv { return Vec::new(); }
    let mut out = Vec::new();
    let mut stack = vec![ROOT];
    let mut libraries = 0;
    while let Some(n) = stack.pop() {
        let node = &tree.nodes[n];
        if !node.is_dir || node.is_reparse_point() || node.attributes & crate::dupes::NOT_LOCAL != 0 { continue; }
        if n != ROOT && node.name.contains(['\\', '/', ':']) { continue; }
        stack.extend_from_slice(&node.children);
        let root = tree.path_of(n);
        if !root.file_name().is_some_and(|s| s.eq_ignore_ascii_case("common")) { continue; }
        let Some(steamapps) = root.parent().filter(|p| p.file_name().is_some_and(|s| s.eq_ignore_ascii_case("steamapps"))) else { continue };
        libraries += 1;
        if libraries > 32 { break; }
        let Some(installed) = installed_dirs(steamapps) else { continue };
        for &c in &node.children {
            let child = &tree.nodes[c];
            if !child.is_dir || child.is_reparse_point() || child.attributes & crate::dupes::NOT_LOCAL != 0
                || child.size < min || child.name.contains(['\\', '/', ':'])
                || child.name.starts_with('.') || child.name.eq_ignore_ascii_case("Steamworks Shared")
                || installed.contains(&child.name.to_lowercase()) { continue; }
            let path = tree.path_of(c);
            // Check all owners, including other launchers that manage a game in this library.
            if facts.apps.iter().any(|a| a.locations.iter().any(|l| within(l, &path) || within(&path, l))) { continue; }
            out.push(Finding {
                path, owner: None, bytes: child.size, risk: Risk::Orange, remedy: Remedy::Info,
                command: None, title: format!("{} (not registered in this Steam library)", child.name),
                evidence: vec![
                    format!("Checked the library's current manifests; {} registered install folders, none names this folder", installed.len()),
                    "This may be what an uninstalled game left, or a manually managed game: absence from Steam is not proof it is unused".into(),
                    "Look inside for mods, saved games and files you kept before deciding. This suggestion removes nothing".into(),
                    "For registered games, use the launcher's own Manage > Uninstall; manage saved games separately".into(),
                ],
                confidence: 0.5, source: Source::Rule(ID.into()), allow_protected: false,
                trusted: true, builtin: true, on_path: false, action: None, admin: false,
                carved: Vec::new(), gain: None, forced_from: None,
            });
        }
    }
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out.truncate(50);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_complete_manifests_and_other_owners_keep_games_out() {
        let base = std::env::temp_dir().join(format!("sprawl-game-leftovers-{}", std::process::id()));
        let root = base.join("steamapps");
        for folder in ["Installed", "Remains", "OtherLauncher", "Steamworks Shared"] {
            std::fs::create_dir_all(root.join("common").join(folder)).unwrap();
            std::fs::write(root.join("common").join(folder).join("mod.bin"), b"keep mods").unwrap();
        }
        let manifest = root.join("appmanifest_123.acf");
        std::fs::write(&manifest, r#""AppState" { "appid" "123" "name" "A game" "installdir" "Installed" }"#).unwrap();
        let facts = Facts::from_apps(vec![crate::facts::App {
            name: "Other game".into(), publisher: None, kind: crate::facts::AppKind::Epic,
            locations: vec![root.join("common/OtherLauncher")], last_used: None, how_to_remove: "Epic".into(),
        }]);
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let found = find(&tree, &facts, 1);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].path.ends_with("Remains"));
        assert!(found[0].heuristic() && found[0].leftover());
        assert_eq!(found[0].remedy, Remedy::Info);
        assert!(found[0].action.is_none() && found[0].command.is_none());
        std::fs::write(root.join("appmanifest_456.acf"), "unreadable as a manifest").unwrap();
        assert!(find(&tree, &facts, 1).is_empty(), "a broken manifest is uncertainty, not an uninstall");
        std::fs::remove_file(root.join("appmanifest_456.acf")).unwrap();
        std::fs::remove_file(manifest).unwrap();
        assert!(find(&tree, &facts, 1).is_empty(), "a bare folder name doesn't establish a Steam library");
        assert_eq!(std::fs::read(root.join("common/Remains/mod.bin")).unwrap(), b"keep mods");
        std::fs::remove_dir_all(base).unwrap();
    }
}
