//! Stop other instances of this exact installation before update publication. Never by name
//! alone: hold a process handle, check its token account and full executable path, then stop
//! and wait on that same handle. The updater's own PID is always excluded.

#[repr(C)]
struct Entry {
    size: u32, usage: u32, pid: u32, heap: usize, module: u32, threads: u32,
    parent: u32, priority: i32, flags: u32, name: [u16; 260],
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
    fn Process32FirstW(snapshot: isize, entry: *mut Entry) -> i32;
    fn Process32NextW(snapshot: isize, entry: *mut Entry) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    fn GetCurrentProcess() -> isize;
    fn TerminateProcess(process: isize, code: u32) -> i32;
    fn WaitForSingleObject(handle: isize, milliseconds: u32) -> u32;
    fn CloseHandle(handle: isize) -> i32;
}

struct Handle(isize);
impl Drop for Handle { fn drop(&mut self) { unsafe { CloseHandle(self.0); } } }

fn current_image() -> Result<Vec<u16>, String> {
    let current = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(GetCurrentProcess() as _) };
    sprawl_core::identity::process_image_path(current).map_err(|e| e.to_string())
}

fn matches(process: &Handle, expected: &[u16], account: &str) -> bool {
    let borrowed = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(process.0 as _) };
    if sprawl_core::identity::process_account_sid(borrowed).ok().as_deref() != Some(account) { return false; }
    sprawl_core::identity::process_image_path(borrowed).is_ok_and(|path| path == expected)
}

/// Milliseconds left until `deadline`, for `WaitForSingleObject`.
fn left(deadline: std::time::Instant) -> u32 {
    deadline.saturating_duration_since(std::time::Instant::now()).as_millis().min(u32::MAX as u128 - 1) as u32
}

/// A process this one can't stop: an elevated copy seen from one without administrator rights,
/// usually the copy that has just handed its update over and is still closing. Wait for it to
/// end, at most until `deadline`, on the same handle that was verified. Ok: it ended, or it isn't
/// this installation's. Err: it is still running.
fn wait_unstoppable(pid: u32, expected: &[u16], account: &str, deadline: std::time::Instant) -> Result<(), ()> {
    let waitable = unsafe { OpenProcess(0x1000 | 0x0010_0000, 0, pid) };
    if waitable == 0 {
        // not even a wait: refuse when it is this installation's
        let query = unsafe { OpenProcess(0x1000, 0, pid) };
        return if query != 0 && matches(&Handle(query), expected, account) { Err(()) } else { Ok(()) };
    }
    let process = Handle(waitable);
    if !matches(&process, expected, account) { return Ok(()); }
    if unsafe { WaitForSingleObject(process.0, left(deadline)) } == 0 { Ok(()) } else { Err(()) }
}

fn stop_candidates(expected: &[u16], account: &str, current: u32, candidates: &[u32]) -> Result<usize, String> {
    let mut stopped = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    for &pid in candidates {
        if pid == current { continue; }
        // Query, terminate and wait rights requested together so verification and termination
        // cannot cross PID reuse. Never terminate a process verified through another handle.
        let process = unsafe { OpenProcess(0x1000 | 0x0010_0000 | 1, 0, pid) };
        if process == 0 {
            if wait_unstoppable(pid, expected, account, deadline).is_err() {
                return Err("Another Sprawl process from this installation, perhaps running as administrator, is still open. Close it and try Update again.".into());
            }
            continue;
        }
        let process = Handle(process);
        if !matches(&process, expected, account) { continue; }
        if unsafe { WaitForSingleObject(process.0, 0) } == 0 { continue; }
        if unsafe { TerminateProcess(process.0, 0) } == 0 && unsafe { WaitForSingleObject(process.0, 0) } != 0 {
            return Err("Another Sprawl process could not be stopped. Close it and try Update again.".into());
        }
        if unsafe { WaitForSingleObject(process.0, left(deadline)) } != 0 {
            return Err("Another Sprawl process has not finished closing. Try Update again in a moment.".into());
        }
        stopped += 1;
    }
    Ok(stopped)
}

/// How many other processes of this installation, for this account, are running (windows, the
/// low-space check, an AI app's `--mcp` connection). The update asks before it stops them
/// (Fable core review P2-5).
pub fn count_other_instances() -> Result<usize, String> {
    let (image, account, candidates) = list()?;
    let current = std::process::id();
    Ok(candidates.iter().filter(|&&pid| pid != current).filter(|&&pid| {
        let query = unsafe { OpenProcess(0x1000 | 0x0010_0000, 0, pid) };
        if query == 0 { return false; }
        let h = Handle(query);
        matches(&h, &image, &account) && unsafe { WaitForSingleObject(h.0, 0) } != 0
    }).count())
}

/// Before an update another copy handed over: wait, at most `wait`, for the processes of this
/// installation this one can't stop (the elevated copy that handed over and is closing). How many
/// are still running.
pub fn wait_for_unstoppable(wait: std::time::Duration) -> Result<usize, String> {
    let (image, account, candidates) = list()?;
    let deadline = std::time::Instant::now() + wait;
    let current = std::process::id();
    Ok(candidates.iter().filter(|&&pid| pid != current).filter(|&&pid| {
        let stoppable = unsafe { OpenProcess(0x1000 | 0x0010_0000 | 1, 0, pid) };
        if stoppable != 0 {
            drop(Handle(stoppable));
            return false;
        }
        wait_unstoppable(pid, &image, &account, deadline).is_err()
    }).count())
}

pub fn stop_other_instances() -> Result<usize, String> {
    let (image, account, candidates) = list()?;
    stop_candidates(&image, &account, std::process::id(), &candidates)
}

/// This exe's image, the account, and the processes with its file name.
fn list() -> Result<(Vec<u16>, String, Vec<u32>), String> {
    crate::setup::per_user_only()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let _directory = sprawl_core::local::directory(exe.parent().ok_or("No installation folder")?).map_err(|e| e.to_string())?;
    let account = sprawl_core::identity::account_sid().map_err(|e| e.to_string())?;
    let image = current_image()?;
    let snapshot = unsafe { CreateToolhelp32Snapshot(2, 0) };
    if snapshot == -1 { return Err("Windows could not list Sprawl processes; try Update again.".into()); }
    let snapshot = Handle(snapshot);
    let mut entry: Entry = unsafe { std::mem::zeroed() };
    entry.size = std::mem::size_of::<Entry>() as u32;
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    let mut candidates = Vec::new();
    while more != 0 {
        let length = entry.name.iter().position(|&c| c == 0).unwrap_or(entry.name.len());
        let name = String::from_utf16_lossy(&entry.name[..length]);
        if exe.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&name)) { candidates.push(entry.pid); }
        if candidates.len() > 1024 { return Err("Too many Sprawl processes; close other instances before updating.".into()); }
        more = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    if std::io::Error::last_os_error().raw_os_error() != Some(18) { return Err("Windows could not finish listing processes; try Update again.".into()); }
    Ok((image, account, candidates))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposable_update_peer() {
        let Some(ready) = std::env::var_os("SPRAWL_DISPOSABLE_UPDATE_PEER") else { return };
        std::fs::write(ready, b"ready").unwrap();
        loop { std::thread::park(); }
    }

    #[test]
    fn updater_stops_only_its_verified_disposable_peer() {
        use std::os::windows::process::CommandExt;
        let exe = std::env::current_exe().unwrap();
        let account = sprawl_core::identity::account_sid().unwrap();
        let image = current_image().unwrap();
        let ready = std::env::temp_dir().join(format!("sprawl-update-peer-{}.ready", std::process::id()));
        let _ = std::fs::remove_file(&ready);
        let mut child = std::process::Command::new(&exe).args(["--exact", "processes::tests::disposable_update_peer", "--nocapture"])
            .env("SPRAWL_DISPOSABLE_UPDATE_PEER", &ready).creation_flags(0x0800_0000).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
        let result = || {
            let start = std::time::Instant::now();
            while !ready.exists() && start.elapsed().as_secs() < 5 { std::thread::sleep(std::time::Duration::from_millis(20)); }
            assert!(ready.exists(), "the disposable peer never started");
            let candidates = [child.id(), std::process::id()];
            let mut other_case = image.clone();
            let letter = other_case.iter_mut().find(|c| **c == b's' as u16 || **c == b'S' as u16).unwrap();
            *letter ^= 0x20;
            assert_eq!(stop_candidates(&other_case, &account, std::process::id(), &candidates).unwrap(), 0, "case-distinct native paths are not the same installation");
            assert_eq!(stop_candidates(&image, "S-1-5-999999", std::process::id(), &candidates).unwrap(), 0);
            assert!(child.try_wait().unwrap().is_none());
            assert_eq!(stop_candidates(&image, &account, std::process::id(), &candidates).unwrap(), 1);
            assert!(child.try_wait().unwrap().is_some());
        };
        // Only the explicitly spawned fixture process can be terminated in this test.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(result));
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(ready);
        if let Err(panic) = result { std::panic::resume_unwind(panic); }
    }

    /// The wait for a copy this one can't stop: it times out while the copy runs, ends as soon as
    /// it closes, and ignores processes of another installation or account.
    #[test]
    fn waits_for_a_copy_it_cannot_stop_to_close() {
        use std::os::windows::process::CommandExt;
        use std::time::{Duration, Instant};
        let exe = std::env::current_exe().unwrap();
        let account = sprawl_core::identity::account_sid().unwrap();
        let image = current_image().unwrap();
        let ready = std::env::temp_dir().join(format!("sprawl-wait-peer-{}.ready", std::process::id()));
        let _ = std::fs::remove_file(&ready);
        let mut child = std::process::Command::new(&exe).args(["--exact", "processes::tests::disposable_update_peer", "--nocapture"])
            .env("SPRAWL_DISPOSABLE_UPDATE_PEER", &ready).creation_flags(0x0800_0000).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
        let pid = child.id();
        let result = || {
            let start = Instant::now();
            while !ready.exists() && start.elapsed().as_secs() < 5 { std::thread::sleep(Duration::from_millis(20)); }
            assert!(ready.exists(), "the disposable peer never started");
            assert!(wait_unstoppable(pid, &image, "S-1-5-999999", Instant::now() + Duration::from_secs(5)).is_ok(), "another account's process is not waited for");
            let t = Instant::now();
            assert!(wait_unstoppable(pid, &image, &account, Instant::now() + Duration::from_millis(300)).is_err(), "still running at the deadline");
            assert!(t.elapsed() >= Duration::from_millis(250));
            // closed by someone else meanwhile: the wait ends when it does, not at the deadline
            let killer = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                let h = unsafe { OpenProcess(0x0001, 0, pid) };
                assert_ne!(h, 0);
                let h = Handle(h);
                unsafe { TerminateProcess(h.0, 0) };
            });
            let t = Instant::now();
            assert!(wait_unstoppable(pid, &image, &account, Instant::now() + Duration::from_secs(10)).is_ok());
            assert!(t.elapsed() < Duration::from_secs(5));
            killer.join().unwrap();
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(result));
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(ready);
        if let Err(panic) = result { std::panic::resume_unwind(panic); }
    }
}
