//! Hidden space: the difference between what the drive says is used and what the scan found,
//! and what usually explains it (shadow copies and restore points, reserved storage, file
//! system records, folders that couldn't be read). Only for scans of a whole drive.

use std::path::Path;

use sprawl_core::{Tree, ROOT};

#[derive(Clone, Debug)]
pub struct Part {
    pub label: &'static str,
    /// Known size, when Windows tells us.
    pub bytes: Option<u64>,
    /// What it is and what to do about it.
    pub note: String,
}

#[derive(Clone, Debug)]
pub struct HiddenSpace {
    /// Used space Windows reports for the drive.
    pub used: u64,
    /// Space on disk the scan accounted for.
    pub scanned: u64,
    /// `used - scanned` (negative when hard links make the scan count files twice).
    pub gap: i64,
    /// The gap is worth explaining: over 1 GB and over 0.5% of what's used. Smaller ones are
    /// files changing while the scan ran.
    pub notable: bool,
    pub parts: Vec<Part>,
}

/// Account for the gap on the drive scanned as `tree` (none unless the scan's root is a drive
/// root). `shadow_bytes` is what [`shadow_storage`] returned, passed in so callers can run that
/// slow query off the UI thread.
pub fn account(tree: &Tree, shadow_bytes: Option<u64>) -> Option<HiddenSpace> {
    let root = tree.path_of(ROOT);
    let s = root.to_string_lossy();
    let b = s.as_bytes();
    if !(b.len() <= 3 && b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') {
        return None;
    }
    let (total, free) = disk_space(&root)?;
    let used = total.saturating_sub(free);
    let scanned = tree.nodes[ROOT].allocated;
    let gap = used as i64 - scanned as i64;
    let mut parts = Vec::new();
    parts.push(Part {
        label: "Shadow copies and restore points",
        bytes: shadow_bytes,
        note: match shadow_bytes {
            Some(_) => "Old versions of files kept by System Restore and backups. Remove older restore points in System Properties > System Protection > Configure, or Disk Cleanup > Clean up system files > More Options.".into(),
            None => "Old versions of files kept by System Restore and backups. Their size can only be read as administrator.".into(),
        },
    });
    if tree.errors > 0 {
        parts.push(Part {
            label: "Folders Sprawl couldn't open",
            bytes: None,
            note: format!("{} folders refused access; their contents aren't in the map. Restart as administrator to see inside them.", tree.errors),
        });
    }
    if gap > 0 {
        parts.push(Part {
            label: "Reserved storage",
            bytes: None,
            note: "Space Windows keeps free for updates, often about 7 GB. Check it with 'DISM /Online /Get-ReservedStorageState' as administrator.".into(),
        });
        parts.push(Part {
            label: "File system records",
            bytes: None,
            note: "The NTFS master file table and other records, usually well under 1% of the drive.".into(),
        });
    } else {
        parts.push(Part {
            label: "Files counted more than once",
            bytes: None,
            note: "Hard links (WinSxS shares most of its files with System32) are counted at every place they appear, so the scan adds up to more than the drive holds.".into(),
        });
    }
    let notable = gap.unsigned_abs() > (1u64 << 30).max(used / 200);
    Some(HiddenSpace { used, scanned, gap, notable, parts })
}

/// Bytes used by shadow copies on the drive at `root`, from WMI. Needs administrator rights;
/// `None` otherwise. Starts PowerShell, so it takes a second or two: call it off the UI thread.
pub fn shadow_storage(root: &Path) -> Option<u64> {
    let volume = volume_guid(root)?;
    let script = "Get-CimInstance Win32_ShadowStorage -ErrorAction Stop | ForEach-Object { $_.Volume.DeviceID + '|' + $_.UsedSpace }";
    let out = run_hidden("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", script], 15)?;
    let mut sum = None;
    for line in out.lines() {
        let Some((dev, used)) = line.trim().rsplit_once('|') else { continue };
        if dev.eq_ignore_ascii_case(&volume) {
            *sum.get_or_insert(0u64) += used.trim().parse::<u64>().ok()?;
        }
    }
    // the query worked but listed nothing for this drive: no shadow copies
    sum.or_else(|| (!out.contains("Access denied")).then_some(0))
}

/// Run a program without a window and return its output, or `None` after `secs` seconds.
fn run_hidden(exe: &str, args: &[&str], secs: u64) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut s);
        s
    });
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success().then(|| reader.join().ok()).flatten(),
            Ok(None) if start.elapsed().as_secs() < secs => std::thread::sleep(std::time::Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    fn GetVolumeNameForVolumeMountPointW(mount: *const u16, name: *mut u16, len: u32) -> i32;
}

fn wide_dir(root: &Path) -> Vec<u16> {
    let mut s = root.to_string_lossy().into_owned();
    if !s.ends_with('\\') {
        s.push('\\');
    }
    s.encode_utf16().chain(Some(0)).collect()
}

#[cfg(windows)]
fn disk_space(root: &Path) -> Option<(u64, u64)> {
    let w = wide_dir(root);
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) } != 0;
    ok.then_some((total, free))
}

/// `\\?\Volume{guid}\` of a drive root.
#[cfg(windows)]
fn volume_guid(root: &Path) -> Option<String> {
    let w = wide_dir(root);
    let mut buf = [0u16; 64];
    let ok = unsafe { GetVolumeNameForVolumeMountPointW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } != 0;
    let n = buf.iter().position(|&c| c == 0)?;
    ok.then(|| String::from_utf16_lossy(&buf[..n]))
}

#[cfg(not(windows))]
fn disk_space(_: &Path) -> Option<(u64, u64)> {
    None
}

#[cfg(not(windows))]
fn volume_guid(_: &Path) -> Option<String> {
    None
}
