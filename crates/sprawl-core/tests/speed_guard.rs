//! Release guard against scan slowdowns like 1.0's (every folder's whole ancestor chain opened
//! before listing it: 15-55x slower). Timing-sensitive, so ignored in normal runs; the release
//! tool runs it with `--ignored` in release mode before building. Disposable data only.
use std::{fs, path::Path, time::{Duration, Instant}};

/// The least work any folder scan must do: list every folder and read each entry's metadata.
fn plain_listing(dir: &Path) -> (u64, u64) {
    let (mut files, mut bytes) = (0, 0);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let m = e.metadata().unwrap();
            if m.is_dir() { stack.push(e.path()); } else { files += 1; bytes += m.len(); }
        }
    }
    (files, bytes)
}

fn best_of(runs: usize, mut f: impl FnMut()) -> Duration {
    (0..runs).map(|_| { let start = Instant::now(); f(); start.elapsed() }).min().unwrap()
}

#[test]
#[ignore = "timing; run by sprawl-release build"]
fn folder_inventory_stays_close_to_a_plain_listing() {
    // Deep and wide: 3^6 = 729 leaf folders six levels down, 1,093 folders, 2,186 files.
    let root = std::env::temp_dir().join(format!("sprawl-speed-guard-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let mut level = vec![root.clone()];
    for _ in 0..6 {
        level = level.iter().flat_map(|p| (0..3).map(move |i| p.join(format!("level-folder-{i}")))).collect();
    }
    let mut folders = 0u64;
    for leaf in &level {
        fs::create_dir_all(leaf).unwrap();
    }
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        folders += 1;
        for n in 0..2 { fs::write(d.join(format!("file-{n}.bin")), [7u8; 64]).unwrap(); }
        for e in fs::read_dir(&d).unwrap().flatten() { if e.path().is_dir() { stack.push(e.path()); } }
    }
    let (files, bytes) = plain_listing(&root);
    assert_eq!(files, folders * 2);

    let _warm = plain_listing(&root);
    let plain = best_of(5, || { plain_listing(&root); });
    let inventory = best_of(5, || {
        let tree = sprawl_core::walk::inventory(&root, &sprawl_core::Progress::new()).unwrap();
        assert_eq!((tree.nodes[sprawl_core::ROOT].files, tree.nodes[sprawl_core::ROOT].size, tree.errors), (files, bytes, 0));
    });
    let auto = best_of(3, || { sprawl_core::scan_auto(&root, &sprawl_core::Progress::new()).unwrap(); });
    fs::remove_dir_all(&root).unwrap();
    println!("plain listing {plain:?}, inventory {inventory:?}, scan_auto {auto:?} ({folders} folders, {files} files)");
    // Inventory runs on every core, so it is normally faster than the single-threaded listing.
    // Generous limits: 3x plus 50 ms of slack. 1.0 was 15-55x.
    let limit = plain * 3 + Duration::from_millis(50);
    assert!(inventory <= limit, "folder inventory {inventory:?} vs plain listing {plain:?}: a scan slowdown");
    assert!(auto <= limit, "scan_auto {auto:?} vs plain listing {plain:?}: a scan slowdown");
}
