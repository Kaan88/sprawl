//! Shell dialogs and integration: folder and file pickers, restarting elevated, and the
//! "Scan with Sprawl" entry in Explorer's right-click menu (per user, no admin needed).

use std::path::PathBuf;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE,
    REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, ShellExecuteW, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST,
    FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

fn hwnd(h: isize) -> HWND {
    HWND(h as *mut _)
}

fn open_dialog(owner: isize, title: &str, folders: bool, multi: bool, csv: bool) -> Vec<PathBuf> {
    let run = || -> windows::core::Result<Vec<PathBuf>> {
        unsafe {
            let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            let mut opts = d.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST;
            if folders {
                opts |= FOS_PICKFOLDERS;
            } else {
                opts |= FOS_FILEMUSTEXIST;
            }
            if multi {
                opts |= FOS_ALLOWMULTISELECT;
            }
            d.SetOptions(opts)?;
            d.SetTitle(&HSTRING::from(title))?;
            if csv {
                let specs = [
                    COMDLG_FILTERSPEC { pszName: w!("CSV files (WizTree or Sprawl export)"), pszSpec: w!("*.csv") },
                    COMDLG_FILTERSPEC { pszName: w!("All files"), pszSpec: w!("*.*") },
                ];
                d.SetFileTypes(&specs)?;
            }
            if d.Show(Some(hwnd(owner))).is_err() {
                return Ok(Vec::new()); // cancelled
            }
            let items = d.GetResults()?;
            let mut out = Vec::new();
            for i in 0..items.GetCount()? {
                let p = items.GetItemAt(i)?.GetDisplayName(SIGDN_FILESYSPATH)?;
                out.push(PathBuf::from(p.to_string().unwrap_or_default()));
                CoTaskMemFree(Some(p.0 as *const _));
            }
            Ok(out)
        }
    };
    run().unwrap_or_else(|e| {
        eprintln!("dialog: {e}");
        Vec::new()
    })
}

/// Pick one folder, or several with `multi`.
pub fn pick_folders(owner: isize, multi: bool) -> Vec<PathBuf> {
    let title = if multi { "Select folders or drives to scan together" } else { "Select a folder to scan" };
    open_dialog(owner, title, true, multi, false)
}

pub fn pick_csv(owner: isize) -> Option<PathBuf> {
    open_dialog(owner, "Open a CSV file exported by WizTree or Sprawl", false, false, true).into_iter().next()
}

/// Start this program again elevated (the UAC prompt appears) with `args`. False when the
/// user declined or it failed.
pub fn restart_elevated(owner: isize, args: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let r = unsafe {
        ShellExecuteW(Some(hwnd(owner)), w!("runas"), &HSTRING::from(exe.as_os_str()), &HSTRING::from(args), PCWSTR::null(), SW_SHOWNORMAL)
    };
    r.0 as isize > 32
}

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemDirectoryW(buf: *mut u16, size: u32) -> u32;
    fn WaitForSingleObject(h: isize, ms: u32) -> u32;
    fn CloseHandle(h: isize) -> i32;
}

/// Windows' System32 folder, from Windows itself (not from an environment variable).
pub fn system_dir() -> Option<PathBuf> {
    let mut buf = [0u16; 520];
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// A started process, watched until it ends.
pub struct Running(isize);

impl Running {
    pub fn ended(&self) -> bool {
        // WAIT_TIMEOUT (258) is the only "still running"; a failed wait mustn't block for good
        unsafe { WaitForSingleObject(self.0, 0) != 258 }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Standard base64 of UTF-16LE text: what PowerShell's -EncodedCommand takes.
fn encoded(text: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                out.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Run PowerShell `command` in a new, visible window, elevated when `admin` (the UAC prompt
/// appears). The command travels on the command line itself (-EncodedCommand): no file that
/// another program could change between the prompt and the run. It starts in System32, so no
/// tool reads its settings from whatever folder Sprawl was started in. None when it couldn't
/// start or the user declined elevation.
pub fn run_powershell(owner: isize, command: &str, admin: bool) -> Option<Running> {
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    let sys = system_dir()?;
    let ps = sys.join(r"WindowsPowerShell\v1.0\powershell.exe");
    let args = format!("-NoProfile -ExecutionPolicy Bypass -EncodedCommand {}", encoded(command));
    if args.len() > 32_000 {
        return None;
    }
    let (file, params, dir) = (HSTRING::from(ps.as_os_str()), HSTRING::from(args), HSTRING::from(sys.as_os_str()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: hwnd(owner),
        lpVerb: if admin { w!("runas") } else { w!("open") },
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        lpDirectory: PCWSTR(dir.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.ok()?;
    let h = info.hProcess.0 as isize;
    (h != 0).then_some(Running(h))
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_utf16() {
        // [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('dir'))
        assert_eq!(super::encoded("dir"), "ZABpAHIA");
        assert_eq!(super::encoded("ab"), "YQBiAA==");
    }

    /// The wrapper Run now uses: a script with param() run as a block with -Run, from
    /// System32, with its finally part after it (Windows PowerShell 5.1).
    #[test]
    fn encoded_wrapper_runs() {
        let script = "param([switch]$Run, [string]$Report)
if ($Run) { Write-Output ('ran in ' + (Get-Location).Path) } else { Write-Output 'dry' }";
        let command = format!("Set-Location -LiteralPath ([Environment]::SystemDirectory)
try {{
& {{
{script}
}} -Run
}} finally {{
Write-Output 'closing'
}}");
        let ps = super::system_dir().unwrap().join(r"WindowsPowerShell\v1.0\powershell.exe");
        let out = std::process::Command::new(ps).args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &super::encoded(&command)]).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
        assert!(text.contains(r"ran in c:\windows\system32") && text.contains("closing"), "{text}");
    }
}

/// Command-line argument quoting for a path.
pub fn quote(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    // A trailing backslash would escape the closing quote ("C:\" -> C:").
    let s = if s.ends_with('\\') { format!("{s}\\") } else { s };
    format!("\"{s}\"")
}

const MENU_KEYS: [&str; 2] = [r"Software\Classes\Directory\shell\Sprawl", r"Software\Classes\Drive\shell\Sprawl"];

fn set_value(key: HKEY, name: PCWSTR, value: &str) -> bool {
    let data: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2) };
    unsafe { RegSetValueExW(key, name, None, REG_SZ, Some(bytes)) == ERROR_SUCCESS }
}

fn create(path: &str) -> Option<HKEY> {
    let mut key = HKEY::default();
    let r = unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
    };
    (r == ERROR_SUCCESS).then_some(key)
}

/// Add or remove "Scan with Sprawl" on folders and drives in Explorer.
pub fn set_explorer_menu(on: bool) -> bool {
    if !on {
        for k in MENU_KEYS {
            unsafe {
                let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(k));
            }
        }
        return true;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = exe.display().to_string();
    let mut ok = true;
    for k in MENU_KEYS {
        let Some(key) = create(k) else { return false };
        ok &= set_value(key, PCWSTR::null(), "Scan with Sprawl");
        ok &= set_value(key, w!("Icon"), &format!("\"{exe}\",0"));
        unsafe {
            let _ = RegCloseKey(key);
        }
        let Some(cmd) = create(&format!(r"{k}\command")) else { return false };
        ok &= set_value(cmd, PCWSTR::null(), &format!("\"{exe}\" \"%V\""));
        unsafe {
            let _ = RegCloseKey(cmd);
        }
    }
    ok
}

/// Move a "Scan with DiskTree" entry left by the app's old name over to the new name.
pub fn migrate_explorer_menu() {
    let old = r"Software\Classes\Directory\shell\DiskTree";
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(old), None, KEY_READ, &mut key) } != ERROR_SUCCESS {
        return;
    }
    unsafe {
        let _ = RegCloseKey(key);
        for k in [old, r"Software\Classes\Drive\shell\DiskTree"] {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(k));
        }
    }
    set_explorer_menu(true);
}

pub fn explorer_menu_installed() -> bool {
    let mut key = HKEY::default();
    let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(MENU_KEYS[0]), None, KEY_READ, &mut key) };
    if r == ERROR_SUCCESS {
        unsafe {
            let _ = RegCloseKey(key);
        }
        true
    } else {
        false
    }
}

