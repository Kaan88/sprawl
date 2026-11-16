//! Code projects in the tree, found by their marker files (`Cargo.toml`, `package.json`,
//! `.git`, ...), with their build outputs and when their sources last changed.
//!
//! A build output is only suggested when its project hasn't been touched for a while: deleting
//! the `target` of the project you are working on just costs a rebuild for nothing.

use std::time::SystemTime;

use sprawl_core::{Tree, ROOT};

use crate::facts::month_year;
use crate::finding::{Finding, Remedy, Risk, Source};
use crate::rules::ps_quote;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rust,
    Node,
    DotNet,
    Gradle,
    Maven,
    Python,
    CMake,
    Go,
    /// Only a `.git` folder.
    Git,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Rust => "Rust",
            Kind::Node => "Node.js",
            Kind::DotNet => ".NET",
            Kind::Gradle => "Gradle",
            Kind::Maven => "Maven",
            Kind::Python => "Python",
            Kind::CMake => "CMake",
            Kind::Go => "Go",
            Kind::Git => "Git",
        }
    }

    /// Output folders this kind of project makes, how they come back, and their risk.
    fn outputs(self) -> &'static [(&'static str, &'static str, Risk)] {
        match self {
            Kind::Rust => &[("target", "the next cargo build", Risk::Green)],
            Kind::Node => &[
                ("node_modules", "npm install (or pnpm/yarn install), which needs the network", Risk::Amber),
                (".next", "the next build", Risk::Green),
                (".nuxt", "the next build", Risk::Green),
                (".parcel-cache", "the next build", Risk::Green),
                (".turbo", "the next build", Risk::Green),
                (".angular", "the next build", Risk::Green),
            ],
            Kind::DotNet => &[("bin", "the next build", Risk::Green), ("obj", "the next build", Risk::Green)],
            Kind::Gradle => &[("build", "the next Gradle build", Risk::Green), (".gradle", "the next Gradle build", Risk::Green)],
            Kind::Maven => &[("target", "the next Maven build", Risk::Green)],
            Kind::Python => &[
                (".venv", "recreating the virtual environment and reinstalling its packages", Risk::Amber),
                ("venv", "recreating the virtual environment and reinstalling its packages", Risk::Amber),
                (".tox", "the next tox run", Risk::Green),
                (".mypy_cache", "the next mypy run", Risk::Green),
                (".pytest_cache", "the next pytest run", Risk::Green),
            ],
            Kind::CMake => &[("build", "running CMake and building again", Risk::Green)],
            Kind::Go | Kind::Git => &[],
        }
    }
}

/// What marks a folder as a project of some kind (lower-case names).
fn kind_of(name: &str) -> Option<Kind> {
    Some(match name {
        "cargo.toml" => Kind::Rust,
        "package.json" => Kind::Node,
        "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts" => Kind::Gradle,
        "pom.xml" => Kind::Maven,
        "pyproject.toml" | "setup.py" | "requirements.txt" | "pipfile" => Kind::Python,
        "cmakelists.txt" => Kind::CMake,
        "go.mod" => Kind::Go,
        n if n.ends_with(".csproj") || n.ends_with(".fsproj") || n.ends_with(".vbproj") || n.ends_with(".sln") => Kind::DotNet,
        _ => return None,
    })
}

/// Folders never searched for projects: they hold other people's packages or our own output.
const SKIP: &[&str] = &["node_modules", ".git", "target", "bin", "obj", ".venv", "venv", "site-packages", "$recycle.bin", "windows", "appdata"];

#[derive(Clone, Debug)]
pub struct Project {
    pub node: usize,
    pub kinds: Vec<Kind>,
    pub has_git: bool,
    /// Newest change among the project's own files (not its outputs, not `.git`'s objects).
    pub last_change: Option<SystemTime>,
    /// (output folder node, how it comes back, risk)
    pub outputs: Vec<(usize, &'static str, Risk)>,
}

impl Project {
    pub fn output_bytes(&self, tree: &Tree) -> u64 {
        self.outputs.iter().map(|&(n, _, _)| tree.nodes[n].size).sum()
    }
}

/// Every project in the tree. Projects inside another project's output (packages in
/// `node_modules`) aren't projects; nested projects (a workspace's members) are.
pub fn find(tree: &Tree) -> Vec<Project> {
    let mut out = Vec::new();
    let mut stack = vec![ROOT];
    while let Some(n) = stack.pop() {
        let node = &tree.nodes[n];
        if !node.is_dir || (n != ROOT && node.is_reparse_point()) {
            continue;
        }
        let mut kinds: Vec<Kind> = Vec::new();
        let mut has_git = false;
        for &c in &node.children {
            let name = tree.nodes[c].name.to_lowercase();
            if name == ".git" {
                has_git = true;
            } else if !tree.nodes[c].is_dir {
                if let Some(k) = kind_of(&name) {
                    if !kinds.contains(&k) {
                        kinds.push(k);
                    }
                }
            }
        }
        if has_git && kinds.is_empty() {
            kinds.push(Kind::Git);
        }
        let mut output_nodes = Vec::new();
        if !kinds.is_empty() {
            let mut outputs = Vec::new();
            for &c in &node.children {
                let child = &tree.nodes[c];
                if !child.is_dir || child.is_reparse_point() {
                    continue;
                }
                let name = child.name.to_lowercase();
                for k in &kinds {
                    if let Some(&(_, how, risk)) = k.outputs().iter().find(|(o, _, _)| *o == name) {
                        if !outputs.iter().any(|&(o, _, _)| o == c) {
                            outputs.push((c, how, risk));
                            output_nodes.push(c);
                        }
                    }
                }
            }
            out.push(Project { node: n, kinds, has_git, last_change: last_change(tree, n, &output_nodes), outputs });
        }
        for &c in &node.children {
            let name = tree.nodes[c].name.to_lowercase();
            if tree.nodes[c].is_dir && !SKIP.contains(&name.as_str()) && !output_nodes.contains(&c) {
                stack.push(c);
            }
        }
    }
    out
}

/// Newest modification among the files of project `n`, leaving out its outputs and `.git`
/// (except `.git\logs\HEAD`, which changes on every commit and checkout).
fn last_change(tree: &Tree, n: usize, outputs: &[usize]) -> Option<SystemTime> {
    let mut best: Option<SystemTime> = None;
    let mut stack = vec![n];
    let mut seen = 0usize;
    while let Some(k) = stack.pop() {
        seen += 1;
        if seen > 2_000_000 {
            break;
        }
        let node = &tree.nodes[k];
        if node.is_dir {
            if k != n && node.is_reparse_point() {
                continue;
            }
            for &c in &node.children {
                if outputs.contains(&c) {
                    continue;
                }
                let child = &tree.nodes[c];
                let lname = child.name.to_lowercase();
                if child.is_dir && lname == ".git" {
                    let head = child.children.iter().find(|&&g| tree.nodes[g].name.eq_ignore_ascii_case("logs")).and_then(|&l| {
                        tree.nodes[l].children.iter().find(|&&h| tree.nodes[h].name.eq_ignore_ascii_case("head")).copied()
                    });
                    if let Some(t) = head.and_then(|h| tree.nodes[h].modified) {
                        best = best.max(Some(t));
                    }
                    continue;
                }
                if child.is_dir && SKIP.contains(&lname.as_str()) {
                    continue;
                }
                stack.push(c);
            }
        } else if let Some(t) = node.modified {
            best = best.max(Some(t));
        }
    }
    best
}

/// Build outputs of projects unchanged for `days`, each at least `min_bytes`.
pub fn build_outputs(tree: &Tree, projects: &[Project], days: u64, min_bytes: u64) -> Vec<Finding> {
    let now = SystemTime::now();
    let mut out = Vec::new();
    for p in projects {
        if p.outputs.is_empty() {
            continue;
        }
        // an unknown date is not "old"
        let Some(last) = p.last_change else { continue };
        let Ok(age) = now.duration_since(last) else { continue };
        if age.as_secs() < days * 86_400 {
            continue;
        }
        let project_name = tree.nodes[p.node].name.clone();
        let total = tree.nodes[p.node].size;
        let kinds: Vec<&str> = p.kinds.iter().map(|k| k.label()).collect();
        for &(n, how, risk) in &p.outputs {
            let node = &tree.nodes[n];
            if node.size < min_bytes {
                continue;
            }
            let path = tree.path_of(n);
            let source = total.saturating_sub(p.output_bytes(tree));
            let mut evidence = vec![
                format!("Build output of the {} project {project_name}", kinds.join(" / ")),
                format!("The project's own files last changed in {}", month_year(last)),
                format!("Comes back with {how}"),
            ];
            if source > 0 {
                evidence.push(format!("{:.0}x the size of the project's own files", node.size as f64 / source as f64));
            }
            if node.is_dir {
                evidence.push(format!("{} files in {} folders", node.files, node.folders));
            }
            out.push(Finding {
                command: Some(format!("Remove-Item -LiteralPath {} -Recurse -Force", ps_quote(&path.display().to_string()))),
                path,
                owner: Some(project_name.clone()),
                bytes: node.size,
                risk,
                remedy: Remedy::Delete,
                title: format!("{} in {project_name}", node.name),
                evidence,
                confidence: 0.9,
                source: Source::Rule("project.build-output".into()),
                allow_protected: false,
                trusted: true,
                builtin: true,
                on_path: true,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_projects_and_outputs() {
        let dir = std::env::temp_dir().join(format!("sprawl-proj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let app = dir.join("app");
        std::fs::create_dir_all(app.join("target").join("release")).unwrap();
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::create_dir_all(app.join("node_modules").join("leftpad")).unwrap();
        std::fs::write(app.join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(app.join("package.json"), "{}").unwrap();
        std::fs::write(app.join("src").join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(app.join("target").join("release").join("app.exe"), vec![0u8; 5000]).unwrap();
        // a package inside node_modules is not a project
        std::fs::write(app.join("node_modules").join("leftpad").join("package.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.join("notes")).unwrap();
        std::fs::write(dir.join("notes").join("a.txt"), "x").unwrap();

        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let ps = find(&tree);
        assert_eq!(ps.len(), 1, "{ps:?}");
        let p = &ps[0];
        assert_eq!(tree.nodes[p.node].name, "app");
        assert!(p.kinds.contains(&Kind::Rust) && p.kinds.contains(&Kind::Node));
        let outs: Vec<&str> = p.outputs.iter().map(|&(n, _, _)| tree.nodes[n].name.as_str()).collect();
        assert!(outs.contains(&"target") && outs.contains(&"node_modules"));
        assert!(p.last_change.is_some());
        // just written, so not old enough to suggest
        assert!(build_outputs(&tree, &ps, 30, 0).is_empty());
        let f = build_outputs(&tree, &ps, 0, 1000);
        assert_eq!(f.len(), 1, "only target is over 1000 bytes");
        assert!(f[0].command.as_deref().unwrap().contains("Remove-Item -LiteralPath '"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
