//! Times what the app does after a scan: the name index, queries, sorts, the filter and the
//! file-type table. Prints counts and times only, no paths.
//!
//!     cargo run --release -p sprawl-core --example bench_search -- C:\

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

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

    let never = AtomicBool::new(false);
    for q in ["e", "a", "report", "*.dll", "cache >1mb", r"\windows\ *.exe", "folder:node_modules", "!e", "zzzzqqqq", "*.log|*.tmp <1kb"] {
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
        let s_folder = if hits.len() <= 3_000_000 {
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
        println!("query {q:<22} {best:8.1} ms  {:>9} hits   sort size {s_size:8.1} ms   sort folder {s_folder:9.1} ms   (keyed {s_keyed:8.1} ms)", hits.len());
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
}
