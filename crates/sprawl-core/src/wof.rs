//! Compressing files in place with Windows' own WOF compression (what `compact /exe` does),
//! in this process, on several threads, through handles Sprawl holds.
//!
//! [`plan`] reads the folder first: each folder is opened through its parent's handle as itself
//! (a link is never followed) and listed through its own handle, and it stays open (held, no
//! delete sharing) until the work is done. Anything that makes the folder unsafe to go through
//! (a link, a cloud or offline item, a name that can't be read, a folder the caller refuses)
//! stops the plan before a single file is changed. [`compress`] then opens each planned file
//! through its held folder's handle, checks it is still a plain local file with one name that
//! isn't compressed yet, and compresses that open file. No path is handed to another program,
//! and no pathname is resolved again.

use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::local::{self, Disposition, Kind, READ_ATTRIBUTES, READ_DATA, SHARE_READ, SHARE_WRITE};

/// The WOF compression algorithms (`compact /exe:<name>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alg {
    Xpress4k = 0,
    Lzx = 1,
    Xpress8k = 2,
    Xpress16k = 3,
}

impl Alg {
    /// From compact.exe's name for it (`xpress8k`).
    pub fn parse(name: &str) -> Option<Alg> {
        match name.to_ascii_lowercase().as_str() {
            "xpress4k" => Some(Alg::Xpress4k),
            "xpress8k" => Some(Alg::Xpress8k),
            "xpress16k" => Some(Alg::Xpress16k),
            "lzx" => Some(Alg::Lzx),
            _ => None,
        }
    }
}

#[link(name = "wofutil")]
extern "system" {
    fn WofSetFileDataLocation(file: *mut core::ffi::c_void, provider: u32, info: *const core::ffi::c_void, len: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn DeviceIoControl(dev: *mut core::ffi::c_void, code: u32, inb: *const core::ffi::c_void, inlen: u32, outb: *mut core::ffi::c_void, outlen: u32, ret: *mut u32, ov: *mut core::ffi::c_void) -> i32;
    fn GetFileInformationByHandleEx(file: *mut core::ffi::c_void, class: i32, info: *mut core::ffi::c_void, size: u32) -> i32;
}

const WOF_TAG: u32 = 0x8000_0017;
/// Cloud placeholder, recall on open or on data access, offline.
const NOT_LOCAL: u32 = 0x1000 | 0x0004_0000 | 0x0040_0000;
const COMPRESSED: u32 = 0x800;
/// Smaller files gain nothing (WOF works in 4 KB chunks).
const SMALLEST: u64 = 4096;

/// The work, read and checked before anything changes.
pub struct Plan {
    /// Every folder in it, held open (the first is the root), with its path.
    dirs: Vec<(File, PathBuf)>,
    /// (folder index, name, size) of each file to compress.
    files: Vec<(usize, String, u64)>,
    /// The root's chain from the drive down, held.
    _root: local::Directory,
    /// What the planned files hold, in bytes.
    pub bytes: u64,
}

impl Plan {
    pub fn files(&self) -> usize { self.files.len() }
}

/// What [`compress`] did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Done {
    /// Files compressed now.
    pub compressed: usize,
    /// What they took on disk before, and take now.
    pub before: u64,
    pub after: u64,
    /// Files another program had open (or that couldn't be opened for this): left as they are.
    pub in_use: usize,
    /// Files Windows wouldn't compress (an error other than "it wouldn't get smaller").
    pub failed: usize,
    /// Files compressed already (a folder listing doesn't show it, so the plan includes them).
    pub already: usize,
    /// Files that changed since the plan (now a link, or a second name).
    pub changed: usize,
    pub stopped: bool,
    /// The first error seen, for the log.
    pub first_error: Option<String>,
}

/// Read `root` for [`compress`]. `check(path, is_dir)` is asked of every item: `Err` refuses
/// the whole folder, `Ok(true)` leaves the item (and, for a folder, all in it) as it is.
pub fn plan(root: &Path, check: &dyn Fn(&Path, bool) -> Result<bool, String>, stop: &AtomicBool) -> Result<Plan, String> {
    let held = local::directory(root).map_err(|e| format!("{} couldn't be opened ({e})", root.display()))?;
    let first = held.handle().and_then(|h| h.try_clone()).map_err(|e| e.to_string())?;
    let mut plan = Plan { dirs: vec![(first, held.path.clone())], files: Vec::new(), _root: held, bytes: 0 };
    let mut buf = Vec::new();
    let mut at = 0;
    while at < plan.dirs.len() {
        if stop.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        let mut entries: Vec<(Vec<u16>, u32, u32, u64)> = Vec::new();
        let listed = crate::walk::dir::list(&plan.dirs[at].0, &mut buf, &mut |name, attrs, tag, size, _, _| {
            if name != [b'.' as u16] && name != [b'.' as u16, b'.' as u16] {
                entries.push((name.to_vec(), attrs, tag, size));
            }
            true
        });
        let here = plan.dirs[at].1.clone();
        if !listed {
            return Err(format!("{} couldn't be read completely now, so nothing was compressed", here.display()));
        }
        for (wide, attrs, tag, size) in entries {
            let Ok(name) = String::from_utf16(&wide) else {
                return Err(format!("a name in {} can't be read, so nothing was compressed", here.display()));
            };
            let path = here.join(&name);
            let dir = attrs & crate::attr::DIRECTORY != 0;
            if attrs & NOT_LOCAL != 0 {
                return Err(format!("{} is a cloud or offline item, so nothing was compressed", path.display()));
            }
            if check(&path, dir)? {
                continue;
            }
            if attrs & crate::attr::REPARSE_POINT != 0 {
                // a file WOF already compressed is data; anything else is a link
                if !dir && tag == WOF_TAG {
                    continue;
                }
                return Err(format!("{} is a link, so nothing was compressed (it could lead out of the folder)", path.display()));
            }
            if dir {
                let (h, _) = local::open_rel(&plan.dirs[at].0, &name, READ_DATA | READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE, Disposition::Open, Kind::Folder)
                    .map_err(|e| format!("{} couldn't be opened ({e}), so nothing was compressed", path.display()))?;
                local::check_plain(&h, Kind::Folder).map_err(|_| format!("{} is not a plain folder now, so nothing was compressed", path.display()))?;
                plan.dirs.push((h, path));
            } else if attrs & COMPRESSED == 0 && size >= SMALLEST {
                plan.bytes += size;
                plan.files.push((at, name, size));
            }
        }
        at += 1;
    }
    Ok(plan)
}

/// Compress every planned file with `alg`, on several threads. `progress(bytes)` is called
/// about every 100 ms with how much of [`Plan::bytes`] has been gone through. Stop ends it
/// after the files being compressed at that moment.
pub fn compress(plan: &Plan, alg: Alg, stop: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Done {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8).min(plan.files.len().max(1));
    let next = AtomicUsize::new(0);
    let gone_through = AtomicU64::new(0);
    let finished = AtomicUsize::new(0);
    let results: Vec<Done> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut d = Done::default();
                    loop {
                        if stop.load(Ordering::Relaxed) {
                            d.stopped = true;
                            break;
                        }
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((dir, name, size)) = plan.files.get(i) else { break };
                        match one(&plan.dirs[*dir].0, name, alg) {
                            Ok(Some((before, after))) => {
                                d.compressed += 1;
                                d.before += before;
                                d.after += after;
                            }
                            Ok(None) => {}
                            Err(Skip::InUse) => d.in_use += 1,
                            Err(Skip::Already) => d.already += 1,
                            Err(Skip::Changed) => d.changed += 1,
                            Err(Skip::Failed(e)) => {
                                d.failed += 1;
                                if d.first_error.is_none() {
                                    d.first_error = Some(format!("{}: {e}", plan.dirs[*dir].1.join(name).display()));
                                }
                            }
                        }
                        gone_through.fetch_add(*size, Ordering::Relaxed);
                    }
                    finished.fetch_add(1, Ordering::Relaxed);
                    d
                })
            })
            .collect();
        while finished.load(Ordering::Relaxed) < threads {
            progress(gone_through.load(Ordering::Relaxed));
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        workers.into_iter().map(|w| w.join().unwrap_or_default()).collect()
    });
    progress(gone_through.load(Ordering::Relaxed));
    let mut out = Done::default();
    for d in results {
        out.compressed += d.compressed;
        out.before += d.before;
        out.after += d.after;
        out.in_use += d.in_use;
        out.already += d.already;
        out.failed += d.failed;
        out.changed += d.changed;
        out.stopped |= d.stopped;
        if out.first_error.is_none() {
            out.first_error = d.first_error;
        }
    }
    out
}

enum Skip {
    InUse,
    Already,
    Changed,
    Failed(io::Error),
}

/// Compress the file `name` in the held folder `dir`: (on disk before, on disk after), or None
/// when Windows says it wouldn't get smaller.
fn one(dir: &File, name: &str, alg: Alg) -> Result<Option<(u64, u64)>, Skip> {
    // read access is all WOF needs; no other program may write it meanwhile
    let (f, _) = local::open_rel(dir, name, READ_DATA | READ_ATTRIBUTES, SHARE_READ, Disposition::Open, Kind::File).map_err(|e| match e.raw_os_error() {
        Some(32 | 33 | 5) => Skip::InUse,
        Some(2 | 3) => Skip::Changed,
        _ => Skip::Failed(e),
    })?;
    local::check_plain(&f, Kind::File).map_err(|_| Skip::Changed)?;
    let (alloc, links) = standard(&f).map_err(Skip::Failed)?;
    if links != 1 {
        return Err(Skip::Changed);
    }
    if backed(&f).map_err(Skip::Failed)? {
        return Err(Skip::Already);
    }
    // FILE_PROVIDER_EXTERNAL_INFO_V1: version 1, the algorithm, no flags
    let info: [u32; 3] = [1, alg as u32, 0];
    // SAFETY: a valid handle and a 12-byte structure, as the call expects.
    let hr = unsafe { WofSetFileDataLocation(f.as_raw_handle().cast(), 2, info.as_ptr().cast(), 12) };
    if hr < 0 {
        // ERROR_COMPRESSION_NOT_BENEFICIAL: left as it was, which is fine
        if hr as u32 == 0x8007_0158 {
            return Ok(None);
        }
        return Err(match hr as u32 {
            0x8007_0020 | 0x8007_0021 => Skip::InUse,
            _ => Skip::Failed(io::Error::from_raw_os_error(if hr as u32 & 0xFFFF_0000 == 0x8007_0000 { hr & 0xFFFF } else { hr })),
        });
    }
    Ok(Some((alloc, on_disk(&f).unwrap_or(alloc))))
}

/// (allocation size, number of names) of an open file (FILE_STANDARD_INFO).
fn standard(f: &File) -> io::Result<(u64, u32)> {
    let mut b = [0u64; 3];
    // SAFETY: 24 bytes, class 1 (FileStandardInfo).
    if unsafe { GetFileInformationByHandleEx(f.as_raw_handle().cast(), 1, b.as_mut_ptr().cast(), 24) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((b[0], b[2] as u32))
}

/// What an open file takes on disk, its streams together (a WOF file keeps its compressed data
/// in a stream of its own).
pub fn on_disk(f: &File) -> io::Result<u64> {
    let mut buf = vec![0u64; 512];
    // SAFETY: the buffer is the size given; class 7 is FileStreamInfo.
    if unsafe { GetFileInformationByHandleEx(f.as_raw_handle().cast(), 7, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len() * 8) };
    let (mut at, mut total) = (0usize, 0u64);
    loop {
        let next = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        total += u64::from_le_bytes(bytes[at + 16..at + 24].try_into().unwrap());
        if next == 0 || at + next + 24 > bytes.len() {
            break;
        }
        at += next;
    }
    Ok(total)
}

/// The open file is backed by WOF (or another provider) already.
fn backed(f: &File) -> io::Result<bool> {
    const FSCTL_GET_EXTERNAL_BACKING: u32 = 0x0009_0310;
    let mut out = [0u8; 64];
    let mut got = 0u32;
    // SAFETY: valid buffers of the sizes given; synchronous handle.
    if unsafe { DeviceIoControl(f.as_raw_handle().cast(), FSCTL_GET_EXTERNAL_BACKING, std::ptr::null(), 0, out.as_mut_ptr().cast(), 64, &mut got, std::ptr::null_mut()) } != 0 {
        return Ok(true);
    }
    let e = io::Error::last_os_error();
    // ERROR_OBJECT_NOT_EXTERNALLY_BACKED; a volume without WOF says it has no such request
    if matches!(e.raw_os_error(), Some(342 | 1 | 50)) { Ok(false) } else { Err(e) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_compressed_in_place_through_held_handles() {
        let owned = local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-wof").unwrap();
        let root = owned.path().to_path_buf();
        std::fs::create_dir_all(root.join(r"bin\deep")).unwrap();
        let text: Vec<u8> = b"Sprawl compresses this line well. ".iter().copied().cycle().take(1 << 20).collect();
        for p in [r"a.dll", r"bin\b.exe", r"bin\deep\c.dat"] {
            std::fs::write(root.join(p), &text).unwrap();
        }
        std::fs::write(root.join("tiny.txt"), b"small").unwrap();
        std::fs::write(root.join("kept.bin"), &text).unwrap();
        let no = AtomicBool::new(false);
        let kept = root.join("kept.bin");
        let p = plan(&root, &|p, _| Ok(p == kept), &no).unwrap();
        assert_eq!((p.files(), p.bytes), (3, 3 << 20), "the tiny file and the kept one aren't planned");
        let mut seen = 0;
        let d = compress(&p, Alg::Xpress4k, &no, &mut |b| seen = b);
        drop(p);
        assert_eq!((d.compressed, d.in_use, d.failed, d.changed, seen), (3, 0, 0, 0, 3 << 20), "{d:?}");
        assert!(d.after * 4 < d.before, "{d:?}");
        // compressed now, and the same bytes when read
        assert_eq!(std::fs::read(root.join(r"bin\deep\c.dat")).unwrap(), text);
        // again, without the kept file: the three are seen as compressed already
        let p = plan(&root, &|_, _| Ok(false), &no).unwrap();
        let d = compress(&p, Alg::Xpress4k, &no, &mut |_| {});
        drop(p);
        assert_eq!((d.compressed, d.already, d.changed), (1, 3, 0), "{d:?}");
        // a file with a second name is left alone
        std::fs::write(root.join("twin.bin"), &text).unwrap();
        std::fs::hard_link(root.join("twin.bin"), root.join(r"bin\twin2.bin")).unwrap();
        let p = plan(&root, &|_, _| Ok(false), &no).unwrap();
        let d = compress(&p, Alg::Xpress4k, &no, &mut |_| {});
        drop(p);
        assert_eq!((d.compressed, d.already, d.changed), (0, 4, 2), "{d:?}");
        std::fs::remove_file(root.join(r"bin\twin2.bin")).unwrap();
        // a link inside: the whole folder is refused before anything changes
        std::fs::create_dir(root.join("target")).unwrap();
        let link = root.join("bin").join("out");
        assert!(std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(root.join("target")).output().unwrap().status.success());
        assert!(plan(&root, &|_, _| Ok(false), &no).is_err_and(|e| e.contains("link")));
        std::fs::remove_dir(&link).unwrap();
        // a refused item refuses the whole folder
        assert!(plan(&root, &|p, _| if p.ends_with("deep") { Err("skipped".into()) } else { Ok(false) }, &no).is_err());
        drop(owned);
    }

    #[test]
    fn a_file_another_program_writes_is_left_alone() {
        let owned = local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-wof-busy").unwrap();
        let root = owned.path().to_path_buf();
        let text: Vec<u8> = b"busy busy busy ".iter().copied().cycle().take(256 << 10).collect();
        std::fs::write(root.join("busy.bin"), &text).unwrap();
        std::fs::write(root.join("free.bin"), &text).unwrap();
        let no = AtomicBool::new(false);
        let p = plan(&root, &|_, _| Ok(false), &no).unwrap();
        let busy = std::fs::OpenOptions::new().write(true).open(root.join("busy.bin")).unwrap();
        let d = compress(&p, Alg::Xpress8k, &no, &mut |_| {});
        drop((busy, p));
        assert_eq!((d.compressed, d.in_use), (1, 1), "{d:?}");
        drop(owned);
    }
}
