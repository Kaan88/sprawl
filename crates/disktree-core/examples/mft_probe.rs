//! Compare the MFT scanner with the folder walk on a real volume. Needs an elevated prompt for
//! the MFT part (without it, scan_auto falls back to the walk and the report says so).
//!
//! ```text
//! cargo build --release -p disktree-core --example mft_probe
//! target\release\examples\mft_probe.exe C:\ mft-report.txt
//! ```
//!
//! Reads only: the volume is opened for reading, nothing is written except the report file.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use disktree_core::{format_size, is_elevated, mft, scan_auto, walk, Progress, ScanMethod, Tree, ROOT};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().unwrap_or_else(|| r"C:\".into()));
    let report_path = args.next().map(PathBuf::from);
    let mut r = String::new();

    let _ = writeln!(r, "DiskTree MFT probe - {}", path.display());
    let _ = writeln!(r, "elevated: {}   NTFS: {}   volume: {:?}", is_elevated(), mft::is_ntfs(&path), mft::volume_of(&path));
    let _ = writeln!(r, "node size: {} bytes", std::mem::size_of::<disktree_core::Node>());
    let _ = writeln!(r);

    // 1. scan_auto with live progress sampled like a UI would.
    let p = Progress::new();
    let t0 = Instant::now();
    let (auto, samples) = std::thread::scope(|s| {
        let job = s.spawn(|| scan_auto(&path, &p));
        let mut samples = Vec::new();
        let mut polls = 0u64;
        let mut worst_poll = Duration::ZERO;
        while !job.is_finished() {
            let q = Instant::now();
            let snap = p.snapshot();
            worst_poll = worst_poll.max(q.elapsed());
            polls += 1;
            if polls % 30 == 1 {
                samples.push(format!(
                    "  {:6.2}s {:<16} {:>5} {:>9} files {:>8} folders {:>10}  top: {}  ext: {}  current: {}",
                    t0.elapsed().as_secs_f64(),
                    snap.phase,
                    snap.fraction.map(|f| format!("{:.0}%", f * 100.0)).unwrap_or_else(|| "?".into()),
                    snap.files,
                    snap.folders,
                    format_size(snap.bytes),
                    snap.top.first().map(|t| format!("{} {}", t.0, format_size(t.1))).unwrap_or_default(),
                    snap.by_ext.first().map(|e| format!("{} {}", e.0, format_size(e.1))).unwrap_or_default(),
                    snap.current.chars().take(80).collect::<String>(),
                ));
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        samples.push(format!("  ({polls} snapshots, slowest {:.3} ms)", worst_poll.as_secs_f64() * 1000.0));
        (job.join().expect("scan thread"), samples)
    });
    let auto_secs = t0.elapsed().as_secs_f64();
    let auto = match auto {
        Ok(t) => t,
        Err(e) => {
            let _ = writeln!(r, "scan_auto failed: {e}");
            finish(&r, report_path);
            return;
        }
    };
    let method = p.snapshot().method;
    let _ = writeln!(r, "scan_auto: {:.2} s via {:?}{}", auto_secs, method, if method == ScanMethod::Mft { "" } else { "  (NOT the MFT - run elevated on an NTFS volume)" });
    for s in samples {
        let _ = writeln!(r, "{s}");
    }
    summary(&mut r, "scan_auto", &auto);

    // 2. The walk.
    let p2 = Progress::new();
    let t1 = Instant::now();
    let walked = walk::walk(&path, &p2);
    let walk_secs = t1.elapsed().as_secs_f64();
    let walked = match walked {
        Ok(t) => t,
        Err(e) => {
            let _ = writeln!(r, "walk failed: {e}");
            finish(&r, report_path);
            return;
        }
    };
    let _ = writeln!(r, "\nwalk: {:.2} s", walk_secs);
    summary(&mut r, "walk", &walked);

    // 3. Differences, top level and one level down where they differ most.
    let _ = writeln!(r, "\nTop-level differences (scan_auto minus walk), biggest size difference first:");
    let diffs = compare(&mut r, &auto, ROOT, &walked, ROOT, 60);
    for (name, _) in diffs.into_iter().take(6) {
        let (Some(a), Some(w)) = (child(&auto, ROOT, &name), child(&walked, ROOT, &name)) else { continue };
        let _ = writeln!(r, "\n  inside {name}:");
        compare(&mut r, &auto, a, &walked, w, 15);
    }
    finish(&r, report_path);
}

fn summary(r: &mut String, label: &str, t: &Tree) {
    let n = &t.nodes[ROOT];
    let _ = writeln!(
        r,
        "  {label}: {} files, {} folders, size {} ({} bytes), allocated {} ({} bytes), {} nodes, {} errors",
        n.files,
        n.folders,
        format_size(n.size),
        n.size,
        format_size(n.allocated),
        n.allocated,
        t.nodes.len(),
        t.errors
    );
}

fn child(t: &Tree, parent: usize, name: &str) -> Option<usize> {
    t.nodes[parent].children.iter().copied().find(|&c| t.nodes[c].name.eq_ignore_ascii_case(name))
}

/// Print children of `a` (in tree `ta`) against children of `w` (in tree `tw`); returns names by
/// absolute size difference.
fn compare(r: &mut String, ta: &Tree, a: usize, tw: &Tree, w: usize, limit: usize) -> Vec<(String, u64)> {
    let mut rows: HashMap<String, (String, Option<usize>, Option<usize>)> = HashMap::new();
    for &c in &ta.nodes[a].children {
        rows.entry(ta.nodes[c].name.to_lowercase()).or_insert((ta.nodes[c].name.clone(), None, None)).1 = Some(c);
    }
    for &c in &tw.nodes[w].children {
        rows.entry(tw.nodes[c].name.to_lowercase()).or_insert((tw.nodes[c].name.clone(), None, None)).2 = Some(c);
    }
    let mut v: Vec<(String, Option<usize>, Option<usize>, u64)> = rows
        .into_values()
        .map(|(name, x, y)| {
            let sx = x.map_or(0, |i| ta.nodes[i].size);
            let sy = y.map_or(0, |i| tw.nodes[i].size);
            (name, x, y, sx.abs_diff(sy))
        })
        .collect();
    v.sort_by(|p, q| q.3.cmp(&p.3).then_with(|| p.0.cmp(&q.0)));
    let _ = writeln!(r, "  {:<40} {:>16} {:>16} {:>14} {:>9} {:>9} {:>14}", "name", "size (auto)", "size (walk)", "diff", "files+-", "folders+-", "alloc (auto)");
    for (name, x, y, d) in v.iter().take(limit) {
        let g = |t: &Tree, i: &Option<usize>| i.map(|i| (t.nodes[i].size, t.nodes[i].files, t.nodes[i].folders, t.nodes[i].allocated));
        let (ax, wy) = (g(ta, x), g(tw, y));
        let sz = |o: Option<(u64, u64, u64, u64)>| o.map_or("-".to_string(), |s| s.0.to_string());
        let df = |k: fn(&(u64, u64, u64, u64)) -> u64| (ax.as_ref().map_or(0, k) as i64) - (wy.as_ref().map_or(0, k) as i64);
        let _ = writeln!(
            r,
            "  {:<40} {:>16} {:>16} {:>14} {:>+9} {:>+9} {:>14}",
            name.chars().take(40).collect::<String>(),
            sz(ax),
            sz(wy),
            format_size(*d),
            df(|s| s.1),
            df(|s| s.2),
            ax.map_or("-".into(), |s| format_size(s.3)),
        );
    }
    v.into_iter().map(|(n, _, _, d)| (n, d)).collect()
}

fn finish(r: &str, report: Option<PathBuf>) {
    print!("{r}");
    if let Some(p) = report {
        match std::fs::write(&p, r) {
            Ok(()) => println!("\nreport written to {}", p.display()),
            Err(e) => eprintln!("could not write {}: {e}", p.display()),
        }
    }
}
