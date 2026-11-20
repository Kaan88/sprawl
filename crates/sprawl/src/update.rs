//! Updates: Sprawl looks for a newer release on GitHub (when it starts and once a day while
//! open, or from Help > Check for Updates), shows what changed, and on "Update and restart"
//! downloads it, checks it and swaps it in. See `sprawl-install` for what is checked.
//!
//! A copy installed for all users (Program Files) can't change its own folder: it downloads the
//! release's Setup instead, checks it against the signed list of files, holds it so it can't
//! change, and runs it as administrator. A per-user copy can do the same to move to Program
//! Files (Options > Restart as Administrator offers it).
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
    /// The release's Setup and where to get it, when its signed list has one.
    setup: Option<(FileEntry, String)>,
}

enum Msg {
    Found(Avail),
    Latest,
    Failed(String),
    Staged(String, Staged),
    Progress(String),
    /// Setup, downloaded and checked, held open so it can't change before it runs.
    SetupReady(PathBuf, std::fs::File),
}

/// "Always run as administrator" is on but this copy may not elevate: say why at start.
pub(crate) static ADMIN_REFUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How Setup runs for an update: progress only, Sprawl closed and started again after.
const SETUP_ARGS: &str = "/SILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS /RESTARTSPRAWL";

/// Where Setup is downloaded: a new folder under `%LOCALAPPDATA%\Sprawl` each time.
fn setup_parent() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Sprawl"))
}

/// Download Setup into a new folder of its own, check its size and SHA-256 against the signed
/// list, then reopen it so no one else can write, rename or delete it, and check it again
/// from that handle: what runs is what was checked.
fn fetch_setup(a: &Avail, progress: &dyn Fn(String)) -> Result<(PathBuf, std::fs::File), String> {
    use std::io::{Seek, Write};
    use std::os::windows::fs::OpenOptionsExt;
    sprawl_install::setup::per_user_only()?;
    let (f, url) = a.setup.as_ref().ok_or(format!("this release has no Setup Sprawl can check. Get it from github.com/{REPO}/releases."))?;
    if f.name != sprawl_install::setup_name(&a.version) { return Err("the release's Setup has the wrong name".into()); }
    let parent = setup_parent().ok_or("there is no local app data folder")?;
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let parent = sprawl_core::local::directory(&parent).map_err(|e| e.to_string())?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = parent.path.join(format!("setup-{}-{nanos}", std::process::id()));
    std::fs::create_dir(&dir).map_err(|e| format!("could not make a folder for Setup ({e})"))?;
    let _dir = sprawl_core::local::directory(&dir).map_err(|e| e.to_string())?;
    progress(format!("Downloading Setup ({:.1} MB)...", f.size as f64 / 1048576.0));
    let bytes = fetch(url, (f.size as usize).saturating_add(1).min(512 << 20))?;
    if bytes.len() as u64 != f.size || sprawl_install::sha256_hex(&bytes) != f.sha256 {
        return Err("Setup didn't download intact; nothing was changed".into());
    }
    let p = dir.join(&f.name);
    {
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&p).map_err(|e| format!("could not save Setup: {e}"))?;
        out.write_all(&bytes).and_then(|_| out.sync_all()).map_err(|e| format!("could not save Setup: {e}"))?;
    }
    // FILE_SHARE_READ only: others may read and run it, not change it
    let mut held = std::fs::OpenOptions::new().read(true).share_mode(0x1).open(&p).map_err(|e| format!("could not hold Setup: {e}"))?;
    if sprawl_install::setup::is_link(&p) { return Err("Setup's file was replaced; nothing was run".into()); }
    let sha = sprawl_install::setup::hash_file(&mut held).map_err(|e| format!("could not check Setup: {e}"))?;
    if sha != f.sha256 || held.metadata().map(|m| m.len()).ok() != Some(f.size) {
        return Err("Setup changed after it downloaded; nothing was run".into());
    }
    let _ = held.rewind();
    Ok((p, held))
}

/// Setup folders from earlier updates (Setup has finished with them by the next start).
fn clean_setup_downloads() {
    let Some(parent) = setup_parent() else { return };
    let Ok(rd) = std::fs::read_dir(&parent) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("setup-") && e.file_type().is_ok_and(|t| t.is_dir()) && !sprawl_install::setup::is_link(&p) {
            let _ = std::fs::remove_dir_all(&p);
        }
    }
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
    /// Started by an elevated copy to install this version without administrator rights.
    /// Cleared by the first check's answer, whatever it is.
    pub(crate) resume: Option<String>,
    /// Why a hand-off from an elevated copy could not be taken (shown at start).
    resume_error: Option<String>,
    /// Installing for all users from a per-user copy: the check takes the latest release even
    /// when it is this version, and goes straight to Setup; messages go to the status bar.
    all_users: bool,
    /// The user agreed to close the other Sprawl processes for this update.
    others_ok: bool,
}

impl Updater {
    pub(crate) fn resuming(resume: Option<Result<String, String>>) -> Self {
        match resume {
            Some(Ok(v)) => Self { resume: Some(v), ..Default::default() },
            Some(Err(e)) => Self { resume_error: Some(e), ..Default::default() },
            None => Self::default(),
        }
    }
}

/// The argument an elevated Sprawl passes to the copy it starts without administrator rights.
pub(crate) const RESUME_ARG: &str = "--resume-update";

/// At start, for `--resume-update <version>`: the version an elevated copy asked this one to
/// install. The request is bound to this launch (an argument, not a file) and used once. It is
/// accepted only when Windows confirms this copy is not elevated; anyone able to start Sprawl
/// with it could equally press Update, and the release must still be newer, signed and match.
pub(crate) fn resume_request(arg: Option<String>) -> Option<Result<String, String>> {
    let version = arg?;
    if sprawl_core::elevation().ok() != Some(false) {
        return Some(Err("Sprawl could not reopen without administrator rights to install the update. Close Sprawl, turn off Options > Always Run as Administrator, open Sprawl normally and update.".into()));
    }
    let ok = sprawl_install::parse_version(&version).is_some() && newer(&version, current());
    ok.then_some(Ok(version))
}

/// An elevated Sprawl can't update its per-user folder (1.0 refused, which left "Always run as
/// administrator" copies unable to update). Start this exe as the signed-in user without
/// administrator rights (with the desktop shell as its parent) and let that copy download,
/// check and install. Nothing is written while elevated.
fn hand_off_unelevated(version: &str) -> Result<(), String> {
    if sprawl_install::parse_version(version).is_none() { return Err("the release's version is not valid".into()); }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    sprawl_core::identity::launch_unelevated(&exe, &[RESUME_ARG, version])
        .map(drop)
        .map_err(|e| format!("could not start Sprawl without administrator rights ({e})"))
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

/// Ask GitHub for the latest release; if it's newer (or `any`), fetch and check its signed
/// manifest.
fn check(any: bool) -> Result<Option<Avail>, String> {
    let rel = get_json(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
    let tag = rel["tag_name"].as_str().ok_or("the release has no version")?;
    let version = tag.trim_start_matches('v').to_string();
    if !newer(&version, current()) && !(any && sprawl_install::parse_version(&version).is_some()) {
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
    let setup = match m.setup {
        Some(f) => Some((f.clone(), url(&f.name)?)),
        None => None,
    };
    let notes = rel["body"].as_str().unwrap_or("").to_string();
    Ok(Some(Avail { version, notes, files, setup }))
}

/// A downloaded file waiting to be swapped in: name, where it is, its expected SHA-256.
type Staged = Vec<(String, PathBuf, String)>;

/// Download each file into `<dir>\.sprawl-update`, checking its size and SHA-256.
fn stage(a: &Avail, dir: &std::path::Path, progress: &dyn Fn(String)) -> Result<Staged, String> {
    use std::io::Write;
    sprawl_install::setup::per_user_only()?;
    let root = sprawl_core::local::directory(dir).map_err(|e| e.to_string())?;
    let dir = &root.path;
    if sprawl_install::recovery::running_copy() { return Err("This is the retained recovery copy. Use Sprawl Setup to restore the main installation.".into()); }
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
        let any = self.update.all_users;
        std::thread::spawn(move || {
            let _ = tx.send(match guarded(|| check(any)) {
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
                sprawl_install::recovery::cleanup(&dir);
                remove_old(&dir);
                clean_stage(&dir);
            }
            clean_setup_downloads();
            }
            // every start looks (a release since yesterday's look is offered at once)
            self.update.next = Some(std::time::Instant::now() + AGAIN);
            if let Some(e) = self.update.resume_error.take() {
                text(&self.root, "FootLeft", &e);
            }
            if ADMIN_REFUSED.swap(false, std::sync::atomic::Ordering::Relaxed) && self.shot.is_none() {
                self.offer_all_users();
            }
            if self.update.resume.is_some() && self.shot.is_none() && !crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) {
                text(&self.root, "FootLeft", "Installing the update without administrator rights...");
                self.update_check(true);
            } else if self.update_auto() {
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
        let status = if self.update.all_users { "FootLeft" } else { "UpdateStatus" };
        match msg {
            Msg::Progress(s) => {
                text(&self.root, status, &s);
                return;
            }
            Msg::SetupReady(path, held) => {
                self.update.rx = None;
                self.run_setup(&path, held);
            }
            Msg::Found(a) if self.update.all_users => {
                self.update.rx = None;
                self.update.avail = Some(a.clone());
                self.start_setup(a);
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
                let resume = self.update.resume.take().is_some_and(|v| v == a.version);
                self.update.avail = Some(a);
                self.update_card(true);
                // handed over by the elevated copy that just closed: carry on with its Update
                if resume {
                    self.update_command("UpdateNow");
                }
            }
            Msg::Latest => {
                self.update.rx = None;
                self.update.resume = None;
                self.update.all_users = false;
                self.update_checked();
                if self.update.manual {
                    text(&self.root, "FootLeft", &format!("Sprawl {} is the latest version.", current()));
                }
            }
            Msg::Failed(e) => {
                self.update.rx = None;
                self.update.resume = None;
                self.update.applying = false;
                self.update.others_ok = false;
                if std::mem::take(&mut self.update.all_users) {
                    text(&self.root, "FootLeft", &format!("Sprawl couldn't be installed for all users: {e}"));
                } else if self.update.avail.is_some() {
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
                if crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) || self.shot.is_some() { return; }
                if self.advisor.cleanup.running.is_some() || self.advisor.file_op() || self.job.is_some() {
                    text(&self.root, "UpdateStatus", "A scan or file operation is running. Update when it has finished."); return;
                }
                let Some(a) = self.update.avail.clone() else { return };
                if sprawl_core::elevation().unwrap_or(false) {
                    text(&self.root, "UpdateStatus", "Reopening Sprawl without administrator rights to install the update...");
                    match hand_off_unelevated(&a.version) {
                        Ok(()) => {
                            if let Some(p) = crate::win32::window_place(self.hwnd) { self.settings.set("window", p); }
                            self.settings.save();
                            if self.window.Close().is_err() { std::process::exit(0); }
                        }
                        Err(e) => text(&self.root, "UpdateStatus", &format!("{e}. Close Sprawl, turn off Options > Always Run as Administrator, and update from a normal Sprawl.")),
                    }
                    return;
                }
                if let Err(e) = sprawl_install::setup::per_user_only() { text(&self.root, "UpdateStatus", &e); return; }
                // installed for all users: Setup updates it, as administrator (it closes the
                // other Sprawl windows the polite way, and asks when one won't)
                if sprawl_install::machine::is_protected_copy() {
                    return self.start_setup(a);
                }
                // the other Sprawl processes are closed for the swap: asked first, since one
                // may be mid clean-up or serving an AI app
                let others = sprawl_install::processes::count_other_instances().unwrap_or(0);
                if others > 0 && !self.update.others_ok {
                    let text = format!("{others} other Sprawl process{} running: another window, the low-space check, or an AI app connected to Sprawl. Updating closes {} at once; a clean-up running there stops part way.\n\nClose {} first if you can, or let the update close {}.", if others == 1 { " is" } else { "es are" }, if others == 1 { "it" } else { "them" }, if others == 1 { "it" } else { "them" }, if others == 1 { "it" } else { "them" });
                    self.ask("Close the other Sprawl processes?", &text, "Close and update", true, |s| {
                        s.update.others_ok = true;
                        s.update_command("UpdateNow");
                    });
                    return;
                }
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

    /// From a per-user copy (Restart as Administrator, or Always Run as Administrator at
    /// start): say why it won't elevate, and offer to install for all users.
    pub(crate) fn offer_all_users(&mut self) {
        let text = format!("{}\n\nInstall Sprawl for all users to run it as administrator. Sprawl downloads the latest Setup, checks it against the signed release, and runs it; Windows asks for administrator rights. Setup puts Sprawl in Program Files and removes this copy; your settings and scans stay.", sprawl_install::machine::WHY_NOT_ELEVATED);
        self.ask("Install Sprawl for all users?", &text, "Install for all users", false, |s| s.install_for_all_users());
    }

    fn install_for_all_users(&mut self) {
        if self.update.applying || self.update.rx.is_some() { return; }
        if crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) || self.shot.is_some() || sprawl_advisor::offline() {
            text(&self.root, "FootLeft", "Sprawl can't download Setup in this run.");
            return;
        }
        if self.advisor.cleanup.running.is_some() || self.advisor.file_op() || self.job.is_some() {
            text(&self.root, "FootLeft", "A scan or file operation is running. Try again when it has finished.");
            return;
        }
        self.update.all_users = true;
        self.update.manual = true;
        text(&self.root, "FootLeft", "Looking for the latest Sprawl Setup...");
        let (tx, rx) = channel();
        self.update.rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(match guarded(|| check(true)) {
                Ok(Some(a)) => Msg::Found(a),
                Ok(None) => Msg::Failed("there is no release to install".into()),
                Err(e) => Msg::Failed(e),
            });
        });
    }

    /// Download and check Setup on another thread.
    fn start_setup(&mut self, a: Avail) {
        self.update.applying = true;
        let (tx, rx) = channel();
        self.update.rx = Some(rx);
        std::thread::spawn(move || {
            let ptx = tx.clone();
            let r = guarded(|| fetch_setup(&a, &move |s| { let _ = ptx.send(Msg::Progress(s)); }));
            let _ = tx.send(match r {
                Ok((p, f)) => Msg::SetupReady(p, f),
                Err(e) => Msg::Failed(e),
            });
        });
    }

    /// Run the checked Setup as administrator (Windows asks), then close: Setup starts the new
    /// Sprawl when it is done.
    fn run_setup(&mut self, path: &std::path::Path, held: std::fs::File) {
        let status = if self.update.all_users { "FootLeft" } else { "UpdateStatus" };
        if self.advisor.cleanup.running.is_some() || self.advisor.file_op() || self.job.is_some() {
            self.update.applying = false;
            self.update.all_users = false;
            text(&self.root, status, "A scan or file operation started meanwhile; try again when it's done.");
            return;
        }
        text(&self.root, status, "Starting Setup (Windows asks for administrator rights)...");
        let started = crate::dialogs::run_elevated(self.hwnd, path, SETUP_ARGS);
        drop(held);
        if !started {
            self.update.applying = false;
            self.update.all_users = false;
            text(&self.root, status, "Setup didn't start: administrator rights were not given. Nothing was changed.");
            return;
        }
        if let Some(p) = crate::win32::window_place(self.hwnd) {
            self.settings.set("window", p);
        }
        self.settings.save();
        if self.window.Close().is_err() {
            std::process::exit(0);
        }
    }

    /// Put the checked files in place and start the new version.
    fn update_swap(&mut self, version: &str, staged: &Staged) {
        let fail = |s: &mut Self, msg: String| {
            s.update.applying = false;
            s.update.others_ok = false;
            text(&s.root, "UpdateStatus", &msg);
        };
        if let Err(e) = sprawl_install::setup::per_user_only() { return fail(self, e); }
        if !newer(version, current()) { return fail(self, "This update is not newer than this copy of Sprawl.".into()); }
        if crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) || self.shot.is_some() { return fail(self, "Updates are disabled in this run.".into()); }
        if self.advisor.cleanup.running.is_some() || self.advisor.file_op() || self.job.is_some() {
            return fail(self, "A scan or file operation is running; press Update again when it's done.".into());
        }
        let Some(exe) = std::env::current_exe().ok() else { return };
        let Some(dir) = exe.parent().map(PathBuf::from) else { return };
        text(&self.root, "UpdateStatus", "Closing other Sprawl processes before updating...");
        if let Err(e) = sprawl_install::processes::stop_other_instances() { return fail(self, e); }
        // The installer hashes open sources and publishes those exact handles.
        if let Err(e) = swap_in(&dir, version, staged) {
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
