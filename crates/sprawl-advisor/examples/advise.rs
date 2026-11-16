//! Run the built-in rule pack over a scan and print what it found (titles and sizes; no paths
//! unless `--paths` is given).
//!
//!     cargo run --release -p sprawl-advisor --example advise -- C:\ [--paths]

use std::path::PathBuf;

use sprawl_advisor::{advise, RulePack};
use sprawl_core::{format_size, scan_auto, Progress};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let show = args.iter().any(|a| a == "--paths");
    let path = PathBuf::from(args.iter().find(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| r"C:\".into()));
    let tree = scan_auto(&path, &Progress::default()).expect("scan");
    let (ok, held) = advise(&tree, &[RulePack::builtin()]);
    let total: u64 = ok.iter().map(|f| f.bytes).sum();
    println!("{} findings, {} in total", ok.len(), format_size(total));
    for f in &ok {
        let rule = match &f.source {
            sprawl_advisor::Source::Rule(id) => id.as_str(),
            sprawl_advisor::Source::Model(m) => m.as_str(),
        };
        println!("  {:>10}  {:<7} {:<32} {rule}", format_size(f.bytes), format!("{:?}", f.risk), f.title);
        if show {
            println!("              {}", f.path.display());
        }
    }
    for (f, why) in &held {
        println!("  held back: {} ({}): {why}", f.title, format_size(f.bytes));
    }
}
