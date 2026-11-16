//! Scan a folder and save it into the scan history (as Sprawl does after a scan).
//!
//!     cargo run --release -p sprawl-advisor --example save_snapshot -- <folder>

use std::time::SystemTime;

fn main() {
    let path = std::path::PathBuf::from(std::env::args().nth(1).expect("folder"));
    let tree = sprawl_core::scan(&path, |_| {}).expect("scan");
    let base = sprawl_advisor::history::base_dir().expect("LOCALAPPDATA");
    sprawl_advisor::history::prepare(&tree, SystemTime::now(), true).save(&base, u64::MAX).expect("save");
    println!("saved {} items under {}", tree.nodes.len(), base.display());
}
