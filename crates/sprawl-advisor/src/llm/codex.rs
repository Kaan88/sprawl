//! The user's own Codex CLI: `codex exec` with its login, no key needed. Like Claude Code
//! without Sprawl's server, it answers from the text it is given. Codex can't be given an
//! empty tool set, so before every run Sprawl asks it (in runs that send nothing) what the
//! model would be offered, and starts it only when that is the checked set: its tool features
//! off, every MCP server it would start switched off by name, and no tool beyond those that
//! can't act. Its sandbox is read-only besides. The executable itself is trusted code: it
//! runs with Sprawl's own rights, which is why it isn't started as administrator unless
//! allowed.

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

/// The tools Codex 0.170 still offers with every feature above off, by namespace. None can
/// act: `exec` runs code through the code-mode host, which is off (its calls fail with
/// "code-mode host is disabled"), and the rest wait, ask or message within the run, or start
/// a helper agent under the same settings. A tool or namespace not listed here (an MCP
/// server's, a new feature's) is a Codex Sprawl hasn't checked, and it isn't started.
const CHECKED_TOOLS: [(&str, &[&str]); 2] = [
    ("functions", &["exec", "wait", "request_user_input", "request_user_input_async", "send_message_to_user_async"]),
    ("collaboration", &["followup_task", "interrupt_agent", "list_agents", "send_message", "spawn_agent", "wait_agent"]),
];

/// Features in [`OFF`] that Codex 0.170 keeps on whatever it is told; what they would offer
/// is checked through the tool list instead.
const STAYS_ON: [&str; 1] = ["unified_exec"];

/// The settings every run and every check shares: nothing asked, no web search, the tool
/// features off.
fn common() -> Vec<String> {
    let mut a: Vec<String> = ["-c", "approval_policy=\"never\"", "-c", "web_search=\"disabled\""].iter().map(|s| s.to_string()).collect();
    for f in OFF {
        a.push("--disable".into());
        a.push(f.into());
    }
    a
}

/// Arguments for `codex exec`: JSON events out, the prompt on stdin, no session saved, the
/// user's config.toml (their MCP servers, hooks, profiles) and rules files not loaded, a
/// read-only sandbox, the [`common`] settings, and `servers_off` (`-c
/// mcp_servers.<name>.enabled=false` for each server the check found).
pub(crate) fn args(model: &str, dir: &Path, servers_off: &[String]) -> Vec<String> {
    let mut a: Vec<String> = ["exec", "--json", "--color", "never", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral", "--ignore-user-config", "--ignore-rules"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    a.extend(common());
    a.extend(servers_off.iter().cloned());
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

/// A folder made for one run (none existed by its name), checked to be a plain empty folder,
/// and removed with what Codex left in it when the run is over.
struct Fresh(PathBuf);

impl Drop for Fresh {
    fn drop(&mut self) {
        // links inside are removed as links, never followed
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A real folder: not a link or junction, and not a file.
fn plain_dir(p: &Path) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    let m = std::fs::symlink_metadata(p).map_err(|e| format!("{} couldn't be checked ({e})", p.display()))?;
    if !m.is_dir() || m.file_attributes() & 0x400 != 0 {
        return Err(format!("{} isn't a plain folder", p.display()));
    }
    Ok(())
}

/// Where the run folders go: Sprawl's own advisor folder, never Temp.
fn base() -> Result<PathBuf, String> {
    let advisor = super::advisor_dir().ok_or("this account's AppData folder isn't known")?;
    let base = advisor.join("codex");
    std::fs::create_dir_all(&base).map_err(|e| format!("couldn't make {} ({e})", base.display()))?;
    plain_dir(&advisor)?;
    plain_dir(&base)?;
    Ok(base)
}

/// A new, empty folder of `base`, made by this call alone.
fn fresh(base: &Path, what: &str) -> Result<Fresh, String> {
    for n in 0..100u32 {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let d = base.join(format!("{what}-{}-{nanos}-{n}", std::process::id()));
        match std::fs::create_dir(&d) {
            Ok(()) => {
                let f = Fresh(d);
                plain_dir(&f.0)?;
                if std::fs::read_dir(&f.0).map_err(|e| e.to_string())?.next().is_some() {
                    return Err(format!("{} wasn't empty", f.0.display()));
                }
                return Ok(f);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("couldn't make a folder for Codex ({e})")),
        }
    }
    Err("couldn't make a folder for Codex".into())
}

/// The Codex home folder its login is in: `CODEX_HOME`, or `.codex` in the user's folder.
fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".codex")))
}

/// One of Codex's own commands that sends nothing (`mcp list`, `features list`, `debug
/// prompt-input`), run in `dir` with `home` as its Codex home: what it printed, or why not.
fn probe(exe: &Path, dir: &Path, home: &Path, sub: &[&str], extra: &[String], tail: &[&str]) -> Result<String, String> {
    use std::io::Read;
    let mut cmd = Command::new(exe);
    cmd.args(sub).args(common()).args(extra).args(tail).current_dir(dir).env("CODEX_HOME", home);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut run = super::contain::spawn(&mut cmd, 0x0800_0000).map_err(|e| format!("couldn't start Codex ({e})"))?;
    let mut out = run.child.stdout.take().ok_or("no stdout")?;
    let mut err = run.child.stderr.take().ok_or("no stderr")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = (&mut out).take(8 << 20).read_to_end(&mut v);
        let _ = tx.send(v);
    });
    let (etx, erx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = (&mut err).take(64 << 10).read_to_end(&mut v);
        let _ = etx.send(String::from_utf8_lossy(&v).lines().find(|l| l.starts_with("Error")).unwrap_or("").to_string());
    });
    let got = rx.recv_timeout(Duration::from_secs(30));
    let waited = Instant::now();
    let status = loop {
        match run.child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if waited.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(50)),
            _ => break None,
        }
    };
    run.end();
    let got = got.map_err(|_| "Codex didn't finish a check in time".to_string())?;
    match status {
        Some(s) if s.success() => Ok(String::from_utf8_lossy(&got).into_owned()),
        Some(s) => Err(match erx.recv_timeout(Duration::from_secs(2)).ok().filter(|l| !l.is_empty()) {
            Some(l) => super::short(l.trim_start_matches("Error:").trim()),
            None => format!("a check ended with an error ({s})"),
        }),
        None => Err("Codex didn't finish a check in time".into()),
    }
}

/// The servers `codex mcp list --json` printed: (name, enabled). A name that couldn't be
/// switched off by name in a setting is refused.
fn parse_servers(out: &str) -> Result<Vec<(String, bool)>, String> {
    let t = out.trim();
    if t.is_empty() {
        return Ok(Vec::new());
    }
    let v: Value = serde_json::from_str(t).map_err(|_| "its list of MCP servers couldn't be read".to_string())?;
    let list = v.as_array().ok_or("its list of MCP servers couldn't be read")?;
    list.iter()
        .map(|s| {
            let name = s["name"].as_str().ok_or("an MCP server without a name")?;
            if name.is_empty() || name.len() > 64 || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                return Err(format!("an MCP server named {:?}, which can't be switched off by name", super::short(name)));
            }
            Ok((name.to_string(), s["enabled"].as_bool().ok_or("an MCP server whose state isn't given")?))
        })
        .collect()
}

/// `codex features list`: each feature in [`OFF`] is known to this Codex and off (bar
/// [`STAYS_ON`]).
fn check_features(out: &str) -> Result<(), String> {
    for f in OFF {
        let line = out.lines().map(|l| l.split_whitespace().collect::<Vec<_>>()).find(|w| w.first() == Some(&f));
        match line.as_deref().and_then(|w| w.last().copied()) {
            Some("false") => {}
            Some(_) if STAYS_ON.contains(&f) => {}
            Some(_) => return Err(format!("its feature {f} stays on")),
            None => return Err(format!("this version doesn't list its feature {f}")),
        }
    }
    Ok(())
}

/// `codex debug prompt-input`: the tools the model would be offered are within
/// [`CHECKED_TOOLS`].
fn check_tools(out: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(out.trim()).map_err(|_| "what it would offer the model couldn't be read".to_string())?;
    let items = v.as_array().ok_or("what it would offer the model couldn't be read")?;
    for item in items {
        let Some(tools) = item.get("tools") else { continue };
        for ns in tools.as_array().ok_or("its tool list couldn't be read")? {
            let name = ns["name"].as_str().unwrap_or("");
            let allowed = CHECKED_TOOLS.iter().find(|(n, _)| *n == name).filter(|_| ns["type"] == "namespace");
            let Some((_, allowed)) = allowed else { return Err(format!("it would offer the model tools Sprawl hasn't checked ({})", super::short(name))) };
            for t in ns["tools"].as_array().ok_or("its tool list couldn't be read")? {
                let tool = t["name"].as_str().unwrap_or("");
                if !allowed.contains(&tool) {
                    return Err(format!("it would offer the model a tool Sprawl hasn't checked ({name}.{})", super::short(tool)));
                }
            }
        }
    }
    Ok(())
}

/// Before a run: what this Codex, in this folder, would start and offer, asked with a home
/// folder of its own (`home`, without the user's config.toml, as the run skips it). Every MCP
/// server it lists is switched off by name and must then list as off; the tool features must
/// be off and the tools offered within [`CHECKED_TOOLS`]. The settings that switch the
/// servers off, for the run; or why it isn't started.
fn verify(exe: &Path, dir: &Path, home: &Path) -> Result<Vec<String>, String> {
    // a configuration an administrator put in the login's home isn't seen by the check
    if let Some(h) = codex_home() {
        for f in ["managed_config.toml", "requirements.toml"] {
            if h.join(f).exists() {
                return Err(format!("a managed configuration ({}) Sprawl can't check", h.join(f).display()));
            }
        }
    }
    let listed = parse_servers(&probe(exe, dir, home, &["mcp", "list", "--json"], &[], &[])?)?;
    let off: Vec<String> = listed.iter().flat_map(|(n, _)| ["-c".to_string(), format!("mcp_servers.{n}.enabled=false")]).collect();
    let again = parse_servers(&probe(exe, dir, home, &["mcp", "list", "--json"], &off, &[])?)?;
    if let Some((n, _)) = again.iter().find(|(n, on)| *on || !listed.iter().any(|(l, _)| l == n)) {
        return Err(format!("its MCP server {n} couldn't be switched off"));
    }
    check_features(&probe(exe, dir, home, &["features", "list"], &off, &[])?)?;
    check_tools(&probe(exe, dir, home, &["debug", "prompt-input"], &off, &["x"])?)?;
    Ok(off)
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
    let mut completed = false;
    for line in out.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        match v["type"].as_str() {
            Some("item.completed") if v["item"]["type"] == "agent_message" => text = v["item"]["text"].as_str().map(str::to_string),
            Some("turn.completed") => {
                completed = true;
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
        Some(t) if completed => Ok((t, usage)),
        Some(_) => Err("Codex stopped before finishing its answer".into()),
        None if out.trim().is_empty() => Err("Codex gave no answer. Is it signed in? Run codex login once.".into()),
        None => Err(format!("Codex answered something unexpected: {}", super::short(out))),
    }
}

/// The output ends with the turn's own last event, whole (a parsed line, not a mention).
fn finished(out: &str) -> bool {
    out.ends_with('\n') && out.lines().rev().find(|l| !l.trim().is_empty()).and_then(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .is_some_and(|v| matches!(v["type"].as_str(), Some("turn.completed" | "turn.failed")))
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
        // a folder of its own, and the check, before anything is sent; both folders go when
        // this returns, after the run (declared before it, so dropped after it ends)
        let base = base().map_err(|e| format!("Codex isn't started: {e}"))?;
        let dir = fresh(&base, "run").map_err(|e| format!("Codex isn't started: {e}"))?;
        let home = fresh(&base, "check").map_err(|e| format!("Codex isn't started: {e}"))?;
        let off = verify(&self.exe, &dir.0, &home.0).map_err(|e| format!("Codex isn't started, as it isn't a set-up Sprawl has checked: {e}"))?;
        drop(home);
        log_request(&format!("codex exec ({})", self.exe.display()), &self.prompt);
        let mut cmd = Command::new(&self.exe);
        cmd.args(args(&self.model, &dir.0, &off)).current_dir(&dir.0);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // in a job with everything it starts: Stop, the time limit and every return end them all
        // (0x0800_0000: CREATE_NO_WINDOW)
        let mut run = super::contain::spawn(&mut cmd, 0x0800_0000).map_err(|e| format!("couldn't start Codex: {e}"))?;
        let mut stdin = run.child.stdin.take().ok_or("no stdin")?;
        let prompt = self.prompt.clone();
        // never waited for: a helper holding stdin without reading it can't keep this waiting
        std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
        });
        // kept as it comes (at most 16 MB): a helper process that outlives Codex and still holds
        // the pipe mustn't keep its answer from being read
        let mut stdout = run.child.stdout.take().ok_or("no stdout")?;
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let (otx, orx) = std::sync::mpsc::channel::<()>();
        let g2 = got.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = std::io::Read::read(&mut stdout, &mut chunk) {
                if n == 0 {
                    break;
                }
                if let Ok(mut g) = g2.lock() {
                    if g.len() < 16 << 20 {
                        g.extend_from_slice(&chunk[..n]);
                    }
                }
            }
            let _ = otx.send(());
        });
        // its log lines, read as they come so a full pipe can't stall it (the first 64 KB kept):
        // the "Error:" line says why when it refused to start (an option this version doesn't know)
        let mut stderr = run.child.stderr.take().ok_or("no stderr")?;
        let (etx, erx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let (mut kept, mut chunk) = (Vec::new(), [0u8; 8192]);
            while let Ok(n) = std::io::Read::read(&mut stderr, &mut chunk) {
                if n == 0 {
                    break;
                }
                if kept.len() < 64 << 10 {
                    kept.extend_from_slice(&chunk[..n]);
                }
            }
            let s = String::from_utf8_lossy(&kept).into_owned();
            let _ = etx.send(s.lines().find(|l| l.starts_with("Error")).unwrap_or("").to_string());
        });
        let start = Instant::now();
        let status = loop {
            match run.child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if self.stop.as_ref().is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed)) => {
                    run.end();
                    return Err(super::STOPPED.into());
                }
                Ok(None) if start.elapsed() < Duration::from_secs(900) => {
                    std::thread::sleep(Duration::from_millis(500));
                    tick();
                }
                _ => {
                    run.end();
                    return Err("Codex took longer than 15 minutes".into());
                }
            }
        };
        // the end of the output, or the turn's last event, whichever comes first (30 s at most)
        let text_now = || got.lock().map(|g| String::from_utf8_lossy(&g).into_owned()).unwrap_or_default();
        let waited = Instant::now();
        while orx.recv_timeout(Duration::from_millis(100)).is_err() && waited.elapsed() < Duration::from_secs(30) {
            if finished(&text_now()) {
                break;
            }
        }
        // whatever it started ends now, which closes the pipes it held
        run.end();
        let out = text_now();
        let error_line = || erx.recv_timeout(Duration::from_secs(2)).ok().filter(|l| !l.is_empty());
        let (text, usage) = match parse(&out) {
            // an answer counts only with the turn's end and a clean exit
            Ok(_) if !status.success() => {
                return Err(match error_line() {
                    Some(l) => format!("Codex: {}", super::short(l.trim_start_matches("Error:").trim())),
                    None => format!("Codex stopped with an error ({status})"),
                });
            }
            Ok(r) => r,
            Err(e) => {
                return Err(match error_line() {
                    Some(l) if out.trim().is_empty() => format!("Codex: {}", super::short(l.trim_start_matches("Error:").trim())),
                    _ => e,
                });
            }
        };
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
        let a = super::args("gpt-6-luna", dir, &["-c".into(), "mcp_servers.node_repl.enabled=false".into()]);
        assert!(a.windows(2).any(|w| w[0] == "-c" && w[1] == "mcp_servers.node_repl.enabled=false"), "the servers the check found are switched off");
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
        assert!(!super::args("--dangerously-bypass-approvals-and-sandbox", dir, &[]).contains(&"-m".to_string()), "a model name can't be a flag");
        assert!(!super::args("", dir, &[]).contains(&"-m".to_string()), "no model: Codex picks its default");
    }

    #[test]
    fn only_a_checked_codex_starts() {
        // Astra 1.0.2 S5 #1: servers by name, features off, tools within the checked set
        assert_eq!(super::parse_servers("").unwrap(), vec![]);
        assert_eq!(super::parse_servers(r#"[{"name":"node_repl","enabled":true},{"name":"x-y_1","enabled":false}]"#).unwrap(), vec![("node_repl".to_string(), true), ("x-y_1".to_string(), false)]);
        assert!(super::parse_servers(r#"[{"name":"a.b","enabled":true}]"#).is_err(), "a name a setting can't address");
        assert!(super::parse_servers(r#"[{"name":"a","enabled":"yes"}]"#).is_err());
        assert!(super::parse_servers("WARNING: something").is_err());
        let all_off: String = super::OFF.iter().map(|f| format!("{f:<40} stable  {}\n", if super::STAYS_ON.contains(f) { "true" } else { "false" })).collect();
        assert!(super::check_features(&all_off).is_ok());
        assert!(super::check_features(&all_off.replace("shell_tool                               stable  false", "shell_tool                               stable  true")).unwrap_err().contains("shell_tool"));
        assert!(super::check_features(&all_off.replace("hooks", "hooks_renamed")).unwrap_err().contains("doesn't list"));
        let tools = |extra: &str| format!(r#"[{{"type":"additional_tools","tools":[{{"type":"namespace","name":"functions","tools":[{{"type":"custom","name":"exec"}},{{"type":"function","name":"wait"}}{extra}]}}]}},{{"type":"message","role":"user","content":[]}}]"#);
        assert!(super::check_tools(&tools("")).is_ok());
        assert!(super::check_tools(&tools(r#",{"type":"function","name":"shell"}"#)).unwrap_err().contains("functions.shell"));
        assert!(super::check_tools(r#"[{"type":"additional_tools","tools":[{"type":"namespace","name":"mcp__node_repl","tools":[]}]}]"#).is_err(), "an MCP server's tools");
        assert!(super::check_tools(r#"[{"type":"additional_tools","tools":[{"type":"function","name":"functions","tools":[]}]}]"#).is_err(), "only namespaces");
        assert!(super::check_tools("not json").is_err());
        assert!(super::check_tools("[]").is_ok(), "no tools at all");
    }

    #[test]
    fn each_run_has_a_folder_of_its_own() {
        let base = std::env::temp_dir().join(format!("sprawl-codex-fresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let a = super::fresh(&base, "run").unwrap();
        let b = super::fresh(&base, "run").unwrap();
        assert_ne!(a.0, b.0);
        std::fs::write(a.0.join("left.txt"), b"x").unwrap();
        let (pa, pb) = (a.0.clone(), b.0.clone());
        drop(a);
        drop(b);
        assert!(!pa.exists() && !pb.exists(), "removed after the run");
        assert!(super::plain_dir(&base.join("missing")).is_err());
        std::fs::write(base.join("file"), b"x").unwrap();
        assert!(super::plain_dir(&base.join("file")).is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// The Codex installed here passes the check (sends nothing; run by hand with --ignored).
    #[test]
    #[ignore]
    fn the_installed_codex_is_checked() {
        let exe = crate::config::Detected::probe().codex.expect("no Codex here");
        let base = super::base().unwrap();
        let dir = super::fresh(&base, "run").unwrap();
        let home = super::fresh(&base, "check").unwrap();
        let off = super::verify(&exe, &dir.0, &home.0);
        eprintln!("{exe:?}: {off:?}");
        assert!(off.is_ok());
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
        // an answer without the turn's end is not an answer
        assert!(super::parse(r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"half"}}"#).unwrap_err().contains("before finishing"));
        // the end is a whole, parsed last line, not a mention of it
        assert!(super::finished(&format!("{out}\n")));
        assert!(!super::finished(out), "a line not yet ended");
        assert!(!super::finished(concat!(r#"{"type":"item.completed","item":{"type":"agent_message","text":"\"turn.completed\""}}"#, "\n")));
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
