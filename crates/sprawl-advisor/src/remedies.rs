//! Remedies that free space without deleting anything: compressing installed programs
//! (`compact /exe:...`, which Sprawl runs when ticked: F1), letting OneDrive keep files online
//! only, and moving big things to a drive with room (tips, for the user to do).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sprawl_core::{format_size, Tree, ROOT};

use crate::finding::{within, Action, Finding, Remedy, Risk, Source};
use crate::rules::ps_quote;
use crate::safety::{Never, TailMatcher};
use crate::segments;

const GB: u64 = 1 << 30;
const LIBRARY_CHILD: u64 = 200 << 20;
/// OneDrive and other cloud files: the data is online only (a placeholder).
const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
const RECALL_ON_OPEN: u32 = 0x0004_0000;
const OFFLINE: u32 = 0x1000;
/// "Always keep on this device".
const PINNED: u32 = 0x0008_0000;
/// A file `compact /exe` compressed is stored behind a reparse point (WOF), not with the old
/// compressed attribute.
const REPARSE: u32 = 0x400;

/// How every compress command Sprawl writes begins (F1): what tells a compress run from a
/// tool's clean-up, which is measured by what the folder lost on disk, not in size.
pub const COMPRESS: &str = "compact.exe /c /s /a /i /q /exe:";

/// The algorithm a compress command of Sprawl's uses (`xpress16k`).
pub fn compress_alg(command: &str) -> Option<&'static str> {
    sprawl_core::wof::Alg::parse(command.strip_prefix(COMPRESS)?.split(' ').next()?).map(sprawl_core::wof::Alg::name)
}

/// The algorithms the user can pick for a compress (B1): name, label, and the share of LZX's
/// saving each gets (what the estimates use).
pub const COMPRESS_CHOICES: &[(&str, &str, f64)] = &[
    ("xpress4k", "XPRESS4K: fastest", 0.65),
    ("xpress8k", "XPRESS8K", 0.75),
    ("xpress16k", "XPRESS16K", 0.85),
    ("lzx", "LZX: smallest, slower to read", 1.0),
];

fn compress_share(alg: &str) -> Option<f64> {
    let alg = sprawl_core::wof::Alg::parse(alg)?.name();
    COMPRESS_CHOICES.iter().find(|(a, _, _)| *a == alg).map(|c| c.2)
}

/// `f`, one of Sprawl's compress findings, with the algorithm `alg` instead of its own: the
/// same command on the same files (only the algorithm word changes), its saving estimate scaled
/// to that algorithm. None when `f` isn't a compress or `alg` isn't one of [`COMPRESS_CHOICES`].
pub fn with_compress_alg(f: &Finding, alg: &str) -> Option<Finding> {
    let (Some((Action::Tool { command }, admin)), Some(new)) = (crate::clean::action(f), compress_share(alg)) else { return None };
    let old = compress_alg(&command)?;
    let old_share = compress_share(old)?;
    let (_, rest) = command.strip_prefix(COMPRESS)?.split_once(' ')?;
    let mut out = f.clone();
    out.set_action(Action::Tool { command: format!("{COMPRESS}{} {rest}", sprawl_core::wof::Alg::parse(alg)?.name()) }, admin);
    out.gain = f.gain.map(|g| (g as f64 * new / old_share) as u64);
    Some(out)
}

/// One exact uncompress argument. Validation/pinning belongs to the batch caller; no recursion.
pub fn uncompress_command(path: &Path) -> String {
    format!("compact.exe /u /a /i /q /exe {}", crate::rules::ps_quote(&path.display().to_string()))
}

/// Which directory the user was shown when asked to confirm Undo.
pub struct ReviewedUndo { path: PathBuf, id: (u64, u128) }

pub fn review_uncompress(path: &Path) -> Result<ReviewedUndo, String> {
    let root = sprawl_core::local::directory(path).map_err(|e| e.to_string())?;
    Ok(ReviewedUndo { path: root.path.clone(), id: root.identity().map_err(|e| e.to_string())? })
}

/// Undo a recorded compression only on the reviewed directory in a current supported scope.
pub fn uncompress(reviewed: &ReviewedUndo, keep: &[PathBuf], excluded: &[PathBuf]) -> Result<(), String> {
    crate::require_write().map_err(|e| e.to_string())?;
    uncompress_dispatch(reviewed, keep, crate::memory::refresh_keeps, |root, keep| {
        let policy = CompactPolicy::at_dispatch()?;
        let mut excluded = excluded.to_vec();
        excluded.extend(crate::exclusions::for_external_cleanup()?);
        uncompress_current(&root.path, keep, &excluded, &policy)
    })
}

fn uncompress_dispatch(reviewed: &ReviewedUndo, keep: &[PathBuf], refresh: impl FnOnce(&mut Vec<PathBuf>) -> Result<(), String>,
    run: impl FnOnce(&sprawl_core::local::Directory, &[PathBuf]) -> Result<(), String>) -> Result<(), String> {
    let mut keep = keep.to_vec();
    refresh(&mut keep)?;
    let root = sprawl_core::local::directory(&reviewed.path).map_err(|e| e.to_string())?;
    if root.identity().map_err(|e| e.to_string())? != reviewed.id {
        return Err("This isn't the folder you confirmed. Nothing was uncompressed.".into());
    }
    run(&root, &keep)
}

fn uncompress_current(path: &Path, keep: &[PathBuf], excluded: &[PathBuf], policy: &CompactPolicy) -> Result<(), String> {
    if crate::exclusions::intersects(path, excluded) || keep.iter().any(|p| within(path, p) || within(p, path)) {
        return Err("This scope is excluded or kept.".into());
    }
    let scope = policy.scope(path)?;
    if !live_eligible(path, scope.origin, policy, excluded, &AtomicBool::new(false), &mut CompressionBatch::default()) {
        return Err("This record is no longer in a supported program folder or library. It was left alone.".into());
    }
    uncompress_scope(path, keep, excluded, policy)
}

/// Reuse the parent inventory only while its subtree has no name, size or attribute changes.
/// The selected program still gets its own fresh held plan and identity check.
#[derive(Default)]
pub struct CompressionBatch {
    libraries: std::collections::HashMap<(u64, u128), LibraryEvidence>,
    exclusions: Vec<PathBuf>,
}

struct LibraryEvidence { watch: LibraryWatch, members: Vec<PathBuf> }

struct LibraryWatch(windows_sys::Win32::Foundation::HANDLE);

impl LibraryWatch {
    fn start(parent: &Path) -> Option<Self> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{Foundation::INVALID_HANDLE_VALUE, Storage::FileSystem::*};
        let mut full = std::ffi::OsString::new();
        if !parent.as_os_str().to_string_lossy().starts_with(r"\\?\") { full.push(r"\\?\"); }
        full.push(parent);
        let path: Vec<_> = full.encode_wide().chain(Some(0)).collect();
        // A one-shot signal, never re-armed after a change. That avoids a gap between
        // consuming a notification and taking the snapshot that will back the new cache.
        let h = unsafe { FindFirstChangeNotificationW(path.as_ptr(), 1, FILE_NOTIFY_CHANGE_FILE_NAME
            | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_ATTRIBUTES) };
        (h != INVALID_HANDLE_VALUE && !h.is_null()).then(|| Self(h))
    }

    fn unchanged(&self) -> bool {
        use windows_sys::Win32::{Foundation::WAIT_TIMEOUT, System::Threading::WaitForSingleObject};
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_TIMEOUT }
    }
}

impl Drop for LibraryWatch {
    fn drop(&mut self) { unsafe { windows_sys::Win32::Storage::FileSystem::FindCloseChangeNotification(self.0) }; }
}

impl CompressionBatch {
    fn is_member(&mut self, parent: &sprawl_core::local::Directory, path: &Path, excluded: &[PathBuf],
        inventory: impl FnOnce() -> Option<Vec<PathBuf>>) -> bool {
        if self.exclusions != excluded {
            self.libraries.clear();
            self.exclusions = excluded.to_vec();
        }
        let Ok(id) = parent.identity() else { return false; };
        let contains = |members: &[PathBuf]| members.iter().any(|p| same_path(p, path));
        if let Some(entry) = self.libraries.get(&id) {
            if entry.watch.unchanged() { return contains(&entry.members); }
        }
        self.libraries.remove(&id);
        let watch = LibraryWatch::start(&parent.path); // Arm before reading any peer.
        let Some(members) = inventory() else { return false; };
        let eligible = contains(&members);
        if let Some(watch) = watch {
            if !watch.unchanged() { return false; } // Changed while reading: trust no partial snapshot.
            self.libraries.insert(id, LibraryEvidence { watch, members });
        }
        // Without a watch the fresh scan is usable once, never cached.
        eligible
    }
}

fn live_eligible(path: &Path, origin: Origin, policy: &CompactPolicy, excluded: &[PathBuf], stop: &AtomicBool, batch: &mut CompressionBatch) -> bool {
    if policy.configured_at_or_below(path) { return false; }
    if origin != Origin::Other { return true; } // Steam uses the same layout test as discovery.
    path.parent().filter(|p| p.parent().is_some()).is_some_and(|parent| {
        let Ok(parent) = sprawl_core::local::directory(parent) else { return false; };
        batch.is_member(&parent, path, excluded, || {
            let progress = sprawl_core::Progress::with_exclusions(excluded);
            let tree = sprawl_core::walk::inventory_until(&parent.path, &progress, &|| stop.load(Ordering::Relaxed)).ok()?;
            if tree.errors != 0 { return None; }
            detected_members(&tree, policy, stop)
        })
    })
}

fn uncompress_scope(path: &Path, keep: &[PathBuf], excluded: &[PathBuf], policy: &CompactPolicy) -> Result<(), String> {
    let _root = sprawl_core::local::directory(path).map_err(|e| e.to_string())?;
    let tree = sprawl_core::walk::walk(path, &sprawl_core::Progress::new()).map_err(|e| e.to_string())?;
    if tree.errors != 0 { return Err("The folder could not be read completely; nothing was uncompressed.".into()); }
    let tails = policy.never.tail_matcher();
    let mut work = Vec::new();
    for (n, node) in tree.nodes.iter().enumerate() {
        let p = tree.path_of(n);
        if crate::exclusions::intersects(&p, excluded) || keep.iter().any(|k| within(&p, k) || within(k, &p))
            || unsafe_compression_attributes(node) || live_blocked_child(&p, &tails) {
            return Err("A protected, kept, excluded, cloud or linked item is inside this scope; nothing was uncompressed.".into());
        }
        // WOF files can appear as ordinary Archive files through the filter. Every plain
        // file in this explicitly approved scope is therefore an exact uncompress target.
        if !node.is_dir || node.is_compressed() { work.push((p, node.is_dir)); }
    }
    // Files first, then directory compression defaults; no recursive switch or wildcard is
    // passed to compact. Every exact argument stays pinned for the native operation.
    work.sort_by_key(|(p, dir)| (*dir, std::cmp::Reverse(p.components().count())));
    let mut offset = 0;
    while offset < work.len() {
        let dir = work[offset].1;
        let mut end = offset;
        let mut chars = 0;
        while end < work.len() && work[end].1 == dir && end - offset < 64 {
            let next = work[end].0.as_os_str().len() + 8;
            if next > 4000 { return Err("A path is too long to uncompress safely in a command batch.".into()); }
            if chars + next > 4000 { break; }
            chars += next; end += 1;
        }
        let pins: Vec<_> = work[offset..end].iter().map(|(p, expected_dir)| -> Result<_, String> {
            let pinned = sprawl_core::local::storage(p).map_err(|e| e.to_string())?;
            if pinned.metadata().map_err(|e| e.to_string())?.is_dir() != *expected_dir { return Err("A file became a folder or the reverse; it was left alone.".into()); }
            if !expected_dir && pinned.file_links().map_err(|e| e.to_string())? != 1 {
                return Err("A file has another hard-link name; it was left alone.".into());
            }
            Ok(pinned)
        }).collect::<Result<_, _>>()?;
        let result = crate::clean::uncompress_batch(pins, dir);
        if result.stopped || !result.problems.is_empty() || result.more_problems != 0 { return Err(result.problems.join("; ")); }
        offset = end;
    }
    let measured = sprawl_core::walk::walk(path, &sprawl_core::Progress::new()).map_err(|e| e.to_string())?;
    if measured.errors != 0 || measured.nodes.iter().any(|n| n.is_compressed()) { return Err("Some files remain compressed or could not be checked; the record was kept.".into()); }
    for (n, node) in measured.nodes.iter().enumerate().filter(|(_, node)| !node.is_dir) {
        let pinned = sprawl_core::local::storage(&measured.path_of(n)).map_err(|e| e.to_string())?;
        if node.is_compressed() || pinned.externally_backed().map_err(|e| e.to_string())? {
            return Err("Some WOF-backed files remain compressed; the record was kept.".into());
        }
    }
    Ok(())
}

/// Identity and eligibility of the exact directory shown in review, not just its spelling.
#[derive(Clone, Debug)]
pub struct ReviewedCompression { path: PathBuf, command: String, id: (u64, u128), origin: Origin }

impl ReviewedCompression {
    pub fn matches(&self, path: &Path, command: &str) -> bool { self.path == path && self.command == command }
}

pub fn review_compression(path: &Path, command: &str) -> Result<ReviewedCompression, String> {
    review_compression_in(path, command, &CompactPolicy::current())
}

fn review_compression_in(path: &Path, command: &str, policy: &CompactPolicy) -> Result<ReviewedCompression, String> {
    if compress_alg(command).is_none() || !command.ends_with(&ps_quote(&format!("{}\\*", path.display()))) {
        return Err("not a compression command for this folder".into());
    }
    let root = sprawl_core::local::directory(path).map_err(|e| e.to_string())?;
    let origin = policy.scope(&root.path)?.origin;
    let id = root.identity().map_err(|e| format!("couldn't identify the directory ({e})"))?;
    Ok(ReviewedCompression { path: path.to_path_buf(), command: command.into(), id, origin })
}

/// Run the reviewed compress command, checking the scope again now rather than in the scan:
/// Sprawl compresses the files itself ([`sprawl_core::wof`], what compact.exe does, on several
/// threads). The folder is read again completely through handles it holds, and a folder that
/// can't be, or that holds a link, a cloud file or a skipped folder now, is left whole. Each
/// file is opened through its held folder; one with another name elsewhere (a hard link)
/// is left as it is, since its storage is shared with a file outside the folder. Kept items are
/// left too. `progress` gets how many bytes have been gone through, of the folder's total.
pub(crate) fn compress_scope(reviewed: &ReviewedCompression, spare: &crate::clean::Spare, stop: &AtomicBool, progress: &mut dyn FnMut(u64), batch: &mut CompressionBatch) -> Result<sprawl_core::wof::Done, String> {
    crate::require_write().map_err(|e| e.to_string())?;
    compress_scope_in(reviewed, spare, stop, progress, &CompactPolicy::at_dispatch()?, &crate::exclusions::for_external_cleanup()?, batch)
}

fn compress_scope_in(reviewed: &ReviewedCompression, spare: &crate::clean::Spare, stop: &AtomicBool, progress: &mut dyn FnMut(u64), policy: &CompactPolicy, excluded: &[PathBuf], batch: &mut CompressionBatch) -> Result<sprawl_core::wof::Done, String> {
    let path = &reviewed.path;
    let alg = compress_alg(&reviewed.command).and_then(sprawl_core::wof::Alg::parse).ok_or("not one of Sprawl's compress commands")?;
    // a folder to skip in it or above it is known before anything is read: nothing is listed
    if crate::exclusions::intersects(path, excluded) {
        return Err(format!("{} holds or is in a folder you skip, so nothing was compressed", path.display()));
    }
    if stop.load(Ordering::Relaxed) {
        return Ok(sprawl_core::wof::Done { stopped: true, ..Default::default() });
    }
    let root = sprawl_core::local::directory(path).map_err(|e| e.to_string())?;
    let path = &root.path;
    if root.identity().map_err(|e| e.to_string())? != reviewed.id {
        return Err("the folder is not the one reviewed; scan again".into());
    }
    if crate::exclusions::intersects(path, excluded) {
        return Err(format!("{} holds or is in a folder you skip, so nothing was compressed", path.display()));
    }
    let origin = policy.scope(path)?.origin;
    let eligible = origin == reviewed.origin && live_eligible(path, origin, policy, excluded, stop, batch);
    if stop.load(Ordering::Relaxed) {
        return Ok(sprawl_core::wof::Done { stopped: true, ..Default::default() });
    }
    if !eligible {
        return Err("the folder no longer qualifies as the program or library reviewed".into());
    }
    let tails = policy.never.tail_matcher();
    let check = |p: &Path, _dir: bool| -> Result<bool, String> {
        if crate::exclusions::intersects(p, excluded) {
            return Err(format!("{} is a folder you skip, so nothing was compressed", p.display()));
        }
        if live_blocked_child(p, &tails) {
            return Err(format!("{} is protected, so nothing was compressed", p.display()));
        }
        Ok(spare.keeps(p))
    };
    let plan = match sprawl_core::wof::plan(path, &check, stop) {
        Err(_) if stop.load(Ordering::Relaxed) => return Ok(sprawl_core::wof::Done { stopped: true, ..Default::default() }),
        r => r?,
    };
    if origin == Origin::Other && (!plan.has_executable || plan.logical_bytes < LIBRARY_CHILD) {
        return Err("the folder no longer has the program files required for a detected library".into());
    }
    // The plan can take a while. Invalidate and recheck if peers changed while it was built.
    if origin == Origin::Other && !live_eligible(path, origin, policy, excluded, stop, batch) {
        if stop.load(Ordering::Relaxed) { return Ok(sprawl_core::wof::Done { stopped: true, ..Default::default() }); }
        return Err("the folder no longer qualifies as the program or library reviewed".into());
    }
    Ok(sprawl_core::wof::compress(&plan, alg, stop, progress))
}

/// The command is one of Sprawl's own compress commands.
pub fn is_compress(a: &Action) -> bool {
    matches!(a, Action::Tool { command } if command.starts_with(COMPRESS))
}

/// F3: the rules whose disk file Sprawl can shrink: WSL's distributions and Docker Desktop's
/// disk. Other virtual disks (Hyper-V, VirtualBox, VMware) stay information only.
pub const SHRINK_RULES: [&str; 2] = ["vm.wsl-disk", "vm.docker-disk"];

/// How every shrink command Sprawl writes begins: a line that names it, so the run is told apart
/// (measured by what the file lost on disk, never recorded as compressed).
pub const SHRINK: &str = "# Sprawl: shrink a virtual disk file (nothing inside it is deleted)\n";

/// How the follow-up to a stopped shrink begins.
pub const DETACH: &str = "# Sprawl: detach a virtual disk file a stopped shrink left attached\n";

/// After a shrink was stopped (or hit the time limit) diskpart may have been ended between
/// attaching the file and detaching it, and an attachment outlives diskpart: WSL or Docker then
/// can't start. This detaches it (no error when it wasn't attached) and fails when the file is
/// still held.
pub fn detach_command(path: &Path) -> String {
    let Some(sys) = crate::clean::system_dir() else { return format!("{DETACH}throw 'The Windows system directory could not be verified.'"); };
    if path.to_string_lossy().contains(['"', '\r', '\n', '\0']) { return format!("{DETACH}throw 'The virtual disk path cannot be passed safely to diskpart.'"); }
    let q = ps_quote(&path.display().to_string());
    let diskpart = ps_quote(&sys.join("diskpart.exe").display().to_string());
    format!(
        "{DETACH}$p = {q}\n\
         \"select vdisk file=`\"$p`\"`ndetach vdisk noerr`nexit\" | & {diskpart} | Out-Null\n\
         Start-Sleep -Seconds 1\n\
         try {{ [System.IO.File]::Open($p, 'Open', 'Read', 'None').Close() }} catch {{ throw 'The disk file is still in use.' }}"
    )
}

/// The command is one of Sprawl's own shrink commands.
pub fn is_shrink(a: &Action) -> bool {
    matches!(a, Action::Tool { command } if command.starts_with(SHRINK))
}

/// The finding shrinks its disk file in place.
pub fn shrinks(f: &Finding) -> bool {
    f.action.as_ref().is_some_and(is_shrink)
}

/// The action changes how its files are stored, not what they hold: a compress or a shrink.
/// Its win is what the path lost on disk.
pub fn in_place(a: &Action) -> bool {
    is_compress(a) || is_shrink(a)
}

/// The PowerShell that shrinks the disk file at `path`: Docker Desktop must be closed (for its
/// disk), every WSL distribution is shut down and checked to be gone, then Windows' diskpart
/// attaches the file read-only, compacts it and detaches it. diskpart is on every edition (the
/// Hyper-V `Optimize-VHD` isn't on Home). Nothing is started again afterwards.
pub fn shrink_command(path: &Path, docker: bool) -> String {
    let Some(sys) = crate::clean::system_dir() else { return format!("{SHRINK}throw 'The Windows system directory could not be verified.'"); };
    if path.to_string_lossy().contains(['"', '\r', '\n', '\0']) { return format!("{SHRINK}throw 'The virtual disk path cannot be passed safely to diskpart.'"); }
    let q = ps_quote(&path.display().to_string());
    let docker_check = if docker {
        "if (Get-Process 'Docker Desktop', 'com.docker.backend' -ErrorAction SilentlyContinue) { throw 'Docker Desktop is running: quit it first (its icon by the clock > Quit Docker Desktop).' }\n"
    } else {
        ""
    };
    let wsl = format!("& {}", ps_quote(&sys.join("wsl.exe").display().to_string()));
    let diskpart = ps_quote(&sys.join("diskpart.exe").display().to_string());
    format!(
        "{SHRINK}{docker_check}{wsl} --shutdown\n\
         Start-Sleep -Seconds 3\n\
         if (Get-Process vmmem, vmmemWSL -ErrorAction SilentlyContinue) {{ throw 'WSL is still running after wsl --shutdown: nothing was changed.' }}\n\
         $p = {q}\n\
         $a = (Get-Item -LiteralPath $p).Length\n\
         \"select vdisk file=`\"$p`\"`nattach vdisk readonly`ncompact vdisk`ndetach vdisk`nexit\" | & {diskpart}\n\
         $b = (Get-Item -LiteralPath $p).Length\n\
         \"Before: $a bytes. After: $b bytes.\"\n\
         if ($b -ge $a) {{ throw 'The disk file is no smaller. diskpart says why in the lines above (in use, or nothing to give back).' }}"
    )
}

/// Turn the WSL and Docker disk tips (built-in rules only) into a shrink Sprawl runs when
/// ticked. How much it gives back is unknown until tried, so its gain is 0 and it is never
/// ticked by Sprawl.
pub fn shrinkable(f: &mut Finding) {
    let id = match &f.source {
        Source::Rule(id) if SHRINK_RULES.contains(&id.as_str()) => id.clone(),
        _ => return,
    };
    if !(f.builtin && f.trusted) || f.remedy != Remedy::Info {
        return;
    }
    let docker = id == "vm.docker-disk";
    f.remedy = Remedy::Command;
    f.risk = Risk::Amber;
    f.gain = Some(0);
    f.evidence = vec![
        if docker {
            "Docker's images, containers and volumes. The file grows but never shrinks by itself. To make room inside it first, run 'docker system prune' (add '-a --volumes' to remove more).".into()
        } else {
            "The disk of a Linux distribution. It grows but never shrinks by itself: delete files inside Linux first, then shrink it here.".into()
        },
        format!(
            "Tick it and Clean up, and Sprawl {}shuts down WSL (everything running in Linux stops), then compacts the file with Windows' diskpart, as administrator. Nothing inside it is deleted, and nothing is started again.",
            if docker { "checks that Docker Desktop is closed, " } else { "" }
        ),
        "How much it gives back is known only after: the space files you deleted inside used to take.".into(),
        "Never delete the file itself: that deletes the whole distribution.".into(),
    ];
    // Sprawl's own command from the built-in rules; it changes the file's size, never what it
    // holds, so the never-list (disk images are on it) doesn't veto it, as for DISM
    f.allow_protected = true;
    f.set_action(Action::Tool { command: shrink_command(&f.path.clone(), docker) }, true);
}

/// The finding compresses its folder in place (nothing is deleted).
pub fn compresses(f: &Finding) -> bool {
    f.action.as_ref().is_some_and(is_compress)
}

/// What everything under `path` takes on disk, compressed files at their compressed size;
/// links are not followed. None when `path` can't be read.
pub fn on_disk(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCompressedFileSizeW(name: *const u16, high: *mut u32) -> u32;
    }
    let size = |p: &Path| -> Option<u64> {
        let w: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut high = 0u32;
        // SAFETY: a NUL-terminated wide path and a valid out pointer
        unsafe { windows_sys::Win32::Foundation::SetLastError(0); }
        let low = unsafe { GetCompressedFileSizeW(w.as_ptr(), &mut high) };
        if low == u32::MAX && unsafe { windows_sys::Win32::Foundation::GetLastError() } != 0 { return None; }
        Some((high as u64) << 32 | low as u64)
    };
    let root = sprawl_core::local::storage(path).ok()?;
    let excluded = crate::exclusions::configured();
    if crate::exclusions::intersects(&root.path, &excluded) { return None; }
    let top = root.metadata().ok()?;
    if !top.is_dir() {
        if root.file_links().ok()? != 1 { return None; }
        return size(&root.path);
    }
    let (mut total, mut visited) = (0u64, 0usize);
    let start = std::time::Instant::now();
    let mut stack = vec![root.path.clone()];
    // Measuring only: the root stays pinned, entries come from the listings, and only each
    // file itself is opened for its link count (1.0 pinned every ancestor of every file).
    use std::os::windows::fs::MetadataExt;
    while let Some(d) = stack.pop() {
        let rd = std::fs::read_dir(&d).ok()?;
        for e in rd {
            visited += 1;
            if visited > 1_000_000 || start.elapsed().as_secs() >= 30 { return None; }
            let e = e.ok()?;
            let m = e.metadata().ok()?;
            // A cloud placeholder, an offline file or a link folder can't be measured here; a
            // file is measured on itself (a visible WOF file counts, at its compressed size).
            if m.file_attributes() & crate::dupes::NOT_LOCAL != 0 { return None; }
            if m.is_dir() {
                if m.file_attributes() & sprawl_core::attr::REPARSE_POINT != 0 { return None; }
                stack.push(e.path());
            } else {
                total = total.checked_add(sprawl_core::local::file_on_disk(&e.path()).ok()?)?;
            }
        }
    }
    Some(total)
}

fn info(tree: &Tree, n: usize, id: &str, title: String, evidence: Vec<String>, bytes: u64, risk: Risk, remedy: Remedy) -> Finding {
    Finding {
        path: tree.path_of(n),
        owner: None,
        bytes,
        risk,
        remedy,
        command: None,
        title,
        evidence,
        confidence: 0.7,
        source: Source::Rule(format!("remedy.{id}")),
        allow_protected: false,
        trusted: true,
        builtin: true,
        on_path: false,
        action: None,
        admin: false,
        carved: Vec::new(),
        gain: None,
        forced_from: None,
    }
}

/// The share LZX compression usually saves, by file type (0 for formats already compressed).
fn lzx_ratio(ext: &str) -> f64 {
    match ext {
        "exe" | "dll" | "sys" | "ocx" | "node" | "pyd" | "so" => 0.45,
        "pdb" | "lib" | "obj" | "a" => 0.6,
        "json" | "xml" | "txt" | "js" | "css" | "html" | "htm" | "py" | "lua" | "ini" | "cfg" | "log" | "md" | "ts" | "map" => 0.65,
        // extension() says "(none)" for a file without one
        "dat" | "bin" | "pak" | "" | "(none)" => 0.15,
        _ => 0.0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin { Program, Steam, Configured, Other }

struct CompactScope {
    origin: Origin,
    program_files: bool,
}

struct CompactPolicy {
    folders: Vec<PathBuf>,
    home: Option<PathBuf>,
    program_data: Option<PathBuf>,
    program_files: Vec<PathBuf>,
    never: Never,
    #[cfg(test)]
    allow_fixture_home: bool,
}

fn same_path(a: &Path, b: &Path) -> bool { within(a, b) && within(b, a) }

fn library_path(path: &Path) -> bool {
    path.is_absolute() && path.file_name().is_some()
}

impl CompactPolicy {
    fn at_dispatch() -> Result<Self, String> {
        let mut policy = Self::current();
        policy.folders = crate::program_folders::parse(&crate::exclusions::checked_setting(crate::program_folders::SETTING)?)?;
        Ok(policy)
    }

    fn current() -> Self {
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
        let never = Never::for_compression(home.as_deref().and_then(Path::to_str));
        Self {
            folders: crate::program_folders::configured(),
            home,
            program_data: std::env::var_os("ProgramData").map(PathBuf::from),
            program_files: ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"].iter()
                .filter_map(std::env::var_os).map(PathBuf::from).filter(|p| library_path(p)).collect(),
            never,
            #[cfg(test)]
            allow_fixture_home: false,
        }
    }

    fn configured_parent(&self, path: &Path) -> bool {
        path.parent().is_some_and(|p| crate::program_folders::allowed(p, self.home.as_deref(), &self.never)
            && self.folders.iter().any(|f| same_path(p, f)))
    }

    fn configured_at_or_below(&self, path: &Path) -> bool {
        // A named library is never a target. Also don't take an enclosing folder as a target
        // in place of the individual programs in an explicitly named library farther down.
        self.folders.iter().any(|f| within(f, path))
    }

    fn scope(&self, path: &Path) -> Result<CompactScope, String> {
        self.scope_with_never(path, &self.never)
    }

    fn scope_with_never(&self, path: &Path, never: &Never) -> Result<CompactScope, String> {
        if !never.complete() { return Err("The protected Windows folders couldn't be checked.".into()); }
        let raw = path.to_str().ok_or("This folder name can't be checked.")?;
        let segs = segments(raw.strip_prefix(r"\\?\").unwrap_or(raw));
        let [_, top, ..] = segs.as_slice() else { return Err("A drive root can't be compressed or uncompressed.".into()); };
        let (top, leaf) = (top.as_str(), segs.last().unwrap().as_str());
        let parent = path.parent().ok_or("This folder has no parent.")?;
        let configured = self.configured_parent(path);
        let allow_home_child = configured;
        #[cfg(test)]
        let allow_home_child = allow_home_child || self.allow_fixture_home;
        if !path.is_absolute() || never.is_user_data(raw) || matches!(top, "windows" | "programdata")
            || self.program_data.as_deref().is_some_and(|p| within(path, p) || within(p, path))
            || segs.iter().any(|s| s == "windowsapps")
            || crate::finding::other_account(path, self.home.as_deref())
            || self.home.as_deref().is_some_and(|h| within(h, path) || (within(path, h) && !allow_home_child))
        {
            return Err("This is a protected Windows or user folder.".into());
        }
        let steam = segs.iter().position(|s| s == "steamapps");
        if steam.is_some_and(|at| segs.len() != at + 3 || segs[at + 1] != "common") {
            return Err("Only individual programs in steamapps\\common can be compressed or uncompressed.".into());
        }
        let standard_pf = matches!(top, "program files" | "program files (x86)");
        let program_files = standard_pf || self.program_files.iter().any(|p| within(path, p));
        let program = (standard_pf && segs.len() == 3) || self.program_files.iter().any(|p| same_path(parent, p));
        if (program_files && !program && steam.is_none())
            || (program && matches!(leaf, "common files" | "steam"))
        {
            return Err("Only individual Program Files products can be compressed or uncompressed.".into());
        }
        if let Some(hit) = never.compression_hit(raw, program_files) {
            return Err(format!("This scope is protected by {hit}."));
        }
        let tails = never.tail_matcher();
        if parent.ancestors().any(|p| tails.at_path(p).is_some()) {
            return Err("This folder is inside a protected item.".into());
        }
        let origin = if steam.is_some() { Origin::Steam } else if program { Origin::Program }
            else if configured { Origin::Configured } else { Origin::Other };
        Ok(CompactScope { origin, program_files })
    }
}

/// Safety checks may revalidate an existing finding after the settings or scan have changed.
pub(crate) fn compression_scope_allowed(path: &Path, never: &Never) -> Result<(), String> {
    let path = sprawl_core::local::lexical(path).map_err(|e| e.to_string())?;
    CompactPolicy::current().scope_with_never(&path, never).map(|_| ())
}

fn blocked_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("windowsapps") || name.eq_ignore_ascii_case("steamapps")
}

pub(crate) fn live_blocked_child(path: &Path, tails: &TailMatcher<'_>) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_none_or(blocked_name) || tails.at_path(path).is_some()
}

pub(crate) fn unsafe_compression_attributes(node: &sprawl_core::Node) -> bool {
    node.attributes & crate::dupes::NOT_LOCAL != 0
        || node.is_reparse_point() && (node.is_dir || node.attributes & sprawl_core::attr::WOF == 0)
}

#[derive(Clone, Copy, Default)]
struct CompactSummary {
    saving: f64,
    has_exe: bool,
    blocked: bool,
    library: bool,
}

fn compact_summaries(tree: &Tree, never: &Never, stop: Option<&AtomicBool>) -> Option<Vec<CompactSummary>> {
    let tails = never.tail_matcher();
    let mut summary = vec![CompactSummary::default(); tree.nodes.len()];
    // Children always have higher indices. Each file's estimate and executable evidence
    // are read once, before deciding which of the directory scopes to show.
    for (n, node) in tree.nodes.iter().enumerate().rev() {
        if stop.is_some_and(|s| s.load(Ordering::Relaxed)) { return None; }
        let bad_attributes = unsafe_compression_attributes(node);
        let mut here = CompactSummary {
            blocked: bad_attributes || blocked_name(&node.name)
                || (tails.maybe(&node.name) && tails.at_path(&tree.path_of(n)).is_some()),
            ..Default::default()
        };
        if node.is_dir && !bad_attributes {
            let (mut large, mut with_exe) = (0u64, 0u64);
            for &c in &node.children {
                let child = &tree.nodes[c];
                let sub = summary[c];
                here.has_exe |= sub.has_exe;
                here.blocked |= sub.blocked;
                here.saving += sub.saving;
                if child.is_dir && child.size >= LIBRARY_CHILD {
                    large += 1;
                    with_exe += u64::from(sub.has_exe);
                }
            }
            here.library = with_exe >= 2 && with_exe * 3 >= large * 2;
        } else if !node.is_dir && !bad_attributes {
            here.has_exe = node.name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case("exe"));
            if !node.is_compressed() && node.attributes & REPARSE == 0 && node.size >= 4096 {
                here.saving = node.size as f64 * lzx_ratio(&node.extension());
            }
        }
        summary[n] = here;
    }
    Some(summary)
}

fn detected_members(tree: &Tree, policy: &CompactPolicy, stop: &AtomicBool) -> Option<Vec<PathBuf>> {
    if tree.nodes.is_empty() || !library_path(&tree.root_path) || unsafe_compression_attributes(&tree.nodes[ROOT]) {
        return Some(Vec::new());
    }
    let summary = compact_summaries(tree, &policy.never, Some(stop))?;
    if !summary[ROOT].library { return Some(Vec::new()); }
    let mut members = Vec::new();
    for &c in &tree.nodes[ROOT].children {
        if stop.load(Ordering::Relaxed) { return None; }
        let node = &tree.nodes[c];
        if node.is_dir && node.size >= LIBRARY_CHILD && summary[c].has_exe && !summary[c].blocked {
            members.push(tree.path_of(c));
        }
    }
    Some(members)
}

/// Worthwhile, non-overlapping compression scopes. Uses scan evidence, the in-memory
/// Program folders list, and Windows' current personal-folder locations.
pub fn compact(tree: &Tree) -> Vec<Finding> {
    compact_in(tree, &CompactPolicy::current(), |p| sprawl_core::local::lexical(p).is_ok())
}

fn compact_in(tree: &Tree, policy: &CompactPolicy, is_local: impl Fn(&Path) -> bool) -> Vec<Finding> {
    if tree.nodes.is_empty() || tree.method == sprawl_core::ScanMethod::Csv || !policy.never.complete() { return Vec::new(); }
    let roots: &[usize] = if tree.nodes[ROOT].name == sprawl_core::MULTIPLE { &tree.nodes[ROOT].children } else { &[ROOT] };
    let mut stack: Vec<_> = roots.iter().copied().filter(|&n| {
        let p = tree.path_of(n);
        p.is_absolute() && is_local(&p)
    }).map(|n| (n, false)).collect();
    if stack.is_empty() { return Vec::new(); }
    let Some(summary) = compact_summaries(tree, &policy.never, None) else { return Vec::new(); };
    let mut out = Vec::new();
    while let Some((n, under_library)) = stack.pop() {
        let node = &tree.nodes[n];
        if !node.is_dir || node.size < 2 * GB || unsafe_compression_attributes(node) { continue; }
        let path = tree.path_of(n);
        let here = summary[n];
        let candidate = policy.scope(&path).ok().filter(|scope| {
            !here.blocked && !policy.configured_at_or_below(&path)
                && (scope.origin != Origin::Other || (under_library && here.has_exe))
        });
        let Some(scope) = candidate else {
            // A drive root isn't a library. The first qualifying library on the way down
            // takes its direct programs; a program's own layout cannot create extra cards.
            let library = library_path(&path) && here.library;
            stack.extend(node.children.iter().copied()
                .filter(|&c| tree.nodes[c].is_dir && tree.nodes[c].size >= 2 * GB).map(|c| (c, library)));
            continue;
        };
        if node.allocated < node.size - node.size / 10 { continue; }
        let (alg, share) = if scope.program_files { ("xpress8k", 0.75) } else { ("xpress16k", 0.85) };
        let saving = (here.saving * share) as u64;
        if saving < GB { continue; }
        let mut f = info(
            tree,
            n,
            "compact",
            format!("Compress {}", tree.nodes[n].name),
            vec![
                format!("Windows can store these program files compressed ({}): about {} of {} saved, the program runs as before. Nothing is deleted.", alg.to_uppercase(), format_size(saving), format_size(node.size)),
                "Compress it now (or tick it and review), and Sprawl compresses its files with Windows' own compression, showing how far it is. Files in use are skipped: close the program first.".into(),
                if scope.origin == Origin::Steam { "An update or 'verify files' in Steam writes some files back uncompressed; compress it again after big updates. A few games with anti-cheat check their files: if one complains, uncompress it.".into() } else { "An update writes changed files back uncompressed; compress it again after big updates.".into() },
                format!("Undo with: compact /u /s /a /i /exe {}", ps_quote(&format!("{}\\*", path.display()))),
            ],
            // the folder's own size (what the safety checks compare); the saving is its gain
            node.size,
            Risk::Green,
            Remedy::Command,
        );
        f.gain = Some(saving);
        let admin = policy.program_files.iter().any(|p| within(&path, p));
        // Single-quoted: a folder name can't run anything.
        f.set_action(Action::Tool { command: format!("{COMPRESS}{alg} {}", ps_quote(&format!("{}\\*", path.display()))) }, admin);
        out.push(f);
    }
    out.sort_by(|a, b| b.expected_gain().cmp(&a.expected_gain()));
    out.truncate(10);
    out
}

/// OneDrive folders with a lot kept on this PC that hasn't changed in 90 days: OneDrive can keep
/// it online only (downloaded again when opened).
pub fn onedrive(tree: &Tree) -> Vec<Finding> {
    let now = std::time::SystemTime::now();
    let mut out = Vec::new();
    let mut stack = vec![ROOT];
    while let Some(n) = stack.pop() {
        let node = &tree.nodes[n];
        let is_onedrive = n != ROOT && node.name.to_lowercase().starts_with("onedrive");
        // OneDrive's own folder may carry a cloud reparse mark: it is still looked at (only
        // what the scan holds is counted, and nothing is read from disk here)
        if !node.is_dir || (n != ROOT && node.is_reparse_point() && !is_onedrive) {
            continue;
        }
        if !is_onedrive {
            if segments(&tree.path_of(n).to_string_lossy()).len() < 4 {
                stack.extend(node.children.iter().copied().filter(|&c| tree.nodes[c].is_dir));
            }
            continue;
        }
        let (mut local, mut pinned) = (0u64, 0u64);
        let mut s2 = vec![n];
        while let Some(k) = s2.pop() {
            let x = &tree.nodes[k];
            if x.is_dir {
                s2.extend_from_slice(&x.children);
                continue;
            }
            let online_only = x.attributes & (RECALL_ON_DATA_ACCESS | RECALL_ON_OPEN | OFFLINE) != 0;
            let old = x.modified.and_then(|m| now.duration_since(m).ok()).is_some_and(|d| d.as_secs() > 90 * 86_400);
            if !online_only && old {
                local += x.size;
                if x.attributes & PINNED != 0 {
                    pinned += x.size;
                }
            }
        }
        if local < 2 * GB {
            continue;
        }
        let path = tree.path_of(n);
        let mut ev = vec![
            format!("{} of files untouched for 3 months are stored on this PC as well as in OneDrive.", format_size(local)),
            "Right-click the folder in Explorer > Free up space: they stay listed and download again when opened. Needs the internet then.".into(),
            format!("Or in PowerShell: attrib +U -P {} /s /d", ps_quote(&format!("{}\\*", path.display()))),
        ];
        if pinned > 0 {
            ev.push(format!("{} of it is set to 'Always keep on this device'; Free up space undoes that.", format_size(pinned)));
        }
        let mut f = info(tree, n, "onedrive", format!("{}: {} can be online only", node.name, format_size(local)), ev, node.size, Risk::Amber, Remedy::Info);
        f.gain = Some(local);
        out.push(f);
    }
    out
}

/// Big things that could live on another drive with room, when this one is filling up.
pub fn moves(tree: &Tree, drives: &[(char, u64, u64)]) -> Vec<Finding> {
    let root = tree.path_of(ROOT);
    let s = root.to_string_lossy();
    let Some(letter) = s.chars().next().filter(|c| c.is_ascii_alphabetic() && s.get(1..2) == Some(":")) else { return Vec::new() };
    let letter = letter.to_ascii_uppercase();
    let Some(&(_, total, free)) = drives.iter().find(|d| d.0 == letter) else { return Vec::new() };
    // only when this drive is getting full, and another has real room
    if total == 0 || free > total / 6 {
        return Vec::new();
    }
    let Some(&(other, _, other_free)) = drives.iter().filter(|d| d.0 != letter).max_by_key(|d| d.2) else { return Vec::new() };
    if other_free < 50 * GB {
        return Vec::new();
    }
    let room = format!("{other}: has {} free", format_size(other_free));
    let mut out = Vec::new();
    let mut add = |n: usize, id: &str, title: String, how: &str| {
        let bytes = tree.nodes[n].size;
        if bytes >= 10 * GB && bytes < other_free {
            out.push(info(tree, n, id, title, vec![room.clone(), how.to_string()], bytes, Risk::Amber, Remedy::Move));
        }
    };
    let find = |p: &str| crate::find_path(tree, Path::new(p));
    let profile = std::env::var("USERPROFILE").unwrap_or_default();
    for (sub, label) in [("Videos", "Videos"), ("Downloads", "Downloads"), ("Pictures", "Pictures"), ("Music", "Music")] {
        if let Some(n) = find(&format!("{profile}\\{sub}")) {
            let how = format!("Create {other}:\\{label} first, then right-click {label} > Properties > Location, choose that new folder and Move. Never choose {other}:\\ itself: the whole drive would become your {label} folder.");
            add(n, "move-known-folder", format!("Move {label} to {other}:"), &how);
        }
    }
    // Steam libraries on this drive
    let mut stack = vec![ROOT];
    while let Some(n) = stack.pop() {
        let node = &tree.nodes[n];
        if !node.is_dir || (n != ROOT && node.is_reparse_point()) {
            continue;
        }
        if node.name.eq_ignore_ascii_case("steamapps") {
            if let Some(c) = node.children.iter().copied().find(|&c| tree.nodes[c].name.eq_ignore_ascii_case("common")) {
                add(c, "move-steam", format!("Move Steam games to {other}:"), "Steam > Settings > Storage: add a library on the other drive, then select games and Move. Steam copies them and keeps them working.");
            }
            continue;
        }
        if segments(&tree.path_of(n).to_string_lossy()).len() < 5 {
            stack.extend(node.children.iter().copied().filter(|&c| tree.nodes[c].is_dir));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compress_takes_another_algorithm() {
        let mut f = Finding {
            path: r"C:\Games\X".into(), owner: None, bytes: 10_000, risk: Risk::Green,
            remedy: Remedy::Command, command: None, title: "Compress X".into(), evidence: vec![],
            confidence: 0.7, source: Source::Rule("remedy.compact".into()), allow_protected: true,
            trusted: true, builtin: true, on_path: false, action: None, admin: false,
            carved: vec![], gain: Some(7_500), forced_from: None,
        };
        f.set_action(Action::Tool { command: format!(r"{COMPRESS}xpress8k 'C:\Games\X\*'") }, false);
        let g = with_compress_alg(&f, "LZX").unwrap();
        assert_eq!(g.gain, Some(10_000));
        let Some((Action::Tool { command }, false)) = crate::clean::action(&g) else { panic!() };
        assert_eq!(command, format!(r"{COMPRESS}lzx 'C:\Games\X\*'"));
        assert_eq!(compress_alg(&command), Some("lzx"));
        assert!(with_compress_alg(&f, "zip").is_none());
        // not a compress: nothing to change
        let mut d = f.clone();
        d.set_action(Action::Tool { command: "Dism.exe /Online /Cleanup-Image /StartComponentCleanup".into() }, true);
        assert!(with_compress_alg(&d, "lzx").is_none());
    }

    #[test]
    fn ratios() {
        assert!(lzx_ratio("dll") > 0.3 && lzx_ratio("mp4") == 0.0 && lzx_ratio("zip") == 0.0 && lzx_ratio("(none)") > 0.0);
    }

    /// A built tree: OneDrive with old local files, online-only files and a pinned one.
    #[test]
    fn onedrive_counts_old_local_files() {
        use sprawl_core::{Node, ScanMethod};
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(200 * 86_400);
        let node = |name: &str, dir: bool, size: u64, attributes: u32, parent: Option<usize>, children: Vec<usize>| Node {
            name: name.into(),
            is_dir: dir,
            size,
            allocated: size,
            files: if dir { 0 } else { 1 },
            folders: 0,
            modified: Some(old),
            attributes,
            parent,
            children,
            id: 0,
        };
        let mut tree = Tree { volumes: Vec::new(),
            root_path: r"C:\Users\sam".into(),
            nodes: vec![
                node(r"C:\Users\sam", true, 0, 0, None, vec![1]),
                // the sync root marked as a cloud reparse point
                node("OneDrive", true, 0, 0x400, Some(0), vec![2, 3, 4]),
                node("photos.zip", false, 3 * GB, 0, Some(1), vec![]),
                node("video.mp4", false, 5 * GB, RECALL_ON_DATA_ACCESS, Some(1), vec![]),
                node("pinned.iso", false, GB, PINNED, Some(1), vec![]),
            ],
            errors: 0,
            method: ScanMethod::Walk,
        };
        tree.compute_totals();
        let f = onedrive(&tree);
        assert_eq!(f.len(), 1);
        assert!(f[0].title.contains("4.0 GB"), "local and old: 3 GB + the pinned 1 GB, not the online-only 5 GB: {}", f[0].title);
        assert!(f[0].evidence.iter().any(|e| e.contains("Always keep")));
        assert!(f[0].evidence.iter().any(|e| e.contains("attrib +U -P '")), "{:?}", f[0].evidence);
    }

    #[test]
    fn shown_commands_quote_paths() {
        for name in ["x$(calc)", "x`y", "x\u{201d}; calc"] {
            let q = ps_quote(&format!("D:\\Games\\{name}\\*"));
            assert!(q.starts_with('\'') && q.ends_with('\''), "{q}");
            assert!(!q[1..q.len() - 1].replace("''", "").contains('\''), "{q}");
        }
    }

    #[test]
    fn moves_only_when_full_and_room_elsewhere() {
        let tree = Tree { root_path: r"C:\Fixture".into(), nodes: vec![sprawl_core::Node {
            name: r"C:\Fixture".into(), is_dir: true, ..Default::default()
        }], ..Default::default() };
        // this scan's folder isn't on a drive that is nearly full: nothing
        assert!(moves(&tree, &[('C', 100 * GB, GB), ('D', 500 * GB, 400 * GB)]).is_empty());
    }
}

#[cfg(test)]
mod compress_tests {
    use super::*;

    fn policy(home: Option<&Path>, folders: &[&Path]) -> CompactPolicy {
        CompactPolicy {
            folders: folders.iter().map(|p| p.to_path_buf()).collect(),
            home: home.map(Path::to_path_buf),
            program_data: None,
            program_files: Vec::new(),
            never: Never::compression_fixture(),
            allow_fixture_home: false,
        }
    }

    fn fixture(root: &str, files: &[(&str, u64)]) -> Tree {
        use sprawl_core::Node;
        let mut tree = Tree {
            root_path: root.into(), method: sprawl_core::ScanMethod::Walk,
            nodes: vec![Node { name: root.into(), is_dir: true, ..Node::default() }],
            ..Tree::default()
        };
        for &(file, bytes) in files {
            let mut parts = file.split('\\').peekable();
            let mut dir = ROOT;
            while let Some(name) = parts.next() {
                if let Some(c) = tree.nodes[dir].children.iter().copied().find(|&c| tree.nodes[c].name == name) {
                    dir = c;
                    continue;
                }
                let n = tree.nodes.len();
                let is_dir = parts.peek().is_some();
                let size = if is_dir { 0 } else { bytes };
                tree.nodes.push(Node { name: name.into(), parent: Some(dir), is_dir, size, allocated: size,
                    files: u64::from(!is_dir), ..Node::default() });
                tree.nodes[dir].children.push(n);
                dir = n;
            }
        }
        tree.compute_totals();
        tree
    }

    fn offers(tree: &Tree, policy: &CompactPolicy) -> Vec<(PathBuf, String)> {
        compact_in(tree, policy, |_| true).into_iter().map(|f| {
            let alg = match f.action.unwrap() {
                Action::Tool { command } => compress_alg(&command).unwrap().to_owned(),
                _ => panic!("not compression"),
            };
            (f.path, alg)
        }).collect()
    }

    #[test]
    fn detected_library_offers_direct_exe_bearing_programs_only() {
        let tree = fixture(r"D:\", &[
            (r"Games\Cyberpunk 2077\bin\game.exe", 3 * GB),
            (r"Games\Cyberpunk 2077\tools\helper.EXE", 3 * GB),
            (r"Games\Assassin's Creed Shadows\bin\game.exe", 4 * GB),
            (r"Games\Data\asset.json", 5 * GB),
            (r"Games\SmallData\asset.json", LIBRARY_CHILD - 1),
            (r"Other\big.exe", 7 * GB),
        ]);
        let policy = policy(None, &[]);
        assert_eq!(offers(&tree, &policy), [
            (r"D:\Games\Cyberpunk 2077".into(), "xpress16k".into()),
            (r"D:\Games\Assassin's Creed Shadows".into(), "xpress16k".into()),
        ]);
        let library = tree.subtree(crate::find_path(&tree, Path::new(r"D:\Games")).unwrap());
        assert_eq!(offers(&library, &policy).len(), 2, "a scan rooted at the library works too");
        let members = detected_members(&library, &policy, &AtomicBool::new(false)).unwrap();
        assert!(members.iter().any(|p| same_path(p, Path::new(r"D:\Games\Cyberpunk 2077"))));
        assert!(!members.iter().any(|p| same_path(p, Path::new(r"D:\Games\Data"))));
    }

    #[test]
    fn detection_needs_two_exe_children_and_two_thirds_of_sizeable_children() {
        let policy = policy(None, &[]);
        let make = |second, fourth| fixture(r"D:\Games", &[
            (r"First\bin\game.exe", 4 * GB), (r"Second\game.exe", second),
            (r"Data\file.txt", LIBRARY_CHILD), (r"Fourth\file.txt", fourth),
        ]);
        assert_eq!(offers(&make(LIBRARY_CHILD, LIBRARY_CHILD - 1), &policy).len(), 1);
        assert!(offers(&make(LIBRARY_CHILD - 1, LIBRARY_CHILD - 1), &policy).is_empty());
        assert!(offers(&make(LIBRARY_CHILD, LIBRARY_CHILD), &policy).is_empty());
    }

    #[test]
    fn configured_library_needs_no_exe_and_checks_only_its_children() {
        let tree = fixture(r"D:\Collections", &[
            (r"Chosen\First\nested\data.txt", 5 * GB),
            (r"Chosen\Second\data.dll", 4 * GB),
            (r"Chosen\Small\data.txt", GB),
            (r"Chosen\NoGain\asset.zip", 10 * GB),
            (r"Other\Third\data.txt", 6 * GB),
        ]);
        let expected = [
            (r"D:\Collections\Chosen\First".into(), "xpress16k".into()),
            (r"D:\Collections\Chosen\Second".into(), "xpress16k".into()),
        ];
        let policy = policy(None, &[Path::new(r"d:\collections\CHOSEN\")]);
        assert_eq!(offers(&tree, &policy), expected);
        let mut tree = tree;
        let first = crate::find_path(&tree, Path::new(r"D:\Collections\Chosen\First")).unwrap();
        tree.nodes[first].allocated = tree.nodes[first].size - tree.nodes[first].size / 10 - 1;
        assert_eq!(offers(&tree, &policy), expected[1..]);
    }

    #[test]
    fn keeps_program_files_and_steam_conventions_without_exe_evidence() {
        let tree = fixture(r"D:\", &[
            (r"Program Files\Company\payload.dll", 4 * GB),
            (r"Program Files (x86)\Another\payload.dll", 4 * GB),
            (r"Program Files\Steam\steamapps\common\Old Game\asset.txt", 4 * GB),
            (r"SteamLibrary\steamapps\common\New Game\asset.txt", 4 * GB),
            (r"SteamLibrary\steamapps\downloading\Game\setup.exe", 4 * GB),
        ]);
        let policy = policy(None, &[Path::new(r"D:\SteamLibrary\steamapps\downloading")]);
        let got: std::collections::HashMap<_, _> = offers(&tree, &policy).into_iter().collect();
        assert_eq!(got.len(), 4, "{got:?}");
        for path in [r"D:\Program Files\Company", r"D:\Program Files (x86)\Another", r"D:\Program Files\Steam\steamapps\common\Old Game"] {
            assert_eq!(got.get(Path::new(path)).map(String::as_str), Some("xpress8k"));
        }
        assert_eq!(got.get(Path::new(r"D:\SteamLibrary\steamapps\common\New Game")).map(String::as_str), Some("xpress16k"));
    }

    #[test]
    fn protected_scopes_stay_blocked_even_when_configured() {
        let home = Path::new(r"C:\Users\sam");
        for (root, configured, file) in [
            (r"C:\Windows", r"C:\Windows", r"NotAnApp\payload.exe"),
            (r"C:\Windows\WinSxS", r"C:\Windows\WinSxS", r"NotAnApp\payload.exe"),
            (r"D:\ProgramData", r"D:\ProgramData", r"Vendor\payload.exe"),
            (r"D:\ProgramData\Vendor", r"D:\ProgramData\Vendor", r"Package\payload.exe"),
            (r"C:\Users\sam", r"C:\Users\sam", r"Programs\payload.exe"),
            (r"C:\Users\other\Programs", r"C:\Users\other\Programs", r"Product\payload.exe"),
            (r"D:\WindowsApps\Programs", r"D:\WindowsApps\Programs", r"Product\payload.exe"),
            (r"D:\Store", r"D:\Store", r"WindowsApps\payload.exe"),
            (r"D:\Steam", r"D:\Steam", r"steamapps\payload.exe"),
            (r"D:\", r"D:\", r"Unscoped\payload.exe"),
            (r"D:\Tools", r"D:\Tools", r"wallet.kdbx\payload.exe"),
            (r"D:\Boot\Tools", r"D:\Boot\Tools", r"Product\payload.exe"),
            (r"D:\Boxes\vault.kdbx\Tools", r"D:\Boxes\vault.kdbx\Tools", r"Product\payload.exe"),
        ] {
            let tree = fixture(root, &[(file, 4 * GB)]);
            assert!(offers(&tree, &policy(Some(home), &[Path::new(configured)])).is_empty(), "{root}\\{file}");
        }
    }

    #[test]
    fn only_an_explicit_library_below_this_profile_is_allowed() {
        let root = r"C:\Users\sam\Programs";
        let home = Path::new(r"C:\Users\sam");
        let tree = fixture(root, &[(r"One\file.exe", 4 * GB), (r"Two\data.exe", 3 * GB)]);
        assert!(offers(&tree, &policy(Some(home), &[])).is_empty());
        let tree = fixture(root, &[(r"One\file.txt", 4 * GB)]);
        assert_eq!(offers(&tree, &policy(Some(home), &[Path::new(root)])), [
            (r"C:\Users\sam\Programs\One".into(), "xpress16k".into()),
        ]);
        let tree = fixture(r"D:\Profiles\sam", &[(r"Library\One\file.txt", 4 * GB)]);
        let home = Path::new(r"D:\Profiles\sam");
        assert!(offers(&tree, &policy(Some(home), &[home])).is_empty());
        assert_eq!(offers(&tree, &policy(Some(home), &[Path::new(r"D:\Profiles\sam\Library")])).len(), 1);
    }

    #[test]
    fn never_list_inside_a_program_blocks_it_even_in_program_files() {
        let root = r"D:\Program Files";
        let tree = fixture(root, &[
            (r"VaultApp\bin\code.exe", 4 * GB), (r"VaultApp\bin\content.kdbx", 1),
            (r"LoginApp\bin\code.exe", 4 * GB), (r"LoginApp\Default\Cookies", 1),
            (r"CloudApp\bin\code.exe", 4 * GB), (r"CloudApp\WindowsApps\Package\payload.dll", 1),
            (r"SafeApp\bin\code.exe", 4 * GB),
        ]);
        let policy = policy(None, &[Path::new(root)]);
        assert_eq!(offers(&tree, &policy), [(r"D:\Program Files\SafeApp".into(), "xpress8k".into())]);
    }

    #[test]
    fn links_block_the_candidate_and_are_not_executable_evidence() {
        let root = r"D:\Chosen";
        let files = [(r"LinkedFolder\bin\code.exe", 4 * GB), (r"LinkedFile\bin\code.exe", 4 * GB), (r"Plain\bin\code.exe", 4 * GB)];
        let mut tree = fixture(root, &files);
        let index = |tree: &Tree, p: &str| crate::find_path(tree, &Path::new(root).join(p)).unwrap();
        for p in [r"LinkedFolder\bin", r"LinkedFile\bin\code.exe"] {
            let n = index(&tree, p);
            tree.nodes[n].attributes |= REPARSE;
        }
        assert!(offers(&tree, &policy(None, &[])).is_empty());
        let policy = policy(None, &[Path::new(root)]);
        assert_eq!(offers(&tree, &policy), [(r"D:\Chosen\Plain".into(), "xpress16k".into())]);
        tree.nodes[ROOT].attributes |= REPARSE;
        assert!(offers(&tree, &policy).is_empty(), "a link ancestor also rules it out");
    }

    #[test]
    fn no_network_paths_even_when_configured() {
        for root in [r"\\server\share\Library", r"X:\Mapped\Library"] {
            let tree = fixture(root, &[(r"One\payload.txt", 4 * GB)]);
            let policy = policy(None, &[Path::new(root)]);
            assert!(compact_in(&tree, &policy, |_| false).is_empty(), "{root}");
        }
        let root = Path::new(r"\\server\share\Library");
        assert!(sprawl_core::local::lexical(root).is_err());
        assert!(policy(None, &[root]).scope(&root.join("One")).is_err());
    }

    fn fixture_profile(path: &Path) -> Option<&Path> {
        path.ancestors().find(|p| p.parent().and_then(Path::file_name).is_some_and(|n| n.eq_ignore_ascii_case("Users")))
    }

    fn runtime_policy(dir: &Path) -> CompactPolicy {
        policy(fixture_profile(dir), &[dir.parent().unwrap()])
    }

    #[test]
    fn reviewed_compression_refuses_replacement_and_lost_library_evidence() {
        let temp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-compress-identity").unwrap();
        let library = temp.path();
        let game = library.join("Game");
        let peer = library.join("Peer");
        for dir in [&game, &peer] {
            std::fs::create_dir(dir).unwrap();
            std::fs::File::create(dir.join("game.exe")).unwrap().set_len(LIBRARY_CHILD).unwrap();
        }
        // TEMP can be under Users; exercise detected-library logic on the fixture itself.
        let mut policy = policy(fixture_profile(&game), &[]);
        policy.allow_fixture_home = true;
        let command = format!("{COMPRESS}xpress8k {}", ps_quote(&format!("{}\\*", game.display())));
        let receipt = review_compression_in(&game, &command, &policy).unwrap();
        assert_eq!(receipt.origin, Origin::Other);
        let stop = AtomicBool::new(false);
        assert!(live_eligible(&game, receipt.origin, &policy, &[], &stop, &mut CompressionBatch::default()));
        let check = || compress_scope_in(&receipt, &crate::clean::Spare::default(), &stop, &mut |_| {}, &policy, &[], &mut CompressionBatch::default());
        std::fs::rename(peer.join("game.exe"), peer.join("game.txt")).unwrap();
        assert!(check().unwrap_err().contains("no longer qualifies"), "same directory, but the live library evidence is gone");
        std::fs::rename(game.as_path(), library.join("Old")).unwrap();
        std::fs::create_dir(&game).unwrap();
        assert!(check().unwrap_err().contains("not the one reviewed"));
    }

    #[test]
    fn sibling_compression_reuses_one_inventory_but_rechecks_the_selected_program() {
        let temp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-compress-siblings").unwrap();
        let a = temp.path().join("First Game");
        let b = temp.path().join("Second Game");
        for dir in [&a, &b] {
            std::fs::create_dir(dir).unwrap();
            std::fs::File::create(dir.join("program.exe")).unwrap().set_len(LIBRARY_CHILD).unwrap();
        }
        let mut policy = policy(fixture_profile(&a), &[]);
        policy.allow_fixture_home = true;
        let mut batch = CompressionBatch::default();
        let stop = AtomicBool::new(false);
        assert!(live_eligible(&a, Origin::Other, &policy, &[], &stop, &mut batch));
        let parent = sprawl_core::local::directory(temp.path()).unwrap();
        assert!(batch.is_member(&parent, &b, &[], || panic!("re-inventoried unchanged siblings")));
        let command = format!("{COMPRESS}xpress8k {}", ps_quote(&format!("{}\\*", b.display())));
        let reviewed = review_compression_in(&b, &command, &policy).unwrap();
        std::fs::rename(b.join("program.exe"), b.join("personal.txt")).unwrap();
        let error = compress_scope_in(&reviewed, &crate::clean::Spare::default(), &stop, &mut |_| panic!("wrote a non-program"),
            &policy, &[], &mut batch).unwrap_err();
        assert!(error.contains("no longer qualifies"));
        assert!(!batch.is_member(&parent, &b, &[a], || None), "new exclusions invalidate the inventory");
    }

    #[test]
    fn warm_library_cache_refuses_a_game_after_its_nested_peer_changes() {
        let temp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-compress-peer-change").unwrap();
        let game = temp.path().join("Selected Game");
        let peer = temp.path().join(r"Peer\bin");
        std::fs::create_dir(&game).unwrap();
        std::fs::create_dir_all(&peer).unwrap();
        std::fs::File::create(game.join("game.exe")).unwrap().set_len(LIBRARY_CHILD).unwrap();
        let witness = peer.join("peer.exe");
        std::fs::File::create(&witness).unwrap().set_len(LIBRARY_CHILD).unwrap();
        let mut policy = policy(fixture_profile(&game), &[]);
        policy.allow_fixture_home = true;
        let mut batch = CompressionBatch::default();
        let stop = AtomicBool::new(false);
        let keep = crate::clean::Spare::default();
        let command = format!("{COMPRESS}xpress8k {}", ps_quote(&format!("{}\\*", game.display())));
        let reviewed = review_compression_in(&game, &command, &policy).unwrap();
        let parent = sprawl_core::local::directory(temp.path()).unwrap();
        assert!(live_eligible(&game, Origin::Other, &policy, &[], &stop, &mut batch));
        assert!(batch.is_member(&parent, &game, &[], || panic!("the inventory was not warm")));
        // Changing a grandchild's size does not change the reviewed folder's identity; the
        // selected folder still qualifies on its own, but no longer has a qualifying peer.
        std::fs::OpenOptions::new().write(true).open(&witness).unwrap().set_len(1).unwrap();
        let mut check = || compress_scope_in(&reviewed, &keep, &stop, &mut |_| panic!("compression began without a peer"), &policy, &[], &mut batch);
        assert!(check().unwrap_err().contains("no longer qualifies"));
        std::fs::OpenOptions::new().write(true).open(&witness).unwrap().set_len(LIBRARY_CHILD).unwrap();
        assert!(live_eligible(&game, Origin::Other, &policy, &[], &stop, &mut batch));
        std::fs::rename(&witness, peer.join("peer.txt")).unwrap();
        assert!(!live_eligible(&game, Origin::Other, &policy, &[], &stop, &mut batch));
    }

    #[test]
    fn saved_program_libraries_cannot_override_personal_and_redirected_folders() {
        let home = Path::new(r"C:\Users\sam");
        for root in [r"C:\Users\sam\Documents", r"C:\Users\sam\Desktop", r"C:\Users\sam\Pictures", r"D:\Redirected\Photos", r"D:\Redirected"] {
            let mut policy = policy(Some(home), &[Path::new(root)]);
            policy.never = Never::compression_places(home.to_str(), vec![r"D:\Redirected\Photos".into()]);
            assert!(!crate::program_folders::allowed(Path::new(root), Some(home), &policy.never), "{root}");
            let tree = fixture(root, &[(r"One\payload.txt", 4 * GB)]);
            assert!(!policy.configured_parent(&Path::new(root).join("One")), "{root}");
            assert!(compact_in(&tree, &policy, |_| true).is_empty(), "{root}");
        }
        let policy = policy(Some(home), &[Path::new(r"D:\Games")]);
        assert!(policy.configured_parent(Path::new(r"D:\Games\One")));
    }

    #[test]
    fn steam_layout_and_all_algorithms_support_undo_without_registration() {
        let temp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-steam-undo").unwrap();
        let path = temp.path().join(r"steamapps\common\Example");
        std::fs::create_dir_all(&path).unwrap();
        let file = path.join("compressible.txt");
        let text = "fixture content with repeated words\n".repeat(16000);
        let policy = runtime_policy(&path);
        assert_eq!(policy.scope(&path).unwrap().origin, Origin::Steam);
        for &(name, _, _) in COMPRESS_CHOICES {
            std::fs::write(&file, &text).unwrap();
            let command = format!("{COMPRESS}{} {}", name.to_ascii_uppercase(), ps_quote(&format!("{}\\*", path.display())));
            assert_eq!(compress_alg(&command), Some(name));
            let receipt = review_compression_in(&path, &command, &policy).unwrap();
            let d = compress_scope_in(&receipt, &crate::clean::Spare::default(), &AtomicBool::new(false), &mut |_| {}, &policy, &[], &mut CompressionBatch::default()).unwrap();
            assert_eq!(d.compressed, 1, "{name}: {d:?}");
            uncompress_current(&path, &[], &[], &policy).unwrap();
            assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        }
    }

    #[test]
    fn undo_dispatch_refuses_a_new_kept_child_and_unreadable_protections() {
        let temp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-undo-keep").unwrap();
        let game = temp.path().join("Example");
        std::fs::create_dir(&game).unwrap();
        let policy = runtime_policy(&game);
        let earlier = temp.path().join("Other");
        let reviewed = review_uncompress(&game).unwrap();
        let checked = uncompress_dispatch(&reviewed, std::slice::from_ref(&earlier), |seen| {
            assert_eq!(seen, &[earlier.clone()]);
            seen.push(game.join("new-keep"));
            Ok(())
        }, |root, keep| uncompress_current(&root.path, keep, &[], &policy));
        assert!(checked.unwrap_err().contains("kept"));
        assert!(uncompress_dispatch(&reviewed, &[], |_| Err("unreadable".into()), |_, _| panic!("Undo ran without a known Keep list")).is_err());
    }

    #[test]
    fn undo_refuses_directory_replaced_after_confirmation_was_shown() {
        let temp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-undo-replaced").unwrap();
        let game = temp.path().join("Game");
        let moved = temp.path().join("Old Game");
        std::fs::create_dir(&game).unwrap();
        let reviewed = review_uncompress(&game).unwrap();
        std::fs::rename(&game, &moved).unwrap();
        std::fs::create_dir(&game).unwrap();
        let error = uncompress_dispatch(&reviewed, &[], |_| Ok(()), |_, _| panic!("Undo targeted the replacement")).unwrap_err();
        assert!(error.contains("isn't the folder you confirmed"));
        std::fs::remove_dir(&game).unwrap();
        std::fs::rename(&moved, &game).unwrap();
        uncompress_dispatch(&reviewed, &[], |_| Ok(()), |held, _| {
            assert!(std::fs::rename(&game, &moved).is_err(), "matches and stays held while Undo runs");
            assert_eq!(held.identity().unwrap(), reviewed.id);
            Ok(())
        }).unwrap();
    }

    #[test]
    fn disk_size_is_complete_without_reading_file_contents() {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" { fn SetFileAttributesW(path: *const u16, attributes: u32) -> i32; }
        let dir = std::env::temp_dir().join(format!("sprawl-size-completeness-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let path = dir.join("sub/data.bin");
        std::fs::write(&path, vec![1u8; 100_000]).unwrap();
        assert!(on_disk(&dir).unwrap() > 0);
        let alias = dir.join("alias.bin");
        std::fs::hard_link(&path, &alias).unwrap();
        assert_eq!(on_disk(&dir), None, "aliases cannot be counted as independent physical allocation");
        assert_eq!(on_disk(&path), None, "an alias outside the measured leaf also makes allocation unknown");
        std::fs::remove_file(alias).unwrap();
        let locked = std::fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        assert!(on_disk(&dir).unwrap() > 0, "metadata-only sizing needs no content read");
        drop(locked);
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), 0x1000) }, 0);
        assert_eq!(on_disk(&dir), None, "an offline child invalidates the whole measurement");
        assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), 0x80) }, 0);
        assert!(on_disk(&dir).unwrap() > 0);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(on_disk(&dir), None);
    }

    /// The command's normal safety and WOF path, and the undo of a configured program, on a
    /// disposable tree. No real settings are used to opt this fixture in.
    #[test]
    fn compress_runs_and_keeps_the_files() {
        let dir = std::env::temp_dir().join(format!("sprawl-compress-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let text: String = (0..60_000).map(|i| format!("line {i} of a very compressible text file\n")).collect();
        std::fs::write(dir.join("a.txt"), &text).unwrap();
        std::fs::write(dir.join("sub").join("b.json"), &text).unwrap();
        let before = on_disk(&dir).unwrap();
        let policy = runtime_policy(&dir);
        let command = format!("{COMPRESS}xpress8k {}", ps_quote(&format!("{}\\*", dir.display())));
        let receipt = review_compression_in(&dir, &command, &policy).unwrap();
        let o = compress_scope_in(&receipt, &crate::clean::Spare::default(), &AtomicBool::new(false), &mut |_| {}, &policy, &[], &mut CompressionBatch::default()).unwrap();
        let after = on_disk(&dir).unwrap();
        assert_eq!((o.compressed, o.failed), (2, 0), "{o:?}");
        assert!(after < before / 2, "before {before}, after {after}, {o:?}");
        assert_eq!(std::fs::read_to_string(dir.join("sub").join("b.json")).unwrap(), text);
        let walk = sprawl_core::walk::walk(&dir, &sprawl_core::Progress::new()).unwrap();
        assert_eq!(walk.errors, 0);
        assert!(walk.nodes.iter().filter(|n| !n.is_dir).all(|n| n.is_compressed()), "WOF hidden by FindFirstFile must still be recorded");
        let file_walk = sprawl_core::walk::walk(&dir.join("a.txt"), &sprawl_core::Progress::new()).unwrap();
        assert!(file_walk.nodes[ROOT].is_compressed(), "an explicit compressed file root needs the same evidence");
        // and Uncompress puts them back as they were
        let undo = |keep: &[PathBuf], excluded: &[PathBuf]| uncompress_current(&dir, keep, excluded, &policy);
        assert!(undo(&[dir.join("sub/b.json")], &[]).is_err(), "kept descendants block the whole operation");
        assert!(undo(&[], &[dir.join("sub/b.json")]).is_err(), "excluded descendants block the whole operation");
        let unconfigured = self::policy(fixture_profile(&dir), &[]);
        assert!(uncompress_current(&dir, &[], &[], &unconfigured).is_err(), "an arbitrary remembered temp path is never an eligible application scope");
        let protected = dir.join("sub/saved.kdbx");
        std::fs::write(&protected, b"protected").unwrap();
        assert!(undo(&[], &[]).is_err_and(|e| e.contains("protected")));
        assert!(sprawl_core::walk::walk(&dir.join("a.txt"), &sprawl_core::Progress::new()).unwrap().nodes[ROOT].is_compressed());
        std::fs::remove_file(protected).unwrap();
        undo(&[], &[]).expect("exact pinned file batches uncompress the configured fixture");
        let back = on_disk(&dir).unwrap();
        assert!(back >= before / 10 * 9, "before {before}, back {back}");
        assert_eq!(compress_alg(&format!("{COMPRESS}xpress16k 'x'")), Some("xpress16k"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Compression checks the folder as it is now: a file with a second name outside is left
    /// as it is, and a link that appeared after the scan stops the whole run.
    #[test]
    fn compress_checks_the_folder_now() {
        let base = std::env::temp_dir().join(format!("sprawl-compress-now-{}", crate::store::nonce()));
        let dir = base.join("game");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let text: String = (0..30_000).map(|i| format!("line {i} of a very compressible text file\n")).collect();
        std::fs::write(dir.join("a.txt"), &text).unwrap();
        std::fs::write(base.join("outside.txt"), &text).unwrap();
        std::fs::hard_link(base.join("outside.txt"), dir.join("sub").join("shared.txt")).unwrap();
        let command = format!("{COMPRESS}xpress8k {}", crate::rules::ps_quote(&format!("{}\\*", dir.display())));
        let stop = std::sync::atomic::AtomicBool::new(false);
        let spare = crate::clean::Spare::default();
        let policy = runtime_policy(&dir);
        let receipt = review_compression_in(&dir, &command, &policy).unwrap();
        let compress = || compress_scope_in(&receipt, &spare, &stop, &mut |_| {}, &policy, &[], &mut CompressionBatch::default());
        // a command for another folder than the one given is refused
        assert!(review_compression_in(&base, &command, &policy).is_err());
        // a newly added protected file vetoes compression before another file is changed
        let vault = dir.join("sub/new.kdbx");
        std::fs::write(&vault, b"protected").unwrap();
        assert!(compress().is_err_and(|e| e.contains("protected")));
        assert!(!sprawl_core::walk::walk(&dir.join("a.txt"), &sprawl_core::Progress::new()).unwrap().nodes[ROOT].is_compressed());
        std::fs::remove_file(vault).unwrap();
        // the shared file is left uncompressed; the plain one is compressed
        let d = compress().unwrap();
        assert_eq!((d.compressed, d.changed), (1, 1), "{d:?}");
        assert!(d.after < d.before, "{d:?}");
        let walk = sprawl_core::walk::walk(&base, &sprawl_core::Progress::new()).unwrap();
        let compressed = |name: &str| walk.nodes.iter().any(|n| n.name == name && n.is_compressed());
        assert!(compressed("a.txt"));
        assert!(!compressed("shared.txt") && !compressed("outside.txt"));
        // compressed files are data, not links: compressing again is fine
        let d = compress().unwrap();
        assert_eq!((d.compressed, d.already), (0, 1), "{d:?}");
        // a junction that appeared after the scan: nothing is compressed
        std::fs::create_dir_all(base.join("elsewhere")).unwrap();
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(dir.join("sub").join("link")).arg(base.join("elsewhere")).output().is_ok_and(|o| o.status.success());
        if made {
            assert!(compress().is_err_and(|e| e.contains("is a link")));
            let _ = std::fs::remove_dir(dir.join("sub").join("link"));
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod shrink_tests {
    use std::path::PathBuf;
    use super::*;

    fn disk_finding(path: &Path, bytes: u64, id: &str) -> Finding {
        Finding {
            path: path.to_path_buf(),
            owner: Some("WSL".into()),
            bytes,
            risk: Risk::Orange,
            remedy: Remedy::Info,
            command: None,
            title: "WSL Linux disk".into(),
            evidence: vec![],
            confidence: 0.9,
            source: Source::Rule(id.into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: false,
            action: None,
            admin: false,
            carved: Vec::new(),
            gain: None,
            forced_from: None,
        }
    }

    /// The WSL and Docker disk tips become Sprawl's shrink, which the safety checks let through
    /// although disk images are on the never-list; from anywhere else they stay tips.
    #[test]
    fn a_disk_tip_becomes_a_shrink_only_from_the_built_in_rules() {
        let dir = std::env::temp_dir().join(format!("sprawl-shrink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let disk = dir.join("ext4.vhdx");
        std::fs::write(&disk, vec![1u8; 3 << 20]).unwrap();
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let n = crate::find_path(&tree, &disk).unwrap();
        let never = crate::safety::Never::default_list();

        let mut f = disk_finding(&disk, tree.nodes[n].size, "vm.wsl-disk");
        // as a tip, a delete of it is refused by the never-list
        let del = Finding { remedy: Remedy::Delete, ..f.clone() };
        assert!(crate::safety::check(&del, &tree, &never).is_err());
        shrinkable(&mut f);
        assert!(shrinks(&f) && f.admin && f.allow_protected && f.remedy == Remedy::Command && f.gain == Some(0));
        assert!(crate::safety::check(&f, &tree, &never).is_ok(), "{:?}", crate::safety::check(&f, &tree, &never));
        let (a, admin) = crate::clean::action(&f).unwrap();
        assert!(admin && is_shrink(&a) && in_place(&a) && !is_compress(&a));
        let Action::Tool { command } = &a else { panic!() };
        assert!(command.contains("wsl.exe' --shutdown") && command.contains("compact vdisk") && !command.contains("Docker Desktop"));
        // runs as administrator: its first program is Windows' own
        let sys = crate::clean::system_dir().unwrap();
        assert!(crate::clean::elevated_tool_ok(command, &|p| Some(sys.join(p)), std::slice::from_ref(&sys)).is_ok());

        let mut d = disk_finding(&disk, tree.nodes[n].size, "vm.docker-disk");
        shrinkable(&mut d);
        let Some(Action::Tool { command: dc }) = &d.action else { panic!() };
        assert!(dc.find("Docker Desktop").unwrap() < dc.find("wsl.exe").unwrap());
        let diskpart = ps_quote(&sys.join("diskpart.exe").display().to_string());
        assert!(dc.contains(&format!("| & {diskpart}")) && command.contains(&diskpart));
        assert!(!dc.contains("$env:SystemRoot") && !command.contains("$env:SystemRoot"));
        assert!(crate::clean::elevated_tool_ok(&format!("{command}\n& 'C:\\fake\\evil.exe'"), &|p| Some(PathBuf::from(p)), std::slice::from_ref(&sys)).is_err(), "a marker alone never approves later statements");

        // a local pack's rule with the same id, a model's finding, another rule: unchanged
        for mut other in [
            Finding { builtin: false, ..disk_finding(&disk, 1, "vm.wsl-disk") },
            Finding { source: Source::Model("m".into()), ..disk_finding(&disk, 1, "vm.wsl-disk") },
            disk_finding(&disk, 1, "vm.hyperv-disk"),
        ] {
            shrinkable(&mut other);
            assert!(other.action.is_none() && other.remedy == Remedy::Info);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The command is valid PowerShell for any path, and a quote or `$` in it stays text.
    #[test]
    fn the_shrink_command_parses_for_odd_paths() {
        let ps = PathBuf::from(std::env::var("SystemRoot").unwrap()).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        for p in [r"C:\Users\a\AppData\Local\wsl\{1}\ext4.vhdx", r"C:\Users\o'brien\$x\ext4.vhdx"] {
            let cmd = shrink_command(Path::new(p), true);
            let out = std::process::Command::new(&ps)
                .args(["-NoProfile", "-NonInteractive", "-Command", "$e=$null; $t=$null; $a=[Management.Automation.Language.Parser]::ParseInput($env:SPRAWL_T,[ref]$t,[ref]$e); $e.Count; ($t | Where-Object { $_.Kind -eq 'Variable' } | ForEach-Object { $_.Name }) -join ','"])
                .env("SPRAWL_T", &cmd)
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            let mut lines = text.lines();
            assert_eq!(lines.next().map(str::trim), Some("0"), "{cmd}\n{text}");
            // the only variables are the script's own: the path's `$x` is inside quotes
            let vars = lines.next().unwrap_or("").trim().to_string();
            assert!(vars.split(',').all(|v| ["p", "a", "b", "env:SystemRoot"].contains(&v)), "{vars}");
        }
    }
}
