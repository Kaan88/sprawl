//! Which AI the advisor talks to, where its key lives, and what is already on this machine.
//!
//! The choice (provider, model, base URL) is kept in `%APPDATA%\Sprawl\advisor\ai.toml`.
//! API keys never touch that file: they go to Windows Credential Manager under
//! `Sprawl/<provider>` (per user, protected by Windows).

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    /// Rules only, nothing leaves the machine.
    #[default]
    Off,
    /// Anthropic Messages API with the user's key.
    Anthropic,
    /// The user's own Claude Code (`claude -p`), with its login.
    ClaudeCode,
    /// Any OpenAI-compatible endpoint: OpenAI, OpenRouter, Ollama, LM Studio, vLLM.
    OpenAi,
    /// The user's own Codex CLI (`codex exec`), with its login.
    Codex,
}

impl Provider {
    pub const ALL: [Provider; 5] = [Provider::Off, Provider::Anthropic, Provider::ClaudeCode, Provider::Codex, Provider::OpenAi];

    pub fn label(self) -> &'static str {
        match self {
            Provider::Off => "Off: rules only, nothing is sent anywhere",
            Provider::Anthropic => "Claude (Anthropic API key)",
            Provider::ClaudeCode => "Claude Code (uses your claude login)",
            Provider::OpenAi => "OpenAI-compatible (OpenAI, OpenRouter, Ollama, LM Studio)",
            Provider::Codex => "Codex (uses your codex login)",
        }
    }

    pub fn needs_key(self) -> bool {
        matches!(self, Provider::Anthropic | Provider::OpenAi)
    }

    /// The Credential Manager name the app used before it was renamed from DiskTree.
    fn old_credential(self) -> &'static str {
        match self {
            Provider::Anthropic => "DiskTree/anthropic",
            _ => "DiskTree/openai-compatible",
        }
    }

    fn credential(self) -> &'static str {
        match self {
            Provider::Anthropic => "Sprawl/anthropic",
            _ => "Sprawl/openai-compatible",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Anthropic => "claude-opus-5-5",
            // the advisor's work is mostly reading lists; Haiku does it fast and costs the least
            Provider::ClaudeCode => "claude-haiku-5-5",
            // Codex picks its own default unless one is chosen
            Provider::OpenAi | Provider::Codex => "",
            Provider::Off => "",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AiConfig {
    pub provider: Provider,
    #[serde(default)]
    pub model: String,
    /// OpenAI-compatible only, e.g. `http://localhost:11434/v1`.
    #[serde(default)]
    pub base_url: String,
    /// "Let the AI search the web (asks each time)": off unless the user turns it on. Off, the
    /// chat isn't offered a web search at all.
    #[serde(default)]
    pub web_lookup: bool,
}

fn dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Sprawl").join("advisor"))
}

impl AiConfig {
    pub fn load() -> AiConfig {
        dir()
            .and_then(|d| std::fs::read_to_string(d.join("ai.toml")).ok())
            .and_then(|t| toml::from_str(t.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let d = dir().ok_or_else(|| std::io::Error::other("APPDATA is not set"))?;
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        crate::write_local(&d.join("ai.toml"), text.as_bytes(), true)
    }
}

// ------------------------------------------------------------------ Credential Manager

#[cfg(windows)]
mod cred {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::Security::Credentials::{CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn write(target: &str, secret: &str) -> bool {
        let mut t = wide(target);
        let mut user = wide("Sprawl");
        let blob = secret.as_bytes();
        let c = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: t.as_mut_ptr(),
            Comment: std::ptr::null_mut(),
            LastWritten: FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 },
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: std::ptr::null_mut(),
            UserName: user.as_mut_ptr(),
        };
        unsafe { CredWriteW(&c, 0) != 0 }
    }

    pub fn read(target: &str) -> Option<String> {
        let t = wide(target);
        let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        unsafe {
            if CredReadW(t.as_ptr(), CRED_TYPE_GENERIC, 0, &mut p) == 0 || p.is_null() {
                return None;
            }
            let c = &*p;
            // an empty credential may come back with a null blob
            let out = if c.CredentialBlob.is_null() || c.CredentialBlobSize == 0 {
                None
            } else {
                String::from_utf8(std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()).ok()
            };
            CredFree(p as *const _);
            out
        }
    }

    pub fn delete(target: &str) -> bool {
        let t = wide(target);
        unsafe { CredDeleteW(t.as_ptr(), CRED_TYPE_GENERIC, 0) != 0 }
    }
}

/// Store (or with an empty string, remove) the API key for a provider, for the server at
/// `base_url` (OpenAI-compatible: the key is only ever sent to that host).
pub fn set_key(p: Provider, key: &str, base_url: &str) -> bool {
    let key = key.trim();
    let host_target = format!("{}/host", p.credential());
    if key.is_empty() {
        cred::delete(p.credential());
        cred::delete(&host_target);
        return true;
    }
    let host = crate::llm::agent::host_of(base_url).unwrap_or_default();
    cred::write(p.credential(), key) && cred::write(&host_target, &host)
}

/// The host a provider's key was saved for, when that was recorded.
pub fn key_host(p: Provider) -> Option<String> {
    cred::read(&format!("{}/host", p.credential())).filter(|h| !h.is_empty())
}

pub fn key(p: Provider) -> Option<String> {
    if let Some(k) = cred::read(p.credential()) {
        return Some(k);
    }
    // a key saved under the app's old name moves over to the new one
    let k = cred::read(p.old_credential())?;
    if cred::write(p.credential(), &k) {
        cred::delete(p.old_credential());
    }
    Some(k)
}

/// The app was called DiskTree before 0.1.0: move its settings, advisor config and scan history
/// to the new folders, unless the new ones already exist.
pub fn migrate_old_folders() {
    for var in ["APPDATA", "LOCALAPPDATA"] {
        let Some(base) = std::env::var_os(var).map(PathBuf::from) else { continue };
        let (old, new) = (base.join("DiskTree"), base.join("Sprawl"));
        if old.is_dir() && !new.exists() {
            let _ = std::fs::rename(&old, &new);
        }
    }
}

pub fn has_key(p: Provider) -> bool {
    key(p).is_some_and(|k| !k.is_empty())
}

// ------------------------------------------------------------------ detection

/// What the setup wizard can offer without the user typing anything.
#[derive(Clone, Debug, Default)]
pub struct Detected {
    /// `claude` on PATH.
    pub claude_code: Option<PathBuf>,
    /// `codex` on PATH, or the one the Codex app keeps for itself.
    pub codex: Option<PathBuf>,
    pub ollama: bool,
    pub lm_studio: bool,
    pub anthropic_env: bool,
    pub openai_env: bool,
}

fn listening(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok()
}

fn on_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    // only absolute folders: an empty or relative entry would mean the current folder
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).filter(|d| d.is_absolute()).collect();
    // the native installer puts claude here even when PATH wasn't refreshed yet
    if let Some(h) = std::env::var_os("USERPROFILE") {
        dirs.push(PathBuf::from(h).join(".local").join("bin"));
    }
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

/// The newest `codex.exe` the Codex desktop app keeps under `%USERPROFILE%\.codex\packages`
/// (it isn't put on PATH): `app-server-daemon\releases\<version>\bin\codex.exe`.
fn codex_app_cli() -> Option<PathBuf> {
    let releases = PathBuf::from(std::env::var_os("USERPROFILE")?).join(".codex").join("packages").join("app-server-daemon").join("releases");
    let mut found: Vec<(Vec<u64>, PathBuf)> = std::fs::read_dir(&releases)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let exe = e.path().join("bin").join("codex.exe");
            let version: Vec<u64> = name.split(['-', '+']).next()?.split('.').map(|n| n.parse().ok()).collect::<Option<_>>()?;
            exe.is_file().then_some((version, exe))
        })
        .collect();
    found.sort();
    found.pop().map(|(_, p)| p)
}

impl Detected {
    pub fn probe() -> Detected {
        Detected {
            claude_code: on_path(&["claude.exe", "claude.cmd"]),
            codex: on_path(&["codex.exe", "codex.cmd"]).or_else(codex_app_cli),
            ollama: listening(11434),
            lm_studio: listening(1234),
            anthropic_env: std::env::var_os("ANTHROPIC_API_KEY").is_some(),
            openai_env: std::env::var_os("OPENAI_API_KEY").is_some(),
        }
    }

    /// The best choice for someone who hasn't picked: Claude Code if it is installed (no key
    /// needed), then an API key, then a local model server, then off. Never Codex: it keeps
    /// tools Sprawl can't remove, so only the user picks it (see `llm::codex::RISK`).
    pub fn suggest(&self) -> AiConfig {
        if self.claude_code.is_some() {
            AiConfig { provider: Provider::ClaudeCode, model: Provider::ClaudeCode.default_model().into(), ..AiConfig::default() }
        } else if self.anthropic_env {
            AiConfig { provider: Provider::Anthropic, model: Provider::Anthropic.default_model().into(), ..AiConfig::default() }
        } else if self.ollama {
            AiConfig { provider: Provider::OpenAi, base_url: "http://localhost:11434/v1".into(), ..AiConfig::default() }
        } else if self.lm_studio {
            AiConfig { provider: Provider::OpenAi, base_url: "http://localhost:1234/v1".into(), ..AiConfig::default() }
        } else {
            AiConfig::default()
        }
    }

    /// One line for the wizard: what was found.
    pub fn summary(&self) -> String {
        let mut found = Vec::new();
        if self.claude_code.is_some() {
            found.push("Claude Code");
        }
        if self.codex.is_some() {
            found.push("Codex");
        }
        if self.anthropic_env {
            found.push("ANTHROPIC_API_KEY");
        }
        if self.ollama {
            found.push("Ollama (port 11434)");
        }
        if self.lm_studio {
            found.push("LM Studio (port 1234)");
        }
        if self.openai_env {
            found.push("OPENAI_API_KEY");
        }
        if found.is_empty() {
            "Nothing found on this PC yet. You can add a key or install Claude Code or Codex later.".into()
        } else {
            format!("Found on this PC: {}", found.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_as_toml() {
        let c = AiConfig { provider: Provider::OpenAi, model: "llama3".into(), base_url: "http://localhost:11434/v1".into(), ..AiConfig::default() };
        assert!(!AiConfig::default().web_lookup, "the web search is off until the user turns it on");
        let t = toml::to_string(&c).unwrap();
        assert!(t.contains("provider = \"open-ai\""));
        assert_eq!(toml::from_str::<AiConfig>(&t).unwrap(), c);
        assert_eq!(toml::from_str::<AiConfig>("provider = \"off\"").unwrap(), AiConfig::default());
        assert_eq!(toml::from_str::<AiConfig>("provider = \"codex\"").unwrap().provider, Provider::Codex);
    }

    #[test]
    fn credential_store_round_trip() {
        let target = format!("Sprawl/test-{}", std::process::id());
        assert!(cred::write(&target, "sk-test-123"));
        assert_eq!(cred::read(&target).as_deref(), Some("sk-test-123"));
        assert!(cred::delete(&target));
        assert_eq!(cred::read(&target), None);
    }
}
