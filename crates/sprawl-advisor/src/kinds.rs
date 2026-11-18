//! Knowing what things are (A2), by their layout. First kind: an archive next to the folder it
//! was extracted to. A `.zip`'s own index (its central directory: names and sizes, nothing
//! unpacked) is compared with the files the scan found; when every file in it is there, the
//! same size, the archive is a spare copy of the folder.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use sprawl_core::{format_size, Tree};

use crate::finding::{Action, Finding, Remedy, Risk, Source};

/// The rule id of an archive whose contents are extracted beside it.
pub const ARCHIVE: &str = "kind.archive-extracted";

/// An archive's index larger than this isn't read.
const MOST_INDEX: u64 = 32 << 20;
/// Nor one of more entries.
const MOST_ENTRIES: u64 = 200_000;

/// (path inside the archive with `\`, uncompressed size) of each file in a zip, from its central
/// directory. None when it isn't a zip Sprawl can read, or a name's encoding isn't known (an
/// old non-UTF-8 name with other than ASCII characters).
pub(crate) fn zip_files(path: &std::path::Path) -> Option<Vec<(String, u64)>> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    // the end record: in the last 64 KB (its comment) plus its own 22 bytes
    let tail_len = len.min(65_557);
    f.seek(SeekFrom::Start(len - tail_len)).ok()?;
    let mut tail = vec![0u8; tail_len as usize];
    f.read_exact(&mut tail).ok()?;
    let at = (0..tail.len().saturating_sub(21)).rev().find(|&i| tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])?;
    let u16_at = |b: &[u8], i: usize| -> Option<u64> { b.get(i..i + 2).map(|x| u64::from(u16::from_le_bytes([x[0], x[1]]))) };
    let u32_at = |b: &[u8], i: usize| -> Option<u64> { b.get(i..i + 4).map(|x| u64::from(u32::from_le_bytes([x[0], x[1], x[2], x[3]]))) };
    let u64_at = |b: &[u8], i: usize| -> Option<u64> { b.get(i..i + 8).map(|x| u64::from_le_bytes(x.try_into().unwrap())) };
    let eocd = &tail[at..];
    let (mut entries, mut cd_size, mut cd_at) = (u16_at(eocd, 10)?, u32_at(eocd, 12)?, u32_at(eocd, 16)?);
    if entries == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_at == 0xFFFF_FFFF {
        // ZIP64: a locator just before the end record points at the ZIP64 end record
        let loc = tail.get(at.checked_sub(20)?..at)?;
        if loc[..4] != [0x50, 0x4b, 0x06, 0x07] {
            return None;
        }
        let z_at = u64_at(loc, 8)?;
        f.seek(SeekFrom::Start(z_at)).ok()?;
        let mut z = [0u8; 56];
        f.read_exact(&mut z).ok()?;
        if z[..4] != [0x50, 0x4b, 0x06, 0x06] {
            return None;
        }
        entries = u64_at(&z, 32)?;
        cd_size = u64_at(&z, 40)?;
        cd_at = u64_at(&z, 48)?;
    }
    if cd_size > MOST_INDEX || entries > MOST_ENTRIES || cd_at.checked_add(cd_size)? > len {
        return None;
    }
    f.seek(SeekFrom::Start(cd_at)).ok()?;
    let mut cd = vec![0u8; cd_size as usize];
    f.read_exact(&mut cd).ok()?;
    let mut out = Vec::new();
    let mut i = 0usize;
    for _ in 0..entries {
        let h = cd.get(i..i + 46)?;
        if h[..4] != [0x50, 0x4b, 0x01, 0x02] {
            return None;
        }
        let flags = u16_at(h, 8)?;
        let mut size = u32_at(h, 24)?;
        let (n_len, x_len, c_len) = (u16_at(h, 28)? as usize, u16_at(h, 30)? as usize, u16_at(h, 32)? as usize);
        let name = cd.get(i + 46..i + 46 + n_len)?;
        let extra = cd.get(i + 46 + n_len..i + 46 + n_len + x_len)?;
        if size == 0xFFFF_FFFF {
            // the ZIP64 extra field: the uncompressed size comes first
            let mut j = 0;
            let mut found = None;
            while j + 4 <= extra.len() {
                let (id, l) = (u16_at(extra, j)?, u16_at(extra, j + 2)? as usize);
                if id == 1 {
                    found = u64_at(extra, j + 4);
                    break;
                }
                j += 4 + l;
            }
            size = found?;
        }
        let name = if flags & 0x800 != 0 || name.is_ascii() { String::from_utf8(name.to_vec()).ok()? } else { return None };
        if !name.ends_with('/') && !name.ends_with('\\') {
            out.push((name.replace('/', "\\"), size));
        }
        i += 46 + n_len + x_len + c_len;
    }
    Some(out)
}

/// The child of `n` called `name` (ignoring case).
fn child(tree: &Tree, n: usize, name: &str) -> Option<usize> {
    tree.nodes[n].children.iter().copied().find(|&c| tree.nodes[c].name.eq_ignore_ascii_case(name) || tree.nodes[c].name.to_lowercase() == name.to_lowercase())
}

/// The node at `rel` (`a\b\c`) below `n`, through no link.
fn below(tree: &Tree, mut n: usize, rel: &str) -> Option<usize> {
    for part in rel.split('\\').filter(|p| !p.is_empty()) {
        if part == "." || part == ".." || tree.nodes[n].is_reparse_point() && n != sprawl_core::ROOT {
            return None;
        }
        n = child(tree, n, part)?;
    }
    Some(n)
}

/// Zip archives of at least `min` bytes whose every file is extracted beside them, the same
/// size: in a folder named after the archive (`x.zip` and `x\`), or next to it (`x.zip` holding
/// `x/...`, extracted where it lies). The archive is offered for the Recycle Bin as the user's
/// own file; the folder stays.
pub fn archives(tree: &Tree, min: u64) -> Vec<Finding> {
    let never = crate::safety::Never::default_list();
    if !never.complete() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut stack = vec![sprawl_core::ROOT];
    while let Some(n) = stack.pop() {
        let node = &tree.nodes[n];
        if node.is_dir {
            if n == sprawl_core::ROOT || !node.is_reparse_point() {
                stack.extend_from_slice(&node.children);
            }
            continue;
        }
        let Some(stem) = node.name.len().checked_sub(4).filter(|_| node.name.to_lowercase().ends_with(".zip")).map(|i| &node.name[..i]) else { continue };
        if node.size < min || node.is_reparse_point() || stem.is_empty() {
            continue;
        }
        let Some(parent) = node.parent else { continue };
        let path = tree.path_of(n);
        if never.hit(&path.to_string_lossy().to_lowercase()).is_some() {
            continue;
        }
        let Some(files) = zip_files(&path) else { continue };
        let total: u64 = files.iter().map(|f| f.1).sum();
        if files.is_empty() || total == 0 {
            continue;
        }
        // where it was extracted: its own folder, or here when everything in it is under `stem\`
        let mut roots = Vec::new();
        if let Some(d) = child(tree, parent, stem).filter(|&d| tree.nodes[d].is_dir && !tree.nodes[d].is_reparse_point()) {
            roots.push(d);
        }
        let first = |p: &str| p.split('\\').next().unwrap_or("").to_lowercase();
        if files.iter().all(|(p, _)| first(p) == stem.to_lowercase() && p.contains('\\')) {
            roots.push(parent);
        }
        let Some(root) = roots.into_iter().find(|&r| files.iter().all(|(p, s)| below(tree, r, p).is_some_and(|k| k != n && !tree.nodes[k].is_dir && tree.nodes[k].size == *s))) else { continue };
        let folder = if root == parent { tree.path_of(child(tree, parent, stem).unwrap_or(parent)) } else { tree.path_of(root) };
        let mut f = Finding {
            path,
            owner: None,
            bytes: node.size,
            risk: Risk::Orange,
            remedy: Remedy::Delete,
            command: None,
            title: format!("{} (already extracted)", node.name),
            evidence: vec![
                format!("All {} files in it are in {}, the same size ({} unpacked)", files.len(), folder.display(), format_size(total)),
                "Read from the archive's own list of contents; nothing was unpacked or compared byte for byte".into(),
                "If you changed files in the folder since, the archive is the original: keep it if you may want that back".into(),
                "Only the archive goes; the folder stays".into(),
            ],
            confidence: 0.8,
            source: Source::Rule(ARCHIVE.into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: true,
            action: None,
            admin: false,
            carved: Vec::new(),
            gain: None,
            forced_from: None,
        };
        f.set_action(Action::Remove, false);
        out.push(f);
    }
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out.truncate(50);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stored (uncompressed) zip of `files`, written by hand: local headers, central directory,
    /// end record.
    fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, data) in files {
            let at = out.len() as u32;
            let mut local = vec![0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            local.extend((data.len() as u32).to_le_bytes());
            local.extend((data.len() as u32).to_le_bytes());
            local.extend((name.len() as u16).to_le_bytes());
            local.extend(0u16.to_le_bytes());
            out.extend(local);
            out.extend(name.as_bytes());
            out.extend(*data);
            let mut h = vec![0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend([0u8; 12]);
            h.extend(at.to_le_bytes());
            h.extend(name.as_bytes());
            cd.extend(h);
        }
        let cd_at = out.len() as u32;
        out.extend(&cd);
        out.extend([0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((cd.len() as u32).to_le_bytes());
        out.extend(cd_at.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    #[test]
    fn archives_already_extracted() {
        let dir = std::env::temp_dir().join(format!("sprawl-kinds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(r"mod\textures")).unwrap();
        std::fs::create_dir_all(dir.join(r"pack\pack")).unwrap();
        std::fs::create_dir_all(dir.join("half")).unwrap();
        let a = vec![1u8; 3000];
        let b = vec![2u8; 5000];
        // x.zip and x\
        std::fs::write(dir.join("mod.zip"), zip(&[("readme.txt", &a), ("textures/", b""), ("textures/rock.dds", &b)])).unwrap();
        std::fs::write(dir.join(r"mod\readme.txt"), &a).unwrap();
        std::fs::write(dir.join(r"mod\textures\rock.dds"), &b).unwrap();
        // pack.zip holding pack/..., extracted into pack\ (Explorer's Extract All: pack\pack\...)
        std::fs::write(dir.join("pack.zip"), zip(&[("pack/a.bin", &a)])).unwrap();
        std::fs::write(dir.join(r"pack\pack\a.bin"), &a).unwrap();
        // half extracted: one file missing
        std::fs::write(dir.join("half.zip"), zip(&[("a.bin", &a), ("b.bin", &b)])).unwrap();
        std::fs::write(dir.join(r"half\a.bin"), &a).unwrap();
        // a size differs
        std::fs::write(dir.join("other.zip"), zip(&[("half/a.bin", &b)])).unwrap();
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let found = archives(&tree, 1);
        let titles: Vec<&str> = found.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(titles.len(), 2, "{titles:?}");
        assert!(titles.contains(&"mod.zip (already extracted)") && titles.contains(&"pack.zip (already extracted)"), "{titles:?}");
        for f in &found {
            assert_eq!((f.remedy, f.risk, f.action.clone()), (Remedy::Delete, Risk::Orange, Some(Action::Remove)));
            assert!(f.path.extension().is_some_and(|e| e == "zip"), "only the archive goes");
            let never = crate::safety::Never::default_list();
            assert!(crate::safety::check_in(f, &tree, &never, &[], Some(&dir), None).is_ok(), "passes the checks");
        }
        assert_eq!(zip_files(&dir.join("mod.zip")).unwrap(), vec![("readme.txt".to_string(), 3000), (r"textures\rock.dds".to_string(), 5000)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn not_zips_and_broken_ones() {
        let dir = std::env::temp_dir().join(format!("sprawl-kinds-bad-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("a.zip"), b"PK not really").unwrap();
        assert_eq!(zip_files(&dir.join("a.zip")), None);
        let mut z = zip(&[("a.bin", b"abc")]);
        let n = z.len();
        z[n - 6] = 0xff; // the index's offset past the end
        std::fs::write(dir.join("b.zip"), &z).unwrap();
        assert_eq!(zip_files(&dir.join("b.zip")), None);
        std::fs::write(dir.join("c.zip"), b"").unwrap();
        assert_eq!(zip_files(&dir.join("c.zip")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
