//! Run the built-in rule pack over a scan and print what it found (titles and sizes; no paths
//! unless `--paths` is given).
//!
//!     cargo run --release -p sprawl-advisor --example advise -- C:\ [--paths]

use std::path::PathBuf;

use sprawl_advisor::{advise_all, facts::Facts, RulePack};
use sprawl_core::{format_size, scan_auto, Progress};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let show = args.iter().any(|a| a == "--paths");
    let path = PathBuf::from(args.iter().find(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| r"C:\".into()));
    let tree = scan_auto(&path, &Progress::default()).expect("scan");
    let t0 = std::time::Instant::now();
    let mut facts = Facts::gather();
    facts.attach_usage(&tree);
    let dated = facts.apps.iter().filter(|a| a.last_used.is_some()).count();
    println!("{} apps known ({} with a last-used date) in {:.2} s", facts.apps.len(), dated, t0.elapsed().as_secs_f64());
    let ps = sprawl_advisor::projects::find(&tree);
    let out_bytes: u64 = ps.iter().map(|p| p.output_bytes(&tree)).sum();
    println!("{} projects, {} of build outputs (all ages)", ps.len(), format_size(out_bytes));
    if std::env::var_os("SPRAWL_PROJECTS").is_some() {
        let mut outs: Vec<(u64, String)> = ps.iter().flat_map(|p| p.outputs.iter().map(|&(n, _, _)| (tree.nodes[n].size, tree.path_of(n).display().to_string()))).collect();
        outs.sort_by(|a, b| b.0.cmp(&a.0));
        for (b, p) in outs.iter().take(25) {
            println!("    {:>10}  {p}", format_size(*b));
        }
        let mut tops: std::collections::HashMap<String, usize> = Default::default();
        for p in &ps {
            let path = tree.path_of(p.node).display().to_string();
            let key: String = path.split('\\').take(3).collect::<Vec<_>>().join("\\");
            *tops.entry(key).or_default() += 1;
        }
        let mut t: Vec<_> = tops.into_iter().collect();
        t.sort_by(|a, b| b.1.cmp(&a.1));
        for (k, c) in t.iter().take(15) {
            println!("    {c:>6}  {k}");
        }
    }
    let (ok, held) = advise_all(&tree, &[RulePack::builtin()], &facts);
    let total: u64 = ok.iter().map(|f| f.bytes).sum();
    println!("{} findings, {} in total", ok.len(), format_size(total));
    for f in &ok {
        let rule = match &f.source {
            sprawl_advisor::Source::Rule(id) => id.as_str(),
            sprawl_advisor::Source::Model(m) => m.as_str(),
        };
        println!("  {:>10}  {:<7} {:<32} {rule:<24} {}", format_size(f.bytes), format!("{:?}", f.risk), f.title, f.owner.as_deref().unwrap_or(""));
        if show {
            println!("              {}", f.path.display());
        }
    }
    for (f, why) in &held {
        println!("  held back: {} ({}): {why}", f.title, format_size(f.bytes));
    }
}
