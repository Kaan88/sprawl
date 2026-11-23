//! Registry privacy parts (F8, `docs/plans/future-features.md`): the short lists Explorer keeps
//! in this account's own registry, such as the Run box or the recent documents.
//!
//! They follow the privacy rules of [`crate::privacy`], and add what a registry write needs:
//! - **A fixed list, compiled in.** The parts are [`PARTS`]. No rule pack, community import,
//!   settings file or model can name a key: `cleaners::expand` refuses a privacy class at all,
//!   and the only code that makes an [`Action::Reg`] is this module. Every key is under HKCU and
//!   under `Software\Microsoft\Windows\CurrentVersion\Explorer`; never HKLM, never Prefetch,
//!   UserAssist, ShellBags or anything under Policies. A test checks the list against those names.
//! - **Values, never keys.** A part deletes the values of its key (and, for the two parts that
//!   keep one subkey per file type, the values of those subkeys too). The keys themselves stay.
//! - **A backup before anything goes.** Every value that will be removed (name, type, exact
//!   bytes and the key it is in) is written first to a file in Sprawl's data folder, atomically
//!   ([`crate::write_local`]). If that file can't be written, nothing is deleted and the reason
//!   is returned. The last ten backups are kept.
//! - **Put back, into those keys only.** [`restore`] writes every value back with the same type
//!   and bytes, and refuses a backup that names any other key (or a subkey of one the part
//!   doesn't keep): a backup is a file on disk someone could edit.
//! - **Off unless the user turns privacy cleaners on**, never ticked for the user, never in a
//!   space total, at least amber, and plain words that promise nothing (as [`crate::privacy`]).
//! - **No disk space.** The parts remove no files: they are measured by how many values they
//!   would remove, never in bytes.
//!
//! One honesty note is part of every part: Explorer holds some of these lists in memory, so a
//! change may only show after signing out and back in ([`SIGN_OUT_NOTE`]).
//!
//! The registry calls are declared here rather than through windows-sys's
//! `Win32_System_Registry` feature, for the same reason as in [`crate::reg`]: one more feature
//! makes windows-sys's rustc command line too long for the cmd-based rustc wrapper.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::finding::{Action, Finding, Remedy, Risk, Source};

use crate::reg::HKCU;

// Declared here for the same reason as in `reg.rs` (see the module comment).
type HKEY = *mut core::ffi::c_void;

const KEY_READ: u32 = 0x20019;
const KEY_WRITE: u32 = 0x20006;
const KEY_ALL_ACCESS: u32 = 0xF003F;
const REG_OPTION_NON_VOLATILE: u32 = 0;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_MORE_DATA: u32 = 234;
const ERROR_NO_MORE_ITEMS: u32 = 259;
/// The largest value this module will back up. Registry MRU values are far smaller; a bigger
/// one means the key is not what the list says, and then nothing is removed.
const MAX_VALUE: usize = 64 << 10;
/// The largest backup file it will read back.
const MAX_BACKUP: u64 = 4 << 20;
/// How many backups are kept.
pub const KEEP_BACKUPS: usize = 10;

/// The folder Sprawl keeps its own files in, for the backups.
pub const FOLDER: &str = "registry-backups";
const PREFIX: &str = "registry-backup-";

/// Said on every part: Explorer may not forget until the user signs out.
pub const SIGN_OUT_NOTE: &str = "Explorer keeps some of these lists in memory: the change may only show after you sign out and back in.";

/// The app that owns every part: Explorer's own lists.
pub const OWNER: &str = "Windows Explorer";

/// One fixed registry list. `key` is relative to HKCU; `subkeys` means the part also removes
/// the values of each immediate subkey (the per-file-type lists of the Open/Save dialogs and
/// the recent documents), never the subkeys themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegPart {
    /// The rule id, as `registry.explorer/runmru`.
    pub id: &'static str,
    pub title: &'static str,
    /// What the user loses, in plain words.
    pub cost: &'static str,
    /// What it is and what goes.
    pub why: &'static str,
    pub key: &'static str,
    pub subkeys: bool,
}

#[cfg(test)]
const EXPLORER: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer";
/// Every part, compiled in. In the order the user meets them.
pub const PARTS: &[RegPart] = &[
    RegPart {
        id: "registry.explorer/runmru",
        title: "Run box history",
        cost: "The Run box forgets the commands you typed.",
        why: "The Run box (Windows key + R) keeps what you typed in one list under this account's Explorer settings. Sprawl removes the list's values and leaves the key itself.",
        key: r"Software\Microsoft\Windows\CurrentVersion\Explorer\RunMRU",
        subkeys: false,
    },
    RegPart {
        id: "registry.explorer/typedpaths",
        title: "Typed paths in the address bar",
        cost: "Explorer's address bar forgets the paths you typed.",
        why: "When you type a folder path into Explorer's address bar, it keeps the path in a list under this account's Explorer settings. Sprawl removes the list's values and leaves the key itself.",
        key: r"Software\Microsoft\Windows\CurrentVersion\Explorer\TypedPaths",
        subkeys: false,
    },
    RegPart {
        id: "registry.explorer/wordwheel",
        title: "Explorer search box history",
        cost: "Explorer's search box forgets what you searched for.",
        why: "Explorer's search box keeps the words you searched for under this account's Explorer settings. Sprawl removes the list's values and leaves the key itself.",
        key: r"Software\Microsoft\Windows\CurrentVersion\Explorer\WordWheelQuery",
        subkeys: false,
    },
    RegPart {
        id: "registry.explorer/opensave",
        title: "Open and Save dialog recent files",
        cost: "Open and Save dialogs forget their list of recent files.",
        why: "When you open or save a file, Windows keeps the recent choices in a list with one subkey per file type. Sprawl removes the values in that key and in those subkeys, and leaves every key itself.",
        key: r"Software\Microsoft\Windows\CurrentVersion\Explorer\ComDlg32\OpenSavePidlMRU",
        subkeys: true,
    },
    RegPart {
        id: "registry.explorer/lastvisited",
        title: "Open and Save dialog recent folders",
        cost: "Open and Save dialogs forget the folders you used last.",
        why: "When you open or save a file, Windows remembers the folder you were in, in a list under this account's Explorer settings. Sprawl removes the list's values and leaves the key itself.",
        key: r"Software\Microsoft\Windows\CurrentVersion\Explorer\ComDlg32\LastVisitedPidlMRU",
        subkeys: false,
    },
    RegPart {
        id: "registry.explorer/recentdocs",
        title: "Recent documents",
        cost: "Explorer forgets your list of recent documents.",
        why: "Explorer's recent documents list keeps the documents you opened in a list with one subkey per file type. Sprawl removes the values in that key and in those subkeys, and leaves every key itself. The documents stay on disk.",
        key: r"Software\Microsoft\Windows\CurrentVersion\Explorer\RecentDocs",
        subkeys: true,
    },
];

/// The part whose rule id is `id`.
pub fn part(id: &str) -> Option<&'static RegPart> {
    PARTS.iter().find(|p| p.id == id)
}

/// A rule id that is a registry privacy part's.
pub fn is_id(id: &str) -> bool {
    part(id).is_some()
}

/// Where the backups live: `%LOCALAPPDATA%\Sprawl\registry-backups`, beside the clean-up log.
pub fn backup_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Sprawl").join(FOLDER))
}

/// How many values `id` would remove now.
pub fn count(id: &str) -> usize {
    part(id).map_or(0, |p| read_all(p, "").map_or(0, |v| v.len()))
}

/// The registry parts that have something to remove now, as findings (not by the scan: a
/// registry value is not in the tree). Only with privacy cleaners on, as their caller decides.
pub fn find() -> Vec<Finding> {
    find_in("")
}

/// [`find`] with the fixed keys taken below `root` under HKCU (empty for this account's own
/// registry; a test key otherwise).
pub(crate) fn find_in(root: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for p in PARTS {
        let Ok(values) = read_all(p, root) else { continue };
        if values.is_empty() {
            continue;
        }
        let n = values.len();
        let mut f = Finding {
            // not a file: this names the list, and keeps each part's tick apart
            path: PathBuf::from(format!(r"HKCU\{}", p.key)),
            owner: Some(OWNER.into()),
            bytes: 0,
            risk: Risk::Amber,
            remedy: Remedy::Delete,
            command: None,
            title: format!("{OWNER}: {}", p.title),
            evidence: vec![
                p.cost.to_string(),
                p.why.to_string(),
                format!("{n} {} would be removed. Each value is written to a backup first, and can be put back after the clean-up.", if n == 1 { "value" } else { "values" }),
                SIGN_OUT_NOTE.to_string(),
                crate::privacy::DELETED_NOTE.to_string(),
            ],
            confidence: 1.0,
            source: Source::Rule(p.id.into()),
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
        // the action was made here, so the command is none: `set_action` says the same
        f.set_action(Action::Reg { part: p.id.into() }, false);
        out.push(f);
    }
    out
}

/// What a registry clean-up did, for the done row's "Put back".
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub part: String,
    pub title: String,
    pub removed: usize,
    pub backup: PathBuf,
}

/// Remove `id`'s values now: read them all, write the backup, then delete each value. Nothing is
/// deleted when the backup can't be written. Returns what went (None when nothing did) and the
/// problems, in plain words.
pub fn remove(id: &str) -> (Option<Done>, Vec<String>) {
    let Some(p) = part(id) else {
        return (None, vec!["not a registry part Sprawl knows".into()]);
    };
    let Some(dir) = backup_dir() else {
        return (None, vec!["nothing was removed: Sprawl's data folder couldn't be found".into()]);
    };
    remove_in(p, "", &dir)
}

pub(crate) fn remove_in(part: &RegPart, root: &str, dir: &Path) -> (Option<Done>, Vec<String>) {
    let values = match read_all(part, root) {
        Ok(v) => v,
        Err(e) => return (None, vec![format!("nothing was removed: {e}")]),
    };
    if values.is_empty() {
        return (None, Vec::new());
    }
    let backup = match write_backup(part, &values, dir) {
        Ok(p) => p,
        Err(e) => return (None, vec![format!("nothing was removed: the backup couldn't be written ({e})")]),
    };
    let mut problems = Vec::new();
    let mut removed = 0;
    for (key, name, _, _) in &values {
        match delete_value(root, key, name) {
            Ok(true) => removed += 1,
            Ok(false) => {}
            Err(e) => problems.push(format!("{key}: {e}")),
        }
    }
    if removed == 0 {
        problems.push("no value could be removed; the values are still there and the backup is kept".into());
    }
    let done = (removed > 0).then(|| Done {
        part: part.id.into(),
        title: part.title.into(),
        removed,
        backup,
    });
    (done, problems)
}

/// Put every value of a backup back, with the same type and bytes. Refuses a backup that names a
/// key outside the fixed list (it is a file someone could edit), or a type Sprawl doesn't write.
pub fn restore(path: &Path) -> Result<usize, String> {
    restore_in(path, "")
}

pub(crate) fn restore_in(path: &Path, root: &str) -> Result<usize, String> {
    let backup = read_backup(path)?;
    let part = part(&backup.part).ok_or_else(|| format!("\"{}\" is not a registry part Sprawl knows", safe(&backup.part)))?;
    if backup.version != 1 {
        return Err(format!("this backup is from another version of Sprawl ({})", backup.version));
    }
    if backup.values.len() > 10_000 {
        return Err("this backup names too many values".into());
    }
    // check every key and every value before writing anything
    let mut checked: Vec<(String, String, u32, Vec<u8>)> = Vec::new();
    for v in &backup.values {
        if !key_allowed(part, &v.key) {
            return Err(format!("\"{}\" is not one of the fixed keys for {}, so nothing was put back", safe(&v.key), part.title));
        }
        if !allowed_kind(v.kind) {
            return Err(format!("\"{}\" has a type Sprawl doesn't put back ({}), so nothing was put back", safe(&v.name), v.kind));
        }
        if v.name.len() > 16_383 || v.name.chars().any(char::is_control) {
            return Err("this backup has a value name Sprawl doesn't write".into());
        }
        let data = unhex(&v.data).map_err(|_| format!("\"{}\" in \"{}\" is not valid data, so nothing was put back", safe(&v.name), safe(&v.key)))?;
        if data.len() > MAX_VALUE {
            return Err(format!("\"{}\" in \"{}\" is too large to put back, so nothing was put back", safe(&v.name), safe(&v.key)));
        }
        checked.push((v.key.clone(), v.name.clone(), v.kind, data));
    }
    let mut written = 0;
    for (key, value_name, kind, data) in &checked {
        let key = join(root, key);
        let h = create(&key).map_err(|e| format!("{}: {e}; {} value(s) were put back before it", safe(&key), written))?;
        let name = wide(value_name);
        let r = unsafe { RegSetValueExW(h.0, name.as_ptr(), 0, *kind, data.as_ptr(), data.len() as u32) };
        if r != 0 {
            return Err(format!("{}: Windows couldn't write \"{}\" ({}); {} value(s) were put back before it", safe(&key), safe(value_name), r, written));
        }
        written += 1;
    }
    Ok(written)
}

/// A value read from the registry: its key relative to HKCU, its name, its type and bytes.
type Value = (String, String, u32, Vec<u8>);

/// Every value of `part` now, including those in its immediate subkeys when it has them.
fn read_all(part: &RegPart, root: &str) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let key = part.key.to_string();
    for (name, kind, data) in read_values(root, &key)? {
        out.push((key.clone(), name, kind, data));
    }
    if part.subkeys {
        for sub in read_subkeys(root, &key)? {
            if !plain_name(&sub) {
                continue;
            }
            let child = format!(r"{key}\{sub}");
            for (name, kind, data) in read_values(root, &child)? {
                out.push((child.clone(), name, kind, data));
            }
        }
    }
    Ok(out)
}

fn read_values(root: &str, key: &str) -> Result<Vec<(String, u32, Vec<u8>)>, String> {
    let path = join(root, key);
    let Some(h) = open(&path, KEY_READ) else {
        // a key that isn't there has no values; any other refusal is an error worth saying
        return match last_error() {
            ERROR_FILE_NOT_FOUND => Ok(Vec::new()),
            e => Err(format!("{} couldn't be opened ({e})", safe(key))),
        };
    };
    let mut out = Vec::new();
    let mut name = vec![0u16; 16_384];
    let mut data = vec![0u8; MAX_VALUE];
    for i in 0..100_000u32 {
        let mut nlen = name.len() as u32;
        let mut dlen = data.len() as u32;
        let mut kind = 0u32;
        let r = unsafe { RegEnumValueW(h.0, i, name.as_mut_ptr(), &mut nlen, std::ptr::null(), &mut kind, data.as_mut_ptr(), &mut dlen) };
        match r {
            0 => {
                // a value whose type Sprawl can't write back would break "Put back": hold the
                // whole part, before anything is removed
                if !allowed_kind(kind) {
                    return Err(format!("a value in {} has a type Sprawl can't put back, so nothing was removed", safe(key)));
                }
                out.push((String::from_utf16_lossy(&name[..nlen as usize]), kind, data[..dlen as usize].to_vec()));
            }
            ERROR_MORE_DATA => return Err(format!("a value in {} is too large to back up, so nothing was removed", safe(key))),
            ERROR_NO_MORE_ITEMS => break,
            e => return Err(format!("{} couldn't be read completely ({e})", safe(key))),
        }
    }
    Ok(out)
}

fn read_subkeys(root: &str, key: &str) -> Result<Vec<String>, String> {
    let path = join(root, key);
    let Some(h) = open(&path, KEY_READ) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    let mut name = vec![0u16; 512];
    for i in 0..100_000u32 {
        let mut nlen = name.len() as u32;
        let r = unsafe { RegEnumKeyExW(h.0, i, name.as_mut_ptr(), &mut nlen, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
        match r {
            0 => out.push(String::from_utf16_lossy(&name[..nlen as usize])),
            ERROR_MORE_DATA => continue,
            _ => break,
        }
    }
    Ok(out)
}

fn delete_value(root: &str, key: &str, name: &str) -> Result<bool, String> {
    let path = join(root, key);
    let Some(h) = open(&path, KEY_WRITE) else {
        return match last_error() {
            ERROR_FILE_NOT_FOUND => Ok(false),
            e => Err(format!("couldn't be opened ({e})")),
        };
    };
    let n = wide(name);
    match unsafe { RegDeleteValueW(h.0, n.as_ptr()) } {
        0 => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        e => Err(format!("Windows didn't remove it ({e})")),
    }
}

#[cfg(test)]
fn key_exists(root: &str, key: &str) -> bool {
    open(&join(root, key), KEY_READ).is_some()
}

fn open(path: &str, sam: u32) -> Option<Handle> {
    let w = wide(path);
    let mut h: HKEY = std::ptr::null_mut();
    let r = unsafe { RegOpenKeyExW(HKCU, w.as_ptr(), 0, sam, &mut h) };
    (r == 0).then_some(Handle(h))
}

fn create(path: &str) -> Result<Handle, String> {
    let w = wide(path);
    let mut h: HKEY = std::ptr::null_mut();
    let mut disposition = 0u32;
    let r = unsafe { RegCreateKeyExW(HKCU, w.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_ALL_ACCESS, std::ptr::null(), &mut h, &mut disposition) };
    if r == 0 {
        Ok(Handle(h))
    } else {
        Err(format!("Windows couldn't open it ({r})"))
    }
}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

struct Handle(HKEY);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

/// A backup file: one part, and every value that goes, with its exact type and bytes.
#[derive(Serialize, Deserialize)]
struct Backup {
    version: u32,
    part: String,
    when: u64,
    values: Vec<BackupValue>,
}

#[derive(Serialize, Deserialize)]
struct BackupValue {
    /// The fixed key, or (for the two parts with subkeys) one of its immediate subkeys.
    key: String,
    name: String,
    kind: u32,
    /// The bytes, in hex.
    data: String,
}

fn write_backup(part: &RegPart, values: &[Value], dir: &Path) -> Result<PathBuf, String> {
    let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let backup = Backup {
        version: 1,
        part: part.id.into(),
        when,
        values: values
            .iter()
            .map(|(key, name, kind, data)| BackupValue { key: key.clone(), name: name.clone(), kind: *kind, data: hex(data) })
            .collect(),
    };
    let bytes = serde_json::to_vec(&backup).map_err(|e| e.to_string())?;
    let name = format!("{PREFIX}{when:010}-{}.json", crate::store::nonce());
    let path = dir.join(name);
    crate::write_local(&path, &bytes, false).map_err(|e| format!("{}: {e}", path.display()))?;
    prune(dir);
    Ok(path)
}

fn read_backup(path: &Path) -> Result<Backup, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() || meta.len() > MAX_BACKUP {
        return Err(format!("{} is not a backup file Sprawl wrote", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{} is not a backup file Sprawl can read ({e})", path.display()))
}

/// Keep the last [`KEEP_BACKUPS`] backups; the oldest go first.
fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(PREFIX) && n.ends_with(".json")))
        .collect();
    if files.len() <= KEEP_BACKUPS {
        return;
    }
    files.sort();
    let extra = files.len() - KEEP_BACKUPS;
    for p in files.into_iter().take(extra) {
        let _ = crate::remove_local(&p);
    }
}

/// Whether `key` is one of `part`'s fixed keys: the key itself, or (when the part keeps one
/// subkey per file type) exactly one plain child of it.
fn key_allowed(part: &RegPart, key: &str) -> bool {
    let key = key.trim_matches('\\');
    if key.eq_ignore_ascii_case(part.key) {
        return true;
    }
    if !part.subkeys {
        return false;
    }
    let parent = part.key.as_bytes();
    let b = key.as_bytes();
    if b.len() > parent.len() + 1 && b[..parent.len()].eq_ignore_ascii_case(parent) && b[parent.len()] == b'\\' {
        let child = &key[parent.len() + 1..];
        return plain_name(child);
    }
    false
}

/// A registry key or value name Sprawl will write under a fixed key: not empty, not `.` or `..`,
/// no separator or control character, and not absurdly long.
fn plain_name(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.len() <= 255 && !s.chars().any(|c| c.is_control() || matches!(c, '\\' | '/' | ':'))
}

/// The value types Sprawl puts back: the ones Windows' own lists use, never a link or something
/// whose meaning depends on a program.
fn allowed_kind(kind: u32) -> bool {
    matches!(kind, 1 | 2 | 3 | 4 | 7 | 11)
}

fn join(root: &str, key: &str) -> String {
    if root.is_empty() {
        key.to_string()
    } else {
        format!(r"{}\{key}", root.trim_end_matches('\\'))
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn hex(data: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 15) as usize] as char);
    }
    s
}

fn unhex(s: &str) -> Result<Vec<u8>, ()> {
    if s.len() % 2 != 0 {
        return Err(());
    }
    let digit = |c: u8| -> Result<u8, ()> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err(()),
        }
    };
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    for pair in b.chunks(2) {
        out.push((digit(pair[0])? << 4) | digit(pair[1])?);
    }
    Ok(out)
}

/// A name from a file, for a message: one line, short, no control characters.
fn safe(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(200).collect()
}

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(hkey: HKEY, subkey: *const u16, options: u32, sam: u32, result: *mut HKEY) -> u32;
    fn RegCreateKeyExW(hkey: HKEY, subkey: *const u16, reserved: u32, class: *const u16, options: u32, sam: u32, security: *const core::ffi::c_void, result: *mut HKEY, disposition: *mut u32) -> u32;
    fn RegCloseKey(hkey: HKEY) -> u32;
    fn RegEnumKeyExW(hkey: HKEY, index: u32, name: *mut u16, name_len: *mut u32, reserved: *const u32, class: *mut u16, class_len: *mut u32, written: *mut u64) -> u32;
    fn RegEnumValueW(hkey: HKEY, index: u32, name: *mut u16, name_len: *mut u32, reserved: *const u32, kind: *mut u32, data: *mut u8, data_len: *mut u32) -> u32;
    fn RegDeleteValueW(hkey: HKEY, name: *const u16) -> u32;
    fn RegSetValueExW(hkey: HKEY, name: *const u16, reserved: u32, kind: u32, data: *const u8, len: u32) -> u32;
}

#[cfg(test)]
#[link(name = "advapi32")]
extern "system" {
    fn RegDeleteKeyW(hkey: HKEY, subkey: *const u16) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLastError() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key under HKCU\Software\SprawlTest-<random> for one test, removed when it goes out of
    /// scope, also when the test fails part way.
    struct TestKey(String);

    impl Drop for TestKey {
        fn drop(&mut self) {
            delete_tree(&self.0);
        }
    }

    fn test_root() -> String {
        format!(r"Software\SprawlTest-{}", crate::store::nonce())
    }

    /// A folder for one test's backups, removed when it goes out of scope.
    struct TestDir(PathBuf);

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn test_dir() -> TestDir {
        let d = std::env::temp_dir().join(format!("sprawl-regbackup-{}", crate::store::nonce()));
        std::fs::create_dir_all(&d).unwrap();
        TestDir(d)
    }

    fn delete_tree(path: &str) {
        let Some(h) = open(path, KEY_ALL_ACCESS) else { return };
        let mut name = vec![0u16; 512];
        let mut subs = Vec::new();
        for i in 0..100_000u32 {
            let mut nlen = name.len() as u32;
            let r = unsafe { RegEnumKeyExW(h.0, i, name.as_mut_ptr(), &mut nlen, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
            if r != 0 {
                break;
            }
            subs.push(String::from_utf16_lossy(&name[..nlen as usize]));
        }
        drop(h);
        for s in subs {
            delete_tree(&format!(r"{path}\{s}"));
        }
        let w = wide(path);
        unsafe {
            RegDeleteKeyW(HKCU, w.as_ptr());
        }
    }

    fn set_in(root: &str, key: &str, name: &str, kind: u32, data: &[u8]) {
        let h = create(&join(root, key)).unwrap();
        let n = wide(name);
        let r = unsafe { RegSetValueExW(h.0, n.as_ptr(), 0, kind, data.as_ptr(), data.len() as u32) };
        assert_eq!(r, 0, "couldn't write a test value");
    }

    fn values_of(root: &str, key: &str) -> Vec<(String, u32, Vec<u8>)> {
        read_values(root, key).unwrap()
    }

    /// Every value name in `key`, whatever its type (a test of a type read_values refuses).
    fn raw_names(root: &str, key: &str) -> Vec<String> {
        let h = open(&join(root, key), KEY_READ).unwrap();
        let mut out = Vec::new();
        let mut name = vec![0u16; 16_384];
        let mut data = vec![0u8; 1024];
        for i in 0..1000u32 {
            let mut nlen = name.len() as u32;
            let mut dlen = data.len() as u32;
            let mut kind = 0u32;
            let r = unsafe { RegEnumValueW(h.0, i, name.as_mut_ptr(), &mut nlen, std::ptr::null(), &mut kind, data.as_mut_ptr(), &mut dlen) };
            if r != 0 {
                break;
            }
            out.push(String::from_utf16_lossy(&name[..nlen as usize]));
        }
        out
    }

    fn one(values: &[(String, u32, Vec<u8>)], name: &str) -> (u32, Vec<u8>) {
        let v = values.iter().find(|(n, _, _)| n == name).unwrap_or_else(|| panic!("{name} is not there: {values:?}"));
        (v.1, v.2.clone())
    }

    const SZ: u32 = 1;
    const BINARY: u32 = 3;
    const DWORD: u32 = 4;
    const EXPAND: u32 = 2;

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|c| c.to_le_bytes()).chain([0, 0]).collect()
    }

    /// A backup goes into Sprawl's folder before a value does, and holds every value's exact
    /// type and bytes; the key itself stays.
    #[test]
    fn the_backup_is_written_before_anything_is_removed() {
        let root = test_root();
        let _key = TestKey(root.clone());
        let dir = test_dir();
        let part = part("registry.explorer/runmru").unwrap();
        set_in(&root, part.key, "a", SZ, &utf16("notepad"));
        set_in(&root, part.key, "MRUListEx", BINARY, &[0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(read_all(part, &root).unwrap().len(), 2);

        let (done, problems) = remove_in(part, &root, &dir.0);
        assert!(problems.is_empty(), "{problems:?}");
        let done = done.unwrap();
        assert_eq!((done.removed, done.part.as_str()), (2, part.id));
        assert!(done.backup.starts_with(&dir.0) && done.backup.exists());
        // the backup holds both values, under the fixed key, as the exact bytes
        let backup = read_backup(&done.backup).unwrap();
        assert_eq!((backup.version, backup.part.as_str(), backup.values.len()), (1, part.id, 2));
        for v in &backup.values {
            assert_eq!(v.key, part.key);
            let original = if v.name == "a" { (SZ, utf16("notepad")) } else { (BINARY, vec![0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]) };
            assert_eq!((v.kind, unhex(&v.data).unwrap()), original);
        }
        // every value is gone, the key is not
        assert!(read_all(part, &root).unwrap().is_empty());
        assert!(key_exists(&root, part.key));
    }

    /// A backup that can't be written means nothing is deleted, with the reason.
    #[test]
    fn nothing_is_removed_when_the_backup_cannot_be_written() {
        let root = test_root();
        let _key = TestKey(root.clone());
        let dir = test_dir();
        let part = part("registry.explorer/typedpaths").unwrap();
        set_in(&root, part.key, "url1", SZ, &utf16(r"C:\Users\sam\Documents"));
        // a plain file where the backup folder should be
        let blocked = dir.0.join("blocked");
        std::fs::write(&blocked, b"not a folder").unwrap();
        let (done, problems) = remove_in(part, &root, &blocked);
        assert!(done.is_none());
        assert!(problems.iter().any(|p| p.contains("backup") && p.contains("nothing was removed")), "{problems:?}");
        let left = read_all(part, &root).unwrap();
        assert_eq!(left.len(), 1, "the value must still be there");
        assert!(key_exists(&root, part.key));
    }

    /// A backup puts back the same types and bytes, the per-file-type subkeys included, and
    /// creates any subkey that is not there any more.
    #[test]
    fn restore_puts_back_the_same_type_and_bytes() {
        let root = test_root();
        let _key = TestKey(root.clone());
        let dir = test_dir();
        let part = part("registry.explorer/recentdocs").unwrap();
        set_in(&root, part.key, "MRUListEx", BINARY, &[1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
        set_in(&root, part.key, "0", SZ, &utf16("report.docx"));
        set_in(&root, &format!(r"{}\docx", part.key), "0", SZ, &utf16("report.docx"));
        set_in(&root, &format!(r"{}\docx", part.key), "MRUListEx", BINARY, &[0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
        set_in(&root, &format!(r"{}\Folder", part.key), "0", SZ, &utf16(r"C:\Users\sam\Documents"));
        set_in(&root, part.key, "flag", DWORD, &7u32.to_le_bytes());
        set_in(&root, part.key, "old", EXPAND, &utf16(r"%USERPROFILE%\Documents"));
        let before = values_of(&root, part.key);
        assert_eq!(read_all(part, &root).unwrap().len(), 7);
        assert_eq!(count_in(part, &root), 7);

        let (done, problems) = remove_in(part, &root, &dir.0);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(done.unwrap().removed, 7);
        assert!(read_all(part, &root).unwrap().is_empty());
        // parent keys, subkeys included, stay
        assert!(key_exists(&root, part.key));
        assert!(key_exists(&root, &format!(r"{}\docx", part.key)));
        assert!(key_exists(&root, &format!(r"{}\Folder", part.key)));

        let backup = read_backup(&dir.0.read_dir().unwrap().next().unwrap().unwrap().path()).unwrap();
        let path = dir.0.join("reuse.json");
        crate::write_local(&path, &serde_json::to_vec(&backup).unwrap(), false).unwrap();
        assert_eq!(restore_in(&path, &root).unwrap(), 7);
        let after = values_of(&root, part.key);
        for name in ["MRUListEx", "0", "flag", "old"] {
            assert_eq!(one(&after, name), one(&before, name), "{name}");
        }
        let sub = values_of(&root, &format!(r"{}\docx", part.key));
        assert_eq!(one(&sub, "0"), (SZ, utf16("report.docx")));
        assert_eq!(one(&sub, "MRUListEx"), (BINARY, vec![0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]));
        let folder = values_of(&root, &format!(r"{}\Folder", part.key));
        assert_eq!(one(&folder, "0"), (SZ, utf16(r"C:\Users\sam\Documents")));
    }

    /// A value of a type Sprawl can't put back holds the whole part: no backup is written and
    /// nothing is removed, so "Put back" can always be exact.
    #[test]
    fn a_value_type_sprawl_cannot_put_back_holds_the_whole_part() {
        let root = test_root();
        let _key = TestKey(root.clone());
        let dir = test_dir();
        let part = part("registry.explorer/runmru").unwrap();
        set_in(&root, part.key, "a", SZ, &utf16("notepad"));
        set_in(&root, part.key, "none", 0, &[]);
        let (done, problems) = remove_in(part, &root, &dir.0);
        assert!(done.is_none());
        assert!(problems.iter().any(|p| p.contains("can't put back") && p.contains("nothing was removed")), "{problems:?}");
        let left = read_all(part, &root).unwrap_err();
        assert!(left.contains("can't put back"), "{left}");
        let raw = raw_names(&root, part.key);
        assert_eq!(raw.len(), 2, "both values are still there: {raw:?}");
        assert_eq!(dir.0.read_dir().unwrap().count(), 0, "no backup was written");
    }

    /// A backup that names a key outside the fixed list is refused whole, before anything is
    /// written; so is a subkey under a part that keeps none.
    #[test]
    fn restore_refuses_a_key_outside_the_list() {
        let root = test_root();
        let _key = TestKey(root.clone());
        let dir = test_dir();
        let part = part("registry.explorer/runmru").unwrap();
        set_in(&root, part.key, "a", SZ, &utf16("notepad"));
        let (_, problems) = remove_in(part, &root, &dir.0);
        assert!(problems.is_empty(), "{problems:?}");
        let path = dir.0.read_dir().unwrap().next().unwrap().unwrap().path();
        let bytes = std::fs::read(&path).unwrap();
        let mut backup: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        backup["values"][0]["key"] = serde_json::json!(r"Software\Microsoft\Windows\CurrentVersion\Run");
        let bad = dir.0.join("bad.json");
        crate::write_local(&bad, &serde_json::to_vec(&backup).unwrap(), false).unwrap();
        let err = restore_in(&bad, &root).unwrap_err();
        assert!(err.contains("not one of the fixed keys") && err.contains("nothing was put back"), "{err}");
        // the fixed key is still empty: the refused file wrote nothing anywhere
        assert!(read_all(part, &root).unwrap().is_empty());

        // a subkey under a part that has none
        let mut backup: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        backup["values"][0]["key"] = serde_json::json!(format!(r"{}\sub", part.key));
        let bad = dir.0.join("bad2.json");
        crate::write_local(&bad, &serde_json::to_vec(&backup).unwrap(), false).unwrap();
        let err = restore_in(&bad, &root).unwrap_err();
        assert!(err.contains("not one of the fixed keys"), "{err}");
        assert!(read_all(part, &root).unwrap().is_empty());

        // a part Sprawl doesn't know
        let mut backup: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        backup["part"] = serde_json::json!("registry.explorer/passwords");
        let bad = dir.0.join("bad3.json");
        crate::write_local(&bad, &serde_json::to_vec(&backup).unwrap(), false).unwrap();
        assert!(restore_in(&bad, &root).is_err());
    }

    /// The compiled list: the six required lists, HKCU Explorer only, and none of the names that
    /// are never cleaned.
    #[test]
    fn the_list_holds_only_allowed_keys() {
        assert_eq!(PARTS.len(), 6);
        let wanted = [
            "RunMRU",
            "TypedPaths",
            "WordWheelQuery",
            r"ComDlg32\OpenSavePidlMRU",
            r"ComDlg32\LastVisitedPidlMRU",
            "RecentDocs",
        ];
        let forbidden = ["prefetch", "userassist", "shellbags", "policies", "run", "runonce", "startup", "shell folders"];
        for w in wanted {
            assert!(PARTS.iter().any(|p| p.key.ends_with(w)), "{w} is missing");
        }
        for p in PARTS {
            assert!(p.key.starts_with(EXPLORER), "{}: {}", p.id, p.key);
            assert!(!p.key.to_lowercase().contains("hkey") && !p.key.to_lowercase().contains("hklm"), "{}", p.key);
            let segs: Vec<String> = p.key.split('\\').map(str::to_lowercase).collect();
            for f in forbidden {
                assert!(!segs.iter().any(|s| s == f), "{} names {f}", p.id);
            }
            assert!(is_id(p.id) && part(p.id).is_some());
            assert!(!p.title.is_empty() && !p.cost.is_empty() && p.cost.len() < 240, "{}", p.id);
            assert!(p.why.contains("leaves"), "{}: the words say the key stays", p.id);
        }
        assert_eq!(PARTS.iter().filter(|p| p.subkeys).map(|p| p.id).collect::<Vec<_>>(), ["registry.explorer/opensave", "registry.explorer/recentdocs"]);
        // no two parts share an id or a key
        for (i, a) in PARTS.iter().enumerate() {
            for b in &PARTS[i + 1..] {
                assert_ne!(a.id, b.id);
                assert_ne!(a.key.to_lowercase(), b.key.to_lowercase());
            }
        }
    }

    /// The words promise nothing: the privacy module's own list, for every string a part, a
    /// finding or a backup message shows.
    #[test]
    fn the_words_promise_nothing() {
        let root = test_root();
        let _key = TestKey(root.clone());
        let part = part("registry.explorer/wordwheel").unwrap();
        set_in(&root, part.key, "0", SZ, &utf16("invoice"));
        let found = find_in(&root);
        assert_eq!(found.len(), 1);
        let mut texts = vec![SIGN_OUT_NOTE.to_string(), crate::privacy::DELETED_NOTE.to_string()];
        for p in PARTS {
            texts.extend([p.title.to_string(), p.cost.to_string(), p.why.to_string()]);
        }
        texts.extend(found[0].evidence.clone());
        texts.push(found[0].title.clone());
        for t in &texts {
            let low = t.to_lowercase();
            for w in crate::privacy::PROMISE_WORDS {
                assert!(!low.contains(w), "{w:?} in {t:?}");
            }
        }
        // and each part says what is lost, in one line
        for p in PARTS {
            assert!(p.cost.ends_with('.'), "{}", p.cost);
        }
    }

    /// Only Sprawl's own finding, for a part in the list, is accepted by the executor; a model
    /// or a made-up part is not. Off, nothing is offered.
    #[test]
    fn only_sprawls_own_registry_findings_are_accepted() {
        let root = test_root();
        let _key = TestKey(root.clone());
        let part = part("registry.explorer/runmru").unwrap();
        assert!(find_in(&root).is_empty(), "nothing to offer with no values");
        set_in(&root, part.key, "a", SZ, &utf16("notepad"));
        let found = find_in(&root);
        assert_eq!(found.len(), 1);
        let f = &found[0];
        assert!(f.privacy() && f.risk == Risk::Amber && f.bytes == 0 && f.command.is_none());
        assert!(crate::clean::action(f).is_some());
        let mut forged = f.clone();
        forged.action = Some(Action::Reg { part: "registry.explorer/passwords".into() });
        assert!(crate::clean::action(&forged).is_none(), "a part not in the list");
        let mut forged = f.clone();
        forged.action = Some(Action::Reg { part: part.id.into() });
        forged.builtin = false;
        assert!(crate::clean::action(&forged).is_none(), "not built in");
        let mut forged = f.clone();
        forged.action = Some(Action::Reg { part: part.id.into() });
        forged.command = Some("Remove-Item x".into());
        assert!(crate::clean::action(&forged).is_none(), "with a command");
    }

    fn count_in(part: &RegPart, root: &str) -> usize {
        read_all(part, root).map_or(0, |v| v.len())
    }
}
