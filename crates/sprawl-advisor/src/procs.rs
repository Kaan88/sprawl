//! Which programs are running in this Windows session, by file name: for a cleaner's "is the
//! application open" check. Read-only; Sprawl never ends a process.

use std::collections::HashSet;

/// Lower-case file names (`discord.exe`) of the processes in this session, or `None` when
/// Windows couldn't say (a caller treats that as "open").
pub fn running() -> Option<HashSet<String>> {
    #[cfg(windows)]
    {
        win::running()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Whether any of `names` (`Discord.exe`) is running, by [`running`]; `None` when unknown.
pub fn any_running(names: &[String], now: Option<&HashSet<String>>) -> Option<bool> {
    let now = now?;
    Some(names.iter().any(|n| now.contains(&n.to_lowercase())))
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    pub(super) fn running() -> Option<HashSet<String>> {
        let mut mine = 0u32;
        // SAFETY: plain calls with a valid out pointer.
        if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut mine) } == 0 {
            return None;
        }
        // SAFETY: a snapshot handle, closed below.
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return None;
        }
        let mut out = HashSet::new();
        // SAFETY: the entry is zeroed with its size set, as the API asks.
        let mut e: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = unsafe { Process32FirstW(snap, &mut e) } != 0;
        let mut any = false;
        while ok {
            any = true;
            let mut session = u32::MAX;
            // a process of another session (another user) doesn't hold this user's files; one
            // whose session can't be read is counted, to be on the safe side
            let same = unsafe { ProcessIdToSessionId(e.th32ProcessID, &mut session) } == 0 || session == mine;
            if same {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                out.insert(String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase());
            }
            ok = unsafe { Process32NextW(snap, &mut e) } != 0;
        }
        let complete = unsafe { GetLastError() } == ERROR_NO_MORE_FILES;
        // SAFETY: the handle from above.
        unsafe { CloseHandle(snap) };
        (any && complete).then_some(out)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn this_process_is_running() {
        let now = super::running().expect("the process list");
        let me = std::env::current_exe().unwrap().file_name().unwrap().to_string_lossy().to_lowercase();
        assert!(now.contains(&me), "{me}");
        assert_eq!(super::any_running(&[me.to_uppercase()], Some(&now)), Some(true));
        assert_eq!(super::any_running(&["no-such-program.exe".into()], Some(&now)), Some(false));
        assert_eq!(super::any_running(&["x.exe".into()], None), None);
    }
}
