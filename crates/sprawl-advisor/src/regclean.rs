//! Registry privacy parts (F8, `docs/plans/future-features.md`): the short lists Explorer keeps
//! in this account's own registry, such as the Run box or the recent documents.
//!
//! They follow the privacy rules of [`crate::privacy`], and add what a registry write needs:
//! - **A fixed list, compiled in.** The parts are [`PARTS`]. No rule pack, community import,
//!   settings file or model can name a key: `cleaners::expand` refuses a privacy class at all,
//!   `RulePack::parse` keeps the `registry.` and `privacy.` prefixes for Sprawl, and the only
//!   code that makes an [`Action::Reg`] is this module. Every key is under HKCU and under
//!   `Software\Microsoft\Windows\CurrentVersion\Explorer`; never HKLM, never Prefetch,
//!   UserAssist, ShellBags or anything under Policies. A test checks the list against those
//!   names.
//! - **Values, never keys.** A part deletes the values of its key (and, for the two parts that
//!   keep one subkey per file type, the values of those subkeys too). The keys themselves stay.
//! - **Never through a link.** Every key is opened with `REG_OPTION_OPEN_LINK`, so a symbolic
//!   link key (a `SymbolicLinkValue` of type `REG_LINK`) is seen as the link itself, held, and
//!   never followed to another key (Sprawl may run elevated).
//! - **A backup before anything goes.** Every value that will be removed (name, type, exact
//!   bytes and the key it is in) is written first to a file in Sprawl's data folder, atomically
//!   ([`crate::write_local`]), and read back and compared. If that fails, nothing is deleted and
//!   the reason is returned. The last ten backups are kept, none younger than ten minutes; the
//!   backup just written is never pruned.
//! - **Put back, into those keys only.** [`restore`] writes the removed values back with the
//!   same type and bytes, and refuses a backup that names any other key (or a subkey of one the
//!   part doesn't keep): a backup is a file on disk someone could edit. It adds or overwrites
//!   the values it holds; values written since it was made stay.
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
const KEY_SET_VALUE: u32 = 0x0002;
/// Read a value and delete or write one.
const KEY_EDIT: u32 = KEY_READ | KEY_SET_VALUE;
#[cfg(test)]
const KEY_ALL_ACCESS: u32 = 0xF003F;
const REG_OPTION_NON_VOLATILE: u32 = 0;
/// Open a symbolic link key as the link itself, never its target.
const REG_OPTION_OPEN_LINK: u32 = 0x0008;
#[cfg(test)]
const REG_OPTION_CREATE_LINK: u32 = 0x0002;
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
/// A backup is not pruned until it is this old (seconds), so a clock that went back or forward
/// cannot make the newest backups go by themselves.
const MIN_AGE: u64 = 600;
/// The most subkeys a restore may make.
const MAX_CREATED: usize = 500;
/// The value Windows uses to store a symbolic link key's target.
const LINK_VALUE: &str = "SymbolicLinkValue";
const REG_LINK_KIND: u32 = 6;

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

/// How many values each part would remove now, by rule id (the ones with nothing are left out).
pub fn counts() -> std::collections::HashMap<String, usize> {
    counts_in("")
}

pub(crate) fn counts_in(root: &str) -> std::collections::HashMap<String, usize> {
    let mut out = std::collections::HashMap::new();
    for p in PARTS {
        if let Ok(values) = read_all(p, root) {
            if !values.is_empty() {
                out.insert(p.id.to_string(), values.len());
            }
        }
    }
    out
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

/// One backup on disk, as Settings lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub part: String,
    /// The part's title, or its id when this build doesn't know it.
    pub title: String,
    /// When it was written, as seconds since 1970 (local time is the app's to show).
    pub when: u64,
    pub values: usize,
}

/// The backups kept, newest first. A file that isn't one of Sprawl's backups is left out.
pub fn backups() -> Vec<BackupInfo> {
    match backup_dir() {
        Some(d) => backups_in(&d),
        None => Vec::new(),
    }
}

pub(crate) fn backups_in(dir: &Path) -> Vec<BackupInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<BackupInfo> = entries
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| backup_stamp(n).is_some()))
        .filter_map(|e| {
            let path = e.path();
            let b = read_backup(&path).ok()?;
            let p = part(&b.part)?;
            Some(BackupInfo { path, part: b.part, title: p.title.to_string(), when: b.when, values: b.values.len() })
        })
        .collect();
    out.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| b.path.cmp(&a.path)));
    out
}

/// Remove `id`'s values now: read them all, write the backup and check it, then delete each
/// value that is still exactly what was backed up. Nothing is deleted when any of that fails.
/// Returns what went (None when nothing did) and the problems, in plain words.
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
    // the file must read back exactly as written before a value goes: if it can't, Put back
    // could not be trusted, so nothing is removed
    match read_backup(&backup) {
        Ok(b) if backup_matches(&b, part, &values) => {}
        Ok(_) => return (None, vec![format!("nothing was removed: the backup at {} didn't read back as written", backup.display())]),
        Err(e) => return (None, vec![format!("nothing was removed: the backup at {} couldn't be read back ({e})", backup.display())]),
    }
    let mut problems = Vec::new();
    let mut removed = 0;
    // each key opened once, and each value read again through that handle just before it goes
    let mut groups: std::collections::BTreeMap<&str, Vec<&Value>> = Default::default();
    for v in &values {
        groups.entry(v.0.as_str()).or_default().push(v);
    }
    for (key, items) in &groups {
        let path = join(root, key);
        let h = match open_key(&path, KEY_EDIT) {
            Ok(Some(h)) => h,
            // the key is gone: its values went with it
            Ok(None) => continue,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        for (_, name, kind, data) in items {
            match read_one(&h, name) {
                Ok(Some((k, d))) if k == *kind && d == *data => match delete_value(&h, name) {
                    Ok(true) => removed += 1,
                    Ok(false) => {}
                    Err(e) => problems.push(format!("{}: {}", safe(key), e)),
                },
                Ok(None) => problems.push(format!("{}: {} is gone; left as it is", safe(key), safe(name))),
                // changed since it was read: it is not the value that was backed up, so it stays
                Ok(Some(_)) => problems.push(format!("{}: {} changed since it was read; left as it is", safe(key), safe(name))),
                Err(e) => problems.push(format!("{}: {}", safe(key), e)),
            }
        }
    }
    if removed == 0 {
        problems.push("no value could be removed; the backup is kept".into());
    }
    let done = (removed > 0).then(|| Done {
        part: part.id.into(),
        title: part.title.into(),
        removed,
        backup: backup.clone(),
    });
    // only now, and never the backup this run wrote
    prune(dir, Some(&backup), now());
    (done, problems)
}

/// Put the values of a backup back, with the same type and bytes, into the fixed keys only.
/// `expect`: the part the caller offered to put back (the done row's), which the backup must
/// name too. Refuses a backup that names a key outside the list, or a type Sprawl doesn't write.
pub fn restore(path: &Path, expect: Option<&str>) -> Result<usize, String> {
    restore_in(path, "", expect)
}

pub(crate) fn restore_in(path: &Path, root: &str, expect: Option<&str>) -> Result<usize, String> {
    let backup = read_backup(path)?;
    let part = part(&backup.part).ok_or_else(|| format!("\"{}\" is not a registry part Sprawl knows", safe(&backup.part)))?;
    if expect.is_some_and(|e| e != backup.part) {
        return Err(format!("the backup is for {}, not {}, so nothing was put back", safe(&backup.part), safe(expect.unwrap_or(""))));
    }
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
        if !key_name_ok(&v.key) || !value_name_ok(&v.name) {
            return Err("this backup has a key or value name Sprawl doesn't write, so nothing was put back".into());
        }
        let data = unhex(&v.data).map_err(|_| format!("\"{}\" in \"{}\" is not valid data, so nothing was put back", safe(&v.name), safe(&v.key)))?;
        if data.len() > MAX_VALUE {
            return Err(format!("\"{}\" in \"{}\" is too large to put back, so nothing was put back", safe(&v.name), safe(&v.key)));
        }
        checked.push((v.key.clone(), v.name.clone(), v.kind, data));
    }
    // the keys to write, each looked at once; a missing key is made, up to a cap
    let mut keys: Vec<String> = Vec::new();
    for (key, ..) in &checked {
        if !keys.contains(key) {
            keys.push(key.clone());
        }
    }
    let mut to_make = 0;
    for key in &keys {
        // a child is only ever made under the part's own key, and that key is looked at as
        // itself first (a link there would send the child elsewhere)
        if key != part.key {
            match open_key(&join(root, part.key), KEY_EDIT) {
                Ok(_) => {}
                Err(e) => return Err(format!("{e}; nothing was put back")),
            }
        }
        match open_key(&join(root, key), KEY_EDIT) {
            Ok(Some(_)) => {}
            Ok(None) => to_make += 1,
            Err(e) => return Err(format!("{e}; nothing was put back")),
        }
    }
    if to_make > MAX_CREATED {
        return Err(format!("this backup names {to_make} keys to make; Sprawl makes {MAX_CREATED} at most, so nothing was put back"));
    }
    let mut opened: std::collections::HashMap<String, Handle> = Default::default();
    for key in &keys {
        no_link_on_the_way(root, key)?;
        let path = join(root, key);
        let h = match open_key(&path, KEY_EDIT) {
            Ok(Some(h)) => h,
            Ok(None) => {
                // made only where a key of this backup's own part is missing; checked for a
                // link again, so a link put there meanwhile is never written through
                match create_key(&path) {
                    Ok(h) => drop(h),
                    Err(e) => return Err(format!("{}: Windows couldn't make it ({}); nothing more was put back", safe(key), e)),
                }
                match open_key(&path, KEY_EDIT) {
                    Ok(Some(h)) => h,
                    Ok(None) => return Err(format!("{}: Windows made it, then it wasn't there; nothing more was put back", safe(key))),
                    Err(e) => return Err(format!("{e}; nothing more was put back")),
                }
            }
            Err(e) => return Err(format!("{e}; nothing more was put back")),
        };
        opened.insert(key.clone(), h);
    }
    let mut written = 0;
    for (key, value_name, kind, data) in &checked {
        let Some(h) = opened.get(key) else {
            return Err(format!("{}: its key wasn't open; {} value(s) were put back before it", safe(key), written));
        };
        let name = wide(value_name);
        let r = unsafe { RegSetValueExW(h.0, name.as_ptr(), 0, *kind, data.as_ptr(), data.len() as u32) };
        if r != 0 {
            return Err(format!("{}: Windows couldn't write \"{}\" ({}); {} value(s) were put back before it", safe(key), safe(value_name), r, written));
        }
        written += 1;
    }
    Ok(written)
}

/// A value read from the registry: its key relative to HKCU, its name, its type and bytes.
type Value = (String, String, u32, Vec<u8>);

/// Every value of `part` now, including those in its immediate subkeys when it has them.
fn read_all(part: &RegPart, root: &str) -> Result<Vec<Value>, String> {
    no_link_on_the_way(root, part.key)?;
    let mut out = Vec::new();
    let key = part.key.to_string();
    for (name, kind, data) in read_values(root, &key)? {
        out.push((key.clone(), name, kind, data));
    }
    if part.subkeys {
        for sub in read_subkeys(root, &key)? {
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
    let Some(h) = open_key(&path, KEY_READ)? else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    let mut name = vec![0u16; 16_384];
    let mut data = vec![0u8; MAX_VALUE];
    let mut ended = false;
    for i in 0..100_000u32 {
        let mut nlen = name.len() as u32;
        let mut dlen = data.len() as u32;
        let mut kind = 0u32;
        let r = unsafe { RegEnumValueW(h.0, i, name.as_mut_ptr(), &mut nlen, std::ptr::null(), &mut kind, data.as_mut_ptr(), &mut dlen) };
        match r {
            0 => {
                let name = String::from_utf16(&name[..nlen as usize]).map_err(|_| format!("a value name in {} isn't valid text, so nothing was removed", safe(key)))?;
                if !value_name_ok(&name) {
                    return Err(format!("a value name in {} isn't one Sprawl writes back, so nothing was removed", safe(key)));
                }
                // a value whose type Sprawl can't write back would break "Put back": hold the
                // whole part, before anything is removed
                if kind == REG_LINK_KIND {
                    return Err(format!("{} is a registry link, so nothing was removed", safe(key)));
                }
                if !allowed_kind(kind) {
                    return Err(format!("a value in {} has a type Sprawl can't put back, so nothing was removed", safe(key)));
                }
                out.push((name, kind, data[..dlen as usize].to_vec()));
            }
            ERROR_MORE_DATA => return Err(format!("a value in {} is too large to back up, so nothing was removed", safe(key))),
            ERROR_NO_MORE_ITEMS => {
                ended = true;
                break;
            }
            e => return Err(format!("{} couldn't be read completely ({e})", safe(key))),
        }
    }
    if !ended {
        return Err(format!("{} has more values than Sprawl reads (100,000), so nothing was removed", safe(key)));
    }
    Ok(out)
}

fn read_subkeys(root: &str, key: &str) -> Result<Vec<String>, String> {
    let path = join(root, key);
    let Some(h) = open_key(&path, KEY_READ)? else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    let mut name = vec![0u16; 512];
    let mut ended = false;
    for i in 0..100_000u32 {
        let mut nlen = name.len() as u32;
        let r = unsafe { RegEnumKeyExW(h.0, i, name.as_mut_ptr(), &mut nlen, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
        match r {
            0 => {
                let name = String::from_utf16(&name[..nlen as usize]).map_err(|_| format!("a subkey name under {} isn't valid text, so nothing was removed", safe(key)))?;
                if !plain_name(&name) {
                    return Err(format!("a subkey name under {} isn't one Sprawl reads, so nothing was removed", safe(key)));
                }
                out.push(name);
            }
            ERROR_MORE_DATA => continue,
            ERROR_NO_MORE_ITEMS => {
                ended = true;
                break;
            }
            e => return Err(format!("the subkeys of {} couldn't be read completely ({e})", safe(key))),
        }
    }
    if !ended {
        return Err(format!("{} has more subkeys than Sprawl reads (100,000), so nothing was removed", safe(key)));
    }
    Ok(out)
}

/// The value `name` in the open key: its type and bytes, or None when it is gone.
fn read_one(h: &Handle, name: &str) -> Result<Option<(u32, Vec<u8>)>, String> {
    let n = wide(name);
    let mut kind = 0u32;
    let mut size = 0u32;
    let r = unsafe { RegQueryValueExW(h.0, n.as_ptr(), std::ptr::null(), &mut kind, std::ptr::null_mut(), &mut size) };
    if r == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if r != 0 {
        return Err(format!("Windows couldn't read it ({r})"));
    }
    if size as usize > MAX_VALUE {
        return Err("it is larger than Sprawl backs up".into());
    }
    let mut data = vec![0u8; size as usize];
    let mut size2 = size;
    let r = unsafe { RegQueryValueExW(h.0, n.as_ptr(), std::ptr::null(), &mut kind, data.as_mut_ptr(), &mut size2) };
    if r == ERROR_MORE_DATA {
        return Err("it changed while it was read".into());
    }
    if r != 0 {
        return Err(format!("Windows couldn't read it ({r})"));
    }
    data.truncate(size2 as usize);
    Ok(Some((kind, data)))
}

fn delete_value(h: &Handle, name: &str) -> Result<bool, String> {
    let n = wide(name);
    match unsafe { RegDeleteValueW(h.0, n.as_ptr()) } {
        0 => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        e => Err(format!("Windows didn't remove it ({e})")),
    }
}

#[cfg(test)]
fn key_exists(root: &str, key: &str) -> bool {
    matches!(open_key(&join(root, key), KEY_READ), Ok(Some(_)))
}

/// Open `path` under HKCU as itself, never through a link key. None when it isn't there.
fn open_key(path: &str, sam: u32) -> Result<Option<Handle>, String> {
    match open(path, sam, REG_OPTION_OPEN_LINK) {
        Ok(h) => {
            if is_link(&h)? {
                Err(format!("{} is a registry link, so nothing was done", safe(path)))
            } else {
                Ok(Some(h))
            }
        }
        Err(ERROR_FILE_NOT_FOUND) => Ok(None),
        Err(e) => Err(format!("{} couldn't be opened ({e})", safe(path))),
    }
}

/// Every key on the way to `path`, opened as itself: a link at any level would send reads and
/// writes elsewhere, so the part is held before anything is read or written.
fn no_link_on_the_way(root: &str, key: &str) -> Result<(), String> {
    let mut path = root.trim_end_matches('\\').to_string();
    for seg in key.split('\\').filter(|s| !s.is_empty()) {
        path = if path.is_empty() { seg.to_string() } else { format!(r"{path}\{seg}") };
        match open(&path, KEY_READ, REG_OPTION_OPEN_LINK) {
            Ok(h) => {
                if is_link(&h)? {
                    return Err(format!("{} is a registry link, so nothing was done", safe(&path)));
                }
            }
            // the rest is missing: there is nothing further to follow
            Err(ERROR_FILE_NOT_FOUND) => return Ok(()),
            Err(e) => return Err(format!("{} couldn't be opened ({e})", safe(&path))),
        }
    }
    Ok(())
}

/// Whether an opened key is a symbolic link (its `SymbolicLinkValue` is there).
fn is_link(h: &Handle) -> Result<bool, String> {
    let n = wide(LINK_VALUE);
    let mut size = 0u32;
    let r = unsafe { RegQueryValueExW(h.0, n.as_ptr(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), &mut size) };
    match r {
        0 => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        e => Err(format!("Windows couldn't tell whether a key is a registry link ({e})")),
    }
}

/// The open error is the code RegOpenKeyExW returned (it does not set the thread's last error).
fn open(path: &str, sam: u32, options: u32) -> Result<Handle, u32> {
    let w = wide(path);
    let mut h: HKEY = std::ptr::null_mut();
    let r = unsafe { RegOpenKeyExW(HKCU, w.as_ptr(), options, sam, &mut h) };
    if r == 0 {
        Ok(Handle(h))
    } else {
        Err(r)
    }
}

fn create_key(path: &str) -> Result<Handle, u32> {
    let w = wide(path);
    let mut h: HKEY = std::ptr::null_mut();
    let mut disposition = 0u32;
    let r = unsafe { RegCreateKeyExW(HKCU, w.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_EDIT, std::ptr::null(), &mut h, &mut disposition) };
    if r == 0 {
        Ok(Handle(h))
    } else {
        Err(r)
    }
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
    let when = now();
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

/// The backup file holds exactly the values read, for the same part.
fn backup_matches(b: &Backup, part: &RegPart, values: &[Value]) -> bool {
    b.version == 1
        && b.part == part.id
        && b.values.len() == values.len()
        && b.values.iter().zip(values).all(|(v, (key, name, kind, data))| v.key == *key && v.name == *name && v.kind == *kind && unhex(&v.data).is_ok_and(|d| d == *data))
}

/// Keep the last [`KEEP_BACKUPS`] backups. Runs after the deletes; `keep` (the backup just
/// written) is never removed, a backup younger than [`MIN_AGE`] isn't either, and only files of
/// Sprawl's own backup shape are counted or removed. A file that can't be removed is left and
/// still counts as kept.
fn prune(dir: &Path, keep: Option<&Path>, now: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(u64, PathBuf)> = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        let Some(stamp) = path.file_name().and_then(|n| n.to_str()).and_then(backup_stamp) else { continue };
        if keep.is_some_and(|k| k == path) || now.saturating_sub(stamp) < MIN_AGE {
            continue;
        }
        files.push((stamp, path));
    }
    if files.len() <= KEEP_BACKUPS {
        return;
    }
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    for (_, p) in files.into_iter().skip(KEEP_BACKUPS) {
        let _ = crate::remove_local(&p);
    }
}

/// The timestamp in a backup file's own name (`registry-backup-<10 digits>-<nonce>.json`), or
/// None for any other name.
fn backup_stamp(name: &str) -> Option<u64> {
    let stem = name.strip_prefix(PREFIX)?.strip_suffix(".json")?;
    let (stamp, nonce) = stem.split_once('-')?;
    if stamp.len() != 10 || !stamp.bytes().all(|b| b.is_ascii_digit()) || nonce.is_empty() || nonce.len() > 80 || !nonce.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return None;
    }
    stamp.parse().ok()
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
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

/// A registry key name Sprawl will write under a fixed key: not empty, not `.` or `..`, no
/// separator or control character, and not absurdly long.
fn plain_name(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.len() <= 255 && !s.chars().any(|c| c.is_control() || matches!(c, '\\' | '/' | ':'))
}

/// A value name Sprawl reads and writes: one line, no NUL or other control character.
fn value_name_ok(s: &str) -> bool {
    s.len() <= 16_383 && !s.chars().any(char::is_control)
}

/// A key path from a backup: plain segments only (checked again here, so a file someone edited
/// cannot name `..` or a separator).
fn key_name_ok(s: &str) -> bool {
    let t = s.trim_matches('\\');
    !t.is_empty() && t.split('\\').all(|seg| plain_name(seg) && seg != "." && seg != "..")
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
    fn RegQueryValueExW(hkey: HKEY, name: *const u16, reserved: *const u32, kind: *mut u32, data: *mut u8, len: *mut u32) -> u32;
    fn RegDeleteValueW(hkey: HKEY, name: *const u16) -> u32;
    fn RegSetValueExW(hkey: HKEY, name: *const u16, reserved: u32, kind: u32, data: *const u8, len: u32) -> u32;
}

#[cfg(test)]
#[link(name = "advapi32")]
extern "system" {
    fn RegDeleteKeyW(hkey: HKEY, subkey: *const u16) -> u32;
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
        // a link key is opened as itself, so it can be removed like any other empty key
        let Ok(h) = open(path, KEY_ALL_ACCESS, REG_OPTION_OPEN_LINK) else { return };
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
        let path = join(root, key);
        if !matches!(open_key(&path, KEY_EDIT), Ok(Some(_))) {
            create_key(&path).unwrap();
        }
        let h = match open_key(&path, KEY_EDIT) {
            Ok(Some(h)) => h,
            _ => panic!("couldn't open a test key"),
        };
        let n = wide(name);
        let r = unsafe { RegSetValueExW(h.0, n.as_ptr(), 0, kind, data.as_ptr(), data.len() as u32) };
        assert_eq!(r, 0, "couldn't write a test value");
    }

    fn values_of(root: &str, key: &str) -> Vec<(String, u32, Vec<u8>)> {
        read_values(root, key).unwrap()
    }

    /// Every value name in `key`, whatever its type (a test of a type read_values refuses).
    fn raw_names(root: &str, key: &str) -> Vec<String> {
        let h = open_key(&join(root, key), KEY_READ).unwrap().unwrap();
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

    /// Ten backups stamped later than the new one can't prune it away: it is protected, and
    /// the values go with it intact.
    #[test]
    fn ten_later_dated_backups_do_not_prune_the_new_one() {
        let root = test_root();
        let _key = TestKey(root.clone());
        let dir = test_dir();
        let part = part("registry.explorer/typedpaths").unwrap();
        set_in(&root, part.key, "url1", SZ, &utf16(r"C:\Users\sam\Documents"));
        let later = now() + 1000;
        for i in 0..10 {
            let p = dir.0.join(format!("{PREFIX}{:010}-{:x}.json", later + i, i));
            crate::write_local(&p, br#"{"version":1,"part":"registry.explorer/typedpaths","when":1,"values":[]}"#, false).unwrap();
        }
        let (done, problems) = remove_in(part, &root, &dir.0);
        assert!(problems.is_empty(), "{problems:?}");
        let done = done.unwrap();
        assert_eq!(done.removed, 1);
        // the new backup is still there, readable, and holds the value that went
        let read = read_backup(&done.backup).unwrap();
        assert_eq!((read.part.as_str(), read.values.len()), (part.id, 1));
        assert_eq!(unhex(&read.values[0].data).unwrap(), utf16(r"C:\Users\sam\Documents"));
        assert_eq!(dir.0.read_dir().unwrap().count(), 11, "the ten later ones and the new one");
        assert!(read_all(part, &root).unwrap().is_empty());
    }

    /// Prune keeps the ten newest by their parsed stamp, never the file it is given, and leaves
    /// a file it can't remove (it still counts as kept).
    #[test]
    fn prune_keeps_ten_by_stamp_and_never_the_new_one() {
        let dir = test_dir();
        let body = br#"{"version":1,"part":"registry.explorer/runmru","when":1,"values":[]}"#;
        // twelve dated files (an hour to twelve hours after the epoch) and the "new" one, whose
        // stamp is older than all of them
        let mut made = Vec::new();
        for i in 0..12u64 {
            let p = dir.0.join(format!("{PREFIX}{:010}-{:x}.json", 1000 + i * 1000, i));
            crate::write_local(&p, body, false).unwrap();
            made.push(p);
        }
        let new = dir.0.join(format!("{PREFIX}0000000500-aa.json"));
        crate::write_local(&new, body, false).unwrap();
        // a folder of the backup shape that can't be removed
        let folder = dir.0.join(format!("{PREFIX}0000000000-bb.json"));
        std::fs::create_dir(&folder).unwrap();
        prune(&dir.0, Some(&new), 20_000);
        assert!(new.exists(), "the backup just written stays");
        assert!(folder.exists(), "a file that can't be removed is left");
        // the ten newest of the twelve dated files stay; 1000 and 2000 go
        assert!(!made[0].exists() && !made[1].exists(), "the two oldest go");
        assert!(made[2..].iter().all(|p| p.exists()), "the ten newest stay");
        assert_eq!(dir.0.read_dir().unwrap().count(), 12, "ten + the new one + the folder");
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
    /// creates any subkey that is not there any more. It won't put back another part's backup.
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

        let path = dir.0.read_dir().unwrap().next().unwrap().unwrap().path();
        // another part's backup is refused
        assert!(restore_in(&path, &root, Some("registry.explorer/runmru")).unwrap_err().contains("not registry.explorer/runmru"));
        assert_eq!(restore_in(&path, &root, Some(part.id)).unwrap(), 7);
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
        let err = restore_in(&bad, &root, None).unwrap_err();
        assert!(err.contains("not one of the fixed keys") && err.contains("nothing was put back"), "{err}");
        // the fixed key is still empty: the refused file wrote nothing anywhere
        assert!(read_all(part, &root).unwrap().is_empty());

        // a subkey under a part that has none
        let mut backup: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        backup["values"][0]["key"] = serde_json::json!(format!(r"{}\sub", part.key));
        let bad = dir.0.join("bad2.json");
        crate::write_local(&bad, &serde_json::to_vec(&backup).unwrap(), false).unwrap();
        let err = restore_in(&bad, &root, None).unwrap_err();
        assert!(err.contains("not one of the fixed keys"), "{err}");
        assert!(read_all(part, &root).unwrap().is_empty());

        // a part Sprawl doesn't know
        let mut backup: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        backup["part"] = serde_json::json!("registry.explorer/passwords");
        let bad = dir.0.join("bad3.json");
        crate::write_local(&bad, &serde_json::to_vec(&backup).unwrap(), false).unwrap();
        assert!(restore_in(&bad, &root, None).is_err());
    }

    /// The compiled list: the six required lists, HKCU Explorer only, and none of the names that
    /// are never cleaned.
    #[test]
    fn the_list_holds_only_allowed_keys() {
        assert_eq!(PARTS.len(), 6);
        let explorer = r"Software\Microsoft\Windows\CurrentVersion\Explorer";
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
            assert!(p.key.starts_with(explorer), "{}: {}", p.id, p.key);
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

    /// A value name Sprawl can't write back (a control character) holds the whole part, and a
    /// subkey name like that does too.
    #[test]
    fn a_name_sprawl_cannot_write_holds_the_whole_part() {
        let root = test_root();
        let _key = TestKey(root.clone());
        let dir = test_dir();
        let mru = super::part("registry.explorer/runmru").unwrap();
        set_in(&root, mru.key, "a", SZ, &utf16("notepad"));
        set_in(&root, mru.key, "bad\u{1}name", SZ, &utf16("x"));
        let (done, problems) = remove_in(mru, &root, &dir.0);
        assert!(done.is_none());
        assert!(problems.iter().any(|p| p.contains("value name") && p.contains("nothing was removed")), "{problems:?}");
        assert_eq!(raw_names(&root, mru.key).len(), 2, "both values are still there");

        // a per-file-type subkey whose name can't be written back
        let docs = super::part("registry.explorer/recentdocs").unwrap();
        create_key(&join(&root, &format!("{}\\bad\u{1}ext", docs.key))).unwrap();
        let (done, problems) = remove_in(docs, &root, &dir.0);
        assert!(done.is_none());
        assert!(problems.iter().any(|p| p.contains("subkey name") && p.contains("nothing was removed")), "{problems:?}");
        assert_eq!(raw_names(&root, mru.key).len(), 2);
    }

    /// A registry link key is never followed: its target is not read, deleted or restored, and
    /// the part is held whole.
    #[test]
    fn a_link_key_is_held_and_its_target_untouched() {
        let root = test_root();
        let _key = TestKey(root.clone());
        let dir = test_dir();
        let part = part("registry.explorer/recentdocs").unwrap();
        let target_key = format!(r"{root}\target");
        set_in(&root, &target_key, "secret", SZ, &utf16("keep me"));
        // the part's key, with one real value and one link subkey pointing at `target`
        set_in(&root, part.key, "0", SZ, &utf16("mine.txt"));
        let sid = sprawl_core::identity::account_sid().expect("this account's SID");
        let target = format!(r"\REGISTRY\USER\{sid}\{target_key}");
        create_link(&join(&root, part.key), "link", &target).expect("a link key can be made in this account's registry");

        // neither offered nor read through
        let found = find_in(&root);
        assert!(!found.iter().any(|f| matches!(&f.source, Source::Rule(id) if id == part.id)), "the part is held, not offered");
        let err = read_all(part, &root).unwrap_err();
        assert!(err.contains("registry link"), "{err}");
        let (done, problems) = remove_in(part, &root, &dir.0);
        assert!(done.is_none(), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("registry link")), "{problems:?}");
        // the real value stays, the target stays untouched, no backup was written
        assert_eq!(values_of(&root, part.key).len(), 1);
        assert_eq!(one(&values_of(&root, &target_key), "secret"), (SZ, utf16("keep me")));
        assert_eq!(dir.0.read_dir().unwrap().count(), 0);
    }

    /// A link at a fixed key's own path is held too, and its target is untouched. (A real
    /// fixed key is never written by the tests; this is a link of the same shape under the
    /// test root, opened exactly as a part's key is.)
    #[test]
    fn a_link_at_a_fixed_key_is_held() {
        let root = test_root();
        let _key = TestKey(root.clone());
        let target_key = format!(r"{root}\target");
        set_in(&root, &target_key, "secret", SZ, &utf16("keep me"));
        let sid = sprawl_core::identity::account_sid().expect("this account's SID");
        let target = format!(r"\REGISTRY\USER\{sid}\{target_key}");
        create_link(&root, "ownlink", &target).expect("a link key can be made");
        let link = format!(r"{root}\ownlink");
        // a key at the fixed path is seen as the link, so nothing reads, deletes or writes it
        assert!(open_key(&link, KEY_EDIT).err().is_some_and(|e| e.contains("registry link")));
        assert!(read_values("", &link).unwrap_err().contains("registry link"));
        // and a link on the way to a deeper key holds it too
        let err = no_link_on_the_way(&root, r"ownlink\leaf").unwrap_err();
        assert!(err.contains("registry link"), "{err}");
        assert!(no_link_on_the_way(&root, r"plain\leaf").is_ok(), "a missing key on the way is fine");
        assert_eq!(one(&values_of(&root, &target_key), "secret"), (SZ, utf16("keep me")));
    }

    /// What the app lists in Settings: the kept backups, newest first, with their part and count.
    #[test]
    fn the_kept_backups_are_listed() {
        let root = test_root();
        let _key = TestKey(root.clone());
        let dir = test_dir();
        let part = part("registry.explorer/runmru").unwrap();
        set_in(&root, part.key, "a", SZ, &utf16("notepad"));
        let (done, _) = remove_in(part, &root, &dir.0);
        let done = done.unwrap();
        let list = backups_in(&dir.0);
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].part.as_str(), list[0].title.as_str(), list[0].values), (part.id, "Run box history", 1));
        assert_eq!(list[0].path, done.backup);
        // a file not of the backup shape is left out, and one that can't be read
        std::fs::write(dir.0.join("notes.json"), b"{}").unwrap();
        std::fs::write(dir.0.join(format!("{PREFIX}0000000001-aa.json")), b"not json").unwrap();
        assert_eq!(backups_in(&dir.0).len(), 1);
    }

    fn count_in(part: &RegPart, root: &str) -> usize {
        read_all(part, root).map_or(0, |v| v.len())
    }

    /// A symbolic link key at `parent\name`, pointing at `target`, for the tests.
    fn create_link(parent: &str, name: &str, target: &str) -> Result<(), u32> {
        let path = format!(r"{parent}\{name}");
        let w = wide(&path);
        let mut h: HKEY = std::ptr::null_mut();
        let mut disposition = 0u32;
        let r = unsafe { RegCreateKeyExW(HKCU, w.as_ptr(), 0, std::ptr::null(), REG_OPTION_CREATE_LINK, KEY_ALL_ACCESS, std::ptr::null(), &mut h, &mut disposition) };
        if r != 0 {
            return Err(r);
        }
        let handle = Handle(h);
        let n = wide(LINK_VALUE);
        let data: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
        let bytes: Vec<u8> = data.iter().flat_map(|c| c.to_le_bytes()).collect();
        let r = unsafe { RegSetValueExW(handle.0, n.as_ptr(), 0, REG_LINK_KIND, bytes.as_ptr(), bytes.len() as u32) };
        if r != 0 {
            return Err(r);
        }
        Ok(())
    }
}
