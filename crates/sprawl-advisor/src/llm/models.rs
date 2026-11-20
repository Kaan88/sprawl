//! The models a provider offers, for the Model dropdown: Anthropic's `/v1/models` with the
//! stored key, or an OpenAI-compatible server's `/models` (OpenAI, OpenRouter, Ollama, LM Studio,
//! vLLM). Claude Code has no such list, so it gets the current Claude models.

use std::time::Duration;

use super::http;
use crate::config::{self, Provider};

/// The current Claude models, for Claude Code: its CLI has no way to list them.
pub const CLAUDE: [&str; 4] = ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5", "claude-fable-5-2"];

/// Well-known OpenAI-compatible services, for the dropdown beside the address: picking one only
/// fills the address in. A key still goes only to the host it was saved (or typed) for.
pub const PRESETS: [(&str, &str); 10] = [
    ("OpenAI", "https://api.openai.com/v1"),
    ("DeepSeek", "https://api.deepseek.com/v1"),
    ("OpenRouter", "https://openrouter.ai/api/v1"),
    ("Google Gemini", "https://generativelanguage.googleapis.com/v1beta/openai"),
    ("xAI", "https://api.x.ai/v1"),
    ("Mistral", "https://api.mistral.ai/v1"),
    ("Groq", "https://api.groq.com/openai/v1"),
    ("Together AI", "https://api.together.xyz/v1"),
    ("Ollama (this PC)", "http://localhost:11434/v1"),
    ("LM Studio (this PC)", "http://localhost:1234/v1"),
];

/// The dropdown's entries: "Custom address" first, then [`PRESETS`].
pub fn preset_names() -> Vec<String> {
    std::iter::once("Custom address".to_string()).chain(PRESETS.iter().map(|(n, _)| n.to_string())).collect()
}

/// The dropdown entry for `url`: its preset's place in [`preset_names`], else 0 (custom). An
/// empty address is OpenAI's.
pub fn preset_index(url: &str) -> usize {
    let norm = |u: &str| u.trim().trim_end_matches('/').to_lowercase();
    let u = if url.trim().is_empty() { PRESETS[0].1.to_string() } else { norm(url) };
    PRESETS.iter().position(|(_, p)| norm(p) == u).map_or(0, |i| i + 1)
}

/// The address of dropdown entry `i` ([`preset_names`]); None for "Custom address".
pub fn preset_url(i: usize) -> Option<&'static str> {
    i.checked_sub(1).and_then(|i| PRESETS.get(i)).map(|(_, u)| *u)
}

/// The model ids `provider` offers at `base_url` (OpenAI-compatible only), or why there's no list.
/// `typed` is a key typed in the settings and not saved yet: it is used in place of the saved
/// one, so a new key can be tried before Save. For an OpenAI-compatible server it counts as a
/// key for the address typed with it, under the same https rule as a saved one.
pub fn list(provider: Provider, base_url: &str, typed: Option<&str>) -> Result<Vec<String>, String> {
    let typed = typed.map(str::trim).filter(|k| !k.is_empty()).map(str::to_string);
    let (url, headers, sort) = match provider {
        Provider::Anthropic => {
            let key = typed.or_else(|| config::key(provider)).filter(|k| !k.is_empty()).ok_or("no API key saved yet")?;
            ("https://api.anthropic.com/v1/models?limit=1000".to_string(), vec![("x-api-key", key), ("anthropic-version", super::anthropic::VERSION.to_string())], false)
        }
        Provider::OpenAi => {
            let base = if base_url.trim().is_empty() { "https://api.openai.com/v1" } else { base_url.trim() };
            let mut h = Vec::new();
            // the same rule as a chat: https (or this machine), and only the host the key is for
            let key = match typed {
                Some(k) => super::agent::key_for(base, Some(k), super::agent::host_of(base), None)?,
                None => super::agent::openai_key(base).ok().flatten(),
            };
            if let Some(k) = key {
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
    // what each Claude model says about effort comes with the list
    if provider == Provider::Anthropic {
        super::effort::remember_anthropic(&r.body);
    }
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

    #[test]
    fn presets_fill_and_match_the_address() {
        use super::{preset_index, preset_names, preset_url, PRESETS};
        assert_eq!(preset_names().len(), PRESETS.len() + 1);
        assert_eq!(preset_index(""), 1, "empty is OpenAI");
        assert_eq!(preset_index("https://API.deepseek.com/v1/"), 2);
        assert_eq!(preset_index("https://example.com/v1"), 0);
        assert_eq!(preset_url(0), None);
        for i in 1..=PRESETS.len() {
            assert_eq!(preset_index(preset_url(i).unwrap()), i);
            let u = preset_url(i).unwrap();
            assert!(u.starts_with("https://") || u.starts_with("http://localhost:"), "{u}");
        }
    }

    /// A key typed before Save goes to the address typed with it, under the saved key's rules.
    #[test]
    fn a_typed_key_follows_the_typed_address() {
        use crate::llm::agent::{host_of, key_for};
        let k = |url: &str| key_for(url, Some("sk-typed".into()), host_of(url), None);
        assert_eq!(k("https://api.deepseek.com/v1").unwrap().as_deref(), Some("sk-typed"));
        assert_eq!(k("http://localhost:11434/v1").unwrap().as_deref(), Some("sk-typed"), "this PC over http is fine");
        assert!(k("http://example.com/v1").is_err(), "never over plain http to another machine");
    }
}
