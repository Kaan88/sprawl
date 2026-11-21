//! Git folders worth a look (D, 1.0.4): old worktrees, `.git` folders git could pack smaller,
//! clones that could be downloaded again, and the same repository cloned twice.
//!
//! Sprawl never deletes inside `.git`. What it knows about a repository comes from git itself,
//! asked read only (`status`, `log`, `stash list`, `worktree list`, `remote`, `ls-files`), with
//! optional locks off and a time limit; without git, or when git couldn't answer, nothing is
//! suggested. A repository with anything uncommitted, unpushed, stashed or hidden from status is
//! never suggested for removal. The clean-ups are git's own: `git worktree remove` (which
//! refuses a worktree with changes), `git gc`.
//!
//! A scanned repository is anyone's: an extracted archive can hold a `.git` whose config names
//! programs git runs (a `filter` during `status`, `gpg.program` during `log`, ...). So before git
//! is asked anything, Sprawl reads the repository's config itself, through pinned local handles,
//! and leaves the repository alone unless every setting is on a short list of plain ones
//! ([`config_ok`]); a worktree's `gitdir:` must be a local folder of the usual shape. Git is
//! never run for advice as administrator, and the check is made again just before a clean-up
//! runs git.

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

/// Settings on every git call, over the repository's own: no fsmonitor or hooks, no signature
/// checks (they run gpg), no network. Defence in depth: [`repo_ok`] is what keeps a
/// repository's programs from running.
const HARDEN: [&str; 10] = ["-c", "core.fsmonitor=false", "-c", "core.untrackedCache=false", "-c", "core.hooksPath=NUL", "-c", "log.showSignature=false", "-c", "protocol.allow=never"];

/// The same, for a clean-up's command line.
const HARDEN_CMD: &str = "-c core.fsmonitor=false -c core.hooksPath=NUL -c log.showSignature=false -c protocol.allow=never";

/// Run `git -C dir args`, read only: optional locks off (a status doesn't rewrite the index), no
/// prompts, no window, [`HARDEN`]ed, at most 15 seconds and 1 MB of output. None when git isn't
/// there, timed out, or couldn't be read. Only for a repository [`repo_ok`] passed.
pub fn run_git(dir: &Path, args: &[&str]) -> Option<(bool, String)> {
    run_git_until(dir, args, Instant::now() + Duration::from_secs(15))
}

/// [`run_git`], ended at `deadline` (or 15 seconds, the sooner). Git runs in a job with all it
/// starts: a time-out ends them all and waits for git, so nothing is left running and the
/// reader isn't left blocked (Fable 1.0.4-1.0.5 P3-7).
fn run_git_until(dir: &Path, args: &[&str], deadline: Instant) -> Option<(bool, String)> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let deadline = deadline.min(Instant::now() + Duration::from_secs(15));
    if Instant::now() >= deadline {
        return None;
    }
    let mut cmd = Command::new("git");
    cmd
        .args(HARDEN)
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut run = crate::llm::contain::spawn(&mut cmd, 0x0800_0000).ok()?;
    let child = &mut run.child;
    let out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.take(1 << 20).read_to_string(&mut s);
        s
    });
    loop {
        match run.child.try_wait() {
            Ok(Some(status)) => {
                // a helper git started may still hold the pipe: it ends with the job
                run.end();
                return Some((status.success(), reader.join().ok()?));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                run.end();
                let _ = reader.join();
                return None;
            }
        }
    }
}

/// Whether git may be run for advice: only when Windows confirms Sprawl is not elevated. The
/// "tools from my own folders" permission is for clean-ups the user reviews, not for git in
/// every scanned repository.
pub fn git_allowed() -> bool {
    sprawl_core::elevation().ok() == Some(false)
}

/// The findings, with git run by [`run_git`] when allowed.
pub fn find(tree: &Tree, projects: &[Project], idle_days: u64) -> Vec<Finding> {
    if !git_allowed() {
        return Vec::new();
    }
    // each call within the whole budget too, not only between repositories
    let until = Instant::now() + BUDGET;
    find_with(tree, projects, idle_days, MIN_BYTES, SystemTime::now(), &|d: &Path, a: &[&str]| run_git_until(d, a, until), &repo_ok, &worktree_main)
}

/// The settings a repository's config may hold for Sprawl to ask git about it: plain ones that
/// name no program, command, include or hook. Section, then its keys (lower case).
const PLAIN: &[(&str, &[&str])] = &[
    ("core", &["repositoryformatversion", "filemode", "bare", "logallrefupdates", "symlinks", "ignorecase", "autocrlf", "precomposeunicode", "longpaths", "eol", "safecrlf", "quotepath", "checkstat", "trustctime"]),
    ("remote", &["url", "pushurl", "fetch", "push", "prune", "tagopt"]),
    ("branch", &["remote", "merge", "rebase", "pushremote", "description"]),
    ("user", &["name", "email", "signingkey"]),
    ("submodule", &["url", "active"]),
    ("extensions", &["worktreeconfig", "objectformat"]),
    ("init", &["defaultbranch"]),
    ("pull", &["rebase", "ff"]),
    ("fetch", &["prune"]),
    ("index", &["version"]),
    // Git LFS writes this one line into a repository; its filter is in the user's own config
    ("lfs", &["repositoryformatversion"]),
];

/// The most of a config file Sprawl reads.
const MAX_CONFIG: u64 = 64 << 10;

/// Whether the text of a git config file holds only [`PLAIN`] settings. Anything else (a
/// `filter`, `diff`, `include`, `gpg`, `alias`, `credential` section, `core.fsmonitor`,
/// `core.hooksPath`, `core.sshCommand`, `gc.recentObjectsHook`, a continued line, ...) is no.
pub fn config_ok(text: &str) -> bool {
    fn key_ok(section: &str, line: &str) -> bool {
        let key = line.split('=').next().unwrap_or("").trim().to_ascii_lowercase();
        !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && PLAIN.iter().any(|(s, keys)| *s == section && keys.contains(&key.as_str()))
    }
    let mut section = String::new();
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let l = raw.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
            continue;
        }
        if l.ends_with('\\') {
            return false;
        }
        if let Some(rest) = l.strip_prefix('[') {
            let Some((head, after)) = rest.split_once(']') else { return false };
            // `[remote "origin"]`, or the old `[branch.main]`
            section = head.trim().split([' ', '"', '.', '\t']).next().unwrap_or("").to_ascii_lowercase();
            if !PLAIN.iter().any(|(s, _)| *s == section) {
                return false;
            }
            let after = after.trim();
            if after.is_empty() || after.starts_with('#') || after.starts_with(';') {
                continue;
            }
            if !key_ok(&section, after) {
                return false;
            }
            continue;
        }
        if section.is_empty() || !key_ok(&section, l) {
            return false;
        }
    }
    true
}

/// A config file below `dir` passes [`config_ok`]: read through a pinned chain (a link is an
/// error). Not there is fine; unreadable is no.
fn config_file_ok(dir: &Path, below: &[&str], name: &str) -> bool {
    match sprawl_core::local::read_checked(dir, below, name, MAX_CONFIG) {
        Ok(None) => true,
        Ok(Some(b)) => std::str::from_utf8(&b).is_ok_and(config_ok),
        Err(_) => false,
    }
}

/// Whether git may be asked about the repository at `repo` (a folder with a `.git` folder):
/// local, its `config` and `config.worktree` plain, and not pointing elsewhere (`commondir`).
pub fn repo_ok(repo: &Path) -> bool {
    sprawl_core::local::lexical(repo).is_ok()
        && matches!(sprawl_core::local::read_checked(repo, &[".git"], "commondir", 4096), Ok(None))
        && config_file_ok(repo, &[".git"], "config")
        && config_file_ok(repo, &[".git"], "config.worktree")
}

/// The main repository of the linked worktree at `wt`, read from its `.git` file by Sprawl, not
/// git: `gitdir: <main>\.git\worktrees\<name>`, a local folder whose `commondir` is `../..`, with
/// its own `config.worktree` plain. None for anything else (a share, another shape).
pub fn worktree_main(wt: &Path) -> Option<PathBuf> {
    let text = sprawl_core::local::read_checked(wt, &[], ".git", 4096).ok()??;
    let text = String::from_utf8(text).ok()?;
    let target = text.trim().strip_prefix("gitdir:")?.trim().replace('/', "\\");
    let target = if Path::new(&target).is_absolute() { PathBuf::from(&target) } else { wt.join(&target) };
    let gitdir = sprawl_core::local::lexical(&target).ok()?;
    let name = gitdir.file_name()?.to_str()?.to_string();
    let worktrees = gitdir.parent()?;
    let dot_git = worktrees.parent()?;
    if !worktrees.file_name()?.eq_ignore_ascii_case("worktrees") || !dot_git.file_name()?.eq_ignore_ascii_case(".git") {
        return None;
    }
    let main = dot_git.parent()?.to_path_buf();
    let common = sprawl_core::local::read_checked(&main, &[".git", "worktrees", &name], "commondir", 4096).ok()??;
    if String::from_utf8(common).ok()?.trim().replace('\\', "/").trim_end_matches('/') != "../.." {
        return None;
    }
    config_file_ok(&main, &[".git", "worktrees", &name], "config.worktree").then_some(main)
}

/// Git's own removal of the worktree `wt` of `main`.
pub fn worktree_command(main: &Path, wt: &Path) -> String {
    format!("git {HARDEN_CMD} -C {} worktree remove {}", ps_quote(&main.display().to_string()), ps_quote(&wt.display().to_string()))
}

/// Git's packing of the repository at `repo`.
pub fn gc_command(repo: &Path) -> String {
    format!("git {HARDEN_CMD} -C {} gc --quiet", ps_quote(&repo.display().to_string()))
}

/// Just before a clean-up runs `command` on `path` (a worktree, or a `.git` folder): the same
/// config checks again, and the command exactly the one Sprawl builds for that path now.
pub fn recheck(path: &Path, command: &str) -> Result<(), String> {
    let changed = "the repository's git settings or folders changed since the scan, or name programs; scan again";
    let expected = if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case(".git")) {
        let repo = path.parent().ok_or(changed)?;
        if !repo_ok(repo) {
            return Err(changed.into());
        }
        gc_command(repo)
    } else {
        let main = worktree_main(path).ok_or(changed)?;
        if !repo_ok(&main) {
            return Err(changed.into());
        }
        worktree_command(&main, path)
    };
    if expected != command {
        return Err("not the git command Sprawl builds for this folder".into());
    }
    Ok(())
}

/// Whether a clean-up command is one of these git ones.
pub fn is_git_command(command: &str) -> bool {
    command.starts_with("git ")
}

/// A remote git clone can fetch from again: a server URL, not a folder or a `file://` path.
fn server_url(u: &str) -> bool {
    let u = u.trim();
    let l = u.to_ascii_lowercase();
    if ["https://", "http://", "ssh://", "git://", "git+ssh://", "ssh+git://"].iter().any(|p| l.starts_with(p)) {
        return true;
    }
    // `user@host:path` (scp style); `C:\x` and `.\x:y` are folders
    !u.contains("://") && u.split_once(':').is_some_and(|(host, path)| host.len() > 1 && !host.contains(['\\', '/']) && !path.is_empty())
}

/// Tracked files hidden from `status` (marked assume-unchanged or skip-worktree, so a local
/// edit there isn't seen). None: unknown (git didn't answer, or the list was cut short).
fn hidden_from_status(run: GitRun, dir: &Path) -> Option<bool> {
    match run(dir, &["ls-files", "-v"]) {
        Some((true, s)) if s.len() < (1 << 20) - 1 => Some(s.lines().any(|l| l.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == 'S'))),
        _ => None,
    }
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

/// [`find`] with git's answers from `run`, sizes from `min` up; `repo_ok` and `worktree_main`
/// are the config checks ([`repo_ok`], [`worktree_main`]).
#[allow(clippy::too_many_arguments)]
pub fn find_with(tree: &Tree, projects: &[Project], idle_days: u64, min: u64, now: SystemTime, run: GitRun, repo_ok: &dyn Fn(&Path) -> bool, worktree_main: &dyn Fn(&Path) -> Option<PathBuf>) -> Vec<Finding> {
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
            // its main repository from its .git file, read by Sprawl; both configs plain
            let Some(main) = worktree_main(&path) else { continue };
            if !repo_ok(&main) {
                continue;
            }
            asked += 1;
            if clean(run, &path) != Some(true) || hidden_from_status(run, &path) != Some(false) {
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
            f.set_action(Action::Tool { command: worktree_command(&main, &path) }, false);
            out.push(f);
            continue;
        }
        // nothing at all about a repository whose config names programs (gc would run them)
        if !repo_ok(&path) {
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
            f.set_action(Action::Tool { command: gc_command(&path) }, false);
            out.push(f);
        }
        // a clone untouched for months with everything on its remote
        if node.size < min || idle.is_none_or(|d| d < CLONE_IDLE_DAYS) {
            continue;
        }
        asked += 1;
        let Some(url) = line(run, &path, &["remote", "get-url", "origin"]).filter(|u| !u.is_empty()) else { continue };
        remotes.push((normal_url(&url), path.clone(), node.size));
        // a remote that is a server (a folder may be gone, or in the same clean-up); every
        // commit reachable from HEAD, a branch or a tag known to be on it; nothing changed,
        // stashed or hidden from status; no other worktrees
        let safe = server_url(&url)
            && clean(run, &path) == Some(true)
            && line(run, &path, &["log", "HEAD", "--branches", "--tags", "--not", "--remotes", "-1", "--format=%H"]).is_some_and(|s| s.is_empty())
            && line(run, &path, &["stash", "list"]).is_some_and(|s| s.is_empty())
            && hidden_from_status(run, &path) == Some(false)
            && line(run, &path, &["worktree", "list", "--porcelain"]).is_some_and(|s| s.lines().filter(|l| l.starts_with("worktree ")).count() == 1);
        if !safe {
            continue;
        }
        let when = p.last_change.map(month_year).unwrap_or_default();
        // the user's own work: orange, so it goes to the Recycle Bin (when that is on), needs
        // "my own files" ticked, and is never picked to reach a goal
        out.push(finding(
            path,
            format!("Clone you may be able to download again: {name}"),
            node.size,
            Risk::Orange,
            Remedy::Delete,
            CLONE,
            vec![
                format!("Untouched since {when}; every branch, tag and commit was on its remote ({}) when it was last fetched. The remote may have changed since.", short_url(&url)),
                "Nothing uncommitted, new, unpushed or stashed in it, and no other worktrees: git clone brings back what was pushed.".into(),
                "Files git ignores in it (build output, local settings such as .env) and git's own local settings and hooks in .git aren't on the remote and go with it.".into(),
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
        big(&base.join(r"main\.git\worktrees\wt\commondir"), 0);
        std::fs::write(base.join(r"main\.git\worktrees\wt\commondir"), "../..\n").unwrap();
        std::fs::write(base.join(r"main\.git\config"), "[core]\n\trepositoryformatversion = 0\n\tbare = false\n[remote \"origin\"]\n\turl = https://example.com/a.git\n").unwrap();
        std::fs::write(base.join(r"wt\.git"), format!("gitdir: {}/main/.git/worktrees/wt\n", base.display())).unwrap();
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
        say(&base.join("wt"), "status --porcelain=v1 --untracked-files=all", true, "");
        say(&base.join("wt"), "ls-files -v", true, "H big.bin\n");
        say(&base.join("wt"), "rev-parse --abbrev-ref HEAD", true, "feature\n");
        say(&main, "rev-parse HEAD", true, "abc\n");
        say(&base.join("wt"), "merge-base --is-ancestor HEAD abc", true, "");
        for c in ["clone", "clone2"] {
            let d = base.join(c);
            say(&d, "remote get-url origin", true, "https://user:secret@example.com/team/tool.git\n");
            say(&d, "status --porcelain=v1 --untracked-files=all", true, "");
            say(&d, "log HEAD --branches --tags --not --remotes -1 --format=%H", true, "");
            say(&d, "stash list", true, if c == "clone2" { "stash@{0}: WIP" } else { "" });
            say(&d, "ls-files -v", true, "H data.bin\n");
            say(&d, "worktree list --porcelain", true, &format!("worktree {}\nHEAD abc\n", d.display()));
        }
        let run = |dir: &Path, args: &[&str]| answers.borrow().get(&(dir.display().to_string(), args.join(" "))).cloned();
        let found = find_with(&tree, &projects, 30, 1 << 20, SystemTime::now(), &run, &repo_ok, &worktree_main);
        let by = |id: &str| found.iter().filter(|f| matches!(&f.source, Source::Rule(r) if r == id)).collect::<Vec<_>>();
        let wt = by(WORKTREE);
        assert_eq!(wt.len(), 1, "{found:#?}");
        assert_eq!(wt[0].path, base.join("wt"));
        let Some(Action::Tool { command }) = &wt[0].action else { panic!() };
        assert_eq!(command, &worktree_command(&main, &base.join("wt")));
        assert!(command.contains("core.hooksPath=NUL") && command.contains(&ps_quote(&main.display().to_string())));
        assert_eq!(recheck(&base.join("wt"), command), Ok(()));
        assert!(recheck(&base.join("wt"), &command.replace("worktree remove", "worktree remove --force")).is_err());
        assert!(wt[0].evidence[1].contains("merged"), "{:?}", wt[0].evidence);
        let gc = by(GIT_GC);
        assert_eq!((gc.len(), gc[0].path.clone(), gc[0].gain), (1, main.join(".git"), Some(2 << 20)));
        // the clone with a stash is never suggested; the user name and password never shown
        let clones = by(CLONE);
        assert_eq!(clones.len(), 1);
        assert_eq!(clones[0].path, base.join("clone"));
        assert!(clones[0].evidence[0].contains("example.com/team/tool.git") && !clones[0].evidence.iter().any(|e| e.contains("secret")));
        assert_eq!(clones[0].action, Some(Action::Remove));
        assert_eq!(clones[0].risk, Risk::Orange);
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
        let found = find_with(&tree, &projects, 30, 1 << 20, SystemTime::now(), &|_, _| None, &repo_ok, &worktree_main);
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
            find_with(&tree, &projects, 0, 0, SystemTime::now(), &run_git, &repo_ok, &worktree_main).into_iter().filter(|f| matches!(&f.source, Source::Rule(r) if r == WORKTREE)).collect::<Vec<_>>()
        };
        let wt = found(&base);
        assert_eq!(wt.len(), 1, "{wt:#?}");
        assert!(wt[0].path.ends_with("wt") && wt[0].evidence[1].contains("side"), "{:?}", wt[0].evidence);
        std::fs::write(base.join("wt").join("new.txt"), "n").unwrap();
        assert!(found(&base).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A repository whose config names a program, or points elsewhere, is never asked.
    #[test]
    fn a_config_that_can_run_programs_stops_everything() {
        assert!(config_ok("[core]\n\trepositoryformatversion = 0\n\tfilemode = false\n\tbare = false\n\tlogallrefupdates = true\n\tsymlinks = false\n\tignorecase = true\n[remote \"origin\"]\n\turl = https://github.com/a/b.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n[lfs]\n\trepositoryformatversion = 0\n"));
        assert!(config_ok("\u{feff}# a comment\n[branch.main]\nremote = origin\n"));
        for bad in [
            "[filter \"x\"]\n\tclean = evil.exe\n",
            "[core]\n\tfsmonitor = evil.exe\n",
            "[core]\n\thooksPath = C:/x\n",
            "[core]\n\tsshCommand = evil\n",
            "[core]\n\tpager = evil\n",
            "[gpg]\n\tprogram = evil\n",
            "[log]\n\tshowSignature = true\n",
            "[include]\n\tpath = other\n",
            "[includeIf \"gitdir:C:/\"]\n\tpath = other\n",
            "[gc]\n\trecentObjectsHook = evil\n",
            "[alias]\n\tst = !evil\n",
            "[credential]\n\thelper = evil\n",
            "[remote \"o\"]\n\tuploadpack = evil\n",
            "[submodule \"s\"]\n\tupdate = !evil\n",
            "[extensions]\n\tpartialClone = o\n",
            "url = x\n",
            "[core] fsmonitor = evil\n",
            "[core]\n\tbare = false \\\n",
            "[core\n",
        ] {
            assert!(!config_ok(bad), "{bad}");
        }
        let base = test_base("sprawl-devgit-config");
        fixture(&base);
        let base = sprawl_core::local::lexical(&base).unwrap();
        let main = base.join("main");
        assert!(repo_ok(&main));
        assert_eq!(worktree_main(&base.join("wt")), Some(main.clone()));
        // the worktree's own settings count too
        std::fs::write(main.join(r".git\worktrees\wt\config.worktree"), "[filter \"x\"]\nclean = evil\n").unwrap();
        assert_eq!(worktree_main(&base.join("wt")), None);
        std::fs::remove_file(main.join(r".git\worktrees\wt\config.worktree")).unwrap();
        // a gitdir on a share, or of another shape, is not followed
        for to in [r"\\host\share\x", "../elsewhere", "//host/share/.git/worktrees/wt"] {
            std::fs::write(base.join(r"wt\.git"), format!("gitdir: {to}\n")).unwrap();
            assert_eq!(worktree_main(&base.join("wt")), None, "{to}");
        }
        // a repository with a program in its config: no finding at all, not even git gc
        std::fs::write(main.join(r".git\config"), "[filter \"x\"]\n\tclean = evil.exe\n").unwrap();
        assert!(!repo_ok(&main));
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let projects = crate::projects::find(&tree);
        let asked = std::cell::Cell::new(0);
        let found = find_with(&tree, &projects, 30, 1 << 20, SystemTime::now(), &|_, _| { asked.set(asked.get() + 1); None }, &repo_ok, &worktree_main);
        assert!(found.iter().all(|f| !f.path.starts_with(&main)), "{found:#?}");
        // and a .git pointing elsewhere by commondir
        std::fs::write(main.join(r".git\config"), "[core]\nbare = false\n").unwrap();
        std::fs::write(main.join(r".git\commondir"), r"\\host\share").unwrap();
        assert!(!repo_ok(&main));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_clone_needs_a_server_remote() {
        for u in ["https://github.com/a/b.git", "ssh://git@host/a", "git@github.com:a/b.git", "host:path/b"] {
            assert!(server_url(u), "{u}");
        }
        for u in [r"C:\code\b", "../b", "file:///C:/code/b", r"\\server\share\b", "./x:y", "/c/code/b"] {
            assert!(!server_url(u), "{u}");
        }
    }

    #[test]
    fn urls() {
        assert_eq!(short_url("https://user:pw@github.com/a/b.git"), "github.com/a/b.git");
        assert_eq!(short_url("git@github.com:a/b.git"), "github.com:a/b.git");
        assert_eq!(normal_url("https://GitHub.com/a/b.git/"), normal_url("https://github.com/a/b"));
    }
}
