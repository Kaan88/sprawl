//! Compare directory inventory with the previous per-file WOF probing on disposable data.
//! Never reads a user-supplied scope or a whole drive.
use sprawl_core::{walk, Progress, ROOT};
use std::{fs, time::Instant};

fn cpu_seconds() -> f64 {
    use windows_sys::Win32::{Foundation::FILETIME, System::Threading::{GetCurrentProcess, GetProcessTimes}};
    let mut times = [FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 }; 4];
    let [created, exited, kernel, user] = &mut times;
    assert_ne!(unsafe { GetProcessTimes(GetCurrentProcess(), created, exited, kernel, user) }, 0);
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    (ticks(times[2]) + ticks(times[3])) as f64 / 10_000_000.0
}

fn main() {
    let root = std::env::temp_dir().join(format!("sprawl-inventory-bench-{}", std::process::id()));
    fs::create_dir(&root).expect("unique scratch root");
    for d in 0..20 {
        let dir = root.join(format!("folder-{d}"));
        fs::create_dir(&dir).unwrap();
        for n in 0..100 { fs::write(dir.join(format!("file-{n}.bin")), [7u8; 128]).unwrap(); }
    }
    for fast in [false, true, false, true] {
        let start = Instant::now();
        let cpu = cpu_seconds();
        let tree = if fast { walk::inventory(&root, &Progress::new()) } else { walk::walk(&root, &Progress::new()) }.unwrap();
        assert_eq!(tree.nodes[ROOT].files, 2000);
        assert_eq!(tree.errors, 0);
        println!("{}: {:.3}s wall, {:.3}s CPU, {} files, {} bytes", if fast { "inventory" } else { "per-file WOF" }, start.elapsed().as_secs_f64(), cpu_seconds() - cpu, tree.nodes[ROOT].files, tree.nodes[ROOT].size);
    }
    fs::remove_dir_all(root).unwrap();
}
