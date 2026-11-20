//! Reasoning effort: the levels each model really offers, and what is sent for it.
//!
//! Fetched where the provider says: Anthropic's `/v1/models` (`capabilities.effort`, read when
//! the Model list is fetched) and Codex's own model cache (`supported_reasoning_levels`). Where
//! nothing can be fetched, the tables below, from each provider's docs. A model that isn't
//! known (not fetched, not in a table) offers no levels: nothing is sent for it and the
//! provider's own default applies. A saved level the model doesn't offer is never sent.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::config::{AiConfig, Provider};

/// What the Messages API takes in `output_config.effort`, per model, from the Effort page of
/// the Claude docs (https://platform.claude.com/docs/en/build-with-claude/effort, read
/// 2026-11-19): every model it lists takes low, medium and high; `xhigh` and `max` only where
/// the page says. Models it doesn't list (Haiku 4.5, Sonnet 4.5 and older) take no effort.
/// Used until `/v1/models` has been read with a key, and for Claude Code (which has no list).
const ANTHROPIC: [(&str, &[&str]); 17] = [
    ("claude-fable-5-2", &["low", "medium", "high", "xhigh", "max"]),
    ("claude-mythos-5-2", &["low", "medium", "high", "xhigh", "max"]),
    ("claude-fable-5-1", &["low", "medium", "high", "xhigh", "max"]),
    ("claude-mythos-5-1", &["low", "medium", "high", "xhigh", "max"]),
    ("claude-fable-5", &["low", "medium", "high", "xhigh", "max"]),
    ("claude-mythos-5", &["low", "medium", "high", "xhigh", "max"]),
    // max, but not xhigh
    ("claude-mythos-preview", &["low", "medium", "high", "max"]),
    ("claude-opus-5-5", &["low", "medium", "high", "xhigh", "max"]),
    ("claude-opus-5", &["low", "medium", "high", "xhigh", "max"]),
    ("claude-sonnet-5-5", &["low", "medium", "high", "xhigh", "max"]),
    ("claude-sonnet-5", &["low", "medium", "high", "xhigh", "max"]),
    ("claude-haiku-5-5", &["low", "medium", "high", "xhigh", "max"]),
    ("claude-opus-4-8", &["low", "medium", "high", "xhigh", "max"]),
    ("claude-opus-4-7", &["low", "medium", "high", "xhigh", "max"]),
    ("claude-opus-4-6", &["low", "medium", "high", "max"]),
    ("claude-sonnet-4-6", &["low", "medium", "high", "max"]),
    ("claude-opus-4-5", &["low", "medium", "high"]),
];

/// What Claude Code's `--effort` accepts (`claude --help`, Claude Code 2.1.328: "low, medium,
/// high, xhigh, max"). A model gets the levels both it and the CLI take.
const CLAUDE_CODE_CLI: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// What OpenAI's own API takes in `reasoning_effort`, per model, from its models page
/// (https://developers.openai.com/api/docs/models, read 2026-11-19, "Reasoning levels"; the
/// reasoning guide, https://developers.openai.com/api/docs/guides/reasoning, says the levels
/// depend on the model). Only for api.openai.com: another server with the same model name
/// (OpenRouter, a local one) may take other values, so it gets no levels.
const OPENAI: [(&str, &[&str]); 4] = [
    ("gpt-6.2-astra", &["low", "medium", "high", "xhigh", "max"]),
    ("gpt-6.2-sol", &["low", "medium", "high", "xhigh", "max"]),
    ("gpt-6.1-terra", &["none", "low", "medium", "high", "xhigh", "max"]),
    ("gpt-6-luna", &["none", "low", "medium", "high", "xhigh", "max"]),
];

/// The order levels are shown in; a fetched level not named here goes after them.
const ORDER: [&str; 8] = ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"];

/// What `/v1/models` said, per model id, since the list was last fetched.
static FETCHED: Mutex<Option<HashMap<String, Vec<String>>>> = Mutex::new(None);

/// A level name that is one plain word: it goes into a request body or a command line.
fn plain(l: &str) -> bool {
    !l.is_empty() && l.len() <= 16 && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Known levels first, in [`ORDER`], then the rest as they came.
fn ordered(mut v: Vec<String>) -> Vec<String> {
    v.retain(|l| plain(l));
    let mut seen = std::collections::HashSet::new();
    v.retain(|l| seen.insert(l.clone()));
    v.sort_by_key(|l| ORDER.iter().position(|o| o == l).unwrap_or(ORDER.len()));
    v
}

/// The levels in one `/v1/models` entry's `capabilities.effort`: none when it doesn't say
/// effort is supported.
pub(crate) fn from_capabilities(model: &serde_json::Value) -> Vec<String> {
    let e = &model["capabilities"]["effort"];
    if e["supported"].as_bool() != Some(true) {
        return Vec::new();
    }
    let Some(o) = e.as_object() else { return Vec::new() };
    ordered(o.iter().filter(|(k, v)| k.as_str() != "supported" && v["supported"].as_bool() == Some(true)).map(|(k, _)| k.clone()).collect())
}

/// Keep what a fetched `/v1/models` list says about effort.
pub(crate) fn remember_anthropic(body: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else { return };
    let Some(list) = v["data"].as_array() else { return };
    let map: HashMap<String, Vec<String>> = list.iter().filter_map(|m| Some((m["id"].as_str()?.to_string(), from_capabilities(m)))).collect();
    if let Ok(mut f) = FETCHED.lock() {
        f.get_or_insert_with(HashMap::new).extend(map);
    }
}

/// A table entry for `model`: the exact id, or one with a date after it
/// ("claude-opus-4-5-20251101"); the longest match wins.
fn table<'a>(t: &'a [(&str, &[&str])], model: &str) -> Option<&'a [&'a str]> {
    t.iter()
        .filter(|(id, _)| model == *id || model.strip_prefix(id).is_some_and(|r| r.len() > 1 && r.starts_with('-') && r[1..].bytes().all(|b| b.is_ascii_digit())))
        .max_by_key(|(id, _)| id.len())
        .map(|(_, l)| *l)
}

fn owned(l: &[&str]) -> Vec<String> {
    l.iter().map(|s| s.to_string()).collect()
}

/// The Anthropic API's levels for `model`: what `/v1/models` said, else the docs' table.
fn anthropic(model: &str) -> Vec<String> {
    if let Some(l) = FETCHED.lock().ok().and_then(|f| f.as_ref().and_then(|m| m.get(model).cloned())) {
        return l;
    }
    table(&ANTHROPIC, model).map(owned).unwrap_or_default()
}

/// The host of an OpenAI-compatible address is OpenAI's own API (an empty address means it).
fn is_openai(base_url: &str) -> bool {
    let b = base_url.trim();
    if b.is_empty() {
        return true;
    }
    let rest = b.strip_prefix("https://").unwrap_or("");
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    host.eq_ignore_ascii_case("api.openai.com")
}

/// The levels `model` offers through `provider` (at `base_url`, for an OpenAI-compatible
/// server), in order. Empty: no effort control, and nothing is sent.
pub fn levels(provider: Provider, base_url: &str, model: &str) -> Vec<String> {
    let model = model.trim();
    if model.is_empty() {
        // the provider picks its own model: which one isn't known here
        return Vec::new();
    }
    match provider {
        Provider::Anthropic => anthropic(model),
        Provider::ClaudeCode => table(&ANTHROPIC, model).map(|l| l.iter().filter(|x| CLAUDE_CODE_CLI.contains(x)).map(|s| s.to_string()).collect()).unwrap_or_default(),
        Provider::Codex => ordered(super::codex::efforts(model)),
        Provider::OpenAi if is_openai(base_url) => table(&OPENAI, model).map(owned).unwrap_or_default(),
        Provider::OpenAi | Provider::Off => Vec::new(),
    }
}

/// What Sprawl sends when the user hasn't picked a level: `high` on the Anthropic API, as it
/// always has (Opus 5.5 would otherwise run at its own default, `medium`), where the model
/// takes it. Nothing elsewhere: the provider's own default applies.
pub fn sprawl_default(provider: Provider, levels: &[String]) -> Option<&'static str> {
    (provider == Provider::Anthropic && levels.iter().any(|l| l == "high")).then_some("high")
}

/// The level to send for `model` with these settings: the saved one when the model offers it,
/// else [`sprawl_default`]. None: nothing is sent.
pub fn pick(cfg: &AiConfig, model: &str) -> Option<String> {
    pick_from(cfg, model, &levels(cfg.provider, &cfg.base_url, model))
}

pub(crate) fn pick_from(cfg: &AiConfig, model: &str, levels: &[String]) -> Option<String> {
    if let Some(saved) = cfg.efforts.get(model.trim()) {
        if levels.iter().any(|l| l == saved) {
            return Some(saved.clone());
        }
    }
    sprawl_default(cfg.provider, levels).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn anthropic_table_follows_the_docs() {
        assert_eq!(table(&ANTHROPIC, "claude-opus-5-5").unwrap().len(), 5);
        assert_eq!(table(&ANTHROPIC, "claude-opus-4-6").unwrap(), &["low", "medium", "high", "max"]);
        assert_eq!(table(&ANTHROPIC, "claude-opus-4-5-20251101").unwrap(), &["low", "medium", "high"]);
        assert_eq!(table(&ANTHROPIC, "claude-mythos-preview").unwrap(), &["low", "medium", "high", "max"]);
        // "claude-opus-5-5" is not "claude-opus-5" with a date
        assert_eq!(table(&ANTHROPIC, "claude-opus-5-5-20261101").unwrap().len(), 5);
        assert!(table(&ANTHROPIC, "claude-haiku-4-5").is_none(), "no effort on Haiku 4.5");
        assert!(table(&ANTHROPIC, "claude-opus-5-x").is_none() && table(&ANTHROPIC, "claude-opus-5-").is_none());
        assert!(levels(Provider::Anthropic, "", "claude-new-9").is_empty(), "an unknown model gets nothing");
    }

    #[test]
    fn capabilities_are_read() {
        let m = json!({"id": "x", "capabilities": {"effort": {"supported": true, "low": {"supported": true}, "max": {"supported": true}, "xhigh": {"supported": false}, "high": {"supported": true}, "turbo": {"supported": true}, "Bad Level": {"supported": true}}}});
        assert_eq!(from_capabilities(&m), ["low", "high", "max", "turbo"]);
        assert!(from_capabilities(&json!({"capabilities": {"effort": {"supported": false, "low": {"supported": true}}}})).is_empty());
        assert!(from_capabilities(&json!({"id": "y"})).is_empty());
        // a fetched answer wins over the table, also when it says no effort
        remember_anthropic(r#"{"data":[{"id":"claude-test-fetched","capabilities":{"effort":{"supported":true,"medium":{"supported":true}}}},{"id":"claude-opus-4-6-test","capabilities":{}}]}"#);
        assert_eq!(levels(Provider::Anthropic, "", "claude-test-fetched"), ["medium"]);
        assert!(levels(Provider::Anthropic, "", "claude-opus-4-6-test").is_empty());
        // Claude Code has no list: the table only
        assert!(levels(Provider::ClaudeCode, "", "claude-test-fetched").is_empty());
        assert_eq!(levels(Provider::ClaudeCode, "", "claude-sonnet-4-6"), ["low", "medium", "high", "max"]);
    }

    #[test]
    fn openai_only_on_openai() {
        assert_eq!(levels(Provider::OpenAi, "", "gpt-6-luna")[0], "none");
        assert_eq!(levels(Provider::OpenAi, "https://api.openai.com/v1", "gpt-6.2-sol").len(), 5);
        assert!(levels(Provider::OpenAi, "https://openrouter.ai/api/v1", "gpt-6.2-sol").is_empty());
        assert!(levels(Provider::OpenAi, "http://api.openai.com/v1", "gpt-6.2-sol").is_empty(), "not over plain http");
        assert!(levels(Provider::OpenAi, "https://api.openai.com.evil.example/v1", "gpt-6.2-sol").is_empty());
        assert!(levels(Provider::OpenAi, "", "llama3").is_empty());
        assert!(levels(Provider::Off, "", "gpt-6-luna").is_empty());
    }

    #[test]
    fn only_offered_levels_are_sent() {
        let mut cfg = AiConfig { provider: Provider::Anthropic, model: "claude-opus-5-5".into(), ..AiConfig::default() };
        assert_eq!(pick(&cfg, "claude-opus-5-5").as_deref(), Some("high"), "Sprawl's default where the model takes it");
        cfg.efforts.insert("claude-opus-5-5".into(), "low".into());
        assert_eq!(pick(&cfg, "claude-opus-5-5").as_deref(), Some("low"));
        cfg.efforts.insert("claude-opus-4-5".into(), "max".into());
        assert_eq!(pick(&cfg, "claude-opus-4-5").as_deref(), Some("high"), "a level the model lacks isn't sent");
        assert_eq!(pick(&cfg, "claude-haiku-4-5"), None, "no effort at all");
        assert_eq!(pick(&cfg, "claude-unknown"), None);
        let mut cc = AiConfig { provider: Provider::ClaudeCode, ..AiConfig::default() };
        assert_eq!(pick(&cc, "claude-haiku-5-5"), None, "Claude Code's own default");
        cc.efforts.insert("claude-haiku-5-5".into(), "xhigh".into());
        assert_eq!(pick(&cc, "claude-haiku-5-5").as_deref(), Some("xhigh"));
        cc.efforts.insert("claude-haiku-5-5".into(), "--dangerously-skip-permissions".into());
        assert_eq!(pick(&cc, "claude-haiku-5-5"), None);
    }
}
