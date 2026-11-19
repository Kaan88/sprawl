//! The install folder, swapping files in, and the Apps entry. Installing is Inno Setup's
//! (`installer/sprawl.iss`); Sprawl itself only keeps the version in its Apps entry current
//! after an update, and still uninstalls copies set up by 0.2.0-0.2.2's own installer.

use std::path::{Path, PathBuf};

use windows::core::HSTRING;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteTreeW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_YESNO};

/// Inno Setup's Apps entry (`AppId=Sprawl`).
const INNO_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Sprawl_is1";
/// The Apps entry of 0.2.0-0.2.2's own installer.
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Sprawl";
/// The Explorer "Scan with Sprawl" entries (see the app's dialogs.rs); removed on uninstall.
const MENU_KEYS: [&str; 2] = [r"Software\Classes\Directory\shell\Sprawl", r"Software\Classes\Drive\shell\Sprawl"];

/// Updates and per-user maintenance never inherit an administrator token.
pub fn per_user_only() -> Result<(), String> {
    match sprawl_core::elevation() {
        Ok(false) => Ok(()),
        Ok(true) => Err("Close Sprawl and reopen it normally to update or maintain this per-user installation.".into()),
        Err(_) => Err("Windows could not establish that Sprawl is running without administrator rights.".into()),
    }
}

pub const LEGACY_ALERT_TASK: &str = "Sprawl low-space check";

pub fn alert_task_name() -> Result<String, String> {
    sprawl_core::identity::account_sid().map(|sid| format!("{LEGACY_ALERT_TASK} {sid}")).map_err(|e| e.to_string())
}

pub fn system_dir() -> Result<PathBuf, String> {
    #[link(name = "kernel32")]
    extern "system" { fn GetSystemDirectoryW(buf: *mut u16, len: u32) -> u32; }
    let mut buf = [0u16; 32768];
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() { return Err("Windows' system directory is unknown".into()); }
    String::from_utf16(&buf[..n]).map(PathBuf::from).map_err(|e| e.to_string())
}

/// What Task Scheduler itself says about a task: its principal and its actions (type, program,
/// arguments). Read through the Task Scheduler API, never parsed out of XML text.
#[derive(Debug, Clone, PartialEq)]
struct TaskFacts {
    user_id: String,
    group_id: String,
    logon: i32,
    actions: Vec<(i32, String, String)>,
}

/// Whether the task is the former shared low-space check of exactly this account: its principal
/// is this account's SID (a name is looked up to its SID) and no group, it doesn't run as a
/// group or service, and its only action is Sprawl's own `sprawl.exe --check`.
fn legacy_task_owned(facts: &TaskFacts, sid: &str, sid_of: &dyn Fn(&str) -> Option<String>) -> bool {
    const LOGON_GROUP: i32 = 4;
    const LOGON_SERVICE: i32 = 5;
    const ACTION_EXEC: i32 = 0;
    if !facts.group_id.trim().is_empty() || matches!(facts.logon, LOGON_GROUP | LOGON_SERVICE) { return false; }
    let user = facts.user_id.trim();
    if user.is_empty() { return false; }
    let principal = if user.starts_with("S-1-") { Some(user.to_string()) } else { sid_of(user) };
    if !principal.is_some_and(|p| p.eq_ignore_ascii_case(sid)) { return false; }
    let [(kind, program, args)] = facts.actions.as_slice() else { return false; };
    let program = program.trim().trim_matches('"');
    *kind == ACTION_EXEC
        && Path::new(program).file_name().is_some_and(|n| n.eq_ignore_ascii_case("sprawl.exe"))
        && args.trim() == "--check"
}

/// An account name's SID, as text.
fn sid_of_account(name: &str) -> Option<String> {
    #[link(name = "advapi32")]
    extern "system" {
        fn LookupAccountNameW(system: *const u16, name: *const u16, sid: *mut core::ffi::c_void, sid_len: *mut u32, domain: *mut u16, domain_len: *mut u32, kind: *mut i32) -> i32;
        fn ConvertSidToStringSidW(sid: *mut core::ffi::c_void, out: *mut *mut u16) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" { fn LocalFree(ptr: *mut core::ffi::c_void) -> *mut core::ffi::c_void; }
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let mut sid = [0u64; 16];
    let mut sid_len = std::mem::size_of_val(&sid) as u32;
    let mut domain = [0u16; 256];
    let mut domain_len = domain.len() as u32;
    let mut kind = 0;
    // SAFETY: every buffer is sized as passed; the SID text is freed with LocalFree.
    unsafe {
        if LookupAccountNameW(std::ptr::null(), wide.as_ptr(), sid.as_mut_ptr().cast(), &mut sid_len, domain.as_mut_ptr(), &mut domain_len, &mut kind) == 0 { return None; }
        let mut out = std::ptr::null_mut();
        if ConvertSidToStringSidW(sid.as_mut_ptr().cast(), &mut out) == 0 || out.is_null() { return None; }
        let mut len = 0;
        while len < 184 && *out.add(len) != 0 { len += 1; }
        let text = String::from_utf16(std::slice::from_raw_parts(out, len)).ok();
        LocalFree(out.cast());
        text.filter(|t| len < 184 && t.starts_with("S-1-"))
    }
}

/// Run `f` with the root task folder, on a thread of its own that joins COM's multithreaded
/// apartment (the caller may be an STA UI thread).
fn with_task_root<T: Send + 'static>(f: impl FnOnce(&windows::Win32::System::TaskScheduler::ITaskFolder) -> Option<T> + Send + 'static) -> Option<T> {
    std::thread::spawn(move || {
        use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
        use windows::Win32::System::TaskScheduler::{ITaskService, TaskScheduler};
        use windows::Win32::System::Variant::VARIANT;
        // SAFETY: COM is initialized on this thread for the calls below and uninitialized after
        // every interface is dropped.
        unsafe {
            if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() { return None; }
            let out = (|| {
                let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER).ok()?;
                let none = VARIANT::default();
                service.Connect(&none, &none, &none, &none).ok()?;
                let root = service.GetFolder(&windows::core::BSTR::from("\\")).ok()?;
                f(&root)
            })();
            CoUninitialize();
            out
        }
    }).join().ok().flatten()
}

/// The former shared task's facts, or None when it doesn't exist or can't be read completely.
fn legacy_task_facts(root: &windows::Win32::System::TaskScheduler::ITaskFolder) -> Option<TaskFacts> {
    use windows::core::{Interface, BSTR};
    use windows::Win32::System::TaskScheduler::{IExecAction, TASK_ACTION_TYPE, TASK_LOGON_TYPE};
    // SAFETY: plain COM calls on live interfaces; out-parameters are owned BSTRs and integers.
    unsafe {
        let task = root.GetTask(&BSTR::from(LEGACY_ALERT_TASK)).ok()?;
        let definition = task.Definition().ok()?;
        let principal = definition.Principal().ok()?;
        let (mut user_id, mut group_id, mut logon) = (BSTR::new(), BSTR::new(), TASK_LOGON_TYPE(0));
        principal.UserId(&mut user_id).ok()?;
        principal.GroupId(&mut group_id).ok()?;
        principal.LogonType(&mut logon).ok()?;
        let collection = definition.Actions().ok()?;
        let mut count = 0;
        collection.Count(&mut count).ok()?;
        if !(0..=16).contains(&count) { return None; }
        let mut actions = Vec::new();
        for i in 1..=count {
            let action = collection.get_Item(i).ok()?;
            let mut kind = TASK_ACTION_TYPE(-1);
            action.Type(&mut kind).ok()?;
            let (mut program, mut args) = (BSTR::new(), BSTR::new());
            if let Ok(exec) = action.cast::<IExecAction>() {
                exec.Path(&mut program).ok()?;
                exec.Arguments(&mut args).ok()?;
            }
            actions.push((kind.0, program.to_string(), args.to_string()));
        }
        Some(TaskFacts { user_id: user_id.to_string(), group_id: group_id.to_string(), logon: logon.0, actions })
    }
}

/// The argument that makes `sprawl.exe` retire the former shared task and exit (started by an
/// elevated Sprawl without administrator rights).
pub const MIGRATE_ARG: &str = "--migrate-alert-task";

/// Whether the former shared task exists and is exactly this account's low-space check. Read-only.
fn legacy_task_is_ours() -> bool {
    let Ok(sid) = sprawl_core::identity::account_sid() else { return false; };
    with_task_root(move |root| legacy_task_facts(root).map(|facts| legacy_task_owned(&facts, &sid, &sid_of_account))).unwrap_or(false)
}

/// Delete the former shared task name only after Task Scheduler identifies its principal
/// as this process's exact SID. Unknown or other-account tasks are always left alone.
/// Elevated, Sprawl only looks: the delete runs in a copy started as the signed-in user without
/// administrator rights, so it can remove no more than that user could (1.0 never migrated
/// from an always-elevated Sprawl, which left the old task running).
pub fn migrate_alert_task() {
    if !legacy_task_is_ours() { return; }
    match sprawl_core::elevation() {
        Ok(false) => delete_legacy_task(),
        Ok(true) => {
            let Ok(exe) = std::env::current_exe() else { return; };
            if let Ok(p) = sprawl_core::identity::launch_unelevated(&exe, &[MIGRATE_ARG]) {
                let _ = sprawl_core::identity::wait_exit(&p, 30_000);
            }
        }
        Err(_) => {}
    }
}

/// For [`MIGRATE_ARG`]: retire the former shared task here, never elevated and never by
/// starting another copy.
pub fn migrate_alert_task_unelevated() {
    if per_user_only().is_ok() && legacy_task_is_ours() { delete_legacy_task(); }
}

/// Checks the task again and deletes it in the same Task Scheduler session, without
/// administrator rights. Task Scheduler deletes by name only: a task swapped in between this
/// check and the delete would have to be one this account may delete anyway, with the same
/// principal and Sprawl's own `--check` action.
fn delete_legacy_task() {
    if per_user_only().is_err() { return; }
    let Ok(sid) = sprawl_core::identity::account_sid() else { return; };
    with_task_root(move |root| {
        let facts = legacy_task_facts(root)?;
        if !legacy_task_owned(&facts, &sid, &sid_of_account) { return None; }
        // SAFETY: a plain COM call on the live folder interface.
        unsafe { root.DeleteTask(&windows::core::BSTR::from(LEGACY_ALERT_TASK), 0).ok() }
    });
}

pub fn remove_alert_task() {
    use std::os::windows::process::CommandExt;
    if per_user_only().is_err() { return; }
    let (Ok(task), Ok(sys)) = (alert_task_name(), system_dir()) else { return; };
    let _ = std::process::Command::new(sys.join("schtasks.exe")).current_dir(&sys)
        .args(["/Delete", "/TN", &task, "/F"]).creation_flags(0x0800_0000)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    migrate_alert_task();
}

/// `%LOCALAPPDATA%\Programs\Sprawl`.
pub fn install_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Programs").join("Sprawl"))
}

/// The Start menu shortcut 0.2.0-0.2.2's own installer made.
fn shortcut_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Microsoft").join("Windows").join("Start Menu").join("Programs").join("Sprawl.lnk"))
}

fn same(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase();
    n(a) == n(b)
}

fn here() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf))
}

/// A text value of `HKCU\<key>`.
fn reg_str(key: &str, name: &str) -> Option<String> {
    let mut k = HKEY::default();
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(key), None, KEY_READ, &mut k) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let r = RegQueryValueExW(k, &HSTRING::from(name), None, None, Some(buf.as_mut_ptr().cast()), Some(&mut size));
        let _ = RegCloseKey(k);
        (r == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).min(buf.len())]).trim_end_matches('\0').to_string())
    }
}

/// Where Inno Setup installed Sprawl, if it did.
fn inno_dir() -> Option<PathBuf> {
    reg_str(INNO_KEY, "InstallLocation").filter(|s| !s.trim().is_empty()).map(|s| PathBuf::from(s.trim().trim_end_matches('\\')))
}

/// Whether this program runs from the installed folder (Inno Setup's, or the old installer's).
pub fn is_installed_copy() -> bool {
    let Some(here) = here() else { return false };
    inno_dir().is_some_and(|d| same(&here, &d)) || (reg_str(UNINSTALL_KEY, "DisplayVersion").is_some() && install_dir().is_some_and(|d| same(&here, &d)))
}

/// Whether this copy was set up by 0.2.0-0.2.2's own installer (its `--uninstall` is ours).
fn is_legacy_copy() -> bool {
    let Some(here) = here() else { return false };
    reg_str(UNINSTALL_KEY, "DisplayVersion").is_some() && install_dir().is_some_and(|d| same(&here, &d))
}

/// Remove the `.old` files a previous swap left (they may still be in use until now).
pub fn remove_old(dir: &Path) {
    if per_user_only().is_err() { return; }
    let Ok(_held) = sprawl_core::local::directory(dir) else { return; };
    // An interrupted swap may have left the old bundle as the only usable copy. Do not
    // discard any backup while a required current file is missing or redirected.
    let Ok(_current) = crate::FILES.iter().map(|name| sprawl_core::local::file(&dir.join(name), 512 << 20)).collect::<Result<Vec<_>, _>>() else { return; };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.strip_suffix(".old").is_some_and(|name| crate::FILES.iter().any(|owned| owned.eq_ignore_ascii_case(name))) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Where an update is downloaded to before it is swapped in.
pub const STAGE: &str = ".sprawl-update";

/// Empty the staging folder: only file names an update could have written, then the folder
/// itself if that leaves it empty (never recursive, never through a link).
pub fn clean_stage(dir: &Path) {
    if per_user_only().is_err() { return; }
    let Ok(_held) = sprawl_core::local::directory(dir) else { return; };
    let st = dir.join(STAGE);
    let Ok(_stage) = sprawl_core::local::directory(&st) else { return; };
    if let Ok(rd) = std::fs::read_dir(&st) {
        for e in rd.flatten() {
            if crate::FILES.iter().any(|owned| owned.eq_ignore_ascii_case(&e.file_name().to_string_lossy())) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    drop(_stage);
    let _ = std::fs::remove_dir(&st);
}

/// A junction, symbolic link or other reparse point.
pub fn is_link(p: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0)
}

pub(crate) fn hash_file(file: &mut std::fs::File) -> Result<String, String> {
    use std::io::Read;
    use sha2::{Digest, Sha256};
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    if len > 512 << 20 { return Err("The update file exceeds 512 MB".into()); }
    let mut hash = Sha256::new();
    let mut buf = [0u8; 64 << 10];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        hash.update(&buf[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) fn check_hash(file: &mut std::fs::File, sha: &str) -> Result<(), String> {
    if hash_file(file)? != sha { return Err("An update file changed before it could be used".into()); }
    Ok(())
}

/// Keep all newly installed launch files stable, after hashing their open handles, until
/// the caller has started the replacement process. No elevated launch is permitted.
pub fn verified_launch(dir: &Path, staged: &[(String, PathBuf, String)]) -> Result<Vec<sprawl_core::local::LocalFile>, String> {
    per_user_only()?;
    staged.iter().map(|(name, _, sha)| {
        if !crate::manifest::valid_name(name) { return Err("Invalid update name".into()); }
        let mut held = sprawl_core::local::file(&dir.join(name), 512 << 20).map_err(|e| e.to_string())?;
        check_hash(&mut held.file, sha)?;
        Ok(held)
    }).collect()
}

fn rename_open(file: &std::fs::File, to: &Path) -> std::io::Result<()> {
    use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
    #[link(name = "kernel32")]
    extern "system" { fn SetFileInformationByHandle(file: *mut core::ffi::c_void, class: u32, info: *const core::ffi::c_void, size: u32) -> i32; }
    let name: Vec<u16> = std::ffi::OsStr::new(&format!(r"\??\{}", to.display())).encode_wide().collect();
    let head = 8 + std::mem::size_of::<usize>() + 4;
    let size = head + name.len() * 2 + 2;
    let mut buf = vec![0u64; size.div_ceil(8)];
    let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * 8) };
    bytes[8 + std::mem::size_of::<usize>()..head].copy_from_slice(&((name.len() * 2) as u32).to_le_bytes());
    for (i, c) in name.iter().enumerate() { bytes[head + i * 2..head + i * 2 + 2].copy_from_slice(&c.to_le_bytes()); }
    if unsafe { SetFileInformationByHandle(file.as_raw_handle().cast(), 3, buf.as_ptr().cast(), size as u32) } != 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

pub(crate) fn delete_open(file: &std::fs::File) -> bool {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" { fn SetFileInformationByHandle(file: *mut core::ffi::c_void, class: u32, info: *const core::ffi::c_void, size: u32) -> i32; }
    // POSIX disposition removes this checked publication's name immediately, allowing its
    // old file to be restored without closing the handle or touching a replacement name.
    let flags = 3u32;
    unsafe { SetFileInformationByHandle(file.as_raw_handle().cast(), 21, (&flags as *const u32).cast(), 4) != 0 }
}

/// Put staged files (name, path, expected hash) into `dir`, after retaining a complete
/// previous launch bundle. Current files move aside to `<name>.old`; failures attempt
/// rollback without overwriting unrelated arrivals. Only plain release file names are used.
pub fn swap_in(dir: &Path, version: &str, staged: &[(String, PathBuf, String)]) -> Result<(), String> {
    swap_in_with(dir, version, staged, || {})
}

fn swap_in_with(dir: &Path, version: &str, staged: &[(String, PathBuf, String)], before_publish: impl FnOnce()) -> Result<(), String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    per_user_only()?;
    let root = sprawl_core::local::directory(dir).map_err(|e| e.to_string())?;
    let dir = &root.path;
    let _stage = sprawl_core::local::directory(&dir.join(STAGE)).map_err(|e| e.to_string())?;
    if let Some((n, _, _)) = staged.iter().find(|(n, _, _)| !crate::manifest::valid_name(n)) {
        return Err(format!("refused to write {n}"));
    }
    if !staged.iter().any(|(name, _, _)| name == "sprawl.exe") { return Err("The update has no program file".into()); }
    let mut seen = std::collections::HashSet::new();
    let mut checked = Vec::new();
    for (name, from, sha) in staged {
        if !seen.insert(name.to_ascii_lowercase()) || sprawl_core::local::lexical(from).map_err(|e| e.to_string())? != dir.join(STAGE).join(name) { return Err("Invalid or repeated staged update path".into()); }
        // DELETE is needed for a rename by handle. Delete sharing lets the checked handle
        // move; withholding write sharing keeps the bytes stable through hashing/publication.
        let mut file = std::fs::OpenOptions::new().access_mode(0x8000_0000 | 0x0001_0000).share_mode(1 | 4)
            .custom_flags(0x0020_0000 | 0x0010_0000).open(from).map_err(|e| e.to_string())?;
        let m = file.metadata().map_err(|e| e.to_string())?;
        if !m.is_file() || m.file_attributes() & (0x400 | 0x1000 | 0x0004_0000 | 0x0040_0000) != 0 { return Err("An update file is linked or offline".into()); }
        check_hash(&mut file, sha)?;
        checked.push((name.clone(), file));
    }
    let replacements = checked.iter().zip(staged).map(|((name, file), (_, _, sha))| {
        file.metadata().map(|m| crate::FileEntry { name: name.clone(), size: m.len(), sha256: sha.clone() }).map_err(|e| e.to_string())
    }).collect::<Result<Vec<_>, _>>()?;
    let _recovery = crate::recovery::prepare(dir, version, replacements)?;
    remove_old(dir);
    let mut aside: Vec<(PathBuf, std::fs::File)> = Vec::new();
    let mut placed: Vec<usize> = Vec::new();
    // put everything back; says whether that worked
    let undo = |aside: &[(PathBuf, std::fs::File)], placed: &[usize], checked: Vec<(String, std::fs::File)>| -> bool {
        let mut ok = true;
        for (i, (_, file)) in checked.into_iter().enumerate() {
            if placed.contains(&i) { ok &= delete_open(&file); }
            drop(file);
        }
        for (cur, old) in aside {
            ok &= rename_open(old, cur).is_ok();
        }
        ok
    };
    let back = |ok: bool| if ok { "nothing was changed".to_string() } else { format!("and putting the old files back failed too. Open {}\\{}\\sprawl.exe to use the previous copy, then reinstall with Sprawl Setup", dir.display(), crate::recovery::DIRECTORY) };
    for (name, _, _) in staged {
        let cur = dir.join(name);
        let current = std::fs::OpenOptions::new().access_mode(0x0001_0080).share_mode(1 | 2 | 4)
            .custom_flags(0x0020_0000 | 0x0010_0000).open(&cur);
        match current {
            Ok(current) => {
            let valid = current.metadata().is_ok_and(|m| m.is_file() && m.file_attributes() & (0x400 | 0x1000 | 0x0004_0000 | 0x0040_0000) == 0);
            if !valid { let ok = undo(&aside, &placed, checked); return Err(format!("The current {name} is linked or offline; {}", back(ok))); }
            let old = dir.join(format!("{name}.old"));
            if let Err(e) = rename_open(&current, &old) {
                let ok = undo(&aside, &placed, checked);
                return Err(format!("could not move {name} aside ({e}); {}", back(ok)));
            }
            aside.push((cur, current));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => { let ok = undo(&aside, &placed, checked); return Err(format!("could not open {name} for updating ({e}); {}", back(ok))); }
        }
    }
    before_publish();
    for i in 0..checked.len() {
        let name = checked[i].0.to_string();
        let to = dir.join(&name);
        if let Err(e) = rename_open(&checked[i].1, &to) {
            let ok = undo(&aside, &placed, checked);
            return Err(format!("could not put the new {name} in place ({e}); {}", back(ok)));
        }
        placed.push(i);
    }
    Ok(())
}

fn set_str(key: HKEY, name: &str, value: &str) {
    let data: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2) };
    unsafe {
        let _ = RegSetValueExW(key, &HSTRING::from(name), None, REG_SZ, Some(bytes));
    }
}

/// After an update: the version in the Apps entry of the install in `dir`. Only an entry that
/// exists and points at `dir` is touched; none is created.
pub fn register(dir: &Path, version: &str) -> Result<(), String> {
    per_user_only()?;
    let key_name = if inno_dir().is_some_and(|d| same(dir, &d)) {
        INNO_KEY
    } else if reg_str(UNINSTALL_KEY, "DisplayVersion").is_some() && install_dir().is_some_and(|d| same(dir, &d)) {
        UNINSTALL_KEY
    } else {
        return Ok(());
    };
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(key_name), None, KEY_WRITE, &mut key) } != ERROR_SUCCESS {
        return Err("could not update the Apps entry".into());
    }
    set_str(key, "DisplayVersion", version);
    if key_name == INNO_KEY {
        set_str(key, "DisplayName", "Sprawl");
    }
    unsafe {
        let _ = RegCloseKey(key);
    }
    Ok(())
}

pub fn message(text: &str, question: bool, warn: bool) -> bool {
    let kind = if question { MB_YESNO | MB_ICONQUESTION } else if warn { MB_OK | MB_ICONWARNING } else { MB_OK | MB_ICONINFORMATION };
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Sprawl"), kind) == IDYES }
}

/// `sprawl.exe --uninstall`, from Settings > Apps for copies set up by 0.2.0-0.2.2's own
/// installer (Inno Setup's copies uninstall with Inno's uninstaller): remove the shortcut, the
/// Apps entry, the Explorer menu entries and the program files. Settings, history and the clean-up log stay
/// (they are in %APPDATA%\Sprawl and %LOCALAPPDATA%\Sprawl). `quiet` skips the questions.
pub fn uninstall(quiet: bool) {
    if let Err(e) = per_user_only() { if !quiet { message(&e, false, true); } return; }
    let Some(dir) = install_dir() else { return };
    if !is_legacy_copy() {
        if !quiet {
            message(&format!("This copy of Sprawl isn't the installed one.\n\nThe installed copy is in {}.", dir.display()), false, true);
        }
        return;
    }
    if !quiet && !message("Remove Sprawl from this PC?\n\nYour settings and scan history are kept, in case you install it again.", true, false) {
        return;
    }
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(UNINSTALL_KEY));
        for k in MENU_KEYS {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(k));
        }
    }
    if let Some(l) = shortcut_path() {
        let _ = std::fs::remove_file(l);
    }
    // the low-space check's scheduled task, if the setting made one
    remove_alert_task();
    // only Sprawl's own files; the folder itself goes only if that leaves it empty
    remove_old(&dir);
    clean_stage(&dir);
    if !quiet {
        message("Sprawl has been removed.", false, false);
    }
    // This program and the DLL it has loaded are in use until it exits, and another Sprawl
    // window may still be open: a short-lived cmd tries every 2 seconds for 2 minutes, then
    // removes the folder if it is empty. cmd and ping by full path, started in System32.
    use std::os::windows::process::CommandExt;
    let sys = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into())).join("System32");
    let dels: String = crate::FILES.iter().map(|n| format!("del /f /q \"{}\" 2>nul & ", dir.join(n).display())).collect();
    let script = format!(
        "for /l %i in (1,1,60) do @(\"{ping}\" -n 3 127.0.0.1 >nul & {dels}rmdir \"{d}\" 2>nul & if not exist \"{d}\" exit)",
        ping = sys.join("ping.exe").display(),
        d = dir.display()
    );
    let _ = std::process::Command::new(sys.join("cmd.exe")).current_dir(&sys).raw_arg("/d /q /c").raw_arg(script).creation_flags(0x0800_0000).spawn();
}

#[cfg(test)]
mod update_tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    fn fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sprawl-update-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(dir.join(STAGE)).unwrap();
        dir
    }

    #[test]
    fn legacy_task_migration_requires_the_exact_principal() {
        let sid = "S-1-5-21-100-200-300-1001";
        let ours = TaskFacts { user_id: sid.into(), group_id: String::new(), logon: 3, actions: vec![(0, r"C:\Users\a\AppData\Local\Programs\Sprawl\sprawl.exe".into(), "--check".into())] };
        let lookup = |name: &str| (name == r"PC\me").then(|| sid.to_string());
        assert!(legacy_task_owned(&ours, sid, &lookup));
        // the quoted program form schtasks /TR leaves, and a principal given as a name
        assert!(legacy_task_owned(&TaskFacts { actions: vec![(0, r#""C:\Sprawl\sprawl.exe""#.into(), " --check ".into())], ..ours.clone() }, sid, &lookup));
        assert!(legacy_task_owned(&TaskFacts { user_id: r"PC\me".into(), ..ours.clone() }, sid, &lookup));
        // another account, an unknown name, no principal, a group or a service principal
        assert!(!legacy_task_owned(&TaskFacts { user_id: "S-1-5-21-100-200-300-1002".into(), ..ours.clone() }, sid, &lookup));
        assert!(!legacy_task_owned(&TaskFacts { user_id: r"COMPUTER\sameusername".into(), ..ours.clone() }, sid, &lookup));
        assert!(!legacy_task_owned(&TaskFacts { user_id: String::new(), ..ours.clone() }, sid, &lookup));
        assert!(!legacy_task_owned(&TaskFacts { group_id: "S-1-5-32-545".into(), ..ours.clone() }, sid, &lookup));
        assert!(!legacy_task_owned(&TaskFacts { logon: 4, ..ours.clone() }, sid, &lookup));
        assert!(!legacy_task_owned(&TaskFacts { logon: 5, ..ours.clone() }, sid, &lookup));
        // anything but Sprawl's one --check action
        assert!(!legacy_task_owned(&TaskFacts { actions: vec![], ..ours.clone() }, sid, &lookup));
        assert!(!legacy_task_owned(&TaskFacts { actions: vec![(0, r"C:\x\other.exe".into(), "--check".into())], ..ours.clone() }, sid, &lookup));
        assert!(!legacy_task_owned(&TaskFacts { actions: vec![(0, r"C:\x\sprawl.exe".into(), "--check --other".into())], ..ours.clone() }, sid, &lookup));
        assert!(!legacy_task_owned(&TaskFacts { actions: vec![(5, r"C:\x\sprawl.exe".into(), "--check".into())], ..ours.clone() }, sid, &lookup));
        let mut two = ours.clone();
        two.actions.push((0, r"C:\Windows\System32\cmd.exe".into(), "/c x".into()));
        assert!(!legacy_task_owned(&two, sid, &lookup));
        // read-only: Task Scheduler answers through the API, and the account name maps to its SID
        assert_eq!(with_task_root(|_| Some(7)), Some(7));
        let me = sprawl_core::identity::account_sid().unwrap();
        let name = format!("{}\\{}", std::env::var("USERDOMAIN").unwrap(), std::env::var("USERNAME").unwrap());
        assert_eq!(sid_of_account(&name).as_deref(), Some(me.as_str()));
        let actual = sprawl_core::identity::account_sid().unwrap();
        assert_eq!(alert_task_name().unwrap(), format!("{LEGACY_ALERT_TASK} {actual}"));
    }

    #[test]
    fn checked_source_is_renamed_even_after_its_name_changes() {
        let dir = fixture("handle");
        let source = dir.join(STAGE).join("sprawl.exe");
        std::fs::write(&source, b"verified replacement").unwrap();
        let mut file = std::fs::OpenOptions::new().access_mode(0x8000_0000 | 0x0001_0000).share_mode(1 | 4).open(&source).unwrap();
        check_hash(&mut file, &crate::sha256_hex(b"verified replacement")).unwrap();
        assert!(std::fs::write(&source, b"changed").is_err());
        let moved = dir.join(STAGE).join("moved.exe");
        std::fs::rename(&source, &moved).unwrap();
        std::fs::write(&source, b"unrelated new arrival").unwrap();
        rename_open(&file, &dir.join("sprawl.exe")).unwrap();
        drop(file);
        assert_eq!(std::fs::read(dir.join("sprawl.exe")).unwrap(), b"verified replacement");
        assert_eq!(std::fs::read(source).unwrap(), b"unrelated new arrival");
        assert!(!moved.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn update_preflight_and_launch_guards_preserve_checked_bytes() {
        let dir = fixture("swap");
        std::fs::write(dir.join("sprawl.exe"), b"old program").unwrap();
        std::fs::write(dir.join(STAGE).join("sprawl.exe"), b"new program").unwrap();
        let mut files = vec![("sprawl.exe".into(), dir.join(STAGE).join("sprawl.exe"), crate::sha256_hex(b"wrong"))];
        assert!(swap_in(&dir, "99.0.0", &files).is_err());
        assert_eq!(std::fs::read(dir.join("sprawl.exe")).unwrap(), b"old program");
        if per_user_only().is_ok() {
            files[0].2 = crate::sha256_hex(b"new program");
            std::fs::write(dir.join(crate::FILES[1]), b"old bootstrap").unwrap();
            swap_in(&dir, "99.0.0", &files).unwrap();
            assert_eq!(std::fs::read(dir.join("sprawl.exe.old")).unwrap(), b"old program");
            let launch = verified_launch(&dir, &files).unwrap();
            assert!(std::fs::write(dir.join("sprawl.exe"), b"changed").is_err());
            assert!(std::fs::rename(dir.join("sprawl.exe"), dir.join("displaced.exe")).is_err());
            drop(launch);
            assert_eq!(std::fs::read(dir.join("sprawl.exe")).unwrap(), b"new program");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn maintenance_keeps_unrelated_files_and_stage_subfolders() {
        let dir = fixture("maintenance");
        for name in crate::FILES { std::fs::write(dir.join(name), b"current").unwrap(); }
        for name in ["sprawl.exe.old", "another.exe.old", "notes.old"] { std::fs::write(dir.join(name), b"keep or remove by ownership").unwrap(); }
        for name in ["sprawl.exe", "another.exe", "notes.txt"] { std::fs::write(dir.join(STAGE).join(name), b"staged").unwrap(); }
        std::fs::create_dir(dir.join(STAGE).join("private")).unwrap();
        remove_old(&dir);
        clean_stage(&dir);
        assert!(dir.join("another.exe.old").exists() && dir.join("notes.old").exists());
        assert!(dir.join(STAGE).join("another.exe").exists() && dir.join(STAGE).join("private").exists());
        if per_user_only().is_ok() { assert!(!dir.join("sprawl.exe.old").exists() && !dir.join(STAGE).join("sprawl.exe").exists()); }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_second_publication_keeps_new_arrivals_and_restores_by_handle() {
        if per_user_only().is_err() { return; }
        let dir = fixture("rollback");
        let files: Vec<_> = crate::FILES.iter().map(|&name| {
            std::fs::write(dir.join(name), format!("old {name}")).unwrap();
            let new = format!("new {name}");
            let source = dir.join(STAGE).join(name);
            std::fs::write(&source, &new).unwrap();
            (name.to_string(), source, crate::sha256_hex(new.as_bytes()))
        }).collect();
        let blocked = dir.join(crate::FILES[1]);
        let result = swap_in_with(&dir, "99.0.0", &files, || std::fs::write(&blocked, b"unrelated new arrival").unwrap());
        assert!(result.is_err());
        assert_eq!(std::fs::read(dir.join("sprawl.exe")).unwrap(), b"old sprawl.exe");
        assert_eq!(std::fs::read(&blocked).unwrap(), b"unrelated new arrival");
        assert_eq!(std::fs::read(dir.join(format!("{}.old", crate::FILES[1]))).unwrap(), format!("old {}", crate::FILES[1]).as_bytes());
        for name in crate::FILES {
            assert_eq!(std::fs::read(dir.join(crate::recovery::DIRECTORY).join(name)).unwrap(), format!("old {name}").as_bytes());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn interruption_after_displacement_retains_a_complete_previous_launch_bundle() {
        if per_user_only().is_err() { return; }
        let dir = fixture("interrupted");
        let mut old = Vec::new();
        let files: Vec<_> = crate::FILES.iter().map(|&name| {
            let bytes = format!("complete old {name}");
            std::fs::write(dir.join(name), &bytes).unwrap();
            old.push((name.to_string(), dir.join(crate::recovery::DIRECTORY).join(name), crate::sha256_hex(bytes.as_bytes())));
            let source = dir.join(STAGE).join(name);
            std::fs::write(&source, b"new file").unwrap();
            (name.to_string(), source, crate::sha256_hex(b"new file"))
        }).collect();
        let interrupted = std::panic::catch_unwind(|| swap_in_with(&dir, "99.0.0", &files, || panic!("simulated process interruption")));
        assert!(interrupted.is_err());
        assert!(!dir.join("sprawl.exe").exists(), "interruption happened after the old names moved");
        let backup = dir.join(crate::recovery::DIRECTORY);
        assert!(backup.join("READY").exists());
        let launch = verified_launch(&backup, &old).unwrap();
        assert_eq!(launch.len(), crate::FILES.len());
        drop(launch);
        crate::recovery::cleanup(&dir);
        assert!(backup.join("READY").exists(), "an incomplete main folder cannot discard its recovery copy");
        for name in crate::FILES { std::fs::rename(dir.join(format!("{name}.old")), dir.join(name)).unwrap(); }
        crate::recovery::cleanup(&dir);
        assert!(!backup.exists(), "a complete restored bundle may discard recovery at its own normal startup");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
