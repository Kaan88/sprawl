//! The loop: the model reads the report, calls tools, reports findings, and finishes with a
//! summary. Providers without tool calling (Claude Code here, small local models) get the same
//! report once and answer with JSON, which goes through the same checks.

use serde_json::Value;
use sprawl_core::Tree;

use super::anthropic::Anthropic;
use super::claude_code::ClaudeCode;
use super::openai::OpenAi;
use super::tools::{specs, Tools};
use super::{clip, Chat};
use crate::config::{self, AiConfig, Detected, Provider};
use crate::facts::Facts;
use crate::finding::Finding;
use crate::hidden::HiddenSpace;

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
            let key = config::key(Provider::OpenAi).or_else(|| std::env::var("OPENAI_API_KEY").ok());
            Box::new(OpenAi::new(&cfg.base_url, key, model, system, first, &tools))
        }
        Provider::ClaudeCode => {
            let exe = Detected::probe().claude_code.ok_or("Claude Code (claude) isn't installed or isn't on PATH.")?;
            Box::new(ClaudeCode::new(&exe, model, &system, &first))
        }
    })
}

/// The JSON object in a model's single answer (inside a ```json fence or bare).
pub(crate) fn json_in(text: &str) -> Option<Value> {
    let body = text.split("```json").nth(1).and_then(|t| t.split("```").next()).unwrap_or(text);
    let (a, b) = (body.find('{')?, body.rfind('}')?);
    serde_json::from_str(body.get(a..=b)?).ok()
}

/// Analyse `tree` with the configured model. `already` are the rule findings (not repeated);
/// `goal` is the space the user needs, if they said. `progress` gets short status lines.
pub fn run(tree: &Tree, cfg: &AiConfig, already: &[Finding], hidden: Option<&HiddenSpace>, goal: Option<u64>, progress: &mut dyn FnMut(String)) -> Result<AiRun, String> {
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
    let report = super::report::build(tree, &facts, already, hidden, goal);
    let mut tools = Tools::new(tree, &facts, &model, already);
    let mut run = AiRun { model: model.clone(), ..AiRun::default() };

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
                if first.calls.is_empty() {
                    if tools.summary.is_none() && !first.text.trim().is_empty() {
                        tools.summary = Some(first.text.trim().to_string());
                    }
                    break;
                }
                let mut results = Vec::new();
                for c in &first.calls {
                    if let Some(p) = c.input["path"].as_str() {
                        progress(format!("{}: {p}", c.name.replace('_', " ")));
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
    run.summary = tools.summary.take().unwrap_or(run.summary);
    run.findings = std::mem::take(&mut tools.accepted);
    run.rejected = std::mem::take(&mut tools.rejected);
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
        let err = run(&tree, &AiConfig::default(), &[], None, None, &mut |_| {}).unwrap_err();
        assert!(err.contains("AI is off"));
    }
}
