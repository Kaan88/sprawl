//! The loop: the model reads the report, calls tools, reports findings, and finishes with a
//! summary. Providers without tool calling (Claude Code here, small local models) get the same
//! report once and answer with JSON, which goes through the same checks.

use serde_json::Value;
use sprawl_core::Tree;

use super::anthropic::Anthropic;
use super::claude_code::ClaudeCode;
use super::openai::OpenAi;
use super::tools::{chat_specs, specs, Tools};
use super::{clip, Chat};
use crate::config::{self, AiConfig, Detected, Provider};
use crate::facts::Facts;
use crate::finding::Finding;
use crate::hidden::HiddenSpace;
use crate::memory::{Label, Memory, Pref, Proposal};

const SYSTEM: &str = "You are the disk-space advisor inside Sprawl, a disk analyser for Windows. \
The user is a power user who wants to win back space safely. You get a compact report of one scan and tools to look closer. \
Sprawl's built-in rules have already found the well-known caches and leftovers listed in the report; don't report those again.

Your job: find what the rules can't. Identify big folders nobody explained yet (what app or tool made them, whether they are still needed), \
duplicated or forgotten data, old downloads, game or app leftovers, oversized logs and dumps, and anything unusually large for what it is. \
Look before you judge: use the tools to check contents, dates and owners. Report each opportunity with report_finding, citing the exact bytes the tools gave. \
Be honest about risk: anything that is the user's own work, photos, documents, saves or projects is orange. Never suggest touching Windows itself, \
program installations (suggest uninstalling instead), or anything you can't explain. Fewer, solid findings beat many guesses. \
When you are done, call finish with a short summary.";

const SINGLE: &str = "You have no tools in this conversation. Answer with one JSON object and nothing else:
{\"summary\": \"a few sentences\", \"findings\": [{\"path\": \"C:\\\\...\", \"bytes\": 123, \"title\": \"...\", \"risk\": \"green|amber|orange|red\", \
\"remedy\": \"delete|uninstall|move|info\", \"evidence\": [\"...\"], \"confidence\": 0.8}]}
Only report paths that appear in the report, with the exact bytes shown there. Risk: green = comes back by itself, amber = costs something, \
orange = the user's own files, red = never touch. Sprawl writes any command itself.";

const CHAT: &str = "You answer questions about the user's disk inside Sprawl, a disk analyser for Windows. \
You get a compact report of the scan and tools to look closer; use them before you answer, and answer briefly and concretely, with sizes. \
To show something on the user's map, call set_filter. You can't delete anything: to win back space, point the user to the Advisor's findings, \
its 'I need N GB' plan, or 'Ask Claude'. If you learn what a folder is, or the user states a preference, call remember.";

/// Something the chat has for the app.
#[derive(Debug)]
pub enum ChatEvent {
    Progress(String),
    Reply(String),
    /// Show this filter on the map (empty clears it).
    Filter(String),
    Remember { prefs: Vec<Pref>, labels: Vec<Label> },
    Failed(String),
}

/// "Ask your disk": answer each question from `questions` until the channel closes. Runs on its
/// own thread with its own copy of the scan, so filtering the map doesn't end it.
pub fn chat(tree: &Tree, cfg: &AiConfig, memory: &Memory, notes: &str, questions: std::sync::mpsc::Receiver<String>, events: &mut dyn FnMut(ChatEvent), consent: &dyn Fn(&str) -> bool) {
    let Ok(first) = questions.recv() else { return };
    if cfg.provider == Provider::Off {
        events(ChatEvent::Failed("AI is off. Choose a provider with Set up AI.".into()));
        return;
    }
    let model = if cfg.model.trim().is_empty() { cfg.provider.default_model().to_string() } else { cfg.model.trim().to_string() };
    events(ChatEvent::Progress("Reading the scan...".into()));
    let mut facts = Facts::gather();
    facts.attach_usage(tree);
    let report = super::report::build(tree, &facts, &[], None, None) + notes + &memory.context();
    let mut tools = Tools::new(tree, &facts, &model, &[], memory);
    tools.consent = Some(consent);
    tools.allowed = Some(chat_specs().iter().map(|t| t.name).collect());
    let tools_ok = cfg.provider != Provider::ClaudeCode;
    let opening = format!("{report}\n\n# The user asks\n{first}");
    let open_chat = |with_tools: bool| -> Result<Box<dyn Chat>, String> {
        let specs = if with_tools { chat_specs() } else { Vec::new() };
        let system = if with_tools { CHAT.to_string() } else { format!("{CHAT}\nYou have no tools in this conversation: answer from the report.") };
        Ok(match cfg.provider {
            Provider::Off => return Err("AI is off".into()),
            Provider::Anthropic => {
                let key = config::key(Provider::Anthropic).or_else(|| std::env::var("ANTHROPIC_API_KEY").ok()).filter(|k| !k.trim().is_empty());
                Box::new(Anthropic::new(key.ok_or("No Anthropic API key. Add one in Options > AI setup.")?, &model, system, opening.clone(), &specs))
            }
            Provider::OpenAi => Box::new(OpenAi::new(&cfg.base_url, openai_key(&cfg.base_url)?, &model, system, opening.clone(), &specs)),
            Provider::ClaudeCode => {
                let exe = Detected::probe().claude_code.ok_or("Claude Code (claude) isn't installed or isn't on PATH.")?;
                Box::new(ClaudeCode::new(&exe, &model, &system, &opening))
            }
        })
    };
    let mut chat = match open_chat(tools_ok) {
        Ok(c) => c,
        Err(e) => return events(ChatEvent::Failed(e)),
    };
    let mut with_tools = tools_ok;
    let mut question_open = true;
    let mut first_question = true;
    loop {
        if !question_open {
            let Ok(q) = questions.recv() else { return };
            chat.say(&q);
            first_question = false;
        }
        question_open = false;
        let mut failed = false;
        events(ChatEvent::Progress(format!("Asking {model}...")));
        let mut turns = 0;
        let mut reply = String::new();
        loop {
            let t = match chat.turn() {
                Ok(t) => t,
                // only the first question can start again without tools: the opening holds it
                Err(e) if turns == 0 && first_question && with_tools && cfg.provider == Provider::OpenAi && e.to_lowercase().contains("tool") => {
                    with_tools = false;
                    match open_chat(false) {
                        Ok(c) => chat = c,
                        Err(e) => return events(ChatEvent::Failed(e)),
                    }
                    continue;
                }
                Err(e) => {
                    events(ChatEvent::Failed(e));
                    failed = true;
                    break;
                }
            };
            turns += 1;
            if !t.text.trim().is_empty() {
                reply = super::tools::clean_text_long(t.text.trim(), 6000);
            }
            if t.calls.is_empty() {
                break;
            }
            if turns >= 15 {
                // every call needs an answer, or the conversation can't go on: say stop, and
                // take one last turn for the reply
                chat.answer(t.calls.iter().map(|c| (c.id.clone(), "Turn limit reached: answer the user now with what you have.".to_string(), true)).collect());
                if let Ok(last) = chat.turn() {
                    if !last.text.trim().is_empty() {
                        reply = super::tools::clean_text_long(last.text.trim(), 6000);
                    }
                    if !last.calls.is_empty() {
                        chat.answer(last.calls.iter().map(|c| (c.id.clone(), "No more tools now.".to_string(), true)).collect());
                    }
                }
                break;
            }
            let mut results = Vec::new();
            for c in &t.calls {
                let what: String = c.input["path"].as_str().or(c.input["query"].as_str()).or(c.input["filter"].as_str()).unwrap_or("").chars().take(160).collect();
                events(ChatEvent::Progress(format!("{} {what}", c.name.replace('_', " "))));
                let (text, err) = match tools.call(&c.name, &c.input) {
                    Ok(t) => (t, false),
                    Err(e) => (e, true),
                };
                if let Some(f) = tools.filter.take() {
                    events(ChatEvent::Filter(f));
                }
                results.push((c.id.clone(), clip(text, 12_000), err));
            }
            chat.answer(results);
        }
        if !tools.prefs.is_empty() || !tools.labels.is_empty() {
            events(ChatEvent::Remember { prefs: std::mem::take(&mut tools.prefs), labels: std::mem::take(&mut tools.labels) });
        }
        // always an answer, so the app never waits for one that isn't coming
        if !failed {
            events(ChatEvent::Reply(if reply.is_empty() { "(No answer came back. Try asking again.)".into() } else { reply }));
        }
    }
}

#[derive(Debug, Default)]
pub struct AiRun {
    pub findings: Vec<Finding>,
    /// What the model claimed that didn't pass the checks, and why.
    pub rejected: Vec<(Finding, String)>,
    pub summary: String,
    pub model: String,
    /// Model turns taken.
    pub turns: usize,
    /// The model had tools (or answered once from the report).
    pub with_tools: bool,
    /// What it asked to remember, and the rules it proposed: for the app to add to its memory.
    pub prefs: Vec<Pref>,
    pub labels: Vec<Label>,
    pub proposals: Vec<Proposal>,
}

enum Mode {
    Tools,
    Single,
}

fn open(cfg: &AiConfig, model: &str, report: &str, mode: &Mode) -> Result<Box<dyn Chat>, String> {
    let tools = if matches!(mode, Mode::Tools) { specs() } else { Vec::new() };
    let (system, first) = match mode {
        Mode::Tools => (SYSTEM.to_string(), report.to_string()),
        Mode::Single => (format!("{SYSTEM}\n\n{SINGLE}"), report.to_string()),
    };
    Ok(match cfg.provider {
        Provider::Off => return Err("AI is off. Choose a provider in Options > AI setup.".into()),
        Provider::Anthropic => {
            let key = config::key(Provider::Anthropic).or_else(|| std::env::var("ANTHROPIC_API_KEY").ok()).filter(|k| !k.trim().is_empty());
            let key = key.ok_or("No Anthropic API key. Add one in Options > AI setup.")?;
            Box::new(Anthropic::new(key, model, system, first, &tools))
        }
        Provider::OpenAi => {
            if model.trim().is_empty() {
                return Err("Set a model name in Options > AI setup.".into());
            }
            Box::new(OpenAi::new(&cfg.base_url, openai_key(&cfg.base_url)?, model, system, first, &tools))
        }
        Provider::ClaudeCode => {
            let exe = Detected::probe().claude_code.ok_or("Claude Code (claude) isn't installed or isn't on PATH.")?;
            Box::new(ClaudeCode::new(&exe, model, &system, &first))
        }
    })
}

/// Claude Code drives itself through Sprawl's MCP server (read-only tools on a snapshot of this
/// scan). What it reports is recorded by that server, then every claim is replayed here through
/// the same checks as any other model's: nothing it wrote is taken as it is.
#[allow(clippy::too_many_arguments)]
fn run_driven(tree: &Tree, facts: &Facts, model: &str, report: &str, already: &[Finding], memory: &Memory, exe: &std::path::Path, progress: &mut dyn FnMut(String)) -> Result<AiRun, String> {
    let claude = Detected::probe().claude_code.ok_or("Claude Code (claude) isn't installed or isn't on PATH.")?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = super::advisor_dir().ok_or("APPDATA is not set")?.join("claude-code-runs").join(format!("{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        progress("Handing the scan to Claude Code...".into());
        let tree_file = dir.join("scan.dtsn");
        let mut w = std::io::BufWriter::new(std::fs::File::create(&tree_file).map_err(|e| e.to_string())?);
        crate::snapshot::write(tree, std::time::SystemTime::now(), 1, &mut w).map_err(|e| e.to_string())?;
        drop(w);
        let out_file = dir.join("out.jsonl");
        std::fs::write(&out_file, "").map_err(|e| e.to_string())?;
        let cfg_file = dir.join("mcp.json");
        std::fs::write(&cfg_file, crate::mcp::driven_config(exe, &tree_file, &out_file, model)).map_err(|e| e.to_string())?;
        let first = format!("{report}\n\nUse Sprawl's tools (mcp__sprawl__...) to look closer, report each opportunity with report_finding, and end with finish.");
        let mut cc = ClaudeCode::new(&claude, model, SYSTEM, &first).with_mcp(&cfg_file);
        progress(format!("Claude Code ({model}) is reading the scan..."));
        // show what it does as it goes: the server notes every tool call
        let started = std::time::Instant::now();
        let mut seen = 0usize;
        let mut last = String::new();
        let answer = cc.turn_with(&mut || {
            let text = std::fs::read_to_string(&out_file).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            for l in lines.iter().skip(seen) {
                if let Some(p) = serde_json::from_str::<serde_json::Value>(l).ok().and_then(|v| v["progress"].as_str().map(str::to_string)) {
                    last = p;
                }
            }
            seen = lines.len();
            let secs = started.elapsed().as_secs();
            progress(if last.is_empty() { format!("Claude Code ({model}) is reading the scan... {secs} s") } else { format!("Claude Code: {last} ({secs} s)") });
        })?;
        // replay what it reported through this process's own checks
        let mut tools = Tools::new(tree, facts, model, already, memory);
        let recorded = std::fs::read_to_string(&out_file).unwrap_or_default();
        let mut turns = 1;
        for line in recorded.lines().take(500) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let name = v["tool"].as_str().unwrap_or("");
            if matches!(name, "report_finding" | "remember" | "propose_rule" | "finish") {
                let _ = tools.call(name, &v["input"]);
                turns += 1;
            }
        }
        let mut run = AiRun { model: format!("{model} (Claude Code)"), with_tools: true, turns, ..AiRun::default() };
        run.summary = super::tools::clean_text_long(&tools.summary.take().unwrap_or(answer.text), 2000);
        run.findings = std::mem::take(&mut tools.accepted);
        run.findings.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        run.rejected = std::mem::take(&mut tools.rejected);
        run.prefs = std::mem::take(&mut tools.prefs);
        run.labels = std::mem::take(&mut tools.labels);
        run.proposals = std::mem::take(&mut tools.proposals);
        Ok(run)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The key for an OpenAI-compatible endpoint. `OPENAI_API_KEY` from the environment goes only to
/// OpenAI itself; no key ever goes over plain http to another machine.
pub(crate) fn openai_key(base_url: &str) -> Result<Option<String>, String> {
    let url = if base_url.trim().is_empty() { "https://api.openai.com/v1" } else { base_url.trim() };
    let (secure, host, _, _) = super::http::split_url(&format!("{}/x", url.trim_end_matches('/')))?;
    let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1");
    let key = config::key(Provider::OpenAi).filter(|k| !k.trim().is_empty()).or_else(|| (host == "api.openai.com").then(|| std::env::var("OPENAI_API_KEY").ok()).flatten());
    if key.is_some() && !secure && !local {
        return Err(format!("{host} isn't https: an API key would go over the network unencrypted. Use https, or remove the key."));
    }
    Ok(key)
}

/// The JSON object in a model's single answer (inside a ```json fence or bare).
pub(crate) fn json_in(text: &str) -> Option<Value> {
    let body = text.split("```json").nth(1).and_then(|t| t.split("```").next()).unwrap_or(text);
    let (a, b) = (body.find('{')?, body.rfind('}')?);
    serde_json::from_str(body.get(a..=b)?).ok()
}

/// Analyse `tree` with the configured model. `already` are the rule findings (not repeated);
/// `goal` is the space the user needs, if they said. `progress` gets short status lines.
/// `notes` is added to the report (growth over time, from the app).
pub fn run(tree: &Tree, cfg: &AiConfig, already: &[Finding], hidden: Option<&HiddenSpace>, goal: Option<u64>, memory: &Memory, notes: &str, progress: &mut dyn FnMut(String)) -> Result<AiRun, String> {
    if cfg.provider == Provider::Off {
        return Err("AI is off. Choose a provider in Options > AI setup.".into());
    }
    if tree.method == sprawl_core::ScanMethod::Csv {
        return Err("This view came from a CSV file. Scan the folder to get advice.".into());
    }
    let model = if cfg.model.trim().is_empty() { cfg.provider.default_model().to_string() } else { cfg.model.trim().to_string() };
    progress("Gathering what Windows knows about your apps...".into());
    let mut facts = Facts::gather();
    facts.attach_usage(tree);
    let report = super::report::build(tree, &facts, already, hidden, goal) + notes + &memory.context();
    let mut tools = Tools::new(tree, &facts, &model, already, memory);
    tools.allowed = Some(specs().iter().map(|t| t.name).collect());
    let mut run = AiRun { model: model.clone(), ..AiRun::default() };

    // Claude Code with Sprawl's own tools, when the app can serve them
    if cfg.provider == Provider::ClaudeCode {
        if let Some(exe) = super::mcp_exe() {
            return run_driven(tree, &facts, &model, &report, already, memory, exe, progress);
        }
    }
    let mut mode = if cfg.provider == Provider::ClaudeCode { Mode::Single } else { Mode::Tools };
    let mut chat = open(cfg, &model, &report, &mode)?;
    progress(format!("Asking {model}..."));
    let mut first = match chat.turn() {
        Ok(t) => t,
        // a local model without tool calling: ask once instead
        Err(e) if matches!(mode, Mode::Tools) && cfg.provider == Provider::OpenAi && e.to_lowercase().contains("tool") => {
            progress("This model can't use tools; asking it once from the report...".into());
            mode = Mode::Single;
            chat = open(cfg, &model, &report, &mode)?;
            chat.turn()?
        }
        Err(e) => return Err(e),
    };
    run.turns = 1;

    match mode {
        Mode::Single => {
            let v = json_in(&first.text).ok_or_else(|| format!("The model's answer had no JSON in it: {}", super::short(&first.text)))?;
            for f in v["findings"].as_array().into_iter().flatten() {
                let _ = tools.report_finding(f);
            }
            run.summary = v["summary"].as_str().unwrap_or("").to_string();
        }
        Mode::Tools => {
            run.with_tools = true;
            loop {
                if !first.text.trim().is_empty() {
                    progress(super::short(first.text.trim()));
                }
                // a long server-side step, or cut off by the length limit: let it carry on
                if first.calls.is_empty() && (first.paused || first.truncated) && run.turns < 32 {
                    if first.truncated {
                        chat.say("You were cut off. Carry on, briefly.");
                    }
                    first = chat.turn()?;
                    run.turns += 1;
                    continue;
                }
                if first.calls.is_empty() {
                    if tools.summary.is_none() && !first.text.trim().is_empty() {
                        tools.summary = Some(first.text.trim().to_string());
                    }
                    break;
                }
                let mut results = Vec::new();
                for c in &first.calls {
                    if let Some(p) = c.input["path"].as_str() {
                        progress(format!("{}: {}", c.name.replace('_', " "), p.chars().take(160).collect::<String>()));
                    }
                    let (text, err) = match tools.call(&c.name, &c.input) {
                        Ok(t) => (t, false),
                        Err(e) => (e, true),
                    };
                    results.push((c.id.clone(), clip(text, 12_000), err));
                }
                chat.answer(results);
                if tools.summary.is_some() {
                    break;
                }
                if run.turns == 28 {
                    chat.say("Please report anything else you are sure of now, then call finish.");
                }
                if run.turns >= 32 {
                    break;
                }
                first = chat.turn()?;
                run.turns += 1;
            }
        }
    }
    run.summary = super::tools::clean_text_long(&tools.summary.take().unwrap_or(run.summary), 2000);
    run.findings = std::mem::take(&mut tools.accepted);
    run.rejected = std::mem::take(&mut tools.rejected);
    run.prefs = std::mem::take(&mut tools.prefs);
    run.labels = std::mem::take(&mut tools.labels);
    run.proposals = std::mem::take(&mut tools.proposals);
    run.findings.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_json_in_an_answer() {
        assert_eq!(json_in("Here:\n```json\n{\"summary\": \"s\", \"findings\": []}\n```").unwrap()["summary"], "s");
        assert_eq!(json_in("{\"summary\": \"bare\"}").unwrap()["summary"], "bare");
        assert!(json_in("no json").is_none());
    }

    #[test]
    fn off_is_refused_without_sending_anything() {
        let tree = sprawl_core::scan(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").as_path(), |_| {}).unwrap();
        let err = run(&tree, &AiConfig::default(), &[], None, None, &Memory::default(), "", &mut |_| {}).unwrap_err();
        assert!(err.contains("AI is off"));
    }
}
