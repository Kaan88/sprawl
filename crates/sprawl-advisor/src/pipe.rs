//! A local named pipe between `sprawl --mcp` and the running app. One JSON request per line,
//! one JSON answer per line. Local clients only; the first instance is ours or nothing.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a connected client has to send its whole request line. The pipe serves one client
/// at a time, so a client that connects and sends nothing would hold it against every other
/// client of this account; after this it is disconnected and the blocking read gives up.
const REQUEST_LIMIT: Duration = Duration::from_secs(5);

/// When the client being read from has run out of time, for the watchdog thread.
#[derive(Default)]
struct Deadline {
    due: Mutex<Option<Instant>>,
    stop: AtomicBool,
    cancel: Option<Arc<AtomicBool>>,
}

/// Cancels the serving thread's blocking read once the deadline passes, so a client that sends
/// nothing lets go of the pipe. The check and the cancel happen under the same lock that the
/// serving thread clears the deadline with, so a later client's read is never cancelled for
/// this one's silence.
///
/// It must be the read that is cancelled, not the connection: the pipe handle is a synchronous
/// one, so every operation on it queues behind the pending read, and `DisconnectNamedPipe` from
/// here would wait for the very read it is meant to end — both threads then wait for ever.
/// `CancelSynchronousIo` asks the kernel to abandon that thread's current synchronous call, and
/// the serving thread disconnects as usual on its way round the loop.
#[cfg(windows)]
fn watchdog(thread: usize, deadline: Arc<Deadline>) {
    while !deadline.stop.load(Ordering::Relaxed) {
        std::thread::park_timeout(Duration::from_millis(20));
        if deadline.stop.load(Ordering::Relaxed) { break; }
        let due = deadline.due.lock().unwrap_or_else(|e| e.into_inner());
        if due.is_some_and(|t| Instant::now() >= t || deadline.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed))) {
            // SAFETY: the thread handle is this process's own, duplicated for the watchdog and
            // open for as long as it runs. A cancel that found nothing to cancel (the read has
            // just ended) fails with ERROR_NOT_FOUND and changes nothing. Keep trying until
            // the exchange clears its deadline, including across write and flush.
            unsafe { ffi::CancelSynchronousIo(thread as isize); }
        }
    }
    unsafe { ffi::CloseHandle(thread as isize); }
}

/// Until dropped, interrupt this exchange's synchronous read or write at its deadline. The
/// caller owns the pipe and drops it after an interrupted exchange; no stale reply is reused.
#[cfg(windows)]
struct Watch {
    deadline: Arc<Deadline>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl Watch {
    fn start(deadline: Arc<Deadline>) -> std::io::Result<Self> {
        let handle = this_thread()? as usize;
        let watched = deadline.clone();
        match std::thread::Builder::new().spawn(move || watchdog(handle, watched)) {
            Ok(thread) => Ok(Self { deadline, thread: Some(thread) }),
            Err(e) => { unsafe { ffi::CloseHandle(handle as isize); } Err(e) }
        }
    }
}

#[cfg(windows)]
impl Drop for Watch {
    fn drop(&mut self) {
        self.deadline.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

/// This thread, as a handle another thread may use (`GetCurrentThread` is only a stand-in for
/// "whoever asks").
#[cfg(windows)]
fn this_thread() -> std::io::Result<isize> {
    let mut out = 0isize;
    let me = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() } as isize;
    // SAFETY: both are pseudo-handles for this process and this thread; `out` is written only
    // when the call succeeds.
    if unsafe { ffi::DuplicateHandle(me, ffi::GetCurrentThread(), me, &mut out, 0, 0, ffi::SAME_ACCESS) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(out)
}

/// Best-effort interruption of this thread's synchronous Windows I/O. Some drivers can't
/// cancel a pending call, so callers must still bound how long they wait for the thread.
pub(crate) struct SyncIo(std::os::windows::io::OwnedHandle);

impl SyncIo {
    pub(crate) fn current() -> std::io::Result<Self> {
        use std::os::windows::io::FromRawHandle;
        Ok(Self(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(this_thread()? as _) }))
    }

    pub(crate) fn cancel(&self) {
        use std::os::windows::io::AsRawHandle;
        unsafe { ffi::CancelSynchronousIo(self.0.as_raw_handle() as isize); }
    }
}

/// While a scan is running, interrupt its coordinator's blocking opens/reads/writes on stop.
pub(crate) fn watch_current_io(stop: impl Fn() -> bool + Send + 'static) -> std::io::Result<impl Drop> {
    struct WatchThread(Arc<AtomicBool>, Option<std::thread::JoinHandle<()>>);
    impl Drop for WatchThread {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
            if let Some(thread) = self.1.take() {
                thread.thread().unpark();
                let _ = thread.join();
            }
        }
    }
    let io = SyncIo::current()?;
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let thread = std::thread::Builder::new().spawn(move || {
        while !finished.load(Ordering::Relaxed) {
            if stop() { io.cancel(); }
            std::thread::park_timeout(Duration::from_millis(25));
        }
    })?;
    Ok(WatchThread(done, Some(thread)))
}

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
    extern "system" {
        pub fn LocalFree(ptr: *mut c_void) -> *mut c_void;
        pub fn CancelSynchronousIo(thread: isize) -> i32;
        pub fn DuplicateHandle(src_process: isize, src: isize, dst_process: isize, dst: *mut isize, access: u32, inherit: i32, options: u32) -> i32;
    }
    /// DUPLICATE_SAME_ACCESS.
    pub const SAME_ACCESS: u32 = 2;
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
    let deadline = Arc::new(Deadline::default());
    let _watch = match Watch::start(deadline.clone()) {
        Ok(watch) => watch,
        Err(e) => { unsafe { ffi::CloseHandle(h); } return Err(e); }
    };
    loop {
        let ok = unsafe { ffi::ConnectNamedPipe(h, std::ptr::null_mut()) } != 0 || std::io::Error::last_os_error().raw_os_error() == Some(ffi::ERROR_PIPE_CONNECTED);
        let client = if ok { peer(h, false) } else { None };
        if let Some(client_elevated) = client {
            // the handle stays ours: the File is never dropped
            let file = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_handle(h as _) });
            let mut reader = BufReader::new(&*file);
            let mut line = String::new();
            // One request per connection (the client connects afresh for every call): the
            // identity read below is the one this connection was opened with, so a reused or
            // handed-on connection can't carry a second request under it. A line is at most
            // 1 MB: a client that never sends a newline can't fill memory. The watchdog cuts
            // the connection after REQUEST_LIMIT, so one that sends nothing at all can't hold
            // the pipe either; the deadline is cleared before the answer, which may take long.
            *deadline.due.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + REQUEST_LIMIT);
            let asked = (&mut reader).take(1 << 20).read_line(&mut line).is_ok_and(|n| n > 0) && line.ends_with('\n');
            *deadline.due.lock().unwrap_or_else(|e| e.into_inner()) = None;
            if asked {
                // Who opened this connection: the security context the pipe recorded when the
                // client opened it, not only the process behind a client PID (which could have
                // exited and been reused). That authenticates the opening, not each write: a
                // connection handed to another process would still read as its opener's, which
                // is why it carries this one request only. Administrator only when both say so;
                // another account, or no answer, gets no answer from the scan.
                let mut out = match message_identity(h) {
                    Some((account, elevated)) if account == sid => answer(line.trim(), client_elevated && elevated),
                    _ => "{\"ok\":false,\"text\":\"Sprawl couldn't confirm who is asking.\"}".to_string(),
                }
                .replace(['\r', '\n'], " ");
                out.push('\n');
                *deadline.due.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + REQUEST_LIMIT);
                if (&*file).write_all(out.as_bytes()).is_ok() {
                    // The client must consume the answer before disconnect; one that doesn't
                    // read must not hold the only instance in WriteFile or FlushFileBuffers.
                    unsafe { ffi::FlushFileBuffers(h); }
                }
                *deadline.due.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
    /// `None` means no authenticated app, not a busy pipe. Only the latter is retried, and
    /// only before delivering the request: never repeat a call whose answer may have been lost.
    pub fn connect(until: Instant, cancel: &AtomicBool) -> std::io::Result<Option<Client>> {
        Self::connect_to(&name()?, until, cancel)
    }

    pub(crate) fn connect_to(name: &str, until: Instant, cancel: &AtomicBool) -> std::io::Result<Option<Client>> {
        let mut o = std::fs::OpenOptions::new();
        o.read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            o.security_qos_flags(ffi::SECURITY_IDENTIFICATION);
        }
        // the app serves one client at a time: while it answers another, the pipe is busy
        // (ERROR_PIPE_BUSY) for a moment, which isn't the same as the app being gone
        let f = loop {
            check_time(until, cancel)?;
            match o.open(name) {
                Ok(f) => break f,
                Err(e) if e.raw_os_error() == Some(231) => {
                    std::thread::sleep(Duration::from_millis(20).min(until.saturating_duration_since(Instant::now())));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            }
        };
        #[cfg(windows)]
        if !server_is_us(&f) {
            return Ok(None);
        }
        let writer = f.try_clone()?;
        Ok(Some(Client { reader: BufReader::new(f), writer }))
    }

    pub fn ask(mut self, request: &str, until: Instant, cancel: &Arc<AtomicBool>) -> std::io::Result<String> {
        check_time(until, cancel)?;
        let _watch = Watch::start(Arc::new(Deadline { due: Mutex::new(Some(until)), stop: AtomicBool::new(false), cancel: Some(cancel.clone()) }))?;
        let mut line = request.replace(['\r', '\n'], " ");
        line.push('\n');
        let result = (|| {
            self.writer.write_all(line.as_bytes())?;
            check_time(until, cancel)?;
            let mut out = String::new();
            (&mut self.reader).take(1 << 20).read_line(&mut out)?;
            if !out.ends_with('\n') { return Err(std::io::Error::other("the app didn't send a complete answer")); }
            Ok(out.trim().to_string())
        })();
        check_time(until, cancel)?;
        result
    }
}

fn check_time(until: Instant, cancel: &AtomicBool) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    if cancel.load(Ordering::Relaxed) { Err(Error::new(ErrorKind::Interrupted, "request cancelled")) }
    else if Instant::now() >= until { Err(Error::new(ErrorKind::TimedOut, "pipe deadline expired")) }
    else { Ok(()) }
}

/// An authenticated fake peer on a unique pipe, serving just one test request.
#[cfg(test)]
pub(crate) fn test_peer(answer: impl FnOnce(String) -> Option<String> + Send + 'static) -> (String, std::thread::JoinHandle<()>) {
    let mut answer = Some(answer);
    test_peer_requests(1, move |line| answer.take().unwrap()(line))
}

#[cfg(test)]
pub(crate) fn test_peer_requests(count: usize, mut answer: impl FnMut(String) -> Option<String> + Send + 'static) -> (String, std::thread::JoinHandle<()>) {
    use std::os::windows::io::FromRawHandle;
    let name = format!(r"\\.\pipe\sprawl-test-{}", crate::store::nonce());
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let h = unsafe { ffi::CreateNamedPipeW(wide.as_ptr(), ffi::ACCESS_DUPLEX | ffi::FIRST_INSTANCE, ffi::REJECT_REMOTE, 1, 4096, 4096, 0, std::ptr::null()) };
    assert_ne!(h, -1);
    let thread = std::thread::spawn(move || {
        let file = unsafe { std::fs::File::from_raw_handle(h as _) };
        let deadline = Arc::new(Deadline::default());
        let _watch = Watch::start(deadline.clone()).unwrap();
        for _ in 0..count {
            *deadline.due.lock().unwrap() = Some(Instant::now() + REQUEST_LIMIT);
            let ok = unsafe { ffi::ConnectNamedPipe(h, std::ptr::null_mut()) } != 0 || std::io::Error::last_os_error().raw_os_error() == Some(ffi::ERROR_PIPE_CONNECTED);
            assert!(ok);
            let mut line = String::new();
            assert!(BufReader::new(&file).read_line(&mut line).unwrap() > 0);
            *deadline.due.lock().unwrap() = None;
            if let Some(out) = answer(line) {
                *deadline.due.lock().unwrap() = Some(Instant::now() + REQUEST_LIMIT);
                if writeln!(&file, "{out}").is_ok() { unsafe { ffi::FlushFileBuffers(h); } }
                *deadline.due.lock().unwrap() = None;
            }
            unsafe { ffi::DisconnectNamedPipe(h); }
        }
    });
    (name, thread)
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
    fn a_client_that_connects_and_sends_nothing_is_cut_off() {
        use std::io::BufRead;
        use std::os::windows::io::FromRawHandle;
        use std::sync::atomic::Ordering;
        use std::time::{Duration, Instant};
        let name = format!(r"\\.\pipe\sprawl-watchdog-test-{}", std::process::id());
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let h = unsafe { super::ffi::CreateNamedPipeW(wide.as_ptr(), super::ffi::ACCESS_DUPLEX | super::ffi::FIRST_INSTANCE, super::ffi::REJECT_REMOTE, 1, 4096, 4096, 0, std::ptr::null()) };
        assert_ne!(h, -1);
        let _silent = std::fs::OpenOptions::new().read(true).write(true).open(&name).unwrap();
        let ok = unsafe { super::ffi::ConnectNamedPipe(h, std::ptr::null_mut()) } != 0 || std::io::Error::last_os_error().raw_os_error() == Some(super::ffi::ERROR_PIPE_CONNECTED);
        assert!(ok);
        let deadline = std::sync::Arc::new(super::Deadline::default());
        *deadline.due.lock().unwrap() = Some(Instant::now() + Duration::from_millis(100));
        let watch = {
            let (thread, deadline) = (super::this_thread().unwrap() as usize, deadline.clone());
            std::thread::spawn(move || super::watchdog(thread, deadline))
        };
        let started = Instant::now();
        let file = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_handle(h as _) });
        let mut line = String::new();
        let read = std::io::BufReader::new(&*file).read_line(&mut line);
        assert!(read.is_err() || read.unwrap() == 0, "the read should have been cut short, not answered");
        assert!(started.elapsed() < Duration::from_secs(5), "the watchdog took too long");
        // and the pipe is the serving thread's to disconnect afterwards, as the loop does
        assert_ne!(unsafe { super::ffi::DisconnectNamedPipe(h) }, 0);
        deadline.stop.store(true, Ordering::Relaxed);
        watch.join().unwrap();
        unsafe { super::ffi::CloseHandle(h) };
    }

    #[test]
    fn pipe_identity_is_the_windows_token_account() {
        let sid = sprawl_core::identity::account_sid().unwrap();
        assert_eq!(super::name().unwrap(), format!(r"\\.\pipe\sprawl-mcp-{sid}"));
        let process = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(windows_sys::Win32::System::Threading::GetCurrentProcess()) };
        assert_eq!(sprawl_core::identity::process_account_sid(process).unwrap(), sid);
    }
}
