//! Local paths pinned against ancestor replacement. Reject remote/device aliases before I/O.
use std::fs::File;
use std::io;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetFinalPathNameByHandleW, GetFileInformationByHandleEx};

const FLAGS: u32 = 0x0200_0000 | 0x0020_0000 | 0x0010_0000; // backup, open reparse, no recall

#[link(name = "wofutil")]
extern "system" {
    fn WofIsExternalFile(path: *const u16, external: *mut i32, provider: *mut u32, info: *mut core::ffi::c_void, len: *mut u32) -> i32;
}

pub fn lexical(path: &Path) -> io::Result<PathBuf> {
    let raw = path.to_str().ok_or_else(|| io::Error::other("The path isn't valid Unicode"))?;
    let raw = if let Some(rest) = raw.strip_prefix(r"\\?\") {
        let b = rest.as_bytes();
        if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
            return Err(io::Error::other("Network and device paths aren't permitted here"));
        }
        rest
    } else { raw };
    if raw.starts_with(['\\', '/']) || raw.contains(['\0', '\r', '\n', '*', '?']) { return Err(io::Error::other("Network, device and wildcard paths aren't permitted here")); }
    let path = std::path::absolute(Path::new(raw))?;
    let s = path.to_str().ok_or_else(|| io::Error::other("The path isn't valid Unicode"))?;
    let b = s.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || !matches!(b[2], b'\\' | b'/')
        || s[3..].contains(':') || path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(io::Error::other("A plain local drive path is required"));
    }
    let root = [b[0] as u16, b':' as u16, b'\\' as u16, 0];
    // A drive-letter query only; never probe a UNC or a mapped remote filesystem.
    let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
    if !matches!(kind, 2 | 3 | 5 | 6) { return Err(io::Error::other("The drive isn't a known local drive")); }
    Ok(path)
}

/// A scan root on a network share: a `\\server\share` path or a mapped remote drive. Scans
/// there only list names (no pins, no WOF probes, no MFT), as before 1.0. Every storage
/// action still goes through `lexical`, which refuses these, so nothing on a share is changed.
pub fn remote_scan_root(path: &Path) -> Option<PathBuf> {
    // The spelling given, the normalized one, and its long-name form must all be a share or a
    // mapped drive: `\\?\UNC\.\C:\x` would normalize to the device path `\\.\C:\x`. The long
    // name is looked up only for a spelling that already passed.
    remote_spelling(path.to_str()?)?;
    let normal = crate::root_spelling(path);
    remote_spelling(&normal)?;
    let long = crate::long_name(&normal).unwrap_or(normal);
    remote_spelling(&long)?;
    Some(PathBuf::from(long))
}

/// A share's server or share name: no device-like (`.`, `?`), empty, dot-only or trailing
/// dot/space name, nothing a path parser reads as more than one name.
fn share_component(c: &str) -> bool {
    !c.is_empty() && !c.starts_with(['.', '?']) && !c.ends_with(['.', ' '])
        && !c.contains([':', '\0', '\r', '\n', '*', '?', '"', '<', '>', '|', '/'])
}

/// [`remote_scan_root`]'s check of one spelling: `\\server\share\...`, `\\?\UNC\server\share\...`
/// (one validator for both), or `Y:\...` on a mapped remote drive.
fn remote_spelling(raw: &str) -> Option<()> {
    let raw = raw.replace('/', "\\");
    let share = raw.strip_prefix(r"\\?\UNC\").or_else(|| raw.strip_prefix(r"\\").filter(|r| !r.starts_with(['?', '.'])));
    let tail = if let Some(rest) = share {
        let mut parts = rest.split('\\');
        if !parts.next().is_some_and(share_component) || !parts.next().is_some_and(share_component) { return None; }
        rest
    } else {
        let s = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
        let b = s.as_bytes();
        // `Y:\...` only: `Y:` and `Y:folder` name the drive's current folder, not its root
        if s.starts_with('\\') || b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' { return None; }
        let root = [b[0] as u16, b':' as u16, b'\\' as u16, 0];
        if unsafe { GetDriveTypeW(root.as_ptr()) } != 4 { return None; }
        &s[2..]
    };
    if tail.contains([':', '\0', '\r', '\n', '*', '?', '"', '<', '>', '|']) || tail.split('\\').any(|c| c == ".." || c == ".") { return None; }
    Some(())
}

/// The root of a scan: pinned when local, listed as it is when on a network share.
pub struct ScanRoot {
    pub path: PathBuf,
    pub pinned: Option<Directory>,
}

/// `allow_wof` as in [`storage`]: a WOF file may itself be the root.
pub fn scan_root(path: &Path, allow_wof: bool) -> io::Result<ScanRoot> {
    if let Some(path) = remote_scan_root(path) { return Ok(ScanRoot { path, pinned: None }); }
    let d = pin(path, false, allow_wof)?;
    Ok(ScanRoot { path: d.path.clone(), pinned: Some(d) })
}

pub fn final_path(file: &File) -> io::Result<PathBuf> {
    let mut out = vec![0u16; 512];
    loop {
        let n = unsafe { GetFinalPathNameByHandleW(file.as_raw_handle().cast(), out.as_mut_ptr(), out.len() as u32, 0) } as usize;
        if n == 0 { return Err(io::Error::last_os_error()); }
        if n < out.len() { return Ok(PathBuf::from(String::from_utf16_lossy(&out[..n]))); }
        if n > 32_768 { return Err(io::Error::other("The final path is too long")); }
        out.resize(n + 1, 0);
    }
}

fn same(a: &Path, b: &Path) -> bool {
    let key = |p: &Path| p.to_string_lossy().trim_start_matches(r"\\?\").trim_end_matches('\\').replace('/', "\\").to_lowercase();
    key(a) == key(b)
}

#[derive(Debug)]
pub struct Directory {
    pub path: PathBuf,
    _handles: Vec<File>,
}

impl Directory {
    pub fn file_links(&self) -> io::Result<u32> {
        use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
        let file = self._handles.last().ok_or_else(|| io::Error::other("No pinned file"))?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 { return Err(io::Error::last_os_error()); }
        Ok(info.nNumberOfLinks)
    }

    pub fn metadata(&self) -> io::Result<std::fs::Metadata> {
        self._handles.last().ok_or_else(|| io::Error::other("No pinned path"))?.metadata()
    }
    /// WOF intentionally hides its reparse attribute from ordinary directory listings.
    /// Query only while this exact local path and all its ancestors remain pinned.
    pub fn externally_backed(&self) -> io::Result<bool> {
        let mut name = crate::walk::long_path(&self.path);
        name.push(0);
        let (mut external, mut provider) = (0i32, 0u32);
        let hr = unsafe { WofIsExternalFile(name.as_ptr(), &mut external, &mut provider, std::ptr::null_mut(), std::ptr::null_mut()) };
        if hr < 0 { return Err(io::Error::other(format!("WOF backing could not be checked: {hr:08X}"))); }
        Ok(external != 0)
    }

    /// Query a listed plain child while this parent chain and the exact child remain pinned.
    /// Metadata access only: no file contents, cloud recall, or leaf redirection.
    pub(crate) fn child_externally_backed(&self, name: &str) -> io::Result<bool> {
        let parts: Vec<_> = Path::new(name).components().collect();
        if !matches!(parts.as_slice(), [std::path::Component::Normal(_)]) || name.contains([':', '\\', '/', '\0', '*', '?']) {
            return Err(io::Error::other("Not one plain child name"));
        }
        let path = self.path.join(name);
        let leaf = std::fs::OpenOptions::new().access_mode(0x80).share_mode(1).custom_flags(FLAGS).open(&path)?;
        let metadata = leaf.metadata()?;
        if !metadata.is_file() || metadata.file_attributes() & (crate::attr::REPARSE_POINT | 0x1000 | 0x0004_0000 | 0x0040_0000) != 0 {
            return Err(io::Error::other("The listed child is no longer a plain local file"));
        }
        if !same(&final_path(&leaf)?, &crate::canonical_root(&path)) { return Err(io::Error::other("The listed child resolved elsewhere")); }
        Directory { path, _handles: vec![leaf] }.externally_backed()
    }
}

/// The hard-link count of a listed leaf, opened as itself (no reparse follow, no recall).
/// Only the leaf is opened: for measuring, not for acting on the file.
pub fn links(path: &Path) -> io::Result<u32> {
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    let path = lexical(path)?;
    let h = std::fs::OpenOptions::new().access_mode(0x80).share_mode(1 | 2 | 4).custom_flags(FLAGS).open(&path)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(h.as_raw_handle().cast(), &mut info) } == 0 { return Err(io::Error::last_os_error()); }
    Ok(info.nNumberOfLinks)
}

/// What one file takes on disk (compressed files, WOF ones included, at their compressed size),
/// measured on the file itself: opened as itself first (a link or other reparse point other than
/// WOF, a cloud or offline file, or a file with other names is refused), then opened through the
/// compression filter and checked to be the same file before its size is read. Measuring only.
pub fn file_on_disk(path: &Path) -> io::Result<u64> {
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    let path = lexical(path)?;
    let id = |f: &File| -> io::Result<BY_HANDLE_FILE_INFORMATION> {
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(f.as_raw_handle().cast(), &mut info) } == 0 { return Err(io::Error::last_os_error()); }
        Ok(info)
    };
    let itself = std::fs::OpenOptions::new().access_mode(0x80).share_mode(1 | 2 | 4).custom_flags(FLAGS).open(&path)?;
    let mut tag = [0u32; 2];
    if unsafe { GetFileInformationByHandleEx(itself.as_raw_handle().cast(), 9, tag.as_mut_ptr().cast(), 8) } == 0 { return Err(io::Error::last_os_error()); }
    let (attrs, tag) = (tag[0], tag[1]);
    if attrs & crate::attr::DIRECTORY != 0 || attrs & (0x1000 | 0x0004_0000 | 0x0040_0000) != 0
        || attrs & crate::attr::REPARSE_POINT != 0 && tag != 0x8000_0017 {
        return Err(io::Error::other("Not a plain local file"));
    }
    let a = id(&itself)?;
    if a.nNumberOfLinks != 1 { return Err(io::Error::other("The file has other names")); }
    // through the compression filter (no reparse flag): only now does WOF report its size
    let through = std::fs::OpenOptions::new().access_mode(0x80).share_mode(1 | 2 | 4).custom_flags(0x0010_0000).open(&path)?;
    let b = id(&through)?;
    if (a.dwVolumeSerialNumber, a.nFileIndexHigh, a.nFileIndexLow) != (b.dwVolumeSerialNumber, b.nFileIndexHigh, b.nFileIndexLow) {
        return Err(io::Error::other("The file changed while it was measured"));
    }
    let mut compressed = [0u64; 3]; // FILE_COMPRESSION_INFO: CompressedFileSize first
    if unsafe { GetFileInformationByHandleEx(through.as_raw_handle().cast(), 8, compressed.as_mut_ptr().cast(), 24) } != 0 {
        return Ok(compressed[0]);
    }
    // a volume without compression: the file's size, as GetCompressedFileSizeW gives it
    let mut standard = [0u64; 3]; // FILE_STANDARD_INFO: AllocationSize, EndOfFile, ...
    if unsafe { GetFileInformationByHandleEx(through.as_raw_handle().cast(), 1, standard.as_mut_ptr().cast(), 24) } == 0 { return Err(io::Error::last_os_error()); }
    Ok(standard[1])
}

/// Open every ancestor as itself without delete sharing, from the local drive downward.
pub fn directory(path: &Path) -> io::Result<Directory> {
    pin(path, true, false)
}

/// A scan root may itself be a file; its ancestor chain must still be plain local folders.
pub fn scope(path: &Path) -> io::Result<Directory> { pin(path, false, false) }

/// Pin a storage-operation leaf. WOF files are data; other reparse leaves stay refused.
pub fn storage(path: &Path) -> io::Result<Directory> { pin(path, false, true) }

fn pin(path: &Path, require_directory: bool, allow_wof: bool) -> io::Result<Directory> {
    let path = lexical(path)?;
    let mut ancestors: Vec<PathBuf> = path.ancestors().map(Path::to_path_buf).collect();
    ancestors.reverse();
    let mut handles = Vec::new();
    for p in ancestors {
        let h = std::fs::OpenOptions::new().access_mode(0x80).share_mode(1 | 2).custom_flags(FLAGS).open(&p)?;
        let m = h.metadata()?;
        let mut is_wof = false;
        if allow_wof && p == path && !m.is_dir() && m.file_attributes() & crate::attr::REPARSE_POINT != 0 {
            let mut tag = [0u32; 2];
            is_wof = unsafe { GetFileInformationByHandleEx(h.as_raw_handle().cast(), 9, tag.as_mut_ptr().cast(), 8) } != 0 && tag[1] == 0x8000_0017;
        }
        if (!m.is_dir() && (p != path || require_directory)) || m.file_attributes() & crate::attr::REPARSE_POINT != 0 && !is_wof
            || m.file_attributes() & (0x1000 | 0x0004_0000 | 0x0040_0000) != 0 {
            return Err(io::Error::other("A scanned folder or ancestor is a junction or reparse point"));
        }
        let expected = crate::canonical_root(&p);
        if !same(&final_path(&h)?, &expected) { return Err(io::Error::other("The opened folder resolved elsewhere")); }
        handles.push(h);
    }
    Ok(Directory { path: crate::canonical_root(&path), _handles: handles })
}

pub struct LocalFile {
    pub file: File,
    _parent: Directory,
}

/// A bounded plain local file, kept stable while it is read. No cloud recall or links.
pub fn file(path: &Path, max: u64) -> io::Result<LocalFile> {
    let path = lexical(path)?;
    let parent = directory(path.parent().ok_or_else(|| io::Error::other("No parent folder"))?)?;
    let file = std::fs::OpenOptions::new().read(true).share_mode(1).custom_flags(FLAGS).open(&path)?;
    let m = file.metadata()?;
    const CLOUD: u32 = 0x1000 | 0x0004_0000 | 0x0040_0000;
    if !m.is_file() || m.len() > max || m.file_attributes() & (crate::attr::REPARSE_POINT | CLOUD) != 0 {
        return Err(io::Error::other("Not a bounded plain local file"));
    }
    if !same(&final_path(&file)?, &crate::canonical_root(&path)) { return Err(io::Error::other("The opened file resolved elsewhere")); }
    Ok(LocalFile { file, _parent: parent })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_file_is_measured_on_itself_compressed_or_not() {
        let dir = std::env::temp_dir().join(format!("sprawl-on-disk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("plain.bin");
        std::fs::write(&plain, vec![7u8; 10_000]).unwrap();
        assert_eq!(file_on_disk(&plain).unwrap(), 10_000);
        // a file with a second name is refused (its space isn't this name's alone)
        std::fs::hard_link(&plain, dir.join("again.bin")).unwrap();
        assert!(file_on_disk(&plain).is_err());
        // a WOF-compressed file counts, at its compressed size (Astra's case)
        let packed = dir.join("packed.bin");
        std::fs::write(&packed, vec![0u8; 256 * 1024]).unwrap();
        let sys = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32").join("compact.exe");
        let ran = std::process::Command::new(sys).args(["/c", "/exe:xpress4k"]).arg(&packed).output().is_ok_and(|o| o.status.success());
        if ran {
            let size = file_on_disk(&packed).unwrap();
            assert!(size < 256 * 1024, "{size}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_device_and_stream_paths_are_rejected_before_open() {
        for p in [r"\\server\share\file", r"\\?\UNC\server\share\file", r"\\.\C:\file", r"C:\file:stream"] { assert!(lexical(Path::new(p)).is_err(), "{p}"); }
        // Shares may be scanned (listed only); device, stream and half-named paths may not.
        for p in [r"\\server\share", r"\\server\share\dir", r"\\?\UNC\server\share\dir", "//server/share/dir"] { assert!(remote_scan_root(Path::new(p)).is_some(), "{p}"); }
        for p in [r"\\.\C:\file", r"\\?\C:\file", r"\\server", r"\\server\\", r"\\server\share\f:s", r"\\server\share\..\x", r"\\?\GLOBALROOT\x", r"C:\Windows"] { assert!(remote_scan_root(Path::new(p)).is_none(), "{p}"); }
        // the verbatim form gets the same server/share check as the plain one (Astra 1.0.2 S1 #5)
        for p in [r"\\?\UNC\.\C:\x", r"\\?\UNC\?\C:\x", r"\\?\UNC\..\share", r"\\?\UNC\server.\share", r"\\?\UNC\server\share.\x", r"\\?\UNC\server\share \x", r"\\?\UNC\server\\x", r"\\?\UNC\server\.\x", r"\\server\share\.\x"] {
            assert!(remote_scan_root(Path::new(p)).is_none(), "{p}");
        }
        assert!(remote_spelling(r"\\.\C:\x").is_none() && remote_spelling(r"\\?\C:\x").is_none());
        let root = std::env::temp_dir().join(format!("sprawl-local-pin-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("manifest"), b"bounded").unwrap();
        let pinned = file(&root.join("manifest"), 100).unwrap();
        assert!(std::fs::rename(&root, root.with_extension("moved")).is_err(), "the ancestor is pinned");
        assert!(std::fs::write(root.join("manifest"), b"changed").is_err(), "the read file cannot change");
        drop(pinned);
        assert!(file(&root.join("manifest"), 1).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
