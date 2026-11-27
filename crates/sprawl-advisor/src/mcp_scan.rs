//! The MCP's own scans and the trees it answers from (next round, Phase 1).
//!
//! `scan_path` scans a folder or drive with the folder walk and the user's own rights, never the
//! MFT and never elevated (the user's decision, 2026-11-18: an AI's scan sees no more than the
//! user could list). The running app makes it when it isn't elevated; an elevated app hands it
//! back to the server's own process ([`PASS_TO_SERVER`]), unless the user turned on "MCP tools
//! reading the disk (as administrator)" (2026-11-19): then the elevated app scans (still the
//! folder walk, never the MFT). `scan_status` follows the scan; `list_scans` lists every tree the other tools can
//! answer from (this session's scans and History's saved ones). A finished scan goes into
//! History when the app's setting keeps History, so the user sees it there.
//!
//! Every tool here stays read-only: a scan reads names and sizes, nothing else.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};
use sprawl_core::{Progress, Tree};

use crate::facts::Facts;
use crate::finding::Finding;
use crate::llm::tools::Tools;
use crate::llm::ToolSpec;
use crate::memory::Memory;
use crate::mcp::Workers;
use crate::mcp_cancel::Cancel;

/// The longest `wait_seconds` honoured: MCP clients give up on a call after about a minute.
pub const MAX_WAIT: u64 = 50;
const PREPARE_LIMIT: Duration = Duration::from_secs(10);
const SCAN_LIMIT: Duration = Duration::from_secs(600);
/// A Windows driver may not unwind after cancellation. Never accumulate more stuck scans.
const MAX_RETIRED: usize = 2;
/// Trees this process scanned that it keeps (a whole drive can take a few hundred MB).
const KEEP_SCANS: usize = 3;
/// Full History trees retain their facts and findings across alternating queries.
const KEEP_HISTORY: usize = 3;
/// Older than this, an answer suggests scanning again.
const STALE_SECS: u64 = 86_400;

/// The answer the tools give (here and in the app) for a path outside the tree they have.
pub const NOT_IN_SCAN: &str = "isn't in the scan";
/// The elevated app's answer to a scan tool: the server scans in its own process instead.
pub const PASS_TO_SERVER: &str = "Sprawl runs as administrator, so the server scans with your own rights instead.";

/// The app's answer when it has no tree open.
pub const NO_SCAN_OPEN: &str = "No scan is open in Sprawl yet.";
pub const GATHERING_FACTS: &str = "Sprawl is still gathering facts for this scan; try again shortly.";
const PREPARING_FINDINGS: &str = "Sprawl is still preparing findings for this scan; try again shortly.";

pub fn scan_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "scan_path",
            description: "Scan a folder or a whole drive (such as D:\\ or C:\\Users\\me\\Projects) so the other tools can answer about it. \
                Returns a job number. With wait_seconds (up to 50) it waits that long and answers with the result when the scan is done \
                in time; a folder usually is, a whole drive can take minutes (follow it with scan_status). A path already scanned isn't \
                scanned again unless rescan is true. Network paths are refused. Read-only on the disk: a scan reads names and sizes. When History is on in Sprawl, the scan is saved there (folders only).",
            schema: json!({"type": "object", "properties": {
                "path": {"type": "string", "description": "A full path: a drive (D:\\) or a folder."},
                "rescan": {"type": "boolean", "description": "Scan again even if Sprawl has a scan of this path."},
                "wait_seconds": {"type": "integer", "description": "Wait up to this many seconds (max 50) for the scan to finish."}
            }, "required": ["path"]}),
        },
        ToolSpec {
            name: "scan_status",
            description: "How a scan started with scan_path is going: its stage, files and bytes so far, or done or failed. Without job, the latest scan.",
            schema: json!({"type": "object", "properties": {
                "job": {"type": "integer", "description": "The job number scan_path returned."},
                "wait_seconds": {"type": "integer", "description": "Wait up to this many seconds (max 50) for it to finish."}
            }}),
        },
        ToolSpec {
            name: "list_scans",
            description: "Every scan the other tools can answer from: the scans made here and History's saved scans, each with its root, how old it is and its size.",
            schema: json!({"type": "object", "properties": {}}),
        },
    ]
}

/// Whether `name` is one of this module's tools.
pub fn is_scan_tool(name: &str) -> bool {
    matches!(name, "scan_path" | "scan_status" | "list_scans")
}

/// "3 hours ago"
pub fn age(t: SystemTime) -> String {
    let s = t.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    match s {
        0..=89 => "just now".into(),
        90..=5399 => format!("{} minutes ago", (s + 30) / 60),
        5400..=129_599 => format!("{} hours ago", (s + 1800) / 3600),
        _ => format!("{} days ago", (s + 43_200) / 86_400),
    }
}

fn is_stale(t: SystemTime) -> bool {
    t.elapsed().is_ok_and(|d| d.as_secs() > STALE_SECS)
}

/// Select an observation by exact canonical component identity, including case.
fn covers(root: &Path, path: &Path) -> bool {
    crate::history::covers(root, path)
}

/// The `path` a call names, if any.
pub fn path_of(input: &Value) -> Option<PathBuf> {
    let p = input["path"].as_str()?.trim().trim_matches('"').replace('/', "\\");
    (!p.is_empty()).then(|| PathBuf::from(p))
}

/// Pin a plain local folder and refuse paths the MCP is not allowed to scan.
fn checked_root(path: &Path) -> Result<sprawl_core::local::Directory, String> {
    if !path.is_absolute() {
        return Err(format!("{} isn't a full path: give one such as D:\\ or C:\\Users\\me\\Projects.", path.display()));
    }
    let pinned = match sprawl_core::local::directory(path) {
        Ok(p) => p,
        Err(e) => return Err(format!("{} isn't a plain local folder Sprawl can open; network, device and redirected paths are refused: {e}", path.display())),
    };
    let root = &pinned.path;
    let s = root.to_string_lossy();
    if s.starts_with(r"\\") && !s.starts_with(r"\\?\Volume") {
        return Err(format!("{} is a network path: Sprawl doesn't scan those.", path.display()));
    }
    if is_network_drive(&s) {
        return Err(format!("{} is on a network drive: Sprawl doesn't scan those.", path.display()));
    }
    if !root.is_dir() {
        return Err(format!("{} isn't a folder or drive Sprawl can open.", path.display()));
    }
    Ok(pinned)
}

#[cfg(windows)]
fn is_network_drive(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
        return false;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
    }
    let w: Vec<u16> = format!("{}:\\", b[0] as char).encode_utf16().chain(Some(0)).collect();
    // DRIVE_REMOTE
    unsafe { GetDriveTypeW(w.as_ptr()) == 4 }
}

#[cfg(not(windows))]
fn is_network_drive(_: &str) -> bool {
    false
}

/// A tree this process holds: one it scanned, or one loaded from History.
pub struct Held {
    pub root: PathBuf,
    pub taken: SystemTime,
    /// History's file it was loaded from; `None` for a scan made here.
    file: Option<PathBuf>,
    tree: Tree,
    facts: Preparation<Facts>,
    found: Preparation<Vec<Finding>>,
    /// Scanned by the running app (in the background), not by the server's own process.
    in_app: bool,
}

struct Preparation<T> {
    ready: OnceLock<T>,
    running: AtomicBool,
}

impl<T> Default for Preparation<T> {
    fn default() -> Self { Self { ready: OnceLock::new(), running: AtomicBool::new(false) } }
}

impl Held {
    fn new(tree: Tree, taken: SystemTime, file: Option<PathBuf>, in_app: bool) -> Held {
        Held { root: tree.root_path.clone(), taken, file, tree, facts: Preparation::default(), found: Preparation::default(), in_app }
    }

    fn prepare<T: Send + Sync + 'static>(self: &Arc<Self>, workers: &Workers, select: fn(&Held) -> &Preparation<T>, retry: &str,
        make: impl FnOnce(&Held) -> T + Send + 'static) -> Result<&T, String> {
        let slot = select(self);
        if let Some(result) = slot.ready.get() { return Ok(result); }
        if slot.running.swap(true, Ordering::AcqRel) {
            return Err(retry.into());
        }
        struct Running<T>(Arc<Held>, fn(&Held) -> &Preparation<T>);
        impl<T> Drop for Running<T> {
            fn drop(&mut self) { (self.1)(&self.0).running.store(false, Ordering::Release); }
        }
        let job = Running(self.clone(), select);
        if let Some(result) = slot.ready.get() { return Ok(result); }
        workers.spawn(move || {
            let result = make(&job.0);
            let _ = (job.1)(&job.0).ready.set(result);
        })?;
        Err(retry.into())
    }

    fn facts_with(self: &Arc<Self>, workers: &Workers, gather: impl FnOnce(&Tree) -> Facts + Send + 'static) -> Result<&Facts, String> {
        self.prepare(workers, |held| &held.facts, GATHERING_FACTS, move |held| gather(&held.tree))
    }

    fn findings_with(self: &Arc<Self>, workers: &Workers, evaluate: impl FnOnce(&Tree, &Facts) -> Vec<Finding> + Send + 'static) -> Result<&[Finding], String> {
        self.facts.ready.get().ok_or(GATHERING_FACTS)?;
        self.prepare(workers, |held| &held.found, PREPARING_FINDINGS, move |held| {
            evaluate(&held.tree, held.facts.ready.get().expect("facts are already published"))
        }).map(Vec::as_slice)
    }

    /// The first line of every answer from this tree: where it is from and how old.
    fn note(&self, app_running: bool) -> String {
        let when = age(self.taken);
        let mut s = match (&self.file, app_running) {
            (None, _) if self.in_app => format!("(From the scan of {} Sprawl made in the background for this server, {when}; it isn't the one on Sprawl's screen.", self.root.display()),
            (None, _) => format!("(From the scan of {} made through this server, {when}.", self.root.display()),
            (Some(_), false) => format!("(Sprawl isn't running, so this is from History's saved scan of {}, taken {when}. Scans done since aren't in it.", self.root.display()),
            (Some(_), true) => format!("(This isn't in the scan open in Sprawl, so it is from History's saved scan of {}, taken {when}.", self.root.display()),
        };
        if is_stale(self.taken) {
            s += " That is old: scan_path with rescan gives a fresh one.";
        }
        s + ")\n\n"
    }

    fn call(self: &Arc<Self>, name: &str, input: &Value, memory: &Memory, app_running: bool, cancel: &Cancel, preparers: &Workers) -> Result<String, String> {
        let note = self.note(app_running);
        cancel.check()?;
        let facts = self.facts_with(preparers, |tree| {
            let mut f = Facts::gather();
            f.attach_usage(tree);
            f
        })?;
        cancel.check()?;
        let answer = match name {
            "scan_report" | "findings" => {
                let found = self.findings_with(preparers, crate::mcp::rules_for)?;
                cancel.check()?;
                Ok(if name == "findings" { crate::mcp::findings_list(found) } else { crate::llm::report::build(&self.tree, facts, Some(found), None, None, &memory.labels) })
            }
            "cleaners" | "set_filter" if app_running => Err(format!("That works only for the scan open in Sprawl; {} isn't in it.", self.root.display())),
            "cleaners" | "set_filter" => Err("That needs Sprawl open: start Sprawl and ask again.".to_string()),
            _ => Tools::new(&self.tree, facts, "claude-code", &[], memory).at_scan(self.taken).call_cancellable(name, input, cancel.flag()),
        };
        match answer {
            Ok(t) => Ok(note + &t),
            Err(e) => Err(note + &e),
        }
    }
}

enum JobState {
    Running {
        work: Arc<ScanWork>,
        handle: std::thread::JoinHandle<Result<ScanResult, String>>,
    },
    Done {
        files: u64,
        bytes: u64,
        secs: f64,
        method: &'static str,
        history: String,
    },
    Failed(String),
    Reused(String),
}

enum ScanResult {
    Tree(Tree, String, f64),
    Reused(String),
}

struct ScanWork {
    cancel: Arc<Cancel>,
    progress: OnceLock<Arc<Progress>>,
    saving: AtomicBool,
    started: Instant,
    prepare_by: Instant,
    expired: OnceLock<&'static str>,
}

impl ScanWork {
    fn limit(&self) -> Option<&'static str> {
        if let Some(reason) = self.expired.get() { return Some(*reason); }
        let expired = if self.started.elapsed() >= SCAN_LIMIT { Some("The scan exceeded its 10-minute limit.") }
        else if self.progress.get().is_none() && Instant::now() >= self.prepare_by {
            Some("Sprawl couldn't check the path and folders to skip within 10 s.")
        } else { None };
        expired.map(|reason| *self.expired.get_or_init(|| reason)).or_else(|| self.expired.get().copied())
    }

    fn check(&self) -> Result<(), String> {
        if let Some(reason) = self.limit() { return Err(reason.into()); }
        self.cancel.check()
    }
}

type ScanThread = std::thread::JoinHandle<Result<ScanResult, String>>;

struct Job {
    id: u64,
    root: PathBuf,
    taken: SystemTime,
    state: JobState,
}

/// This process's scans and the trees it answers from.
pub struct Index {
    state: Mutex<ScanState>,
    history: Mutex<HistoryCache>,
    in_app: bool,
    preparers: Workers,
}

#[derive(Default)]
struct HistoryCache {
    recent: VecDeque<(crate::history::Entry, Arc<Held>)>,
}

impl HistoryCache {
    fn get(&mut self, entry: &crate::history::Entry) -> Option<Arc<Held>> {
        // Entry equality includes the native file identity and metadata, not just the path.
        let at = self.recent.iter().position(|(key, _)| key == entry)?;
        let pair = self.recent.remove(at).expect("position in this cache");
        let held = pair.1.clone();
        self.recent.push_back(pair);
        Some(held)
    }
}

impl Default for Index {
    fn default() -> Self {
        Self { state: Mutex::default(), history: Mutex::default(), in_app: false, preparers: Workers::new(2) }
    }
}

#[derive(Default)]
struct ScanState {
    jobs: Vec<Job>,
    /// Retired threads are never polled for a result or associated with another job.
    retired: Vec<ScanThread>,
    /// Scans made here, newest last. Whole-tree answers use these outside the state lock.
    held: Vec<Arc<Held>>,
    /// Tests and screenshot runs: History is neither read nor written.
    no_history: bool,
    /// The app's own scan is running: don't start a second one.
    busy: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct ScanReply {
    pub text: String,
    pub job: Option<u64>,
    pub running: bool,
    pub started: bool,
    pub failed: bool,
}

/// Odd IDs belong to the GUI; even IDs belong to this MCP process. Include a per-process
/// seed so a restarted GUI doesn't mistake a previous GUI's job for one of its own.
pub(crate) fn app_job(id: u64) -> bool { id % 2 == 1 }

fn job_id(in_app: bool) -> u64 {
    use std::hash::{Hash, Hasher};
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    let next = NEXT.get_or_init(|| {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        crate::store::nonce().hash(&mut hash);
        AtomicU64::new((hash.finish() & ((1 << 39) - 1)) << 12)
    });
    2 * next.fetch_add(1, Ordering::Relaxed) + u64::from(in_app)
}

impl ScanState {
    /// A finished scan becomes a held tree. Called before every answer.
    fn poll(&mut self, in_app: bool) {
        self.retired.retain(|handle| !handle.is_finished());
        for j in &mut self.jobs {
            let JobState::Running { work, handle } = &j.state else { continue };
            if !handle.is_finished() {
                if let Err(reason) = work.check() {
                    work.cancel.stop();
                    if let Some(p) = work.progress.get() { p.cancel(); }
                    let JobState::Running { handle, .. } = std::mem::replace(&mut j.state, JobState::Failed(reason)) else { unreachable!() };
                    self.retired.push(handle);
                }
                continue;
            }
            let JobState::Running { handle, .. } = std::mem::replace(&mut j.state, JobState::Failed(String::new())) else { unreachable!() };
            j.state = match handle.join() {
                Ok(Ok(ScanResult::Tree(tree, history, secs))) => {
                    let s = &tree.nodes[sprawl_core::ROOT];
                    let done = JobState::Done { files: s.files, bytes: s.size, secs, method: tree.method.label(), history };
                    // a scan of the same root replaces the older one
                    self.held.retain(|h| h.root != tree.root_path);
                    j.root = tree.root_path.clone();
                    self.held.push(Arc::new(Held::new(tree, j.taken, None, in_app)));
                    if self.held.len() > KEEP_SCANS {
                        self.held.remove(0);
                    }
                    done
                }
                Ok(Ok(ScanResult::Reused(text))) => JobState::Reused(text),
                Ok(Err(e)) => JobState::Failed(e),
                Err(_) => JobState::Failed("the scan stopped with an internal error".into()),
            };
        }
        // Keep status for recent jobs without growing forever during a long GUI session.
        if self.jobs.len() > 64 { self.jobs.drain(..self.jobs.len() - 64); }
    }

    fn running(&self) -> Option<&Job> {
        self.jobs.iter().find(|j| matches!(j.state, JobState::Running { .. }))
    }

    fn start(&mut self, in_app: bool, root: PathBuf, rescan: bool, cancel: Arc<Cancel>, prepare_policy: impl FnOnce() -> Arc<Progress> + Send + 'static) -> Result<u64, String> {
        self.start_with(in_app, root, rescan, cancel, PREPARE_LIMIT, prepare_policy)
    }

    fn start_with(&mut self, in_app: bool, root: PathBuf, rescan: bool, cancel: Arc<Cancel>, prepare_limit: Duration, prepare_policy: impl FnOnce() -> Arc<Progress> + Send + 'static) -> Result<u64, String> {
        self.retired.retain(|handle| !handle.is_finished());
        if self.retired.len() >= MAX_RETIRED {
            return Err("Sprawl can't start another scan: too many stopped scans are still waiting for Windows I/O. Try again when one has cleared.".into());
        }
        let id = job_id(in_app);
        let started = Instant::now();
        let work = Arc::new(ScanWork { cancel, progress: OnceLock::new(), saving: AtomicBool::new(false),
            started, prepare_by: started + prepare_limit, expired: OnceLock::new() });
        let (w, r) = (work.clone(), root.clone());
        let taken = SystemTime::now();
        let quiet = self.no_history;
        let known: Vec<_> = self.held.iter().rev().map(|h| (h.root.clone(), h.taken)).collect();
        let handle = std::thread::Builder::new().spawn(move || {
            let watched = w.clone();
            let _watch = crate::pipe::watch_current_io(move || {
                let stop = watched.limit().is_some() || watched.cancel.stopped();
                if stop {
                    if let Some(p) = watched.progress.get() { p.cancel(); }
                }
                stop
            }).ok();
            w.check()?;
            let pinned = checked_root(&r)?;
            let root = &pinned.path;
            w.check()?;
            if !rescan {
                if let Some((path, when)) = known.iter().find(|(path, _)| covers(path, root)) {
                    return Ok(ScanResult::Reused(format!("{} is in the scan of {} made {}; the other tools answer from it. Pass rescan: true to scan again.", root.display(), path.display(), age(*when))));
                }
            }
            let p = prepare_policy();
            w.check()?;
            let _ = w.progress.set(p.clone());
            // Don't poll the cross-process event on every file in a large directory.
            let next_check = AtomicU64::new(0);
            let stop = || {
                let ms = w.started.elapsed().as_millis() as u64;
                let next = next_check.load(Ordering::Relaxed);
                if ms >= next && next_check.compare_exchange(next, ms + 25, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                    if w.check().is_err() { p.cancel(); }
                }
                p.is_cancelled()
            };
            // the folder walk, never the MFT: only what the user's own rights can list
            let walked = sprawl_core::walk::walk_local_until(pinned, &p, &stop);
            w.check()?;
            let tree = walked.map_err(|e| format!("The scan of {} failed: {e}", r.display()))?;
            // into History as the app saves its own scans, when the user keeps History
            let base = if quiet { None } else { crate::history::base_dir() };
            let policy = if quiet { crate::history::Retention::parse(|_| None) } else { crate::history::Retention::load() };
            w.check()?;
            let history = match (policy.enabled, base) {
                (true, Some(base)) => {
                    w.saving.store(true, Ordering::Relaxed);
                    // folders only (Fable AI review P3-10): small, and the first to go when History is over
                    // its cap, so a scan the AI asked for doesn't push out the user's own snapshots
                    let prepared = crate::history::prepare(&tree, taken, false);
                    w.check()?;
                    match prepared.save_retained_until(&base, policy, &|| w.limit().is_some() || w.cancel.stopped()) {
                        Ok(()) => "Saved in Sprawl's History (folders only).".to_string(),
                        Err(e) => format!("Not saved in History: {e}."),
                    }
                }
                (_, None) if quiet => "Not saved in History (this run of Sprawl writes nothing).".to_string(),
                (false, _) => "Not saved in History (History is off in Sprawl's settings).".to_string(),
                (true, None) => "Not saved in History (no place for it).".to_string(),
            };
            w.check()?;
            Ok(ScanResult::Tree(tree, history, w.started.elapsed().as_secs_f64()))
        }).map_err(|e| format!("Sprawl couldn't start a scan worker: {e}"))?;
        self.jobs.push(Job { id, root, taken, state: JobState::Running { work, handle } });
        Ok(id)
    }

    fn status(&self, id: u64, started: bool) -> Result<ScanReply, String> {
        let j = self.jobs.iter().find(|j| j.id == id).ok_or_else(|| format!("There is no scan {id}."))?;
        let head = format!("Scan {} of {}: ", j.id, j.root.display());
        let mut running = false;
        let text = match &j.state {
            JobState::Running { work, .. } => {
                running = true;
                let details = if let Some(progress) = work.progress.get() {
                    let s = progress.snapshot();
                    let stage = if work.saving.load(Ordering::Relaxed) { "saving to History" } else if s.phase.is_empty() { "starting" } else { s.phase };
                    let frac = s.fraction.map(|f| format!(", about {:.0}% done", f * 100.0)).unwrap_or_default();
                    format!("{stage}{frac}, {}: {} files, {} folders, {} so far", s.method.label(), s.files, s.folders, sprawl_core::format_size(s.bytes))
                } else { "checking the path and folders to skip".into() };
                format!(
                    "{head}still scanning ({details}; {:.0} s). Call scan_status with job {} (and wait_seconds) to follow it.",
                    work.started.elapsed().as_secs_f64(),
                    j.id
                )
            }
            JobState::Done { files, bytes, secs, method, history } => format!(
                "{head}done in {secs:.1} s ({method}): {files} files, {}. {history} The other tools now answer about it: pass a path inside it, or call scan_report with path {}.",
                sprawl_core::format_size(*bytes),
                j.root.display()
            ),
            JobState::Failed(e) => format!("{head}failed. {e}"),
            JobState::Reused(text) => text.clone(),
        };
        Ok(ScanReply { text, job: Some(id), running, started, failed: matches!(j.state, JobState::Failed(_)) })
    }

    fn list(&self) -> String {
        let mut s = String::new();
        for h in &self.held {
            s += &format!("- {} (scanned here {}): {}\n", h.root.display(), age(h.taken), sprawl_core::format_size(h.tree.nodes[sprawl_core::ROOT].size));
        }
        if let Some(j) = self.running() {
            s += &format!("- {} (scan {} running)\n", j.root.display(), j.id);
        }
        s
    }
}

impl Index {
    /// The app's index: `history` false in a screenshot run, which writes nothing.
    pub fn for_app(history: bool) -> Index {
        Index { state: Mutex::new(ScanState { no_history: !history, ..Default::default() }), in_app: true, ..Default::default() }
    }

    /// The GUI shares its facts worker budget with the on-screen tree.
    pub fn with_facts_workers(mut self, workers: Workers) -> Self {
        self.preparers = workers;
        self
    }

    fn state(&self) -> std::sync::MutexGuard<'_, ScanState> { self.state.lock().unwrap_or_else(|e| e.into_inner()) }

    pub fn set_app_state(&self, busy: bool, history: bool) {
        let mut state = self.state();
        state.busy = busy;
        state.no_history = !history;
    }

    /// Only a snapshot; waiting never holds the index or occupies the GUI pipe.
    pub(crate) fn scan_now(&self, name: &str, input: &Value, as_admin: bool, cancel: Arc<Cancel>) -> Result<ScanReply, String> {
        cancel.check()?;
        if self.in_app && !as_admin && !sprawl_core::elevation().is_ok_and(|elevated| !elevated) {
            return Err(PASS_TO_SERVER.into());
        }
        if name == "list_scans" {
            return Ok(ScanReply { text: self.list(), job: None, running: false, started: false, failed: false });
        }
        let mut state = self.state();
        state.poll(self.in_app);
        cancel.check()?;
        match name {
            "scan_path" => {
                let path = path_of(input).ok_or("give a `path`: a drive such as D:\\ or a folder")?;
                if !path.is_absolute() {
                    return Err(format!("{} isn't a full path: give one such as D:\\ or C:\\Users\\me\\Projects.", path.display()));
                }
                if let Some(job) = state.running() {
                    return if job.root == path { state.status(job.id, false) }
                        else { Err(format!("Scan {} of {} is still active; one scan at a time. Follow it with scan_status.", job.id, job.root.display())) };
                }
                if state.busy {
                    return Err("Sprawl is scanning on its screen right now; one scan at a time. Ask again when it has finished.".into());
                }
                let id = state.start(self.in_app, path, input["rescan"].as_bool() == Some(true), cancel, crate::exclusions::progress)?;
                state.status(id, true)
            }
            "scan_status" => {
                let id = input["job"].as_u64().or_else(|| state.jobs.last().map(|j| j.id)).ok_or("No scan has been started here: call scan_path.")?;
                state.status(id, false)
            }
            _ => Err(format!("there is no tool called {name}")),
        }
    }

    fn list(&self) -> String {
        let (mut s, no_history) = {
            let mut state = self.state();
            state.poll(self.in_app);
            (state.list(), state.no_history)
        };
        let base = if no_history { None } else { crate::history::base_dir() };
        let saved = base.as_deref().map(crate::history::last_scans).unwrap_or_default();
        if !saved.is_empty() {
            s += "In History (the newest saved scan of each folder):\n";
            for (root, when, bytes) in saved {
                let full = base.as_deref().is_some_and(|b| crate::history::covering(b, &root).iter().any(|e| e.full && e.root == root));
                let note = if full { "" } else { ", folder sizes only" };
                s += &format!("- {} ({}{note}): {}\n", root.display(), age(when), sprawl_core::format_size(bytes));
            }
        }
        if s.is_empty() {
            return "No scans yet: call scan_path with a drive or folder.".into();
        }
        s
    }

    /// A scan made here that holds `path`, the newest first.
    pub fn has_scan_for(&self, path: &Path) -> bool {
        let mut state = self.state();
        state.poll(self.in_app);
        state.held.iter().any(|h| covers(&h.root, path))
    }

    /// Answer a query tool from the freshest tree that holds the call's path (any tree when it
    /// names none): a scan made here, or History's newest full snapshot.
    pub fn answer(&self, name: &str, input: &Value, memory: &Memory, app_running: bool) -> Result<String, String> {
        self.answer_cancellable(name, input, memory, app_running, &Cancel::default())
    }

    pub(crate) fn answer_cancellable(&self, name: &str, input: &Value, memory: &Memory, app_running: bool, cancel: &Cancel) -> Result<String, String> {
        if !crate::mcp::bridge_specs().iter().any(|t| t.name == name) {
            return Err(format!("there is no tool called {name}"));
        }
        cancel.check()?;
        let path = path_of(input);
        let held = self.pick(path.as_deref()).ok_or_else(|| match &path {
            Some(p) => format!("{} isn't in any scan Sprawl has: call scan_path with it (seconds for a folder, minutes for a whole drive).", p.display()),
            None if app_running => format!("{NO_SCAN_OPEN} Call scan_path with a drive or folder."),
            None => "Sprawl isn't running and has no saved scan yet: call scan_path with a drive or folder.".into(),
        })?;
        held.call(name, input, memory, app_running, cancel, &self.preparers)
    }

    /// The tree to answer from, loading it from History when that is
    /// the freshest.
    fn pick(&self, path: Option<&Path>) -> Option<Arc<Held>> {
        let (ours, no_history) = {
            let mut state = self.state();
            state.poll(self.in_app);
            (state.held.iter().filter(|h| path.is_none_or(|p| covers(&h.root, p))).max_by_key(|h| h.taken).cloned(), state.no_history)
        };
        let saved = (!no_history).then(crate::history::base_dir).flatten().and_then(|base| {
            let list = match path {
                Some(p) => crate::history::covering(&base, p),
                None => crate::history::list(&base),
            };
            list.into_iter().filter(|e| e.full).max_by_key(|e| e.taken)
        });
        let Some(e) = saved.filter(|e| ours.as_ref().is_none_or(|h| h.taken < e.taken)) else { return ours; };
        self.history_tree(e)
    }

    fn history_tree(&self, entry: crate::history::Entry) -> Option<Arc<Held>> {
        if let Some(h) = self.history.lock().unwrap_or_else(|e| e.into_inner()).get(&entry) {
            return Some(h);
        }
        let tree = crate::history::load(&entry).ok()?;
        let held = Arc::new(Held::new(tree, entry.taken, Some(entry.file.clone()), self.in_app));
        let mut history = self.history.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(same) = history.get(&entry) { return Some(same); }
        let evicted = (history.recent.len() >= KEEP_HISTORY).then(|| history.recent.pop_front());
        history.recent.push_back((entry, held.clone()));
        drop(history);
        drop(evicted);
        Some(held)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_what_it_shouldnt_scan() {
        assert!(checked_root(Path::new("relative\\dir")).err().is_some_and(|r| r.contains("full path")));
        assert!(checked_root(Path::new(r"\\server\share")).err().is_some_and(|r| r.contains("network")));
        let absent = std::env::temp_dir().join(format!("sprawl-absent-{}", crate::store::nonce()));
        assert!(checked_root(&absent).err().is_some_and(|r| r.contains("can open")));
        assert!(checked_root(&std::env::temp_dir()).is_ok());
    }

    #[test]
    fn covers_by_segments() {
        assert!(covers(Path::new(r"C:\"), Path::new(r"c:\Users\x")));
        assert!(!covers(Path::new(r"C:\Users"), Path::new(r"C:\users")));
        assert!(!covers(Path::new(r"C:\Users"), Path::new(r"C:\Users2")));
        assert!(!covers(Path::new(r"D:\"), Path::new(r"C:\x")));
    }

    #[test]
    fn held_tree_returns_retry_without_waiting_for_a_single_facts_gather() {
        let tree = Tree { volumes: Vec::new(), root_path: r"C:\fixture".into(), nodes: Vec::new(), errors: 0, method: sprawl_core::ScanMethod::Walk };
        let held = Arc::new(Held::new(tree, SystemTime::now(), None, true));
        let workers = Workers::new(1);
        let (enter, started) = std::sync::mpsc::channel();
        let (release, done) = std::sync::mpsc::channel();
        assert!(matches!(held.facts_with(&workers, move |_| {
            enter.send(()).unwrap();
            done.recv_timeout(Duration::from_secs(3)).unwrap();
            Facts::default()
        }), Err(e) if e == GATHERING_FACTS));
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        for _ in 0..6 {
            assert!(matches!(held.facts_with(&workers, |_| panic!("duplicate facts gather")), Err(e) if e == GATHERING_FACTS));
        }
        release.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        while held.facts_with(&workers, |_| panic!("ready facts should be reused")).is_err() {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn report_and_findings_retry_without_waiting_for_the_same_rule_evaluation() {
        let tree = Tree { volumes: Vec::new(), root_path: r"C:\fixture".into(), nodes: Vec::new(), errors: 0, method: sprawl_core::ScanMethod::Walk };
        let held = Arc::new(Held::new(tree, SystemTime::now(), None, true));
        assert!(held.facts.ready.set(Facts::default()).is_ok());
        let workers = Workers::new(1);
        let (entered, started) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        assert!(matches!(held.findings_with(&workers, move |_, _| {
            entered.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(3)).unwrap();
            Vec::new()
        }), Err(e) if e == PREPARING_FINDINGS));
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        let memory = Memory::default();
        let call = |name| held.call(name, &json!({}), &memory, true, &Cancel::default(), &workers);
        for name in ["scan_report", "findings", "scan_report", "findings"] {
            assert_eq!(call(name).unwrap_err(), PREPARING_FINDINGS);
        }
        release.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            match call("findings") {
                Ok(text) => { assert!(text.contains("none matched")); break; }
                Err(e) => { assert_eq!(e, PREPARING_FINDINGS); assert!(Instant::now() < until); }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn alternating_history_queries_reuse_preparations_and_answer() {
        let dir = std::env::temp_dir().join(format!("sprawl-mcp-history-{}", crate::store::nonce()));
        let base = dir.join("history");
        let taken = SystemTime::now();
        for folder in ["alpha", "beta"] {
            let root = dir.join(folder);
            std::fs::create_dir_all(&root).unwrap();
            let tree = sprawl_core::scan(&root, |_| {}).unwrap();
            crate::history::prepare(&tree, taken, true).save(&base, u64::MAX).unwrap();
        }
        let mut entries: Vec<_> = crate::history::list(&base).into_iter().filter(|e| e.full).collect();
        entries.sort_by(|a, b| a.root.cmp(&b.root));
        assert_eq!(entries.len(), 2);
        let index = Index::for_app(false);
        let memory = Memory::default();
        let mut first_trees = Vec::new();
        for e in &entries {
            let held = index.history_tree(e.clone()).unwrap();
            assert!(matches!(held.facts_with(&index.preparers, |_| Facts::default()), Err(e) if e == GATHERING_FACTS));
            first_trees.push(held);
        }
        let until = Instant::now() + Duration::from_secs(3);
        while first_trees.iter().any(|h| h.facts.ready.get().is_none()) {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(10));
        }
        for held in &first_trees {
            loop {
                match held.findings_with(&index.preparers, |_, _| Vec::new()) {
                    Ok(_) => break,
                    Err(_) => assert!(Instant::now() < until),
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        for (entry, original) in entries.iter().zip(&first_trees) {
            let held = index.history_tree(entry.clone()).unwrap();
            assert!(Arc::ptr_eq(&held, original), "A/B/A/B must not recreate a held tree");
            let answer = held.call("findings", &json!({}), &memory, true, &Cancel::default(), &index.preparers).unwrap();
            assert!(answer.contains(&entry.root.display().to_string()) && answer.contains("none matched"), "{answer}");
        }
        // The same filename with a different native identity must not get the old preparation.
        let copy = entries[0].file.with_extension("old");
        std::fs::rename(&entries[0].file, &copy).unwrap();
        std::fs::copy(&copy, &entries[0].file).unwrap();
        let replacement = crate::history::list(&base).into_iter().find(|e| e.file == entries[0].file).unwrap();
        let held = index.history_tree(replacement).unwrap();
        assert!(!Arc::ptr_eq(&held, &first_trees[0]) && held.facts.ready.get().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn retired_scan_cannot_delete_a_snapshot_after_validation_resumes() {
        let dir = std::env::temp_dir().join(format!("sprawl-mcp-delete-{}", crate::store::nonce()));
        let root = dir.join("data");
        let base = dir.join("history");
        std::fs::create_dir_all(&root).unwrap();
        let tree = sprawl_core::scan(&root, |_| {}).unwrap();
        crate::history::prepare(&tree, SystemTime::now(), false).save(&base, u64::MAX).unwrap();
        let entry = crate::history::list(&base).pop().unwrap();
        let file = entry.file.clone();
        let index = Index::for_app(false);
        let cancel = Arc::new(Cancel::default());
        let work = cancel.clone();
        let (entered, validated) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let (send, outcome) = std::sync::mpsc::channel();
        let id = index.state().start(true, root, true, cancel.clone(), move || {
            let result = crate::history::remove_entry_checked(&entry, &|| {
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
                work.check().map_err(|_| std::io::ErrorKind::Interrupted.into())
            });
            send.send(result).unwrap();
            Progress::new()
        }).unwrap();
        validated.recv_timeout(Duration::from_secs(2)).unwrap();
        cancel.stop();
        let status = index.scan_now("scan_status", &json!({"job": id}), true, Arc::new(Cancel::default())).unwrap();
        assert!(status.failed && !status.running);
        assert_eq!(index.state().retired.len(), 1);
        release.send(()).unwrap();
        assert_eq!(outcome.recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        assert!(file.is_file(), "a retired scan must leave the validated snapshot alone");
        for handle in index.state().retired.drain(..) { assert!(handle.join().unwrap().is_err()); }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scans_a_folder_and_answers_from_it() {
        let dir = std::env::temp_dir().join(format!("sprawl-mcp-scan-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        std::fs::write(dir.join("inner").join("a.bin"), vec![0u8; 5000]).unwrap();
        let ix = Index { state: Mutex::new(ScanState { no_history: true, ..Default::default() }), ..Default::default() };
        let p = dir.to_string_lossy().to_string();
        let ask = |name, input: Value| ix.scan_now(name, &input, false, Arc::new(Cancel::default())).unwrap();
        let wait = |mut reply: ScanReply| {
            let until = Instant::now() + Duration::from_secs(10);
            while reply.running {
                assert!(Instant::now() < until, "{}", reply.text);
                std::thread::sleep(Duration::from_millis(20));
                reply = ask("scan_status", json!({"job": reply.job}));
            }
            reply.text
        };
        let first = wait(ask("scan_path", json!({"path": p})));
        assert!(first.contains("done"), "{first}");
        assert!(first.contains("Not saved in History"), "{first}");
        let again = wait(ask("scan_path", json!({"path": format!("{p}\\inner")})));
        assert!(again.contains("rescan"), "{again}");
        assert!(ix.has_scan_for(&dir.join("inner")));
        let memory = Memory::default();
        let until = Instant::now() + Duration::from_secs(10);
        let kids = loop {
            match ix.answer("list_children", &json!({"path": format!("{p}\\inner")}), &memory, false) {
                Err(e) if e == GATHERING_FACTS && Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
                result => break result.unwrap(),
            }
        };
        assert!(kids.contains("a.bin") && kids.contains("made through this server"), "{kids}");
        let none = ix.answer("list_children", &json!({"path": r"Q:\elsewhere"}), &memory, false).unwrap_err();
        assert!(none.contains("call scan_path"), "{none}");
        assert!(ask("list_scans", json!({})).text.contains(&p));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expired_preflight_releases_the_slot_while_the_worker_is_still_blocked() {
        let dir = std::env::temp_dir().join(format!("sprawl-mcp-preflight-{}", crate::store::nonce()));
        let first_root = dir.join("first");
        let second_root = dir.join("second");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir(&second_root).unwrap();
        let index = Index::for_app(false);
        let client = Cancel::default();
        let cancel = Arc::new(Cancel::import(&client.export().unwrap()).unwrap());
        let (entered, waiting) = std::sync::mpsc::channel();
        let (release, done) = std::sync::mpsc::channel();
        let job = index.state().start_with(true, first_root.clone(), true, cancel, Duration::from_millis(500), move || {
            entered.send(()).unwrap();
            let _ = done.recv_timeout(Duration::from_secs(5));
            Progress::new()
        }).unwrap();
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        let status = || index.scan_now("scan_status", &json!({"job": job}), true, Arc::new(Cancel::default())).unwrap();
        let first = status();
        assert!(app_job(job) && first.running);
        assert!(first.text.contains("still scanning") && first.text.contains("folders to skip"), "{}", first.text);
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let expired = status();
            if !expired.running {
                assert!(expired.failed && expired.text.contains("folders to skip within"), "{}", expired.text);
                break;
            }
            assert!(Instant::now() < until, "{}", expired.text);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(index.state().retired.len(), 1);
        // Use the normal scan_path path while the first worker still has not returned.
        let next = index.scan_now("scan_path", &json!({"path": second_root}), true, Arc::new(Cancel::default())).unwrap();
        assert_ne!(next.job, Some(job));
        assert!(next.started && !next.failed, "{}", next.text);
        release.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while index.scan_now("scan_status", &json!({"job": next.job}), true, Arc::new(Cancel::default())).unwrap().running
            || !index.state().retired.is_empty() {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!index.has_scan_for(&first_root) && index.has_scan_for(&second_root));
        assert!(status().failed, "late worker exit did not alter the retired job");
        std::fs::remove_dir(&first_root).unwrap();
        std::fs::remove_dir(&second_root).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn cancelled_scan_workers_are_capped_until_their_io_unwinds() {
        let dir = std::env::temp_dir().join(format!("sprawl-mcp-retired-{}", crate::store::nonce()));
        std::fs::create_dir(&dir).unwrap();
        let mut state = ScanState { no_history: true, ..Default::default() };
        let mut releases = Vec::new();
        for _ in 0..MAX_RETIRED {
            let client = Cancel::default();
            let cancel = Arc::new(Cancel::import(&client.export().unwrap()).unwrap());
            let (ready, entered) = std::sync::mpsc::channel();
            let (release, done) = std::sync::mpsc::channel();
            let job = state.start(true, dir.clone(), true, cancel, move || {
                ready.send(()).unwrap();
                let _ = done.recv_timeout(Duration::from_secs(5));
                Progress::new()
            }).unwrap();
            entered.recv_timeout(Duration::from_secs(2)).unwrap();
            client.stop();
            state.poll(true);
            let status = state.status(job, false).unwrap();
            assert!(status.failed && status.text.contains("cancelled"));
            releases.push(release);
        }
        let e = state.start(true, dir.clone(), true, Arc::new(Cancel::default()), Progress::new).unwrap_err();
        assert!(e.contains("too many stopped scans"), "{e}");
        for release in releases { release.send(()).unwrap(); }
        for handle in state.retired { assert!(handle.join().unwrap().is_err()); }
        std::fs::remove_dir(&dir).unwrap();
    }
}
