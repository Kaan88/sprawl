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
    /// What was published last, to republish only when it changes.
    published: usize,
    findings: String,
    memory: Memory,
    filters: Vec<String>,
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Shared::default()))
}

/// Serve the pipe on its own thread (a second Sprawl window just doesn't get it).
pub(crate) fn start() {
    std::thread::spawn(|| {
        // app facts are gathered once per scan, on first use
        let mut facts: Option<(usize, Facts)> = None;
        let _ = sprawl_advisor::pipe::serve(|line| {
            let (tree, findings, memory) = {
                let s = shared().lock().unwrap_or_else(|e| e.into_inner());
                (s.tree.upgrade(), s.findings.clone(), s.memory.clone())
            };
            let key = tree.as_ref().map_or(0, |t| Arc::as_ptr(t) as usize);
            if facts.as_ref().is_none_or(|(k, _)| *k != key) {
                let mut f = Facts::gather();
                if let Some(t) = &tree {
                    f.attach_usage(t);
                }
                facts = Some((key, f));
            }
            let f = &facts.as_ref().expect("just set").1;
            let mut wanted = None;
            // a panic answers an error and leaves the pipe working
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sprawl_advisor::mcp::answer_bridge(line, tree.as_deref(), f, &memory, &findings, &mut |x| wanted = Some(x))))
                .unwrap_or_else(|_| r#"{"ok":false,"text":"Sprawl hit an internal error answering that."}"#.to_string());
            if let Some(x) = wanted {
                shared().lock().unwrap_or_else(|e| e.into_inner()).filters.push(x);
            }
            out
        });
    });
}

/// Called every frame with the tree on screen (cheap: only a change is stored).
pub(crate) fn publish_tree(t: Option<&Arc<Tree>>) {
    let key = t.map_or(0, |t| Arc::as_ptr(t) as usize);
    let mut s = shared().lock().unwrap_or_else(|e| e.into_inner());
    if s.published != key {
        s.published = key;
        s.tree = t.map(Arc::downgrade).unwrap_or_default();
    }
}

pub(crate) fn publish_findings(text: String) {
    shared().lock().unwrap_or_else(|e| e.into_inner()).findings = text;
}

pub(crate) fn publish_memory(m: &Memory) {
    shared().lock().unwrap_or_else(|e| e.into_inner()).memory = m.clone();
}

/// Filters asked for since the last call.
pub(crate) fn take_filters() -> Vec<String> {
    std::mem::take(&mut shared().lock().unwrap_or_else(|e| e.into_inner()).filters)
}

/// The line that adds Sprawl to the user's Claude Code.
pub(crate) fn setup_command() -> String {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "sprawl.exe".into());
    // user scope: in every folder, not only the one it was added in
    format!("claude mcp add --scope user sprawl -- \"{exe}\" --mcp")
}
