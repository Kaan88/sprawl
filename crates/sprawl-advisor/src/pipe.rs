//! A local named pipe between `sprawl --mcp` and the running app. One JSON request per line,
//! one JSON answer per line. Local clients only; the first instance is ours or nothing.

use std::io::{BufRead, BufReader, Write};

/// `\\.\pipe\sprawl-mcp-<user>`: one per Windows user.
pub fn name() -> String {
    let user: String = std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric()).take(32).collect();
    format!(r"\\.\pipe\sprawl-mcp-{user}")
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
    }
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
    let wide: Vec<u16> = name().encode_utf16().chain(Some(0)).collect();
    let h = unsafe { ffi::CreateNamedPipeW(wide.as_ptr(), ffi::ACCESS_DUPLEX | ffi::FIRST_INSTANCE, ffi::REJECT_REMOTE, 1, 64 << 10, 64 << 10, 0, std::ptr::null()) };
    if h == -1 {
        return Err(std::io::Error::last_os_error());
    }
    loop {
        let ok = unsafe { ffi::ConnectNamedPipe(h, std::ptr::null_mut()) } != 0 || std::io::Error::last_os_error().raw_os_error() == Some(ffi::ERROR_PIPE_CONNECTED);
        if ok {
            // the handle stays ours: the File is never dropped
            let file = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_handle(h as _) });
            let mut reader = BufReader::new(&*file);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 0) {
                if line.len() > 1 << 20 {
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

impl Client {
    /// `None` when the app isn't running.
    pub fn connect() -> Option<Client> {
        let f = std::fs::OpenOptions::new().read(true).write(true).open(name()).ok()?;
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
