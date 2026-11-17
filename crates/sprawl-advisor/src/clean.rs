//! Clean-up done by Sprawl itself, in place of the PowerShell functions in [`RM_FN`]
//! (which stay for "Save as script"). Same guarantees as those:
//! - a junction or symbolic link is removed itself, never followed (also one made after the scan);
//! - a name ending in a dot or space is skipped (Windows would act on its namesake);
//! - read-only files go too; a folder's read-only bit is cleared only once it is empty;
//! - `clear` and `match` do nothing when their folder is no longer a plain folder;
//! - an error on one item is noted and the rest carry on.
//!
//! Tool commands (npm, pip, DISM...) run as a hidden process from System32, their output kept.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use crate::rules::RM_FN;
use crate::Finding;

/// What a finding's command does, read back from the command Sprawl itself wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The path itself (a folder with everything in it, or a file).
    Remove,
    /// What is inside the folder, leaving anything created or changed in the last `days`.
    Clear { days: u32 },
    /// Files matching a PowerShell-style pattern anywhere below the folder.
    Match { pattern: String },
    /// A built-in rule's own tool command, run by PowerShell without a window.
    Tool { command: String },
}

/// A finding's action, and whether it needs administrator rights. None when its command isn't
/// one Sprawl wrote, or names a different path than the finding.
pub fn action(f: &Finding) -> Option<(Action, bool)> {
    let cmd = f.command.as_deref()?;
    let admin = crate::export::needs_admin(cmd);
    let body = cmd.trim_end().strip_suffix("# as administrator").unwrap_or(cmd).trim_end();
    let Some(tail) = body.strip_prefix(RM_FN) else {
        // only the built-in pack's own commands; never one from a model or a local pack
        return (f.trusted && f.builtin).then(|| (Action::Tool { command: body.to_string() }, admin));
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
    // the command must act on the finding's own path
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
const READONLY: u32 = 0x1;
const DIRECTORY: u32 = 0x10;

fn attrs(m: &std::fs::Metadata) -> u32 {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes()
    }
    #[cfg(not(windows))]
    {
        let _ = m;
        0
    }
}

/// Run `action` on `path`. `progress` gets the bytes freed so far.
pub fn run(path: &Path, action: &Action, stop: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Outcome {
    let mut o = Outcome::default();
    match action {
        Action::Remove => match std::fs::symlink_metadata(path) {
            Ok(_) => rm(path, stop, &mut o, progress),
            Err(e) => o.problem(path, e),
        },
        Action::Clear { days } => {
            let Some(kids) = plain_dir(path, &mut o) else { return o };
            let cut = SystemTime::now().checked_sub(Duration::from_secs(*days as u64 * 86_400)).unwrap_or(SystemTime::UNIX_EPOCH);
            for k in kids {
                if stop.load(Ordering::Relaxed) {
                    o.stopped = true;
                    break;
                }
                if *days == 0 || !is_new(&k, cut) {
                    rm(&k, stop, &mut o, progress);
                }
            }
        }
        Action::Match { pattern } => {
            if plain_dir(path, &mut o).is_some() {
                matching(path, pattern, stop, &mut o, progress);
            }
        }
        Action::Tool { command } => tool(command, stop, &mut o),
    }
    if stop.load(Ordering::Relaxed) {
        o.stopped = true;
    }
    o
}

/// What is in `p`, when it is still a plain folder (not a link).
fn plain_dir(p: &Path, o: &mut Outcome) -> Option<Vec<PathBuf>> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() && attrs(&m) & REPARSE == 0 => match std::fs::read_dir(p) {
            Ok(rd) => Some(rd.flatten().map(|e| e.path()).collect()),
            Err(e) => {
                o.problem(p, e);
                None
            }
        },
        Ok(_) => {
            o.problem(p, "not a plain folder any more");
            None
        }
        Err(e) => {
            o.problem(p, e);
            None
        }
    }
}

/// Created or changed after `cut`: itself, or anything below it (links not followed). A folder
/// that can't be read counts as new.
fn is_new(p: &Path, cut: SystemTime) -> bool {
    let Ok(m) = std::fs::symlink_metadata(p) else { return true };
    if m.modified().is_ok_and(|t| t > cut) || m.created().is_ok_and(|t| t > cut) {
        return true;
    }
    if !m.is_dir() || attrs(&m) & REPARSE != 0 {
        return false;
    }
    let Ok(rd) = std::fs::read_dir(p) else { return true };
    rd.flatten().any(|e| is_new(&e.path(), cut))
}

fn odd_name(p: &Path) -> bool {
    p.file_name().map(|n| n.to_string_lossy()).is_some_and(|n| n.ends_with('.') || n.ends_with(' '))
}

/// Remove `p`: a link itself, a folder after what is in it, a file.
fn rm(p: &Path, stop: &AtomicBool, o: &mut Outcome, progress: &mut dyn FnMut(u64)) {
    if stop.load(Ordering::Relaxed) {
        o.stopped = true;
        return;
    }
    if odd_name(p) {
        o.problem(p, "skipped, its name ends in a dot or space");
        return;
    }
    let m = match std::fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) => return o.problem(p, e),
    };
    let a = attrs(&m);
    if a & REPARSE != 0 {
        // the link itself, never what it points to
        // (a junction's metadata says "link", not "folder": the attribute tells which it is)
        let r = if a & DIRECTORY != 0 { remove_link_dir(p) } else { std::fs::remove_file(p) };
        if let Err(e) = r {
            o.problem(p, e);
        }
        return;
    }
    if m.is_dir() {
        match std::fs::read_dir(p) {
            Ok(rd) => {
                for e in rd.flatten() {
                    rm(&e.path(), stop, o, progress);
                    if o.stopped {
                        return;
                    }
                }
            }
            Err(e) => return o.problem(p, e),
        }
        // read-only cleared only once empty: a folder that stays keeps its attributes
        let empty = std::fs::read_dir(p).map(|mut r| r.next().is_none()).unwrap_or(false);
        if !empty {
            return; // what stayed inside is already noted
        }
        if a & READONLY != 0 {
            let mut perm = m.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = std::fs::set_permissions(p, perm);
        }
        if let Err(e) = std::fs::remove_dir(p) {
            o.problem(p, e);
        }
    } else {
        if a & READONLY != 0 {
            let mut perm = m.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = std::fs::set_permissions(p, perm);
        }
        match std::fs::remove_file(p) {
            Ok(()) => {
                o.freed += m.len();
                o.files += 1;
                progress(o.freed);
            }
            Err(e) => o.problem(p, e),
        }
    }
}

/// Files below `dir` matching `pattern`, links not followed.
fn matching(dir: &Path, pattern: &str, stop: &AtomicBool, o: &mut Outcome, progress: &mut dyn FnMut(u64)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return o.problem(dir, "can't be read") };
    for e in rd.flatten() {
        if stop.load(Ordering::Relaxed) {
            o.stopped = true;
            return;
        }
        let p = e.path();
        let Ok(m) = std::fs::symlink_metadata(&p) else { continue };
        if attrs(&m) & REPARSE != 0 {
            continue;
        }
        if m.is_dir() {
            matching(&p, pattern, stop, o, progress);
        } else if like(&e.file_name().to_string_lossy(), pattern) {
            rm(&p, stop, o, progress);
        }
    }
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

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemDirectoryW(buf: *mut u16, size: u32) -> u32;
    fn RemoveDirectoryW(path: *const u16) -> i32;
}

/// A junction or directory link, removed as the link (RemoveDirectoryW never follows one).
fn remove_link_dir(p: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let w: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { RemoveDirectoryW(w.as_ptr()) } != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn system_dir() -> Option<PathBuf> {
    let mut buf = [0u16; 520];
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// A built-in tool command, by Windows PowerShell without a window, from System32 (so no tool
/// reads settings from the folder Sprawl started in); its last lines of output are kept.
fn tool(command: &str, stop: &AtomicBool, o: &mut Outcome) {
    use std::io::Read;
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    let Some(sys) = system_dir() else { return o.problem(Path::new(command), "Windows' System32 folder wasn't found") };
    let ps = sys.join(r"WindowsPowerShell\v1.0\powershell.exe");
    let mut cmd = std::process::Command::new(ps);
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &format!("& {{ {command} }} 2>&1 | Out-String -Width 200")])
        .current_dir(&sys)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return o.problem(Path::new(command), e),
    };
    let mut out = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = out.as_mut() {
            let _ = o.take(1 << 20).read_to_string(&mut s);
        }
        s
    });
    let started = std::time::Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.code(),
            Ok(None) => {
                // a tool that hangs (DISM can take long; an hour is the limit)
                if stop.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(3600) {
                    let _ = child.kill();
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
    let text = reader.join().unwrap_or_default();
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
        }
    }

    #[test]
    fn actions_read_back() {
        let p = r"C:\Users\sam\it's ‘odd’";
        let q = ps_quote(p);
        assert_eq!(action(&f(p, format!("{RM_FN}; sprawl-rm (Get-Item -LiteralPath {q} -Force)"))), Some((Action::Remove, false)));
        assert_eq!(action(&f(p, format!("{RM_FN}; sprawl-clear {q} 7   # as administrator"))), Some((Action::Clear { days: 7 }, true)));
        assert_eq!(action(&f(p, format!("{RM_FN}; sprawl-match {q} '*.dmp'"))), Some((Action::Match { pattern: "*.dmp".into() }, false)));
        // a command for another path, or not Sprawl's own, does nothing
        assert_eq!(action(&f(r"C:\other", format!("{RM_FN}; sprawl-clear {q} 0"))), None);
        assert_eq!(action(&f(p, format!("{RM_FN}; sprawl-clear {q} 0; Remove-Item C:\\"))), None);
        assert_eq!(action(&f(p, "npm cache clean --force".into())), None, "a tool command only from the built-in pack");
        let mut b = f(p, "npm cache clean --force".into());
        b.trusted = true;
        b.builtin = true;
        assert_eq!(action(&b), Some((Action::Tool { command: "npm cache clean --force".into() }, false)));
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

        let m = run(&cache, &Action::Match { pattern: "*.dmp".into() }, &stop, &mut |_| {});
        assert_eq!((m.freed, m.files), (500, 1));
        assert!(cache.join("sub").join("c.txt").exists());

        let c = run(&cache, &Action::Clear { days: 7 }, &stop, &mut |_| {});
        assert_eq!(c.freed, 0, "everything is new: nothing goes");

        let c = run(&cache, &Action::Clear { days: 0 }, &stop, &mut |_| {});
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
            let c = run(&cache, &Action::Clear { days: 0 }, &stop, &mut |_| {});
            assert_eq!(c.freed, 0);
            assert!(target.join("inner").join("precious.txt").exists());
            let r = run(&cache, &Action::Remove, &stop, &mut |_| {});
            assert!(r.problems.is_empty(), "{:?}", r.problems);
            assert!(!cache.exists() && target.join("inner").join("precious.txt").exists(), "removing a link leaves its target");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_tool_runs_hidden_with_output() {
        let mut o = Outcome::default();
        tool("Write-Output 'hello from a tool'; (Get-Location).Path", &AtomicBool::new(false), &mut o);
        assert!(o.output.iter().any(|l| l == "hello from a tool"), "{:?}", o.output);
        assert!(o.output.iter().any(|l| l.to_lowercase().ends_with(r"\system32")), "{:?}", o.output);
        let mut bad = Outcome::default();
        tool("exit 3", &AtomicBool::new(false), &mut bad);
        assert!(bad.problems.iter().any(|p| p.contains("code 3")));
    }
}
