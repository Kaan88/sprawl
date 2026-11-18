//! F2: older versions side by side. Several installed versions of one toolchain, the older ones
//! offered for removal through the toolchain's own command (`rustup toolchain uninstall`,
//! `nvm uninstall`, `fnm uninstall`), never a folder delete. Never the default or active one,
//! never the newest, never one a project in the scan asks for (`rust-toolchain.toml`, `.nvmrc`,
//! `.node-version`), nor one rustup keeps for a folder (an override).

use std::path::{Path, PathBuf};

use sprawl_core::{format_size, Tree};

use crate::finding::{Action, Finding, Remedy, Risk, Source};
use crate::rules::ps_quote;

/// A version name Sprawl puts in a command: letters, digits, dots, dashes and underscores only.
fn plain(name: &str) -> bool {
    !name.is_empty() && name.len() <= 80 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn child(tree: &Tree, n: usize, name: &str) -> Option<usize> {
    tree.nodes[n].children.iter().copied().find(|&c| tree.nodes[c].name.eq_ignore_ascii_case(name))
}

fn dirs(tree: &Tree, n: usize) -> Vec<usize> {
    tree.nodes[n].children.iter().copied().filter(|&c| tree.nodes[c].is_dir && !tree.nodes[c].is_reparse_point()).collect()
}

/// `1.99.1` / `v18.17.0` / `18` as numbers, for "newest"; None when it isn't a version.
fn numbers(v: &str) -> Option<Vec<u64>> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let parts: Option<Vec<u64>> = v.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

/// What projects in the scan ask for: (the file's folder, the version text).
#[derive(Default)]
pub struct Pins {
    pub rust: Vec<(PathBuf, String)>,
    pub node: Vec<(PathBuf, String)>,
}

/// Read the version files at the top of each project the engine found. They are a few bytes
/// each; a file that can't be read pins nothing.
pub fn pins(tree: &Tree, projects: &[crate::projects::Project]) -> Pins {
    let mut p = Pins::default();
    for pr in projects {
        let dir = tree.path_of(pr.node);
        for (name, rust) in [("rust-toolchain.toml", true), ("rust-toolchain", true), (".nvmrc", false), (".node-version", false)] {
            if child(tree, pr.node, name).is_none() {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(dir.join(name)) else { continue };
            let text: String = text.chars().take(2000).collect();
            let v = if name == "rust-toolchain.toml" {
                // channel = "1.99.1"
                text.lines().find_map(|l| {
                    let (k, v) = l.split_once('=')?;
                    (k.trim() == "channel").then(|| v.trim().trim_matches(['"', '\'']).to_string())
                })
            } else {
                text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string)
            };
            if let Some(v) = v.filter(|v| !v.is_empty()) {
                if rust { &mut p.rust } else { &mut p.node }.push((dir.clone(), v));
            }
        }
    }
    p
}

fn finding(tree: &Tree, n: usize, id: &str, owner: &str, title: String, evidence: Vec<String>, command: String) -> Finding {
    let mut f = Finding {
        path: tree.path_of(n),
        owner: Some(owner.into()),
        bytes: tree.nodes[n].size,
        risk: Risk::Amber,
        remedy: Remedy::Command,
        command: None,
        title,
        evidence,
        confidence: 0.8,
        source: Source::Rule(format!("versions.{id}")),
        allow_protected: false,
        trusted: true,
        builtin: true,
        on_path: false,
        action: None,
        admin: false,
        carved: Vec::new(),
        gain: None,
        forced_from: None,
    };
    f.set_action(Action::Tool { command }, false);
    f
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

/// Everything F2 finds in `tree` (empty when the toolchains' folders aren't in it).
pub fn find(tree: &Tree, projects: &[crate::projects::Project]) -> Vec<Finding> {
    let pins = pins(tree, projects);
    let mut out = rust(tree, &pins);
    out.extend(node(tree, &pins));
    out
}

/// rustup's toolchains: a numbered (`1.99.1-...`) or dated (`nightly-2025-01-01-...`)
/// toolchain that isn't the default, an override or a project's pin. Channels (`stable`,
/// `beta`, `nightly`) update in place and are left alone.
fn rust(tree: &Tree, pins: &Pins) -> Vec<Finding> {
    let rustup = std::env::var_os("RUSTUP_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".rustup")));
    let Some(rustup) = rustup else { return Vec::new() };
    let Some(tc) = crate::find_path(tree, &rustup.join("toolchains")) else { return Vec::new() };
    let settings = std::fs::read_to_string(rustup.join("settings.toml")).unwrap_or_default();
    rust_from(tree, tc, &settings, pins)
}

fn rust_from(tree: &Tree, tc: usize, settings: &str, pins: &Pins) -> Vec<Finding> {
    // default_toolchain = "stable-x86_64-pc-windows-msvc"; [overrides] "C:\\x" = "1.99.1-..."
    let mut kept: Vec<(String, String)> = Vec::new();
    let mut in_overrides = false;
    for l in settings.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            in_overrides = l == "[overrides]";
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let v = v.trim().trim_matches('"').to_string();
        if k.trim() == "default_toolchain" {
            kept.push((v, "your default".into()));
        } else if in_overrides {
            kept.push((v, format!("set for {}", k.trim().trim_matches('"').replace("\\\\", "\\"))));
        }
    }
    let names: Vec<(usize, String)> = dirs(tree, tc).into_iter().map(|n| (n, tree.nodes[n].name.clone())).collect();
    let all: Vec<&str> = names.iter().map(|(_, s)| s.as_str()).collect();
    let mut out = Vec::new();
    for (n, name) in &names {
        let channel = name.split('-').next().unwrap_or("");
        let numbered = channel.starts_with(|c: char| c.is_ascii_digit()) && numbers(channel).is_some();
        let dated = matches!(channel, "nightly" | "beta") && name.split('-').nth(1).is_some_and(|y| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()));
        if !(numbered || dated) || !plain(name) {
            continue;
        }
        let matches = |want: &str| name == want || name.starts_with(&format!("{want}-"));
        if kept.iter().any(|(k, _)| matches(k)) {
            continue;
        }
        if pins.rust.iter().any(|(_, v)| matches(v)) {
            continue;
        }
        let others: Vec<&str> = all.iter().copied().filter(|o| o != name).collect();
        out.push(finding(
            tree,
            *n,
            "rust",
            "Rust",
            format!("Rust toolchain {channel}{}", if dated { format!(" {}", name.splitn(3, '-').nth(1).unwrap_or("")) } else { String::new() }),
            vec![
                format!("An older Rust toolchain beside {}. Nothing in this scan asks for it: no rust-toolchain file names it and it isn't your default.", others.join(", ")),
                format!("Tick it and Clean up, and Sprawl runs rustup's own 'rustup toolchain uninstall {name}' ({}). Rust stays installed.", format_size(tree.nodes[*n].size)),
                "A project outside this scan that asks for it gets it back from rustup on its next build.".into(),
            ],
            format!("rustup toolchain uninstall {}", ps_quote(name)),
        ));
    }
    out
}

/// Node versions kept by nvm for Windows (`%APPDATA%\nvm\v18.17.0`) and fnm
/// (`%APPDATA%\fnm\node-versions\v18.17.0`): not the one in use, not the newest, not a pin.
fn node(tree: &Tree, pins: &Pins) -> Vec<Finding> {
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    let mut out = Vec::new();
    // nvm: the version in use is where its link (NVM_SYMLINK) points
    let nvm_home = std::env::var_os("NVM_HOME").map(PathBuf::from).or_else(|| appdata.as_ref().map(|a| a.join("nvm")));
    if let Some(root) = nvm_home.as_ref().and_then(|h| crate::find_path(tree, h)) {
        let link = std::env::var_os("NVM_SYMLINK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_default()).join("nodejs"));
        let active = std::fs::read_link(&link).ok();
        out.extend(node_from(tree, root, active.as_deref(), pins, "nvm", "nvm for Windows"));
    }
    // fnm: the default is the alias link aliases\default
    let fnm_dir = std::env::var_os("FNM_DIR").map(PathBuf::from).or_else(|| appdata.as_ref().map(|a| a.join("fnm")));
    if let Some(dir) = fnm_dir {
        if let Some(root) = crate::find_path(tree, &dir.join("node-versions")) {
            let active = std::fs::read_link(dir.join("aliases").join("default")).ok();
            out.extend(node_from(tree, root, active.as_deref(), pins, "fnm", "fnm"));
        }
    }
    out
}

fn node_from(tree: &Tree, root: usize, active: Option<&Path>, pins: &Pins, tool: &str, tool_name: &str) -> Vec<Finding> {
    let versions: Vec<(usize, String, Vec<u64>)> = dirs(tree, root)
        .into_iter()
        .filter_map(|n| {
            let name = tree.nodes[n].name.clone();
            let v = numbers(&name)?;
            (name.starts_with(['v', 'V']) && plain(&name)).then_some((n, name, v))
        })
        .collect();
    if versions.len() < 2 {
        return Vec::new();
    }
    let newest = versions.iter().map(|v| &v.2).max().cloned().unwrap_or_default();
    // the active one: the link names its folder (or a folder inside it)
    let is_active = |n: usize| active.is_some_and(|a| crate::finding::within(a, &tree.path_of(n)));
    let pinned = |v: &[u64]| {
        pins.node.iter().any(|(_, p)| {
            let Some(want) = numbers(p) else { return false };
            // "18" pins every 18.x; "18.17.0" only that one
            v.len() >= want.len() && v[..want.len()] == want[..]
        })
    };
    let all: Vec<&str> = versions.iter().map(|v| v.1.as_str()).collect();
    let mut out = Vec::new();
    for (n, name, v) in &versions {
        if *v == newest || is_active(*n) || pinned(v) {
            continue;
        }
        let bare = name.trim_start_matches(['v', 'V']);
        let others: Vec<&str> = all.iter().copied().filter(|o| o != name).collect();
        out.push(finding(
            tree,
            *n,
            tool,
            "Node.js",
            format!("Node.js {name} ({tool_name})"),
            vec![
                format!("An older Node.js beside {}. It isn't the one in use, and no .nvmrc or .node-version in this scan asks for it.", others.join(", ")),
                format!("Tick it and Clean up, and Sprawl runs '{tool} uninstall {bare}' ({}). Its global packages go with it.", format_size(tree.nodes[*n].size)),
            ],
            format!("{tool} uninstall {}", ps_quote(bare)),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use sprawl_core::ROOT;
    use super::*;

    fn tree_with(tag: &str, dirs: &[&str]) -> (PathBuf, Tree) {
        let base = std::env::temp_dir().join(format!("sprawl-versions-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for d in dirs {
            let p = base.join(d);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("f.bin"), vec![0u8; 1000]).unwrap();
        }
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        (base, tree)
    }

    #[test]
    fn rust_keeps_the_default_channels_overrides_and_pins() {
        let (base, tree) = tree_with("rust", &[
            "stable-x86_64-pc-windows-msvc",
            "1.99.1-x86_64-pc-windows-msvc",
            "1.80.0-x86_64-pc-windows-msvc",
            "1.70.0-x86_64-pc-windows-msvc",
            "nightly-2025-01-01-x86_64-pc-windows-msvc",
            "nightly-x86_64-pc-windows-msvc",
        ]);
        let settings = "default_toolchain = \"stable-x86_64-pc-windows-msvc\"\n[overrides]\n\"C:\\\\work\\\\old\" = \"1.80.0-x86_64-pc-windows-msvc\"\n";
        let pins = Pins { rust: vec![(PathBuf::from(r"C:\p"), "1.70.0".into())], node: vec![] };
        let f = rust_from(&tree, ROOT, settings, &pins);
        let names: Vec<String> = f.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.contains(&"1.99.1-x86_64-pc-windows-msvc".to_string()));
        assert!(names.contains(&"nightly-2025-01-01-x86_64-pc-windows-msvc".to_string()));
        let one = f.iter().find(|f| f.path.ends_with("1.99.1-x86_64-pc-windows-msvc")).unwrap();
        assert_eq!(one.risk, Risk::Amber);
        assert!(matches!(&one.action, Some(Action::Tool { command }) if command == "rustup toolchain uninstall '1.99.1-x86_64-pc-windows-msvc'"));
        assert!(crate::clean::action(one).is_some());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn node_keeps_the_newest_the_active_and_pins() {
        let (base, tree) = tree_with("node", &["v16.20.0", "v18.17.0", "v18.19.1", "v20.11.0", "v14.0.0", "not-a-version"]);
        let active = base.join("v14.0.0");
        let pins = Pins { rust: vec![], node: vec![(PathBuf::from(r"C:\p"), "18".into())] };
        let f = node_from(&tree, ROOT, Some(&active), &pins, "nvm", "nvm for Windows");
        let names: Vec<String> = f.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        // v20 newest, v14 active, both v18 pinned by "18": only v16 is offered
        assert_eq!(names, vec!["v16.20.0".to_string()]);
        assert!(matches!(&f[0].action, Some(Action::Tool { command }) if command == "nvm uninstall '16.20.0'"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A project's rust-toolchain.toml and .nvmrc, found by the engine's own project finder.
    #[test]
    fn a_project_in_the_scan_pins_its_versions() {
        // not in %TEMP%: the project finder leaves everything under AppData alone
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join(format!("test-versions-pins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let p = base.join("app");
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Cargo.toml"), "[package]\nname = \"app\"\n").unwrap();
        std::fs::write(p.join("src").join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(p.join("rust-toolchain.toml"), "[toolchain]\nchannel = \"1.99.1\"\ncomponents = [\"clippy\"]\n").unwrap();
        std::fs::write(p.join("package.json"), "{}").unwrap();
        std::fs::write(p.join(".nvmrc"), "v18\n").unwrap();
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let projects = crate::projects::find(&tree);
        let pins = pins(&tree, &projects);
        assert_eq!(pins.rust.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), vec!["1.99.1"]);
        assert_eq!(pins.node.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), vec!["v18"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn odd_names_are_never_put_in_a_command() {
        assert!(plain("1.99.1-x86_64-pc-windows-msvc"));
        assert!(!plain("1.0'; rm -r x"));
        assert!(!plain("a b"));
        assert_eq!(numbers("v18.17.0"), Some(vec![18, 17, 0]));
        assert_eq!(numbers("stable"), None);
    }
}
