//! Counts how much build output each gate of `projects::build_outputs` holds back. No paths.
//!
//!     cargo run --release -p sprawl-advisor --example gates -- C:\

use std::time::SystemTime;

use sprawl_advisor::{facts::Facts, projects};
use sprawl_core::{format_size, scan_auto, Progress};

fn main() {
    let path = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| r"C:\".into()));
    let tree = scan_auto(&path, &Progress::default()).expect("scan");
    let mut facts = Facts::gather();
    facts.attach_usage(&tree);
    let ps = projects::find(&tree);
    let now = SystemTime::now();
    let idle = |t: SystemTime, days: u64| now.duration_since(t).is_ok_and(|a| a.as_secs() >= days * 86_400);
    let (mut all, mut small, mut owned, mut nodate, mut recent, mut progs, mut pass) = ((0u64, 0u32), (0u64, 0u32), (0u64, 0u32), (0u64, 0u32), (0u64, 0u32), (0u64, 0u32), (0u64, 0u32));
    let mut by_age = [(0u64, 0u32); 4];
    for p in &ps {
        for &(o, _, _) in &p.outputs {
            let b = tree.nodes[o].size;
            let add = |x: &mut (u64, u32)| {
                x.0 += b;
                x.1 += 1;
            };
            add(&mut all);
            if b < 100 << 20 {
                add(&mut small);
                continue;
            }
            if facts.owner_of(&tree.path_of(p.node)).is_some() {
                add(&mut owned);
                continue;
            }
            let Some(ch) = p.last_change else {
                add(&mut nodate);
                continue;
            };
            if !idle(ch, 30) {
                add(&mut recent);
                continue;
            }
            let k = if idle(ch, 365) { 3 } else if idle(ch, 180) { 2 } else if idle(ch, 90) { 1 } else { 0 };
            add(&mut by_age[k]);
            let g = facts.output_programs(&tree, o);
            if !facts.runs_complete && g.any {
                add(&mut progs);
                continue;
            }
            add(&mut pass);
        }
    }
    let show = |l: &str, x: (u64, u32)| println!("{l:<52} {:>10}  {:>5} outputs", format_size(x.0), x.1);
    println!("{} projects; runs_complete (Prefetch readable): {}", ps.len(), facts.runs_complete);
    show("all build outputs", all);
    show("  under 100 MB (not listed)", small);
    show("  inside an installed app", owned);
    show("  project date unknown", nodate);
    show("  project changed in the last 30 days", recent);
    show("  idle 30-90 days", by_age[0]);
    show("  idle 90-180 days", by_age[1]);
    show("  idle 180-365 days", by_age[2]);
    show("  idle over a year", by_age[3]);
    show("  idle, but holds programs and no Prefetch: skipped", progs);
    show("  offered", pass);
}
