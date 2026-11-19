//! The models a provider offers, for the Model dropdown: Anthropic's `/v1/models` with the
//! stored key, or an OpenAI-compatible server's `/models` (OpenAI, OpenRouter, Ollama, LM Studio,
//! vLLM). Claude Code has no such list, so it gets the current Claude models.

use std::time::Duration;

use super::http;
use crate::config::{self, Provider};

/// The current Claude models, for Claude Code: its CLI has no way to list them.
pub const CLAUDE: [&str; 4] = ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5", "claude-fable-5-2"];

/// The model ids `provider` offers at `base_url` (OpenAI-compatible only), or why there's no list.
pub fn list(provider: Provider, base_url: &str) -> Result<Vec<String>, String> {
    let (url, headers, sort) = match provider {
        Provider::Anthropic => {
            let key = config::key(provider).filter(|k| !k.is_empty()).ok_or("no API key saved yet")?;
            ("https://api.anthropic.com/v1/models?limit=1000".to_string(), vec![("x-api-key", key), ("anthropic-version", super::anthropic::VERSION.to_string())], false)
        }
        Provider::OpenAi => {
            let base = if base_url.trim().is_empty() { "https://api.openai.com/v1" } else { base_url.trim() };
            let mut h = Vec::new();
            // the same rule as a chat: https (or this machine), and only the host the key is for
            if let Some(k) = super::agent::openai_key(base).ok().flatten() {
                h.push(("Authorization", format!("Bearer {k}")));
            }
            (format!("{}/models", base.trim_end_matches('/')), h, true)
        }
        Provider::ClaudeCode => return Ok(CLAUDE.iter().map(|m| m.to_string()).collect()),
        // Codex's own list, from its cache: nothing is fetched
        Provider::Codex => return Ok(super::codex::models()),
        Provider::Off => return Err("no provider chosen".into()),
    };
    let r = http::get(&url, &headers, Duration::from_secs(10))?;
    match r.status {
        200 => {}
        401 | 403 => return Err("the key was refused".into()),
        s => return Err(format!("the server answered {s}")),
    }
    let ids = parse(&r.body)?;
    Ok(if sort {
        let mut ids = ids;
        ids.sort_by_key(|a| a.to_lowercase());
        ids
    } else {
        ids
    })
}

/// The key the model list would carry to `base_url` (see [`super::agent::key_for`]).
#[cfg(test)]
pub(crate) fn auth_for(base_url: &str, saved: Option<String>, bound: Option<String>, env: Option<String>) -> Option<String> {
    super::agent::key_for(base_url, saved, bound, env).ok().flatten()
}

/// `{"data":[{"id":..}]}` (Anthropic, OpenAI), or Ollama's own `{"models":[{"name":..}]}`.
fn parse(body: &str) -> Result<Vec<String>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|_| "the answer isn't a list of models".to_string())?;
    let mut ids: Vec<String> = v["data"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect()).unwrap_or_default();
    if ids.is_empty() {
        ids = v["models"].as_array().map(|a| a.iter().filter_map(|m| m["name"].as_str().or(m["model"].as_str()).map(String::from)).collect()).unwrap_or_default();
    }
    let mut seen = std::collections::HashSet::new();
    ids.retain(|m| !m.trim().is_empty() && m.len() < 200 && seen.insert(m.clone()));
    if ids.is_empty() {
        return Err("the server listed no models".into());
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn lists() {
        assert_eq!(parse(r#"{"data":[{"id":"claude-opus-5-5"},{"id":"claude-sonnet-5-5"}],"has_more":false}"#).unwrap(), ["claude-opus-5-5", "claude-sonnet-5-5"]);
        assert_eq!(parse(r#"{"object":"list","data":[{"id":"qwen3:32b"},{"id":"qwen3:32b"}]}"#).unwrap(), ["qwen3:32b"]);
        assert_eq!(parse(r#"{"models":[{"name":"llama3"}]}"#).unwrap(), ["llama3"]);
        assert!(parse(r#"{"data":[]}"#).is_err());
        assert!(parse("<html>").is_err());
    }
}
