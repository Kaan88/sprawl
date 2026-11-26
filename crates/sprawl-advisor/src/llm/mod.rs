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
pub mod codex;
#[cfg(windows)]
pub(crate) mod contain;
pub mod effort;
pub mod http;
pub mod models;
pub mod more_tools;
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
/// What a cache read costs, as a share of the input price.
pub(crate) fn cache_read_share(model: &str) -> f64 {
    if model.starts_with("claude-fable-5-") || model.starts_with("claude-mythos-5-") {
        0.025
    } else if model.starts_with("claude-opus-5-5") || model.starts_with("claude-sonnet-5-5") {
        0.05
    } else {
        0.1
    }
}

/// Price per million tokens (input, output) for an exact model id, or one with a date or
/// version suffix after it.
pub(crate) fn anthropic_price(model: &str) -> Option<(f64, f64)> {
    let known = ["claude-opus-5-5", "claude-opus-5", "claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-5-5", "claude-sonnet-5", "claude-sonnet-4-6", "claude-haiku-5-5", "claude-haiku-4-5", "claude-fable-5-2", "claude-fable-5-1", "claude-fable-5"];
    // the longest id it starts with: "claude-opus-5-5-20261101" is Opus 5.5, not Opus 5
    let model = known.iter().filter(|k| model == **k || model.strip_prefix(**k).is_some_and(|r| r.starts_with('-') && r[1..].bytes().all(|b| b.is_ascii_digit()))).max_by_key(|k| k.len())?;
    exact_price(model)
}

fn exact_price(model: &str) -> Option<(f64, f64)> {
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
        self.turn_ticking(&mut || {})
    }

    fn turn_ticking(&mut self, tick: &mut dyn FnMut()) -> Result<Turn, String> {
        if self.stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(STOPPED.into());
        }
        let t = self.inner.turn_ticking(tick)?;
        self.usage.borrow_mut().add(t.usage);
        if self.stop.load(std::sync::atomic::Ordering::Relaxed) {
            // the turn is in the history already: its tool calls get an answer, or the next
            // question would be refused (a tool call must be followed by its result)
            if !t.calls.is_empty() {
                self.inner.answer(t.calls.iter().map(|c| (c.id.clone(), "Stopped by the user.".to_string(), true)).collect());
            }
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

    fn stream_to(&mut self, sink: Option<Sink>) {
        self.inner.stream_to(sink)
    }
}

/// POST JSON and read the answer as server-sent events, each `data:` payload going to
/// `event` as it arrives. Retried like [`post_json`] while nothing has arrived yet.
pub(crate) fn post_events(url: &str, headers: &[(&str, String)], body: &Value, timeout: Duration, event: &mut dyn FnMut(&str)) -> Result<(), String> {
    let text = body.to_string();
    log_request(&format!("POST {url} (streamed)"), &text);
    let mut wait = 4;
    for attempt in 0..3 {
        let mut sse = http::Sse::default();
        let mut got = false;
        let r = http::post_stream(url, headers, text.as_bytes(), timeout, &mut |bytes| {
            got = true;
            for d in sse.feed(bytes) {
                event(&d);
            }
        });
        match r {
            Ok(r) if (200..300).contains(&r.status) => return Ok(()),
            Ok(r) => {
                if attempt < 2 && !got && matches!(r.status, 429 | 500 | 502 | 503 | 529) {
                    std::thread::sleep(Duration::from_secs(r.retry_after.unwrap_or(wait).min(30)));
                    wait *= 3;
                    continue;
                }
                return Err(format!("HTTP {}: {}", r.status, error_message(&r.body)));
            }
            Err(e) if attempt < 2 && !got && e.contains("couldn't connect") => {
                std::thread::sleep(Duration::from_secs(wait));
                wait *= 3;
            }
            Err(e) => return Err(e),
        }
    }
    Err("no answer".into())
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
    /// Stream the text of each coming turn to `sink` (the turn's text so far), where the
    /// provider can. `None` turns it off.
    fn stream_to(&mut self, _sink: Option<Sink>) {}
    /// [`Chat::turn`], calling `tick` now and then while it waits, where the provider can
    /// (Claude Code, whose tools run in Sprawl's own MCP server meanwhile).
    fn turn_ticking(&mut self, _tick: &mut dyn FnMut()) -> Result<Turn, String> {
        self.turn()
    }
}

/// Gets a turn's text so far, while it is written.
pub type Sink = Box<dyn FnMut(&str)>;

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
    if crate::read_only() { return None; }
    advisor_dir().map(|d| d.join("requests.log"))
}

/// Append one outgoing request to the log (kept under 20 MB: the old half moves to `.old`).
/// Through checked handles, also when elevated: the folders on the way are held, the log is
/// opened as itself (a link at its name is refused, never followed) with one name only, and
/// it is renamed through its handle.
pub(crate) fn log_request(what: &str, body: &str) {
    if crate::read_only() { return; }
    let Some(d) = advisor_dir() else { return };
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let _ = crate::store::append_checked(&d, "requests.log", 20 << 20, "requests.log.old", |f| {
        writeln!(f, "===== {} (unix {secs}) {what} ({} bytes)\n{body}\n", crate::facts::month_year(std::time::SystemTime::now()), body.len())
    });
}

/// Empty the request log (and its older halves) through checked handles.
pub fn clear_request_log() -> std::io::Result<()> {
    crate::require_write()?;
    let Some(d) = advisor_dir() else { return Ok(()) };
    let mut result = Ok(());
    for name in ["requests.log", "requests.log.old", "requests.old"] {
        if let Err(e) = crate::store::remove_checked(&d, name) { result = Err(e); }
    }
    result
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
