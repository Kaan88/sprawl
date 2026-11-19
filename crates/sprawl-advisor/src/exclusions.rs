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

/// The list couldn't be read: every drive, and every network path, counts as skipped.
fn refuse_all() -> Vec<PathBuf> {
    (b'A'..=b'Z').map(|drive| PathBuf::from(format!("{}:\\", drive as char))).chain(std::iter::once(PathBuf::from(sprawl_core::progress::UNREADABLE_POLICY))).collect()
}

/// Whether `excluded` comes from a list that couldn't be read.
pub fn unreadable(excluded: &[PathBuf]) -> bool {
    excluded.iter().any(|e| e.as_os_str() == sprawl_core::progress::UNREADABLE_POLICY)
}

fn configured_at(path: &Path) -> Vec<PathBuf> {
    use std::io::Read;
    // An ordinary open, which lets Settings replace the file meanwhile: a pinned read without
    // delete sharing (1.0) made a save that came at the same moment fail, silently.
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        // no settings yet: nothing skipped. A link left there that leads nowhere is not "no
        // settings": the list can't be read.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && std::fs::symlink_metadata(path).is_err() => return Vec::new(),
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
        // on a mapped network drive: the same folder by its share's name, and by every other
        // letter mapped to that share, so a scan of \\server\share (or Z:) skips it too
        for alias in network_aliases(&path) { if !out.contains(&alias) { out.push(alias); } }
    }
    out
}

/// `\\server\share` for each mapped network drive letter (`Y:`), from Windows' own list of
/// connections: nothing is asked of the network.
fn mapped_drives() -> Vec<(char, String)> {
    #[link(name = "mpr")]
    extern "system" { fn WNetGetConnectionW(local: *const u16, remote: *mut u16, len: *mut u32) -> u32; }
    #[link(name = "kernel32")]
    extern "system" { fn GetLogicalDrives() -> u32; fn GetDriveTypeW(root: *const u16) -> u32; }
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u8 {
        if mask & (1 << i) == 0 { continue; }
        let letter = (b'A' + i) as char;
        let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain(Some(0)).collect();
        if unsafe { GetDriveTypeW(root.as_ptr()) } != 4 { continue; }
        let local: Vec<u16> = format!("{letter}:").encode_utf16().chain(Some(0)).collect();
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        if unsafe { WNetGetConnectionW(local.as_ptr(), buf.as_mut_ptr(), &mut len) } != 0 { continue; }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let remote = String::from_utf16_lossy(&buf[..n]).trim_end_matches('\\').to_string();
        if remote.starts_with(r"\\") && !remote.starts_with(r"\\?") && !remote.starts_with(r"\\.") && remote.len() > 2 { out.push((letter, remote)); }
    }
    out
}

fn network_aliases(path: &Path) -> Vec<PathBuf> {
    let text = path.to_string_lossy();
    let Some(letter) = text.chars().next().map(|c| c.to_ascii_uppercase()) else { return Vec::new(); };
    if text.as_bytes().get(1) != Some(&b':') { return Vec::new(); }
    let drives = mapped_drives();
    let Some((_, share)) = drives.iter().find(|(l, _)| *l == letter) else { return Vec::new(); };
    let rest = text[2..].trim_start_matches('\\');
    let unc = if rest.is_empty() { share.clone() } else { format!("{share}\\{rest}") };
    let mut out = vec![PathBuf::from(&unc)];
    for (other, remote) in &drives {
        if *other != letter && remote.eq_ignore_ascii_case(share) {
            out.push(PathBuf::from(format!("{other}:\\{rest}")));
        }
    }
    out
}

pub fn progress() -> std::sync::Arc<sprawl_core::Progress> { sprawl_core::Progress::with_exclusions(&configured()) }

pub fn intersects(path: &Path, excluded: &[PathBuf]) -> bool {
    if excluded.is_empty() { return false; }
    if unreadable(excluded) { return true; }
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
        // both as written, plus the share's own name where this PC maps Z: to a share
        assert_eq!(excluded[..2], [dir.join("skip"), PathBuf::from(r"Z:\unavailable\scope")]);
        assert!(excluded[2..].iter().all(|p| p.to_string_lossy().starts_with(r"\\") && p.ends_with(r"unavailable\scope")), "{excluded:?}");
        assert!(intersects(&dir.join("skip/private"), &excluded));
        assert!(intersects(&dir, &excluded), "cleanup/export of a containing folder is still blocked");
        let tree = sprawl_core::walk::walk(&dir, &sprawl_core::Progress::with_exclusions(&excluded)).unwrap();
        assert!(!tree.nodes.iter().any(|n| n.name == "skip" || n.name == "private"));
        std::fs::write(&settings, "scanExclude=not a valid absolute scope").unwrap();
        assert!(intersects(&dir.join("public"), &configured_at(&settings)), "invalid policy refuses instead of silently disabling protection");
        let locked = std::fs::OpenOptions::new().read(true).share_mode(0).open(&settings).unwrap();
        assert!(intersects(&dir, &configured_at(&settings)), "unreadable policy also refuses");
        // network paths too (Astra's case: A: to Z: alone don't cover a share)
        let share = Path::new(r"\\server\share\private");
        assert!(intersects(share, &configured_at(&settings)));
        assert!(sprawl_core::Progress::with_exclusions(&configured_at(&settings)).excludes(share));
        assert!(sprawl_core::walk::inventory(&dir, &sprawl_core::Progress::with_exclusions(&configured_at(&settings))).is_err());
        drop(locked);
        std::fs::write(&settings, "scanExclude=Z:\\UNAVAI~1\\scope").unwrap();
        assert!(intersects(Path::new(r"Z:\Some long directory\scope"), &configured_at(&settings)), "an unresolved short alias must conservatively hold its unambiguous ancestor");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
