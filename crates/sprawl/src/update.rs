//! Updates: Sprawl looks for a newer release on GitHub (at most once a day, or from Help >
//! Check for Updates), shows what changed, and on "Update and restart" downloads it, checks it
//! and swaps it in. See `sprawl-install` for what is checked.
//!
//! What is sent: a plain GET to api.github.com and GitHub's download hosts, with Sprawl's name
//! as the user agent. Nothing about the PC or its files.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use sprawl_advisor::llm::http;
use sprawl_install::{newer, verify, FileEntry, MANIFEST, PUBLIC_KEY, REPO, SIGNATURE};

use crate::bindings::Microsoft::UI::Xaml::{UIElement, Visibility};
use crate::state::{find, text, State};

/// This version. `SPRAWL_PRETEND_VERSION` (for testing updates) makes it look older; the
/// signature and file checks are the same either way.
pub(crate) fn current() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| std::env::var("SPRAWL_PRETEND_VERSION").ok().filter(|v| sprawl_install::parse_version(v).is_some()).unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()))
}

/// Where releases and their files may come from (a download redirects to GitHub's storage).
const HOSTS: [&str; 4] = ["api.github.com", "github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];

#[derive(Clone)]
pub(crate) struct Avail {
    pub(crate) version: String,
    pub(crate) notes: String,
    files: Vec<(FileEntry, String)>,
}

enum Msg {
    Found(Avail),
    Latest,
    Failed(String),
    Staged(String, Vec<(String, PathBuf)>),
    Progress(String),
}

#[derive(Default)]
pub(crate) struct Updater {
    rx: Option<Receiver<Msg>>,
    avail: Option<Avail>,
    /// The check was asked for (Help menu): say so when there's nothing new, or it failed.
    manual: bool,
    started: bool,
    /// Downloading or swapping: the buttons do nothing meanwhile.
    applying: bool,
}

fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let r = http::get(url, &[("Accept", "application/vnd.github+json".into()), ("User-Agent", format!("Sprawl/{}", current()))], Duration::from_secs(20))?;
    if r.status != 200 {
        return Err(format!("GitHub answered {}", r.status));
    }
    serde_json::from_str(&r.body).map_err(|e| e.to_string())
}

fn fetch(url: &str, max: usize) -> Result<Vec<u8>, String> {
    http::download(url, &HOSTS, &[("User-Agent", format!("Sprawl/{}", current()))], max, Duration::from_secs(120))
}

/// Ask GitHub for the latest release; if it's newer, fetch and check its signed manifest.
fn check() -> Result<Option<Avail>, String> {
    let rel = get_json(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
    let tag = rel["tag_name"].as_str().ok_or("the release has no version")?;
    let version = tag.trim_start_matches('v').to_string();
    if !newer(&version, current()) {
        return Ok(None);
    }
    let assets: Vec<(String, String)> = rel["assets"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| Some((x["name"].as_str()?.to_string(), x["browser_download_url"].as_str()?.to_string()))).collect())
        .unwrap_or_default();
    let url = |name: &str| assets.iter().find(|(n, _)| n == name).map(|(_, u)| u.clone()).ok_or(format!("the release has no {name}"));
    let manifest = fetch(&url(MANIFEST)?, 1 << 20)?;
    let sig = fetch(&url(SIGNATURE)?, 4 << 10)?;
    let m = verify(&manifest, &sig, &PUBLIC_KEY)?;
    if m.version != version {
        return Err(format!("the release says {version} but its files say {}", m.version));
    }
    let files = m.files.into_iter().map(|f| url(&f.name).map(|u| (f, u))).collect::<Result<Vec<_>, _>>()?;
    let notes = rel["body"].as_str().unwrap_or("").to_string();
    Ok(Some(Avail { version, notes, files }))
}

/// Download each file into `<dir>\.update`, checking its size and SHA-256.
fn stage(a: &Avail, dir: &std::path::Path, progress: &dyn Fn(String)) -> Result<Vec<(String, PathBuf)>, String> {
    let st = dir.join(".update");
    let _ = std::fs::remove_dir_all(&st);
    std::fs::create_dir_all(&st).map_err(|_| format!("Sprawl's folder ({}) can't be written to, so it can't update itself here. Get the new version from github.com/{REPO}/releases.", dir.display()))?;
    let mut out = Vec::new();
    for (i, (f, url)) in a.files.iter().enumerate() {
        progress(format!("Downloading {} of {}: {} ({:.1} MB)", i + 1, a.files.len(), f.name, f.size as f64 / 1048576.0));
        let bytes = fetch(url, (f.size as usize).saturating_add(1).min(512 << 20))?;
        if bytes.len() as u64 != f.size || sprawl_install::sha256_hex(&bytes) != f.sha256 {
            return Err(format!("{} didn't download intact; nothing was changed", f.name));
        }
        let p = st.join(&f.name);
        std::fs::write(&p, &bytes).map_err(|e| format!("could not save {}: {e}", f.name))?;
        out.push((f.name.clone(), p));
    }
    Ok(out)
}

/// Plain text from the release notes (Markdown): bullets as dots, headings and emphasis bare.
fn plain(md: &str) -> String {
    md.lines()
        .map(|l| {
            let t = l.trim_end();
            let t = t.trim_start_matches('#').trim_start();
            let t = if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) { format!("\u{2022} {r}") } else { t.to_string() };
            t.replace("**", "").replace('`', "")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

impl State {
    fn update_card(&self, show: bool) {
        if let Ok(e) = find::<UIElement>(&self.root, "UpdateCard") {
            let _ = e.SetVisibility(if show { Visibility::Visible } else { Visibility::Collapsed });
        }
    }

    /// Look for a newer release on another thread. `manual`: asked from the Help menu.
    pub(crate) fn update_check(&mut self, manual: bool) {
        if self.update.rx.is_some() {
            return;
        }
        self.update.manual = manual;
        if manual {
            text(&self.root, "FootLeft", "Looking for a newer Sprawl...");
        }
        let (tx, rx) = channel();
        self.update.rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(match check() {
                Ok(Some(a)) => Msg::Found(a),
                Ok(None) => Msg::Latest,
                Err(e) => Msg::Failed(e),
            });
        });
        self.settings.set("update_last", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
        self.settings.save();
    }

    /// Each frame: the daily check once at start, and answers from the threads.
    pub(crate) fn update_poll(&mut self) {
        if !self.update.started {
            self.update.started = true;
            let _ = std::env::current_exe().map(|e| e.parent().map(sprawl_install::setup::remove_old));
            let _ = std::fs::remove_dir_all(std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(".update"))).unwrap_or_default());
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let last = self.settings.num("update_last").unwrap_or(0.0) as u64;
            let on = crate::state::find::<crate::bindings::Microsoft::UI::Xaml::Controls::ToggleMenuFlyoutItem>(&self.root, "oUpdates").and_then(|t| t.IsChecked()).unwrap_or(true);
            if on && self.shot.is_none() && !crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) && now.saturating_sub(last) > 20 * 3600 {
                self.update_check(false);
            }
        }
        let Some(rx) = &self.update.rx else { return };
        let Ok(msg) = rx.try_recv() else { return };
        match msg {
            Msg::Progress(s) => {
                text(&self.root, "UpdateStatus", &s);
                return;
            }
            Msg::Found(a) => {
                self.update.rx = None;
                if !self.update.manual && self.settings.get("update_skip") == Some(a.version.as_str()) {
                    return;
                }
                text(&self.root, "UpdateTitle", &format!("Sprawl {} is out", a.version));
                text(&self.root, "UpdateSub", &format!("You have {}. What's new:", current()));
                text(&self.root, "UpdateNotes", &plain(&a.notes));
                text(&self.root, "UpdateStatus", "");
                self.update.avail = Some(a);
                self.update_card(true);
            }
            Msg::Latest => {
                self.update.rx = None;
                if self.update.manual {
                    text(&self.root, "FootLeft", &format!("Sprawl {} is the latest version.", current()));
                }
            }
            Msg::Failed(e) => {
                self.update.rx = None;
                self.update.applying = false;
                if self.update.avail.is_some() {
                    text(&self.root, "UpdateStatus", &format!("The update didn't work: {e}"));
                } else if self.update.manual {
                    text(&self.root, "FootLeft", &format!("Couldn't look for updates: {e}"));
                }
            }
            Msg::Staged(version, staged) => {
                self.update.rx = None;
                self.update_swap(&version, &staged);
            }
        }
    }

    /// The card's buttons.
    pub(crate) fn update_command(&mut self, name: &str) {
        match name {
            "UpdateLater" => self.update_card(false),
            "UpdateSkip" => {
                if let Some(a) = &self.update.avail {
                    self.settings.set("update_skip", &a.version);
                    self.settings.save();
                }
                self.update_card(false);
            }
            "UpdateNow" => {
                if self.update.applying || self.update.rx.is_some() {
                    return;
                }
                let Some(a) = self.update.avail.clone() else { return };
                let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)) else { return };
                self.update.applying = true;
                let (tx, rx) = channel();
                self.update.rx = Some(rx);
                std::thread::spawn(move || {
                    let ptx = tx.clone();
                    let r = stage(&a, &dir, &move |s| {
                        let _ = ptx.send(Msg::Progress(s));
                    });
                    let _ = tx.send(match r {
                        Ok(s) => Msg::Staged(a.version.clone(), s),
                        Err(e) => Msg::Failed(e),
                    });
                });
            }
            _ => {}
        }
    }

    /// Put the checked files in place and start the new version.
    fn update_swap(&mut self, version: &str, staged: &[(String, PathBuf)]) {
        if self.advisor.cleanup.running.is_some() {
            self.update.applying = false;
            text(&self.root, "UpdateStatus", "A clean-up is running; press Update again when it's done.");
            return;
        }
        let Some(exe) = std::env::current_exe().ok() else { return };
        let Some(dir) = exe.parent().map(PathBuf::from) else { return };
        if let Err(e) = sprawl_install::setup::swap_in(&dir, staged) {
            self.update.applying = false;
            text(&self.root, "UpdateStatus", &format!("The update didn't work, so nothing was changed: {e}"));
            return;
        }
        let _ = std::fs::remove_dir_all(dir.join(".update"));
        if sprawl_install::setup::is_installed_copy() {
            let _ = sprawl_install::setup::register(&dir, version);
        }
        if let Some(p) = crate::win32::window_place(self.hwnd) {
            self.settings.set("window", p);
        }
        self.settings.save();
        let mut cmd = std::process::Command::new(&exe);
        if let Some(t) = &self.tree {
            if t.method != sprawl_core::ScanMethod::Csv && t.nodes[sprawl_core::ROOT].name != sprawl_core::MULTIPLE {
                cmd.arg(t.path_of(sprawl_core::ROOT));
            }
        }
        if cmd.spawn().is_ok() {
            std::process::exit(0);
        }
        text(&self.root, "UpdateStatus", "Updated. Close and open Sprawl to use the new version.");
    }
}
