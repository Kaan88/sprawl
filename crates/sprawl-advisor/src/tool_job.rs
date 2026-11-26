//! Windows tool launches: no user code before job assignment, and no return with live members.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, ERROR_BROKEN_PIPE, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::Globalization::CompareStringOrdinal;
use windows_sys::Win32::System::{JobObjects::*, Pipes::{CreatePipe, PeekNamedPipe}, Threading::*};

fn checked(ok: i32) -> io::Result<()> {
    if ok != 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

struct Job { handle: OwnedHandle, draining: bool }

impl Job {
    fn new() -> io::Result<Self> {
        let raw = unsafe { CreateJobObjectW(null(), null()) };
        if raw.is_null() { return Err(io::Error::last_os_error()); }
        let job = Self { handle: unsafe { OwnedHandle::from_raw_handle(raw) }, draining: false };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // Intentionally no BREAKAWAY_OK or SILENT_BREAKAWAY_OK.
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        checked(unsafe { SetInformationJobObject(raw, JobObjectExtendedLimitInformation, (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32) })?;
        Ok(job)
    }

    fn add(&self, process: HANDLE) -> io::Result<()> {
        checked(unsafe { AssignProcessToJobObject(self.handle.as_raw_handle(), process) })
    }

    fn active(&self) -> io::Result<u32> {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        checked(unsafe { QueryInformationJobObject(self.handle.as_raw_handle(), JobObjectBasicAccountingInformation,
            (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(), size_of_val(&info) as u32, null_mut()) })?;
        Ok(info.ActiveProcesses)
    }

    fn drain(&mut self) {
        // Terminating the job is not a wait. Keep the job (and the caller's file pins) until
        // Windows confirms that all its members are gone; an accounting error isn't zero.
        while self.draining {
            if self.active().is_ok_and(|n| n == 0) {
                self.draining = false;
                break;
            }
            unsafe { TerminateJobObject(self.handle.as_raw_handle(), 1) };
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) { self.drain(); }
}

struct Attributes { _buf: Vec<usize>, list: LPPROC_THREAD_ATTRIBUTE_LIST }

impl Attributes {
    fn for_handles(handles: &[HANDLE]) -> io::Result<Self> {
        let mut bytes = 0;
        unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes) };
        if bytes == 0 { return Err(io::Error::last_os_error()); }
        let mut buf = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let list = buf.as_mut_ptr().cast();
        checked(unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut bytes) })?;
        let attrs = Self { _buf: buf, list };
        checked(unsafe { UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, handles.as_ptr().cast(),
            size_of_val(handles), null_mut(), null_mut()) })?;
        Ok(attrs)
    }
}

impl Drop for Attributes {
    fn drop(&mut self) { unsafe { DeleteProcThreadAttributeList(self.list) }; }
}

fn child_handle(handle: HANDLE) -> io::Result<OwnedHandle> {
    let mut copy = null_mut();
    unsafe {
        let here = GetCurrentProcess();
        checked(DuplicateHandle(here, handle, here, &mut copy, 0, 1, DUPLICATE_SAME_ACCESS))?;
        Ok(OwnedHandle::from_raw_handle(copy))
    }
}

fn wide(s: &OsStr) -> io::Result<Vec<u16>> {
    let mut s: Vec<_> = s.encode_wide().collect();
    if s.contains(&0) { return Err(io::Error::new(io::ErrorKind::InvalidInput, "a tool argument contains a NUL")); }
    s.push(0);
    Ok(s)
}

pub(super) fn environment(git: bool, overrides: &[(&str, &OsStr)]) -> io::Result<Vec<u16>> {
    fn key_cmp(a: &[u16], b: &[u16]) -> std::cmp::Ordering {
        (unsafe { CompareStringOrdinal(a.as_ptr(), a.len() as i32, b.as_ptr(), b.len() as i32, 1) }).cmp(&2)
    }
    let mut vars = std::env::vars_os().filter(|(k, _)| !git || !k.to_string_lossy().to_ascii_uppercase().starts_with("GIT_"))
        .map(|(k, v)| Ok((wide(&k)?, wide(&v)?))).collect::<io::Result<Vec<_>>>()?;
    for (key, value) in overrides {
        let key = wide(OsStr::new(key))?;
        vars.retain(|(k, _)| !key_cmp(k, &key).is_eq());
        vars.push((key, wide(value)?));
    }
    vars.sort_by(|(a, _), (b, _)| key_cmp(a, b));
    let mut block = Vec::new();
    for (mut k, v) in vars {
        *k.last_mut().unwrap() = b'=' as u16;
        block.extend(k);
        block.extend(v);
    }
    if block.is_empty() { block.push(0); }
    block.push(0);
    Ok(block)
}

pub(super) struct Running { job: Job, process: OwnedHandle, output: std::fs::File }

impl Running {
    pub(super) fn start(app: &Path, args: &str, cwd: &Path, env: &[u16]) -> io::Result<Self> {
        Self::start_with(app, args, cwd, env, Job::add)
    }

    fn start_with(app: &Path, args: &str, cwd: &Path, env: &[u16], assign: impl FnOnce(&Job, HANDLE) -> io::Result<()>) -> io::Result<Self> {
        let mut job = Job::new()?;
        let mut command = OsString::from("\"");
        command.push(app);
        command.push("\" ");
        command.push(args); // caller passes only fixed switches and a base64 script
        let mut command = wide(&command)?;
        let app = wide(app.as_os_str())?;
        let cwd = wide(cwd.as_os_str())?;
        let nul = std::fs::OpenOptions::new().read(true).write(true).open("NUL")?;
        let std_null = child_handle(nul.as_raw_handle())?;
        let (mut read, mut write) = (null_mut(), null_mut());
        checked(unsafe { CreatePipe(&mut read, &mut write, null(), 0) })?;
        let output = unsafe { std::fs::File::from_raw_handle(read) };
        let write = unsafe { OwnedHandle::from_raw_handle(write) };
        let child_write = child_handle(write.as_raw_handle())?;
        let handles = [std_null.as_raw_handle(), child_write.as_raw_handle()];
        let attrs = Attributes::for_handles(&handles)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = handles[0];
        startup.StartupInfo.hStdError = handles[0];
        startup.StartupInfo.hStdOutput = handles[1];
        startup.lpAttributeList = attrs.list;
        let mut created = PROCESS_INFORMATION::default();
        checked(unsafe { CreateProcessW(app.as_ptr(), command.as_mut_ptr(), null(), null(), 1,
            CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            env.as_ptr().cast(), cwd.as_ptr(), &startup.StartupInfo, &mut created) })?;
        let process = unsafe { OwnedHandle::from_raw_handle(created.hProcess) };
        let thread = unsafe { OwnedHandle::from_raw_handle(created.hThread) };
        if let Err(e) = assign(&job, process.as_raw_handle()) {
            // The only thread was never resumed, even if the containing job refused nesting.
            unsafe {
                TerminateProcess(process.as_raw_handle(), 1);
                WaitForSingleObject(process.as_raw_handle(), INFINITE);
            }
            return Err(io::Error::other(format!("Windows would not keep the suspended tool in a job ({e})")));
        }
        job.draining = true;
        if unsafe { ResumeThread(thread.as_raw_handle()) } != 1 {
            return Err(io::Error::other("Windows did not resume the contained tool normally"));
        }
        Ok(Self { job, process, output })
    }

    // No read ever waits on an empty pipe, even if an unrelated process inherited a writer.
    pub(super) fn read_ready(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut ready = 0;
        let result = checked(unsafe { PeekNamedPipe(self.output.as_raw_handle(), null_mut(), 0, null_mut(), &mut ready, null_mut()) });
        if result.as_ref().is_err_and(|e| e.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32)) { return Ok(0); }
        result?;
        if ready == 0 { return Ok(0); }
        let n = buf.len().min(ready as usize);
        self.output.read(&mut buf[..n])
    }

    pub(super) fn wait(&mut self, limit: Duration, stop: impl Fn() -> bool, mut collect: impl FnMut(&[u8])) -> io::Result<End> {
        let began = Instant::now();
        let mut buf = [0; 8192];
        let mut cut = false;
        let mut drain_output = |running: &mut Self, rounds: usize| {
            if cut { return; }
            for _ in 0..rounds {
                match running.read_ready(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => collect(&buf[..n]),
                    Err(_) => { cut = true; break; }
                }
            }
        };
        let result = loop {
            let by_stop = stop();
            if by_stop || began.elapsed() >= limit {
                self.job.drain();
                break if by_stop { Status::Stopped } else { Status::TimedOut };
            }
            // Bound each drain so a constantly writing tool can't starve Stop.
            drain_output(self, 32);
            if self.job.active()? == 0 {
                self.job.draining = false;
                if unsafe { WaitForSingleObject(self.process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
                    return Err(io::Error::last_os_error());
                }
                let mut code = 0;
                checked(unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) })?;
                break Status::Exited(code);
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // Once the job is empty only the buffered tail is left (the pipe has finite capacity).
        drain_output(self, 32);
        Ok(End { status: result, cut })
    }
}

pub(super) enum Status { Exited(u32), Stopped, TimedOut }
pub(super) struct End { pub status: Status, pub cut: bool }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containment_failure_never_executes_the_suspended_fixture() {
        let tmp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-job-refuse").unwrap();
        let marker = tmp.path().join("started.txt");
        let sys = super::super::system_dir().unwrap();
        let exe = sys.join("cmd.exe");
        let env = environment(false, &[]).unwrap();
        let args = "/d /c echo started>started.txt";
        let refused = Running::start_with(&exe, args, tmp.path(), &env, |_, process| {
            assert!(!process.is_null());
            checked(unsafe { AssignProcessToJobObject(null_mut(), process) })
        });
        assert!(refused.err().unwrap().to_string().contains("suspended tool in a job"));
        assert!(!marker.exists());
        let mut control = Running::start(&exe, args, tmp.path(), &env).unwrap();
        assert!(matches!(control.wait(Duration::from_secs(10), || false, |_| {}).unwrap().status, Status::Exited(0)));
        assert!(marker.is_file(), "the fixture would write if resumed");
    }
}
