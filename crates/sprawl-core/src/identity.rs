//! The Windows account owning this process. Never derived from environment names.
use std::io;

pub fn account_sid() -> io::Result<String> {
    unsafe { token_account_sid(windows_sys::Win32::System::Threading::GetCurrentProcess()) }
}

/// The actual account token of a local peer, independent of its environment or image name.
/// Whether `process` runs elevated (as administrator).
pub fn process_elevated(process: std::os::windows::io::BorrowedHandle<'_>) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    unsafe { token_elevated(process.as_raw_handle()) }
}

pub fn process_account_sid(process: std::os::windows::io::BorrowedHandle<'_>) -> io::Result<String> {
    use std::os::windows::io::AsRawHandle;
    unsafe { token_account_sid(process.as_raw_handle()) }
}

/// Native image-path code units from a held process. Compare these exactly: Windows folders
/// can be case-sensitive, so folding a path is not proof of installation identity.
pub fn process_image_path(process: std::os::windows::io::BorrowedHandle<'_>) -> io::Result<Vec<u16>> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Threading::QueryFullProcessImageNameW;
    let mut path = vec![0u16; 32_768];
    let mut length = path.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process.as_raw_handle(), 1, path.as_mut_ptr(), &mut length) } == 0 { return Err(io::Error::last_os_error()); }
    if length == 0 || length as usize >= path.len() { return Err(io::Error::other("Incomplete process image path")); }
    path.truncate(length as usize);
    Ok(path)
}

/// Whether a process's token is elevated. `process` needs query access.
pub(crate) unsafe fn token_elevated(process: windows_sys::Win32::Foundation::HANDLE) -> io::Result<bool> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows_sys::Win32::System::Threading::OpenProcessToken;
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 { return Err(io::Error::last_os_error()); }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(token, TokenElevation, (&mut elevation as *mut TOKEN_ELEVATION).cast(), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len);
        let error = (ok == 0).then(io::Error::last_os_error);
        CloseHandle(token);
        if let Some(error) = error { return Err(error); }
        if len as usize != std::mem::size_of::<TOKEN_ELEVATION>() { return Err(io::Error::other("Windows returned an incomplete elevation state")); }
        Ok(elevation.TokenIsElevated != 0)
    }
}

/// Whether the desktop shell (Explorer) runs under this process's account: false when an
/// administrator's credentials were typed for Sprawl (another account), None when it can't be
/// told (no shell, no access).
pub fn shell_is_this_account() -> Option<bool> {
    use std::os::windows::io::{AsHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    #[link(name = "user32")]
    extern "system" {
        fn GetShellWindow() -> *mut core::ffi::c_void;
        fn GetWindowThreadProcessId(hwnd: *mut core::ffi::c_void, pid: *mut u32) -> u32;
    }
    let mine = account_sid().ok()?;
    unsafe {
        let shell = GetShellWindow();
        if shell.is_null() {
            return None;
        }
        let mut pid = 0u32;
        if GetWindowThreadProcessId(shell, &mut pid) == 0 || pid == 0 {
            return None;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let h = OwnedHandle::from_raw_handle(h);
        Some(process_account_sid(h.as_handle()).ok()? == mine)
    }
}

/// Start `exe` with `args` as the signed-in user without administrator rights, from an
/// elevated Sprawl. The new process is created with the desktop shell (Explorer) as its parent,
/// so it takes the shell's token. The shell must run under this account and must not itself be
/// elevated; otherwise nothing starts. Arguments are plain words (letters, digits, `.`, `-`).
/// Returns the new process, so a caller can wait for it.
pub fn launch_unelevated(exe: &std::path::Path, args: &[&str]) -> io::Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, CREATE_NO_WINDOW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, OpenProcess, UpdateProcThreadAttribute,
        EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_CREATE_PROCESS, PROCESS_INFORMATION,
        PROCESS_QUERY_LIMITED_INFORMATION, PROC_THREAD_ATTRIBUTE_PARENT_PROCESS, STARTUPINFOEXW,
    };
    #[link(name = "user32")]
    extern "system" {
        fn GetShellWindow() -> *mut core::ffi::c_void;
        fn GetWindowThreadProcessId(hwnd: *mut core::ffi::c_void, pid: *mut u32) -> u32;
    }
    if args.iter().any(|a| a.is_empty() || !a.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Unsupported argument for an unelevated start"));
    }
    let exe_text = exe.as_os_str();
    if exe_text.encode_wide().any(|c| c == u16::from(b'"')) || !exe.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Unsupported program path for an unelevated start"));
    }
    let mine = account_sid()?;
    unsafe {
        let shell = GetShellWindow();
        if shell.is_null() { return Err(io::Error::other("The Windows desktop shell is not running")); }
        let mut pid = 0u32;
        if GetWindowThreadProcessId(shell, &mut pid) == 0 || pid == 0 { return Err(io::Error::other("The Windows desktop shell's process is unknown")); }
        let parent: HANDLE = OpenProcess(PROCESS_CREATE_PROCESS | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if parent.is_null() { return Err(io::Error::last_os_error()); }
        let parent = OwnedHandle::from_raw_handle(parent);
        use std::os::windows::io::{AsHandle, AsRawHandle};
        if process_account_sid(parent.as_handle())? != mine {
            return Err(io::Error::other("The Windows desktop shell runs under another account"));
        }
        if token_elevated(parent.as_raw_handle())? {
            return Err(io::Error::other("The Windows desktop shell runs as administrator"));
        }
        let mut size = 0usize;
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size);
        if size == 0 { return Err(io::Error::last_os_error()); }
        let mut list = vec![0u64; size.div_ceil(8)];
        let attrs = list.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        if InitializeProcThreadAttributeList(attrs, 1, 0, &mut size) == 0 { return Err(io::Error::last_os_error()); }
        let raw_parent: HANDLE = parent.as_raw_handle();
        let updated = UpdateProcThreadAttribute(attrs, 0, PROC_THREAD_ATTRIBUTE_PARENT_PROCESS as usize, (&raw_parent as *const HANDLE).cast(), std::mem::size_of::<HANDLE>(), std::ptr::null_mut(), std::ptr::null());
        if updated == 0 { let e = io::Error::last_os_error(); DeleteProcThreadAttributeList(attrs); return Err(e); }
        let mut cmd: Vec<u16> = std::iter::once(u16::from(b'"')).chain(exe_text.encode_wide()).chain(std::iter::once(u16::from(b'"'))).collect();
        for a in args { cmd.push(u16::from(b' ')); cmd.extend(a.encode_utf16()); }
        cmd.push(0);
        let app: Vec<u16> = exe_text.encode_wide().chain(Some(0)).collect();
        let mut si: STARTUPINFOEXW = std::mem::zeroed();
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si.lpAttributeList = attrs;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(app.as_ptr(), cmd.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0, EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW, std::ptr::null(), std::ptr::null(), &si.StartupInfo, &mut pi);
        let error = (ok == 0).then(io::Error::last_os_error);
        DeleteProcThreadAttributeList(attrs);
        if let Some(e) = error { return Err(e); }
        CloseHandle(pi.hThread);
        Ok(OwnedHandle::from_raw_handle(pi.hProcess))
    }
}

/// Wait for a process started by [`launch_unelevated`]; its exit code, or None on timeout.
pub fn wait_exit(process: &std::os::windows::io::OwnedHandle, timeout_ms: u32) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    unsafe {
        if WaitForSingleObject(process.as_raw_handle(), timeout_ms) != 0 { return None; }
        let mut code = 0u32;
        (GetExitCodeProcess(process.as_raw_handle(), &mut code) != 0).then_some(code)
    }
}

unsafe fn token_account_sid(process: windows_sys::Win32::Foundation::HANDLE) -> io::Result<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::OpenProcessToken;
    #[link(name = "advapi32")]
    extern "system" { fn ConvertSidToStringSidW(sid: *mut core::ffi::c_void, out: *mut *mut u16) -> i32; }
    #[link(name = "kernel32")]
    extern "system" { fn LocalFree(ptr: *mut core::ffi::c_void) -> *mut core::ffi::c_void; }
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 { return Err(io::Error::last_os_error()); }
        let mut buf = [0u64; 64];
        let mut used = 0;
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), std::mem::size_of_val(&buf) as u32, &mut used);
        let error = (ok == 0).then(io::Error::last_os_error);
        CloseHandle(token);
        if let Some(error) = error { return Err(error); }
        if (used as usize) < std::mem::size_of::<TOKEN_USER>() || used as usize > std::mem::size_of_val(&buf) { return Err(io::Error::other("Incomplete Windows account information")); }
        let sid = (*(buf.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        let mut out = std::ptr::null_mut();
        if ConvertSidToStringSidW(sid, &mut out) == 0 { return Err(io::Error::last_os_error()); }
        if out.is_null() { return Err(io::Error::other("Windows returned no account SID")); }
        let mut len = 0;
        while len < 184 && *out.add(len) != 0 { len += 1; }
        let text = String::from_utf16(std::slice::from_raw_parts(out, len));
        LocalFree(out.cast());
        let text = text.map_err(|_| io::Error::other("Invalid account SID text"))?;
        if len == 184 || !text.starts_with("S-1-") || !text[4..].bytes().all(|b| b.is_ascii_digit() || b == b'-') { return Err(io::Error::other("Invalid Windows account SID")); }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    /// Needs a desktop shell; run by hand from an elevated and a normal prompt.
    #[test]
    #[ignore = "starts a process through the desktop shell"]
    fn unelevated_launch_runs_as_this_account_without_elevation() {
        use std::os::windows::io::{AsHandle, AsRawHandle};
        let ping = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32").join("PING.EXE");
        let child = super::launch_unelevated(&ping, &["-n", "3", "127.0.0.1"]).unwrap();
        assert!(!unsafe { super::token_elevated(child.as_raw_handle()) }.unwrap());
        assert_eq!(super::process_account_sid(child.as_handle()).unwrap(), super::account_sid().unwrap());
        assert_eq!(super::wait_exit(&child, 20_000), Some(0));
        assert!(super::launch_unelevated(&ping, &["a b"]).is_err());
        assert!(super::launch_unelevated(&ping, &["\"x"]).is_err());
        assert!(super::launch_unelevated(std::path::Path::new("PING.EXE"), &[]).is_err());
        println!("this process elevated: {:?}", crate::elevation());
    }
}
