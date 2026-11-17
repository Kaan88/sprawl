//! How long each folder the Cleaners tab reads with no scan takes, one by one.
//!
//!     cargo run --release -p sprawl-advisor --example root_times
fn main() {
    let (roots, out) = sprawl_advisor::cleaners::quick_roots(&sprawl_advisor::cleaners::catalogue(&[sprawl_advisor::RulePack::builtin()]));
    println!("left out of the quick read: {out:?}");
    println!("elevated: {}", sprawl_core::is_elevated());
    let all = std::time::Instant::now();
    for r in roots {
        let t = std::time::Instant::now();
        // WALK=1: a folder walk, as a user who is not administrator gets
        let tree = if std::env::var_os("WALK").is_some() { sprawl_core::walk::walk(&r, &sprawl_core::Progress::default()) } else { sprawl_core::scan_auto(&r, &sprawl_core::Progress::default()) };
        let (items, size) = tree.as_ref().map_or((0, 0), |t| (t.nodes[sprawl_core::ROOT].items(), t.nodes[sprawl_core::ROOT].size));
        println!("{:>6.2} s {:>9} items {:>10}  {}", t.elapsed().as_secs_f64(), items, sprawl_core::format_size(size), r.display());
    }
    println!("all: {:.2} s", all.elapsed().as_secs_f64());
}
