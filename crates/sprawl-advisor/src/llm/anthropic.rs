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
    sink: Option<super::Sink>,
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
        Anthropic { sink: None, key, model, system, tools, messages: vec![json!({"role": "user", "content": first})] }
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
        let u = &resp["usage"];
        let written = u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
        let input = u["input_tokens"].as_u64().unwrap_or(0) + written;
        let cached = u["cache_read_input_tokens"].as_u64().unwrap_or(0);
        let output = u["output_tokens"].as_u64().unwrap_or(0);
        // the model that answered (a fallback may have)
        let model = resp["model"].as_str().unwrap_or(&self.model);
        // cache writes (5 minutes) cost 1.25 times the input price; reads a share of it
        let cost_usd = super::anthropic_price(model).map(|(i, o)| ((input - written) as f64 * i + written as f64 * i * 1.25 + cached as f64 * i * super::cache_read_share(model) + output as f64 * o) / 1e6);
        let usage = super::Usage { input, cached, output, cost_usd };
        let mut turn = Turn { truncated: stop == "max_tokens", usage, ..Turn::default() };
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
            turn.paused = true;
        }
        Ok(turn)
    }
}

/// The Messages API's stream events, put back together into the message they describe (the
/// same shape a request without streaming answers with), so it is echoed back unchanged.
#[derive(Default)]
pub(crate) struct Assembler {
    msg: Value,
    blocks: Vec<Value>,
    /// Tool inputs arrive as pieces of JSON text.
    partial: Vec<String>,
    pub(crate) text: String,
    pub(crate) error: Option<String>,
}

impl Assembler {
    pub(crate) fn event(&mut self, data: &str) {
        let Ok(e) = serde_json::from_str::<Value>(data) else { return };
        match e["type"].as_str() {
            Some("message_start") => self.msg = e["message"].clone(),
            Some("content_block_start") => {
                self.blocks.push(e["content_block"].clone());
                self.partial.push(String::new());
            }
            Some("content_block_delta") => {
                let (Some(b), Some(p)) = (self.blocks.last_mut(), self.partial.last_mut()) else { return };
                let d = &e["delta"];
                match d["type"].as_str() {
                    Some("text_delta") => {
                        let t = d["text"].as_str().unwrap_or("");
                        let cur = b["text"].as_str().unwrap_or("").to_string() + t;
                        b["text"] = json!(cur);
                        self.text.push_str(t);
                    }
                    Some("thinking_delta") => {
                        let cur = b["thinking"].as_str().unwrap_or("").to_string() + d["thinking"].as_str().unwrap_or("");
                        b["thinking"] = json!(cur);
                    }
                    Some("signature_delta") => b["signature"] = d["signature"].clone(),
                    Some("input_json_delta") => p.push_str(d["partial_json"].as_str().unwrap_or("")),
                    _ => {}
                }
            }
            Some("content_block_stop") => {
                if let (Some(b), Some(p)) = (self.blocks.last_mut(), self.partial.last()) {
                    if !p.is_empty() {
                        b["input"] = serde_json::from_str(p).unwrap_or_else(|_| json!({"_unparsed": p}));
                    }
                }
            }
            Some("message_delta") => {
                for (k, v) in e["delta"].as_object().into_iter().flatten() {
                    self.msg[k.as_str()] = v.clone();
                }
                if let Some(o) = e["usage"]["output_tokens"].as_u64() {
                    self.msg["usage"]["output_tokens"] = json!(o);
                }
            }
            Some("error") => self.error = Some(e["error"]["message"].as_str().unwrap_or("the stream failed").to_string()),
            _ => {}
        }
    }

    pub(crate) fn message(mut self) -> Result<Value, String> {
        if let Some(e) = self.error {
            return Err(e);
        }
        self.msg["content"] = Value::Array(self.blocks);
        Ok(self.msg)
    }
}

impl Chat for Anthropic {
    fn turn(&mut self) -> Result<Turn, String> {
        let mut headers = vec![("x-api-key", self.key.clone()), ("anthropic-version", VERSION.to_string())];
        if has_fallbacks(&self.model) {
            headers.push(("anthropic-beta", FALLBACK_BETA.to_string()));
        }
        let plain = self.body();
        let resp = match self.sink.as_mut() {
            None => post_json(URL, &headers, &plain, Duration::from_secs(600))?,
            Some(sink) => {
                let mut body = plain;
                body["stream"] = json!(true);
                let mut a = Assembler::default();
                super::post_events(URL, &headers, &body, Duration::from_secs(900), &mut |d| {
                    let before = a.text.len();
                    a.event(d);
                    if a.text.len() != before {
                        sink(&a.text);
                    }
                })?;
                a.message()?
            }
        };
        self.take(&resp)
    }

    fn stream_to(&mut self, sink: Option<super::Sink>) {
        self.sink = sink;
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
    fn stream_rebuilds_the_message() {
        let mut a = Assembler::default();
        for e in [
            r#"{"type":"message_start","message":{"model":"claude-opus-5-5","usage":{"input_tokens":10}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hel"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"lo"}}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t1","name":"largest","input":{}}}"#,
            r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"path\": \"C:"}}"#,
            r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\\\\x\"}"}}"#,
            r#"{"type":"content_block_stop","index":2}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}"#,
        ] {
            a.event(e);
        }
        assert_eq!(a.text, "Hello");
        let m = a.message().unwrap();
        assert_eq!(m["stop_reason"], "tool_use");
        assert_eq!(m["content"][0]["signature"], "sig", "thinking comes back unchanged");
        assert_eq!(m["content"][2]["input"]["path"], "C:\\x");
        let mut c = Anthropic::new("k".into(), "", "s".into(), "r".into(), &[]);
        let t = c.take(&m).unwrap();
        assert_eq!((t.text.as_str(), t.calls.len(), t.usage.output), ("Hello", 1, 7));
        let mut bad = Assembler::default();
        bad.event(r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#);
        assert!(bad.message().is_err());
    }

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
