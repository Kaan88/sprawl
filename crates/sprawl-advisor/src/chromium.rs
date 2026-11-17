//! One recogniser for programs built on Chromium (Electron apps such as Discord, Slack, Teams
//! or VS Code, and Chromium browsers): a folder holding `Local State` beside a folder only
//! Chromium makes, or a `Cookies` file (or `Network\Cookies`) beside a `Cache` folder, is
//! Chromium's. What Chromium keeps there as caches is known and offered; what must stay
//! (cookies, local storage, sign-ins, form data) is on the never-list by name, so a clean-up
//! skips it wherever it is.
//!
//! Browsers' own profiles are on the never-list as a whole: what is found inside them is held
//! back by the safety checks and left to the browser's own "clear browsing data".

use std::collections::HashSet;

use sprawl_core::{Tree, ROOT};

use crate::finding::{Action, Finding, Remedy, Risk, Source};

/// The rule id of what this finds.
pub const ID: &str = "chromium.cache";

/// What Chromium keeps as caches in its folder: (path inside it, risk, what it is).
const PARTS: &[(&str, Risk, &str)] = &[
    ("Cache", Risk::Green, "Pages, images and scripts it saved to show them faster. It downloads them again when needed."),
    ("Code Cache", Risk::Green, "Scripts it compiled ahead, to start faster. It compiles them again."),
    ("GPUCache", Risk::Green, "Shaders compiled for the graphics card. It compiles them again; the first start may be slower."),
    ("DawnCache", Risk::Green, "Shaders compiled for the graphics card. It compiles them again."),
    ("DawnGraphiteCache", Risk::Green, "Shaders compiled for the graphics card. It compiles them again."),
    ("DawnWebGPUCache", Risk::Green, "Shaders compiled for the graphics card. It compiles them again."),
    ("GrShaderCache", Risk::Green, "Shaders compiled for the graphics card. It compiles them again."),
    ("GraphiteDawnCache", Risk::Green, "Shaders compiled for the graphics card. It compiles them again."),
    ("ShaderCache", Risk::Green, "Shaders compiled for the graphics card. It compiles them again."),
    (r"Service Worker\ScriptCache", Risk::Green, "Scripts web pages inside it saved to start faster. They download again."),
    (r"Service Worker\CacheStorage", Risk::Amber, "Copies web pages inside it keep to work offline. They download again; until then those pages don't work offline."),
    (r"Crashpad\reports", Risk::Green, "Crash reports it saved and may not have sent. Only useful to its makers."),
];

/// Folders only Chromium makes, one of which must sit beside `Local State`.
const BESIDE_LOCAL_STATE: &[&str] = &["Cache", "Code Cache", "GPUCache", "Crashpad", "Default", "Network", "Local Storage"];

/// The child of `n` called `name` (any case).
fn child(tree: &Tree, n: usize, name: &str) -> Option<usize> {
    tree.nodes[n].children.iter().copied().find(|&c| tree.nodes[c].name.eq_ignore_ascii_case(name))
}

fn dir(tree: &Tree, n: usize, name: &str) -> Option<usize> {
    child(tree, n, name).filter(|&c| tree.nodes[c].is_dir && !tree.nodes[c].is_reparse_point())
}

fn file(tree: &Tree, n: usize, name: &str) -> Option<usize> {
    child(tree, n, name).filter(|&c| !tree.nodes[c].is_dir)
}

/// Why folder `n` is Chromium's, if it is.
pub fn recognise(tree: &Tree, n: usize) -> Option<&'static str> {
    if !tree.nodes[n].is_dir || tree.nodes[n].is_reparse_point() {
        return None;
    }
    if file(tree, n, "Local State").is_some() && BESIDE_LOCAL_STATE.iter().any(|d| dir(tree, n, d).is_some()) {
        return Some("its Local State file");
    }
    let cookies = file(tree, n, "Cookies").is_some() || dir(tree, n, "Network").and_then(|k| file(tree, k, "Cookies")).is_some();
    if cookies && dir(tree, n, "Cache").is_some() {
        return Some("its Cookies file beside a Cache folder");
    }
    None
}

/// Chromium's own `Cache` holds `Cache_Data` (or, older, an `index` file): anything else called
/// Cache in its folder is not offered.
fn chromium_cache(tree: &Tree, c: usize) -> bool {
    dir(tree, c, "Cache_Data").is_some() || file(tree, c, "index").is_some()
}

/// A name for the program whose folder `n` is: the installed app that owns it, else the
/// folder's name, or its parent's when the folder's own name says nothing (`User Data`,
/// `Default`, `EBWebView`, `CefCache`, `0`...).
fn app_name(tree: &Tree, n: usize, facts: &crate::facts::Facts) -> String {
    if let Some(a) = facts.owner_of(&tree.path_of(n)) {
        return a.name.clone();
    }
    let mut k = n;
    for _ in 0..3 {
        let name = tree.nodes[k].name.to_lowercase();
        let generic = ["user data", "default", "electron", "ebwebview", "webview2", "cefcache", "cef", "client", "browser", "htmlcache", "browsercache", "chromium-cache", "web", "webview"].contains(&name.as_str())
            || name.starts_with("profile ")
            || name.starts_with("webcache")
            || name.starts_with("wv2profile")
            || name.chars().all(|c| c.is_ascii_digit());
        // never up to AppData's own folders: "Roaming" names nothing
        let top = |p: usize| ["roaming", "local", "locallow", "appdata"].contains(&tree.nodes[p].name.to_lowercase().as_str());
        match tree.nodes[k].parent {
            Some(p) if generic && p != ROOT && !top(p) => k = p,
            _ => break,
        }
    }
    tree.nodes[k].name.clone()
}

/// In a profile's AppData (`X:\Users\<name>\AppData\...`), where programs keep their data. A
/// Chromium folder anywhere else (a test run's in a build output, one in the Recycle Bin) is
/// not offered.
fn in_app_data(tree: &Tree, n: usize) -> bool {
    let segs = crate::segments(&tree.path_of(n).to_string_lossy());
    (0..segs.len().saturating_sub(3)).any(|i| segs[i] == "users" && segs[i + 2] == "appdata")
}

/// The caches in every folder of the tree that is Chromium's (in AppData), at least
/// `min_bytes` each, not already in `seen` (lower-case paths) and not protected (a Store app's
/// package, a browser profile: those would only be held back). Sprawl clears each folder's
/// contents.
pub fn find(tree: &Tree, facts: &crate::facts::Facts, seen: &HashSet<String>, min_bytes: u64) -> Vec<Finding> {
    let never = crate::safety::Never::default_list();
    let mut out = Vec::new();
    let mut stack = vec![ROOT];
    while let Some(n) = stack.pop() {
        let node = &tree.nodes[n];
        if !node.is_dir || (n != ROOT && node.is_reparse_point()) {
            continue;
        }
        stack.extend(node.children.iter().copied().filter(|&c| tree.nodes[c].is_dir));
        let Some(how) = recognise(tree, n) else { continue };
        if !in_app_data(tree, n) {
            continue;
        }
        let app = app_name(tree, n, facts);
        for &(rel, risk, why) in PARTS {
            let mut k = Some(n);
            for seg in rel.split('\\') {
                k = k.and_then(|i| dir(tree, i, seg));
            }
            let Some(k) = k else { continue };
            if rel == "Cache" && !chromium_cache(tree, k) {
                continue;
            }
            let part = &tree.nodes[k];
            if part.size < min_bytes {
                continue;
            }
            let path = tree.path_of(k);
            if seen.contains(&path.to_string_lossy().to_lowercase()) || never.hit(&path.to_string_lossy()).is_some() {
                continue;
            }
            let mut f = Finding {
                path,
                owner: Some(app.clone()),
                bytes: part.size,
                risk,
                remedy: Remedy::Delete,
                command: None,
                title: format!("{app}: {}", rel.rsplit('\\').next().unwrap_or(rel)),
                evidence: vec![
                    why.to_string(),
                    format!("{app} is built on Chromium: Sprawl knows its folder by {how}, and what Chromium keeps there"),
                    "Its cookies, local storage and sign-ins sit beside it and stay; close it first for the most to go".into(),
                    format!("{} files in {} folders", part.files, part.folders),
                ],
                confidence: 0.9,
                source: Source::Rule(ID.into()),
                allow_protected: false,
                trusted: true,
                builtin: true,
                on_path: true,
                action: None,
                admin: false,
                carved: Vec::new(),
                gain: None,
            };
            f.set_action(Action::Clear { days: 0 }, false);
            out.push(f);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &std::path::Path, len: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![0u8; len]).unwrap();
    }

    #[test]
    fn an_electron_app_is_recognised_and_only_its_caches_offered() {
        // a profile's AppData, built where a test may write
        let base = std::env::temp_dir().join(format!("sprawl-chromium-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let base = base.join("Users").join("sam").join("AppData");
        let app = base.join("Roaming").join("chatapp");
        write(&app.join("Local State"), 10);
        write(&app.join("Cache").join("Cache_Data").join("f_000001"), 3000);
        write(&app.join("Code Cache").join("js").join("a"), 2000);
        write(&app.join("GPUCache").join("index"), 1500);
        write(&app.join("Service Worker").join("CacheStorage").join("x"), 1200);
        write(&app.join("Crashpad").join("reports").join("r.dmp"), 1100);
        write(&app.join("Local Storage").join("leveldb").join("000.log"), 5000);
        write(&app.join("IndexedDB").join("https_x_0.indexeddb.leveldb").join("1.ldb"), 5000);
        write(&app.join("Network").join("Cookies"), 100);
        // a Cache that isn't Chromium's beside nothing of Chromium's: not this
        let other = base.join("Roaming").join("tool");
        write(&other.join("Cache").join("a.bin"), 4000);
        write(&other.join("Cookies"), 10);
        // Cookies beside a Cache: an older Chromium profile
        let old = base.join("Local").join("old").join("Default");
        write(&old.join("Cookies"), 10);
        write(&old.join("Cache").join("index"), 1000);
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let facts = crate::facts::Facts::from_apps(Vec::new());
        let found = find(&tree, &facts, &HashSet::new(), 1000);
        let mut titles: Vec<&str> = found.iter().map(|f| f.title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, ["chatapp: Cache", "chatapp: CacheStorage", "chatapp: Code Cache", "chatapp: GPUCache", "chatapp: reports", "old: Cache"], "{titles:?}");
        assert!(found.iter().all(|f| f.source == Source::Rule(ID.into()) && f.action == Some(Action::Clear { days: 0 })));
        assert_eq!(found.iter().find(|f| f.title.ends_with("CacheStorage")).unwrap().risk, Risk::Amber);
        assert!(found.iter().filter(|f| !f.title.ends_with("CacheStorage")).all(|f| f.risk == Risk::Green));
        // already found by a rule or cleaner: not again
        let seen: HashSet<String> = [app.join("Cache").to_string_lossy().to_lowercase()].into();
        assert_eq!(find(&tree, &facts, &seen, 1000).len(), found.len() - 1);
        // what must stay is protected by name, wherever it is, and a clean-up of the whole
        // folder leaves it
        let never = crate::safety::Never::default_list();
        for p in [r"Local Storage\leveldb", r"IndexedDB\https_x_0.indexeddb.leveldb", r"IndexedDB\https_x_0.indexeddb.blob", r"Network\Cookies", r"Default\Cookies", r"Profile 2\Cookies", "Login Data", "Web Data"] {
            assert!(never.path_hit(&app.join(p)).is_some(), "{p}");
        }
        // by Chromium's layout, not any folder of that name (a library's code)
        for p in [r"node_modules\undici\lib\web\cookies", r"src\IndexedDB", r"docs\Local Storage"] {
            assert!(never.path_hit(&base.join(p)).is_none(), "{p}");
        }
        let n = crate::find_path(&tree, &app).unwrap();
        let carved: Vec<String> = never.hits_inside(&tree, n).iter().map(|&(k, _)| tree.nodes[k].name.clone()).collect();
        assert!(carved.contains(&"leveldb".to_string()) && carved.contains(&"https_x_0.indexeddb.leveldb".to_string()) && carved.contains(&"Cookies".to_string()), "{carved:?}");
        let o = crate::clean::run(&app, &Action::Clear { days: 0 }, &crate::clean::Spare::default(), &std::sync::atomic::AtomicBool::new(false), &mut |_| {});
        assert!(app.join("Local Storage").join("leveldb").join("000.log").exists());
        assert!(app.join("IndexedDB").join("https_x_0.indexeddb.leveldb").join("1.ldb").exists());
        assert!(app.join("Network").join("Cookies").exists());
        assert!(!app.join("Cache").exists() && !app.join("GPUCache").exists());
        assert_eq!(o.kept_protected, 3, "{:?}", o.problems);
        // one outside a profile's AppData is not offered
        // (%TEMP% is itself in AppData: the repo's target folder isn't)
        let elsewhere = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join(format!("sprawl-chromium-x-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&elsewhere);
        write(&elsewhere.join("app").join("Local State"), 10);
        write(&elsewhere.join("app").join("Cache").join("Cache_Data").join("f"), 3000);
        let t2 = sprawl_core::scan(&elsewhere, |_| {}).unwrap();
        assert!(find(&t2, &facts, &HashSet::new(), 1000).is_empty());
        let _ = std::fs::remove_dir_all(&elsewhere);
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("sprawl-chromium-{}", std::process::id())));
    }
}
