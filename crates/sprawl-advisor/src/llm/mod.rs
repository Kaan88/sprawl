//! Talking to a language model: the report it starts from, the tools it uses to look closer,
//! and the loop between them.
//!
//! Rust finds, the model judges. The model never sees the whole tree, only a compact report and
//! what it asks for through the tools. Everything it claims goes through
//! [`tools::Tools::report_finding`] and [`crate::safety::check`] before it is shown, and it never
//! writes command text: it names a remedy, and Rust writes the command.
//!
//! Every request body is appended to `%APPDATA%\Sprawl\advisor\requests.log` (never the key), so
//! the user can see exactly what left the machine.

pub mod agent;
pub mod anthropic;
pub mod claude_code;
pub mod http;
pub mod openai;
pub mod report;
pub mod tools;

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// What one model turn produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Turn {
    pub text: String,
    pub calls: Vec<ToolCall>,
    /// The turn hit the output limit: what came back may be cut short.
    pub truncated: bool,
    /// The server paused a long step; sending the conversation again lets it carry on.
    pub paused: bool,
    /// What this turn used, when the provider says.
    pub usage: Usage,
}

/// Tokens a run used, and its cost when the provider reports it (Claude Code) or Sprawl can
/// work it out (the Anthropic API, from published prices).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: u64,
    /// Read from the prompt cache (much cheaper).
    pub cached: u64,
    pub output: u64,
    pub cost_usd: Option<f64>,
}

impl Usage {
    pub fn add(&mut self, o: Usage) {
        self.input += o.input;
        self.cached += o.cached;
        self.output += o.output;
        self.cost_usd = match (self.cost_usd, o.cost_usd) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
    }

    /// "12.3k tokens, about $0.04"
    pub fn text(&self) -> String {
        let t = self.input + self.cached + self.output;
        if t == 0 && self.cost_usd.is_none() {
            return String::new();
        }
        let tokens = if t >= 1000 { format!("{:.1}k tokens", t as f64 / 1000.0) } else { format!("{t} tokens") };
        match self.cost_usd {
            Some(c) if c >= 0.01 => format!("{tokens}, about ${c:.2}"),
            Some(_) => format!("{tokens}, under $0.01"),
            None => tokens,
        }
    }
}

/// Dollars per million tokens (input, output) of Anthropic models, from the published prices.
/// Cache reads cost about 5% of input.
pub(crate) fn anthropic_price(model: &str) -> Option<(f64, f64)> {
    Some(match model {
        "claude-opus-5-5" => (4.0, 20.0),
        "claude-opus-5" | "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => (5.0, 25.0),
        "claude-sonnet-5-5" | "claude-sonnet-5" => (2.0, 10.0),
        "claude-sonnet-4-6" => (3.0, 15.0),
        "claude-haiku-5-5" => (0.10, 0.50),
        "claude-haiku-4-5" => (1.0, 5.0),
        "claude-fable-5-2" => (8.0, 40.0),
        "claude-fable-5-1" | "claude-fable-5" => (10.0, 50.0),
        _ => return None,
    })
}

/// What the user's Stop button sets. Checked before every step.
pub type Stop = std::sync::Arc<std::sync::atomic::AtomicBool>;

/// The error a stopped run ends with.
pub const STOPPED: &str = "Stopped.";

/// A conversation that stops when asked and adds up what it used.
pub struct Metered {
    inner: Box<dyn Chat>,
    stop: Stop,
    pub usage: std::rc::Rc<std::cell::RefCell<Usage>>,
}

impl Metered {
    pub fn new(inner: Box<dyn Chat>, stop: Stop, usage: std::rc::Rc<std::cell::RefCell<Usage>>) -> Metered {
        Metered { inner, stop, usage }
    }
}

impl Chat for Metered {
    fn turn(&mut self) -> Result<Turn, String> {
        if self.stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(STOPPED.into());
        }
        let t = self.inner.turn()?;
        self.usage.borrow_mut().add(t.usage);
        if self.stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(STOPPED.into());
        }
        Ok(t)
    }

    fn answer(&mut self, results: Vec<(String, String, bool)>) {
        self.inner.answer(results)
    }

    fn say(&mut self, text: &str) {
        self.inner.say(text)
    }
}

/// A tool the model may call, with a JSON Schema for its input.
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
}

/// A conversation with one provider. The provider keeps the history in its own wire format.
pub trait Chat {
    /// Send the conversation so far and return the model's next turn.
    fn turn(&mut self) -> Result<Turn, String>;
    /// Answer every tool call of the last turn: (call id, result text, is an error).
    fn answer(&mut self, results: Vec<(String, String, bool)>);
    /// Add a plain user message (a nudge to finish).
    fn say(&mut self, text: &str);
}

static MCP_EXE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// The program that serves `--mcp` (the app sets it to itself), so Claude Code can be given
/// Sprawl's tools.
pub fn set_mcp_exe(p: PathBuf) {
    let _ = MCP_EXE.set(p);
}

pub(crate) fn mcp_exe() -> Option<&'static PathBuf> {
    MCP_EXE.get()
}

/// Where the request log and other advisor files live.
pub fn advisor_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Sprawl").join("advisor"))
}

pub fn request_log_path() -> Option<PathBuf> {
    advisor_dir().map(|d| d.join("requests.log"))
}

/// Append one outgoing request to the log (kept under 20 MB: the old half moves to `.old`).
pub(crate) fn log_request(what: &str, body: &str) {
    let Some(p) = request_log_path() else { return };
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > 20 << 20) {
        let _ = std::fs::rename(&p, p.with_extension("log.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "===== {} (unix {secs}) {what} ({} bytes)\n{body}\n", crate::facts::month_year(std::time::SystemTime::now()), body.len());
    }
}

/// POST JSON and parse the JSON answer. Retries twice on rate limits and server errors.
pub(crate) fn post_json(url: &str, headers: &[(&str, String)], body: &Value, timeout: Duration) -> Result<Value, String> {
    let text = body.to_string();
    log_request(&format!("POST {url}"), &text);
    let mut wait = 4;
    for attempt in 0..3 {
        match http::post(url, headers, text.as_bytes(), timeout) {
            Ok(r) if (200..300).contains(&r.status) => {
                return serde_json::from_str(&r.body).map_err(|e| format!("the answer isn't JSON ({e}): {}", short(&r.body)));
            }
            Ok(r) => {
                // not 504: the request may have run, and been paid for
                if attempt < 2 && matches!(r.status, 429 | 500 | 502 | 503 | 529) {
                    std::thread::sleep(Duration::from_secs(r.retry_after.unwrap_or(wait).min(30)));
                    wait *= 3;
                    continue;
                }
                return Err(format!("HTTP {}: {}", r.status, error_message(&r.body)));
            }
            // only when it never got there: a timeout or a dropped connection may have run, and
            // been paid for; a bad address won't get better
            Err(e) if attempt < 2 && e.contains("couldn't connect") => {
                std::thread::sleep(Duration::from_secs(wait));
                wait *= 3;
            }
            Err(e) => return Err(e),
        }
    }
    Err("no answer".into())
}

/// The `error.message` of an API error body, or the start of the body.
fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string).or_else(|| v["error"].as_str().map(str::to_string)))
        .unwrap_or_else(|| short(body))
}

pub(crate) fn short(s: &str) -> String {
    let s: String = s.chars().take(300).collect();
    s.replace(['\r', '\n'], " ")
}

/// Cut `s` to at most `max` characters, saying so.
pub(crate) fn clip(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("\n[cut short]");
    out
}
