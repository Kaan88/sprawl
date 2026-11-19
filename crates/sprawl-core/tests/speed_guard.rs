//! Release guard against scan slowdowns like 1.0's (every folder's whole ancestor chain opened
//! before listing it: 15-55x slower). Timing-sensitive, so ignored in normal runs; the release
//! tool runs it with `--ignored` in release mode before building. Disposable data only: the
//! folder is created fresh by this run and only that folder is removed.
use std::{fs, io, path::{Path, PathBuf}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

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

/// A folder this run made itself: `create_dir` fails on an existing name, so an old folder or one
/// that another process put there is never reused or removed. Removed on drop.
struct Fresh(PathBuf);

impl Fresh {
    fn new() -> Fresh {
        let base = std::env::temp_dir();
        let pid = std::process::id();
        for attempt in 0..100u32 {
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
            let p = base.join(format!("sprawl-speed-guard-{pid}-{nanos:08x}-{attempt}"));
            match fs::create_dir(&p) {
                Ok(()) => return Fresh(p),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("can't create the speed-guard folder {}: {e}", p.display()),
            }
        }
        panic!("no fresh speed-guard folder name after 100 tries");
    }
}

impl Drop for Fresh {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Best-of timings for the plain listing and each scan, measured in turns (plain, inventory,
/// scan_auto, plain, ...) so that a burst of load on the machine lands on both sides instead of
/// only on the candidate.
fn paired(root: &Path, runs: usize, files: u64, bytes: u64, auto: bool) -> (Duration, Duration, Option<Duration>) {
    let (mut plain, mut inventory, mut scan) = (Duration::MAX, Duration::MAX, Duration::MAX);
    for _ in 0..runs {
        let start = Instant::now();
        plain_listing(root);
        plain = plain.min(start.elapsed());
        let start = Instant::now();
        let tree = sprawl_core::walk::inventory(root, &sprawl_core::Progress::new()).unwrap();
        inventory = inventory.min(start.elapsed());
        assert_eq!((tree.nodes[sprawl_core::ROOT].files, tree.nodes[sprawl_core::ROOT].size, tree.errors), (files, bytes, 0));
        if auto {
            let start = Instant::now();
            sprawl_core::scan_auto(root, &sprawl_core::Progress::new()).unwrap();
            scan = scan.min(start.elapsed());
        }
    }
    (plain, inventory, auto.then_some(scan))
}

#[test]
#[ignore = "timing; run by sprawl-release build"]
fn folder_inventory_stays_close_to_a_plain_listing() {
    // Deep and wide: 3^6 = 729 leaf folders six levels down, 1,093 folders, 2,186 files.
    let fresh = Fresh::new();
    let root = fresh.0.clone();
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
        for e in fs::read_dir(&d).unwrap() { let e = e.unwrap(); if e.file_type().unwrap().is_dir() { stack.push(e.path()); } }
    }
    let (files, bytes) = plain_listing(&root);
    assert_eq!(files, folders * 2);

    // Elevated, scan_auto reads the whole volume's MFT for this small folder: that is not a
    // folder benchmark, so it is timed only when the process is known to be unelevated.
    let auto = matches!(sprawl_core::elevation(), Ok(false));
    if !auto {
        println!("elevated or unknown: scan_auto (the MFT path) is not timed here");
    }
    let _warm = plain_listing(&root);
    // Inventory runs on every core, so it is normally faster than the single-threaded listing.
    // Generous limits: 3x plus 50 ms of slack. 1.0 was 15-55x.
    let slow = |plain: Duration, inv: Duration, scan: Option<Duration>| {
        let limit = plain * 3 + Duration::from_millis(50);
        inv > limit || scan.is_some_and(|s| s > limit)
    };
    let (plain, inventory, scan) = paired(&root, 5, files, bytes, auto);
    println!("plain listing {plain:?}, inventory {inventory:?}, scan_auto {scan:?} ({folders} folders, {files} files)");
    if slow(plain, inventory, scan) {
        // One slow window can be the machine, not the build: a failure must repeat in a second
        // paired run before it blocks a release.
        let (plain, inventory, scan) = paired(&root, 5, files, bytes, auto);
        println!("again: plain listing {plain:?}, inventory {inventory:?}, scan_auto {scan:?}");
        assert!(!slow(plain, inventory, scan), "folder inventory {inventory:?} / scan_auto {scan:?} vs plain listing {plain:?}, twice: a scan slowdown");
    }
}
