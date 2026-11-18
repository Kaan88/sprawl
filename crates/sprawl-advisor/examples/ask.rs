//! Run the AI advisor on a folder from the command line.
//!
//! `cargo run --release -p sprawl-advisor --example ask -- <folder> [provider] [model] [goal GB]`
//! provider: claude-code (default), anthropic or open-ai (base URL from SPRAWL_BASE_URL).

use std::path::Path;

use sprawl_advisor::config::{AiConfig, Provider};
use sprawl_advisor::{advise_all, facts::Facts, RulePack};
use sprawl_core::format_size;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = Path::new(args.first().map(String::as_str).unwrap_or("."));
    let provider = match args.get(1).map(String::as_str).unwrap_or("claude-code") {
        "anthropic" => Provider::Anthropic,
        "open-ai" => Provider::OpenAi,
        _ => Provider::ClaudeCode,
    };
    let cfg = AiConfig { provider, model: args.get(2).cloned().unwrap_or_default(), base_url: std::env::var("SPRAWL_BASE_URL").unwrap_or_default(), web_lookup: false };
    let goal = args.get(3).and_then(|g| g.parse::<f64>().ok()).map(|g| (g * (1u64 << 30) as f64) as u64);
    let scan_before = std::time::SystemTime::now();
    let tree = sprawl_core::scan(path, |_| {}).expect("scan");
    let mut facts = Facts::gather();
    facts.attach_usage(&tree);
    let (rules, _) = advise_all(&tree, &[RulePack::builtin()], &facts);
    println!("{} rule findings", rules.len());
    let started = std::time::Instant::now();
    // SPRAWL_MCP_EXE=<sprawl.exe>: Claude Code gets Sprawl's tools, as in the app
    if let Some(exe) = std::env::var_os("SPRAWL_MCP_EXE") {
        sprawl_advisor::llm::set_mcp_exe(exe.into());
    }
    let memory = sprawl_advisor::memory::Memory::load();
    match sprawl_advisor::llm::agent::run(&tree, scan_before, &cfg, &rules, None, goal, &memory, "", Default::default(), &mut |s| println!("  .. {s}")) {
        Ok(run) => {
            println!("{} in {:.0} s, {} turns, tools: {}, {}", run.model, started.elapsed().as_secs_f64(), run.turns, run.with_tools, run.usage.text());
            println!("summary: {}", run.summary);
            for f in &run.findings {
                println!("  {:>10}  {:?}  {}  {}", format_size(f.bytes), f.risk, f.title, f.path.display());
            }
            for (f, why) in &run.rejected {
                println!("  rejected: {} ({why})", f.path.display());
            }
        }
        Err(e) => println!("failed: {e}"),
    }
}
