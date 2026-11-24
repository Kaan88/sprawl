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

/// Whether this machine keeps last-access times: `NtfsDisableLastAccessUpdate`'s low two bits
/// say whether Windows updates them. 0 (`fsutil` "User Managed, updates enabled") and 2 ("System
/// Managed, updates enabled", which Windows picks at boot for a system volume of 128 GB or less)
/// are on; 1 and 3 are off. Older Windows writes 0x80000000/0x80000002/0x80000001/0x80000003,
/// whose low bits mean the same. Either way Windows updates a time lazily (up to an hour), so it
/// is a guide to use, never a clock.
pub fn access_times_kept() -> bool {
    crate::reg::Key::open(crate::reg::HKLM, r"SYSTEM\CurrentControlSet\Control\FileSystem", crate::reg::View::Native)
        .and_then(|k| k.dword("NtfsDisableLastAccessUpdate"))
        .is_some_and(keeps_last_access)
}

/// [`access_times_kept`]'s reading of the registry value, on its own so it can be tested: the
/// low two bits are the system's own on/off choice for NTFS (0 and 2 update last-access times,
/// 1 and 3 don't); the high bit only says who decided.
fn keeps_last_access(v: u32) -> bool {
    matches!(v & 3, 0 | 2)
}

/// Why no time can be read at all: not the limit, not the machine, but the view itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unseen {
    /// A client that may only see the scan on screen.
    ScanOnly,
    /// A view loaded from a CSV file, not read from the disk now.
    Csv,
}

/// The drive letter of a plain local path (`X:\...`), or None for a relative, UNC or device path:
/// those are never read, so a relative path is never resolved against this process's folder.
pub(crate) fn drive_of(path: &std::path::Path) -> Option<char> {
    let s = path.to_string_lossy();
    let b = s.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\').then(|| (b[0] as char).to_ascii_uppercase())
}

/// What one path's last-access lookup gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LastOpened {
    /// The newest access time, and whether everything at the path was read.
    At(SystemTime, bool),
    /// The newest time of the part that was read, but the reading limit ran out before the
    /// rest: never take the rest for "not opened", and never decide the whole path either way.
    Limited(SystemTime),
    /// Not read: this answer's reading limit ran out first. Never take it for "not opened".
    Unread,
    /// No time here: this PC keeps none, the drive can't be read for them, or there is nothing
    /// to have one from (a folder holding no files).
    Unknown,
}

/// What one match's last-opened time said about a `da:` range. One value per state, so a caller
/// can't pass a reading limit or a partly read folder off as "not opened".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InRange {
    /// Read, and its newest time is inside the range.
    Yes,
    /// Read, and its newest time is outside the range.
    No,
    /// The reading limit ran out before all of it was read.
    Unread,
    /// Read only in part (a folder inside was skipped or couldn't be read): never decided.
    Partial,
    /// No time here (this PC, the drive, a skip, or nothing to have one from).
    Unknown,
}

/// What `gave` says about a match whose time must be inside `[from, to)` (seconds since 1970):
/// kept, out of range, or never decided and why.
pub fn in_range(gave: LastOpened, from: i64, to: i64) -> InRange {
    match gave {
        LastOpened::At(t, true) => {
            let s = sprawl_core::query::unix_secs(t);
            if from <= s && s < to {
                InRange::Yes
            } else {
                InRange::No
            }
        }
        LastOpened::At(_, false) => InRange::Partial,
        LastOpened::Limited(_) => InRange::Unread,
        LastOpened::Unread => InRange::Unread,
        LastOpened::Unknown => InRange::Unknown,
    }
}

/// The most entries one answer reads for last-access times.
pub const ANSWER_BUDGET: usize = 40_000;
/// The most entries read for one path asked about.
pub const PER_PATH: usize = 4_000;
const NOT_KEPT: &str = "this PC has Windows' last-access updates turned off";
const NOT_FIXED: &str = "this drive isn't a local fixed disk, so its times aren't read";
const NOT_NTFS: &str = "this drive isn't NTFS";
const SKIPPED: &str = "it is in a folder skipped in Settings > Scanning";
const SKIPPED_UNKNOWN: &str = "the folders to skip in Settings > Scanning couldn't be read";
const NO_FILES: &str = "it holds no files to have one from";
/// An open that says NotFound: the item (or the folder it was in) was removed since the scan.
pub(crate) const GONE: &str = "the item is gone";
/// Everything else an open refuses: a link or a cloud file is never followed or recalled, and a
/// name Sprawl doesn't open is never opened. None of these means the item is gone.
pub(crate) const NOT_READ: &str = "it is a link, a cloud file that isn't on this PC, a name Sprawl doesn't open, or Windows wouldn't let it be opened, so it isn't read";
const UNREADABLE: &str = "the time couldn't be read";

/// Counts one more answer without a time under `why`.
pub(crate) fn tally_reason(reasons: &mut Vec<(&'static str, usize)>, why: &'static str) {
    match reasons.iter_mut().find(|(w, _)| *w == why) {
        Some((_, n)) => *n += 1,
        None => reasons.push((why, 1)),
    }
}

/// One line per reason an answer had no time: `{n} {one|many} {what}: {reason}.`
pub(crate) fn reason_lines(reasons: &[(&'static str, usize)], one: &str, many: &str, what: &str) -> Vec<String> {
    reasons.iter().map(|&(why, n)| format!("{n} {} {what}: {why}.", if n == 1 { one } else { many })).collect()
}

/// The error an open gave, as the reason said for it: only NotFound means gone.
pub(crate) fn refusal_of(e: &std::io::Error) -> &'static str {
    if e.kind() == std::io::ErrorKind::NotFound {
        GONE
    } else {
        NOT_READ
    }
}

/// Last-access times for one answer: the setting and the folders to skip are read once, each
/// drive is asked about once, and the work is bounded, so an answer about a folder of a million
/// files stays quick. [`Accessed::off`] reads nothing (a client that may only see the scan on
/// screen), and [`Accessed::off_for`] reads nothing for a view that has no disk times.
pub struct Accessed {
    kept: bool,
    /// Nothing is read: a client that may only see the scan, or a view from a CSV file.
    unseen: Option<Unseen>,
    /// Entries still allowed to be read for this answer.
    left: usize,
    excluded: Vec<PathBuf>,
    /// The folder last pinned: a search's matches are often siblings, and one open serves them
    /// all instead of opening the whole chain for each file.
    parent: Option<(PathBuf, sprawl_core::local::Directory)>,
    drives: std::collections::HashMap<char, Drive>,
    /// How many answers had no time for each reason (the machine, the drive, a skip, a refused
    /// or vanished item, nothing to have one from): each count is said with its own reason.
    reasons: Vec<(&'static str, usize)>,
    /// Why the last [`Accessed::read`] found nothing, when the item itself was refused or gone.
    refusal: Option<&'static str>,
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
        Accessed { kept: access_times_kept(), unseen: None, left: ANSWER_BUDGET, excluded: crate::exclusions::configured(), parent: None, drives: Default::default(), reasons: Vec::new(), refusal: None }
    }

    /// For a client that may only see the scan on screen: nothing is read, and no note is added
    /// about the machine (it never looked).
    pub fn off() -> Accessed {
        Accessed::off_for(Unseen::ScanOnly)
    }

    /// For a view that holds no disk times at all: a client that may only see the scan, or a
    /// view loaded from a CSV file. Nothing is read.
    pub fn off_for(unseen: Unseen) -> Accessed {
        Accessed { kept: false, unseen: Some(unseen), left: 0, excluded: Vec::new(), parent: None, drives: Default::default(), reasons: Vec::new(), refusal: None }
    }

    /// The same with a given reading limit, to see what running out says (a test, or an answer
    /// smaller than [`ANSWER_BUDGET`]).
    pub fn with_budget(left: usize) -> Accessed {
        Accessed { kept: true, unseen: None, left, excluded: Vec::new(), parent: None, drives: Default::default(), reasons: Vec::new(), refusal: None }
    }

    /// The same with given folders to skip, to see what a skipped folder says.
    #[cfg(test)]
    pub(crate) fn with_excluded(left: usize, excluded: Vec<PathBuf>) -> Accessed {
        Accessed { kept: true, unseen: None, left, excluded, parent: None, drives: Default::default(), reasons: Vec::new(), refusal: None }
    }

    fn tally(&mut self, why: &'static str) {
        tally_reason(&mut self.reasons, why);
    }

    /// Pretends the machine keeps (or doesn't keep) last-access times, for a test in another crate.
    #[doc(hidden)]
    pub fn set_kept_for_test(&mut self, kept: bool) {
        self.kept = kept;
    }

    /// Whether this machine keeps last-access times at all.
    pub fn kept(&self) -> bool {
        self.kept
    }

    /// Whether this is a client that may only see the scan on screen, or a CSV view (it never
    /// reads the disk and never says anything about it).
    pub fn silent(&self) -> bool {
        self.unseen.is_some()
    }

    /// Why nothing is read at all, when it is the view rather than the machine.
    pub fn unseen(&self) -> Option<Unseen> {
        self.unseen
    }

    /// When anything at `path` was last opened. For a folder this is the newest time of the
    /// files in it: listing a folder moves its own time, so its own is never used.
    pub fn of(&mut self, path: &std::path::Path) -> LastOpened {
        if self.unseen.is_some() {
            return LastOpened::Unknown;
        }
        if !self.kept {
            self.tally(NOT_KEPT);
            return LastOpened::Unknown;
        }
        if self.left == 0 {
            return LastOpened::Unread;
        }
        // a folder the user skips is not read, and a list that can't be read refuses them all
        if crate::exclusions::unreadable(&self.excluded) {
            self.tally(SKIPPED_UNKNOWN);
            return LastOpened::Unknown;
        }
        if self.excluded.iter().any(|p| crate::finding::within(path, p)) {
            self.tally(SKIPPED);
            return LastOpened::Unknown;
        }
        let Some(drive) = drive_of(path) else {
            self.tally(NOT_FIXED);
            return LastOpened::Unknown;
        };
        match self.drive_kind(drive) {
            Drive::FixedNtfs => {}
            Drive::NotFixed => {
                self.tally(NOT_FIXED);
                return LastOpened::Unknown;
            }
            Drive::NotNtfs => {
                self.tally(NOT_NTFS);
                return LastOpened::Unknown;
            }
        }
        let mut budget = self.left.min(PER_PATH);
        self.refusal = None;
        let (t, complete) = self.read(path, &mut budget);
        self.left -= self.left.min(PER_PATH) - budget;
        match t {
            // everything read: a file that used the budget's last unit was still read whole
            Some(t) if complete => LastOpened::At(t, true),
            // the reading limit stopped the walk before the end: a guide, never a decision
            Some(t) if budget == 0 => LastOpened::Limited(t),
            // a skipped or unreadable folder inside: read only in part
            Some(t) => LastOpened::At(t, false),
            // the limit stopped it, or nothing there could be read: an answer, not a "no"
            None if budget == 0 => LastOpened::Unread,
            // everything was read and no file was there to have a time (times-16)
            None if complete => {
                self.tally(NO_FILES);
                LastOpened::Unknown
            }
            None => {
                let why = self.refusal.take().unwrap_or(UNREADABLE);
                self.tally(why);
                LastOpened::Unknown
            }
        }
    }

    /// What kind of volume `drive` is, asked once for the answer.
    fn drive_kind(&mut self, drive: char) -> Drive {
        *self.drives.entry(drive).or_insert_with(|| {
            if !fixed_drive(drive) {
                Drive::NotFixed
            } else if !ntfs(drive) {
                Drive::NotNtfs
            } else {
                Drive::FixedNtfs
            }
        })
    }

    /// Whether `path` is on a drive whose last-access times are read at all (a local fixed NTFS
    /// disk). A search asks this before "too many to read": narrowing it could never help on a
    /// drive that is never read.
    pub fn reads_drive(&mut self, path: &std::path::Path) -> bool {
        match drive_of(path) {
            Some(d) => self.drive_kind(d) == Drive::FixedNtfs,
            None => false,
        }
    }

    /// The newest last-access time at `path`: a file from its own attributes, opened through its
    /// folder's handle with full sharing (so it is never held against a rename or delete), a
    /// folder from the files in it. The folder last pinned is kept for the next sibling.
    fn read(&mut self, path: &std::path::Path, budget: &mut usize) -> (Option<SystemTime>, bool) {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
            // a drive's root: nothing above it to pin
            return accessed_in_with(path, budget, &self.excluded);
        };
        if let Err(e) = self.pin(parent) {
            self.refusal = Some(refusal_of(&e));
            return (None, false);
        }
        let Some((_, dir)) = self.parent.as_ref() else { return (None, false) };
        use sprawl_core::local::{Disposition, Kind, READ_ATTRIBUTES, SHARE_DELETE, SHARE_READ, SHARE_WRITE};
        let file = match dir.open_in(name, READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE | SHARE_DELETE, Disposition::Open, Kind::Either, false) {
            Ok(f) => f,
            Err(e) => {
                self.refusal = Some(refusal_of(&e));
                return (None, false);
            }
        };
        let Ok(m) = file.metadata() else { return (None, false) };
        if m.is_file() {
            *budget -= 1;
            let time = m.accessed().ok();
            return (time, time.is_some());
        }
        // a folder: pinned again through its own handle for the walk (no delete sharing), as it
        // was before the file leaves were opened with full sharing, so the folder can't be flipped
        // for a junction between the check here and the walk below
        drop(file);
        accessed_in_with(path, budget, &self.excluded)
    }

    /// The folder at `path` pinned, reusing the one already held when it is the same folder.
    fn pin(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        if self.parent.as_ref().is_some_and(|(p, _)| p == path) {
            return Ok(());
        }
        self.parent = None;
        let dir = sprawl_core::local::directory(path)?;
        self.parent = Some((path.to_path_buf(), dir));
        Ok(())
    }

    /// [`Accessed::of`] as a line builder takes it: the time and whether all of the path was
    /// read, or None with the reason counted in `unread` (the limit) or `unknown` (this PC).
    pub fn time(&mut self, path: &std::path::Path, unread: &mut usize, unknown: &mut usize) -> Option<(SystemTime, bool)> {
        match self.of(path) {
            LastOpened::At(t, complete) => Some((t, complete)),
            // the newest time among what was read: shown, and said as only part of it
            LastOpened::Limited(t) => Some((t, false)),
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
    if access.unseen.is_some() {
        return out;
    }
    let count = |n: usize| format!("{n} {}", if n == 1 { one } else { many });
    if unread > 0 {
        out.push(format!("{} not checked for a last-opened time: the reading limit ran out (a smaller folder or a narrower search gives more).", count(unread)));
    }
    if unknown > 0 {
        if access.reasons.is_empty() {
            out.push(format!("{} with no last-opened time here: {UNREADABLE}.", count(unknown)));
        } else {
            out.extend(reason_lines(&access.reasons, one, many, "with no last-opened time here"));
        }
    }
    out
}

/// The line for matches a date filter couldn't decide because only part of them was read (a
/// folder inside was skipped or couldn't be read): never worded as the reading limit, which
/// wasn't what stopped it.
pub fn partial_note(partial: usize, one: &str, many: &str) -> Option<String> {
    (partial > 0).then(|| {
        format!(
            "{} {} not fully read, so its last-opened time wasn't checked (part of it was skipped or couldn't be read).",
            partial,
            if partial == 1 { one } else { many }
        )
    })
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
    if *budget == 0 || excluded.iter().any(|p| crate::finding::within(path, p)) { return (None, false); }
    let Ok(held) = sprawl_core::local::scope(path) else { return (None, false) };
    let Ok(top) = held.metadata() else { return (None, false) };
    if top.is_file() {
        *budget -= 1;
        let time = top.accessed().ok();
        return (time, time.is_some());
    }
    walk_accessed(path, budget, excluded)
}

/// The newest last-access time of the files at or below the plain folder `path` (its chain is
/// pinned by the caller), never entering a link and never counting a skipped folder.
fn walk_accessed(path: &std::path::Path, budget: &mut usize, excluded: &[PathBuf]) -> (Option<SystemTime>, bool) {
    const REPARSE: u32 = 0x400;
    use std::os::windows::fs::MetadataExt;
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

    /// Last-access times count as kept unless Windows' own two bits say off: 0 and 2 update them
    /// (0x80000002 is what Windows picks for a small system volume), 1 and 3 don't.
    #[test]
    fn access_times_count_as_kept_unless_windows_says_off() {
        for v in [0, 2, 0x8000_0000, 0x8000_0002] {
            assert!(keeps_last_access(v), "{v:#x}");
        }
        for v in [1, 3, 0x8000_0001, 0x8000_0003] {
            assert!(!keeps_last_access(v), "{v:#x}");
        }
    }

    /// Only a plain `X:\...` path is read: a relative or drive-relative one is never resolved
    /// against this process's folder.
    #[test]
    fn only_a_plain_drive_path_is_read() {
        assert_eq!(drive_of(std::path::Path::new(r"c:\Users\me\a.txt")), Some('C'));
        assert_eq!(drive_of(std::path::Path::new(r"D:\")), Some('D'));
        for p in [r"C:Users\me\a.txt", r"Users\me\a.txt", r"\\server\share\a.txt", r"\\?\C:\a.txt", r"a/b/c"] {
            assert_eq!(drive_of(std::path::Path::new(p)), None, "{p}");
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
        // the folder last read stays pinned for its siblings: let it go before the folder goes
        drop(a);
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
        let mut none = Accessed { kept: false, unseen: None, left: 10, excluded: Vec::new(), parent: None, drives: Default::default(), reasons: Vec::new(), refusal: None };
        assert_eq!(none.of(&file), LastOpened::Unknown);
        assert!(notes(0, 1, "item", "items", &none)[0].contains("last-access updates turned off"));
        // a restricted client may not see the disk: nothing is said
        assert!(notes(1, 2, "match", "matches", &Accessed::off()).is_empty());
        // a folder skipped in Settings > Scanning is not read, and says why without the limit
        let mut skipped = Accessed::with_excluded(10, vec![dir.join("skipped")]);
        std::fs::create_dir_all(dir.join("skipped")).unwrap();
        std::fs::write(dir.join("skipped").join("secret.txt"), b"x").unwrap();
        assert_eq!(skipped.of(&dir.join("skipped").join("secret.txt")), LastOpened::Unknown);
        assert_eq!(skipped.left, 10, "no read is spent on a skipped folder");
        let said = notes(0, 1, "match", "matches", &skipped);
        assert!(said[0].contains("skipped in Settings > Scanning"), "{said:?}");
        // a list that can't be read refuses every path
        let mut unreadable = Accessed::with_excluded(10, vec![PathBuf::from(sprawl_core::progress::UNREADABLE_POLICY)]);
        assert_eq!(unreadable.of(&file), LastOpened::Unknown);
        assert!(notes(0, 1, "match", "matches", &unreadable)[0].contains("couldn't be read"));
        // a part read only in part is never worded as the limit
        let partial = partial_note(3, "match", "matches").unwrap();
        assert!(partial.starts_with("3 matches not fully read") && !partial.contains("reading limit"), "{partial}");
        assert!(partial_note(0, "match", "matches").is_none());
        assert!(partial_note(1, "match", "matches").unwrap().contains("its last-opened time"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A walk the reading limit stopped is the reading limit (times-12); a part skipped or
    /// unreadable is the partial note, and never the limit (times-12); a folder that holds no
    /// files says so instead of "the time couldn't be read" (times-16).
    #[test]
    fn a_cut_walk_and_a_partial_walk_are_told_apart() {
        let dir = std::env::temp_dir().join(format!("sprawl-accessed-cut-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("skipped")).unwrap();
        for i in 0..5 {
            std::fs::write(dir.join(format!("f{i}.txt")), b"x").unwrap();
        }
        std::fs::write(dir.join("skipped").join("s.txt"), b"x").unwrap();
        // a limit smaller than the folder: Limited, counted as the reading limit, never as a "no"
        let mut cut = Accessed::with_budget(3);
        let r = cut.of(&dir);
        assert!(matches!(r, LastOpened::Limited(_)), "the limit stopped the walk: {r:?}");
        assert_eq!(in_range(r, 0, i64::MAX), InRange::Unread);
        let (mut unread, mut unknown) = (0, 0);
        let mut shown = Accessed::with_budget(3);
        let t = shown.time(&dir, &mut unread, &mut unknown);
        assert!(t.is_some_and(|(_, complete)| !complete), "the newest of what was read is shown, said as only part");
        assert_eq!((unread, unknown), (0, 0), "a shown time isn't also counted as not read");
        // the same folder read whole, with a skipped part: partial, not the limit
        let mut part = Accessed::with_excluded(10, vec![dir.join("skipped")]);
        let r = part.of(&dir);
        assert!(matches!(r, LastOpened::At(_, false)), "a skipped part is not the limit: {r:?}");
        assert_eq!(part.left, 4, "six entries read (five files and the skipped folder), the skipped folder not entered");
        assert_eq!(in_range(r, 0, i64::MAX), InRange::Partial);
        // an empty folder is not "couldn't be read"
        std::fs::create_dir_all(dir.join("empty")).unwrap();
        let mut a = Accessed::with_budget(10);
        assert_eq!(a.of(&dir.join("empty")), LastOpened::Unknown);
        let said = notes(0, 1, "folder", "folders", &a);
        assert!(said[0].contains("holds no files"), "{said:?}");
        drop(a);
        drop(cut);
        drop(shown);
        drop(part);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A `da:` range's decision keeps every state apart: only a whole read can be a yes or a no.
    #[test]
    fn a_time_that_was_not_fully_read_is_never_a_no() {
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        assert_eq!(in_range(LastOpened::At(t, true), 0, 2000), InRange::Yes);
        assert_eq!(in_range(LastOpened::At(t, true), 0, 1000), InRange::No, "the range is half-open");
        assert_eq!(in_range(LastOpened::At(t, false), 0, 2000), InRange::Partial);
        assert_eq!(in_range(LastOpened::Limited(t), 0, 2000), InRange::Unread);
        assert_eq!(in_range(LastOpened::Unread, 0, 2000), InRange::Unread);
        assert_eq!(in_range(LastOpened::Unknown, 0, 2000), InRange::Unknown);
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

    /// Items with no time are counted per reason: an empty folder holds no files, a file gone
    /// since the scan is gone, a link is refused, and none is said for another (times-17).
    #[test]
    fn each_reason_for_no_time_has_its_own_count() {
        let dir = std::env::temp_dir().join(format!("sprawl-reasons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("empty")).unwrap();
        std::fs::write(dir.join("kept.txt"), b"x").unwrap();
        // a long non-Latin name is an ordinary NTFS name: 100 characters, 300 UTF-8 bytes
        let long = format!("{}.txt", "\u{4e2d}".repeat(100));
        std::fs::write(dir.join(&long), b"x").unwrap();
        let junction = dir.join("junc");
        let linked = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(dir.join("empty"))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let mut a = Accessed::with_budget(100);
        let (mut unread, mut unknown) = (0, 0);
        assert!(a.time(&dir.join(&long), &mut unread, &mut unknown).is_some(), "a long non-Latin name is read");
        assert!(a.time(&dir.join("kept.txt"), &mut unread, &mut unknown).is_some());
        assert!(a.time(&dir.join("empty"), &mut unread, &mut unknown).is_none());
        assert!(a.time(&dir.join("gone.txt"), &mut unread, &mut unknown).is_none());
        assert!(a.time(&dir.join("no-folder").join("x.txt"), &mut unread, &mut unknown).is_none());
        let mut expected = 3;
        if linked {
            assert!(a.time(&junction, &mut unread, &mut unknown).is_none());
            expected += 1;
        }
        assert_eq!(unknown, expected);
        let lines = notes(unread, unknown, "item", "items", &a);
        let said = lines.join("\n");
        assert_eq!(lines.iter().filter(|l| l.contains("holds no files")).count(), 1, "{said}");
        assert!(lines.iter().any(|l| l.starts_with("1 item") && l.contains("holds no files")), "{said}");
        assert!(lines.iter().any(|l| l.starts_with("2 items") && l.contains("the item is gone")), "{said}");
        assert!(lines.iter().all(|l| !(l.contains("holds no files") && l.contains("gone"))), "{said}");
        drop(a);
        if linked {
            let _ = std::fs::remove_dir(&junction);
        }
        std::fs::remove_dir_all(dir).unwrap();
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
