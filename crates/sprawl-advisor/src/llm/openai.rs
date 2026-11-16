//! Any OpenAI-compatible chat completions endpoint: OpenAI, OpenRouter, Ollama, LM Studio, vLLM.

use std::time::Duration;

use serde_json::{json, Value};

use super::{post_json, Chat, ToolCall, ToolSpec, Turn};

pub struct OpenAi {
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
        let mut turn = Turn { text: msg["content"].as_str().unwrap_or("").to_string(), truncated: choice["finish_reason"] == "length", ..Turn::default() };
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
        let resp = post_json(&self.url, &headers, &self.body(), Duration::from_secs(900))?;
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
