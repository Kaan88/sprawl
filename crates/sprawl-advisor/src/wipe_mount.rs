//! Private, GUID-rooted mount points for cipher. No drive-letter path crosses this boundary.
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use sprawl_core::local::{self, Directory, Disposition, Kind, DELETE, READ_DATA, READ_ATTRIBUTES, WRITE_DATA, SHARE_READ, SHARE_WRITE, SHARE_DELETE};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, LocalFree},
    Security::{Authorization::*, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, SECURITY_ATTRIBUTES},
    Storage::FileSystem::{CreateDirectoryW, DeleteVolumeMountPointW, GetDriveTypeW, GetVolumeNameForVolumeMountPointW, SetVolumeMountPointW, FILE_FLAG_BACKUP_SEMANTICS},
    System::IO::DeviceIoControl,
};

const PRIVATE: &str = "wipe-mounts-v1";
// Normal (including unelevated same-user) processes cannot plant or rewrite a recovery record.
const ACL: &str = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";
const READ_CONTROL: u32 = 0x0002_0000;
const REPARSE: u32 = 0x400;
const MOUNT_TAG: u32 = 0xa0000003;
const GET_REPARSE: u32 = 0x0009_00a8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Volume(String);

impl Volume {
    pub(super) fn parse(path: &str) -> io::Result<Self> {
        let path = path.to_ascii_lowercase();
        let id = path.strip_prefix(r"\\?\volume{").and_then(|s| s.strip_suffix("}\\"));
        if let Some(id) = id.filter(|s| s.len() == 36) {
            if id.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() }) {
                return Ok(Self(format!("\\\\?\\Volume{{{id}}}\\")));
            }
        }
        Err(io::Error::other("Windows did not provide a volume GUID"))
    }

    pub(super) fn path(&self) -> &Path { Path::new(&self.0) }
    pub(super) fn id(&self) -> &str { &self.0[11..47] }
}

pub(super) fn fixed(path: &Path) -> bool { unsafe { GetDriveTypeW(super::wide(path).as_ptr()) == 3 } }

pub(super) fn volume_at(path: &Path) -> io::Result<Volume> {
    let mut out = [0u16; 64];
    checked(unsafe { GetVolumeNameForVolumeMountPointW(mount_name(path).as_ptr(), out.as_mut_ptr(), out.len() as u32) })?;
    let end = out.iter().position(|b| *b == 0).ok_or_else(|| io::Error::other("unterminated volume name"))?;
    Volume::parse(&String::from_utf16_lossy(&out[..end]))
}

/// Read the real system Windows directory, not the caller's SystemRoot environment.
/// Its open handle and the DOS root must identify the same fixed volume.
#[derive(Clone)]
pub(super) struct SystemVolume {
    pub(super) root: PathBuf,
    pub(super) volume: Volume,
}

pub(super) fn system_volume() -> io::Result<SystemVolume> {
    let windows = crate::clean::windows_directory()
        .ok_or_else(|| io::Error::other("Windows did not report its system directory"))?;
    let root = super::drive_root(&windows).ok_or_else(|| io::Error::other("Windows has no ordinary system drive"))?;
    // The kernel's SystemRoot alias reaches the running installation without resolving
    // its drive letter first. A process-local DOS alias is not proof of system identity.
    let dir = fs::OpenOptions::new().access_mode(READ_ATTRIBUTES).share_mode(SHARE_READ | SHARE_WRITE | SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(r"\\?\GLOBALROOT\SystemRoot\")?;
    let volume = qualified_root(&local::volume_path(&dir)?)?;
    if !fixed(volume.path()) || !fixed(&root) || volume_at(&root)? != volume {
        return Err(io::Error::other("the system drive doesn't name the fixed volume holding Windows"));
    }
    Ok(SystemVolume { root, volume })
}

fn checked(ok: i32) -> io::Result<()> { if ok != 0 { Ok(()) } else { Err(io::Error::last_os_error()) } }

fn mount_name(path: &Path) -> Vec<u16> {
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if !wide.ends_with(&[b'\\' as u16]) { wide.push(b'\\' as u16); }
    wide.push(0);
    wide
}

fn qualified_root(path: &Path) -> io::Result<Volume> {
    let s = path.to_str().ok_or_else(|| io::Error::other("the private path isn't Unicode"))?;
    Volume::parse(s.get(..49).ok_or_else(|| io::Error::other("the private path has no host GUID"))?)
}

struct Host { dir: Directory, path: PathBuf, volume: Volume }

impl Host {
    fn open(site: &HostSite, create: bool, target: Option<&Volume>) -> io::Result<Self> {
        let data_dir = local::mount_parent(&site.parent)?;
        let parent = data_dir.volume_path()?;
        let volume = volume_at(qualified_root(&parent)?.path())?;
        if !fixed(volume.path()) { return Err(io::Error::other("the private mount needs a fixed local host volume")); }
        if !allowed_host(&volume, site.required.as_ref(), target) {
            return Err(io::Error::other("the mount host is on the target or isn't on the expected system volume"));
        }
        let path = parent.join(site.namespace);
        if create {
            let mut sd = null_mut();
            let acl: Vec<u16> = ACL.encode_utf16().chain(Some(0)).collect();
            checked(unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(acl.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut()) })?;
            let attrs = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
            let made = checked(unsafe { CreateDirectoryW(super::wide(&path).as_ptr(), &attrs) });
            unsafe { LocalFree(sd) };
            if let Err(e) = made {
                if e.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) { return Err(e); }
            }
        }
        let acl = data_dir.open_in(site.namespace, READ_CONTROL, SHARE_READ, Disposition::Open, Kind::Folder, false)?;
        let dir = local::mount_parent(&data_dir.path.join(site.namespace))?;
        if dir.volume_path()? != path || local::full_identity(&acl)? != dir.identity()? || !trusted_acl(&acl)? {
            return Err(io::Error::other("the private mount folder isn't an administrator-owned Sprawl folder"));
        }
        Ok(Self { dir, path, volume })
    }
}

fn allowed_host(actual: &Volume, required: Option<&Volume>, target: Option<&Volume>) -> bool {
    target != Some(actual) && required.is_none_or(|volume| volume == actual)
}

struct HostSite {
    parent: PathBuf,
    namespace: &'static str,
    required: Option<Volume>,
}

fn host_sites(data: Option<&Path>, system: Option<&SystemVolume>) -> Vec<HostSite> {
    let mut sites = Vec::new();
    if let Some(parent) = data {
        sites.push(HostSite { parent: parent.to_owned(), namespace: PRIVATE, required: None });
    }
    if let Some(system) = system {
        for parent in [system.root.join("ProgramData"), system.root.clone()] {
            sites.push(HostSite { parent, namespace: "Sprawl-wipe-mounts-v1", required: Some(system.volume.clone()) });
        }
    }
    sites
}

fn other_host<H>(data: &Path, system: Option<&SystemVolume>, mut open: impl FnMut(&HostSite) -> io::Result<H>) -> io::Result<H> {
    let mut errors = Vec::new();
    for site in host_sites(Some(data), system) {
        match open(&site) {
            Ok(host) => return Ok(host),
            Err(e) => errors.push(format!("{}: {e}", site.parent.join(site.namespace).display())),
        }
    }
    Err(io::Error::other(format!("no usable fixed host volume for the private mount ({})", errors.join("; "))))
}

fn trusted_acl(file: &File) -> io::Result<bool> {
    let mut sd = null_mut();
    let fields = DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION;
    let err = unsafe { GetSecurityInfo(file.as_raw_handle(), SE_FILE_OBJECT, fields, null_mut(), null_mut(), null_mut(), null_mut(), &mut sd) };
    if err != 0 { return Err(io::Error::from_raw_os_error(err as i32)); }
    let (mut text, mut size) = (null_mut(), 0);
    let converted = checked(unsafe { ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, SDDL_REVISION_1, fields, &mut text, &mut size) });
    unsafe { LocalFree(sd) };
    converted?;
    let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, size as usize) });
    unsafe { LocalFree(text.cast()) };
    let s = s.trim_end_matches('\0');
    // Windows may mark the protected ACL as auto-inherited.
    Ok(s == ACL || s.strip_prefix("O:BAD:PAI").is_some_and(|rest| ACL.strip_prefix("O:BAD:P") == Some(rest)))
}

#[derive(Debug)]
struct Record { id: (u64, u128), volume: Volume }

impl Record {
    fn read(file: &File) -> io::Result<Self> {
        let mut text = String::new();
        file.take(257).read_to_string(&mut text)?;
        if text.len() > 256 { return Err(io::Error::other("oversized private mount recovery record")); }
        let parse = || {
            let mut lines = text.lines();
            if lines.next()? != "sprawl-wipe-mount-v1" { return None; }
            let serial = u64::from_str_radix(lines.next()?, 16).ok()?;
            let id = u128::from_str_radix(lines.next()?, 16).ok()?;
            let volume = Volume::parse(lines.next()?).ok()?;
            lines.next().is_none().then_some(Self { id: (serial, id), volume })
        };
        parse().ok_or_else(|| io::Error::other("invalid private mount recovery record"))
    }

    fn write(&self, mut file: &File) -> io::Result<()> {
        writeln!(file, "sprawl-wipe-mount-v1\n{:016x}\n{:032x}\n{}", self.id.0, self.id.1, self.volume.0)?;
        file.sync_all()
    }
}

fn lease_name(name: &str) -> String { format!("{name}.lease") }

fn valid_name(name: &str) -> bool {
    name.strip_prefix("mount-").is_some_and(|tail| {
        let parts: Vec<_> = tail.split('-').collect();
        parts.len() == 3 && parts.iter().all(|s| !s.is_empty() && s.len() <= 40 && s.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// Read a mount's stored substitute name, even when the destination is offline. Junctions
/// to ordinary directories, symlinks and unknown reparse tags aren't ours to detach.
fn binding(file: &File) -> io::Result<Option<Volume>> {
    if file.metadata()?.file_attributes() & REPARSE == 0 { return Ok(None); }
    let mut out = [0u8; 16 * 1024];
    let mut n = 0;
    checked(unsafe { DeviceIoControl(file.as_raw_handle(), GET_REPARSE, null(), 0, out.as_mut_ptr().cast(), out.len() as u32, &mut n, null_mut()) })?;
    let buf = out.get(..n as usize).ok_or_else(|| io::Error::other("invalid reparse data size"))?;
    parse_binding(buf).map(Some)
}

fn parse_binding(buf: &[u8]) -> io::Result<Volume> {
    let invalid = || io::Error::other("the folder isn't Sprawl's volume mount");
    if buf.len() < 16 || u32::from_le_bytes(buf[..4].try_into().unwrap()) != MOUNT_TAG { return Err(invalid()); }
    let off = u16::from_le_bytes([buf[8], buf[9]]) as usize;
    let len = u16::from_le_bytes([buf[10], buf[11]]) as usize;
    let name = buf.get(16 + off..16 + off + len).filter(|s| s.len() % 2 == 0).ok_or_else(invalid)?;
    let chars: Vec<_> = name.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    let raw = String::from_utf16(&chars).map_err(|_| invalid())?;
    let raw = raw.strip_prefix(r"\??\").ok_or_else(invalid)?;
    Volume::parse(&format!(r"\\?\{raw}"))
}

pub(super) trait Mount: Send + Sync + 'static {
    fn attach(&mut self) -> io::Result<()>;
    fn verified_volume(&mut self) -> io::Result<Volume>;
    fn path(&self) -> &Path;
    fn cleanup(&mut self) -> io::Result<()>;
}

pub(super) struct NativeMount {
    host: Arc<Host>,
    name: String,
    path: PathBuf,
    owned: Record,
    // The anchor keeps the file ID alive without taking part in sharing. The hold pins the
    // name through Set/DeleteVolumeMountPoint; the read-only freeze blocks retagging at run time.
    anchor: File,
    hold: Option<File>,
    freeze: Option<File>,
    lease: Option<File>,
}

impl NativeMount {
    pub(super) fn prepare(data: &Path, system: Option<&SystemVolume>, volume: &Volume) -> io::Result<Self> {
        let host = Arc::new(other_host(data, system, |site| Host::open(site, true, Some(volume)))?);
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(io::Error::other)?.as_nanos();
        let (name, created) = loop {
            let name = format!("mount-{}-{now}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
            match host.dir.create_empty_directory(&name) {
                Ok(hold) => break (name, hold),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        };
        let (owned, anchor, lease) = prepare_lease(&host.dir, &name, &created, volume, local::full_identity, Record::write)?;
        // The Win32 mount API must not have to share DELETE with our hold. If we cannot reopen,
        // the synced lease now lets startup recover the same plain, empty directory.
        drop(created);
        let hold = host.dir.open_dir_itself(&name, READ_DATA, SHARE_READ | SHARE_WRITE)?;
        let mut mount = Self::from_owned(host, name, hold, anchor, owned)?;
        mount.lease = Some(lease);
        Ok(mount)
    }

    fn from_owned(host: Arc<Host>, name: String, hold: File, anchor: File, owned: Record) -> io::Result<Self> {
        if local::full_identity(&anchor)? != owned.id || local::full_identity(&hold)? != owned.id {
            return Err(io::Error::other("the private mount folder was replaced"));
        }
        let path = host.path.join(&name);
        Ok(Self { host, name, path, owned, anchor, hold: Some(hold), freeze: None, lease: None })
    }

    pub(super) fn pins(&self) -> Vec<&File> {
        self.host.dir.pins().iter().chain(self.freeze.iter()).chain(self.lease.iter()).collect()
    }

    fn remove_owned(&mut self) -> io::Result<()> {
        drop(self.freeze.take());
        let hold = self.hold.as_ref().ok_or_else(|| io::Error::other("no owned mount handle"))?;
        detach_owned(binding(hold)?.as_ref(), &self.owned.volume, || detach_mount(&self.path))?;
        drop(self.hold.take());
        let dir = self.host.dir.open_in(&self.name, DELETE | READ_DATA, SHARE_READ, Disposition::Open, Kind::Folder, false)?;
        if local::full_identity(&dir)? != local::full_identity(&self.anchor)? {
            return Err(io::Error::other("the private mount folder was replaced"));
        }
        local::dispose_empty_directory(&dir)?;
        if let Some(lease) = self.lease.take() { local::dispose(&lease)?; }
        Ok(())
    }
}

fn prepare_lease(dir: &Directory, name: &str, created: &File, volume: &Volume,
    identity: impl FnOnce(&File) -> io::Result<(u64, u128)>,
    persist: impl FnOnce(&Record, &File) -> io::Result<()>,
) -> io::Result<(Record, File, File)> {
    let mut lease = None;
    let result = (|| {
        let owned = Record { id: identity(created)?, volume: volume.clone() };
        let anchor = dir.open_dir_itself(name, READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE | SHARE_DELETE)?;
        if local::full_identity(&anchor)? != owned.id {
            return Err(io::Error::other("the new private mount folder was replaced"));
        }
        lease = Some(dir.open_in(&lease_name(name), READ_DATA | WRITE_DATA | DELETE, SHARE_READ,
            Disposition::Create, Kind::File, true)?);
        persist(&owned, lease.as_ref().unwrap())?;
        Ok((owned, anchor))
    })();
    match result {
        Ok((owned, anchor)) => Ok((owned, anchor, lease.unwrap())),
        Err(e) => {
            // No mount call yet. Only the handle from FILE_CREATE can authorize rollback,
            // even if full_identity failed. A pre-existing lease was never opened.
            let rollback = local::dispose_empty_directory(created)
                .and_then(|()| lease.as_ref().map_or(Ok(()), local::dispose));
            Err(match rollback {
                Ok(()) => e,
                Err(clean) => io::Error::other(format!("{e}; couldn't roll back the new private mount folder: {clean}")),
            })
        }
    }
}

fn detach_mount(path: &Path) -> io::Result<()> {
    let name = mount_name(path);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        match checked(unsafe { DeleteVolumeMountPointW(name.as_ptr()) }) {
            Err(e) if e.raw_os_error() == Some(32) && std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            result => return result,
        }
    }
}

fn detach_owned(mounted: Option<&Volume>, expected: &Volume, detach: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    match mounted {
        Some(volume) if volume == expected => detach(),
        Some(_) => Err(io::Error::other("the private mount no longer has its recorded binding")),
        None => Ok(()), // Already detached or never attached. The empty folder can still be ours.
    }
}

impl Mount for NativeMount {
    fn attach(&mut self) -> io::Result<()> {
        checked(unsafe { SetVolumeMountPointW(mount_name(&self.path).as_ptr(), super::wide(self.owned.volume.path()).as_ptr()) })
    }

    fn verified_volume(&mut self) -> io::Result<Volume> {
        let frozen = self.host.dir.open_dir_itself(&self.name, READ_DATA, SHARE_READ)?;
        if local::full_identity(&frozen)? != self.owned.id || binding(&frozen)?.as_ref() != Some(&self.owned.volume) {
            return Err(io::Error::other("the owned mount does not point at the reviewed GUID"));
        }
        self.freeze = Some(frozen);
        volume_at(&self.path)
    }

    fn path(&self) -> &Path { &self.path }

    fn cleanup(&mut self) -> io::Result<()> { self.remove_owned() }
}

/// The Windows runner only takes a GUID-rooted child directory, never a target drive letter.
pub(super) fn cipher_args(path: &Path) -> io::Result<String> {
    qualified_root(path)?;
    let raw = path.to_str().unwrap();
    if raw.len() <= 49 || raw.ends_with('\\') || raw[49..].contains(['"', '\r', '\n', '\0', ':'])
        || !path.file_name().and_then(|n| n.to_str()).is_some_and(valid_name) {
        return Err(io::Error::other("a private mount directory is required"));
    }
    Ok(format!("\"/w:{raw}\""))
}

pub(super) fn sweep(data: Option<&Path>, system: Option<&SystemVolume>) {
    for site in host_sites(data, system) {
        if let Err(e) = sweep_site(&site) { eprintln!("Sprawl couldn't check for leftover mounts in {}: {e}", site.parent.join(site.namespace).display()); }
    }
}

fn sweep_site(site: &HostSite) -> io::Result<()> {
    let host = match Host::open(site, false, None) {
        Ok(host) => Arc::new(host),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    sweep_host(host)
}

fn sweep_host(host: Arc<Host>) -> io::Result<()> {
    for f in fs::read_dir(&host.path)? {
        let Ok(f) = f else { continue; };
        let file = f.file_name();
        let Some((file, name)) = file.to_str().and_then(|f| f.strip_suffix(".lease").filter(|s| valid_name(s)).map(|s| (f, s))) else { continue; };
        let result = sweep_one(host.clone(), file, name);
        if let Err(e) = result { eprintln!("Sprawl left private wipe mount {} alone: {e}", host.path.join(name).display()); }
    }
    Ok(())
}

fn sweep_one(host: Arc<Host>, file: &str, name: &str) -> io::Result<()> {
    let lease = match host.dir.open_in(file, READ_DATA | WRITE_DATA | DELETE, SHARE_READ, Disposition::Open, Kind::File, true) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound || e.raw_os_error() == Some(32) => return Ok(()),
        Err(e) => return Err(e),
    };
    let record = Record::read(&lease)?;
    let hold = host.dir.open_dir_itself(name, READ_DATA, SHARE_READ | SHARE_WRITE)?;
    if !sweep_matches(&record, local::full_identity(&hold)?, binding(&hold)?.as_ref(), &host.volume) {
        return Ok(());
    }
    let anchor = host.dir.open_dir_itself(name, READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE | SHARE_DELETE)?;
    let mut mount = NativeMount::from_owned(host, name.to_owned(), hold, anchor, record)?;
    mount.lease = Some(lease);
    mount.remove_owned()
}

fn sweep_matches(record: &Record, id: (u64, u128), mounted: Option<&Volume>, host: &Volume) -> bool {
    id == record.id && mounted.is_none_or(|volume| volume == &record.volume) && host != &record.volume
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(n: u32) -> Volume {
        Volume::parse(&format!("\\\\?\\Volume{{00000000-0000-0000-0000-{n:012x}}}\\")).unwrap()
    }

    #[test]
    fn same_volume_local_data_uses_a_mount_host_on_the_system_volume() {
        let target = volume(2);
        let system = SystemVolume { root: r"C:\".into(), volume: volume(1) };
        let data = Path::new(r"E:\Local Data\Sprawl");
        let mut candidates = Vec::new();
        let chosen = other_host(data, Some(&system), |site| {
            candidates.push(site.parent.clone());
            let actual = if site.parent == data { &target } else { &system.volume };
            if !allowed_host(actual, site.required.as_ref(), Some(&target)) {
                return Err(io::Error::other("this host is the target"));
            }
            Ok((actual.clone(), site.parent.join(site.namespace)))
        }).unwrap();
        assert_eq!(candidates, [data, Path::new(r"C:\ProgramData")]);
        assert_eq!(chosen, (system.volume.clone(), PathBuf::from(r"C:\ProgramData\Sprawl-wipe-mounts-v1")));
        assert!(!allowed_host(&volume(3), Some(&system.volume), Some(&target)), "redirected ProgramData cannot masquerade as a system-volume host");
        let sweep = host_sites(Some(data), Some(&system));
        assert!(sweep.iter().any(|site| site.parent.join(site.namespace) == chosen.1), "startup checks the fallback namespace too");
    }

    #[test]
    fn startup_sweep_only_accepts_the_recorded_folder_and_binding() {
        let a = volume(1);
        let b = volume(2);
        let host = volume(9);
        let record = Record { id: (11, 22), volume: a.clone() };
        assert!(sweep_matches(&record, (11, 22), Some(&a), &host));
        assert!(!sweep_matches(&record, (11, 23), Some(&a), &host), "replacement folder");
        assert!(!sweep_matches(&record, (12, 22), Some(&a), &host), "replacement host");
        assert!(!sweep_matches(&record, (11, 22), Some(&b), &host), "different mount");
        assert!(sweep_matches(&record, (11, 22), None, &host), "already detached is allowed with the exact recorded ID");
        assert!(!sweep_matches(&record, (11, 22), Some(&a), &a), "same-volume mount");
        assert!(!valid_name("mount-1-2-3-not-sprawl"));
    }

    #[test]
    fn cleanup_detaches_only_an_owned_binding_before_removing_the_empty_folder() {
        let a = volume(1);
        let b = volume(2);
        detach_owned(None, &a, || panic!("already detached")).unwrap();
        assert!(detach_owned(Some(&b), &a, || panic!("the binding was replaced")).is_err());
        let mut called = false;
        detach_owned(Some(&a), &a, || { called = true; Ok(()) }).unwrap();
        assert!(called);
        assert!(detach_owned(Some(&a), &a, || Err(io::Error::other("simulated detach error"))).is_err(),
            "a failed detach never authorizes directory removal");
    }

    #[test]
    fn failed_prelease_setup_rolls_back_only_the_exact_new_directory_and_lease() {
        let tmp = local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-lease-setup").unwrap();
        let dir = local::directory(tmp.path()).unwrap();
        let target = volume(2);
        for (name, stage) in [("mount-1-2-3", "identity"), ("mount-1-2-4", "collision"), ("mount-1-2-5", "write")] {
            let created = dir.create_empty_directory(name).unwrap();
            let note = tmp.path().join(lease_name(name));
            if stage == "collision" { fs::write(&note, "foreign lease").unwrap(); }
            let result = prepare_lease(&dir, name, &created, &target, |file| {
                if stage == "identity" { Err(io::Error::other("identity failure")) } else { local::full_identity(file) }
            }, |_, mut file| {
                assert_eq!(stage, "write");
                file.write_all(b"unfinished record")?;
                Err(io::Error::other("lease write failure"))
            });
            assert!(result.is_err(), "{stage}");
            drop(created);
            assert!(!tmp.path().join(name).exists(), "{stage}");
            if stage == "collision" {
                assert_eq!(fs::read_to_string(note).unwrap(), "foreign lease");
            } else {
                assert!(!note.exists(), "{stage}");
            }
        }
    }

    #[test]
    fn startup_recovers_detached_empty_folder_but_not_an_unrecorded_crash_or_nonempty_folder() {
        let tmp = local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-lease-crash").unwrap();
        let host = Arc::new(Host { dir: local::directory(tmp.path()).unwrap(), path: tmp.path().to_owned(), volume: volume(1) });
        let target = volume(2);
        let unrecorded = "mount-1-2-3";
        drop(host.dir.create_empty_directory(unrecorded).unwrap());
        for (name, contents) in [("mount-1-2-4", false), ("mount-1-2-5", true)] {
            let created = host.dir.create_empty_directory(name).unwrap();
            let (_, anchor, lease) = prepare_lease(&host.dir, name, &created, &target, local::full_identity, Record::write).unwrap();
            drop((created, anchor, lease)); // valid lease on disk, but no live worker or mount
            if contents { fs::write(tmp.path().join(name).join("ordinary.txt"), "keep").unwrap(); }
        }
        let malformed = "mount-1-2-6";
        drop(host.dir.create_empty_directory(malformed).unwrap());
        fs::write(tmp.path().join(lease_name(malformed)), "incomplete record").unwrap();
        sweep_host(host).unwrap();
        assert!(tmp.path().join(unrecorded).is_dir(), "no lease means no ownership");
        assert!(!tmp.path().join("mount-1-2-4").exists(), "the recorded, detached empty folder was removed");
        assert!(!tmp.path().join("mount-1-2-4.lease").exists());
        assert!(tmp.path().join("mount-1-2-5.lease").exists());
        assert_eq!(fs::read_to_string(tmp.path().join(r"mount-1-2-5\ordinary.txt")).unwrap(), "keep");
        assert!(tmp.path().join(malformed).is_dir());
        assert_eq!(fs::read_to_string(tmp.path().join(lease_name(malformed))).unwrap(), "incomplete record");
    }

    #[test]
    fn only_guid_reparse_targets_are_volume_mounts() {
        let target = volume(1);
        let nt = format!(r"\??\{}", &target.0[4..]);
        fn buffer(name: &str) -> Vec<u8> {
            let chars: Vec<_> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let mut buf = vec![0; 16];
            buf[..4].copy_from_slice(&MOUNT_TAG.to_le_bytes());
            buf[10..12].copy_from_slice(&(chars.len() as u16).to_le_bytes());
            buf.extend(chars);
            buf
        }
        assert_eq!(parse_binding(&buffer(&nt)).unwrap(), target);
        assert!(parse_binding(&buffer(r"\??\C:\ordinary")).is_err(), "ordinary junctions do not qualify");
        assert!(parse_binding(&buffer(&nt)[..17]).is_err(), "truncated target");
    }
}
