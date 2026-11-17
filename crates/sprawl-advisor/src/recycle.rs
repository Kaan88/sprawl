//! "Send my own files to the Recycle Bin": where an item goes instead of being deleted.
//!
//! Sprawl moves the item itself, through the handle the clean-up opened and checked (see
//! [`crate::clean::run_with`]), into the user's own folder of the volume's Recycle Bin, and
//! writes the bin's `$I` record beside it so Explorer lists it and can put it back. It never asks
//! the shell to recycle: with no confirmations, the shell deletes for good what doesn't fit.
//!
//! An item that can't go to the bin (a network folder, a removable drive, a bin that is off or
//! not set up, too big for the bin) is left as it is, with the reason. It is never deleted
//! instead.

use std::path::{Path, PathBuf};

/// The Recycle Bin of one volume, for this user.
#[derive(Clone, Debug)]
pub struct Bin {
    /// `X:\$Recycle.Bin\<user SID>`.
    pub dir: PathBuf,
    /// The drive, as the user knows it (`C:`).
    pub drive: String,
    /// How much more this clean-up may send to it, in bytes: what the bin holds at most.
    pub room: u64,
}

/// The Recycle Bin for `path`'s volume, or why there is none Sprawl can use.
pub fn bin_for(path: &Path) -> Result<Bin, String> {
    let s = path.to_string_lossy();
    let b = s.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || (b[2] != b'\\' && b[2] != b'/') {
        return Err("a network folder has no Recycle Bin".into());
    }
    let drive = format!("{}:", (b[0] as char).to_ascii_uppercase());
    let root = format!("{drive}\\");
    if win::drive_type(&root) != 3 {
        return Err(format!("{drive} has no Recycle Bin Sprawl can use (only fixed drives do)"));
    }
    let sid = win::user_sid().ok_or("Windows couldn't say which account this is")?;
    let dir = PathBuf::from(format!(r"{root}$Recycle.Bin\{sid}"));
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if m.is_dir() && !is_link(&m) => {}
        _ => return Err(format!("the Recycle Bin on {drive} isn't set up for you yet (delete any file there with Explorer once, then try again)")),
    }
    let key = win::volume_guid(&root).and_then(|g| crate::reg::Key::open(crate::reg::HKCU, &format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{g}"), crate::reg::View::Native));
    if key.as_ref().and_then(|k| k.dword("NukeOnDelete")) == Some(1) {
        return Err(format!("the Recycle Bin is turned off on {drive}"));
    }
    // its size: as set in its properties, else Windows' default share of the drive
    let cap = match key.as_ref().and_then(|k| k.dword("MaxCapacity")) {
        Some(mb) => u64::from(mb) << 20,
        None => win::total_bytes(&root).ok_or(format!("the size of {drive} couldn't be read"))? / 20,
    };
    // what is in it already counts: Windows doesn't hold to the limit for what Sprawl puts there
    let room = cap.saturating_sub(held(&dir));
    Ok(Bin { dir, drive, room })
}

/// What the bin folder `dir` holds now, by its `$I` records' sizes (each item's size as it was
/// deleted, also for a whole folder). A record that can't be read counts its item's own size.
pub(crate) fn held(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut total = 0u64;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix("$I") else { continue };
        let from_record = std::fs::read(e.path()).ok().filter(|r| r.len() >= 16).map(|r| u64::from_le_bytes(r[8..16].try_into().unwrap()));
        total = total.saturating_add(from_record.unwrap_or_else(|| std::fs::symlink_metadata(dir.join(format!("$R{rest}"))).map_or(0, |m| m.len())));
    }
    total
}

fn is_link(m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & 0x400 != 0
}

/// The names an item gets in the bin: `$R` and `$I`, the same six characters, its extension.
pub(crate) fn names(original: &Path, seed: u64) -> (String, String) {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
    let mut id = String::new();
    for _ in 0..6 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        id.push(CHARS[(x % CHARS.len() as u64) as usize] as char);
    }
    let ext = original.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (format!("$R{id}{ext}"), format!("$I{id}{ext}"))
}

/// The bin's `$I` record (version 2, Windows 10 and later): the size, when it was deleted, and
/// where it came from, so Explorer lists it and Restore puts it back.
pub(crate) fn record(original: &Path, bytes: u64, deleted_filetime: i64) -> Vec<u8> {
    let path: Vec<u16> = original.to_string_lossy().encode_utf16().chain(Some(0)).collect();
    let mut r = Vec::with_capacity(28 + path.len() * 2);
    r.extend_from_slice(&2i64.to_le_bytes());
    r.extend_from_slice(&bytes.to_le_bytes());
    r.extend_from_slice(&deleted_filetime.to_le_bytes());
    r.extend_from_slice(&(path.len() as u32).to_le_bytes());
    for c in path {
        r.extend_from_slice(&c.to_le_bytes());
    }
    r
}

mod win {
    use std::ffi::c_void;

    type H = *mut c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
        fn GetVolumeNameForVolumeMountPointW(mount: *const u16, name: *mut u16, len: u32) -> i32;
        fn GetDiskFreeSpaceExW(dir: *const u16, free_to_caller: *mut u64, total: *mut u64, free: *mut u64) -> i32;
        fn GetCurrentProcess() -> H;
        #[link_name = "CloseHandle"]
        fn close_handle(h: isize) -> i32;
        fn LocalFree(p: *mut c_void) -> *mut c_void;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(process: H, access: u32, token: *mut H) -> i32;
        fn GetTokenInformation(token: H, class: i32, buf: *mut c_void, len: u32, needed: *mut u32) -> i32;
        fn ConvertSidToStringSidW(sid: *mut c_void, out: *mut *mut u16) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub(super) fn drive_type(root: &str) -> u32 {
        unsafe { GetDriveTypeW(wide(root).as_ptr()) }
    }

    /// `{guid}` of the volume mounted at `root`.
    pub(super) fn volume_guid(root: &str) -> Option<String> {
        let mut buf = [0u16; 64];
        if unsafe { GetVolumeNameForVolumeMountPointW(wide(root).as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } == 0 {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let name = String::from_utf16_lossy(&buf[..n]);
        let a = name.find('{')?;
        let b = name.find('}')?;
        (a < b).then(|| name[a..=b].to_string())
    }

    pub(super) fn total_bytes(root: &str) -> Option<u64> {
        let (mut a, mut t, mut f) = (0u64, 0u64, 0u64);
        (unsafe { GetDiskFreeSpaceExW(wide(root).as_ptr(), &mut a, &mut t, &mut f) } != 0).then_some(t)
    }

    /// This process's account, as `S-1-5-21-...`.
    pub(super) fn user_sid() -> Option<String> {
        const TOKEN_QUERY: u32 = 0x8;
        const TOKEN_USER: i32 = 1;
        let mut token: H = std::ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let mut buf = vec![0u64; 64];
        let mut need = 0u32;
        let ok = unsafe { GetTokenInformation(token, TOKEN_USER, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, &mut need) };
        unsafe { close_handle(token as isize) };
        if ok == 0 {
            return None;
        }
        // TOKEN_USER starts with a SID_AND_ATTRIBUTES whose first field points at the SID
        let sid = buf[0] as usize as *mut c_void;
        let mut out: *mut u16 = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid, &mut out) } == 0 || out.is_null() {
            return None;
        }
        let mut n = 0;
        while unsafe { *out.add(n) } != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(out, n) });
        unsafe { LocalFree(out.cast()) };
        s.starts_with("S-1-").then_some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The room left counts what the bin already holds, by its records.
    #[test]
    fn what_the_bin_holds_is_counted() {
        let dir = std::env::temp_dir().join(format!("sprawl-held-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("$IAAAAAA.txt"), record(Path::new(r"C:\a.txt"), 700, 0)).unwrap();
        std::fs::write(dir.join("$RAAAAAA.txt"), vec![0u8; 700]).unwrap();
        std::fs::write(dir.join("$IBBBBBB"), record(Path::new(r"C:\folder"), 5000, 0)).unwrap();
        std::fs::create_dir_all(dir.join("$RBBBBBB")).unwrap();
        // a short record: its item's own size
        std::fs::write(dir.join("$ICCCCCC.bin"), b"x").unwrap();
        std::fs::write(dir.join("$RCCCCCC.bin"), vec![0u8; 30]).unwrap();
        std::fs::write(dir.join("desktop.ini"), b"[x]").unwrap();
        assert_eq!(held(&dir), 700 + 5000 + 30);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_record_is_what_explorer_reads() {
        let r = record(Path::new(r"C:\Users\me\Old project"), 1234, 0x01DA_0000_0000_0000);
        assert_eq!(&r[0..8], &2i64.to_le_bytes());
        assert_eq!(u64::from_le_bytes(r[8..16].try_into().unwrap()), 1234);
        assert_eq!(i64::from_le_bytes(r[16..24].try_into().unwrap()), 0x01DA_0000_0000_0000);
        let n = u32::from_le_bytes(r[24..28].try_into().unwrap()) as usize;
        assert_eq!(n, r"C:\Users\me\Old project".len() + 1, "characters, with the closing zero");
        assert_eq!(r.len(), 28 + n * 2);
        let back: Vec<u16> = r[28..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(String::from_utf16_lossy(&back[..n - 1]), r"C:\Users\me\Old project");
        assert_eq!(back[n - 1], 0);
    }

    #[test]
    fn names_pair_up_and_keep_the_extension() {
        let (r, i) = names(Path::new(r"C:\x\report.final.docx"), 7);
        assert!(r.starts_with("$R") && i.starts_with("$I") && r[2..] == i[2..], "{r} {i}");
        assert!(r.ends_with(".docx") && r.len() == "$R".len() + 6 + ".docx".len());
        let (r2, _) = names(Path::new(r"C:\x\folder"), 8);
        assert_eq!(r2.len(), 8);
        assert_ne!(names(Path::new("a"), 1).0, names(Path::new("a"), 2).0);
    }

    /// Read-only: the system drive's bin is found (its folder and settings only are read), or
    /// the reason is said.
    #[test]
    fn the_system_drive_has_a_bin_or_says_why() {
        let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        match bin_for(&Path::new(&format!(r"{sys}\Windows"))) {
            Ok(b) => {
                assert!(b.dir.to_string_lossy().contains(r"$Recycle.Bin\S-1-"), "{:?}", b.dir);
                assert!(b.room > 0);
                println!("{} {} MB", b.dir.display(), b.room >> 20);
            }
            Err(why) => assert!(!why.is_empty()),
        }
    }

    #[test]
    fn no_bin_for_a_network_folder() {
        assert!(bin_for(Path::new(r"\\server\share\x")).unwrap_err().contains("network"));
        assert!(bin_for(Path::new(r"relative\x")).is_err());
    }
}
