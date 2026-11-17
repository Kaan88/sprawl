//! Ask one question about a folder from the command line, printing the answer as it streams.
//!
//! `cargo run --release -p sprawl-advisor --example chat -- <folder> <question> [provider] [model]`

use std::path::Path;
use std::time::Instant;

use sprawl_advisor::config::{AiConfig, Provider};
use sprawl_advisor::llm::agent::{chat, ChatEvent};
use sprawl_advisor::memory::Memory;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tree = sprawl_core::scan(Path::new(&args[0]), |_| {}).expect("scan");
    let provider = match args.get(2).map(String::as_str).unwrap_or("claude-code") {
        "anthropic" => Provider::Anthropic,
        "open-ai" => Provider::OpenAi,
        _ => Provider::ClaudeCode,
    };
    let cfg = AiConfig { provider, model: args.get(3).cloned().unwrap_or_default(), base_url: std::env::var("SPRAWL_BASE_URL").unwrap_or_default() };
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(args[1].clone()).unwrap();
    drop(tx);
    let start = Instant::now();
    let pieces = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let p2 = pieces.clone();
    let partial: std::rc::Rc<dyn Fn(&str)> = std::rc::Rc::new(move |t: &str| {
        p2.set(p2.get() + 1);
        if p2.get() == 1 {
            println!("  first text after {:.1} s", start.elapsed().as_secs_f64());
        }
        let _ = t;
    });
    chat(&tree, &cfg, &Memory::default(), "", rx, &mut |e| match e {
        ChatEvent::Reply(r) => println!("reply after {:.1} s, {} streamed pieces:\n{r}", start.elapsed().as_secs_f64(), pieces.get()),
        ChatEvent::Failed(e) => println!("failed: {e}"),
        ChatEvent::Progress(p) => println!("  .. {p}"),
        _ => {}
    }, &|_| false, Default::default(), partial);
}
