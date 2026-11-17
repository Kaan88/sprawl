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
    if let Action::Match { pattern } = &a {
        if pattern.is_empty() || pattern.contains(['\\', '/']) {
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
#[cfg(test)]
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
}

/// What a clean-up leaves alone besides the never-list. Tool commands can't honour it: the
/// program decides what it removes.
#[derive(Clone, Debug, Default)]
pub struct Spare {
    /// When the scan began: anything created or changed after it stays.
    pub since: Option<SystemTime>,
    /// Folders and files the user chose to keep: they, and everything inside them, stay.
    pub keep: Vec<PathBuf>,
}

impl Spare {
    /// `p` is one of the kept paths, or inside one.
    pub fn keeps(&self, p: &Path) -> bool {
        let p = lower(p);
        self.keep.iter().any(|k| p.starts_with(lower(k)))
    }

    /// The item was created or changed after the scan (by its open handle's times).
    fn is_new(&self, info: &win::Info) -> bool {
        let Some(since) = self.since else { return false };
        let ft = filetime(since);
        if info.attrs & DIRECTORY != 0 && info.attrs & REPARSE == 0 {
            // a folder's own write and change times move whenever something inside it does,
            // so only its birth counts; what is inside is looked at one by one
            info.created > ft
        } else {
            info.created > ft || info.newest > ft
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
    let mut o = Outcome::default();
    match action {
        Action::Tool { command } => tool(command, stop, &mut o),
        _ => tree(path, action, spare, stop, &mut o, progress),
    }
    if stop.load(Ordering::Relaxed) {
        o.stopped = true;
    }
    o
}

/// What [`run`] would free on `path` now, by the same tests and without deleting anything: the
/// files a pattern names, or the children a keep-days clear would take. Links are not followed
/// and count nothing; never-list names, kept paths and anything new since the scan count
/// nothing either. For Remove and a plain Clear the scan's size is the answer (None).
pub fn would_free(path: &Path, action: &Action, spare: &Spare) -> Option<u64> {
    let cut = match action {
        Action::Clear { days } if *days > 0 => Some(SystemTime::now().checked_sub(Duration::from_secs(*days as u64 * 86_400)).unwrap_or(SystemTime::UNIX_EPOCH)),
        Action::Match { .. } => None,
        _ => return None,
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
    let top = std::fs::symlink_metadata(path).ok()?;
    if !top.is_dir() || attrs(&top) & REPARSE != 0 {
        return Some(0);
    }
    let mut total = 0u64;
    let mut stack: Vec<PathBuf> = list(path).ok()?;
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
                stack.extend(list(&k).unwrap_or_default());
            }
        } else if pattern.is_none_or(|p| like(&name, p)) && !new(&m) {
            total += m.len();
        }
    }
    Some(total)
}

/// One open folder and what is left to do in it.
struct Frame {
    h: win::Handle,
    path: PathBuf,
    kids: std::vec::IntoIter<PathBuf>,
    /// Delete the folder once it is empty (not the root of Clear and Match, nor any folder of Match).
    remove: bool,
}

fn tree(path: &Path, action: &Action, spare: &Spare, stop: &AtomicBool, o: &mut Outcome, progress: &mut dyn FnMut(u64)) {
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
    if spare.is_new(&info) {
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
            delete_one(path, &h, &info, o, progress);
        } else {
            o.problem(path, "not a plain folder any more");
        }
        return;
    }
    let kids = match list(path) {
        Ok(k) => k,
        Err(e) => return o.problem(path, e),
    };
    // Clear leaves what is new; that is decided for the root's own children only
    let kids: Vec<PathBuf> = match cut {
        Some(cut) => kids.into_iter().filter(|k| !is_new(k, cut)).collect(),
        None => kids,
    };
    let mut stack = vec![Frame { h, path: path.to_path_buf(), kids: kids.into_iter(), remove: matches!(action, Action::Remove) }];
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
        if spare.is_new(&info) {
            // Match only removes the files it names: what it wouldn't touch isn't counted
            if pattern.is_none_or(|p| info.attrs & DIRECTORY == 0 && like(&name, p)) {
                o.kept_new += 1;
            }
            continue;
        }
        if info.attrs & DIRECTORY != 0 && info.attrs & REPARSE == 0 {
            match list(&k) {
                Ok(kids) => stack.push(Frame { h, path: k, kids: kids.into_iter(), remove: pattern.is_none() }),
                Err(e) => o.problem(&k, e),
            }
        } else if info.attrs & REPARSE != 0 && info.attrs & DIRECTORY != 0 {
            // a junction or folder link: Match leaves it, the rest remove the link itself
            if pattern.is_none() {
                delete_one(&k, &h, &info, o, progress);
            }
        } else if pattern.is_none_or(|p| like(&name, p)) {
            delete_one(&k, &h, &info, o, progress);
        }
    }
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

fn list(p: &Path) -> std::io::Result<Vec<PathBuf>> {
    Ok(std::fs::read_dir(p)?.flatten().map(|e| e.path()).collect())
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

/// PowerShell's `-like`: `*`, `?` and `[...]`, ignoring case.
pub fn like(name: &str, pattern: &str) -> bool {
    fn go(n: &[char], p: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|i| go(&n[i..], &p[1..])),
            Some('?') => !n.is_empty() && go(&n[1..], &p[1..]),
            Some('[') => {
                let Some(end) = p.iter().position(|&c| c == ']') else { return !n.is_empty() && n[0] == '[' && go(&n[1..], &p[1..]) };
                let Some(&c) = n.first() else { return false };
                let set = &p[1..end];
                let mut hit = false;
                let mut i = 0;
                while i < set.len() {
                    if i + 2 < set.len() && set[i + 1] == '-' {
                        hit |= set[i] <= c && c <= set[i + 2];
                        i += 3;
                    } else {
                        hit |= set[i] == c;
                        i += 1;
                    }
                }
                hit && go(&n[1..], &p[end + 1..])
            }
            Some(&c) => n.first() == Some(&c) && go(&n[1..], &p[1..]),
        }
    }
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    go(&n, &p)
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
        const DELETE: u32 = 0x0001_0000;
        const READ_ATTRIBUTES: u32 = 0x80;
        const WRITE_ATTRIBUTES: u32 = 0x100;
        const SYNCHRONIZE: u32 = 0x0010_0000;
        const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
        const OPEN_EXISTING: u32 = 3;
        const BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        let w = wide(p);
        let h = unsafe { CreateFileW(w.as_ptr(), DELETE | READ_ATTRIBUTES | WRITE_ATTRIBUTES | SYNCHRONIZE, SHARE_READ_WRITE, std::ptr::null_mut(), OPEN_EXISTING, BACKUP_SEMANTICS | OPEN_REPARSE_POINT, std::ptr::null_mut()) };
        if h as isize == -1 {
            return Err(std::io::Error::last_os_error());
        }
        let h = Handle(h);
        let mut b = ByHandle::default();
        if unsafe { GetFileInformationByHandle(h.0, &mut b) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut tag = [0u32; 2];
        if b.attrs & super::REPARSE != 0 && unsafe { GetFileInformationByHandleEx(h.0, 9, tag.as_mut_ptr().cast(), 8) } == 0 {
            // a reparse point whose kind can't be told: treat it as a link
            tag[1] = super::NAME_SURROGATE;
        }
        let ft = |t: [u32; 2]| ((t[1] as u64) << 32 | t[0] as u64) as i64;
        let (mut created, mut newest) = (ft(b.created), ft(b.written));
        let mut basic = Basic { created: 0, accessed: 0, written: 0, changed: 0, attrs: 0 };
        if unsafe { GetFileInformationByHandleEx(h.0, 0, (&mut basic as *mut Basic).cast(), std::mem::size_of::<Basic>() as u32) } != 0 {
            created = created.max(basic.created);
            newest = newest.max(basic.written).max(basic.changed);
        }
        Ok((h, Info { attrs: b.attrs, tag: tag[1], size: (b.size_high as u64) << 32 | b.size_low as u64, links: b.links, created, newest }))
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

fn system_dir() -> Option<PathBuf> {
    let mut buf = [0u16; 520];
    let n = unsafe { win::GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// Folders only administrators can write to: Windows, Program Files (both). A program an
/// elevated Sprawl starts must come from one of them.
fn system_roots() -> Vec<PathBuf> {
    ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)"].iter().filter_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute()).collect()
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
    let body = command.split('#').next().unwrap_or("");
    let words: Vec<&str> = body.split_whitespace().collect();
    let mut programs = Vec::new();
    if let Some(&first) = words.first() {
        if first.eq_ignore_ascii_case("Start-Process") {
            if let Some(&p) = words.get(1) {
                programs.push(p);
            }
        } else {
            programs.push(first);
        }
    }
    // PowerShell's own commands, and its words that are not commands at all
    const KEYWORDS: [&str; 6] = ["exit", "return", "if", "foreach", "throw", "break"];
    let cmdlet = |w: &str| {
        // Verb-Noun, letters only, each part capitalised: Clear-RecycleBin, not npm-x or foo-bar
        let verb_noun = w.split_once('-').is_some_and(|(v, n)| {
            let part = |p: &str| p.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && p.chars().all(|c| c.is_ascii_alphabetic());
            part(v) && part(n)
        });
        verb_noun || KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(w))
    };
    for p in programs.into_iter().filter(|w| !cmdlet(w)) {
        let p = p.trim_matches(['"', '\'']);
        let Some(found) = resolve(p) else { return Err(format!("not run as administrator: {p} wasn't found")) };
        if !roots.iter().any(|r| crate::finding::within(&found, r)) {
            return Err(format!("not run as administrator: {p} is {}, in a folder your account can change. Run it from a normal (not elevated) Sprawl.", found.display()));
        }
    }
    Ok(())
}

/// Why a built-in tool command won't run from this Sprawl: it is elevated and the command starts
/// a program from a folder the user can change (see [`elevated_tool_ok`]). None when it runs.
pub fn elevated_refusal(command: &str) -> Option<String> {
    if !sprawl_core::is_elevated() {
        return None;
    }
    elevated_tool_ok(command, &|name| on_path(name), &system_roots()).err()
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
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    let Some(sys) = system_dir() else { return o.problem(Path::new(command), "Windows' System32 folder wasn't found") };
    let elevated = sprawl_core::is_elevated();
    if let Some(why) = elevated_refusal(command) {
        return o.problem(Path::new(command), why);
    }
    let ps = sys.join(r"WindowsPowerShell\v1.0\powershell.exe");
    // the command on lines of its own (a trailing comment can't swallow the rest), its exit code
    // kept, and the output in UTF-8
    // A program's text on stderr is not a failure; any other error (not found, a cmdlet's) is.
    let script = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8\n$Error.Clear()\n$o = & {{\n{command}\n}} 2>&1; $c = $LASTEXITCODE\n$o | Out-String -Width 200\n\
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
    if elevated {
        // no PowerShell module from the user's own folders loads into an elevated run
        let pf = std::env::var_os("ProgramFiles").map(PathBuf::from);
        let modules: Vec<String> = [pf.map(|p| p.join(r"WindowsPowerShell\Modules")), Some(sys.join(r"WindowsPowerShell\v1.0\Modules"))].into_iter().flatten().map(|p| p.display().to_string()).collect();
        cmd.env("PSModulePath", modules.join(";"));
    }
    let job = win::Job::new();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return o.problem(Path::new(command), e),
    };
    let in_job = job.as_ref().is_some_and(|j| j.add(&child));
    let mut out = child.stdout.take();
    let reader = std::thread::spawn(move || {
        // read to the end (a tool must never block on a full pipe), keep the last megabyte
        let mut keep: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        while let Some(Ok(n)) = out.as_mut().map(|o| o.read(&mut buf)) {
            if n == 0 {
                break;
            }
            keep.extend_from_slice(&buf[..n]);
            if keep.len() > 2 << 20 {
                keep.drain(..keep.len() - (1 << 20));
            }
        }
        String::from_utf8_lossy(&keep).into_owned()
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
    let text = if o.stopped && !in_job { String::new() } else { reader.join().unwrap_or_default() };
    let lines: Vec<String> = text.lines().map(str::trim_end).filter(|l| !l.is_empty()).map(|l| crate::llm::clip(l.to_string(), 200)).collect();
    o.output = lines[lines.len().saturating_sub(12)..].to_vec();
    if let Some(c) = code.filter(|&c| c != 0) {
        o.problem(Path::new(command.split_whitespace().next().unwrap_or(command)), format!("ended with code {c}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::ps_quote;
    use crate::{Remedy, Risk, Source};

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

    #[test]
    fn like_patterns() {
        assert!(like("crash.DMP", "*.dmp"));
        assert!(!like("crash.dmpx", "*.dmp"));
        assert!(like("a1.log", "a?.log"));
        assert!(like("b.log", "[a-c].log"));
        assert!(!like("d.log", "[a-c].log"));
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
            assert!(o.problems.iter().any(|p| p.contains("leads to")), "{:?}", o.problems);
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
        std::thread::sleep(Duration::from_millis(50));
        std::fs::write(cache.join("new.bin"), vec![0u8; 7]).unwrap();
        std::fs::write(cache.join("old_dir").join("new2.bin"), vec![0u8; 7]).unwrap();
        std::fs::create_dir_all(cache.join("new_dir")).unwrap();
        std::fs::write(cache.join("new_dir").join("x.bin"), vec![0u8; 7]).unwrap();
        std::fs::write(cache.join("new.dmp"), vec![0u8; 7]).unwrap();
        let spare = Spare { since: Some(scan), keep: vec![] };
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
        let spare = Spare { since: None, keep: vec![PathBuf::from(temp.join("mytool").to_string_lossy().to_uppercase() + "\\"), temp.join("mine.txt")] };
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

    #[test]
    fn elevated_tools_come_from_system_folders_only() {
        let roots = vec![PathBuf::from(r"C:\Windows"), PathBuf::from(r"C:\Program Files")];
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
        assert!(elevated_tool_ok("dotnet nuget locals all --clear", &resolve, &roots).is_ok());
        assert!(elevated_tool_ok("Clear-RecycleBin -Force", &resolve, &roots).is_ok());
        assert!(elevated_tool_ok("exit 3", &resolve, &roots).is_ok(), "a PowerShell word, not a program");
        assert!(elevated_tool_ok("npm-run cache", &resolve, &roots).is_err(), "a hyphen alone doesn't make a cmdlet");
        assert!(elevated_tool_ok("Start-Process cleanmgr -ArgumentList \"/d $env:SystemDrive\" -Verb RunAs", &resolve, &roots).is_ok());
        let e = elevated_tool_ok("npm cache clean --force", &resolve, &roots).unwrap_err();
        assert!(e.contains("your account can change"), "{e}");
        assert!(elevated_tool_ok("pip cache purge", &resolve, &roots).is_err(), "not found: not run");
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
        let kept = Spare { since: None, keep: vec![logs.join("sub")] };
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

    #[test]
    fn stop_ends_what_a_tool_started() {
        // Stop is pressed once the tool is seen running (it writes a marker first), not after a
        // fixed time: a slow start on a busy machine can't make this pass or fail by chance.
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
