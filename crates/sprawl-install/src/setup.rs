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
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.strip_suffix(".old").is_some_and(crate::manifest::valid_name) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Where an update is downloaded to before it is swapped in.
pub const STAGE: &str = ".sprawl-update";

/// Empty the staging folder: only file names an update could have written, then the folder
/// itself if that leaves it empty (never recursive, never through a link).
pub fn clean_stage(dir: &Path) {
    let st = dir.join(STAGE);
    if is_link(&st) {
        let _ = std::fs::remove_dir(&st);
        return;
    }
    if let Ok(rd) = std::fs::read_dir(&st) {
        for e in rd.flatten() {
            if crate::manifest::valid_name(&e.file_name().to_string_lossy()) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let _ = std::fs::remove_dir(&st);
}

/// A junction, symbolic link or other reparse point.
pub fn is_link(p: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0)
}

/// Put `staged` files (name, path of the new file on the same drive) into `dir`. Each current
/// file is moved aside to `<name>.old` first; if any step fails, everything is put back as it
/// was. Only names in [`crate::FILES`] are written.
pub fn swap_in(dir: &Path, staged: &[(String, PathBuf)]) -> Result<(), String> {
    if let Some((n, _)) = staged.iter().find(|(n, _)| !crate::manifest::valid_name(n)) {
        return Err(format!("refused to write {n}"));
    }
    remove_old(dir);
    let mut aside: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut placed: Vec<PathBuf> = Vec::new();
    // put everything back; says whether that worked
    let undo = |aside: &[(PathBuf, PathBuf)], placed: &[PathBuf]| -> bool {
        let mut ok = true;
        for p in placed {
            ok &= std::fs::remove_file(p).is_ok();
        }
        for (cur, old) in aside {
            ok &= std::fs::rename(old, cur).is_ok();
        }
        ok
    };
    let back = |ok: bool| if ok { "nothing was changed" } else { "and putting the old files back failed too; reinstall Sprawl with its Setup" };
    for (name, _) in staged {
        let cur = dir.join(name);
        if cur.exists() {
            let old = dir.join(format!("{name}.old"));
            if let Err(e) = std::fs::rename(&cur, &old) {
                let ok = undo(&aside, &placed);
                return Err(format!("could not move {name} aside ({e}); {}", back(ok)));
            }
            aside.push((cur, old));
        }
    }
    for (name, from) in staged {
        let to = dir.join(name);
        if let Err(e) = std::fs::rename(from, &to).or_else(|_| std::fs::copy(from, &to).map(|_| ())) {
            // a copy that failed half-way leaves a partial file
            let _ = std::fs::remove_file(&to);
            let ok = undo(&aside, &placed);
            return Err(format!("could not put the new {name} in place ({e}); {}", back(ok)));
        }
        placed.push(to);
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
    {
        use std::os::windows::process::CommandExt;
        let sys = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into())).join("System32");
        let _ = std::process::Command::new(sys.join("schtasks.exe"))
            .current_dir(&sys)
            .args(["/Delete", "/TN", "Sprawl low-space check", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
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
