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
}

impl Provider {
    pub const ALL: [Provider; 4] = [Provider::Off, Provider::Anthropic, Provider::ClaudeCode, Provider::OpenAi];

    pub fn label(self) -> &'static str {
        match self {
            Provider::Off => "Off: rules only, nothing is sent anywhere",
            Provider::Anthropic => "Claude (Anthropic API key)",
            Provider::ClaudeCode => "Claude Code (uses your claude login)",
            Provider::OpenAi => "OpenAI-compatible (OpenAI, OpenRouter, Ollama, LM Studio)",
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
            Provider::Anthropic | Provider::ClaudeCode => "claude-opus-5-5",
            Provider::OpenAi => "",
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
        std::fs::create_dir_all(&d)?;
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write(d.join("ai.toml"), text)
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
            let s = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
            let out = String::from_utf8(s.to_vec()).ok();
            CredFree(p as *const _);
            out
        }
    }

    pub fn delete(target: &str) -> bool {
        let t = wide(target);
        unsafe { CredDeleteW(t.as_ptr(), CRED_TYPE_GENERIC, 0) != 0 }
    }
}

/// Store (or with an empty string, remove) the API key for a provider.
pub fn set_key(p: Provider, key: &str) -> bool {
    let key = key.trim();
    if key.is_empty() {
        cred::delete(p.credential());
        return true;
    }
    cred::write(p.credential(), key)
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
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    // the native installer puts claude here even when PATH wasn't refreshed yet
    if let Some(h) = std::env::var_os("USERPROFILE") {
        dirs.push(PathBuf::from(h).join(".local").join("bin"));
    }
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

impl Detected {
    pub fn probe() -> Detected {
        Detected {
            claude_code: on_path(&["claude.exe", "claude.cmd"]),
            ollama: listening(11434),
            lm_studio: listening(1234),
            anthropic_env: std::env::var_os("ANTHROPIC_API_KEY").is_some(),
            openai_env: std::env::var_os("OPENAI_API_KEY").is_some(),
        }
    }

    /// The best choice for someone who hasn't picked: Claude Code if it is installed (no key
    /// needed), then a local model server, then off.
    pub fn suggest(&self) -> AiConfig {
        if self.claude_code.is_some() {
            AiConfig { provider: Provider::ClaudeCode, model: Provider::ClaudeCode.default_model().into(), base_url: String::new() }
        } else if self.anthropic_env {
            AiConfig { provider: Provider::Anthropic, model: Provider::Anthropic.default_model().into(), base_url: String::new() }
        } else if self.ollama {
            AiConfig { provider: Provider::OpenAi, model: String::new(), base_url: "http://localhost:11434/v1".into() }
        } else if self.lm_studio {
            AiConfig { provider: Provider::OpenAi, model: String::new(), base_url: "http://localhost:1234/v1".into() }
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
            "Nothing found on this PC yet. You can add a key or install Claude Code later.".into()
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
        let c = AiConfig { provider: Provider::OpenAi, model: "llama3".into(), base_url: "http://localhost:11434/v1".into() };
        let t = toml::to_string(&c).unwrap();
        assert!(t.contains("provider = \"open-ai\""));
        assert_eq!(toml::from_str::<AiConfig>(&t).unwrap(), c);
        assert_eq!(toml::from_str::<AiConfig>("provider = \"off\"").unwrap(), AiConfig::default());
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
