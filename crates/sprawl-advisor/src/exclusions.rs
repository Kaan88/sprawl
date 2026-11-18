//! Shared scan scopes for the app, background scans and MCP. No glob interpretation.
use std::path::{Path, PathBuf};

pub fn parse(text: &str) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for value in text.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let value = value.trim_matches('"').replace('/', "\\");
        let b = value.as_bytes();
        let p = PathBuf::from(&value);
        if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\'
            || value.contains(['*', '?', '%', '\r', '\n', '\0', '"']) || value[2..].contains(':')
            || value[3..].split('\\').any(|part| matches!(part, "." | "..") || part.ends_with('.') || part.ends_with(' ')) {
            return Err("Folders to skip must be full absolute paths separated by semicolons, without wildcards, variables or ..".into());
        }
        // An exclusion remains policy even when its drive disappears or its folder changes.
        // Never inspect availability or follow aliases just to remember what must be skipped.
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
    let mut file = match sprawl_core::local::file(path, 1 << 20) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(_) => return refuse_all(),
    };
    let mut text = String::new();
    if file.file.read_to_string(&mut text).is_err() { return refuse_all(); }
    let value = text.trim_start_matches('\u{feff}').lines().rev().filter_map(|l| l.split_once('='))
        .find(|(key, _)| key.trim() == "scanExclude").map_or("", |(_, value)| value);
    parse(value).unwrap_or_else(|_| refuse_all())
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
        std::fs::remove_dir_all(dir).unwrap();
    }
}
