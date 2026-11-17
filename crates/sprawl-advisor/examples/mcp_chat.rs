//! Drive `sprawl --mcp --chat` by hand, as Claude Code would in the chat, with no model:
//! list its tools, call a look tool and suggest_ticks, and a web lookup that this program
//! answers the way the app does (it never searches).
//!
//!     cargo run --release -p sprawl-advisor --example mcp_chat -- <sprawl.exe> <folder> [--web]
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let exe = std::path::PathBuf::from(&args[1]);
    let folder = std::path::PathBuf::from(&args[2]);
    let web = args.iter().any(|a| a == "--web");
    let dir = std::env::temp_dir().join(format!("sprawl-mcp-chat-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tree = sprawl_core::scan(&folder, |_| {}).expect("scan");
    let tree_file = dir.join("scan.dtsn");
    let mut w = std::io::BufWriter::new(std::fs::File::create(&tree_file).unwrap());
    sprawl_advisor::snapshot::write(&tree, std::time::SystemTime::now(), 1, &mut w).unwrap();
    drop(w);
    let out = dir.join("out.jsonl");
    std::fs::write(&out, "").unwrap();
    let mut a = vec!["--mcp".to_string(), "--tree".into(), tree_file.display().to_string(), "--out".into(), out.display().to_string(), "--model".into(), "m".into(), "--chat".into()];
    if web {
        a.push("--web".into());
    }
    let mut child = Command::new(&exe).args(&a).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("start");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |msg: serde_json::Value, answer_lookup: bool| -> serde_json::Value {
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        if answer_lookup {
            // as the app: wait for the lookup line, answer beside the out file
            for _ in 0..100 {
                let t = std::fs::read_to_string(&out).unwrap_or_default();
                if let Some(l) = t.lines().find(|l| l.contains("\"lookup\"")) {
                    println!("  server asked the app: {l}");
                    let v: serde_json::Value = serde_json::from_str(l).unwrap();
                    let n = v["lookup"].as_u64().unwrap();
                    let q = sprawl_advisor::llm::tools::shown_query(&v["input"]);
                    println!("  the app would show and send: {q:?}");
                    let ans = sprawl_advisor::mcp::lookup_answer(&out, n);
                    std::fs::write(&ans, serde_json::json!({"ok": true, "text": sprawl_advisor::llm::tools::mark_untrusted("- (no search was made: a hand check)")}).to_string()).unwrap();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let init = ask(serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}), false);
    println!("initialize: {}", init["result"]["serverInfo"]);
    let list = ask(serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}), false);
    let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    println!("tools: {names:?}");
    let root = tree.path_of(sprawl_core::ROOT).display().to_string();
    let r = ask(serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "largest", "arguments": {"path": root, "what": "files"}}}), false);
    println!("largest: {}", r["result"]["content"][0]["text"].as_str().unwrap_or("").lines().take(3).collect::<Vec<_>>().join(" | "));
    let r = ask(serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "suggest_ticks", "arguments": {"parts": ["dev.vscode/cache"], "why": "a cache"}}}), false);
    println!("suggest_ticks: {}", r["result"]["content"][0]["text"]);
    let r = ask(serde_json::json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "report_finding", "arguments": {}}}), false);
    println!("report_finding (not a chat tool): isError={} {}", r["result"]["isError"], r["result"]["content"][0]["text"]);
    let r = ask(serde_json::json!({"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {"name": "web_lookup", "arguments": {"query": r"what is C:\Users\someone\AppData\Local\DXCache"}}}), web);
    println!("web_lookup: isError={} {}", r["result"]["isError"], r["result"]["content"][0]["text"].as_str().unwrap_or("").lines().next().unwrap_or(""));
    drop(stdin);
    let _ = child.wait();
    println!("out file:\n{}", std::fs::read_to_string(&out).unwrap_or_default());
    let _ = std::fs::remove_dir_all(&dir);
}
