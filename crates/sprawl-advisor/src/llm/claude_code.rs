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
    /// Everything so far: the instructions, the report and the conversation as text.
    prompt: String,
    /// There is something new to answer.
    pending: bool,
}

impl ClaudeCode {
    pub fn new(exe: &Path, model: &str, system: &str, first: &str) -> ClaudeCode {
        ClaudeCode { exe: exe.to_path_buf(), model: model.trim().to_string(), prompt: format!("{system}\n\n{first}"), pending: true }
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

impl Chat for ClaudeCode {
    fn turn(&mut self) -> Result<Turn, String> {
        if !self.pending {
            return Ok(Turn::default());
        }
        self.pending = false;
        log_request(&format!("claude -p ({})", self.exe.display()), &self.prompt);
        // a .cmd shim has to go through cmd; the prompt goes on stdin, so nothing is quoted
        let is_cmd = self.exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd"));
        let mut cmd = if is_cmd {
            let mut c = Command::new(std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into()));
            c.arg("/d").arg("/c").arg(&self.exe);
            c
        } else {
            Command::new(&self.exe)
        };
        cmd.args(["-p", "--output-format", "json"]);
        if !self.model.is_empty() && self.model.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
            cmd.args(["--model", &self.model]);
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
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
        let reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stdout.read_to_string(&mut s);
            s
        });
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if start.elapsed() < Duration::from_secs(900) => std::thread::sleep(Duration::from_millis(200)),
                _ => {
                    let _ = child.kill();
                    return Err("Claude Code took longer than 15 minutes".into());
                }
            }
        }
        let _ = writer.join();
        let out = reader.join().unwrap_or_default();
        let text = parse(&out)?;
        // each new process gets the whole conversation
        self.prompt.push_str(&format!("\n\nYour answer:\n{text}"));
        Ok(Turn { text, ..Turn::default() })
    }

    fn answer(&mut self, _: Vec<(String, String, bool)>) {}

    fn say(&mut self, text: &str) {
        self.prompt.push_str(&format!("\n\nThe user:\n{text}"));
        self.pending = true;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_print_mode_output() {
        assert_eq!(super::parse(r#"{"type":"result","is_error":false,"result":"hello"}"#).unwrap(), "hello");
        assert!(super::parse(r#"{"type":"result","is_error":true,"result":"not logged in"}"#).is_err());
        assert!(super::parse("garbage").is_err());
    }
}
