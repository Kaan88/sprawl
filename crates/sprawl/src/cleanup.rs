//! Clean-up inside the Advisor: tick findings, review them, and Sprawl removes them itself,
//! showing each item's progress. The tree, the map and the findings update in place afterwards.

use std::collections::HashSet;
use std::path::PathBuf;
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
    /// Ticked findings, by lower-case path.
    pub ticked: HashSet<String>,
    /// The report generation the default ticks were set for.
    pub ticked_for: Option<u64>,
    /// Ticks carried over a restart as administrator, for the first report.
    pub pending: Option<HashSet<String>>,
    /// The review panel is open.
    pub review: bool,
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
}

pub(crate) struct Done {
    pub freed: u64,
    /// Items that freed something.
    pub items: usize,
    /// Sent to the Recycle Bin (freed when the bin is emptied), and the items that sent some.
    pub recycled: u64,
    pub recycled_items: usize,
    /// Privacy sets that went, and the browsers that held one back (open).
    pub privacy_items: usize,
    pub privacy_open: Vec<String>,
    /// Left because they were new since the scan, or kept by the user.
    pub kept_new: u64,
    pub kept_chosen: u64,
    pub kept_protected: u64,
    pub problems: Vec<String>,
    pub stopped: bool,
    pub output: Vec<String>,
}

enum Msg {
    Start(usize),
    Progress(usize, u64),
    /// (item, how it went, a scan of what is there now: None when it is gone, or not read)
    Finished(usize, Outcome, Option<Option<Tree>>),
    End,
}

/// One folder read again after it was cleaned. A plain walk: exact and quick for one folder,
/// where the volume's MFT (when elevated) would read the whole drive, maybe not yet seeing
/// what was just deleted.
pub(crate) fn rescan(p: &std::path::Path) -> std::io::Result<Tree> {
    sprawl_core::walk::walk(p, &sprawl_core::Progress::new())
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
    if files.is_empty() {
        return None;
    }
    match sprawl_advisor::inuse::in_use(&files) {
        Some(names) if names.is_empty() => None,
        Some(names) => Some(format!("{}: left alone, in use by {}", path.display(), names.join(", "))),
        None => Some(format!("{}: left alone, Windows couldn't say which programs use it", path.display())),
    }
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
    // a tool's own clean-up acts on the account running it: only the user's own folders
    if matches!(action, Action::Tool { .. }) {
        let mine = std::env::var_os("USERPROFILE").is_some_and(|home| PathBuf::from(key(&f.path)).starts_with(PathBuf::from(home.to_string_lossy().to_lowercase())));
        if !mine {
            return Some("Another account's");
        }
    }
    None
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
    on && f.risk >= Risk::Orange && !f.privacy() && !matches!(clean::action(f), Some((Action::Tool { .. }, _)))
}

/// Whether `f` may be in a sum of space, a tick Sprawl makes, the usual set or a suggestion:
/// never a privacy part. The report keeps those apart already; this holds every such place to it.
pub(crate) fn counts_as_space(f: &Finding) -> bool {
    !f.privacy()
}

/// What "Tick the safe ones" may tick.
pub(crate) fn safe_tick(f: &Finding, counted: bool) -> bool {
    counts_as_space(f) && untickable(f, counted).is_none() && held_in_app(f).is_none() && f.risk == Risk::Green && f.builtin && !f.heuristic() && !f.other_account() && !matches!(f.source, Source::Model(_)) && f.bytes >= 1 << 20
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
    match clean::action(f) {
        Some((Action::Tool { command }, _)) => clean::elevated_refusal(&command),
        _ => None,
    }
}

impl State {
    /// Ticks for a new report: the user's own ticks that still apply, else none. Sprawl never
    /// ticks a delete by itself; "Tick the safe ones" does it on request.
    pub(crate) fn cleanup_defaults(&mut self) {
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
        r.findings.iter().zip(&r.counted).filter(|(f, c)| safe_tick(f, **c)).map(|(f, _)| key(&f.path)).collect()
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
            .map(|(i, f)| (i, f.clone()))
            .collect()
    }

    /// The ticked privacy parts of the report on screen. Never for a one-row review.
    pub(crate) fn ticked_privacy(&self) -> Vec<Finding> {
        let Some(r) = self.advice() else { return Vec::new() };
        if self.advisor.cleanup.single.is_some() {
            return Vec::new();
        }
        r.privacy.iter().filter(|f| self.advisor.cleanup.privacy_ticked.contains(&key(&f.path))).cloned().collect()
    }

    /// Tick or untick privacy part `i` (the user's own tick: the only way one is ticked).
    pub(crate) fn privacy_toggle(&mut self, i: usize) {
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
        if !c.ticked.remove(&f) {
            c.ticked.insert(f);
        }
        // the ticks are what counts now, and a changed set is confirmed again
        c.single = None;
        c.own_ok = false;
        c.auto = false;
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
        let plan = sprawl_advisor::plan::plan(&r.findings, &r.counted, need);
        let ticked: HashSet<String> = plan.picked.iter().filter(|&&i| counts_as_space(&r.findings[i]) && untickable(&r.findings[i], r.counted[i]).is_none() && held_in_app(&r.findings[i]).is_none()).map(|&i| key(&r.findings[i].path)).collect();
        let bytes = r.findings.iter().filter(|f| ticked.contains(&key(&f.path))).map(|f| f.bytes).sum();
        self.advisor.cleanup.ticked = ticked;
        self.advisor.cleanup.auto = false;
        Some(bytes)
    }

    /// Start cleaning what is ticked: each item is checked again first.
    pub(crate) fn cleanup_start(&mut self) {
        if self.advisor.cleanup.running.is_some() {
            return;
        }
        // checked against the whole scan, not a filtered view of it
        let Some(t) = self.active_tree() else { return };
        let picked = self.ticked_findings();
        let privacy = self.ticked_privacy();
        if picked.is_empty() && privacy.is_empty() {
            return;
        }
        if !privacy.is_empty() && !self.advisor.cleanup.privacy_ok {
            self.adv_say("Tick \"I understand what each of these removes\" first, or untick the privacy items.");
            return;
        }
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
        let spare = clean::Spare { since: Some(self.active_began()), keep: self.advisor.memory.kept_paths() };
        self.cleaner_procs(true);
        self.remember_usual(&picked.iter().map(|(_, f)| f.clone()).collect::<Vec<_>>());
        let bin_on = self.recycle_on();
        let mut work: Vec<(PathBuf, Action, u64, Option<(String, Vec<String>)>, bool)> = Vec::new();
        let mut items = Vec::new();
        let mut held = Vec::new();
        // the privacy sets after the rest, checked the same way (their own set checks inside)
        for f in picked.into_iter().map(|(_, f)| f).chain(privacy) {
            if let Some(why) = held_in_app(&f) {
                held.push(format!("{}: not cleaned, {why}", f.title));
                continue;
            }
            if spare.keeps(&f.path) {
                held.push(format!("{}: not cleaned, you chose to keep it", f.title));
                continue;
            }
            // the same checks as when it was found, against this scan
            let f = match sprawl_advisor::safety::check(&f, &t, &never) {
                Ok(f) => f,
                Err(why) => {
                    held.push(format!("{}: not cleaned, {why}", f.title));
                    continue;
                }
            };
            let Some((action, _)) = clean::action(&f) else {
                held.push(format!("{}: not cleaned, its command is not one Sprawl can run", f.title));
                continue;
            };
            // its application must be closed: checked now, and again just before it runs
            let gate = self.part_gate(&f);
            if let Some((app, Closed::Must)) = self.app_open(&f) {
                held.push(format!("{}: not cleaned, {app} is open. Close it, then clean up again.", f.title));
                continue;
            }
            let recycle = goes_to_bin(&f, bin_on);
            work.push((f.path.clone(), action, f.bytes, gate, recycle));
            items.push(Item {
                path: f.path.clone(),
                title: f.title.clone(),
                bytes: f.bytes,
                state: ItemState::Waiting,
                rule: match &f.source {
                    sprawl_advisor::Source::Rule(r) => Some(r.clone()),
                    sprawl_advisor::Source::Model(_) => None,
                },
                recycle,
                privacy: f.privacy(),
            });
        }
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
            for (i, (path, action, before, gate, recycle)) in work.into_iter().enumerate() {
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                let _ = tx.send(Msg::Start(i));
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
                let mut out = clean::run_with(&path, &action, &spare, bin, &stop2, &mut |freed| {
                    if last.elapsed().as_millis() >= 200 {
                        last = Instant::now();
                        let _ = tx.send(Msg::Progress(i, freed));
                    }
                });
                // what is there now, for the tree
                let now = match std::fs::symlink_metadata(&path) {
                    Ok(_) => rescan(&path).ok().map(Some),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(None),
                    Err(_) => None,
                };
                // a tool removes things itself: what it freed is what the folder lost
                if matches!(action, Action::Tool { .. }) {
                    if let Some(now) = &now {
                        out.freed = before.saturating_sub(now.as_ref().map_or(0, |t| t.nodes[sprawl_core::ROOT].size));
                    }
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
        c.running = Some(Running { items, rx, stop, rescans: Vec::new(), held });
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
        let spare = clean::Spare { since: Some(self.active_began()), keep: self.advisor.memory.kept_paths() };
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
            None => (f.bytes, f.estimate()),
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
                        if let Some(p) = &self.advisor.foot {
                            crate::state::text(p, &format!("AdvCS{i}"), &format!("Removing... {}", sprawl_core::format_size(freed)));
                        }
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
        if ended {
            self.cleanup_finish();
        } else if redraw {
            self.advisor_redraw();
        }
    }

    fn cleanup_finish(&mut self) {
        let Some(run) = self.advisor.cleanup.running.take() else { return };
        log_run(&run.items, &run.held, run.stop.load(Ordering::Relaxed));
        let mut done = Done { freed: 0, items: 0, recycled: 0, recycled_items: 0, privacy_items: 0, privacy_open: Vec::new(), kept_new: 0, kept_chosen: 0, kept_protected: 0, problems: run.held.clone(), stopped: run.stop.load(Ordering::Relaxed), output: Vec::new() };
        for it in &run.items {
            if let ItemState::Done(o) = &it.state {
                // a privacy set: counted on its own, never in what was freed; one that went is
                // unticked, one held back stays ticked for "Check again"
                if it.privacy {
                    let browser = it.rule.as_deref().and_then(sprawl_advisor::privacy::part).map(|p| p.browser().short);
                    if o.problems.is_empty() && o.files > 0 {
                        done.privacy_items += 1;
                        self.advisor.cleanup.privacy_ticked.remove(&key(&it.path));
                    } else {
                        for who in o.problems.iter().filter_map(|p| holder(p, browser)) {
                            if !done.privacy_open.contains(&who) {
                                done.privacy_open.push(who);
                            }
                        }
                    }
                    done.problems.extend(o.problems.iter().map(|p| format!("{}: {p}", it.title)));
                    continue;
                }
                if o.freed > 0 {
                    done.items += 1;
                }
                if o.recycled > 0 {
                    done.recycled_items += 1;
                }
                done.freed += o.freed;
                done.recycled += o.recycled;
                done.kept_new += o.kept_new;
                done.kept_chosen += o.kept_chosen;
                done.kept_protected += o.kept_protected;
                done.problems.extend(o.problems.iter().take(3).cloned());
                if o.more_problems > 0 || o.problems.len() > 3 {
                    done.problems.push(format!("{}: and {} more", it.title, o.more_problems + o.problems.len().saturating_sub(3) as u64));
                }
                done.output.extend(o.output.iter().cloned());
                // remembered as done when most of it went, never over the user's own Keep
                let kept = self.advisor.memory.hides(&it.path).is_some_and(|d| d.verdict == sprawl_advisor::memory::Verdict::Keep);
                if o.freed + o.recycled >= it.bytes / 2 && !kept {
                    self.advisor.memory.decide(&it.path, &it.title, sprawl_advisor::memory::Verdict::Done);
                }
            }
        }
        // what each cleaner part really freed this time, for the AI's numbers next time
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut freed: std::collections::BTreeMap<String, u64> = Default::default();
        for it in &run.items {
            if let (Some(rule), ItemState::Done(o), false) = (&it.rule, &it.state, it.privacy) {
                *freed.entry(rule.clone()).or_default() += o.freed;
            }
        }
        for (rule, b) in freed {
            self.advisor.memory.freed.insert(rule, (b, now));
        }
        self.save_memory();
        self.advisor.cleanup.done = Some(done);
        // what went no longer counts as ticked, nor is put back by the Cleaners tab on this report
        self.advisor.cl.preview = None;
        self.advisor.cleanup.ticked.clear();
        self.advisor.cleanup.ticked_for = None;
        // the tree as it is now: each item's folder scanned again, or taken out when gone
        self.apply_rescans(run.rescans);
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
            let freed: u64 = run.items.iter().filter(|it| !it.recycle && !it.privacy).map(|it| match &it.state {
                ItemState::Running(f) => *f,
                ItemState::Done(o) => o.freed,
                ItemState::Waiting => 0,
            }).sum();
            x += &format!(
                r##"<Border Padding="14,12" CornerRadius="8" Background="#142F7BF6" BorderBrush="#552F7BF6" BorderThickness="1"><StackPanel Spacing="8"><StackPanel Orientation="Horizontal" Spacing="10"><ProgressRing IsActive="True" Width="18" Height="18"/><TextBlock Text="Cleaning up... {} freed so far" {num} FontSize="15" FontWeight="SemiBold" VerticalAlignment="Center"/><Button x:Name="AdvCleanStop" Content="Stop" Padding="10,2" FontSize="12" CornerRadius="5"/></StackPanel>"##,
                self.size(freed),
                num = crate::look::NUM
            );
            for (i, it) in run.items.iter().enumerate() {
                let (glyph, colour, state) = match &it.state {
                    ItemState::Waiting => ("&#xE823;", "#FF8E97AA", "Waiting".to_string()),
                    ItemState::Running(_) if it.privacy => ("&#xE895;", "#FF8FB4FF", "Removing...".to_string()),
                    ItemState::Done(o) if it.privacy && o.problems.is_empty() => ("&#xE73E;", "#FF4CC38A", "Removed".to_string()),
                    ItemState::Done(_) if it.privacy => ("&#xE7BA;", "#FFF5B342", "Held back: nothing in this set went".to_string()),
                    ItemState::Running(f) if it.recycle => ("&#xE895;", "#FF8FB4FF", format!("To the Recycle Bin... {}", self.size(*f))),
                    ItemState::Running(f) => ("&#xE895;", "#FF8FB4FF", format!("Removing... {}", self.size(*f))),
                    ItemState::Done(o) if it.recycle && o.problems.is_empty() => ("&#xE73E;", "#FF4CC38A", format!("{} to the Recycle Bin", self.size(o.recycled))),
                    ItemState::Done(o) if it.recycle => ("&#xE7BA;", "#FFF5B342", format!("{} to the Recycle Bin; some couldn't go", self.size(o.recycled))),
                    ItemState::Done(o) if o.problems.is_empty() => ("&#xE73E;", "#FF4CC38A", format!("Freed {}", self.size(o.freed))),
                    ItemState::Done(o) => ("&#xE7BA;", "#FFF5B342", format!("Freed {}; some couldn't go", self.size(o.freed))),
                };
                x += &format!(
                    r##"<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="18"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><FontIcon Glyph="{glyph}" FontSize="13" Foreground="{colour}"/><TextBlock Grid.Column="1" Text="{}" FontSize="12" TextTrimming="CharacterEllipsis"/><TextBlock x:Name="AdvCS{i}" Grid.Column="2" Text="{}" {num} MinWidth="160" TextAlignment="Right" FontSize="12" Foreground="#FFB9C2D6"/></Grid>"##,
                    ui::esc(&it.title),
                    ui::esc(&state),
                    num = crate::look::NUM
                );
            }
            x += "</StackPanel></Border>";
        } else if let Some(d) = &c.done {
            let fine = d.problems.is_empty() && !d.stopped && d.freed + d.recycled + d.privacy_items as u64 > 0;
            let (bg, border, glyph, colour) = if fine { ("#144CC38A", "#554CC38A", "&#xE73E;", "#FF4CC38A") } else { ("#14F5B342", "#55F5B342", "&#xE7BA;", "#FFF5B342") };
            let items = if d.items == 1 { "1 item".to_string() } else { format!("{} items", d.items) };
            let head = match (d.stopped, d.items) {
                (true, 0) if d.recycled == 0 && d.privacy_items == 0 => "Stopped. Nothing was freed.".to_string(),
                (false, 0) if d.recycled == 0 && d.privacy_items == 0 => "Nothing was freed.".to_string(),
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
            let head = match d.privacy_items {
                0 => head,
                n => format!("{head} {} removed.", privacy_count(n)).trim().to_string(),
            };
            // what happened to them, in plain words
            let gone = match (d.items > 0, d.recycled > 0) {
                (true, true) => "Caches were deleted for good; your own files are in the Recycle Bin, and their space is freed when the bin is emptied.",
                (false, true) => "They are in the Recycle Bin: Explorer can put them back, and their space is freed when the bin is emptied.",
                (false, false) if d.privacy_items > 0 => PV_DONE,
                (false, false) => "Nothing was removed.",
                _ => "They were deleted for good.",
            };
            x += &format!(
                r##"<Border Padding="14,12" CornerRadius="8" Background="{bg}" BorderBrush="{border}" BorderThickness="1"><StackPanel Spacing="6"><Grid><StackPanel Orientation="Horizontal" Spacing="10"><FontIcon Glyph="{glyph}" FontSize="16" Foreground="{colour}"/><TextBlock Text="{}" {num} FontSize="15" FontWeight="SemiBold"/></StackPanel><Button x:Name="AdvCleanedOk" HorizontalAlignment="Right" Content="OK" Padding="12,2" FontSize="12" CornerRadius="5"/></Grid><TextBlock Text="{gone} The tree, the map and the findings are updated.{}" FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap"/><StackPanel Orientation="Horizontal" Spacing="8"><Button x:Name="AdvCleanMore" Content="Find more with AI" Padding="10,3" FontSize="12" CornerRadius="5"/><Button x:Name="AdvCleanLog" Content="See what was removed" Padding="10,3" FontSize="12" CornerRadius="5"/></StackPanel>"##,
                ui::esc(&head),
                self.tree.as_ref().and_then(|t| crate::win32::disk_space(&t.path_of(sprawl_core::ROOT))).map_or(String::new(), |(_, free)| ui::esc(&format!(" The disk now has {} free.", self.size(free)))),
                num = crate::look::NUM
            );
            // a privacy set held because its browser was open leads somewhere
            if !d.privacy_open.is_empty() {
                x += &format!(
                    r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" FontSize="12" Foreground="#FFF5B342" TextWrapping="Wrap" VerticalAlignment="Center"/><Button x:Name="AdvPvAgain" Content="Check again" Padding="10,2" FontSize="12" CornerRadius="5"/></StackPanel>"##,
                    ui::esc(&close_then(&d.privacy_open))
                );
            }
            if !d.problems.is_empty() {
                x += r##"<TextBlock Text="Some things couldn't go (in use, no access, or no room in the Recycle Bin): they stay as they were." FontSize="12" Foreground="#FFF5B342" TextWrapping="Wrap"/>"##;
                for p in d.problems.iter().take(8) {
                    x += &format!(r##"<TextBlock Text="{}" FontSize="12" Foreground="#FF8E97AA" TextWrapping="Wrap" Margin="10,0,0,0"/>"##, ui::esc(p));
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
                x += &format!(r##"<TextBlock Text="Left alone: {}." FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap"/>"##, ui::esc(&parts.join(", ")));
            }
            if !d.output.is_empty() {
                x += &format!(
                    r##"<Border Padding="10,8" CornerRadius="5" Background="#33000000"><TextBlock FontFamily="Consolas" FontSize="12" TextWrapping="Wrap" IsTextSelectionEnabled="True" Text="{}"/></Border>"##,
                    d.output.iter().map(|l| ui::esc(l)).collect::<Vec<_>>().join("&#10;")
                );
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
        let mut x = format!(
            r##"<Border Padding="12,10" CornerRadius="8" Background="#0FFFFFFF" BorderBrush="#22FFFFFF" BorderThickness="1"><StackPanel Spacing="8">
<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
<StackPanel VerticalAlignment="Center"><TextBlock Text="{}" {num} FontSize="15" FontWeight="SemiBold"/><TextBlock Text="{}" FontSize="12" Foreground="#FFA0A8B8" TextWrapping="Wrap"/></StackPanel>
<StackPanel Grid.Column="1" Orientation="Horizontal" Spacing="8">{prev}<Button x:Name="AdvClean" Content="Clean up..." Padding="16,6" FontSize="13" CornerRadius="6" Background="#FF2F7BF6" Foreground="White" IsEnabled="{}"/></StackPanel></Grid>
<StackPanel Orientation="Horizontal" Spacing="6"><Button x:Name="AdvSelSafe" Content="Tick the safe ones" Padding="10,3" FontSize="12" CornerRadius="5"/><Button x:Name="AdvSelNone" Content="Untick all" Padding="10,3" FontSize="12" CornerRadius="5"/><TextBlock Text="or tick enough for" FontSize="12" VerticalAlignment="Center" Margin="8,0,0,0" Foreground="#FFB9C2D6"/><TextBox x:Name="AdvGoal" Width="64" PlaceholderText="{goal}"/><TextBlock Text="GB" FontSize="12" VerticalAlignment="Center" Foreground="#FFB9C2D6"/><Button x:Name="AdvSelGoal" Content="Tick" Padding="10,3" FontSize="12" CornerRadius="5"/></StackPanel>"##,
            match (picked.is_empty(), privacy) {
                (true, 0) => "Nothing ticked".to_string(),
                (true, n) => format!("{} ticked", privacy_count(n)),
                (false, 0) => format!("{} ticked · {}{}", picked.len(), if up_to { "up to " } else { "" }, self.size(bytes)),
                (false, n) => format!("{} ticked · {}{}, and {}", picked.len(), if up_to { "up to " } else { "" }, self.size(bytes), privacy_count(n)),
            },
            if self.advisor.cleanup.auto && !picked.is_empty() { "The safe ones are ticked. Change the ticks, then Clean up." } else { "Tick what should go, then Clean up. Sprawl removes it itself and shows what was freed." },
            if picked.is_empty() && privacy == 0 { "False" } else { "True" },
            prev = if self.advisor.cleaners_tab_on() {
                format!(
                    r##"<Button x:Name="AdvClPrev" Content="Preview" Padding="14,6" FontSize="13" CornerRadius="6" IsEnabled="{}" ToolTipService.ToolTip="The exact files that would go, counted now. Nothing is deleted."/>"##,
                    if picked.is_empty() || matches!(self.advisor.cl.preview, Some(crate::cleaners::PreviewRun::Counting(_))) { "False" } else { "True" }
                )
            } else {
                String::new()
            },
        );
        if admin_waiting {
            x += r##"<StackPanel Orientation="Horizontal" Spacing="8"><FontIcon Glyph="&#xE7EF;" FontSize="13" Foreground="#FFF5B342"/><TextBlock Text="Some items need administrator rights." FontSize="12" Foreground="#FFB9C2D6" VerticalAlignment="Center"/><Button x:Name="AdvAdmin" Content="Restart as administrator" Padding="10,2" FontSize="12" CornerRadius="5"/></StackPanel>"##;
        }
        x += r##"<Button x:Name="AdvExport" Padding="0" FontSize="12" BorderThickness="0" Background="Transparent" Foreground="#FF8E97AA" Content="Save as a script instead..."/>"##;
        x += "</StackPanel></Border>";
        x
    }

    /// The review before anything goes.
    pub(crate) fn cleanup_review_xaml(&self) -> String {
        let all = self.ticked_findings();
        // what the run will hold back is shown with its reason, and not counted
        let held: Vec<(Finding, String)> = all.iter().filter_map(|(_, f)| held_in_app(f).map(|why| (f.clone(), why))).collect();
        let picked: Vec<(usize, Finding)> = all.into_iter().filter(|(_, f)| held_in_app(f).is_none()).collect();
        let privacy = self.ticked_privacy();
        let bytes: u64 = picked.iter().map(|(_, f)| self.review_size(f).0).sum();
        let up_to = picked.iter().any(|(_, f)| self.review_size(f).1);
        let own = picked.iter().any(|(_, f)| f.risk >= Risk::Orange);
        let bin_on = self.recycle_on();
        let mut x = format!(
            r##"<Border Padding="14,12" CornerRadius="8" Background="#0FFFFFFF" BorderBrush="#552F7BF6" BorderThickness="1"><StackPanel Spacing="8"><TextBlock Text="{}" FontSize="16" FontWeight="SemiBold"/><TextBlock Text="{}" FontSize="13" FontWeight="SemiBold" Foreground="#FFF5B342" TextWrapping="Wrap"/><TextBlock Text="{}" FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap"/>"##,
            {
                // the privacy items: by number, never in the space
                let space = match picked.len() {
                    0 => String::new(),
                    n => format!("{}, {}", if n == 1 { "1 item".to_string() } else { format!("{n} items") }, if up_to { format!("up to {}", self.size(bytes)) } else { format!("about {}", self.size(bytes)) }),
                };
                match (space.is_empty(), privacy.len()) {
                    (_, 0) => format!("Clean up {space}?"),
                    (true, n) => format!("Clean up {}?", privacy_count(n)),
                    (false, n) => format!("Clean up {space}, and {}?", privacy_count(n)),
                }
            },
            {
                let binned = picked.iter().filter(|(_, f)| goes_to_bin(f, bin_on)).count();
                if picked.is_empty() {
                    String::new()
                } else if binned == 0 {
                    "Deleted for good, not sent to the Recycle Bin.".to_string()
                } else if binned == picked.len() {
                    "Sent to the Recycle Bin: Explorer can put them back. Their space is freed when the bin is emptied. What can't go to the bin (too big for it, or a drive without one) is left as it is.".to_string()
                } else {
                    "Your own files (marked below) go to the Recycle Bin, and their space is freed when the bin is emptied; the rest is deleted for good.".to_string()
                }
            },
            {
                let tools = picked.iter().filter(|(_, f)| matches!(clean::action(f), Some((Action::Tool { .. }, _)))).count();
                let own = format!(
                    "Sprawl removes them itself. Links inside are removed as links (what they point to stays), files a program has locked are skipped, and anything created or changed since the scan began ({}) is left alone, as is anything you chose to keep.",
                    sprawl_core::local_time(self.active_began())
                );
                let tool = "Each item with a program of its own runs that program's clean-up command, shown below, without a window. The program decides what goes: files new since the scan, or ones you chose to keep, may go too.";
                if picked.is_empty() {
                    String::new()
                } else if tools == picked.len() {
                    tool.to_string()
                } else if tools > 0 {
                    format!("{own} {tool}")
                } else {
                    own
                }
            }
        );
        // only privacy items: no space lines, no space list
        if picked.is_empty() {
            x = x.replace(r##"<TextBlock Text="" FontSize="13" FontWeight="SemiBold" Foreground="#FFF5B342" TextWrapping="Wrap"/><TextBlock Text="" FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap"/>"##, "");
        }
        // the rows scroll; the boxes and buttons under them stay in view
        x += r##"<ScrollViewer MaxHeight="190" VerticalScrollBarVisibility="Auto" HorizontalScrollBarVisibility="Disabled" Padding="0,0,12,0"><StackPanel Spacing="8">"##;
        for (_, f) in &picked {
            let what = crate::advisor::risk_words_for(f);
            let ai = if matches!(f.source, Source::Model(_)) { " · suggested by the AI" } else { "" };
            x += &format!(
                r##"<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="10"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><Ellipse Width="8" Height="8" Fill="{}"/><StackPanel Grid.Column="1"><TextBlock Text="{}" FontSize="13" TextTrimming="CharacterEllipsis"/><TextBlock Text="{}{ai}" FontSize="12" Foreground="#FF8E97AA" TextTrimming="CharacterEllipsis"/></StackPanel><TextBlock Grid.Column="2" Text="{}" FontSize="13" VerticalAlignment="Center"/></Grid>"##,
                crate::advisor::risk_hex(f.risk),
                ui::esc(&f.title),
                ui::esc(&match clean::action(f) {
                    Some((Action::Tool { command }, _)) => format!("{what} · runs: {command}"),
                    _ if goes_to_bin(f, bin_on) => format!("{what} · to the Recycle Bin · {}", f.path.display()),
                    _ => format!("{what} · {}", f.path.display()),
                }),
                match self.review_size(f) {
                    (b, true) => format!("up to {}", self.size(b)),
                    (b, false) if f.estimate() => format!("{} now", self.size(b)),
                    (b, false) => self.size(b),
                }
            );
        }
        for (f, why) in &held {
            x += &format!(
                r##"<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="10"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><FontIcon Glyph="&#xE7BA;" FontSize="10" Foreground="#FFF5B342"/><StackPanel Grid.Column="1"><TextBlock Text="{}" FontSize="13" TextTrimming="CharacterEllipsis" Foreground="#FF8E97AA"/><TextBlock Text="Held back: {}" FontSize="12" Foreground="#FFF5B342" TextWrapping="Wrap"/></StackPanel><TextBlock Grid.Column="2" Text="not run" FontSize="13" VerticalAlignment="Center" Foreground="#FF8E97AA"/></Grid>"##,
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
                r##"<StackPanel Orientation="Horizontal" Spacing="8"><FontIcon Glyph="&#xE823;" FontSize="13" Foreground="#FFF5B342"/><TextBlock Text="The scan is {age} old. What changed since it began is left alone, so this may free less than shown. Scan again first for a full clean-up." FontSize="12" Foreground="#FFF5B342" TextWrapping="Wrap" MaxWidth="760" VerticalAlignment="Center"/><Button x:Name="AdvRescanOld" Content="Scan again" Padding="10,2" FontSize="12" CornerRadius="5" VerticalAlignment="Center"/></StackPanel>"##
            );
        }
        for (_, f) in picked.iter().filter(|(_, f)| crate::advisor::gone_for_good(f) && !goes_to_bin(f, bin_on)) {
            x += &format!(r##"<TextBlock Text="{}: what is in it now cannot be brought back afterwards." FontSize="12" Foreground="#FFF5B342" TextWrapping="Wrap"/>"##, ui::esc(&f.title));
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
            x += r##"<StackPanel Spacing="2"><TextBlock Text="Kept, because you chose to keep them:" FontSize="12" Foreground="#FFB9C2D6"/>"##;
            for k in kept.iter().take(6) {
                x += &format!(r##"<TextBlock Text="{}" FontSize="12" Foreground="#FF8E97AA" TextTrimming="CharacterEllipsis" Margin="10,0,0,0"/>"##, ui::esc(&k.display().to_string()));
            }
            if kept.len() > 6 {
                x += &format!(r##"<TextBlock Text="and {} more" FontSize="12" Foreground="#FF8E97AA" Margin="10,0,0,0"/>"##, kept.len() - 6);
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
                    r##"<TextBlock Text="{} {} open: close {} first, or its parts stay. Sprawl never closes a program." FontSize="12" Foreground="#FFF5B342" TextWrapping="Wrap"/>"##,
                    ui::esc(&must.join(", ")),
                    if must.len() == 1 { "is" } else { "are" },
                    if must.len() == 1 { "it" } else { "them" }
                );
            }
            if !warn.is_empty() {
                x += &format!(r##"<TextBlock Text="{} {} open: files in use are skipped." FontSize="12" Foreground="#FFB9A36A" TextWrapping="Wrap"/>"##, ui::esc(&warn.join(", ")), if warn.len() == 1 { "is" } else { "are" });
            }
            x += r##"<Button x:Name="AdvClCheck" Content="Check again" Padding="10,2" FontSize="12" CornerRadius="5"/></StackPanel>"##;
        }
        if own {
            // all of the user's own go to the bin: they can be brought back from there
            let all_binned = picked.iter().filter(|(_, f)| f.risk >= Risk::Orange).all(|(_, f)| goes_to_bin(f, bin_on));
            x += &format!(
                r##"<CheckBox x:Name="AdvOwnOk" IsChecked="{}" Content="{}" FontSize="13"/>"##,
                if self.advisor.cleanup.own_ok { "True" } else { "False" },
                if all_binned { "These are my own files and I want them gone (to the Recycle Bin)" } else { "These are my own files, I want them gone, and I know they can't be brought back" }
            );
        }
        x += &format!(r##"<StackPanel Orientation="Horizontal" Spacing="8"><Button x:Name="AdvCleanNow" Content="Clean up now" Padding="16,6" FontSize="13" CornerRadius="6" Background="#FF2F7BF6" Foreground="White" IsEnabled="{}"/>"##, if (picked.is_empty() && privacy.is_empty()) || (own && !self.advisor.cleanup.own_ok) || (!privacy.is_empty() && !self.advisor.cleanup.privacy_ok) { "False" } else { "True" });
        x += r##"<Button x:Name="AdvReviewBack" Content="Back" Padding="14,6" FontSize="13" CornerRadius="6"/></StackPanel>"##;
        x += "</StackPanel></Border>";
        x
    }
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
            && self.advisor.foot.as_ref().and_then(|f| crate::find::<crate::bindings::Microsoft::UI::Xaml::Controls::TextBlock>(f, "AdvMsg").ok()).is_some_and(|t| {
                let _ = t.SetText(&msg.into());
                let _ = t.SetVisibility(crate::bindings::Microsoft::UI::Xaml::Visibility::Visible);
                true
            });
        if !shown {
            crate::state::text(&self.root, "StatusText", msg);
        }
    }

    /// The Advisor's foot, pinned under the list: the message, then the clean-up's status, review
    /// or bar, whatever section is open.
    pub(crate) fn advisor_foot_xaml(&self) -> String {
        let mut x = format!(r##"<StackPanel {} Spacing="8" Padding="18,10,18,12" BorderBrush="#22FFFFFF" BorderThickness="0,1,0,0" Background="#FF1A1D27">"##, crate::ui::NS);
        let msg = self.advisor.msg.borrow().clone();
        x += &format!(
            r##"<TextBlock x:Name="AdvMsg" Text="{}" FontSize="12" Foreground="#FF8FB4FF" TextWrapping="Wrap" Visibility="{}"/>"##,
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
            } else if self.advice().is_some_and(|r| !r.privacy.is_empty() || r.findings.iter().zip(&r.counted).any(|(f, k)| !matches!(untickable(f, *k), Some("")))) {
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
pub(crate) const PV_REVIEW_HEAD: &str = "Privacy items, not counted in the space above. Each set goes whole, or not at all, and only with its browser closed:";
pub(crate) const PV_OK: &str = "I understand what each of these removes";
pub(crate) const PV_DONE: &str = "The privacy items' files were deleted; the browser starts new, empty ones the next time it opens.";

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
        let spare = clean::Spare { since: Some(self.active_began()), keep: self.advisor.memory.kept_paths() };
        let mut x = format!(r##"<Border Padding="10,8" CornerRadius="6" Background="#0AFFFFFF" BorderBrush="#33F5B342" BorderThickness="1"><StackPanel Spacing="6"><TextBlock Text="{}" FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap"/>"##, ui::esc(PV_REVIEW_HEAD));
        let mut open: Vec<String> = Vec::new();
        for f in privacy {
            let part = match &f.source {
                Source::Rule(id) => sprawl_advisor::privacy::part(id),
                _ => None,
            };
            let files = clean::action(f).and_then(|(a, _)| clean::preview(&f.path, &a, &spare, 0)).map(|p| p.biggest.iter().map(|(p, b)| format!("{} ({})", p.display(), self.size(*b))).collect::<Vec<_>>()).unwrap_or_default();
            let held = self.app_open(f).map(|(app, _)| app);
            if let Some(app) = &held {
                let short = part.map_or(app.as_str(), |p| p.browser().short).to_string();
                if !open.contains(&short) {
                    open.push(short);
                }
            }
            x += &format!(
                r##"<StackPanel Spacing="1"><StackPanel Orientation="Horizontal" Spacing="8"><Ellipse Width="8" Height="8" Fill="{}" VerticalAlignment="Center"/><TextBlock Text="{}" FontSize="13"/></StackPanel><TextBlock Text="{}" FontSize="12" Foreground="#FFF5B342" TextWrapping="Wrap" Margin="16,0,0,0"/>"##,
                crate::advisor::risk_hex(f.risk),
                ui::esc(&f.title),
                ui::esc(part.map_or("", |p| p.cost))
            );
            if files.is_empty() {
                x += r##"<TextBlock Text="Its files couldn't be listed now: it will be held back." FontSize="12" Foreground="#FF8E97AA" Margin="16,0,0,0"/>"##;
            }
            for line in &files {
                x += &format!(r##"<TextBlock Text="{}" {} FontSize="12" Foreground="#FF8E97AA" TextTrimming="CharacterEllipsis" Margin="16,0,0,0" ToolTipService.ToolTip="{0}"/>"##, ui::esc(line), crate::look::NUM);
            }
            x += "</StackPanel>";
        }
        x += &format!(r##"<TextBlock Text="{}" FontSize="12" Foreground="#FF8E97AA" TextWrapping="Wrap"/>"##, ui::esc(sprawl_advisor::privacy::DELETED_NOTE));
        if !open.is_empty() {
            x += &format!(
                r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" FontSize="12" Foreground="#FFF5B342" TextWrapping="Wrap" VerticalAlignment="Center"/><Button x:Name="AdvClCheck" Content="Check again" Padding="10,2" FontSize="12" CornerRadius="5"/></StackPanel>"##,
                ui::esc(&close_then(&open))
            );
        }
        x += &format!(r##"<CheckBox x:Name="AdvPrivOk" IsChecked="{}" Content="{}" FontSize="13"/></StackPanel></Border>"##, if self.advisor.cleanup.privacy_ok { "True" } else { "False" }, PV_OK);
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
                s += &format!("{}\tfreed {} bytes, {} files\r\n", it.path.display(), o.freed, o.files);
                if o.recycled > 0 {
                    s += &format!("\t  sent to the Recycle Bin: {} bytes, {} files\r\n", o.recycled, o.recycled_files);
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
            PV_OK.into(),
            PV_DONE.into(),
            privacy_count(1),
            privacy_count(3),
            close_then(&["Chrome".into(), "Edge".into()]),
            pref.label.into(),
            pref.help.into(),
            "Removing...".into(),
            "Removed".into(),
            "Held back: nothing in this set went".into(),
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
        };
        assert!(untickable(&f, true).is_some());
        f.remedy = Remedy::Delete;
        f.set_action(Action::Remove, false);
        assert_eq!(untickable(&f, true), None);
        assert!(f.risk >= Risk::Orange, "the own-files box is needed");
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
