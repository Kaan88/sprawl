//! The running app's side of `sprawl --mcp`: a thread serving the named pipe with the scan on
//! screen, the Advisor's findings and the user's memory. A filter a model asks to show is
//! queued for the UI thread, which applies it as if typed.

use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::sync::mpsc;
use std::time::Instant;

use sprawl_advisor::facts::Facts;
use sprawl_advisor::memory::Memory;
use sprawl_core::Tree;

#[derive(Default)]
struct Shared {
    /// Weak: the app's tree isn't kept alive (or copied on every sort) for a pipe nobody uses.
    tree: Weak<Tree>,
    map_tree: Weak<Tree>,
    revision: u64,
    scan_before: Option<std::time::SystemTime>,
    findings: String,
    /// The Advisor's findings as a list, for the report's "found by the rules" (None: not yet).
    found: Option<Arc<Vec<sprawl_advisor::finding::Finding>>>,
    facts: Option<Arc<Facts>>,
    memory: Memory,
    filters: Vec<(u64, Weak<Tree>, String)>,
    /// The app's own scan is running (don't start a second one).
    scanning: bool,
}

fn same_tree(weak: &Weak<Tree>, tree: Option<&Arc<Tree>>) -> bool {
    match (weak.upgrade(), tree) { (Some(a), Some(b)) => Arc::ptr_eq(&a, b), (None, None) => true, _ => false }
}

impl Shared {
    fn tree(&mut self, tree: Option<&Arc<Tree>>, before: std::time::SystemTime, map: Option<&Arc<Tree>>) -> bool {
        let time = tree.map(|_| before);
        let changed = !same_tree(&self.tree, tree) || self.scan_before != time;
        self.map_tree = map.map(Arc::downgrade).unwrap_or_default();
        if changed {
            self.revision = self.revision.wrapping_add(1);
            self.tree = tree.map(Arc::downgrade).unwrap_or_default();
            self.scan_before = time;
            self.findings.clear();
            self.found = None;
            self.facts = None;
            self.filters.clear();
        }
        changed
    }

    fn queue_filter(&mut self, revision: u64, tree: Option<&Arc<Tree>>, filter: String) -> bool {
        if self.revision != revision || !same_tree(&self.tree, tree) || !same_tree(&self.map_tree, tree) { return false; }
        self.filters.push((revision, tree.map(Arc::downgrade).unwrap_or_default(), filter));
        true
    }

    fn take_filters(&mut self, map: Option<&Arc<Tree>>) -> Vec<String> {
        std::mem::take(&mut self.filters).into_iter().filter_map(|(revision, tree, filter)|
            (revision == self.revision && same_tree(&tree, map)).then_some(filter)).collect()
    }
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Shared::default()))
}

/// Serve the pipe on its own thread (a second Sprawl window just doesn't get it).
pub(crate) fn start() {
    std::thread::spawn(|| {
        use sprawl_advisor::mcp::{bridge_request_options, wait_for_bridge_reply, Workers};
        let facts = Arc::new(Mutex::new(None));
        let index = Arc::new(sprawl_advisor::mcp_scan::Index::for_app(false));
        let workers = Workers::new(4);
        let error = |text: &str| serde_json::json!({"ok": false, "text": text}).to_string();
        let served = sprawl_advisor::pipe::serve(|line, client_elevated| {
            let received = Instant::now();
            let (cancel, scan_tool, wait) = match bridge_request_options(line) { Ok(req) => req, Err(e) => return error(&e) };
            let until = received + wait;
            if Instant::now() >= until { return error("Sprawl's window ran out of time answering; try again."); }
            let (send, recv) = mpsc::channel();
            let (line, index, facts, work) = (line.to_string(), index.clone(), facts.clone(), cancel.clone());
            if let Err(e) = workers.spawn(move || {
                let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| answer(&line, client_elevated, scan_tool, &index, &facts, work)))
                    .unwrap_or_else(|_| error("Sprawl hit an internal error answering that."));
                let _ = send.send(out);
            }) { return error(&e); }
            wait_for_bridge_reply(&cancel, until, recv).unwrap_or_else(|e| error(&e))
        });
        // the pipe's name was taken: by another Sprawl window (normal), or by a program of
        // another account that made it first (clients refuse that server; Fable AI review
        // P3-10). Either way the bridge is off in this window, and the log says why.
        if let Err(e) = served {
            if let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("Sprawl")) {
                let msg = format!("The MCP bridge didn't start in this window ({e}): another Sprawl window has it, or another program took its name first.
");
                let _ = sprawl_advisor::write_local(&dir.join("bridge.log"), msg.as_bytes(), true);
            }
        }
    });
}

type FactsCache = Mutex<Option<(Weak<Tree>, Arc<Facts>)>>;

fn answer(line: &str, client_elevated: bool, scan_tool: bool, index: &sprawl_advisor::mcp_scan::Index, facts: &FactsCache, cancel: Arc<sprawl_advisor::mcp_cancel::Cancel>) -> String {
    use sprawl_advisor::mcp::answer_bridge_cancellable;
    // An elevated app reads the disk for an unelevated client only when the user allowed
    // it; an elevation Windows can't confirm counts as elevated.
    let disk = client_elevated || !sprawl_core::elevation().unwrap_or(true) || crate::admin::store::get(crate::admin::MCP_DISK.key);
    if scan_tool {
        let scanning = shared().lock().unwrap_or_else(|e| e.into_inner()).scanning;
        index.set_app_state(scanning, !crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed));
        return answer_bridge_cancellable(line, None, None, None, &Memory::default(), "", None, index, disk, &mut |_| {}, cancel);
    }
    let (tree, revision, scan_before, findings, found, report_facts, memory) = {
        let s = shared().lock().unwrap_or_else(|e| e.into_inner());
        (s.tree.upgrade(), s.revision, s.scan_before, s.findings.clone(), s.found.clone(), s.facts.clone(), s.memory.clone())
    };
    // A restricted client still sees only what the app has already published about this tree.
    let gathered;
    let scoped;
    let f = if disk {
        gathered = report_facts.or_else(|| {
            let cached = facts.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
                .filter(|(weak, _)| same_tree(weak, tree.as_ref())).map(|(_, f)| f.clone());
            if cached.is_some() || cancel.stopped() { return cached; }
            let mut f = if tree.is_some() { Facts::gather() } else { Facts::default() };
            if let Some(t) = &tree { f.attach_usage(t); }
            let f = Arc::new(f);
            *facts.lock().unwrap_or_else(|e| e.into_inner()) = Some((tree.as_ref().map(Arc::downgrade).unwrap_or_default(), f.clone()));
            Some(f)
        });
        gathered.as_deref()
    } else {
        scoped = report_facts.as_deref().zip(tree.as_deref()).map(|(f, t)| f.scoped_to(t));
        scoped.as_ref()
    };
    let mut wanted = None;
    let out = answer_bridge_cancellable(line, tree.as_deref(), scan_before, f, &memory, &findings, found.as_deref().map(|v| v.as_slice()), index, disk, &mut |x| wanted = Some(x), cancel.clone());
    if let Some(x) = wanted {
        let mut s = shared().lock().unwrap_or_else(|e| e.into_inner());
        if cancel.stopped() { return r#"{"ok":false,"text":"Sprawl cancelled the request."}"#.into(); }
        if !s.queue_filter(revision, tree.as_ref(), x) {
            return r#"{"ok":false,"text":"That scan changed or is not the tree displayed on the map; no filter was applied."}"#.into();
        }
    }
    out
}

/// Called every frame with the tree on screen (cheap: only a change is stored).
pub(crate) fn publish_tree(t: Option<&Arc<Tree>>, scan_before: std::time::SystemTime, map: Option<&Arc<Tree>>) -> bool {
    let mut s = shared().lock().unwrap_or_else(|e| e.into_inner());
    s.tree(t, scan_before, map)
}

/// Called every frame: whether the app's own scan is running.
pub(crate) fn publish_scanning(on: bool) {
    shared().lock().unwrap_or_else(|e| e.into_inner()).scanning = on;
}

/// The Advisor's findings as text and as a list (`None`: a new analysis has started).
pub(crate) fn publish_findings(tree: &Arc<Tree>, before: std::time::SystemTime, map: Option<&Arc<Tree>>, facts: &Arc<Facts>, text: String, found: &[sprawl_advisor::finding::Finding]) {
    let mut s = shared().lock().unwrap_or_else(|e| e.into_inner());
    s.tree(Some(tree), before, map);
    s.findings = text;
    s.found = Some(Arc::new(found.to_vec()));
    s.facts = Some(facts.clone());
}

pub(crate) fn clear_findings() {
    let mut s = shared().lock().unwrap_or_else(|e| e.into_inner());
    s.findings.clear();
    s.found = None;
    s.facts = None;
}

pub(crate) fn publish_memory(m: &Memory) {
    shared().lock().unwrap_or_else(|e| e.into_inner()).memory = m.clone();
}

/// Filters asked for since the last call.
pub(crate) fn take_filters(map: Option<&Arc<Tree>>) -> Vec<String> {
    shared().lock().unwrap_or_else(|e| e.into_inner()).take_filters(map)
}

#[cfg(test)]
mod context_tests {
    use super::*;
    fn tree() -> Arc<Tree> { Arc::new(Tree { volumes: Vec::new(), root_path: r"C:\Data".into(), nodes: vec![sprawl_core::Node { is_dir: true, ..Default::default() }], errors: 0, method: sprawl_core::ScanMethod::Walk }) }

    #[test]
    fn switching_source_drops_reports_and_filters() {
        let (a, b) = (tree(), tree());
        let now = std::time::SystemTime::now();
        let mut shared = Shared::default();
        assert!(shared.tree(Some(&a), now, Some(&a)));
        shared.findings = "report for A".into();
        shared.found = Some(Arc::new(Vec::new()));
        shared.facts = Some(Arc::new(Facts::default()));
        let revision = shared.revision;
        assert!(shared.queue_filter(revision, Some(&a), "*.txt".into()));
        assert!(shared.tree(Some(&b), now, Some(&a)));
        assert!(shared.findings.is_empty() && shared.found.is_none() && shared.facts.is_none());
        assert!(shared.take_filters(Some(&a)).is_empty());
        assert!(!shared.queue_filter(revision, Some(&a), "old response".into()));
        assert!(!shared.queue_filter(shared.revision, Some(&b), "own-read filter".into()));
        assert!(!same_tree(&Arc::downgrade(&a), Some(&b)), "equal root spellings do not identify a tree");
    }

    #[test]
    fn filters_are_rechecked_at_ui_delivery() {
        let (a, b) = (tree(), tree());
        let mut shared = Shared::default();
        shared.tree(Some(&a), std::time::SystemTime::now(), Some(&a));
        assert!(shared.queue_filter(shared.revision, Some(&a), "*.txt".into()));
        assert!(shared.take_filters(Some(&b)).is_empty());
        assert!(shared.queue_filter(shared.revision, Some(&a), "*.rs".into()));
        assert_eq!(shared.take_filters(Some(&a)), vec!["*.rs"]);
        let weak = Arc::downgrade(&a);
        drop(a);
        assert!(!same_tree(&weak, Some(&b)), "an expired cache identity cannot match a new allocation");
    }
}

/// The line that adds Sprawl to the user's Claude Code.
pub(crate) fn setup_command() -> String {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "sprawl.exe".into());
    // user scope: in every folder, not only the one it was added in
    format!("claude mcp add --scope user sprawl -- \"{exe}\" --mcp")
}
