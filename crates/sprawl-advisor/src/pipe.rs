//! A local named pipe between `sprawl --mcp` and the running app. One JSON request per line,
//! one JSON answer per line. Local clients only; the first instance is ours or nothing.

use std::io::{BufRead, BufReader, Read, Write};

/// `\\.\pipe\sprawl-mcp-<user>`: one per Windows user.
pub fn name() -> std::io::Result<String> {
    Ok(format!(r"\\.\pipe\sprawl-mcp-{}", sprawl_core::identity::account_sid()?))
}

#[cfg(windows)]
mod ffi {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        pub fn CreateNamedPipeW(name: *const u16, open_mode: u32, pipe_mode: u32, max: u32, out_buf: u32, in_buf: u32, timeout: u32, sa: *const c_void) -> isize;
        pub fn ConnectNamedPipe(h: isize, overlapped: *mut c_void) -> i32;
        pub fn DisconnectNamedPipe(h: isize) -> i32;
        pub fn FlushFileBuffers(h: isize) -> i32;
        pub fn CloseHandle(h: isize) -> i32;
        pub fn GetNamedPipeServerProcessId(h: isize, pid: *mut u32) -> i32;
        pub fn GetNamedPipeClientProcessId(h: isize, pid: *mut u32) -> i32;
        pub fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        pub fn GetCurrentThread() -> isize;
    }
    #[link(name = "advapi32")]
    extern "system" {
        pub fn ImpersonateNamedPipeClient(h: isize) -> i32;
        pub fn RevertToSelf() -> i32;
        pub fn OpenThreadToken(thread: isize, access: u32, as_self: i32, token: *mut isize) -> i32;
        pub fn GetTokenInformation(token: *mut c_void, class: i32, info: *mut c_void, len: u32, used: *mut u32) -> i32; // as recycle.rs declares it
        pub fn ConvertSidToStringSidW(sid: *mut c_void, out: *mut *mut u16) -> i32;
    }
    pub const TOKEN_QUERY: u32 = 0x0008;
    pub const TOKEN_USER: i32 = 1;
    pub const TOKEN_ELEVATION: i32 = 20;
    #[repr(C)]
    pub struct SecurityAttributes { pub length: u32, pub descriptor: *mut c_void, pub inherit: i32 }
    #[link(name = "advapi32")]
    extern "system" {
        pub fn ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl: *const u16, revision: u32, out: *mut *mut c_void, size: *mut u32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" { pub fn LocalFree(ptr: *mut c_void) -> *mut c_void; }
    pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    /// SECURITY_IDENTIFICATION: the server may learn who connects, not act as them.
    pub const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;
    pub const ACCESS_DUPLEX: u32 = 3;
    pub const FIRST_INSTANCE: u32 = 0x0008_0000;
    pub const REJECT_REMOTE: u32 = 8;
    pub const ERROR_PIPE_CONNECTED: i32 = 535;
}

/// Serve requests on the pipe forever, one client at a time: `answer` turns each request line
/// into an answer line. Returns only if the pipe can't be made (another app instance has it).
#[cfg(windows)]
/// Serve the app's pipe. `answer` gets each request line, and whether the client process runs
/// as administrator (false when Windows can't say).
pub fn serve(mut answer: impl FnMut(&str, bool) -> String) -> std::io::Result<()> {
    use std::os::windows::io::FromRawHandle;
    let sid = sprawl_core::identity::account_sid()?;
    let wide: Vec<u16> = name()?.encode_utf16().chain(Some(0)).collect();
    // A protected DACL admits only this account. No inherited Everyone/Admins grants.
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})").encode_utf16().chain(Some(0)).collect();
    let mut descriptor = std::ptr::null_mut();
    if unsafe { ffi::ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, std::ptr::null_mut()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let sa = ffi::SecurityAttributes { length: std::mem::size_of::<ffi::SecurityAttributes>() as u32, descriptor, inherit: 0 };
    let h = unsafe { ffi::CreateNamedPipeW(wide.as_ptr(), ffi::ACCESS_DUPLEX | ffi::FIRST_INSTANCE, ffi::REJECT_REMOTE, 1, 64 << 10, 64 << 10, 0, (&sa as *const ffi::SecurityAttributes).cast()) };
    let error = (h == -1).then(std::io::Error::last_os_error);
    unsafe { ffi::LocalFree(descriptor); }
    if h == -1 {
        return Err(error.unwrap());
    }
    loop {
        let ok = unsafe { ffi::ConnectNamedPipe(h, std::ptr::null_mut()) } != 0 || std::io::Error::last_os_error().raw_os_error() == Some(ffi::ERROR_PIPE_CONNECTED);
        let client = if ok { peer(h, false) } else { None };
        if let Some(client_elevated) = client {
            // the handle stays ours: the File is never dropped
            let file = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_handle(h as _) });
            let mut reader = BufReader::new(&*file);
            let mut line = String::new();
            // a line is at most 1 MB: a client that never sends a newline can't fill memory
            while (&mut reader).take(1 << 20).read_line(&mut line).is_ok_and(|n| n > 0) {
                if !line.ends_with('\n') {
                    break;
                }
                // Who wrote this message: the pipe's own record of the client's security
                // context, not only the process behind a client PID (which could have exited and
                // been reused). Administrator only when both say so; another account, or no
                // answer, ends the connection.
                let mut out = match message_identity(h) {
                    Some((account, elevated)) if account == sid => answer(line.trim(), client_elevated && elevated),
                    _ => {
                        let _ = (&*file).write_all(b"{\"ok\":false,\"text\":\"Sprawl couldn't confirm who is asking.\"}\n");
                        break;
                    }
                }.replace(['\r', '\n'], " ");
                out.push('\n');
                if (&*file).write_all(out.as_bytes()).is_err() {
                    break;
                }
                line.clear();
            }
            unsafe {
                ffi::FlushFileBuffers(h);
            }
        }
        unsafe {
            ffi::DisconnectNamedPipe(h);
        }
    }
    #[allow(unreachable_code)]
    {
        unsafe { ffi::CloseHandle(h) };
        Ok(())
    }
}

/// A connection to the app's pipe.
pub struct Client {
    reader: BufReader<std::fs::File>,
    writer: std::fs::File,
}

/// The process at the other end of the pipe runs the same program file as this one.
#[cfg(windows)]
fn server_is_us(f: &std::fs::File) -> bool {
    use std::os::windows::io::AsRawHandle;
    peer_is_us(f.as_raw_handle() as isize, true)
}

#[cfg(windows)]
fn peer_is_us(pipe: isize, server: bool) -> bool {
    peer(pipe, server).is_some()
}

/// The other end runs this same program for this same account: whether it runs as
/// administrator (false when Windows can't say). None when it isn't us.
#[cfg(windows)]
fn peer(pipe: isize, server: bool) -> Option<bool> {
    let mut pid = 0u32;
    let ok = unsafe { if server { ffi::GetNamedPipeServerProcessId(pipe, &mut pid) } else { ffi::GetNamedPipeClientProcessId(pipe, &mut pid) } };
    if ok == 0 {
        return None;
    }
    let h = unsafe { ffi::OpenProcess(ffi::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h == 0 {
        return None;
    }
    // Hold the same process while querying both token and image: no PID reuse between them.
    let peer = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(h as _) };
    let same_account = sprawl_core::identity::account_sid().ok().zip(sprawl_core::identity::process_account_sid(peer).ok()).is_some_and(|(ours, theirs)| ours == theirs);
    let theirs = sprawl_core::identity::process_image_path(peer);
    let current = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(windows_sys::Win32::System::Threading::GetCurrentProcess()) };
    let ours = sprawl_core::identity::process_image_path(current);
    let elevated = sprawl_core::identity::process_elevated(peer).unwrap_or(false);
    unsafe { ffi::CloseHandle(h) };
    (same_account && ours.ok().zip(theirs.ok()).is_some_and(|(ours, theirs)| ours == theirs)).then_some(elevated)
}

/// The account (SID text) and elevation of the security context the client's last message on
/// `pipe` was written with, at identification level (the client connects with
/// SECURITY_IDENTIFICATION: Sprawl can learn who it is, not act as it). None when Windows can't
/// say. Sprawl never goes on running as the client: a failed revert ends the process.
#[cfg(windows)]
fn message_identity(pipe: isize) -> Option<(String, bool)> {
    // SAFETY: the impersonation is undone before anything else runs on this thread; the token
    // handle and the SID text are released here.
    unsafe {
        if ffi::ImpersonateNamedPipeClient(pipe) == 0 { return None; }
        let mut token = 0isize;
        let opened = ffi::OpenThreadToken(ffi::GetCurrentThread(), ffi::TOKEN_QUERY, 1, &mut token) != 0;
        if ffi::RevertToSelf() == 0 { std::process::abort(); }
        if !opened { return None; }
        let out = token_identity(token);
        ffi::CloseHandle(token);
        out
    }
}

/// SAFETY: `token` is an open token handle with TOKEN_QUERY.
#[cfg(windows)]
unsafe fn token_identity(token: isize) -> Option<(String, bool)> {
    unsafe {
        let mut buf = [0u64; 64];
        let mut used = 0u32;
        if ffi::GetTokenInformation(token as *mut std::ffi::c_void, ffi::TOKEN_USER, buf.as_mut_ptr().cast(), std::mem::size_of_val(&buf) as u32, &mut used) == 0 { return None; }
        if (used as usize) < std::mem::size_of::<usize>() { return None; }
        // TOKEN_USER { SID_AND_ATTRIBUTES { Sid, Attributes } }: the SID pointer comes first
        let sid = *(buf.as_ptr() as *const *mut std::ffi::c_void);
        let mut text = std::ptr::null_mut();
        if sid.is_null() || ffi::ConvertSidToStringSidW(sid, &mut text) == 0 || text.is_null() { return None; }
        let mut len = 0;
        while len < 184 && *text.add(len) != 0 { len += 1; }
        let account = String::from_utf16(std::slice::from_raw_parts(text, len)).ok();
        ffi::LocalFree(text.cast());
        let mut elevation = 0u32;
        if ffi::GetTokenInformation(token as *mut std::ffi::c_void, ffi::TOKEN_ELEVATION, (&mut elevation as *mut u32).cast(), 4, &mut used) == 0 { return None; }
        account.filter(|a| len < 184 && a.starts_with("S-1-")).map(|a| (a, elevation != 0))
    }
}

impl Client {
    /// `None` when the app isn't running, or when whatever holds the pipe isn't this same
    /// program (another process could have taken the name first).
    pub fn connect() -> Option<Client> {
        let mut o = std::fs::OpenOptions::new();
        o.read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            o.security_qos_flags(ffi::SECURITY_IDENTIFICATION);
        }
        // the app serves one client at a time: while it answers another, the pipe is busy
        // (ERROR_PIPE_BUSY) for a moment, which isn't the same as the app being gone
        let mut tries = 0;
        let f = loop {
            match o.open(name().ok()?) {
                Ok(f) => break f,
                Err(e) if e.raw_os_error() == Some(231) && tries < 40 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => return None,
            }
        };
        #[cfg(windows)]
        if !server_is_us(&f) {
            return None;
        }
        let writer = f.try_clone().ok()?;
        Some(Client { reader: BufReader::new(f), writer })
    }

    pub fn ask(&mut self, request: &str) -> std::io::Result<String> {
        let mut line = request.replace(['\r', '\n'], " ");
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        let mut out = String::new();
        self.reader.read_line(&mut out)?;
        if out.is_empty() {
            return Err(std::io::Error::other("the app closed the connection"));
        }
        Ok(out.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_message_carries_the_writers_account_and_elevation() {
        use std::io::Write;
        use std::os::windows::fs::OpenOptionsExt;
        let name = format!(r"\\.\pipe\sprawl-identity-test-{}", std::process::id());
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let h = unsafe { super::ffi::CreateNamedPipeW(wide.as_ptr(), super::ffi::ACCESS_DUPLEX | super::ffi::FIRST_INSTANCE, super::ffi::REJECT_REMOTE, 1, 4096, 4096, 0, std::ptr::null()) };
        assert_ne!(h, -1);
        let mut client = std::fs::OpenOptions::new().read(true).write(true).security_qos_flags(super::ffi::SECURITY_IDENTIFICATION).open(&name).unwrap();
        client.write_all(b"hello\n").unwrap();
        let ok = unsafe { super::ffi::ConnectNamedPipe(h, std::ptr::null_mut()) } != 0 || std::io::Error::last_os_error().raw_os_error() == Some(super::ffi::ERROR_PIPE_CONNECTED);
        assert!(ok);
        let mut buf = [0u8; 6];
        let mut read = 0u32;
        #[link(name = "kernel32")]
        extern "system" { fn ReadFile(h: isize, buf: *mut u8, len: u32, read: *mut u32, overlapped: *mut std::ffi::c_void) -> i32; }
        assert_ne!(unsafe { ReadFile(h, buf.as_mut_ptr(), 6, &mut read, std::ptr::null_mut()) }, 0);
        let (account, elevated) = super::message_identity(h).expect("the client's context");
        assert_eq!(account, sprawl_core::identity::account_sid().unwrap());
        assert_eq!(elevated, sprawl_core::elevation().unwrap());
        // after the revert this thread is itself again
        assert_eq!(sprawl_core::identity::account_sid().unwrap(), account);
        unsafe { super::ffi::DisconnectNamedPipe(h); super::ffi::CloseHandle(h); }
    }

    #[test]
    fn pipe_identity_is_the_windows_token_account() {
        let sid = sprawl_core::identity::account_sid().unwrap();
        assert_eq!(super::name().unwrap(), format!(r"\\.\pipe\sprawl-mcp-{sid}"));
        let process = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(windows_sys::Win32::System::Threading::GetCurrentProcess()) };
        assert_eq!(sprawl_core::identity::process_account_sid(process).unwrap(), sid);
    }
}
