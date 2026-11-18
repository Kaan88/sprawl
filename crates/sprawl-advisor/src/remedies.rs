//! Remedies that free space without deleting anything: compressing installed programs
//! (`compact /exe:...`, which Sprawl runs when ticked: F1), letting OneDrive keep files online
//! only, and moving big things to a drive with room (tips, for the user to do).

use std::path::Path;

use sprawl_core::{format_size, Tree, ROOT};

use crate::finding::{Action, Finding, Remedy, Risk, Source};
use crate::rules::ps_quote;
use crate::segments;

const GB: u64 = 1 << 30;
const COMPRESSED: u32 = 0x800;
/// OneDrive and other cloud files: the data is online only (a placeholder).
const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
const RECALL_ON_OPEN: u32 = 0x0004_0000;
const OFFLINE: u32 = 0x1000;
/// "Always keep on this device".
const PINNED: u32 = 0x0008_0000;
/// A file `compact /exe` compressed is stored behind a reparse point (WOF), not with the old
/// compressed attribute.
const REPARSE: u32 = 0x400;

/// How every compress command Sprawl writes begins (F1): what tells a compress run from a
/// tool's clean-up, which is measured by what the folder lost on disk, not in size.
pub const COMPRESS: &str = "compact.exe /c /s /a /i /q /exe:";

/// The command is one of Sprawl's own compress commands.
pub fn is_compress(a: &Action) -> bool {
    matches!(a, Action::Tool { command } if command.starts_with(COMPRESS))
}

/// The finding compresses its folder in place (nothing is deleted).
pub fn compresses(f: &Finding) -> bool {
    f.action.as_ref().is_some_and(is_compress)
}

/// What everything under `path` takes on disk, compressed files at their compressed size;
/// links are not followed. None when `path` can't be read.
pub fn on_disk(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCompressedFileSizeW(name: *const u16, high: *mut u32) -> u32;
    }
    let size = |p: &Path, len: u64| -> u64 {
        let w: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut high = 0u32;
        // SAFETY: a NUL-terminated wide path and a valid out pointer
        let low = unsafe { GetCompressedFileSizeW(w.as_ptr(), &mut high) };
        if low == u32::MAX && high == 0 {
            len
        } else {
            (high as u64) << 32 | low as u64
        }
    };
    let top = std::fs::symlink_metadata(path).ok()?;
    if !top.is_dir() {
        return Some(size(path, top.len()));
    }
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.file_type().is_symlink() {
                continue;
            }
            if m.is_dir() {
                use std::os::windows::fs::MetadataExt;
                // a junction is a link too
                if m.file_attributes() & REPARSE == 0 {
                    stack.push(e.path());
                }
            } else {
                total += size(&e.path(), m.len());
            }
        }
    }
    Some(total)
}

fn info(tree: &Tree, n: usize, id: &str, title: String, evidence: Vec<String>, bytes: u64, risk: Risk, remedy: Remedy) -> Finding {
    Finding {
        path: tree.path_of(n),
        owner: None,
        bytes,
        risk,
        remedy,
        command: None,
        title,
        evidence,
        confidence: 0.7,
        source: Source::Rule(format!("remedy.{id}")),
        allow_protected: false,
        trusted: true,
        builtin: true,
        on_path: false,
        action: None,
        admin: false,
        carved: Vec::new(),
        gain: None,
        forced_from: None,
    }
}

/// The share LZX compression usually saves, by file type (0 for formats already compressed).
fn lzx_ratio(ext: &str) -> f64 {
    match ext {
        "exe" | "dll" | "sys" | "ocx" | "node" | "pyd" | "so" => 0.45,
        "pdb" | "lib" | "obj" | "a" => 0.6,
        "json" | "xml" | "txt" | "js" | "css" | "html" | "htm" | "py" | "lua" | "ini" | "cfg" | "log" | "md" | "ts" | "map" => 0.65,
        // extension() says "(none)" for a file without one
        "dat" | "bin" | "pak" | "" | "(none)" => 0.15,
        _ => 0.0,
    }
}

/// Installed programs that `compact /exe:lzx` would shrink a lot: big folders right under
/// Program Files (or a games library) whose files aren't compressed yet.
pub fn compact(tree: &Tree) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut stack = vec![ROOT];
    while let Some(n) = stack.pop() {
        let node = &tree.nodes[n];
        if !node.is_dir || (n != ROOT && node.is_reparse_point()) {
            continue;
        }
        let segs = segments(&tree.path_of(n).to_string_lossy());
        let program_dir = segs.len() == 3 && (segs[1] == "program files" || segs[1] == "program files (x86)") || segs.len() >= 4 && segs[segs.len() - 2] == "common" && segs.iter().any(|s| s == "steamapps");
        if !program_dir {
            if segs.len() < 6 {
                stack.extend(node.children.iter().copied().filter(|&c| tree.nodes[c].is_dir));
            }
            continue;
        }
        // Steam's own folder holds its games, which are suggested one by one: look inside it
        if node.name.eq_ignore_ascii_case("steam") {
            stack.extend(node.children.iter().copied().filter(|&c| tree.nodes[c].is_dir));
            continue;
        }
        // Store apps: nobody may write there, and taking ownership breaks them
        if node.size < 2 * GB || node.name.eq_ignore_ascii_case("windowsapps") {
            continue;
        }
        // already compressed (on disk smaller than the data)? then nothing to gain
        if node.allocated < node.size / 10 * 9 {
            continue;
        }
        let mut saving = 0f64;
        let mut s2 = vec![n];
        while let Some(k) = s2.pop() {
            let x = &tree.nodes[k];
            if x.is_dir {
                if !x.is_reparse_point() {
                    s2.extend_from_slice(&x.children);
                }
            } else if x.attributes & (COMPRESSED | REPARSE) == 0 && x.size >= 4096 {
                saving += x.size as f64 * lzx_ratio(&x.extension());
            }
        }
        let path = tree.path_of(n);
        let game = segs.iter().any(|s| s == "steamapps");
        // the plan's defaults (future-features.md F1): games read big files and gain from the
        // bigger chunks; programs in daily use start faster with the smaller ones. Both save a
        // little less than LZX, which the ratios are for.
        let (alg, share) = if game { ("xpress16k", 0.85) } else { ("xpress8k", 0.75) };
        let saving = (saving * share) as u64;
        if saving < GB {
            continue;
        }
        out.push(info(
            tree,
            n,
            "compact",
            format!("Compress {}", tree.nodes[n].name),
            vec![
                format!("Windows can store these program files compressed ({}): about {} of {} saved, the program runs as before. Nothing is deleted.", alg.to_uppercase(), format_size(saving), format_size(node.size)),
                "Tick it and Clean up, and Sprawl runs Windows' own compact.exe on it. That takes minutes on a big folder; files in use are skipped. Close the program first.".into(),
                if game { "An update or 'verify files' in Steam writes some files back uncompressed; compress it again after big updates. A few games with anti-cheat check their files: if one complains, uncompress it.".into() } else { "An update writes changed files back uncompressed; compress it again after big updates.".into() },
                format!("Undo with: compact /u /s /a /i /exe {}", ps_quote(&format!("{}\\*", path.display()))),
            ],
            // the folder's own size (what the safety checks compare); the saving is its gain
            node.size,
            Risk::Green,
            Remedy::Command,
        ));
        if let Some(f) = out.last_mut() {
            f.gain = Some(saving);
            // Sprawl's own command, from the built-in rules: compact.exe on the folder's files.
            // It deletes nothing, so the never-list (Program Files is on it) doesn't veto it, as
            // for DISM; it acts on the files inside, never on the folder or through a link.
            f.allow_protected = true;
            let pf = |v: &str| std::env::var_os(v).is_some_and(|d| crate::finding::within(&path, Path::new(&d)));
            let admin = pf("ProgramFiles") || pf("ProgramFiles(x86)");
            // single-quoted: a folder name can't run anything
            f.set_action(Action::Tool { command: format!("{COMPRESS}{alg} {}", ps_quote(&format!("{}\\*", path.display()))) }, admin);
        }
    }
    out.sort_by(|a, b| b.expected_gain().cmp(&a.expected_gain()));
    out.truncate(10);
    out
}

/// OneDrive folders with a lot kept on this PC that hasn't changed in 90 days: OneDrive can keep
/// it online only (downloaded again when opened).
pub fn onedrive(tree: &Tree) -> Vec<Finding> {
    let now = std::time::SystemTime::now();
    let mut out = Vec::new();
    let mut stack = vec![ROOT];
    while let Some(n) = stack.pop() {
        let node = &tree.nodes[n];
        let is_onedrive = n != ROOT && node.name.to_lowercase().starts_with("onedrive");
        // OneDrive's own folder may carry a cloud reparse mark: it is still looked at (only
        // what the scan holds is counted, and nothing is read from disk here)
        if !node.is_dir || (n != ROOT && node.is_reparse_point() && !is_onedrive) {
            continue;
        }
        if !is_onedrive {
            if segments(&tree.path_of(n).to_string_lossy()).len() < 4 {
                stack.extend(node.children.iter().copied().filter(|&c| tree.nodes[c].is_dir));
            }
            continue;
        }
        let (mut local, mut pinned) = (0u64, 0u64);
        let mut s2 = vec![n];
        while let Some(k) = s2.pop() {
            let x = &tree.nodes[k];
            if x.is_dir {
                s2.extend_from_slice(&x.children);
                continue;
            }
            let online_only = x.attributes & (RECALL_ON_DATA_ACCESS | RECALL_ON_OPEN | OFFLINE) != 0;
            let old = x.modified.and_then(|m| now.duration_since(m).ok()).is_some_and(|d| d.as_secs() > 90 * 86_400);
            if !online_only && old {
                local += x.size;
                if x.attributes & PINNED != 0 {
                    pinned += x.size;
                }
            }
        }
        if local < 2 * GB {
            continue;
        }
        let path = tree.path_of(n);
        let mut ev = vec![
            format!("{} of files untouched for 3 months are stored on this PC as well as in OneDrive.", format_size(local)),
            "Right-click the folder in Explorer > Free up space: they stay listed and download again when opened. Needs the internet then.".into(),
            format!("Or in PowerShell: attrib +U -P {} /s /d", ps_quote(&format!("{}\\*", path.display()))),
        ];
        if pinned > 0 {
            ev.push(format!("{} of it is set to 'Always keep on this device'; Free up space undoes that.", format_size(pinned)));
        }
        let mut f = info(tree, n, "onedrive", format!("{}: {} can be online only", node.name, format_size(local)), ev, node.size, Risk::Amber, Remedy::Info);
        f.gain = Some(local);
        out.push(f);
    }
    out
}

/// Big things that could live on another drive with room, when this one is filling up.
pub fn moves(tree: &Tree, drives: &[(char, u64, u64)]) -> Vec<Finding> {
    let root = tree.path_of(ROOT);
    let s = root.to_string_lossy();
    let Some(letter) = s.chars().next().filter(|c| c.is_ascii_alphabetic() && s.get(1..2) == Some(":")) else { return Vec::new() };
    let letter = letter.to_ascii_uppercase();
    let Some(&(_, total, free)) = drives.iter().find(|d| d.0 == letter) else { return Vec::new() };
    // only when this drive is getting full, and another has real room
    if total == 0 || free > total / 6 {
        return Vec::new();
    }
    let Some(&(other, _, other_free)) = drives.iter().filter(|d| d.0 != letter).max_by_key(|d| d.2) else { return Vec::new() };
    if other_free < 50 * GB {
        return Vec::new();
    }
    let room = format!("{other}: has {} free", format_size(other_free));
    let mut out = Vec::new();
    let mut add = |n: usize, id: &str, title: String, how: &str| {
        let bytes = tree.nodes[n].size;
        if bytes >= 10 * GB && bytes < other_free {
            out.push(info(tree, n, id, title, vec![room.clone(), how.to_string()], bytes, Risk::Amber, Remedy::Move));
        }
    };
    let find = |p: &str| crate::find_path(tree, Path::new(p));
    let profile = std::env::var("USERPROFILE").unwrap_or_default();
    for (sub, label) in [("Videos", "Videos"), ("Downloads", "Downloads"), ("Pictures", "Pictures"), ("Music", "Music")] {
        if let Some(n) = find(&format!("{profile}\\{sub}")) {
            let how = format!("Create {other}:\\{label} first, then right-click {label} > Properties > Location, choose that new folder and Move. Never choose {other}:\\ itself: the whole drive would become your {label} folder.");
            add(n, "move-known-folder", format!("Move {label} to {other}:"), &how);
        }
    }
    // Steam libraries on this drive
    let mut stack = vec![ROOT];
    while let Some(n) = stack.pop() {
        let node = &tree.nodes[n];
        if !node.is_dir || (n != ROOT && node.is_reparse_point()) {
            continue;
        }
        if node.name.eq_ignore_ascii_case("steamapps") {
            if let Some(c) = node.children.iter().copied().find(|&c| tree.nodes[c].name.eq_ignore_ascii_case("common")) {
                add(c, "move-steam", format!("Move Steam games to {other}:"), "Steam > Settings > Storage: add a library on the other drive, then select games and Move. Steam copies them and keeps them working.");
            }
            continue;
        }
        if segments(&tree.path_of(n).to_string_lossy()).len() < 5 {
            stack.extend(node.children.iter().copied().filter(|&c| tree.nodes[c].is_dir));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratios() {
        assert!(lzx_ratio("dll") > 0.3 && lzx_ratio("mp4") == 0.0 && lzx_ratio("zip") == 0.0 && lzx_ratio("(none)") > 0.0);
    }

    /// A built tree: OneDrive with old local files, online-only files and a pinned one.
    #[test]
    fn onedrive_counts_old_local_files() {
        use sprawl_core::{Node, ScanMethod};
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(200 * 86_400);
        let node = |name: &str, dir: bool, size: u64, attributes: u32, parent: Option<usize>, children: Vec<usize>| Node {
            name: name.into(),
            is_dir: dir,
            size,
            allocated: size,
            files: if dir { 0 } else { 1 },
            folders: 0,
            modified: Some(old),
            attributes,
            parent,
            children,
        };
        let mut tree = Tree {
            root_path: r"C:\Users\sam".into(),
            nodes: vec![
                node(r"C:\Users\sam", true, 0, 0, None, vec![1]),
                // the sync root marked as a cloud reparse point
                node("OneDrive", true, 0, 0x400, Some(0), vec![2, 3, 4]),
                node("photos.zip", false, 3 * GB, 0, Some(1), vec![]),
                node("video.mp4", false, 5 * GB, RECALL_ON_DATA_ACCESS, Some(1), vec![]),
                node("pinned.iso", false, GB, PINNED, Some(1), vec![]),
            ],
            errors: 0,
            method: ScanMethod::Walk,
        };
        tree.compute_totals();
        let f = onedrive(&tree);
        assert_eq!(f.len(), 1);
        assert!(f[0].title.contains("4.0 GB"), "local and old: 3 GB + the pinned 1 GB, not the online-only 5 GB: {}", f[0].title);
        assert!(f[0].evidence.iter().any(|e| e.contains("Always keep")));
        assert!(f[0].evidence.iter().any(|e| e.contains("attrib +U -P '")), "{:?}", f[0].evidence);
    }

    #[test]
    fn shown_commands_quote_paths() {
        for name in ["x$(calc)", "x`y", "x\u{201d}; calc"] {
            let q = ps_quote(&format!("D:\\Games\\{name}\\*"));
            assert!(q.starts_with('\'') && q.ends_with('\''), "{q}");
            assert!(!q[1..q.len() - 1].replace("''", "").contains('\''), "{q}");
        }
    }

    #[test]
    fn moves_only_when_full_and_room_elsewhere() {
        let tree = sprawl_core::scan(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").as_path(), |_| {}).unwrap();
        // this scan's folder isn't on a drive that is nearly full: nothing
        assert!(moves(&tree, &[('C', 100 * GB, GB), ('D', 500 * GB, 400 * GB)]).is_empty());
    }
}

#[cfg(test)]
mod compress_tests {
    use super::*;

    /// Sprawl's own compress command, run as a clean-up runs it, makes the folder smaller on
    /// disk and keeps every file as it was.
    #[test]
    fn compress_runs_and_keeps_the_files() {
        let dir = std::env::temp_dir().join(format!("sprawl-compress-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let text: String = (0..60_000).map(|i| format!("line {i} of a very compressible text file\n")).collect();
        std::fs::write(dir.join("a.txt"), &text).unwrap();
        std::fs::write(dir.join("sub").join("b.json"), &text).unwrap();
        let before = on_disk(&dir).unwrap();
        let a = Action::Tool { command: format!("{COMPRESS}xpress8k {}", crate::rules::ps_quote(&format!("{}\\*", dir.display()))) };
        assert!(is_compress(&a));
        let o = crate::clean::run_with(&dir, &a, &crate::clean::Spare::default(), None, &std::sync::atomic::AtomicBool::new(false), &mut |_| {});
        let after = on_disk(&dir).unwrap();
        assert!(o.problems.is_empty(), "{:?} {:?}", o.problems, o.output);
        assert!(after < before / 2, "before {before}, after {after}, {:?}", o.output);
        assert_eq!(std::fs::read_to_string(dir.join("sub").join("b.json")).unwrap(), text);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
