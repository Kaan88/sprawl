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
        Err(e) if e.kind() == std::io::ErrorKind::NotFound
            && matches!(std::fs::symlink_metadata(path), Err(m) if m.kind() == std::io::ErrorKind::NotFound) => return Vec::new(),
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
    with_aliases_mapped(paths, &mapped_drives())
}

fn with_aliases_mapped(paths: Vec<PathBuf>, mapped: &Mapped) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let add = |out: &mut Vec<PathBuf>, p: PathBuf| if !out.contains(&p) { out.push(p); };
    for path in paths {
        add(&mut out, path.clone());
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
        // On a network drive: the same folder by its share's name, and by every letter mapped to
        // that share or to a folder above or below it, so a scan of \\server\share (or Z:)
        // skips it too. The resolved long name and a short name's held parent get the same.
        let mut spellings = vec![path.clone()];
        if let Some(alias) = alias { add(&mut out, alias.clone()); spellings.push(alias); }
        for spelling in &spellings {
            for alias in network_aliases(spelling, mapped, &mut out) { add(&mut out, alias); }
        }
    }
    out
}

/// Windows' own list of network drive connections: nothing is asked of the network.
#[derive(Debug, Default)]
struct Mapped {
    /// `\\server\share[\folder]` of each mapped network drive letter.
    letters: Vec<(char, String)>,
    /// Network drive letters whose share couldn't be read.
    failed: Vec<char>,
}

/// `\\server\share` for each mapped network drive letter (`Y:`).
fn mapped_drives() -> Mapped {
    #[link(name = "mpr")]
    extern "system" { fn WNetGetConnectionW(local: *const u16, remote: *mut u16, len: *mut u32) -> u32; }
    #[link(name = "kernel32")]
    extern "system" { fn GetLogicalDrives() -> u32; fn GetDriveTypeW(root: *const u16) -> u32; }
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Mapped::default();
    for i in 0..26u8 {
        if mask & (1 << i) == 0 { continue; }
        let letter = (b'A' + i) as char;
        let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain(Some(0)).collect();
        if unsafe { GetDriveTypeW(root.as_ptr()) } != 4 { continue; }
        let local: Vec<u16> = format!("{letter}:").encode_utf16().chain(Some(0)).collect();
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        if unsafe { WNetGetConnectionW(local.as_ptr(), buf.as_mut_ptr(), &mut len) } != 0 { out.failed.push(letter); continue; }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let remote = String::from_utf16_lossy(&buf[..n]).trim_end_matches('\\').to_string();
        if remote.starts_with(r"\\") && !remote.starts_with(r"\\?") && !remote.starts_with(r"\\.") && remote.len() > 2 { out.letters.push((letter, remote)); }
        else { out.failed.push(letter); }
    }
    out
}

/// Every other spelling of `path` through the network: its `\\server\share` form and each
/// mapped letter whose share contains it (`Z:` mapped to `\\srv\share\sub` for a skip at
/// `\\srv\share\sub\x` gives `Z:\x`) or lies inside it (that whole letter). When a drive's
/// share can't be read and the skip may be reached through it, that letter is skipped whole, and
/// when the skip itself is on such a drive, every share path is ([`REMOTE_UNKNOWN`]).
fn network_aliases(path: &Path, mapped: &Mapped, policy: &mut Vec<PathBuf>) -> Vec<PathBuf> {
    use sprawl_core::progress::REMOTE_UNKNOWN;
    let text = path.to_string_lossy().replace('/', "\\");
    let letter = text.chars().next().filter(|_| text.as_bytes().get(1) == Some(&b':')).map(|c| c.to_ascii_uppercase());
    let mut out = Vec::new();
    let unc = match letter {
        Some(letter) if mapped.failed.contains(&letter) => {
            // which share this is can't be told: every share path, and every network letter
            if !policy.iter().any(|p| p.as_os_str() == REMOTE_UNKNOWN) { policy.push(PathBuf::from(REMOTE_UNKNOWN)); }
            out.extend(mapped.letters.iter().map(|(l, _)| *l).chain(mapped.failed.iter().copied()).map(|l| PathBuf::from(format!("{l}:\\"))));
            return out;
        }
        Some(letter) => match mapped.letters.iter().find(|(l, _)| *l == letter) {
            Some((_, share)) => {
                let rest = text[2..].trim_start_matches('\\');
                if rest.is_empty() { share.clone() } else { format!("{share}\\{rest}") }
            }
            None => return out, // a local drive
        },
        None if text.starts_with(r"\\") => text.trim_end_matches('\\').to_string(),
        None => return out,
    };
    out.push(PathBuf::from(&unc));
    for (other, remote) in &mapped.letters {
        if crate::finding::within(Path::new(&unc), Path::new(remote)) {
            let rest = unc[remote.len()..].trim_start_matches('\\');
            out.push(PathBuf::from(format!("{other}:\\{rest}")));
        } else if crate::finding::within(Path::new(remote), Path::new(&unc)) {
            out.push(PathBuf::from(format!("{other}:\\")));
        }
    }
    // a letter whose share is unknown may be this share
    out.extend(mapped.failed.iter().map(|l| PathBuf::from(format!("{l}:\\"))));
    out
}

pub fn progress() -> std::sync::Arc<sprawl_core::Progress> { sprawl_core::Progress::with_exclusions(&configured()) }

pub fn intersects(path: &Path, excluded: &[PathBuf]) -> bool {
    if excluded.is_empty() { return false; }
    if unreadable(excluded) { return true; }
    let text = path.to_string_lossy().replace('/', "\\");
    if text.starts_with(r"\\") && !text.starts_with(r"\\?\") && !text.starts_with(r"\\.\") || text.to_ascii_lowercase().starts_with(r"\\?\unc\") {
        if excluded.iter().any(|e| e.as_os_str() == sprawl_core::progress::REMOTE_UNKNOWN) { return true; }
    }
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

    #[test]
    fn network_aliases_cover_overlapping_and_unknown_mappings() {
        let mapped = Mapped { letters: vec![('Y', r"\\srv\share".into()), ('Z', r"\\srv\share\sub".into()), ('X', r"\\other\data".into())], failed: vec![] };
        let all = |p: &str| with_aliases_mapped(vec![PathBuf::from(p)], &mapped);
        // a skip on Y: inside the folder Z: maps: the share's name, and Z: below its own root
        let y = all(r"Y:\sub\private");
        for want in [r"\\srv\share\sub\private", r"Z:\private"] { assert!(y.contains(&PathBuf::from(want)), "{want} missing from {y:?}"); }
        assert!(!y.iter().any(|p| p.starts_with("X:")), "{y:?}");
        // and the other way round: a skip on Z: is also Y:\sub\...
        let z = all(r"Z:\private");
        for want in [r"\\srv\share\sub\private", r"Y:\sub\private"] { assert!(z.contains(&PathBuf::from(want)), "{want} missing from {z:?}"); }
        // a skip of a whole share folder that a letter maps inside of: that whole letter
        let whole = all(r"\\srv\share");
        for want in [r"Y:\", r"Z:\"] { assert!(whole.contains(&PathBuf::from(want)), "{want} missing from {whole:?}"); }
        let progress = sprawl_core::Progress::with_exclusions(&y);
        assert!(progress.excludes(Path::new(r"Z:\private\x")) && progress.excludes(Path::new(r"\\SRV\share\sub\private")));
        assert!(!progress.excludes(Path::new(r"\\srv\share\other")));
        // a network letter whose share can't be read: a skip on it refuses every share path and
        // every network letter; a skip elsewhere on the network refuses that letter
        let unknown = Mapped { letters: vec![('Y', r"\\srv\share".into())], failed: vec!['W'] };
        let on_w = with_aliases_mapped(vec![PathBuf::from(r"W:\private")], &unknown);
        assert!(intersects(Path::new(r"\\anything\at\all"), &on_w));
        assert!(sprawl_core::Progress::with_exclusions(&on_w).excludes(Path::new(r"\\anything\at\all")));
        assert!(intersects(Path::new(r"Y:\data"), &on_w) && intersects(Path::new(r"W:\other"), &on_w));
        assert!(!intersects(Path::new(r"C:\Users"), &on_w), "local drives stay scannable");
        let on_y = with_aliases_mapped(vec![PathBuf::from(r"Y:\private")], &unknown);
        assert!(intersects(Path::new(r"W:\x"), &on_y));
        assert!(!intersects(Path::new(r"\\other\share"), &on_y));
    }
}
