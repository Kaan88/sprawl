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

/// A share's server name: no device-like (`.`, `?`), empty or trailing dot/space name,
/// nothing a path parser reads as more than one name.
fn server_component(c: &str) -> bool {
    !c.starts_with(['.', '?']) && share_component(c)
}

/// A share name: as a server name, but it may begin with a dot (`\\nas\.photos`) as long as it
/// isn't only dots (a trailing dot is refused, so `.` and `..` are too).
fn share_component(c: &str) -> bool {
    !c.is_empty() && !c.ends_with(['.', ' '])
        && !c.contains([':', '\0', '\r', '\n', '*', '?', '"', '<', '>', '|', '/'])
}

/// [`remote_scan_root`]'s check of one spelling: `\\server\share\...`, `\\?\UNC\server\share\...`
/// (one validator for both), or `Y:\...` on a mapped remote drive.
fn remote_spelling(raw: &str) -> Option<()> {
    let raw = raw.replace('/', "\\");
    let share = raw.strip_prefix(r"\\?\UNC\").or_else(|| raw.strip_prefix(r"\\").filter(|r| !r.starts_with(['?', '.'])));
    let tail = if let Some(rest) = share {
        let mut parts = rest.split('\\');
        if !parts.next().is_some_and(server_component) || !parts.next().is_some_and(share_component) { return None; }
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

impl ScanRoot {
    /// On the network, as told when it was opened: a share is listed as it is (yes); a pinned
    /// root is asked through its own handle (no, yes, or None when that can't be told).
    pub fn on_network(&self) -> Option<bool> {
        match &self.pinned {
            None => Some(true),
            Some(d) => d.handle().ok().and_then(on_network),
        }
    }
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

/// A pinned path: every folder from the drive down, each opened as itself and checked to be
/// where its path says. The last one (the leaf) is held so that no one, Sprawl included, can
/// rename, replace or delete it while this lives: open it again by name (a tool given its
/// path, a file opened inside it) and it is the same one. To rename or delete a name instead,
/// hold its parent and open the name through it ([`Directory::open_in`]) with delete access.
#[derive(Debug)]
pub struct Directory {
    pub path: PathBuf,
    _handles: Vec<File>,
}

// ---- names opened through their folder's handle (NtCreateFile relative to it): no path is
// resolved, so no folder above can be swapped for a link meanwhile, and a link at the name is
// opened as itself, never followed.

#[repr(C)]
struct UnicodeString { len: u16, max: u16, buf: *const u16 }
#[repr(C)]
struct ObjectAttributes { len: u32, root: *mut core::ffi::c_void, name: *const UnicodeString, attrs: u32, sd: *const core::ffi::c_void, qos: *const core::ffi::c_void }
#[repr(C)]
struct IoStatus { status: usize, info: usize }

#[link(name = "ntdll")]
extern "system" {
    fn NtCreateFile(handle: *mut *mut core::ffi::c_void, access: u32, oa: *const ObjectAttributes, io: *mut IoStatus, alloc: *const i64, attrs: u32, share: u32, disposition: u32, options: u32, ea: *const core::ffi::c_void, ea_len: u32) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
    fn NtSetInformationFile(handle: *mut core::ffi::c_void, io: *mut IoStatus, info: *const core::ffi::c_void, len: u32, class: u32) -> i32;
    fn NtQueryVolumeInformationFile(handle: *mut core::ffi::c_void, io: *mut IoStatus, info: *mut core::ffi::c_void, len: u32, class: u32) -> i32;
}

/// Whether the volume `f` was opened on is reached through the network, asked of that handle
/// (FileFsDeviceInformation): the device is a network file system or a remote device. None
/// when Windows doesn't say.
pub fn on_network(f: &File) -> Option<bool> {
    const FILE_FS_DEVICE_INFORMATION: u32 = 4;
    const FILE_REMOTE_DEVICE: u32 = 0x10;
    // DFS, network, network file system
    const NETWORK_TYPES: [u32; 3] = [0x06, 0x12, 0x14];
    let mut info = [0u32; 2]; // DeviceType, Characteristics
    let mut io = IoStatus { status: 0, info: 0 };
    let r = unsafe { NtQueryVolumeInformationFile(f.as_raw_handle().cast(), &mut io, info.as_mut_ptr().cast(), 8, FILE_FS_DEVICE_INFORMATION) };
    (r >= 0).then(|| info[1] & FILE_REMOTE_DEVICE != 0 || NETWORK_TYPES.contains(&info[0]))
}

#[link(name = "kernel32")]
extern "system" {
    fn SetFileInformationByHandle(file: *mut core::ffi::c_void, class: i32, info: *const core::ffi::c_void, size: u32) -> i32;
}

/// How [`Directory::open_in`] treats the name: open what is there, make it (failing when
/// something is there), or either.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Disposition { Open = 1, Create = 2, OpenOrCreate = 3 }

/// What [`Directory::open_in`] may find at the name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind { Folder, File, Either }

pub const DELETE: u32 = 0x0001_0000;
pub const READ_DATA: u32 = 0x1;
pub const WRITE_DATA: u32 = 0x2;
pub const APPEND_DATA: u32 = 0x4;
pub const READ_ATTRIBUTES: u32 = 0x80;
pub const SHARE_READ: u32 = 1;
pub const SHARE_WRITE: u32 = 2;
pub const SHARE_DELETE: u32 = 4;

/// One plain name: no separator, stream, wildcard or device character, not `.` or `..`, and
/// not ending in a dot or space (which Windows paths would drop).
fn plain_name(name: &str) -> io::Result<Vec<u16>> {
    if name.is_empty() || name.len() > 255 || matches!(name, "." | "..") || name.ends_with(['.', ' '])
        || name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) || name.chars().any(|c| (c as u32) < 32) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Not one plain name"));
    }
    Ok(name.encode_utf16().collect())
}

/// Open `name` in the folder `dir` is open on, through that handle (synchronous, as with
/// backup semantics, the link itself, no cloud recall). (handle, whether it was made now).
pub(crate) fn open_rel(dir: &File, name: &str, access: u32, share: u32, how: Disposition, kind: Kind) -> io::Result<(File, bool)> {
    use std::os::windows::io::FromRawHandle;
    let wide = plain_name(name)?;
    let us = UnicodeString { len: (wide.len() * 2) as u16, max: (wide.len() * 2) as u16, buf: wide.as_ptr() };
    // OBJ_CASE_INSENSITIVE, as CreateFileW opens names
    let oa = ObjectAttributes { len: std::mem::size_of::<ObjectAttributes>() as u32, root: dir.as_raw_handle().cast(), name: &us, attrs: 0x40, sd: std::ptr::null(), qos: std::ptr::null() };
    let mut status_block = IoStatus { status: 0, info: 0 };
    let mut h: *mut core::ffi::c_void = std::ptr::null_mut();
    // FILE_SYNCHRONOUS_IO_NONALERT, FILE_OPEN_FOR_BACKUP_INTENT, FILE_OPEN_REPARSE_POINT,
    // FILE_OPEN_NO_RECALL; FILE_DIRECTORY_FILE or FILE_NON_DIRECTORY_FILE
    let options = 0x20 | 0x4000 | 0x0020_0000 | match kind { Kind::Folder => 0x1, Kind::File => 0x40 | 0x0040_0000, Kind::Either => 0 };
    // SAFETY: every pointer is valid for the call; the handle is owned below.
    let status = unsafe { NtCreateFile(&mut h, access | 0x0010_0000, &oa, &mut status_block, std::ptr::null(), 0, share, how as u32, options, std::ptr::null(), 0) };
    if status < 0 {
        // SAFETY: plain conversion.
        return Err(io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32));
    }
    // SAFETY: a fresh handle from NtCreateFile.
    Ok((unsafe { File::from_raw_handle(h.cast()) }, status_block.info == 2))
}

/// Attributes of an open handle (FILE_ATTRIBUTE_TAG_INFO).
fn attributes(f: &File) -> io::Result<u32> {
    let mut tag = [0u32; 2];
    // SAFETY: the buffer is the size given; class 9 is FileAttributeTagInfo.
    if unsafe { GetFileInformationByHandleEx(f.as_raw_handle().cast(), 9, tag.as_mut_ptr().cast(), 8) } == 0 { return Err(io::Error::last_os_error()); }
    Ok(tag[0])
}

/// A plain local folder or file: no link or other reparse point, no cloud or offline item.
pub(crate) fn check_plain(f: &File, kind: Kind) -> io::Result<u32> {
    let a = attributes(f)?;
    let dir = a & crate::attr::DIRECTORY != 0;
    if a & crate::attr::REPARSE_POINT != 0 || a & (0x1000 | 0x0004_0000 | 0x0040_0000) != 0
        || kind == Kind::Folder && !dir || kind == Kind::File && dir {
        return Err(io::Error::other("Not a plain local folder or file"));
    }
    Ok(a)
}

/// The volume serial of an open handle as FILE_ID_INFO gives it (all 64 bits; the serial in
/// [`identity`] is only the low 32), or None where Windows doesn't give it (FAT, for one).
pub fn volume_id(f: &File) -> Option<u64> {
    let mut full = [0u64; 3];
    // SAFETY: the buffer is the size given; class 18 is FileIdInfo.
    (unsafe { GetFileInformationByHandleEx(f.as_raw_handle().cast(), 18, full.as_mut_ptr().cast(), 24) } != 0).then_some(full[0])
}

/// Volume and file index of an open handle: which file it is.
pub fn identity(f: &File) -> io::Result<(u32, u64)> {
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a valid out-pointer.
    if unsafe { GetFileInformationByHandle(f.as_raw_handle().cast(), &mut info) } == 0 { return Err(io::Error::last_os_error()); }
    Ok((info.dwVolumeSerialNumber, (info.nFileIndexHigh as u64) << 32 | info.nFileIndexLow as u64))
}

/// Mark an opened item (opened with DELETE) to go, through its handle: its name goes at once
/// where the volume allows (POSIX semantics), a read-only file too without its attributes
/// being changed (other names of the same file keep theirs). Falls back to the plain mark.
pub fn dispose(f: &File) -> io::Result<()> {
    // FILE_DISPOSITION_DELETE | POSIX_SEMANTICS | IGNORE_READONLY_ATTRIBUTE
    let flags = 0x1u32 | 0x2 | 0x10;
    // SAFETY: the buffer is the size given; class 21 is FileDispositionInfoEx.
    if unsafe { SetFileInformationByHandle(f.as_raw_handle().cast(), 21, (&flags as *const u32).cast(), 4) } != 0 { return Ok(()); }
    let ex = io::Error::last_os_error();
    if !matches!(ex.raw_os_error(), Some(1 | 50 | 87)) { return Err(ex); }
    let delete = 1u8;
    // SAFETY: class 4 is FileDispositionInfo, one BOOLEAN.
    if unsafe { SetFileInformationByHandle(f.as_raw_handle().cast(), 4, (&delete as *const u8).cast(), 1) } != 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

impl Directory {
    pub(crate) fn handle(&self) -> io::Result<&File> {
        self._handles.last().ok_or_else(|| io::Error::other("No pinned folder"))
    }

    /// The plain folder `name` in this one, opened through this folder's handle as itself (a
    /// link there is refused, never followed) and held like this one. `create`: made when
    /// missing. NotFound when it isn't there.
    pub fn child(mut self, name: &str, create: bool) -> io::Result<Directory> {
        let how = if create { Disposition::OpenOrCreate } else { Disposition::Open };
        let (h, _) = open_rel(self.handle()?, name, READ_DATA | READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE, how, Kind::Folder)?;
        check_plain(&h, Kind::Folder)?;
        self.path = self.path.join(name);
        self._handles.push(h);
        Ok(self)
    }

    /// `name` in this folder, opened through its handle as itself with `access` and `share`
    /// (a link at the name is refused, never followed; so are cloud and offline items). A file
    /// must have one name only when `single` (it is written or removed).
    pub fn open_in(&self, name: &str, access: u32, share: u32, how: Disposition, kind: Kind, single: bool) -> io::Result<File> {
        let (f, _) = open_rel(self.handle()?, name, access | READ_ATTRIBUTES, share, how, kind)?;
        let a = check_plain(&f, kind)?;
        if single && a & crate::attr::DIRECTORY == 0 && self.links_of(&f)? != 1 { return Err(io::Error::other("The file has other names")); }
        Ok(f)
    }

    fn links_of(&self, f: &File) -> io::Result<u32> {
        links_of_handle(f)
    }

    /// Rename `file` (opened with DELETE, in this folder) to `name` in this folder, the new
    /// name given relative to this folder's handle. `replace`: over what has that name.
    pub fn rename_in(&self, file: &File, name: &str, replace: bool) -> io::Result<()> {
        let wide = plain_name(name)?;
        // FILE_RENAME_INFO: ReplaceIfExists, RootDirectory, FileNameLength, FileName
        let head = 8 + std::mem::size_of::<usize>() + 4;
        let size = head + wide.len() * 2 + 2;
        let mut buf = vec![0u64; size.div_ceil(8)];
        // SAFETY: `buf` holds at least `size` bytes.
        let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * 8) };
        bytes[0] = u8::from(replace);
        bytes[8..8 + std::mem::size_of::<usize>()].copy_from_slice(&(self.handle()?.as_raw_handle() as usize).to_le_bytes());
        bytes[8 + std::mem::size_of::<usize>()..head].copy_from_slice(&((wide.len() * 2) as u32).to_le_bytes());
        for (i, c) in wide.iter().enumerate() { bytes[head + i * 2..head + i * 2 + 2].copy_from_slice(&c.to_le_bytes()); }
        let mut status_block = IoStatus { status: 0, info: 0 };
        // SAFETY: class 10 is FileRenameInformation; the buffer is the size given.
        let status = unsafe { NtSetInformationFile(file.as_raw_handle().cast(), &mut status_block, buf.as_ptr().cast(), size as u32, 10) };
        // SAFETY: plain conversion.
        if status >= 0 { Ok(()) } else { Err(io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)) }
    }

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

    /// The names directly in this folder, as the filesystem lists them (each one is opened
    /// through this folder's handle before it is used, so the list is only a list).
    pub fn names(&self) -> io::Result<Vec<String>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(&self.path)? {
            let e = e?;
            if let Ok(n) = e.file_name().into_string() {
                out.push(n);
            }
        }
        Ok(out)
    }

    /// Remove the plain folder `name` in this folder, and the plain files directly in it, each
    /// through a handle and never by path: a link at the name or inside is refused, a file with
    /// another name (a hard link) is left, and so is any folder inside. The folder goes when it
    /// is empty; anything left in it means the folder stays too.
    pub fn remove_tree(&self, name: &str) -> io::Result<()> {
        let (h, _) = open_rel(self.handle()?, name, DELETE | READ_DATA | READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE, Disposition::Open, Kind::Folder)?;
        check_plain(&h, Kind::Folder)?;
        let path = self.path.join(name);
        for e in std::fs::read_dir(&path)? {
            let e = e?;
            let n = e.file_name().into_string().map_err(|_| io::Error::other("Not a plain name"))?;
            let (f, _) = open_rel(&h, &n, DELETE | READ_DATA | READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE, Disposition::Open, Kind::File)?;
            check_plain(&f, Kind::File)?;
            if links_of_handle(&f)? != 1 { return Err(io::Error::other("A file in the folder has other names")); }
            dispose(&f)?;
        }
        dispose(&h)
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

/// The hard-link count of an open handle.
fn links_of_handle(f: &File) -> io::Result<u32> {
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a valid out-pointer.
    if unsafe { GetFileInformationByHandle(f.as_raw_handle().cast(), &mut info) } == 0 { return Err(io::Error::last_os_error()); }
    Ok(info.nNumberOfLinks)
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
        // Each folder above holds the next open, so it can't be renamed. The leaf itself is
        // held with an access that takes part in sharing (listing a folder, reading a file),
        // and without delete sharing: no one can rename, replace or delete it while it is held
        // (an attribute-only handle would let it be renamed away under its name).
        let access = if p == path { READ_DATA | READ_ATTRIBUTES } else { READ_ATTRIBUTES };
        let h = std::fs::OpenOptions::new().access_mode(access).share_mode(1 | 2).custom_flags(FLAGS).open(&p)?;
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

/// A folder made by this process alone and held as its own until dropped: test fixtures and
/// Codex's run folders. It is made through its held parent's handle by the very call that
/// returns its handle (an existing folder of that name is never taken), so the handle is the
/// folder just made; that handle shares no delete, so while it lives no one can rename,
/// replace or delete the folder. Drop removes what is inside through that handle, each item
/// opened through its folder's handle as itself (a link is removed as a link) and removed
/// through its own handle, a read-only file without its attributes being changed (another name
/// of the same file keeps them). The folder itself is then opened again through the held
/// parent and removed only if it is the same folder (by its file index). Anything that can't
/// be removed so stays, and then so does the folder.
#[derive(Debug)]
pub struct OwnedDir {
    path: PathBuf,
    name: String,
    /// Its volume and full 128-bit file id (FILE_ID_INFO): unique on ReFS too, where the
    /// short id isn't, and on NTFS not reused for another folder (Astra 1.0.2 round 4, S5).
    id: (u64, u128),
    held: Option<File>,
    /// An attribute-only handle on the same folder, open from its making to its removal: it
    /// takes no part in sharing (so it doesn't stop the removal's own open), and while it is
    /// open the folder's id can't be given to another one. The folder opened again to be
    /// removed is compared with it, two open handles (Astra 1.0.3 review, S5).
    anchor: Option<File>,
    parent: Directory,
    /// Run between letting go of the hold and opening the folder to remove it.
    #[cfg(test)]
    before_reopen: Option<Hook>,
}

#[cfg(test)]
struct Hook(Box<dyn FnOnce()>);

#[cfg(test)]
impl std::fmt::Debug for Hook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("Hook") }
}

/// The volume's serial and the full 128-bit id of an open file or folder (FILE_ID_INFO).
pub fn full_identity(f: &File) -> io::Result<(u64, u128)> {
    let mut b = [0u64; 3];
    // SAFETY: 24 bytes; class 18 is FileIdInfo.
    if unsafe { GetFileInformationByHandleEx(f.as_raw_handle().cast(), 18, b.as_mut_ptr().cast(), 24) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((b[0], (b[2] as u128) << 64 | b[1] as u128))
}

impl OwnedDir {
    /// A new folder in `parent`, named `<prefix>-<pid>-<nanos>-<try>`.
    pub fn create_in(parent: &Path, prefix: &str) -> io::Result<OwnedDir> {
        Self::create_under(directory(parent)?, prefix)
    }

    /// [`OwnedDir::create_in`] a folder already held.
    pub fn create_under(parent: Directory, prefix: &str) -> io::Result<OwnedDir> {
        for n in 0..100u32 {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
            let name = format!("{prefix}-{}-{nanos}-{n}", std::process::id());
            // listing access, so the sharing rules apply to this handle (attribute-only handles
            // take no part in them); no delete access of its own, so a program may still use it
            // as its current folder
            match open_rel(parent.handle()?, &name, READ_DATA | READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE, Disposition::Create, Kind::Folder) {
                Ok((held, _)) => {
                    let id = full_identity(&held)?;
                    // the hold shares no delete, so the name is still this folder
                    let anchor = open_rel(parent.handle()?, &name, READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE | SHARE_DELETE, Disposition::Open, Kind::Folder)
                        .ok().filter(|(a, _)| full_identity(a).ok() == Some(id)).map(|(a, _)| a);
                    return Ok(OwnedDir {
                        path: parent.path.join(&name),
                        name,
                        id,
                        held: Some(held),
                        anchor,
                        parent,
                        #[cfg(test)]
                        before_reopen: None,
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::other("No free folder name"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
type ChildHook = std::cell::RefCell<Option<Box<dyn FnMut(&str)>>>;

#[cfg(test)]
thread_local! {
    /// Run with each name teardown has just opened (a test tries to move it out then).
    static AFTER_CHILD_OPEN: ChildHook = const { std::cell::RefCell::new(None) };
}

/// Remove everything in the folder `dir` is open on, through handles only (see [`OwnedDir`]).
/// False when anything stayed.
fn empty_through(dir: &File, depth: u32) -> bool {
    if depth > 64 { return false; }
    let mut names: Vec<Vec<u16>> = Vec::new();
    let mut buf = Vec::new();
    let listed = crate::walk::dir::list(dir, &mut buf, &mut |name, _, _, _, _, _| {
        if name != [b'.' as u16] && name != [b'.' as u16, b'.' as u16] { names.push(name.to_vec()); }
        true
    });
    if !listed { return false; }
    let mut all = true;
    for wide in names {
        let Ok(name) = String::from_utf16(&wide) else { all = false; continue };
        // no delete sharing: nothing can rename or move it out of the owned folder while it is
        // gone through and removed (Astra 1.0.2 round 4, S6)
        let Ok((h, _)) = open_rel(dir, &name, DELETE | READ_DATA | READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE, Disposition::Open, Kind::Either) else { all = false; continue };
        #[cfg(test)]
        AFTER_CHILD_OPEN.with(|f| if let Some(f) = f.borrow_mut().as_mut() { f(&name) });
        let Ok(a) = attributes(&h) else { all = false; continue };
        if a & crate::attr::DIRECTORY != 0 && a & crate::attr::REPARSE_POINT == 0 && !empty_through(&h, depth + 1) {
            all = false;
            continue;
        }
        all &= dispose(&h).is_ok();
    }
    all
}

impl Drop for OwnedDir {
    fn drop(&mut self) {
        let Some(held) = self.held.take() else { return };
        // without the anchor, the folder can't be known across the reopen: it is left (empty)
        let anchor = self.anchor.take();
        let emptied = empty_through(&held, 0);
        // the hold shares no delete, so it goes before the folder can be opened to be removed
        drop(held);
        let Some(anchor) = anchor.filter(|_| emptied) else { return };
        #[cfg(test)]
        if let Some(f) = self.before_reopen.take() { (f.0)(); }
        let Ok(parent) = self.parent.handle() else { return };
        // no delete sharing: from this open to the removal it can't be swapped; it goes only
        // when it is the very folder made: the same full id on the same volume as the anchor,
        // open all along, else it is left
        let Ok((h, _)) = open_rel(parent, &self.name, DELETE | READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE, Disposition::Open, Kind::Folder) else { return };
        let same = full_identity(&anchor).ok() == Some(self.id) && full_identity(&h).ok() == Some(self.id);
        if check_plain(&h, Kind::Folder).is_ok() && same {
            let _ = dispose(&h);
        }
        drop(h);
        drop(anchor);
    }
}

/// The bytes of `trusted\below...\name`, read through a checked chain: `trusted` pinned from
/// the drive down, then each folder below it and the file itself opened through its parent's
/// handle as itself (a link anywhere is an error, never followed) and held while it is read.
/// None only when a name on the way, or the file, is genuinely not there. At most `max` bytes.
pub fn read_checked(trusted: &Path, below: &[&str], name: &str, max: u64) -> io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let mut held = directory(trusted)?;
    for part in below {
        held = match held.child(part, false) {
            Ok(h) => h,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
    }
    let mut file = match held.open_in(name, READ_DATA, SHARE_READ, Disposition::Open, Kind::File, false) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if file.metadata()?.len() > max { return Err(io::Error::other("The file is bigger than expected")); }
    let mut bytes = Vec::new();
    (&mut file).take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max { return Err(io::Error::other("The file is bigger than expected")); }
    Ok(Some(bytes))
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
mod owned_tests {
    use super::OwnedDir;

    #[test]
    fn an_owned_folder_is_held_and_removed_with_what_is_in_it() {
        let a = OwnedDir::create_in(&std::env::temp_dir(), "sprawl-owned").unwrap();
        let b = OwnedDir::create_in(&std::env::temp_dir(), "sprawl-owned").unwrap();
        assert_ne!(a.path(), b.path());
        std::fs::create_dir(a.path().join("inner")).unwrap();
        std::fs::write(a.path().join("inner").join("f.txt"), b"x").unwrap();
        std::fs::write(a.path().join("ro.txt"), b"x").unwrap();
        let mut perm = std::fs::metadata(a.path().join("ro.txt")).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(a.path().join("ro.txt"), perm).unwrap();
        // held: it can't be renamed away and replaced
        let moved = a.path().with_extension("moved");
        assert!(std::fs::rename(a.path(), &moved).is_err());
        // a path below it can still be pinned
        assert!(super::directory(&a.path().join("inner")).is_ok());
        let (pa, pb) = (a.path().to_path_buf(), b.path().to_path_buf());
        drop(a);
        drop(b);
        assert!(!pa.exists() && !pb.exists());
    }

    #[test]
    fn removing_a_read_only_file_leaves_its_other_names_as_they_are() {
        // Astra 1.0.2 round 3, S6: the fixture's teardown changed attributes by path
        let keep = OwnedDir::create_in(&std::env::temp_dir(), "sprawl-owned-keep").unwrap();
        let other = keep.path().join("other.txt");
        std::fs::write(&other, b"x").unwrap();
        let mut perm = std::fs::metadata(&other).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&other, perm).unwrap();
        let a = OwnedDir::create_in(&std::env::temp_dir(), "sprawl-owned-links").unwrap();
        std::fs::hard_link(&other, a.path().join("again.txt")).unwrap();
        let pa = a.path().to_path_buf();
        drop(a);
        assert!(!pa.exists(), "the read-only name went, through its handle");
        assert!(std::fs::metadata(&other).unwrap().permissions().readonly(), "the other name is still read-only");
        let mut perm = std::fs::metadata(&other).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(&other, perm).unwrap();
    }

    #[test]
    fn a_folder_put_in_place_of_an_owned_one_stays() {
        // the folder is removed only if it is the one made, not an empty one at its name
        let parent = OwnedDir::create_in(&std::env::temp_dir(), "sprawl-owned-parent").unwrap();
        let a = OwnedDir::create_in(parent.path(), "inner").unwrap();
        let pa = a.path().to_path_buf();
        // Drop lets go of the hold before it opens the folder to remove it: swapped right there
        // (Astra 1.0.2 round 4, S6: the test took the hold first, so Drop returned at once), the
        // folder now at the name is not the one made, and stays
        let mut a = a;
        let (from, to) = (pa.clone(), parent.path().join("moved"));
        a.before_reopen = Some(super::Hook(Box::new(move || {
            std::fs::rename(&from, &to).unwrap();
            std::fs::create_dir(&from).unwrap();
        })));
        drop(a);
        assert!(pa.exists(), "another folder at its name is not removed");
        assert!(parent.path().join("moved").exists(), "the one made, moved away, isn't followed");
        std::fs::remove_dir(&pa).unwrap();
        std::fs::remove_dir(parent.path().join("moved")).unwrap();
        // not swapped: removed at the same step
        let b = OwnedDir::create_in(parent.path(), "inner").unwrap();
        let pb = b.path().to_path_buf();
        drop(b);
        assert!(!pb.exists());
        // round 4, S6: a folder in it, once teardown has opened it, can't be moved out while it
        // is gone through (the move is tried right after that open)
        let c = OwnedDir::create_in(parent.path(), "inner").unwrap();
        std::fs::create_dir_all(c.path().join("sub").join("deep")).unwrap();
        std::fs::write(c.path().join("sub").join("deep").join("f.txt"), b"x").unwrap();
        let (from, to) = (c.path().join("sub"), parent.path().join("escaped"));
        let tried = std::rc::Rc::new(std::cell::Cell::new(None));
        let t2 = tried.clone();
        super::AFTER_CHILD_OPEN.with(|h| *h.borrow_mut() = Some(Box::new(move |name: &str| {
            if name == "sub" { t2.set(Some(std::fs::rename(&from, &to).is_ok())); }
        })));
        let pc = c.path().to_path_buf();
        drop(c);
        super::AFTER_CHILD_OPEN.with(|h| *h.borrow_mut() = None);
        assert_eq!(tried.get(), Some(false), "the open child can't be moved out");
        assert!(!parent.path().join("escaped").exists() && !pc.exists(), "all of it removed");
    }

    #[test]
    fn a_pinned_leaf_cant_be_renamed_away() {
        // Astra 1.0.2 round 3: an attribute-only hold let the leaf be renamed under its name
        let owned = OwnedDir::create_in(&std::env::temp_dir(), "sprawl-pin-leaf").unwrap();
        let f = owned.path().join("data.bin");
        std::fs::write(&f, b"x").unwrap();
        std::fs::create_dir(owned.path().join("sub")).unwrap();
        let held = super::storage(&f).unwrap();
        assert!(std::fs::rename(&f, owned.path().join("moved.bin")).is_err(), "a held file keeps its name");
        assert!(std::fs::read(&f).is_ok(), "and can still be read by name");
        drop(held);
        let held = super::directory(&owned.path().join("sub")).unwrap();
        assert!(std::fs::rename(owned.path().join("sub"), owned.path().join("gone")).is_err(), "a held folder keeps its name");
        std::fs::write(owned.path().join("sub").join("in.txt"), b"y").unwrap();
        drop(held);
        // through a held folder: names opened as themselves, renamed relative to it
        let dir = super::directory(owned.path()).unwrap();
        let file = dir.open_in("data.bin", super::DELETE, super::SHARE_READ, super::Disposition::Open, super::Kind::File, true).unwrap();
        dir.rename_in(&file, "renamed.bin", false).unwrap();
        assert!(owned.path().join("renamed.bin").exists() && !f.exists());
        super::dispose(&file).unwrap();
        drop(file);
        assert!(!owned.path().join("renamed.bin").exists());
        assert!(dir.open_in("nothing", super::READ_DATA, super::SHARE_READ, super::Disposition::Open, super::Kind::File, false).unwrap_err().kind() == std::io::ErrorKind::NotFound);
        assert!(dir.open_in(r"sub\in.txt", super::READ_DATA, super::SHARE_READ, super::Disposition::Open, super::Kind::File, false).is_err(), "one name only");
        let sub = super::directory(owned.path()).unwrap().child("sub", false).unwrap();
        assert!(sub.open_in("in.txt", super::READ_DATA, super::SHARE_READ, super::Disposition::Open, super::Kind::File, false).is_ok());
        let made = super::directory(owned.path()).unwrap().child("new", true).unwrap();
        assert!(made.path.ends_with("new") && owned.path().join("new").is_dir());
        drop((dir, sub, made));
        drop(owned);
    }
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
        // a share name may begin with a dot, not be only dots (re-review S1 #5)
        assert!(remote_spelling(r"\\nas\.photos\2024").is_some() && remote_spelling(r"\\?\UNC\nas\.photos").is_some());
        for bad in [r"\\nas\.", r"\\nas\..", r"\\nas\...\x", r"\\.nas\share", r"\\?\UNC\.\C:\x", r"\\?\UNC\..\x", r"\\nas\?x"] {
            assert!(remote_spelling(bad).is_none(), "{bad}");
        }
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
