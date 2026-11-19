//! Clean-up done by Sprawl itself, in place of the PowerShell functions in [`RM_FN`]
//! (which stay for "Save as script"). Same guarantees as those:
//! - a junction or symbolic link is removed itself, never followed (also one made after the scan,
//!   or one swapped in for a folder while it is being cleaned: see [`run`]);
//! - a name ending in a dot or space is skipped (Windows would act on its namesake);
//! - read-only files go too; a folder's read-only bit is cleared only once it is empty;
//! - `clear` and `match` do nothing when their folder is no longer a plain folder;
//! - an error on one item is noted and the rest carry on;
//! - anything on the never-list by the end of its path (a disk image, a vault, a mail profile)
//!   stays, also when it appeared after the scan;
//! - anything created or changed after the scan stays ([`Spare::since`]), and so does anything
//!   the user chose to keep ([`Spare::keep`]), at any depth; both are counted.
//!
//! Tool commands (npm, pip, DISM...) run as a hidden process from System32, their output kept.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use crate::rules::RM_FN;
use crate::Finding;

pub use crate::finding::Action;

/// A finding's action, and whether it needs administrator rights. The action is the one the
/// code that wrote the finding gave it ([`Finding::set_action`]); command text is never read
/// back. A tool command runs only when it comes from the built-in pack.
pub fn action(f: &Finding) -> Option<(Action, bool)> {
    let a = f.action.clone()?;
    if matches!(a, Action::Tool { .. }) && !(f.trusted && f.builtin) {
        return None;
    }
    // a privacy part's set: only as the privacy module made it, for its own part
    if let Action::Set { part, .. } = &a {
        let own = matches!(&f.source, crate::finding::Source::Rule(id) if id == part);
        if !(own && f.trusted && f.builtin && f.command.is_none() && crate::privacy::is_privacy_id(part)) {
            return None;
        }
    }
    if let Action::Match { pattern } = &a {
        if pattern.is_empty() || pattern.contains(['\\', '/']) {
            return None;
        }
    }
    // F4: only as the duplicate search made it, its path the first copy
    if let Action::Link { keep, copies } = &a {
        let own = matches!(&f.source, crate::finding::Source::Rule(id) if id == "remedy.duplicates");
        if !(own && f.trusted && f.builtin && f.command.is_none() && !f.admin && copies.first().is_some_and(|c| same_path(c, &f.path)) && !copies.iter().any(|c| same_path(c, keep))) {
            return None;
        }
    }
    Some((a, f.admin))
}

/// The PowerShell text of an action on `path`, for "Save as script" (the functions in
/// [`RM_FN`] do what [`run`] does).
pub fn script(path: &Path, action: &Action, admin: bool) -> String {
    let q = crate::rules::ps_quote(&path.display().to_string());
    let s = match action {
        Action::Remove => format!("{RM_FN}; sprawl-rm (Get-Item -LiteralPath {q} -Force)"),
        Action::Clear { days } => format!("{RM_FN}; sprawl-clear {q} {days}"),
        Action::Match { pattern } => format!("{RM_FN}; sprawl-match {q} {}", crate::rules::ps_quote(pattern)),
        Action::Tool { command } => command.clone(),
        // never run from a script: the browser must be closed, checked by Sprawl
        Action::Set { .. } => "# Not in a script: privacy parts are cleaned only inside Sprawl, with the browser closed.".to_string(),
        // never run from a script: each copy is compared again just before
        Action::Link { .. } => "# Not in a script: copies are linked only inside Sprawl, after comparing them again.".to_string(),
    };
    if admin && !crate::export::needs_admin(&s) {
        format!("{s}   # as administrator")
    } else {
        s
    }
}

/// The action a command Sprawl wrote stands for, read back: only for the test that the typed
/// action and the script text agree.
#[cfg(test)]
pub(crate) fn parse_command(f: &Finding) -> Option<(Action, bool)> {
    let cmd = f.command.as_deref()?;
    let admin = crate::export::needs_admin(cmd);
    let body = cmd.trim_end().strip_suffix("# as administrator").unwrap_or(cmd).trim_end();
    let Some(tail) = body.strip_prefix(RM_FN) else {
        return Some((Action::Tool { command: cmd.to_string() }, admin));
    };
    let tail = tail.strip_prefix("; ")?;
    let (act, path) = if let Some(r) = tail.strip_prefix("sprawl-rm (Get-Item -LiteralPath ") {
        let (p, rest) = quoted(r)?;
        (rest == " -Force)").then_some((Action::Remove, p))?
    } else if let Some(r) = tail.strip_prefix("sprawl-clear ") {
        let (p, rest) = quoted(r)?;
        let days = rest.strip_prefix(' ')?.parse::<u32>().ok()?;
        (Action::Clear { days }, p)
    } else if let Some(r) = tail.strip_prefix("sprawl-match ") {
        let (p, rest) = quoted(r)?;
        let (pattern, rest) = quoted(rest.strip_prefix(' ')?)?;
        if !rest.is_empty() || pattern.is_empty() {
            return None;
        }
        (Action::Match { pattern }, p)
    } else {
        return None;
    };
    (path.to_lowercase() == f.path.to_string_lossy().to_lowercase()).then_some((act, admin))
}

/// A PowerShell single-quoted string at the start of `s` (quotes doubled inside, typographic
/// ones too, as [`crate::rules::ps_quote`] writes them), and what follows it.
fn quoted(s: &str) -> Option<(String, &str)> {
    let q = |c: char| matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}');
    let mut it = s.char_indices();
    let (_, first) = it.next()?;
    if !q(first) {
        return None;
    }
    let mut out = String::new();
    while let Some((i, c)) = it.next() {
        if q(c) {
            match it.clone().next() {
                Some((_, d)) if q(d) => {
                    it.next();
                    out.push(c);
                }
                _ => return Some((out, &s[i + c.len_utf8()..])),
            }
        } else {
            out.push(c);
        }
    }
    None
}

/// How an item went.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    /// Bytes of the files removed.
    pub freed: u64,
    pub files: u64,
    /// What couldn't go, and why (the first few).
    pub problems: Vec<String>,
    /// Problems past the ones kept.
    pub more_problems: u64,
    /// A tool's last lines of output.
    pub output: Vec<String>,
    pub stopped: bool,
    /// Items left because they were created or changed after the scan.
    pub kept_new: u64,
    /// Items left because the user chose to keep them.
    pub kept_chosen: u64,
    /// Items left because they are on the never-list (a mail store, a disk image).
    pub kept_protected: u64,
    /// Bytes sent to the Recycle Bin (freed only when the bin is emptied), and their files.
    pub recycled: u64,
    pub recycled_files: u64,
    /// A tool finished without a problem, but what it saved couldn't be measured (its folder
    /// couldn't be read completely before or after): nothing is credited, and it isn't a failure.
    pub saving_unknown: bool,
}

/// What a clean-up leaves alone besides the never-list. Tool commands can't honour it: the
/// program decides what it removes.
#[derive(Clone, Default)]
pub struct Spare {
    /// When the scan began: anything created or changed after it stays.
    pub since: Option<SystemTime>,
    /// Folders and files the user chose to keep: they, and everything inside them, stay.
    pub keep: Vec<PathBuf>,
    /// What the scan saw at a path (see [`seen_in`]): which file was there. An item at a path
    /// that isn't the one the scan saw there (another file or folder put in its place, or an
    /// old one moved in, which keeps its old times) is left as new. A folder whose own times
    /// changed after the scan is gone through, item by item, only when the scan knew it there
    /// by its id; otherwise it is held whole until it is scanned again. None: nothing is known.
    pub seen: Option<Seen>,
}

/// What the scan saw at a path: which file was there (its id on the volume, see
/// [`sprawl_core::Node::id`]), nothing (the scan listed that folder with ids and nothing had
/// that name), or not known (the scan didn't cover it, or had no ids there).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SeenAt {
    Item(u64),
    Nothing,
    Unknown,
}

/// See [`Spare::seen`].
pub type Seen = std::sync::Arc<dyn Fn(&Path) -> SeenAt + Send + Sync>;

/// [`Spare::seen`] from the tree the clean-up was reviewed on, each name matched in its own
/// case. The entries of the last folders looked in are kept by name (a clean-up goes folder by
/// folder).
pub fn seen_in(tree: std::sync::Arc<sprawl_core::Tree>) -> Seen {
    type Folder = (PathBuf, Option<std::collections::HashMap<String, u64>>);
    let recent: std::sync::Mutex<std::collections::VecDeque<Folder>> = std::sync::Mutex::new(std::collections::VecDeque::new());
    std::sync::Arc::new(move |p: &Path| {
        let node = |n: usize| match tree.nodes[n].id { 0 => SeenAt::Unknown, id => SeenAt::Item(id) };
        let (Some(parent), Some(name)) = (p.parent(), p.file_name().and_then(|n| n.to_str())) else {
            return crate::history::find_path(&tree, p).map_or(SeenAt::Unknown, node);
        };
        let mut recent = recent.lock().unwrap_or_else(|e| e.into_inner());
        let at = match recent.iter().position(|(f, _)| f == parent) {
            Some(i) => i,
            None => {
                // a folder the scan listed with ids: its entries by name (0: not known)
                let kids = crate::history::find_path(&tree, parent).filter(|&n| tree.nodes[n].is_dir && tree.nodes[n].id != 0)
                    .map(|n| tree.nodes[n].children.iter().map(|&c| (tree.nodes[c].name.clone(), tree.nodes[c].id)).collect());
                if recent.len() >= 16 { recent.pop_back(); }
                recent.push_front((parent.to_path_buf(), kids));
                0
            }
        };
        match &recent[at].1 {
            Some(kids) => match kids.get(name) {
                Some(0) => SeenAt::Unknown,
                Some(&id) => SeenAt::Item(id),
                None => SeenAt::Nothing,
            },
            // the parent isn't a folder the scan listed with ids: the path itself, a scan root
            None => crate::history::find_path(&tree, p).map_or(SeenAt::Unknown, node),
        }
    })
}

impl std::fmt::Debug for Spare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spare").field("since", &self.since).field("keep", &self.keep).field("seen", &self.seen.is_some()).finish()
    }
}

impl Spare {
    /// `p` is one of the kept paths, or inside one.
    pub fn keeps(&self, p: &Path) -> bool {
        let p = lower(p);
        self.keep.iter().any(|k| p.starts_with(lower(k)))
    }

    /// The item at `p` was created or changed after the scan (by its open handle's times), or
    /// isn't the one the scan saw there (by its id).
    fn is_new(&self, p: &Path, info: &win::Info) -> bool {
        let Some(since) = self.since else { return false };
        let ft = filetime(since);
        if info.created > ft {
            return true;
        }
        let seen = self.seen.as_ref().map_or(SeenAt::Unknown, |s| s(p));
        match seen {
            // another item put at this path, or one moved in from elsewhere with its old times
            SeenAt::Item(id) if id != info.id => return true,
            SeenAt::Nothing => return true,
            _ => {}
        }
        if info.attrs & DIRECTORY != 0 && info.attrs & REPARSE == 0 {
            // A folder's own write and change times move whenever something inside it does.
            // When the scan knew this very folder here (by its id), what is inside is looked at
            // one by one, each by this same test. Otherwise a changed folder may be another one
            // put in its place, with old files inside that keep their old times: it is held
            // whole until it is scanned again.
            info.newest > ft && !matches!(seen, SeenAt::Item(_))
        } else {
            info.newest > ft
        }
    }
}

fn lower(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase())
}

/// A time as Windows stores it in a file (100 ns steps since 1601).
fn filetime(t: SystemTime) -> i64 {
    const UNIX_AT: i64 = 116_444_736_000_000_000;
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => UNIX_AT + (d.as_nanos() / 100) as i64,
        Err(e) => UNIX_AT - (e.duration().as_nanos() / 100) as i64,
    }
}

impl Outcome {
    fn problem(&mut self, p: &Path, why: impl std::fmt::Display) {
        if self.problems.len() < 8 {
            self.problems.push(format!("{}: {why}", p.display()));
        } else {
            self.more_problems += 1;
        }
    }
}
const REPARSE: u32 = 0x400;
const DIRECTORY: u32 = 0x10;
/// Junctions and symbolic links (and other "points elsewhere" tags) have this bit in their tag.
const NAME_SURROGATE: u32 = 0x2000_0000;

/// Run `action` on `path`, leaving what `spare` names. `progress` gets the bytes freed so far.
///
/// Everything is done through handles: each item is opened without following a link, checked
/// and deleted through that handle, and each folder stays open (and so can't be renamed or
/// swapped for a link) while what is in it goes. The root's real path is checked once open.
pub fn run(path: &Path, action: &Action, spare: &Spare, stop: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Outcome {
    run_with(path, action, spare, None, stop, progress)
}

/// [`run`], sending what would be deleted to the Recycle Bin when `bin` is given (the user's own
/// files). The same items are left as for a delete: new since the scan, kept, never-list names,
/// odd names, a path that leads elsewhere. A link can't go to the bin and stays where it is (a
/// delete removes it as a link). A folder goes to the
/// bin whole only when nothing inside it must stay; otherwise what can go goes, one by one. What
/// can't go to the bin (none for this volume, too big for it) stays as it is, with the reason:
/// it is never deleted instead. `progress` gets the bytes freed and sent to the bin so far.
pub fn run_with(path: &Path, action: &Action, spare: &Spare, mut bin: Option<&mut Result<crate::recycle::Bin, String>>, stop: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Outcome {
    let mut o = Outcome::default();
    // Network shares and mapped network drives are scanned read only: no action of any kind
    // (a tool command, a privacy set, hard links, a delete) runs on one, whatever offered it.
    // A tool also names its targets inside the command (compact on a folder, a disk image to
    // shrink): every path written there must be local too, not just the finding's own path.
    let named: Vec<PathBuf> = match action {
        Action::Tool { command } => quoted_paths(command),
        _ => Vec::new(),
    };
    let targets: Vec<&Path> = match action {
        Action::Set { files, .. } => std::iter::once(path).chain(files.iter().map(PathBuf::as_path)).collect(),
        Action::Link { keep, copies } => std::iter::once(path).chain(std::iter::once(keep.as_path())).chain(copies.iter().map(PathBuf::as_path)).collect(),
        _ => std::iter::once(path).chain(named.iter().map(PathBuf::as_path)).collect(),
    };
    if let Some(e) = targets.iter().find_map(|t| sprawl_core::local::lexical(t).err()) {
        o.problem(path, format!("left alone: {e}"));
        return o;
    }
    match action {
        // Sprawl's own compression: each file checked now and named to compact.exe, never /s
        Action::Tool { command } if crate::remedies::is_compress(action) => {
            if let Err(e) = crate::remedies::compress_scope(path, command, spare, stop) {
                o.problem(path, format!("left alone: {e}"));
            }
        }
        // A package manager clears wherever it is set up to keep its cache, which need not be
        // the folder that was reviewed. So it is told that folder (a flag, or the variable it
        // reads before its settings files), asked with the very same binding where it would
        // clear, and run only when its whole answer is that folder, held meanwhile.
        Action::Tool { command } => match package_tool(command) {
            None => tool(command, stop, &mut o),
            Some(Err(why)) => o.problem(path, format!("left alone: {why}")),
            Some(Ok(scoped)) => {
                let env = scoped.env(path);
                let env: Vec<(&str, &std::ffi::OsStr)> = env.iter().map(|(k, v)| (*k, v.as_os_str())).collect();
                let mut asked = Outcome::default();
                let answer = tool_with(scoped.probe, stop, &mut asked, &env);
                if asked.stopped {
                    o.stopped = true;
                } else if let Some(why) = asked.problems.first() {
                    o.problem(path, format!("left alone: where it keeps its cache couldn't be asked ({why})"));
                } else {
                    match answer.ok_or_else(|| "its answer about where it keeps its cache was too long or couldn't be read whole".to_string())
                        .and_then(|text| scope_matches(path, &scoped, &text.lines().map(str::to_string).collect::<Vec<_>>())) {
                        Ok(held) => {
                            tool_with(scoped.run, stop, &mut o, &env);
                            drop(held);
                        }
                        Err(why) => o.problem(path, format!("left alone: {why}")),
                    }
                }
            }
        },
        Action::Set { part, files } => match (crate::privacy::part(part), crate::privacy::Places::current()) {
            (Some(p), Some(places)) if same_path(path, files.first().map_or(Path::new(""), |f| f.as_path())) => {
                let now = crate::procs::running();
                set(p, files, spare, stop, &places, now.as_ref(), &crate::inuse::in_use, &mut o, progress)
            }
            (Some(_), None) => o.problem(path, "left alone: this account's AppData folders couldn't be worked out"),
            _ => o.problem(path, "left alone: not a privacy part Sprawl knows"),
        },
        Action::Link { keep, copies } => {
            if copies.first().is_some_and(|c| same_path(c, path)) {
                link(keep, copies, spare, stop, &mut o, progress)
            } else {
                o.problem(path, "left alone: not the copies Sprawl found")
            }
        }
        _ => tree(path, action, spare, bin.as_deref_mut(), stop, &mut o, progress),
    }
    if stop.load(Ordering::Relaxed) {
        o.stopped = true;
    }
    o
}

/// A package manager's clean-up command, with how to ask it where its cache is. Both commands
/// are bound to the reviewed folder: by a flag naming `$env:SPRAWL_TOOL_DIR` (set to it), or
/// by the variable in `var`, which the tool reads before its settings files.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PackageTool {
    /// The command that prints where the cache is.
    probe: &'static str,
    /// The command that clears it (NuGet: only the package folder, never `all`).
    run: &'static str,
    /// How the probe's answer reads and how it must relate to the reviewed folder.
    answer: Answer,
    /// The variable that sets the tool's cache, when it has no flag for it.
    var: Option<&'static str>,
    /// Settings both commands run with besides (Go: its saved settings and flags off).
    fixed: &'static [(&'static str, &'static str)],
}

impl PackageTool {
    /// The environment both commands run with: the reviewed folder, by name.
    fn env(&self, reviewed: &Path) -> Vec<(&'static str, PathBuf)> {
        let mut env = vec![("SPRAWL_TOOL_DIR", reviewed.to_path_buf())];
        if let Some(var) = self.var { env.push((var, reviewed.to_path_buf())); }
        env.extend(self.fixed.iter().map(|&(k, v)| (k, PathBuf::from(v))));
        env
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Answer {
    /// One line, the reviewed folder itself.
    Same,
    /// One line `label: path`, the reviewed folder itself.
    Labelled(&'static str),
}

/// The package-manager commands of the built-in rules, bound to the reviewed folder; Err for
/// one that can't be (it is held). Other commands are not package caches.
fn package_tool(command: &str) -> Option<Result<PackageTool, &'static str>> {
    let t = |probe, run, answer, var, fixed| Some(Ok(PackageTool { probe, run, answer, var, fixed }));
    match command.trim() {
        // npm also rotates its logs wherever `logs-dir` says, on every start: they go in the
        // reviewed folder (where npm keeps them by default), for both commands
        "npm cache clean --force" => t(r#"npm config get cache --cache "$env:SPRAWL_TOOL_DIR" --logs-dir "$env:SPRAWL_TOOL_DIR\_logs""#,
            r#"npm cache clean --force --cache "$env:SPRAWL_TOOL_DIR" --logs-dir "$env:SPRAWL_TOOL_DIR\_logs""#, Answer::Same, None, &[]),
        "pip cache purge" => t(r#"pip cache dir --cache-dir "$env:SPRAWL_TOOL_DIR""#, r#"pip cache purge --cache-dir "$env:SPRAWL_TOOL_DIR""#, Answer::Same, None, &[]),
        // `all` also clears the HTTP cache, temp and plugin caches, wherever they are set up
        "dotnet nuget locals all --clear" | "dotnet nuget locals global-packages --clear" =>
            t("dotnet nuget locals global-packages --list", "dotnet nuget locals global-packages --clear", Answer::Labelled("global-packages:"), Some("NUGET_PACKAGES"), &[]),
        // pnpm's prune also removes metadata and old `dlx` folders in its separate cache folder,
        // which `--store-dir` doesn't move: it can't be limited to the store reviewed
        "pnpm store prune" => Some(Err("pnpm's clean-up also removes things outside the store reviewed (in its cache folder). Run pnpm store prune yourself.")),
        // Yarn 1 says where its versioned cache is, but clears the folder above it, and makes
        // folders while it is asked: what it clears isn't what it names
        "yarn cache clean" => Some(Err("Yarn's clean-up clears more than the cache folder it names. Run yarn cache clean yourself.")),
        // GOFLAGS (from the environment, or saved in Go's settings file) can add other things to
        // clean: both commands run with Go's saved settings off (GOENV=off) and no flags, and no
        // other Go version fetched (GOTOOLCHAIN=local)
        "go clean -cache" => t("go env GOCACHE", "go clean -cache", Answer::Same, Some("GOCACHE"), &[("GOENV", "off"), ("GOFLAGS", ""), ("GOTOOLCHAIN", "local")]),
        // conda adds CONDA_PKGS_DIRS to the package folders its settings name, and clears them
        // all: it can't be limited to the folder reviewed
        "conda clean --all -y" => Some(Err("conda's clean-up can't be limited to the folder reviewed (it clears every package folder its settings name). Run conda clean yourself.")),
        _ => None,
    }
}

/// The reviewed cache folder (or the tool's own folder inside it), held while the tool runs:
/// every folder on the way, and the place itself without delete sharing, so neither can be
/// renamed or swapped for a link meanwhile.
type Held = (sprawl_core::local::Directory, std::fs::File);

fn hold_place(place: &Path) -> std::io::Result<Held> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let chain = sprawl_core::local::directory(place)?;
    // FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES; share read and write, not delete; the
    // folder itself, never a link's target
    let file = std::fs::OpenOptions::new().access_mode(0x1 | 0x80).share_mode(1 | 2)
        .custom_flags(0x0200_0000 | 0x0020_0000).open(&chain.path)?;
    let m = file.metadata()?;
    if !m.is_dir() || m.file_attributes() & REPARSE != 0 { return Err(std::io::Error::other("not a plain folder")); }
    if !same_path(&sprawl_core::local::final_path(&file)?, &chain.path) { return Err(std::io::Error::other("resolved elsewhere")); }
    Ok((chain, file))
}

/// Whether the place the tool says it would clear (its whole answer) is the reviewed folder
/// `path` (or inside it, where that is the tool's layout), local, not skipped in Settings, and
/// no link. That place, held (see [`hold_place`]).
fn scope_matches(path: &Path, tool: &PackageTool, output: &[String]) -> Result<Vec<Held>, String> {
    let lines: Vec<&str> = output.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    let unknown = || "where it keeps its cache couldn't be told from its answer".to_string();
    let places: Vec<PathBuf> = match tool.answer {
        Answer::Same => match lines.as_slice() {
            [one] => vec![PathBuf::from(one.trim_matches('"'))],
            _ => return Err(unknown()),
        },
        Answer::Labelled(label) => match lines.as_slice() {
            [one] => vec![PathBuf::from(one.strip_prefix(label).ok_or_else(unknown)?.trim())],
            _ => return Err(unknown()),
        },
    };
    let skipped = crate::exclusions::configured();
    let reviewed = lower(path);
    let mut held = Vec::new();
    for place in &places {
        if !place.is_absolute() { return Err(unknown()); }
        if let Err(e) = sprawl_core::local::lexical(place) { return Err(format!("its cache is at {} ({e})", place.display())); }
        if crate::exclusions::intersects(place, &skipped) { return Err(format!("its cache at {} is in a folder skipped in Settings > Scanning", place.display())); }
        let here = lower(place);
        if here != reviewed { return Err(format!("it keeps its cache at {}, not in the folder reviewed", place.display())); }
        // the place itself: every folder down to it opened as itself (no link, no share), held
        match hold_place(place) {
            Ok(h) => held.push(h),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("its cache at {} couldn't be checked ({e})", place.display())),
        }
    }
    if held.is_empty() { Err("its cache folder isn't there any more".into()) } else { Ok(held) }
}

/// The paths written in a tool command as PowerShell single-quoted strings (`''` is a quote),
/// a trailing `\*` taken off: anything spelled like a drive path (`X:\`) or a network or
/// device path (`\\`). Other quoted text (a message) is not a path.
fn quoted_paths(command: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\'' { continue; }
        let mut s = String::new();
        loop {
            match chars.next() {
                Some('\'') if chars.peek() == Some(&'\'') => { chars.next(); s.push('\''); }
                Some('\'') | None => break,
                Some(c) => s.push(c),
            }
        }
        let b = s.as_bytes();
        let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/');
        if drive || s.starts_with(r"\\") || s.starts_with("//") {
            out.push(PathBuf::from(s.strip_suffix(r"\*").unwrap_or(&s)));
        }
    }
    out
}

/// What [`run`] would free on `path` now, by the same tests and without deleting anything: the
/// files a pattern names, or the children a keep-days clear would take. Links are not followed
/// and count nothing; never-list names, kept paths and anything new since the scan count
/// nothing either. For Remove and a plain Clear the scan's size is the answer (None).
pub fn would_free(path: &Path, action: &Action, spare: &Spare) -> Option<u64> {
    match action {
        Action::Clear { days } if *days > 0 => {}
        Action::Match { .. } => {}
        _ => return None,
    }
    preview(path, action, spare, 0).map(|p| p.bytes)
}

/// What a clean-up of `path` would remove now, by the run's own tests, without removing
/// anything: the bytes, the files, and the `keep` biggest of them. None for a tool's command
/// (the tool decides) or when `path` can't be read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub bytes: u64,
    pub files: u64,
    /// (file, bytes), the biggest first.
    pub biggest: Vec<(PathBuf, u64)>,
}

pub fn preview(path: &Path, action: &Action, spare: &Spare, keep: usize) -> Option<Preview> {
    // a privacy part's set: every file listed (there are a few), whatever `keep` says
    if let Action::Set { part, files } = action {
        let (Some(p), Some(places)) = (crate::privacy::part(part), crate::privacy::Places::current()) else { return Some(Preview::default()) };
        if !same_path(path, files.first().map_or(Path::new(""), |f| f.as_path())) {
            return Some(Preview::default());
        }
        return Some(set_preview(p, files, spare, &places));
    }
    let cut = match action {
        Action::Clear { days } if *days > 0 => Some(SystemTime::now().checked_sub(Duration::from_secs(*days as u64 * 86_400)).unwrap_or(SystemTime::UNIX_EPOCH)),
        Action::Tool { .. } | Action::Link { .. } => return None,
        _ => None,
    };
    let pattern = match action {
        Action::Match { pattern } => Some(pattern.as_str()),
        _ => None,
    };
    use std::os::windows::fs::MetadataExt;
    let since = spare.since.map(filetime);
    // as the run sees it (it also counts a later rename or move as new, which this can't see)
    let new = |m: &std::fs::Metadata| since.is_some_and(|s| m.creation_time() as i64 > s || m.last_write_time() as i64 > s);
    let born_new = |m: &std::fs::Metadata| since.is_some_and(|s| m.creation_time() as i64 > s);
    let mut out = Preview::default();
    // the biggest so far, smallest on top
    let mut heap: std::collections::BinaryHeap<std::cmp::Reverse<(u64, PathBuf)>> = std::collections::BinaryHeap::new();
    let mut take = |k: PathBuf, len: u64, out: &mut Preview| {
        out.bytes += len;
        out.files += 1;
        if keep > 0 {
            heap.push(std::cmp::Reverse((len, k)));
            if heap.len() > keep {
                heap.pop();
            }
        }
    };
    let top = std::fs::symlink_metadata(path).ok()?;
    if attrs(&top) & REPARSE != 0 || odd_name(path) || never().path_hit(path).is_some() || spare.keeps(path) {
        return Some(out);
    }
    if !top.is_dir() {
        // a file: only Remove takes it
        if matches!(action, Action::Remove) && !new(&top) {
            take(path.to_path_buf(), top.len(), &mut out);
        }
        drop(take);
        out.biggest = heap.into_sorted_vec().into_iter().map(|std::cmp::Reverse((b, p))| (p, b)).collect();
        return Some(out);
    }
    let mut stack: Vec<PathBuf> = list(path).ok()?.0;
    if let Some(cut) = cut {
        stack.retain(|k| !is_new(k, cut));
    }
    while let Some(k) = stack.pop() {
        let name = k.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if odd_name(&k) || never().path_hit(&k).is_some() || spare.keeps(&k) {
            continue;
        }
        let Ok(m) = std::fs::symlink_metadata(&k) else { continue };
        if attrs(&m) & REPARSE != 0 {
            continue;
        }
        if m.is_dir() {
            if !born_new(&m) {
                stack.extend(list(&k).map(|l| l.0).unwrap_or_default());
            }
        } else if pattern.is_none_or(|p| like(&name, p)) && !new(&m) {
            take(k, m.len(), &mut out);
        }
    }
    drop(take);
    out.biggest = heap.into_sorted_vec().into_iter().map(|std::cmp::Reverse((b, p))| (p, b)).collect();
    Some(out)
}

/// A privacy part's files on disk now: those listed (each must be one of the part's own files,
/// in one profile, the part's first file among them, none twice) and every side file of theirs
/// (`-journal`, `-wal`, `-shm`) there is now, whether or not the scan saw it.
pub(crate) fn set_members(part: &crate::privacy::PrivacyPart, files: &[PathBuf], places: &crate::privacy::Places) -> Result<Vec<PathBuf>, String> {
    set_members_with_exclusions(part, files, places, &crate::exclusions::configured())
}

fn set_members_with_exclusions(part: &crate::privacy::PrivacyPart, files: &[PathBuf], places: &crate::privacy::Places, excluded: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    if files.is_empty() {
        return Err("no files named".into());
    }
    let mut profile: Option<PathBuf> = None;
    let mut main = false;
    let mut out: Vec<PathBuf> = Vec::new();
    for f in files {
        let m = crate::privacy::member(part, f, places).ok_or_else(|| format!("{} is not one of this part's files", f.display()))?;
        if m.side.is_some() {
            return Err(format!("{}: a side file goes with its database, not on its own", f.display()));
        }
        match &profile {
            Some(p) if !same_path(p, &m.profile) => return Err("the files are in different profiles".into()),
            _ => profile = Some(m.profile.clone()),
        }
        main |= m.file == 0;
        if out.iter().any(|o| same_path(o, f)) {
            return Err(format!("{} is named twice", f.display()));
        }
        out.push(f.clone());
    }
    if !main {
        return Err(format!("{} isn't among the files", part.files[0]));
    }
    for f in files {
        for sfx in crate::privacy::SIDE_FILES {
            let side = PathBuf::from(format!("{}{sfx}", f.display()));
            match std::fs::symlink_metadata(&side) {
                Ok(_) => out.push(side),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                // can't tell whether it is there: the set can't be known to be complete
                Err(e) => return Err(format!("{}: {e}", side.display())),
            }
        }
    }
    if let Some(p) = out.iter().find(|p| crate::exclusions::intersects(p, excluded)) {
        return Err(format!("{} is excluded in Settings > Scanning; nothing in this privacy set can be removed", p.display()));
    }
    Ok(out)
}

/// What [`set`] would delete now, by its tests other than "is the browser open" and "can each
/// file be opened alone": all the files of the set, or nothing.
pub(crate) fn set_preview(part: &crate::privacy::PrivacyPart, files: &[PathBuf], spare: &Spare, places: &crate::privacy::Places) -> Preview {
    use std::os::windows::fs::MetadataExt;
    let Ok(members) = set_members(part, files, places) else { return Preview::default() };
    let since = spare.since.map(filetime);
    let mut all = Vec::new();
    for m in &members {
        let Ok(md) = std::fs::symlink_metadata(m) else { return Preview::default() };
        let new = since.is_some_and(|s| md.creation_time() as i64 > s || md.last_write_time() as i64 > s);
        if md.is_dir() || attrs(&md) & REPARSE != 0 || spare.keeps(m) || new || has_link_above(m, places) {
            return Preview::default();
        }
        all.push((m.clone(), md.len()));
    }
    all.sort_by(|a, b| b.1.cmp(&a.1));
    Preview { bytes: all.iter().map(|x| x.1).sum(), files: all.len() as u64, biggest: all }
}

/// A folder between this account's AppData and `p` is a junction or link (or `p` isn't below
/// AppData at all).
fn has_link_above(p: &Path, places: &crate::privacy::Places) -> bool {
    let roots = [lower(&places.local), lower(&places.roaming)];
    let mut cur = p.parent();
    while let Some(d) = cur {
        if roots.iter().any(|r| *r == lower(d)) {
            return false;
        }
        match std::fs::symlink_metadata(d) {
            Ok(m) if attrs(&m) & REPARSE == 0 => {}
            _ => return true,
        }
        cur = d.parent();
    }
    true
}

/// Delete a privacy part's set: every file listed and their side files, all or none.
///
/// In order, and the whole set left as it is at the first "no":
/// 1. the files are the part's own, in one profile of this account ([`set_members`]), each
///    listed one still there, none kept by the user, no link in the folders above them;
/// 2. the browser is not running (`now`, its process names; no answer counts as running);
/// 3. no program has any of the files open (`in_use`, the Restart Manager; no answer counts as
///    in use);
/// 4. every file opens alone (no sharing: fails if anything else has it open, and nothing else
///    can open it while Sprawl holds it), is a plain file, is where its path says (no folder
///    above it swapped for a link) and didn't change since the scan;
/// 5. every file is marked to be deleted when its handle closes; if one can't be, the marks
///    already made are taken back;
/// 6. the handles close, and the files go.
///
/// Steps 2 and 3 are the "just before" check; the app makes the same check when the review opens.
#[allow(clippy::too_many_arguments)]
pub(crate) fn set(
    part: &crate::privacy::PrivacyPart,
    files: &[PathBuf],
    spare: &Spare,
    stop: &AtomicBool,
    places: &crate::privacy::Places,
    now: Option<&std::collections::HashSet<String>>,
    in_use: &dyn Fn(&[PathBuf]) -> Option<Vec<String>>,
    o: &mut Outcome,
    progress: &mut dyn FnMut(u64),
) {
    let browser = part.browser();
    let at = files.first().cloned().unwrap_or_default();
    let whole = "nothing in this set was deleted";
    let members = match set_members(part, files, places) {
        Ok(m) => m,
        Err(e) => return o.problem(&at, format!("{e}; {whole}")),
    };
    if let Some(gone) = files.iter().find(|f| std::fs::symlink_metadata(f).is_err()) {
        return o.problem(gone, format!("no longer there, so the set changed since the scan; {whole}"));
    }
    if let Some(k) = members.iter().find(|m| spare.keeps(m)) {
        o.kept_chosen += 1;
        return o.problem(k, format!("you chose to keep it; {whole}"));
    }
    if let Some(m) = members.iter().find(|m| has_link_above(m, places)) {
        return o.problem(m, format!("a folder above it is a junction or link; {whole}"));
    }
    let exes: Vec<String> = browser.running.iter().map(|r| r.to_string()).collect();
    match crate::procs::any_running(&exes, now) {
        Some(false) => {}
        Some(true) => return o.problem(&at, format!("{} is open; {whole}", browser.short)),
        None => return o.problem(&at, format!("Windows couldn't say whether {} is open; {whole}", browser.short)),
    }
    match in_use(&members) {
        Some(names) if names.is_empty() => {}
        Some(names) => return o.problem(&at, format!("in use by {}; {whole}", names.join(", "))),
        None => return o.problem(&at, format!("Windows couldn't say which programs use these files; {whole}")),
    }
    if stop.load(Ordering::Relaxed) {
        o.stopped = true;
        return;
    }
    let mut open: Vec<(PathBuf, win::Handle, win::Info)> = Vec::new();
    for m in &members {
        // every member, side files included, on a local drive (a mapped share is read only)
        if let Err(e) = sprawl_core::local::lexical(m) {
            return o.problem(m, format!("{e}; {whole}"));
        }
        let (h, info) = match win::open_alone(m) {
            Ok(x) => x,
            Err(e) => return o.problem(m, format!("couldn't be opened alone ({e}): in use or locked; {whole}")),
        };
        if info.attrs & (DIRECTORY | REPARSE) != 0 {
            return o.problem(m, format!("not a plain file; {whole}"));
        }
        match win::final_path(&h) {
            Some(real) if same_path(&real, m) => {}
            _ => return o.problem(m, format!("doesn't lead where its path says; {whole}")),
        }
        if spare.is_new(m, &info) {
            o.kept_new += 1;
            return o.problem(m, format!("changed since the scan; scan again; {whole}"));
        }
        open.push((m.clone(), h, info));
    }
    // Side files first, databases last (`open` has the databases first): should a mark ever fail
    // to be taken back, or the run stop between two closes, a database left without its journal
    // is opened fresh by the browser, while a journal or log left beside a new database could
    // be replayed into it.
    open.reverse();
    let mut marked = 0;
    for (m, h, _) in &open {
        if let Err(e) = win::mark_delete(h, true) {
            for (p, h2, _) in &open[..marked] {
                if let Err(e2) = win::mark_delete(h2, false) {
                    o.problem(p, format!("its delete mark couldn't be taken back ({e2})"));
                }
            }
            return o.problem(m, format!("couldn't be marked for deleting ({e}); {whole}"));
        }
        marked += 1;
    }
    let sizes: Vec<(PathBuf, u64, u32)> = open.iter().map(|(p, _, i)| (p.clone(), i.size, i.links)).collect();
    // the handles close, side files first: the files go
    for x in open {
        drop(x);
    }
    for (p, size, links) in sizes {
        match std::fs::symlink_metadata(&p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                o.files += 1;
                if links <= 1 {
                    o.freed += size;
                }
            }
            _ => o.problem(&p, "still there after its handle closed"),
        }
    }
    progress(o.freed);
}

/// One open folder and what is left to do in it.
struct Frame {
    h: win::Handle,
    path: PathBuf,
    kids: std::vec::IntoIter<PathBuf>,
    /// Delete the folder once it is empty (not the root of Clear and Match, nor any folder of Match).
    remove: bool,
}

fn tree(path: &Path, action: &Action, spare: &Spare, mut bin: Option<&mut Result<crate::recycle::Bin, String>>, stop: &AtomicBool, o: &mut Outcome, progress: &mut dyn FnMut(u64)) {
    if stop.load(Ordering::Relaxed) {
        o.stopped = true;
        return;
    }
    let path_buf = match sprawl_core::local::lexical(path) { Ok(p) => p, Err(e) => return o.problem(path, e) };
    let path = path_buf.as_path();
    let Some(parent) = path.parent() else { return o.problem(path, "a whole drive cannot be cleaned"); };
    let _parent = match sprawl_core::local::directory(parent) { Ok(p) => p, Err(e) => return o.problem(path, e) };
    if odd_name(path) {
        return o.problem(path, "skipped, its name ends in a dot or space");
    }
    if spare.keeps(path) {
        o.kept_chosen += 1;
        return;
    }
    let (h, info) = match win::open(path) {
        Ok(x) => x,
        Err(e) => return o.problem(path, e),
    };
    if spare.is_new(path, &info) {
        o.kept_new += 1;
        return;
    }
    // the path must still lead where it did: no folder above it swapped for a link
    match win::final_path(&h) {
        Some(real) if same_path(&real, path) => {}
        Some(real) => return o.problem(path, format!("now leads to {} instead; left alone", real.display())),
        None => return o.problem(path, "its real location couldn't be checked; left alone"),
    }
    let plain_dir = info.attrs & DIRECTORY != 0 && info.attrs & REPARSE == 0;
    let cut = match action {
        Action::Clear { days } if *days > 0 => Some(SystemTime::now().checked_sub(Duration::from_secs(*days as u64 * 86_400)).unwrap_or(SystemTime::UNIX_EPOCH)),
        _ => None,
    };
    let pattern = match action {
        Action::Match { pattern } => Some(pattern.as_str()),
        _ => None,
    };
    if !plain_dir {
        if matches!(action, Action::Remove) {
            match bin.as_deref_mut() {
                Some(_) if recycling_refused(&info) => {
                    o.problem(path, "left alone: a cloud or non-link reparse file cannot be recycled safely");
                }
                // a file to the bin; a link can't go there, and is never deleted instead
                Some(b) if info.attrs & REPARSE == 0 => {
                    if let Err(why) = recycle_one(path, &h, info.size, 1, b, o, progress) {
                        o.problem(path, why);
                    }
                }
                Some(_) => o.problem(path, LINK_LEFT),
                None => delete_one(path, &h, &info, o, progress),
            }
        } else {
            o.problem(path, "not a plain folder any more");
        }
        return;
    }
    // a cloud or offline folder is not listed: listing it can download it
    if info.attrs & crate::dupes::NOT_LOCAL != 0 {
        return o.problem(path, "left alone: a cloud or offline folder");
    }
    // to the bin: the whole folder when nothing in it must stay. One move, one record that
    // Explorer can put back; 1.0 recycled every file alone, flushing a record for each and
    // leaving the emptied folders behind.
    if let (Some(b), Action::Remove) = (bin.as_deref_mut(), action) {
        if let Some((bytes, files)) = whole(path, spare, stop) {
            if stop.load(Ordering::Relaxed) {
                o.stopped = true;
                return;
            }
            match recycle_one(path, &h, bytes, files, b, o, progress) {
                Ok(()) => return,
                Err(Moved::Held(why)) => return o.problem(path, why),
                // it couldn't be moved as one (a file in it is open): one by one below
                Err(Moved::Failed(_)) => {}
            }
        }
    }
    let kids = match list(path) {
        Ok((k, 0)) => k,
        Ok((k, n)) => {
            o.problem(path, unreadable(n));
            k
        }
        Err(e) => return o.problem(path, e),
    };
    // Clear leaves what is new; that is decided for the root's own children only
    let kids: Vec<PathBuf> = match cut {
        Some(cut) => kids.into_iter().filter(|k| !is_new(k, cut)).collect(),
        None => kids,
    };
    // what goes to the bin goes as files and whole folders: a folder left holding something
    // that must stay is never removed
    let recycling = bin.is_some();
    let mut stack = vec![Frame { h, path: path.to_path_buf(), kids: kids.into_iter(), remove: matches!(action, Action::Remove) && !recycling }];
    while let Some(top) = stack.last_mut() {
        if stop.load(Ordering::Relaxed) {
            o.stopped = true;
            return;
        }
        let Some(k) = top.kids.next() else {
            let f = stack.pop().unwrap();
            if f.remove {
                match win::delete(&f.h) {
                    Ok(()) => {}
                    // what stayed inside is already noted
                    Err(e) if e.raw_os_error() == Some(145) => {}
                    Err(e) => o.problem(&f.path, e),
                }
            }
            continue;
        };
        if odd_name(&k) {
            o.problem(&k, "skipped, its name ends in a dot or space");
            continue;
        }
        let name = k.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if never().path_hit(&k).is_some() {
            o.kept_protected += 1;
            continue;
        }
        if spare.keeps(&k) {
            o.kept_chosen += 1;
            continue;
        }
        let (h, info) = match win::open(&k) {
            Ok(x) => x,
            Err(e) => {
                o.problem(&k, e);
                continue;
            }
        };
        if spare.is_new(&k, &info) {
            // Match only removes the files it names: what it wouldn't touch isn't counted
            if pattern.is_none_or(|p| info.attrs & DIRECTORY == 0 && like(&name, p)) {
                o.kept_new += 1;
            }
            continue;
        }
        if info.attrs & DIRECTORY != 0 && info.attrs & REPARSE == 0 {
            if info.attrs & crate::dupes::NOT_LOCAL != 0 {
                o.problem(&k, "left alone: a cloud or offline folder");
                continue;
            }
            if pattern.is_none() {
                if let Some(b) = bin.as_deref_mut() {
                    if let Some((bytes, files)) = whole(&k, spare, stop) {
                        if stop.load(Ordering::Relaxed) {
                            o.stopped = true;
                            return;
                        }
                        match recycle_one(&k, &h, bytes, files, b, o, progress) {
                            Ok(()) => continue,
                            Err(Moved::Held(why)) => {
                                o.problem(&k, why);
                                continue;
                            }
                            Err(Moved::Failed(_)) => {}
                        }
                    }
                }
            }
            match list(&k) {
                Ok((kids, unread)) => {
                    if unread > 0 {
                        o.problem(&k, unreadable(unread));
                    }
                    stack.push(Frame { h, path: k, kids: kids.into_iter(), remove: pattern.is_none() && !recycling })
                }
                Err(e) => o.problem(&k, e),
            }
        } else if info.attrs & REPARSE != 0 && info.attrs & DIRECTORY != 0 {
            // a junction or folder link: Match leaves it; a delete removes the link itself, and
            // with the Recycle Bin it stays (a link can't go to the bin)
            if pattern.is_none() {
                if recycling && recycling_refused(&info) {
                    o.problem(&k, "left alone: a non-link reparse folder cannot be recycled safely");
                } else if recycling {
                    o.problem(&k, LINK_LEFT);
                } else { delete_one(&k, &h, &info, o, progress); }
            }
        } else if pattern.is_none_or(|p| like(&name, p)) {
            match bin.as_deref_mut() {
                Some(_) if recycling_refused(&info) => {
                    o.problem(&k, "left alone: a cloud or non-link reparse file cannot be recycled safely");
                }
                Some(b) if info.attrs & REPARSE == 0 => {
                    if let Err(why) = recycle_one(&k, &h, info.size, 1, b, o, progress) {
                        o.problem(&k, why);
                    }
                }
                Some(_) => o.problem(&k, LINK_LEFT),
                None => delete_one(&k, &h, &info, o, progress),
            }
        }
    }
}

/// Everything in folder `p` would go by a Remove: nothing in it is new since the scan (by the
/// same handle times as one item at a time, change time included), kept, on the never-list or
/// oddly named, no link, junction, cloud or offline item (those go one item at a time, or
/// stay), and all of it can be read. Its bytes and files. `p` itself must be a plain local
/// folder. None as soon as Stop is pressed. Moving the folder is a rename: links inside are
/// moved as links, never followed, and the bin can undo it.
fn whole(p: &Path, spare: &Spare, stop: &AtomicBool) -> Option<(u64, u64)> {
    let plain = |info: &win::Info| info.attrs & (REPARSE | crate::dupes::NOT_LOCAL) == 0;
    let own = win::open_attrs(p).ok()?.1;
    if !plain(&own) || own.attrs & DIRECTORY == 0 || spare.is_new(p, &own) {
        return None;
    }
    let (mut bytes, mut files) = (0u64, 0u64);
    let readable = |p: &Path| list(p).ok().filter(|l| l.1 == 0).map(|l| l.0);
    let mut stack = readable(p)?;
    while let Some(k) = stack.pop() {
        if stop.load(Ordering::Relaxed) || odd_name(&k) || never().path_hit(&k).is_some() || spare.keeps(&k) {
            return None;
        }
        let (_h, info) = win::open_attrs(&k).ok()?;
        if !plain(&info) || spare.is_new(&k, &info) {
            return None;
        }
        if info.attrs & DIRECTORY != 0 {
            stack.extend(readable(&k)?);
        } else {
            bytes += info.size;
            files += 1;
        }
    }
    Some((bytes, files))
}

/// Why a link stays when its folder goes to the Recycle Bin: the bin can't take a link, and
/// removing it for good would break the review's promise that everything can be put back.
const LINK_LEFT: &str = "left in place: a link can't go to the Recycle Bin (what it points to is untouched)";

fn recycling_refused(info: &win::Info) -> bool {
    info.attrs & crate::dupes::NOT_LOCAL != 0 || info.attrs & REPARSE != 0 && info.tag & NAME_SURROGATE == 0
}

/// Delete one opened item (a file, a link, an empty folder) through its handle.
fn delete_one(p: &Path, h: &win::Handle, info: &win::Info, o: &mut Outcome, progress: &mut dyn FnMut(u64)) {
    match win::delete(h) {
        Ok(()) => {
            if info.attrs & DIRECTORY == 0 {
                o.files += 1;
                // a link frees nothing; a file with other names keeps its data
                let link = info.attrs & REPARSE != 0 && info.tag & NAME_SURROGATE != 0;
                if !link && info.links <= 1 {
                    o.freed += info.size;
                    progress(o.freed);
                }
            }
        }
        Err(e) if e.raw_os_error() == Some(145) => {}
        Err(e) => o.problem(p, e),
    }
}

/// Why an item didn't go to the bin.
enum Moved {
    /// It can't (no bin, too big): it stays, with this reason.
    Held(String),
    /// Moving it failed (a file in it is open): a folder can still go one item at a time.
    Failed(String),
}

impl std::fmt::Display for Moved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Moved::Held(s) | Moved::Failed(s) => f.write_str(s),
        }
    }
}

/// Move one opened item (a file, or a folder with everything in it) into `bin`, with the bin's
/// record beside it so Explorer can put it back.
fn recycle_one(p: &Path, h: &win::Handle, bytes: u64, files: u64, bin: &mut Result<crate::recycle::Bin, String>, o: &mut Outcome, progress: &mut dyn FnMut(u64)) -> Result<(), Moved> {
    let b = match bin {
        Ok(b) => b,
        Err(why) => return Err(Moved::Held(format!("not sent to the Recycle Bin, {why}; left as it is"))),
    };
    if bytes > b.room {
        return Err(Moved::Held(format!("too big for the Recycle Bin on {} ({} MB is all it can still take); left as it is", b.drive, b.room >> 20)));
    }
    static SEED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = filetime(SystemTime::now());
    for _ in 0..8 {
        let seed = (now as u64).wrapping_add(SEED.fetch_add(0x1_0001, Ordering::Relaxed)).wrapping_add(std::process::id() as u64);
        let (r, i) = crate::recycle::names(p, seed);
        let (to, rec) = (b.path().join(&r), b.path().join(&i));
        // the record first (a record without its item is ignored by Explorer; an item without
        // its record would be hidden in the bin)
        use std::os::windows::fs::OpenOptionsExt;
        let mut record_file = match std::fs::OpenOptions::new().write(true).access_mode(0x4000_0000 | 0x0001_0000).share_mode(0).create_new(true).open(&rec) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Moved::Held(format!("not sent to the Recycle Bin, its record couldn't be written ({e}); left as it is"))),
        };
        use std::io::Write;
        if let Err(e) = record_file.write_all(&crate::recycle::record(p, bytes, now)).and_then(|_| record_file.sync_all()) {
            let _ = win::delete_file(&record_file);
            return Err(Moved::Held(format!("not sent to the Recycle Bin, its record couldn't be saved ({e}); left as it is")));
        }
        return match win::rename(h, &to) {
            Ok(()) => {
                b.room = b.room.saturating_sub(bytes);
                o.recycled += bytes;
                o.recycled_files += files;
                progress(o.freed + o.recycled);
                Ok(())
            }
            Err(e) => {
                let _ = win::delete_file(&record_file);
                Err(Moved::Failed(format!("couldn't be sent to the Recycle Bin ({e}); left as it is")))
            }
        };
    }
    Err(Moved::Held("not sent to the Recycle Bin, no free name in it; left as it is".into()))
}

/// The entries of folder `p`, and how many of them couldn't be read.
fn list(p: &Path) -> std::io::Result<(Vec<PathBuf>, usize)> {
    let (mut got, mut unread) = (Vec::new(), 0);
    for e in std::fs::read_dir(p)? {
        match e {
            Ok(e) => got.push(e.path()),
            Err(_) => unread += 1,
        }
    }
    Ok((got, unread))
}

/// Said on a folder some of whose entries couldn't be read: they, and the folder, stay.
fn unreadable(n: usize) -> String {
    format!("{n} item{} in it couldn't be read; left as {}", if n == 1 { "" } else { "s" }, if n == 1 { "it is" } else { "they are" })
}

/// F4: each copy replaced by a hard link to `keep`. Each must be a plain local file (not a link,
/// not a never-list name, not one the user keeps, not created or changed since the scan, not
/// read-only), the same size and volume as `keep` and not already one file with it. Both are
/// then pinned against writes and renames and compared through their handles. The checked copy
/// is renamed by handle to a no-replace backup, a link is created only at its vacant old name,
/// then that same original handle is deleted. A failed publish restores by no-replace rename,
/// or preserves the backup if another file occupies the old name. Nothing is overwritten.
/// Freed: zero while another hard link still holds the original data.
fn link(keep: &Path, copies: &[PathBuf], spare: &Spare, stop: &AtomicBool, o: &mut Outcome, progress: &mut dyn FnMut(u64)) {
    link_with(keep, copies, spare, stop, o, progress, &mut |_| {}, &mut |_| {});
}

/// `before_publish` runs with the copy's vacant name, `after_last_check` with the original's
/// backup name just before it goes (tests put their races there).
#[allow(clippy::too_many_arguments)]
fn link_with(keep: &Path, copies: &[PathBuf], spare: &Spare, stop: &AtomicBool, o: &mut Outcome, progress: &mut dyn FnMut(u64), before_publish: &mut dyn FnMut(&Path), after_last_check: &mut dyn FnMut(&Path)) {
    use std::os::windows::fs::OpenOptionsExt;
    let pin_parent = |p: &Path| sprawl_core::local::directory(p.parent().ok_or_else(|| std::io::Error::other("no parent"))?);
    let _kp = match pin_parent(keep) { Ok(p) => p, Err(e) => return o.problem(keep, format!("left alone: {e}")) };
    let mut kf = match sprawl_core::local::file(keep, u64::MAX) { Ok(f) => f, Err(e) => return o.problem(keep, e) };
    let ki = match win::file_info(&kf.file) { Ok(i) => i, Err(e) => return o.problem(keep, e) };
    if odd_name(keep) || never().path_hit(keep).is_some() || spare.is_new(keep, &ki) {
        o.kept_new += 1;
        return o.problem(keep, "left alone: protected or changed since the scan");
    }
    for c in copies {
        if stop.load(Ordering::Relaxed) { o.stopped = true; return; }
        if spare.keeps(c) { o.kept_chosen += 1; continue; }
        if odd_name(c) || never().path_hit(c).is_some() { o.problem(c, "left alone: protected or ambiguous name"); continue; }
        let _cp = match pin_parent(c) { Ok(p) => p, Err(e) => { o.problem(c, e); continue; } };
        // DELETE access is ours, but no other handle may write, rename or unlink this file.
        let mut cf = match std::fs::OpenOptions::new().access_mode(0x8000_0000 | 0x0001_0000).share_mode(1)
            .custom_flags(0x0020_0000 | 0x0010_0000).open(c) {
            Ok(f) => f, Err(e) => { o.problem(c, format!("left alone: in use or unavailable ({e})")); continue; }
        };
        let ci = match win::file_info(&cf) { Ok(i) => i, Err(e) => { o.problem(c, e); continue; } };
        if ci.attrs & (DIRECTORY | REPARSE | crate::dupes::NOT_LOCAL | 1) != 0
            || !sprawl_core::local::final_path(&cf).is_ok_and(|p| same_path(&p, c)) {
            o.problem(c, "left alone: not the expected plain local writable file"); continue;
        }
        if spare.is_new(c, &ci) { o.kept_new += 1; continue; }
        if ci.size != ki.size { o.problem(c, "left alone: no longer the same size"); continue; }
        match (crate::dupes::file_id(&kf.file), crate::dupes::file_id(&cf)) {
            (Some(a), Some(b)) if a == b => continue,
            (Some(a), Some(b)) if a.0 == b.0 => {},
            _ => { o.problem(c, "left alone: file identity or volume could not be verified"); continue; }
        }
        match crate::dupes::same_open(&mut kf.file, &mut cf, stop) {
            Some(true) => {},
            Some(false) => { o.problem(c, "left alone: no longer the same bytes"); continue; }
            None => { o.problem(c, "left alone: comparison stopped or failed"); continue; }
        }
        // The copy's name will open the kept file: data in an extra stream (an application's
        // metadata, a download mark) would be lost with the copy's last name, and its bytes can
        // change after any comparison. So neither file may have one at all; a list that can't
        // be read holds the copy too.
        match (win::named_streams(&kf.file), win::named_streams(&cf)) {
            (Ok(a), Ok(b)) if a.is_empty() && b.is_empty() => {},
            (Ok(_), Ok(_)) => { o.problem(c, "left alone: it or the kept copy has data in an extra stream (a download mark, for one), which a link could lose"); continue; }
            (Err(e), _) | (_, Err(e)) => { o.problem(c, format!("left alone: its extra streams couldn't be checked ({e})")); continue; }
        }
        let Some(name) = c.file_name() else { continue };
        let backup = c.with_file_name(format!("{}.sprawl-original", name.to_string_lossy()));
        // Rename the checked handle, never a pathname replacement. Every subsequent operation
        // also refuses an occupied name. Failure leaves the original recoverable, never removes
        // a third party's file, and never unlinks a temporary name by pathname.
        if let Err(e) = win::rename_file(&cf, &backup) { o.problem(c, format!("left alone: couldn't preserve the original ({e})")); continue; }
        before_publish(c);
        if let Err(e) = std::fs::hard_link(keep, c) {
            let restored = win::rename_file(&cf, c).is_ok();
            o.problem(c, if restored { format!("not linked; original restored ({e})") }
                else { format!("not linked ({e}); original preserved at {} because its old name is occupied", backup.display()) });
            continue;
        }
        // Sharing is kept per stream, so the handle above doesn't stop another program adding a
        // stream to the original. The original is first marked for deletion when this handle
        // closes (undoable until then): from that moment Windows refuses to open any stream of
        // it, a new one included. Only then is the stream list read a last time; a stream that
        // appeared before the mark is in it, and keeps the original (mark undone).
        if let Err(e) = win::delete_on_close(&cf, true) {
            o.problem(c, format!("linked, but original copy remains at {} ({e}); no space credited", backup.display())); continue;
        }
        if !win::named_streams(&cf).is_ok_and(|s| s.is_empty()) {
            let why = match win::delete_on_close(&cf, false) {
                Ok(()) => format!("so it is kept at {}", backup.display()),
                // not seen in practice: said as it is
                Err(e) => format!("and its removal couldn't be called off ({e}), so it may be gone with that stream"),
            };
            o.problem(c, format!("linked, but the original gained an extra stream (or its streams couldn't be read again), {why}; no space credited")); continue;
        }
        after_last_check(&backup);
        drop(cf);
        if ci.links == 1 { o.freed += ci.size; }
        o.files += 1;
        progress(o.freed);
    }
}
fn never() -> &'static crate::safety::Never {
    static N: std::sync::OnceLock<crate::safety::Never> = std::sync::OnceLock::new();
    N.get_or_init(crate::safety::Never::default_list)
}

/// `real` (from the handle, `\\?\C:\...` or `\\?\UNC\...`) is `want`.
fn same_path(real: &Path, want: &Path) -> bool {
    let norm = |s: String| -> String {
        let s = s.replace('/', "\\");
        let s = if let Some(r) = s.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{r}")
        } else if let Some(r) = s.strip_prefix(r"\\?\") {
            r.to_string()
        } else {
            s
        };
        s.trim_end_matches('\\').to_lowercase()
    };
    norm(real.to_string_lossy().into_owned()) == norm(want.to_string_lossy().into_owned())
}

/// Created or changed after `cut`: itself, or anything below it (links not followed). A folder
/// that can't be read, or a name Windows would mistake, counts as new (so it stays).
fn is_new(p: &Path, cut: SystemTime) -> bool {
    let mut stack = vec![p.to_path_buf()];
    while let Some(p) = stack.pop() {
        if odd_name(&p) {
            return true;
        }
        let Ok(m) = std::fs::symlink_metadata(&p) else { return true };
        if m.modified().is_ok_and(|t| t > cut) || m.created().is_ok_and(|t| t > cut) {
            return true;
        }
        if m.is_dir() && attrs(&m) & REPARSE == 0 {
            match std::fs::read_dir(&p) {
                Ok(rd) => stack.extend(rd.flatten().map(|e| e.path())),
                Err(_) => return true,
            }
        }
    }
    false
}

fn attrs(m: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes()
}

fn odd_name(p: &Path) -> bool {
    p.file_name().map(|n| n.to_string_lossy()).is_some_and(|n| n.ends_with('.') || n.ends_with(' '))
}

/// PowerShell's `-like`: `*`, `?` and `[...]`, ignoring case. Each (name, pattern) position is
/// worked out once, so a pattern with many `*` against a long name can't take exponential time.
pub fn like(name: &str, pattern: &str) -> bool {
    fn go(n: &[char], p: &[char], i: usize, j: usize, memo: &mut [Option<bool>]) -> bool {
        let at = i * (p.len() + 1) + j;
        if let Some(r) = memo[at] {
            return r;
        }
        let r = match p.get(j) {
            None => i == n.len(),
            Some('*') => (i..=n.len()).any(|k| go(n, p, k, j + 1, memo)),
            Some('?') => i < n.len() && go(n, p, i + 1, j + 1, memo),
            Some('[') => match p[j..].iter().position(|&c| c == ']').map(|e| j + e) {
                None => n.get(i) == Some(&'[') && go(n, p, i + 1, j + 1, memo),
                Some(end) => match n.get(i) {
                    None => false,
                    Some(&c) => {
                        let set = &p[j + 1..end];
                        let mut hit = false;
                        let mut k = 0;
                        while k < set.len() {
                            if k + 2 < set.len() && set[k + 1] == '-' {
                                hit |= set[k] <= c && c <= set[k + 2];
                                k += 3;
                            } else {
                                hit |= set[k] == c;
                                k += 1;
                            }
                        }
                        hit && go(n, p, i + 1, end + 1, memo)
                    }
                },
            },
            Some(&c) => n.get(i) == Some(&c) && go(n, p, i + 1, j + 1, memo),
        };
        memo[at] = Some(r);
        r
    }
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let mut memo = vec![None; (n.len() + 1) * (p.len() + 1)];
    go(&n, &p, 0, 0, &mut memo)
}

/// The Windows calls the clean-up is made of.
mod win {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    type H = *mut c_void;

    #[repr(C)]
    #[derive(Default)]
    struct ByHandle {
        attrs: u32,
        created: [u32; 2],
        accessed: [u32; 2],
        written: [u32; 2],
        serial: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }

    #[repr(C)]
    struct Basic {
        created: i64,
        accessed: i64,
        written: i64,
        changed: i64,
        attrs: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disposition: u32, flags: u32, template: H) -> H;
        #[link_name = "CloseHandle"]
        fn close_handle(h: isize) -> i32;
        fn GetFileInformationByHandle(h: H, info: *mut ByHandle) -> i32;
        fn GetFileInformationByHandleEx(h: H, class: i32, buf: *mut c_void, size: u32) -> i32;
        fn SetFileInformationByHandle(h: H, class: i32, buf: *const c_void, size: u32) -> i32;
        fn GetFinalPathNameByHandleW(h: H, buf: *mut u16, len: u32, flags: u32) -> u32;
        pub(super) fn GetSystemDirectoryW(buf: *mut u16, size: u32) -> u32;
        fn CreateJobObjectW(sa: *mut c_void, name: *const u16) -> H;
        fn SetInformationJobObject(job: H, class: i32, info: *const c_void, size: u32) -> i32;
        fn AssignProcessToJobObject(job: H, process: H) -> i32;
        fn TerminateJobObject(job: H, code: u32) -> i32;
    }

    pub(super) struct Handle(H);

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { close_handle(self.0 as isize) };
        }
    }

    // only ever used by the thread that opened it
    unsafe impl Send for Handle {}

    pub(super) struct Info {
        pub attrs: u32,
        pub tag: u32,
        pub size: u64,
        pub links: u32,
        /// When it was made, as a FILETIME.
        pub created: i64,
        /// The latest of its write and change times (a rename or move counts), as a FILETIME.
        pub newest: i64,
        /// Which file it is on its volume (`nFileIndexHigh:Low`).
        pub id: u64,
    }

    /// A path for the wide calls; a long one gets the `\\?\` form (std does the same in its own).
    fn wide(p: &Path) -> Vec<u16> {
        let s = p.as_os_str().to_string_lossy();
        let long = if s.len() < 240 || s.starts_with(r"\\?\") {
            None
        } else if let Some(r) = s.strip_prefix(r"\\") {
            Some(format!(r"\\?\UNC\{r}"))
        } else if s.as_bytes().get(1) == Some(&b':') {
            Some(format!(r"\\?\{s}"))
        } else {
            None
        };
        match long {
            Some(l) => std::ffi::OsStr::new(&l).encode_wide().chain(Some(0)).collect(),
            None => p.as_os_str().encode_wide().chain(Some(0)).collect(),
        }
    }

    /// The item itself (a link is not followed), open for deleting. Others may keep reading and
    /// writing it, but nobody can rename or delete it (or a folder above it) while it is open.
    pub(super) fn open(p: &Path) -> std::io::Result<(Handle, Info)> {
        open_with(p, 0x1 | 0x2)
    }

    /// The item itself, for its attributes and times only: shared with everyone, links not
    /// followed, cloud files not downloaded.
    pub(super) fn open_attrs(p: &Path) -> std::io::Result<(Handle, Info)> {
        const READ_ATTRIBUTES: u32 = 0x80;
        const SYNCHRONIZE: u32 = 0x0010_0000;
        const OPEN_EXISTING: u32 = 3;
        const BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const OPEN_NO_RECALL: u32 = 0x0010_0000;
        let w = wide(p);
        let h = unsafe { CreateFileW(w.as_ptr(), READ_ATTRIBUTES | SYNCHRONIZE, 0x1 | 0x2 | 0x4, std::ptr::null_mut(), OPEN_EXISTING, BACKUP_SEMANTICS | OPEN_REPARSE_POINT | OPEN_NO_RECALL, std::ptr::null_mut()) };
        if h as isize == -1 {
            return Err(std::io::Error::last_os_error());
        }
        let h = Handle(h);
        let info = info(&h)?;
        Ok((h, info))
    }

    /// The item itself, open for deleting, shared with nobody: fails when any program has it
    /// open, and no program can open it while this handle is held.
    pub(super) fn open_alone(p: &Path) -> std::io::Result<(Handle, Info)> {
        open_with(p, 0)
    }

    fn open_with(p: &Path, share: u32) -> std::io::Result<(Handle, Info)> {
        const DELETE: u32 = 0x0001_0000;
        const READ_ATTRIBUTES: u32 = 0x80;
        const WRITE_ATTRIBUTES: u32 = 0x100;
        const SYNCHRONIZE: u32 = 0x0010_0000;
        const OPEN_EXISTING: u32 = 3;
        const BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        let w = wide(p);
        let h = unsafe { CreateFileW(w.as_ptr(), DELETE | READ_ATTRIBUTES | WRITE_ATTRIBUTES | SYNCHRONIZE, share, std::ptr::null_mut(), OPEN_EXISTING, BACKUP_SEMANTICS | OPEN_REPARSE_POINT, std::ptr::null_mut()) };
        if h as isize == -1 {
            return Err(std::io::Error::last_os_error());
        }
        let h = Handle(h);
        let info = info(&h)?;
        Ok((h, info))
    }

    fn info(h: &Handle) -> std::io::Result<Info> {
        let mut b = ByHandle::default();
        if unsafe { GetFileInformationByHandle(h.0, &mut b) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut tag = [0u32; 2];
        if b.attrs & super::REPARSE != 0 && unsafe { GetFileInformationByHandleEx(h.0, 9, tag.as_mut_ptr().cast(), 8) } == 0 {
            // Unknown is not evidence of a name-surrogate link: recycling must refuse it.
            tag[1] = 0;
        }
        let ft = |t: [u32; 2]| ((t[1] as u64) << 32 | t[0] as u64) as i64;
        let (mut created, mut newest) = (ft(b.created), ft(b.written));
        let mut basic = Basic { created: 0, accessed: 0, written: 0, changed: 0, attrs: 0 };
        if unsafe { GetFileInformationByHandleEx(h.0, 0, (&mut basic as *mut Basic).cast(), std::mem::size_of::<Basic>() as u32) } != 0 {
            created = created.max(basic.created);
            newest = newest.max(basic.written).max(basic.changed);
        }
        Ok(Info { attrs: b.attrs, tag: tag[1], size: (b.size_high as u64) << 32 | b.size_low as u64, links: b.links, created, newest, id: (b.index_high as u64) << 32 | b.index_low as u64 })
    }

    // Borrow the File's handle only for the synchronous call; ownership remains with File.
    fn borrowed(file: &std::fs::File) -> std::mem::ManuallyDrop<Handle> {
        use std::os::windows::io::AsRawHandle;
        std::mem::ManuallyDrop::new(Handle(file.as_raw_handle().cast()))
    }

    pub(super) fn file_info(file: &std::fs::File) -> std::io::Result<Info> { info(&borrowed(file)) }
    pub(super) fn rename_file(file: &std::fs::File, to: &Path) -> std::io::Result<()> { rename(&borrowed(file), to) }
    pub(super) fn delete_file(file: &std::fs::File) -> std::io::Result<()> { delete(&borrowed(file)) }

    /// Mark the open file to be deleted when its last handle closes (`yes`), or call that off.
    /// Not the POSIX kind: the name stays until then, and the mark can be undone. The file must
    /// not be read-only.
    pub(super) fn delete_on_close(file: &std::fs::File, yes: bool) -> std::io::Result<()> {
        let flag = yes as u8;
        if unsafe { SetFileInformationByHandle(borrowed(file).0, 4, (&flag as *const u8).cast(), 1) } != 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    /// The open file's named data streams (`:name:$DATA`) and their sizes, through its handle;
    /// the unnamed main stream is left out.
    pub(super) fn named_streams(file: &std::fs::File) -> std::io::Result<Vec<(String, u64)>> {
        const FILE_STREAM_INFO: i32 = 7;
        const MORE_DATA: i32 = 234;
        let h = borrowed(file);
        let mut buf = vec![0u64; 512];
        loop {
            if unsafe { GetFileInformationByHandleEx(h.0, FILE_STREAM_INFO, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32) } != 0 { break; }
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(MORE_DATA) || buf.len() >= 1 << 17 { return Err(e); }
            buf.resize(buf.len() * 4, 0);
        }
        // FILE_STREAM_INFO: NextEntryOffset u32, StreamNameLength u32 (bytes), StreamSize i64,
        // StreamAllocationSize i64, StreamName [u16]
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len() * 8) };
        let (mut at, mut out) = (0usize, Vec::new());
        loop {
            let field = |o: usize, n: usize| bytes.get(at + o..at + o + n).ok_or_else(|| std::io::Error::other("malformed stream list"));
            let next = u32::from_le_bytes(field(0, 4)?.try_into().unwrap()) as usize;
            let len = u32::from_le_bytes(field(4, 4)?.try_into().unwrap()) as usize;
            let size = i64::from_le_bytes(field(8, 8)?.try_into().unwrap());
            let name: Vec<u16> = field(24, len)?.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
            let name = String::from_utf16(&name).map_err(|_| std::io::Error::other("a stream name isn't valid text"))?;
            if name != "::$DATA" {
                out.push((name, u64::try_from(size).map_err(|_| std::io::Error::other("malformed stream size"))?));
            }
            if next == 0 { break; }
            at += next;
        }
        Ok(out)
    }

    /// Delete exactly the opened item (an empty folder, a file, a link), read-only or not.
    pub(super) fn delete(h: &Handle) -> std::io::Result<()> {
        // POSIX semantics (the name goes at once) and read-only ignored: Windows 10 1809 and later
        let flags: u32 = 0x1 | 0x2 | 0x10;
        if unsafe { SetFileInformationByHandle(h.0, 21, (&flags as *const u32).cast(), 4) } != 0 {
            return Ok(());
        }
        let first = std::io::Error::last_os_error();
        // 87 (an older Windows, or a volume without it): clear read-only, then delete on close
        if first.raw_os_error() != Some(87) && first.raw_os_error() != Some(1) {
            return Err(first);
        }
        let mut b = ByHandle::default();
        if unsafe { GetFileInformationByHandle(h.0, &mut b) } != 0 && b.attrs & 0x1 != 0 {
            let a = b.attrs & !0x1;
            let basic = Basic { created: 0, accessed: 0, written: 0, changed: 0, attrs: if a == 0 { 0x80 } else { a } };
            unsafe { SetFileInformationByHandle(h.0, 0, (&basic as *const Basic).cast(), std::mem::size_of::<Basic>() as u32) };
        }
        let yes: u8 = 1;
        if unsafe { SetFileInformationByHandle(h.0, 4, (&yes as *const u8).cast(), 1) } != 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    /// Move the opened item to `to` (same volume), never over something already there.
    pub(super) fn rename(h: &Handle, to: &Path) -> std::io::Result<()> {
        // FILE_RENAME_INFO: flags (4), padding, root handle (pointer), name length (4), name
        let name: Vec<u16> = std::ffi::OsStr::new(&format!(r"\??\{}", to.display())).encode_wide().collect();
        let head = 8 + std::mem::size_of::<usize>() + 4;
        let size = head + name.len() * 2 + 2;
        let mut buf = vec![0u64; size.div_ceil(8)];
        let bytes: &mut [u8] = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast(), buf.len() * 8) };
        // ReplaceIfExists stays 0, RootDirectory null
        bytes[8 + std::mem::size_of::<usize>()..head].copy_from_slice(&((name.len() * 2) as u32).to_le_bytes());
        for (k, c) in name.iter().enumerate() {
            bytes[head + 2 * k..head + 2 * k + 2].copy_from_slice(&c.to_le_bytes());
        }
        if unsafe { SetFileInformationByHandle(h.0, 3, buf.as_ptr().cast(), size as u32) } != 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    /// Mark the opened item to be deleted when its handle closes (`on`), or take the mark back.
    /// Not POSIX semantics: the name stays until the handle closes, so the mark can be undone.
    pub(super) fn mark_delete(h: &Handle, on: bool) -> std::io::Result<()> {
        // FILE_DISPOSITION_INFO_EX: DELETE and IGNORE_READONLY, or DO_NOT_DELETE (0)
        let flags: u32 = if on { 0x1 | 0x10 } else { 0 };
        if unsafe { SetFileInformationByHandle(h.0, 21, (&flags as *const u32).cast(), 4) } != 0 {
            return Ok(());
        }
        let first = std::io::Error::last_os_error();
        if first.raw_os_error() != Some(87) && first.raw_os_error() != Some(1) {
            return Err(first);
        }
        // an older Windows: FILE_DISPOSITION_INFO (a read-only file can't be marked this way)
        let yes: u8 = on.into();
        if unsafe { SetFileInformationByHandle(h.0, 4, (&yes as *const u8).cast(), 1) } != 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    /// Where the opened item really is.
    pub(super) fn final_path(h: &Handle) -> Option<PathBuf> {
        let mut buf = vec![0u16; 1024];
        loop {
            let n = unsafe { GetFinalPathNameByHandleW(h.0, buf.as_mut_ptr(), buf.len() as u32, 0) } as usize;
            if n == 0 {
                return None;
            }
            if n < buf.len() {
                return Some(PathBuf::from(String::from_utf16_lossy(&buf[..n])));
            }
            buf.resize(n + 1, 0);
        }
    }

    /// A job that ends every process in it when told to, or when Sprawl closes.
    pub(super) struct Job(Handle);

    impl Job {
        pub(super) fn new() -> Option<Job> {
            let h = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
            if h.is_null() {
                return None;
            }
            let job = Job(Handle(h));
            // JOBOBJECT_EXTENDED_LIMIT_INFORMATION, with LimitFlags = KILL_ON_JOB_CLOSE
            let mut info = [0u8; 144];
            let flags_at = 16;
            info[flags_at..flags_at + 4].copy_from_slice(&0x2000u32.to_le_bytes());
            let size = if cfg!(target_pointer_width = "64") { 144 } else { 112 };
            unsafe { SetInformationJobObject(job.0 .0, 9, info.as_ptr().cast(), size) };
            Some(job)
        }

        pub(super) fn add(&self, child: &std::process::Child) -> bool {
            use std::os::windows::io::AsRawHandle;
            unsafe { AssignProcessToJobObject(self.0 .0, child.as_raw_handle()) != 0 }
        }

        pub(super) fn end(&self) {
            unsafe { TerminateJobObject(self.0 .0, 1) };
        }
    }
}

pub(crate) fn system_dir() -> Option<PathBuf> {
    let mut buf = [0u16; 520];
    let n = unsafe { win::GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// Inbox programs directly in the system directory. Other tool installations require the
/// existing explicit administrator lockout override; broad folder ancestry isn't trust.
fn system_roots() -> Vec<PathBuf> {
    system_dir().into_iter().collect()
}

fn system_command(command: &str, sys: &Path) -> String {
    if let Some(rest) = command.strip_prefix("Start-Process cleanmgr ") {
        return format!("Start-Process {} {rest}", crate::rules::ps_quote(&sys.join("cleanmgr.exe").display().to_string()));
    }
    for (prefix, file) in [("DISM ", "Dism.exe"), ("compact.exe ", "compact.exe"), ("wsl.exe ", "wsl.exe")] {
        if let Some(rest) = command.strip_prefix(prefix) {
            return format!("& {} {rest}", crate::rules::ps_quote(&sys.join(file).display().to_string()));
        }
    }
    command.to_string()
}

/// Where PATH (with PATHEXT) finds `name`, as PowerShell would for a program.
fn on_path(name: &str) -> Option<PathBuf> {
    let exts: Vec<String> = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into()).split(';').filter(|e| !e.is_empty()).map(str::to_string).collect();
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if Path::new(name).extension().is_some() && dir.join(name).is_file() {
            return Some(dir.join(name));
        }
        for e in &exts {
            let p = dir.join(format!("{name}{e}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Whether a built-in tool command may run while Sprawl is elevated: every program it starts
/// must resolve to a folder only administrators can write to (a user-writable `npm.cmd` would
/// run as administrator). PowerShell's own commands (`Verb-Noun`) are fine; their modules are
/// limited to the system's folders when the command runs.
pub(crate) fn elevated_tool_ok(command: &str, resolve: &dyn Fn(&str) -> Option<PathBuf>, roots: &[PathBuf]) -> Result<(), String> {
    let program_ok = |p: &str| -> Result<(), String> {
        let Some(found) = resolve(p) else { return Err(format!("not run as administrator: {p} wasn't found")); };
        if !found.is_absolute() || found.components().any(|c| matches!(c, std::path::Component::ParentDir))
            || !found.parent().is_some_and(|parent| roots.iter().any(|r| same_path(parent, r))) {
            return Err(format!("not run as administrator: {p} is {}, outside the verified system program directory (your account can change other tool folders). Run from normal Sprawl.", found.display()));
        }
        Ok(())
    };
    if command.starts_with(crate::remedies::SHRINK) {
        let line = command.lines().find_map(|l| l.strip_prefix("$p = ")).ok_or("not run as administrator: unverified multi-step command")?;
        let (path, rest) = quoted(line).ok_or("not run as administrator: malformed shrink path")?;
        if !rest.trim().is_empty() || ![false, true].into_iter().any(|docker| crate::remedies::shrink_command(Path::new(&path), docker) == command) {
            return Err("not run as administrator: shrink command differs from the generated form".into());
        }
        let sys = system_dir().ok_or("Windows' system directory is unknown")?;
        for name in ["wsl.exe", "diskpart.exe"] { program_ok(&sys.join(name).display().to_string())?; }
        return Ok(());
    }
    let lines: Vec<_> = command.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    if lines.len() != 1 { return Err("not run as administrator: empty or unverified multi-step command".into()); }
    let line = lines[0];
    // Only one simple invocation. Quoted path punctuation is data; unquoted control syntax
    // must not hide a later program from the elevated gate.
    let mut chars = line.char_indices().peekable();
    let mut quote = None;
    let mut end = line.len();
    while let Some((i, c)) = chars.next() {
        match quote {
            Some(q) if c == q => {
                if chars.peek().is_some_and(|(_, next)| *next == q) { chars.next(); }
                else { quote = None; }
            },
            Some('"') if c == '$' && chars.peek().is_some_and(|(_, next)| *next == '(') => return Err("not run as administrator: nested command expression".into()),
            Some('"') if c == '`' => { chars.next(); },
            Some(_) => {},
            None if c == '#' => { end = i; break; },
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == '&' && i == 0 => {},
            None if matches!(c, ';' | '|' | '&' | '{' | '}' | '(' | ')' | '`') => return Err("not run as administrator: unverified compound command".into()),
            _ => {},
        }
    }
    if quote.is_some() { return Err("not run as administrator: unterminated quoted argument".into()); }
    let body = line[..end].trim();
    let (first, rest) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
    if first == "&" || first.eq_ignore_ascii_case("Start-Process") {
        let rest = rest.trim_start();
        let program = if let Some((p, _)) = quoted(rest) { p }
            else if let Some(rest) = rest.strip_prefix('"') {
                let p = rest.split_once('"').map(|(p, _)| p).ok_or("not run as administrator: malformed program")?;
                if p.contains(['$', '`']) { return Err("not run as administrator: computed program".into()); }
                p.to_string()
            } else { rest.split_whitespace().next().ok_or("not run as administrator: no program named")?.to_string() };
        return program_ok(&program);
    }
    if ["Clear-RecycleBin", "Delete-DeliveryOptimizationCache", "Write-Output", "Write-Host", "exit", "throw"].iter().any(|c| first.eq_ignore_ascii_case(c)) { return Ok(()); }
    program_ok(first)
}
/// Why a built-in tool command won't run from this Sprawl: it is elevated and the command starts
/// a program from a folder the user can change (see [`elevated_tool_ok`]). None when it runs.
pub fn elevated_refusal(command: &str) -> Option<String> {
    match checked_tool_elevation(sprawl_core::elevation()) {
        Ok(elevated) => refusal_unless_allowed(elevated_risk_at(command, elevated), crate::ELEVATED_USER_TOOLS.load(Ordering::Relaxed)),
        Err(error) => Some(error),
    }
}

fn checked_tool_elevation(status: std::io::Result<bool>) -> Result<bool, String> {
    status.map_err(|_| "Windows could not establish this process's elevation; no owner command will run.".into())
}

/// Why `command` would not be run as administrator (see [`elevated_tool_ok`]): None when Sprawl
/// isn't elevated or the command's programs are in system folders.
pub fn elevated_risk(command: &str) -> Option<String> {
    match checked_tool_elevation(sprawl_core::elevation()) {
        Ok(elevated) => elevated_risk_at(command, elevated),
        Err(error) => Some(error),
    }
}

fn elevated_risk_at(command: &str, elevated: bool) -> Option<String> {
    if !elevated {
        return None;
    }
    let Some(sys) = system_dir() else { return Some("Windows' system folder could not be established, so this elevated command cannot be checked.".into()); };
    let command = system_command(command, &sys);
    elevated_tool_ok(&command, &|name| {
        if Path::new(name).is_absolute() { return Some(PathBuf::from(name)); }
        let sys = system_dir()?;
        for candidate in [sys.join(name), sys.join(format!("{name}.exe"))] {
            if candidate.is_file() { return Some(candidate); }
        }
        on_path(name)
    }, &system_roots()).err()
}

/// The risk stops the command unless the user allowed it (Settings > Scanning, yes this session).
fn refusal_unless_allowed(risk: Option<String>, allowed: bool) -> Option<String> {
    risk.filter(|_| !allowed).map(|r| format!("{r} You can allow it in Settings > Scanning."))
}

/// PowerShell's -EncodedCommand form: base64 of the UTF-16 text.
fn encoded(s: &str) -> String {
    const B: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut out = String::with_capacity(bytes.len() / 3 * 4 + 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= c.len() { B[(n >> shift & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// A built-in tool command, by Windows PowerShell without a window, from System32 (so no tool
/// reads settings from the folder Sprawl started in); its last lines of output are kept. The
/// tool and anything it starts run in a job, so Stop (or closing Sprawl) ends all of them.
fn tool(command: &str, stop: &AtomicBool, o: &mut Outcome) {
    tool_with(command, stop, o, &[]);
}

/// The most output a tool's answer is read whole up to.
const TOOL_OUTPUT: usize = 1 << 20;

/// [`tool`], with `env` set for it. Its whole output (each line as the tool wrote it, never
/// wrapped) when that was read completely and is at most [`TOOL_OUTPUT`]; None when it was
/// longer, a read failed, or the run was stopped.
fn tool_with(command: &str, stop: &AtomicBool, o: &mut Outcome, env: &[(&str, &std::ffi::OsStr)]) -> Option<String> {
    let Some(sys) = system_dir() else { o.problem(Path::new(command), "Windows' System32 folder wasn't found"); return None };
    let command = system_command(command, &sys);
    let command = command.as_str();
    let elevated = match checked_tool_elevation(sprawl_core::elevation()) {
        Ok(elevated) => elevated,
        Err(error) => { o.problem(Path::new(command), error); return None }
    };
    if let Some(why) = refusal_unless_allowed(elevated_risk_at(command, elevated), crate::ELEVATED_USER_TOOLS.load(Ordering::Relaxed)) {
        o.problem(Path::new(command), why);
        return None;
    }
    run_powershell(command, &sys, elevated, stop, o, env)
}

/// The run itself, once [`tool_with`] has let `command` through: see there.
fn run_powershell(command: &str, sys: &Path, elevated: bool, stop: &AtomicBool, o: &mut Outcome, env: &[(&str, &std::ffi::OsStr)]) -> Option<String> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    let ps = sys.join(r"WindowsPowerShell\v1.0\powershell.exe");
    // the command on lines of its own (a trailing comment can't swallow the rest), its exit code
    // kept, and the output in UTF-8
    // A program's text on stderr is not a failure; any other error (not found, a cmdlet's) is.
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8\n$Error.Clear()\n$o = & {{\n{command}\n}} 2>&1; $c = $LASTEXITCODE\nforeach ($l in @($o)) {{ [Console]::Out.WriteLine(\"$l\") }}\n\
         $bad = @($Error | Where-Object {{ \"$($_.FullyQualifiedErrorId)\" -notlike 'NativeCommandError*' }})\n\
         if ($c) {{ exit $c }} elseif ($bad.Count) {{ exit 1 }}"
    );
    let mut cmd = std::process::Command::new(ps);
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encoded(&script)])
        .current_dir(&sys)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    for (k, v) in env {
        cmd.env(k, v);
    }
    if elevated {
        // no PowerShell module from the user's own folders loads into an elevated run
        cmd.env("PSModulePath", sys.join(r"WindowsPowerShell\v1.0\Modules"));
        if !crate::ELEVATED_USER_TOOLS.load(Ordering::Relaxed) {
            cmd.env("PATH", &sys).env("PATHEXT", ".EXE;.COM");
        }
    }
    let job = win::Job::new();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => { o.problem(Path::new(command), e); return None }
    };
    let in_job = job.as_ref().is_some_and(|j| j.add(&child));
    let mut out = child.stdout.take();
    let reader = std::thread::spawn(move || {
        // read to the end (a tool must never block on a full pipe); past TOOL_OUTPUT the
        // answer is cut (only its last part is kept, to show), and a failed read cuts it too
        let mut keep: Vec<u8> = Vec::new();
        let mut cut = out.is_none();
        let mut buf = [0u8; 8192];
        while let Some(o) = out.as_mut() {
            match o.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    keep.extend_from_slice(&buf[..n]);
                    if keep.len() > TOOL_OUTPUT {
                        cut = true;
                        if keep.len() > 2 * TOOL_OUTPUT {
                            keep.drain(..keep.len() - TOOL_OUTPUT);
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => {
                    cut = true;
                    break;
                }
            }
        }
        (String::from_utf8_lossy(&keep).into_owned(), cut)
    });
    let started = std::time::Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.code(),
            Ok(None) => {
                // a tool that hangs (DISM can take long; an hour is the limit)
                if stop.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(3600) {
                    match &job {
                        Some(j) if in_job => j.end(),
                        _ => {
                            let _ = child.kill();
                        }
                    }
                    let _ = child.wait();
                    o.stopped = true;
                    break None;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                o.problem(Path::new(command), e);
                break None;
            }
        }
    };
    // what the tool started may still hold the pipe: end the job before waiting for the output
    if let Some(j) = &job {
        if in_job {
            j.end();
        }
    }
    let (text, cut) = if o.stopped && !in_job { (String::new(), true) } else { reader.join().unwrap_or((String::new(), true)) };
    let lines: Vec<String> = text.lines().map(str::trim_end).filter(|l| !l.is_empty()).map(|l| crate::llm::clip(l.to_string(), 200)).collect();
    o.output = lines[lines.len().saturating_sub(12)..].to_vec();
    if let Some(c) = code.filter(|&c| c != 0) {
        o.problem(Path::new(command.split_whitespace().next().unwrap_or(command)), format!("ended with code {c}"));
    }
    (!cut && !o.stopped).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::ps_quote;
    use crate::{Remedy, Risk, Source};

    #[test]
    fn recycle_never_falls_back_to_deleting_wof_cloud_or_unknown_reparse_files() {
        let mut info = win::Info { attrs: REPARSE, tag: 0x8000_0017, size: 99, links: 1, created: 0, newest: 0, id: 0 };
        assert!(recycling_refused(&info), "WOF is ordinary data, not a disposable link");
        info.tag = 0;
        assert!(recycling_refused(&info), "an unknown tag must stay");
        info.tag = 0xA000_000C;
        assert!(!recycling_refused(&info), "a known symlink is removed only as the link itself");
        info.attrs |= 0x0040_0000;
        assert!(recycling_refused(&info), "a cloud object must stay");
        info.attrs = sprawl_core::attr::COMPRESSED;
        assert!(!recycling_refused(&info), "plain NTFS compression can be moved to the bin");
    }

    #[test]
    fn a_copy_with_an_extra_stream_of_its_own_is_not_linked() {
        let owned = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-link-streams").unwrap();
        let root = owned.path().to_path_buf();
        std::fs::create_dir_all(&root).unwrap();
        let keep = root.join("keep.bin");
        let copy = root.join("copy.bin");
        std::fs::write(&keep, b"same bytes").unwrap();
        std::fs::write(&copy, b"same bytes").unwrap();
        // the copy alone carries data in a named stream (Astra 1.0.2 S4 #5)
        std::fs::write(format!("{}:notes", copy.display()), b"only here").unwrap();
        let mut o = Outcome::default();
        link(&keep, &[copy.clone()], &Spare::default(), &AtomicBool::new(false), &mut o, &mut |_| {});
        assert_eq!(o.files, 0, "{:?}", o.problems);
        assert!(o.problems.iter().any(|p| p.contains("extra stream")), "{:?}", o.problems);
        assert_eq!(std::fs::read(format!("{}:notes", copy.display())).unwrap(), b"only here");
        // the kept copy alone has one: the copy's name would gain it, and it is held too
        std::fs::write(&copy, b"same bytes").unwrap();
        std::fs::write(format!("{}:notes", keep.display()), b"kept only").unwrap();
        let mut o = Outcome::default();
        link(&keep, &[copy.clone()], &Spare::default(), &AtomicBool::new(false), &mut o, &mut |_| {});
        assert_eq!(o.files, 0, "{:?}", o.problems);
        // the very same stream on both is held too: its bytes could change after any comparison
        // (Astra 1.0.2 re-review S4 #5)
        std::fs::write(format!("{}:notes", copy.display()), b"kept only").unwrap();
        let mut o = Outcome::default();
        link(&keep, &[copy.clone()], &Spare::default(), &AtomicBool::new(false), &mut o, &mut |_| {});
        assert_eq!(o.files, 0, "{:?}", o.problems);
        let file = std::fs::File::open(&keep).unwrap();
        assert_eq!(win::named_streams(&file).unwrap(), vec![(":notes:$DATA".to_string(), 9)]);
        drop(file);
        // a stream added to the original between the link and its removal: the original stays
        std::fs::remove_file(&keep).unwrap();
        std::fs::write(&keep, b"same bytes").unwrap();
        std::fs::remove_file(&copy).unwrap();
        std::fs::write(&copy, b"same bytes").unwrap();
        let mut o = Outcome::default();
        link_with(&keep, &[copy.clone()], &Spare::default(), &AtomicBool::new(false), &mut o, &mut |_| {},
            &mut |c| std::fs::write(format!("{}.sprawl-original:late", c.display()), b"late").unwrap(), &mut |_| {});
        assert_eq!((o.files, o.freed), (0, 0), "{:?}", o.problems);
        assert_eq!(std::fs::read(format!("{}.sprawl-original:late", copy.display())).unwrap(), b"late");
        let backup = root.join("copy.bin.sprawl-original");
        assert!(backup.exists(), "the mark was called off: the original stays");
        std::fs::remove_file(&backup).unwrap();
        // after the last list, until the original's handle closes, no stream can be added to it
        // (round 3: the list and the removal were separate steps)
        std::fs::remove_file(&copy).unwrap();
        std::fs::write(&copy, b"same bytes").unwrap();
        let mut o = Outcome::default();
        let mut tried = Vec::new();
        link_with(&keep, &[copy.clone()], &Spare::default(), &AtomicBool::new(false), &mut o, &mut |_| {}, &mut |_| {}, &mut |b| {
            for (how, r) in [("new", std::fs::OpenOptions::new().write(true).create_new(true).open(format!("{}:later", b.display()))),
                ("any", std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(format!("{}:later", b.display()))),
                ("main", std::fs::OpenOptions::new().read(true).open(b))] {
                tried.push((how, r.is_ok()));
            }
        });
        assert_eq!(tried, [("new", false), ("any", false), ("main", false)], "the original can't be opened, nor a stream added");
        assert_eq!(o.files, 1, "{:?}", o.problems);
        assert!(!backup.exists());
        drop(owned);
    }

    #[test]
    fn linking_preserves_a_concurrent_replacement_and_external_alias() {
        let root = std::env::temp_dir().join(format!("sprawl-link-race-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let keep = root.join("keep.bin");
        let copy = root.join("copy.bin");
        let alias = root.join("external.bin");
        std::fs::write(&keep, b"same bytes").unwrap();
        std::fs::write(&copy, b"same bytes").unwrap();
        let mut outcome = Outcome::default();
        link_with(&keep, &[copy.clone()], &Spare::default(), &AtomicBool::new(false), &mut outcome, &mut |_| {},
            &mut |vacant| std::fs::write(vacant, b"new unrelated data").unwrap(), &mut |_| {});
        assert_eq!(std::fs::read(&copy).unwrap(), b"new unrelated data");
        let backup = root.join("copy.bin.sprawl-original");
        assert_eq!(std::fs::read(&backup).unwrap(), b"same bytes");
        assert_eq!(outcome.freed, 0);
        assert!(outcome.problems.iter().any(|p| p.contains("original preserved")));
        // A pre-existing backup is never removed or overwritten either.
        let mut outcome = Outcome::default();
        std::fs::write(&copy, b"same bytes").unwrap();
        link(&keep, &[copy.clone()], &Spare::default(), &AtomicBool::new(false), &mut outcome, &mut |_| {});
        assert_eq!(outcome.files, 0);
        assert_eq!(std::fs::read(&backup).unwrap(), b"same bytes");
        std::fs::remove_file(&backup).unwrap();
        std::fs::hard_link(&copy, &alias).unwrap();
        let mut outcome = Outcome::default();
        link(&keep, &[copy.clone()], &Spare::default(), &AtomicBool::new(false), &mut outcome, &mut |_| {});
        assert_eq!((outcome.files, outcome.freed), (1, 0), "an external alias still owns the original data: {:?}", outcome.problems);
        assert_eq!(std::fs::read(&alias).unwrap(), b"same bytes");
        assert!(!backup.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn excluded_live_privacy_journal_holds_the_whole_set() {
        let base = std::env::temp_dir().join(format!("sprawl-set-excluded-{}", std::process::id()));
        let places = crate::privacy::Places { local: base.join("Local"), roaming: base.join("Roaming") };
        let profile = places.local.join(r"Google\Chrome\User Data\Default\Network");
        std::fs::create_dir_all(&profile).unwrap();
        let db = profile.join("Cookies");
        let journal = profile.join("Cookies-journal");
        std::fs::write(&db, b"database").unwrap();
        std::fs::write(&journal, b"journal").unwrap();
        let part = crate::privacy::part("privacy.chrome/cookies").unwrap();
        assert_eq!(set_members_with_exclusions(part, &[db.clone()], &places, &[]).unwrap().len(), 2);
        assert!(set_members_with_exclusions(part, &[db.clone()], &places, &[journal.clone()]).unwrap_err().contains("excluded"));
        assert!(db.exists() && journal.exists());
        std::fs::remove_dir_all(base).unwrap();
    }

    fn f(path: &str, cmd: String) -> Finding {
        Finding {
            path: path.into(),
            owner: None,
            bytes: 0,
            risk: Risk::Green,
            remedy: Remedy::Delete,
            command: Some(cmd),
            title: "t".into(),
            evidence: vec![],
            confidence: 1.0,
            source: Source::Rule("r".into()),
            allow_protected: false,
            trusted: false,
            builtin: false,
            on_path: true,
            action: None,
            admin: false,
            carved: Vec::new(),
            gain: None,
            forced_from: None,
        }
    }

    #[test]
    fn typed_actions_and_script_agree() {
        let p = r"C:\Users\sam\it's ‘odd’";
        for (a, admin) in [
            (Action::Remove, false),
            (Action::Clear { days: 7 }, true),
            (Action::Clear { days: 0 }, false),
            (Action::Match { pattern: "*.d'mp".into() }, true),
        ] {
            let mut x = f(p, String::new());
            x.set_action(a.clone(), admin);
            assert_eq!(action(&x), Some((a.clone(), admin)));
            assert_eq!(parse_command(&x), Some((a, admin)), "{:?}", x.command);
        }
        // command text alone does nothing: the action is typed
        let q = ps_quote(p);
        assert_eq!(action(&f(p, format!("{RM_FN}; sprawl-clear {q} 0"))), None);
        // a tool command only from the built-in pack
        let mut t = f(p, String::new());
        t.set_action(Action::Tool { command: "npm cache clean --force".into() }, false);
        assert_eq!(action(&t), None);
        t.trusted = true;
        t.builtin = true;
        assert_eq!(action(&t), Some((Action::Tool { command: "npm cache clean --force".into() }, false)));
        // a pattern can't reach into other folders
        let mut m = f(p, String::new());
        m.set_action(Action::Match { pattern: r"..\*".into() }, false);
        assert_eq!(action(&m), None);
    }

    /// What a privacy set stands on: a file opens alone only when nobody else has it open, and a
    /// delete mark can be taken back until the handle closes.
    #[test]
    fn a_delete_mark_can_be_taken_back() {
        let base = std::env::temp_dir().join(format!("sprawl-mark-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let (a, b) = (base.join("a.db"), base.join("b.db"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        // marked, then taken back: it stays
        let (h, _) = win::open_alone(&a).unwrap();
        win::mark_delete(&h, true).unwrap();
        win::mark_delete(&h, false).unwrap();
        drop(h);
        assert!(a.exists());
        // read-only files too
        let mut perm = std::fs::metadata(&b).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&b, perm).unwrap();
        let (h, _) = win::open_alone(&b).unwrap();
        // held alone: nobody else can open it meanwhile
        assert!(std::fs::File::open(&b).is_err());
        win::mark_delete(&h, true).unwrap();
        assert!(b.exists(), "nothing goes before the handle closes");
        drop(h);
        assert!(!b.exists());
        // anyone else holding it: it doesn't open alone
        let other = std::fs::File::open(&a).unwrap();
        assert!(win::open_alone(&a).is_err());
        drop(other);
        assert!(win::open_alone(&a).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn like_patterns() {
        assert!(like("crash.DMP", "*.dmp"));
        assert!(!like("crash.dmpx", "*.dmp"));
        assert!(like("a1.log", "a?.log"));
        assert!(like("b.log", "[a-c].log"));
        assert!(!like("d.log", "[a-c].log"));
        assert!(like("[x", "[x"), "a [ without its ] is itself");
        assert!(like("abc", "*") && like("", "*") && !like("", "?"));
        // many stars against a long name that doesn't match: quick, not exponential
        let t = std::time::Instant::now();
        assert!(!like(&"a".repeat(200), "*a*a*a*a*a*a*a*a*b"));
        assert!(t.elapsed() < std::time::Duration::from_secs(1), "{:?}", t.elapsed());
    }

    #[test]
    fn removes_without_following_links() {
        let base = std::env::temp_dir().join(format!("sprawl-clean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let target = base.join("keep");
        let cache = base.join("cache");
        std::fs::create_dir_all(target.join("inner")).unwrap();
        std::fs::write(target.join("inner").join("precious.txt"), b"keep me").unwrap();
        std::fs::create_dir_all(cache.join("sub")).unwrap();
        std::fs::write(cache.join("a.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(cache.join("sub").join("b.dmp"), vec![0u8; 500]).unwrap();
        std::fs::write(cache.join("sub").join("c.txt"), vec![0u8; 200]).unwrap();
        let ro = cache.join("ro.bin");
        std::fs::write(&ro, vec![0u8; 300]).unwrap();
        let mut perm = std::fs::metadata(&ro).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&ro, perm).unwrap();
        // a junction inside the cache, pointing at what must stay
        let link = cache.join("link");
        let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&target).output().is_ok_and(|o| o.status.success());
        let stop = AtomicBool::new(false);

        let m = run(&cache, &Action::Match { pattern: "*.dmp".into() }, &Spare::default(), &stop, &mut |_| {});
        assert_eq!((m.freed, m.files), (500, 1));
        assert!(cache.join("sub").join("c.txt").exists());

        let c = run(&cache, &Action::Clear { days: 7 }, &Spare::default(), &stop, &mut |_| {});
        assert_eq!(c.freed, 0, "everything is new: nothing goes");

        let c = run(&cache, &Action::Clear { days: 0 }, &Spare::default(), &stop, &mut |_| {});
        assert_eq!(c.freed, 1000 + 200 + 300, "{:?}", c.problems);
        let left: Vec<_> = std::fs::read_dir(&cache).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(cache.exists() && left.is_empty(), "emptied, kept: {left:?} {:?}", c.problems);
        assert!(target.join("inner").join("precious.txt").exists(), "the junction's target stays");
        if ok {
            assert!(!link.exists(), "the junction itself went");
        }

        // a cache that became a link is not cleared through it
        let _ = std::fs::remove_dir(&cache);
        if std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&cache).arg(&target).output().is_ok_and(|o| o.status.success()) {
            let c = run(&cache, &Action::Clear { days: 0 }, &Spare::default(), &stop, &mut |_| {});
            assert_eq!(c.freed, 0);
            assert!(target.join("inner").join("precious.txt").exists());
            let r = run(&cache, &Action::Remove, &Spare::default(), &stop, &mut |_| {});
            assert!(r.problems.is_empty(), "{:?}", r.problems);
            assert!(!cache.exists() && target.join("inner").join("precious.txt").exists(), "removing a link leaves its target");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_path_that_leads_elsewhere_is_left_alone() {
        let base = std::env::temp_dir().join(format!("sprawl-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let target = base.join("target");
        std::fs::create_dir_all(target.join("cache")).unwrap();
        std::fs::write(target.join("cache").join("victim.txt"), b"keep me").unwrap();
        // "parent" is a junction to "target": parent\cache is really target\cache
        let parent = base.join("parent");
        if std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&parent).arg(&target).output().is_ok_and(|o| o.status.success()) {
            let o = run(&parent.join("cache"), &Action::Clear { days: 0 }, &Spare::default(), &AtomicBool::new(false), &mut |_| {});
            assert_eq!(o.freed, 0, "{:?}", o.problems);
            assert!(target.join("cache").join("victim.txt").exists());
            assert!(o.problems.iter().any(|p| p.contains("leads to") || p.contains("ancestor is a junction")), "{:?}", o.problems);
        }
        // a folder being cleaned can't be renamed (and swapped for a link) while it is open
        let cache = base.join("cache2");
        std::fs::create_dir_all(&cache).unwrap();
        let (h, _) = win::open(&cache).unwrap();
        assert!(std::fs::rename(&cache, base.join("moved")).is_err());
        drop(h);
        // a vault that appeared after the scan stays
        std::fs::write(cache.join("passwords.kdbx"), b"x").unwrap();
        std::fs::write(cache.join("junk.tmp"), b"x").unwrap();
        let o = run(&cache, &Action::Clear { days: 0 }, &Spare::default(), &AtomicBool::new(false), &mut |_| {});
        assert!(cache.join("passwords.kdbx").exists() && !cache.join("junk.tmp").exists(), "{:?}", o.problems);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// What a scan of `base` taken now would have seen, for [`Spare::seen`].
    fn seen_now(base: &Path) -> Option<Seen> {
        Some(seen_in(std::sync::Arc::new(sprawl_core::walk::inventory(base, &sprawl_core::Progress::new()).unwrap())))
    }

    #[test]
    fn only_the_items_the_scan_saw_are_gone_through() {
        // Astra 1.0.2 S1 #3 (round 3): what is at a path must be the item the scan saw there, by
        // its id; name, kind and length are not enough
        let owned = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-moved-dir").unwrap();
        let base = owned.path().to_path_buf();
        let cache = base.join("cache");
        let stop = AtomicBool::new(false);
        let fill = |at: &Path, byte: u8| {
            std::fs::create_dir_all(at.join("was_here")).unwrap();
            std::fs::write(at.join("old.bin"), vec![byte; 30]).unwrap();
            std::fs::write(at.join("was_here").join("old2.bin"), vec![byte; 20]).unwrap();
        };
        let scanned = || {
            std::thread::sleep(Duration::from_millis(50));
            let scan = SystemTime::now();
            let seen = seen_now(&base);
            std::thread::sleep(Duration::from_millis(50));
            Spare { since: Some(scan), keep: Vec::new(), seen }
        };
        // 1. a whole old folder moved into the cache keeps its old times, as do the files in
        // it: it stays, and what the scan saw in the cache goes
        fill(&cache, 0);
        std::fs::create_dir_all(base.join("mine").join("deep")).unwrap();
        std::fs::write(base.join("mine").join("deep").join("thesis.doc"), vec![0u8; 9]).unwrap();
        let spare = scanned();
        std::fs::rename(base.join("mine"), cache.join("mine")).unwrap();
        let o = run(&cache, &Action::Clear { days: 0 }, &spare, &stop, &mut |_| {});
        assert!(cache.join("mine").join("deep").join("thesis.doc").exists(), "took a moved-in file: {:?}", o.problems);
        assert!(!cache.join("old.bin").exists() && !cache.join("was_here").exists(), "{:?}", o.problems);
        assert_eq!((o.freed, o.kept_new), (50, 1), "{:?}", o.problems);
        std::fs::remove_dir_all(cache.join("mine")).unwrap();
        // 2. an old look-alike put in place of the reviewed folder: the same names and lengths,
        // other bytes. Not the folder the scan saw: held whole
        fill(&cache, 0);
        fill(&base.join("elsewhere"), 7);
        let spare = scanned();
        std::fs::rename(&cache, base.join("cache.was")).unwrap();
        std::fs::rename(base.join("elsewhere"), &cache).unwrap();
        let o = run(&cache, &Action::Clear { days: 0 }, &spare, &stop, &mut |_| {});
        assert!(cache.join("old.bin").exists() && cache.join("was_here").join("old2.bin").exists(), "{:?}", o.problems);
        assert_eq!(o.freed, 0);
        // 3. only a folder inside it replaced so: the cache is the one scanned, so its own file
        // goes; the look-alike folder isn't the one the scan saw there, and stays with its file
        std::fs::remove_dir_all(&cache).unwrap();
        std::fs::rename(base.join("cache.was"), &cache).unwrap();
        fill(&base.join("elsewhere"), 7);
        let spare = scanned();
        std::fs::rename(cache.join("was_here"), base.join("was_here.was")).unwrap();
        std::fs::rename(base.join("elsewhere").join("was_here"), cache.join("was_here")).unwrap();
        let o = run(&cache, &Action::Clear { days: 0 }, &spare, &stop, &mut |_| {});
        assert!(!cache.join("old.bin").exists() && std::fs::read(cache.join("was_here").join("old2.bin")).unwrap() == vec![7u8; 20], "{:?}", o.problems);
        assert_eq!((o.freed, o.kept_new), (30, 1));
        // 4. a file the scan saw, still there with its id, goes; one made since stays
        std::fs::remove_dir_all(&cache).unwrap();
        fill(&cache, 0);
        let spare = scanned();
        std::fs::write(cache.join("fresh.bin"), b"new").unwrap();
        let o = run(&cache, &Action::Clear { days: 0 }, &spare, &stop, &mut |_| {});
        assert!(!cache.join("old.bin").exists() && cache.join("fresh.bin").exists(), "{:?}", o.problems);
        // 5. without knowing what the scan saw, a changed folder is never gone through
        fill(&cache, 0);
        let blind = Spare { seen: None, ..scanned() };
        std::fs::write(cache.join("fresh2.bin"), b"new").unwrap();
        let o = run(&cache, &Action::Remove, &blind, &stop, &mut |_| {});
        assert!(cache.join("old.bin").exists() && o.freed == 0, "{:?}", o.problems);
        drop(owned);
    }

    #[test]
    fn what_is_new_since_the_scan_stays() {
        let base = std::env::temp_dir().join(format!("sprawl-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let cache = base.join("cache");
        std::fs::create_dir_all(cache.join("old_dir")).unwrap();
        std::fs::write(cache.join("old.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(cache.join("old_dir").join("old2.bin"), vec![0u8; 50]).unwrap();
        std::fs::write(cache.join("old.dmp"), vec![0u8; 10]).unwrap();
        // the scan begins here; NTFS times have 100 ns steps, so wait past the same tick
        std::thread::sleep(Duration::from_millis(50));
        let scan = SystemTime::now();
        let seen = seen_now(&base);
        std::thread::sleep(Duration::from_millis(50));
        std::fs::write(cache.join("new.bin"), vec![0u8; 7]).unwrap();
        std::fs::write(cache.join("old_dir").join("new2.bin"), vec![0u8; 7]).unwrap();
        std::fs::create_dir_all(cache.join("new_dir")).unwrap();
        std::fs::write(cache.join("new_dir").join("x.bin"), vec![0u8; 7]).unwrap();
        std::fs::write(cache.join("new.dmp"), vec![0u8; 7]).unwrap();
        let spare = Spare { since: Some(scan), keep: vec![], seen: seen.clone() };
        let stop = AtomicBool::new(false);

        // Match counts only the new files it would have removed
        let m = run(&cache, &Action::Match { pattern: "*.dmp".into() }, &spare, &stop, &mut |_| {});
        assert_eq!((m.freed, m.kept_new), (10, 1), "{:?}", m.problems);
        assert!(cache.join("new.dmp").exists());

        let o = run(&cache, &Action::Clear { days: 0 }, &spare, &stop, &mut |_| {});
        assert_eq!(o.freed, 150, "{:?}", o.problems);
        // new.bin, new.dmp, new_dir (whole), old_dir\new2.bin
        assert_eq!(o.kept_new, 4);
        assert!(cache.join("new.bin").exists() && cache.join("new_dir").join("x.bin").exists() && cache.join("old_dir").join("new2.bin").exists());
        assert!(!cache.join("old.bin").exists() && !cache.join("old_dir").join("old2.bin").exists());

        // a whole folder made after the scan is left as it is
        let r = run(&cache.join("new_dir"), &Action::Remove, &spare, &stop, &mut |_| {});
        assert_eq!((r.freed, r.kept_new), (0, 1));
        assert!(cache.join("new_dir").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn what_the_user_keeps_stays_at_any_depth() {
        let base = std::env::temp_dir().join(format!("sprawl-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let temp = base.join("Temp");
        std::fs::create_dir_all(temp.join("MyTool").join("deep")).unwrap();
        std::fs::create_dir_all(temp.join("junk")).unwrap();
        std::fs::write(temp.join("MyTool").join("deep").join("state.db"), vec![0u8; 40]).unwrap();
        std::fs::write(temp.join("junk").join("a.tmp"), vec![0u8; 30]).unwrap();
        std::fs::write(temp.join("b.tmp"), vec![0u8; 20]).unwrap();
        std::fs::write(temp.join("mine.txt"), vec![0u8; 5]).unwrap();
        // kept in another case and with a trailing slash: the same folder
        let spare = Spare { since: None, keep: vec![PathBuf::from(temp.join("mytool").to_string_lossy().to_uppercase() + "\\"), temp.join("mine.txt")], ..Default::default() };
        assert!(spare.keeps(&temp.join("MyTool").join("deep").join("state.db")));
        assert!(!spare.keeps(&temp.join("MyToolbox")), "a name that only starts the same is not inside");
        let o = run(&temp, &Action::Clear { days: 0 }, &spare, &AtomicBool::new(false), &mut |_| {});
        assert_eq!((o.freed, o.kept_chosen), (50, 2), "{:?}", o.problems);
        assert!(temp.join("MyTool").join("deep").join("state.db").exists() && temp.join("mine.txt").exists());
        assert!(!temp.join("junk").exists() && !temp.join("b.tmp").exists());
        // Remove on a folder that holds a kept one: the rest goes, the kept one and its parents stay
        let o = run(&base, &Action::Remove, &spare, &AtomicBool::new(false), &mut |_| {});
        assert!(o.problems.is_empty(), "{:?}", o.problems);
        assert!(temp.join("MyTool").join("deep").join("state.db").exists());
        // and a target that is itself kept is not touched
        let o = run(&temp.join("MyTool"), &Action::Remove, &spare, &AtomicBool::new(false), &mut |_| {});
        assert_eq!((o.freed, o.kept_chosen), (0, 1));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A tool command from the user's folders, as administrator: held back unless allowed.
    #[test]
    fn a_risky_tool_runs_only_when_allowed() {
        assert!(checked_tool_elevation(Err(std::io::Error::other("token unavailable"))).is_err(), "unknown status is rejected before the override is considered");
        assert_eq!(checked_tool_elevation(Ok(false)).unwrap(), false);
        assert_eq!(checked_tool_elevation(Ok(true)).unwrap(), true);
        let risk = Some(r"not run as administrator: npm is C:\Users\sam\npm.cmd".to_string());
        assert!(refusal_unless_allowed(risk.clone(), false).is_some_and(|w| w.contains("Settings > Scanning")));
        assert_eq!(refusal_unless_allowed(risk, true), None);
        assert_eq!(refusal_unless_allowed(None, false), None);
        assert!(!crate::ELEVATED_USER_TOOLS.load(std::sync::atomic::Ordering::Relaxed), "off until the app says so");
    }

    #[test]
    fn elevated_tools_come_from_system_folders_only() {
        let roots = vec![PathBuf::from(r"C:\Windows\System32")];
        let resolve = |n: &str| -> Option<PathBuf> {
            match n.to_lowercase().as_str() {
                "dism" => Some(r"C:\Windows\System32\Dism.exe".into()),
                "cleanmgr" => Some(r"C:\Windows\System32\cleanmgr.exe".into()),
                "npm" => Some(r"C:\Users\sam\AppData\Roaming\npm\npm.cmd".into()),
                "dotnet" => Some(r"C:\Program Files\dotnet\dotnet.exe".into()),
                _ => None,
            }
        };
        assert!(elevated_tool_ok("DISM /Online /Cleanup-Image /StartComponentCleanup   # as administrator", &resolve, &roots).is_ok());
        assert!(elevated_tool_ok("dotnet nuget locals all --clear", &resolve, &roots).is_err());
        assert!(elevated_tool_ok("Clear-RecycleBin -Force", &resolve, &roots).is_ok());
        assert!(elevated_tool_ok("exit 3", &resolve, &roots).is_ok(), "a PowerShell word, not a program");
        assert!(elevated_tool_ok("npm-run cache", &resolve, &roots).is_err(), "a hyphen alone doesn't make a cmdlet");
        assert!(elevated_tool_ok("Start-Process cleanmgr -ArgumentList \"/d $env:SystemDrive\" -Verb RunAs", &resolve, &roots).is_ok());
        let e = elevated_tool_ok("npm cache clean --force", &resolve, &roots).unwrap_err();
        assert!(e.contains("your account can change"), "{e}");
        assert!(elevated_tool_ok("pip cache purge", &resolve, &roots).is_err(), "not found: not run");
        for command in ["# comment only", "# leading comment\nif ($true) { npm cache clean }", "Clear-RecycleBin; npm cache clean", "if ($true) { npm cache clean }", "Clear-RecycleBin | npm", "Write-Output \"$(npm cache clean)\""] {
            assert!(elevated_tool_ok(command, &resolve, &roots).is_err(), "{command}");
        }
        let nested = |_: &str| Some(PathBuf::from(r"C:\Windows\System32\writable\evil.exe"));
        assert!(elevated_tool_ok("evil.exe", &nested, &roots).is_err(), "only direct protected system programs");
        // a lookalike folder name is not Program Files
        let near = |_: &str| Some(PathBuf::from(r"C:\Program Files Evil\x.exe"));
        assert!(elevated_tool_ok("x", &near, &roots).is_err());
    }

    #[test]
    fn what_would_go_is_counted_by_the_run_s_own_tests() {
        let base = std::env::temp_dir().join(format!("sprawl-would-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let logs = base.join("logs");
        std::fs::create_dir_all(logs.join("sub")).unwrap();
        std::fs::write(logs.join("a.log"), vec![0u8; 100]).unwrap();
        std::fs::write(logs.join("sub").join("b.log"), vec![0u8; 20]).unwrap();
        std::fs::write(logs.join("c.txt"), vec![0u8; 5]).unwrap();
        // modified 40 days ago, but created just now: "new" to keep-days, as to the run
        let old = std::fs::File::options().write(true).open(logs.join("a.log")).unwrap();
        old.set_modified(SystemTime::now() - Duration::from_secs(40 * 86_400)).unwrap();
        drop(old);
        let none = Spare::default();
        let m = Action::Match { pattern: "*.log".into() };
        assert_eq!(would_free(&logs, &m, &none), Some(120));
        let keep7 = Action::Clear { days: 7 };
        assert_eq!(would_free(&logs, &keep7, &none), Some(0), "the old-logs case: nothing would go");
        assert_eq!(run(&logs, &keep7, &none, &AtomicBool::new(false), &mut |_| {}).freed, 0, "and nothing does");
        assert_eq!(would_free(&logs, &Action::Remove, &none), None);
        // what the user keeps is not counted
        let kept = Spare { since: None, keep: vec![logs.join("sub")], ..Default::default() };
        assert_eq!(would_free(&logs, &m, &kept), Some(100));
        let freed = run(&logs, &m, &kept, &AtomicBool::new(false), &mut |_| {}).freed;
        assert_eq!(freed, 100, "the count and the run agree");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn deep_folders_dont_overflow() {
        let base = std::env::temp_dir().join(format!("sprawl-deep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mut p = base.clone();
        for _ in 0..1200 {
            p.push("d");
        }
        // std makes the long paths work
        if std::fs::create_dir_all(&p).is_ok() {
            std::fs::write(p.join("f.bin"), b"abc").unwrap();
            let o = run(&base, &Action::Remove, &Spare::default(), &AtomicBool::new(false), &mut |_| {});
            assert_eq!(o.freed, 3, "{:?}", o.problems);
            assert!(!base.exists());
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The report's case: a temp folder with a mail store and a link inside. The finding is
    /// kept, the mail store carved out of it; the run leaves it and removes the link as a link.
    #[test]
    fn what_is_protected_inside_is_carved_out_and_left() {
        let base = std::env::temp_dir().join(format!("sprawl-carve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (temp, outside) = (base.join("Temp"), base.join("outside"));
        std::fs::create_dir_all(temp.join("old")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(temp.join("a.tmp"), vec![0u8; 1000]).unwrap();
        std::fs::write(temp.join("old").join("mail.pst"), vec![0u8; 3_000_000]).unwrap(); // past the size tolerance
        std::fs::write(temp.join("old").join("b.tmp"), vec![0u8; 200]).unwrap();
        std::fs::write(outside.join("precious.txt"), b"keep me").unwrap();
        let linked = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(temp.join("link")).arg(&outside).output().is_ok_and(|o| o.status.success());
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let n = crate::find_path(&tree, &temp).unwrap();
        let mut fd = f(&temp.display().to_string(), String::new());
        fd.bytes = tree.nodes[n].size;
        fd.trusted = true;
        fd.builtin = true;
        fd.set_action(Action::Clear { days: 0 }, false);
        let never = crate::safety::Never::default_list();
        let ok = crate::safety::check(&fd, &tree, &never).expect("kept, with the mail store carved out");
        assert_eq!(ok.carved.len(), 1);
        assert_eq!(ok.bytes, 1200, "the mail store isn't counted");
        assert!(ok.evidence.iter().any(|e| e.contains("mail.pst")));
        if linked {
            assert!(ok.evidence.iter().any(|e| e.contains("removed as links")), "{:?}", ok.evidence);
        }
        // checked again just before the run (as the app does): still kept, nothing repeated
        let again = crate::safety::check(&ok, &tree, &never).expect("a carved finding passes again");
        assert_eq!((again.bytes, again.carved.len(), again.evidence.len()), (ok.bytes, 1, ok.evidence.len()));
        // what the review would count, and what the run frees, agree
        assert_eq!(would_free(&temp, &Action::Match { pattern: "*".into() }, &Spare::default()), Some(1200));
        let o = run(&temp, &Action::Clear { days: 0 }, &Spare::default(), &AtomicBool::new(false), &mut |_| {});
        assert_eq!(o.freed, 1200, "{:?}", o.problems);
        assert_eq!(o.kept_protected, 1);
        assert!(o.problems.is_empty(), "a carved item is not a problem: {:?}", o.problems);
        assert!(temp.join("old").join("mail.pst").exists(), "the mail store stays");
        assert!(!temp.join("a.tmp").exists() && !temp.join("old").join("b.tmp").exists());
        assert!(outside.join("precious.txt").exists(), "the link's target stays");
        if linked {
            assert!(std::fs::symlink_metadata(temp.join("link")).is_err(), "the link itself went");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Preview counts what the run would take, by the run's tests, and deletes nothing.
    #[test]
    fn preview_lists_what_would_go_and_removes_nothing() {
        let base = std::env::temp_dir().join(format!("sprawl-preview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        let old = |p: &std::path::Path, n: usize| {
            std::fs::write(p, vec![0u8; n]).unwrap();
            let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
            f.set_modified(SystemTime::now() - Duration::from_secs(10 * 86_400)).unwrap();
        };
        old(&base.join("a.log"), 300);
        old(&base.join("b.txt"), 100);
        old(&base.join("sub").join("c.log"), 200);
        old(&base.join("mail.pst"), 5000);
        let none = Spare::default();
        let all = preview(&base, &Action::Remove, &none, 2).unwrap();
        assert_eq!((all.bytes, all.files), (600, 3), "the .pst is protected");
        let names: Vec<(String, u64)> = all.biggest.iter().map(|(p, b)| (p.file_name().unwrap().to_string_lossy().into_owned(), *b)).collect();
        assert_eq!(names, [("a.log".to_string(), 300), ("c.log".to_string(), 200)]);
        let m = Action::Match { pattern: "*.log".into() };
        let logs = preview(&base, &m, &none, 10).unwrap();
        assert_eq!((logs.bytes, logs.files), (500, 2));
        assert_eq!(would_free(&base, &m, &none), Some(logs.bytes), "the same count as the measure");
        assert_eq!(preview(&base, &Action::Clear { days: 0 }, &none, 0).unwrap().bytes, 600);
        // made or changed after the scan began: stays
        let spare = Spare { since: Some(SystemTime::now()), keep: Vec::new(), ..Default::default() };
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(base.join("new.log"), vec![0u8; 50]).unwrap();
        assert_eq!(preview(&base, &m, &spare, 0).unwrap().bytes, 500, "the new log stays");
        std::fs::remove_file(base.join("new.log")).unwrap();
        // kept by the user
        let kept = Spare { since: None, keep: vec![base.join("sub")], ..Default::default() };
        assert_eq!(preview(&base, &Action::Remove, &kept, 0).unwrap().bytes, 400);
        // a tool decides itself; a single file only goes with Remove
        assert!(preview(&base, &Action::Tool { command: "x".into() }, &none, 0).is_none());
        assert_eq!(preview(&base.join("a.log"), &Action::Remove, &none, 1).unwrap().files, 1);
        assert_eq!(preview(&base.join("a.log"), &Action::Clear { days: 0 }, &none, 1).unwrap().files, 0);
        // nothing was removed
        assert!(base.join("a.log").exists() && base.join("sub").join("c.log").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_mail_profile_inside_is_found_and_left() {
        let base = std::env::temp_dir().join(format!("sprawl-tbird-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let prof = base.join("old").join("Thunderbird").join("Profiles").join("x.default");
        std::fs::create_dir_all(&prof).unwrap();
        std::fs::write(prof.join("Inbox"), b"mail").unwrap();
        std::fs::write(base.join("old").join("junk.tmp"), b"junk").unwrap();
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let never = crate::safety::Never::default_list();
        let hits = never.hits_inside(&tree, sprawl_core::ROOT);
        assert_eq!(hits.len(), 1, "the Profiles folder, not what is inside it");
        assert!(tree.path_of(hits[0].0).ends_with(r"Thunderbird\Profiles"));
        let o = run(&base, &Action::Clear { days: 0 }, &Spare::default(), &AtomicBool::new(false), &mut |_| {});
        assert!(prof.join("Inbox").exists(), "the mail stays");
        assert!(!base.join("old").join("junk.tmp").exists());
        assert_eq!(o.kept_protected, 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A bin of the fixture's own (on the same volume), never the user's Recycle Bin.
    fn test_bin(base: &Path, room: u64) -> Result<crate::recycle::Bin, String> {
        let dir = base.join("bin");
        std::fs::create_dir_all(&dir).unwrap();
        crate::recycle::Bin::new(dir, "C:".into(), room)
    }

    /// What is in the test bin: the `$R` items, by their record's original path.
    fn in_bin(base: &Path) -> Vec<(PathBuf, u64, PathBuf)> {
        let dir = base.join("bin");
        let mut out = Vec::new();
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some(rest) = n.strip_prefix("$I") {
                let r = std::fs::read(e.path()).unwrap();
                let len = u32::from_le_bytes(r[24..28].try_into().unwrap()) as usize;
                let w: Vec<u16> = r[28..28 + 2 * (len - 1)].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                let item = dir.join(format!("$R{rest}"));
                assert!(item.exists(), "a record without its item: {n}");
                out.push((PathBuf::from(String::from_utf16_lossy(&w)), u64::from_le_bytes(r[8..16].try_into().unwrap()), item));
            }
        }
        out.sort();
        out
    }

    #[test]
    fn a_whole_folder_move_spares_what_the_item_check_spares_and_obeys_stop() {
        let base = std::env::temp_dir().join(format!("sprawl-bin-moved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let cache = base.join("Cache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("old.bin"), vec![0u8; 40]).unwrap();
        std::fs::write(base.join("mine.doc"), vec![0u8; 9]).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let scan = SystemTime::now();
        let seen = seen_now(&base);
        std::thread::sleep(Duration::from_millis(50));
        // an existing file moved in after the scan keeps its birth and write times; only its
        // change time is new (Astra's case)
        std::fs::rename(base.join("mine.doc"), cache.join("mine.doc")).unwrap();
        let spare = Spare { since: Some(scan), keep: Vec::new(), seen: seen.clone() };
        let stop = AtomicBool::new(false);
        let mut bin = test_bin(&base, 1 << 30);
        let o = run_with(&cache, &Action::Remove, &spare, Some(&mut bin), &stop, &mut |_| {});
        assert!(o.problems.is_empty(), "{:?}", o.problems);
        assert!(cache.join("mine.doc").exists(), "the moved-in file stays");
        // the folder is the one the scan saw (by its id), so it is gone through item by item:
        // the file the scan saw there goes, the one it didn't stays (round 3, S1 #3)
        assert!(!cache.join("old.bin").exists());
        assert_eq!((o.recycled, o.kept_new), (40, 1));
        drop(bin);
        let _ = std::fs::remove_dir_all(base.join("bin"));

        // Stop already pressed: nothing moves, and the outcome says stopped
        let other = base.join("Other");
        std::fs::create_dir_all(other.join("sub")).unwrap();
        std::fs::write(other.join("sub").join("x.bin"), vec![0u8; 10]).unwrap();
        let stopped = AtomicBool::new(true);
        let mut bin = test_bin(&base, 1 << 30);
        let o = run_with(&other, &Action::Remove, &Spare::default(), Some(&mut bin), &stopped, &mut |_| {});
        assert!(o.stopped);
        assert_eq!((o.recycled, o.recycled_files), (0, 0));
        assert!(other.join("sub").join("x.bin").exists());
        assert!(in_bin(&base).is_empty());
        drop(bin);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_package_tool_runs_only_on_the_reviewed_cache() {
        let owned = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-tool-scope").unwrap();
        let base = owned.path().to_path_buf();
        let cache = base.join("packages");
        std::fs::create_dir_all(cache.join("v10")).unwrap();
        std::fs::create_dir_all(base.join("other")).unwrap();
        let out = |lines: &[String]| lines.to_vec();
        let s = |p: &Path| p.display().to_string();
        // NuGet: never `all`; only the package folder, and only when it is the one reviewed
        let nuget = package_tool("dotnet nuget locals all --clear").unwrap().unwrap();
        assert_eq!(nuget.run, "dotnet nuget locals global-packages --clear");
        assert_eq!(package_tool("dotnet nuget locals global-packages --clear"), Some(Ok(nuget)));
        // bound to the reviewed folder by the variable NuGet reads first (re-review S1 #4)
        assert!(nuget.env(&cache).contains(&("NUGET_PACKAGES", cache.clone())));
        assert!(scope_matches(&cache, &nuget, &out(&[format!("global-packages: {}\\", s(&cache))])).is_ok());
        assert!(scope_matches(&cache, &nuget, &out(&[format!("global-packages: {}", s(&base.join("other")))])).is_err());
        assert!(scope_matches(&cache, &nuget, &out(&[r"global-packages: \\server\share\packages".into()])).is_err());
        assert!(scope_matches(&cache, &nuget, &out(&["something else".into()])).is_err());
        assert!(scope_matches(&cache, &nuget, &[]).is_err());
        // one-line answers: the folder itself
        let npm = package_tool("npm cache clean --force").unwrap().unwrap();
        // the probe and the run name the same folder, by flag, and keep npm's logs in it
        // (round 3: npm rotates logs wherever logs-dir says, on every start)
        for c in [npm.probe, npm.run] {
            assert!(c.contains(r#"--cache "$env:SPRAWL_TOOL_DIR""#) && c.ends_with(r#"--logs-dir "$env:SPRAWL_TOOL_DIR\_logs""#), "{c}");
        }
        assert!(npm.env(&cache).contains(&("SPRAWL_TOOL_DIR", cache.clone())));
        assert!(scope_matches(&cache, &npm, &out(&[s(&cache)])).is_ok());
        assert!(scope_matches(&cache, &npm, &out(&[s(&cache.join("v10"))])).is_err());
        assert!(scope_matches(&cache, &npm, &out(&[s(&cache), s(&cache)])).is_err(), "two answers are no answer");
        assert!(scope_matches(&cache, &npm, &out(&["relative\\store".into()])).is_err());
        // conda, pnpm and Yarn clear more than the one folder they can be told: held (round 3)
        for held in ["conda clean --all -y", "pnpm store prune", "yarn cache clean"] {
            assert!(matches!(package_tool(held), Some(Err(_))), "{held}");
        }
        // Go: its saved settings and ambient flags off for both commands, no toolchain switch
        let go = package_tool("go clean -cache").unwrap().unwrap();
        let env = go.env(&cache);
        for want in [("GOCACHE", cache.clone()), ("GOENV", "off".into()), ("GOFLAGS", "".into()), ("GOTOOLCHAIN", "local".into())] {
            assert!(env.contains(&want), "{want:?}");
        }
        // the place is held while the tool runs: it can't be renamed (or swapped for a link)
        let held = scope_matches(&cache, &npm, &out(&[s(&cache)])).unwrap();
        assert!(std::fs::rename(&cache, base.join("moved")).is_err());
        drop(held);
        std::fs::rename(&cache, base.join("moved")).unwrap();
        std::fs::rename(base.join("moved"), &cache).unwrap();
        // a folder skipped in Settings, or not there at all
        assert!(scope_matches(&base.join("gone"), &npm, &out(&[s(&base.join("gone"))])).is_err());
        assert_eq!(package_tool("Clear-RecycleBin -Force"), None, "not a package cache");
        // the answer is read whole, every line as written (not the last 12 lines, clipped to
        // 200 characters), and an answer past the limit is no answer
        let stop = AtomicBool::new(false);
        let mut o = Outcome::default();
        let long = "x".repeat(300);
        let sys = system_dir().unwrap();
        let text = run_powershell(&format!("1..40 | ForEach-Object {{ Write-Output ('{long}' + $_) }}"), &sys, false, &stop, &mut o, &[]).expect("read whole");
        assert_eq!(text.lines().count(), 40, "{:?}", o.problems);
        assert!(text.lines().all(|l| l.len() > 300));
        let mut o = Outcome::default();
        assert!(run_powershell("1..3000 | ForEach-Object { Write-Output ('y' * 400) }", &sys, false, &stop, &mut o, &[]).is_none(), "1.2 MB is cut");
        // the binding reaches the tool
        let mut o = Outcome::default();
        let dir = std::ffi::OsString::from(r"C:\Reviewed\cache");
        let text = run_powershell(r#"Write-Output "$env:SPRAWL_TOOL_DIR""#, &sys, false, &stop, &mut o, &[("SPRAWL_TOOL_DIR", dir.as_os_str())]).unwrap();
        assert_eq!(text.trim(), r"C:\Reviewed\cache");
        drop(owned);
    }

    #[test]
    fn a_tool_command_naming_a_share_is_refused() {
        assert_eq!(quoted_paths(r"compact /c /s /a /i /exe:xpress8k 'C:\Games\It''s\*'"), vec![PathBuf::from(r"C:\Games\It's")]);
        assert_eq!(quoted_paths("throw 'Docker Desktop is running: quit it first'"), Vec::<PathBuf>::new());
        let stop = AtomicBool::new(false);
        let local = std::env::temp_dir();
        let cmd = format!("compact /c /s /a /i /exe:xpress8k {}", crate::rules::ps_quote(r"\\server\share\Games\*"));
        let o = run(&local, &Action::Tool { command: cmd }, &Spare::default(), &stop, &mut |_| {});
        assert!(o.problems.iter().any(|p| p.contains("left alone")), "{:?}", o.problems);
    }

    #[test]
    fn nothing_runs_on_a_network_path() {
        let stop = AtomicBool::new(false);
        let share = Path::new(r"\\server\share\Games\Big");
        for action in [Action::Tool { command: "compact /c /s /a /i /exe:xpress16k 'x'".into() }, Action::Remove, Action::Clear { days: 0 }] {
            let o = run(share, &action, &Spare::default(), &stop, &mut |_| {});
            assert!(o.problems.iter().any(|p| p.contains("left alone")), "{:?}", o.problems);
            assert_eq!((o.freed, o.files), (0, 0));
        }
        let local = std::env::temp_dir().join("sprawl-link-keep.bin");
        let o = run(&local, &Action::Link { keep: local.clone(), copies: vec![local.clone(), share.join("copy.bin")] }, &Spare::default(), &stop, &mut |_| {});
        assert!(o.problems.iter().any(|p| p.contains("left alone")), "{:?}", o.problems);
    }

    #[test]
    fn own_files_go_to_the_bin_whole_or_one_by_one() {
        let base = std::env::temp_dir().join(format!("sprawl-bin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let proj = base.join("Old project");
        std::fs::create_dir_all(proj.join("src")).unwrap();
        std::fs::write(proj.join("src").join("main.rs"), vec![1u8; 300]).unwrap();
        std::fs::write(proj.join("notes.txt"), vec![1u8; 200]).unwrap();
        std::fs::set_permissions(proj.join("notes.txt"), {
            let mut p = std::fs::metadata(proj.join("notes.txt")).unwrap().permissions();
            p.set_readonly(true);
            p
        })
        .unwrap();
        let stop = AtomicBool::new(false);

        // a folder with nothing in it to spare goes whole, with one record Explorer reads
        let mut bin = test_bin(&base, 1 << 30);
        let o = run_with(&proj, &Action::Remove, &Spare::default(), Some(&mut bin), &stop, &mut |_| {});
        assert!(o.problems.is_empty(), "{:?}", o.problems);
        assert_eq!((o.freed, o.recycled, o.recycled_files), (0, 500, 2), "nothing is freed until the bin is emptied");
        assert!(!proj.exists());
        let got = in_bin(&base);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].0.clone(), got[0].1), (proj.clone(), 500));
        assert!(got[0].2.join("src").join("main.rs").exists() && got[0].2.join("notes.txt").exists(), "all of it, read-only files too");
        assert_eq!(bin.as_ref().unwrap().room, (1 << 30) - 500);
        drop(bin);
        let _ = std::fs::remove_dir_all(base.join("bin"));

        // something to spare inside: the rest goes one by one, what must stay stays where it is
        let mine = base.join("Mine");
        std::fs::create_dir_all(mine.join("keep")).unwrap();
        std::fs::create_dir_all(mine.join("old")).unwrap();
        std::fs::write(mine.join("keep").join("k.txt"), vec![0u8; 10]).unwrap();
        std::fs::write(mine.join("old").join("o.txt"), vec![0u8; 20]).unwrap();
        std::fs::write(mine.join("vault.kdbx"), vec![0u8; 30]).unwrap();
        std::fs::write(mine.join("a.bin"), vec![0u8; 40]).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let scan = SystemTime::now();
        let seen = seen_now(&base);
        std::thread::sleep(Duration::from_millis(50));
        std::fs::write(mine.join("old").join("new.txt"), vec![0u8; 5]).unwrap();
        let spare = Spare { since: Some(scan), keep: vec![mine.join("keep")], seen: seen.clone() };
        let mut bin = test_bin(&base, 1 << 30);
        let o = run_with(&mine, &Action::Remove, &spare, Some(&mut bin), &stop, &mut |_| {});
        assert!(o.problems.is_empty(), "{:?}", o.problems);
        assert_eq!((o.recycled, o.freed), (60, 0), "a.bin and old\\o.txt");
        assert_eq!((o.kept_chosen, o.kept_protected, o.kept_new), (1, 1, 1));
        assert!(mine.join("keep").join("k.txt").exists() && mine.join("vault.kdbx").exists() && mine.join("old").join("new.txt").exists());
        assert!(!mine.join("a.bin").exists() && !mine.join("old").join("o.txt").exists());
        let got: Vec<PathBuf> = in_bin(&base).into_iter().map(|g| g.0).collect();
        assert_eq!(got, vec![mine.join("a.bin"), mine.join("old").join("o.txt")]);
        drop(bin);
        let _ = std::fs::remove_dir_all(base.join("bin"));

        // too big for the bin, or no bin: held back with the reason, never deleted instead
        let big = base.join("Big");
        std::fs::create_dir_all(&big).unwrap();
        std::fs::write(big.join("x.iso2"), vec![0u8; 1000]).unwrap();
        let mut bin = test_bin(&base, 999);
        let o = run_with(&big, &Action::Remove, &Spare::default(), Some(&mut bin), &stop, &mut |_| {});
        assert_eq!((o.freed, o.recycled), (0, 0));
        assert!(o.problems.iter().any(|p| p.contains("too big for the Recycle Bin")), "{:?}", o.problems);
        assert!(big.join("x.iso2").exists());
        drop(bin);
        let mut none: Result<crate::recycle::Bin, String> = Err("a network folder has no Recycle Bin".into());
        let o = run_with(&big.join("x.iso2"), &Action::Remove, &Spare::default(), Some(&mut none), &stop, &mut |_| {});
        assert_eq!((o.freed, o.recycled), (0, 0));
        assert!(o.problems.iter().any(|p| p.contains("not sent to the Recycle Bin")), "{:?}", o.problems);
        assert!(big.join("x.iso2").exists());

        // with the bin, a link inside stays where it is (Astra 1.0.2 S4 #1): nothing is deleted
        // for good under "To the Recycle Bin", and what it points to is untouched
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("precious.txt"), b"keep").unwrap();
        let withlink = base.join("WithLink");
        std::fs::create_dir_all(&withlink).unwrap();
        std::fs::write(withlink.join("f.txt"), vec![0u8; 7]).unwrap();
        if std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(withlink.join("link")).arg(&outside).output().is_ok_and(|o| o.status.success()) {
            let mut bin = test_bin(&base, 1 << 30);
            let o = run_with(&withlink, &Action::Remove, &Spare::default(), Some(&mut bin), &stop, &mut |_| {});
            assert!(o.problems.iter().all(|p| p.contains("a link can't go to the Recycle Bin")) && o.problems.len() == 1, "{:?}", o.problems);
            assert_eq!(o.recycled, 7, "the link counts nothing");
            assert!(outside.join("precious.txt").exists());
            // the folder went one item at a time: the link left as it is, never into the bin
            assert!(std::fs::symlink_metadata(withlink.join("link")).is_ok(), "the link stays");
            let got: Vec<PathBuf> = in_bin(&base).into_iter().map(|g| g.0).collect();
            assert_eq!(got, vec![withlink.join("f.txt")], "only the file is in the bin");
            drop(bin);
            let _ = std::fs::remove_dir_all(base.join("bin"));
        }

        // a pattern sends only the files it names
        let cache = base.join("Downloads");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("a.msi"), vec![0u8; 11]).unwrap();
        std::fs::write(cache.join("b.txt"), vec![0u8; 12]).unwrap();
        let mut bin = test_bin(&base, 1 << 30);
        let o = run_with(&cache, &Action::Match { pattern: "*.msi".into() }, &Spare::default(), Some(&mut bin), &stop, &mut |_| {});
        assert_eq!((o.recycled, o.freed), (11, 0), "{:?}", o.problems);
        assert!(!cache.join("a.msi").exists() && cache.join("b.txt").exists());
        drop(bin);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_unreadable_entry_is_said_on_its_folder() {
        // read_dir can't be made to fail on one entry in a test; the count and the words can
        let dir = std::env::temp_dir().join(format!("sprawl-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a"), b"x").unwrap();
        assert_eq!(list(&dir).unwrap(), (vec![dir.join("a")], 0));
        assert_eq!(unreadable(1), "1 item in it couldn't be read; left as it is");
        assert_eq!(unreadable(3), "3 items in it couldn't be read; left as they are");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stop_ends_what_a_tool_started() {
        // Stop is pressed once the tool is seen running (it writes a marker first), not after a
        // fixed time: a slow start on a busy machine can't make this pass or fail by chance.
        // Elevated, the compound command below is refused by design (elevated_tool_ok).
        if !matches!(sprawl_core::elevation(), Ok(false)) { return eprintln!("elevated: the unelevated tool run is not tested here"); }
        let marker = std::env::temp_dir().join(format!("sprawl-stop-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let (s, m) = (stop.clone(), marker.clone());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let at = seen.clone();
        std::thread::spawn(move || {
            let begin = std::time::Instant::now();
            while !m.exists() && begin.elapsed() < Duration::from_secs(120) {
                std::thread::sleep(Duration::from_millis(50));
            }
            *at.lock().unwrap() = Some(std::time::Instant::now());
            s.store(true, Ordering::Relaxed);
        });
        let mut o = Outcome::default();
        // ping is a program of its own, below PowerShell; it runs for about 100 s if not stopped
        tool(&format!("New-Item -ItemType File -Path {} | Out-Null; ping -n 100 127.0.0.1", ps_quote(&marker.display().to_string())), &stop, &mut o);
        let ended = std::time::Instant::now();
        let _ = std::fs::remove_file(&marker);
        assert!(o.stopped, "not stopped: {:?} {:?}", o.problems, o.output);
        let pressed = seen.lock().unwrap().expect("the tool started");
        assert!(ended.duration_since(pressed) < Duration::from_secs(10), "took {:?} after Stop", ended.duration_since(pressed));
    }

    #[test]
    fn a_tool_runs_hidden_with_output() {
        // Elevated, several of these commands are refused by design (elevated_tool_ok).
        if !matches!(sprawl_core::elevation(), Ok(false)) { return eprintln!("elevated: the unelevated tool run is not tested here"); }
        let mut o = Outcome::default();
        tool("Write-Output 'hello from a tool'; (Get-Location).Path", &AtomicBool::new(false), &mut o);
        assert!(o.output.iter().any(|l| l == "hello from a tool"), "{:?}", o.output);
        assert!(o.output.iter().any(|l| l.to_lowercase().ends_with(r"\system32")), "{:?}", o.output);
        let mut bad = Outcome::default();
        tool("exit 3", &AtomicBool::new(false), &mut bad);
        assert!(bad.problems.iter().any(|p| p.contains("code 3")), "{:?}", bad.problems);
        // a failing program's code is kept (Out-String used to hide it), and a trailing comment
        // doesn't swallow the rest
        let mut failed = Outcome::default();
        tool("cmd /c exit 5   # a comment", &AtomicBool::new(false), &mut failed);
        assert!(failed.problems.iter().any(|p| p.contains("code 5")), "{:?}", failed.problems);
        let mut missing = Outcome::default();
        tool("no-such-tool-sprawl --clean", &AtomicBool::new(false), &mut missing);
        assert!(!missing.problems.is_empty(), "a tool that isn't there is not a success");
        // non-ASCII output survives
        let mut text = Outcome::default();
        tool("Write-Output 'çğış ok'", &AtomicBool::new(false), &mut text);
        assert!(text.output.iter().any(|l| l == "çğış ok"), "{:?}", text.output);
    }
}
