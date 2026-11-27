//! `sprawl --mcp`: Sprawl as an MCP server (JSON-RPC over stdio) for Claude Code.
//!
//! Three ways to run:
//! - from the user's own Claude Code (`claude mcp add sprawl -- sprawl.exe --mcp`): it can scan
//!   folders and drives itself (`mcp_scan`), and it talks to
//!   the running app over a named pipe, so it sees the scan on screen, the Advisor's findings,
//!   and can set the map's filter; with no app running it reads History's newest snapshot;
//! - driven by the app (`--tree <snapshot> --out <file>`): the app runs `claude -p` with only
//!   this server's tools; what the model reports is written to `--out`, and the app replays every
//!   claim through its own checks before showing anything.
//!
//! Every tool is read-only. Nothing here deletes or runs anything.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sprawl_core::Tree;

use crate::facts::Facts;
use crate::finding::Finding;
use crate::llm::tools::{chat_specs, specs, Tools};
use crate::llm::ToolSpec;
use crate::memory::Memory;
use crate::mcp_cancel::Cancel;
use crate::mcp_scan::{Index, ScanReply};

#[path = "mcp_stdio.rs"]
mod stdio;

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
pub const CHAT_EFFECTS: &[&str] = &["set_filter", "suggest_ticks", "remember", "propose_rule", "propose_cleaner"];

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
    ToolSpec { name: "scan_report", description: "Start here: the scan's totals, biggest folders, file types and what Sprawl's rules found.", schema: which_scan() }
}

/// An optional `path` naming which scan a whole-scan tool is about.
fn which_scan() -> Value {
    json!({"type": "object", "properties": {"path": {"type": "string", "description": "Which scan: a path inside it (default: the scan open in Sprawl, else the newest)."}}})
}

/// The tools Claude Code gets from `sprawl --mcp` with no app arguments: the bridge's, and
/// the scans this server makes.
pub fn user_specs() -> Vec<ToolSpec> {
    let mut v = bridge_specs();
    v.extend(crate::mcp_scan::scan_specs());
    v
}

/// Tools offered through the running app: looking, plus its findings and the map filter.
pub fn bridge_specs() -> Vec<ToolSpec> {
    let mut v = local_specs(Mode::Look);
    v.push(ToolSpec {
        name: "findings",
        description: "What Sprawl's Advisor found for the scan on screen: each with its risk, size, path and evidence. Don't delete anything yourself: \
            the user ticks findings in Sprawl's Advisor and presses Clean up; Sprawl removes them itself, links as links.",
        schema: which_scan(),
    });
    v.push(ToolSpec {
        name: "cleaners",
        description: "Sprawl's Cleaners for the scan on screen: each application found, its parts (caches, logs) with risk, size and paths. Read-only: \
            the user ticks parts in the Advisor and presses Clean up.",
        schema: which_scan(),
    });
    v.extend(chat_specs().into_iter().filter(|t| t.name == "set_filter"));
    v.extend(crate::llm::more_tools::specs());
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
    /// The rules' findings for this tree, run in this process on first need.
    found: Option<Vec<Finding>>,
}

/// Sprawl's rules (built-in and the user's checked local packs) over `tree`, through the
/// safety checks: what the app's Advisor would list, for a tree the app isn't showing.
pub fn rules_for(tree: &Tree, facts: &Facts) -> Vec<Finding> {
    let mut packs = vec![crate::rules::RulePack::builtin()];
    packs.extend(crate::memory::local_packs());
    crate::advise_all(tree, &packs, facts).0
}

/// The rules' findings as the `findings` tool lists them: one row each with its risk, the
/// rule (or model) that found it, what Sprawl would do, the confidence, the exact bytes and the
/// path, then its evidence. The app publishes its Advisor's list in the same form.
pub fn findings_list(found: &[Finding]) -> String {
    use std::fmt::Write;
    if found.is_empty() {
        return "Sprawl's rules ran over this scan and none matched.".into();
    }
    let mut s = String::from("Each: [risk | found by | what Sprawl would do | confidence] title: size (bytes): path\n");
    for f in found {
        let by = match &f.source {
            crate::finding::Source::Rule(id) => format!("rule {id}"),
            crate::finding::Source::Model(m) => format!("model {m}"),
        };
        let does = match f.remedy {
            crate::finding::Remedy::Delete if f.command.is_some() => "deletes it (Clean up)",
            crate::finding::Remedy::Delete => "delete",
            crate::finding::Remedy::Command if f.command.is_some() => "runs the owner's clean-up (Clean up)",
            crate::finding::Remedy::Command => "the owner's clean-up",
            crate::finding::Remedy::Uninstall => "uninstall its app",
            crate::finding::Remedy::Move => "move it to another drive",
            crate::finding::Remedy::Info => "nothing: information",
        };
        let _ = writeln!(s, "- [{:?} | {by} | {does} | {:.2}] {}: {} ({} bytes): {}", f.risk, f.confidence, f.title, sprawl_core::format_size(f.bytes), f.bytes, f.path.display());
        for e in &f.evidence {
            let _ = writeln!(s, "    {e}");
        }
    }
    s + "Nothing here deletes: the user ticks findings in Sprawl's Advisor and presses Clean up; Sprawl removes them itself.\n"
}

/// Where the app answers web lookup `n` of the chat whose out file is `out`.
pub fn lookup_answer(out: &Path, n: u64) -> PathBuf {
    out.with_extension(format!("lookup-{n}"))
}

impl<'a> Local<'a> {
    pub fn new(tree: &'a Tree, facts: &'a Facts, memory: &Memory, model: &str, out: Option<std::fs::File>) -> Local<'a> {
        let mode = if out.is_some() { Mode::Driven } else { Mode::Look };
        Local { tree, facts, tools: Tools::new(tree, facts, model, &[], memory), mode, out, out_path: None, lookups: 0, found: None }
    }

    pub fn call(&mut self, name: &str, input: &Value) -> Result<String, String> {
        self.call_cancellable(name, input, &Cancel::default())
    }

    fn call_cancellable(&mut self, name: &str, input: &Value, cancel: &Cancel) -> Result<String, String> {
        cancel.check()?;
        if !local_specs(self.mode).iter().any(|t| t.name == name) {
            return Err(format!("there is no tool called {name}"));
        }
        if name == "scan_report" {
            let found = self.found.get_or_insert_with(|| rules_for(self.tree, self.facts));
            return Ok(crate::llm::report::build(self.tree, self.facts, Some(found), None, None, self.tools.memory_labels()));
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
            return self.ask_app(input, cancel);
        }
        self.tools.call_cancellable(name, input, cancel.flag())
    }

    /// A web lookup in the chat: the app takes the query out of paths and names, asks the user,
    /// searches and writes the answer beside the out file. Nothing goes out from here.
    fn ask_app(&mut self, input: &Value, cancel: &Cancel) -> Result<String, String> {
        let (Some(f), Some(out)) = (self.out.as_mut(), self.out_path.as_ref()) else { return Err("there is no tool called web_lookup".into()) };
        let n = self.lookups;
        self.lookups += 1;
        let _ = writeln!(f, "{}", json!({"lookup": n, "input": input}));
        let _ = f.flush();
        let answer = lookup_answer(out, n);
        let start = std::time::Instant::now();
        // the user may take a while to decide; the app answers no after ten minutes
        while start.elapsed() < std::time::Duration::from_secs(660) {
            cancel.check()?;
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

const LATEST_PROTOCOL: &str = "2025-06-18";
const SUPPORTED_PROTOCOLS: &[&str] = &["2024-11-05", "2025-03-26", LATEST_PROTOCOL];
const TOOL_LIMIT: Duration = Duration::from_secs(25);
const PIPE_LIMIT: Duration = Duration::from_secs(5);
const GUI_MAX_WAIT: Duration = Duration::from_millis(TOOL_LIMIT.as_millis() as u64 + 250);
const WEB_CONSENT_LIMIT: Duration = Duration::from_secs(660);
const SCAN_POLL: Duration = Duration::from_millis(150);

fn guarded(call: impl FnOnce() -> Result<String, String>) -> Result<String, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(call))
        .unwrap_or_else(|_| Err("Sprawl hit an internal error answering that tool.".into()))
}

fn tool_reply(id: Value, answer: Result<String, String>) -> Value {
    let (mut text, err) = match answer { Ok(t) => (t, false), Err(e) => (e, true) };
    if let Some((cut, _)) = text.char_indices().nth(40_000) {
        text.truncate(cut);
        text.push_str("\n[cut short]");
    }
    json!({"jsonrpc": "2.0", "id": id, "result": {
        "content": [{"type": "text", "text": text}], "isError": err
    }})
}

/// Answer one JSON-RPC message; `None` for a notification.
pub fn handle(msg: &Value, specs: &[ToolSpec], call: &mut dyn FnMut(&str, &Value) -> Result<String, String>) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let result = match msg["method"].as_str().unwrap_or("") {
        "initialize" => {
            let version = msg["params"]["protocolVersion"].as_str()
                .filter(|v| SUPPORTED_PROTOCOLS.contains(v)).unwrap_or(LATEST_PROTOCOL);
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
            let result = if specs.iter().any(|t| t.name == name) { guarded(|| call(name, &args)) }
                else { Err(format!("there is no tool called {name}")) };
            return Some(tool_reply(id, result));
        }
        "shutdown" => Value::Null,
        m => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("no method {m}")}})),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// Bound the number of workers even when a timed-out Windows call can't unwind yet.
#[derive(Clone)]
pub struct Workers(Arc<AtomicUsize>, usize);

impl Workers {
    pub fn new(max: usize) -> Self { Self(Arc::new(AtomicUsize::new(0)), max) }

    pub fn spawn(&self, call: impl FnOnce() + Send + 'static) -> Result<(), String> {
        struct Slot(Arc<AtomicUsize>);
        impl Drop for Slot {
            fn drop(&mut self) { self.0.fetch_sub(1, Ordering::Relaxed); }
        }
        let mut count = self.0.load(Ordering::Relaxed);
        loop {
            if count >= self.1 { return Err("Sprawl is still busy with other requests; try again.".into()); }
            match self.0.compare_exchange_weak(count, count + 1, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(n) => count = n,
            }
        }
        let slot = Slot(self.0.clone());
        std::thread::Builder::new().spawn(move || { let _slot = slot; call(); })
            .map(|_| ()).map_err(|e| format!("Sprawl couldn't start a tool worker: {e}"))
    }
}

struct Request {
    cancel: Arc<Cancel>,
    deadline: Instant,
    wait: Duration,
    limit: Duration,
    // The latest usable status, if the budget expires while waiting for another snapshot.
    status: Mutex<Option<(Instant, String)>>,
    // An acknowledgement sequence number and owner bit for a scan that can outlive its reply.
    background: AtomicU64,
    finished: Mutex<Option<(Instant, Result<String, String>)>>,
}

impl Request {
    fn new(name: &str, input: &Value, received: Instant) -> Self {
        let scan = scan_dispatch(name);
        let wait = Duration::from_secs(input["wait_seconds"].as_u64().unwrap_or(0).min(crate::mcp_scan::MAX_WAIT));
        // A zero wait does not wait for the scan, but allows a bounded dispatch to its owner.
        let limit = if scan && !wait.is_zero() { wait } else if scan { PIPE_LIMIT }
            else if name == "web_lookup" { WEB_CONSENT_LIMIT } else { TOOL_LIMIT };
        Self { cancel: Arc::new(Cancel::default()), deadline: received + limit, wait, limit,
            status: Mutex::new(None), background: AtomicU64::new(0), finished: Mutex::new(None) }
    }

    fn keep_background(&self, in_app: bool) {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        self.background.store(NEXT.fetch_add(1, Ordering::Relaxed) * 2 + u64::from(in_app), Ordering::Relaxed);
    }

    fn time_left(&self) -> Duration { self.deadline.saturating_duration_since(Instant::now()) }

    fn observe_status(&self, observed_at: Instant, text: &str) {
        if observed_at <= self.deadline {
            *self.status.lock().unwrap_or_else(|e| e.into_inner()) = Some((observed_at, text.to_string()));
        }
    }

    fn timed_out(&self) -> Result<String, String> {
        if let Some((_, text)) = self.status.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
            .filter(|(observed_at, _)| *observed_at <= self.deadline).cloned() {
            return Ok(text);
        }
        self.cancel.stop();
        Err(format!("Sprawl didn't finish the request within {} s; try again.", self.limit.as_secs()))
    }

    fn settle(&self, now: Instant) -> Option<Result<String, String>> {
        let result = self.finished.lock().unwrap_or_else(|e| e.into_inner()).take();
        match result {
            Some((finished, result)) if finished <= self.deadline => Some(result),
            Some(_) => Some(self.timed_out()),
            None if now >= self.deadline => Some(self.timed_out()),
            None => None,
        }
    }
}

enum Event {
    Line(Vec<u8>, Instant),
    BadLine,
    Finished(Value, Arc<Request>),
    Closed,
}

type Call = dyn Fn(&str, &Value, &Request) -> Result<String, String> + Send + Sync;

fn dispatch(specs: &[ToolSpec], call: Arc<Call>, events: mpsc::SyncSender<Event>, incoming: mpsc::Receiver<Event>,
    output_open: impl Fn() -> bool, mut reply: impl FnMut(Value) -> bool) {
    let workers = Workers::new(8);
    let mut active: HashMap<Value, Arc<Request>> = HashMap::new();
    let mut background: [Option<(Value, Arc<Request>)>; 2] = [None, None];
    // Dispatch only controls here. No disk reads, tool code, pipe waits or stdout writes.
    let mut shutdown = None;
    let mut output_ok = true;
    let parse_error = || json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}});
    'dispatch: while output_open() {
        macro_rules! respond {
            ($reply:expr) => {
                if !reply($reply) { output_ok = false; break 'dispatch; }
            };
        }
        let now = Instant::now();
        active.retain(|id, request| {
            if !output_ok { return true; }
            let Some(result) = request.settle(now) else { return true; };
            keep_scan(&mut background, id, request);
            output_ok = reply(tool_reply(id.clone(), result));
            false
        });
        if !output_ok { break; }
        // Even with a full input queue or a silent stdin, notice an output failure promptly.
        let next = active.values().map(|r| r.time_left()).fold(stdio::CHECK_OUTPUT, Duration::min);
        let event = match incoming.recv_timeout(next) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        match event {
            Event::Closed => break,
            Event::BadLine => { respond!(parse_error()); }
            Event::Finished(id, request) => {
                if active.get(&id).is_some_and(|r| Arc::ptr_eq(r, &request)) {
                    if let Some(result) = request.settle(Instant::now()) {
                        active.remove(&id);
                        keep_scan(&mut background, &id, &request);
                        respond!(tool_reply(id, result));
                    }
                }
            }
            Event::Line(line, received) => {
                if line.iter().all(u8::is_ascii_whitespace) { continue; }
                let Ok(msg) = serde_json::from_slice::<Value>(&line) else {
                    respond!(parse_error());
                    continue;
                };
                let id = msg.get("id");
                if let Some(id) = id.filter(|id| active.contains_key(*id)) {
                    respond!(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32600, "message": "id already in use"}}));
                    continue;
                }
                match (msg["method"].as_str(), id) {
                    (Some("notifications/cancelled"), None) => {
                        let id = &msg["params"]["requestId"];
                        if let Some(request) = active.remove(id) {
                            request.cancel.stop();
                            respond!(tool_reply(id.clone(), Err("Sprawl cancelled the request.".into())));
                        } else {
                            for (_, request) in background.iter().flatten().filter(|(key, _)| key == id) {
                                request.cancel.stop();
                            }
                        }
                    }
                    (Some("shutdown" | "exit"), id) => {
                        shutdown = id.cloned();
                        break;
                    }
                    (Some("tools/call"), Some(id)) => {
                        let name = msg["params"]["name"].as_str().unwrap_or("");
                        if !specs.iter().any(|t| t.name == name) {
                            respond!(tool_reply(id.clone(), Err(format!("there is no tool called {name}"))));
                            continue;
                        }
                        let input = msg["params"].get("arguments").cloned().unwrap_or_else(|| json!({}));
                        let request = Arc::new(Request::new(name, &input, received));
                        let (worker_id, worker, name, events, call) = (id.clone(), request.clone(), name.to_string(), events.clone(), call.clone());
                        match workers.spawn(move || {
                            let result = guarded(|| {
                                worker.cancel.check()?;
                                if worker.time_left().is_zero() { return worker.timed_out(); }
                                call(&name, &input, &worker)
                            });
                            *worker.finished.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), result));
                            // The dispatcher checks completions even if its bounded queue is full.
                            let _ = events.try_send(Event::Finished(worker_id, worker));
                        }) {
                            Ok(()) => { active.insert(id.clone(), request); }
                            Err(e) => { respond!(tool_reply(id.clone(), Err(e))); }
                        }
                    }
                    _ => if let Some(r) = handle(&msg, specs, &mut |_, _| unreachable!()) { respond!(r); },
                }
            }
        }
    }
    for request in active.values().chain(background.iter().flatten().map(|(_, r)| r)) {
        request.cancel.stop();
    }
    if !output_ok || !output_open() { return; }
    if let Some(id) = shutdown {
        if !reply(json!({"jsonrpc": "2.0", "id": id, "result": null})) { return; }
    }
    for (id, _) in active {
        if !reply(tool_reply(id, Err("Sprawl is shutting down.".into()))) { break; }
    }
}

fn keep_scan(background: &mut [Option<(Value, Arc<Request>)>; 2], id: &Value, request: &Arc<Request>) {
    let stamp = request.background.load(Ordering::Relaxed);
    if stamp == 0 { return; }
    let current = &mut background[(stamp % 2) as usize];
    if current.as_ref().is_none_or(|(_, r)| r.background.load(Ordering::Relaxed) < stamp) {
        *current = Some((id.clone(), request.clone()));
    }
}

/// stdin can block even after shutdown; stdout can block if the client stops reading.
/// Neither blocks the dispatcher, and neither is joined indefinitely.
fn serve(specs: Vec<ToolSpec>, call: impl Fn(&str, &Value, &Request) -> Result<String, String> + Send + Sync + 'static) {
    stdio::serve_io(&specs, Arc::new(call), std::io::BufReader::new(std::io::stdin()), std::io::stdout(), stdio::WRITE_LIMIT);
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

struct BridgeAnswer(Value);

impl BridgeAnswer {
    fn text(&self) -> &str { self.0["text"].as_str().unwrap_or("Sprawl's window returned an invalid answer; try again.") }
    fn ok(&self) -> bool { self.0["ok"].as_bool() == Some(true) }
    fn result(self) -> Result<String, String> {
        if self.ok() { Ok(self.text().into()) } else { Err(self.text().into()) }
    }
    fn scan(self) -> Result<ScanReply, String> {
        if !self.ok() { return Err(self.text().into()); }
        serde_json::from_value(self.0["scan"].clone()).map_err(|_| "Sprawl's window didn't return a scan status; try again.".into())
    }
}

/// Connect afresh every time; a busy or timed-out app isn't a missing app.
fn ask_app(name: &str, input: &Value, request: &Request) -> Result<Option<BridgeAnswer>, String> {
    ask_pipe(name, input, request.deadline, &request.cancel, crate::pipe::Client::connect)
}

fn scan_dispatch(name: &str) -> bool { matches!(name, "scan_path" | "scan_status") }

fn ask_pipe(name: &str, input: &Value, deadline: Instant, cancel: &Cancel,
    connect: impl FnOnce(Instant, &std::sync::atomic::AtomicBool) -> std::io::Result<Option<crate::pipe::Client>>) -> Result<Option<BridgeAnswer>, String> {
    let now = Instant::now();
    let connect_until = deadline.min(now + PIPE_LIMIT);
    let Some(client) = connect(connect_until, cancel.flag()).map_err(|e| match e.kind() {
        std::io::ErrorKind::TimedOut => format!("Sprawl's window is busy; couldn't connect in {} s. Try again.", connect_until.saturating_duration_since(now).as_secs_f64().ceil() as u64),
        _ => pipe_error(e),
    })? else { return Ok(None); };
    // Scan requests over the pipe are non-waiting dispatch/status snapshots. Ordinary tools
    // may need to work over the full tree, but never get a fresh budget after connecting.
    let until = if scan_dispatch(name) { connect_until } else { deadline };
    let key = pipe_cancel_key(name, cancel.export())?;
    let left = until.saturating_duration_since(Instant::now());
    let line = pipe_request(name, input, left, key);
    let answer = client.ask(&line.to_string(), until, cancel.flag()).map_err(|e| match e.kind() {
        std::io::ErrorKind::TimedOut => format!("Sprawl's window didn't answer in {} s; try again.", left.as_secs_f64().ceil() as u64),
        _ => pipe_error(e),
    })?;
    serde_json::from_str(&answer).map(BridgeAnswer).map(Some)
        .map_err(|_| "Sprawl's window returned an invalid answer; try again.".into())
}

fn pipe_cancel_key(name: &str, key: std::io::Result<String>) -> Result<Option<String>, String> {
    match key {
        Ok(key) => Ok(Some(key)),
        Err(e) if name == "scan_path" => Err(format!("Sprawl couldn't start a cancellable scan in its window: {e}. Try again.")),
        Err(_) => Ok(None),
    }
}

fn pipe_error(e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::Interrupted { "Sprawl cancelled the request.".into() }
    else { format!("Couldn't get an answer from Sprawl's window: {e}; try again.") }
}

fn pipe_request(name: &str, input: &Value, left: Duration, key: Option<String>) -> Value {
    // Only a relative budget goes on the wire; the client keeps the original absolute deadline.
    let millis = left.as_nanos().div_ceil(1_000_000).min(GUI_MAX_WAIT.as_millis()) as u64;
    let mut req = json!({"tool": name, "input": input, "timeout_ms": millis});
    if let Some(key) = key { req["cancel"] = json!(key); }
    req
}

#[derive(Default)]
struct User {
    index: Index,
    memory: OnceLock<Memory>,
    settings: OnceLock<()>,
    last: Mutex<Option<u64>>,
}

impl User {
    fn scans(&self, name: &str, input: &Value, request: &Request) -> Result<String, String> {
        self.scans_with(name, input, request, |name, input| ask_app(name, input, request))
    }

    fn scans_with(&self, name: &str, input: &Value, request: &Request, from_app: impl Fn(&str, &Value) -> Result<Option<BridgeAnswer>, String>) -> Result<String, String> {
        use crate::mcp_scan::{app_job, PASS_TO_SERVER};
        let local = |name: &str, input: &Value| self.index.scan_now(name, input, false, request.cancel.clone());
        let original_job = if name == "scan_status" {
            match input.get("job") {
                Some(id) => Some(id.as_u64().ok_or("Give the integer job number returned by scan_path.")?),
                None => *self.last.lock().unwrap_or_else(|e| e.into_inner()),
            }
        } else { None };
        let mut input = input.clone();
        if let Some(args) = input.as_object_mut() { args.remove("wait_seconds"); }
        if let Some(job) = original_job { input["job"] = json!(job); }
        let mut answer = match original_job {
            Some(id) if !app_job(id) => local(name, &input)?,
            Some(id) => from_app(name, &input)?.ok_or_else(|| format!("Scan {id} was started in Sprawl's window, which isn't available now."))?.scan()?,
            None => match from_app(name, &input) {
                Ok(Some(a)) if a.ok() || a.text() != PASS_TO_SERVER => a.scan()?,
                Ok(_) => local(name, &input)?,
                Err(e) => {
                    // The GUI may have received scan_path just before the pipe failed.
                    // Its new job must not keep running without an acknowledged owner.
                    request.cancel.stop();
                    return Err(e);
                }
            },
        };
        let mut observed_at = Instant::now();
        if observed_at > request.deadline { return request.timed_out(); }
        if name == "scan_path" {
            if let Some(job) = answer.job {
                *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(job);
                if answer.started && answer.running {
                    request.keep_background(app_job(job));
                }
            }
        }
        while let Some(job) = answer.job.filter(|_| answer.running && !request.wait.is_zero()) {
            request.observe_status(observed_at, &answer.text);
            let left = request.time_left();
            std::thread::sleep(SCAN_POLL.min(left));
            if request.time_left().is_zero() { break; }
            request.cancel.check()?;
            let input = json!({"job": job});
            answer = if app_job(job) {
                match from_app("scan_status", &input) {
                    Ok(Some(a)) => a.scan()?,
                    Ok(None) => return Err(format!("Scan {job} was started in Sprawl's window, which isn't available now.")),
                    Err(_) if request.time_left().is_zero() => break,
                    Err(e) => return Err(format!("{e} Follow scan {job} with scan_status.")),
                }
            } else {
                local("scan_status", &input)?
            };
            observed_at = Instant::now();
            if observed_at > request.deadline { return request.timed_out(); }
        }
        if name == "scan_path" && answer.failed { Err(answer.text) } else { Ok(answer.text) }
    }

    fn call(&self, name: &str, input: &Value, request: &Request, load_settings: fn()) -> Result<String, String> {
        use crate::mcp_scan::{path_of, NOT_IN_SCAN, NO_SCAN_OPEN};
        if !input.is_object() { return Err("Tool arguments must be an object.".into()); }
        if crate::mcp_scan::is_scan_tool(name) { return self.scans(name, input, request); }
        let local = |app_running| {
            request.cancel.check()?;
            self.settings.get_or_init(load_settings);
            let memory = self.memory.get_or_init(Memory::load_readonly);
            self.index.answer_cancellable(name, input, memory, app_running, &request.cancel)
        };
        let app_only = matches!(name, "cleaners" | "set_filter");
        if !app_only && path_of(input).is_some_and(|p| self.index.has_scan_for(&p)) { return local(false); }
        match ask_app(name, input, request) {
            Ok(Some(a)) if !a.ok() && (a.text().ends_with(NOT_IN_SCAN) || a.text() == NO_SCAN_OPEN) => local(true),
            Ok(Some(a)) => a.result(),
            Ok(None) => local(false),
            Err(e) => { request.cancel.stop(); Err(e) }
        }
    }
}

/// Entry point for `sprawl --mcp [--tree <file> --out <file> --model <name>]`.
pub fn main(args: &[String], load_settings: fn()) {
    let dbg = std::env::var_os("SPRAWL_MCP_DEBUG").is_some();
    let step = |s: &str| if dbg { eprintln!("mcp: {s}") };
    step("start");
    let Some(tree_file) = arg(args, "--tree").map(PathBuf::from) else {
        // the user's own Claude Code: a path in a scan made here is answered from it; else the
        // running app when there is one (checked on every call); what the app doesn't have,
        // and everything while it isn't running, from the freshest of this server's scans and
        // History's, said so in every answer
        step("serving through the app, this server's scans or History");
        let user = User::default();
        serve(user_specs(), move |name, input, request| user.call(name, input, request, load_settings));
        return;
    };
    // Driven/chat calls share report state and must run serially. Load their snapshot and facts
    // on that worker, not before the stdio dispatcher can answer initialize or shutdown.
    let out_path = arg(args, "--out").map(PathBuf::from);
    let mode = if out_path.is_none() { Mode::Look } else if args.iter().any(|a| a == "--chat") {
        Mode::Chat { web: args.iter().any(|a| a == "--web") }
    } else { Mode::Driven };
    let args = args.to_vec();
    let (send, recv) = mpsc::sync_channel(1);
    std::thread::spawn(move || { load_settings(); driven(&args, tree_file, out_path, mode, recv); });
    serve(local_specs(mode), move |name, input, request| {
        let (reply, answer) = mpsc::channel();
        send.try_send(DrivenCall { name: name.into(), input: input.clone(), cancel: request.cancel.clone(), reply })
            .map_err(|e| match e {
                mpsc::TrySendError::Full(_) => "Sprawl is still busy with another report request; try again.",
                mpsc::TrySendError::Disconnected(_) => "Sprawl couldn't start the report worker.",
            })?;
        loop {
            request.cancel.check()?;
            let left = request.time_left();
            if left.is_zero() { return request.timed_out(); }
            match answer.recv_timeout(left.min(Duration::from_millis(50))) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err("Sprawl's report worker stopped before answering.".into()),
            }
        }
    });
}

struct DrivenCall {
    name: String,
    input: Value,
    cancel: Arc<Cancel>,
    reply: mpsc::Sender<Result<String, String>>,
}

fn driven(args: &[String], tree_file: PathBuf, out_path: Option<PathBuf>, mode: Mode, recv: mpsc::Receiver<DrivenCall>) {
    let tree = sprawl_core::local::file(&tree_file, crate::snapshot::MAX_FILE).ok().and_then(|mut f| crate::snapshot::read(&mut std::io::BufReader::new(&mut f.file)).ok());
    let Some((tree, meta)) = tree else {
        for call in recv { let _ = call.reply.send(Err("Sprawl couldn't read the scan it was given.".into())); }
        return;
    };
    let mut facts = Facts::gather();
    facts.attach_usage(&tree);
    let out = out_path.as_ref().and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
    let model = arg(args, "--model").unwrap_or_else(|| "claude-code".into());
    // read only: this process never renames or rewrites the user's memory
    let memory = Memory::load_readonly();
    let mut local = Local::new(&tree, &facts, &memory, &model, out);
    local.tools.scan_before = Some(meta.taken);
    if local.out.is_some() && matches!(mode, Mode::Chat { .. }) {
        local.mode = mode;
        local.out_path = out_path;
    }
    for call in recv {
        let result = guarded(|| local.call_cancellable(&call.name, &call.input, &call.cancel));
        let _ = call.reply.send(result);
    }
}

/// The app's side of the pipe: answer one request line against `tree`. `filter` receives a
/// filter the model wants shown; `findings` is the Advisor's list as text, and `found` the same
/// as a list (`None` while the Advisor is still looking).
/// `disk`: the client may have the disk read for it with the app's rights (new scans, the
/// background scans' answers, manifests). False when the app runs as administrator and the
/// client doesn't, unless the user allowed it: then only the scan on screen answers.
#[allow(clippy::too_many_arguments)]
/// One request through the app. `disk`: this client may have the app read the disk (it is
/// elevated too, the app isn't, or the user's switch allows it); without it only the tree on
/// screen and what the app already published answer, and `facts` must be that published copy
/// cut down to the tree (None: not published yet).
pub fn answer_bridge(line: &str, tree: Option<&Tree>, scan_before: Option<std::time::SystemTime>, facts: Option<&Facts>, memory: &Memory, findings: &str, found: Option<&[Finding]>, index: &Index, disk: bool, filter: &mut dyn FnMut(String)) -> String {
    answer_bridge_cancellable(line, tree, scan_before, facts, memory, findings, found, index, disk, filter, Arc::new(Cancel::default()))
}

#[allow(clippy::too_many_arguments)]
pub fn answer_bridge_cancellable(line: &str, tree: Option<&Tree>, scan_before: Option<std::time::SystemTime>, facts: Option<&Facts>, memory: &Memory, findings: &str, found: Option<&[Finding]>, index: &Index, disk: bool, filter: &mut dyn FnMut(String), cancel: Arc<Cancel>) -> String {
    let reply = |ok: bool, text: String| json!({"ok": ok, "text": crate::llm::clip(text, 40_000)}).to_string();
    let req = match bridge_request(line) { Ok(req) => req, Err(error) => return reply(false, error) };
    if let Err(e) = cancel.check() { return reply(false, e); }
    let name = req["tool"].as_str().unwrap_or("");
    let input = &req["input"];
    let as_reply = |a: Result<String, String>| match a {
        Ok(t) => reply(true, t),
        Err(e) => reply(false, e),
    };
    let elevated_only = || reply(false, ELEVATED_ONLY.into());
    let not_ready = || reply(false, "Sprawl is still reading this scan's apps: ask again in a moment.".into());
    // scans asked for by a server: made here, in the background, with the app's rights when
    // allowed; otherwise handed back to the server, which scans with its own
    if crate::mcp_scan::is_scan_tool(name) {
        return match index.scan_now(name, input, disk, cancel) {
            Ok(mut scan) => {
                scan.text = crate::llm::clip(scan.text, 40_000);
                json!({"ok": true, "text": scan.text, "scan": scan}).to_string()
            }
            Err(e) => reply(false, e),
        };
    }
    // a path the screen's tree doesn't hold, in a background scan: answered from that
    let elsewhere = crate::mcp_scan::path_of(input).filter(|p| tree.is_none_or(|t| crate::find_path(t, p).is_none()));
    if !disk {
        // the scan on screen only: no background scan, History or fresh read
        if elsewhere.is_some() && !matches!(name, "cleaners" | "set_filter") && tree.is_some() { return elevated_only(); }
        if matches!(name, "growth" | "read_manifest") { return elevated_only(); }
    } else if elsewhere.is_some_and(|p| !matches!(name, "cleaners" | "set_filter") && index.has_scan_for(&p)) {
        return as_reply(index.answer_cancellable(name, input, memory, true, &cancel));
    }
    let Some(tree) = tree else { return reply(false, crate::mcp_scan::NO_SCAN_OPEN.into()) };
    // a whole-scan tool asked about another scan: the server answers it from its own or History's
    if matches!(name, "findings" | "cleaners" | "scan_report") {
        if let Some(p) = crate::mcp_scan::path_of(input).filter(|p| crate::find_path(tree, p).is_none()) {
            return reply(false, format!("{} {}", p.display(), crate::mcp_scan::NOT_IN_SCAN));
        }
    }
    match name {
        "findings" => {
            let f = findings.split(CLEANERS_MARK).next().unwrap_or("");
            let empty = match found {
                None => "The Advisor is still looking at this scan: ask again in a moment.",
                Some(_) => "Sprawl's rules ran over this scan and none matched.",
            };
            reply(true, if f.trim().is_empty() { empty.into() } else { f.to_string() })
        }
        "cleaners" => {
            let c = findings.split_once(CLEANERS_MARK).map_or("", |(_, c)| c);
            reply(true, if c.trim().is_empty() { "The Advisor hasn't listed cleaners yet (or is still looking).".into() } else { c.to_string() })
        }
        "scan_report" => match facts {
            Some(facts) if !disk => reply(true, crate::llm::report::build_restricted(tree, facts, found, &memory.labels)),
            Some(facts) => reply(true, crate::llm::report::build(tree, facts, found, None, None, &memory.labels)),
            None => not_ready(),
        },
        "owner_of" | "search" if facts.is_none() => not_ready(),
        _ if bridge_specs().iter().any(|t| t.name == name) => {
            let none = Facts::default();
            let facts = facts.unwrap_or(&none);
            let mut tools = if disk { Tools::new(tree, facts, "claude-code", &[], memory) } else { Tools::restricted(tree, facts, memory) };
            tools.scan_before = scan_before;
            match tools.call_cancellable(name, input, cancel.flag()) {
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

/// Why a client that isn't administrator gets only the scan on screen from an elevated app.
pub const ELEVATED_ONLY: &str = "Sprawl runs as administrator and this client doesn't, so it answers only from the scan on screen (new scans run with this client's own rights). The switch \"MCP tools reading the disk (as administrator)\" in Settings > Scanning lets Sprawl read the disk for it with administrator rights.";

fn bridge_request(line: &str) -> Result<Value, String> {
    let req: Value = serde_json::from_str(line).map_err(|_| "bad request")?;
    let name = req["tool"].as_str().ok_or("bad request")?;
    if !user_specs().iter().any(|tool| tool.name == name) { return Err(format!("there is no tool called {name}")); }
    if !req["input"].is_object() { return Err("bad request".into()); }
    Ok(req)
}

/// Gate the host's entire request preparation, including lazy facts and cache/index work.
pub fn with_bridge_request<T>(line: &str, answer: impl FnOnce() -> T) -> Result<T, String> {
    bridge_request(line)?;
    Ok(answer())
}

/// Called only after the named pipe has authenticated the peer. The event is not authority to
/// query anything; the usual tool allowlist and disk/elevation gate still apply.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BridgeKind { ScanControl, ScanList, Query }

pub fn bridge_request_options(line: &str) -> Result<(Arc<Cancel>, BridgeKind, Duration), String> {
    let req = bridge_request(line)?;
    let name = req["tool"].as_str().unwrap_or("");
    let limit = if scan_dispatch(name) { PIPE_LIMIT } else { GUI_MAX_WAIT };
    let wait = match req.get("timeout_ms") {
        Some(value) => Duration::from_millis(value.as_u64().ok_or("bad request timeout")?).min(limit),
        None => if scan_dispatch(name) { PIPE_LIMIT } else { TOOL_LIMIT },
    };
    let cancel = match req.get("cancel") {
        Some(key) => Cancel::import(key.as_str().ok_or("bad cancellation signal")?)
            .map_err(|_| "Sprawl couldn't open the request's cancellation signal.")?,
        None if name == "scan_path" => return Err("A scan in Sprawl's window needs a cancellation signal.".into()),
        None => Cancel::default(),
    };
    let kind = if scan_dispatch(name) { BridgeKind::ScanControl }
        else if name == "list_scans" { BridgeKind::ScanList } else { BridgeKind::Query };
    Ok((Arc::new(cancel), kind, wait))
}

/// Wait off the GUI thread. Poll imported cancellation so search sees it during traversal.
pub fn wait_for_bridge_reply(cancel: &Cancel, until: Instant, recv: mpsc::Receiver<(Instant, String)>) -> Result<String, String> {
    loop {
        cancel.check()?;
        let left = until.saturating_duration_since(Instant::now());
        match recv.recv_timeout(left.min(Duration::from_millis(20))) {
            Ok((finished, out)) if finished <= until => return Ok(out),
            Err(mpsc::RecvTimeoutError::Timeout) if !left.is_zero() => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) if !left.is_zero() => return Err("Sprawl's window stopped answering; try again.".into()),
            _ => {
                cancel.stop();
                return Err("Sprawl's window ran out of time answering; try again.".into());
            }
        }
    }
}

/// Run the live permission gate only after admission. A gate that returns after its request
/// expired cannot start privileged work, and it never holds the single pipe past the deadline.
pub fn bridge_work(pool: &Workers, cancel: Arc<Cancel>, until: Instant,
    gate: impl FnOnce() -> bool + Send + 'static,
    answer: impl FnOnce(bool, Arc<Cancel>) -> String + Send + 'static) -> Result<String, String> {
    let (send, recv) = mpsc::channel();
    let work = cancel.clone();
    let timeout = || "Sprawl's window ran out of time answering; try again.".to_string();
    pool.spawn(move || {
        let out = guarded(|| {
            work.check()?;
            if Instant::now() >= until { return Err(timeout()); }
            let disk = gate();
            work.check()?;
            if Instant::now() >= until { return Err(timeout()); }
            Ok(answer(disk, work))
        }).unwrap_or_else(|e| json!({"ok": false, "text": e}).to_string());
        let _ = send.send((Instant::now(), out));
    })?;
    wait_for_bridge_reply(&cancel, until, recv)
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

    struct Session {
        events: mpsc::SyncSender<Event>,
        replies: mpsc::Receiver<Value>,
    }

    impl Session {
        fn new(call: impl Fn(&str, &Value, &Request) -> Result<String, String> + Send + Sync + 'static) -> Self {
            let (events, incoming) = mpsc::sync_channel(8);
            let (out, replies) = mpsc::channel();
            let workers = events.clone();
            std::thread::spawn(move || dispatch(&user_specs(), Arc::new(call), workers, incoming, || true, |r| out.send(r).is_ok()));
            Self { events, replies }
        }

        fn send(&self, msg: Value) {
            self.events.send(Event::Line(msg.to_string().into_bytes(), Instant::now())).unwrap();
        }

        fn tool(&self, id: Value, name: &str, arguments: Value) {
            self.send(json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}}));
        }

        fn reply(&self) -> Value { self.replies.recv_timeout(Duration::from_secs(3)).expect("stdio reply") }
    }

    impl Drop for Session {
        fn drop(&mut self) { let _ = self.events.send(Event::Closed); }
    }

    #[test]
    fn stalled_pipe_times_out_without_blocking_ping_or_tool_list() {
        let (ready, accepted) = mpsc::channel();
        let (release, done) = mpsc::channel();
        let (name, peer) = crate::pipe::test_peer(move |_| {
            ready.send(()).unwrap();
            let _ = done.recv_timeout(Duration::from_secs(5));
            None
        });
        let session = Session::new(move |tool, args, request| {
            ask_pipe(tool, args, Instant::now() + Duration::from_secs(1), &request.cancel, |until, cancel| crate::pipe::Client::connect_to(&name, until, cancel))?
                .expect("authenticated fake pipe").result()
        });
        let started = Instant::now();
        session.tool(json!(1), "scan_report", json!({}));
        accepted.recv_timeout(Duration::from_secs(2)).unwrap();
        session.send(json!({"id": 2, "method": "ping"}));
        session.send(json!({"id": 3, "method": "tools/list"}));
        assert_eq!(session.reply(), json!({"jsonrpc": "2.0", "id": 2, "result": {}}));
        assert_eq!(session.reply()["id"], 3);
        let timed_out = session.reply();
        assert_eq!(timed_out["id"], 1);
        assert_eq!(timed_out["result"]["isError"], true);
        assert!(timed_out["result"]["content"][0]["text"].as_str().unwrap().contains("window didn't answer in 1 s"));
        assert!(started.elapsed() < Duration::from_secs(3));
        release.send(()).unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn a_slow_gui_answer_gets_the_tool_budget_but_a_second_connection_does_not() {
        let (ready, accepted) = mpsc::channel();
        let (release, done) = mpsc::channel();
        let (name, peer) = crate::pipe::test_peer(move |line| {
            let (cancel, kind, wait) = bridge_request_options(&line).unwrap();
            assert!(kind == BridgeKind::Query && wait > PIPE_LIMIT && wait <= TOOL_LIMIT, "{wait:?}");
            let until = Instant::now() + wait;
            let (send, recv) = mpsc::channel();
            std::thread::spawn(move || {
                if done.recv_timeout(Duration::from_secs(10)).is_ok() {
                    // Finish past the old 5 s cutoff, not right on it.
                    std::thread::sleep(Duration::from_millis(100));
                    let _ = send.send((Instant::now(), json!({"ok": true, "text": "slow report"}).to_string()));
                }
            });
            ready.send(()).unwrap();
            Some(wait_for_bridge_reply(&cancel, until, recv).unwrap())
        });
        let peer_name = name.clone();
        let session = Session::new(move |tool, args, request| {
            ask_pipe(tool, args, request.deadline, &request.cancel, |until, cancel| crate::pipe::Client::connect_to(&peer_name, until, cancel))?
                .expect("authenticated fake pipe").result()
        });
        let started = Instant::now();
        session.tool(json!(1), "scan_report", json!({}));
        accepted.recv_timeout(Duration::from_secs(3)).unwrap();
        let second = Instant::now();
        let busy = ask_pipe("search", &json!({}), second + TOOL_LIMIT, &Cancel::default(), |until, cancel| {
            crate::pipe::Client::connect_to(&name, until, cancel)
        });
        assert!(matches!(&busy, Err(e) if e.contains("busy") && e.contains("5 s")));
        assert!(second.elapsed() >= PIPE_LIMIT && second.elapsed() < PIPE_LIMIT + Duration::from_secs(2));
        release.send(()).unwrap();
        let reply = session.reply();
        assert_eq!(reply["result"]["content"][0]["text"], "slow report");
        assert_eq!(reply["result"]["isError"], false);
        assert!(started.elapsed() > PIPE_LIMIT && started.elapsed() < TOOL_LIMIT);
        peer.join().unwrap();
    }

    #[test]
    fn bridge_budgets_are_clamped_and_a_cancel_key_is_optional() {
        let line = pipe_request("search", &json!({}), Duration::from_millis(9123), None);
        assert!(line.get("cancel").is_none());
        let (cancel, kind, wait) = bridge_request_options(&line.to_string()).unwrap();
        assert!(!cancel.stopped() && kind == BridgeKind::Query);
        assert_eq!(wait, Duration::from_millis(9123));
        for (tool, limit) in [("largest", GUI_MAX_WAIT), ("scan_path", PIPE_LIMIT), ("scan_status", PIPE_LIMIT)] {
            let line = json!({"tool": tool, "input": {}, "timeout_ms": u64::MAX, "cancel": cancel.export().unwrap()});
            assert_eq!(bridge_request_options(&line.to_string()).unwrap().2, limit);
        }
    }

    #[test]
    fn gui_scan_dispatch_requires_a_cancel_key_but_queries_do_not() {
        let failed = || std::io::Error::other("simulated event failure");
        assert!(pipe_cancel_key("scan_path", Err(failed())).is_err());
        assert!(pipe_cancel_key("search", Err(failed())).unwrap().is_none());
        let line = pipe_request("scan_path", &json!({}), PIPE_LIMIT, None);
        assert!(matches!(bridge_request_options(&line.to_string()), Err(e) if e.contains("needs a cancellation signal")));
    }

    #[test]
    fn duplicate_active_ids_are_refused_before_any_method_runs() {
        let (ready, started) = mpsc::channel();
        let (release, done) = mpsc::channel();
        let done = Mutex::new(done);
        let session = Session::new(move |_, _, _| {
            ready.send(()).unwrap();
            done.lock().unwrap().recv_timeout(Duration::from_secs(3)).unwrap();
            Ok("original".into())
        });
        session.tool(json!(1), "search", json!({}));
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        for method in ["ping", "tools/list", "tools/call", "shutdown"] {
            session.send(json!({"id": 1, "method": method}));
            assert_eq!(session.reply()["error"], json!({"code": -32600, "message": "id already in use"}));
        }
        release.send(()).unwrap();
        assert_eq!(session.reply()["result"]["content"][0]["text"], "original");
    }

    #[test]
    fn completion_time_decides_the_reply_even_if_the_receiver_is_delayed() {
        let now = Instant::now();
        let early = Request::new("search", &json!({}), now);
        let after = early.deadline + Duration::from_millis(1);
        *early.finished.lock().unwrap() = Some((early.deadline, Ok("on time".into())));
        assert_eq!(early.settle(after).unwrap().unwrap(), "on time");
        let late = Request::new("search", &json!({}), now);
        *late.finished.lock().unwrap() = Some((after, Ok("too late".into())));
        assert!(late.settle(after).unwrap().unwrap_err().contains("within 25 s"));
        assert!(late.cancel.stopped());
        let scan = Request::new("scan_path", &json!({}), now);
        scan.observe_status(scan.deadline, "still scanning");
        *scan.finished.lock().unwrap() = Some((after, Ok("too late".into())));
        assert_eq!(scan.settle(after).unwrap().unwrap(), "still scanning");
        assert!(!scan.cancel.stopped());

        let (send, recv) = mpsc::channel();
        let cancel = Cancel::default();
        send.send((after, "late GUI result".into())).unwrap();
        assert!(wait_for_bridge_reply(&cancel, early.deadline, recv).unwrap_err().contains("ran out of time"));
        assert!(cancel.stopped());
        let (send, recv) = mpsc::channel();
        let past = now - Duration::from_millis(1);
        send.send((past, "on-time GUI result".into())).unwrap();
        assert_eq!(wait_for_bridge_reply(&Cancel::default(), past, recv).unwrap(), "on-time GUI result");
    }

    #[test]
    fn a_poll_observed_after_the_deadline_cannot_replace_an_on_time_status() {
        let input = json!({"wait_seconds": 1});
        let request = Request::new("scan_status", &input, Instant::now());
        let on_time = request.deadline;
        let late = on_time + Duration::from_millis(1);
        request.observe_status(on_time, "on-time scan status");
        request.observe_status(late, "late scan status");
        assert_eq!(request.settle(late).unwrap().unwrap(), "on-time scan status");
        assert!(!request.cancel.stopped());

        let late_only = Request::new("scan_status", &input, on_time - Duration::from_secs(1));
        late_only.observe_status(late, "late scan status");
        assert!(late_only.settle(late).unwrap().unwrap_err().contains("within 1 s"));
        assert!(late_only.cancel.stopped());
    }

    #[test]
    fn a_stalled_gui_permission_gate_releases_the_pipe_and_grants_no_late_access() {
        struct Report(Option<mpsc::Sender<bool>>);
        impl Drop for Report {
            fn drop(&mut self) { if let Some(send) = self.0.take() { let _ = send.send(false); } }
        }
        let (entered, ready) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let (proof, admitted) = mpsc::channel();
        let (mut resume, mut probe) = (Some(resume), Some(Report(Some(proof))));
        let pool = Workers::new(2);
        let (name, peer) = crate::pipe::test_peer_requests(2, move |line| {
            let (cancel, _, wait) = bridge_request_options(&line).unwrap();
            let first = resume.take();
            let gate_entered = entered.clone();
            let late = probe.take();
            let result = bridge_work(&pool, cancel, Instant::now() + wait, move || {
                if let Some(resume) = first {
                    gate_entered.send(()).unwrap();
                    let _ = resume.recv_timeout(Duration::from_secs(5));
                }
                true
            }, move |disk, _| {
                if let Some(mut late) = late { late.0.take().unwrap().send(true).unwrap(); }
                json!({"ok": disk, "text": "second request answered"}).to_string()
            });
            Some(result.unwrap_or_else(|e| json!({"ok": false, "text": e}).to_string()))
        });
        let call = |name: &str, wait| {
            let deadline = Instant::now() + Duration::from_secs(3);
            let cancel = Cancel::default();
            let pipe = crate::pipe::Client::connect_to(name, deadline, cancel.flag()).unwrap().unwrap();
            let result = pipe.ask(&json!({"tool": "largest", "input": {}, "timeout_ms": wait}).to_string(), deadline, cancel.flag()).unwrap();
            serde_json::from_str::<Value>(&result).unwrap()
        };
        let first_name = name.clone();
        let first = std::thread::spawn(move || call(&first_name, 300));
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        let second = call(&name, 2000);
        assert_eq!(second["text"], "second request answered");
        assert!(matches!(admitted.try_recv(), Err(mpsc::TryRecvError::Empty)), "the first gate is still paused");
        let expired = first.join().unwrap();
        assert_eq!(expired["ok"], false);
        assert!(expired["text"].as_str().unwrap().contains("ran out of time"));
        release.send(()).unwrap();
        assert!(!admitted.recv_timeout(Duration::from_secs(2)).unwrap(), "the late gate must not start privileged work");
        peer.join().unwrap();
    }

    #[test]
    fn cancellation_signals_the_worker_and_ignores_its_late_result() {
        let (started, ready) = mpsc::channel();
        let (stopped, observed) = mpsc::channel();
        let session = Session::new(move |_, _, request| {
            started.send(()).unwrap();
            while !request.cancel.stopped() && !request.time_left().is_zero() {
                std::thread::sleep(Duration::from_millis(5));
            }
            stopped.send(request.cancel.stopped()).unwrap();
            Ok("late".into())
        });
        session.tool(json!("query"), "search", json!({"query": "*.tmp"}));
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        session.send(json!({"method": "notifications/cancelled", "params": {"requestId": "query"}}));
        let cancelled = session.reply();
        assert_eq!(cancelled["id"], "query");
        assert_eq!(cancelled["result"]["isError"], true);
        assert!(cancelled["result"]["content"][0]["text"].as_str().unwrap().contains("cancelled"));
        assert!(observed.recv_timeout(Duration::from_secs(2)).unwrap());
        session.send(json!({"id": 2, "method": "ping"}));
        assert_eq!(session.reply()["id"], 2);
    }

    #[test]
    fn a_panicking_tool_replies_with_an_error_and_the_server_keeps_running() {
        let session = Session::new(|_, _, _| panic!("fixture panic"));
        session.tool(json!(1), "largest", json!({}));
        let result = session.reply();
        assert_eq!(result["id"], 1);
        assert_eq!(result["result"]["isError"], true);
        assert!(result["result"]["content"][0]["text"].as_str().unwrap().contains("internal error"));
        session.send(json!({"id": 2, "method": "ping"}));
        assert_eq!(session.reply()["id"], 2);
    }

    #[test]
    fn initialize_negotiates_a_supported_version() {
        let session = Session::new(|_, _, _| unreachable!());
        for (requested, expected) in [
            (json!("2024-11-05"), "2024-11-05"), (json!("2025-03-26"), "2025-03-26"),
            (json!(LATEST_PROTOCOL), LATEST_PROTOCOL), (json!("2099-01-01"), LATEST_PROTOCOL),
            (Value::Null, LATEST_PROTOCOL)
        ] {
            session.send(json!({"id": 1, "method": "initialize", "params": {"protocolVersion": requested}}));
            assert_eq!(session.reply()["result"]["protocolVersion"], expected);
        }
    }

    #[test]
    fn shutdown_signals_already_acknowledged_background_scans() {
        let (send, recv) = mpsc::channel();
        let session = Session::new(move |_, _, request| {
            let remote = Cancel::import(&request.cancel.export().unwrap()).unwrap();
            send.send(remote).unwrap();
            request.keep_background(true);
            Ok("still scanning".into())
        });
        session.tool(json!(1), "scan_path", json!({}));
        let remote = recv.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(session.reply()["id"], 1);
        assert!(!remote.stopped());
        session.send(json!({"id": 2, "method": "shutdown"}));
        assert_eq!(session.reply(), json!({"jsonrpc": "2.0", "id": 2, "result": null}));
        assert!(session.replies.recv_timeout(Duration::from_secs(2)).is_err(), "dispatcher exits");
        assert!(remote.stopped(), "the GUI's signal is set even after the first reply");
    }

    #[test]
    fn scan_status_uses_the_original_owner_instead_of_falling_back() {
        let user = User::default();
        let input = json!({"job": 42});
        let request = Request::new("scan_status", &input, Instant::now());
        assert_eq!(user.scans_with("scan_status", &input, &request, |_, _| panic!("local job must not ask GUI")).unwrap_err(), "There is no scan 42.");
        *user.last.lock().unwrap() = Some(43);
        let error = user.scans_with("scan_status", &json!({}), &request, |tool, args| {
            assert_eq!(tool, "scan_status");
            assert_eq!(args["job"], 43);
            Ok(None)
        }).unwrap_err();
        assert!(error.contains("43") && error.contains("window"), "{error}");
    }

    #[test]
    fn scan_wait_budget_returns_the_job_even_if_a_gui_status_call_stalls() {
        let (send, recv) = mpsc::channel();
        let (release, stalled) = mpsc::channel();
        let stalled = Mutex::new(stalled);
        let user = User::default();
        let session = Session::new(move |tool, input, request| {
            user.scans_with(tool, input, request, |tool, args| {
                if tool == "scan_path" {
                    send.send(request.cancel.clone()).unwrap();
                    return Ok(Some(BridgeAnswer(json!({"ok": true, "scan": {
                        "text": "Scan 43 is still scanning. Call scan_status with job 43.",
                        "job": 43, "running": true, "started": true, "failed": false
                    }}))));
                }
                assert_eq!(args["job"], 43);
                let _ = stalled.lock().unwrap().recv_timeout(Duration::from_secs(3));
                Err("fixture stalled".into())
            })
        });
        let start = Instant::now();
        session.tool(json!(1), "scan_path", json!({"wait_seconds": 1}));
        let cancel = recv.recv_timeout(Duration::from_secs(2)).unwrap();
        let reply = session.reply();
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["isError"], false);
        assert!(reply["result"]["content"][0]["text"].as_str().unwrap().contains("Call scan_status with job 43"));
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(!cancel.stopped(), "a wait timeout is not a scan cancellation");
        session.send(json!({"method": "notifications/cancelled", "params": {"requestId": 1}}));
        session.send(json!({"method": "ping", "id": 2}));
        assert_eq!(session.reply()["id"], 2);
        assert!(cancel.stopped(), "the initiating ID still cancels a background scan");
        release.send(()).unwrap();
    }

    #[test]
    fn unadvertised_calls_never_reach_effects_or_scan_analysis() {
        let mut calls = 0;
        for name in ["propose_rule", "propose_cleaner", "report_finding", "web_lookup", "remember", "finish"] {
            let request = json!({"id": 1, "method": "tools/call", "params": {"name": name}});
            let result = handle(&request, &user_specs(), &mut |_, _| { calls += 1; Ok("unexpected".into()) }).unwrap();
            assert_eq!(result["result"]["isError"], true);
            let mut index = crate::mcp_scan::Index::default();
            let input = json!({"path": "Q:\\other", "title": "draft"});
            assert!(index.answer(name, &input, &Memory::default(), false).unwrap_err().starts_with("there is no tool called"));
            let result = answer_bridge(&json!({"tool": name, "input": input}).to_string(), None, None, Some(&Facts::default()), &Memory::default(), "", None, &mut index, true, &mut |_| panic!("unexpected filter"));
            assert!(result.contains("there is no tool called"));
        }
        assert_eq!(calls, 0);
        for line in ["not JSON", r#"{"tool":"propose_rule","input":{}}"#, r#"{"tool":"scan_report","input":null}"#] {
            assert!(with_bridge_request(line, || panic!("facts preparation reached before validation")).is_err());
        }
    }

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
        assert!(c.contains(&"propose_rule") && CHAT_EFFECTS.contains(&"propose_rule"));
        assert!(!names.contains(&"propose_rule"), "external read-only mode never writes proposals");
        assert!(c.contains(&"suggest_ticks") && c.contains(&"set_filter") && c.contains(&"largest") && !c.contains(&"web_lookup") && !c.contains(&"report_finding"), "{c:?}");
        let w: Vec<&str> = local_specs(Mode::Chat { web: true }).iter().map(|t| t.name).collect();
        assert!(w.contains(&"web_lookup"));
        assert_eq!(w.len(), c.len() + 1);
        let cfg = chat_config(Path::new("s.exe"), Path::new("t"), Path::new("o"), "m", false);
        assert!(cfg.contains("--chat") && !cfg.contains("--web"));
        assert!(chat_config(Path::new("s.exe"), Path::new("t"), Path::new("o"), "m", true).contains("--web"));
    }

    fn bridge_fixture() -> (PathBuf, Tree) {
        let dir = std::env::temp_dir().join(format!("sprawl-bridge-{}", crate::store::nonce()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lib.rs"), b"// test fixture").unwrap();
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        (dir, tree)
    }

    #[test]
    fn bridge_answers() {
        let (dir, tree) = bridge_fixture();
        let facts = Facts::from_apps(Vec::new());
        let mut shown = None;
        let root = tree.path_of(sprawl_core::ROOT).display().to_string();
        let a = answer_bridge(&json!({"tool": "list_children", "input": {"path": root}}).to_string(), Some(&tree), Some(std::time::SystemTime::now()), Some(&facts), &Memory::default(), "", None, &mut crate::mcp_scan::Index::default(), true, &mut |f| shown = Some(f));
        assert!(a.contains("\"ok\":true") && a.contains("lib.rs"), "{a}");
        let f = answer_bridge(&json!({"tool": "set_filter", "input": {"filter": "*.rs"}}).to_string(), Some(&tree), Some(std::time::SystemTime::now()), Some(&facts), &Memory::default(), "", None, &mut crate::mcp_scan::Index::default(), true, &mut |f| shown = Some(f));
        assert!(f.contains("\"ok\":true") && shown.as_deref() == Some("*.rs"));
        let n = answer_bridge(&json!({"tool": "report_finding", "input": {}}).to_string(), Some(&tree), Some(std::time::SystemTime::now()), Some(&facts), &Memory::default(), "", None, &mut crate::mcp_scan::Index::default(), true, &mut |_| {});
        assert!(n.contains("no tool"), "reporting isn't offered over the bridge");
        // the report says whether the rules ran, never a bare "(nothing)"
        let ask = |found: Option<&[Finding]>| answer_bridge(&json!({"tool": "scan_report", "input": {}}).to_string(), Some(&tree), Some(std::time::SystemTime::now()), Some(&facts), &Memory::default(), "", found, &mut crate::mcp_scan::Index::default(), true, &mut |_| {});
        assert!(ask(None).contains("haven't run"));
        assert!(ask(Some(&[])).contains("none matched"));
        let one = Finding {
            path: tree.path_of(sprawl_core::ROOT),
            owner: None,
            bytes: 1,
            risk: crate::finding::Risk::Green,
            remedy: crate::finding::Remedy::Delete,
            command: None,
            title: "a test finding".into(),
            evidence: vec![],
            confidence: 1.0,
            source: crate::finding::Source::Rule("t".into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: true,
            action: None,
            admin: false,
            carved: Vec::new(),
            gain: None,
            forced_from: None,
        };
        assert!(ask(Some(std::slice::from_ref(&one))).contains("a test finding"));
        let fl = answer_bridge(&json!({"tool": "findings", "input": {}}).to_string(), Some(&tree), Some(std::time::SystemTime::now()), Some(&facts), &Memory::default(), "", None, &mut crate::mcp_scan::Index::default(), true, &mut |_| {});
        assert!(fl.contains("still looking"), "{fl}");
        // a whole-scan tool about a path the app's tree doesn't hold goes back to the server
        for tool in ["scan_report", "findings", "cleaners"] {
            let other = answer_bridge(&json!({"tool": tool, "input": {"path": r"Q:\elsewhere"}}).to_string(), Some(&tree), Some(std::time::SystemTime::now()), Some(&facts), &Memory::default(), "", None, &mut crate::mcp_scan::Index::default(), true, &mut |_| {});
            assert!(other.contains("\"ok\":false") && other.contains(crate::mcp_scan::NOT_IN_SCAN), "{other}");
        }
        let inside = answer_bridge(&json!({"tool": "scan_report", "input": {"path": root}}).to_string(), Some(&tree), Some(std::time::SystemTime::now()), Some(&facts), &Memory::default(), "", None, &mut crate::mcp_scan::Index::default(), true, &mut |_| {});
        assert!(inside.contains("\"ok\":true"), "{inside}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn elevated_app_reads_only_the_screen_for_a_client_that_isnt() {
        let (dir, tree) = bridge_fixture();
        let root = tree.path_of(sprawl_core::ROOT).display().to_string();
        let app = crate::facts::App { name: "Here".into(), publisher: None, kind: crate::facts::AppKind::Installed, locations: vec![PathBuf::from(&root)], last_used: Some((std::time::SystemTime::now(), crate::facts::UsedFrom::Prefetch)), how_to_remove: String::new() };
        let away = crate::facts::App { name: "Elsewhere app".into(), locations: vec![PathBuf::from(r"Q:\Programs\Away")], ..app.clone() };
        let published = Facts::from_apps(vec![app, away]);
        let scoped = published.scoped_to(&tree);
        assert_eq!(scoped.apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Here"], "only apps of the scan on screen");
        let ask_with = |tool: &str, input: Value, disk: bool, facts: Option<&Facts>, index: &mut crate::mcp_scan::Index| answer_bridge(&json!({"tool": tool, "input": input}).to_string(), Some(&tree), None, facts, &Memory::default(), "", None, index, disk, &mut |_| {});
        let ask = |tool: &str, input: Value, disk: bool| ask_with(tool, input, disk, Some(&scoped), &mut crate::mcp_scan::Index::for_app(false));
        // disk reads beyond the screen: refused without the switch, answered with it
        for (tool, input) in [("read_manifest", json!({"path": format!(r"{root}\lib.rs")})), ("growth", json!({"path": root}))] {
            let no = ask(tool, input.clone(), false);
            assert!(no.contains("\"ok\":false") && no.contains("answers only from the scan on screen"), "{tool}: {no}");
            assert!(!ask(tool, input, true).contains("answers only from the scan on screen"), "{tool} answers when allowed");
        }
        // scans, through the app's own index: without the switch an elevated app hands them to
        // the client's own process; with it, the app scans (Astra 1.0.2 S3 #3)
        let elevated_app = !matches!(sprawl_core::elevation(), Ok(false));
        let off = ask("list_scans", json!({}), false);
        assert_eq!(off.contains(crate::mcp_scan::PASS_TO_SERVER), elevated_app, "{off}");
        let on = ask("list_scans", json!({}), true);
        assert!(!on.contains(crate::mcp_scan::PASS_TO_SERVER) && !on.contains("answers only from"), "{on}");
        // owner lookups only inside the tree; nothing about other apps in the report
        for disk in [false, true] {
            let outside = ask("owner_of", json!({"path": r"Q:\Programs\Away\x.exe"}), disk);
            assert!(outside.contains("\"ok\":false") && !outside.contains("Elsewhere app"), "{outside}");
        }
        let before = crate::llm::report::UNEXPLAINED_CALLS.with(|c| c.get());
        let report = ask("scan_report", json!({}), false);
        assert!(report.contains("\"ok\":true") && !report.contains("Elsewhere app"), "{report}");
        assert_eq!(crate::llm::report::UNEXPLAINED_CALLS.with(|c| c.get()), before, "the restricted report reads no settings (re-review S3 #1)");
        ask("scan_report", json!({}), true);
        assert_eq!(crate::llm::report::UNEXPLAINED_CALLS.with(|c| c.get()), before + 1, "the full one does");
        // facts not published yet: not ready, nothing gathered
        let early = ask_with("scan_report", json!({}), false, None, &mut crate::mcp_scan::Index::for_app(false));
        assert!(early.contains("\"ok\":false") && early.contains("still reading"), "{early}");
        // the scan on screen still answers
        let a = ask("list_children", json!({"path": root}), false);
        assert!(a.contains("\"ok\":true") && a.contains("lib.rs"), "{a}");
        let a = ask_with("largest", json!({"path": root}), false, None, &mut crate::mcp_scan::Index::for_app(false));
        assert!(a.contains("\"ok\":true"), "{a}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
