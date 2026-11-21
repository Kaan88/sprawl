//! Duplicate files: the same size, then the same first and last 64 KB, then the same bytes
//! all the way through (compared, not hashed). Hard links of one file (WinSxS and System32 share
//! most of theirs) are one file, not copies. Information findings: which copies there are and
//! how to keep one. Nothing is deleted or linked by Sprawl.

use std::collections::HashMap;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use sprawl_core::{format_size, Tree};

use crate::finding::{Finding, Remedy, Risk, Source};

/// (volume serial, file index): the same for every hard link of one file.
#[cfg(windows)]
pub(crate) fn file_id(f: &File) -> Option<(u32, u64)> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    #[derive(Default)]
    struct Info {
        attributes: u32,
        created: [u32; 2],
        accessed: [u32; 2],
        written: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(h: *mut std::ffi::c_void, info: *mut Info) -> i32;
    }
    let mut info = Info::default();
    let ok = unsafe { GetFileInformationByHandle(f.as_raw_handle(), &mut info) } != 0;
    ok.then(|| (info.volume, (u64::from(info.index_high) << 32) | u64::from(info.index_low)))
}

#[cfg(not(windows))]
pub(crate) fn file_id(_: &File) -> Option<(u32, u64)> {
    None
}

/// A hash of the first and last 64 KB.
fn sample(f: &mut File, size: u64) -> Option<u64> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut buf = vec![0u8; 64 << 10];
    f.seek(SeekFrom::Start(0)).ok()?;
    let n = fill(f, &mut buf).ok()?;
    buf[..n].hash(&mut h);
    if size > 128 << 10 {
        f.seek(SeekFrom::Start(size - (64 << 10))).ok()?;
        let n = fill(f, &mut buf).ok()?;
        buf[..n].hash(&mut h);
    }
    Some(h.finish())
}

/// Read until `buf` is full or the file ends (one `read` may return less).
fn fill(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// The same bytes all the way through. Counts what it read into `read`.
fn same(a: &PathBuf, b: &PathBuf, read: &mut u64, cancel: &AtomicBool) -> bool {
    let (Ok(mut x), Ok(mut y)) = (open(a), open(b)) else { return false };
    compare(&mut x, &mut y, read, cancel) == Some(true)
}

/// Two open files compared from their start: None when one couldn't be read or `cancel` was set.
pub(crate) fn same_open(x: &mut File, y: &mut File, cancel: &AtomicBool) -> Option<bool> {
    x.seek(SeekFrom::Start(0)).ok()?;
    y.seek(SeekFrom::Start(0)).ok()?;
    compare(x, y, &mut 0, cancel)
}

fn compare(x: &mut File, y: &mut File, read: &mut u64, cancel: &AtomicBool) -> Option<bool> {
    let mut bx = vec![0u8; 1 << 20];
    let mut by = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let (Ok(nx), Ok(ny)) = (fill(x, &mut bx), fill(y, &mut by)) else { return None };
        *read += (nx + ny) as u64;
        if nx != ny || bx[..nx] != by[..ny] {
            return Some(false);
        }
        if nx == 0 {
            return Some(true);
        }
    }
}

pub struct Dupes {
    pub findings: Vec<Finding>,
    /// Bytes that copies take beyond one of each.
    pub wasted: u64,
    /// Stopped early (the read budget ran out, or cancelled).
    pub partial: bool,
}

/// Look for duplicate files of at least `min_size` bytes, reading at most `budget` bytes in
/// full comparisons. `progress` gets a line now and then.
/// Cloud placeholders and offline files: reading them would download them.
pub(crate) const NOT_LOCAL: u32 = 0x1000 | 0x0004_0000 | 0x0040_0000;

/// Open without recalling a cloud file's data.
fn open(p: &std::path::Path) -> std::io::Result<File> {
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.custom_flags(0x0010_0000); // FILE_FLAG_OPEN_NO_RECALL
    }
    o.open(p)
}

pub fn find(tree: &Tree, min_size: u64, budget: u64, cancel: &AtomicBool, progress: &mut dyn FnMut(String)) -> Dupes {
    let never = crate::safety::Never::default_list();
    // a copy in a protected place (Windows, Program Files, the Installer cache, a VM disk) is
    // the one to keep
    let protected = |p: &std::path::Path| never.hit(&p.to_string_lossy().to_lowercase()).is_some() || !never.complete();
    let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
    // originals left behind by a link run that was interrupted between the two names
    let mut interrupted: Vec<usize> = Vec::new();
    let mut stack = vec![sprawl_core::ROOT];
    while let Some(k) = stack.pop() {
        let n = &tree.nodes[k];
        if n.is_dir {
            if k == sprawl_core::ROOT || !n.is_reparse_point() {
                stack.extend_from_slice(&n.children);
            }
        } else {
            // by bytes, not a lower-cased copy: this runs for every file node in the scan
            let b = n.name.as_bytes();
            if b.len() > BACKUP_SUFFIX.len() && b[b.len() - BACKUP_SUFFIX.len()..].eq_ignore_ascii_case(BACKUP_SUFFIX.as_bytes()) {
                interrupted.push(k);
            }
            if n.size >= min_size && !n.is_reparse_point() && n.attributes & NOT_LOCAL == 0 {
                by_size.entry(n.size).or_default().push(k);
            }
        }
    }
    let mut groups: Vec<(u64, Vec<usize>)> = by_size.into_iter().filter(|(_, v)| v.len() > 1).collect();
    // the biggest possible waste first, so a budget cut keeps what matters
    groups.sort_by_key(|(s, v)| std::cmp::Reverse(s * (v.len() as u64 - 1)));
    let mut out = Dupes { findings: Vec::new(), wasted: 0, partial: false };
    let mut read = 0u64;
    let total = groups.len();
    for (gi, (size, nodes)) in groups.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) || read > budget {
            out.partial = true;
            break;
        }
        if gi % 20 == 0 {
            progress(format!("Comparing files of the same size: {gi} of {total} groups, {} read", format_size(read)));
        }
        // one entry per real file (hard links collapse), then by sample
        let mut seen = std::collections::HashSet::new();
        let mut by_sample: HashMap<u64, Vec<(usize, PathBuf)>> = HashMap::new();
        for n in nodes {
            if cancel.load(Ordering::Relaxed) {
                out.partial = true;
                break;
            }
            let p = tree.path_of(n);
            let Ok(mut f) = open(&p) else { continue };
            if let Some(id) = file_id(&f) {
                if !seen.insert(id) {
                    continue;
                }
            }
            if let Some(h) = sample(&mut f, size) {
                read += (128 << 10).min(size);
                by_sample.entry(h).or_default().push((n, p));
            }
        }
        for (_, cands) in by_sample.into_iter().filter(|(_, v)| v.len() > 1) {
            // clusters of identical files, each checked against its first member
            let mut clusters: Vec<Vec<(usize, PathBuf)>> = Vec::new();
            for c in cands {
                // the budget and Stop count inside a group too
                if cancel.load(Ordering::Relaxed) || read > budget {
                    out.partial = true;
                    break;
                }
                progress(format!("Comparing {} files byte for byte, {} read", format_size(size), format_size(read)));
                match clusters.iter_mut().find(|cl| same(&cl[0].1, &c.1, &mut read, cancel)) {
                    Some(cl) => cl.push(c),
                    None => clusters.push(vec![c]),
                }
            }
            for mut cl in clusters.into_iter().filter(|cl| cl.len() > 1) {
                // copies in protected places stay; if every copy is protected, it isn't a suggestion
                let (kept, free): (Vec<_>, Vec<_>) = cl.drain(..).partition(|(_, p)| protected(p));
                if free.is_empty() {
                    continue;
                }
                // every protected copy stays, and is named; only the others may go
                let keep_note = (!kept.is_empty()).then(|| format!("Keep {}: in a protected place.", kept.iter().map(|(_, p)| p.display().to_string()).collect::<Vec<_>>().join(" and ")));
                let may_go = if kept.is_empty() { free.len() - 1 } else { free.len() };
                if may_go == 0 {
                    continue;
                }
                // F4: with no protected copy, all on one drive and none synced by OneDrive (it
                // doesn't follow hard links), the copies can become links to the plainest-named
                let linkable = kept.is_empty() && one_drive_letter(free.iter().map(|(_, p)| p.as_path())) && !free.iter().any(|(_, p)| synced(p));
                let mut free = free;
                if linkable {
                    let k = (0..free.len()).min_by_key(|&i| (tree.nodes[free[i].0].name.len(), free[i].1.as_os_str().len())).unwrap_or(0);
                    let keeper = free.remove(k);
                    free.push(keeper);
                }
                // the finding points at a copy that may go
                let cl: Vec<(usize, PathBuf)> = free.into_iter().chain(kept).collect();
                let extra = size * may_go as u64;
                out.wasted += extra;
                // named by the plainest of them: "album.flac", not "album (1).flac"
                let name = cl.iter().map(|(n, _)| tree.nodes[*n].name.as_str()).min_by_key(|n| (n.len(), *n)).unwrap_or_default().to_string();
                let mut evidence = vec![format!("{} identical copies, {} each: byte for byte the same.", cl.len(), format_size(size))];
                for (_, p) in cl.iter().take(8) {
                    evidence.push(p.display().to_string());
                }
                if cl.len() > 8 {
                    evidence.push(format!("and {} more", cl.len() - 8));
                }
                let protected_kept = keep_note.is_some();
                if let Some(k) = keep_note {
                    evidence.push(k);
                }
                let keep_path = cl.last().map(|(_, p)| p.clone()).unwrap_or_default();
                if linkable {
                    evidence.push(format!(
                        "Link: keeps {} and makes the other {} hard links to it. Every path stays and opens the same file; the space of one is used. Then a change to one changes all, and each takes the kept one's dates, owner, permissions and marks. A copy whose owner, permissions or hidden, system, compressed or encrypted marks differ from the kept one is left alone, so this often does nothing for copies in different folders. Each is compared again just before.",
                        keep_path.display(),
                        if cl.len() == 2 { "copy".to_string() } else { format!("{} copies", cl.len() - 1) }
                    ));
                } else {
                    evidence.push(if protected_kept {
                        "Delete only the copies outside protected places, or replace them with hard links to a kept one (New-Item -ItemType HardLink): then a change to one changes all.".into()
                    } else {
                        "Keep one and delete the others, or, where a copy must stay in its place, replace it with a hard link to the one you keep (New-Item -ItemType HardLink): then a change to one changes all.".to_string()
                    });
                }
                let mut f = Finding {
                    path: cl[0].1.clone(),
                    owner: None,
                    bytes: extra,
                    // copies of the user's own files are theirs to choose between
                    risk: Risk::Orange,
                    remedy: Remedy::Info,
                    command: None,
                    title: format!("{} copies of {name}", cl.len()),
                    evidence,
                    confidence: 1.0,
                    source: Source::Rule("remedy.duplicates".into()),
                    allow_protected: false,
                    trusted: true,
                    builtin: true,
                    on_path: false,
                    action: None,
                    admin: false,
                    carved: Vec::new(),
                    gain: None,
                    forced_from: None,
                };
                if linkable {
                    f.remedy = Remedy::Command;
                    let copies: Vec<PathBuf> = cl[..cl.len() - 1].iter().map(|(_, p)| p.clone()).collect();
                    f.set_action(crate::finding::Action::Link { keep: keep_path, copies }, false);
                }
                out.findings.push(f);
            }
        }
    }
    out.findings.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    // the notice is added after the cut: it frees nothing, so by size it would always be the
    // first thing dropped, and a drive with fifty duplicate sets is exactly where it matters
    out.findings.truncate(49);
    if !interrupted.is_empty() {
        out.findings.insert(0, interrupted_link(tree, &interrupted));
    }
    out
}

/// The name a link run gives the original while it has no name of its own.
pub(crate) const BACKUP_SUFFIX: &str = ".sprawl-original";

/// The rule id of the notice about files left under that name.
pub const INTERRUPTED: &str = "remedy.interrupted-link";

/// Files left under that name: the link run was interrupted (Sprawl ended, or the PC lost
/// power) between the rename and the link. The file is whole; only its name is the backup one.
fn interrupted_link(tree: &Tree, nodes: &[usize]) -> Finding {
    let bytes: u64 = nodes.iter().map(|&n| tree.nodes[n].size).sum();
    let mut evidence = vec![
        "Sprawl renames a copy to this before replacing it with a link to the file it matches. A name like this left behind means that run was interrupted: the file is whole and is the original, only under its backup name.".to_string(),
        format!("If nothing holds its real name (take \"{BACKUP_SUFFIX}\" off the end and look), the rename puts it back. If something does, the link was already made and this is a spare copy of the same bytes, which you can delete once you have checked that."),
    ];
    for &n in nodes.iter().take(8) {
        evidence.push(tree.path_of(n).display().to_string());
    }
    if nodes.len() > 8 {
        evidence.push(format!("and {} more", nodes.len() - 8));
    }
    Finding {
        path: tree.path_of(nodes[0]),
        owner: None,
        bytes: 0,
        // the user's own file under the wrong name: nothing here is Sprawl's to remove
        risk: Risk::Orange,
        remedy: Remedy::Info,
        command: None,
        title: format!(
            "{} left by an interrupted link, {}",
            if nodes.len() == 1 { "A file".to_string() } else { format!("{} files", nodes.len()) },
            format_size(bytes)
        ),
        evidence,
        confidence: 1.0,
        source: Source::Rule(INTERRUPTED.into()),
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

/// All on one drive letter (`C:`): hard links can't cross volumes (checked again when linking).
fn one_drive_letter<'a>(mut paths: impl Iterator<Item = &'a std::path::Path>) -> bool {
    let letter = |p: &std::path::Path| {
        let s = p.to_string_lossy().to_lowercase();
        let b = s.as_bytes();
        (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\').then(|| b[0])
    };
    let Some(first) = paths.next().and_then(letter) else { return false };
    paths.all(|p| letter(p) == Some(first))
}

/// Inside a OneDrive folder: it syncs each path as its own file and doesn't keep links.
fn synced(p: &std::path::Path) -> bool {
    let low = p.to_string_lossy().to_lowercase();
    ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"].iter().any(|v| std::env::var_os(v).is_some_and(|d| {
        let d = d.to_string_lossy().to_lowercase();
        !d.is_empty() && low.starts_with(&format!("{}\\", d.trim_end_matches('\\')))
    }))
}

// ---------------------------------------------------------------- from the scan alone (A3)

/// Files only named alike count as one name: `movie.mp4`, `movie (1).mp4`, `movie - Copy.mp4`.
fn same_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let (stem, ext) = match lower.rfind('.') {
        Some(i) if i > 0 => (&lower[..i], &lower[i..]),
        _ => (lower.as_str(), ""),
    };
    let mut s = stem.trim_end().to_string();
    loop {
        let before = s.len();
        // " (2)", "(2)"
        if s.ends_with(')') {
            if let Some(i) = s.rfind('(') {
                if i > 0 && s[i + 1..s.len() - 1].chars().all(|c| c.is_ascii_digit()) && i + 2 < s.len() {
                    s.truncate(i);
                }
            }
        }
        for suffix in [" - copy", " copy", "_copy", "-copy"] {
            if let Some(t) = s.strip_suffix(suffix) {
                s = t.to_string();
            }
        }
        s = s.trim_end().to_string();
        if s.len() == before {
            break;
        }
    }
    format!("{s}{ext}")
}

fn hash_of(parts: &[(String, u64, u64)]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

/// Copies found from the scan alone, nothing read: files of the same size and name in two
/// places (at least `min_file` bytes), and whole folders with the same file names and sizes all
/// the way down (at least `min_folder` bytes): a copied game, a backup of a backup. "Probably":
/// "Find duplicate files" reads the files to be sure. Protected places (Windows, Program Files)
/// and cloud files are left out.
pub fn likely(tree: &Tree, min_file: u64, min_folder: u64) -> Vec<Finding> {
    let never = crate::safety::Never::default_list();
    if !never.complete() {
        return Vec::new();
    }
    let protected = |n: usize| never.hit(&tree.path_of(n).to_string_lossy().to_lowercase()).is_some();
    let n_nodes = tree.nodes.len();
    // post-order: a folder's signature from its children's (name, size, signature)
    let mut sig = vec![0u64; n_nodes];
    let mut files_in = vec![0u64; n_nodes];
    let mut order = Vec::with_capacity(n_nodes);
    let mut stack = vec![(sprawl_core::ROOT, false)];
    while let Some((k, done)) = stack.pop() {
        let node = &tree.nodes[k];
        if done || !node.is_dir {
            order.push(k);
            continue;
        }
        stack.push((k, true));
        if k == sprawl_core::ROOT || !node.is_reparse_point() {
            stack.extend(node.children.iter().map(|&c| (c, false)));
        }
    }
    for &k in &order {
        let node = &tree.nodes[k];
        if node.is_dir {
            let mut parts: Vec<(String, u64, u64)> = node.children.iter().map(|&c| (tree.nodes[c].name.to_lowercase(), tree.nodes[c].size, sig[c])).collect();
            parts.sort();
            sig[k] = hash_of(&parts);
            files_in[k] = node.children.iter().map(|&c| if tree.nodes[c].is_dir { files_in[c] } else { 1 }).sum();
        } else {
            sig[k] = hash_of(&[(node.name.to_lowercase(), node.size, 0)]);
        }
    }
    let mut out = Vec::new();
    // folders: the biggest groups first; a group inside one already named is part of it
    let mut by_sig: HashMap<(u64, u64), Vec<usize>> = HashMap::new();
    for &k in &order {
        let node = &tree.nodes[k];
        if k != sprawl_core::ROOT && node.is_dir && !node.is_reparse_point() && node.size >= min_folder && files_in[k] >= 2 {
            by_sig.entry((sig[k], node.size)).or_default().push(k);
        }
    }
    let mut groups: Vec<Vec<usize>> = by_sig.into_values().filter(|v| v.len() > 1).collect();
    groups.sort_by_key(|g| std::cmp::Reverse(tree.nodes[g[0]].size * (g.len() as u64 - 1)));
    let mut named: Vec<PathBuf> = Vec::new();
    let inside_named = |p: &std::path::Path, named: &[PathBuf]| named.iter().any(|d| crate::finding::within(p, d));
    for g in groups {
        let paths: Vec<PathBuf> = g.iter().map(|&n| tree.path_of(n)).collect();
        // nested copies (a folder and its own copy inside it) or already named
        if paths.iter().any(|p| inside_named(p, &named)) || paths.iter().any(|a| paths.iter().any(|b| a != b && crate::finding::within(a, b))) {
            continue;
        }
        if g.iter().any(|&n| protected(n)) {
            continue;
        }
        let size = tree.nodes[g[0]].size;
        let mut evidence = vec![format!(
            "The same {} file names and sizes all the way down, {} each. Sprawl hasn't read them: the names and sizes say they are probably copies.",
            files_in[g[0]],
            format_size(size)
        )];
        evidence.extend(paths.iter().take(8).map(|p| p.display().to_string()));
        evidence.push("Often a copied game or a backup of a backup. Check which one a program or a backup uses before removing the other.".into());
        out.push(likely_finding(paths[0].clone(), size * (g.len() as u64 - 1), format!("Probably {} copies of the folder {}", g.len(), tree.nodes[g[0]].name), evidence, "remedy.folder-dupes"));
        named.extend(paths);
    }
    // files: same size and name, outside the folders named above
    let mut by_name: HashMap<(u64, String), Vec<usize>> = HashMap::new();
    for &k in &order {
        let node = &tree.nodes[k];
        if !node.is_dir && node.size >= min_file && !node.is_reparse_point() && node.attributes & NOT_LOCAL == 0 {
            by_name.entry((node.size, same_name(&node.name))).or_default().push(k);
        }
    }
    let mut groups: Vec<(u64, Vec<usize>)> = by_name.into_iter().filter(|(_, v)| v.len() > 1).map(|((s, _), v)| (s, v)).collect();
    groups.sort_by_key(|(s, g)| std::cmp::Reverse(s * (g.len() as u64 - 1)));
    for (size, g) in groups {
        let g: Vec<usize> = g.into_iter().filter(|&n| !protected(n)).collect();
        if g.len() < 2 {
            continue;
        }
        let paths: Vec<PathBuf> = g.iter().map(|&n| tree.path_of(n)).collect();
        if paths.iter().all(|p| inside_named(p, &named)) {
            continue;
        }
        let mut evidence = vec![format!("The same name and size, {} each, in {} places. Sprawl hasn't read them: Find duplicate files compares them byte for byte.", format_size(size), g.len())];
        evidence.extend(paths.iter().take(8).map(|p| p.display().to_string()));
        if paths.len() > 8 {
            evidence.push(format!("and {} more", paths.len() - 8));
        }
        // named by the plainest of them: "film.mkv", not "film (1).mkv"
        let name = g.iter().map(|&n| tree.nodes[n].name.as_str()).min_by_key(|n| (n.len(), *n)).unwrap_or_default();
        out.push(likely_finding(paths[0].clone(), size * (g.len() as u64 - 1), format!("Probably {} copies of {}", g.len(), name), evidence, LIKELY));
    }
    out.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    out.truncate(50);
    out
}

/// The source of a file group found from the scan alone; "Find duplicate files" replaces them.
pub const LIKELY: &str = "remedy.likely-dupes";

fn likely_finding(path: PathBuf, bytes: u64, title: String, evidence: Vec<String>, id: &str) -> Finding {
    Finding {
        path,
        owner: None,
        bytes,
        risk: Risk::Orange,
        remedy: Remedy::Info,
        command: None,
        title,
        evidence,
        confidence: 0.6,
        source: Source::Rule(id.into()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_found_links_and_lookalikes_not() {
        let dir = std::env::temp_dir().join(format!("sprawl-dupes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir.join("a").join("movie.bin"), &data).unwrap();
        std::fs::write(dir.join("b").join("movie copy.bin"), &data).unwrap();
        // same size, same start and end, one byte different in the middle
        let mut other = data.clone();
        other[150_000] ^= 1;
        std::fs::write(dir.join("b").join("lookalike.bin"), &other).unwrap();
        // a hard link is the same file, not a copy
        std::fs::hard_link(dir.join("a").join("movie.bin"), dir.join("a").join("link.bin")).unwrap();
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let d = find(&tree, 1000, 1 << 30, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(d.findings.len(), 1, "{:?}", d.findings.iter().map(|f| &f.evidence).collect::<Vec<_>>());
        assert!(d.findings[0].title.starts_with("2 copies"), "{}", d.findings[0].title);
        assert_eq!(d.wasted, 300_000);
        // F4: one drive, nothing protected: a link to the plainest name
        let f = &d.findings[0];
        assert_eq!(f.remedy, Remedy::Command);
        assert!(f.command.is_none(), "never in a script");
        assert_eq!(f.action, Some(crate::finding::Action::Link { keep: dir.join("a").join("movie.bin"), copies: vec![dir.join("b").join("movie copy.bin")] }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn id_of(p: &std::path::Path) -> (u32, u64) {
        file_id(&File::open(p).unwrap()).unwrap()
    }

    /// F4 from start to end: found, checked, run. A copy changed after the search stays; one
    /// that is the same becomes the kept file under its own name.
    #[test]
    fn copies_become_links_and_changed_ones_stay() {
        use crate::clean::{self, Action, Spare};
        let dir = std::env::temp_dir().join(format!("sprawl-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["a", "b", "c"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 249) as u8).collect();
        std::fs::write(dir.join("a").join("song.flac"), &data).unwrap();
        std::fs::write(dir.join("b").join("song (1).flac"), &data).unwrap();
        std::fs::write(dir.join("c").join("song - Copy.flac"), &data).unwrap();
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let d = find(&tree, 1000, 1 << 30, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(d.findings.len(), 1);
        let never = crate::safety::Never::default_list();
        let f = crate::safety::check_in(&d.findings[0], &tree, &never, &[], Some(&dir), None).expect("passes the checks");
        assert_eq!(f.bytes, 400_000);
        let (action, admin) = clean::action(&f).expect("Sprawl runs it");
        assert!(!admin);
        let Action::Link { keep, copies } = &action else { panic!("{action:?}") };
        assert_eq!(keep, &dir.join("a").join("song.flac"));
        // a forged one is refused, by the checks and by the gate
        let mut forged = f.clone();
        forged.source = Source::Rule("local.mine".into());
        assert!(crate::safety::check_in(&forged, &tree, &never, &[], Some(&dir), None).is_err());
        assert!(clean::action(&forged).is_none());
        // one copy changes after the search
        let mut other = data.clone();
        other[100_000] ^= 1;
        std::fs::write(&copies[1], &other).unwrap();
        let o = clean::run(&f.path, &action, &Spare::default(), &AtomicBool::new(false), &mut |_| {});
        assert_eq!((o.freed, o.files), (200_000, 1), "{:?}", o.problems);
        assert!(o.problems.iter().any(|p| p.contains("no longer the same bytes")), "{:?}", o.problems);
        assert_eq!(id_of(keep), id_of(&copies[0]), "now one file");
        assert_ne!(id_of(keep), id_of(&copies[1]));
        assert_eq!(std::fs::read(&copies[0]).unwrap(), data);
        assert!(!copies[0].with_file_name("song (1).flac.sprawl-link").exists());
        // run again: already one file, nothing more
        let o = clean::run(&f.path, &action, &Spare::default(), &AtomicBool::new(false), &mut |_| {});
        assert_eq!(o.freed, 0);
        // a copy the user keeps, or new since the scan, stays
        // ("new since the scan" counts from just before this rewrite, not a fixed 5 s before
        // the last run, which a slow, busy test run could exceed)
        let rewritten = std::time::SystemTime::now() - std::time::Duration::from_secs(1);
        std::fs::write(&copies[1], &data).unwrap();
        let o = clean::run(&f.path, &action, &Spare { since: None, keep: vec![copies[1].clone()], ..Default::default() }, &AtomicBool::new(false), &mut |_| {});
        assert_eq!((o.freed, o.kept_chosen), (0, 1));
        let o = clean::run(&f.path, &action, &Spare { since: Some(rewritten), keep: vec![], ..Default::default() }, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(o.freed, 0, "{:?}", o.problems);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_alike_are_one_name() {
        assert_eq!(same_name("Movie (1).MP4"), "movie.mp4");
        assert_eq!(same_name("movie - Copy.mp4"), "movie.mp4");
        assert_eq!(same_name("movie - Copy (2).mp4"), "movie.mp4");
        assert_eq!(same_name("report 2024.pdf"), "report 2024.pdf");
        assert_eq!(same_name("(1)"), "(1)");
    }

    /// A copied folder is one finding, its files aren't listed again; loose copies of a file
    /// are found by name and size, nothing read.
    #[test]
    fn likely_copies_from_the_scan() {
        let dir = std::env::temp_dir().join(format!("sprawl-likely-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for top in ["game", r"backup\game"] {
            std::fs::create_dir_all(dir.join(top).join("data")).unwrap();
            std::fs::write(dir.join(top).join("game.exe"), vec![1u8; 4000]).unwrap();
            std::fs::write(dir.join(top).join("data").join("level.pak"), vec![2u8; 9000]).unwrap();
        }
        // same names, one size different: not a copy
        std::fs::create_dir_all(dir.join("other").join("data")).unwrap();
        std::fs::write(dir.join("other").join("game.exe"), vec![1u8; 4000]).unwrap();
        std::fs::write(dir.join("other").join("data").join("level.pak"), vec![2u8; 9001]).unwrap();
        std::fs::create_dir_all(dir.join("dl")).unwrap();
        std::fs::write(dir.join("dl").join("film.mkv"), vec![3u8; 20000]).unwrap();
        std::fs::write(dir.join("film (1).mkv"), vec![4u8; 20000]).unwrap();
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let f = likely(&tree, 10_000, 10_000);
        let titles: Vec<&str> = f.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(titles, vec!["Probably 2 copies of film.mkv", "Probably 2 copies of the folder game"], "{titles:?}");
        assert_eq!(f[1].bytes, 13_000);
        assert!(f.iter().all(|f| f.remedy == Remedy::Info && f.action.is_none()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
