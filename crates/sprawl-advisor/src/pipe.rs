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
    }
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
pub fn serve(mut answer: impl FnMut(&str) -> String) -> std::io::Result<()> {
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
        if ok && peer_is_us(h, false) {
            // the handle stays ours: the File is never dropped
            let file = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_handle(h as _) });
            let mut reader = BufReader::new(&*file);
            let mut line = String::new();
            // a line is at most 1 MB: a client that never sends a newline can't fill memory
            while (&mut reader).take(1 << 20).read_line(&mut line).is_ok_and(|n| n > 0) {
                if !line.ends_with('\n') {
                    break;
                }
                let mut out = answer(line.trim()).replace(['\r', '\n'], " ");
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
    let mut pid = 0u32;
    let ok = unsafe { if server { ffi::GetNamedPipeServerProcessId(pipe, &mut pid) } else { ffi::GetNamedPipeClientProcessId(pipe, &mut pid) } };
    if ok == 0 {
        return false;
    }
    let h = unsafe { ffi::OpenProcess(ffi::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h == 0 {
        return false;
    }
    // Hold the same process while querying both token and image: no PID reuse between them.
    let peer = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(h as _) };
    let same_account = sprawl_core::identity::account_sid().ok().zip(sprawl_core::identity::process_account_sid(peer).ok()).is_some_and(|(ours, theirs)| ours == theirs);
    let theirs = sprawl_core::identity::process_image_path(peer);
    let current = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(windows_sys::Win32::System::Threading::GetCurrentProcess()) };
    let ours = sprawl_core::identity::process_image_path(current);
    unsafe { ffi::CloseHandle(h) };
    same_account && ours.ok().zip(theirs.ok()).is_some_and(|(ours, theirs)| ours == theirs)
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
    fn pipe_identity_is_the_windows_token_account() {
        let sid = sprawl_core::identity::account_sid().unwrap();
        assert_eq!(super::name().unwrap(), format!(r"\\.\pipe\sprawl-mcp-{sid}"));
        let process = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(windows_sys::Win32::System::Threading::GetCurrentProcess()) };
        assert_eq!(sprawl_core::identity::process_account_sid(process).unwrap(), sid);
    }
}
