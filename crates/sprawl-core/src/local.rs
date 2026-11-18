//! Local paths pinned against ancestor replacement. Reject remote/device aliases before I/O.
use std::fs::File;
use std::io;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetFinalPathNameByHandleW};

const FLAGS: u32 = 0x0200_0000 | 0x0020_0000 | 0x0010_0000; // backup, open reparse, no recall

pub fn lexical(path: &Path) -> io::Result<PathBuf> {
    let raw = path.to_str().ok_or_else(|| io::Error::other("The path isn't valid Unicode"))?;
    let raw = if let Some(rest) = raw.strip_prefix(r"\\?\") {
        let b = rest.as_bytes();
        if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
            return Err(io::Error::other("Network and device paths aren't permitted here"));
        }
        rest
    } else { raw };
    if raw.starts_with(['\\', '/']) || raw.contains('\0') { return Err(io::Error::other("Network and device paths aren't permitted here")); }
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

/// Open every ancestor as itself without delete sharing, from the local drive downward.
pub fn directory(path: &Path) -> io::Result<Directory> {
    pin(path, true)
}

/// A scan root may itself be a file; its ancestor chain must still be plain local folders.
pub fn scope(path: &Path) -> io::Result<Directory> { pin(path, false) }

fn pin(path: &Path, require_directory: bool) -> io::Result<Directory> {
    let path = lexical(path)?;
    let mut ancestors: Vec<PathBuf> = path.ancestors().map(Path::to_path_buf).collect();
    ancestors.reverse();
    let mut handles = Vec::new();
    for p in ancestors {
        let h = std::fs::OpenOptions::new().access_mode(0x80).share_mode(1 | 2).custom_flags(FLAGS).open(&p)?;
        let m = h.metadata()?;
        if (!m.is_dir() && (p != path || require_directory)) || m.file_attributes() & crate::attr::REPARSE_POINT != 0 {
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
    fn remote_device_and_stream_paths_are_rejected_before_open() {
        for p in [r"\\server\share\file", r"\\?\UNC\server\share\file", r"\\.\C:\file", r"C:\file:stream"] { assert!(lexical(Path::new(p)).is_err(), "{p}"); }
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
