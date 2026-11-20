//! Installed for all users: Sprawl in Program Files, which only administrators can change.
//!
//! Sprawl runs as administrator only from such a copy. A per-user copy (in
//! `%LOCALAPPDATA%\Programs\Sprawl`) can be changed by any program running as the user, so
//! starting it elevated would hand that program administrator rights at the next UAC prompt.
//! A copy in Program Files updates by running the release's Setup, which is checked against
//! the signed manifest first (see the app's update.rs).

use std::path::{Path, PathBuf};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

/// Inno Setup's per-user Apps entry, from 1.0.4 and before.
const PER_USER_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Sprawl_is1";

/// The Program Files folder, from Windows (not from the environment, which the user sets).
pub fn program_files() -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

fn lower(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches('\\').to_lowercase()
}

/// Whether `dir` is a folder right inside `program_files`, not a link.
pub fn is_protected_dir(dir: &Path, program_files: &Path) -> bool {
    dir.parent().is_some_and(|p| lower(p) == lower(program_files)) && !crate::setup::is_link(dir)
}

/// Whether this copy runs from Program Files, where only administrators can change it.
pub fn is_protected_copy() -> bool {
    let Some(here) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else { return false };
    program_files().is_some_and(|pf| is_protected_dir(&here, &pf))
}

/// Whether this copy may start itself as administrator: from Program Files, or a debug build
/// (development only; a release build never takes this from the environment).
pub fn may_elevate() -> bool {
    cfg!(debug_assertions) || is_protected_copy()
}

/// Why this copy won't start as administrator, for the user.
pub const WHY_NOT_ELEVATED: &str = "Sprawl runs as administrator only when it is installed for all users, in Program Files. This copy is in your own folder, where any program running as you could change it, and that program would then get administrator rights too.";

/// A per-user install from 1.0.4 or before, other than this copy: its folder and uninstaller.
pub fn per_user_install() -> Option<(PathBuf, PathBuf)> {
    let dir = crate::setup::reg_str(PER_USER_KEY, "InstallLocation").map(|s| PathBuf::from(s.trim().trim_end_matches('\\')))?;
    let unins = crate::setup::reg_str(PER_USER_KEY, "UninstallString")?;
    let unins = PathBuf::from(unins.trim().trim_matches('"'));
    // only Inno's uninstaller in that same folder
    let name = unins.file_name()?.to_string_lossy().to_lowercase();
    let ok = unins.parent().is_some_and(|p| lower(p) == lower(&dir)) && name.starts_with("unins") && name.ends_with(".exe");
    let here = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    (ok && here.is_none_or(|h| lower(&h) != lower(&dir))).then_some((dir, unins))
}

/// Remove the per-user install a copy in Program Files replaces: its own uninstaller, run
/// without administrator rights (it is in a folder the user can change, so never elevated),
/// silently. Waits until its Apps entry is gone, at most `wait`.
pub fn remove_per_user_install(wait: std::time::Duration) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    crate::setup::per_user_only()?;
    if !is_protected_copy() {
        return Err("only a copy installed for all users replaces the per-user one".into());
    }
    let Some((_, unins)) = per_user_install() else { return Ok(()) };
    std::process::Command::new(&unins)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"])
        .creation_flags(0x0800_0000)
        .spawn()
        .map_err(|e| format!("the old per-user Sprawl's uninstaller didn't start ({e})"))?;
    // Inno's uninstaller hands over to a copy of itself and the first one ends at once: watch
    // the Apps entry instead
    let until = std::time::Instant::now() + wait;
    while std::time::Instant::now() < until {
        if per_user_install().is_none() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    Err("the old per-user Sprawl is still installed; remove it in Settings > Apps".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_folder_right_in_program_files_is_protected() {
        let pf = Path::new(r"C:\Program Files");
        assert!(is_protected_dir(Path::new(r"C:\Program Files\Sprawl"), pf));
        assert!(is_protected_dir(Path::new(r"c:\program files\Sprawl\"), Path::new(r"C:\Program Files\")));
        assert!(!is_protected_dir(Path::new(r"C:\Users\me\AppData\Local\Programs\Sprawl"), pf));
        assert!(!is_protected_dir(Path::new(r"C:\Program Files\Sprawl\sub"), pf));
        assert!(!is_protected_dir(Path::new(r"C:\Program Files (x86)\Sprawl"), pf));
        assert!(program_files().is_some_and(|p| p.is_absolute()));
    }
}
