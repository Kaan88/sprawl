//! The install folder, swapping files in, and the Apps entry. Installing is Inno Setup's
//! (`installer/sprawl.iss`); Sprawl itself only keeps the version in its Apps entry current
//! after an update, and still uninstalls copies set up by 0.2.0-0.2.2's own installer.

use std::path::{Path, PathBuf};

use windows::core::HSTRING;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteTreeW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_YESNO};

/// Inno Setup's Apps entry (`AppId=Sprawl`).
const INNO_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Sprawl_is1";
/// The Apps entry of 0.2.0-0.2.2's own installer.
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Sprawl";
/// The Explorer "Scan with Sprawl" entries (see the app's dialogs.rs); removed on uninstall.
const MENU_KEYS: [&str; 2] = [r"Software\Classes\Directory\shell\Sprawl", r"Software\Classes\Drive\shell\Sprawl"];

/// Updates and per-user maintenance never inherit an administrator token.
pub fn per_user_only() -> Result<(), String> {
    match sprawl_core::elevation() {
        Ok(false) => Ok(()),
        Ok(true) => Err("Close Sprawl and reopen it normally to update or maintain this per-user installation.".into()),
        Err(_) => Err("Windows could not establish that Sprawl is running without administrator rights.".into()),
    }
}

pub const LEGACY_ALERT_TASK: &str = "Sprawl low-space check";

pub fn alert_task_name() -> Result<String, String> {
    sprawl_core::identity::account_sid().map(|sid| format!("{LEGACY_ALERT_TASK} {sid}")).map_err(|e| e.to_string())
}

pub fn system_dir() -> Result<PathBuf, String> {
    #[link(name = "kernel32")]
    extern "system" { fn GetSystemDirectoryW(buf: *mut u16, len: u32) -> u32; }
    let mut buf = [0u16; 32768];
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() { return Err("Windows' system directory is unknown".into()); }
    String::from_utf16(&buf[..n]).map(PathBuf::from).map_err(|e| e.to_string())
}

fn legacy_task_owned(xml: &str, sid: &str) -> bool {
    let Some((_, rest)) = xml.split_once("<UserId>") else { return false; };
    let Some((owner, rest)) = rest.split_once("</UserId>") else { return false; };
    owner.trim() == sid && !rest.contains("<UserId>")
}

/// Delete the former shared task name only after Task Scheduler identifies its principal
/// as this process's exact SID. Unknown or other-account tasks are always left alone.
pub fn migrate_alert_task() {
    use std::os::windows::process::CommandExt;
    if per_user_only().is_err() { return; }
    let (Ok(sid), Ok(sys)) = (sprawl_core::identity::account_sid(), system_dir()) else { return; };
    let Ok(out) = std::process::Command::new(sys.join("schtasks.exe")).current_dir(&sys)
        .args(["/Query", "/TN", LEGACY_ALERT_TASK, "/XML"]).creation_flags(0x0800_0000).output() else { return; };
    if !out.status.success() || out.stdout.len() > 1 << 20 { return; }
    let xml = if out.stdout.starts_with(&[0xff, 0xfe]) {
        String::from_utf16(&out.stdout[2..].chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>()).ok()
    } else { String::from_utf8(out.stdout).ok() };
    if !xml.is_some_and(|xml| legacy_task_owned(&xml, &sid)) { return; }
    let _ = std::process::Command::new(sys.join("schtasks.exe")).current_dir(&sys)
        .args(["/Delete", "/TN", LEGACY_ALERT_TASK, "/F"]).creation_flags(0x0800_0000)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
}

pub fn remove_alert_task() {
    use std::os::windows::process::CommandExt;
    if per_user_only().is_err() { return; }
    let (Ok(task), Ok(sys)) = (alert_task_name(), system_dir()) else { return; };
    let _ = std::process::Command::new(sys.join("schtasks.exe")).current_dir(&sys)
        .args(["/Delete", "/TN", &task, "/F"]).creation_flags(0x0800_0000)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    migrate_alert_task();
}

/// `%LOCALAPPDATA%\Programs\Sprawl`.
pub fn install_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Programs").join("Sprawl"))
}

/// The Start menu shortcut 0.2.0-0.2.2's own installer made.
fn shortcut_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Microsoft").join("Windows").join("Start Menu").join("Programs").join("Sprawl.lnk"))
}

fn same(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase();
    n(a) == n(b)
}

fn here() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf))
}

/// A text value of `HKCU\<key>`.
fn reg_str(key: &str, name: &str) -> Option<String> {
    let mut k = HKEY::default();
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(key), None, KEY_READ, &mut k) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let r = RegQueryValueExW(k, &HSTRING::from(name), None, None, Some(buf.as_mut_ptr().cast()), Some(&mut size));
        let _ = RegCloseKey(k);
        (r == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).min(buf.len())]).trim_end_matches('\0').to_string())
    }
}

/// Where Inno Setup installed Sprawl, if it did.
fn inno_dir() -> Option<PathBuf> {
    reg_str(INNO_KEY, "InstallLocation").filter(|s| !s.trim().is_empty()).map(|s| PathBuf::from(s.trim().trim_end_matches('\\')))
}

/// Whether this program runs from the installed folder (Inno Setup's, or the old installer's).
pub fn is_installed_copy() -> bool {
    let Some(here) = here() else { return false };
    inno_dir().is_some_and(|d| same(&here, &d)) || (reg_str(UNINSTALL_KEY, "DisplayVersion").is_some() && install_dir().is_some_and(|d| same(&here, &d)))
}

/// Whether this copy was set up by 0.2.0-0.2.2's own installer (its `--uninstall` is ours).
fn is_legacy_copy() -> bool {
    let Some(here) = here() else { return false };
    reg_str(UNINSTALL_KEY, "DisplayVersion").is_some() && install_dir().is_some_and(|d| same(&here, &d))
}

/// Remove the `.old` files a previous swap left (they may still be in use until now).
pub fn remove_old(dir: &Path) {
    if per_user_only().is_err() { return; }
    let Ok(_held) = sprawl_core::local::directory(dir) else { return; };
    // An interrupted swap may have left the old bundle as the only usable copy. Do not
    // discard any backup while a required current file is missing or redirected.
    let Ok(_current) = crate::FILES.iter().map(|name| sprawl_core::local::file(&dir.join(name), 512 << 20)).collect::<Result<Vec<_>, _>>() else { return; };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.strip_suffix(".old").is_some_and(|name| crate::FILES.iter().any(|owned| owned.eq_ignore_ascii_case(name))) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Where an update is downloaded to before it is swapped in.
pub const STAGE: &str = ".sprawl-update";

/// Empty the staging folder: only file names an update could have written, then the folder
/// itself if that leaves it empty (never recursive, never through a link).
pub fn clean_stage(dir: &Path) {
    if per_user_only().is_err() { return; }
    let Ok(_held) = sprawl_core::local::directory(dir) else { return; };
    let st = dir.join(STAGE);
    let Ok(_stage) = sprawl_core::local::directory(&st) else { return; };
    if let Ok(rd) = std::fs::read_dir(&st) {
        for e in rd.flatten() {
            if crate::FILES.iter().any(|owned| owned.eq_ignore_ascii_case(&e.file_name().to_string_lossy())) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    drop(_stage);
    let _ = std::fs::remove_dir(&st);
}

/// A junction, symbolic link or other reparse point.
pub fn is_link(p: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0)
}

fn check_hash(file: &mut std::fs::File, sha: &str) -> Result<(), String> {
    use std::io::Read;
    use sha2::{Digest, Sha256};
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    if len > 512 << 20 { return Err("The update file exceeds 512 MB".into()); }
    let mut hash = Sha256::new();
    let mut buf = [0u8; 64 << 10];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        hash.update(&buf[..n]);
    }
    if format!("{:x}", hash.finalize()) != sha { return Err("An update file changed before it could be used".into()); }
    Ok(())
}

/// Keep all newly installed launch files stable, after hashing their open handles, until
/// the caller has started the replacement process. No elevated launch is permitted.
pub fn verified_launch(dir: &Path, staged: &[(String, PathBuf, String)]) -> Result<Vec<sprawl_core::local::LocalFile>, String> {
    per_user_only()?;
    staged.iter().map(|(name, _, sha)| {
        if !crate::manifest::valid_name(name) { return Err("Invalid update name".into()); }
        let mut held = sprawl_core::local::file(&dir.join(name), 512 << 20).map_err(|e| e.to_string())?;
        check_hash(&mut held.file, sha)?;
        Ok(held)
    }).collect()
}

fn rename_open(file: &std::fs::File, to: &Path) -> std::io::Result<()> {
    use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
    #[link(name = "kernel32")]
    extern "system" { fn SetFileInformationByHandle(file: *mut core::ffi::c_void, class: u32, info: *const core::ffi::c_void, size: u32) -> i32; }
    let name: Vec<u16> = std::ffi::OsStr::new(&format!(r"\??\{}", to.display())).encode_wide().collect();
    let head = 8 + std::mem::size_of::<usize>() + 4;
    let size = head + name.len() * 2 + 2;
    let mut buf = vec![0u64; size.div_ceil(8)];
    let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * 8) };
    bytes[8 + std::mem::size_of::<usize>()..head].copy_from_slice(&((name.len() * 2) as u32).to_le_bytes());
    for (i, c) in name.iter().enumerate() { bytes[head + i * 2..head + i * 2 + 2].copy_from_slice(&c.to_le_bytes()); }
    if unsafe { SetFileInformationByHandle(file.as_raw_handle().cast(), 3, buf.as_ptr().cast(), size as u32) } != 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

fn delete_open(file: &std::fs::File) -> bool {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" { fn SetFileInformationByHandle(file: *mut core::ffi::c_void, class: u32, info: *const core::ffi::c_void, size: u32) -> i32; }
    // POSIX disposition removes this checked publication's name immediately, allowing its
    // old file to be restored without closing the handle or touching a replacement name.
    let flags = 3u32;
    unsafe { SetFileInformationByHandle(file.as_raw_handle().cast(), 21, (&flags as *const u32).cast(), 4) != 0 }
}

/// Put `staged` files (name, path of the new file on the same drive) into `dir`. Each current
/// file is moved aside to `<name>.old` first; if any step fails, everything is put back as it
/// was. Only names in [`crate::FILES`] are written.
pub fn swap_in(dir: &Path, staged: &[(String, PathBuf, String)]) -> Result<(), String> {
    swap_in_with(dir, staged, || {})
}

fn swap_in_with(dir: &Path, staged: &[(String, PathBuf, String)], before_publish: impl FnOnce()) -> Result<(), String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    per_user_only()?;
    let root = sprawl_core::local::directory(dir).map_err(|e| e.to_string())?;
    let dir = &root.path;
    let _stage = sprawl_core::local::directory(&dir.join(STAGE)).map_err(|e| e.to_string())?;
    if let Some((n, _, _)) = staged.iter().find(|(n, _, _)| !crate::manifest::valid_name(n)) {
        return Err(format!("refused to write {n}"));
    }
    if !staged.iter().any(|(name, _, _)| name == "sprawl.exe") { return Err("The update has no program file".into()); }
    let mut seen = std::collections::HashSet::new();
    let mut checked = Vec::new();
    for (name, from, sha) in staged {
        if !seen.insert(name.to_ascii_lowercase()) || sprawl_core::local::lexical(from).map_err(|e| e.to_string())? != dir.join(STAGE).join(name) { return Err("Invalid or repeated staged update path".into()); }
        // DELETE is needed for a rename by handle. Delete sharing lets the checked handle
        // move; withholding write sharing keeps the bytes stable through hashing/publication.
        let mut file = std::fs::OpenOptions::new().access_mode(0x8000_0000 | 0x0001_0000).share_mode(1 | 4)
            .custom_flags(0x0020_0000 | 0x0010_0000).open(from).map_err(|e| e.to_string())?;
        let m = file.metadata().map_err(|e| e.to_string())?;
        if !m.is_file() || m.file_attributes() & (0x400 | 0x1000 | 0x0004_0000 | 0x0040_0000) != 0 { return Err("An update file is linked or offline".into()); }
        check_hash(&mut file, sha)?;
        checked.push((name.clone(), file));
    }
    remove_old(dir);
    let mut aside: Vec<(PathBuf, std::fs::File)> = Vec::new();
    let mut placed: Vec<usize> = Vec::new();
    // put everything back; says whether that worked
    let undo = |aside: &[(PathBuf, std::fs::File)], placed: &[usize], checked: Vec<(String, std::fs::File)>| -> bool {
        let mut ok = true;
        for (i, (_, file)) in checked.into_iter().enumerate() {
            if placed.contains(&i) { ok &= delete_open(&file); }
            drop(file);
        }
        for (cur, old) in aside {
            ok &= rename_open(old, cur).is_ok();
        }
        ok
    };
    let back = |ok: bool| if ok { "nothing was changed" } else { "and putting the old files back failed too; reinstall Sprawl with its Setup" };
    for (name, _, _) in staged {
        let cur = dir.join(name);
        let current = std::fs::OpenOptions::new().access_mode(0x0001_0080).share_mode(1 | 2 | 4)
            .custom_flags(0x0020_0000 | 0x0010_0000).open(&cur);
        match current {
            Ok(current) => {
            let valid = current.metadata().is_ok_and(|m| m.is_file() && m.file_attributes() & (0x400 | 0x1000 | 0x0004_0000 | 0x0040_0000) == 0);
            if !valid { let ok = undo(&aside, &placed, checked); return Err(format!("The current {name} is linked or offline; {}", back(ok))); }
            let old = dir.join(format!("{name}.old"));
            if let Err(e) = rename_open(&current, &old) {
                let ok = undo(&aside, &placed, checked);
                return Err(format!("could not move {name} aside ({e}); {}", back(ok)));
            }
            aside.push((cur, current));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => { let ok = undo(&aside, &placed, checked); return Err(format!("could not open {name} for updating ({e}); {}", back(ok))); }
        }
    }
    before_publish();
    for i in 0..checked.len() {
        let name = checked[i].0.to_string();
        let to = dir.join(&name);
        if let Err(e) = rename_open(&checked[i].1, &to) {
            let ok = undo(&aside, &placed, checked);
            return Err(format!("could not put the new {name} in place ({e}); {}", back(ok)));
        }
        placed.push(i);
    }
    Ok(())
}

fn set_str(key: HKEY, name: &str, value: &str) {
    let data: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2) };
    unsafe {
        let _ = RegSetValueExW(key, &HSTRING::from(name), None, REG_SZ, Some(bytes));
    }
}

/// After an update: the version in the Apps entry of the install in `dir`. Only an entry that
/// exists and points at `dir` is touched; none is created.
pub fn register(dir: &Path, version: &str) -> Result<(), String> {
    per_user_only()?;
    let key_name = if inno_dir().is_some_and(|d| same(dir, &d)) {
        INNO_KEY
    } else if reg_str(UNINSTALL_KEY, "DisplayVersion").is_some() && install_dir().is_some_and(|d| same(dir, &d)) {
        UNINSTALL_KEY
    } else {
        return Ok(());
    };
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(key_name), None, KEY_WRITE, &mut key) } != ERROR_SUCCESS {
        return Err("could not update the Apps entry".into());
    }
    set_str(key, "DisplayVersion", version);
    if key_name == INNO_KEY {
        set_str(key, "DisplayName", "Sprawl");
    }
    unsafe {
        let _ = RegCloseKey(key);
    }
    Ok(())
}

pub fn message(text: &str, question: bool, warn: bool) -> bool {
    let kind = if question { MB_YESNO | MB_ICONQUESTION } else if warn { MB_OK | MB_ICONWARNING } else { MB_OK | MB_ICONINFORMATION };
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Sprawl"), kind) == IDYES }
}

/// `sprawl.exe --uninstall`, from Settings > Apps for copies set up by 0.2.0-0.2.2's own
/// installer (Inno Setup's copies uninstall with Inno's uninstaller): remove the shortcut, the
/// Apps entry, the Explorer menu entries and the program files. Settings, history and the clean-up log stay
/// (they are in %APPDATA%\Sprawl and %LOCALAPPDATA%\Sprawl). `quiet` skips the questions.
pub fn uninstall(quiet: bool) {
    if let Err(e) = per_user_only() { if !quiet { message(&e, false, true); } return; }
    let Some(dir) = install_dir() else { return };
    if !is_legacy_copy() {
        if !quiet {
            message(&format!("This copy of Sprawl isn't the installed one.\n\nThe installed copy is in {}.", dir.display()), false, true);
        }
        return;
    }
    if !quiet && !message("Remove Sprawl from this PC?\n\nYour settings and scan history are kept, in case you install it again.", true, false) {
        return;
    }
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(UNINSTALL_KEY));
        for k in MENU_KEYS {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(k));
        }
    }
    if let Some(l) = shortcut_path() {
        let _ = std::fs::remove_file(l);
    }
    // the low-space check's scheduled task, if the setting made one
    remove_alert_task();
    // only Sprawl's own files; the folder itself goes only if that leaves it empty
    remove_old(&dir);
    clean_stage(&dir);
    if !quiet {
        message("Sprawl has been removed.", false, false);
    }
    // This program and the DLL it has loaded are in use until it exits, and another Sprawl
    // window may still be open: a short-lived cmd tries every 2 seconds for 2 minutes, then
    // removes the folder if it is empty. cmd and ping by full path, started in System32.
    use std::os::windows::process::CommandExt;
    let sys = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into())).join("System32");
    let dels: String = crate::FILES.iter().map(|n| format!("del /f /q \"{}\" 2>nul & ", dir.join(n).display())).collect();
    let script = format!(
        "for /l %i in (1,1,60) do @(\"{ping}\" -n 3 127.0.0.1 >nul & {dels}rmdir \"{d}\" 2>nul & if not exist \"{d}\" exit)",
        ping = sys.join("ping.exe").display(),
        d = dir.display()
    );
    let _ = std::process::Command::new(sys.join("cmd.exe")).current_dir(&sys).raw_arg("/d /q /c").raw_arg(script).creation_flags(0x0800_0000).spawn();
}

#[cfg(test)]
mod update_tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    fn fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sprawl-update-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(dir.join(STAGE)).unwrap();
        dir
    }

    #[test]
    fn legacy_task_migration_requires_the_exact_principal() {
        let sid = "S-1-5-21-100-200-300-1001";
        assert!(legacy_task_owned(&format!("<Task><Principal><UserId>{sid}</UserId></Principal></Task>"), sid));
        assert!(!legacy_task_owned("<UserId>S-1-5-21-100-200-300-1002</UserId>", sid));
        assert!(!legacy_task_owned("<UserId>COMPUTER\\sameusername</UserId>", sid));
        assert!(!legacy_task_owned(&format!("<Author>{sid}</Author>"), sid));
        assert!(!legacy_task_owned(&format!("<UserId>{sid}</UserId><UserId>other</UserId>"), sid));
        let actual = sprawl_core::identity::account_sid().unwrap();
        assert_eq!(alert_task_name().unwrap(), format!("{LEGACY_ALERT_TASK} {actual}"));
    }

    #[test]
    fn checked_source_is_renamed_even_after_its_name_changes() {
        let dir = fixture("handle");
        let source = dir.join(STAGE).join("sprawl.exe");
        std::fs::write(&source, b"verified replacement").unwrap();
        let mut file = std::fs::OpenOptions::new().access_mode(0x8000_0000 | 0x0001_0000).share_mode(1 | 4).open(&source).unwrap();
        check_hash(&mut file, &crate::sha256_hex(b"verified replacement")).unwrap();
        assert!(std::fs::write(&source, b"changed").is_err());
        let moved = dir.join(STAGE).join("moved.exe");
        std::fs::rename(&source, &moved).unwrap();
        std::fs::write(&source, b"unrelated new arrival").unwrap();
        rename_open(&file, &dir.join("sprawl.exe")).unwrap();
        drop(file);
        assert_eq!(std::fs::read(dir.join("sprawl.exe")).unwrap(), b"verified replacement");
        assert_eq!(std::fs::read(source).unwrap(), b"unrelated new arrival");
        assert!(!moved.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn update_preflight_and_launch_guards_preserve_checked_bytes() {
        let dir = fixture("swap");
        std::fs::write(dir.join("sprawl.exe"), b"old program").unwrap();
        std::fs::write(dir.join(STAGE).join("sprawl.exe"), b"new program").unwrap();
        let mut files = vec![("sprawl.exe".into(), dir.join(STAGE).join("sprawl.exe"), crate::sha256_hex(b"wrong"))];
        assert!(swap_in(&dir, &files).is_err());
        assert_eq!(std::fs::read(dir.join("sprawl.exe")).unwrap(), b"old program");
        if per_user_only().is_ok() {
            files[0].2 = crate::sha256_hex(b"new program");
            swap_in(&dir, &files).unwrap();
            assert_eq!(std::fs::read(dir.join("sprawl.exe.old")).unwrap(), b"old program");
            let launch = verified_launch(&dir, &files).unwrap();
            assert!(std::fs::write(dir.join("sprawl.exe"), b"changed").is_err());
            assert!(std::fs::rename(dir.join("sprawl.exe"), dir.join("displaced.exe")).is_err());
            drop(launch);
            assert_eq!(std::fs::read(dir.join("sprawl.exe")).unwrap(), b"new program");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn maintenance_keeps_unrelated_files_and_stage_subfolders() {
        let dir = fixture("maintenance");
        for name in crate::FILES { std::fs::write(dir.join(name), b"current").unwrap(); }
        for name in ["sprawl.exe.old", "another.exe.old", "notes.old"] { std::fs::write(dir.join(name), b"keep or remove by ownership").unwrap(); }
        for name in ["sprawl.exe", "another.exe", "notes.txt"] { std::fs::write(dir.join(STAGE).join(name), b"staged").unwrap(); }
        std::fs::create_dir(dir.join(STAGE).join("private")).unwrap();
        remove_old(&dir);
        clean_stage(&dir);
        assert!(dir.join("another.exe.old").exists() && dir.join("notes.old").exists());
        assert!(dir.join(STAGE).join("another.exe").exists() && dir.join(STAGE).join("private").exists());
        if per_user_only().is_ok() { assert!(!dir.join("sprawl.exe.old").exists() && !dir.join(STAGE).join("sprawl.exe").exists()); }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_second_publication_keeps_new_arrivals_and_restores_by_handle() {
        if per_user_only().is_err() { return; }
        let dir = fixture("rollback");
        let files: Vec<_> = crate::FILES.iter().map(|&name| {
            std::fs::write(dir.join(name), format!("old {name}")).unwrap();
            let new = format!("new {name}");
            let source = dir.join(STAGE).join(name);
            std::fs::write(&source, &new).unwrap();
            (name.to_string(), source, crate::sha256_hex(new.as_bytes()))
        }).collect();
        let blocked = dir.join(crate::FILES[1]);
        let result = swap_in_with(&dir, &files, || std::fs::write(&blocked, b"unrelated new arrival").unwrap());
        assert!(result.is_err());
        assert_eq!(std::fs::read(dir.join("sprawl.exe")).unwrap(), b"old sprawl.exe");
        assert_eq!(std::fs::read(&blocked).unwrap(), b"unrelated new arrival");
        assert_eq!(std::fs::read(dir.join(format!("{}.old", crate::FILES[1]))).unwrap(), format!("old {}", crate::FILES[1]).as_bytes());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
