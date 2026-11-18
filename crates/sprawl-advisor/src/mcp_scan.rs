//! The MCP's own scans and the trees it answers from (next round, Phase 1).
//!
//! `scan_path` scans a folder or drive with the folder walk and the user's own rights, never the
//! MFT and never elevated (the user's decision, 2026-11-18: an AI's scan sees no more than the
//! user could list). The running app makes it when it isn't elevated; an elevated app hands it
//! back to the server's own process ([`PASS_TO_SERVER`]). `scan_status` follows the scan; `list_scans` lists every tree the other tools can
//! answer from (this session's scans and History's saved ones). A finished scan goes into
//! History when the app's setting keeps History, so the user sees it there.
//!
//! Every tool here stays read-only: a scan reads names and sizes, nothing else.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};
use sprawl_core::{Progress, Tree};

use crate::facts::Facts;
use crate::finding::Finding;
use crate::llm::tools::Tools;
use crate::llm::ToolSpec;
use crate::memory::Memory;

/// The longest `wait_seconds` honoured: MCP clients give up on a call after about a minute.
const MAX_WAIT: u64 = 50;
/// Trees this process scanned that it keeps (a whole drive can take a few hundred MB).
const KEEP_SCANS: usize = 3;
/// Older than this, an answer suggests scanning again.
const STALE_SECS: u64 = 86_400;

/// The answer the tools give (here and in the app) for a path outside the tree they have.
pub const NOT_IN_SCAN: &str = "isn't in the scan";
/// The elevated app's answer to a scan tool: the server scans in its own process instead.
pub const PASS_TO_SERVER: &str = "Sprawl runs as administrator, so the server scans with your own rights instead.";

/// The app's answer when it has no tree open.
pub const NO_SCAN_OPEN: &str = "No scan is open in Sprawl yet.";

pub fn scan_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "scan_path",
            description: "Scan a folder or a whole drive (such as D:\\ or C:\\Users\\me\\Projects) so the other tools can answer about it. \
                Returns a job number. With wait_seconds (up to 50) it waits that long and answers with the result when the scan is done \
                in time; a folder usually is, a whole drive can take minutes (follow it with scan_status). A path already scanned isn't \
                scanned again unless rescan is true. Network paths are refused. Read-only: a scan reads names and sizes.",
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

/// `root` holds `path` (same folder or inside it), compared as the tools compare paths.
fn covers(root: &Path, path: &Path) -> bool {
    let r = crate::segments(&root.to_string_lossy());
    let p = crate::segments(&path.to_string_lossy());
    p.len() >= r.len() && p[..r.len()] == r[..]
}

/// The `path` a call names, if any.
pub fn path_of(input: &Value) -> Option<PathBuf> {
    let p = input["path"].as_str()?.trim().trim_matches('"').replace('/', "\\");
    (!p.is_empty()).then(|| PathBuf::from(p))
}

/// Why `path` can't be scanned, if it can't.
fn refusal(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return Some(format!("{} isn't a full path: give one such as D:\\ or C:\\Users\\me\\Projects.", path.display()));
    }
    let pinned = match sprawl_core::local::directory(path) {
        Ok(p) => p,
        Err(e) => return Some(format!("{} isn't a plain local folder Sprawl can open; network, device and redirected paths are refused: {e}", path.display())),
    };
    let root = pinned.path.clone();
    let s = root.to_string_lossy();
    if s.starts_with(r"\\") && !s.starts_with(r"\\?\Volume") {
        return Some(format!("{} is a network path: Sprawl doesn't scan those.", path.display()));
    }
    if is_network_drive(&s) {
        return Some(format!("{} is on a network drive: Sprawl doesn't scan those.", path.display()));
    }
    if !root.is_dir() {
        return Some(format!("{} isn't a folder or drive Sprawl can open.", path.display()));
    }
    None
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

/// The app's History settings: whether it keeps History, and its cap in bytes.
fn history_settings() -> (bool, u64) {
    let text = std::env::var_os("APPDATA").and_then(|a| std::fs::read_to_string(PathBuf::from(a).join("Sprawl").join("settings.ini")).ok()).unwrap_or_default();
    let get = |k: &str| text.trim_start_matches('\u{feff}').lines().filter_map(|l| l.split_once('=')).find(|(a, _)| a.trim() == k).map(|(_, v)| v.trim().to_string());
    let on = get("snapshots").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let mb = get("snapshot_cap_mb").and_then(|v| v.parse::<f64>().ok()).unwrap_or(1024.0);
    (on, (mb * 1024.0 * 1024.0) as u64)
}

/// A tree this process holds: one it scanned, or one loaded from History.
pub struct Held {
    pub root: PathBuf,
    pub taken: SystemTime,
    /// History's file it was loaded from; `None` for a scan made here.
    file: Option<PathBuf>,
    tree: Tree,
    facts: Option<Facts>,
    found: Option<Vec<Finding>>,
    /// Scanned by the running app (in the background), not by the server's own process.
    in_app: bool,
}

impl Held {
    fn new(tree: Tree, taken: SystemTime, file: Option<PathBuf>, in_app: bool) -> Held {
        Held { root: tree.root_path.clone(), taken, file, tree, facts: None, found: None, in_app }
    }

    fn facts(&mut self) -> &Facts {
        let tree = &self.tree;
        self.facts.get_or_insert_with(|| {
            let mut f = Facts::gather();
            f.attach_usage(tree);
            f
        })
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

    fn call(&mut self, name: &str, input: &Value, memory: &Memory, app_running: bool) -> Result<String, String> {
        let note = self.note(app_running);
        self.facts();
        let facts = self.facts.as_ref().expect("made above");
        let answer = match name {
            "scan_report" | "findings" => {
                let found = self.found.get_or_insert_with(|| crate::mcp::rules_for(&self.tree, facts));
                Ok(if name == "findings" { crate::mcp::findings_list(found) } else { crate::llm::report::build(&self.tree, facts, Some(found), None, None) })
            }
            "cleaners" | "set_filter" if app_running => Err(format!("That works only for the scan open in Sprawl; {} isn't in it.", self.root.display())),
            "cleaners" | "set_filter" => Err("That needs Sprawl open: start Sprawl and ask again.".to_string()),
            _ => Tools::new(&self.tree, facts, "claude-code", &[], memory).at_scan(self.taken).call(name, input),
        };
        match answer {
            Ok(t) => Ok(note + &t),
            Err(e) => Err(note + &e),
        }
    }
}

enum JobState {
    Running {
        progress: Arc<Progress>,
        saving: Arc<AtomicBool>,
        handle: std::thread::JoinHandle<Result<(Tree, String), String>>,
    },
    Done {
        files: u64,
        bytes: u64,
        secs: f64,
        method: &'static str,
        history: String,
    },
    Failed(String),
}

struct Job {
    id: u64,
    root: PathBuf,
    started: Instant,
    taken: SystemTime,
    state: JobState,
}

/// This process's scans and the trees it answers from.
#[derive(Default)]
pub struct Index {
    jobs: Vec<Job>,
    /// Scans made here (newest last), then at most one tree loaded from History.
    held: Vec<Held>,
    next: u64,
    /// Tests and screenshot runs: History is neither read nor written.
    no_history: bool,
    /// Kept by the running app for every server that asks it (its scans use the app's rights,
    /// the MFT when it runs as administrator, and stay off the user's screen).
    in_app: bool,
    /// The app's own scan is running: a second one waits for it.
    busy: bool,
}

impl Index {
    /// The app's index: `history` false in a screenshot run, which writes nothing.
    pub fn for_app(history: bool) -> Index {
        Index { no_history: !history, in_app: true, ..Default::default() }
    }

    /// Before each request the app says whether its own scan is running (`scan_path` waits
    /// for it) and whether it may write History (not in a screenshot run).
    pub fn set_app_state(&mut self, busy: bool, history: bool) {
        self.busy = busy;
        self.no_history = !history;
    }

    fn base(&self) -> Option<PathBuf> {
        if self.no_history { None } else { crate::history::base_dir() }
    }

    /// A finished scan becomes a held tree. Called before every answer.
    fn poll(&mut self) {
        for j in &mut self.jobs {
            let JobState::Running { handle, .. } = &j.state else { continue };
            if !handle.is_finished() {
                continue;
            }
            let JobState::Running { progress, handle, .. } = std::mem::replace(&mut j.state, JobState::Failed(String::new())) else { unreachable!() };
            j.state = match handle.join() {
                Ok(Ok((tree, history))) => {
                    let s = progress.snapshot();
                    let done = JobState::Done { files: s.files, bytes: s.bytes, secs: j.started.elapsed().as_secs_f64(), method: tree.method.label(), history };
                    // a scan of the same root replaces the older one
                    self.held.retain(|h| !(h.file.is_none() && h.root == tree.root_path));
                    let at = self.held.iter().take_while(|h| h.file.is_none()).count();
                    self.held.insert(at, Held::new(tree, j.taken, None, self.in_app));
                    let scans = self.held.iter().filter(|h| h.file.is_none()).count();
                    if scans > KEEP_SCANS {
                        self.held.remove(0);
                    }
                    done
                }
                Ok(Err(e)) => JobState::Failed(e),
                Err(_) => JobState::Failed("the scan stopped with an internal error".into()),
            };
        }
    }

    fn running(&self) -> Option<&Job> {
        self.jobs.iter().find(|j| matches!(j.state, JobState::Running { .. }))
    }

    /// Answer `scan_path`, `scan_status` or `list_scans`.
    pub fn scan_call(&mut self, name: &str, input: &Value) -> Result<String, String> {
        if !is_scan_tool(name) { return Err(format!("there is no tool called {name}")); }
        // an app running as administrator would scan with its rights: the server does it
        if self.in_app && !sprawl_core::elevation().is_ok_and(|elevated| !elevated) {
            return Err(PASS_TO_SERVER.into());
        }
        self.poll();
        let wait = input["wait_seconds"].as_u64().unwrap_or(0).min(MAX_WAIT);
        match name {
            "scan_path" => {
                let path = path_of(input).ok_or("give a `path`: a drive such as D:\\ or a folder")?;
                if let Some(r) = refusal(&path) {
                    return Err(r);
                }
                let pinned = sprawl_core::local::directory(&path).map_err(|e| format!("The scan root changed or cannot be opened: {e}"))?;
                let root = pinned.path.clone();
                if let Some(j) = self.running() {
                    if j.root == root {
                        let id = j.id;
                        return Ok(self.wait_for(id, wait));
                    }
                    return Err(format!("Scan {} of {} is still running; one scan at a time. Follow it with scan_status.", j.id, j.root.display()));
                }
                if self.busy {
                    return Err("Sprawl is scanning on its screen right now; one scan at a time. Ask again when it has finished.".into());
                }
                if input["rescan"].as_bool() != Some(true) {
                    if let Some(h) = self.held.iter().rfind(|h| h.file.is_none() && covers(&h.root, &root)) {
                        return Ok(format!("{} is in the scan of {} made {}; the other tools answer from it. Pass rescan: true to scan again.", root.display(), h.root.display(), age(h.taken)));
                    }
                }
                let id = self.start(root);
                Ok(self.wait_for(id, wait))
            }
            "scan_status" => {
                let id = match input["job"].as_u64() {
                    Some(id) => id,
                    None => self.jobs.last().map(|j| j.id).ok_or("No scan has been started here: call scan_path.")?,
                };
                if !self.jobs.iter().any(|j| j.id == id) {
                    return Err(format!("There is no scan {id}."));
                }
                Ok(self.wait_for(id, wait))
            }
            "list_scans" => Ok(self.list()),
            _ => Err(format!("there is no tool called {name}")),
        }
    }

    fn start(&mut self, root: PathBuf) -> u64 {
        self.next += 1;
        let id = self.next;
        let progress = crate::exclusions::progress();
        let saving = Arc::new(AtomicBool::new(false));
        let (p, sv, r) = (progress.clone(), saving.clone(), root.clone());
        let taken = SystemTime::now();
        let base = self.base();
        let quiet = self.no_history;
        let handle = std::thread::spawn(move || {
            // the folder walk, never the MFT: only what the user's own rights can list
            let tree = sprawl_core::walk::walk(&r, &p).map_err(|e| format!("The scan of {} failed: {e}", r.display()))?;
            // into History as the app saves its own scans, when the user keeps History
            let (on, cap) = history_settings();
            let history = match (on, base) {
                (true, Some(base)) => {
                    sv.store(true, Ordering::Relaxed);
                    match crate::history::prepare(&tree, taken, true).save(&base, cap) {
                        Ok(()) => "Saved in Sprawl's History.".to_string(),
                        Err(e) => format!("Not saved in History: {e}."),
                    }
                }
                (false, _) => "Not saved in History (History is off in Sprawl's settings).".to_string(),
                (true, None) if quiet => "Not saved in History (this run of Sprawl writes nothing).".to_string(),
                (true, None) => "Not saved in History (no place for it).".to_string(),
            };
            Ok((tree, history))
        });
        self.jobs.push(Job { id, root, started: Instant::now(), taken, state: JobState::Running { progress, saving, handle } });
        id
    }

    /// The state of job `id`, after waiting up to `wait` seconds for it to finish.
    fn wait_for(&mut self, id: u64, wait: u64) -> String {
        let until = Instant::now() + Duration::from_secs(wait);
        loop {
            self.poll();
            let running = self.jobs.iter().any(|j| j.id == id && matches!(j.state, JobState::Running { .. }));
            if !running || Instant::now() >= until {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let Some(j) = self.jobs.iter().find(|j| j.id == id) else { return format!("There is no scan {id}.") };
        let head = format!("Scan {} of {}: ", j.id, j.root.display());
        match &j.state {
            JobState::Running { progress, saving, .. } => {
                let s = progress.snapshot();
                let stage = if saving.load(Ordering::Relaxed) { "saving to History" } else if s.phase.is_empty() { "starting" } else { s.phase };
                let frac = s.fraction.map(|f| format!(", about {:.0}% done", f * 100.0)).unwrap_or_default();
                format!(
                    "{head}running ({stage}{frac}, {}): {} files, {} folders, {} so far, {:.0} s. Call scan_status with job {} (and wait_seconds) to follow it.",
                    s.method.label(),
                    s.files,
                    s.folders,
                    sprawl_core::format_size(s.bytes),
                    j.started.elapsed().as_secs_f64(),
                    j.id
                )
            }
            JobState::Done { files, bytes, secs, method, history } => format!(
                "{head}done in {secs:.1} s ({method}): {files} files, {}. {history} The other tools now answer about it: pass a path inside it, or call scan_report with path {}.",
                sprawl_core::format_size(*bytes),
                j.root.display()
            ),
            JobState::Failed(e) => format!("{head}failed. {e}"),
        }
    }

    fn list(&self) -> String {
        let mut s = String::new();
        for h in self.held.iter().filter(|h| h.file.is_none()) {
            s += &format!("- {} (scanned here {}): {}\n", h.root.display(), age(h.taken), sprawl_core::format_size(h.tree.nodes[sprawl_core::ROOT].size));
        }
        if let Some(j) = self.running() {
            s += &format!("- {} (scan {} running)\n", j.root.display(), j.id);
        }
        let base = self.base();
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
    pub fn has_scan_for(&mut self, path: &Path) -> bool {
        self.poll();
        self.held.iter().any(|h| h.file.is_none() && covers(&h.root, path))
    }

    /// Answer a query tool from the freshest tree that holds the call's path (any tree when it
    /// names none): a scan made here, or History's newest full snapshot.
    pub fn answer(&mut self, name: &str, input: &Value, memory: &Memory, app_running: bool) -> Result<String, String> {
        if !crate::mcp::bridge_specs().iter().any(|t| t.name == name) {
            return Err(format!("there is no tool called {name}"));
        }
        self.poll();
        let path = path_of(input);
        let i = self.pick(path.as_deref()).ok_or_else(|| match &path {
            Some(p) => format!("{} isn't in any scan Sprawl has: call scan_path with it (seconds for a folder, minutes for a whole drive).", p.display()),
            None if app_running => format!("{NO_SCAN_OPEN} Call scan_path with a drive or folder."),
            None => "Sprawl isn't running and has no saved scan yet: call scan_path with a drive or folder.".into(),
        })?;
        self.held[i].call(name, input, memory, app_running)
    }

    /// The index in `held` of the tree to answer from, loading it from History when that is
    /// the freshest.
    fn pick(&mut self, path: Option<&Path>) -> Option<usize> {
        let ours = self.held.iter().enumerate().filter(|(_, h)| h.file.is_none() && path.is_none_or(|p| covers(&h.root, p))).max_by_key(|(_, h)| h.taken).map(|(i, h)| (i, h.taken));
        let saved = self.base().and_then(|base| {
            let list = match path {
                Some(p) => crate::history::covering(&base, p),
                None => crate::history::list(&base),
            };
            list.into_iter().filter(|e| e.full).max_by_key(|e| e.taken)
        });
        match (ours, saved) {
            (Some((i, t)), Some(e)) if t >= e.taken => Some(i),
            (Some((i, _)), None) => Some(i),
            (_, Some(e)) => {
                if let Some(i) = self.held.iter().position(|h| h.file.as_ref() == Some(&e.file)) {
                    return Some(i);
                }
                // a History scan saved by this process is the scan made here: same moment
                if let Some((i, t)) = ours {
                    if t == e.taken {
                        return Some(i);
                    }
                }
                let tree = crate::history::load(&e).ok()?;
                // one tree from History at a time
                self.held.retain(|h| h.file.is_none());
                self.held.push(Held::new(tree, e.taken, Some(e.file.clone()), self.in_app));
                Some(self.held.len() - 1)
            }
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_what_it_shouldnt_scan() {
        assert!(refusal(Path::new("relative\\dir")).is_some_and(|r| r.contains("full path")));
        assert!(refusal(Path::new(r"\\server\share")).is_some_and(|r| r.contains("network")));
        assert!(refusal(Path::new(r"C:\no such folder here 7f3a")).is_some_and(|r| r.contains("can open")));
        assert!(refusal(&std::env::temp_dir()).is_none());
    }

    #[test]
    fn covers_by_segments() {
        assert!(covers(Path::new(r"C:\"), Path::new(r"c:\Users\x")));
        assert!(covers(Path::new(r"C:\Users"), Path::new(r"C:\users")));
        assert!(!covers(Path::new(r"C:\Users"), Path::new(r"C:\Users2")));
        assert!(!covers(Path::new(r"D:\"), Path::new(r"C:\x")));
    }

    #[test]
    fn scans_a_folder_and_answers_from_it() {
        let dir = std::env::temp_dir().join(format!("sprawl-mcp-scan-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        std::fs::write(dir.join("inner").join("a.bin"), vec![0u8; 5000]).unwrap();
        let mut ix = Index { no_history: true, ..Default::default() };
        let p = dir.to_string_lossy().to_string();
        let first = ix.scan_call("scan_path", &json!({"path": p, "wait_seconds": 30})).unwrap();
        assert!(first.contains("done"), "{first}");
        assert!(first.contains("Not saved in History"), "{first}");
        let again = ix.scan_call("scan_path", &json!({"path": format!("{p}\\inner")})).unwrap();
        assert!(again.contains("rescan"), "{again}");
        assert!(ix.has_scan_for(&dir.join("inner")));
        let memory = Memory::default();
        let kids = ix.answer("list_children", &json!({"path": format!("{p}\\inner")}), &memory, false).unwrap();
        assert!(kids.contains("a.bin") && kids.contains("made through this server"), "{kids}");
        let none = ix.answer("list_children", &json!({"path": r"Q:\elsewhere"}), &memory, false).unwrap_err();
        assert!(none.contains("call scan_path"), "{none}");
        assert!(ix.scan_call("list_scans", &json!({})).unwrap().contains(&p));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
