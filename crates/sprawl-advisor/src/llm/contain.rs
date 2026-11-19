//! An AI command-line program (Claude Code, Codex) and everything it starts, in one Windows
//! job: the program starts suspended, joins the job, and only then runs, so no helper it starts
//! can be outside it. Stop, a timeout, or dropping the job (any return, Sprawl closing) ends
//! them all.
use std::process::{Child, Command};

type H = *mut core::ffi::c_void;

#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(sa: H, name: *const u16) -> H;
    fn SetInformationJobObject(job: H, class: i32, info: *const core::ffi::c_void, size: u32) -> i32;
    fn AssignProcessToJobObject(job: H, process: H) -> i32;
    fn TerminateJobObject(job: H, code: u32) -> i32;
    #[link_name = "CloseHandle"]
    fn close_job_handle(h: H) -> i32;
}

/// The program, in its job.
pub(crate) struct Contained {
    pub(crate) child: Child,
    job: H,
}

// the job handle is only used by the thread that owns this
unsafe impl Send for Contained {}

const CREATE_SUSPENDED: u32 = 0x4;

/// Start `cmd` (with `flags`, such as CREATE_NO_WINDOW) inside a new job that ends everything
/// in it when closed. Refused, with nothing left running, when the job can't be made or joined.
pub(crate) fn spawn(cmd: &mut Command, flags: u32) -> Result<Contained, String> {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    // SAFETY: no security attributes, no name
    let job = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
    if job.is_null() {
        return Err(format!("couldn't make a job for it: {}", std::io::Error::last_os_error()));
    }
    // JOBOBJECT_EXTENDED_LIMIT_INFORMATION with LimitFlags = KILL_ON_JOB_CLOSE (no breakaway)
    let mut info = [0u8; 144];
    info[16..20].copy_from_slice(&0x2000u32.to_le_bytes());
    let size = if cfg!(target_pointer_width = "64") { 144 } else { 112 };
    // SAFETY: a valid job handle and a buffer of the size given
    if unsafe { SetInformationJobObject(job, 9, info.as_ptr().cast(), size) } == 0 {
        let e = std::io::Error::last_os_error();
        unsafe { close_job_handle(job) };
        return Err(format!("couldn't set up its job: {e}"));
    }
    cmd.creation_flags(flags | CREATE_SUSPENDED);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            unsafe { close_job_handle(job) };
            return Err(e.to_string());
        }
    };
    let mut c = Contained { child, job };
    // SAFETY: valid handles; the process hasn't run a single instruction of its own yet
    if unsafe { AssignProcessToJobObject(job, c.child.as_raw_handle()) } == 0 {
        let e = std::io::Error::last_os_error();
        let _ = c.child.kill();
        let _ = c.child.wait();
        return Err(format!("couldn't put it in a job: {e}"));
    }
    if !resume(c.child.id()) {
        c.end();
        return Err("couldn't start it after putting it in its job".into());
    }
    Ok(c)
}

/// Resume every thread of the suspended process `pid` (a new process has one).
fn resume(pid: u32) -> bool {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32};
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    // SAFETY: a snapshot of all threads, closed below; entries are read into a sized struct
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap.is_null() || snap as isize == -1 {
            return false;
        }
        let mut e: THREADENTRY32 = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut resumed = 0;
        let mut ok = Thread32First(snap, &mut e) != 0;
        while ok {
            if e.th32OwnerProcessID == pid {
                let t = OpenThread(THREAD_SUSPEND_RESUME, 0, e.th32ThreadID);
                if !t.is_null() {
                    if ResumeThread(t) != u32::MAX {
                        resumed += 1;
                    }
                    close_job_handle(t);
                }
            }
            ok = Thread32Next(snap, &mut e) != 0;
        }
        close_job_handle(snap);
        resumed > 0
    }
}

impl Contained {
    /// End the program and everything it started, and wait for the program.
    pub(crate) fn end(&mut self) {
        // SAFETY: the job handle is ours until drop
        unsafe { TerminateJobObject(self.job, 1) };
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Contained {
    fn drop(&mut self) {
        // closing the last handle ends whatever is still in the job (KILL_ON_JOB_CLOSE)
        unsafe { close_job_handle(self.job) };
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

    /// A helper the program starts is in the job: ending it ends the helper, which then stops
    /// holding the output pipe.
    #[test]
    fn helpers_end_with_the_program() {
        let sys = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap().join("System32");
        let mut cmd = Command::new(sys.join("cmd.exe"));
        // cmd starts a second cmd that waits a minute, and returns at once
        cmd.args(["/c", "start", "/b", "cmd", "/c", "ping -n 60 127.0.0.1 >nul"]).stdout(Stdio::piped()).stdin(Stdio::null()).stderr(Stdio::null());
        let mut c = super::spawn(&mut cmd, 0x0800_0000).unwrap();
        let mut out = c.child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut s = Vec::new();
            let _ = std::io::Read::read_to_end(&mut out, &mut s);
            let _ = tx.send(());
        });
        let _ = c.child.wait();
        // the helper still holds the pipe
        assert!(rx.recv_timeout(std::time::Duration::from_millis(500)).is_err());
        c.end();
        assert!(rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok(), "ending the job closed the helper's pipe");
    }
}
