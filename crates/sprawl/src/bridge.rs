//! The running app's side of `sprawl --mcp`: a thread serving the named pipe with the scan on
//! screen, the Advisor's findings and the user's memory. A filter a model asks to show is
//! queued for the UI thread, which applies it as if typed.

use std::sync::{Arc, Mutex, OnceLock, Weak};
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
        use sprawl_advisor::mcp::{bridge_request_options, bridge_work, BridgeKind, Workers};
        let facts = Arc::new(Mutex::new(FactsCache::default()));
        // Eight bridge task threads at most: four queries (also History lists), two reserved
        // scan controls, and two facts preparers shared by on- and off-screen trees. Scan
        // traversal has its own separately bounded lifecycle.
        let workers = Workers::new(4);
        let controls = Workers::new(2);
        let preparers = Workers::new(2);
        let index = Arc::new(sprawl_advisor::mcp_scan::Index::for_app(false).with_facts_workers(preparers.clone()));
        let error = |text: &str| serde_json::json!({"ok": false, "text": text}).to_string();
        let served = sprawl_advisor::pipe::serve(|line, client_elevated| {
            let received = Instant::now();
            let (cancel, kind, wait) = match bridge_request_options(line) { Ok(req) => req, Err(e) => return error(&e) };
            let until = received + wait;
            if Instant::now() >= until { return error("Sprawl's window ran out of time answering; try again."); }
            if cancel.stopped() { return error("Sprawl cancelled the request."); }
            let (line, index, facts, preparers) = (line.to_string(), index.clone(), facts.clone(), preparers.clone());
            let pool = if kind == BridgeKind::ScanControl { &controls } else { &workers };
            bridge_work(pool, cancel, until,
                move || client_elevated || !sprawl_core::elevation().unwrap_or(true) || crate::admin::store::get(crate::admin::MCP_DISK.key),
                move |disk, work| {
                    let view = (kind == BridgeKind::Query).then(View::current);
                    let cached = match &view {
                        Some(view) if disk && view.facts.is_none() => match FactsCache::reserve(&facts, view.tree.as_ref()) {
                            Ok(Some(FactsPrep::Ready(facts))) => Some(facts),
                            Ok(Some(FactsPrep::Gather(job))) => {
                                // Never hold a query/control worker or the pipe during the gather.
                                if let Err(e) = preparers.spawn(move || { job.gather(); }) { return error(&e); }
                                return error(GATHERING_FACTS);
                            }
                            Ok(None) => None,
                            Err(e) => return error(e),
                        },
                        _ => None,
                    };
                    answer(&line, disk, view, &index, cached, work)
                }).unwrap_or_else(|e| error(&e))
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

#[derive(Default)]
struct FactsCache {
    ready: Option<(Weak<Tree>, Arc<Facts>)>,
    gathering: Vec<Weak<Tree>>,
}

enum FactsPrep { Ready(Arc<Facts>), Gather(FactsJob) }

use sprawl_advisor::mcp_scan::GATHERING_FACTS;

impl FactsCache {
    fn reserve(cache: &Arc<Mutex<Self>>, tree: Option<&Arc<Tree>>) -> Result<Option<FactsPrep>, &'static str> {
        let Some(tree) = tree else { return Ok(None); };
        let mut state = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, facts)) = state.ready.as_ref().filter(|(weak, _)| same_tree(weak, Some(tree))) {
            return Ok(Some(FactsPrep::Ready(facts.clone())));
        }
        let weak = Arc::downgrade(tree);
        if state.gathering.iter().any(|pending| pending.ptr_eq(&weak)) {
            return Err(GATHERING_FACTS);
        }
        state.gathering.push(weak);
        Ok(Some(FactsPrep::Gather(FactsJob { tree: tree.clone(), cache: cache.clone() })))
    }
}

struct FactsJob { tree: Arc<Tree>, cache: Arc<Mutex<FactsCache>> }

impl FactsJob {
    fn gather(self) -> Arc<Facts> {
        let mut f = Facts::gather();
        f.attach_usage(&self.tree);
        let facts = Arc::new(f);
        // An old worker must not evict the current tree's prepared facts on late completion.
        let view = shared().lock().unwrap_or_else(|e| e.into_inner());
        if same_tree(&view.tree, Some(&self.tree)) {
            self.cache.lock().unwrap_or_else(|e| e.into_inner()).ready = Some((Arc::downgrade(&self.tree), facts.clone()));
        }
        facts
    }
}

impl Drop for FactsJob {
    fn drop(&mut self) {
        let weak = Arc::downgrade(&self.tree);
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).gathering.retain(|pending| !pending.ptr_eq(&weak));
    }
}

struct View {
    tree: Option<Arc<Tree>>,
    revision: u64,
    scan_before: Option<std::time::SystemTime>,
    findings: String,
    found: Option<Arc<Vec<sprawl_advisor::finding::Finding>>>,
    facts: Option<Arc<Facts>>,
    memory: Memory,
}

impl View {
    fn current() -> Self {
        let s = shared().lock().unwrap_or_else(|e| e.into_inner());
        Self { tree: s.tree.upgrade(), revision: s.revision, scan_before: s.scan_before,
            findings: s.findings.clone(), found: s.found.clone(), facts: s.facts.clone(), memory: s.memory.clone() }
    }
}

fn answer(line: &str, disk: bool, view: Option<View>, index: &sprawl_advisor::mcp_scan::Index, cached: Option<Arc<Facts>>, cancel: Arc<sprawl_advisor::mcp_cancel::Cancel>) -> String {
    use sprawl_advisor::mcp::answer_bridge_cancellable;
    let Some(View { tree, revision, scan_before, findings, found, facts: report_facts, memory }) = view else {
        let scanning = shared().lock().unwrap_or_else(|e| e.into_inner()).scanning;
        index.set_app_state(scanning, !crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed));
        return answer_bridge_cancellable(line, None, None, None, &Memory::default(), "", None, index, disk, &mut |_| {}, cancel);
    };
    // A restricted client still sees only what the app has already published about this tree.
    let gathered;
    let scoped;
    let f = if disk {
        gathered = report_facts.or(cached);
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
    fn facts_reservation_is_single_flight_per_tree_and_clears_on_exit() {
        let cache = Arc::new(Mutex::new(FactsCache::default()));
        let (a, b) = (tree(), tree());
        let first = FactsCache::reserve(&cache, Some(&a)).unwrap();
        assert!(matches!(first.as_ref(), Some(FactsPrep::Gather(_))));
        for _ in 0..6 {
            assert!(matches!(FactsCache::reserve(&cache, Some(&a)), Err(GATHERING_FACTS)));
        }
        let other = FactsCache::reserve(&cache, Some(&b)).unwrap();
        assert!(matches!(other.as_ref(), Some(FactsPrep::Gather(_))), "same path isn't the same tree");
        drop(first);
        assert!(matches!(FactsCache::reserve(&cache, Some(&a)), Ok(Some(FactsPrep::Gather(_)))));
        assert!(matches!(FactsCache::reserve(&cache, Some(&b)), Err(GATHERING_FACTS)));
        drop(other);
        assert!(FactsCache::reserve(&cache, None).unwrap().is_none());
    }

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
