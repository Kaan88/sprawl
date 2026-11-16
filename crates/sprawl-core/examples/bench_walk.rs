//! Times the folder walk against the old single-threaded std::fs scan.
//!
//! `cargo run --release -p sprawl-core --example bench_walk -- "C:\Program Files" [threads...]`

use std::path::PathBuf;
use std::time::Instant;

use sprawl_core::{scan_std, walk, Progress, ROOT};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().unwrap_or_else(|| r"C:\Program Files".into()));
    let threads: Vec<usize> = args.filter_map(|a| a.parse().ok()).collect();

    let t = Instant::now();
    let old = scan_std(&path, |_| {}).expect("scan_std");
    let r = &old.nodes[ROOT];
    println!("std::fs scan     {:7.2} s  files {} folders {} size {} errors {}", t.elapsed().as_secs_f64(), r.files, r.folders, r.size, old.errors);
    drop(old);

    let default_threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(4, 16);
    let runs = if threads.is_empty() { vec![default_threads, default_threads, default_threads] } else { threads };
    for n in runs {
        let p = Progress::new();
        let t = Instant::now();
        let tree = walk::walk_with(&path, &p, n).expect("walk");
        let r = &tree.nodes[ROOT];
        println!(
            "walk {:2} threads {:7.2} s  files {} folders {} size {} allocated {} errors {}",
            n,
            t.elapsed().as_secs_f64(),
            r.files,
            r.folders,
            r.size,
            r.allocated,
            tree.errors
        );
    }
}
