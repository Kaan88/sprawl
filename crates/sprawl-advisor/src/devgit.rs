//! Git folders worth a look (D, 1.0.4): old worktrees, `.git` folders git could pack smaller,
//! clones that could be downloaded again, and the same repository cloned twice.
//!
//! Sprawl never deletes inside `.git`. What it knows about a repository comes from git itself,
//! asked read only (`status`, `log`, `stash list`, `worktree list`, `remote`), with optional locks
//! off and a time limit; without git, or when git couldn't answer, nothing is suggested. A
//! repository with anything uncommitted, unpushed or stashed is never suggested for removal.
//! The clean-ups are git's own: `git worktree remove` (which refuses a worktree with changes),
//! `git gc`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use sprawl_core::{format_size, Tree};

use crate::facts::month_year;
use crate::finding::{Action, Finding, Remedy, Risk, Source};
use crate::projects::Project;
use crate::rules::ps_quote;

/// The rule ids, which the app lets past its "another account's" gate: git acts on the folder
/// it is named, for whoever runs it.
pub const WORKTREE: &str = "dev.worktree";
pub const GIT_GC: &str = "dev.git-gc";
pub const CLONE: &str = "dev.old-clone";
pub const SAME_CLONE: &str = "dev.same-clone";

/// A clone untouched for this long may be suggested (with everything on its remote).
const CLONE_IDLE_DAYS: u64 = 180;
/// Smallest worktree, clone and `.git` saving worth a line.
const MIN_BYTES: u64 = 200 << 20;
/// At most this many repositories are asked, and for at most this long in all.
const MAX_REPOS: usize = 60;
const BUDGET: Duration = Duration::from_secs(30);

/// Git's answer: whether it succeeded, and what it wrote (stdout).
pub type GitRun<'a> = &'a dyn Fn(&Path, &[&str]) -> Option<(bool, String)>;

/// Run `git -C dir args`, read only: optional locks off (a status doesn't rewrite the index), no
/// prompts, no window, fsmonitor off (a repository's settings can't start a program), at most
/// 15 seconds and 1 MB of output. None when git isn't there, timed out, or couldn't be read.
pub fn run_git(dir: &Path, args: &[&str]) -> Option<(bool, String)> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut child = Command::new("git")
        .args(["-c", "core.fsmonitor=false", "-c", "core.untrackedCache=false", "-C"])
        .arg(dir)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()
        .ok()?;
    let out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.take(1 << 20).read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some((status.success(), reader.join().ok()?)),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
}

/// Whether git may be run here: not as administrator, unless the user allowed tools from their
/// own folders (git lives in one), as for any tool clean-up.
pub fn git_allowed() -> bool {
    crate::clean::elevated_refusal("git status").is_none()
}

/// The findings, with git run by [`run_git`] when allowed.
pub fn find(tree: &Tree, projects: &[Project], idle_days: u64) -> Vec<Finding> {
    if !git_allowed() {
        return Vec::new();
    }
    find_with(tree, projects, idle_days, MIN_BYTES, SystemTime::now(), &run_git)
}

fn days_since(now: SystemTime, t: Option<SystemTime>) -> Option<u64> {
    t.and_then(|t| now.duration_since(t).ok()).map(|d| d.as_secs() / 86_400)
}

fn finding(path: PathBuf, title: String, bytes: u64, risk: Risk, remedy: Remedy, id: &str, evidence: Vec<String>) -> Finding {
    Finding {
        path,
        owner: None,
        bytes,
        risk,
        remedy,
        command: None,
        title,
        evidence,
        confidence: 0.85,
        source: Source::Rule(id.into()),
        allow_protected: false,
        trusted: true,
        builtin: true,
        on_path: true,
        action: None,
        admin: false,
        carved: Vec::new(),
        gain: None,
        forced_from: None,
    }
}

/// The `.git` child of `n`, and whether it is a folder (a worktree has a file there).
fn dot_git(tree: &Tree, n: usize) -> Option<(usize, bool)> {
    tree.nodes[n].children.iter().copied().find(|&c| tree.nodes[c].name.eq_ignore_ascii_case(".git")).map(|c| (c, tree.nodes[c].is_dir))
}

fn child(tree: &Tree, n: usize, name: &str) -> Option<usize> {
    tree.nodes[n].children.iter().copied().find(|&c| tree.nodes[c].name.eq_ignore_ascii_case(name))
}

/// Loose objects (bytes) and pack files (count) in a `.git` folder, from the scan.
fn objects(tree: &Tree, git: usize) -> (u64, usize) {
    let Some(obj) = child(tree, git, "objects") else { return (0, 0) };
    let mut loose = 0;
    for &c in &tree.nodes[obj].children {
        let n = &tree.nodes[c];
        if n.is_dir && n.name.len() == 2 && n.name.chars().all(|ch| ch.is_ascii_hexdigit()) {
            loose += n.size;
        }
    }
    let packs = child(tree, obj, "pack").map_or(0, |p| tree.nodes[p].children.iter().filter(|&&c| tree.nodes[c].name.to_ascii_lowercase().ends_with(".pack")).count());
    (loose, packs)
}

/// One line of output, trimmed, when git succeeded.
fn line(run: GitRun, dir: &Path, args: &[&str]) -> Option<String> {
    match run(dir, args) {
        Some((true, s)) => Some(s.trim().to_string()),
        _ => None,
    }
}

/// Nothing changed or new (untracked files included) in the checkout at `dir`; None: unknown.
fn clean(run: GitRun, dir: &Path) -> Option<bool> {
    line(run, dir, &["status", "--porcelain=v1", "--untracked-files=all"]).map(|s| s.is_empty())
}

/// [`find`] with git's answers from `run`, sizes from `min` up.
pub fn find_with(tree: &Tree, projects: &[Project], idle_days: u64, min: u64, now: SystemTime, run: GitRun) -> Vec<Finding> {
    let started = Instant::now();
    let mut out = Vec::new();
    // remote URL -> clones seen with it (path, size)
    let mut remotes: Vec<(String, PathBuf, u64)> = Vec::new();
    let mut asked = 0;
    for p in projects.iter().filter(|p| p.has_git) {
        if asked >= MAX_REPOS || started.elapsed() > BUDGET {
            break;
        }
        let Some((g, is_dir)) = dot_git(tree, p.node) else { continue };
        let node = &tree.nodes[p.node];
        let path = tree.path_of(p.node);
        let name = node.name.clone();
        let idle = days_since(now, p.last_change);
        if !is_dir {
            // a linked worktree: its commits live in the main repository, so removing it loses
            // only what isn't committed, which git itself refuses to remove
            if node.size < min || idle.is_none_or(|d| d < idle_days) {
                continue;
            }
            asked += 1;
            let Some(common) = line(run, &path, &["rev-parse", "--path-format=absolute", "--git-common-dir"]) else { continue };
            let common = PathBuf::from(common.replace('/', "\\"));
            let Some(main) = common.parent().map(Path::to_path_buf) else { continue };
            if clean(run, &path) != Some(true) {
                continue;
            }
            let branch = line(run, &path, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
            let merged = line(run, &main, &["rev-parse", "HEAD"])
                .and_then(|head| run(&path, &["merge-base", "--is-ancestor", "HEAD", &head]))
                .map(|(ok, _)| ok);
            let mut evidence = vec![
                format!("A git worktree of {}, last changed {} days ago, with nothing uncommitted or new in it", main.display(), idle.unwrap_or(0)),
                match (branch.as_str(), merged) {
                    ("HEAD" | "", _) => "Not on a branch (a detached checkout); its commits stay in the main repository".to_string(),
                    (b, Some(true)) => format!("Its branch {b} is merged into the main checkout's; the branch stays"),
                    (b, _) => format!("Its branch {b} stays in the main repository with its commits, merged or not"),
                },
                "Removed with git's own 'git worktree remove', which refuses if anything in it changed meanwhile. Files git ignores in it (build output, local settings such as .env) go with it.".into(),
            ];
            if !p.outputs.is_empty() {
                evidence.push(format!("Its build output alone is {}", format_size(p.output_bytes(tree))));
            }
            let mut f = finding(path.clone(), format!("Old git worktree: {name}"), node.size, Risk::Amber, Remedy::Command, WORKTREE, evidence);
            f.owner = main.file_name().map(|n| n.to_string_lossy().into_owned());
            f.set_action(Action::Tool { command: format!("git -C {} worktree remove {}", ps_quote(&main.display().to_string()), ps_quote(&path.display().to_string())) }, false);
            out.push(f);
            continue;
        }
        // a .git folder git could pack smaller: many loose objects or many packs
        let (loose, packs) = objects(tree, g);
        let gain = loose / 2;
        if gain >= min || packs >= 30 {
            let gpath = tree.path_of(g);
            let mut f = finding(
                gpath,
                format!("Git history of {name} could be packed smaller"),
                tree.nodes[g].size,
                Risk::Amber,
                Remedy::Command,
                GIT_GC,
                vec![
                    format!("{} of loose objects and {packs} pack files in {}\\.git", format_size(loose), path.display()),
                    "git gc packs them together: the history stays the same. It also removes unreachable objects older than two weeks (from deleted branches or dropped stashes).".into(),
                ],
            );
            f.gain = Some(gain);
            f.owner = Some(name.clone());
            f.set_action(Action::Tool { command: format!("git -C {} gc --quiet", ps_quote(&path.display().to_string())) }, false);
            out.push(f);
        }
        // a clone untouched for months with everything on its remote
        if node.size < min || idle.is_none_or(|d| d < CLONE_IDLE_DAYS) {
            continue;
        }
        asked += 1;
        let Some(url) = line(run, &path, &["remote", "get-url", "origin"]).filter(|u| !u.is_empty()) else { continue };
        remotes.push((normal_url(&url), path.clone(), node.size));
        let safe = clean(run, &path) == Some(true)
            && line(run, &path, &["log", "--branches", "--not", "--remotes", "-1", "--format=%H"]).is_some_and(|s| s.is_empty())
            && line(run, &path, &["stash", "list"]).is_some_and(|s| s.is_empty())
            && line(run, &path, &["worktree", "list", "--porcelain"]).is_some_and(|s| s.lines().filter(|l| l.starts_with("worktree ")).count() == 1);
        if !safe {
            continue;
        }
        let when = p.last_change.map(month_year).unwrap_or_default();
        out.push(finding(
            path,
            format!("Clone you can download again: {name}"),
            node.size,
            Risk::Amber,
            Remedy::Delete,
            CLONE,
            vec![
                format!("Untouched since {when}; every branch is on its remote ({})", short_url(&url)),
                "Nothing uncommitted, new, unpushed or stashed in it, and no other worktrees: git clone brings it back.".into(),
                "Files git ignores in it (build output, local settings such as .env) aren't on the remote and go with it.".into(),
            ],
        ));
        if let Some(f) = out.last_mut() {
            f.owner = Some(name);
            f.set_action(Action::Remove, false);
        }
    }
    // the same repository cloned more than once: each copy after the biggest, as a tip
    remotes.sort_by(|a, b| a.0.cmp(&b.0).then(b.2.cmp(&a.2)));
    for w in remotes.windows(2) {
        if w[0].0 == w[1].0 {
            out.push(finding(
                w[1].1.clone(),
                format!("Second copy of {}", w[0].1.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())),
                w[1].2,
                Risk::Amber,
                Remedy::Info,
                SAME_CLONE,
                vec![format!("The same repository ({}) is also cloned at {}. If you don't need both, keep one.", short_url(&w[0].0), w[0].1.display())],
            ));
        }
    }
    out
}

/// A remote URL compared whole: lower case, no trailing `.git` or slash.
fn normal_url(u: &str) -> String {
    let u = u.trim().trim_end_matches('/').to_lowercase();
    u.strip_suffix(".git").unwrap_or(&u).to_string()
}

/// The host and path of a remote, without any user name or password in it.
fn short_url(u: &str) -> String {
    let u = u.trim();
    let rest = u.split_once("://").map_or(u, |(_, r)| r);
    let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
    rest.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Not under %TEMP%: that is in AppData, where nothing counts as a project.
    fn test_base(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join(format!("{name}-{}", std::process::id()))
    }

    fn old(p: &Path) {
        let t = SystemTime::now() - Duration::from_secs(400 * 86_400);
        for e in walk(p) {
            if let Ok(f) = std::fs::File::options().write(true).open(&e) {
                let _ = f.set_modified(t);
            }
        }
    }

    fn walk(p: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![p.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let q = e.path();
                if q.is_dir() { stack.push(q); } else { out.push(q); }
            }
        }
        out
    }

    /// A fixture: a main repository with a big .git, a worktree of it, and an old clone (the
    /// scan sees folders; git's answers are given by the test).
    fn fixture(base: &Path) {
        let _ = std::fs::remove_dir_all(base);
        let big = |p: &Path, n: usize| { std::fs::create_dir_all(p.parent().unwrap()).unwrap(); std::fs::write(p, vec![1u8; n]).unwrap(); };
        big(&base.join(r"main\.git\objects\ab\0123"), 4 << 20);
        big(&base.join(r"main\src\a.rs"), 10);
        big(&base.join(r"wt\big.bin"), 2 << 20);
        std::fs::write(base.join(r"wt\.git"), "gitdir: x").unwrap();
        big(&base.join(r"clone\.git\HEAD"), 10);
        big(&base.join(r"clone\data.bin"), 3 << 20);
        big(&base.join(r"clone2\.git\HEAD"), 10);
        big(&base.join(r"clone2\data.bin"), 2 << 20);
        old(base);
    }

    #[test]
    fn finds_worktrees_packs_and_clones_from_gits_answers() {
        let base = test_base("sprawl-devgit");
        fixture(&base);
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        // the path as scanned (no "..")
        let base = tree.root_path.clone();
        let projects = crate::projects::find(&tree);
        let main = base.join("main");
        let answers: RefCell<HashMap<(String, String), (bool, String)>> = RefCell::new(HashMap::new());
        let say = |dir: &Path, args: &str, ok: bool, out: &str| { answers.borrow_mut().insert((dir.display().to_string(), args.to_string()), (ok, out.to_string())); };
        say(&base.join("wt"), "rev-parse --path-format=absolute --git-common-dir", true, &format!("{}\n", main.join(".git").display()));
        say(&base.join("wt"), "status --porcelain=v1 --untracked-files=all", true, "");
        say(&base.join("wt"), "rev-parse --abbrev-ref HEAD", true, "feature\n");
        say(&main, "rev-parse HEAD", true, "abc\n");
        say(&base.join("wt"), "merge-base --is-ancestor HEAD abc", true, "");
        for c in ["clone", "clone2"] {
            let d = base.join(c);
            say(&d, "remote get-url origin", true, "https://user:secret@example.com/team/tool.git\n");
            say(&d, "status --porcelain=v1 --untracked-files=all", true, "");
            say(&d, "log --branches --not --remotes -1 --format=%H", true, "");
            say(&d, "stash list", true, if c == "clone2" { "stash@{0}: WIP" } else { "" });
            say(&d, "worktree list --porcelain", true, &format!("worktree {}\nHEAD abc\n", d.display()));
        }
        let run = |dir: &Path, args: &[&str]| answers.borrow().get(&(dir.display().to_string(), args.join(" "))).cloned();
        let found = find_with(&tree, &projects, 30, 1 << 20, SystemTime::now(), &run);
        let by = |id: &str| found.iter().filter(|f| matches!(&f.source, Source::Rule(r) if r == id)).collect::<Vec<_>>();
        let wt = by(WORKTREE);
        assert_eq!(wt.len(), 1, "{found:#?}");
        assert_eq!(wt[0].path, base.join("wt"));
        let Some(Action::Tool { command }) = &wt[0].action else { panic!() };
        assert_eq!(command, &format!("git -C {} worktree remove {}", ps_quote(&main.display().to_string()), ps_quote(&base.join("wt").display().to_string())));
        assert!(wt[0].evidence[1].contains("merged"), "{:?}", wt[0].evidence);
        let gc = by(GIT_GC);
        assert_eq!((gc.len(), gc[0].path.clone(), gc[0].gain), (1, main.join(".git"), Some(2 << 20)));
        // the clone with a stash is never suggested; the user name and password never shown
        let clones = by(CLONE);
        assert_eq!(clones.len(), 1);
        assert_eq!(clones[0].path, base.join("clone"));
        assert!(clones[0].evidence[0].contains("example.com/team/tool.git") && !clones[0].evidence.iter().any(|e| e.contains("secret")));
        assert_eq!(clones[0].action, Some(Action::Remove));
        // and the same remote twice is a tip on the smaller copy
        let same = by(SAME_CLONE);
        assert_eq!((same.len(), same[0].path.clone(), same[0].remedy), (1, base.join("clone2"), Remedy::Info));
        // each passes the checks every finding goes through
        let never = crate::safety::Never::default_list();
        for f in &found {
            assert!(crate::safety::check(f, &tree, &never).is_ok(), "{}: {:?}", f.title, crate::safety::check(f, &tree, &never).err());
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// When git can't answer (not installed, timed out), nothing that needs its answer is said.
    #[test]
    fn no_answer_no_suggestion() {
        let base = test_base("sprawl-devgit-none");
        fixture(&base);
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let projects = crate::projects::find(&tree);
        let found = find_with(&tree, &projects, 30, 1 << 20, SystemTime::now(), &|_, _| None);
        assert!(found.iter().all(|f| matches!(&f.source, Source::Rule(r) if r == GIT_GC)), "{found:#?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// With real git: a clean worktree is found with git's own remove command; a new file in it
    /// stops the suggestion. Skipped without git, or where git may not run (elevated).
    #[test]
    fn a_real_worktree() {
        if !git_allowed() || run_git(Path::new("."), &["--version"]).is_none() {
            return;
        }
        let base = test_base("sprawl-devgit-real");
        let _ = std::fs::remove_dir_all(&base);
        let main = base.join("main");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::write(main.join("a.txt"), "a").unwrap();
        let git = |dir: &Path, args: &[&str]| assert!(run_git(dir, args).is_some_and(|(ok, _)| ok), "git {args:?}");
        git(&main, &["init", "-q", "-b", "main"]);
        git(&main, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "."]);
        git(&main, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "a"]);
        git(&main, &["worktree", "add", "-q", "-b", "side", "../wt"]);
        let found = |base: &Path| {
            let tree = sprawl_core::scan(base, |_| {}).unwrap();
            let projects = crate::projects::find(&tree);
            find_with(&tree, &projects, 0, 0, SystemTime::now(), &run_git).into_iter().filter(|f| matches!(&f.source, Source::Rule(r) if r == WORKTREE)).collect::<Vec<_>>()
        };
        let wt = found(&base);
        assert_eq!(wt.len(), 1, "{wt:#?}");
        assert!(wt[0].path.ends_with("wt") && wt[0].evidence[1].contains("side"), "{:?}", wt[0].evidence);
        std::fs::write(base.join("wt").join("new.txt"), "n").unwrap();
        assert!(found(&base).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn urls() {
        assert_eq!(short_url("https://user:pw@github.com/a/b.git"), "github.com/a/b.git");
        assert_eq!(short_url("git@github.com:a/b.git"), "github.com:a/b.git");
        assert_eq!(normal_url("https://GitHub.com/a/b.git/"), normal_url("https://github.com/a/b"));
    }
}
