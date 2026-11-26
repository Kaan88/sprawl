//! Clean-up inside the Advisor: tick findings, review them, and Sprawl removes them itself,
//! showing each item's progress. The tree, the map and the findings update in place afterwards.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Instant;

use sprawl_advisor::clean::{self, Action, Outcome};
use sprawl_advisor::cleaners::Closed;
use sprawl_advisor::{Finding, Remedy, Risk, Source};
use sprawl_core::Tree;

use crate::state::State;
use crate::ui;

/// The ticks and the clean-up's state, kept in the Advisor.
#[derive(Default)]
pub(crate) struct Cleanup {
    context: Option<SelectionContext>,
    confirmation_selection: Option<(HashSet<String>, HashSet<String>, Option<String>)>,
    /// Ticked findings, by lower-case path.
    pub ticked: HashSet<String>,
    /// The compression the user picked for a compress item (B1), by lower-case path.
    pub algs: std::collections::HashMap<String, String>,
    /// The report generation the default ticks were set for.
    pub ticked_for: Option<u64>,
    /// Ticks carried over a restart as administrator, for the first report.
    pub pending: Option<HashSet<String>>,
    /// How many ticks did not fit on that restart's command line, to say once.
    pub pending_left: usize,
    /// The review panel is open.
    pub review: bool,
    /// External commands shown as runnable in the review, not just ticked.
    reviewed_tools: std::cell::RefCell<std::collections::HashMap<(PathBuf, String), Result<clean::ReviewedExternal, String>>>,
    reviewed_compressions: std::cell::RefCell<std::collections::HashMap<(PathBuf, String), Result<sprawl_advisor::remedies::ReviewedCompression, String>>>,
    /// "These are my own files and I want them gone" is ticked.
    pub own_ok: bool,
    /// The ticks are the ones Sprawl chose.
    pub auto: bool,
    /// The review is for this one finding (its row's Clean up...), not for the ticks.
    pub single: Option<String>,
    pub running: Option<Running>,
    /// The last clean-up's result, shown until dismissed.
    pub done: Option<Done>,
    /// What a pattern or keep-days item would free now, counted when the review opens (by path key).
    pub measured: std::collections::HashMap<String, u64>,
    measuring: Option<Receiver<(String, u64)>>,
    /// Ticked privacy parts (Report::privacy), by lower-case path: only ever by the user, one by
    /// one. A set that went is unticked; one held back stays ticked for "Check again".
    pub privacy_ticked: HashSet<String>,
    /// "I understand what each of these removes" is ticked.
    pub privacy_ok: bool,
    /// A row Sprawl doesn't tick that was clicked once (R7c), by lower-case path: the row says
    /// why it isn't ticked, and a second click ticks it.
    pub armed: Option<String>,
    /// Ticked from the AI's suggestion (R9), by lower-case path: shown in the AI's purple
    /// until the user touches the row.
    pub ai_ticked: HashSet<String>,
}

struct SelectionContext {
    tree: std::sync::Weak<Tree>,
    findings: Vec<Finding>,
    privacy: Vec<Finding>,
    definitions: Vec<sprawl_advisor::cleaners::CleanerDef>,
}

impl SelectionContext {
    fn same(&self, other: &Self) -> bool {
        self.tree.upgrade().zip(other.tree.upgrade()).is_some_and(|(a, b)| Arc::ptr_eq(&a, &b))
            && self.findings == other.findings && self.privacy == other.privacy && self.definitions == other.definitions
    }

    fn unchanged(&self, finding: &Finding, next: &Self) -> bool {
        self.findings.iter().chain(&self.privacy).any(|old| old == finding)
            && match &finding.source {
                Source::Rule(id) => sprawl_advisor::cleaners::selection_fingerprint(&self.definitions, id)
                    == sprawl_advisor::cleaners::selection_fingerprint(&next.definitions, id),
                _ => true,
            }
    }
}

impl Cleanup {
    pub(crate) fn begin_review(&mut self) {
        self.reviewed_tools.get_mut().clear();
        self.reviewed_compressions.get_mut().clear();
        self.review = true;
    }

    pub(crate) fn clear_privacy_selection(&mut self) {
        self.privacy_ticked.clear();
        self.privacy_ok = false;
        self.review = false;
        self.reviewed_tools.get_mut().clear();
        self.reviewed_compressions.get_mut().clear();
        self.confirmation_selection = None;
        self.measuring = None;
    }

    fn sync_confirmation_selection(&mut self) -> bool {
        let next = (self.ticked.clone(), self.privacy_ticked.clone(), self.single.clone());
        if self.confirmation_selection.as_ref() == Some(&next) { return false; }
        self.own_ok = false;
        self.privacy_ok = false;
        self.reviewed_tools.get_mut().clear();
        self.reviewed_compressions.get_mut().clear();
        self.confirmation_selection = Some(next);
        true
    }

    fn bind(&mut self, next: Option<SelectionContext>) -> bool {
        if self.context.as_ref().zip(next.as_ref()).is_some_and(|(a, b)| a.same(b)) || self.context.is_none() && next.is_none() { return false; }
        // No report for now (the Advisor reading the tree again after a clean-up): the ticks
        // wait, still bound to the findings they were made on, and are reconciled with the next
        // report when it comes. Nothing can run meanwhile: there is nothing to review.
        if next.is_none() {
            let had = self.review || self.single.is_some() || self.armed.is_some() || self.own_ok || self.privacy_ok;
            self.own_ok = false;
            self.privacy_ok = false;
            self.review = false;
            self.single = None;
            self.armed = None;
            self.measuring = None;
            return had;
        }
        if let Some(old) = &self.context {
            self.ticked.retain(|path| next.as_ref().is_some_and(|n| n.findings.iter().any(|f| key(&f.path) == *path && old.unchanged(f, n))));
            self.privacy_ticked.retain(|path| next.as_ref().is_some_and(|n| n.privacy.iter().any(|f| key(&f.path) == *path && old.unchanged(f, n))));
        }
        self.own_ok = false;
        self.privacy_ok = false;
        self.review = false;
        self.single = None;
        self.armed = None;
        self.ai_ticked.clear();
        self.measured.clear();
        self.measuring = None;
        self.context = next;
        true
    }
}

#[cfg(test)]
#[test]
fn revoking_privacy_keeps_normal_ticks_but_closes_the_review() {
    let mut c = Cleanup { privacy_ok: true, review: true, ..Cleanup::default() };
    c.privacy_ticked.insert("private item".into());
    c.ticked.insert("space item".into());
    c.clear_privacy_selection();
    assert!(!c.privacy_ok && !c.review && c.privacy_ticked.is_empty());
    assert!(c.ticked.contains("space item"));
}

pub(crate) struct Item {
    pub path: PathBuf,
    pub title: String,
    pub bytes: u64,
    pub state: ItemState,
    /// The rule (a cleaner part) it came from, if any: what it freed is remembered for it.
    pub rule: Option<String>,
    /// It goes to the Recycle Bin (the user's own files, with the setting on).
    pub recycle: bool,
    /// A privacy part's set: not counted in what was freed.
    pub privacy: bool,
    /// The tool command it runs, written into the clean-up log with its output (R7).
    pub command: Option<String>,
    /// Advice shown only if this item actually removed something.
    pub prevention: Option<&'static str>,
    pub learn: bool,
    /// What happens to it (deleted, to the bin, compressed in place...), as the review said.
    pub fate: Fate,
    /// The clean-up is over and its folder is being read again for what is left.
    pub measuring: bool,
}

pub(crate) enum ItemState {
    Waiting,
    Running(u64),
    Done(Outcome),
}

pub(crate) struct Running {
    pub items: Vec<Item>,
    rx: Receiver<Msg>,
    pub stop: Arc<AtomicBool>,

    /// What each item's folder holds now: None when it is gone.
    rescans: Vec<(PathBuf, Option<Tree>)>,
    /// Ticked items that didn't pass the checks again, and why.
    held: Vec<String>,
    /// When Stop was pressed: past [`STOP_WAIT`] the item in hand says it is finishing.
    pub stop_at: Option<Instant>,
    /// That was said.
    told_wait: bool,
}

/// How long Stop waits for the item in hand before saying it can't be stopped part way. The
/// run is never ended while its worker still acts: its result (what was freed, a compress to
/// record, the rescan) would be lost, and the guards that read `running` would stop guarding
/// (Fable 1.0.4-1.0.5 P2-1). "Close anyway" stays the way out.
const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Default)]
pub(crate) struct Done {
    pub freed: u64,
    /// Items that freed something.
    pub items: usize,
    /// Sent to the Recycle Bin (freed when the bin is emptied), and the items that sent some.
    pub recycled: u64,
    pub recycled_items: usize,
    /// Privacy sets that went, and the browsers that held one back (open).
    pub privacy_items: usize,
    /// Registry parts that went, with the backup each can be put back from.
    pub registry_items: usize,
    pub registry: Vec<sprawl_advisor::regclean::Done>,
    pub privacy_open: Vec<String>,
    /// Tools that finished but whose saving couldn't be measured.
    pub unmeasured: usize,
    /// Of `items`: deleted (by Sprawl, or by the program's own command), compressed in place,
    /// copies made links, virtual disks shrunk. Only the first deletes anything.
    pub deleted_items: usize,
    pub compressed_items: usize,
    pub linked_items: usize,
    pub shrunk_items: usize,
    /// Files compressed, found compressed already, and left because a program had them open.
    pub compressed_files: usize,
    pub already_compressed: usize,
    pub in_use: usize,
    /// Left because they were new since the scan, or kept by the user.
    pub kept_new: u64,
    pub kept_chosen: u64,
    pub kept_protected: u64,
    pub problems: Vec<String>,
    pub stopped: bool,
    pub output: Vec<String>,
    pub prevention: Vec<&'static str>,
}

enum Msg {
    Start(usize),
    Progress(usize, u64),
    /// The item's clean-up is over; its folder is being read again.
    Measuring(usize),
    /// (item, how it went, a scan of what is there now: None when it is gone, or not read)
    Finished(usize, Outcome, Option<Option<Tree>>),
    End,
}

/// One folder read again after it was cleaned. A plain walk: exact and quick for one folder,
/// where the volume's MFT (when elevated) would read the whole drive, maybe not yet seeing
/// what was just deleted. The quick listing a scan uses (sizes, times and attributes from the
/// folder entries): the detailed walk opens every file for hidden compression and pins every
/// folder's whole chain, which took many minutes on a 600,000-file Temp folder.
pub(crate) fn rescan(p: &std::path::Path) -> std::io::Result<Tree> {
    sprawl_core::walk::inventory(p, &sprawl_advisor::exclusions::progress())
}

/// `path` read again after its clean-up: None when it can't be read (or Stop was pressed,
/// the tree then keeps what it had), Some(None) when it is gone. `detailed` opens every file
/// for its compression (after a compress, where the on-disk sizes are the point).
fn rescan_after(path: &Path, detailed: bool, stop: Option<&Arc<AtomicBool>>) -> Option<Option<Tree>> {
    let path = sprawl_core::local::lexical(path).ok()?;
    let _parent = sprawl_core::local::directory(path.parent().unwrap_or(&path)).ok()?;
    match std::fs::symlink_metadata(&path) {
        Ok(_) => {
            let progress = sprawl_advisor::exclusions::progress();
            // Stop also ends the reading: a watcher passes it on until the walk returns
            let done = Arc::new(AtomicBool::new(false));
            if let Some(stop) = stop {
                let (p, s, d) = (progress.clone(), stop.clone(), done.clone());
                std::thread::spawn(move || {
                    while !d.load(Ordering::Relaxed) {
                        if s.load(Ordering::Relaxed) {
                            p.cancel();
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                });
            }
            let tree = if detailed { sprawl_core::walk::walk(&path, &progress) } else { sprawl_core::walk::inventory(&path, &progress) };
            done.store(true, Ordering::Relaxed);
            tree.ok().map(Some)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}

fn complete_size(scan: Option<&Option<Tree>>) -> Option<u64> {
    match scan? {
        None => Some(0),
        Some(tree) if tree.errors == 0 => tree.nodes.get(sprawl_core::ROOT).map(|n| n.size),
        _ => None,
    }
}

/// A running item's state: a compress by how far through it is (the review's size is the
/// folder's), anything else by what it freed so far.
fn running_label(it: &Item, so_far: u64) -> String {
    match it.fate {
        _ if it.measuring => "Measuring what's left...".to_string(),
        Fate::Compress if it.bytes > 0 => format!("Compressing... {}%, {} of {}", (so_far.min(it.bytes) * 100 / it.bytes).min(99), crate::format::size(so_far.min(it.bytes)), crate::format::size(it.bytes)),
        Fate::Compress => "Compressing...".to_string(),
        _ if it.privacy => "Removing...".to_string(),
        _ if it.recycle => format!("To the Recycle Bin... {}", crate::format::size(so_far)),
        _ => format!("Removing... {}", crate::format::size(so_far)),
    }
}

/// A finished compress's row: what it saved, and what it left.
fn compress_label(o: &Outcome) -> (&'static str, String, String) {
    let saved = crate::format::size(o.freed);
    let Some(c) = &o.compress else {
        return ("&#xE7BA;", crate::tokens::refer("warn"), "Not compressed; nothing changed".to_string());
    };
    let busy = if c.in_use > 0 { format!("; {} file(s) in use left as they were", c.in_use) } else { String::new() };
    match (succeeded(o), c.compressed) {
        (true, 0) if c.already > 0 => ("&#xE73E;", crate::tokens::refer("good"), "Already compressed".to_string()),
        (true, 0) => ("&#xE73E;", crate::tokens::refer("good"), format!("Nothing to compress{busy}")),
        (true, n) => ("&#xE73E;", crate::tokens::refer("good"), format!("Compressed {n} files, saved {saved}{busy}")),
        (false, _) if o.stopped => ("&#xE7BA;", crate::tokens::refer("warn"), format!("Stopped part way: saved {saved}")),
        (false, _) => ("&#xE7BA;", crate::tokens::refer("warn"), format!("Saved {saved}; some files couldn't be compressed")),
    }
}

/// The done banner's second line: what happened to the items, by what each did. Only a delete
/// says deleted: compressed, linked and shrunk items are all still there.
fn done_words(d: &Done) -> String {
    let mut parts: Vec<String> = Vec::new();
    match (d.deleted_items > 0, d.recycled > 0) {
        (true, true) => parts.push("Caches were deleted for good; your own files are in the Recycle Bin, and their space is freed when the bin is emptied.".into()),
        (false, true) => parts.push("Your files are in the Recycle Bin: Explorer can put them back, and their space is freed when the bin is emptied.".into()),
        (true, false) => parts.push(if d.compressed_items + d.linked_items + d.shrunk_items > 0 { "The caches were deleted for good." } else { "They were deleted for good." }.into()),
        (false, false) => {}
    }
    if d.compressed_items > 0 || d.compressed_files + d.already_compressed > 0 {
        let mut s = format!("Compressed in place: nothing was deleted, every file opens as before{}.", if d.compressed_files > 0 { format!(" ({} files compressed)", d.compressed_files) } else { String::new() });
        if d.compressed_files == 0 && d.already_compressed > 0 {
            s = "It was compressed already: nothing was changed or deleted.".into();
        }
        if d.in_use > 0 {
            s += &format!(" {} file(s) a program had open were left as they were: close it and compress again for those.", d.in_use);
        }
        s += " Undo it under Compressed.";
        parts.push(s);
    }
    if d.linked_items > 0 {
        parts.push("Copies now share one file on disk: every path still opens the same content.".into());
    }
    if d.shrunk_items > 0 {
        parts.push("The virtual disk was shrunk in place: nothing inside it was deleted.".into());
    }
    if d.privacy_items > 0 {
        parts.push(PV_DONE.into());
    }
    if d.registry_items > 0 {
        parts.push(PV_REG_DONE.into());
    }
    if parts.is_empty() {
        parts.push("Nothing was removed.".into());
    }
    parts.join(" ")
}

fn succeeded(out: &Outcome) -> bool { !out.stopped && out.problems.is_empty() && out.more_problems == 0 }

/// A compress of `path` that ended (`ok`: with no problems) goes into the Compressed list.
fn note_compressed(list: &mut Vec<sprawl_advisor::memory::Compressed>, path: &str, alg: &str, now: u64, bytes: u64, freed: u64, d: &sprawl_core::wof::Done, ok: bool) {
    // only a pass through every file stops it being offered, which the scan alone can't
    // tell; one stopped or with files busy, changed or failing keeps its retry
    let complete = ok && !d.stopped && d.in_use + d.failed + d.changed == 0;
    let same = |r: &sprawl_advisor::memory::Compressed| r.path.eq_ignore_ascii_case(path);
    // a retry that found nothing more to compress (the files done already are left out of
    // its plan, the busy ones didn't get smaller or are gone) still completes a partial one
    let completes = complete && list.iter().any(|r| same(r) && r.partial);
    // compressed now (some files or all), or found compressed already by an earlier run:
    // recorded, so it can be undone
    if freed == 0 && d.compressed == 0 && !(ok && d.already > 0) && !completes {
        return;
    }
    // an earlier run's saving is kept: a run over files compressed already saves no more
    let earlier = list.iter().filter(|r| same(r)).map(|r| r.saved).max().unwrap_or(0);
    list.retain(|r| !same(r));
    // never over what the folder holds (or, if more, what this run measured)
    let saved = earlier.saturating_add(freed).min(bytes.max(freed));
    list.push(sprawl_advisor::memory::Compressed { path: path.to_string(), alg: alg.to_string(), when: now, saved, size: bytes, partial: !complete });
}

fn review_changed(before: &Finding, now: &Finding) -> bool {
    before.path != now.path || before.risk != now.risk || before.remedy != now.remedy
        || before.action != now.action || before.admin != now.admin || before.command != now.command
        || before.carved != now.carved || before.builtin != now.builtin || before.trusted != now.trusted
}

fn credit_tool(out: &mut Outcome, before: Option<u64>, after: Option<u64>) {
    out.freed = 0;
    if !succeeded(out) { return; }
    match before.zip(after) {
        Some((before, after)) => out.freed = before.saturating_sub(after),
        // done, but unmeasured: told apart from a failure, still credited nothing (Astra)
        None => out.saving_unknown = true,
    }
}

pub(crate) fn key(p: &std::path::Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// Why a part whose application must be closed can't be cleaned now, or None when it can:
/// one of `exes` runs, a program has one of its files open, or Windows can't say.
fn still_open(path: &std::path::Path, app: &str, exes: &[String]) -> Option<String> {
    let now = sprawl_advisor::procs::running();
    match sprawl_advisor::procs::any_running(exes, now.as_ref()) {
        Some(false) => {}
        Some(true) => return Some(format!("{}: left alone, {app} is open", path.display())),
        None => return Some(format!("{}: left alone, Windows couldn't say whether {app} is open", path.display())),
    }
    let files = sprawl_advisor::inuse::files_on_disk(path, sprawl_advisor::inuse::MAX_FILES);
    if !files.complete {
        return Some(format!("{}: left alone, not every file could be checked for use (unreadable, linked or too many files)", path.display()));
    }
    let files = files.files;
    if files.is_empty() {
        return None;
    }
    match sprawl_advisor::inuse::in_use(&files) {
        Some(names) if names.is_empty() => None,
        Some(names) => Some(format!("{}: left alone, in use by {}", path.display(), names.join(", "))),
        None => Some(format!("{}: left alone, Windows couldn't say which programs use it", path.display())),
    }
}

/// The built-in rules whose command is Windows' own clean-up of the whole PC (C, 1.0.4): DISM
/// on the component store, emptying the Recycle Bin, Disk Clean-up for Windows.old. They act on
/// Windows, not on one account's folders, so the "another account's" gate doesn't apply; they
/// still need administrator where they do (held with "Restart as administrator"), and run only
/// as the built-in pack wrote them. Never a user's or the AI's rule.
pub(crate) const MACHINE_WIDE: &[&str] = &["win.component-store", "win.recycle-bin", "win.windows-old"];

pub(crate) fn machine_wide(f: &Finding) -> bool {
    // (and git's own clean-ups of a repository, D: git acts on the folder it is named)
    f.builtin && f.trusted && matches!(&f.source, Source::Rule(id) if MACHINE_WIDE.contains(&id.as_str())
        || id == sprawl_advisor::devgit::WORKTREE || id == sprawl_advisor::devgit::GIT_GC)
}

/// Why a finding can't be ticked, or None when it can.
pub(crate) fn untickable(f: &Finding, counted: bool) -> Option<&'static str> {
    if !matches!(f.remedy, Remedy::Delete | Remedy::Command) || f.risk == Risk::Red {
        return Some("");
    }
    let Some((action, _)) = clean::action(f) else { return Some("") };
    if !counted {
        return Some("Inside another item");
    }
    // a tool's own clean-up acts on the account running it: only the user's own folders (a
    // compress acts on the files named, for every account alike)
    if matches!(action, Action::Tool { .. }) && !sprawl_advisor::remedies::is_compress(&action) && !machine_wide(f) {
        let mine = std::env::var_os("USERPROFILE").is_some_and(|home| PathBuf::from(key(&f.path)).starts_with(PathBuf::from(home.to_string_lossy().to_lowercase())));
        if !mine {
            return Some("Another account's");
        }
    }
    None
}

/// R7c: whether a finding Sprawl doesn't tick may be ticked on a second click: a tip or a
/// folder left behind holding the user's files, made a delete of the whole path to the Recycle
/// Bin. Never Red, a model's finding, a privacy part, another account's, a whole folder of the
/// profile (Downloads, Documents...) or above it, Windows or programs, OneDrive (deleting there
/// deletes in the cloud too), or a tip whose win isn't the folder's size (compress, free up).
pub(crate) fn second_try(f: &Finding, counted: bool) -> bool {
    if f.forced_from.is_some() || untickable(f, counted).is_none() || !counted {
        return false;
    }
    if f.risk == Risk::Red || !matches!(f.remedy, Remedy::Info | Remedy::Move) || f.action.is_some() || f.command.is_some() || f.gain.is_some() {
        return false;
    }
    if matches!(f.source, Source::Model(_)) || !f.builtin || f.privacy() || f.other_account() {
        return false;
    }
    // the interrupted-link notice names several files and Sprawl must touch none of them: a
    // "tick it anyway" delete would take only the first (Fable backlog P3-5)
    if matches!(&f.source, Source::Rule(id) if id == sprawl_advisor::dupes::INTERRUPTED) {
        return false;
    }
    if second_try_place_refused(&PathBuf::from(key(&f.path))) {
        return false;
    }
    // what the deletion checks would refuse anyway gets no box (they run again at the click,
    // with links and the scan's root, and at Clean up)
    static NEVER: std::sync::OnceLock<sprawl_advisor::safety::Never> = std::sync::OnceLock::new();
    let never = NEVER.get_or_init(sprawl_advisor::safety::Never::default_list);
    never.hit(&f.path.to_string_lossy()).is_none()
}

/// The places [`second_try`] never makes a delete of, by lower-case path.
fn second_try_place_refused(p: &Path) -> bool {
    let low = |v: &str| std::env::var_os(v).map(|x| PathBuf::from(x.to_string_lossy().to_lowercase()));
    // a drive's root, or a file or folder right in it (pagefile.sys, Windows, Users...)
    // (`c:` `\` and a name are three components)
    if p.components().count() <= 3 {
        return true;
    }
    for v in ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData", "OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
        if low(v).is_some_and(|d| p.starts_with(&d)) {
            return true;
        }
    }
    if let Some(home) = low("USERPROFILE") {
        // the profile, what holds it, and its own folders whole (Downloads, AppData...)
        if home.starts_with(p) || p.parent().is_some_and(|q| q == home) {
            return true;
        }
        for sub in ["appdata\\local", "appdata\\roaming", "appdata\\locallow"] {
            if p == home.join(sub) {
                return true;
            }
        }
    }
    false
}

/// "3 hours" when the scan began an hour or more before `now`; None for a fresh one.
pub(crate) fn scan_age_words(began: std::time::SystemTime, now: std::time::SystemTime) -> Option<String> {
    scan_age_words_after(began, now, sprawl_advisor::tuning::get(&sprawl_advisor::tuning::OLD_SCAN_MINUTES))
}

/// [`scan_age_words`] with the warning from `after` minutes (Settings, at least 5).
pub(crate) fn scan_age_words_after(began: std::time::SystemTime, now: std::time::SystemTime, after: u64) -> Option<String> {
    let mins = now.duration_since(began).ok()?.as_secs() / 60;
    match mins {
        m if m < after.max(1) => None,
        0..=59 => Some(format!("{mins} minutes")),
        60..=119 => Some("an hour".into()),
        120..=2879 => Some(format!("{} hours", mins / 60)),
        _ => Some(format!("{} days", mins / 1440)),
    }
}

/// Whether `f` goes to the Recycle Bin instead of being deleted: the user's own files (orange),
/// with Settings > Cleaning's "Send my own files to the Recycle Bin" on (`on`). Caches (green,
/// amber) are deleted: recycling them would free nothing. A program's own clean-up command
/// decides for itself.
pub(crate) fn goes_to_bin(f: &Finding, on: bool) -> bool {
    // ticked on a second click: always, whatever the setting (R7c)
    if f.forced_from.is_some() {
        return true;
    }
    // a link (F4) deletes nothing: the kept copy holds the bytes
    on && f.risk >= Risk::Orange && !f.privacy() && !f.links() && !matches!(clean::action(f), Some((Action::Tool { .. }, _)))
}

/// Whether "Tick enough for N GB" may pick `f`: not what can't be undone, nor git's removals.
pub(crate) fn goal_may_pick(f: &Finding) -> bool {
    !crate::advisor::gone_for_good(f) && !matches!(&f.source, Source::Rule(id) if id == sprawl_advisor::devgit::WORKTREE || id == sprawl_advisor::devgit::CLONE)
}

/// The built-in rule that empties the Recycle Bin.
pub(crate) const EMPTY_BIN: &str = "win.recycle-bin";

/// When a run sends anything to the Recycle Bin, emptying the bin goes first: after, it would
/// erase for good what the run just sent there (Fable 1.0.4-1.0.5 P1-1). `key`: whether an
/// item empties the bin, and whether it goes to the bin.
pub(crate) fn empty_bin_first<A, B>(work: &mut Vec<A>, items: &mut Vec<B>, key: impl Fn(&B) -> (bool, bool)) {
    if !items.iter().any(|i| key(i).1) {
        return;
    }
    if let Some(k) = items.iter().position(|i| key(i).0).filter(|&k| k > 0 && k < work.len()) {
        let w = work.remove(k);
        work.insert(0, w);
        let it = items.remove(k);
        items.insert(0, it);
    }
}

/// Whether `f` may be in a sum of space, a tick Sprawl makes, the usual set or a suggestion:
/// never a privacy part. The report keeps those apart already; this holds every such place to it.
pub(crate) fn counts_as_space(f: &Finding) -> bool {
    !f.privacy()
}

/// What "Tick the safe ones" may tick.
pub(crate) fn safe_tick(f: &Finding, counted: bool) -> bool {
    counts_as_space(f) && untickable(f, counted).is_none() && held_in_app(f).is_none() && f.risk == Risk::Green && !sprawl_advisor::remedies::compresses(f) && !sprawl_advisor::remedies::shrinks(f) && f.builtin && !f.heuristic() && !f.other_account() && !matches!(f.source, Source::Model(_)) && f.bytes >= 1 << 20
}

/// It needs administrator rights and this Sprawl doesn't have them: it can be ticked (for a
/// script), but the in-app clean-up holds it back.
pub(crate) fn needs_admin(f: &Finding) -> bool {
    clean::action(f).is_some_and(|(_, admin)| admin) && !sprawl_core::is_elevated()
}

/// Why the in-app clean-up holds a ticked item back before it starts: it needs administrator,
/// or (elevated) its command starts a program from a folder the user can change.
pub(crate) fn held_in_app(f: &Finding) -> Option<String> {
    if needs_admin(f) {
        return Some("needs administrator: restart as administrator, or save as a script".into());
    }
    // as another administrator account (its password typed at the prompt), Windows would
    // empty that account's bin, not the signed-in user's, and free nothing shown (Fable
    // 1.0.4-1.0.5 P3-9). Not knowing counts as another account.
    if matches!(&f.source, Source::Rule(id) if id == EMPTY_BIN) && sprawl_core::elevation().unwrap_or(true) && sprawl_core::identity::shell_is_this_account() != Some(true) {
        return Some("Sprawl runs as another account than the one signed in, so it would empty that account's Recycle Bin, not yours: empty it from the desktop".into());
    }
    match clean::action(f) {
        Some((Action::Tool { command }, _)) => clean::elevated_refusal(&command),
        _ => None,
    }
}

impl State {
    /// Why the run would leave `f` as it is: [`held_in_app`], or a Recycle Bin it should go to
    /// that its drive doesn't have (a network drive). Sprawl never deletes for good instead.
    pub(crate) fn held_here(&self, f: &Finding) -> Option<String> {
        held_in_app(f).or_else(|| {
            let why = goes_to_bin(f, self.recycle_on()).then(|| sprawl_advisor::recycle::no_bin(&f.path)).flatten()?;
            Some(format!("{why}, so it would stay: Sprawl doesn't delete for good what is meant for the bin. Turn \"Send my own files to the Recycle Bin\" off in Settings to delete such files for good"))
        })
    }

    fn selection_context(&self) -> Option<SelectionContext> {
        let (tree, r) = (self.active_tree()?, self.advice()?);
        Some(SelectionContext { tree: Arc::downgrade(&tree), findings: r.findings.clone(), privacy: r.privacy.clone(), definitions: r.catalogue.clone() })
    }

    pub(crate) fn cleanup_context_sync(&mut self) -> bool {
        let unchanged = self.advisor.cleanup.context.as_ref().zip(self.active_tree()).zip(self.advice()).is_some_and(|((old, tree), report)|
            old.tree.upgrade().is_some_and(|previous| Arc::ptr_eq(&previous, &tree))
                && old.findings == report.findings && old.privacy == report.privacy && old.definitions == report.catalogue);
        if unchanged { return self.advisor.cleanup.sync_confirmation_selection(); }
        let next = self.selection_context();
        let changed = self.advisor.cleanup.bind(next);
        let selection_changed = self.advisor.cleanup.sync_confirmation_selection();
        if changed { self.advisor.cl.preview = None; }
        changed || selection_changed
    }

    /// An explicit user change to a finding keeps their ticks, but requires fresh consent.
    pub(crate) fn cleanup_accept_context(&mut self) {
        self.advisor.cleanup.context = self.selection_context();
        self.advisor.cleanup.own_ok = false;
        self.advisor.cleanup.privacy_ok = false;
        self.advisor.cleanup.measured.clear();
        self.advisor.cleanup.measuring = None;
        self.advisor.cl.preview = None;
    }

    /// Ticks for a new report: the user's own ticks that still apply, else none. Sprawl never
    /// ticks a delete by itself; "Tick the safe ones" does it on request.
    pub(crate) fn cleanup_defaults(&mut self) {
        self.cleanup_context_sync();
        let Some(r) = self.advice() else { return };
        if self.advisor.cleanup.ticked_for == Some(r.tree) {
            return;
        }
        let gen = r.tree;
        if let Some(p) = self.advisor.cleanup.pending.as_ref() {
            let ticked: HashSet<String> = r.findings.iter().zip(&r.counted).filter(|(f, c)| untickable(f, **c).is_none()).map(|(f, _)| key(&f.path)).filter(|k| p.contains(k)).collect();
            let c = &mut self.advisor.cleanup;
            c.ticked = ticked;
            c.ticked_for = Some(gen);
            c.auto = false;
            c.pending = None;
            let left = std::mem::take(&mut c.pending_left);
            if left > 0 {
                self.adv_say(&format!("{left} of the ticks couldn't be carried over the restart: tick those again."));
            }
            return;
        }
        // ticks the user chose stay, for findings that are still there and can be ticked
        if !self.advisor.cleanup.auto && self.advisor.cleanup.ticked_for.is_some() {
            let old = &self.advisor.cleanup.ticked;
            let kept: HashSet<String> = r.findings.iter().zip(&r.counted).filter(|(f, c)| untickable(f, **c).is_none()).map(|(f, _)| key(&f.path)).filter(|k| old.contains(k)).collect();
            let c = &mut self.advisor.cleanup;
            c.ticked = kept;
            c.ticked_for = Some(gen);
            c.review = false;
            c.single = None;
            return;
        }
        let c = &mut self.advisor.cleanup;
        c.ticked.clear();
        c.ticked_for = Some(gen);
        c.review = false;
        c.own_ok = false;
        c.single = None;
        c.auto = false;
    }

    /// What "Tick the safe ones" ticks: what comes back by itself, from Sprawl's own rules (not a
    /// guess by shape), inside the user's own profile, 1 MB or more.
    fn safe_ticks(&self) -> HashSet<String> {
        let Some(r) = self.advice() else { return HashSet::new() };
        r.findings.iter().zip(&r.counted).filter(|(f, c)| safe_tick(f, **c) && !r.quiet.contains(&key(&f.path))).map(|(f, _)| key(&f.path)).collect()
    }

    /// The ticked findings of the report on screen, in list order (index, finding).
    pub(crate) fn ticked_findings(&self) -> Vec<(usize, Finding)> {
        let Some(r) = self.advice() else { return Vec::new() };
        r.findings
            .iter()
            .enumerate()
            .filter(|(i, f)| {
                let k = key(&f.path);
                let chosen = match &self.advisor.cleanup.single {
                    Some(one) => *one == k,
                    None => self.advisor.cleanup.ticked.contains(&k),
                };
                chosen && untickable(f, r.counted[*i]).is_none()
            })
            .map(|(i, f)| (i, self.with_choices(f)))
            .collect()
    }

    /// `f` as the user set it up: a compress with the algorithm they picked. The clean-up's
    /// checks then run on that command.
    pub(crate) fn with_choices(&self, f: &Finding) -> Finding {
        self.advisor.cleanup.algs.get(&key(&f.path))
            .and_then(|alg| sprawl_advisor::remedies::with_compress_alg(f, alg))
            .unwrap_or_else(|| f.clone())
    }

    /// The algorithm a compress item will use: the user's pick, or the one Sprawl chose.
    pub(crate) fn compress_alg_of(&self, f: &Finding) -> Option<String> {
        let f = self.with_choices(f);
        match sprawl_advisor::clean::action(&f) {
            Some((sprawl_advisor::clean::Action::Tool { command }, _)) => sprawl_advisor::remedies::compress_alg(&command).map(str::to_string),
            _ => None,
        }
    }

    /// The ticked privacy parts of the report on screen. Never for a one-row review.
    pub(crate) fn ticked_privacy(&self) -> Vec<Finding> {
        if !self.privacy_on() { return Vec::new(); }
        let Some(r) = self.advice() else { return Vec::new() };
        if self.advisor.cleanup.single.is_some() {
            return Vec::new();
        }
        r.privacy.iter().filter(|f| self.advisor.cleanup.privacy_ticked.contains(&key(&f.path))).cloned().collect()
    }

    /// Tick or untick privacy part `i` (the user's own tick: the only way one is ticked).
    pub(crate) fn privacy_toggle(&mut self, i: usize) {
        if !self.privacy_on() { return; }
        let Some(k) = self.advice().and_then(|r| r.privacy.get(i)).map(|f| key(&f.path)) else { return };
        let c = &mut self.advisor.cleanup;
        if !c.privacy_ticked.remove(&k) {
            c.privacy_ticked.insert(k);
        }
        c.privacy_ok = false;
        c.single = None;
    }

    pub(crate) fn cleanup_toggle(&mut self, i: usize) {
        let Some(f) = self.advice().and_then(|r| r.findings.get(i)).map(|f| key(&f.path)) else { return };
        let c = &mut self.advisor.cleanup;
        // the user's own now
        c.ai_ticked.remove(&f);
        if !c.ticked.remove(&f) {
            c.ticked.insert(f);
        }
        // the ticks are what counts now, and a changed set is confirmed again
        c.single = None;
        c.own_ok = false;
        c.auto = false;
    }

    /// R7c: a row Sprawl doesn't tick, clicked. The first click says why it isn't ticked; the
    /// second makes it a delete of the whole path to the Recycle Bin and ticks it, once the
    /// deletion checks pass for it as such (the never-list, links, the scan's root). It is
    /// checked again at Clean up like every other, and the review asks for the "my own files"
    /// box. A new report (Check again, a filter) has it as it was.
    pub(crate) fn second_try_click(&mut self, i: usize) {
        if self.own_active() || self.advisor.cleanup.running.is_some() {
            return;
        }
        let Some(r) = self.advisor.report.as_ref().filter(|r| r.tree == self.tree_gen) else { return };
        let Some(f) = r.findings.get(i) else { return };
        if !second_try(f, r.counted[i]) {
            return;
        }
        let k = key(&f.path);
        if self.advisor.cleanup.armed.as_deref() != Some(k.as_str()) {
            self.advisor.cleanup.armed = Some(k);
            return;
        }
        self.advisor.cleanup.armed = None;
        let mut g = f.clone();
        g.forced_from = Some(g.remedy);
        g.remedy = Remedy::Delete;
        g.risk = g.risk.max(Risk::Orange);
        g.on_path = true;
        // Sprawl writes this command itself (the same Remove a left-behind folder gets)
        g.trusted = true;
        g.set_action(Action::Remove, false);
        g.evidence.push("You ticked this yourself: all of it goes to the Recycle Bin".into());
        let Some(t) = self.active_tree() else { return };
        let never = sprawl_advisor::safety::Never::default_list();
        match sprawl_advisor::safety::check(&g, &t, &never) {
            Ok(g) => {
                if let Some(r) = self.advisor.report.as_mut() {
                    r.findings[i] = g;
                }
                let c = &mut self.advisor.cleanup;
                c.ticked.insert(k);
                c.single = None;
                c.own_ok = false;
                c.auto = false;
                self.cleanup_accept_context();
            }
            Err(why) => {
                let title = f.title.clone();
                self.adv_say(&format!("{title} can't be ticked: {why}"));
            }
        }
    }

    /// Tick all of `keys` (a cleaner's part), or untick them when they are all ticked already.
    pub(crate) fn cleanup_tick_many(&mut self, keys: Vec<String>) {
        if keys.is_empty() {
            return;
        }
        let c = &mut self.advisor.cleanup;
        if keys.iter().all(|k| c.ticked.contains(k)) {
            for k in &keys {
                c.ticked.remove(k);
            }
        } else {
            c.ticked.extend(keys);
        }
        c.single = None;
        c.own_ok = false;
        c.auto = false;
    }

    /// Tick what comes back by itself (on the user's request).
    pub(crate) fn cleanup_select_safe(&mut self) {
        self.cleanup_defaults();
        let safe = self.safe_ticks();
        let c = &mut self.advisor.cleanup;
        c.ticked = safe;
        c.single = None;
        c.own_ok = false;
        c.auto = true;
    }

    /// Tick enough for `need` bytes: the plan's choice (green first, then amber).
    pub(crate) fn cleanup_select_enough(&mut self, need: u64) -> Option<u64> {
        let r = self.advice()?;
        // never what can't be undone (emptying the bin, Windows.old) or git's removals: the user
        // ticks those by hand (Fable 1.0.4-1.0.5 P2-5)
        let allowed: Vec<bool> = r.findings.iter().zip(&r.counted).map(|(f, &c)| c && !r.quiet.contains(&key(&f.path)) && goal_may_pick(f)).collect();
        let plan = sprawl_advisor::plan::plan(&r.findings, &allowed, need);
        let ticked: HashSet<String> = plan.picked.iter().filter(|&&i| !sprawl_advisor::remedies::compresses(&r.findings[i]) && !sprawl_advisor::remedies::shrinks(&r.findings[i]) && counts_as_space(&r.findings[i]) && untickable(&r.findings[i], r.counted[i]).is_none() && held_in_app(&r.findings[i]).is_none()).map(|&i| key(&r.findings[i].path)).collect();
        let bytes = r.findings.iter().filter(|f| ticked.contains(&key(&f.path))).map(|f| f.bytes).sum();
        self.advisor.cleanup.ticked = ticked;
        self.advisor.cleanup.auto = false;
        Some(bytes)
    }

    /// Start cleaning what is ticked: each item is checked again first.
    pub(crate) fn cleanup_start(&mut self) {
        if crate::settings::read_only() || self.advisor.cleanup.running.is_some() || self.advisor.file_op() || self.job.is_some() || self.update.applying {
            return;
        }
        if self.cleanup_context_sync() {
            self.adv_say("The scan or cleanup actions changed. Review the current items and confirm again.");
            self.render_advisor();
            return;
        }
        // approved rules since the review: the findings may be classed differently now
        if self.owed_before_cleaning() {
            self.advisor.cleanup.review = false;
            return;
        }
        // checked against the whole scan, not a filtered view of it
        let Some(t) = self.active_tree() else { return };
        let picked = self.ticked_findings();
        let privacy = self.ticked_privacy();
        if !privacy.is_empty() && !crate::settings::Settings::disk_enabled(sprawl_advisor::privacy::SETTING) {
            self.advisor.cleanup.clear_privacy_selection();
            self.adv_say("Privacy cleaners are off or their setting couldn't be read. Review the remaining items again.");
            self.render_advisor();
            return;
        }
        if picked.is_empty() && privacy.is_empty() {
            return;
        }
        if !privacy.is_empty() && !self.advisor.cleanup.privacy_ok {
            self.adv_say("Tick \"I understand what each of these removes\" first, or untick the privacy items.");
            return;
        }
        // the Keep list must be known: not read, nothing is cleaned; and what is on disk now
        // counts too (another Sprawl window may have kept something since)
        if let Some(e) = self.advisor.memory_error.clone() {
            self.adv_say(&format!("Nothing was cleaned. {}", crate::advisor::memory_unreadable(&e)));
            return;
        }
        // A command held back on screen cannot become runnable behind that review, even if
        // another instance removed a protection. The executor also checks current protections.
        let (picked, mut held) = {
            let reviewed = self.advisor.cleanup.reviewed_tools.borrow();
            let mut held = Vec::new();
            let picked: Vec<_> = picked.into_iter().filter(|(_, f)| {
                let Some(command) = external_command(f) else { return true; };
                if self.advisor.cleanup.review && reviewed.get(&(f.path.clone(), command)).is_some_and(|r| r.as_ref().is_ok_and(|r| r.matches_finding(f))) { return true; }
                held.push(format!("{}: not cleaned, the external command wasn't shown as runnable in this review", f.title));
                false
            }).collect();
            (picked, held)
        };
        let needs_ok = picked.iter().any(|(_, f)| f.risk >= Risk::Orange);
        if needs_ok && !self.advisor.cleanup.own_ok {
            self.adv_say("Tick \"These are my own files and I want them gone\" first, or untick your own files.");
            return;
        }
        // a tool command from the user's own folders, as administrator: allowed in Settings, asked
        // once; refused, the executor holds it back with the reason
        let risky = picked.iter().any(|(_, f)| matches!(clean::action(f), Some((Action::Tool { command }, _)) if clean::elevated_risk(&command).is_some()));
        if risky {
            if let Err(why) = self.admin_allows(&crate::admin::USER_TOOLS) {
                self.adv_say(&why);
            }
        }
        let never = sprawl_advisor::safety::Never::default_list();
        // what the run leaves: anything new since the scan began, and what the user keeps
        let mut spare = self.spare();
        if let Err(e) = sprawl_advisor::memory::refresh_keeps(&mut spare.keep) {
            self.adv_say(&format!("Nothing was cleaned: {e}. Try again."));
            return;
        }
        self.cleaner_procs(true);
        self.remember_usual(&picked.iter().map(|(_, f)| f.clone()).collect::<Vec<_>>());
        let bin_on = self.recycle_on();
        let mut work = Vec::new();
        let mut items = Vec::new();
        // the privacy sets after the rest, checked the same way (their own set checks inside)
        for f in picked.into_iter().map(|(_, f)| f).chain(privacy) {
            if let Some(why) = self.held_here(&f) {
                held.push(format!("{}: not cleaned, {why}", f.title));
                continue;
            }
            if spare.keeps(&f.path) {
                held.push(format!("{}: not cleaned, you chose to keep it", f.title));
                continue;
            }
            // the same checks as when it was found, against this scan; a registry part has no
            // path in the tree: its own module reads the fixed keys again and backs them up. A
            // finding merely *named* registry.* without an executable action never skips this.
            let registry_action = matches!(f.action, Some(Action::Reg { .. })) && clean::action(&f).is_some();
            let f = if registry_action {
                f
            } else {
                match sprawl_advisor::safety::check(&f, &t, &never) {
                    Ok(current) if review_changed(&f, &current) => {
                        held.push(format!("{}: its risk or cleanup scope changed. Check again and review the updated item before cleaning it.", f.title));
                        continue;
                    }
                    Ok(f) => f,
                    Err(why) => {
                        held.push(format!("{}: not cleaned, {why}", f.title));
                        continue;
                    }
                }
            };
            let Some((action, _)) = clean::action(&f) else {
                held.push(format!("{}: not cleaned, its command is not one Sprawl can run", f.title));
                continue;
            };
            let reviewed = if let Some(command) = external_command(&f) {
                let tools = self.advisor.cleanup.reviewed_tools.borrow();
                let Some(Ok(receipt)) = tools.get(&(f.path.clone(), command)).filter(|r| r.as_ref().is_ok_and(|r| r.matches_finding(&f))) else {
                    held.push(format!("{}: not cleaned, review the external command again", f.title));
                    continue;
                };
                Some(receipt.clone())
            } else { None };
            let compression = if let Some(command) = compression_command(&f) {
                let reviewed = self.advisor.cleanup.reviewed_compressions.borrow();
                let Some(Ok(receipt)) = reviewed.get(&(f.path.clone(), command.clone())).filter(|receipt|
                    self.advisor.cleanup.review && receipt.as_ref().is_ok_and(|r| r.matches(&f.path, &command))) else {
                    held.push(format!("{}: not compressed, review the directory again", f.title));
                    continue;
                };
                Some(receipt.clone())
            } else { None };
            // its application must be closed: checked now, and again just before it runs
            let gate = self.part_gate(&f);
            if let Some((app, Closed::Must)) = self.app_open(&f) {
                held.push(format!("{}: not cleaned, {app} is open. Close it, then clean up again.", f.title));
                continue;
            }
            let recycle = goes_to_bin(&f, bin_on);
            work.push((f.path.clone(), action, gate, recycle, f.privacy(), reviewed, compression));
            items.push(Item {
                path: f.path.clone(),
                title: f.title.clone(),
                bytes: f.bytes,
                state: ItemState::Waiting,
                measuring: false,
                rule: match &f.source {
                    sprawl_advisor::Source::Rule(r) => Some(r.clone()),
                    sprawl_advisor::Source::Model(_) => None,
                },
                recycle,
                privacy: f.privacy(),
                prevention: sprawl_advisor::prevention::for_finding(&f),
                learn: !f.privacy() && !f.heuristic()
                    && matches!(f.source, Source::Rule(_)) && !f.links()
                    && !sprawl_advisor::remedies::compresses(&f) && !sprawl_advisor::remedies::shrinks(&f),
                command: match clean::action(&f) {
                    Some((Action::Tool { command }, _)) => Some(command),
                    _ => None,
                },
                fate: outcome(&f, bin_on),
            });
        }
        empty_bin_first(&mut work, &mut items, |i: &Item| (i.rule.as_deref() == Some(EMPTY_BIN), i.recycle));
        if items.is_empty() {
            if held.is_empty() {
                self.adv_say("Nothing was cleaned: the items no longer pass the safety checks. Scan again.");
            } else {
                self.adv_say(&format!("Nothing was cleaned. {}", held.join(". ")));
            }
            return;
        }
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        std::thread::spawn(move || {
            // each volume's Recycle Bin, found once; what this run sends counts against its room
            let mut bins: std::collections::HashMap<String, Result<sprawl_advisor::recycle::Bin, String>> = Default::default();
            for (i, (path, action, gate, recycle, privacy, reviewed, compression)) in work.into_iter().enumerate() {
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                let _ = tx.send(Msg::Start(i));
                if privacy && !crate::settings::Settings::disk_enabled(sprawl_advisor::privacy::SETTING) {
                    let mut out = Outcome::default();
                    out.problems.push("Privacy cleaners are off or their setting couldn't be read.".into());
                    let _ = tx.send(Msg::Finished(i, out, None));
                    continue;
                }
                // an application that must be closed: the process list and the files' users,
                // just before; an answer Windows can't give counts as open
                if let Some((app, exes)) = &gate {
                    if let Some(why) = still_open(&path, app, exes) {
                        let mut out = Outcome::default();
                        out.problems.push(why);
                        let _ = tx.send(Msg::Finished(i, out, None));
                        continue;
                    }
                }
                let mut last = Instant::now();
                let bin = recycle.then(|| {
                    let vol = path.to_string_lossy().chars().take(2).collect::<String>().to_uppercase();
                    bins.entry(vol).or_insert_with(|| sprawl_advisor::recycle::bin_for(&path))
                });
                let tool = matches!(action, Action::Tool { .. });
                // a compress measures what it saved on each file it compresses: the folder isn't
                // read twice more for it
                let compress = sprawl_advisor::remedies::is_compress(&action);
                let in_place = sprawl_advisor::remedies::in_place(&action);
                let before = if compress { None }
                    else if in_place { sprawl_advisor::remedies::on_disk(&path) }
                    else if tool { complete_size(rescan_after(&path, false, None).as_ref()) } else { None };
                if let Err(why) = sprawl_advisor::memory::refresh_keeps(&mut spare.keep) {
                    let mut out = Outcome::default();
                    out.problems.push(why);
                    let _ = tx.send(Msg::Finished(i, out, None));
                    continue;
                }
                if spare.keeps(&path) {
                    let mut out = Outcome::default();
                    out.problems.push("You chose to keep this path after review.".into());
                    let _ = tx.send(Msg::Finished(i, out, None));
                    continue;
                }
                let mut progress = |freed| {
                    if last.elapsed().as_millis() >= 200 {
                        last = Instant::now();
                        let _ = tx.send(Msg::Progress(i, freed));
                    }
                };
                let mut out = if let Some(reviewed) = &reviewed {
                    clean::run_reviewed_external(reviewed, &spare, &stop2)
                } else if let Some(receipt) = &compression {
                    clean::run_reviewed_compression(&path, receipt, &spare, &stop2, &mut progress)
                } else {
                    clean::run_with(&path, &action, &spare, bin, &stop2, &mut progress)
                };
                // what is there now, for the tree
                let _ = tx.send(Msg::Measuring(i));
                let now = rescan_after(&path, compress, Some(&stop2));
                if compress {
                    // what the files compressed now saved, also when it was stopped part way
                    out.freed = out.compress.as_ref().map_or(0, |d| d.before.saturating_sub(d.after));
                } else if tool {
                    let after = if in_place { sprawl_advisor::remedies::on_disk(&path) }
                        else { complete_size(now.as_ref()) };
                    credit_tool(&mut out, before, after);
                }
                let _ = tx.send(Msg::Finished(i, out, now));
            }
            let _ = tx.send(Msg::End);
        });
        let at = std::time::Instant::now();
        self.own_deletes.extend(items.iter().map(|it| (it.path.to_string_lossy().trim_end_matches('\\').to_lowercase(), at)));
        let c = &mut self.advisor.cleanup;
        c.review = false;
        c.single = None;
        c.done = None;
        c.running = Some(Running { items, rx, stop, rescans: Vec::new(), held, stop_at: None, told_wait: false });
        self.advisor_redraw();
    }

    /// The review opened: count what each pattern or keep-days item would free now, by the
    /// clean-up's own tests, off the UI thread.
    pub(crate) fn cleanup_measure(&mut self) {
        // the report's own count is still going, and covers these
        if self.advisor.cleanup.measuring.is_some() {
            return;
        }
        let c = &self.advisor.cleanup;
        let ticked: Vec<Finding> = self.ticked_findings().into_iter().map(|(_, f)| f).filter(|f| !c.measured.contains_key(&key(&f.path))).collect();
        self.measure(ticked);
    }

    /// A new report: what each "up to" finding would free now, so the headline and the bar
    /// count that, not the scan's whole folder.
    pub(crate) fn cleanup_measure_report(&mut self) {
        self.advisor.cleanup.measured.clear();
        let all: Vec<Finding> = self.advice().map(|r| r.findings.iter().zip(&r.counted).filter(|(_, c)| **c).map(|(f, _)| f.clone()).collect()).unwrap_or_default();
        self.measure(all);
    }

    /// Count again, on a thread, what the run would free for each of `findings` that is an
    /// estimate; the answers replace the earlier ones as they come.
    fn measure(&mut self, findings: Vec<Finding>) {
        let spare = self.spare();
        let work: Vec<(PathBuf, Action)> = findings.into_iter().filter(|f| f.estimate()).filter_map(|f| Some((f.path.clone(), clean::action(&f)?.0))).collect();
        let c = &mut self.advisor.cleanup;
        if work.is_empty() {
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for (p, a) in work {
                if let Some(b) = clean::would_free(&p, &a, &spare) {
                    if tx.send((key(&p), b)).is_err() {
                        return;
                    }
                }
            }
        });
        c.measuring = Some(rx);
    }

    /// The size the review shows for a ticked finding: counted now, or "up to" the scan's.
    pub(crate) fn review_size(&self, f: &Finding) -> (u64, bool) {
        match self.advisor.cleanup.measured.get(&key(&f.path)) {
            Some(&b) => (b, false),
            None => match f.gain {
                Some(g) => (g, true),
                None => (f.bytes, f.estimate()),
            },
        }
    }

    /// Progress from the clean-up thread; at the end, the tree is updated in place.
    pub(crate) fn cleanup_poll(&mut self) {
        if let Some(rx) = &self.advisor.cleanup.measuring {
            let mut got = Vec::new();
            let done = loop {
                match rx.try_recv() {
                    Ok(m) => got.push(m),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break false,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break true,
                }
            };
            if done {
                self.advisor.cleanup.measuring = None;
            }
            let any = !got.is_empty();
            self.advisor.cleanup.measured.extend(got);
            // the review as answers come; the headline and the bar once all are in
            let review = self.advisor.cleanup.review && self.advisor.cleanup.running.is_none();
            if (review && any) || done {
                self.advisor_redraw();
            }
        }
        let Some(run) = self.advisor.cleanup.running.as_mut() else { return };
        let mut redraw = false;
        let mut ended = false;
        for m in run.rx.try_iter().collect::<Vec<_>>() {
            match m {
                Msg::Start(i) => {
                    if let Some(it) = run.items.get_mut(i) {
                        it.state = ItemState::Running(0);
                    }
                    redraw = true;
                }
                Msg::Progress(i, freed) => {
                    if let Some(it) = run.items.get_mut(i) {
                        it.state = ItemState::Running(freed);
                        self.advisor.slot_foot.text(&format!("AdvCS{i}"), &running_label(it, freed));
                    }
                }
                Msg::Measuring(i) => {
                    if let Some(it) = run.items.get_mut(i) {
                        it.measuring = true;
                        let so_far = match it.state { ItemState::Running(f) => f, _ => 0 };
                        self.advisor.slot_foot.text(&format!("AdvCS{i}"), &running_label(it, so_far));
                    }
                }
                Msg::Finished(i, out, now) => {
                    if let Some(it) = run.items.get_mut(i) {
                        if let Some(now) = now {
                            run.rescans.push((it.path.clone(), now));
                        }
                        it.state = ItemState::Done(out);
                    }
                    redraw = true;
                }
                Msg::End => ended = true,
            }
        }
        // Stop pressed and the item in hand still not back (a big file being compressed, a
        // rescan of a large folder, a slow drive): nothing after it starts, and the run ends
        // when it does
        if !ended && !run.told_wait && run.stop_at.is_some_and(|t| t.elapsed() >= STOP_WAIT) {
            run.told_wait = true;
            for (i, it) in run.items.iter().enumerate() {
                if matches!(it.state, ItemState::Running(_)) {
                    self.advisor.slot_foot.text(&format!("AdvCS{i}"), "Finishing this item: Windows can't stop it part way. Nothing after it starts.");
                }
            }
        }
        if ended {
            self.cleanup_finish();
        } else if redraw {
            self.advisor_redraw();
        }
    }

    fn cleanup_finish(&mut self) {
        if std::mem::take(&mut self.advisor.close_after_cleanup) {
            // "Stop and close": the run is recorded as usual (what was freed, a compress to
            // undo later), memory saved, then Sprawl closes
            self.cleanup_finish();
            self.save_memory();
            return self.exit_now();
        }
        let Some(run) = self.advisor.cleanup.running.take() else { return };
        log_run(&run.items, &run.held, run.stop.load(Ordering::Relaxed));
        let mut done = Done { problems: run.held.clone(), stopped: run.stop.load(Ordering::Relaxed), ..Done::default() };
        let learned_at = sprawl_advisor::learning::now();
        for it in &run.items {
            if let (true, Some(rule), ItemState::Done(o)) = (it.learn, &it.rule, &it.state) {
                if succeeded(o) && o.freed > 0 {
                    let remaining = run.rescans.iter().find(|(p, _)| p == &it.path).and_then(|(_, t)| match t {
                        None => Some(0), Some(t) if t.errors == 0 => Some(t.nodes[sprawl_core::ROOT].size), _ => None,
                    });
                    if remaining.is_some() { self.advisor.memory.learning.success(&it.path, rule, o.freed, remaining, learned_at); }
                }
            }
        }
        for it in &run.items {
            if let ItemState::Done(o) = &it.state {
                // a privacy set: counted on its own, never in what was freed; one that went is
                // unticked, one held back stays ticked for "Check again"
                if it.privacy {
                    let browser = it.rule.as_deref().and_then(sprawl_advisor::privacy::part).map(|p| p.browser().short);
                    // a registry part went when its values did; it has its backup to put back
                    let reg_went = o.registry.as_ref().is_some_and(|r| r.removed > 0) && o.problems.is_empty();
                    if let Some(reg) = &o.registry {
                        done.registry.push(reg.clone());
                    }
                    if (o.problems.is_empty() && (o.files > 0 || o.edited.is_some())) || reg_went {
                        if it.rule.as_deref().is_some_and(sprawl_advisor::regclean::is_id) {
                            done.registry_items += 1;
                        } else {
                            done.privacy_items += 1;
                        }
                        self.advisor.cleanup.privacy_ticked.remove(&key(&it.path));
                    } else {
                        for who in o.problems.iter().filter_map(|p| holder(p, browser)) {
                            if !done.privacy_open.contains(&who) {
                                done.privacy_open.push(who);
                            }
                        }
                    }
                    done.problems.extend(o.problems.iter().map(|p| format!("{}: {p}", it.title)));
                    // not a failure, but something the user should see: a copy left behind
                    done.problems.extend(o.notes.iter().map(|n| format!("{}: {n}", it.title)));
                    continue;
                }
                if o.freed > 0 {
                    done.items += 1;
                }
                // what happened to it, for the banner's words: only a delete deletes
                match it.fate {
                    Fate::Compress => {
                        if let Some(c) = &o.compress {
                            done.compressed_files += c.compressed;
                            done.already_compressed += c.already;
                            done.in_use += c.in_use;
                        }
                        if o.freed > 0 { done.compressed_items += 1; }
                    }
                    Fate::Link if o.freed > 0 => done.linked_items += 1,
                    Fate::Shrink if o.freed > 0 => done.shrunk_items += 1,
                    Fate::Gone | Fate::Tool if o.freed > 0 => done.deleted_items += 1,
                    _ => {}
                }
                if o.recycled > 0 {
                    done.recycled_items += 1;
                }
                done.freed += o.freed;
                done.recycled += o.recycled;
                if o.saving_unknown && succeeded(o) { done.unmeasured += 1; }
                done.kept_new += o.kept_new;
                done.kept_chosen += o.kept_chosen;
                done.kept_protected += o.kept_protected;
                done.problems.extend(o.problems.iter().take(3).cloned());
                if o.more_problems > 0 || o.problems.len() > 3 {
                    done.problems.push(format!("{}: and {} more", it.title, o.more_problems + o.problems.len().saturating_sub(3) as u64));
                }
                done.output.extend(o.output.iter().cloned());
                if succeeded(o) && (o.freed > 0 || o.recycled > 0) {
                    if let Some(tip) = it.prevention {
                        if !done.prevention.contains(&tip) { done.prevention.push(tip); }
                    }
                }
                // remembered as done when most of it went, never over the user's own Keep
                let kept = self.advisor.memory.hides(&it.path).is_some_and(|d| d.verdict == sprawl_advisor::memory::Verdict::Keep);
                // (a compress is remembered under Compressed instead, complete or not)
                if it.fate != Fate::Compress && succeeded(o) && o.freed.saturating_add(o.recycled) > 0 && o.freed.saturating_add(o.recycled) >= it.bytes / 2 && !kept {
                    self.advisor.memory.decide(&it.path, &it.title, sprawl_advisor::memory::Verdict::Done);
                }
            }
        }
        // what each cleaner part really freed this time, for the AI's numbers next time
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut freed: std::collections::BTreeMap<String, u64> = Default::default();
        for it in &run.items {
            if let (Some(rule), ItemState::Done(o), false) = (&it.rule, &it.state, it.privacy) {
                if !succeeded(o) || o.freed == 0 { continue; }
                *freed.entry(rule.clone()).or_default() += o.freed;
            }
        }
        for (rule, b) in freed {
            self.advisor.memory.freed.insert(rule, (b, now));
        }
        // the folders compressed, for "Compressed" and Uncompress (F1)
        for it in &run.items {
            let (Some(c), ItemState::Done(o)) = (&it.command, &it.state) else { continue };
            let Some(alg) = sprawl_advisor::remedies::compress_alg(c) else { continue };
            // compressed now (some files or all), or found compressed already by an earlier run:
            // recorded, so it can be undone
            let Some(d) = &o.compress else { continue };
            note_compressed(&mut self.advisor.memory.compressed, &it.path.display().to_string(), alg, now, it.bytes, o.freed, d, succeeded(o));
        }
        self.save_memory();
        self.advisor.cleanup.done = Some(done);
        // what ran no longer counts as ticked, nor is put back by the Cleaners tab on this
        // report; the other ticks stay as they were (a "Clean up just this" runs that one only)
        self.advisor.cl.preview = None;
        // (only the items that started: a stopped batch's waiting ones stay ticked)
        let c = &mut self.advisor.cleanup;
        for it in run.items.iter().filter(|it| !matches!(it.state, ItemState::Waiting)) {
            c.ticked.remove(&key(&it.path));
        }
        c.auto = false;
        // the tree as it is now: each item's folder scanned again, or taken out when gone
        self.apply_rescans(run.rescans);
        // the registry parts read again, so their counts and rows match what went
        self.refresh_registry_advice();
        // the Cleaners tab's own read: read again
        if self.own_active() {
            self.own_start();
        }
    }

    /// The clean-up's panel: running, or the result.
    pub(crate) fn cleanup_status_xaml(&self) -> String {
        let c = &self.advisor.cleanup;
        let mut x = String::new();
        if let Some(run) = &c.running {
            // a compress reports what it has gone through, not what it freed
            let freed: u64 = run.items.iter().filter(|it| !it.recycle && !it.privacy).map(|it| match &it.state {
                ItemState::Running(_) if it.fate == Fate::Compress => 0,
                ItemState::Running(f) => *f,
                ItemState::Done(o) => o.freed,
                ItemState::Waiting => 0,
            }).sum();
            let only_compress = run.items.iter().all(|it| it.fate == Fate::Compress);
            // a compress's saving is measured when it ends: meanwhile, how far through it is
            let through: u64 = run.items.iter().map(|it| match &it.state {
                ItemState::Running(f) => (*f).min(it.bytes),
                ItemState::Done(_) => it.bytes,
                ItemState::Waiting => 0,
            }).sum();
            let total: u64 = run.items.iter().map(|it| it.bytes).sum();
            let head = if only_compress && freed == 0 { format!("Compressing... {} of {} gone through", self.size(through), self.size(total)) }
                else if only_compress { format!("Compressing... {} of {} gone through, {} saved so far", self.size(through), self.size(total), self.size(freed)) }
                else { format!("Cleaning up... {} freed so far", self.size(freed)) };
            x += &format!(
                r##"<Border Padding="14,12" CornerRadius="{{$rc8}}" Background="{{$accentwash}}" BorderBrush="{{$accentline}}" BorderThickness="1"><StackPanel Spacing="8"><StackPanel Orientation="Horizontal" Spacing="10"><ProgressRing IsActive="True" Width="18" Height="18"/><TextBlock Text="{}" {num} FontSize="{{$fs15}}" FontWeight="SemiBold" VerticalAlignment="Center"/><Button x:Name="AdvCleanStop" Content="Stop"/></StackPanel>"##,
                ui::esc(&head),
                num = crate::look::NUM
            );
            for (i, it) in run.items.iter().enumerate() {
                let (glyph, colour, state) = match &it.state {
                    ItemState::Waiting => ("&#xE823;", crate::tokens::refer("text3"), "Waiting".to_string()),
                    ItemState::Running(_) if it.privacy => ("&#xE895;", crate::tokens::refer("link"), "Removing...".to_string()),
                    ItemState::Done(o) if it.privacy && it.rule.as_deref().is_some_and(sprawl_advisor::regclean::is_id) && o.registry.as_ref().is_none_or(|r| r.removed == 0) => ("&#xE7BA;", crate::tokens::refer("warn"), "Nothing was removed".to_string()),
                    ItemState::Done(o) if it.privacy && o.problems.is_empty() && (o.edited.is_some() || o.files > 0 || o.registry.as_ref().is_some_and(|r| r.removed > 0)) => ("&#xE73E;", crate::tokens::refer("good"), "Removed".to_string()),
                    ItemState::Done(o) if it.privacy && o.registry.as_ref().is_some_and(|r| r.removed > 0) => {
                        let n = o.registry.as_ref().map_or(0, |r| r.removed);
                        ("&#xE7BA;", crate::tokens::refer("warn"), format!("{n} {} removed; some couldn't go", if n == 1 { "value" } else { "values" }))
                    }
                    ItemState::Done(_) if it.privacy => ("&#xE7BA;", crate::tokens::refer("warn"), "Held back: nothing was changed".to_string()),
                    ItemState::Running(f) => ("&#xE895;", crate::tokens::refer("link"), running_label(it, *f)),
                    ItemState::Done(o) if it.fate == Fate::Compress => compress_label(o),
                    ItemState::Done(o) if it.recycle && o.problems.is_empty() => ("&#xE73E;", crate::tokens::refer("good"), format!("{} to the Recycle Bin", self.size(o.recycled))),
                    ItemState::Done(o) if it.recycle => ("&#xE7BA;", crate::tokens::refer("warn"), format!("{} to the Recycle Bin; some couldn't go", self.size(o.recycled))),
                    ItemState::Done(o) if o.saving_unknown && succeeded(o) => ("&#xE73E;", crate::tokens::refer("good"), "Done; the saving couldn't be measured".to_string()),
                    ItemState::Done(o) if o.problems.is_empty() => ("&#xE73E;", crate::tokens::refer("good"), format!("Freed {}", self.size(o.freed))),
                    ItemState::Done(o) => ("&#xE7BA;", crate::tokens::refer("warn"), format!("Freed {}; some couldn't go", self.size(o.freed))),
                };
                x += &format!(
                    r##"<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="18"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><FontIcon Glyph="{glyph}" FontSize="{{$fs13}}" Foreground="{colour}"/><TextBlock Grid.Column="1" Text="{}" ToolTipService.ToolTip="{0}" FontSize="{{$fs12}}" TextTrimming="CharacterEllipsis"/><TextBlock x:Name="AdvCS{i}" Grid.Column="2" Text="{}" {num} MinWidth="160" TextAlignment="Right" FontSize="{{$fs12}}" Foreground="{{$text2}}"/></Grid>"##,
                    ui::esc(&it.title),
                    ui::esc(&state),
                    num = crate::look::NUM
                );
            }
            x += "</StackPanel></Border>";
        } else if let Some(d) = &c.done {
            let fine = d.problems.is_empty() && !d.stopped && d.freed + d.recycled + (d.privacy_items + d.registry_items) as u64 + d.unmeasured as u64 > 0;
            let (bg, border, glyph, colour) = if fine { (crate::tokens::refer("goodwash"), crate::tokens::refer("goodline"), "&#xE73E;", crate::tokens::refer("good")) } else { (crate::tokens::refer("warnwash"), crate::tokens::refer("warnline"), "&#xE7BA;", crate::tokens::refer("warn")) };
            let items = if d.items == 1 { "1 item".to_string() } else { format!("{} items", d.items) };
            let head = match (d.stopped, d.items) {
                (true, 0) if d.recycled == 0 && d.privacy_items + d.registry_items == 0 => "Stopped. Nothing was freed.".to_string(),
                (false, 0) if d.recycled == 0 && d.privacy_items + d.registry_items == 0 => "Nothing was freed.".to_string(),
                (true, 0) => "Stopped.".to_string(),
                (false, 0) => String::new(),
                (true, _) => format!("Stopped. Freed {} from {items}.", self.size(d.freed)),
                (false, _) => format!("Freed {} from {items}.", self.size(d.freed)),
            };
            let binned = if d.recycled_items == 1 { "1 item".to_string() } else { format!("{} items", d.recycled_items) };
            let head = match d.recycled {
                0 => head,
                r => format!("{head} {} ({binned}) sent to the Recycle Bin.", self.size(r)).trim().to_string(),
            };
            let head = match d.unmeasured {
                0 => head,
                n => {
                    let head = if head == "Nothing was freed." { String::new() } else { head };
                    let what = if n == 1 { "1 tool finished; what it saved couldn't be measured.".to_string() } else { format!("{n} tools finished; what they saved couldn't be measured.") };
                    format!("{head} {what}").trim().to_string()
                }
            };
            let head = match d.privacy_items + d.registry_items {
                0 => head,
                n => format!("{head} {} removed.", privacy_count(n)).trim().to_string(),
            };
            // what happened to them, in plain words
            let gone = done_words(d);
            let gone = gone.as_str();
            let mut reg = String::new();
            if !d.registry.is_empty() {
                reg += r##"<TextBlock Text="Registry entries removed. Each part's values were copied to a backup first: Put back writes the removed values back; values written since stay." FontSize="{$fs12}" Foreground="{$text2}" TextWrapping="Wrap"/>"##;
                for (i, r) in d.registry.iter().enumerate() {
                    reg += &format!(
                        r##"<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBlock Text="{}" ToolTipService.ToolTip="{0}" FontSize="{{$fs12}}" VerticalAlignment="Center" TextTrimming="CharacterEllipsis"/><StackPanel Grid.Column="1" Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" {num} FontSize="{{$fs12}}" Foreground="{{$text2}}" VerticalAlignment="Center"/><Button x:Name="AdvRegBack{i}" Content="Put back"/></StackPanel></Grid>"##,
                        ui::esc(&r.title),
                        if r.removed == 1 { "1 value".to_string() } else { format!("{} values", r.removed) },
                        num = crate::look::NUM
                    );
                }
            }
            x += &format!(
                r##"<Border Padding="14,12" CornerRadius="{{$rc8}}" Background="{bg}" BorderBrush="{border}" BorderThickness="1"><StackPanel Spacing="6"><Grid><StackPanel Orientation="Horizontal" Spacing="10"><FontIcon Glyph="{glyph}" FontSize="{{$fs16}}" Foreground="{colour}"/><TextBlock Text="{}" {num} FontSize="{{$fs15}}" FontWeight="SemiBold"/></StackPanel><Button x:Name="AdvCleanedOk" HorizontalAlignment="Right" Content="OK"/></Grid>{reg}<TextBlock Text="{gone} The tree, the map and the findings are updated.{}" FontSize="{{$fs12}}" Foreground="{{$text2}}" TextWrapping="Wrap"/><StackPanel Orientation="Horizontal" Spacing="8"><Button x:Name="AdvCleanMore" Content="Find more with AI"/><Button x:Name="AdvCleanLog" Content="See what was removed"/></StackPanel>"##,
                ui::esc(&head),
                self.tree.as_ref().and_then(|t| crate::win32::disk_space(&t.path_of(sprawl_core::ROOT))).map_or(String::new(), |(_, free)| ui::esc(&format!(" The disk now has {} free.", self.size(free)))),
                num = crate::look::NUM
            );
            // a privacy set held because its browser was open leads somewhere
            if !d.privacy_open.is_empty() {
                x += &format!(
                    r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$warn}}" TextWrapping="Wrap" VerticalAlignment="Center"/><Button x:Name="AdvPvAgain" Content="Check again"/></StackPanel>"##,
                    ui::esc(&close_then(&d.privacy_open))
                );
            }
            if !d.problems.is_empty() {
                x += r##"<TextBlock Text="Some things couldn't go (in use, no access, or no room in the Recycle Bin): they stay as they were." FontSize="{$fs12}" Foreground="{$warn}" TextWrapping="Wrap"/>"##;
                for p in d.problems.iter().take(8) {
                    x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" TextWrapping="Wrap" Margin="10,0,0,0"/>"##, ui::esc(p));
                }
            }
            if d.kept_new + d.kept_chosen + d.kept_protected > 0 {
                let mut parts = Vec::new();
                if d.kept_new > 0 {
                    parts.push(format!("{} new since the scan", d.kept_new));
                }
                if d.kept_chosen > 0 {
                    parts.push(format!("{} you chose to keep", d.kept_chosen));
                }
                if d.kept_protected > 0 {
                    parts.push(format!("{} protected (mail stores, disk images and the like)", d.kept_protected));
                }
                x += &format!(r##"<TextBlock Text="Left alone: {}." FontSize="{{$fs12}}" Foreground="{{$text2}}" TextWrapping="Wrap"/>"##, ui::esc(&parts.join(", ")));
            }
            if !d.output.is_empty() {
                x += &format!(
                    r##"<Border Padding="10,8" CornerRadius="{{$rc5}}" Background="{{$shade}}"><TextBlock FontFamily="Consolas" FontSize="{{$fs12}}" TextWrapping="Wrap" IsTextSelectionEnabled="True" Text="{}"/></Border>"##,
                    d.output.iter().map(|l| ui::esc(l)).collect::<Vec<_>>().join("&#10;")
                );
            }
            if !d.prevention.is_empty() {
                x += r##"<TextBlock Text="Keep it from coming back" FontSize="{$fs13}" FontWeight="SemiBold" Margin="0,6,0,0"/>"##;
                for tip in d.prevention.iter().take(5) {
                    x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text2}}" TextWrapping="Wrap" IsTextSelectionEnabled="True"/>"##, ui::esc(tip));
                }
            }
            x += "</StackPanel></Border>";
        }
        x
    }

    /// The bar above the findings: what is ticked, and the ways to tick and clean.
    pub(crate) fn cleanup_bar_xaml(&self, goal: &str) -> String {
        let picked = self.ticked_findings();
        let privacy = self.ticked_privacy().len();
        let bytes: u64 = picked.iter().map(|(_, f)| self.review_size(f).0).sum();
        let up_to = picked.iter().any(|(_, f)| self.review_size(f).1);
        let num = crate::look::NUM;
        let admin_waiting = self.advice().is_some_and(|r| r.findings.iter().zip(&r.counted).any(|(f, c)| untickable(f, *c).is_none() && needs_admin(f)));
        // Nothing ticked: one line with the ways to tick. Review, Preview, Untick all and the
        // script have nothing to work on yet, so they wait until something is ticked.
        if picked.is_empty() && privacy == 0 && !admin_waiting {
            let options = if self.advisor.cleaners_tab_on() {
                ""
            } else {
                r##"<Button x:Name="AdvGoalOptions" Grid.Column="1" Content="Compare options for this goal" VerticalAlignment="Center"/>"##
            };
            return format!(
                r##"<Border Padding="12,8" CornerRadius="{{$rc8}}" Background="{{$surface3}}" BorderBrush="{{$line2}}" BorderThickness="1"><Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
<StackPanel Orientation="Horizontal" Spacing="6" VerticalAlignment="Center"><TextBlock Text="Nothing ticked" FontSize="{{$fs13}}" FontWeight="SemiBold" VerticalAlignment="Center" Margin="0,0,8,0"/><Button x:Name="AdvSelSafe" Content="Tick the safe ones"/><TextBlock Text="or tick enough for" FontSize="{{$fs12}}" VerticalAlignment="Center" Margin="8,0,0,0" Foreground="{{$text2}}"/><TextBox x:Name="AdvGoal" Width="64" PlaceholderText="{goal}"/><TextBlock Text="GB" FontSize="{{$fs12}}" VerticalAlignment="Center" Foreground="{{$text2}}"/><Button x:Name="AdvSelGoal" Content="Tick"/></StackPanel>{options}</Grid></Border>"##
            );
        }
        // B2: what the ticked items take on disk now, and about what is left after
        let on_disk: u64 = picked.iter().map(|(_, f)| f.bytes).sum();
        let sizes = if picked.is_empty() {
            String::new()
        } else {
            format!(
                r##"<TextBlock {num} FontSize="{{$fs12}}" Foreground="{{$text2}}" Text="On disk now {}  ·  after {}{}  ·  frees {}{}"/>"##,
                self.size(on_disk), if up_to { "at least " } else { "about " }, self.size(on_disk.saturating_sub(bytes)), if up_to { "up to " } else { "about " }, self.size(bytes)
            )
        };
        // B6: the count, Review and the tick buttons on one line, side by side; the other ways
        // to tick on the next, the explanation last
        let mut x = format!(
            r##"<Border Padding="12,10" CornerRadius="{{$rc8}}" Background="{{$surface3}}" BorderBrush="{{$line2}}" BorderThickness="1"><StackPanel Spacing="8">
<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" {num} FontSize="{{$fs15}}" FontWeight="SemiBold" VerticalAlignment="Center" Margin="0,0,6,0"/><Button x:Name="AdvClean" Content="Review selection..." Style="{{StaticResource SprawlPrimary}}" IsEnabled="{}"/>{prev}<Border Width="1" Height="22" Margin="4,0" Background="{{$edge}}" VerticalAlignment="Center"/><Button x:Name="AdvSelSafe" Content="Tick the safe ones" VerticalAlignment="Center"/><Button x:Name="AdvSelNone" Content="Untick all" VerticalAlignment="Center"/></StackPanel>
{sizes}
<StackPanel Orientation="Horizontal" Spacing="6"><TextBlock Text="Or tick enough for" FontSize="{{$fs12}}" VerticalAlignment="Center" Foreground="{{$text2}}"/><TextBox x:Name="AdvGoal" Width="64" PlaceholderText="{goal}"/><TextBlock Text="GB" FontSize="{{$fs12}}" VerticalAlignment="Center" Foreground="{{$text2}}"/><Button x:Name="AdvSelGoal" Content="Tick"/></StackPanel>
{}"##,
            match (picked.is_empty(), privacy) {
                (true, 0) => "Nothing ticked".to_string(),
                (true, n) => format!("{} ticked", privacy_count(n)),
                (false, 0) => format!("{} ticked · {}{}", picked.len(), if up_to { "up to " } else { "" }, self.size(bytes)),
                (false, n) => format!("{} ticked · {}{}, and {}", picked.len(), if up_to { "up to " } else { "" }, self.size(bytes), privacy_count(n)),
            },
            if picked.is_empty() && privacy == 0 { "False" } else { "True" },
            if self.advisor.cleanup.auto && !picked.is_empty() {
                ui::note("The safe ones are ticked.", "Change the ticks, then review them: nothing goes before you confirm.")
            } else {
                ui::note("Review what is ticked:", "nothing goes before you confirm. Sprawl removes it itself and shows what was freed.")
            },
            prev = if self.advisor.cleaners_tab_on() {
                format!(
                    r##"<Button x:Name="AdvClPrev" Content="Preview" IsEnabled="{}" ToolTipService.ToolTip="The exact files that would go, counted now. Nothing is deleted."/>"##,
                    if picked.is_empty() || matches!(self.advisor.cl.preview, Some(crate::cleaners::PreviewRun::Counting(_))) { "False" } else { "True" }
                )
            } else {
                String::new()
            },
        );
        if admin_waiting {
            x += r##"<StackPanel Orientation="Horizontal" Spacing="8"><FontIcon Glyph="&#xE7EF;" FontSize="{$fs13}" Foreground="{$warn}"/><TextBlock Text="Some items need administrator rights." FontSize="{$fs12}" Foreground="{$text2}" VerticalAlignment="Center"/><Button x:Name="AdvAdmin" Content="Restart as administrator"/></StackPanel>"##;
        }
        if !self.advisor.cleaners_tab_on() {
            x += r##"<Button x:Name="AdvGoalOptions" Content="Compare options for this goal" HorizontalAlignment="Left"/>"##;
        }
        x += r##"<Button x:Name="AdvExport" Padding="0" FontSize="{$fs12}" BorderThickness="0" Background="Transparent" Foreground="{$text3}" Content="Save as a script instead..."/>"##;
        x += "</StackPanel></Border>";
        x
    }

    /// The review before anything goes.
    pub(crate) fn cleanup_review_xaml(&self) -> String {
        let all = self.ticked_findings();
        // what the run will hold back is shown with its reason, and not counted
        let policy = all.iter().any(|(_, f)| external_command(f).is_some())
            .then(|| clean::ToolProtection::load(&self.advisor.memory.kept_paths()));
        let mut held = Vec::new();
        let mut picked = Vec::new();
        let mut reviewed = self.advisor.cleanup.reviewed_tools.borrow_mut();
        for (i, f) in all {
            let compression_error = compression_command(&f).and_then(|command| {
                // Redraws must not replace the identity that was shown at review.
                let mut receipts = self.advisor.cleanup.reviewed_compressions.borrow_mut();
                receipts.entry((f.path.clone(), command.clone()))
                    .or_insert_with(|| sprawl_advisor::remedies::review_compression(&f.path, &command))
                    .as_ref().err().cloned()
            });
            let tool = external_command(&f);
            let tool_error = tool.as_ref().zip(policy.as_ref()).and_then(|(command, policy)| {
                let held_here = self.held_here(&f);
                reviewed.entry((f.path.clone(), command.clone())).or_insert_with(|| {
                    if let Some(why) = held_here { return Err(why); }
                    policy.as_ref().map_err(String::clone).and_then(|policy| clean::review_external(&f, policy))
                }).as_ref().err().cloned()
            });
            if let Some(why) = compression_error.or(tool_error).or_else(|| self.held_here(&f)) {
                held.push((f, why));
            } else {
                picked.push((i, f));
            }
        }
        let privacy = self.ticked_privacy();
        let bytes: u64 = picked.iter().map(|(_, f)| self.review_size(f).0).sum();
        let up_to = picked.iter().any(|(_, f)| self.review_size(f).1);
        let own = picked.iter().any(|(_, f)| f.risk >= Risk::Orange);
        let bin_on = self.recycle_on();
        let mut x = format!(
            r##"<Border Padding="14,12" CornerRadius="{{$rc8}}" Background="{{$surface3}}" BorderBrush="{{$accentline}}" BorderThickness="1"><StackPanel Spacing="8"><TextBlock Text="{}" FontSize="{{$fs16}}" FontWeight="SemiBold"/><TextBlock Text="{}" FontSize="{{$fs13}}" FontWeight="SemiBold" Foreground="{{$warn}}" TextWrapping="Wrap"/><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text2}}" TextWrapping="Wrap"/>"##,
            // all three escaped: the lead line quotes a group's name ("Deleted for good"), and
            // a quote left in would end the attribute, fail the whole review, and leave the bar
            // on screen as if Review did nothing
            ui::esc(&{
                // the privacy items: by number, never in the space
                // a shrink's win is known only after it ran (F3)
                let unknown = picked.iter().any(|(_, f)| sprawl_advisor::remedies::shrinks(f));
                let space = match picked.len() {
                    0 => String::new(),
                    n => format!(
                        "{}, {}",
                        if n == 1 { "1 item".to_string() } else { format!("{n} items") },
                        if unknown && bytes == 0 {
                            "the space known only after".to_string()
                        } else if unknown {
                            format!("{} and more", self.size(bytes))
                        } else if up_to {
                            format!("up to {}", self.size(bytes))
                        } else {
                            format!("about {}", self.size(bytes))
                        }
                    ),
                };
                match (space.is_empty(), privacy.len()) {
                    // everything ticked is held back
                    (true, 0) => "Nothing ticked can run now".to_string(),
                    (_, 0) => format!("Clean up {space}?"),
                    (true, n) => format!("Clean up {}?", privacy_count(n)),
                    (false, n) => format!("Clean up {space}, and {}?", privacy_count(n)),
                }
            }),
            // the same outcome list as the groups and the button
            ui::esc(&lead_line(&picked.iter().map(|(_, f)| outcome(f, bin_on)).collect::<Vec<_>>(), privacy.len())),
            ui::esc(&{
                let tools = picked.iter().filter(|(_, f)| external_command(f).is_some()).count();
                let in_place = picked.iter().filter(|(_, f)| sprawl_advisor::remedies::compresses(f) || sprawl_advisor::remedies::shrinks(f)).count();
                let own = format!(
                    "Sprawl removes them itself. Links inside are never followed: deleted for good, a link is removed as a link; with the Recycle Bin it stays where it is (what it points to stays either way). Files a program has locked are skipped, and anything created or changed since the scan began ({}) is left alone, as is anything you chose to keep.",
                    sprawl_core::local_time(self.active_began())
                );
                let tool = "Sprawl checks the program's cleanup scope again before launching it. An unknown scope or one that reaches a kept path or skipped folder is held back. Inside an allowed scope the program decides what goes, including files new since the scan.";
                let mut text = Vec::new();
                if tools + in_place < picked.len() { text.push(own.as_str()); }
                if tools > 0 { text.push(tool); }
                text.join(" ")
            })
        );
        // only privacy items: no space lines, no space list
        if picked.is_empty() {
            x = x.replace(r##"<TextBlock Text="" FontSize="{$fs13}" FontWeight="SemiBold" Foreground="{$warn}" TextWrapping="Wrap"/><TextBlock Text="" FontSize="{$fs12}" Foreground="{$text2}" TextWrapping="Wrap"/>"##, "");
        }
        // the rows scroll; the boxes and buttons under them stay in view
        x += r##"<ScrollViewer MaxHeight="190" VerticalScrollBarVisibility="Auto" HorizontalScrollBarVisibility="Disabled" Padding="0,0,12,0"><StackPanel Spacing="8">"##;
        // grouped by what will happen to each, the one that can't be undone first
        let mut rows: Vec<&(usize, Finding)> = picked.iter().collect();
        rows.sort_by_key(|(_, f)| outcome(f, bin_on) as u8);
        let mut group = None;
        for (_, f) in rows {
            let o = outcome(f, bin_on);
            if group != Some(o) {
                group = Some(o);
                let of: Vec<&Finding> = picked.iter().map(|(_, f)| f).filter(|f| outcome(f, bin_on) == o).collect();
                let size: u64 = of.iter().map(|f| self.review_size(f).0).sum();
                let n = if of.len() == 1 { "1 item".to_string() } else { format!("{} items", of.len()) };
                let total = if o == Fate::Link || size == 0 { String::new() } else { format!(" · {}", self.size(size)) };
                x += &format!(
                    r##"<TextBlock Text="{}" FontSize="{{$fs12}}" FontWeight="SemiBold" Foreground="{}" Margin="0,4,0,0"/>"##,
                    ui::esc(&format!("{} · {n}{total}", o.heading())),
                    if o == Fate::Gone { crate::tokens::refer("warn") } else { crate::tokens::refer("text1") }
                );
            }
            let what = crate::advisor::risk_words_for(f);
            let ai = if matches!(f.source, Source::Model(_)) { " · suggested by the AI" } else { "" };
            // a command is shown whole (R7): what runs is never cut short
            let wrap = if matches!(clean::action(f), Some((Action::Tool { .. }, _))) { r#"TextWrapping="Wrap" IsTextSelectionEnabled="True""# } else { r#"TextTrimming="CharacterEllipsis""# };
            x += &format!(
                r##"<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="10"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>{}<StackPanel Grid.Column="1"><TextBlock Text="{}" ToolTipService.ToolTip="{1}" FontSize="{{$fs13}}" TextTrimming="CharacterEllipsis"/><TextBlock Text="{}{ai}" ToolTipService.ToolTip="{2}" FontSize="{{$fs12}}" Foreground="{{$text3}}" {wrap}/></StackPanel><TextBlock Grid.Column="2" Text="{}" FontSize="{{$fs13}}" VerticalAlignment="Center"/></Grid>"##,
                crate::advisor::risk_mark(f.risk, 8, ""),
                ui::esc(&f.title),
                ui::esc(&match clean::action(f) {
                    Some((Action::Tool { command }, _)) => match reviewed.get(&(f.path.clone(), command.clone())) {
                        Some(Ok(receipt)) => format!("{what} · {}", receipt.description()),
                        _ => format!("{what} · runs: {command}"),
                    },
                    Some((Action::Link { keep, copies }, _)) => format!("{} to {}, compared again first", if copies.len() == 1 { "1 copy made a link".to_string() } else { format!("{} copies made links", copies.len()) }, keep.display()),
                    // not "gone for good": the bin can give it back (its group says so)
                    _ if goes_to_bin(f, bin_on) => {
                        let what = if f.forced_from.is_some() { "You ticked it yourself" } else if f.risk == Risk::Orange { "Your own files" } else { what };
                        format!("{what} · {}", f.path.display())
                    }
                    _ => format!("{what} · {}", f.path.display()),
                }),
                match self.review_size(f) {
                    (b, true) => format!("up to {}", self.size(b)),
                    (b, false) if f.estimate() => format!("{} now", self.size(b)),
                    (b, false) => self.size(b),
                }
            );
        }
        if !held.is_empty() {
            x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" FontWeight="SemiBold" Foreground="{{$text3}}" Margin="0,4,0,0"/>"##, ui::esc(&format!("Held back, not run · {}", if held.len() == 1 { "1 item".to_string() } else { format!("{} items", held.len()) })));
        }
        for (f, why) in &held {
            x += &format!(
                r##"<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="10"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><FontIcon Glyph="&#xE7BA;" FontSize="{{$fs10}}" Foreground="{{$warn}}"/><StackPanel Grid.Column="1"><TextBlock Text="{}" ToolTipService.ToolTip="{0}" FontSize="{{$fs13}}" TextTrimming="CharacterEllipsis" Foreground="{{$text3}}"/><TextBlock Text="Held back: {}" FontSize="{{$fs12}}" Foreground="{{$warn}}" TextWrapping="Wrap"/></StackPanel><TextBlock Grid.Column="2" Text="not run" FontSize="{{$fs13}}" VerticalAlignment="Center" Foreground="{{$text3}}"/></Grid>"##,
                ui::esc(&f.title),
                ui::esc(why),
            );
        }
        x += "</StackPanel></ScrollViewer>";
        if picked.is_empty() && held.is_empty() {
            x = x.replace(r##"<ScrollViewer MaxHeight="190" VerticalScrollBarVisibility="Auto" HorizontalScrollBarVisibility="Disabled" Padding="0,0,12,0"><StackPanel Spacing="8"></StackPanel></ScrollViewer>"##, "");
        }
        x += &self.privacy_review_xaml(&privacy);
        // everything changed since the scan began stays: an old scan leaves most of it
        if let Some(age) = scan_age_words(self.active_began(), std::time::SystemTime::now()) {
            x += &format!(
                r##"<StackPanel Orientation="Horizontal" Spacing="8"><FontIcon Glyph="&#xE823;" FontSize="{{$fs13}}" Foreground="{{$warn}}"/><TextBlock Text="The scan is {age} old. What changed since it began is left alone, so this may free less than shown. Scan again first for a full clean-up." FontSize="{{$fs12}}" Foreground="{{$warn}}" TextWrapping="Wrap" MaxWidth="760" VerticalAlignment="Center"/><Button x:Name="AdvRescanOld" Content="Scan again" VerticalAlignment="Center"/></StackPanel>"##
            );
        }
        for (_, f) in picked.iter().filter(|(_, f)| crate::advisor::gone_for_good(f) && !goes_to_bin(f, bin_on)) {
            x += &format!(r##"<TextBlock Text="{}: what is in it now cannot be brought back afterwards." FontSize="{{$fs12}}" Foreground="{{$warn}}" TextWrapping="Wrap"/>"##, ui::esc(&f.title));
        }
        // what the user keeps inside a ticked folder stays: say which
        let kept: Vec<PathBuf> = self
            .advisor
            .memory
            .kept_paths()
            .into_iter()
            .filter(|k| {
                let k = PathBuf::from(key(k));
                picked.iter().any(|(_, f)| !matches!(clean::action(f), Some((Action::Tool { .. }, _))) && k.starts_with(PathBuf::from(key(&f.path))))
            })
            .collect();
        if !kept.is_empty() {
            x += r##"<StackPanel Spacing="2"><TextBlock Text="Kept, because you chose to keep them:" FontSize="{$fs12}" Foreground="{$text2}"/>"##;
            for k in kept.iter().take(6) {
                x += &format!(r##"<TextBlock Text="{}" ToolTipService.ToolTip="{0}" FontSize="{{$fs12}}" Foreground="{{$text3}}" TextTrimming="CharacterEllipsis" Margin="10,0,0,0"/>"##, ui::esc(&k.display().to_string()));
            }
            if kept.len() > 6 {
                x += &format!(r##"<TextBlock Text="and {} more" FontSize="{{$fs12}}" Foreground="{{$text3}}" Margin="10,0,0,0"/>"##, kept.len() - 6);
            }
            x += "</StackPanel>";
        }
        // an open application: a part that must have it closed stays, the rest skip what it holds
        let mut must: Vec<String> = Vec::new();
        let mut warn: Vec<String> = Vec::new();
        // (a privacy set's browser is said in its own block)
        for (_, f) in &picked {
            match self.app_open(f) {
                Some((app, sprawl_advisor::cleaners::Closed::Must)) if !must.contains(&app) => must.push(app),
                Some((app, sprawl_advisor::cleaners::Closed::Warn)) if !warn.contains(&app) => warn.push(app),
                _ => {}
            }
        }
        if !must.is_empty() || !warn.is_empty() {
            x += r##"<StackPanel Spacing="4">"##;
            if !must.is_empty() {
                x += &format!(
                    r##"<TextBlock Text="{} {} open: close {} first, or its parts stay. Sprawl never closes a program." FontSize="{{$fs12}}" Foreground="{{$warn}}" TextWrapping="Wrap"/>"##,
                    ui::esc(&must.join(", ")),
                    if must.len() == 1 { "is" } else { "are" },
                    if must.len() == 1 { "it" } else { "them" }
                );
            }
            if !warn.is_empty() {
                x += &format!(r##"<TextBlock Text="{} {} open: files in use are skipped." FontSize="{{$fs12}}" Foreground="{{$warnquiet}}" TextWrapping="Wrap"/>"##, ui::esc(&warn.join(", ")), if warn.len() == 1 { "is" } else { "are" });
            }
            x += r##"<Button x:Name="AdvClCheck" Content="Check again"/></StackPanel>"##;
        }
        if own {
            // all of the user's own go to the bin: they can be brought back from there
            // copies made links lose nothing, but behave as one file from then on (F4)
            let mine = || picked.iter().filter(|(_, f)| f.risk >= Risk::Orange);
            let all_links = mine().all(|(_, f)| f.links());
            let all_binned = mine().all(|(_, f)| goes_to_bin(f, bin_on) || f.links());
            x += &format!(
                r##"<CheckBox x:Name="AdvOwnOk" IsChecked="{}" Content="{}" FontSize="{{$fs13}}"/>"##,
                if self.advisor.cleanup.own_ok { "True" } else { "False" },
                if all_links {
                    "These are my own files: make the copies links (then a change to one changes all)"
                } else if all_binned {
                    "These are my own files and I want them gone (to the Recycle Bin)"
                } else {
                    "These are my own files, I want them gone, and I know they can't be brought back"
                }
            );
        }
        let outcomes: Vec<Fate> = picked.iter().map(|(_, f)| outcome(f, bin_on)).collect();
        // red when anything is deleted for good (privacy sets skip the Recycle Bin too), the
        // accent when everything can be brought back or is a cleaner's own clean-up
        let style = if outcomes.contains(&Fate::Gone) || !privacy.is_empty() { "SprawlDanger" } else { "SprawlPrimary" };
        x += &format!(r##"<StackPanel Orientation="Horizontal" Spacing="8"><Button x:Name="AdvCleanNow" Content="{}" Style="{{StaticResource {style}}}" IsEnabled="{}"/>"##, ui::esc(&final_action(&outcomes, privacy.len())), if (picked.is_empty() && privacy.is_empty()) || (own && !self.advisor.cleanup.own_ok) || (!privacy.is_empty() && !self.advisor.cleanup.privacy_ok) { "False" } else { "True" });
        x += r##"<Button x:Name="AdvReviewBack" Content="Back"/></StackPanel>"##;
        x += "</StackPanel></Border>";
        x
    }
}

fn external_command(f: &Finding) -> Option<String> {
    let (action, _) = clean::action(f)?;
    if !clean::is_external_cleanup(&action) { return None; }
    match action { Action::Tool { command } => Some(command), _ => None }
}

fn compression_command(f: &Finding) -> Option<String> {
    let (action, _) = clean::action(f)?;
    if !sprawl_advisor::remedies::is_compress(&action) { return None; }
    match action { Action::Tool { command } => Some(command), _ => None }
}

/// What a clean-up does to one item, as the review groups them: the order is the review's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fate {
    /// Deleted, not to the Recycle Bin.
    Gone = 0,
    Bin = 1,
    /// Its own program's clean-up command runs.
    Tool = 2,
    /// Copies made links: nothing is deleted.
    Link = 3,
    /// Compressed in place (compact.exe): nothing is deleted, and it can be undone.
    Compress = 4,
    /// A virtual disk file shrunk: nothing inside it is deleted.
    Shrink = 5,
}

impl Fate {
    fn heading(self) -> &'static str {
        match self {
            Fate::Gone => "Deleted for good",
            Fate::Bin => "To the Recycle Bin",
            Fate::Tool => "Cleaned by its own program (it decides what goes; usually not to the Recycle Bin)",
            Fate::Link => "Copies made links",
            Fate::Compress => "Compressed in place (nothing deleted; can be undone)",
            Fate::Shrink => "Virtual disk shrunk (nothing inside it deleted)",
        }
    }
}

/// The same tests the run uses: a command, a link, the bin, or deleted for good.
pub(crate) fn outcome(f: &Finding, bin_on: bool) -> Fate {
    if sprawl_advisor::remedies::shrinks(f) {
        return Fate::Shrink;
    }
    if sprawl_advisor::remedies::compresses(f) {
        return Fate::Compress;
    }
    match clean::action(f) {
        Some((Action::Tool { .. }, _)) => Fate::Tool,
        _ if f.links() => Fate::Link,
        _ if goes_to_bin(f, bin_on) => Fate::Bin,
        _ => Fate::Gone,
    }
}

/// The review's first line, from the same outcomes as its groups and button: what is deleted
/// for good (privacy sets included: they never go to the Recycle Bin), what goes to the bin,
/// and what only changes in place.
pub(crate) fn lead_line(outcomes: &[Fate], privacy: usize) -> String {
    let count = |o: Fate| outcomes.iter().filter(|x| **x == o).count();
    let (gone, bin) = (count(Fate::Gone), count(Fate::Bin));
    let mut s: Vec<&str> = Vec::new();
    // each group named for what it is, never "the rest": compressed or linked items aren't
    // deleted at all (privacy wording promises nothing about recovery: deleted, not to the bin)
    match (gone > 0, privacy > 0) {
        (true, true) => s.push("What is under \"Deleted for good\", and the privacy items, are deleted without the Recycle Bin."),
        (true, false) if outcomes.iter().all(|o| *o == Fate::Gone) => s.push("Deleted for good, not sent to the Recycle Bin."),
        (true, false) => s.push("What is under \"Deleted for good\" is deleted without the Recycle Bin."),
        (false, true) => s.push("The privacy items are deleted without the Recycle Bin."),
        (false, false) => {}
    }
    if bin > 0 {
        // named whenever anything else is in the review too (a tool's clean-up, links...):
        // an unqualified "Sent to the Recycle Bin" would read as the whole review's fate
        s.push(if privacy > 0 || outcomes.iter().any(|o| *o != Fate::Bin) {
            "What is under \"To the Recycle Bin\" goes there: Explorer can put it back, and its space is freed when the bin is emptied. What can't go to the bin is left as it is."
        } else {
            "Sent to the Recycle Bin: Explorer can put them back. Their space is freed when the bin is emptied. What can't go to the bin (too big for it, or a drive without one) is left as it is."
        });
    }
    if count(Fate::Tool) > 0 {
        s.push("A program's own clean-up decides what it removes, usually for good rather than to the Recycle Bin.");
    }
    if count(Fate::Link) > 0 {
        s.push(if outcomes.iter().all(|o| *o == Fate::Link) && privacy == 0 {
            "Nothing is deleted: each copy becomes a link to the one kept, so every path still opens the same file. Then a change to one changes all."
        } else {
            "Copies made links keep their contents: the kept one holds the bytes, and copies with an extra stream (data of their own, such as a download mark) are left as they are."
        });
    }
    if count(Fate::Compress) > 0 {
        s.push("Compressed in place: nothing is deleted, and it can be undone.");
    }
    if count(Fate::Shrink) > 0 {
        s.push("Virtual disks are shrunk: nothing inside them is deleted.");
    }
    s.join(" ")
}

/// The review's button, naming what it does: "Delete 2 for good, recycle 1". Privacy sets are
/// counted as sets, in the same part as what is deleted for good (none goes to the bin).
pub(crate) fn final_action(outcomes: &[Fate], privacy: usize) -> String {
    let count = |o: Fate| outcomes.iter().filter(|x| **x == o).count();
    let gone = count(Fate::Gone);
    let mut parts = Vec::new();
    // privacy sets go in the same clause, which says the Recycle Bin is skipped for all of it
    match (gone, privacy) {
        (0, 0) => {}
        (n, 0) => parts.push(format!("delete {n} for good")),
        (0, p) => parts.push(format!("delete {} without the Recycle Bin", privacy_count(p))),
        (n, p) => parts.push(format!("delete {n} {} and {} without the Recycle Bin", if n == 1 { "item" } else { "items" }, privacy_count(p))),
    }
    let forever = parts.len();
    for (o, word) in [(Fate::Bin, "recycle {}"), (Fate::Tool, "run {} clean-up"), (Fate::Link, "link {}"), (Fate::Compress, "compress {}"), (Fate::Shrink, "shrink {}")] {
        let n = count(o);
        if n > 0 {
            let s = word.replace("{}", &n.to_string());
            parts.push(if o == Fate::Tool && n > 1 { s + "s" } else { s });
        }
    }
    if parts.is_empty() {
        return "Clean up now".into();
    }
    // too many for a button: what is deleted for good is always named, the rest counted
    if parts.len() > 3 {
        let rest = outcomes.len() - gone;
        parts = if forever > 0 { vec![parts.swap_remove(0), format!("clean up {rest} more")] } else { vec!["clean up now".to_string()] };
    }
    let s = parts.join(", ");
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or(s)
}

impl State {
    /// Settings > Cleaning, "Send my own files to the Recycle Bin" (on unless switched off).
    pub(crate) fn recycle_on(&self) -> bool {
        self.settings.bool("recycleOwn") != Some(false)
    }

    /// Draw the Advisor again when it is on screen.
    pub(crate) fn advisor_redraw(&mut self) {
        if self.view == crate::state::View::Advisor {
            self.render_advisor();
        }
    }
}

impl State {
    /// A message for the user, in the Advisor's foot next to the buttons (or the status line
    /// when the Advisor isn't on screen).
    pub(crate) fn adv_say(&self, msg: &str) {
        *self.advisor.msg.borrow_mut() = Some(msg.to_string());
        let shown = self.view == crate::state::View::Advisor
            && self.advisor.ffind::<crate::bindings::Microsoft::UI::Xaml::Controls::TextBlock>("AdvMsg").is_some_and(|t| {
                let _ = t.SetText(&msg.into());
                let _ = t.SetVisibility(crate::bindings::Microsoft::UI::Xaml::Visibility::Visible);
                self.advisor.slot_foot.touch("AdvMsg");
                true
            });
        if !shown {
            crate::state::text(&self.root, "StatusText", msg);
        }
    }

    /// The Advisor's foot, pinned under the list: the message, then the clean-up's status, review
    /// or bar, whatever section is open.
    pub(crate) fn advisor_foot_xaml(&self) -> String {
        let mut x = format!(r##"<StackPanel {} Spacing="8" Padding="18,10,18,12" BorderBrush="{{$line2}}" BorderThickness="0,1,0,0" Background="{{$panel}}">"##, crate::ui::NS);
        let msg = self.advisor.msg.borrow().clone();
        x += &format!(
            r##"<TextBlock x:Name="AdvMsg" Text="{}" FontSize="{{$fs12}}" Foreground="{{$link}}" TextWrapping="Wrap" Visibility="{}"/>"##,
            ui::esc(msg.as_deref().unwrap_or("")),
            if msg.is_some() { "Visible" } else { "Collapsed" }
        );
        let bare = x.len();
        x += &self.exported_xaml();
        let c = &self.advisor.cleanup;
        x += &self.cleanup_status_xaml();
        if c.running.is_none() {
            if c.review {
                x += &self.cleanup_review_xaml();
            } else if self.advice().is_some_and(|r| !r.privacy.is_empty() || r.findings.iter().zip(&r.counted).any(|(f, k)| !matches!(untickable(f, *k), Some("")))
                || !self.advisor.cleaners_tab_on() && (!r.games.is_empty() || self.advisor.goal_options || r.findings.iter().any(|f| f.remedy == Remedy::Move))) {
                x += &self.cleanup_bar_xaml(crate::advisor::GOAL_EXAMPLE);
            }
        }
        if msg.is_none() && x.len() == bare {
            return format!(r##"<StackPanel {}/>"##, crate::ui::NS);
        }
        x += "</StackPanel>";
        x
    }
}

/// The privacy wording of the review and the result card, held to `privacy::PROMISE_WORDS`.
pub(crate) const PV_REVIEW_HEAD: &str = "Privacy items, deleted without the Recycle Bin and not counted in the space above. Each one goes whole, or not at all, and only with its browser closed:";
pub(crate) const PV_REG_REVIEW: &str = "Registry items: the list's values go, the key itself stays, and every value is copied to a backup first. After the clean-up, Put back writes the removed values back: values written since stay:";
pub(crate) const PV_OK: &str = "I understand what each of these removes";
pub(crate) const PV_DONE: &str = "The privacy items are done: files deleted, or rows taken out of their databases. An edited database may not get smaller.";
pub(crate) const PV_REG_DONE: &str = "The Explorer registry lists were cleared. Explorer keeps some of them in memory: the change may only show after you sign out and back in.";

/// "1 privacy item", "3 privacy items".
pub(crate) fn privacy_count(n: usize) -> String {
    if n == 1 { "1 privacy item".into() } else { format!("{n} privacy items") }
}

/// Who held a privacy set back, from the executor's reason: its browser when that was open (or
/// Windows couldn't say), else the programs Windows named as using its files.
pub(crate) fn holder(problem: &str, browser: Option<&str>) -> Option<String> {
    if let Some(b) = browser.filter(|b| problem.contains(&format!("{b} is open")) || problem.contains(&format!("whether {b} is open"))) {
        return Some(b.to_string());
    }
    let rest = problem.split_once("in use by ")?.1;
    Some(rest.split_once(';').map_or(rest, |(names, _)| names).trim().to_string()).filter(|s| !s.is_empty())
}

/// What a set held back by an open program asks of the user.
pub(crate) fn close_then(browsers: &[String]) -> String {
    format!("Close {}, then Check again.", browsers.join(" and "))
}

impl State {
    /// The review's privacy block: each ticked part, what it removes and its exact files (the
    /// executor's own preview), a browser that is open, and a tick box of its own.
    fn privacy_review_xaml(&self, privacy: &[Finding]) -> String {
        if privacy.is_empty() {
            return String::new();
        }
        let spare = self.spare();
        let mut x = format!(r##"<Border Padding="10,8" CornerRadius="{{$rc6}}" Background="{{$surface1}}" BorderBrush="{{$warnline}}" BorderThickness="1"><StackPanel Spacing="6"><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text2}}" TextWrapping="Wrap"/>"##, ui::esc(PV_REVIEW_HEAD));
        if privacy.iter().any(|f| f.registry()) {
            x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text2}}" TextWrapping="Wrap"/>"##, ui::esc(PV_REG_REVIEW));
        }
        let mut open: Vec<String> = Vec::new();
        for f in privacy {
            let browser = match &f.source {
                Source::Rule(id) => sprawl_advisor::privacy::part(id),
                _ => None,
            };
            let reg = match &f.source {
                Source::Rule(id) => sprawl_advisor::regclean::part(id),
                _ => None,
            };
            let preview = clean::action(f).and_then(|(a, _)| clean::preview(&f.path, &a, &spare, 0));
            let files: Vec<String> = preview.as_ref().map(|p| p.biggest.iter().map(|(p, b)| format!("{} ({})", p.display(), self.size(*b))).collect()).unwrap_or_default();
            let rows = preview.as_ref().map_or(0, |p| p.rows);
            let n = reg.map_or(0, |p| self.registry_values(p.id));
            let held = self.app_open(f).map(|(app, _)| app);
            if let Some(app) = &held {
                let short = browser.map_or(app.as_str(), |p| p.browser().short).to_string();
                if !open.contains(&short) {
                    open.push(short);
                }
            }
            x += &format!(
                r##"<StackPanel Spacing="1"><StackPanel Orientation="Horizontal" Spacing="8">{}<TextBlock Text="{}" FontSize="{{$fs13}}"/></StackPanel><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$warn}}" TextWrapping="Wrap" Margin="16,0,0,0"/>"##,
                crate::advisor::risk_mark(f.risk, 8, crate::advisor::MID),
                ui::esc(&f.title),
                ui::esc(browser.map_or_else(|| reg.map_or("", |p| p.cost), |p| p.cost))
            );
            if reg.is_some() {
                x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" TextWrapping="Wrap" Margin="16,0,0,0"/>"##, ui::esc(&if n == 1 { "1 value would be removed, after it is copied to a backup.".to_string() } else { format!("{n} values would be removed, after they are copied to a backup.") }));
            } else if files.is_empty() {
                x += r##"<TextBlock Text="Its files couldn't be read now: it will be held back." FontSize="{$fs12}" Foreground="{$text3}" Margin="16,0,0,0"/>"##;
            }
            for line in &files {
                x += &format!(r##"<TextBlock Text="{}" {} FontSize="{{$fs12}}" Foreground="{{$text3}}" TextTrimming="CharacterEllipsis" Margin="16,0,0,0" ToolTipService.ToolTip="{0}"/>"##, ui::esc(line), crate::look::NUM);
            }
            if rows > 0 {
                x += &format!(
                    r##"<TextBlock Text="At least {} row{} would go, in one transaction. The file may not get smaller." FontSize="{{$fs12}}" Foreground="{{$text3}}" Margin="16,0,0,0" TextWrapping="Wrap"/>"##,
                    rows,
                    if rows == 1 { "" } else { "s" }
                );
            }
            x += "</StackPanel>";
        }
        x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" TextWrapping="Wrap"/>"##, ui::esc(sprawl_advisor::privacy::DELETED_NOTE));
        if !open.is_empty() {
            x += &format!(
                r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$warn}}" TextWrapping="Wrap" VerticalAlignment="Center"/><Button x:Name="AdvClCheck" Content="Check again"/></StackPanel>"##,
                ui::esc(&close_then(&open))
            );
        }
        x += &format!(r##"<CheckBox x:Name="AdvPrivOk" IsChecked="{}" Content="{}" FontSize="{{$fs13}}"/></StackPanel></Border>"##, if self.advisor.cleanup.privacy_ok { "True" } else { "False" }, PV_OK);
        x
    }
}

/// `%LOCALAPPDATA%\Sprawl\cleanup-log.txt`: what each clean-up removed, kept for the user.
pub(crate) fn log_path() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Sprawl").join("cleanup-log.txt"))
}

/// Add a clean-up to the log (time, each item's path, what it freed and what couldn't go).
fn log_run(items: &[Item], held: &[String], stopped: bool) {
    let Some(p) = log_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut s = format!("\r\n== {} (local time){}\r\n", sprawl_core::local_time(std::time::SystemTime::now()), if stopped { " (stopped)" } else { "" });
    for it in items {
        match &it.state {
            ItemState::Done(o) => {
                match o.edited {
                    // a database edited in place: rows, not bytes (a privacy part, never space)
                    Some(rows) => s += &format!("{}\t{} rows taken out of the database, in one transaction\r\n", it.path.display(), rows),
                    None => s += &format!("{}\tfreed {} bytes, {} files\r\n", it.path.display(), o.freed, o.files),
                }
                // a compress ran in Sprawl itself: what it did, file by file
                let alg = it.command.as_deref().and_then(sprawl_advisor::remedies::compress_alg);
                if let (Some(alg), Some(d)) = (alg, &o.compress) {
                    s += &format!("\t  compressed in place ({alg}, in Sprawl): {} compressed, {} compressed already, {} in use, {} changed since the plan, {} failed{}{}\r\n",
                        d.compressed, d.already, d.in_use, d.changed, d.failed,
                        if d.unmeasured > 0 { format!(", {} not measured", d.unmeasured) } else { String::new() },
                        if d.stopped { ", stopped" } else { "" });
                    if let Some(e) = &d.first_error {
                        s += &format!("\t  first error: {e}\r\n");
                    }
                } else if alg.is_some() {
                    s += "\t  compress in place (in Sprawl): not started\r\n";
                } else if let Some(c) = &it.command {
                    // a tool's command, as run, and the last lines it wrote (R7)
                    s += &format!("\t  ran: {c}\r\n");
                    for l in &o.output {
                        s += &format!("\t  | {l}\r\n");
                    }
                }
                if o.recycled > 0 {
                    s += &format!("\t  sent to the Recycle Bin: {} bytes, {} files\r\n", o.recycled, o.recycled_files);
                }
                if let Some(r) = &o.registry {
                    s += &format!("\t  registry values removed: {} ({})\r\n\t  backup: {}\r\n", r.removed, r.title, r.backup.display());
                }
                if o.kept_new > 0 {
                    s += &format!("\t  left {} items new since the scan\r\n", o.kept_new);
                }
                if o.kept_chosen > 0 {
                    s += &format!("\t  left {} items you chose to keep\r\n", o.kept_chosen);
                }
                for pr in &o.problems {
                    s += &format!("\t  not removed: {pr}\r\n");
                }
                if o.more_problems > 0 {
                    s += &format!("\t  and {} more not removed\r\n", o.more_problems);
                }
                for n in &o.notes {
                    s += &format!("\t  note: {n}\r\n");
                }
            }
            _ => s += &format!("{}\tnot started\r\n", it.path.display()),
        }
    }
    for h in held {
        s += &format!("held back: {h}\r\n");
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = f.write_all(s.as_bytes());
    }
}

#[cfg(test)]
mod admin_tests {
    use super::*;
    use sprawl_advisor::finding::Source;

    #[test]
    fn emptying_the_bin_goes_before_anything_sent_to_it() {
        // (name, empties the bin, goes to the bin)
        let run = |list: &[(&'static str, bool, bool)]| {
            let mut work: Vec<&str> = list.iter().map(|x| x.0).collect();
            let mut items = list.to_vec();
            empty_bin_first(&mut work, &mut items, |i| (i.1, i.2));
            assert_eq!(work, items.iter().map(|x| x.0).collect::<Vec<_>>());
            work
        };
        assert_eq!(run(&[("own", false, true), ("cache", false, false), ("bin", true, false)]), ["bin", "own", "cache"]);
        // nothing sent to the bin: the order stays
        assert_eq!(run(&[("cache", false, false), ("bin", true, false)]), ["cache", "bin"]);
        assert_eq!(run(&[("bin", true, false), ("own", false, true)]), ["bin", "own"]);
        assert_eq!(run(&[("own", false, true)]), ["own"]);
    }

    /// A privacy part as the engine makes one (cookies of a Chrome profile).
    fn privacy_finding(risk: Risk) -> Finding {
        let path = PathBuf::from(r"C:\Users\Sam\AppData\Local\Google\Chrome\User Data\Default\Network\Cookies");
        let mut f = Finding {
            path: path.clone(),
            owner: Some("Google Chrome".into()),
            bytes: 2 << 20,
            risk,
            remedy: Remedy::Delete,
            command: None,
            title: "Chrome: Cookies (Sam)".into(),
            evidence: vec![],
            confidence: 1.0,
            source: Source::Rule("privacy.chrome/cookies".into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: true,
            action: None,
            admin: false,
            carved: Vec::new(),
            gain: None,
            forced_from: None,
        };
        f.set_action(Action::Set { part: "privacy.chrome/cookies".into(), files: vec![path] }, false);
        f
    }

    /// Every place that sums, ticks, plans, saves or suggests skips a privacy part, whatever its
    /// risk or size: the headline sums and "Tick the safe ones" (counts_as_space, safe_tick),
    /// "tick enough for N GB" (the plan, and counts_as_space after it), the usual set
    /// (counts_as_space), the saved script, and the Recycle Bin.
    #[test]
    fn privacy_parts_are_never_summed_ticked_planned_or_saved() {
        for risk in [Risk::Green, Risk::Amber, Risk::Orange] {
            let f = privacy_finding(risk);
            assert!(f.privacy());
            assert!(!counts_as_space(&f), "headline sums and the usual set");
            assert!(!safe_tick(&f, true), "Tick the safe ones");
            assert!(sprawl_advisor::plan::plan(std::slice::from_ref(&f), &[true], 1).picked.is_empty(), "tick enough for N GB");
            let s = sprawl_advisor::export::script(&[&f], r"C:\", std::time::SystemTime::now());
            assert!(!s.contains("Cookies"), "the saved script");
            assert!(!goes_to_bin(&f, true), "a privacy set is never recycled");
        }
    }

    /// A registry part (Run box history) is a privacy part too: never in a sum, never ticked for
    /// the user, never to the Recycle Bin, never written to a script.
    #[test]
    fn registry_parts_are_privacy_parts_too() {
        let mut f = Finding {
            path: PathBuf::from(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\RunMRU"),
            owner: Some("Windows Explorer".into()),
            bytes: 0,
            risk: Risk::Amber,
            remedy: Remedy::Delete,
            command: None,
            title: "Windows Explorer: Run box history".into(),
            evidence: vec![],
            confidence: 1.0,
            source: Source::Rule("registry.explorer/runmru".into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: false,
            action: None,
            admin: false,
            carved: Vec::new(),
            gain: None,
            forced_from: None,
        };
        f.set_action(Action::Reg { part: "registry.explorer/runmru".into() }, false);
        assert!(f.privacy() && f.registry() && f.command.is_none());
        assert!(!counts_as_space(&f), "never in a space total");
        assert!(!goes_to_bin(&f, true), "never to the Recycle Bin");
        let s = sprawl_advisor::export::script(&[&f], r"C:\", std::time::SystemTime::now());
        assert!(!s.contains("RunMRU") && !s.contains("registry"), "never in a saved script: {s}");
    }

    /// The app's privacy wording promises nothing: the engine's word list, for every string the
    /// Privacy group, the review, the result card and Settings show.
    #[test]
    fn the_privacy_wording_promises_nothing() {
        let pref = crate::prefs::get("privacyCleaners").unwrap();
        let texts = [
            crate::cleaners::PV_HEAD.to_string(),
            crate::cleaners::PV_SUB.into(),
            crate::cleaners::PV_NONE.into(),
            crate::cleaners::PV_HELD.into(),
            PV_REVIEW_HEAD.into(),
            PV_REG_REVIEW.into(),
            PV_OK.into(),
            PV_DONE.into(),
            PV_REG_DONE.into(),
            privacy_count(1),
            privacy_count(3),
            close_then(&["Chrome".into(), "Edge".into()]),
            pref.label.into(),
            pref.help.into(),
            "Removing...".into(),
            "Removed".into(),
            "Held back: nothing was changed".into(),
            "Tick \"I understand what each of these removes\" first, or untick the privacy items.".into(),
        ];
        for t in &texts {
            let low = t.to_lowercase();
            for w in sprawl_advisor::privacy::PROMISE_WORDS {
                assert!(!low.contains(w), "{t:?} says {w:?}");
            }
        }
        assert_eq!(close_then(&["Chrome".into()]), "Close Chrome, then Check again.");
        // the program that held the set is the one named
        assert_eq!(holder(r"C:\x\Cookies: Chrome is open; nothing in this set was deleted", Some("Chrome")).as_deref(), Some("Chrome"));
        assert_eq!(holder(r"C:\x\Cookies: Windows couldn't say whether Chrome is open; nothing in this set was deleted", Some("Chrome")).as_deref(), Some("Chrome"));
        assert_eq!(holder(r"C:\x\Cookies: in use by Windows PowerShell 5.1, Notepad; nothing in this set was deleted", Some("Chrome")).as_deref(), Some("Windows PowerShell 5.1, Notepad"));
        assert_eq!(holder("no longer there, so the set changed since the scan; nothing in this set was deleted", Some("Chrome")), None);
        assert!(!"Nothing was removed.".to_lowercase().contains("for good"));
        assert_eq!(pref.default_value().as_deref(), Some("0"), "off by default");
    }

    /// Only the user's own files go to the Recycle Bin, and only with the setting on; caches and
    /// a program's own clean-up command are not recycled.
    #[test]
    fn only_own_files_go_to_the_bin() {
        let mk = |risk: Risk, action: Action| {
            let mut f = Finding {
                path: PathBuf::from(r"C:\Users\Sam\Old project"),
                owner: None,
                bytes: 1 << 20,
                risk,
                remedy: Remedy::Delete,
                command: None,
                title: "x".into(),
                evidence: vec![],
                confidence: 1.0,
                source: Source::Rule("t".into()),
                allow_protected: false,
                trusted: true,
                builtin: true,
                on_path: true,
                action: None,
                admin: false,
                carved: Vec::new(),
                gain: None,
                forced_from: None,
            };
            f.set_action(action, false);
            f
        };
        assert!(goes_to_bin(&mk(Risk::Orange, Action::Remove), true));
        assert!(goes_to_bin(&mk(Risk::Orange, Action::Match { pattern: "*.iso".into() }), true));
        assert!(!goes_to_bin(&mk(Risk::Orange, Action::Remove), false), "the setting off: deleted");
        assert!(!goes_to_bin(&mk(Risk::Green, Action::Remove), true), "a cache frees nothing in the bin");
        assert!(!goes_to_bin(&mk(Risk::Amber, Action::Clear { days: 0 }), true));
        assert!(!goes_to_bin(&mk(Risk::Orange, Action::Tool { command: "npm cache clean --force".into() }), true), "a tool decides itself");
    }

    #[test]
    fn an_item_that_needs_administrator_can_be_ticked_for_a_script() {
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Users\Sam"));
        let mut f = Finding {
            path: home.join(r"AppData\Local\Temp"),
            owner: None,
            bytes: 1 << 20,
            risk: Risk::Green,
            remedy: Remedy::Delete,
            command: None,
            title: "Temp".into(),
            evidence: vec![],
            confidence: 1.0,
            source: Source::Rule("t".into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: true,
            action: None,
            admin: false,
            carved: Vec::new(),
            gain: None,
            forced_from: None,
        };
        f.set_action(Action::Clear { days: 7 }, true);
        // tickable either way; the in-app run holds it back only when not elevated
        assert_eq!(untickable(&f, true), None);
        assert_eq!(held_in_app(&f).is_some(), !sprawl_core::is_elevated());
        f.set_action(Action::Clear { days: 7 }, false);
        assert_eq!(held_in_app(&f), None);
    }

    /// What a gone application left that holds the user's work is shown only: no tick. One
    /// without stays tickable, through the own-files box.
    #[test]
    fn a_leftover_holding_the_users_work_cannot_be_ticked() {
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Users\Sam"));
        let mut f = Finding {
            path: home.join(r"AppData\Local\GoneGame"),
            owner: None,
            bytes: 100 << 20,
            risk: Risk::Orange,
            remedy: Remedy::Info,
            command: None,
            title: "GoneGame (left behind, holds your saved games)".into(),
            evidence: vec![],
            confidence: 0.6,
            source: Source::Rule(sprawl_advisor::leftovers::ID.into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: true,
            action: None,
            admin: false,
            carved: Vec::new(),
            gain: None,
            forced_from: None,
        };
        assert!(untickable(&f, true).is_some());
        f.remedy = Remedy::Delete;
        f.set_action(Action::Remove, false);
        assert_eq!(untickable(&f, true), None);
        assert!(f.risk >= Risk::Orange, "the own-files box is needed");
    }
}

#[cfg(test)]
mod accounting_tests {
    use super::*;

    #[test]
    fn tool_credit_needs_success_and_complete_comparable_sizes() {
        for (before, after, expected) in [(Some(100), Some(40), 60), (Some(100), Some(100), 0),
            (Some(100), Some(120), 0), (None, Some(0), 0), (Some(100), None, 0)] {
            let mut out = Outcome::default();
            credit_tool(&mut out, before, after);
            assert_eq!(out.freed, expected);
            // unmeasured is not a failure, and credits nothing
            assert!(succeeded(&out));
            assert_eq!(out.saving_unknown, before.is_none() || after.is_none());
        }
        for mut out in [Outcome { problems: vec!["tool failed".into()], ..Default::default() },
            Outcome { stopped: true, ..Default::default() }, Outcome { more_problems: 1, ..Default::default() }] {
            out.freed = 900;
            credit_tool(&mut out, Some(1000), Some(1));
            assert_eq!(out.freed, 0, "failure cannot become a successful saving");
            assert!(!succeeded(&out));
        }
        let tree = |errors| Tree { volumes: Vec::new(), root_path: r"C:\Data".into(), nodes: vec![sprawl_core::Node { size: 40, ..Default::default() }], errors, method: sprawl_core::ScanMethod::Walk };
        assert_eq!(complete_size(None), None);
        assert_eq!(complete_size(Some(&None)), Some(0));
        assert_eq!(complete_size(Some(&Some(tree(1)))), None);
        assert_eq!(complete_size(Some(&Some(tree(0)))), Some(40));
    }
}

#[cfg(test)]
mod selection_context_tests {
    use super::*;

    #[test]
    fn changed_definitions_actions_and_selection_require_fresh_consent() {
        let tree = || Arc::new(Tree { volumes: Vec::new(), root_path: r"C:\Data".into(), nodes: vec![sprawl_core::Node::default()], errors: 0, method: sprawl_core::ScanMethod::Walk });
        let a = tree();
        let b = tree();
        let definitions = sprawl_advisor::cleaners::catalogue(&[sprawl_advisor::rules::RulePack::builtin()]);
        let rule = &definitions[0].parts[0].rule;
        let finding: Finding = serde_json::from_value(serde_json::json!({"path": "C:\\Data\\item", "bytes": 10, "risk": "amber", "remedy": "delete", "title": "item", "evidence": [], "confidence": 1.0, "source": {"kind": "rule", "id": rule}})).unwrap();
        let context = |tree: &Arc<Tree>, f: Finding, defs: Vec<sprawl_advisor::cleaners::CleanerDef>| SelectionContext { tree: Arc::downgrade(tree), findings: vec![f], privacy: vec![], definitions: defs };
        let mut c = Cleanup::default();
        c.bind(Some(context(&a, finding.clone(), definitions.clone())));
        c.ticked.insert(key(&finding.path));
        c.sync_confirmation_selection();
        c.own_ok = true;
        c.privacy_ok = true;
        assert!(!c.bind(Some(context(&a, finding.clone(), definitions.clone()))));
        assert!(c.own_ok && c.privacy_ok);
        assert!(c.bind(Some(context(&b, finding.clone(), definitions.clone()))));
        assert!(c.ticked.contains(&key(&finding.path)), "an identical action may stay selected after a fresh scan");
        assert!(!c.own_ok && !c.privacy_ok);
        let mut changed = definitions.clone();
        changed[0].text.push_str("\n# changed");
        assert!(c.bind(Some(context(&b, finding.clone(), changed))));
        assert!(c.ticked.is_empty(), "same rule ID cannot carry a tick to another definition");
        c.ticked.insert(key(&finding.path));
        let mut raised = finding.clone();
        raised.risk = Risk::Orange;
        assert!(review_changed(&finding, &raised), "live revalidation cannot silently raise the consent requirement");
        assert!(!review_changed(&finding, &finding));
        c.bind(Some(context(&b, raised, definitions)));
        assert!(c.ticked.is_empty(), "raised risk cannot inherit the old tick");
        c.own_ok = true;
        c.privacy_ok = true;
        c.privacy_ticked.insert("new privacy selection".into());
        assert!(c.sync_confirmation_selection());
        assert!(!c.own_ok && !c.privacy_ok);
    }

    /// Astra 1.0.3 review: the Advisor reading the tree again after a clean-up shows no report
    /// for a moment; the other ticks wait for the new one, and stay where it still has them.
    #[test]
    fn ticks_wait_through_a_refresh_with_no_report() {
        let a = Arc::new(Tree { volumes: Vec::new(), root_path: r"C:\Data".into(), nodes: vec![sprawl_core::Node::default()], errors: 0, method: sprawl_core::ScanMethod::Walk });
        let b = Arc::new((*a).clone());
        let item = |p: &str| -> Finding { serde_json::from_value(serde_json::json!({"path": p, "bytes": 10, "risk": "green", "remedy": "delete", "title": "x", "evidence": [], "confidence": 1.0, "source": {"kind": "model", "id": "m"}})).unwrap() };
        let (one, two) = (item(r"C:\Data\one"), item(r"C:\Data\two"));
        let ctx = |t: &Arc<Tree>, f: Vec<Finding>| SelectionContext { tree: Arc::downgrade(t), findings: f, privacy: vec![], definitions: vec![] };
        let mut c = Cleanup::default();
        c.bind(Some(ctx(&a, vec![one.clone(), two.clone()])));
        c.ticked.insert(key(&one.path));
        c.ticked.insert(key(&two.path));
        c.bind(None);
        assert_eq!(c.ticked.len(), 2, "no report: the ticks wait");
        // the refreshed report: `one` went (cleaned), `two` is as it was
        c.bind(Some(ctx(&b, vec![two.clone()])));
        assert_eq!(c.ticked.iter().collect::<Vec<_>>(), [&key(&two.path)]);
    }
}

#[cfg(test)]
mod age_tests {
    #[test]
    fn an_old_scan_says_how_old() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let after = |m: u64| t + std::time::Duration::from_secs(m * 60);
        assert_eq!(super::scan_age_words(t, after(59)), None);
        assert_eq!(super::scan_age_words(t, after(61)).as_deref(), Some("an hour"));
        assert_eq!(super::scan_age_words(t, after(185)).as_deref(), Some("3 hours"));
        assert_eq!(super::scan_age_words(t, after(3 * 1440)).as_deref(), Some("3 days"));
        // a clock set back: nothing to say
        assert_eq!(super::scan_age_words(after(10), t), None);
        // the user's own warning time
        assert_eq!(super::scan_age_words_after(t, after(20), 15).as_deref(), Some("20 minutes"));
        assert_eq!(super::scan_age_words_after(t, after(20), 30), None);
    }
}

#[cfg(test)]
mod machine_wide_tests {
    use super::*;

    fn dism(builtin: bool, id: &str) -> Finding {
        let mut f = Finding {
            path: r"C:\Windows\WinSxS".into(), owner: None, bytes: 5 << 30, risk: Risk::Green,
            remedy: Remedy::Command, command: None, title: "Windows component store".into(), evidence: vec![],
            confidence: 0.9, source: Source::Rule(id.into()), allow_protected: true,
            trusted: builtin, builtin, on_path: true, action: None, admin: false,
            carved: vec![], gain: None, forced_from: None,
        };
        f.set_action(Action::Tool { command: "DISM /Online /Cleanup-Image /StartComponentCleanup".into() }, true);
        f
    }

    /// DISM from the built-in pack can be ticked (it acts on Windows, not an account's
    /// folders); the same command from anything else is still another account's.
    #[test]
    fn windows_own_clean_ups_can_be_ticked() {
        assert_eq!(untickable(&dism(true, "win.component-store"), true), None);
        assert_eq!(untickable(&dism(false, "win.component-store"), true), Some(""), "not the built-in pack: no action at all");
        assert_eq!(untickable(&dism(true, "user.dism"), true), Some("Another account's"));
        // git's own clean-ups of a repository act on the folder named, too
        assert_eq!(untickable(&dism(true, sprawl_advisor::devgit::WORKTREE), true), None);
    }
}

#[cfg(test)]
mod compressed_tests {
    use sprawl_advisor::memory::Compressed;
    use sprawl_core::wof::Done;

    fn rec(saved: u64, partial: bool) -> Compressed {
        Compressed { path: r"C:\Games\Old".into(), alg: "xpress8k".into(), when: 1, saved, size: 1000, partial }
    }

    /// Run 1 compressed A and found B busy. The retry leaves A out (compressed already) and B
    /// doesn't get smaller, or is gone: nothing more, no errors, so the record is complete
    /// with run 1's saving.
    #[test]
    fn a_clean_retry_that_saves_nothing_completes_the_record() {
        let mut list = vec![rec(300, true)];
        super::note_compressed(&mut list, r"c:\games\old", "xpress8k", 2, 1000, 0, &Done::default(), true);
        assert_eq!(list, vec![Compressed { path: r"c:\games\old".into(), when: 2, ..rec(300, false) }]);
        // a retry with B still busy stays partial (nothing new to record)
        let mut list = vec![rec(300, true)];
        super::note_compressed(&mut list, r"C:\Games\Old", "xpress8k", 2, 1000, 0, &Done { in_use: 1, ..Done::default() }, true);
        assert_eq!(list, vec![rec(300, true)]);
        // nor does a run with problems complete it
        super::note_compressed(&mut list, r"C:\Games\Old", "xpress8k", 2, 1000, 0, &Done::default(), false);
        assert_eq!(list, vec![rec(300, true)]);
        // and a clean run that saved nothing records nothing for a folder never compressed
        let mut list = Vec::new();
        super::note_compressed(&mut list, r"C:\Games\Old", "xpress8k", 2, 1000, 0, &Done::default(), true);
        assert!(list.is_empty());
    }
}

#[cfg(test)]
mod second_try_tests {
    use super::*;

    #[test]
    fn a_second_click_never_deletes_these_whole() {
        let home = PathBuf::from(std::env::var("USERPROFILE").unwrap().to_lowercase());
        let win = PathBuf::from(std::env::var("SystemRoot").unwrap().to_lowercase());
        for p in [PathBuf::from(r"c:\pagefile.sys"), PathBuf::from(r"c:\games"), win.join(r"temp"), home.clone(), home.parent().unwrap().to_path_buf(), home.join(r"downloads"), home.join(r"appdata"), home.join(r"appdata\local")] {
            assert!(second_try_place_refused(&p), "{}", p.display());
        }
        for p in [home.join(r"downloads\old installers"), home.join(r"appdata\local\sandfall"), PathBuf::from(r"d:\games\old")] {
            assert!(!second_try_place_refused(&p), "{}", p.display());
        }
    }
}

#[cfg(test)]
mod review_tests {
    use super::{done_words, final_action, lead_line, Done, Fate};

    /// The banner after compressing: never "deleted", and what was left is said.
    #[test]
    fn a_compress_is_never_called_a_delete() {
        let d = Done { freed: 3 << 30, items: 1, compressed_items: 1, compressed_files: 900, in_use: 2, ..Done::default() };
        let w = done_words(&d);
        assert!(!w.contains("deleted for good") && w.contains("nothing was deleted") && w.contains("2 file(s)"), "{w}");
        let d = Done { already_compressed: 40, ..Done::default() };
        assert!(done_words(&d).contains("compressed already"));
        // a delete and a compress together: each said as it is
        let d = Done { items: 2, deleted_items: 1, compressed_items: 1, compressed_files: 5, ..Done::default() };
        let w = done_words(&d);
        assert!(w.starts_with("The caches were deleted for good.") && w.contains("Compressed in place"), "{w}");
        assert_eq!(done_words(&Done { items: 1, deleted_items: 1, ..Done::default() }), "They were deleted for good.");
    }

    #[test]
    fn the_button_names_what_it_does() {
        assert_eq!(final_action(&[Fate::Gone], 0), "Delete 1 for good");
        assert_eq!(final_action(&[Fate::Bin, Fate::Gone, Fate::Bin], 0), "Delete 1 for good, recycle 2");
        assert_eq!(final_action(&[Fate::Tool, Fate::Tool], 1), "Delete 1 privacy item without the Recycle Bin, run 2 clean-ups");
        assert_eq!(final_action(&[], 2), "Delete 2 privacy items without the Recycle Bin");
        assert_eq!(final_action(&[Fate::Gone, Fate::Bin], 1), "Delete 1 item and 1 privacy item without the Recycle Bin, recycle 1");
        assert_eq!(final_action(&[Fate::Link], 0), "Link 1");
        assert_eq!(final_action(&[], 0), "Clean up now");
        assert_eq!(final_action(&[Fate::Gone, Fate::Bin, Fate::Tool, Fate::Link], 0), "Delete 1 for good, clean up 3 more", "what can't be undone is always named");
        assert_eq!(final_action(&[Fate::Bin, Fate::Tool, Fate::Link], 2), "Delete 2 privacy items without the Recycle Bin, clean up 3 more", "privacy sets are named with what is deleted (Astra 1.0.2 S4 #2)");
        assert_eq!(final_action(&[Fate::Bin, Fate::Tool, Fate::Link, Fate::Compress], 0), "Clean up now", "too many parts for a button");
        // the first line agrees with the groups (Astra 1.0.2 S4 #3)
        assert!(!lead_line(&[Fate::Compress], 0).contains("Deleted for good"));
        assert!(!lead_line(&[Fate::Shrink], 0).contains("Deleted for good"));
        assert!(lead_line(&[Fate::Tool], 0).contains("usually for good"));
        // it quotes a group's name: escaped, it can sit in a XAML attribute (left raw, the quote
        // ended the attribute and the whole review failed to load, so Review did nothing)
        for o in [&[Fate::Gone, Fate::Tool][..], &[Fate::Gone, Fate::Bin], &[Fate::Bin, Fate::Tool]] {
            let l = lead_line(o, 1);
            assert!(l.contains('"'), "{o:?}: the case this guards");
            assert!(!crate::ui::esc(&l).contains('"'), "{o:?}");
        }
        assert!(lead_line(&[Fate::Bin, Fate::Bin], 0).starts_with("Sent to the Recycle Bin"), "the bin alone: plainly");
        // anything else beside the bin (a tool, compression...): the bin group is named, so its
        // sentence isn't read as the whole review's fate (round 3, S4 #3)
        for o in [&[Fate::Bin, Fate::Compress][..], &[Fate::Bin, Fate::Tool][..], &[Fate::Bin, Fate::Link][..], &[Fate::Bin, Fate::Shrink][..]] {
            let l = lead_line(o, 0);
            assert!(l.starts_with("What is under \"To the Recycle Bin\" goes there") && !l.contains("Sent to the Recycle Bin"), "{o:?}: {l}");
        }
        assert!(lead_line(&[], 1).starts_with("The privacy items are deleted without the Recycle Bin"));
        // mixed: only the counted groups skip the bin; compression deletes nothing (re-review S4 #3)
        for (o, p) in [(&[Fate::Gone, Fate::Bin, Fate::Compress][..], 0), (&[Fate::Bin, Fate::Link][..], 1)] {
            let l = lead_line(o, p);
            assert!(!l.contains("the rest") && l.contains("To the Recycle Bin"), "{l}");
        }
        assert!(lead_line(&[Fate::Gone, Fate::Compress], 0).starts_with("What is under \"Deleted for good\""));
        assert!(lead_line(&[Fate::Link], 0).starts_with("Nothing is deleted"));
        assert!(!lead_line(&[Fate::Link, Fate::Gone], 0).contains("lose nothing"));
        assert_eq!(lead_line(&[], 0), "");
        assert_eq!(final_action(&[Fate::Compress, Fate::Shrink], 0), "Compress 1, shrink 1");
    }
}
