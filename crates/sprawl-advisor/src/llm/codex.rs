//! The user's own Codex CLI: `codex exec` with its login, no key needed. Like Claude Code
//! without Sprawl's server, it answers from the text it is given: every tool Codex has is
//! switched off, and its sandbox is read-only besides.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{log_request, Chat, Turn};

pub struct Codex {
    exe: PathBuf,
    model: String,
    /// Everything so far: the instructions, the report and the conversation as text.
    prompt: String,
    /// There is something new to answer.
    pending: bool,
    /// Set by the user's Stop: the process tree is ended.
    stop: Option<super::Stop>,
    /// Its answer, once written (Codex sends a message whole, not word by word).
    sink: Option<super::Sink>,
}

/// What Codex could act with, switched off: its shell and command runner, code mode (through
/// which its other tools are called: with its host off they fail closed), apps, plugins,
/// browser and computer use, image generation, hooks, sub-agents, memories, goals, skills and
/// tool suggestions. Checked with codex-cli 0.170: every call it tried failed with "code-mode
/// host is disabled".
const OFF: [&str; 16] = [
    "shell_tool", "unified_exec", "code_mode_host", "apps", "plugins", "browser_use", "computer_use", "image_generation", "hooks", "multi_agent", "memories",
    "goals", "sleep_tool", "tool_suggest", "skill_search", "in_app_browser",
];

/// Arguments for `codex exec`: JSON events out, the prompt on stdin, no session saved, the
/// user's config.toml (their MCP servers, hooks, profiles) and rules files not loaded, a
/// read-only sandbox, nothing asked, no web search, and every tool feature off.
pub(crate) fn args(model: &str, dir: &Path) -> Vec<String> {
    let mut a: Vec<String> = [
        "exec", "--json", "--color", "never", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral", "--ignore-user-config", "--ignore-rules",
        "-c", "approval_policy=\"never\"", "-c", "web_search=\"disabled\"",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for f in OFF {
        a.push("--disable".into());
        a.push(f.into());
    }
    a.push("-C".into());
    a.push(dir.display().to_string());
    if !model.is_empty() && !model.starts_with('-') && model.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_') {
        a.push("-m".into());
        a.push(model.into());
    }
    // the prompt comes on stdin
    a.push("-".into());
    a
}

/// An empty folder to run it in, so no project AGENTS.md or config is picked up.
fn workdir() -> PathBuf {
    let d = super::advisor_dir().map(|d| d.join("codex")).unwrap_or_else(std::env::temp_dir);
    let _ = std::fs::create_dir_all(&d);
    d
}

/// The models Codex lists, from its own cache (`%USERPROFILE%\.codex\models_cache*.json`), in
/// its order. Empty when there is none: Codex then picks its default.
pub fn models() -> Vec<String> {
    let Some(home) = std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".codex")) else { return Vec::new() };
    let Ok(dir) = std::fs::read_dir(&home) else { return Vec::new() };
    let mut out: Vec<(i64, String)> = Vec::new();
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if !(name.starts_with("models_cache") && name.ends_with(".json")) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        out.extend(parse_models(&text));
    }
    out.sort_by_key(|(p, _)| *p);
    let mut seen = std::collections::HashSet::new();
    out.into_iter().map(|(_, s)| s).filter(|s| seen.insert(s.clone())).collect()
}

/// The listed models in a Codex model cache: (priority, slug), hidden ones left out.
fn parse_models(text: &str) -> Vec<(i64, String)> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or(Value::Null);
    v["models"]
        .as_array()
        .map(|ms| {
            ms.iter()
                .filter(|m| m["visibility"].as_str() == Some("list"))
                .filter_map(|m| Some((m["priority"].as_i64().unwrap_or(i64::MAX), m["slug"].as_str()?.to_string())))
                .filter(|(_, s)| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_'))
                .collect()
        })
        .unwrap_or_default()
}

impl Codex {
    pub fn new(exe: &Path, model: &str, system: &str, first: &str) -> Codex {
        Codex { exe: exe.to_path_buf(), model: model.trim().to_string(), prompt: format!("{system}\n\n{first}"), pending: true, stop: None, sink: None }
    }

    /// End the run when `stop` is set.
    pub fn with_stop(mut self, stop: super::Stop) -> Codex {
        self.stop = Some(stop);
        self
    }
}

/// Codex's events (`codex exec --json`, one JSON object a line): its last message is the
/// answer, `turn.completed` carries the tokens, and a failed turn or a top-level error is the
/// failure. Items of type "error" are warnings (a setting it ignores) and are skipped.
pub(crate) fn parse(out: &str) -> Result<(String, super::Usage), String> {
    let mut text: Option<String> = None;
    let mut usage = super::Usage::default();
    let mut failed: Option<String> = None;
    for line in out.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        match v["type"].as_str() {
            Some("item.completed") if v["item"]["type"] == "agent_message" => text = v["item"]["text"].as_str().map(str::to_string),
            Some("turn.completed") => {
                let u = &v["usage"];
                let cached = u["cached_input_tokens"].as_u64().unwrap_or(0);
                usage = super::Usage { input: u["input_tokens"].as_u64().unwrap_or(0).saturating_sub(cached), cached, output: u["output_tokens"].as_u64().unwrap_or(0), cost_usd: None };
            }
            Some("turn.failed") => failed = Some(v["error"]["message"].as_str().unwrap_or("failed").to_string()),
            Some("error") => failed = Some(v["message"].as_str().unwrap_or("failed").to_string()),
            _ => {}
        }
    }
    if let Some(why) = failed {
        return Err(format!("Codex: {}", super::short(&why)));
    }
    match text {
        Some(t) => Ok((t, usage)),
        None if out.trim().is_empty() => Err("Codex gave no answer. Is it signed in? Run codex login once.".into()),
        None => Err(format!("Codex answered something unexpected: {}", super::short(out))),
    }
}

/// Why Codex isn't started, when Sprawl runs as administrator: like Claude Code it comes from
/// your own folders and would run as administrator. The same switch allows both.
pub fn elevated_refusal(elevated: bool, allowed: bool) -> Option<&'static str> {
    (elevated && !allowed).then_some("Not as administrator: Codex, found in your own folders, would run as administrator. You can allow it in Settings > Scanning, or restart Sprawl normally.")
}

impl Codex {
    /// [`Chat::turn`], calling `tick` about every half second while Codex works.
    pub fn turn_with(&mut self, tick: &mut dyn FnMut()) -> Result<Turn, String> {
        if !self.pending {
            return Ok(Turn::default());
        }
        self.pending = false;
        // an elevation Windows can't confirm counts as elevated: the switch then has to allow it
        if let Some(why) = elevated_refusal(sprawl_core::elevation().unwrap_or(true), crate::ELEVATED_CLAUDE_CODE.load(std::sync::atomic::Ordering::Relaxed)) {
            return Err(why.into());
        }
        if crate::offline() {
            return Err(crate::OFFLINE_MSG.into());
        }
        log_request(&format!("codex exec ({})", self.exe.display()), &self.prompt);
        let dir = workdir();
        let mut cmd = Command::new(&self.exe);
        cmd.args(args(&self.model, &dir)).current_dir(&dir);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().map_err(|e| format!("couldn't start Codex: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        let prompt = self.prompt.clone();
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
        });
        // kept as it comes: a helper process that outlives Codex and still holds the pipe
        // mustn't keep its answer from being read
        let mut stdout = child.stdout.take().ok_or("no stdout")?;
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let (otx, orx) = std::sync::mpsc::channel::<()>();
        let g2 = got.clone();
        let reader = std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = std::io::Read::read(&mut stdout, &mut chunk) {
                if n == 0 {
                    break;
                }
                if let Ok(mut g) = g2.lock() {
                    g.extend_from_slice(&chunk[..n]);
                }
            }
            let _ = otx.send(());
        });
        // its log lines, read as they come so a full pipe can't stall it: the "Error:" line
        // says why when it refused to start (an option this version doesn't know)
        let mut stderr = child.stderr.take().ok_or("no stderr")?;
        let (etx, erx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut stderr, &mut s);
            let _ = etx.send(s.lines().find(|l| l.starts_with("Error")).unwrap_or("").to_string());
        });
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if self.stop.as_ref().is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed)) => {
                    super::claude_code::kill_tree(&mut child);
                    return Err(super::STOPPED.into());
                }
                Ok(None) if start.elapsed() < Duration::from_secs(900) => {
                    std::thread::sleep(Duration::from_millis(500));
                    tick();
                }
                _ => {
                    super::claude_code::kill_tree(&mut child);
                    return Err("Codex took longer than 15 minutes".into());
                }
            }
        }
        let _ = writer.join();
        // the end of the output, or the turn's last event, whichever comes first (30 s at most)
        let text_now = || got.lock().map(|g| String::from_utf8_lossy(&g).into_owned()).unwrap_or_default();
        let waited = Instant::now();
        while orx.recv_timeout(Duration::from_millis(100)).is_err() && waited.elapsed() < Duration::from_secs(30) {
            let now = text_now();
            // (and that line whole)
            if (now.contains("\"turn.completed\"") || now.contains("\"turn.failed\"")) && now.ends_with('\n') {
                break;
            }
        }
        let out = text_now();
        drop(reader);
        let (text, usage) = parse(&out).map_err(|e| match erx.recv_timeout(Duration::from_secs(2)).ok().filter(|l| !l.is_empty()) {
            Some(l) if out.trim().is_empty() => format!("Codex: {}", super::short(l.trim_start_matches("Error:").trim())),
            _ => e,
        })?;
        if let Some(s) = self.sink.as_mut() {
            s(&text);
        }
        // each new process gets the whole conversation
        self.prompt.push_str(&format!("\n\nYour answer:\n{text}"));
        Ok(Turn { text, usage, ..Turn::default() })
    }
}

impl Chat for Codex {
    fn turn(&mut self) -> Result<Turn, String> {
        self.turn_with(&mut || {})
    }

    fn turn_ticking(&mut self, tick: &mut dyn FnMut()) -> Result<Turn, String> {
        self.turn_with(tick)
    }

    fn answer(&mut self, _: Vec<(String, String, bool)>) {}

    fn stream_to(&mut self, sink: Option<super::Sink>) {
        self.sink = sink;
    }

    fn say(&mut self, text: &str) {
        self.prompt.push_str(&format!("\n\nThe user:\n{text}"));
        self.pending = true;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn as_administrator_only_when_allowed() {
        assert!(super::elevated_refusal(true, false).is_some_and(|w| w.contains("Settings > Scanning")));
        assert_eq!(super::elevated_refusal(true, true), None);
        assert_eq!(super::elevated_refusal(false, false), None);
    }

    #[test]
    fn nothing_to_act_with() {
        let dir = std::path::Path::new(r"C:\x\codex");
        let a = super::args("gpt-6-luna", dir);
        let after = |flag: &str| a[a.iter().position(|x| x == flag).unwrap() + 1].clone();
        assert_eq!(a[0], "exec");
        assert_eq!(after("--sandbox"), "read-only");
        assert_eq!(after("-C"), r"C:\x\codex");
        assert_eq!(after("-m"), "gpt-6-luna");
        assert_eq!(a.last().unwrap(), "-", "the prompt comes on stdin");
        for flag in ["--ephemeral", "--ignore-user-config", "--ignore-rules", "--skip-git-repo-check", "--json"] {
            assert!(a.contains(&flag.to_string()), "{flag}");
        }
        for c in ["approval_policy=\"never\"", "web_search=\"disabled\""] {
            assert!(a.contains(&c.to_string()), "{c}");
        }
        for f in ["shell_tool", "unified_exec", "code_mode_host", "apps", "plugins", "computer_use", "hooks"] {
            assert!(a.windows(2).any(|w| w[0] == "--disable" && w[1] == f), "{f} is off");
        }
        assert!(!a.iter().any(|x| x.contains("dangerously")));
        assert!(!super::args("--dangerously-bypass-approvals-and-sandbox", dir).contains(&"-m".to_string()), "a model name can't be a flag");
        assert!(!super::args("", dir).contains(&"-m".to_string()), "no model: Codex picks its default");
    }

    #[test]
    fn parses_exec_events() {
        let out = r#"{"type":"thread.started","thread_id":"t"}
{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Codex is ignoring 1 unrecognized configuration setting."}}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"Looking."}}
{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"ready"}}
{"type":"turn.completed","usage":{"input_tokens":11611,"cached_input_tokens":611,"output_tokens":5}}"#;
        let (text, usage) = super::parse(out).unwrap();
        assert_eq!(text, "ready", "the last message is the answer; error items are warnings");
        assert_eq!((usage.input, usage.cached, usage.output), (11000, 611, 5));
        assert!(super::parse(r#"{"type":"turn.failed","error":{"message":"not signed in"}}"#).unwrap_err().contains("not signed in"));
        assert!(super::parse("").unwrap_err().contains("codex login"));
        assert!(super::parse("garbage").is_err());
    }

    #[test]
    fn lists_the_models_codex_lists() {
        let cache = r#"{"models":[{"slug":"b-model","visibility":"list","priority":2},{"slug":"hidden","visibility":"hide","priority":0},{"slug":"a-model","visibility":"list","priority":1},{"slug":"bad name","visibility":"list","priority":3}]}"#;
        let mut m = super::parse_models(cache);
        m.sort();
        assert_eq!(m, vec![(1, "a-model".to_string()), (2, "b-model".to_string())]);
        assert!(super::parse_models("not json").is_empty());
    }
}
