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

    /// Output folders this kind of project makes, how they come back, and their risk. A folder
    /// with one of these names is only taken for an output when [`proven`] finds the files its
    /// tool leaves there.
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
            // release builds (signed APKs, mapping files) can live in build
            Kind::Gradle => &[("build", "the next Gradle build", Risk::Amber), (".gradle", "the next Gradle build", Risk::Green)],
            Kind::Maven => &[("target", "the next Maven build", Risk::Green)],
            Kind::Python => &[
                (".venv", "recreating the virtual environment and reinstalling its packages", Risk::Amber),
                ("venv", "recreating the virtual environment and reinstalling its packages", Risk::Amber),
                (".mypy_cache", "the next mypy run", Risk::Green),
                (".pytest_cache", "the next pytest run", Risk::Green),
            ],
            // its cache keeps the options it was configured with, and _deps downloads
            Kind::CMake => &[("build", "running CMake with the same options and building again", Risk::Amber)],
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

const LOCKFILES: &[&str] = &["package-lock.json", "npm-shrinkwrap.json", "yarn.lock", "pnpm-lock.yaml", "bun.lockb", "bun.lock"];

fn child(tree: &Tree, n: usize, name: &str) -> Option<usize> {
    tree.nodes[n].children.iter().copied().find(|&c| tree.nodes[c].name.eq_ignore_ascii_case(name))
}

fn has(tree: &Tree, n: usize, name: &str) -> bool {
    child(tree, n, name).is_some()
}

/// Whether folder `o`, named `name` in project folder `p`, holds what `kind`'s tool leaves in
/// its output. Only then is it suggested: a `bin` or `build` folder may hold real files.
fn proven(tree: &Tree, p: usize, o: usize, name: &str, kind: Kind) -> bool {
    let project_file = || tree.nodes[p].children.iter().any(|&c| {
        let n = tree.nodes[c].name.to_lowercase();
        !tree.nodes[c].is_dir && (n.ends_with(".csproj") || n.ends_with(".fsproj") || n.ends_with(".vbproj"))
    });
    let lockfile = || LOCKFILES.iter().any(|l| has(tree, p, l));
    match (kind, name) {
        (Kind::Rust, "target") => has(tree, o, "CACHEDIR.TAG") || has(tree, o, ".rustc_info.json"),
        (Kind::Maven, "target") => has(tree, o, "maven-status") || has(tree, o, "classes") && has(tree, p, "pom.xml"),
        (Kind::CMake, "build") => has(tree, o, "CMakeCache.txt"),
        (Kind::Gradle, "build") => has(tree, o, "tmp") && ["classes", "libs", "intermediates", "generated", "kotlin"].iter().any(|d| has(tree, o, d)),
        (Kind::Gradle, ".gradle") => has(tree, o, "buildOutputCleanup") || has(tree, o, "file-system.probe"),
        (Kind::DotNet, "obj") => project_file() && has(tree, o, "project.assets.json"),
        (Kind::DotNet, "bin") => {
            let obj_ok = child(tree, p, "obj").is_some_and(|obj| has(tree, obj, "project.assets.json"));
            // only configuration folders inside: nothing hand-placed
            let configs = !tree.nodes[o].children.is_empty()
                && tree.nodes[o].children.iter().all(|&c| {
                    tree.nodes[c].is_dir && ["debug", "release", "x64", "x86", "arm64", "anycpu"].contains(&tree.nodes[c].name.to_lowercase().as_str())
                });
            project_file() && obj_ok && configs
        }
        (Kind::Python, ".venv" | "venv") => has(tree, o, "pyvenv.cfg"),
        (Kind::Python, ".mypy_cache" | ".pytest_cache") => has(tree, o, "CACHEDIR.TAG"),
        (Kind::Node, "node_modules") => {
            lockfile() && [".package-lock.json", ".modules.yaml", ".yarn-integrity", ".yarn-state.yml"].iter().any(|m| has(tree, o, m))
        }
        (Kind::Node, ".next") => lockfile() && (has(tree, o, "BUILD_ID") || has(tree, o, "cache")),
        (Kind::Node, _) => lockfile(),
        _ => false,
    }
}

/// A drive, a profile or one of its top folders (Documents, Desktop, ...), a hidden tool folder
/// in a profile (`.vscode`, `.cargo`) or anything in AppData: a stray marker file there doesn't
/// make it a project whose folders can go.
fn plain_folder(tree: &Tree, n: usize) -> bool {
    let segs = crate::segments(&tree.path_of(n).to_string_lossy());
    if segs.len() <= 1 {
        return true;
    }
    if segs[1] == "users" {
        return segs.len() <= 4 || segs[3].starts_with('.') || segs.iter().any(|s| s == "appdata");
    }
    false
}

#[derive(Clone, Debug)]
pub struct Project {
    pub node: usize,
    pub kinds: Vec<Kind>,
    pub has_git: bool,
    /// Newest change among the project's files, including the top of its outputs (a tool run
    /// from `target\release`, or a fresh build). `None` when unknown.
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
        if !kinds.is_empty() && !plain_folder(tree, n) {
            let mut outputs = Vec::new();
            for &c in &node.children {
                let child = &tree.nodes[c];
                if !child.is_dir || child.is_reparse_point() {
                    continue;
                }
                let name = child.name.to_lowercase();
                for &k in &kinds {
                    if let Some(&(_, how, risk)) = k.outputs().iter().find(|(o, _, _)| *o == name) {
                        if !outputs.iter().any(|&(o, _, _)| o == c) && proven(tree, n, c, &name, k) {
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

/// Newest modification in project `n`: its own files (not inside its outputs, `node_modules`
/// or `.git`, except `.git\logs\HEAD`, which changes on every commit and checkout), plus the
/// top three levels of each output. `None` if the project is too big to walk.
fn last_change(tree: &Tree, n: usize, outputs: &[usize]) -> Option<SystemTime> {
    let mut best: Option<SystemTime> = None;
    let mut seen = 0usize;
    let mut stack = vec![n];
    while let Some(k) = stack.pop() {
        seen += 1;
        if seen > 2_000_000 {
            return None;
        }
        let node = &tree.nodes[k];
        if !node.is_dir {
            best = best.max(node.modified);
            continue;
        }
        if k != n && node.is_reparse_point() {
            continue;
        }
        for &c in &node.children {
            if outputs.contains(&c) {
                continue;
            }
            let child = &tree.nodes[c];
            if child.is_dir && child.name.eq_ignore_ascii_case(".git") {
                let head = child.children.iter().find(|&&g| tree.nodes[g].name.eq_ignore_ascii_case("logs")).and_then(|&l| {
                    tree.nodes[l].children.iter().find(|&&h| tree.nodes[h].name.eq_ignore_ascii_case("head")).copied()
                });
                best = best.max(head.and_then(|h| tree.nodes[h].modified));
                continue;
            }
            if child.is_dir && child.name.eq_ignore_ascii_case("node_modules") {
                continue;
            }
            stack.push(c);
        }
    }
    // a fresh build or a tool run from the output keeps the project active
    for &o in outputs {
        let mut stack = vec![(o, 0)];
        while let Some((k, d)) = stack.pop() {
            best = best.max(tree.nodes[k].modified);
            if d < 3 && tree.nodes[k].is_dir && !tree.nodes[k].is_reparse_point() {
                stack.extend(tree.nodes[k].children.iter().map(|&c| (c, d + 1)));
            }
        }
    }
    best
}

/// Build outputs of projects unchanged for `days`, each at least `min_bytes`.
/// `facts` says which folders belong to installed apps (an app shipped with its `node_modules`
/// is not a project) and when programs in the outputs last ran.
pub fn build_outputs(tree: &Tree, projects: &[Project], facts: &crate::facts::Facts, days: u64, min_bytes: u64) -> Vec<Finding> {
    let now = SystemTime::now();
    let mut out = Vec::new();
    for p in projects {
        if p.outputs.is_empty() || facts.owner_of(&tree.path_of(p.node)).is_some() {
            continue;
        }
        // a tool run from target\release keeps its project in use; an unknown date is not "old"
        let ran = p.outputs.iter().filter_map(|&(o, _, _)| facts.last_run_in(tree, o)).max();
        let Some(changed) = p.last_change else { continue };
        let last = changed.max(ran.unwrap_or(changed));
        let Ok(age) = now.duration_since(last) else { continue };
        if age.as_secs() < days.saturating_mul(86_400) {
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
            // programs in it, and no way to know whether they run (Prefetch unreadable without
            // administrator rights): it may be a tool in daily use
            if !facts.runs_complete && facts.has_programs(tree, n) {
                continue;
            }
            let path = tree.path_of(n);
            let source = total.saturating_sub(p.output_bytes(tree));
            let mut evidence = vec![
                format!("Build output of the {} project {project_name}", kinds.join(" / ")),
                format!("The project's own files last changed in {}", month_year(changed)),
                format!("Comes back with {how}"),
            ];
            if let Some(r) = ran.filter(|&r| r > changed) {
                evidence.push(format!("A program in its output last ran in {}", month_year(r)));
            }
            if source > 0 {
                evidence.push(format!("{:.0}x the size of the project's own files", node.size as f64 / source as f64));
            }
            if node.is_dir {
                evidence.push(format!("{} files in {} folders", node.files, node.folders));
            }
            out.push(Finding {
                // removes links inside without following them
                command: Some(format!("{}; sprawl-rm (Get-Item -LiteralPath {} -Force)", crate::rules::RM_FN, ps_quote(&path.display().to_string()))),
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
        // not under %TEMP%: that is in AppData, where nothing counts as a project
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join(format!("sprawl-proj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let app = dir.join("app");
        std::fs::create_dir_all(app.join("target").join("release")).unwrap();
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::create_dir_all(app.join("node_modules").join("leftpad")).unwrap();
        std::fs::write(app.join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(app.join("package.json"), "{}").unwrap();
        // what cargo and npm leave behind: the proof that these are outputs
        std::fs::write(app.join("target").join("CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55").unwrap();
        std::fs::write(app.join("package-lock.json"), "{}").unwrap();
        std::fs::write(app.join("node_modules").join(".package-lock.json"), "{}").unwrap();
        // look-alikes without proof stay: a CMake `build` with no CMakeCache.txt, a .NET `bin`
        // next to a solution only
        let lib = dir.join("lib");
        std::fs::create_dir_all(lib.join("build")).unwrap();
        std::fs::create_dir_all(lib.join("bin")).unwrap();
        std::fs::write(lib.join("CMakeLists.txt"), "project(x)").unwrap();
        std::fs::write(lib.join("lib.sln"), "").unwrap();
        std::fs::write(lib.join("build").join("toolchain.cmake"), vec![1u8; 5000]).unwrap();
        std::fs::write(lib.join("bin").join("vendor.dll"), vec![1u8; 5000]).unwrap();
        std::fs::write(app.join("src").join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(app.join("target").join("release").join("app.exe"), vec![0u8; 5000]).unwrap();
        // a package inside node_modules is not a project
        std::fs::write(app.join("node_modules").join("leftpad").join("package.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.join("notes")).unwrap();
        std::fs::write(dir.join("notes").join("a.txt"), "x").unwrap();

        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let ps = find(&tree);
        assert_eq!(ps.len(), 2, "{ps:?}");
        let lib_p = ps.iter().find(|p| tree.nodes[p.node].name == "lib").unwrap();
        assert!(lib_p.outputs.is_empty(), "no proof, no output");
        let p = ps.iter().find(|p| tree.nodes[p.node].name == "app").unwrap();
        assert_eq!(tree.nodes[p.node].name, "app");
        assert!(p.kinds.contains(&Kind::Rust) && p.kinds.contains(&Kind::Node));
        let outs: Vec<&str> = p.outputs.iter().map(|&(n, _, _)| tree.nodes[n].name.as_str()).collect();
        assert!(outs.contains(&"target") && outs.contains(&"node_modules"));
        assert!(p.last_change.is_some());
        // just written, so not old enough to suggest
        let mut facts = crate::facts::Facts::from_apps(Vec::new());
        assert!(build_outputs(&tree, &ps, &facts, 30, 0).is_empty());
        // target holds app.exe: without Prefetch nobody knows whether it runs, so it stays
        assert!(build_outputs(&tree, &ps, &facts, 0, 1000).is_empty());
        facts.runs_complete = true;
        let f = build_outputs(&tree, &ps, &facts, 0, 1000);
        assert_eq!(f.len(), 1, "only target is over 1000 bytes");
        assert!(f[0].command.as_deref().unwrap().contains("; sprawl-rm (Get-Item -LiteralPath '"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
