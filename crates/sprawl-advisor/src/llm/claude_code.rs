//! The user's own Claude Code: `claude -p` with its login, no key needed. It gets the report and
//! answers once (no tools here; Sprawl as an MCP server comes later), in the single-report form
//! that [`super::agent`] parses.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{log_request, Chat, Turn};

pub struct ClaudeCode {
    exe: PathBuf,
    model: String,
    /// An MCP config with Sprawl's server alone: its tools are the only ones allowed.
    mcp: Option<PathBuf>,
    /// Everything so far: the instructions, the report and the conversation as text.
    prompt: String,
    /// There is something new to answer.
    pending: bool,
    /// Set by the user's Stop: the process tree is ended.
    stop: Option<super::Stop>,
}

/// Arguments for `claude`: print mode, JSON out, and nothing it could act with. No built-in tools,
/// no MCP servers, the user's and project's settings files (their hooks and allow rules) not
/// loaded, and anything that would ask for permission denied. It reads the prompt and answers.
/// With `mcp`, only Sprawl's own server (read-only tools) is loaded and allowed.
pub(crate) fn args(model: &str, mcp: Option<&Path>) -> Vec<String> {
    // no transcript of the scan saved under ~/.claude, no slash commands or skills
    let mut a: Vec<String> = ["-p", "--output-format", "json", "--tools", "", "--strict-mcp-config", "--restricted", "--permission-prompts", "none", "--no-session-persistence", "--disable-slash-commands"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if let Some(cfg) = mcp {
        a.push("--mcp-config".into());
        a.push(cfg.display().to_string());
        a.push("--allowedTools".into());
        a.push("mcp__sprawl".into());
    }
    if !model.is_empty() && !model.starts_with('-') && model.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
        a.push("--model".into());
        a.push(model.into());
    }
    a
}

/// An empty folder to run it in, so no project CLAUDE.md or settings are picked up.
fn workdir() -> PathBuf {
    let d = super::advisor_dir().map(|d| d.join("claude-code")).unwrap_or_else(std::env::temp_dir);
    let _ = std::fs::create_dir_all(&d);
    d
}

impl ClaudeCode {
    pub fn new(exe: &Path, model: &str, system: &str, first: &str) -> ClaudeCode {
        ClaudeCode { exe: exe.to_path_buf(), model: model.trim().to_string(), mcp: None, prompt: format!("{system}\n\n{first}"), pending: true, stop: None }
    }

    /// End the run when `stop` is set.
    pub fn with_stop(mut self, stop: super::Stop) -> ClaudeCode {
        self.stop = Some(stop);
        self
    }

    /// Give it Sprawl's tools through the MCP config at `cfg` (and nothing else).
    pub fn with_mcp(mut self, cfg: &Path) -> ClaudeCode {
        self.mcp = Some(cfg.to_path_buf());
        self
    }
}

/// What a print-mode run used: its tokens and the cost it reports.
pub(crate) fn usage_of(out: &str) -> super::Usage {
    let v: Value = serde_json::from_str(out.trim()).unwrap_or(Value::Null);
    let u = &v["usage"];
    super::Usage {
        input: u["input_tokens"].as_u64().unwrap_or(0) + u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
        cached: u["cache_read_input_tokens"].as_u64().unwrap_or(0),
        output: u["output_tokens"].as_u64().unwrap_or(0),
        cost_usd: v["total_cost_usd"].as_f64(),
    }
}

/// Parse `claude -p --output-format json` output: `{"type":"result","result":"...","is_error":false}`.
pub(crate) fn parse(out: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(out.trim()).map_err(|_| format!("Claude Code answered something unexpected: {}", super::short(out)))?;
    if v["is_error"].as_bool() == Some(true) {
        return Err(format!("Claude Code: {}", v["result"].as_str().unwrap_or("failed")));
    }
    v["result"].as_str().map(str::to_string).ok_or_else(|| "Claude Code gave no result".into())
}

impl ClaudeCode {
    /// [`Chat::turn`], calling `tick` about every half second while Claude Code works.
    pub fn turn_with(&mut self, tick: &mut dyn FnMut()) -> Result<Turn, String> {
        if !self.pending {
            return Ok(Turn::default());
        }
        self.pending = false;
        log_request(&format!("claude -p ({})", self.exe.display()), &self.prompt);
        // Rust escapes arguments for a .cmd shim itself; the prompt goes on stdin
        let mut cmd = Command::new(&self.exe);
        cmd.args(args(&self.model, self.mcp.as_deref())).current_dir(workdir());
        // stderr isn't read: a full pipe would stall it
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().map_err(|e| format!("couldn't start Claude Code: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        let prompt = self.prompt.clone();
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
        });
        let mut stdout = child.stdout.take().ok_or("no stdout")?;
        let (otx, orx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stdout.read_to_string(&mut s);
            let _ = otx.send(s);
        });
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if self.stop.as_ref().is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed)) => {
                    kill_tree(&mut child);
                    return Err(super::STOPPED.into());
                }
                Ok(None) if start.elapsed() < Duration::from_secs(900) => {
                    std::thread::sleep(Duration::from_millis(500));
                    tick();
                }
                _ => {
                    kill_tree(&mut child);
                    return Err("Claude Code took longer than 15 minutes".into());
                }
            }
        }
        let _ = writer.join();
        // a grandchild still holding the pipe mustn't keep this waiting
        let out = orx.recv_timeout(Duration::from_secs(30)).unwrap_or_default();
        let text = parse(&out)?;
        // each new process gets the whole conversation
        self.prompt.push_str(&format!("\n\nYour answer:\n{text}"));
        Ok(Turn { text, usage: usage_of(&out), ..Turn::default() })
    }
}

impl Chat for ClaudeCode {
    fn turn(&mut self) -> Result<Turn, String> {
        self.turn_with(&mut || {})
    }

    fn answer(&mut self, _: Vec<(String, String, bool)>) {}

    fn say(&mut self, text: &str) {
        self.prompt.push_str(&format!("\n\nThe user:\n{text}"));
        self.pending = true;
    }
}

/// End the process and everything it started (a .cmd shim starts node).
fn kill_tree(child: &mut std::process::Child) {
    let pid = child.id().to_string();
    let taskkill = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join(r"System32\taskkill.exe")).unwrap_or_else(|| "taskkill.exe".into());
    let mut k = Command::new(taskkill);
    k.args(["/T", "/F", "/PID", &pid]).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        k.creation_flags(0x0800_0000);
    }
    let _ = k.status();
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    #[test]
    fn nothing_to_act_with() {
        let a = super::args("claude-opus-5-5", None);
        assert!(!a.contains(&"--mcp-config".to_string()));
        let m = super::args("claude-opus-5-5", Some(std::path::Path::new(r"C:\x\mcp.json")));
        assert_eq!(m[m.iter().position(|x| x == "--allowedTools").unwrap() + 1], "mcp__sprawl", "only Sprawl's tools");
        assert!(m.contains(&"--strict-mcp-config".to_string()));
        let i = a.iter().position(|x| x == "--tools").unwrap();
        assert_eq!(a[i + 1], "", "no built-in tools");
        for flag in ["--strict-mcp-config", "--restricted", "-p", "--no-session-persistence", "--disable-slash-commands"] {
            assert!(a.contains(&flag.to_string()), "{flag}");
        }
        assert_eq!(a[a.iter().position(|x| x == "--permission-prompts").unwrap() + 1], "none");
        assert!(!super::args("--dangerously-skip-permissions", None).contains(&"--model".to_string()), "a model name can't be a flag");
    }

    #[test]
    fn parses_print_mode_output() {
        assert_eq!(super::parse(r#"{"type":"result","is_error":false,"result":"hello"}"#).unwrap(), "hello");
        assert!(super::parse(r#"{"type":"result","is_error":true,"result":"not logged in"}"#).is_err());
        assert!(super::parse("garbage").is_err());
    }
}
