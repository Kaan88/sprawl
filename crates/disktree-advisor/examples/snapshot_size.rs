//! Phase 0 size test: scan a path, write snapshots at several zstd levels, read one back and
//! check it, and print the numbers. Prints no paths.
//!
//!     cargo run --release -p disktree-advisor --example snapshot_size -- C:\

use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use disktree_advisor::snapshot;
use disktree_core::{scan_auto, Progress};

fn main() {
    let path = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| r"C:\".into()));
    let t = Instant::now();
    let tree = scan_auto(&path, &Progress::default()).expect("scan");
    let scan_s = t.elapsed().as_secs_f64();
    let n = tree.nodes.len();
    let names: std::collections::HashSet<&str> = tree.nodes.iter().map(|n| n.name.as_str()).collect();
    println!("scan: {n} items ({} distinct names), {:?}, {scan_s:.1} s", names.len(), tree.method);

    let taken = SystemTime::now();
    let t = Instant::now();
    let raw = snapshot::encode(&tree, taken);
    println!("raw columns: {:.1} MB ({:.1} bytes/item), encoded in {:.2} s", raw.len() as f64 / 1e6, raw.len() as f64 / n as f64, t.elapsed().as_secs_f64());

    let mut keep = Vec::new();
    for level in [3, 9, 15, 19] {
        let t = Instant::now();
        let mut buf = Vec::new();
        snapshot::write(&tree, taken, level, &mut buf).unwrap();
        let w = t.elapsed().as_secs_f64();
        println!(
            "zstd {level:>2}: {:>6.2} MB  ({:.2} bytes/item, 3.3M items -> {:.1} MB)  write {w:.2} s",
            buf.len() as f64 / 1e6,
            buf.len() as f64 / n as f64,
            buf.len() as f64 / n as f64 * 3.3e6 / 1e6
        );
        if level == 9 {
            keep = buf;
        }
    }
    let t = Instant::now();
    let (back, meta) = snapshot::read(&mut keep.as_slice()).expect("read back");
    let r = t.elapsed().as_secs_f64();
    let same = back.nodes.len() == n
        && back.nodes[0].size == tree.nodes[0].size
        && back.nodes[0].allocated == tree.nodes[0].allocated
        && back.nodes[0].files == tree.nodes[0].files
        && back.nodes[0].folders == tree.nodes[0].folders;
    println!("read back level 9 in {r:.2} s: {} items, totals match: {same}", meta.nodes);
    // Folders only, for long-range history.
    let t = Instant::now();
    let fo = snapshot::folders_only(&tree);
    let mut buf = Vec::new();
    snapshot::write(&fo, taken, 9, &mut buf).unwrap();
    println!(
        "folders only: {} nodes, zstd 9: {:.2} MB, root total matches: {}, built+written in {:.2} s",
        fo.nodes.len(),
        buf.len() as f64 / 1e6,
        fo.nodes[0].size == tree.nodes[0].size && fo.nodes[0].allocated == tree.nodes[0].allocated,
        t.elapsed().as_secs_f64()
    );
    // CSV for comparison (the format WizTree uses for exports)
    let mut csv = Vec::new();
    if disktree_core::export_csv(&tree, &mut csv).is_ok() {
        println!("same scan as CSV: {:.1} MB", csv.len() as f64 / 1e6);
    }
}
