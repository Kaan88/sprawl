//! Shared scan scopes for the app, background scans and MCP. No glob interpretation.
use std::path::{Path, PathBuf};

pub fn parse(text: &str) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for value in text.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(value.trim_matches('"'));
        if !p.is_absolute() || value.contains(['*', '?', '%', '\r', '\n']) || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err("Folders to skip must be full absolute paths separated by semicolons, without wildcards, variables or ..".into());
        }
        let p = sprawl_core::canonical_root(&p);
        if !paths.contains(&p) { paths.push(p); }
        if paths.len() > 100 { return Err("Use at most 100 folders to skip.".into()); }
    }
    Ok(paths)
}

pub fn configured() -> Vec<PathBuf> {
    let text = std::env::var_os("APPDATA").and_then(|a| std::fs::read_to_string(PathBuf::from(a).join("Sprawl/settings.ini")).ok()).unwrap_or_default();
    text.lines().filter_map(|l| l.split_once('=')).find(|(k, _)| k.trim() == "scanExclude")
        .and_then(|(_, v)| parse(v).ok()).unwrap_or_default()
}

pub fn progress() -> std::sync::Arc<sprawl_core::Progress> { sprawl_core::Progress::with_exclusions(&configured()) }

pub fn intersects(path: &Path, excluded: &[PathBuf]) -> bool {
    if excluded.is_empty() { return false; }
    let p = sprawl_core::canonical_root(path);
    excluded.iter().any(|e| crate::finding::within(&p, e) || crate::finding::within(e, &p))
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
}
