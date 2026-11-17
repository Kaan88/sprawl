//! `sprawl --mcp`: Sprawl as an MCP server (JSON-RPC over stdio) for Claude Code.
//!
//! Three ways to run:
//! - from the user's own Claude Code (`claude mcp add sprawl -- sprawl.exe --mcp`): it talks to
//!   the running app over a named pipe, so it sees the scan on screen, the Advisor's findings,
//!   and can set the map's filter; with no app running it reads History's newest snapshot;
//! - driven by the app (`--tree <snapshot> --out <file>`): the app runs `claude -p` with only
//!   this server's tools; what the model reports is written to `--out`, and the app replays every
//!   claim through its own checks before showing anything.
//!
//! Every tool is read-only. Nothing here deletes or runs anything.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sprawl_core::Tree;

use crate::facts::Facts;
use crate::llm::tools::{chat_specs, specs, Tools};
use crate::llm::ToolSpec;
use crate::memory::Memory;

/// How the server was started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// From the user's own Claude Code with no app: looking only.
    Look,
    /// The app's analysis run: looking and reporting.
    Driven,
    /// The app's chat: looking, and the chat's tools (the web search only with `web`).
    Chat { web: bool },
}

/// The chat tools whose effect is the app's (a filter, ticks, memory): recorded for the app,
/// which replays them through its own checks.
pub const CHAT_EFFECTS: &[&str] = &["set_filter", "suggest_ticks", "remember", "propose_cleaner"];

/// Tools offered from a snapshot (no app).
fn local_specs(mode: Mode) -> Vec<ToolSpec> {
    let look = ["list_children", "largest", "by_type", "owner_of", "read_manifest"];
    let mut v: Vec<ToolSpec> = chat_specs().into_iter().filter(|t| look.contains(&t.name)).collect();
    v.insert(0, report_spec());
    match mode {
        Mode::Look => {}
        Mode::Driven => {
            let report = ["report_finding", "remember", "propose_rule", "propose_cleaner", "finish"];
            v.extend(specs().into_iter().filter(|t| report.contains(&t.name)));
        }
        Mode::Chat { web } => {
            let have: Vec<&str> = v.iter().map(|t| t.name).collect();
            let more: Vec<ToolSpec> = crate::llm::tools::chat_specs_with(web).into_iter().filter(|t| !have.contains(&t.name)).collect();
            v.extend(more);
        }
    }
    v
}

fn report_spec() -> ToolSpec {
    ToolSpec { name: "scan_report", description: "Start here: the scan's totals, biggest folders, file types and what Sprawl's rules found.", schema: json!({"type": "object", "properties": {}}) }
}

/// Tools offered through the running app: looking, plus its findings and the map filter.
pub fn bridge_specs() -> Vec<ToolSpec> {
    let mut v = local_specs(Mode::Look);
    v.push(ToolSpec {
        name: "findings",
        description: "What Sprawl's Advisor found for the scan on screen: each with its risk, size, path and evidence. Don't delete anything yourself: \
            the user ticks findings in Sprawl's Advisor and presses Clean up; Sprawl removes them itself, links as links.",
        schema: json!({"type": "object", "properties": {}}),
    });
    v.push(ToolSpec {
        name: "cleaners",
        description: "Sprawl's Cleaners for the scan on screen: each application found, its parts (caches, logs) with risk, size and paths. Read-only: \
            the user ticks parts in the Advisor and presses Clean up.",
        schema: json!({"type": "object", "properties": {}}),
    });
    v.extend(chat_specs().into_iter().filter(|t| t.name == "set_filter"));
    v
}

/// The text the app publishes holds the findings, then [`CLEANERS_MARK`] and the cleaners.
pub const CLEANERS_MARK: &str = "\n\u{1e}cleaners\n";

fn tool_list(specs: &[ToolSpec]) -> Value {
    Value::Array(specs.iter().map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.schema})).collect())
}

/// A tool call answered from a tree in this process (a snapshot).
pub struct Local<'a> {
    tree: &'a Tree,
    facts: &'a Facts,
    tools: Tools<'a>,
    mode: Mode,
    /// Driven and chat: every report, remember, proposal and the summary go here as JSON lines.
    out: Option<std::fs::File>,
    /// Where the out file is: a web lookup's answer comes back beside it.
    out_path: Option<PathBuf>,
    lookups: u64,
}

/// Where the app answers web lookup `n` of the chat whose out file is `out`.
pub fn lookup_answer(out: &Path, n: u64) -> PathBuf {
    out.with_extension(format!("lookup-{n}"))
}

impl<'a> Local<'a> {
    pub fn new(tree: &'a Tree, facts: &'a Facts, memory: &Memory, model: &str, out: Option<std::fs::File>) -> Local<'a> {
        let mode = if out.is_some() { Mode::Driven } else { Mode::Look };
        Local { tree, facts, tools: Tools::new(tree, facts, model, &[], memory), mode, out, out_path: None, lookups: 0 }
    }

    pub fn call(&mut self, name: &str, input: &Value) -> Result<String, String> {
        if name == "scan_report" {
            return Ok(crate::llm::report::build(self.tree, self.facts, &[], None, None));
        }
        let allowed = local_specs(self.mode);
        if !allowed.iter().any(|t| t.name == name) {
            return Err(format!("there is no tool called {name}"));
        }
        let chat = matches!(self.mode, Mode::Chat { .. });
        // every call is noted, so the app can show what Claude Code is doing; the reports are
        // replayed there through its own checks, never taken from here. A web query is shown
        // by the app, as it will be sent, not as the model wrote it.
        if let Some(f) = self.out.as_mut() {
            if name != "web_lookup" {
                let what = crate::facts::clean_text(input["path"].as_str().or(input["title"].as_str()).or(input["filter"].as_str()).unwrap_or(""));
                let _ = writeln!(f, "{}", json!({"progress": format!("{} {what}", name.replace('_', " "))}));
            }
            let recorded = if chat { CHAT_EFFECTS.contains(&name) } else { matches!(name, "report_finding" | "remember" | "propose_rule" | "propose_cleaner" | "finish") };
            if recorded {
                let _ = writeln!(f, "{}", json!({"tool": name, "input": input}));
            }
            let _ = f.flush();
        }
        if name == "web_lookup" {
            return self.ask_app(input);
        }
        self.tools.call(name, input)
    }

    /// A web lookup in the chat: the app takes the query out of paths and names, asks the user,
    /// searches and writes the answer beside the out file. Nothing goes out from here.
    fn ask_app(&mut self, input: &Value) -> Result<String, String> {
        let (Some(f), Some(out)) = (self.out.as_mut(), self.out_path.as_ref()) else { return Err("there is no tool called web_lookup".into()) };
        let n = self.lookups;
        self.lookups += 1;
        let _ = writeln!(f, "{}", json!({"lookup": n, "input": input}));
        let _ = f.flush();
        let answer = lookup_answer(out, n);
        let start = std::time::Instant::now();
        // the user may take a while to decide; the app answers no after ten minutes
        while start.elapsed() < std::time::Duration::from_secs(660) {
            if let Some(v) = std::fs::read_to_string(&answer).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
                let _ = std::fs::remove_file(&answer);
                let text = v["text"].as_str().unwrap_or("").to_string();
                return if v["ok"].as_bool() == Some(true) { Ok(text) } else { Err(text) };
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        Err("No answer from Sprawl about this search.".into())
    }
}

/// Answer one JSON-RPC message; `None` for a notification.
pub fn handle(msg: &Value, specs: &[ToolSpec], call: &mut dyn FnMut(&str, &Value) -> Result<String, String>) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let result = match msg["method"].as_str().unwrap_or("") {
        "initialize" => {
            let version = msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18");
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "sprawl", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Sprawl's view of the user's disk: start with scan_report, then look closer. Read-only: nothing here deletes or runs anything."
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tool_list(specs)}),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("");
            let args = msg["params"].get("arguments").cloned().unwrap_or_else(|| json!({}));
            let (text, err) = match call(name, &args) {
                Ok(t) => (t, false),
                Err(e) => (e, true),
            };
            json!({"content": [{"type": "text", "text": crate::llm::clip(text, 40_000)}], "isError": err})
        }
        m => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("no method {m}")}})),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// Run the server on stdin and stdout until stdin closes.
fn serve(specs: Vec<ToolSpec>, call: &mut dyn FnMut(&str, &Value) -> Result<String, String>) {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg, &specs, call),
            Err(_) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}})),
        };
        if let Some(r) = reply {
            if writeln!(stdout, "{r}").and_then(|_| stdout.flush()).is_err() {
                break;
            }
        }
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// The newest full snapshot History has (any folder), as a tree.
fn newest_snapshot() -> Option<Tree> {
    let base = crate::history::base_dir()?;
    let list = crate::history::list(&base);
    let e = list.iter().rev().find(|e| e.full).or(list.last())?;
    crate::history::load(e).ok()
}

/// Entry point for `sprawl --mcp [--tree <file> --out <file> --model <name>]`.
pub fn main(args: &[String]) {
    let dbg = std::env::var_os("SPRAWL_MCP_DEBUG").is_some();
    let step = |s: &str| if dbg { eprintln!("mcp: {s}") };
    step("start");
    let tree_file = arg(args, "--tree").map(PathBuf::from);
    if tree_file.is_none() {
        step("connecting to the app");
        if let Some(mut client) = crate::pipe::Client::connect() {
            // through the running app
            let specs = bridge_specs();
            serve(specs, &mut |name, input| {
                let req = json!({"tool": name, "input": input}).to_string();
                let a: Value = client.ask(&req).ok().and_then(|s| serde_json::from_str(&s).ok()).ok_or("Sprawl closed the connection")?;
                let text = a["text"].as_str().unwrap_or("").to_string();
                if a["ok"].as_bool() == Some(true) { Ok(text) } else { Err(text) }
            });
            return;
        }
    }
    let tree = match &tree_file {
        Some(f) => std::fs::File::open(f).ok().and_then(|f| crate::snapshot::read(&mut std::io::BufReader::new(f)).ok()).map(|(t, _)| t),
        None => newest_snapshot(),
    };
    step("snapshot loaded");
    let Some(tree) = tree else {
        serve(Vec::new(), &mut |_, _| Err("Sprawl isn't running and has no saved scan yet: open Sprawl and scan a drive.".into()));
        return;
    };
    let mut facts = Facts::gather();
    step("facts gathered");
    facts.attach_usage(&tree);
    step("usage attached");
    let out_path = arg(args, "--out").map(PathBuf::from);
    let out = out_path.as_ref().and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
    let model = arg(args, "--model").unwrap_or_else(|| "claude-code".into());
    // read only: this process never renames or rewrites the user's memory
    let memory = Memory::load_readonly();
    let mut local = Local::new(&tree, &facts, &memory, &model, out);
    if local.out.is_some() && args.iter().any(|a| a == "--chat") {
        local.mode = Mode::Chat { web: args.iter().any(|a| a == "--web") };
        local.out_path = out_path;
    }
    step("serving");
    serve(local_specs(local.mode), &mut |name, input| local.call(name, input));
}

/// The app's side of the pipe: answer one request line against `tree`. `filter` receives a
/// filter the model wants shown; `findings` is the Advisor's list as text.
pub fn answer_bridge(line: &str, tree: Option<&Tree>, facts: &Facts, memory: &Memory, findings: &str, filter: &mut dyn FnMut(String)) -> String {
    let reply = |ok: bool, text: String| json!({"ok": ok, "text": text}).to_string();
    let Ok(req) = serde_json::from_str::<Value>(line) else { return reply(false, "bad request".into()) };
    let Some(tree) = tree else { return reply(false, "No scan is open in Sprawl yet.".into()) };
    let name = req["tool"].as_str().unwrap_or("");
    let input = &req["input"];
    match name {
        "findings" => {
            let f = findings.split(CLEANERS_MARK).next().unwrap_or("");
            reply(true, if f.trim().is_empty() { "The Advisor hasn't found anything (or is still looking).".into() } else { f.to_string() })
        }
        "cleaners" => {
            let c = findings.split_once(CLEANERS_MARK).map_or("", |(_, c)| c);
            reply(true, if c.trim().is_empty() { "The Advisor hasn't listed cleaners yet (or is still looking).".into() } else { c.to_string() })
        }
        "scan_report" => reply(true, crate::llm::report::build(tree, facts, &[], None, None)),
        _ if bridge_specs().iter().any(|t| t.name == name) => {
            let mut tools = Tools::new(tree, facts, "claude-code", &[], memory);
            match tools.call(name, input) {
                Ok(t) => {
                    if let Some(f) = tools.filter.take() {
                        filter(f);
                    }
                    reply(true, t)
                }
                Err(e) => reply(false, e),
            }
        }
        _ => reply(false, format!("there is no tool called {name}")),
    }
}

/// The MCP config for a driven `claude -p` run: only this server, with the tree and an out file.
pub fn driven_config(exe: &Path, tree_file: &Path, out_file: &Path, model: &str) -> String {
    json!({"mcpServers": {"sprawl": {"command": exe, "args": ["--mcp", "--tree", tree_file, "--out", out_file, "--model", model]}}}).to_string()
}

/// The MCP config for the chat with Claude Code: the chat's tools, the web search only with `web`.
pub fn chat_config(exe: &Path, tree_file: &Path, out_file: &Path, model: &str, web: bool) -> String {
    let mut args: Vec<Value> = vec!["--mcp".into(), "--tree".into(), json!(tree_file), "--out".into(), json!(out_file), "--model".into(), model.into(), "--chat".into()];
    if web {
        args.push("--web".into());
    }
    json!({"mcpServers": {"sprawl": {"command": exe, "args": args}}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol() {
        let specs = local_specs(Mode::Look);
        let mut call = |name: &str, _: &Value| if name == "scan_report" { Ok("report".to_string()) } else { Err("nope".to_string()) };
        let init = handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}), &specs, &mut call).unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "sprawl");
        assert!(handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}), &specs, &mut call).is_none());
        let list = handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}), &specs, &mut call).unwrap();
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"scan_report") && names.contains(&"largest"));
        assert!(!names.contains(&"report_finding") && !names.contains(&"set_filter") && !names.contains(&"web_lookup"), "{names:?}");
        let r = handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "scan_report"}}), &specs, &mut call).unwrap();
        assert_eq!(r["result"]["content"][0]["text"], "report");
        let e = handle(&json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "x"}}), &specs, &mut call).unwrap();
        assert_eq!(e["result"]["isError"], true);
        assert!(handle(&json!({"jsonrpc": "2.0", "id": 5, "method": "nope"}), &specs, &mut call).unwrap()["error"]["code"] == -32601);
        // driven mode adds the reporting tools, never anything that acts
        let d: Vec<&str> = local_specs(Mode::Driven).iter().map(|t| t.name).collect();
        assert!(d.contains(&"report_finding") && d.contains(&"finish") && !d.contains(&"web_lookup") && !d.contains(&"set_filter"));
        // the chat: its tools, the web search only when switched on, never a finding
        let c: Vec<&str> = local_specs(Mode::Chat { web: false }).iter().map(|t| t.name).collect();
        assert!(c.contains(&"suggest_ticks") && c.contains(&"set_filter") && c.contains(&"largest") && !c.contains(&"web_lookup") && !c.contains(&"report_finding"), "{c:?}");
        let w: Vec<&str> = local_specs(Mode::Chat { web: true }).iter().map(|t| t.name).collect();
        assert!(w.contains(&"web_lookup"));
        assert_eq!(w.len(), c.len() + 1);
        let cfg = chat_config(Path::new("s.exe"), Path::new("t"), Path::new("o"), "m", false);
        assert!(cfg.contains("--chat") && !cfg.contains("--web"));
        assert!(chat_config(Path::new("s.exe"), Path::new("t"), Path::new("o"), "m", true).contains("--web"));
    }

    #[test]
    fn bridge_answers() {
        let tree = sprawl_core::scan(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").as_path(), |_| {}).unwrap();
        let facts = Facts::from_apps(Vec::new());
        let mut shown = None;
        let root = tree.path_of(sprawl_core::ROOT).display().to_string();
        let a = answer_bridge(&json!({"tool": "list_children", "input": {"path": root}}).to_string(), Some(&tree), &facts, &Memory::default(), "", &mut |f| shown = Some(f));
        assert!(a.contains("\"ok\":true") && a.contains("lib.rs"), "{a}");
        let f = answer_bridge(&json!({"tool": "set_filter", "input": {"filter": "*.rs"}}).to_string(), Some(&tree), &facts, &Memory::default(), "", &mut |f| shown = Some(f));
        assert!(f.contains("\"ok\":true") && shown.as_deref() == Some("*.rs"));
        let n = answer_bridge(&json!({"tool": "report_finding", "input": {}}).to_string(), Some(&tree), &facts, &Memory::default(), "", &mut |_| {});
        assert!(n.contains("no tool"), "reporting isn't offered over the bridge");
    }
}
