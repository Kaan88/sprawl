//! Shared scan scopes for the app, background scans and MCP. No glob interpretation.
use std::path::{Path, PathBuf};

pub fn parse(text: &str) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for value in text.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let value = value.trim_matches('"').replace('/', "\\");
        let b = value.as_bytes();
        if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\'
            || value.contains(['*', '?', '%', '\r', '\n', '\0', '"']) || value[2..].contains(':')
            || value[3..].split('\\').any(|part| matches!(part, "." | "..") || part.ends_with('.') || part.ends_with(' ')) {
            return Err("Folders to skip must be full absolute paths separated by semicolons, without wildcards, variables or ..".into());
        }
        // An exclusion remains policy even when its drive disappears or its folder changes.
        // Never inspect availability or follow aliases just to remember what must be skipped.
        let p: PathBuf = Path::new(&value).components().collect();
        if !paths.contains(&p) { paths.push(p); }
        if paths.len() > 100 { return Err("Use at most 100 folders to skip.".into()); }
    }
    Ok(paths)
}

pub fn configured() -> Vec<PathBuf> {
    let Some(base) = std::env::var_os("APPDATA") else { return refuse_all(); };
    configured_at(&PathBuf::from(base).join("Sprawl/settings.ini"))
}

fn refuse_all() -> Vec<PathBuf> { (b'A'..=b'Z').map(|drive| PathBuf::from(format!("{}:\\", drive as char))).collect() }

fn configured_at(path: &Path) -> Vec<PathBuf> {
    use std::io::Read;
    // An ordinary open, which lets Settings replace the file meanwhile: a pinned read without
    // delete sharing (1.0) made a save that came at the same moment fail, silently.
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(_) => return refuse_all(),
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) { return refuse_all(); }
    let mut text = String::new();
    if file.take((1 << 20) + 1).read_to_string(&mut text).is_err() || text.len() > 1 << 20 { return refuse_all(); }
    let value = text.trim_start_matches('\u{feff}').lines().rev().filter_map(|l| l.split_once('='))
        .find(|(key, _)| key.trim() == "scanExclude").map_or("", |(_, value)| value);
    parse(value).map(with_aliases).unwrap_or_else(|_| refuse_all())
}

fn with_aliases(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if !out.contains(&path) { out.push(path.clone()); }
        let alias = match sprawl_core::local::scope(&path) {
            Ok(held) => Some(held.path.clone()),
            Err(_) if path.to_string_lossy().contains('~') => {
                // A disconnected/unreadable 8.3 alias cannot be resolved. Preserve it and
                // hold at the first unambiguous containing folder instead of missing it.
                let mut parent = PathBuf::new();
                for part in path.components() {
                    if part.as_os_str().to_string_lossy().contains('~') { break; }
                    parent.push(part.as_os_str());
                }
                Some(parent)
            }
            Err(_) => None,
        };
        if let Some(alias) = alias { if !out.contains(&alias) { out.push(alias); } }
    }
    out
}

pub fn progress() -> std::sync::Arc<sprawl_core::Progress> { sprawl_core::Progress::with_exclusions(&configured()) }

pub fn intersects(path: &Path, excluded: &[PathBuf]) -> bool {
    if excluded.is_empty() { return false; }
    excluded.iter().any(|e| crate::finding::within(path, e) || crate::finding::within(e, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_scopes_and_validation() {
        let paths = parse(r"C:\Data\skip; D:\Games").unwrap();
        assert!(intersects(Path::new(r"C:\DATA\skip\child"), &paths));
        assert!(intersects(Path::new(r"C:\Data"), &paths));
        assert!(!intersects(Path::new(r"C:\Data\skip-other"), &paths));
        assert_eq!(parse(r"C:\\Data\\skip").unwrap(), vec![PathBuf::from(r"C:\Data\skip")]);
        for bad in [r"relative", r"C:\Data\*", r"C:\Data\..\Other", r"%USERPROFILE%\Games"] { assert!(parse(bad).is_err()); }
    }

    #[test]
    fn unavailable_exclusion_never_disables_other_scopes() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("sprawl-exclusion-policy-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("skip")).unwrap();
        std::fs::write(dir.join("skip/private"), b"secret").unwrap();
        std::fs::write(dir.join("public"), b"ok").unwrap();
        let settings = dir.join("settings.ini");
        std::fs::write(&settings, format!("scanExclude={};Z:\\unavailable\\scope", dir.join("skip").display())).unwrap();
        let excluded = configured_at(&settings);
        assert_eq!(excluded.len(), 2);
        assert!(intersects(&dir.join("skip/private"), &excluded));
        assert!(intersects(&dir, &excluded), "cleanup/export of a containing folder is still blocked");
        let tree = sprawl_core::walk::walk(&dir, &sprawl_core::Progress::with_exclusions(&excluded)).unwrap();
        assert!(!tree.nodes.iter().any(|n| n.name == "skip" || n.name == "private"));
        std::fs::write(&settings, "scanExclude=not a valid absolute scope").unwrap();
        assert!(intersects(&dir.join("public"), &configured_at(&settings)), "invalid policy refuses instead of silently disabling protection");
        let locked = std::fs::OpenOptions::new().read(true).share_mode(0).open(&settings).unwrap();
        assert!(intersects(&dir, &configured_at(&settings)), "unreadable policy also refuses");
        drop(locked);
        std::fs::write(&settings, "scanExclude=Z:\\UNAVAI~1\\scope").unwrap();
        assert!(intersects(Path::new(r"Z:\Some long directory\scope"), &configured_at(&settings)), "an unresolved short alias must conservatively hold its unambiguous ancestor");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
