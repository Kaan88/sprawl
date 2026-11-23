//! Evidence that files and folders were opened (A1): Explorer's Recent items, one shortcut for
//! each file or folder opened from Explorer or a program's Open box, its own date moved each
//! time it is opened again. Read only; from each shortcut only its local target path is taken
//! (a network target, or one without a path, is skipped). It shows use, never disuse: what was
//! opened another way isn't there, and Windows keeps only so many.

use std::path::PathBuf;
use std::time::SystemTime;

/// The most shortcuts read.
const MOST: usize = 5000;

/// (target path as the shortcut spells it, without a trailing `\`, when it was last opened) for
/// each shortcut in this account's Recent items. The case is kept: a restricted view matches
/// it exactly (Astra 1.0.2 round 3, S3 #2), the rest ignore it ([`opened_in`]).
pub fn recent_items() -> Vec<(String, SystemTime)> {
    let Some(dir) = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Recent")) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten().take(MOST) {
        let p = e.path();
        if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
            continue;
        }
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() || m.len() > 64 << 10 {
            continue;
        }
        let (Ok(t), Ok(bytes)) = (m.modified(), std::fs::read(&p)) else { continue };
        if let Some(target) = lnk_target(&bytes) {
            out.push((target.trim_end_matches('\\').to_string(), t));
        }
    }
    out
}

/// The newest time something at `path` or inside it was opened. `exact`: the names in their
/// own case (a restricted view, whose items were kept only as spelled on screen); otherwise
/// case is ignored.
pub fn opened_in(items: &[(String, SystemTime)], path: &str, exact: bool) -> Option<SystemTime> {
    // exact: each name in its own case, but the drive letter as Windows takes it (either
    // case), as everywhere else (Astra 1.0.2 round 4, S3 #2)
    let key = |s: &str| if exact { drive_upper(s.trim_end_matches('\\')) } else { s.trim_end_matches('\\').to_lowercase() };
    let at = key(path);
    let inside = format!("{at}\\");
    items.iter().filter(|(p, _)| { let p = key(p); p == at || p.starts_with(&inside) }).map(|&(_, t)| t).max()
}

/// `c:\x` as `C:\x`: only a drive letter's case changes.
fn drive_upper(s: &str) -> String {
    let mut c = s.chars();
    match (c.next(), c.next()) {
        (Some(d), Some(':')) if d.is_ascii_alphabetic() => format!("{}:{}", d.to_ascii_uppercase(), &s[2..]),
        _ => s.to_string(),
    }
}

/// Whether this machine keeps last-access times: `NtfsDisableLastAccessUpdate` is 0 (on) or
/// 0x80000000 (user managed, on). 1 and 0x80000001 are off; 2 and 0x80000002 are system managed
/// (on for the system volume only) and so not taken as kept for every drive here. Either way
/// Windows updates a time lazily (up to an hour), so it is a guide to use, never a clock.
pub fn access_times_kept() -> bool {
    crate::reg::Key::open(crate::reg::HKLM, r"SYSTEM\CurrentControlSet\Control\FileSystem", crate::reg::View::Native)
        .and_then(|k| k.dword("NtfsDisableLastAccessUpdate"))
        .is_some_and(keeps_last_access)
}

/// [`access_times_kept`]'s reading of the registry value, on its own so it can be tested.
fn keeps_last_access(v: u32) -> bool {
    v == 0 || v == 0x8000_0000
}

/// What one path's last-access lookup gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LastOpened {
    /// The newest access time, and whether everything at the path was read.
    At(SystemTime, bool),
    /// Not read: this answer's reading limit ran out first. Never take it for "not opened".
    Unread,
    /// No time here: this PC keeps none, or the drive can't be read for them.
    Unknown,
}

/// The most entries one answer reads for last-access times.
pub const ANSWER_BUDGET: usize = 40_000;
/// The most entries read for one path asked about.
const PER_PATH: usize = 4_000;
const NOT_KEPT: &str = "this PC has Windows' last-access updates turned off";
const NOT_FIXED: &str = "this drive isn't a local fixed disk, so its times aren't read";
const NOT_NTFS: &str = "this drive isn't NTFS";

/// Last-access times for one answer: the setting and the folders to skip are read once, each
/// drive is asked about once, and the work is bounded, so an answer about a folder of a million
/// files stays quick. [`Accessed::off`] reads nothing (a client that may only see the scan on
/// screen).
pub struct Accessed {
    kept: bool,
    /// A client that may only see the scan: nothing is read and nothing is said about the disk.
    silent: bool,
    /// Entries still allowed to be read for this answer.
    left: usize,
    excluded: Vec<PathBuf>,
    drives: std::collections::HashMap<char, Drive>,
    /// Why no time can be read, when that is the machine or the drive rather than the limit.
    why: Option<&'static str>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Drive {
    FixedNtfs,
    NotFixed,
    NotNtfs,
}

impl Accessed {
    /// For an answer that may look at the disk.
    pub fn new() -> Accessed {
        Accessed { kept: access_times_kept(), silent: false, left: ANSWER_BUDGET, excluded: crate::exclusions::configured(), drives: Default::default(), why: None }
    }

    /// For a client that may only see the scan on screen: nothing is read, and no note is added
    /// about the machine (it never looked).
    pub fn off() -> Accessed {
        Accessed { kept: false, silent: true, left: 0, excluded: Vec::new(), drives: Default::default(), why: None }
    }

    /// The same with a given reading limit, to see what running out says.
    #[cfg(test)]
    pub(crate) fn with_budget(left: usize) -> Accessed {
        Accessed { kept: true, silent: false, left, excluded: Vec::new(), drives: Default::default(), why: None }
    }

    /// Whether this machine keeps last-access times at all.
    pub fn kept(&self) -> bool {
        self.kept
    }

    /// Whether this is a client that may only see the scan on screen (it never reads the disk
    /// and never says anything about it).
    pub fn silent(&self) -> bool {
        self.silent
    }

    /// When anything at `path` was last opened. For a folder this is the newest time of the
    /// files in it: listing a folder moves its own time, so its own is never used.
    pub fn of(&mut self, path: &std::path::Path) -> LastOpened {
        if !self.kept {
            self.why.get_or_insert(NOT_KEPT);
            return LastOpened::Unknown;
        }
        if self.left == 0 {
            return LastOpened::Unread;
        }
        let Some(drive) = path.to_string_lossy().chars().next().map(|c| c.to_ascii_uppercase()).filter(|c| c.is_ascii_alphabetic()) else {
            self.why.get_or_insert(NOT_FIXED);
            return LastOpened::Unknown;
        };
        match *self.drives.entry(drive).or_insert_with(|| {
            if !fixed_drive(drive) {
                Drive::NotFixed
            } else if !ntfs(drive) {
                Drive::NotNtfs
            } else {
                Drive::FixedNtfs
            }
        }) {
            Drive::FixedNtfs => {}
            Drive::NotFixed => {
                self.why.get_or_insert(NOT_FIXED);
                return LastOpened::Unknown;
            }
            Drive::NotNtfs => {
                self.why.get_or_insert(NOT_NTFS);
                return LastOpened::Unknown;
            }
        }
        let mut budget = self.left.min(PER_PATH);
        let (t, complete) = accessed_in_with(path, &mut budget, &self.excluded);
        self.left -= self.left.min(PER_PATH) - budget;
        match t {
            Some(t) => LastOpened::At(t, complete),
            // the limit stopped it, or nothing there could be read: an answer, not a "no"
            None if budget == 0 => LastOpened::Unread,
            None => LastOpened::Unknown,
        }
    }

    /// [`Accessed::of`] as a line builder takes it: the time and whether all of the path was
    /// read, or None with the reason counted in `unread` (the limit) or `unknown` (this PC).
    pub fn time(&mut self, path: &std::path::Path, unread: &mut usize, unknown: &mut usize) -> Option<(SystemTime, bool)> {
        match self.of(path) {
            LastOpened::At(t, complete) => Some((t, complete)),
            LastOpened::Unread => {
                *unread += 1;
                None
            }
            LastOpened::Unknown => {
                *unknown += 1;
                None
            }
        }
    }
}

/// The lines said after an answer for items whose last-access time wasn't read: how many the
/// reading limit left out, and how many have no time here and why. Empty for a restricted
/// client, and when everything asked about was read.
pub fn notes(unread: usize, unknown: usize, one: &str, many: &str, access: &Accessed) -> Vec<String> {
    let mut out = Vec::new();
    if access.silent {
        return out;
    }
    let count = |n: usize| format!("{n} {}", if n == 1 { one } else { many });
    if unread > 0 {
        out.push(format!("{} not checked for a last-opened time: the reading limit ran out (a smaller folder or a narrower search gives more).", count(unread)));
    }
    if unknown > 0 {
        out.push(format!("{} with no last-opened time here: {}.", count(unknown), access.why.unwrap_or("the time couldn't be read")));
    }
    out
}

/// The volume `X:\` is a fixed disk (not a network, removable or optical drive): only those have
/// last-access times worth reading, and a mapped share that reports NTFS would be crawled slowly.
#[cfg(windows)]
pub(crate) fn fixed_drive(drive: char) -> bool {
    const DRIVE_FIXED: u32 = 3;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
    }
    let root: Vec<u16> = format!("{drive}:\\").encode_utf16().chain([0]).collect();
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_FIXED }
}

#[cfg(not(windows))]
fn fixed_drive(_: char) -> bool {
    false
}

/// The volume `X:\` is NTFS (FAT keeps only a date, other file systems their own rules).
#[cfg(windows)]
pub fn ntfs(drive: char) -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetVolumeInformationW(root: *const u16, name: *mut u16, name_len: u32, serial: *mut u32, max: *mut u32, flags: *mut u32, fs: *mut u16, fs_len: u32) -> i32;
    }
    let root: Vec<u16> = format!("{drive}:\\").encode_utf16().chain([0]).collect();
    let mut fs = [0u16; 32];
    let ok = unsafe { GetVolumeInformationW(root.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), fs.as_mut_ptr(), fs.len() as u32) } != 0;
    ok && String::from_utf16_lossy(&fs[..fs.iter().position(|&c| c == 0).unwrap_or(0)]) == "NTFS"
}

#[cfg(not(windows))]
pub fn ntfs(_: char) -> bool {
    false
}

/// The newest last-access time of the files at or below `path` (folders don't count: listing
/// one, as a scan does, moves its time; links aren't followed), from the folder listings alone,
/// so no file contents are read and no file is opened. The starting path is pinned once;
/// below it, folders listed as links aren't entered. At most `budget` entries are read,
/// counted down; the flag says whether everything was read.
pub fn accessed_in(path: &std::path::Path, budget: &mut usize) -> (Option<SystemTime>, bool) {
    accessed_in_with(path, budget, &crate::exclusions::configured())
}

fn accessed_in_with(path: &std::path::Path, budget: &mut usize, excluded: &[PathBuf]) -> (Option<SystemTime>, bool) {
    const REPARSE: u32 = 0x400;
    use std::os::windows::fs::MetadataExt;
    if *budget == 0 || excluded.iter().any(|p| crate::finding::within(path, p)) { return (None, false); }
    let Ok(held) = sprawl_core::local::scope(path) else { return (None, false) };
    let Ok(top) = held.metadata() else { return (None, false) };
    if top.is_file() {
        *budget -= 1;
        let time = top.accessed().ok();
        return (time, time.is_some());
    }
    let mut best: Option<SystemTime> = None;
    let mut complete = true;
    let mut stack = vec![path.to_path_buf()];
    while let Some(d) = stack.pop() {
        if excluded.iter().any(|p| crate::finding::within(&d, p)) { complete = false; continue; }
        let Ok(rd) = std::fs::read_dir(&d) else { complete = false; continue };
        for entry in rd {
            if *budget == 0 {
                return (best, false);
            }
            *budget -= 1;
            let Ok(e) = entry else { complete = false; continue };
            let p = e.path();
            if excluded.iter().any(|x| crate::finding::within(&p, x)) { complete = false; continue; }
            // From the listing itself: opening each file (1.0) made advice slow on big trees.
            let Ok(m) = e.metadata() else { complete = false; continue };
            if m.file_attributes() & (REPARSE | crate::dupes::NOT_LOCAL) != 0 {
                complete = false;
                continue;
            }
            if m.is_dir() {
                stack.push(p);
            } else if let Ok(t) = m.accessed() {
                best = best.max(Some(t));
            } else {
                complete = false;
            }
        }
    }
    (best, complete)
}

fn u16_at(b: &[u8], i: usize) -> Option<usize> {
    b.get(i..i + 2).map(|x| u16::from_le_bytes([x[0], x[1]]) as usize)
}

fn u32_at(b: &[u8], i: usize) -> Option<usize> {
    b.get(i..i + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize)
}

/// A NUL-ended string of plain ASCII at `at` (the ANSI one's code page isn't known here: one
/// with other characters is left out rather than read wrong).
fn ascii(b: &[u8], at: usize) -> Option<String> {
    let s = b.get(at..)?;
    let end = s.iter().position(|&c| c == 0)?;
    s[..end].is_ascii().then(|| String::from_utf8_lossy(&s[..end]).into_owned())
}

/// A NUL-ended UTF-16 string at `at`.
fn wide(b: &[u8], at: usize) -> Option<String> {
    let s = b.get(at..)?;
    let units: Vec<u16> = s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
    String::from_utf16(&units).ok()
}

/// The local path a shell link (`.lnk`, MS-SHLLINK) points at, from its LinkInfo: the local
/// base path and the common path suffix, the Unicode ones when present. None for anything else.
pub(crate) fn lnk_target(b: &[u8]) -> Option<String> {
    // header size, then the link's flags
    if u32_at(b, 0)? != 0x4C {
        return None;
    }
    let flags = u32_at(b, 0x14)?;
    let mut i = 0x4C;
    // HasLinkTargetIDList: skipped
    if flags & 1 != 0 {
        i += 2 + u16_at(b, i)?;
    }
    // HasLinkInfo
    if flags & 2 == 0 {
        return None;
    }
    let info = b.get(i..)?;
    let info = info.get(..u32_at(info, 0)?)?;
    let header = u32_at(info, 4)?;
    // VolumeIDAndLocalBasePath
    if u32_at(info, 8)? & 1 == 0 {
        return None;
    }
    let unicode = if header >= 0x24 { Some((u32_at(info, 0x1C)?, u32_at(info, 0x20)?)) } else { None };
    let (base, suffix) = match unicode {
        Some((bu, su)) if bu != 0 => (wide(info, bu)?, if su != 0 { wide(info, su)? } else { String::new() }),
        _ => (ascii(info, u32_at(info, 0x10)?)?, ascii(info, u32_at(info, 0x18)?)?),
    };
    let mut p = base;
    if !suffix.is_empty() {
        if !p.ends_with('\\') {
            p.push('\\');
        }
        p.push_str(&suffix);
    }
    let bytes = p.as_bytes();
    (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' && !p.contains(['\0', '\n'])).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_evidence_marks_exclusions_and_budget_as_incomplete() {
        let dir = std::env::temp_dir().join(format!("sprawl-access-evidence-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("skipped")).unwrap();
        std::fs::write(dir.join("visible.txt"), b"visible").unwrap();
        std::fs::write(dir.join("skipped/secret.txt"), b"excluded").unwrap();
        assert!(accessed_in_with(&dir, &mut 100, &[]).1);
        assert!(!accessed_in_with(&dir, &mut 100, &[dir.join("skipped")]).1);
        assert_eq!(accessed_in_with(&dir.join("skipped"), &mut 100, &[dir.join("skipped")]), (None, false));
        assert_eq!(accessed_in_with(&dir.join("visible.txt"), &mut 0, &[]), (None, false));
        assert!(!accessed_in_with(&dir, &mut 1, &[]).1);
        assert_eq!(accessed_in_with(&dir.join("missing"), &mut 100, &[]), (None, false));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Only 0 and 0x80000000 are on; the rest are off or system-volume-only.
    #[test]
    fn only_an_explicit_on_counts_as_kept() {
        assert!(keeps_last_access(0));
        assert!(keeps_last_access(0x8000_0000));
        // 1 and 0x80000001 are off
        for v in [1, 0x8000_0001] {
            assert!(!keeps_last_access(v), "{v:#x}");
        }
        // 2, 3, 0x80000002 and 0x80000003 are system managed: the system volume only
        for v in [2, 3, 0x8000_0002, 0x8000_0003] {
            assert!(!keeps_last_access(v), "{v:#x}");
        }
    }

    /// An answer that may only see the scan reads nothing, and a kept machine reads a file's time.
    #[test]
    fn one_answers_access_times() {
        let dir = std::env::temp_dir().join(format!("sprawl-accessed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("kept.txt");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(Accessed::off().of(&file), LastOpened::Unknown, "a restricted answer never reads");
        let mut a = Accessed::new();
        if a.kept() {
            let LastOpened::At(t, complete) = a.of(&file) else { panic!("a file's own time") };
            assert!(complete);
            assert!(t > SystemTime::UNIX_EPOCH);
            // a file that isn't there isn't given a time
            assert_eq!(a.of(&dir.join("gone.txt")), LastOpened::Unknown);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A reading limit that runs out is "not read", never "not opened", and is counted in the
    /// answer; a restricted client hears nothing about the machine.
    #[test]
    fn a_reading_limit_that_ran_out_is_not_a_no() {
        let dir = std::env::temp_dir().join(format!("sprawl-accessed-limit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let mut a = Accessed::with_budget(0);
        assert_eq!(a.of(&file), LastOpened::Unread);
        let said = notes(1, 0, "match", "matches", &a);
        assert_eq!(said, vec!["1 match not checked for a last-opened time: the reading limit ran out (a smaller folder or a narrower search gives more)."]);
        assert!(notes(2, 0, "item", "items", &a)[0].starts_with("2 items not checked"));
        // a PC that keeps none says why, once
        let mut none = Accessed { kept: false, silent: false, left: 10, excluded: Vec::new(), drives: Default::default(), why: None };
        assert_eq!(none.of(&file), LastOpened::Unknown);
        assert!(notes(0, 1, "item", "items", &none)[0].contains("last-access updates turned off"));
        // a restricted client may not see the disk: nothing is said
        assert!(notes(1, 2, "match", "matches", &Accessed::off()).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A link with only a LinkInfo: an ANSI base path and suffix, and the Unicode ones when given.
    fn link(base: &str, suffix: &str, unicode: Option<(&str, &str)>) -> Vec<u8> {
        let mut b = vec![0u8; 0x4C];
        b[0] = 0x4C;
        b[0x14] = 2 | 0x80; // HasLinkInfo, IsUnicode
        let header: usize = if unicode.is_some() { 0x24 } else { 0x1C };
        let mut body = Vec::new();
        let base_at = header;
        body.extend(base.as_bytes());
        body.push(0);
        let suffix_at = header + body.len();
        body.extend(suffix.as_bytes());
        body.push(0);
        let mut uni = (0usize, 0usize);
        if let Some((ub, us)) = unicode {
            uni.0 = header + body.len();
            body.extend(ub.encode_utf16().flat_map(|u| u.to_le_bytes()));
            body.extend([0, 0]);
            uni.1 = header + body.len();
            body.extend(us.encode_utf16().flat_map(|u| u.to_le_bytes()));
            body.extend([0, 0]);
        }
        let mut info = vec![0u8; header];
        let put = |v: &mut Vec<u8>, at: usize, x: usize| v[at..at + 4].copy_from_slice(&(x as u32).to_le_bytes());
        put(&mut info, 0, header + body.len());
        put(&mut info, 4, header);
        put(&mut info, 8, 1);
        put(&mut info, 0x10, base_at);
        put(&mut info, 0x18, suffix_at);
        if unicode.is_some() {
            put(&mut info, 0x1C, uni.0);
            put(&mut info, 0x20, uni.1);
        }
        info.extend(body);
        b.extend(info);
        b
    }

    #[test]
    fn link_targets_are_read() {
        assert_eq!(lnk_target(&link(r"D:\Films\a.mkv", "", None)).as_deref(), Some(r"D:\Films\a.mkv"));
        assert_eq!(lnk_target(&link(r"D:\", r"Films\a.mkv", None)).as_deref(), Some(r"D:\Films\a.mkv"));
        assert_eq!(lnk_target(&link(r"D:\F?lms", "", Some((r"D:\Fïlms\été.mkv", "")))).as_deref(), Some(r"D:\Fïlms\été.mkv"));
        // a non-ASCII ANSI path without the Unicode one: left out
        let mut l = link("D:\\Films", "", None);
        let at = l.iter().position(|&c| c == b'F').unwrap();
        l[at] = 0xE9;
        assert_eq!(lnk_target(&l), None);
        // not a link, cut short, a network path
        assert_eq!(lnk_target(b"hello"), None);
        assert_eq!(lnk_target(&link(r"D:\a", "", None)[..0x50]), None);
        assert_eq!(lnk_target(&link(r"\\server\share\a", "", None)), None);
    }

    #[test]
    fn opened_inside_a_folder() {
        let t = |s| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s);
        let items = vec![(r"d:\films\a.mkv".to_string(), t(10)), (r"d:\films\old\b.mkv".to_string(), t(5)), (r"d:\filmsx\c.mkv".to_string(), t(20))];
        assert_eq!(opened_in(&items, r"d:\films", false), Some(t(10)));
        assert_eq!(opened_in(&items, r"d:\films\old", false), Some(t(5)));
        assert_eq!(opened_in(&items, r"d:\films\a.mkv", false), Some(t(10)));
        assert_eq!(opened_in(&items, r"d:\music", false), None);
        // case: ignored, or (a restricted view) kept
        assert_eq!(opened_in(&items, r"D:\Films", false), Some(t(10)));
        assert_eq!(opened_in(&items, r"D:\Films", true), None);
        assert_eq!(opened_in(&items, r"d:\films\old", true), Some(t(5)));
        // ...but the drive letter's case never decides (round 4, S3 #2)
        assert_eq!(opened_in(&items, r"D:\films\old", true), Some(t(5)));
        assert_eq!(opened_in(&items, r"D:\Films\old", true), None);
    }

    /// Usage is looked up in the app's own folder, never a sibling differing only in case.
    #[test]
    fn a_case_only_sibling_is_never_taken_by_chance() {
        let dir = std::env::temp_dir().join(format!("sprawl-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Game")).unwrap();
        let mut tree = sprawl_core::walk::inventory(&dir, &sprawl_core::Progress::new()).unwrap();
        // a case-sensitive folder can hold `game` beside `Game`: made in the tree
        let mut twin = tree.nodes[tree.nodes[sprawl_core::ROOT].children[0]].clone();
        twin.name = "game".into();
        twin.children.clear();
        let at = tree.nodes.len();
        tree.nodes.push(twin);
        tree.nodes[sprawl_core::ROOT].children.insert(0, at);
        let real = tree.nodes[sprawl_core::ROOT].children[1];
        assert_eq!(crate::find_path_unambiguous(&tree, &dir.join("Game")), Some(real));
        assert_eq!(crate::find_path_unambiguous(&tree, &dir.join("game")), Some(at));
        assert_eq!(crate::find_path_unambiguous(&tree, &dir.join("GAME")), None, "neither by chance");
        assert_eq!(crate::find_path(&tree, &dir.join("GAME")), Some(at), "the plain lookup takes the first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same for two scan roots differing only in case (Astra 1.0.3 review, S3 #2).
    #[test]
    fn a_case_only_scan_root_is_never_taken_by_chance() {
        use sprawl_core::{Node, Tree, MULTIPLE};
        let dir = |name: &str, parent| Node { name: name.into(), is_dir: true, parent, ..Default::default() };
        let mut nodes = vec![dir(MULTIPLE, None), dir(r"C:\Apps\Game", Some(0)), dir(r"C:\Apps\game", Some(0)), dir("bin", Some(1)), dir("bin", Some(2))];
        nodes[0].children = vec![1, 2];
        nodes[1].children = vec![3];
        nodes[2].children = vec![4];
        let tree = Tree { volumes: Vec::new(), root_path: MULTIPLE.into(), nodes, errors: 0, method: Default::default() };
        let find = |p: &str| crate::find_path_unambiguous(&tree, std::path::Path::new(p));
        assert_eq!(find(r"C:\Apps\Game\bin"), Some(3));
        assert_eq!(find(r"C:\Apps\game\bin"), Some(4), "the second root's app gets its own usage");
        assert_eq!(find(r"c:\Apps\game"), Some(2), "the drive letter's case doesn't count");
        assert_eq!(find(r"C:\Apps\GAME\bin"), None, "neither by chance");
    }

    /// The real readers on a small folder of this repository (run by hand).
    #[test]
    #[ignore]
    fn this_pcs_access_times() {
        let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
        let mut budget = 10_000;
        let (t, complete) = accessed_in(&docs, &mut budget);
        eprintln!("kept {} ntfs C {} docs newest access {:?} complete {complete}, {} entries read", access_times_kept(), ntfs('C'), t.map(crate::facts::month_year), 10_000 - budget);
        assert!(t.is_some() && complete);
        let mut small = 2;
        assert!(!accessed_in(&docs, &mut small).1, "a budget cut is said");
    }

    /// This account's own Recent items parse (run by hand: it reads the real folder).
    #[test]
    #[ignore]
    fn this_accounts_recent_items() {
        let items = recent_items();
        let n = std::fs::read_dir(PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(r"Microsoft\Windows\Recent")).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "lnk")).count();
        eprintln!("{} of {n} shortcuts read; first: {:?}", items.len(), items.iter().take(5).map(|(p, _)| p.len()).collect::<Vec<_>>());
        assert!(n == 0 || !items.is_empty());
    }
}
