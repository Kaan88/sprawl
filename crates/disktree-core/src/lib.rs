//! disktree-core: the parts of a WizTree-style disk analyser that don't need a UI.
//!
//! * `scan` walks a folder with `std::fs` (links are not followed) into a flat `Tree`.
//! * Every folder gets its total size, item, file and folder counts.
//! * `extension_stats` groups files by extension; `largest_files` lists the biggest files.
//! * `squarify` and `treemap` lay sizes out as rectangles (the squarified treemap).
//!
//! Reading the NTFS master file table directly (how WizTree is fast) is out of scope here.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One file or folder.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub is_dir: bool,
    /// Bytes: the file's length, or the total of everything below a folder.
    pub size: u64,
    /// Files below a folder (1 for a file).
    pub files: u64,
    /// Folders below a folder, not counting itself.
    pub folders: u64,
    pub modified: Option<SystemTime>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

impl Node {
    /// Files plus folders below this one.
    pub fn items(&self) -> u64 {
        self.files + self.folders
    }

    /// Lower-case extension without the dot, or "(none)".
    pub fn extension(&self) -> String {
        extension_of(&self.name)
    }
}

/// A scanned folder. Children always have higher indices than their parent.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub root_path: PathBuf,
    pub nodes: Vec<Node>,
    /// Entries that couldn't be read (access denied and similar).
    pub errors: u64,
}

pub const ROOT: usize = 0;

pub fn extension_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => "(none)".to_string(),
    }
}

/// Walk `path` and build the tree. `progress` is called every 1000 entries with the count so far.
pub fn scan(path: &Path, mut progress: impl FnMut(u64)) -> std::io::Result<Tree> {
    let meta = fs::symlink_metadata(path)?;
    let mut tree = Tree { root_path: path.to_path_buf(), nodes: Vec::new(), errors: 0 };
    let name = path.display().to_string();
    tree.nodes.push(Node {
        name,
        is_dir: meta.is_dir(),
        size: if meta.is_dir() { 0 } else { meta.len() },
        files: if meta.is_dir() { 0 } else { 1 },
        folders: 0,
        modified: meta.modified().ok(),
        parent: None,
        children: Vec::new(),
    });

    let mut stack: Vec<(usize, PathBuf)> = Vec::new();
    if meta.is_dir() {
        stack.push((ROOT, path.to_path_buf()));
    }
    let mut seen: u64 = 0;
    while let Some((dir_index, dir_path)) = stack.pop() {
        let entries = match fs::read_dir(&dir_path) {
            Ok(e) => e,
            Err(_) => {
                tree.errors += 1;
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    tree.errors += 1;
                    continue;
                }
            };
            // symlink_metadata: links and junctions are counted as themselves, never followed.
            let m = match fs::symlink_metadata(entry.path()) {
                Ok(m) => m,
                Err(_) => {
                    tree.errors += 1;
                    continue;
                }
            };
            let is_dir = m.is_dir();
            let index = tree.nodes.len();
            tree.nodes.push(Node {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir,
                size: if is_dir { 0 } else { m.len() },
                files: if is_dir { 0 } else { 1 },
                folders: 0,
                modified: m.modified().ok(),
                parent: Some(dir_index),
                children: Vec::new(),
            });
            tree.nodes[dir_index].children.push(index);
            if is_dir {
                stack.push((index, entry.path()));
            }
            seen += 1;
            if seen % 1000 == 0 {
                progress(seen);
            }
        }
    }
    tree.compute_totals();
    progress(seen);
    Ok(tree)
}

impl Tree {
    /// Add every node's size and counts into its parent (children come after parents).
    pub fn compute_totals(&mut self) {
        for n in self.nodes.iter_mut().filter(|n| n.is_dir) {
            n.size = 0;
            n.files = 0;
            n.folders = 0;
        }
        for i in (1..self.nodes.len()).rev() {
            let (size, files, folders, is_dir) = {
                let n = &self.nodes[i];
                (n.size, n.files, n.folders, n.is_dir)
            };
            if let Some(p) = self.nodes[i].parent {
                let parent = &mut self.nodes[p];
                parent.size += size;
                parent.files += files;
                parent.folders += folders + u64::from(is_dir);
            }
        }
        for n in &mut self.nodes {
            n.children.shrink_to_fit();
        }
        let nodes = &self.nodes;
        let mut sorted: Vec<Vec<usize>> = nodes.iter().map(|n| n.children.clone()).collect();
        for c in &mut sorted {
            c.sort_by(|a, b| nodes[*b].size.cmp(&nodes[*a].size).then_with(|| nodes[*a].name.cmp(&nodes[*b].name)));
        }
        for (n, c) in self.nodes.iter_mut().zip(sorted) {
            n.children = c;
        }
    }

    /// Share of the parent's size, 0..=100 (100 for the root).
    pub fn percent_of_parent(&self, index: usize) -> f64 {
        match self.nodes[index].parent {
            None => 100.0,
            Some(p) if self.nodes[p].size == 0 => 0.0,
            Some(p) => self.nodes[index].size as f64 * 100.0 / self.nodes[p].size as f64,
        }
    }

    /// Full path of a node.
    pub fn path_of(&self, index: usize) -> PathBuf {
        let mut parts = Vec::new();
        let mut i = index;
        while let Some(p) = self.nodes[i].parent {
            parts.push(self.nodes[i].name.as_str());
            i = p;
        }
        let mut path = self.root_path.clone();
        for part in parts.iter().rev() {
            path.push(part);
        }
        path
    }
}

/// Totals for one file extension.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionStat {
    pub extension: String,
    pub size: u64,
    pub files: u64,
    /// Share of all file bytes, 0..=100.
    pub percent: f64,
}

/// Every extension, biggest total first.
pub fn extension_stats(tree: &Tree) -> Vec<ExtensionStat> {
    let mut map: HashMap<String, (u64, u64)> = HashMap::new();
    let mut total: u64 = 0;
    for n in tree.nodes.iter().filter(|n| !n.is_dir) {
        let e = map.entry(n.extension()).or_insert((0, 0));
        e.0 += n.size;
        e.1 += 1;
        total += n.size;
    }
    let mut v: Vec<ExtensionStat> = map
        .into_iter()
        .map(|(extension, (size, files))| ExtensionStat {
            extension,
            size,
            files,
            percent: if total == 0 { 0.0 } else { size as f64 * 100.0 / total as f64 },
        })
        .collect();
    v.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.extension.cmp(&b.extension)));
    v
}

/// Indices of the `limit` biggest files, biggest first.
pub fn largest_files(tree: &Tree, limit: usize) -> Vec<usize> {
    let mut v: Vec<usize> = (0..tree.nodes.len()).filter(|&i| !tree.nodes[i].is_dir).collect();
    v.sort_by(|a, b| tree.nodes[*b].size.cmp(&tree.nodes[*a].size).then_with(|| a.cmp(b)));
    v.truncate(limit);
    v
}

/// A colour for an extension, stable across runs (hue from a hash, like WizTree's per-type colours).
pub fn extension_colour(extension: &str) -> (u8, u8, u8) {
    // FNV-1a
    let mut h: u32 = 0x811c_9dc5;
    for b in extension.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    let hue = (h % 360) as f64;
    hsv_to_rgb(hue, 0.55, 0.85)
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |f: f64| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to(r), to(g), to(b))
}

/// Human size: "1.5 GB".
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["bytes", "KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.1} {}", UNITS[u])
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }
    pub fn area(&self) -> f64 {
        self.w * self.h
    }
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

fn worst(row: &[f64], side: f64) -> f64 {
    let s: f64 = row.iter().sum();
    if s <= 0.0 || side <= 0.0 {
        return f64::INFINITY;
    }
    let max = row.iter().cloned().fold(f64::MIN, f64::max);
    let min = row.iter().cloned().fold(f64::MAX, f64::min);
    let s2 = s * s;
    let side2 = side * side;
    f64::max(side2 * max / s2, s2 / (side2 * min))
}

/// Squarified treemap (Bruls, Huizing, van Wijk). Returns one rectangle per value, in input order;
/// values of zero or less get an empty rectangle.
pub fn squarify(values: &[f64], bounds: Rect) -> Vec<Rect> {
    let mut out = vec![Rect::new(bounds.x, bounds.y, 0.0, 0.0); values.len()];
    let total: f64 = values.iter().filter(|v| **v > 0.0).sum();
    if total <= 0.0 || bounds.area() <= 0.0 {
        return out;
    }
    let scale = bounds.area() / total;
    let mut order: Vec<usize> = (0..values.len()).filter(|&i| values[i] > 0.0).collect();
    order.sort_by(|a, b| values[*b].partial_cmp(&values[*a]).unwrap_or(std::cmp::Ordering::Equal));

    let mut free = bounds;
    let mut row: Vec<usize> = Vec::new();
    let mut row_areas: Vec<f64> = Vec::new();
    let mut i = 0;
    while i < order.len() {
        let a = values[order[i]] * scale;
        let side = free.w.min(free.h);
        let mut candidate = row_areas.clone();
        candidate.push(a);
        if row.is_empty() || worst(&candidate, side) <= worst(&row_areas, side) {
            row.push(order[i]);
            row_areas.push(a);
            i += 1;
        } else {
            place_row(&row, &row_areas, &mut free, &mut out);
            row.clear();
            row_areas.clear();
        }
    }
    if !row.is_empty() {
        place_row(&row, &row_areas, &mut free, &mut out);
    }
    out
}

fn place_row(row: &[usize], areas: &[f64], free: &mut Rect, out: &mut [Rect]) {
    let s: f64 = areas.iter().sum();
    if free.w >= free.h {
        // A column along the left edge.
        let thick = if free.h > 0.0 { (s / free.h).min(free.w) } else { 0.0 };
        let mut y = free.y;
        for (k, &idx) in row.iter().enumerate() {
            let h = if thick > 0.0 { areas[k] / thick } else { 0.0 };
            out[idx] = Rect::new(free.x, y, thick, h);
            y += h;
        }
        free.x += thick;
        free.w -= thick;
    } else {
        // A row along the top edge.
        let thick = if free.w > 0.0 { (s / free.w).min(free.h) } else { 0.0 };
        let mut x = free.x;
        for (k, &idx) in row.iter().enumerate() {
            let w = if thick > 0.0 { areas[k] / thick } else { 0.0 };
            out[idx] = Rect::new(x, free.y, w, thick);
            x += w;
        }
        free.y += thick;
        free.h -= thick;
    }
}

/// One file's rectangle in the whole-tree treemap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    pub node: usize,
    pub rect: Rect,
}

/// Nested treemap of the files below `start`: folders are split among their children,
/// and tiles smaller than `min_side` pixels are dropped. At most `max_tiles` are returned.
pub fn treemap(tree: &Tree, start: usize, bounds: Rect, min_side: f64, max_tiles: usize) -> Vec<Tile> {
    let mut tiles = Vec::new();
    let mut stack = vec![(start, bounds)];
    while let Some((index, rect)) = stack.pop() {
        if tiles.len() >= max_tiles || rect.w < min_side || rect.h < min_side {
            continue;
        }
        let node = &tree.nodes[index];
        if !node.is_dir {
            tiles.push(Tile { node: index, rect });
            continue;
        }
        let sizes: Vec<f64> = node.children.iter().map(|&c| tree.nodes[c].size as f64).collect();
        let rects = squarify(&sizes, rect);
        for (&child, r) in node.children.iter().zip(rects).rev() {
            stack.push((child, r));
        }
    }
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn squarify_covers_area_and_stays_inside() {
        let values = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let b = Rect::new(0.0, 0.0, 6.0, 4.0);
        let rects = squarify(&values, b);
        let total: f64 = rects.iter().map(Rect::area).sum();
        assert!(close(total, 24.0), "total {total}");
        for (r, v) in rects.iter().zip(values) {
            assert!(close(r.area(), v), "area {} for {v}", r.area());
            assert!(r.x >= -1e-9 && r.y >= -1e-9);
            assert!(r.x + r.w <= 6.0 + 1e-9 && r.y + r.h <= 4.0 + 1e-9);
        }
    }

    #[test]
    fn squarify_handles_zero_and_empty() {
        assert!(squarify(&[], Rect::new(0.0, 0.0, 10.0, 10.0)).is_empty());
        let r = squarify(&[0.0, 5.0], Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(close(r[0].area(), 0.0));
        assert!(close(r[1].area(), 100.0));
    }

    #[test]
    fn extensions_and_formatting() {
        assert_eq!(extension_of("a.TXT"), "txt");
        assert_eq!(extension_of(".gitignore"), "(none)");
        assert_eq!(extension_of("README"), "(none)");
        assert_eq!(format_size(512), "512 bytes");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(extension_colour("exe"), extension_colour("exe"));
    }

    #[test]
    fn scan_totals_and_stats() {
        let dir = std::env::temp_dir().join(format!("disktree-core-test-{}", std::process::id()));
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(dir.join("a.txt"), vec![0u8; 100]).unwrap();
        fs::write(sub.join("b.bin"), vec![0u8; 300]).unwrap();
        fs::write(sub.join("c.txt"), vec![0u8; 50]).unwrap();

        let tree = scan(&dir, |_| {}).unwrap();
        let root = &tree.nodes[ROOT];
        assert_eq!(root.size, 450);
        assert_eq!(root.files, 3);
        assert_eq!(root.folders, 1);
        assert_eq!(root.items(), 4);
        // Biggest child first: sub (350) before a.txt (100).
        assert_eq!(tree.nodes[root.children[0]].name, "sub");
        assert!(close(tree.percent_of_parent(root.children[0]), 350.0 * 100.0 / 450.0));

        let stats = extension_stats(&tree);
        assert_eq!(stats[0].extension, "bin");
        assert_eq!(stats[0].size, 300);
        assert_eq!(stats[1].extension, "txt");
        assert_eq!(stats[1].files, 2);

        let big = largest_files(&tree, 1);
        assert_eq!(tree.nodes[big[0]].name, "b.bin");
        assert_eq!(tree.path_of(big[0]), sub.join("b.bin"));

        let tiles = treemap(&tree, ROOT, Rect::new(0.0, 0.0, 90.0, 50.0), 0.5, 100);
        assert_eq!(tiles.len(), 3);
        let area: f64 = tiles.iter().map(|t| t.rect.area()).sum();
        assert!(close(area, 4500.0));

        // Leaves the temp folder in place: deleting is left to the system's temp cleanup.
    }
}
