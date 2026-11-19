//! The running app's side of `sprawl --mcp`: a thread serving the named pipe with the scan on
//! screen, the Advisor's findings and the user's memory. A filter a model asks to show is
//! queued for the UI thread, which applies it as if typed.

use std::sync::{Arc, Mutex, OnceLock, Weak};

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
    /// The app's own scan is running (a server's scan waits for it).
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
        // app facts are gathered once per scan, on first use
        let mut facts: Option<(Weak<Tree>, Facts)> = None;
        // scans servers ask for, kept on this thread, saved to History as the app's own are
        let mut index = sprawl_advisor::mcp_scan::Index::for_app(false);
        let _ = sprawl_advisor::pipe::serve(|line, client_elevated| {
            // an elevated app reads the disk for a client that isn't only when the user allowed
            // it (an elevation Windows can't confirm counts as elevated)
            let disk = client_elevated || !sprawl_core::elevation().unwrap_or(true) || crate::admin::store::get(crate::admin::MCP_DISK.key);
            sprawl_advisor::mcp::with_bridge_request(line, || {
            let (tree, revision, scan_before, findings, found, report_facts, memory, scanning) = {
                let s = shared().lock().unwrap_or_else(|e| e.into_inner());
                (s.tree.upgrade(), s.revision, s.scan_before, s.findings.clone(), s.found.clone(), s.facts.clone(), s.memory.clone(), s.scanning)
            };
            // a screenshot run writes nothing; its flag is set after this thread starts
            index.set_app_state(scanning, !crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed));
            // A client that may only see the scan on screen gets what the app has published,
            // cut down to that tree: no app facts, recent items or program starts are read for
            // it (they would be, with the app's rights, before anything else was decided).
            let scoped;
            let f: Option<&Facts> = if disk {
                if report_facts.is_none() && facts.as_ref().is_none_or(|(weak, _)| !same_tree(weak, tree.as_ref())) {
                    let mut f = if tree.is_some() { Facts::gather() } else { Facts::default() };
                    if let Some(t) = &tree {
                        f.attach_usage(t);
                    }
                    facts = Some((tree.as_ref().map(Arc::downgrade).unwrap_or_default(), f));
                }
                Some(report_facts.as_deref().unwrap_or_else(|| &facts.as_ref().expect("made for an unfinished report").1))
            } else {
                scoped = report_facts.as_deref().zip(tree.as_deref()).map(|(f, t)| f.scoped_to(t));
                scoped.as_ref()
            };
            let mut wanted = None;
            // a panic answers an error and leaves the pipe working
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sprawl_advisor::mcp::answer_bridge(line, tree.as_deref(), scan_before, f, &memory, &findings, found.as_deref().map(|v| v.as_slice()), &mut index, disk, &mut |x| wanted = Some(x))))
                .unwrap_or_else(|_| r#"{"ok":false,"text":"Sprawl hit an internal error answering that."}"#.to_string());
            if let Some(x) = wanted {
                let mut s = shared().lock().unwrap_or_else(|e| e.into_inner());
                if !s.queue_filter(revision, tree.as_ref(), x) {
                    return r#"{"ok":false,"text":"That scan changed or is not the tree displayed on the map; no filter was applied."}"#.into();
                }
            }
            out
            }).unwrap_or_else(|error| serde_json::json!({"ok": false, "text": error}).to_string())
        });
    });
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
