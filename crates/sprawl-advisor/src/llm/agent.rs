//! The loop: the model reads the report, calls tools, reports findings, and finishes with a
//! summary. Providers without tool calling (Claude Code here, small local models) get the same
//! report once and answer with JSON, which goes through the same checks.

use serde_json::Value;
use sprawl_core::Tree;

use super::anthropic::Anthropic;
use super::claude_code::ClaudeCode;
use super::openai::OpenAi;
use super::tools::{chat_specs, specs, Tools};
use super::{clip, Chat, Metered, Stop, Usage, STOPPED};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use crate::config::{self, AiConfig, Detected, Provider};
use crate::facts::Facts;
use crate::finding::Finding;
use crate::hidden::HiddenSpace;
use crate::memory::{CleanerProposal, Label, Memory, Pref, Proposal};

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
{\"summary\": \"Markdown, short: one sentence, then at most 6 bullets under the bold headings **Quick wins**, **Check first** and **Leave alone** (leave out empty ones). Start each bullet with the size, like '- 7.3 GB: Android build output in E:\\x, regenerates'. No long paragraphs.\", \"findings\": [{\"path\": \"C:\\\\...\", \"bytes\": 123, \"title\": \"...\", \"risk\": \"green|amber|orange|red\", \
\"remedy\": \"delete|uninstall|move|info\", \"evidence\": [\"...\"], \"confidence\": 0.8}]}
Only report paths that appear in the report, with the exact bytes shown there. Risk: green = comes back by itself, amber = costs something, \
orange = the user's own files, red = never touch. Sprawl writes any command itself.";

const CHAT: &str = "You answer questions about the user's disk inside Sprawl, a disk analyser for Windows. \
You get a compact report of the scan and tools to look closer; use them before you answer, and answer briefly and concretely, with sizes. \
Answer in short Markdown: the answer in one line, then bullets that start with the size. To show something on the user's map, call set_filter. You can't delete anything: to win back space, point the user to the Advisor's Findings list \
(tick an item, then Clean up; or 'tick enough for N GB'), or to 'Ask Claude' for findings the rules missed. If you learn what a folder is, or the user states a preference, call remember.";

/// Something the chat has for the app.
#[derive(Debug)]
pub enum ChatEvent {
    Progress(String),
    /// The answer so far, while it is written.
    Partial(String),
    Reply(String),
    /// Show this filter on the map (empty clears it).
    Filter(String),
    Remember { prefs: Vec<Pref>, labels: Vec<Label>, cleaners: Vec<CleanerProposal> },
    Failed(String),
}

/// "Ask your disk": answer each question from `questions` until the channel closes. Runs on its
/// own thread with its own copy of the scan, so filtering the map doesn't end it.
/// `stop` ends the question being answered (it is cleared again for the next one).
#[allow(clippy::too_many_arguments)]
/// `partial` gets each answer's text so far, while it is written.
pub fn chat(tree: &Tree, cfg: &AiConfig, memory: &Memory, notes: &str, questions: std::sync::mpsc::Receiver<String>, events: &mut dyn FnMut(ChatEvent), consent: &dyn Fn(&str) -> bool, stop: Stop, partial: Rc<dyn Fn(&str)>) {
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
        let c: Box<dyn Chat> = match cfg.provider {
            Provider::Off => return Err("AI is off".into()),
            Provider::Anthropic => {
                let key = config::key(Provider::Anthropic).or_else(|| std::env::var("ANTHROPIC_API_KEY").ok()).filter(|k| !k.trim().is_empty());
                Box::new(Anthropic::new(key.ok_or("No Anthropic API key. Add one in Options > AI setup.")?, &model, system, opening.clone(), &specs))
            }
            Provider::OpenAi => Box::new(OpenAi::new(&cfg.base_url, openai_key(&cfg.base_url)?, &model, system, opening.clone(), &specs)),
            Provider::ClaudeCode => {
                let exe = Detected::probe().claude_code.ok_or("Claude Code (claude) isn't installed or isn't on PATH.")?;
                Box::new(ClaudeCode::new(&exe, &model, &system, &opening).with_stop(stop.clone()))
            }
        };
        Ok(Box::new(Metered::new(c, stop.clone(), Rc::new(RefCell::new(Usage::default())))) as Box<dyn Chat>)
    };
    let mut chat = match open_chat(tools_ok) {
        Ok(c) => c,
        Err(e) => return events(ChatEvent::Failed(e)),
    };
    // the answer as it is written, straight to the app
    let stream = |c: &mut Box<dyn Chat>| {
        let p = partial.clone();
        // cleaned like the finished reply: no control or direction-changing characters
        c.stream_to(Some(Box::new(move |t: &str| p(&super::tools::clean_text_long(t, 6000)))));
    };
    stream(&mut chat);
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
        // a Stop for an earlier question doesn't carry over
        stop.store(false, Ordering::Relaxed);
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
                        Ok(c) => {
                            chat = c;
                            stream(&mut chat);
                        }
                        Err(e) => return events(ChatEvent::Failed(e)),
                    }
                    continue;
                }
                Err(e) if e == STOPPED => {
                    events(ChatEvent::Reply(if reply.is_empty() { "(Stopped.)".into() } else { format!("{reply}\n\n(Stopped.)") }));
                    failed = true;
                    break;
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
        let mut cleaners = std::mem::take(&mut tools.cleaner_proposals);
        if !cleaners.is_empty() && !failed {
            events(ChatEvent::Progress("Getting a second opinion on the proposed cleaner...".into()));
            let usage = Rc::new(RefCell::new(Usage::default()));
            for d in check_cleaners(cfg, &model, &mut cleaners, &stop, &usage) {
                reply.push_str(&format!("\n\n({d})"));
            }
        }
        if !tools.prefs.is_empty() || !tools.labels.is_empty() || !cleaners.is_empty() {
            events(ChatEvent::Remember { prefs: std::mem::take(&mut tools.prefs), labels: std::mem::take(&mut tools.labels), cleaners });
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
    /// What the run used (all its calls, the second opinion included).
    pub usage: Usage,
    /// The user pressed Stop: what was found until then is kept.
    pub stopped: bool,
    /// What it asked to remember, and the rules it proposed: for the app to add to its memory.
    pub prefs: Vec<Pref>,
    pub labels: Vec<Label>,
    pub proposals: Vec<Proposal>,
    pub cleaner_proposals: Vec<CleanerProposal>,
}

enum Mode {
    Tools,
    Single,
}

fn open(cfg: &AiConfig, model: &str, report: &str, mode: &Mode, stop: &Stop) -> Result<Box<dyn Chat>, String> {
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
            Box::new(ClaudeCode::new(&exe, model, &system, &first).with_stop(stop.clone()))
        }
    })
}

/// Claude Code drives itself through Sprawl's MCP server (read-only tools on a snapshot of this
/// scan). What it reports is recorded by that server, then every claim is replayed here through
/// the same checks as any other model's: nothing it wrote is taken as it is.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn run_driven(tree: &Tree, facts: &Facts, model: &str, report: &str, already: &[Finding], memory: &Memory, exe: &std::path::Path, stop: &Stop, progress: &mut dyn FnMut(String)) -> Result<AiRun, String> {
    let mut run = run_driven_inner(tree, facts, model, report, already, memory, exe, stop, progress)?;
    let cfg = AiConfig { provider: Provider::ClaudeCode, model: model.to_string(), base_url: String::new() };
    second_opinion(&cfg, model, &mut run, stop, progress);
    run_cleaner_check(&cfg, model, &mut run, stop, progress);
    Ok(run)
}

#[allow(clippy::too_many_arguments)]
fn run_driven_inner(tree: &Tree, facts: &Facts, model: &str, report: &str, already: &[Finding], memory: &Memory, exe: &std::path::Path, stop: &Stop, progress: &mut dyn FnMut(String)) -> Result<AiRun, String> {
    let claude = Detected::probe().claude_code.ok_or("Claude Code (claude) isn't installed or isn't on PATH.")?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    // in Temp (machine-local, never roamed): the snapshot lists every path on the disk
    let runs = std::env::temp_dir().join("sprawl-claude-runs");
    // what an earlier run left (the app closed or crashed during it) goes after a day
    if let Ok(rd) = std::fs::read_dir(&runs) {
        for e in rd.flatten() {
            if e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 86_400) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    let dir = runs.join(format!("{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    /// Removes the run's folder however the run ends, a panic included.
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
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
        let mut cc = ClaudeCode::new(&claude, model, SYSTEM, &first).with_mcp(&cfg_file).with_stop(stop.clone());
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
            let _ = started;
            progress(if last.is_empty() { format!("Claude Code ({model}) is reading the scan") } else { format!("Claude Code: {last}") });
        });
        // replay what it reported through this process's own checks
        let mut tools = Tools::new(tree, facts, model, already, memory);
        let recorded = std::fs::read_to_string(&out_file).unwrap_or_default();
        let mut turns = 1;
        // the reports, not the progress lines, count toward the cap
        let reports: Vec<serde_json::Value> = recorded
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| matches!(v["tool"].as_str(), Some("report_finding" | "remember" | "propose_rule" | "propose_cleaner" | "finish")))
            .take(500)
            .collect();
        turns += recorded.lines().filter(|l| l.contains("\"progress\"")).count();
        // Claude Code ending with an error, or stopped, still keeps what it reported before
        let stopped = matches!(&answer, Err(e) if e == STOPPED);
        let answer = match answer {
            Ok(a) => a,
            Err(e) if reports.is_empty() => return Err(e),
            Err(_) => super::Turn::default(),
        };
        for v in &reports {
            let _ = tools.call(v["tool"].as_str().unwrap_or(""), &v["input"]);
        }
        let mut run = AiRun { model: format!("{model} (Claude Code)"), with_tools: true, turns, usage: answer.usage, stopped, ..AiRun::default() };
        run.summary = super::tools::clean_text_long(&tools.summary.take().unwrap_or(answer.text), 2000);
        run.findings = std::mem::take(&mut tools.accepted);
        run.findings.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        run.rejected = std::mem::take(&mut tools.rejected);
        run.prefs = std::mem::take(&mut tools.prefs);
        run.labels = std::mem::take(&mut tools.labels);
        run.proposals = std::mem::take(&mut tools.proposals);
        run.cleaner_proposals = std::mem::take(&mut tools.cleaner_proposals);
        Ok(run)
    })();
    result
}

const SECOND: &str = "You are a careful second reviewer for a disk-space advisor. Another model suggested the findings below. \
For each, judge only from what is written. Orange means the user decides, so an orange finding about their own files may stay if it says so honestly. \
Drop a finding only when it is wrong: the evidence doesn't support it, the risk is understated (a program still in use, something that doesn't come back \
but is called green), or it would break something. Answer with one JSON object and nothing else, repeating each path exactly: \
{\"verdicts\": [{\"index\": 0, \"path\": \"C:\\\\...\", \"keep\": true, \"why\": \"...\"}]}";

/// A second opinion: a fresh conversation, no tools, judges each finding the first one made.
/// Findings it drops go to `run.rejected` with its reason; if it can't answer, all are kept
/// (they already passed Sprawl's own checks) and a note says so.
fn second_opinion(cfg: &AiConfig, model: &str, run: &mut AiRun, stop: &Stop, progress: &mut dyn FnMut(String)) {
    if run.findings.is_empty() || run.stopped || stop.load(Ordering::Relaxed) {
        return;
    }
    progress("Getting a second opinion on the findings...".into());
    let mut list = String::new();
    for (i, f) in run.findings.iter().enumerate() {
        list.push_str(&format!("{i}. {} ({:?}, {:?}, {} bytes): {}\n   {}\n", f.title, f.risk, f.remedy, f.bytes, f.path.display(), f.evidence.join(" | ")));
    }
    let usage = Rc::new(RefCell::new(Usage::default()));
    let verdicts = one_shot(cfg, model, SECOND, list, stop, &usage).and_then(|t| json_in(&t)).and_then(|v| v["verdicts"].as_array().cloned());
    run.usage.add(*usage.borrow());
    let Some(verdicts) = verdicts else {
        run.summary.push_str("\n\n(The second opinion didn't answer; the findings passed Sprawl's own checks.)");
        return;
    };
    let mut drop: Vec<(usize, String)> = verdicts
        .iter()
        .filter(|v| v["keep"].as_bool() == Some(false))
        // the index and the path must agree, so an off-by-one answer drops nothing
        .filter(|v| v["index"].as_u64().and_then(|i| run.findings.get(i as usize)).is_some_and(|f| v["path"].as_str().is_some_and(|p| p.eq_ignore_ascii_case(&f.path.display().to_string()))))
        .filter_map(|v| Some((v["index"].as_u64()? as usize, super::tools::clean_text_long(v["why"].as_str().unwrap_or("no reason given"), 300))))
        .filter(|(i, _)| *i < run.findings.len())
        .collect();
    drop.sort_by_key(|d| std::cmp::Reverse(d.0));
    drop.dedup_by_key(|d| d.0);
    for (i, why) in drop {
        let f = run.findings.remove(i);
        run.rejected.push((f, format!("second opinion: {why}")));
    }
}

/// One question to a fresh conversation without tools; its answer, or None.
fn one_shot(cfg: &AiConfig, model: &str, system: &str, text: String, stop: &Stop, usage: &Rc<RefCell<Usage>>) -> Option<String> {
    let chat: Result<Box<dyn Chat>, String> = match cfg.provider {
        Provider::Anthropic => config::key(Provider::Anthropic)
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .filter(|k| !k.trim().is_empty())
            .map(|k| Box::new(Anthropic::new(k, model, system.into(), text.clone(), &[])) as Box<dyn Chat>)
            .ok_or_else(|| "no key".to_string()),
        Provider::OpenAi => openai_key(&cfg.base_url).map(|k| Box::new(OpenAi::new(&cfg.base_url, k, model, system.into(), text.clone(), &[])) as Box<dyn Chat>),
        Provider::ClaudeCode => Detected::probe().claude_code.map(|exe| Box::new(ClaudeCode::new(&exe, model, system, &text).with_stop(stop.clone())) as Box<dyn Chat>).ok_or_else(|| "no claude".to_string()),
        Provider::Off => Err("off".into()),
    };
    chat.ok().map(|c| Metered::new(c, stop.clone(), usage.clone())).and_then(|mut c| c.turn().ok()).map(|t| t.text)
}

const CLEANER_CHECK: &str = "You are a careful second reviewer for a disk cleaner. Another model proposed the cleaners below: for one application each, \
folders that can be emptied. Judge each from what is written and what you know of the application. Keep it only when every part holds data the \
application makes again by itself (caches, logs, crash dumps, temporary files), the paths are narrow, and no part holds the user's own files, \
settings, saved games, sign-ins, downloads or history. Answer with one JSON object and nothing else, repeating each id exactly: \
{\"verdicts\": [{\"index\": 0, \"id\": \"local.x\", \"keep\": true, \"why\": \"...\"}]}";

/// A second opinion on proposed cleaners, from a fresh conversation. Those it drops are taken
/// out and returned as lines for the user; the rest carry its note. If it can't answer, all are
/// kept with a note saying so (they passed Sprawl's own checks, and the user approves each).
pub(crate) fn check_cleaners(cfg: &AiConfig, model: &str, list: &mut Vec<CleanerProposal>, stop: &Stop, usage: &Rc<RefCell<Usage>>) -> Vec<String> {
    if list.is_empty() {
        return Vec::new();
    }
    let mut text = String::new();
    for (i, p) in list.iter().enumerate() {
        text += &format!("{i}. {} ({})\n", p.cleaner.name, p.cleaner.id);
        for part in &p.cleaner.parts {
            text += &format!("   - {} [{:?}]: {}\n     {}\n", part.title, part.risk, part.paths.join(" | "), part.why);
        }
    }
    let verdicts = one_shot(cfg, model, CLEANER_CHECK, text, stop, usage).and_then(|t| json_in(&t)).and_then(|v| v["verdicts"].as_array().cloned());
    let Some(verdicts) = verdicts else {
        for p in list.iter_mut() {
            p.note = "The second opinion didn't answer; it passed Sprawl's own checks.".into();
        }
        return Vec::new();
    };
    let mut dropped = Vec::new();
    let mut keep = Vec::new();
    for (i, mut p) in std::mem::take(list).into_iter().enumerate() {
        // the index and the id must agree, so an off-by-one answer judges nothing
        let v = verdicts.iter().find(|v| v["index"].as_u64() == Some(i as u64) && v["id"].as_str() == Some(p.cleaner.id.as_str()));
        let why = v.and_then(|v| v["why"].as_str()).map(|w| super::tools::clean_text_long(w, 300)).unwrap_or_default();
        match v.and_then(|v| v["keep"].as_bool()) {
            Some(false) => dropped.push(format!("The second opinion turned down the cleaner for {}: {why}", p.cleaner.name)),
            Some(true) => {
                p.note = format!("Second opinion: {why}");
                keep.push(p);
            }
            None => {
                p.note = "The second opinion didn't judge this one.".into();
                keep.push(p);
            }
        }
    }
    *list = keep;
    dropped
}

/// [`check_cleaners`] for an analysis run: what it drops goes into the summary.
fn run_cleaner_check(cfg: &AiConfig, model: &str, run: &mut AiRun, stop: &Stop, progress: &mut dyn FnMut(String)) {
    if run.cleaner_proposals.is_empty() || run.stopped || stop.load(Ordering::Relaxed) {
        return;
    }
    progress("Getting a second opinion on the proposed cleaners...".into());
    let usage = Rc::new(RefCell::new(Usage::default()));
    for d in check_cleaners(cfg, model, &mut run.cleaner_proposals, stop, &usage) {
        run.summary.push_str(&format!("\n\n({d})"));
    }
    run.usage.add(*usage.borrow());
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
/// `goal` is the space the user needs, if they said; `notes` is added to the report (growth);
/// `stop` ends the run between steps, keeping what was found. `progress` gets status lines.
#[allow(clippy::too_many_arguments)]
pub fn run(tree: &Tree, cfg: &AiConfig, already: &[Finding], hidden: Option<&HiddenSpace>, goal: Option<u64>, memory: &Memory, notes: &str, stop: Stop, progress: &mut dyn FnMut(String)) -> Result<AiRun, String> {
    if cfg.provider == Provider::Off {
        return Err("AI is off. Choose a provider in Options > AI setup.".into());
    }
    if tree.method == sprawl_core::ScanMethod::Csv {
        return Err("This view came from a CSV file. Scan the folder to get advice.".into());
    }
    let model = if cfg.model.trim().is_empty() { cfg.provider.default_model().to_string() } else { cfg.model.trim().to_string() };
    progress("Gathering what Windows knows about your apps".into());
    let mut facts = Facts::gather();
    facts.attach_usage(tree);
    let report = super::report::build(tree, &facts, already, hidden, goal) + notes + &memory.context();
    let mut tools = Tools::new(tree, &facts, &model, already, memory);
    tools.allowed = Some(specs().iter().map(|t| t.name).collect());
    let mut run = AiRun { model: model.clone(), ..AiRun::default() };

    // Claude Code with Sprawl's own tools, when the app can serve them
    if cfg.provider == Provider::ClaudeCode {
        if let Some(exe) = super::mcp_exe() {
            return run_driven(tree, &facts, &model, &report, already, memory, exe, &stop, progress);
        }
    }
    let usage = Rc::new(RefCell::new(Usage::default()));
    let metered = |c: Box<dyn Chat>| Box::new(Metered::new(c, stop.clone(), usage.clone())) as Box<dyn Chat>;
    let mut mode = if cfg.provider == Provider::ClaudeCode { Mode::Single } else { Mode::Tools };
    let mut chat = metered(open(cfg, &model, &report, &mode, &stop)?);
    progress(format!("Waiting for {model}'s first step"));
    let mut first = match chat.turn() {
        Ok(t) => t,
        // a local model without tool calling: ask once instead
        Err(e) if matches!(mode, Mode::Tools) && cfg.provider == Provider::OpenAi && e.to_lowercase().contains("tool") => {
            progress("This model can't use tools; asking it once from the report".into());
            mode = Mode::Single;
            chat = metered(open(cfg, &model, &report, &mode, &stop)?);
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
            // the next turn, or the end of the loop when the user stopped it
            macro_rules! next {
                () => {
                    match chat.turn() {
                        Ok(t) => t,
                        Err(e) if e == STOPPED => {
                            run.stopped = true;
                            break;
                        }
                        Err(e) => return Err(e),
                    }
                };
            }
            loop {
                // the model's own words between steps aren't an answer: only its steps are shown
                // a long server-side step, or cut off by the length limit: let it carry on
                if first.calls.is_empty() && (first.paused || first.truncated) && run.turns < 32 {
                    if first.truncated {
                        chat.say("You were cut off. Carry on, briefly.");
                    }
                    first = next!();
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
                        progress(format!("{}: {}", c.name.replace('_', " "), crate::facts::clean_text(p)));
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
                if stop.load(Ordering::Relaxed) {
                    run.stopped = true;
                    break;
                }
                if run.turns == 28 {
                    chat.say("Please report anything else you are sure of now, then call finish.");
                }
                if run.turns >= 32 {
                    break;
                }
                first = next!();
                run.turns += 1;
            }
        }
    }
    run.summary = super::tools::clean_text_long(&tools.summary.take().unwrap_or(run.summary), 2000);
    if run.stopped && run.summary.is_empty() {
        run.summary = "Stopped before a summary. What it found until then is below.".into();
    }
    run.findings = std::mem::take(&mut tools.accepted);
    run.rejected = std::mem::take(&mut tools.rejected);
    run.prefs = std::mem::take(&mut tools.prefs);
    run.labels = std::mem::take(&mut tools.labels);
    run.proposals = std::mem::take(&mut tools.proposals);
    run.cleaner_proposals = std::mem::take(&mut tools.cleaner_proposals);
    run.findings.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    run.usage = *usage.borrow();
    second_opinion(cfg, &model, &mut run, &stop, progress);
    run_cleaner_check(cfg, &model, &mut run, &stop, progress);
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
        let err = run(&tree, &AiConfig::default(), &[], None, None, &Memory::default(), "", Stop::default(), &mut |_| {}).unwrap_err();
        assert!(err.contains("AI is off"));
    }
}
