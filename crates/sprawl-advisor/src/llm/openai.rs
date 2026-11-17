//! Any OpenAI-compatible chat completions endpoint: OpenAI, OpenRouter, Ollama, LM Studio, vLLM.

use std::time::Duration;

use serde_json::{json, Value};

use super::{post_json, Chat, ToolCall, ToolSpec, Turn};

pub struct OpenAi {
    sink: Option<super::Sink>,
    url: String,
    key: Option<String>,
    model: String,
    tools: Value,
    messages: Vec<Value>,
}

impl OpenAi {
    /// `base` is the API root, e.g. `http://localhost:11434/v1`.
    pub fn new(base: &str, key: Option<String>, model: &str, system: String, first: String, tools: &[ToolSpec]) -> OpenAi {
        let base = if base.trim().is_empty() { "https://api.openai.com/v1" } else { base.trim() };
        let url = format!("{}/chat/completions", base.trim_end_matches('/'));
        let tools = Value::Array(
            tools.iter().map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.schema}})).collect(),
        );
        OpenAi {
            sink: None,
            url,
            key: key.filter(|k| !k.is_empty()),
            model: model.trim().to_string(),
            tools,
            messages: vec![json!({"role": "system", "content": system}), json!({"role": "user", "content": first})],
        }
    }

    pub(crate) fn body(&self) -> Value {
        let mut body = json!({"model": self.model, "messages": self.messages});
        if self.tools.as_array().is_some_and(|t| !t.is_empty()) {
            body["tools"] = self.tools.clone();
        }
        body
    }

    pub(crate) fn take(&mut self, resp: &Value) -> Result<Turn, String> {
        let choice = &resp["choices"][0];
        let msg = choice["message"].clone();
        if msg.is_null() {
            return Err(format!("no answer in the response: {}", super::short(&resp.to_string())));
        }
        let usage = super::Usage {
            input: resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            output: resp["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            ..super::Usage::default()
        };
        let mut turn = Turn { text: msg["content"].as_str().unwrap_or("").to_string(), truncated: choice["finish_reason"] == "length", usage, ..Turn::default() };
        for c in msg["tool_calls"].as_array().into_iter().flatten() {
            let args = c["function"]["arguments"].as_str().unwrap_or("{}");
            // a malformed argument string still gets an answer: an error saying so
            let input = serde_json::from_str(args).unwrap_or_else(|_| json!({"_unparsed": args}));
            turn.calls.push(ToolCall { id: c["id"].as_str().unwrap_or("").to_string(), name: c["function"]["name"].as_str().unwrap_or("").to_string(), input });
        }
        self.messages.push(msg);
        Ok(turn)
    }
}

impl Chat for OpenAi {
    fn turn(&mut self) -> Result<Turn, String> {
        let mut headers = Vec::new();
        if let Some(k) = &self.key {
            headers.push(("authorization", format!("Bearer {k}")));
        }
        // local models can be slow on a long report
        let plain = self.body();
        let resp = match self.sink.as_mut() {
            None => post_json(&self.url, &headers, &plain, Duration::from_secs(900))?,
            Some(sink) => {
                let mut body = plain;
                body["stream"] = json!(true);
                body["stream_options"] = json!({"include_usage": true});
                let mut a = Chunks::default();
                super::post_events(&self.url, &headers, &body, Duration::from_secs(900), &mut |d| {
                    let before = a.text.len();
                    a.chunk(d);
                    if a.text.len() != before {
                        sink(&a.text);
                    }
                })?;
                a.response()
            }
        };
        self.take(&resp)
    }

    fn answer(&mut self, results: Vec<(String, String, bool)>) {
        for (id, text, err) in results {
            let content = if err { format!("Error: {text}") } else { text };
            self.messages.push(json!({"role": "tool", "tool_call_id": id, "content": content}));
        }
    }

    fn say(&mut self, text: &str) {
        self.messages.push(json!({"role": "user", "content": text}));
    }

    fn stream_to(&mut self, sink: Option<super::Sink>) {
        self.sink = sink;
    }
}

/// Streamed chat completion chunks, put back together into one response.
#[derive(Default)]
pub(crate) struct Chunks {
    pub(crate) text: String,
    calls: Vec<(String, String, String)>,
    finish: Value,
    usage: Value,
}

impl Chunks {
    pub(crate) fn chunk(&mut self, data: &str) {
        let Ok(v) = serde_json::from_str::<Value>(data) else { return };
        if !v["usage"].is_null() {
            self.usage = v["usage"].clone();
        }
        let c = &v["choices"][0];
        if let Some(t) = c["delta"]["content"].as_str() {
            self.text.push_str(t);
        }
        for tc in c["delta"]["tool_calls"].as_array().into_iter().flatten() {
            let i = tc["index"].as_u64().unwrap_or(0) as usize;
            while self.calls.len() <= i {
                self.calls.push(Default::default());
            }
            let e = &mut self.calls[i];
            if let Some(id) = tc["id"].as_str() {
                e.0 = id.to_string();
            }
            if let Some(n) = tc["function"]["name"].as_str() {
                e.1.push_str(n);
            }
            if let Some(a) = tc["function"]["arguments"].as_str() {
                e.2.push_str(a);
            }
        }
        if !c["finish_reason"].is_null() {
            self.finish = c["finish_reason"].clone();
        }
    }

    pub(crate) fn response(self) -> Value {
        let mut msg = json!({"role": "assistant", "content": self.text});
        if !self.calls.is_empty() {
            msg["tool_calls"] = Value::Array(self.calls.into_iter().map(|(id, name, args)| json!({"id": id, "type": "function", "function": {"name": name, "arguments": args}})).collect());
        }
        json!({"choices": [{"message": msg, "finish_reason": self.finish}], "usage": self.usage})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_rebuild_the_answer() {
        let mut c = Chunks::default();
        for d in [
            r#"{"choices":[{"delta":{"content":"Hi "}}]}"#,
            r#"{"choices":[{"delta":{"content":"there"}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"lar","arguments":"{\"pa"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"gest","arguments":"th\":1}"}}]},"finish_reason":"tool_calls"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":3}}"#,
        ] {
            c.chunk(d);
        }
        assert_eq!(c.text, "Hi there");
        let mut o = OpenAi::new("http://localhost:1/v1", None, "m", "s".into(), "r".into(), &[]);
        let t = o.take(&c.response()).unwrap();
        assert_eq!(t.text, "Hi there");
        assert_eq!((t.calls[0].name.as_str(), t.calls[0].input["path"].as_i64()), ("largest", Some(1)));
        assert_eq!(t.usage.output, 3);
    }

    #[test]
    fn tool_calls_round_trip() {
        let tools = [ToolSpec { name: "largest", description: "d", schema: json!({"type": "object"}) }];
        let mut o = OpenAi::new("http://localhost:11434/v1/", None, "llama3", "sys".into(), "report".into(), &tools);
        assert_eq!(o.url, "http://localhost:11434/v1/chat/completions");
        assert_eq!(o.body()["tools"][0]["function"]["name"], "largest");
        let resp = json!({"choices": [{"finish_reason": "tool_calls", "message": {"role": "assistant", "content": null,
            "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "largest", "arguments": "{\"path\":\"C:\\\\x\"}"}}]}}]});
        let t = o.take(&resp).unwrap();
        assert_eq!(t.calls[0].input["path"], "C:\\x");
        o.answer(vec![("c1".into(), "ok".into(), false)]);
        assert_eq!(o.messages.last().unwrap()["tool_call_id"], "c1");
        assert!(o.take(&json!({"error": "x"})).is_err());
    }
}
