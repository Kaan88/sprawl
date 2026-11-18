//! Updates: Sprawl looks for a newer release on GitHub (when it starts and once a day while
//! open, or from Help > Check for Updates), shows what changed, and on "Update and restart"
//! downloads it, checks it and swaps it in. See `sprawl-install` for what is checked.
//!
//! What is sent: a plain GET to api.github.com and GitHub's download hosts, with Sprawl's name
//! as the user agent. Nothing about the PC or its files.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use sprawl_advisor::llm::http;
use sprawl_install::setup::{clean_stage, remove_old, swap_in, STAGE};
use sprawl_install::{newer, verify, FileEntry, MANIFEST, PUBLIC_KEYS, REPO, SIGNATURE};

use crate::bindings::Microsoft::UI::Xaml::Controls::ToggleMenuFlyoutItem;
use crate::bindings::Microsoft::UI::Xaml::{UIElement, Visibility};
use crate::state::{find, text, State};

/// This version. Only debug builds accept the update-test override; production's floor is
/// always the compiled version, regardless of the process environment.
pub(crate) fn current() -> &'static str {
    #[cfg(not(debug_assertions))]
    { env!("CARGO_PKG_VERSION") }
    #[cfg(debug_assertions)]
    {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| std::env::var("SPRAWL_PRETEND_VERSION").ok().filter(|v| sprawl_install::parse_version(v).is_some()).unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()))
    }
}

#[cfg(all(test, not(debug_assertions)))]
#[test]
fn release_version_is_always_the_compiled_floor() {
    assert_eq!(current(), env!("CARGO_PKG_VERSION"));
}

/// Where releases and their files may come from (a download redirects to GitHub's storage).
/// The storage host has changed before, so any host under githubusercontent.com will do; the
/// signature and the hashes are what make a file trusted, not where it came from.
const HOSTS: [&str; 3] = ["api.github.com", "github.com", ".githubusercontent.com"];

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
    Staged(String, Staged),
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
    pub(crate) applying: bool,
    /// When a Sprawl left open looks again (a day after the last look).
    next: Option<std::time::Instant>,
}

/// How often a Sprawl left open looks again.
const AGAIN: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// The API answer, through the same redirect rules as a download (a renamed repository answers
/// with a redirect).
fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let headers = [("Accept", "application/vnd.github+json".to_string()), ("User-Agent", format!("Sprawl/{}", current()))];
    let body = http::download(url, &HOSTS, &headers, 4 << 20, Duration::from_secs(20))?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

/// Run `f`, turning a panic into an error, so a worker can always answer.
fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err("something went wrong inside Sprawl".into()))
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
    let m = verify(&manifest, &sig, &PUBLIC_KEYS)?;
    if m.version != version {
        return Err(format!("the release says {version} but its files say {}", m.version));
    }
    let files = m.files.into_iter().map(|f| url(&f.name).map(|u| (f, u))).collect::<Result<Vec<_>, _>>()?;
    let notes = rel["body"].as_str().unwrap_or("").to_string();
    Ok(Some(Avail { version, notes, files }))
}

/// A downloaded file waiting to be swapped in: name, where it is, its expected SHA-256.
type Staged = Vec<(String, PathBuf, String)>;

/// Download each file into `<dir>\.sprawl-update`, checking its size and SHA-256.
fn stage(a: &Avail, dir: &std::path::Path, progress: &dyn Fn(String)) -> Result<Staged, String> {
    use std::io::Write;
    sprawl_install::setup::per_user_only()?;
    let root = sprawl_core::local::directory(dir).map_err(|e| e.to_string())?;
    let dir = &root.path;
    let st = dir.join(STAGE);
    clean_stage(dir);
    let cant = || format!("Sprawl's folder ({}) can't be written to, so it can't update itself here. Get the new version from github.com/{REPO}/releases.", dir.display());
    std::fs::create_dir(&st).map_err(|_| cant())?;
    // a folder put there as a link could send the files elsewhere
    let _stage = sprawl_core::local::directory(&st).map_err(|_| cant())?;
    let mut out = Vec::new();
    for (i, (f, url)) in a.files.iter().enumerate() {
        if !sprawl_install::valid_name(&f.name) { return Err("Invalid update file name".into()); }
        progress(format!("Downloading {} of {}: {} ({:.1} MB)", i + 1, a.files.len(), f.name, f.size as f64 / 1048576.0));
        let bytes = fetch(url, (f.size as usize).saturating_add(1).min(512 << 20))?;
        if bytes.len() as u64 != f.size || sprawl_install::sha256_hex(&bytes) != f.sha256 {
            return Err(format!("{} didn't download intact; nothing was changed", f.name));
        }
        let p = st.join(&f.name);
        let mut target = std::fs::OpenOptions::new().write(true).create_new(true).open(&p).map_err(|e| format!("could not create {}: {e}", f.name))?;
        target.write_all(&bytes).and_then(|_| target.sync_all()).map_err(|e| format!("could not save {}: {e}", f.name))?;
        out.push((f.name.clone(), p, f.sha256.clone()));
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
            let _ = tx.send(match guarded(check) {
                Ok(Some(a)) => Msg::Found(a),
                Ok(None) => Msg::Latest,
                Err(e) => Msg::Failed(e),
            });
        });
    }

    /// A check got an answer: failures start over.
    fn update_checked(&mut self) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.settings.set("update_last", now);
        self.settings.set("update_fails", 0);
        self.settings.save();
    }

    /// The automatic check may run: the switch is on, not offline, not a screenshot run.
    fn update_auto(&self) -> bool {
        let on = find::<ToggleMenuFlyoutItem>(&self.root, "oUpdates").and_then(|t| t.IsChecked()).unwrap_or(true);
        on && !sprawl_advisor::offline() && self.shot.is_none() && !crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Each frame: the check at start and once a day while open, and answers from the threads.
    pub(crate) fn update_poll(&mut self) {
        if !self.update.started {
            self.update.started = true;
            if !crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) && self.shot.is_none() && sprawl_install::setup::per_user_only().is_ok() {
            if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)) {
                remove_old(&dir);
                clean_stage(&dir);
            }
            }
            // every start looks (a release since yesterday's look is offered at once)
            self.update.next = Some(std::time::Instant::now() + AGAIN);
            if self.update_auto() {
                self.update_check(false);
            }
        }
        // left open: a day later, unless a release is already offered or a check is running
        if self.update.next.is_some_and(|t| std::time::Instant::now() >= t) && self.update.rx.is_none() && self.update.avail.is_none() {
            self.update.next = Some(std::time::Instant::now() + AGAIN);
            if self.update_auto() {
                self.update_check(false);
            }
        }
        let Some(rx) = &self.update.rx else { return };
        let msg = match rx.try_recv() {
            Ok(m) => m,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            // the worker is gone without an answer
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Msg::Failed("the check stopped".into()),
        };
        match msg {
            Msg::Progress(s) => {
                text(&self.root, "UpdateStatus", &s);
                return;
            }
            Msg::Found(a) => {
                self.update.rx = None;
                self.update_checked();
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
                self.update_checked();
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
                } else {
                    // the daily check stays quiet once or twice (offline), then says so
                    let fails = self.settings.num("update_fails").unwrap_or(0.0) as u32 + 1;
                    self.settings.set("update_fails", fails);
                    self.settings.save();
                    if fails >= 3 {
                        text(&self.root, "FootLeft", &format!("Sprawl couldn't look for updates the last {fails} times ({e}). Help > Check for Updates tries again."));
                    }
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
                if let Err(e) = sprawl_install::setup::per_user_only() { text(&self.root, "UpdateStatus", &e); return; }
                if crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) || self.shot.is_some() { return; }
                if self.advisor.cleanup.running.is_some() || self.advisor.uncompress.is_some() || self.job.is_some() {
                    text(&self.root, "UpdateStatus", "A scan or file operation is running. Update when it has finished."); return;
                }
                let Some(a) = self.update.avail.clone() else { return };
                let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)) else { return };
                self.update.applying = true;
                let (tx, rx) = channel();
                self.update.rx = Some(rx);
                std::thread::spawn(move || {
                    let ptx = tx.clone();
                    let r = guarded(|| {
                        stage(&a, &dir, &move |s| {
                            let _ = ptx.send(Msg::Progress(s));
                        })
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
    fn update_swap(&mut self, version: &str, staged: &Staged) {
        let fail = |s: &mut Self, msg: String| {
            s.update.applying = false;
            text(&s.root, "UpdateStatus", &msg);
        };
        if let Err(e) = sprawl_install::setup::per_user_only() { return fail(self, e); }
        if !newer(version, current()) { return fail(self, "This update is not newer than this copy of Sprawl.".into()); }
        if crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) || self.shot.is_some() { return fail(self, "Updates are disabled in this run.".into()); }
        if self.advisor.cleanup.running.is_some() || self.advisor.uncompress.is_some() || self.job.is_some() {
            return fail(self, "A scan or file operation is running; press Update again when it's done.".into());
        }
        let Some(exe) = std::env::current_exe().ok() else { return };
        let Some(dir) = exe.parent().map(PathBuf::from) else { return };
        // The installer hashes open sources and publishes those exact handles.
        if let Err(e) = swap_in(&dir, staged) {
            return fail(self, format!("The update didn't work: {e}"));
        }
        let _launch = match sprawl_install::setup::verified_launch(&dir, staged) {
            Ok(held) => held,
            Err(e) => return fail(self, format!("The replacement could not be verified for launch: {e}. Reinstall with Sprawl Setup.")),
        };
        clean_stage(&dir);
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
            // close the window the normal way (exiting from inside a XAML callback ends the
            // process with a crash code that Windows may report)
            if self.window.Close().is_err() {
                std::process::exit(0);
            }
            return;
        }
        text(&self.root, "UpdateStatus", "Updated. Close and open Sprawl to use the new version.");
    }
}
