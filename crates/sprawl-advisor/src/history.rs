//! Scan history: snapshots saved after each scan, the retention policy, and projecting an old
//! snapshot onto the current tree (for the time-lapse and "what changed").
//!
//! Layout: `%LOCALAPPDATA%\Sprawl\snapshots\<key>\` per scanned root, holding `root.txt`
//! (the root path) and `<unix seconds>.fo.dtsn` (folders only) and `<unix seconds>.full.dtsn`.
//!
//! Retention (see docs/plans/ai-advisor.md): every folders-only snapshot from the last 30
//! days, then the newest per week for 6 months, then the newest per month; the newest 3 full
//! snapshots per root; and a total cap that removes the oldest folders-only snapshots first.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sprawl_core::{Tree, ROOT};

use crate::snapshot::{self, FILES_HERE};

pub const KEEP_FULL: usize = 3;
const DAY: u64 = 86_400;

pub fn base_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|a| PathBuf::from(a).join("Sprawl").join("snapshots"))
}

/// A folder name for a root path: `C:\` -> `C`, `C:\Program Files` -> `C~Program Files`.
pub fn key(root: &Path) -> String {
    let s = root.to_string_lossy();
    let s = s.trim_end_matches(['\\', '/']);
    let k: String = s
        .chars()
        .map(|c| match c {
            '\\' | '/' => '~',
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    k.replace("_~", "~").trim_end_matches('_').to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub root: PathBuf,
    pub taken: SystemTime,
    pub file: PathBuf,
    pub full: bool,
    pub bytes: u64,
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Bodies are encoded by the caller (on its thread); this compresses and writes them.
pub struct Pending {
    /// The scanned folder's size, kept beside the snapshots (the welcome screen shows it).
    pub size: u64,
    pub root: PathBuf,
    pub taken: SystemTime,
    pub folders: Vec<u8>,
    pub full: Option<Vec<u8>>,
}

/// Encode the snapshots for a finished scan (fast; the slow part is [`Pending::save`]).
pub fn prepare(tree: &Tree, taken: SystemTime, full: bool) -> Pending {
    let folders = snapshot::encode(&snapshot::folders_only(tree), taken);
    let full = full.then(|| snapshot::encode(tree, taken));
    Pending { size: tree.nodes[sprawl_core::ROOT].size, root: tree.root_path.clone(), taken, folders, full }
}

impl Pending {
    /// Write into `base` and apply the retention policy with a total cap.
    pub fn save(self, base: &Path, cap_bytes: u64) -> io::Result<()> {
        let dir = base.join(key(&self.root));
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("root.txt"), self.root.to_string_lossy().as_bytes())?;
        let t = secs(self.taken);
        // "<when> <bytes>" of the last scan
        let _ = fs::write(dir.join("last.txt"), format!("{t} {}", self.size));
        let write = |name: String, body: &[u8]| -> io::Result<()> {
            let tmp = dir.join(format!("{name}.tmp"));
            let mut f = io::BufWriter::new(fs::File::create(&tmp)?);
            snapshot::write_body(body, 9, &mut f)?;
            drop(f);
            fs::rename(&tmp, dir.join(name))
        };
        write(format!("{t}.fo.dtsn"), &self.folders)?;
        if let Some(full) = &self.full {
            write(format!("{t}.full.dtsn"), full)?;
        }
        prune(base, cap_bytes, SystemTime::now())
    }
}

/// Each scanned root's last scan: (root, when, the folder's size then), newest first. Roots
/// saved before the size was kept are left out.
pub fn last_scans(base: &Path) -> Vec<(PathBuf, SystemTime, u64)> {
    let mut out = Vec::new();
    let Ok(dirs) = fs::read_dir(base) else { return out };
    for d in dirs.flatten() {
        let (Ok(root), Ok(last)) = (fs::read_to_string(d.path().join("root.txt")), fs::read_to_string(d.path().join("last.txt"))) else { continue };
        let mut it = last.split_whitespace().map(|v| v.parse::<u64>());
        let (Some(Ok(t)), Some(Ok(size))) = (it.next(), it.next()) else { continue };
        let Some(when) = UNIX_EPOCH.checked_add(Duration::from_secs(t)) else { continue };
        out.push((PathBuf::from(root.trim()), when, size));
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// Every snapshot under `base`, oldest first.
pub fn list(base: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let Ok(dirs) = fs::read_dir(base) else { return out };
    for d in dirs.flatten() {
        let Ok(root) = fs::read_to_string(d.path().join("root.txt")) else { continue };
        let Ok(files) = fs::read_dir(d.path()) else { continue };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let (stem, full) = if let Some(s) = name.strip_suffix(".full.dtsn") {
                (s, true)
            } else if let Some(s) = name.strip_suffix(".fo.dtsn") {
                (s, false)
            } else {
                continue;
            };
            let Ok(t) = stem.parse::<u64>() else { continue };
            // a file named with an absurd time is skipped, not a panic
            let Some(taken) = UNIX_EPOCH.checked_add(Duration::from_secs(t)) else { continue };
            out.push(Entry {
                root: PathBuf::from(root.trim()),
                taken,
                file: f.path(),
                full,
                bytes: f.metadata().map(|m| m.len()).unwrap_or(0),
            });
        }
    }
    out.sort_by(|a, b| a.taken.cmp(&b.taken).then(a.full.cmp(&b.full)));
    out
}

/// Snapshots that cover `root` (taken of it or of a folder above it), one per moment (the
/// full one when both exist), oldest first.
pub fn covering(base: &Path, root: &Path) -> Vec<Entry> {
    let want = crate::segments(&root.to_string_lossy());
    let mut v: Vec<Entry> = list(base)
        .into_iter()
        .filter(|e| {
            let r = crate::segments(&e.root.to_string_lossy());
            want.len() >= r.len() && want[..r.len()] == r[..]
        })
        .collect();
    // prefer the full snapshot of the same moment and root; then the closest root
    v.sort_by(|a, b| a.taken.cmp(&b.taken).then(b.full.cmp(&a.full)).then(b.root.as_os_str().len().cmp(&a.root.as_os_str().len())));
    v.dedup_by(|b, a| a.taken == b.taken);
    v
}

/// Which files the policy removes, given every entry and the time now.
pub fn doomed(entries: &[Entry], cap_bytes: u64, now: SystemTime) -> Vec<PathBuf> {
    let mut kill: Vec<PathBuf> = Vec::new();
    let mut roots: Vec<&PathBuf> = entries.iter().map(|e| &e.root).collect();
    roots.sort();
    roots.dedup();
    let now_s = secs(now);
    let mut kept_fo: Vec<&Entry> = Vec::new();
    for root in roots {
        let mine: Vec<&Entry> = entries.iter().filter(|e| &e.root == root).collect();
        // full: newest 3
        let mut full: Vec<&&Entry> = mine.iter().filter(|e| e.full).collect();
        full.sort_by(|a, b| b.taken.cmp(&a.taken));
        kill.extend(full.iter().skip(KEEP_FULL).map(|e| e.file.clone()));
        // folders only: buckets by age
        let mut fo: Vec<&Entry> = mine.iter().copied().filter(|e| !e.full).collect();
        fo.sort_by(|a, b| b.taken.cmp(&a.taken)); // newest first
        let mut seen = std::collections::HashSet::new();
        for e in fo {
            let age = now_s.saturating_sub(secs(e.taken));
            let bucket = if age <= 30 * DAY {
                None
            } else if age <= 183 * DAY {
                Some(("w", secs(e.taken) / (7 * DAY)))
            } else {
                Some(("m", secs(e.taken) / (30 * DAY)))
            };
            match bucket {
                Some(b) if !seen.insert(b) => kill.push(e.file.clone()),
                _ => kept_fo.push(e),
            }
        }
    }
    // the cap: oldest folders-only first, but never a root's newest snapshot
    let mut total: u64 = entries.iter().filter(|e| !kill.contains(&e.file)).map(|e| e.bytes).sum();
    if total > cap_bytes {
        let newest: Vec<(&PathBuf, SystemTime)> = {
            let mut m: std::collections::HashMap<&PathBuf, SystemTime> = std::collections::HashMap::new();
            for e in &kept_fo {
                let t = m.entry(&e.root).or_insert(e.taken);
                *t = (*t).max(e.taken);
            }
            m.into_iter().collect()
        };
        kept_fo.sort_by(|a, b| a.taken.cmp(&b.taken));
        for e in kept_fo {
            if total <= cap_bytes {
                break;
            }
            if newest.iter().any(|(r, t)| *r == &e.root && *t == e.taken) {
                continue;
            }
            total = total.saturating_sub(e.bytes);
            kill.push(e.file.clone());
        }
    }
    kill
}

pub fn prune(base: &Path, cap_bytes: u64, now: SystemTime) -> io::Result<()> {
    for f in doomed(&list(base), cap_bytes, now) {
        let _ = fs::remove_file(f);
    }
    Ok(())
}

pub fn load(e: &Entry) -> io::Result<Tree> {
    let mut f = io::BufReader::new(fs::File::open(&e.file)?);
    Ok(snapshot::read(&mut f)?.0)
}

/// The size every node of `cur` had in `old` (0 where it didn't exist), consistent so folders
/// are the sum of their children. Files a folders-only snapshot doesn't list individually get
/// a share of that folder's old `<files>` total in proportion to their size now.
pub fn project(cur: &Tree, old: &Tree) -> Vec<u64> {
    let mut w = vec![0u64; cur.nodes.len()];
    let Some(start) = crate::find_path(old, &cur.path_of(ROOT)) else { return w };
    let lower = |s: &str| s.to_lowercase();
    let mut stack = vec![(ROOT, start)];
    while let Some((c, o)) = stack.pop() {
        let cn = &cur.nodes[c];
        if !cn.is_dir {
            continue;
        }
        let on = &old.nodes[o];
        let mut by_name: std::collections::HashMap<String, usize> = std::collections::HashMap::with_capacity(on.children.len());
        let mut files_here = None;
        for &k in &on.children {
            if old.nodes[k].name == FILES_HERE && !old.nodes[k].is_dir {
                files_here = Some(old.nodes[k].size);
            } else {
                by_name.insert(lower(&old.nodes[k].name), k);
            }
        }
        let direct_now: u64 = cn.children.iter().filter(|&&k| !cur.nodes[k].is_dir).map(|&k| cur.nodes[k].size).sum();
        let mut shared = Vec::new();
        for &k in &cn.children {
            let kn = &cur.nodes[k];
            match by_name.get(&lower(&kn.name)) {
                Some(&ok) if old.nodes[ok].is_dir == kn.is_dir => {
                    if kn.is_dir {
                        stack.push((k, ok));
                    } else {
                        w[k] = old.nodes[ok].size;
                    }
                }
                _ if !kn.is_dir => shared.push(k),
                _ => {}
            }
        }
        // a folders-only snapshot: share the folder's old file total out by size now, exactly
        if let (Some(total), false) = (files_here, shared.is_empty()) {
            if direct_now > 0 {
                let mut given = 0u64;
                for &k in &shared {
                    w[k] = (total as u128 * cur.nodes[k].size as u128 / direct_now as u128) as u64;
                    given += w[k];
                }
                let big = *shared.iter().max_by_key(|&&k| cur.nodes[k].size).unwrap();
                w[big] += total - given;
            }
        }
    }
    // folders: sum of children (children come after parents)
    for i in (1..cur.nodes.len()).rev() {
        if cur.nodes[i].is_dir && cur.nodes[i].children.is_empty() {
            continue;
        }
        if let Some(p) = cur.nodes[i].parent {
            w[p] += w[i];
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(root: &str, days_ago: u64, full: bool, bytes: u64, now: SystemTime) -> Entry {
        let t = now - Duration::from_secs(days_ago * DAY);
        Entry { root: root.into(), taken: t, file: PathBuf::from(format!("{root}-{days_ago}-{full}")), full, bytes }
    }

    #[test]
    fn keys() {
        assert_eq!(key(Path::new(r"C:\")), "C");
        assert_eq!(key(Path::new(r"C:\Program Files")), "C~Program Files");
        assert_eq!(key(Path::new(r"\\server\share")), "~~server~share");
    }

    #[test]
    fn retention() {
        let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let mut v = Vec::new();
        for d in 0..400 {
            v.push(e("C", d, false, 5, now));
        }
        for d in 0..6 {
            v.push(e("C", d, true, 27, now));
        }
        let kill = doomed(&v, u64::MAX, now);
        let left: Vec<&Entry> = v.iter().filter(|x| !kill.contains(&x.file)).collect();
        assert_eq!(left.iter().filter(|x| x.full).count(), 3);
        assert!(left.iter().filter(|x| x.full).all(|x| x.taken >= now - Duration::from_secs(2 * DAY)));
        let fo = left.iter().filter(|x| !x.full).count();
        // 31 daily + about 22 weekly + about 7 monthly
        assert!((55..=65).contains(&fo), "{fo}");
        // a tight cap removes the oldest but keeps the newest
        let kill = doomed(&v, 100, now);
        let left: Vec<&Entry> = v.iter().filter(|x| !kill.contains(&x.file)).collect();
        assert!(left.iter().any(|x| !x.full && x.taken == now));
        assert!(left.iter().map(|x| x.bytes).sum::<u64>() <= 100 || left.iter().filter(|x| !x.full).count() == 1);
    }

    #[test]
    fn save_list_project() {
        let base = std::env::temp_dir().join(format!("dt-hist-{}", std::process::id()));
        let data = std::env::temp_dir().join(format!("dt-hist-data-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&data);
        fs::create_dir_all(data.join("sub")).unwrap();
        fs::write(data.join("sub").join("a.bin"), vec![1u8; 1000]).unwrap();
        fs::write(data.join("b.bin"), vec![1u8; 300]).unwrap();
        let old = sprawl_core::scan(&data, |_| {}).unwrap();
        // recent, so retention keeps both
        let t0 = UNIX_EPOCH + Duration::from_secs(secs(SystemTime::now()) - 3600);
        prepare(&old, t0, false).save(&base, u64::MAX).unwrap();

        fs::write(data.join("sub").join("a.bin"), vec![1u8; 4000]).unwrap();
        fs::write(data.join("new.bin"), vec![1u8; 50]).unwrap();
        let cur = sprawl_core::scan(&data, |_| {}).unwrap();
        prepare(&cur, t0 + Duration::from_secs(60), true).save(&base, u64::MAX).unwrap();

        let all = covering(&base, &data);
        assert_eq!(all.len(), 2);
        assert!(!all[0].full && all[1].full);
        let first = load(&all[0]).unwrap();
        let w = project(&cur, &first);
        let find = |t: &Tree, name: &str| t.nodes.iter().position(|n| n.name == name).unwrap();
        assert_eq!(w[find(&cur, "sub")], 1000);
        // b.bin and new.bin share the folder's old 300 bytes of direct files by size now
        assert_eq!(w[find(&cur, "b.bin")] + w[find(&cur, "new.bin")], 300);
        assert_eq!(w[ROOT], 1300);
        // a subfolder scan finds its part in the snapshot of the parent
        let sub = sprawl_core::scan(&data.join("sub"), |_| {}).unwrap();
        assert_eq!(covering(&base, &data.join("sub")).len(), 2);
        assert_eq!(project(&sub, &first)[ROOT], 1000);
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&data);
    }
}
