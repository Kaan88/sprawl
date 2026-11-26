//! Windows tool launches: assign the job at creation, and keep scope pins until teardown is proved.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, ERROR_BROKEN_PIPE, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::Globalization::CompareStringOrdinal;
use windows_sys::Win32::System::{JobObjects::*, Pipes::{CreatePipe, PeekNamedPipe}, Threading::*};

fn checked(ok: i32) -> io::Result<()> {
    if ok != 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// Shared by a cache probe and its eventual command; moved to cleanup ownership on a failed wait.
#[derive(Clone, Default)]
pub(crate) struct Pins(Vec<Arc<dyn Send + Sync>>);

impl Pins {
    pub(crate) fn hold(value: impl Send + Sync + 'static) -> Self { Self(vec![Arc::new(value)]) }
    pub(super) fn also(mut self, value: impl Send + Sync + 'static) -> Self {
        self.0.push(Arc::new(value));
        self
    }
}

struct Job(OwnedHandle);

impl Job {
    fn new() -> io::Result<Self> {
        let raw = unsafe { CreateJobObjectW(null(), null()) };
        if raw.is_null() { return Err(io::Error::last_os_error()); }
        let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // Intentionally no BREAKAWAY_OK or SILENT_BREAKAWAY_OK.
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        checked(unsafe { SetInformationJobObject(raw, JobObjectExtendedLimitInformation, (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32) })?;
        Ok(job)
    }

    fn active(&self) -> io::Result<u32> {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        checked(unsafe { QueryInformationJobObject(self.0.as_raw_handle(), JobObjectBasicAccountingInformation,
            (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(), size_of_val(&info) as u32, null_mut()) })?;
        Ok(info.ActiveProcesses)
    }

    fn terminate(&self) -> io::Result<()> {
        checked(unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) })
    }
}

struct Lease { job: Job, process: OwnedHandle, _pins: Pins }

impl Lease {
    fn quiet(&self) -> io::Result<bool> {
        if self.job.active()? != 0 { return Ok(false); }
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

fn issue(problems: &mut Vec<String>, context: &str, error: io::Error) {
    let message = format!("{context}: {error}");
    if !problems.contains(&message) { problems.push(message); }
}

/// A bounded attempt to end a job, preserving errors even if a later attempt succeeds.
fn teardown(attempts: usize, mut quiet: impl FnMut() -> io::Result<bool>, mut terminate: impl FnMut() -> io::Result<()>,
    mut pause: impl FnMut(), problems: &mut Vec<String>) -> bool {
    let mut requested = false;
    for attempt in 0..=attempts {
        match quiet() {
            Ok(true) => return true,
            Ok(false) => {}
            Err(e) => issue(problems, "couldn't verify that the tool job was empty", e),
        }
        if attempt == attempts { break; }
        if !requested {
            match terminate() {
                Ok(()) => requested = true,
                Err(e) => issue(problems, "couldn't terminate the tool job", e),
            }
        }
        pause();
    }
    false
}

// Unconfirmed jobs are quarantined with their pins, even if Windows keeps denying accounting
// or a retry thread can't be created. Recovery is bounded; later launches try again before
// starting a new tool. The last job handle is still kill-on-close at process exit.
static CLEANUP: Mutex<Vec<Lease>> = Mutex::new(Vec::new());

fn reap(pending: &mut Vec<Lease>, mut confirm: impl FnMut(&Lease) -> bool) -> bool {
    pending.retain(|lease| !confirm(lease));
    pending.is_empty()
}

fn reap_pending(problems: &mut Vec<String>) -> bool {
    let mut pending = CLEANUP.lock().unwrap_or_else(|e| e.into_inner());
    reap(&mut pending, |lease| teardown(1, || lease.quiet(), || lease.job.terminate(), || {}, problems))
}

fn log_cleanup(problem: &str) {
    use std::io::Write;
    let _ = writeln!(io::stderr(), "Sprawl tool cleanup: {problem}");
}

fn defer(lease: Lease) {
    CLEANUP.lock().unwrap_or_else(|e| e.into_inner()).push(lease);
    if let Err(e) = std::thread::Builder::new().name("tool-job-cleanup".into()).spawn(|| {
        let mut errors = Vec::new();
        for _ in 0..40 {
            if reap_pending(&mut errors) { break; }
            std::thread::sleep(Duration::from_millis(100));
        }
        for error in errors { log_cleanup(&error); }
    }) {
        // Still owned by CLEANUP; never release the protections because spawning failed.
        log_cleanup(&format!("couldn't start cleanup retry: {e}"));
    }
}

struct Attributes<'a> { _buf: Vec<usize>, list: LPPROC_THREAD_ATTRIBUTE_LIST, _backing: [&'a [HANDLE]; 2] }

impl<'a> Attributes<'a> {
    fn for_launch(handles: &'a [HANDLE], jobs: &'a [HANDLE]) -> io::Result<Self> {
        let mut bytes = 0;
        unsafe { InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes) };
        if bytes == 0 { return Err(io::Error::last_os_error()); }
        let mut buf = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let list = buf.as_mut_ptr().cast();
        checked(unsafe { InitializeProcThreadAttributeList(list, 2, 0, &mut bytes) })?;
        let attrs = Self { _buf: buf, list, _backing: [handles, jobs] };
        checked(unsafe { UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, handles.as_ptr().cast(),
            size_of_val(handles), null_mut(), null_mut()) })?;
        // Both attribute arrays live through CreateProcessW. The job handle itself is not inherited.
        checked(unsafe { UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_JOB_LIST as usize, jobs.as_ptr().cast(),
            size_of_val(jobs), null_mut(), null_mut()) })?;
        Ok(attrs)
    }
}

impl Drop for Attributes<'_> {
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

pub(super) struct Running { lease: Option<Lease>, primary: Option<OwnedHandle>, output: std::fs::File }

enum ReadState { Data(usize), Empty, Eof }

fn pipe_error(error: io::Error) -> io::Result<ReadState> {
    if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) { Ok(ReadState::Eof) } else { Err(error) }
}

impl Running {
    pub(super) fn start(app: &Path, args: &str, cwd: &Path, env: &[u16], pins: Pins) -> io::Result<Self> {
        let mut problems = Vec::new();
        if !reap_pending(&mut problems) {
            return Err(io::Error::other(format!("an earlier tool hasn't been confirmed stopped; protections are still held{}",
                if problems.is_empty() { String::new() } else { format!(" ({})", problems.join("; ")) })));
        }
        for problem in problems { log_cleanup(&problem); }
        Self::create(app, args, cwd, env, pins, |job| job.0.as_raw_handle())
    }

    fn create(app: &Path, args: &str, cwd: &Path, env: &[u16], pins: Pins, job_attribute: impl FnOnce(&Job) -> HANDLE) -> io::Result<Self> {
        let job = Job::new()?;
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
        let jobs = [job_attribute(&job)];
        let attrs = Attributes::for_launch(&handles, &jobs)?;
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
        let primary = unsafe { OwnedHandle::from_raw_handle(created.hThread) };
        // The kernel has added the suspended process to the job before returning success.
        Ok(Self { lease: Some(Lease { job, process, _pins: pins }), primary: Some(primary), output })
    }

    // No read ever waits on an empty pipe, even if an unrelated process inherited a writer.
    fn read_ready(&mut self, buf: &mut [u8]) -> io::Result<ReadState> {
        let mut ready = 0;
        if let Err(e) = checked(unsafe { PeekNamedPipe(self.output.as_raw_handle(), null_mut(), 0, null_mut(), &mut ready, null_mut()) }) {
            return pipe_error(e);
        }
        if ready == 0 { return Ok(ReadState::Empty); }
        let n = buf.len().min(ready as usize);
        match self.output.read(&mut buf[..n]) {
            Ok(0) => Ok(ReadState::Eof),
            Ok(n) => Ok(ReadState::Data(n)),
            Err(e) => pipe_error(e),
        }
    }

    pub(super) fn wait(self, limit: Duration, stop: impl Fn() -> bool, collect: impl FnMut(&[u8])) -> End {
        self.wait_with(limit, stop, collect, Self::read_ready)
    }

    fn wait_with(mut self, limit: Duration, stop: impl Fn() -> bool, mut collect: impl FnMut(&[u8]),
        mut read: impl FnMut(&mut Self, &mut [u8]) -> io::Result<ReadState>) -> End {
        let began = Instant::now();
        let mut buf = [0; 8192];
        let mut eof = false;
        let mut pump = |running: &mut Self, rounds: usize| -> io::Result<bool> {
            if eof { return Ok(false); }
            for _ in 0..rounds {
                match read(running, &mut buf)? {
                    ReadState::Empty => return Ok(false),
                    ReadState::Eof => { eof = true; return Ok(false); }
                    ReadState::Data(n) => collect(&buf[..n]),
                }
            }
            Ok(true)
        };
        let mut end = End { status: Status::Failed, problems: Vec::new(), unconfirmed: false, truncated: false, interrupted: false };
        let mut settled = false;
        let mut read_failed = false;
        if stop() {
            end.status = Status::Stopped;
        } else if let Some(thread) = self.primary.take() {
            let count = unsafe { ResumeThread(thread.as_raw_handle()) };
            if count == u32::MAX {
                issue(&mut end.problems, "couldn't resume the contained tool", io::Error::last_os_error());
            } else if count != 1 {
                end.problems.push(format!("unexpected suspension count while resuming the contained tool ({count})"));
            } else {
                loop {
                    let by_stop = stop();
                    if by_stop || began.elapsed() >= limit {
                        end.status = if by_stop { Status::Stopped } else { Status::TimedOut };
                        break;
                    }
                    // Bounded per turn, even for a verbose tool. Output discarded by the caller
                    // for its size limit is still consumed here so the process can make progress.
                    if let Err(e) = pump(&mut self, 32) {
                        issue(&mut end.problems, "couldn't read tool output", e);
                        read_failed = true;
                        break;
                    }
                    let lease = self.lease.as_ref().unwrap();
                    match lease.quiet() {
                        Ok(true) => {
                            settled = true;
                            let mut code = 0;
                            match checked(unsafe { GetExitCodeProcess(lease.process.as_raw_handle(), &mut code) }) {
                                Ok(()) => end.status = Status::Exited(code),
                                Err(e) => issue(&mut end.problems, "couldn't read the tool's exit code", e),
                            }
                            break;
                        }
                        Ok(false) => std::thread::sleep(Duration::from_millis(20)),
                        Err(e) => {
                            issue(&mut end.problems, "couldn't verify that the tool job was empty", e);
                            break;
                        }
                    }
                }
                end.interrupted = !matches!(end.status, Status::Exited(_));
            }
        }
        if !settled {
            let lease = self.lease.as_ref().unwrap();
            settled = teardown(50, || lease.quiet(), || lease.job.terminate(),
                || std::thread::sleep(Duration::from_millis(20)), &mut end.problems);
        }
        if settled {
            if !read_failed {
                match pump(&mut self, 128) {
                    Ok(more) => end.truncated = more,
                    Err(e) => issue(&mut end.problems, "couldn't read tool output", e),
                }
            }
            self.lease.take(); // No processes left that could use these pins.
        } else {
            end.unconfirmed = true;
            end.problems.push("Windows did not confirm that the tool stopped; its protections are still held while Sprawl checks again".into());
            // Drop transfers the job and its pins to cleanup ownership without waiting.
        }
        end
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() { defer(lease); }
    }
}

pub(super) enum Status { Exited(u32), Stopped, TimedOut, Failed }
pub(super) struct End {
    pub status: Status,
    pub problems: Vec<String>,
    pub unconfirmed: bool,
    pub truncated: bool,
    pub interrupted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd() -> std::path::PathBuf { super::super::system_dir().unwrap().join("cmd.exe") }
    const MARK: &str = "/d /c echo started>started.txt";

    #[test]
    fn containment_failure_never_executes_the_suspended_fixture() {
        let tmp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-job-refuse").unwrap();
        let marker = tmp.path().join("started.txt");
        let exe = cmd();
        let env = environment(false, &[]).unwrap();
        let refused = Running::create(&exe, MARK, tmp.path(), &env, Pins::default(), |_| null_mut());
        assert!(refused.err().unwrap().raw_os_error().is_some());
        assert!(!marker.exists());
        let control = Running::start(&exe, MARK, tmp.path(), &env, Pins::default()).unwrap();
        assert!(matches!(control.wait(Duration::from_secs(10), || false, |_| {}).status, Status::Exited(0)));
        assert!(marker.is_file(), "the fixture would write if resumed");
    }

    #[test]
    #[ignore = "invoked by launcher_death_kills_child_before_resume_without_running_it"]
    fn launcher_fixture() {
        let Some(dir) = std::env::var_os("SPRAWL_TOOL_JOB_TEST_LAUNCHER_DIR") else { return; };
        let dir = Path::new(&dir);
        let child = Running::start(&cmd(), MARK, dir, &environment(false, &[]).unwrap(), Pins::default()).unwrap();
        let pid = unsafe { GetProcessId(child.lease.as_ref().unwrap().process.as_raw_handle()) };
        // Publish only a complete PID; the parent must never open a partial PID.
        let staged = dir.join("child.pid.tmp");
        std::fs::write(&staged, pid.to_string()).unwrap();
        std::fs::rename(staged, dir.join("child.pid")).unwrap();
        // The parent kills this launcher while the fixture child has never been resumed.
        // Do not run its destructor: this tests Windows' job ownership on launcher death.
        loop { std::thread::park(); }
    }

    #[test]
    fn launcher_death_kills_child_before_resume_without_running_it() {
        let tmp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-job-launcher").unwrap();
        let helper = concat!(module_path!(), "::launcher_fixture").split_once("::").unwrap().1;
        let mut launcher = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", helper, "--ignored"])
            .env("SPRAWL_TOOL_JOB_TEST_LAUNCHER_DIR", tmp.path()).spawn().unwrap();
        let start = Instant::now();
        let pid = loop {
            let pid = std::fs::read_to_string(tmp.path().join("child.pid")).ok().and_then(|s| s.parse::<u32>().ok());
            if let Some(pid) = pid.filter(|&pid| pid != 0) { break pid; }
            if start.elapsed() >= Duration::from_secs(10) || launcher.try_wait().unwrap().is_some() {
                let _ = launcher.kill();
                let _ = launcher.wait();
                panic!("fixture launcher did not create the child");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let raw = unsafe { OpenProcess(PROCESS_TERMINATE | 0x0010_0000, 0, pid) }; // SYNCHRONIZE
        let child = (!raw.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(raw) });
        launcher.kill().unwrap();
        launcher.wait().unwrap();
        let child = child.expect("open the fixture child's identity before killing its launcher");
        let signal = unsafe { WaitForSingleObject(child.as_raw_handle(), 5000) };
        if signal != WAIT_OBJECT_0 {
            // Do not strand a test fixture even on a regression.
            checked(unsafe { TerminateProcess(child.as_raw_handle(), 1) }).unwrap();
            assert_eq!(unsafe { WaitForSingleObject(child.as_raw_handle(), 5000) }, WAIT_OBJECT_0);
        }
        assert_eq!(signal, WAIT_OBJECT_0, "the job, not test fallback, must end the child");
        assert!(!tmp.path().join("started.txt").exists());
    }

    #[test]
    fn teardown_reports_termination_and_accounting_errors_and_bounds_retries() {
        let (mut checked, mut killed, mut paused) = (0, 0, 0);
        let mut problems = Vec::new();
        assert!(!teardown(3, || { checked += 1; Err(io::Error::from_raw_os_error(6)) },
            || { killed += 1; Err(io::Error::from_raw_os_error(5)) }, || paused += 1, &mut problems));
        assert_eq!((checked, killed, paused), (4, 3, 3));
        assert_eq!(problems.len(), 2);
        assert!(problems.iter().any(|s| s.contains("empty") && s.contains("os error 6")));
        assert!(problems.iter().any(|s| s.contains("terminate") && s.contains("os error 5")));
        let mut later = Vec::new();
        assert!(teardown(1, || { checked += 1; if checked == 5 { Err(io::Error::from_raw_os_error(6)) } else { Ok(true) } },
            || Ok(()), || {}, &mut later));
        assert_eq!(later.len(), 1, "later confirmation must not erase the first OS failure");
    }

    #[test]
    fn cleanup_owner_keeps_file_pins_until_a_job_is_confirmed_empty() {
        let tmp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-job-pins").unwrap();
        let settings = tmp.path().join("settings.txt");
        std::fs::write(&settings, "fixture").unwrap();
        let pin = sprawl_core::local::file(&settings, 1024).unwrap();
        let mut child = Running::start(&cmd(), MARK, tmp.path(), &environment(false, &[]).unwrap(), Pins::hold(pin)).unwrap();
        let mut pending = vec![child.lease.take().unwrap()]; // same lease the production Drop hands to CLEANUP
        assert!(!reap(&mut pending, |_| false));
        assert!(std::fs::write(&settings, "can't change yet").is_err());
        let mut problems = Vec::new();
        assert!(reap(&mut pending, |lease| teardown(50, || lease.quiet(), || lease.job.terminate(),
            || std::thread::sleep(Duration::from_millis(20)), &mut problems)), "{problems:?}");
        std::fs::write(&settings, "now writable").unwrap();
        assert!(!tmp.path().join("started.txt").exists());
    }

    #[test]
    fn deferred_teardown_retires_the_wipe_marker_only_after_confirmation() {
        let tmp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-job-wipe-marker").unwrap();
        let marker = tmp.path().join("wipe-E.txt");
        let root = Path::new(r"E:\");
        let hold = crate::overwrite::claim_marker(&marker, root).unwrap();
        let mut child = Running::start(&cmd(), MARK, tmp.path(), &environment(false, &[]).unwrap(), Pins::hold(hold)).unwrap();
        let mut pending = vec![child.lease.take().unwrap()];
        assert!(!reap(&mut pending, |_| false), "same retention rule as deferred teardown");
        assert!(std::fs::read_to_string(&marker).unwrap().contains("root=E:"));
        assert!(crate::overwrite::claim_marker(&marker, root).is_err(), "the unconfirmed job still owns the marker");
        let mut problems = Vec::new();
        assert!(reap(&mut pending, |lease| teardown(50, || lease.quiet(), || lease.job.terminate(),
            || std::thread::sleep(Duration::from_millis(20)), &mut problems)), "{problems:?}");
        assert!(!marker.exists(), "the marker retires when the deferred lease is released");
        assert!(!tmp.path().join("started.txt").exists());
        assert!(crate::overwrite::claim_marker(&marker, root).is_ok(), "the marker can be claimed again");
    }

    #[test]
    fn read_failure_ends_the_job_and_is_not_broken_pipe_or_truncation() {
        assert!(matches!(pipe_error(io::Error::from_raw_os_error(ERROR_BROKEN_PIPE as i32)), Ok(ReadState::Eof)));
        let tmp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-job-pipe-error").unwrap();
        let exe = super::super::system_dir().unwrap().join(r"WindowsPowerShell\v1.0\powershell.exe");
        let script = super::super::encoded("[Console]::WriteLine('ready'); Start-Sleep -Seconds 40");
        let running = Running::start(&exe, &format!("-NoProfile -NonInteractive -EncodedCommand {script}"), tmp.path(),
            &environment(false, &[]).unwrap(), Pins::default()).unwrap();
        let process = running.lease.as_ref().unwrap().process.try_clone().unwrap();
        let mut ready = false;
        let end = running.wait_with(Duration::from_secs(10), || false, |_| {}, |running, buf| {
            if ready { return Err(io::Error::from_raw_os_error(6)); }
            let state = running.read_ready(buf)?;
            if matches!(state, ReadState::Data(_)) { ready = true; }
            Ok(state)
        });
        assert!(ready && matches!(end.status, Status::Failed) && end.interrupted);
        assert!(!end.truncated && !end.unconfirmed && end.problems[0].contains("couldn't read tool output"), "{:?}", end.problems);
        assert_eq!(unsafe { WaitForSingleObject(process.as_raw_handle(), 0) }, WAIT_OBJECT_0);
    }
}
