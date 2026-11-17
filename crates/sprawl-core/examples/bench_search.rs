//! Times what the app does after a scan: the name index, queries, sorts, the filter and the
//! file-type table; then the compact search-only form of the same scan (its size against the
//! tree plus the index, its queries and sorts). Prints counts and times only, no paths.
//!
//!     cargo run --release -p sprawl-core --example bench_search -- C:\

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use sprawl_core::compact::{CompactIndex, SortBy};
use sprawl_core::search::{search, Index, Query};
use sprawl_core::{extension_stats, filter_tree, largest_files, scan_auto, Progress, ROOT};

#[repr(C)]
#[derive(Default)]
struct Counters {
    cb: u32,
    faults: u32,
    peak_ws: usize,
    ws: usize,
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    pagefile: usize,
    peak_pagefile: usize,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> isize;
    fn K32GetProcessMemoryInfo(h: isize, c: *mut Counters, cb: u32) -> i32;
}

fn mem() -> (f64, f64) {
    let mut c = Counters { cb: std::mem::size_of::<Counters>() as u32, ..Default::default() };
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    (c.ws as f64 / 1048576.0, c.peak_ws as f64 / 1048576.0)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let path = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| r"C:\".into()));
    let (m0, _) = mem();
    let t = Instant::now();
    let tree = scan_auto(&path, &Progress::default()).expect("scan");
    let n = tree.nodes.len();
    let (m1, p1) = mem();
    println!("scan        {:9.0} ms  {n} items ({} files), {:?}, {} unreadable", ms(t), tree.nodes[ROOT].files, tree.method, tree.errors);
    println!("memory      tree {:.0} MB working set ({:.0} bytes/item), peak {:.0} MB", m1 - m0, (m1 - m0) * 1048576.0 / n as f64, p1);

    let t = Instant::now();
    let index = Index::build(&tree);
    let (m2, _) = mem();
    println!("index build {:9.0} ms  {:.0} MB reported, {:.0} MB working set", ms(t), index.memory() as f64 / 1048576.0, m2 - m1);

    // every folder's place in path order, built once per tree (what the app keeps)
    let t = Instant::now();
    let ranks = folder_ranks(&tree);
    println!("folder ranks {:8.0} ms  {} folders", ms(t), tree.nodes.iter().filter(|n| n.is_dir).count());

    let never = AtomicBool::new(false);
    for q in QUERIES {
        let query = Query::parse(q);
        let mut best = f64::MAX;
        let mut hits = Vec::new();
        for _ in 0..3 {
            let t = Instant::now();
            hits = search(&tree, &index, &query, &never).unwrap_or_default();
            best = best.min(ms(t));
        }
        // the sorts a header click does: by size, and by folder (two paths built per comparison)
        let mut by_size = hits.clone();
        let t = Instant::now();
        by_size.sort_by(|&a, &b| tree.nodes[b].size.cmp(&tree.nodes[a].size));
        let s_size = ms(t);
        // (two paths built per comparison: half a minute for millions of hits, so only for fewer)
        let s_folder = if hits.len() <= 200_000 {
            let mut v = hits.clone();
            let t = Instant::now();
            v.sort_by(|&a, &b| {
                let pa = tree.nodes[a].parent.map(|p| tree.path_of(p));
                let pb = tree.nodes[b].parent.map(|p| tree.path_of(p));
                pa.cmp(&pb)
            });
            ms(t)
        } else {
            -1.0
        };
        // the same sort with each folder path built once
        let t = Instant::now();
        let mut keyed: Vec<(String, usize)> = hits.iter().map(|&a| (tree.nodes[a].parent.map(|p| tree.path_of(p).to_string_lossy().to_lowercase()).unwrap_or_default(), a)).collect();
        keyed.sort();
        let s_keyed = ms(t);
        // by the folder ranks: numbers compared, no paths
        let mut v = hits.clone();
        let t = Instant::now();
        let rank = |n: usize| tree.nodes[n].parent.map_or(0, |p| ranks[p] as u64 + 1);
        v.sort_by(|&a, &b| rank(a).cmp(&rank(b)).then_with(|| tree.nodes[a].name.bytes().map(|c| c.to_ascii_lowercase()).cmp(tree.nodes[b].name.bytes().map(|c| c.to_ascii_lowercase()))));
        let s_ranked = ms(t);
        println!("query {q:<22} {best:8.1} ms  {:>9} hits   sort size {s_size:8.1} ms   sort folder {s_folder:9.1} ms   (keyed {s_keyed:8.1} ms, ranked {s_ranked:7.1} ms)", hits.len());
    }

    let t = Instant::now();
    let f = filter_tree(&tree, "*.dll");
    println!("filter *.dll {:8.0} ms  {} items kept", ms(t), f.nodes.len());
    let t = Instant::now();
    let l = largest_files(&tree, usize::MAX);
    println!("largest_files(all) {:6.0} ms  {} files", ms(t), l.len());
    let t = Instant::now();
    let e = extension_stats(&tree);
    println!("extension_stats {:9.0} ms  {} types", ms(t), e.len());
    let t = Instant::now();
    let c = tree.clone();
    println!("tree clone  {:9.0} ms  (what a delete or a sort does while another thread holds the tree)", ms(t));
    drop(c);
    let (m3, p3) = mem();
    println!("memory      now {:.0} MB, peak {:.0} MB", m3, p3);

    // the compact search-only form: built from the tree, then on its own
    let t = Instant::now();
    let compact = CompactIndex::build(&tree);
    let (m4, _) = mem();
    let per = |b: f64| b / n as f64;
    println!(
        "compact build {:7.0} ms  {:.0} MB reported ({:.1} bytes/item), {:.0} MB working set",
        ms(t),
        compact.memory() as f64 / 1048576.0,
        per(compact.memory() as f64),
        m4 - m3
    );
    println!(
        "bytes/item  tree {:.0} (working set) + index {:.1} = {:.0}   compact {:.1}   ({:.0}%)",
        per((m1 - m0) * 1048576.0),
        per(index.memory() as f64),
        per((m1 - m0) * 1048576.0 + index.memory() as f64),
        per(compact.memory() as f64),
        100.0 * compact.memory() as f64 / ((m1 - m0) * 1048576.0 + index.memory() as f64)
    );
    for q in QUERIES {
        let query = Query::parse(q);
        let mut best = f64::MAX;
        let mut hits = Vec::new();
        for _ in 0..3 {
            let t = Instant::now();
            hits = compact.search(&query, &never).unwrap_or_default();
            best = best.min(ms(t));
        }
        let mut v = hits.clone();
        let t = Instant::now();
        compact.sort(&mut v, SortBy::Size, true);
        let s_size = ms(t);
        let t = Instant::now();
        compact.sort(&mut v, SortBy::Name, false);
        let s_name = ms(t);
        // the first folder sort builds the folder ranks, the next ones reuse them
        let t = Instant::now();
        compact.sort(&mut v, SortBy::Folder, false);
        let s_folder = ms(t);
        println!("compact {q:<20} {best:8.1} ms  {:>9} hits   sort size {s_size:8.1} ms   name {s_name:8.1} ms   folder {s_folder:8.1} ms", hits.len());
    }
    let (m5, p5) = mem();
    println!("memory      now {:.0} MB, peak {:.0} MB (compact {:.0} MB of it, folder ranks included)", m5, p5, compact.memory() as f64 / 1048576.0);
}

/// The queries timed on the tree and on the compact form: the old shapes, then the new keywords.
const QUERIES: &[&str] = &[
    "e",
    "a",
    "report",
    "*.dll",
    "cache >1mb",
    r"\windows\ *.exe",
    "folder:node_modules",
    "!e",
    "zzzzqqqq",
    "*.log|*.tmp <1kb",
    "ext:jpg;png",
    "dm:lastweek",
    "dm:2025 ext:log",
    "size:100mb..2gb",
    "path:system32 ext:dll",
    r"regex:^[a-f0-9]{32}$",
    "case:README",
    "exact:desktop.ini",
    "depth:<=2 folder:",
];

/// Every folder's place in path order (a copy of the app's folder rank, so this example does not
/// depend on where that function lives).
fn folder_ranks(tree: &sprawl_core::Tree) -> Vec<u32> {
    let mut keys: Vec<(Box<[u8]>, u32)> = Vec::new();
    let root: Vec<u8> = tree.root_path.to_string_lossy().bytes().map(|c| c.to_ascii_lowercase()).collect();
    let mut stack: Vec<(usize, Vec<u8>)> = vec![(ROOT, root)];
    while let Some((n, path)) = stack.pop() {
        for &c in &tree.nodes[n].children {
            let child = &tree.nodes[c];
            if !child.is_dir {
                continue;
            }
            let mut p = Vec::with_capacity(path.len() + 1 + child.name.len());
            p.extend_from_slice(&path);
            if !p.ends_with(b"\\") && !p.ends_with(b"/") {
                p.push(b'\\');
            }
            p.extend(child.name.bytes().map(|c| c.to_ascii_lowercase()));
            stack.push((c, p));
        }
        keys.push((path.into_boxed_slice(), n as u32));
    }
    keys.sort_unstable();
    let mut rank = vec![u32::MAX; tree.nodes.len()];
    let mut place = 0u32;
    for (i, (k, n)) in keys.iter().enumerate() {
        if i > 0 && keys[i - 1].0 != *k {
            place = i as u32;
        }
        rank[*n as usize] = place;
    }
    rank
}
