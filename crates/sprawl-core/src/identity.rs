//! The Windows account owning this process. Never derived from environment names.
use std::io;

pub fn account_sid() -> io::Result<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    #[link(name = "advapi32")]
    extern "system" { fn ConvertSidToStringSidW(sid: *mut core::ffi::c_void, out: *mut *mut u16) -> i32; }
    #[link(name = "kernel32")]
    extern "system" { fn LocalFree(ptr: *mut core::ffi::c_void) -> *mut core::ffi::c_void; }
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 { return Err(io::Error::last_os_error()); }
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
