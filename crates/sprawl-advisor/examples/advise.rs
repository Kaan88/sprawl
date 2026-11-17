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
    // --cleaners: only the folders the cleaners look at, as the Cleaners tab reads them with no scan
    let cleaners_only = args.iter().any(|a| a == "--cleaners");
    let ts = std::time::Instant::now();
    let tree = if cleaners_only {
        let roots = sprawl_advisor::cleaners::quick_roots(&sprawl_advisor::cleaners::catalogue(&[RulePack::builtin()])).0;
        sprawl_core::scan_many(&roots, &Progress::default()).expect("scan")
    } else {
        scan_auto(&path, &Progress::default()).expect("scan")
    };
    println!("scanned {} in {:.2} s", format_size(tree.nodes[sprawl_core::ROOT].size), ts.elapsed().as_secs_f64());
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
    // what the Cleaners tab reads when no scan covers the cleaners
    let roots = sprawl_advisor::cleaners::scan_roots(&sprawl_advisor::cleaners::catalogue(&[RulePack::builtin()]));
    println!("cleaner scan roots: {}", roots.len());
    for r in &roots {
        println!("  {}", r.display());
    }
    let t1 = std::time::Instant::now();
    let (ok, held) = advise_all(&tree, &[RulePack::builtin()], &facts);
    let t_adv = t1.elapsed().as_secs_f64();
    let t2 = std::time::Instant::now();
    let owners = facts.owner_map(&tree);
    println!("advise_all {t_adv:.2} s, owner_map {:.2} s ({} nodes owned)", t2.elapsed().as_secs_f64(), owners.iter().filter(|o| o.is_some()).count());
    use sprawl_advisor::finding::Group;
    let total: u64 = ok.iter().filter(|f| !f.in_active_project() && f.group() == Group::Clean).map(|f| f.bytes).sum();
    let left: Vec<_> = ok.iter().filter(|f| f.group() == Group::Leftover).collect();
    println!("left behind by applications no longer installed: {}, {}", left.len(), format_size(left.iter().map(|f| f.bytes).sum()));
    for f in &left {
        let how = if f.remedy == sprawl_advisor::Remedy::Info { "shown only" } else { "deletable" };
        println!("  {:>10}  {how:<10}  {}  ({})", format_size(f.bytes), f.path.display(), f.evidence[3]);
    }
    // how sure: counts by word, and the green ones shown amber for it
    let mut by: std::collections::BTreeMap<&str, usize> = Default::default();
    for f in &ok {
        *by.entry(sprawl_advisor::confidence::words(f.confidence)).or_default() += 1;
    }
    println!("how sure: {by:?}");
    for f in ok.iter().filter(|f| f.evidence.iter().any(|e| e.contains("isn't sure enough"))) {
        println!("  shown amber, not sure enough: {} ({}) {}", f.title, format_size(f.bytes), f.evidence.iter().find(|e| e.starts_with("How sure")).unwrap());
    }
    // growth from History's snapshots
    if let Some(g) = sprawl_advisor::history::base_dir().and_then(|b| sprawl_advisor::growth::growth(&tree, &b, &facts, None, std::time::SystemTime::now())) {
        let gf = sprawl_advisor::growth::findings(&g, &tree, std::time::SystemTime::now());
        println!("growth since {:?}: {} folders grew by 1 GB or more", g.since.map(sprawl_advisor::facts::month_year), gf.len());
        for f in &gf {
            let at = ok.iter().any(|o| o.path == f.path);
            println!("  {}{}  {}", f.title, if at { "  (a line in the finding there)" } else { "" }, f.evidence[1..].join("; "));
        }
    } else {
        println!("growth: no earlier snapshot of this folder");
    }
    for (f, why) in held.iter().filter(|(f, _)| f.bytes >= 1 << 30) {
        println!("held back, as a row: {} ({}): {why}", f.title, format_size(f.bytes));
    }
    let tips: u64 = ok.iter().filter(|f| f.group() == Group::Yourself).map(|f| f.expected_gain()).sum();
    let active: Vec<_> = ok.iter().filter(|f| f.in_active_project()).collect();
    println!("{} findings: {} Sprawl can clean, {} of tips by their gain", ok.len() - active.len(), format_size(total), format_size(tips));
    println!("build outputs of projects being worked on: {}, {}", active.len(), format_size(active.iter().map(|f| f.bytes).sum()));
    let carved: Vec<_> = ok.iter().filter(|f| !f.carved.is_empty()).collect();
    for f in &carved {
        println!("  carved: {} ({}), {} kept", f.title, format_size(f.bytes), f.carved.len());
    }
    for f in ok.iter().filter(|f| !f.in_active_project()) {
        let rule = match &f.source {
            sprawl_advisor::Source::Rule(id) => id.as_str(),
            sprawl_advisor::Source::Model(m) => m.as_str(),
        };
        println!("  {:>10}  {:<7} {:<8} {:<32} {rule:<24} {}", format_size(f.expected_gain()), format!("{:?}", f.risk), format!("{:?}", f.group()), f.title, f.owner.as_deref().unwrap_or(""));
        if show {
            println!("              {}", f.path.display());
        }
    }
    if let Some(h) = sprawl_advisor::hidden::account(&tree, sprawl_advisor::hidden::shadow_storage(&path)) {
        println!("drive used {}, scanned {}, gap {}", format_size(h.used), format_size(h.scanned), h.gap / (1 << 20));
        for p in &h.parts {
            println!("  {:<34} {:>10}  {}", p.label, p.bytes.map(format_size).unwrap_or_default(), p.note);
        }
    }
    for (f, why) in &held {
        println!("  held back: {} ({}): {why}", f.title, format_size(f.bytes));
    }
    // SPRAWL_GROWTH=1: growth from the saved snapshots (read only)
    if std::env::var_os("SPRAWL_GROWTH").is_some() {
        let free = sprawl_advisor::hidden::free_space(&path).map(|(_, f)| f);
        let t0 = std::time::Instant::now();
        match sprawl_advisor::history::base_dir().and_then(|b| sprawl_advisor::growth::growth(&tree, &b, &facts, free, std::time::SystemTime::now())) {
            Some(g) => println!("growth in {:.2} s, {} points:{}", t0.elapsed().as_secs_f64(), g.points.len(), g.text(&tree)),
            None => println!("no earlier snapshot of this folder"),
        }
    }
    // SPRAWL_EXPORT=<file.ps1> writes the clean-up script for every finding with a command
    if let Some(out) = std::env::var_os("SPRAWL_EXPORT") {
        let list: Vec<&sprawl_advisor::Finding> = ok.iter().filter(|f| f.command.is_some()).collect();
        let text = sprawl_advisor::export::script(&list, &path.display().to_string(), std::time::SystemTime::now());
        sprawl_advisor::export::write_script(std::path::Path::new(&out), &text).unwrap();
        println!("script for {} findings written", list.len());
    }
}
