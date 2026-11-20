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
    /// (folder index, name, size, file id or 0) of each file to compress, as listed.
    files: Vec<(usize, String, u64, u64)>,
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
    /// What they took on disk before, and take now (every stream, both times), for the files
    /// that could be measured both times.
    pub before: u64,
    pub after: u64,
    /// Files compressed whose size couldn't be measured before or after: their saving is unknown.
    pub unmeasured: usize,
    /// Files another program had open (a sharing or lock violation): left as they are.
    pub in_use: usize,
    /// Files Windows wouldn't compress (an error other than "it wouldn't get smaller").
    pub failed: usize,
    /// Files compressed already (a folder listing doesn't show it, so the plan includes them).
    pub already: usize,
    /// Files that changed since the plan (another file at the name, now a link, a second name,
    /// or under 4 KB now).
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
        let mut entries: Vec<(Vec<u16>, u32, u32, u64, u64)> = Vec::new();
        let listed = crate::walk::dir::list(&plan.dirs[at].0, &mut buf, &mut |name, attrs, tag, size, _, id| {
            if name != [b'.' as u16] && name != [b'.' as u16, b'.' as u16] {
                entries.push((name.to_vec(), attrs, tag, size, id));
            }
            true
        });
        let here = plan.dirs[at].1.clone();
        if !listed {
            return Err(format!("{} couldn't be read completely now, so nothing was compressed", here.display()));
        }
        for (wide, attrs, tag, size, id) in entries {
            let Ok(name) = String::from_utf16(&wide) else {
                return Err(format!("a name in {} can't be read, so nothing was compressed", here.display()));
            };
            let path = here.join(&name);
            let dir = attrs & crate::attr::DIRECTORY != 0;
            if attrs & NOT_LOCAL != 0 {
                return Err(format!("{} is a cloud or offline item, so nothing was compressed", path.display()));
            }
            // a file WOF already compressed is data; any other reparse point is a link, and it
            // refuses the whole folder even where the caller would leave it be
            let wof = attrs & crate::attr::REPARSE_POINT != 0 && !dir && tag == WOF_TAG;
            if attrs & crate::attr::REPARSE_POINT != 0 && !wof {
                return Err(format!("{} is a link, so nothing was compressed (it could lead out of the folder)", path.display()));
            }
            if check(&path, dir)? || wof {
                continue;
            }
            if dir {
                let (h, _) = local::open_rel(&plan.dirs[at].0, &name, READ_DATA | READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE, Disposition::Open, Kind::Folder)
                    .map_err(|e| format!("{} couldn't be opened ({e}), so nothing was compressed", path.display()))?;
                local::check_plain(&h, Kind::Folder).map_err(|_| format!("{} is not a plain folder now, so nothing was compressed", path.display()))?;
                plan.dirs.push((h, path));
            } else if attrs & COMPRESSED == 0 && size >= SMALLEST {
                plan.bytes += size;
                plan.files.push((at, name, size, id));
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
                        let Some((dir, name, size, id)) = plan.files.get(i) else { break };
                        match one(&plan.dirs[*dir].0, name, *id, alg) {
                            Ok(Some(Some((before, after)))) => {
                                d.compressed += 1;
                                d.before += before;
                                d.after += after;
                            }
                            Ok(Some(None)) => {
                                d.compressed += 1;
                                d.unmeasured += 1;
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
        out.unmeasured += d.unmeasured;
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

/// Compress the file `name` (listed as file `id`; 0, when the listing gave none, is left alone
/// as changed: nothing would show it is still the file planned) in the held folder `dir`: Some(on disk before and after, when both could be measured), or None when
/// Windows says it wouldn't get smaller.
fn one(dir: &File, name: &str, id: u64, alg: Alg) -> Result<Option<Option<(u64, u64)>>, Skip> {
    // read access is all WOF needs; no other program may write it meanwhile
    let (f, _) = local::open_rel(dir, name, READ_DATA | READ_ATTRIBUTES, SHARE_READ, Disposition::Open, Kind::File).map_err(|e| match e.raw_os_error() {
        // a sharing or lock violation: a program has it open. Access denied is a failure, with its reason.
        Some(32 | 33) => Skip::InUse,
        Some(2 | 3) => Skip::Changed,
        _ => Skip::Failed(e),
    })?;
    // the open file must be the one the plan listed, and still worth compressing
    let attrs = local::check_plain(&f, Kind::File).map_err(|_| Skip::Changed)?;
    if id == 0 || local::identity(&f).map_err(Skip::Failed)?.1 != id {
        return Err(Skip::Changed);
    }
    let (size, links) = standard(&f).map_err(Skip::Failed)?;
    if links != 1 || size < SMALLEST {
        return Err(Skip::Changed);
    }
    if attrs & COMPRESSED != 0 || backed(&f).map_err(Skip::Failed)? {
        return Err(Skip::Already);
    }
    let before = on_disk(&f).ok();
    // WOF_FILE_COMPRESSION_INFO_V1: the algorithm, no flags
    let info: [u32; 2] = [alg as u32, 0];
    // SAFETY: a valid handle and the 8-byte structure the file provider (2) expects.
    let hr = unsafe { WofSetFileDataLocation(f.as_raw_handle().cast(), 2, info.as_ptr().cast(), 8) };
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
    // the same streams, measured the same way, before and after
    Ok(Some(before.zip(on_disk(&f).ok())))
}

/// (size, number of names) of an open file (FILE_STANDARD_INFO).
fn standard(f: &File) -> io::Result<(u64, u32)> {
    let mut b = [0u64; 3];
    // SAFETY: 24 bytes, class 1 (FileStandardInfo).
    if unsafe { GetFileInformationByHandleEx(f.as_raw_handle().cast(), 1, b.as_mut_ptr().cast(), 24) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((b[1], b[2] as u32))
}

/// What an open file takes on disk, its streams together (a WOF file keeps its compressed data
/// in a stream of its own).
pub fn on_disk(f: &File) -> io::Result<u64> {
    let mut buf = vec![0u64; 512];
    loop {
        // SAFETY: the buffer is the size given; class 7 is FileStreamInfo.
        if unsafe { GetFileInformationByHandleEx(f.as_raw_handle().cast(), 7, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32) } != 0 {
            break;
        }
        let e = io::Error::last_os_error();
        // ERROR_MORE_DATA, ERROR_INSUFFICIENT_BUFFER: a bigger buffer, up to 8 MB
        if matches!(e.raw_os_error(), Some(234 | 122)) && buf.len() < 1 << 20 {
            let more = buf.len() * 4;
            buf.resize(more, 0);
            continue;
        }
        return Err(e);
    }
    // SAFETY: the buffer's own bytes.
    let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len() * 8) };
    let (mut at, mut total) = (0usize, 0u64);
    loop {
        if at + 24 > bytes.len() {
            return Err(io::Error::other("The stream list was cut short"));
        }
        let next = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        total += u64::from_le_bytes(bytes[at + 16..at + 24].try_into().unwrap());
        if next == 0 {
            break;
        }
        at += next;
    }
    Ok(total)
}

/// The WOF algorithm an open file is stored with, or None when WOF's file provider doesn't
/// back it.
pub fn algorithm(f: &File) -> io::Result<Option<u32>> {
    let (got, out) = backing(f)?;
    // WOF_EXTERNAL_INFO (version, provider: 2 is the file provider), then
    // FILE_PROVIDER_EXTERNAL_INFO_V1 (version, algorithm, flags)
    if got >= 16 && u32::from_le_bytes(out[4..8].try_into().unwrap()) == 2 {
        return Ok(Some(u32::from_le_bytes(out[12..16].try_into().unwrap())));
    }
    Ok(None)
}

/// The open file is backed by WOF (or another provider) already.
fn backed(f: &File) -> io::Result<bool> {
    Ok(backing(f)?.0 > 0)
}

/// FSCTL_GET_EXTERNAL_BACKING: (bytes returned, at least 1 when something backs the file, and
/// 0 when nothing does; the output).
fn backing(f: &File) -> io::Result<(u32, [u8; 64])> {
    const FSCTL_GET_EXTERNAL_BACKING: u32 = 0x0009_0310;
    let mut out = [0u8; 64];
    let mut got = 0u32;
    // SAFETY: valid buffers of the sizes given; synchronous handle.
    if unsafe { DeviceIoControl(f.as_raw_handle().cast(), FSCTL_GET_EXTERNAL_BACKING, std::ptr::null(), 0, out.as_mut_ptr().cast(), 64, &mut got, std::ptr::null_mut()) } != 0 {
        return Ok((got.max(1), out));
    }
    let e = io::Error::last_os_error();
    // ERROR_OBJECT_NOT_EXTERNALLY_BACKED; a volume without WOF says it has no such request
    if matches!(e.raw_os_error(), Some(342 | 1 | 50)) { Ok((0, out)) } else { Err(e) }
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

    fn stored_alg(path: &Path) -> Option<u32> {
        algorithm(&File::open(path).unwrap()).unwrap()
    }

    #[test]
    fn each_algorithm_is_stored_as_asked() {
        let owned = local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-wof-alg").unwrap();
        let root = owned.path().to_path_buf();
        let text: Vec<u8> = b"Each algorithm, as asked. ".iter().copied().cycle().take(512 << 10).collect();
        let no = AtomicBool::new(false);
        for alg in [Alg::Xpress4k, Alg::Xpress8k, Alg::Xpress16k, Alg::Lzx] {
            let dir = root.join(format!("{alg:?}"));
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("f.bin"), &text).unwrap();
            let p = plan(&dir, &|_, _| Ok(false), &no).unwrap();
            let d = compress(&p, alg, &no, &mut |_| {});
            drop(p);
            assert_eq!((d.compressed, d.failed), (1, 0), "{alg:?}: {d:?}");
            assert_eq!(stored_alg(&dir.join("f.bin")), Some(alg as u32), "{alg:?} is what Windows stored");
            assert_eq!(std::fs::read(dir.join("f.bin")).unwrap(), text);
        }
        drop(owned);
    }

    #[test]
    fn the_same_streams_are_measured_before_and_after() {
        let owned = local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-wof-streams").unwrap();
        let root = owned.path().to_path_buf();
        let text: Vec<u8> = b"streams, all of them ".iter().copied().cycle().take(1 << 20).collect();
        std::fs::write(root.join("f.bin"), &text).unwrap();
        // a second stream of 2 MB that WOF leaves as it is
        let other: Vec<u8> = (0..2u32 << 20).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let mut ads = root.join("f.bin").into_os_string();
        ads.push(":other");
        std::fs::write(&ads, &other).unwrap();
        let no = AtomicBool::new(false);
        let p = plan(&root, &|_, _| Ok(false), &no).unwrap();
        let d = compress(&p, Alg::Xpress4k, &no, &mut |_| {});
        drop(p);
        assert_eq!((d.compressed, d.unmeasured), (1, 0), "{d:?}");
        // both counts hold the 2 MB stream, so the saving is the main stream's
        assert!(d.before >= 3 << 20 && d.after >= 2 << 20, "{d:?}");
        assert!(d.before - d.after > 900 << 10, "the main stream's saving is credited: {d:?}");
        drop(owned);
    }

    #[test]
    fn a_file_replaced_after_the_plan_is_left_alone() {
        let owned = local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-wof-swap").unwrap();
        let root = owned.path().to_path_buf();
        let text: Vec<u8> = b"swap swap swap ".iter().copied().cycle().take(256 << 10).collect();
        for n in ["swapped.bin", "shrunk.bin", "kept.bin"] {
            std::fs::write(root.join(n), &text).unwrap();
        }
        let no = AtomicBool::new(false);
        let p = plan(&root, &|_, _| Ok(false), &no).unwrap();
        assert_eq!(p.files(), 3);
        // another plain file at the same name, and one that is small now
        std::fs::write(root.join("other.bin"), &text).unwrap();
        std::fs::remove_file(root.join("swapped.bin")).unwrap();
        std::fs::rename(root.join("other.bin"), root.join("swapped.bin")).unwrap();
        std::fs::write(root.join("shrunk.bin"), b"tiny").unwrap();
        let d = compress(&p, Alg::Xpress4k, &no, &mut |_| {});
        drop(p);
        assert_eq!((d.compressed, d.changed), (1, 2), "{d:?}");
        assert_eq!(stored_alg(&root.join("swapped.bin")), None, "the replacement wasn't touched");
        drop(owned);
    }

    #[test]
    fn a_kept_link_still_refuses_the_folder() {
        let owned = local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-wof-keptlink").unwrap();
        let root = owned.path().to_path_buf();
        std::fs::create_dir(root.join("target")).unwrap();
        let link = root.join("out");
        assert!(std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(root.join("target")).output().unwrap().status.success());
        let no = AtomicBool::new(false);
        let r = plan(&root, &|p, _| Ok(p == link), &no);
        std::fs::remove_dir(&link).unwrap();
        assert!(r.is_err_and(|e| e.contains("link")));
        drop(owned);
    }

    #[test]
    fn a_file_access_is_denied_to_is_a_failure_not_in_use() {
        let owned = local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-wof-denied").unwrap();
        let root = owned.path().to_path_buf();
        let text: Vec<u8> = b"denied denied ".iter().copied().cycle().take(256 << 10).collect();
        let f = root.join("denied.bin");
        std::fs::write(&f, &text).unwrap();
        let no = AtomicBool::new(false);
        let p = plan(&root, &|_, _| Ok(false), &no).unwrap();
        // Everyone may not read it (the owner can still put the access back)
        assert!(std::process::Command::new("icacls").arg(&f).args(["/deny", "*S-1-1-0:(RD)"]).output().unwrap().status.success());
        let d = compress(&p, Alg::Xpress4k, &no, &mut |_| {});
        drop(p);
        assert!(std::process::Command::new("icacls").arg(&f).arg("/reset").output().unwrap().status.success());
        if d.compressed == 1 {
            // an elevated run opens it for backup anyway
            eprintln!("skipped: backup intent let the open through");
        } else {
            assert_eq!((d.in_use, d.failed), (0, 1), "{d:?}");
            assert!(d.first_error.as_deref().is_some_and(|e| e.contains("denied.bin")), "{d:?}");
        }
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
