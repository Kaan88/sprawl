//! The install folder, swapping files in, the Start menu shortcut and the Apps entry.

use std::path::{Path, PathBuf};

use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE,
    REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_YESNO};

const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Sprawl";
/// The Explorer "Scan with Sprawl" entries (see the app's dialogs.rs); removed on uninstall.
const MENU_KEYS: [&str; 2] = [r"Software\Classes\Directory\shell\Sprawl", r"Software\Classes\Drive\shell\Sprawl"];

/// `%LOCALAPPDATA%\Programs\Sprawl`.
pub fn install_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Programs").join("Sprawl"))
}

/// `...\Start Menu\Programs\Sprawl.lnk` for this user.
fn shortcut_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join(r"Microsoft\Windows\Start Menu\Programs\Sprawl.lnk"))
}

fn same(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase();
    n(a) == n(b)
}

/// Whether this program runs from the install folder.
pub fn is_installed_copy() -> bool {
    match (std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)), install_dir()) {
        (Some(here), Some(dir)) => same(&here, &dir),
        _ => false,
    }
}

/// The version in the Apps entry, if Sprawl is installed.
pub fn installed_version() -> Option<String> {
    let mut key = HKEY::default();
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(UNINSTALL_KEY), None, KEY_READ, &mut key) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = [0u16; 64];
        let mut size = (buf.len() * 2) as u32;
        let r = RegQueryValueExW(key, &HSTRING::from("DisplayVersion"), None, None, Some(buf.as_mut_ptr().cast()), Some(&mut size));
        let _ = RegCloseKey(key);
        (r == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).min(buf.len())]).trim_end_matches('\0').to_string())
    }
}

/// Remove the `.old` files a previous swap left (they may still be in use until now).
pub fn remove_old(dir: &Path) {
    for name in crate::FILES {
        let _ = std::fs::remove_file(dir.join(format!("{name}.old")));
    }
}

/// Put `staged` files (name, path of the new file on the same drive) into `dir`. Each current
/// file is moved aside to `<name>.old` first; if any step fails, everything is put back as it
/// was. Only names in [`crate::FILES`] are written.
pub fn swap_in(dir: &Path, staged: &[(String, PathBuf)]) -> Result<(), String> {
    if let Some((n, _)) = staged.iter().find(|(n, _)| !crate::FILES.contains(&n.as_str())) {
        return Err(format!("refused to write {n}"));
    }
    remove_old(dir);
    let mut aside: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut placed: Vec<PathBuf> = Vec::new();
    let undo = |aside: &[(PathBuf, PathBuf)], placed: &[PathBuf]| {
        for p in placed {
            let _ = std::fs::remove_file(p);
        }
        for (cur, old) in aside {
            let _ = std::fs::rename(old, cur);
        }
    };
    for (name, _) in staged {
        let cur = dir.join(name);
        if cur.exists() {
            let old = dir.join(format!("{name}.old"));
            if let Err(e) = std::fs::rename(&cur, &old) {
                undo(&aside, &placed);
                return Err(format!("could not move {name} aside: {e}"));
            }
            aside.push((cur, old));
        }
    }
    for (name, from) in staged {
        let to = dir.join(name);
        if let Err(e) = std::fs::rename(from, &to).or_else(|_| std::fs::copy(from, &to).map(|_| ())) {
            undo(&aside, &placed);
            return Err(format!("could not put the new {name} in place: {e}"));
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

fn set_dword(key: HKEY, name: &str, value: u32) {
    unsafe {
        let _ = RegSetValueExW(key, &HSTRING::from(name), None, REG_DWORD, Some(&value.to_le_bytes()));
    }
}

/// Write the Apps entry for the install in `dir`.
pub fn register(dir: &Path, version: &str) -> Result<(), String> {
    let mut key = HKEY::default();
    let r = unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, &HSTRING::from(UNINSTALL_KEY), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
    };
    if r != ERROR_SUCCESS {
        return Err("could not write the Apps entry".into());
    }
    let exe = dir.join("sprawl.exe");
    let size_kb = crate::FILES.iter().filter_map(|n| std::fs::metadata(dir.join(n)).ok()).map(|m| m.len()).sum::<u64>() / 1024;
    set_str(key, "DisplayName", "Sprawl");
    set_str(key, "DisplayVersion", version);
    set_str(key, "Publisher", "Sprawl");
    set_str(key, "DisplayIcon", &format!("{},0", exe.display()));
    set_str(key, "InstallLocation", &dir.display().to_string());
    set_str(key, "UninstallString", &format!("\"{}\" --uninstall", exe.display()));
    set_str(key, "URLInfoAbout", &format!("https://github.com/{}", crate::REPO));
    set_dword(key, "EstimatedSize", size_kb.min(u32::MAX as u64) as u32);
    set_dword(key, "NoModify", 1);
    set_dword(key, "NoRepair", 1);
    unsafe {
        let _ = RegCloseKey(key);
    }
    Ok(())
}

/// Create the Start menu shortcut to `exe`.
pub fn make_shortcut(exe: &Path) -> Result<(), String> {
    let lnk = shortcut_path().ok_or("no Start menu folder")?;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        link.SetPath(&HSTRING::from(exe.as_os_str())).map_err(|e| e.to_string())?;
        if let Some(d) = exe.parent() {
            let _ = link.SetWorkingDirectory(&HSTRING::from(d.as_os_str()));
        }
        let _ = link.SetDescription(&HSTRING::from("Disk space analyser"));
        let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        file.Save(&HSTRING::from(lnk.as_os_str()), true).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Install `files` (name, contents) as `version` into the install folder: swapped in, so a
/// running copy keeps working until it restarts. Returns the installed `sprawl.exe`.
pub fn install(files: &[(&str, &[u8])], version: &str) -> Result<PathBuf, String> {
    let dir = install_dir().ok_or("no %LOCALAPPDATA%")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let stage = dir.join(".new");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let mut staged = Vec::new();
    for (name, bytes) in files {
        let p = stage.join(name);
        std::fs::write(&p, bytes).map_err(|e| format!("could not write {name}: {e}"))?;
        staged.push((name.to_string(), p));
    }
    let r = swap_in(&dir, &staged);
    let _ = std::fs::remove_dir_all(&stage);
    r?;
    register(&dir, version)?;
    let exe = dir.join("sprawl.exe");
    // a shortcut that fails isn't worth failing the install for
    let _ = make_shortcut(&exe);
    Ok(exe)
}

pub fn message(text: &str, question: bool, warn: bool) -> bool {
    let kind = if question { MB_YESNO | MB_ICONQUESTION } else if warn { MB_OK | MB_ICONWARNING } else { MB_OK | MB_ICONINFORMATION };
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Sprawl"), kind) == IDYES }
}

/// `sprawl.exe --uninstall`, from Settings > Apps: remove the shortcut, the Apps entry, the
/// Explorer menu entries and the program files. Settings, history and the clean-up log stay
/// (they are in %APPDATA%\Sprawl and %LOCALAPPDATA%\Sprawl). `quiet` skips the questions.
pub fn uninstall(quiet: bool) {
    let Some(dir) = install_dir() else { return };
    if !is_installed_copy() {
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
    // only Sprawl's own files; the folder itself goes only if that leaves it empty
    for name in crate::FILES {
        let _ = std::fs::remove_file(dir.join(format!("{name}.old")));
    }
    let _ = std::fs::remove_dir_all(dir.join(".new"));
    let _ = std::fs::remove_dir_all(dir.join(".update"));
    // this program and the DLL it has loaded are in use: a short-lived cmd deletes them once
    // it has closed
    use std::os::windows::process::CommandExt;
    let dels: String = crate::FILES.iter().map(|n| format!("del /f /q \"{}\" & ", dir.join(n).display())).collect();
    let script = format!("ping -n 3 127.0.0.1 >nul & {dels}rmdir \"{}\"", dir.display());
    let _ = std::process::Command::new("cmd.exe").raw_arg("/d /c").raw_arg(script).creation_flags(0x0800_0000).spawn();
    if !quiet {
        message("Sprawl has been removed.", false, false);
    }
}
