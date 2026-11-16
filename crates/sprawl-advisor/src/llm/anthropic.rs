//! Anthropic Messages API over raw HTTP (there is no official Rust SDK).
//!
//! Adaptive thinking, effort set explicitly (Claude Opus 5.5 defaults to `medium`), tools on
//! `auto` (forced tool choice is refused by current models), and the assistant's content echoed
//! back unchanged on every turn, thinking blocks included. On models that support it, a refusal
//! is retried on a fallback model by the API itself (`fallbacks: "default"`).

use std::time::Duration;

use serde_json::{json, Value};

use super::{post_json, Chat, ToolCall, ToolSpec, Turn};

const URL: &str = "https://api.anthropic.com/v1/messages";
const VERSION: &str = "2026-10-21";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

pub struct Anthropic {
    key: String,
    model: String,
    system: String,
    tools: Value,
    messages: Vec<Value>,
}

/// Models that accept `fallbacks: "default"`.
fn has_fallbacks(model: &str) -> bool {
    matches!(model, "claude-opus-5-5" | "claude-opus-5" | "claude-sonnet-5-5" | "claude-fable-5-2" | "claude-fable-5-1")
}

impl Anthropic {
    pub fn new(key: String, model: &str, system: String, first: String, tools: &[ToolSpec]) -> Anthropic {
        let model = if model.trim().is_empty() { "claude-opus-5-5".to_string() } else { model.trim().to_string() };
        let tools = Value::Array(tools.iter().map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.schema})).collect());
        Anthropic { key, model, system, tools, messages: vec![json!({"role": "user", "content": first})] }
    }

    pub(crate) fn body(&self) -> Value {
        let mut body = json!({
            "model": self.model,
            "max_tokens": 16000,
            "system": self.system,
            "messages": self.messages,
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": "high"},
        });
        if self.tools.as_array().is_some_and(|t| !t.is_empty()) {
            body["tools"] = self.tools.clone();
        }
        if has_fallbacks(&self.model) {
            body["fallbacks"] = json!("default");
        }
        body
    }

    /// Read one response: keep its content for the next request, and pull out text and calls.
    pub(crate) fn take(&mut self, resp: &Value) -> Result<Turn, String> {
        let stop = resp["stop_reason"].as_str().unwrap_or("");
        if stop == "refusal" {
            let why = resp["stop_details"]["explanation"].as_str().unwrap_or("no reason given");
            return Err(format!("the model declined: {why}"));
        }
        let content = resp["content"].as_array().cloned().unwrap_or_default();
        let mut turn = Turn { truncated: stop == "max_tokens", ..Turn::default() };
        for block in &content {
            match block["type"].as_str() {
                Some("text") => {
                    if !turn.text.is_empty() {
                        turn.text.push('\n');
                    }
                    turn.text.push_str(block["text"].as_str().unwrap_or(""));
                }
                Some("tool_use") => turn.calls.push(ToolCall {
                    id: block["id"].as_str().unwrap_or("").to_string(),
                    name: block["name"].as_str().unwrap_or("").to_string(),
                    input: block["input"].clone(),
                }),
                _ => {}
            }
        }
        // unchanged, thinking blocks included: they belong to this conversation
        self.messages.push(json!({"role": "assistant", "content": content}));
        // a long server-side step: send it back as it is and the model carries on
        if stop == "pause_turn" && turn.calls.is_empty() {
            turn.text.clear();
        }
        Ok(turn)
    }
}

impl Chat for Anthropic {
    fn turn(&mut self) -> Result<Turn, String> {
        let mut headers = vec![("x-api-key", self.key.clone()), ("anthropic-version", VERSION.to_string())];
        if has_fallbacks(&self.model) {
            headers.push(("anthropic-beta", FALLBACK_BETA.to_string()));
        }
        let resp = post_json(URL, &headers, &self.body(), Duration::from_secs(600))?;
        self.take(&resp)
    }

    fn answer(&mut self, results: Vec<(String, String, bool)>) {
        // all results of one turn go back together, in one user message
        let blocks: Vec<Value> = results.into_iter().map(|(id, text, err)| json!({"type": "tool_result", "tool_use_id": id, "content": text, "is_error": err})).collect();
        self.messages.push(json!({"role": "user", "content": blocks}));
    }

    fn say(&mut self, text: &str) {
        self.messages.push(json!({"role": "user", "content": text}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_tool_turn() {
        let tools = [ToolSpec { name: "largest", description: "d", schema: json!({"type": "object", "properties": {}}) }];
        let mut a = Anthropic::new("k".into(), "", "sys".into(), "report".into(), &tools);
        let b = a.body();
        assert_eq!(b["model"], "claude-opus-5-5");
        assert_eq!(b["thinking"]["type"], "adaptive");
        assert_eq!(b["output_config"]["effort"], "high");
        assert_eq!(b["fallbacks"], "default");
        assert!(b.get("tool_choice").is_none(), "forced tool choice is refused by current models");
        let resp = json!({
            "stop_reason": "tool_use",
            "content": [
                {"type": "thinking", "thinking": "", "signature": "sig"},
                {"type": "text", "text": "Looking closer."},
                {"type": "tool_use", "id": "toolu_1", "name": "largest", "input": {"path": "C:\\x"}}
            ]
        });
        let t = a.take(&resp).unwrap();
        assert_eq!(t.text, "Looking closer.");
        assert_eq!(t.calls.len(), 1);
        assert_eq!(t.calls[0].input["path"], "C:\\x");
        // the thinking block goes back unchanged
        assert_eq!(a.messages[1]["content"][0]["signature"], "sig");
        a.answer(vec![("toolu_1".into(), "ok".into(), false)]);
        assert_eq!(a.messages[2]["content"][0]["tool_use_id"], "toolu_1");
        assert!(a.take(&json!({"stop_reason": "refusal", "content": []})).is_err());
        // an older model gets no fallbacks parameter
        let old = Anthropic::new("k".into(), "claude-haiku-4-5", "s".into(), "r".into(), &[]);
        assert!(old.body().get("fallbacks").is_none() && old.body().get("tools").is_none());
    }
}
