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
fn file_id(f: &File) -> Option<(u32, u64)> {
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
fn file_id(_: &File) -> Option<(u32, u64)> {
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
    let mut bx = vec![0u8; 1 << 20];
    let mut by = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        let (Ok(nx), Ok(ny)) = (fill(&mut x, &mut bx), fill(&mut y, &mut by)) else { return false };
        *read += (nx + ny) as u64;
        if nx != ny || bx[..nx] != by[..ny] {
            return false;
        }
        if nx == 0 {
            return true;
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
const NOT_LOCAL: u32 = 0x1000 | 0x0004_0000 | 0x0040_0000;

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
    let mut stack = vec![sprawl_core::ROOT];
    while let Some(k) = stack.pop() {
        let n = &tree.nodes[k];
        if n.is_dir {
            if k == sprawl_core::ROOT || !n.is_reparse_point() {
                stack.extend_from_slice(&n.children);
            }
        } else if n.size >= min_size && !n.is_reparse_point() && n.attributes & NOT_LOCAL == 0 {
            by_size.entry(n.size).or_default().push(k);
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
                // the finding points at a copy that may go
                let cl: Vec<(usize, PathBuf)> = free.into_iter().chain(kept).collect();
                let extra = size * may_go as u64;
                out.wasted += extra;
                let name = tree.nodes[cl[0].0].name.clone();
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
                evidence.push(if protected_kept {
                    "Delete only the copies outside protected places, or replace them with hard links to a kept one (New-Item -ItemType HardLink): then a change to one changes all.".into()
                } else {
                    "Keep one and delete the others, or, where a copy must stay in its place, replace it with a hard link to the one you keep (New-Item -ItemType HardLink): then a change to one changes all.".to_string()
                });
                out.findings.push(Finding {
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
                });
            }
        }
    }
    out.findings.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    out.findings.truncate(50);
    out
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
