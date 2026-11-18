//! A complete previous launch bundle, flushed before the first update rename.
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use crate::{FileEntry, setup};

pub const DIRECTORY: &str = ".sprawl-recovery";
const RECORD: &str = "recovery.json";
const READY: &str = "READY";
const README: &str = "README.txt";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    old_version: String,
    new_version: String,
    old_files: Vec<FileEntry>,
    new_files: Vec<FileEntry>,
}

/// These handles keep the complete recovery copy unchanged while the update is applied.
pub(crate) struct Prepared {
    _directory: sprawl_core::local::Directory,
    _files: Vec<File>,
}

pub fn running_copy() -> bool {
    std::env::current_exe().ok().and_then(|p| p.parent().and_then(Path::file_name).map(|n| n.eq_ignore_ascii_case(DIRECTORY))).unwrap_or(false)
}

fn create(path: &Path) -> Result<File, String> {
    OpenOptions::new().read(true).write(true).access_mode(0x8000_0000 | 0x4000_0000 | 0x0001_0000).share_mode(1)
        .create_new(true).custom_flags(0x0020_0000 | 0x0010_0000).open(path).map_err(|e| e.to_string())
}

fn text_file(path: &Path, bytes: &[u8]) -> Result<File, String> {
    let mut file = create(path)?;
    file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
    Ok(file)
}

pub(crate) fn prepare(dir: &Path, version: &str, replacements: Vec<FileEntry>) -> Result<Prepared, String> {
    setup::per_user_only()?;
    if running_copy() || crate::parse_version(version).is_none() { return Err("Use Sprawl Setup to restore an installation from its recovery copy.".into()); }
    let root = sprawl_core::local::directory(dir).map_err(|e| e.to_string())?;
    cleanup(dir);
    // Pin the whole old bundle before copying any of it, so all files describe one state.
    let mut current = crate::FILES.iter().map(|name| {
        let held = sprawl_core::local::file(&root.path.join(name), 512 << 20).map_err(|e| e.to_string())?;
        Ok((name.to_string(), held))
    }).collect::<Result<Vec<_>, String>>()?;
    let mut old_files = Vec::new();
    for (name, held) in &mut current {
        let size = held.file.metadata().map_err(|e| e.to_string())?.len();
        let sha256 = setup::hash_file(&mut held.file)?;
        held.file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        old_files.push(FileEntry { name: name.clone(), size, sha256 });
    }
    let mut new_files = old_files.clone();
    for replacement in replacements {
        new_files.retain(|old| !old.name.eq_ignore_ascii_case(&replacement.name));
        new_files.push(replacement);
    }
    let record = Record { schema: 1, old_version: env!("CARGO_PKG_VERSION").into(), new_version: version.into(), old_files, new_files };
    if !valid(&record) { return Err("Invalid recovery manifest".into()); }
    let path = root.path.join(DIRECTORY);
    std::fs::create_dir(&path).map_err(|e| format!("The recovery folder could not be prepared ({e}). Keep {}\\sprawl.exe if restoring an interrupted update, or reinstall with Sprawl Setup.", path.display()))?;
    let directory = sprawl_core::local::directory(&path).map_err(|e| e.to_string())?;
    let mut files = vec![text_file(&path.join(RECORD), &serde_json::to_vec(&record).map_err(|e| e.to_string())?)?];
    for ((name, source), entry) in current.iter_mut().zip(&record.old_files) {
        let mut target = create(&path.join(name))?;
        std::io::copy(&mut source.file, &mut target).map_err(|e| e.to_string())?;
        target.sync_all().map_err(|e| e.to_string())?;
        target.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        setup::check_hash(&mut target, &entry.sha256)?;
        files.push(target);
    }
    files.push(text_file(&path.join(README), b"Sprawl update recovery\r\n\r\nIf the update was interrupted, open sprawl.exe in this folder. All files of the previous launch bundle are kept together here. Reinstall Sprawl with its Setup to restore the main installation. Do not move just the EXE: it needs the accompanying DLL. This copy never updates itself or retargets your scheduled alert task.\r\n")?);
    files.push(text_file(&path.join(READY), b"Complete previous launch bundle\r\n")?);
    Ok(Prepared { _directory: directory, _files: files })
}

fn valid_files(files: &[FileEntry]) -> bool {
    let mut names = std::collections::HashSet::new();
    !files.is_empty() && files.len() <= 64 && files.iter().all(|f| crate::valid_name(&f.name) && f.size <= 512 << 20
        && f.sha256.len() == 64 && f.sha256.bytes().all(|b| b.is_ascii_hexdigit()) && names.insert(f.name.to_ascii_lowercase()))
}

fn valid(record: &Record) -> bool {
    record.schema == 1 && crate::parse_version(&record.old_version).is_some() && crate::parse_version(&record.new_version).is_some()
        && valid_files(&record.old_files) && valid_files(&record.new_files)
        && record.old_files.len() == crate::FILES.len()
        && crate::FILES.iter().all(|name| record.old_files.iter().any(|f| f.name == *name))
        && crate::FILES.iter().all(|name| record.new_files.iter().any(|f| f.name == *name))
}

fn verify_bundle(dir: &Path, files: &[FileEntry]) -> Option<Vec<sprawl_core::local::LocalFile>> {
    files.iter().map(|entry| {
        let mut held = sprawl_core::local::file(&dir.join(&entry.name), 512 << 20).ok()?;
        if held.file.metadata().ok()?.len() != entry.size { return None; }
        setup::check_hash(&mut held.file, &entry.sha256).ok()?;
        Some(held)
    }).collect()
}

fn remove_owned(path: &Path) -> bool {
    let file = OpenOptions::new().access_mode(0x0001_0080).share_mode(1 | 2 | 4)
        .custom_flags(0x0020_0000 | 0x0010_0000).open(path);
    let file = match file { Ok(f) => f, Err(e) => return e.kind() == std::io::ErrorKind::NotFound };
    if !file.metadata().is_ok_and(|m| m.is_file() && m.file_attributes() & (0x400 | 0x1000 | 0x0004_0000 | 0x0040_0000) == 0) { return false; }
    setup::delete_open(&file)
}

/// Called after the normal UI starts. Recovery is discarded only when this running version
/// has a complete matching old or new bundle in the main folder. Mixed/missing files retain it.
pub fn cleanup(dir: &Path) { cleanup_for(dir, env!("CARGO_PKG_VERSION")); }

fn cleanup_for(dir: &Path, version: &str) {
    if setup::per_user_only().is_err() || running_copy() { return; }
    let Ok(root) = sprawl_core::local::directory(dir) else { return; };
    let path = root.path.join(DIRECTORY);
    let Ok(directory) = sprawl_core::local::directory(&path) else { return; };
    let record = (|| {
        let mut file = sprawl_core::local::file(&path.join(RECORD), 64 << 10).ok()?;
        let mut bytes = Vec::new();
        file.file.read_to_end(&mut bytes).ok()?;
        serde_json::from_slice::<Record>(&bytes).ok()
    })();
    let Some(record) = record.filter(valid) else { return; };
    let current = if record.old_version == version { verify_bundle(&root.path, &record.old_files) } else { None }
        .or_else(|| if record.new_version == version { verify_bundle(&root.path, &record.new_files) } else { None });
    let Some(_current) = current else { return; };
    for entry in &record.old_files { if !remove_owned(&path.join(&entry.name)) { return; } }
    for name in [README, READY, RECORD] { if !remove_owned(&path.join(name)) { return; } }
    drop(directory);
    let _ = std::fs::remove_dir(path); // unknown files and subdirectories always stay
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sprawl-recovery-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in crate::FILES { std::fs::write(dir.join(name), format!("old {name}")).unwrap(); }
        dir
    }

    #[test]
    fn only_a_matching_running_bundle_can_retire_recovery() {
        if setup::per_user_only().is_err() { return; }
        let dir = fixture("retire");
        let replacements: Vec<_> = crate::FILES.iter().map(|name| FileEntry { name: name.to_string(), size: 3, sha256: crate::sha256_hex(b"new") }).collect();
        let prepared = prepare(&dir, "99.0.0", replacements).unwrap();
        let backup = dir.join(DIRECTORY);
        assert!(std::fs::write(backup.join("sprawl.exe"), b"tamper").is_err(), "the ready bundle stays locked through publication");
        drop(prepared);
        std::fs::write(dir.join("sprawl.exe"), b"new").unwrap();
        cleanup_for(&dir, "99.0.0");
        assert!(backup.join(READY).exists(), "a mixed bundle is never enough");
        std::fs::write(dir.join(crate::FILES[1]), b"new").unwrap();
        cleanup_for(&dir, "98.0.0");
        assert!(backup.join(READY).exists(), "another running version cannot retire this recovery");
        std::fs::write(backup.join("personal.txt"), b"keep").unwrap();
        cleanup_for(&dir, "99.0.0");
        assert_eq!(std::fs::read(backup.join("personal.txt")).unwrap(), b"keep");
        assert!(!backup.join("sprawl.exe").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_record_cannot_authorise_deletion() {
        if setup::per_user_only().is_err() { return; }
        let dir = fixture("invalid");
        let prepared = prepare(&dir, "99.0.0", Vec::new()).unwrap();
        drop(prepared);
        let path = dir.join(DIRECTORY);
        let mut record: Record = serde_json::from_slice(&std::fs::read(path.join(RECORD)).unwrap()).unwrap();
        record.old_files[0].name = "unrelated.exe".into();
        std::fs::write(path.join("unrelated.exe"), b"keep").unwrap();
        std::fs::write(path.join(RECORD), serde_json::to_vec(&record).unwrap()).unwrap();
        cleanup(&dir);
        assert!(path.join(READY).exists() && path.join("sprawl.exe").exists());
        assert_eq!(std::fs::read(path.join("unrelated.exe")).unwrap(), b"keep");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
