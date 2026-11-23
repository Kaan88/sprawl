//! The Advisor's Cleaners section: the applications on this PC and their parts, BleachBit-like.
//! A part's matches are findings of the report, so a part's tick is the same tick as its rows in
//! Findings, and cleaning goes through the same review and executor. Nothing here deletes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sprawl_advisor::cleaners::{CleanerDef, Class, Closed, Presence, GROUPS, SHAPE};
use sprawl_advisor::{Finding, Risk, Source};

use crate::advisor::Report;
use sprawl_advisor::config::{AiConfig, Provider};
use crate::cleanup::{key, untickable};
use crate::state::State;
use crate::ui;

/// Bulk choices must use the actual finding's raised risk and current gates, not catalogue risk.
fn auto_finding(f: &Finding, counted: bool, quiet: bool) -> bool {
    matches!(f.risk, Risk::Green | Risk::Amber) && !f.other_account() && !quiet && !f.privacy()
        && crate::cleanup::counts_as_space(f) && untickable(f, counted).is_none()
        && crate::cleanup::held_in_app(f).is_none()
}

fn auto_takes(r: &Report, i: usize) -> bool {
    auto_finding(&r.findings[i], r.counted[i], r.quiet.contains(&key(&r.findings[i].path)))
}

/// What an application's box takes, both as the box is drawn and as a press on it ticks:
/// the user's press, so the user's own files too (Orange: the review asks "my own files"), but
/// never a privacy part (ticked one by one), another account's files, or what can't be ticked.
/// The box was drawn from the part's risk while the press took only Green and Amber findings, so
/// an Amber part over the user's own folder (a cleaner for a project in Documents) showed a box
/// that a press left empty.
fn box_finding(f: &Finding, counted: bool, quiet: bool) -> bool {
    f.risk <= Risk::Orange && !f.other_account() && !quiet && !f.privacy()
        && untickable(f, counted).is_none() && crate::cleanup::held_in_app(f).is_none()
}

fn box_takes(r: &Report, i: usize) -> bool {
    box_finding(&r.findings[i], r.counted[i], r.quiet.contains(&key(&r.findings[i].path)))
}

/// What the Cleaners section has open.
#[derive(Default)]
pub(crate) struct ClState {
    /// Open cleaners, by id.
    pub(crate) open: HashSet<String>,
    /// The part whose files are shown (and lit on the map), by its key.
    pub(crate) show: Option<String>,
    /// The map lens for the shown part: (tree generation, a code per node).
    pub(crate) lens: Option<(u64, Arc<Vec<u8>>)>,
    /// Cleaners whose definition is shown.
    pub(crate) def: HashSet<String>,
    /// The programs running in this session, and when that was read (`None` inside: unknown).
    pub(crate) procs: Option<(std::time::Instant, Option<HashSet<String>>)>,
    /// What each proposed cleaner matches in the scan: by (tree generation, cleaner id).
    pub(crate) matches: std::cell::RefCell<HashMap<(u64, String), Option<(usize, u64)>>>,
    /// Community cleaners being fetched.
    pub(crate) fetching: Option<std::sync::mpsc::Receiver<sprawl_advisor::community::Status>>,
    /// Whether this session already looked if a fetch is due.
    pub(crate) fetch_checked: bool,
    /// The list of what the converter left out is shown.
    pub(crate) left_shown: bool,
    /// Parts the AI suggests ticking (part keys), and why.
    pub(crate) suggest: Option<(Vec<String>, String)>,
    /// The AI's reason for each suggested part, where it gave one (by part key, lower case).
    pub(crate) sug_reasons: HashMap<String, String>,
    /// What each part would free, being counted before a question goes to the AI (so it gets
    /// true numbers), and the question waiting for it.
    pub(crate) ai_count: Option<(std::sync::mpsc::Receiver<Vec<(String, u64)>>, String)>,
    /// What that count was already asked to count: a part that can't be counted (its folder
    /// can't be read) is tried once, not before every question for ever.
    pub(crate) ai_tried: std::collections::HashSet<String>,
    /// The part a "?" asked about: its answer goes to the details pane too.
    pub(crate) part_q: Option<String>,
    /// The AI's answers about parts, by part key.
    pub(crate) part_answers: HashMap<String, String>,
    /// The Cleaners tab's own read of the cleaners' folders, for when the scan on screen
    /// doesn't cover them (no scan, another drive, one folder).
    pub(crate) own: Option<Own>,
    /// That read going on: its progress, and where its answer comes.
    pub(crate) own_reading: Option<(Arc<sprawl_core::Progress>, std::sync::mpsc::Receiver<Result<Own, OwnFailed>>)>,
    /// Why the last read gave nothing: said once, in the tab's body beside Check.
    pub(crate) own_failed: Option<OwnFailed>,
    own_count: u64,
    /// The folders the cleaners look at, and the large ones the quick read leaves out
    /// (worked out once).
    pub(crate) roots: std::cell::RefCell<Option<(Vec<std::path::PathBuf>, Vec<std::path::PathBuf>)>>,
    /// The tab just opened: the user's remembered ticks are to be put back.
    pub(crate) restore_pending: bool,
    /// The user changed a tick: remembered at the next redraw.
    pub(crate) user_ticked: bool,
    /// The part shown in the details pane, by its key.
    pub(crate) sel: Option<String>,
    /// What a clean-up of the ticks would remove, counted without removing anything.
    pub(crate) preview: Option<PreviewRun>,
    /// The Cleaners group in the Advisor's findings is folded.
    pub(crate) hide_group: bool,
}

/// How many applications the Advisor's Cleaners group lists.
const GROUP_APPS: usize = 5;

/// One ticked item, previewed.
pub(crate) struct PrevItem {
    pub(crate) title: String,
    pub(crate) key: String,
    /// None: a program's own command decides what goes.
    pub(crate) got: Option<sprawl_advisor::clean::Preview>,
}

pub(crate) enum PreviewRun {
    Counting(std::sync::mpsc::Receiver<Vec<PrevItem>>),
    /// The items, and the ticks they were counted for.
    Done(Vec<PrevItem>, Vec<String>),
}

/// How many of the biggest files a preview lists.
const PREVIEW_FILES: usize = 25;

/// Why the Cleaners tab's own read gave nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OwnFailed {
    /// None of the cleaners' folders is here (or all are excluded): nothing to read, not a failure.
    NoFolders,
    /// The read failed or was stopped.
    Unreadable,
}

/// The cleaners' folders as read by the Cleaners tab itself, and the report made from them.
pub(crate) struct Own {
    pub(crate) tree: Arc<sprawl_core::Tree>,
    pub(crate) report: Report,
    /// When the read began: anything newer stays, as with a scan.
    pub(crate) began: std::time::SystemTime,
    pub(crate) roots: usize,
    /// Folders the cleaners look at that couldn't be opened, with Windows' reason: left out of
    /// the read and named, rather than failing all of it.
    pub(crate) unread: Vec<(std::path::PathBuf, String)>,
}

/// Report generations of the tab's own reads: apart from the scans' own.
const OWN_GEN: u64 = 1 << 62;

/// What a failed own read drops besides the report: the ticks, a review and a single pick made
/// in an earlier own read. A scan's ticks stay.
fn own_failed_drops(c: &mut crate::cleanup::Cleanup) {
    if c.ticked_for.is_some_and(|g| g >= OWN_GEN) {
        c.ticked.clear();
        c.ticked_for = None;
        c.review = false;
        c.single = None;
        c.privacy_ticked.clear();
        c.privacy_ok = false;
        c.ai_ticked.clear();
        c.own_ok = false;
    }
}

/// The folders left out of an own read, each with why Windows wouldn't open it: no guess that
/// administrator rights would help.
fn unread_line(unread: &[(std::path::PathBuf, String)]) -> String {
    let names: Vec<String> = unread.iter().map(|(p, why)| format!("{} ({why})", p.display())).collect();
    format!("Left out, as they couldn't be opened: {}", names.join("; "))
}

/// Folders no cleaner knows are listed from this size.
const UNCLAIMED_MIN: u64 = 100 << 20;
/// At most this many of them.
const UNCLAIMED_MAX: usize = 6;

/// The "keep the last N days" choices offered for a part whose definition keeps `def`.
fn day_choices(def: u32) -> Vec<u32> {
    let mut v: Vec<u32> = [def, 1, 3, 7, 30, 90].into_iter().filter(|&d| d >= def).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// One cleaner as listed.
pub(crate) struct ClRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) group: &'static str,
    /// Index into the report's catalogue (None: the "found by shape" list).
    pub(crate) def: Option<usize>,
    pub(crate) presence: Presence,
    pub(crate) parts: Vec<PartRow>,
}

pub(crate) struct PartRow {
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) risk: Risk,
    pub(crate) class: Class,
    pub(crate) closed: Closed,
    pub(crate) cost: String,
    pub(crate) why: String,
    pub(crate) info: bool,
    /// It clears a folder (so "keep the last N days" applies).
    pub(crate) clears: bool,
    pub(crate) keep_days: Option<u32>,
    /// The findings that are this part (indices into the report).
    pub(crate) findings: Vec<usize>,
}

impl PartRow {
    pub(crate) fn bytes(&self, r: &Report) -> u64 {
        self.findings.iter().filter(|&&i| r.counted[i]).map(|&i| r.findings[i].bytes).sum()
    }

    /// The findings that can be ticked.
    fn tickable<'a>(&'a self, r: &'a Report) -> impl Iterator<Item = usize> + 'a {
        self.findings.iter().copied().filter(move |&i| untickable(&r.findings[i], r.counted[i]).is_none())
    }
}

/// Where in the group order a group goes.
fn group_of(g: &str) -> &'static str {
    GROUPS.iter().copied().find(|x| *x == g).unwrap_or("Other")
}

impl State {
    /// The folders the cleaners look at (built-in and the user's own cleaners), and the large
    /// Windows folders the tab's quick read leaves to a scan of the drive.
    pub(crate) fn cleaner_roots_all(&self) -> (Vec<std::path::PathBuf>, Vec<std::path::PathBuf>) {
        self.advisor
            .cl
            .roots
            .borrow_mut()
            .get_or_insert_with(|| {
                let mut packs = vec![sprawl_advisor::RulePack::builtin()];
                packs.extend(sprawl_advisor::memory::load_packs().0);
                // with privacy cleaners on, the browsers' profile folders are read too
                sprawl_advisor::cleaners::quick_roots(&sprawl_advisor::cleaners::catalogue_with(&packs, self.privacy_on()))
            })
            .clone()
    }

    /// The folders the tab's quick read takes.
    pub(crate) fn cleaner_roots(&self) -> Vec<std::path::PathBuf> {
        self.cleaner_roots_all().0
    }

    /// The scan on screen covers every folder the cleaners look at (a scan of the system
    /// drive does), with no filter on: the Cleaners tab then uses its report.
    pub(crate) fn scan_covers_cleaners(&self) -> bool {
        let Some(t) = self.tree.as_ref() else { return false };
        if self.full_tree.is_some() || t.method == sprawl_core::ScanMethod::Csv {
            return false;
        }
        let low = |p: &str| p.trim_end_matches('\\').to_lowercase();
        let tops: Vec<String> = if t.nodes[sprawl_core::ROOT].name == sprawl_core::MULTIPLE {
            t.nodes[sprawl_core::ROOT].children.iter().map(|&k| low(&t.nodes[k].name)).collect()
        } else {
            vec![low(&t.root_path.to_string_lossy())]
        };
        self.cleaner_roots().iter().all(|r| {
            let r = low(&r.to_string_lossy());
            tops.iter().any(|top| r == *top || r.starts_with(&format!("{top}\\")))
        })
    }

    /// The tree the report on show was made from: the tab's own read, or the whole scan (not a
    /// filtered view of it).
    pub(crate) fn active_tree(&self) -> Option<Arc<sprawl_core::Tree>> {
        if self.own_active() {
            return self.advisor.cl.own.as_ref().map(|o| o.tree.clone());
        }
        self.full_tree.clone().or_else(|| self.tree.clone())
    }

    /// When the tree on show began to be read: anything newer is left alone.
    pub(crate) fn active_began(&self) -> std::time::SystemTime {
        match self.advisor.cl.own.as_ref().filter(|_| self.own_active()) {
            Some(o) => o.began,
            None => self.scan_began,
        }
    }

    /// What a clean-up of the report on show leaves alone: anything new since its read began,
    /// what the user keeps, and anything that is not the item the read saw at its path.
    pub(crate) fn spare(&self) -> sprawl_advisor::clean::Spare {
        sprawl_advisor::clean::Spare {
            since: Some(self.active_began()),
            keep: self.advisor.memory.kept_paths(),
            seen: self.active_tree().map(sprawl_advisor::clean::seen_in),
        }
    }

    /// The Cleaners tab is on show and works from its own read, not the scan's report.
    pub(crate) fn own_active(&self) -> bool {
        if self.view != crate::state::View::Advisor || !self.advisor.cleaners_tab_on() {
            return false;
        }
        // while a scan runs its report isn't there yet: the last read stays on show (nothing
        // can be cleaned meanwhile, and no new read starts)
        if self.job.is_some() {
            return self.advisor.cl.own.is_some();
        }
        !self.scan_covers_cleaners()
    }

    /// Read the cleaners' folders and check them, off the UI thread (the tab opened, Check
    /// again, or after a clean-up). Every finding passes the same safety checks, against this
    /// read.
    pub(crate) fn own_start(&mut self) {
        // not during a Delete or an uncompress: its handles would hold the folders being removed
        if self.job.is_some() || self.advisor.cl.own_reading.is_some() || self.advisor.file_op() {
            return;
        }
        self.advisor.cl.own_failed = None;
        // the user's decisions (what they keep) apply to this read too
        self.advisor.memory_ready();
        let mut roots = self.cleaner_roots();
        self.advisor.cl.own_count += 1;
        let gen = OWN_GEN + self.advisor.cl.own_count;
        let memory = self.advisor.memory.clone();
        let privacy_on = self.privacy_on();
        let progress = sprawl_advisor::exclusions::progress();
        roots.retain(|p| !progress.excludes(&sprawl_core::canonical_root(p)));
        let p2 = progress.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let began = std::time::SystemTime::now();
            let own = (|| {
                if roots.is_empty() {
                    return Err(OwnFailed::NoFolders);
                }
                // one folder this account can't open would fail the whole read: it is left out
                let (mut ok, mut unread) = (Vec::new(), Vec::new());
                for r in roots {
                    match std::fs::read_dir(&r) {
                        Ok(_) => ok.push(r),
                        Err(e) => unread.push((r, e.to_string())),
                    }
                }
                let roots = ok;
                if roots.is_empty() {
                    return Err(OwnFailed::Unreadable);
                }
                let tree = Arc::new(sprawl_core::scan_many_inventory(&roots, &p2).map_err(|_| OwnFailed::Unreadable)?);
                if p2.is_cancelled() { return Err(OwnFailed::Unreadable); }
                let measured_at = began.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let (report, _) = crate::advisor::caught(|| crate::advisor::analyse(&tree, gen, &memory, privacy_on, measured_at)).ok_or(OwnFailed::Unreadable)?;
                Ok(Own { tree, report, began, roots: roots.len(), unread })
            })();
            let _ = tx.send(own);
        });
        self.advisor.cl.own_reading = Some((progress, rx));
    }

    /// The tab's own read, when it is done.
    pub(crate) fn own_poll(&mut self) {
        let Some((progress, rx)) = &self.advisor.cl.own_reading else { return };
        let got = match rx.try_recv() {
            Ok(o) => o,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // the count as it goes, without a redraw
                if self.own_active() {
                    self.advisor.ptext("ClReading", &format!("Reading the folders the cleaners look at... {} items", crate::format::thousands(progress.items())));
                }
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(OwnFailed::Unreadable),
        };
        self.advisor.cl.own_reading = None;
        // the failure is said in the tab's body only, beside its Check button: not again in
        // the header's status line and the foot
        let got = match got {
            Ok(o) => Some(o),
            Err(why) => {
                self.advisor.cl.own_failed = Some(why);
                None
            }
        };
        if let Some(o) = &got {
            if self.advisor.memory.learning.mark_refills(&o.report.refill_observations) { self.save_memory(); }
        }
        // A failed read replaces the last one: its report and what was ticked in it go, so
        // nothing from before can be cleaned up until a read succeeds (Astra 1.0.2 S5 #4).
        if got.is_none() {
            own_failed_drops(&mut self.advisor.cleanup);
        }
        self.advisor.cl.own = got;
        self.invalidate_chat_if_changed();
        if self.own_active() {
            self.cleanup_defaults();
            self.cleanup_measure_report();
            self.render_advisor();
        }
    }

    /// The Cleaners tab opened: read the cleaners' folders when the scan doesn't cover them
    /// and the last read is older than a few minutes.
    pub(crate) fn own_on_open(&mut self) {
        if !self.own_active() {
            return;
        }
        let old = self.advisor.cl.own.as_ref().is_none_or(|o| o.began.elapsed().map_or(true, |d| d.as_secs() > 300));
        if old {
            self.own_start();
        }
    }

    /// The cleaners to list for report `r`, in order: by group, then the biggest first. Cleaners
    /// whose application isn't on this PC are left out.
    pub(crate) fn cleaner_layout(&self, r: &Report) -> Vec<ClRow> {
        let mut by_rule: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, f) in r.findings.iter().enumerate() {
            if let Source::Rule(id) = &f.source {
                by_rule.entry(id.as_str()).or_default().push(i);
            }
        }
        let mut rows: Vec<ClRow> = Vec::new();
        for (k, d) in r.catalogue.iter().enumerate() {
            // the privacy cleaners have a group of their own (privacy_group), never these rows
            if sprawl_advisor::privacy::is_privacy_id(&d.id) || sprawl_advisor::rules::reserved_id(&d.id) {
                continue;
            }
            let presence = r.presence.get(k).cloned().unwrap_or(Presence::Absent);
            if presence == Presence::Absent {
                continue;
            }
            // thousands come from the community files: only those with something here
            if sprawl_advisor::community::is_community(&d.id) && !d.parts.iter().any(|p| by_rule.contains_key(p.rule.as_str())) {
                continue;
            }
            let parts = d
                .parts
                .iter()
                .map(|p| PartRow {
                    key: p.rule.clone(),
                    title: p.title.clone(),
                    risk: p.risk,
                    class: p.class,
                    closed: p.closed,
                    cost: p.cost.clone(),
                    why: p.why.clone(),
                    info: p.info,
                    clears: p.clears,
                    keep_days: p.keep_days,
                    findings: by_rule.get(p.rule.as_str()).cloned().unwrap_or_default(),
                })
                .collect();
            rows.push(ClRow { id: d.id.clone(), name: d.name.clone(), group: group_of(&d.group), def: Some(k), presence, parts });
        }
        // caches of programs no cleaner knows, one part each
        let shape: Vec<PartRow> = by_rule
            .get(SHAPE)
            .into_iter()
            .flatten()
            .map(|&i| {
                let f = &r.findings[i];
                PartRow {
                    key: key(&f.path),
                    title: f.title.clone(),
                    risk: f.risk,
                    class: Class::Space,
                    closed: Closed::Warn,
                    cost: "The program downloads or makes these again; it may be slower once.".into(),
                    why: f.evidence.first().cloned().unwrap_or_default(),
                    info: false,
                    clears: true,
                    keep_days: None,
                    findings: vec![i],
                }
            })
            .collect();
        if !shape.is_empty() {
            rows.push(ClRow { id: SHAPE.into(), name: "Other apps (found by shape)".into(), group: "Other", def: None, presence: Presence::InScan, parts: shape });
        }
        // caches of programs built on Chromium, known by their folder
        let chromium: Vec<PartRow> = by_rule
            .get(sprawl_advisor::chromium::ID)
            .into_iter()
            .flatten()
            .map(|&i| {
                let f = &r.findings[i];
                PartRow {
                    key: key(&f.path),
                    title: f.title.clone(),
                    risk: f.risk,
                    class: Class::Space,
                    closed: Closed::Warn,
                    cost: "The program downloads or makes these again; it may be slower once.".into(),
                    why: f.evidence.first().cloned().unwrap_or_default(),
                    info: false,
                    clears: true,
                    keep_days: None,
                    findings: vec![i],
                }
            })
            .collect();
        if !chromium.is_empty() {
            rows.push(ClRow { id: sprawl_advisor::chromium::ID.into(), name: "Apps built on Chromium (recognised)".into(), group: "Other", def: None, presence: Presence::InScan, parts: chromium });
        }
        let size = |c: &ClRow| -> u64 { c.parts.iter().map(|p| p.bytes(r)).sum() };
        rows.sort_by_key(|c| (GROUPS.iter().position(|g| *g == c.group).unwrap_or(GROUPS.len()), c.id == SHAPE, std::cmp::Reverse(size(c))));
        rows
    }

    /// How many cleaners and parts are listed (for wiring their buttons).
    pub(crate) fn cleaner_counts(&self) -> (usize, usize) {
        let Some(r) = self.advice() else { return (0, 0) };
        let rows = self.cleaner_layout(r);
        (rows.len(), rows.iter().map(|c| c.parts.len()).sum())
    }

    /// Whether the user turned community cleaner `c` on (for its definition as it is now).
    pub(crate) fn community_on(&self, r: &Report, c: &ClRow) -> bool {
        let Some(d) = c.def.map(|k| &r.catalogue[k]) else { return false };
        self.advisor.memory.community_on.get(&d.id).is_some_and(|f| *f == sprawl_advisor::community::fingerprint(&d.text))
    }

    /// Where the community cleaners come from, when they were fetched, and what was left out.
    fn community_card(&self) -> String {
        let off = self.advisor.memory.community_off;
        let st = sprawl_advisor::community::status();
        let mut x = String::from(r##"<Border Padding="12,10" CornerRadius="{$rc7}" Background="{$surface2}" Margin="0,8,0,0"><StackPanel Spacing="4"><TextBlock Text="Community cleaners" FontSize="{$fs13}" FontWeight="SemiBold"/>"##);
        let line = if off {
            "Off: winapp2.ini and BleachBit's cleaners are neither fetched nor shown.".to_string()
        } else if self.advisor.cl.fetching.is_some() {
            "Fetching winapp2.ini and BleachBit's cleaners...".to_string()
        } else if st.fetched == 0 {
            "winapp2.ini and BleachBit's cleaners are fetched from GitHub once a week (a plain download; nothing about this PC is sent).".to_string()
        } else {
            let srcs: Vec<String> = st.sources.iter().map(|s| format!("{}: {} cleaners ({})", s.name, s.imported, s.licence)).collect();
            format!("{}. Fetched {}. Each is checked like your own rules, at least amber, and does nothing until you turn it on.", srcs.join(", "), crate::format::fmt_time(std::time::UNIX_EPOCH + std::time::Duration::from_secs(st.fetched)))
        };
        x += &format!(r##"<TextBlock Text="{}" TextWrapping="Wrap" FontSize="{{$fs12}}" Foreground="{{$text3}}"/>"##, ui::esc(&line));
        for s in st.sources.iter().filter(|s| !s.error.is_empty()) {
            x += &format!(r##"<TextBlock Text="{}" TextWrapping="Wrap" FontSize="{{$fs12}}" Foreground="{{$warn}}"/>"##, ui::esc(&format!("{}: {}", s.name, s.error)));
        }
        let left: usize = st.sources.iter().map(|s| s.left_out.len()).sum();
        x += r##"<StackPanel Orientation="Horizontal" Spacing="8">"##;
        if !off {
            x += r##"<Button x:Name="AdvClUpd" Content="Fetch now"/>"##;
        }
        x += &format!(r##"<Button x:Name="AdvClComm" Content="{}"/>"##, if off { "Turn on" } else { "Turn off" });
        if left > 0 && !off {
            x += &format!(
                r##"<Button x:Name="AdvClLeft" Content="{}" Style="{{StaticResource SprawlSubtle}}"/>"##,
                ui::esc(&format!("{} {} entries left out", if self.advisor.cl.left_shown { "Hide the" } else { "Show the" }, crate::format::thousands(left as u64)))
            );
        }
        x += "</StackPanel>";
        if self.advisor.cl.left_shown && !off {
            x += r##"<ScrollViewer MaxHeight="220" VerticalScrollBarVisibility="Auto"><StackPanel Spacing="1">"##;
            for l in st.sources.iter().flat_map(|s| s.left_out.iter()).take(400) {
                x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" TextWrapping="Wrap"/>"##, ui::esc(&format!("{}: {}", l.entry, l.why)));
            }
            x += "</StackPanel></ScrollViewer>";
        }
        x += "</StackPanel></Border>";
        x
    }

    /// Fetch the community files again, off the UI thread.
    pub(crate) fn community_fetch(&mut self) {
        // the user's choice is in the memory: before it is read, `community_off` is a default
        self.advisor.memory_ready();
        if self.advisor.cl.fetching.is_some() || self.advisor.memory.community_off || sprawl_advisor::offline() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(sprawl_advisor::community::update());
        });
        self.advisor.cl.fetching = Some(rx);
        self.render_advisor();
    }

    /// Each frame: once a session, fetch when a week has passed (only while the Advisor is open,
    /// never in a screenshot run); and take a finished fetch in.
    pub(crate) fn community_poll(&mut self) {
        if !self.advisor.cl.fetch_checked && self.view == crate::state::View::Advisor && self.shot.is_none() {
            // read the user's choice first: an unread memory says "on" (opening on the Cleaners
            // or Advisor tab, before any scan, fetched for users who had it off)
            self.advisor.memory_ready();
        }
        if !self.advisor.cl.fetch_checked && self.view == crate::state::View::Advisor && !self.advisor.memory.community_off && self.shot.is_none() {
            self.advisor.cl.fetch_checked = true;
            if sprawl_advisor::community::due(&sprawl_advisor::community::status()) {
                self.community_fetch();
            }
        }
        let Some(rx) = &self.advisor.cl.fetching else { return };
        let Ok(st) = rx.try_recv() else { return };
        self.advisor.cl.fetching = None;
        let n: usize = st.sources.iter().map(|s| s.imported).sum();
        let errors: Vec<String> = st.sources.iter().filter(|s| !s.error.is_empty()).map(|s| format!("{}: {}", s.name, s.error)).collect();
        if errors.is_empty() {
            self.adv_say(&format!("Community cleaners fetched: {n} can be used on Windows. Checking again with them..."));
        } else {
            self.adv_say(&format!("Community cleaners: {}", errors.join("; ")));
        }
        self.advisor_start();
    }

    /// Big folders of programs no cleaner knows, each with a button that asks the AI for one.
    fn unclaimed_card(&self, r: &Report) -> String {
        let list = self.unclaimed_list(r);
        if list.is_empty() {
            return String::new();
        }
        let ai = AiConfig::load().provider != Provider::Off;
        let mut x = format!(
            r##"<TextBlock Text="Unknown applications" FontSize="{{$fs12}}" FontWeight="SemiBold" Foreground="{{$label}}" Margin="0,10,0,2"/><Border Padding="12,10" CornerRadius="{{$rc7}}" Background="{{$surface2}}"><StackPanel Spacing="4"><TextBlock Text="{}" FontSize="{{$fs13}}" FontWeight="SemiBold"/><TextBlock Text="{}" TextWrapping="Wrap" FontSize="{{$fs12}}" Foreground="{{$text3}}"/>"##,
            ui::esc(&format!("Sprawl doesn't know {} {}", list.len(), if list.len() == 1 { "application" } else { "applications" })),
            if ai {
                "Big folders of programs in your AppData that no cleaner knows. The AI can look inside and propose a cleaner; a second opinion checks it, and you approve it before it is used."
            } else {
                "Big folders of programs in your AppData that no cleaner knows. With AI set up, it can look inside and propose a cleaner for you to approve."
            }
        );
        for (i, (p, bytes)) in list.iter().enumerate() {
            x += &format!(
                r##"<Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="80"/></Grid.ColumnDefinitions><TextBlock Text="{}" FontSize="{{$fs12}}" FontFamily="Consolas" Foreground="{{$text2}}" TextTrimming="CharacterEllipsis" VerticalAlignment="Center" ToolTipService.ToolTip="{0}"/>{}<TextBlock Grid.Column="2" Text="{}" FontSize="{{$fs13}}" HorizontalAlignment="Right" VerticalAlignment="Center"/></Grid>"##,
                ui::esc(&p.display().to_string()),
                if ai { format!(r##"<Button x:Name="AdvClU{i}" Grid.Column="1" Content="Ask the AI to look" Padding="8,1" FontSize="{{$fs12}}" CornerRadius="{{$rc4}}"/>"##) } else { String::new() },
                self.size(*bytes)
            );
        }
        x += "</StackPanel></Border>";
        x
    }

    pub(crate) fn unclaimed_list(&self, r: &Report) -> Vec<(std::path::PathBuf, u64)> {
        let Some(t) = self.active_tree() else { return Vec::new() };
        let mut v = sprawl_advisor::cleaners::unclaimed(&t, &r.catalogue, UNCLAIMED_MIN);
        v.truncate(UNCLAIMED_MAX);
        v
    }

    /// The card of cleaners the AI proposes, in Remembered.
    pub(crate) fn cleaner_proposals_card(&self) -> String {
        let m = &self.advisor.memory;
        if m.cleaner_proposals.is_empty() {
            return String::new();
        }
        let mut x = String::from(
            r##"<Border Padding="12,10" CornerRadius="{$rc7}" Background="{$surface2}"><StackPanel Spacing="6"><TextBlock Text="Cleaners the AI proposes" FontSize="{$fs13}" FontWeight="SemiBold"/><TextBlock Text="Approved cleaners join your own rules and show in Cleaners. Their parts are at least amber, never run as administrator, and nothing is cleaned until you tick it, review it and confirm." TextWrapping="Wrap" FontSize="{$fs12}" Foreground="{$text3}"/>"##,
        );
        for (i, p) in m.cleaner_proposals.iter().enumerate() {
            let c = &p.cleaner;
            let now = match self.proposal_matches(c) {
                Some((0, _)) => "Matches nothing in this scan now".to_string(),
                Some((n, b)) => format!("Matches {} in {n} {} now", self.size(b), if n == 1 { "part" } else { "parts" }),
                None => "Can't be matched against this scan".to_string(),
            };
            x += &format!(
                r##"<StackPanel Spacing="2" Margin="0,4,0,0"><StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" FontSize="{{$fs12}}" FontWeight="SemiBold" VerticalAlignment="Center"/><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" VerticalAlignment="Center"/><Button x:Name="AdvClApprove{i}" Content="Approve" Padding="8,1" FontSize="{{$fs12}}" CornerRadius="{{$rc4}}"/><Button x:Name="AdvClDismiss{i}" Content="Dismiss" Padding="8,1" FontSize="{{$fs12}}" CornerRadius="{{$rc4}}"/></StackPanel>"##,
                ui::esc(&c.name),
                ui::esc(&now)
            );
            for part in &c.parts {
                x += &format!(
                    r##"<StackPanel Orientation="Horizontal" Spacing="8" Margin="12,0,0,0">{}<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text2}}" TextWrapping="Wrap" MaxWidth="600"/></StackPanel><TextBlock Text="{}" FontSize="{{$fs12}}" FontFamily="Consolas" Foreground="{{$text3}}" TextWrapping="Wrap" Margin="28,0,0,0"/>"##,
                    crate::advisor::risk_mark(part.risk, 8, crate::advisor::MID),
                    ui::esc(&format!("{}: {}", part.title, part.why)),
                    ui::esc(&part.paths.join("   "))
                );
            }
            if !p.note.is_empty() {
                x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" FontStyle="Italic" TextWrapping="Wrap" Margin="12,0,0,0"/>"##, ui::esc(&p.note));
            }
            x += "</StackPanel>";
        }
        x += "</StackPanel></Border>";
        x
    }

    /// What a proposed cleaner matches in the scan now (kept per scan).
    fn proposal_matches(&self, c: &sprawl_advisor::cleaners::Cleaner) -> Option<(usize, u64)> {
        let t = self.tree.as_ref()?;
        let k = (self.tree_gen, c.id.clone());
        if let Some(v) = self.advisor.cl.matches.borrow().get(&k) {
            return *v;
        }
        let v = sprawl_advisor::cleaners::match_now(t, c).ok();
        self.advisor.cl.matches.borrow_mut().insert(k, v);
        v
    }

    /// The biggest files a part would remove, and where they are.
    fn part_files(&self, r: &Report, p: &PartRow) -> String {
        let Some(t) = self.active_tree() else { return String::new() };
        let mut files: Vec<usize> = Vec::new();
        let mut count = 0u64;
        for &i in &p.findings {
            let Some(n) = r.nodes.get(i).copied().flatten() else { continue };
            let mut stack = vec![n];
            while let Some(k) = stack.pop() {
                let node = &t.nodes[k];
                if node.is_dir {
                    stack.extend_from_slice(&node.children);
                } else {
                    count += 1;
                    files.push(k);
                }
            }
        }
        files.sort_by_key(|&k| std::cmp::Reverse(t.nodes[k].size));
        let mut x = String::from(r##"<Border Margin="34,0,0,4" Padding="12,8" CornerRadius="{$rc7}" Background="{$surface1}"><StackPanel Spacing="2">"##);
        x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text2}}" TextWrapping="Wrap"/>"##, ui::esc(&p.why));
        for &i in &p.findings {
            x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" IsTextSelectionEnabled="True" TextTrimming="CharacterEllipsis"/>"##, ui::esc(&r.findings[i].path.display().to_string()));
        }
        x += &format!(r##"<TextBlock Text="{} files, the biggest:" FontSize="{{$fs12}}" Foreground="{{$text3}}" Margin="0,4,0,0"/>"##, crate::format::thousands(count));
        for &k in files.iter().take(12) {
            let name = t.nodes[k].name.clone();
            x += &format!(
                r##"<Grid><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="80"/></Grid.ColumnDefinitions><TextBlock Text="{}" FontSize="{{$fs12}}" FontFamily="Consolas" Foreground="{{$text2}}" TextTrimming="CharacterEllipsis"/><TextBlock Grid.Column="1" Text="{}" FontSize="{{$fs12}}" HorizontalAlignment="Right" Foreground="{{$text2}}"/></Grid>"##,
                ui::esc(&name),
                self.size(t.nodes[k].size)
            );
        }
        x += "</StackPanel></Border>";
        x
    }

    /// A Cleaners button: open a cleaner, tick a part, show a part's files, scan a drive.
    pub(crate) fn cleaner_command(&mut self, kind: &str, i: usize) {
        let Some(r) = self.advice() else { return };
        let rows = self.cleaner_layout(r);
        match kind {
            "AdvClO" => {
                let Some(c) = rows.get(i) else { return };
                if !self.advisor.cl.open.remove(&c.id) {
                    self.advisor.cl.open.insert(c.id.clone());
                }
            }
            "AdvClDef" => {
                let Some(c) = rows.get(i) else { return };
                if !self.advisor.cl.def.remove(&c.id) {
                    self.advisor.cl.def.insert(c.id.clone());
                }
            }
            "AdvClScan" => {
                if let Some(Presence::Elsewhere(drive)) = rows.get(i).map(|c| c.presence.clone()) {
                    self.start_scan(drive);
                }
                return;
            }
            "AdvClA" => {
                let Some(c) = rows.get(i) else { return };
                let keys: Vec<String> = c.parts.iter().filter(|p| app_tick_takes(p)).flat_map(|p| p.tickable(r).filter(|&k| box_takes(r, k)).map(|k| key(&r.findings[k].path))).collect();
                // partly ticked: the first press unticks the app, as a box that shows a mark
                let any = c.parts.iter().flat_map(|p| p.findings.iter()).any(|&k| self.advisor.cleanup.ticked.contains(&key(&r.findings[k].path)));
                let all = !keys.is_empty() && keys.iter().all(|k| self.advisor.cleanup.ticked.contains(k));
                if any && !all {
                    let mine: Vec<String> = c.parts.iter().flat_map(|p| p.findings.iter()).map(|&k| key(&r.findings[k].path)).collect();
                    for k in mine {
                        self.advisor.cleanup.ticked.remove(&k);
                    }
                    let c = &mut self.advisor.cleanup;
                    c.single = None;
                    c.own_ok = false;
                    c.auto = false;
                } else {
                    self.cleanup_tick_many(keys);
                }
                self.advisor.focus_after = Some(format!("AdvClA{i}"));
                self.advisor.cl.user_ticked = true;
            }
            "AdvClP" => {
                let Some(p) = rows.iter().flat_map(|c| c.parts.iter()).nth(i) else { return };
                let k = p.key.clone();
                self.advisor.cl.preview = None;
                self.advisor.cl.sel = if self.advisor.cl.sel.as_deref() == Some(k.as_str()) { None } else { Some(k) };
            }
            "AdvClT" => {
                let Some(p) = rows.iter().flat_map(|c| c.parts.iter()).nth(i) else { return };
                let keys: Vec<String> = p.tickable(r).map(|k| key(&r.findings[k].path)).collect();
                self.cleanup_tick_many(keys);
                self.advisor.focus_after = Some(format!("AdvClT{i}"));
                self.advisor.cl.user_ticked = true;
            }
            "AdvClQ" => {
                let Some((c, p)) = rows.iter().flat_map(|c| c.parts.iter().map(move |p| (c, p))).nth(i) else { return };
                let paths: Vec<String> = p.findings.iter().map(|&k| r.findings[k].path.display().to_string()).collect();
                let (bytes, counted) = self.part_true(r, p);
                let q = format!(
                    "What happens if I clean \"{}: {}\" ({}, {})? What would I notice afterwards, and is there a reason to keep it? If you aren't sure, say so.",
                    c.name,
                    p.title,
                    paths.join(", "),
                    if counted { crate::format::size(bytes) } else { format!("up to {}", crate::format::size(bytes)) }
                );
                // the answer shows in this part's details, which stay on it
                let k = p.key.clone();
                self.advisor.cl.sel = Some(k.clone());
                self.advisor.cl.part_answers.remove(&k);
                self.advisor.cl.part_q = Some(k);
                self.ask_about(q);
            }
            "AdvClSgT" | "AdvClSgX" => {
                let Some(p) = rows.iter().flat_map(|c| c.parts.iter()).nth(i) else { return };
                let k = p.key.clone();
                if kind == "AdvClSgT" {
                    self.suggest_tick_one(&k);
                    self.advisor.cl.user_ticked = true;
                } else {
                    self.suggest_decline(Some(k));
                }
            }
            "AdvClU" => {
                let Some((path, bytes)) = self.unclaimed_list(r).into_iter().nth(i) else { return };
                let q = format!(
                    "Look inside {} ({}). Which program is it, and which of its folders come back by themselves (caches, logs, crash dumps)? If some do, propose a cleaner for them with propose_cleaner; leave out anything that is the user's own data, settings or sign-ins.",
                    path.display(),
                    crate::format::size(bytes)
                );
                self.ask_about(q);
            }
            "AdvClF" => {
                let Some(id) = rows.get(i).filter(|c| c.def.is_some_and(|k| !r.catalogue[k].builtin)).map(|c| c.id.clone()) else { return };
                match sprawl_advisor::memory::forget_cleaner(&id) {
                    Ok(()) => {
                        self.adv_say("Cleaner removed from your rules. Checking again without it...");
                        return self.advisor_start();
                    }
                    Err(e) => self.adv_say(&format!("Couldn't remove it: {e}")),
                }
            }
            "AdvClUse" => {
                let Some(d) = rows.get(i).and_then(|c| c.def).map(|k| &r.catalogue[k]).filter(|d| sprawl_advisor::community::is_community(&d.id)) else { return };
                let (id, fp) = (d.id.clone(), sprawl_advisor::community::fingerprint(&d.text));
                let m = &mut self.advisor.memory;
                if m.community_on.get(&id) == Some(&fp) {
                    m.community_on.remove(&id);
                } else {
                    m.community_on.insert(id, fp);
                }
                self.save_memory();
                return self.advisor_start();
            }
            "AdvClApprove" => match self.advisor.memory.cleaner_proposals.get(i).map(|p| p.cleaner.id.clone()).map(|id| (id, self.advisor.memory.approve_cleaner(i))) {
                Some((id, Ok(()))) => {
                    self.advisor.just_approved.push(id);
                    // saved at once, with the check it owes, which runs on leaving this section
                    self.save_memory();
                    self.adv_say("Approved: the cleaner is in your rules. It is used when you leave Remembered.");
                    return self.advisor_changed();
                }
                None => {}
                Some((_, Err(e))) => self.adv_say(&format!("Couldn't add the cleaner: {e}")),
            },
            "AdvClDismiss" => {
                if i < self.advisor.memory.cleaner_proposals.len() {
                    self.advisor.memory.cleaner_proposals.remove(i);
                    self.save_memory();
                }
            }
            "AdvClK" => {
                let Some(p) = rows.iter().flat_map(|c| c.parts.iter()).nth(i) else { return };
                let def = p.keep_days.unwrap_or(0);
                let cur = self.part_days(p);
                let choices = day_choices(def);
                let next = choices.iter().copied().find(|&d| d > cur).unwrap_or(def);
                let (rule, findings) = (p.key.clone(), p.findings.clone());
                if next == def {
                    self.advisor.memory.part_days.remove(&rule);
                } else {
                    self.advisor.memory.part_days.insert(rule, next);
                }
                self.save_memory();
                // the findings of this report act on the new number now
                let r = if self.own_active() { self.advisor.cl.own.as_mut().map(|o| &mut o.report) } else { self.advisor.report.as_mut() };
                if let Some(r) = r {
                    for k in findings {
                        if let Some(f) = r.findings.get_mut(k) {
                            if let Some(sprawl_advisor::clean::Action::Clear { .. }) = f.action {
                                let admin = f.admin;
                                f.set_action(sprawl_advisor::clean::Action::Clear { days: next }, admin);
                            }
                        }
                    }
                }
                self.cleanup_accept_context();
            }
            "AdvClS" => {
                let Some(p) = rows.iter().flat_map(|c| c.parts.iter()).nth(i) else { return };
                if self.advisor.cl.show.as_deref() == Some(p.key.as_str()) {
                    self.advisor.cl.show = None;
                    self.advisor.cl.lens = None;
                } else {
                    let nodes: Vec<usize> = p.findings.iter().filter_map(|&k| r.nodes.get(k).copied().flatten()).collect();
                    self.advisor.cl.show = Some(p.key.clone());
                    self.advisor.cl.lens = self.tree.as_ref().map(|t| {
                        let mut v = vec![0u8; t.nodes.len()];
                        let mut stack = nodes;
                        while let Some(k) = stack.pop() {
                            v[k] = crate::search::LENS_MATCH;
                            stack.extend_from_slice(&t.nodes[k].children);
                        }
                        (self.tree_gen, Arc::new(v))
                    });
                }
                self.map.drawn_once = false;
            }
            _ => return,
        }
        self.render_advisor();
    }

    /// The part's "keep the last N days": the user's choice, never below the definition's.
    pub(crate) fn part_days(&self, p: &PartRow) -> u32 {
        let def = p.keep_days.unwrap_or(0);
        self.advisor.memory.part_days.get(&p.key).copied().unwrap_or(def).max(def)
    }

    /// Read the running programs again when the last look is older than a few seconds.
    pub(crate) fn cleaner_procs(&mut self, force: bool) {
        let stale = self.advisor.cl.procs.as_ref().is_none_or(|(t, _)| t.elapsed().as_secs() >= 3);
        if force || stale {
            self.advisor.cl.procs = Some((std::time::Instant::now(), sprawl_advisor::procs::running()));
        }
    }

    /// The application of `f`'s cleaner part when it is open (or when that can't be told and
    /// the part must have it closed), with what that means for the part.
    pub(crate) fn app_open(&self, f: &Finding) -> Option<(String, Closed)> {
        let r = self.advice()?;
        let Source::Rule(id) = &f.source else { return None };
        let (d, p) = sprawl_advisor::cleaners::part_of(&r.catalogue, id)?;
        if d.running.is_empty() || p.closed == Closed::No {
            return None;
        }
        let now = self.advisor.cl.procs.as_ref().and_then(|(_, s)| s.as_ref());
        match sprawl_advisor::procs::any_running(&d.running, now) {
            Some(true) => Some((d.name.clone(), p.closed)),
            None if p.closed == Closed::Must => Some((d.name.clone(), Closed::Must)),
            _ => None,
        }
    }

    /// For a part that must have its application closed: the application and its programs, to
    /// check again just before the part is cleaned.
    pub(crate) fn part_gate(&self, f: &Finding) -> Option<(String, Vec<String>)> {
        let r = self.advice()?;
        let Source::Rule(id) = &f.source else { return None };
        let (d, p) = sprawl_advisor::cleaners::part_of(&r.catalogue, id)?;
        (p.closed == Closed::Must && !d.running.is_empty()).then(|| (d.name.clone(), d.running.clone()))
    }

    /// The paths "Tick my usual ones" would tick.
    pub(crate) fn usual_keys(&self, r: &Report) -> Vec<String> {
        let usual = &self.advisor.memory.usual;
        if usual.is_empty() {
            return Vec::new();
        }
        r.findings
            .iter()
            .enumerate()
            .filter(|(i, f)| auto_takes(r, *i) && matches!(&f.source, Source::Rule(id) if usual.contains(id)
                && sprawl_advisor::cleaners::selection_fingerprint(&r.catalogue, id).is_some_and(|fp| self.advisor.memory.usual_definitions.get(id) == Some(&fp))))
            .map(|(_, f)| key(&f.path))
            .filter(|k| !self.advisor.cleanup.ticked.contains(k))
            .collect()
    }

    /// Remember the amber space parts in a clean-up, for "Tick my usual ones" next time.
    pub(crate) fn remember_usual(&mut self, picked: &[Finding]) {
        let Some(r) = self.advice() else { return };
        let mut ids: Vec<String> = picked
            .iter()
            .filter(|f| f.risk == Risk::Amber && crate::cleanup::counts_as_space(f))
            .filter_map(|f| if let Source::Rule(id) = &f.source { Some(id.clone()) } else { None })
            .filter(|id| sprawl_advisor::cleaners::part_of(&r.catalogue, id).is_some_and(|(_, p)| p.class == Class::Space))
            .collect();
        ids.sort();
        ids.dedup();
        let definitions = ids.iter().filter_map(|id| sprawl_advisor::cleaners::selection_fingerprint(&r.catalogue, id).map(|fp| (id.clone(), fp))).collect();
        if !ids.is_empty() && (ids != self.advisor.memory.usual || definitions != self.advisor.memory.usual_definitions) {
            self.advisor.memory.usual = ids;
            self.advisor.memory.usual_definitions = definitions;
            self.save_memory();
        }
    }

    /// The cleaners as text, for the read-only MCP tool.
    /// The parts the AI suggested that can be ticked: safe or amber, with something to tick.
    fn suggested_parts<'a>(&self, r: &Report, rows: &'a [ClRow]) -> (Vec<(&'a ClRow, &'a PartRow)>, usize) {
        let Some((keys, _)) = &self.advisor.cl.suggest else { return (Vec::new(), 0) };
        let mut ok = Vec::new();
        let mut left = 0;
        for k in keys {
            // a privacy part is never suggested, whatever the model sent
            let declined = self.advisor.memory.declined_parts.iter().any(|d| d.eq_ignore_ascii_case(k)) || sprawl_advisor::rules::reserved_id(k);
            match rows.iter().flat_map(|c| c.parts.iter().map(move |p| (c, p))).find(|(_, p)| p.key.eq_ignore_ascii_case(k)) {
                // safe and amber space only: never a privacy part, nor one the user turned down
                Some((c, p)) if !declined && p.class == Class::Space && !p.info && matches!(p.risk, Risk::Green | Risk::Amber) && p.tickable(r).next().is_some() && p.tickable(r).all(|i| auto_takes(r, i)) && self.part_true(r, p).0 > 0 => ok.push((c, p)),
                _ => left += 1,
            }
        }
        (ok, left)
    }

    /// The AI's suggestion: which parts, how much, why; Tick them adds their ticks (the review
    /// still comes before anything is deleted).
    fn suggest_card(&self, r: &Report) -> String {
        let Some((_, why)) = &self.advisor.cl.suggest else { return String::new() };
        let rows = self.cleaner_layout(r);
        let (ok, left) = self.suggested_parts(r, &rows);
        let bytes: u64 = ok.iter().map(|(_, p)| self.part_true(r, p).0).sum();
        let mut x = String::from(r##"<Border Padding="12,10" CornerRadius="{$rc7}" Background="{$accentwash}" BorderBrush="{$accentline}" BorderThickness="1"><StackPanel Spacing="6">"##);
        x += &format!(r##"<TextBlock Text="The AI suggests ticking {} part{} ({})" FontSize="{{$fs13}}" FontWeight="SemiBold" TextWrapping="Wrap"/>"##, ok.len(), if ok.len() == 1 { "" } else { "s" }, self.size(bytes));
        if !why.is_empty() {
            x += &format!(r##"<TextBlock Text="{}" TextWrapping="Wrap" FontSize="{{$fs12}}" Foreground="{{$text1}}"/>"##, ui::esc(why));
        }
        for (c, p) in &ok {
            x += &format!(r##"<TextBlock Text="{}: {}   {}" FontSize="{{$fs12}}" Foreground="{{$text2}}" Margin="8,0,0,0" TextWrapping="Wrap"/>"##, ui::esc(&c.name), ui::esc(&p.title), self.size(self.part_true(r, p).0));
        }
        x += r##"<TextBlock Text="Each is marked in the list, where it can be ticked or turned down on its own." TextWrapping="Wrap" FontSize="{$fs12}" Foreground="{$text3}"/>"##;
        if left > 0 {
            x += &format!(r##"<TextBlock Text="{left} more it named can't be ticked here (not found, not safe enough, or nothing to clean); they are left out." TextWrapping="Wrap" FontSize="{{$fs12}}" Foreground="{{$text3}}"/>"##);
        }
        x += r##"<StackPanel Orientation="Horizontal" Spacing="6">"##;
        if !ok.is_empty() {
            x += r##"<Button x:Name="AdvClSugTick" Content="Tick them" ToolTipService.ToolTip="Adds these ticks. You still see the review before anything is deleted."/>"##;
        }
        x += r##"<Button x:Name="AdvClSugNo" Content="Not these" ToolTipService.ToolTip="Turns these down: the AI won't suggest them again."/></StackPanel></StackPanel></Border>"##;
        x
    }

    /// Dev: a suggestion of every part with something in it, as the AI would send it.
    pub(crate) fn suggest_demo(&mut self) {
        let Some(r) = self.advice() else { return };
        let keys: Vec<String> = self.cleaner_layout(r).iter().flat_map(|c| c.parts.iter()).filter(|p| p.bytes(r) > 0).map(|p| p.key.clone()).collect();
        self.advisor.cl.suggest = Some((keys, "These are caches and logs the apps make again by themselves; nothing you made is in them.".into()));
        self.render_advisor();
    }

    /// The user turned down suggested part `k` (or all of them): remembered, not offered again.
    pub(crate) fn suggest_decline(&mut self, k: Option<String>) {
        let Some((keys, _)) = self.advisor.cl.suggest.as_mut() else { return };
        let gone: Vec<String> = match &k {
            Some(k) => keys.iter().filter(|x| x.eq_ignore_ascii_case(k)).cloned().collect(),
            None => std::mem::take(keys),
        };
        keys.retain(|x| !gone.iter().any(|g| g.eq_ignore_ascii_case(x)));
        if keys.is_empty() {
            self.advisor.cl.suggest = None;
        }
        self.advisor.memory_ready();
        for g in gone {
            if !self.advisor.memory.declined_parts.iter().any(|d| d.eq_ignore_ascii_case(&g)) {
                self.advisor.memory.declined_parts.push(g);
            }
        }
        self.save_memory();
    }

    /// Suggested part `k` alone: its ticks added (the review still comes first).
    pub(crate) fn suggest_tick_one(&mut self, k: &str) {
        let Some(r) = self.advice() else { return };
        let rows = self.cleaner_layout(r);
        let keys: Vec<String> = self.suggested_parts(r, &rows).0.iter().filter(|(_, p)| p.key.eq_ignore_ascii_case(k)).flat_map(|(_, p)| p.tickable(r).map(|i| key(&r.findings[i].path))).collect();
        self.cleanup_tick_many(keys);
        if let Some((keys, _)) = self.advisor.cl.suggest.as_mut() {
            keys.retain(|x| !x.eq_ignore_ascii_case(k));
            if keys.is_empty() {
                self.advisor.cl.suggest = None;
            }
        }
    }

    /// A part's size as it really is: counted now (a preview, the review) where it was, the
    /// scan's figure elsewhere; and whether all of it was counted.
    pub(crate) fn part_true(&self, r: &Report, p: &PartRow) -> (u64, bool) {
        let mut all = true;
        let bytes = p
            .findings
            .iter()
            .filter(|&&i| r.counted[i])
            .map(|&i| match self.advisor.cleanup.measured.get(&key(&r.findings[i].path)) {
                Some(&m) => m,
                None => {
                    all &= !r.findings[i].estimate();
                    r.findings[i].bytes
                }
            })
            .sum();
        (bytes, all)
    }

    /// The findings of the parts the AI may suggest whose size is only an estimate: counted
    /// before a question goes, so the AI gets what a clean-up would really free.
    fn uncounted_for_ai(&self, r: &Report) -> Vec<(String, std::path::PathBuf, sprawl_advisor::clean::Action)> {
        self.cleaner_layout(r)
            .iter()
            .flat_map(|c| c.parts.iter())
            .filter(|p| p.class == Class::Space && !p.info && matches!(p.risk, Risk::Green | Risk::Amber))
            .flat_map(|p| p.findings.iter().copied())
            .filter(|&i| r.counted[i] && r.findings[i].estimate() && !self.advisor.cleanup.measured.contains_key(&key(&r.findings[i].path)) && !self.advisor.cl.ai_tried.contains(&key(&r.findings[i].path)))
            .filter_map(|i| sprawl_advisor::clean::action(&r.findings[i]).map(|a| (key(&r.findings[i].path), r.findings[i].path.clone(), a.0)))
            .collect()
    }

    /// Count the estimates first, then ask `q`: true when the count started (the question goes
    /// when it is done).
    pub(crate) fn ai_count_first(&mut self, q: &str) -> bool {
        if self.advisor.cl.ai_count.is_some() {
            return true;
        }
        let Some(r) = self.advice() else { return false };
        let work = self.uncounted_for_ai(r);
        if work.is_empty() {
            return false;
        }
        let spare = self.spare();
        // tried once: what can't be counted keeps the scan's figure, and the question still goes
        self.advisor.cl.ai_tried.extend(work.iter().map(|w| w.0.clone()));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let out: Vec<(String, u64)> = work.into_iter().filter_map(|(k, path, a)| sprawl_advisor::clean::preview(&path, &a, &spare, 0).map(|p| (k, p.bytes))).collect();
            let _ = tx.send(out);
        });
        self.advisor.cl.ai_count = Some((rx, q.to_string()));
        self.advisor.chat.status = Some("Counting what each part would really free, for the AI...".into());
        true
    }

    /// The count for the AI, when done: its numbers kept, and the waiting question asked.
    pub(crate) fn ai_count_poll(&mut self) {
        let Some((rx, _)) = &self.advisor.cl.ai_count else { return };
        let got = match rx.try_recv() {
            Ok(v) => v,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        let Some((_, q)) = self.advisor.cl.ai_count.take() else { return };
        for (k, b) in got {
            self.advisor.cleanup.measured.insert(k, b);
        }
        self.advisor.chat.status = None;
        self.chat_ask(q);
    }

    pub(crate) fn suggest_tick(&mut self) {
        let Some(r) = self.advice() else { return };
        let rows = self.cleaner_layout(r);
        let keys: Vec<String> = self.suggested_parts(r, &rows).0.iter().flat_map(|(_, p)| p.tickable(r).map(|k| key(&r.findings[k].path))).collect();
        self.cleanup_tick_many(keys);
        self.advisor.cl.suggest = None;
    }

    /// The cleaners as the AI is told about them: each part with what a clean-up would really
    /// free now (counted, not the scan's estimate), what it freed the last time, its risk and
    /// kind; and the parts the user turned down.
    pub(crate) fn cleaners_text(&self, r: &Report) -> String {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut s = String::new();
        for c in self.cleaner_layout(r).into_iter().filter(|c| c.presence == Presence::InScan) {
            let parts: Vec<&PartRow> = c.parts.iter().filter(|p| p.bytes(r) > 0).collect();
            if parts.is_empty() {
                continue;
            }
            let total: u64 = parts.iter().map(|p| self.part_true(r, p).0).sum();
            s += &format!("## {} ({})\n", c.name, crate::format::size(total));
            for p in parts {
                let (bytes, counted) = self.part_true(r, p);
                let ticked = p.findings.iter().any(|&i| self.advisor.cleanup.ticked.contains(&key(&r.findings[i].path)));
                let kind = if p.info {
                    " [tip: Sprawl doesn't clean it]".to_string()
                } else if p.class == Class::Privacy {
                    " [privacy: never suggest]".to_string()
                } else {
                    format!(" [{:?}]", p.risk)
                };
                s += &format!(
                    "- {}: {}{kind} [id: {}]{}",
                    p.title,
                    if counted { format!("{} would go now (counted)", crate::format::size(bytes)) } else { format!("up to {} (an estimate; may be much less)", crate::format::size(bytes)) },
                    p.key,
                    if ticked { " (ticked)" } else { "" }
                );
                if let Some((b, when)) = self.advisor.memory.freed.get(&p.key) {
                    s += &format!(", freed {} when cleaned {} days ago", crate::format::size(*b), now.saturating_sub(*when) / 86_400);
                }
                if !p.cost.is_empty() {
                    s += &format!(". If it goes: {}", p.cost);
                }
                s += "\n";
            }
        }
        if s.is_empty() {
            s = "No cleaner has anything here now.\n".into();
        }
        if !self.advisor.memory.declined_parts.is_empty() {
            s += &format!("\nThe user turned these down before; don't suggest them: {}\n", self.advisor.memory.declined_parts.join(", "));
        }
        s + "\nRead-only: the user ticks parts in Sprawl's Cleaners tab and presses Clean up. Suggest from what would go now, not from estimates: a part that frees nothing isn't worth ticking.\n"
    }

    /// The map lens for the part whose files are shown, while the Cleaners section is on.
    pub(crate) fn cleaner_preview(&self) -> Option<Arc<Vec<u8>>> {
        if self.view != crate::state::View::Advisor || !self.advisor.cleaners_on() || self.advisor.cl.show.is_none() {
            return None;
        }
        self.advisor.cl.lens.as_ref().filter(|(g, _)| *g == self.tree_gen).map(|(_, l)| l.clone())
    }
}

impl State {
    /// The Cleaners tab: the applications and their parts on the left, the part picked (or the
    /// preview) on the right. No map: the list takes the room.
    pub(crate) fn cleaners_tab_xaml(&self) -> String {
        let mut x = format!(r##"<Grid {} Padding="18,14,18,8" RowSpacing="10"><Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/></Grid.RowDefinitions>"##, ui::NS);
        let reading = self.advisor.cl.own_reading.as_ref().map(|(p, _)| p.items());
        let Some(r) = self.advice() else {
            let msg = match reading {
                Some(n) => format!("Reading the folders the cleaners look at... {} items", crate::format::thousands(n)),
                None if self.own_active() => match self.advisor.cl.own_failed {
                    Some(OwnFailed::NoFolders) => "None of the folders the cleaners look at is on this PC: there is nothing for them to clean.".into(),
                    Some(OwnFailed::Unreadable) => "The folders the cleaners look at couldn't be read. Press Check to try again.".into(),
                    None => "Press Check to read the folders the cleaners look at.".into(),
                },
                None if self.advisor.running => "Looking for what the cleaners can clean...".into(),
                None => self.advisor.note.unwrap_or("Scan a drive or folder, and the cleaners show what can go.").into(),
            };
            x += &format!(
                r##"<StackPanel Spacing="10"><StackPanel Orientation="Horizontal" Spacing="10">{}<TextBlock x:Name="ClReading" Text="{}" {} TextWrapping="Wrap" Foreground="{{$text2}}" FontSize="{{$fs13}}" VerticalAlignment="Center"/></StackPanel>{}</StackPanel></Grid>"##,
                if reading.is_some() || self.advisor.running { r##"<ProgressRing IsActive="True" Width="18" Height="18"/>"## } else { "" },
                ui::esc(&msg),
                crate::look::NUM,
                if reading.is_none() && !self.advisor.running { r##"<Button x:Name="AdvRefresh" Content="Check"/>"## } else { "" }
            );
            return x;
        };
        let rows = self.cleaner_layout(r);
        let total: u64 = rows.iter().flat_map(|c| c.parts.iter()).filter(|p| !p.info).map(|p| self.part_true(r, p).0).sum();
        let apps = rows.iter().filter(|c| c.parts.iter().any(|p| p.bytes(r) > 0)).count();
        // where the list comes from: this scan, or the tab's own read just now
        let from = match self.advisor.cl.own.as_ref().filter(|_| self.own_active()) {
            Some(o) => {
                let mins = o.began.elapsed().map_or(0, |d| d.as_secs() / 60);
                let when = match mins {
                    0 => "just now".to_string(),
                    1 => "a minute ago".to_string(),
                    m if m < 120 => format!("{m} minutes ago"),
                    m => format!("{} hours ago", m / 60),
                };
                let out = self.cleaner_roots_all().1;
                // the cleaners look at this account's and Windows' folders, wherever the scan is
                let mut s = format!("Checked {when}: the {} folders the cleaners look at (this account's and Windows' own, whatever drive is scanned), read for this tab", o.roots);
                if self.job.is_some() {
                    s += ". A scan is running: this list stays until it ends";
                }
                if !o.unread.is_empty() {
                    s += &format!(". {}", unread_line(&o.unread));
                }
                if !out.is_empty() {
                    let names: Vec<String> = out.iter().map(|p| p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())).collect();
                    s += &format!(". Left out of this quick read, as they are large: {}; scan {} for them", names.join(" and "), std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()));
                }
                s
            }
            None => match self.tree.as_ref() {
                Some(t) => format!("From this scan of {}", t.root_path.display()),
                None => String::new(),
            },
        };
        x += &format!(
            r##"<Grid ColumnSpacing="12"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><StackPanel><TextBlock {} FontSize="{{$fs15}}" TextWrapping="Wrap"><Run Text="{}" FontWeight="SemiBold"/><Run Text="{}" Foreground="{{$text2}}"/></TextBlock><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" TextWrapping="Wrap"/></StackPanel><StackPanel Grid.Column="1" Orientation="Horizontal" Spacing="8" VerticalAlignment="Center">{}<Button x:Name="AdvRefresh" Content="Check again" IsEnabled="{}"/></StackPanel></Grid>"##,
            crate::look::NUM,
            if total > 0 { self.size(total) } else { "Nothing to clean".into() },
            if total > 0 { format!(" in {apps} {}", if apps == 1 { "application" } else { "applications" }) } else { String::new() },
            ui::esc(&from),
            if reading.is_some() { r##"<ProgressRing IsActive="True" Width="16" Height="16"/>"## } else { "" },
            if reading.is_some() || self.advisor.cleanup.running.is_some() { "False" } else { "True" },
        );
        // the list, the details and the AI side by side, each scrolling on its own: an answer
        // never pushes the list or the details away
        let ai = self.cl_ai_open();
        x += &format!(
            r##"<Grid Grid.Row="1" ColumnSpacing="16"><Grid.ColumnDefinitions><ColumnDefinition Width="*" MinWidth="360"/><ColumnDefinition Width="340"/><ColumnDefinition Width="{}"/></Grid.ColumnDefinitions>"##,
            if ai { "380" } else { "Auto" }
        );
        x += r##"<ScrollViewer x:Name="ClScroll" HorizontalScrollBarVisibility="Disabled" Padding="0,0,12,0"><StackPanel Spacing="4">"##;
        x += &self.cleaners_tab_list(r, &rows);
        x += "</StackPanel></ScrollViewer>";
        x += r##"<ScrollViewer x:Name="ClSide" Grid.Column="1" HorizontalScrollBarVisibility="Disabled" Padding="0,0,10,0"><StackPanel Spacing="10">"##;
        x += &self.cleaners_tab_side(r, &rows);
        x += "</StackPanel></ScrollViewer>";
        if ai {
            x += r##"<Grid Grid.Column="2" RowSpacing="6"><Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/></Grid.RowDefinitions><Button x:Name="AdvClAiFold" HorizontalAlignment="Right" Padding="8,1" FontSize="{$fs12}" CornerRadius="{$rc4}" Style="{StaticResource SprawlSubtle}" Content="Hide the AI" ToolTipService.ToolTip="Fold the AI's column away"/><ScrollViewer x:Name="ClAiScroll" Grid.Row="1" HorizontalScrollBarVisibility="Disabled" Padding="0,0,10,0"><StackPanel Spacing="10">"##;
            x += &self.ai_column(r);
            x += "</StackPanel></ScrollViewer></Grid>";
        } else {
            x += r##"<Button x:Name="AdvClAiFold" Grid.Column="2" VerticalAlignment="Top" Content="AI" ToolTipService.ToolTip="Show the AI's column"/>"##;
        }
        x += "</Grid></Grid>";
        x
    }

    /// The AI's column on the Cleaners tab is open (the user's choice, kept).
    pub(crate) fn cl_ai_open(&self) -> bool {
        self.settings.bool("clAiOpen").unwrap_or(true)
    }

    /// The applications, each with a tick for all its parts, and its parts under it when open.
    fn cleaners_tab_list(&self, r: &Report, rows: &[ClRow]) -> String {
        let mut x = String::new();
        if !self.usual_keys(r).is_empty() {
            x += r##"<StackPanel Orientation="Horizontal" Spacing="8"><Button x:Name="AdvClUsual" Content="Tick my usual ones" ToolTipService.ToolTip="The parts that take time to get back which you cleaned last time"/></StackPanel>"##;
        }
        if rows.is_empty() {
            x += r##"<TextBlock Text="No application Sprawl knows is here." TextWrapping="Wrap" Foreground="{$text2}"/>"##;
            x += &self.privacy_group(r);
            return x;
        }
        let running = self.advisor.cleanup.running.is_some();
        let ticked = |i: usize| self.advisor.cleanup.ticked.contains(&key(&r.findings[i].path));
        let mut part_no = 0usize;
        let mut last_group = "";
        for (ci, c) in rows.iter().enumerate() {
            if c.group != last_group {
                last_group = c.group;
                x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" FontWeight="SemiBold" Foreground="{{$text2}}" Margin="0,8,0,0"/>"##, ui::esc(c.group));
            }
            let open = self.advisor.cl.open.contains(&c.id);
            let bytes: u64 = c.parts.iter().filter(|p| !p.info).map(|p| self.part_true(r, p).0).sum();
            let app_tickable: Vec<usize> = c.parts.iter().filter(|p| app_tick_takes(p)).flat_map(|p| p.tickable(r)).filter(|&i| box_takes(r, i)).collect();
            let n_ticked = c.parts.iter().filter(|p| p.findings.iter().any(|&i| ticked(i))).count();
            let community = sprawl_advisor::community::is_community(&c.id);
            let turned_on = community && self.community_on(r, c);
            let sub = match &c.presence {
                _ if community && !turned_on => format!("From the community, not used yet · {}", if c.id.starts_with(sprawl_advisor::community::WINAPP2) { "winapp2.ini" } else { "BleachBit" }),
                Presence::Elsewhere(_) => "Not in what was read".to_string(),
                _ if bytes == 0 => "Nothing to clean now".to_string(),
                _ => {
                    let n = c.parts.iter().filter(|p| p.bytes(r) > 0).count();
                    let mut s = format!("{n} {}", if n == 1 { "part" } else { "parts" });
                    if n_ticked > 0 {
                        s += &format!(" · {n_ticked} ticked");
                    }
                    s
                }
            };
            // the app's tick: its safe and amber space parts (privacy parts are ticked one by one)
            let state = if app_tickable.is_empty() {
                None
            } else if app_tickable.iter().all(|&i| ticked(i)) {
                Some("True")
            } else if app_tickable.iter().any(|&i| ticked(i)) || n_ticked > 0 {
                Some("{x:Null}")
            } else {
                Some("False")
            };
            let tick = match state {
                Some(s) => format!(
                    r##"<CheckBox x:Name="AdvClA{ci}" MinWidth="0" Padding="0" VerticalAlignment="Center" IsThreeState="False" IsChecked="{s}" IsEnabled="{}" AutomationProperties.Name="Clean up {}" ToolTipService.ToolTip="Ticks its caches and logs. Privacy parts (sign-ins, history) are ticked one by one."/>"##,
                    if running { "False" } else { "True" },
                    ui::esc(&c.name)
                ),
                None => String::new(),
            };
            let side = match &c.presence {
                _ if community => format!(
                    r##"<Button x:Name="AdvClUse{ci}" Grid.Column="2" VerticalAlignment="Center" Content="{}" ToolTipService.ToolTip="A community cleaner does nothing until you turn it on. Read its parts first."/>"##,
                    if turned_on { "Stop using" } else { "Use this cleaner" }
                ),
                Presence::Elsewhere(drive) => format!(
                    r##"<Button x:Name="AdvClScan{ci}" Grid.Column="2" VerticalAlignment="Center" Content="Scan {}"/>"##,
                    ui::esc(drive.display().to_string().trim_end_matches('\\'))
                ),
                _ => String::new(),
            };
            x += &format!(
                r##"<Grid ColumnSpacing="6"><Grid.ColumnDefinitions><ColumnDefinition Width="34"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>{tick}<Border Grid.Column="1" CornerRadius="{{$rc7}}" Background="{bg}"/><Button x:Name="AdvClO{ci}" Grid.Column="1" HorizontalAlignment="Stretch" HorizontalContentAlignment="Stretch" Padding="10,6" CornerRadius="{{$rc7}}" BorderThickness="0" Background="Transparent" AutomationProperties.Name="{name}">
<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="14"/><ColumnDefinition Width="*"/><ColumnDefinition Width="90"/></Grid.ColumnDefinitions>
<FontIcon Glyph="{chev}" FontSize="{{$fs10}}" Foreground="{{$text3}}" VerticalAlignment="Center"/>
<StackPanel Grid.Column="1" Spacing="1"><TextBlock Text="{name}" FontSize="{{$fs13}}" TextTrimming="CharacterEllipsis"/><TextBlock Text="{sub}" FontSize="{{$fs12}}" Foreground="{{$text3}}" TextTrimming="CharacterEllipsis"/></StackPanel>
<TextBlock Grid.Column="2" Text="{size}" FontSize="{{$fs13}}" HorizontalAlignment="Right" VerticalAlignment="Center" Opacity="{op}"/>
</Grid></Button>{side}</Grid>"##,
                bg = if open { crate::tokens::refer("rowopen") } else { crate::tokens::refer("surface2") },
                name = ui::esc(&c.name),
                chev = if open { "&#xE70D;" } else { "&#xE76C;" },
                sub = ui::esc(&sub),
                size = if bytes > 0 { self.size(bytes) } else { String::new() },
                op = if bytes > 0 { "1" } else { "0.5" },
            );
            if open {
                x += r##"<StackPanel Spacing="1" Margin="40,0,0,4">"##;
                for p in &c.parts {
                    x += &self.tab_part_row(r, c, p, part_no, running);
                    part_no += 1;
                }
                x += "</StackPanel>";
            } else {
                part_no += c.parts.len();
            }
        }
        x += &self.privacy_group(r);
        x += &self.unclaimed_card(r);
        x += &self.cleaner_proposals_card();
        x += &self.community_card();
        x
    }

    /// One part in the list: its tick, and a button that shows it in the details pane. A part
    /// with a real cost says so while it is ticked.
    fn tab_part_row(&self, r: &Report, c: &ClRow, p: &PartRow, n: usize, running: bool) -> String {
        let bytes = p.bytes(r);
        let tickable: Vec<usize> = p.tickable(r).collect();
        let all_ticked = !tickable.is_empty() && tickable.iter().all(|&i| self.advisor.cleanup.ticked.contains(&key(&r.findings[i].path)));
        let why_not = p.findings.first().and_then(|&i| untickable(&r.findings[i], r.counted[i])).filter(|w| !w.is_empty());
        let tick = if !tickable.is_empty() {
            format!(
                r##"<CheckBox x:Name="AdvClT{n}" MinWidth="0" Padding="0" VerticalAlignment="Center" IsChecked="{}" IsEnabled="{}" AutomationProperties.Name="Clean up {}"/>"##,
                if all_ticked { "True" } else { "False" },
                if running { "False" } else { "True" },
                ui::esc(&format!("{}: {}", c.name, p.title))
            )
        } else if let (Some(w), true) = (why_not, bytes > 0) {
            format!(r##"<FontIcon Glyph="&#xE72E;" FontSize="{{$fs13}}" Foreground="{{$label}}" VerticalAlignment="Center" ToolTipService.ToolTip="Can't be ticked: {0}" AutomationProperties.Name="Can't be ticked: {0}"/>"##, ui::esc(w))
        } else {
            String::new()
        };
        let mut sub = if p.info {
            "Tip".to_string()
        } else {
            match p.class {
                Class::Privacy => "Privacy".to_string(),
                Class::Space => crate::advisor::risk_label(p.risk).to_string(),
            }
        };
        if self.part_left_out(r, c, p) {
            sub += &format!(" · not in the quick read: size shown after a scan of {}", sys_drive());
        }
        let open = p.findings.first().and_then(|&i| self.app_open(&r.findings[i]));
        match &open {
            Some((app, Closed::Must)) => sub += &format!(" · close {app} first"),
            Some((app, Closed::Warn)) => sub += &format!(" · {app} is open"),
            _ => {}
        }
        let cost = if all_ticked && !p.cost.is_empty() && p.risk >= Risk::Amber {
            format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$warn}}" TextWrapping="Wrap"/>"##, ui::esc(&format!("Ticked: {}", p.cost)))
        } else {
            String::new()
        };
        let sel = self.advisor.cl.sel.as_deref() == Some(p.key.as_str());
        // the AI's suggestion, on the row: its reason, Tick and Not this
        let sug = if self.part_suggested(r, p) {
            format!(
                r##"<Border Grid.Column="1" Grid.Row="1" Margin="8,0,0,4" Padding="8,4" CornerRadius="{{$rc5}}" Background="{{$accentwash}}"><StackPanel Spacing="4"><TextBlock TextWrapping="Wrap" FontSize="{{$fs12}}"><Run Text="AI suggests: " FontWeight="SemiBold" Foreground="{{$link}}"/><Run Text="{}" Foreground="{{$text1}}"/></TextBlock><StackPanel Orientation="Horizontal" Spacing="6"><Button x:Name="AdvClSgT{n}" Content="Tick" Padding="8,1" FontSize="{{$fs12}}" CornerRadius="{{$rc4}}" IsEnabled="{}"/><Button x:Name="AdvClSgX{n}" Content="Not this" Padding="8,1" FontSize="{{$fs12}}" CornerRadius="{{$rc4}}" ToolTipService.ToolTip="Turns it down: the AI won't suggest it again."/></StackPanel></StackPanel></Border>"##,
                ui::esc(&self.sug_reason(p)),
                if running || all_ticked { "False" } else { "True" }
            )
        } else {
            String::new()
        };
        let (true_bytes, counted) = self.part_true(r, p);
        format!(
            r##"<Grid ColumnSpacing="6" Opacity="{op}"><Grid.ColumnDefinitions><ColumnDefinition Width="30"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>{tick}<Button x:Name="AdvClP{n}" Grid.Column="1" {bg} HorizontalAlignment="Stretch" HorizontalContentAlignment="Stretch" Padding="8,4" CornerRadius="{{$rc6}}" AutomationProperties.Name="{title}: details">
<Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="10"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>{dot}<StackPanel Grid.Column="1" Spacing="1"><TextBlock Text="{title}" FontSize="{{$fs13}}" TextTrimming="CharacterEllipsis"/><TextBlock Text="{sub}" FontSize="{{$fs12}}" Foreground="{{$text3}}" TextTrimming="CharacterEllipsis"/>{cost}</StackPanel><TextBlock Grid.Column="2" Text="{size}" FontSize="{{$fs13}}" VerticalAlignment="Center"/></Grid></Button>{sug}</Grid>"##,
            op = if bytes > 0 { "1" } else { "0.55" },
            bg = crate::look::pick(crate::look::Pick::Tab, sel),
            dot = if p.info { crate::advisor::tip_mark(8, r#"VerticalAlignment="Top" Margin="0,6,0,0""#) } else { crate::advisor::risk_mark(p.risk, 8, r#"VerticalAlignment="Top" Margin="0,6,0,0""#) },
            title = ui::esc(&p.title),
            sub = ui::esc(&sub),
            size = match (bytes, counted) {
                (0, _) => "-".into(),
                (_, false) => format!("up to {}", self.size(true_bytes)),
                (_, true) => self.size(true_bytes),
            },
        )
    }

    /// Part `p` is in the AI's suggestion (and may be suggested).
    fn part_suggested(&self, r: &Report, p: &PartRow) -> bool {
        self.advisor.cl.suggest.as_ref().is_some_and(|(keys, _)| keys.iter().any(|k| k.eq_ignore_ascii_case(&p.key)))
            && !self.advisor.memory.declined_parts.iter().any(|d| d.eq_ignore_ascii_case(&p.key))
            && p.class == Class::Space
            && !p.info
            && matches!(p.risk, Risk::Green | Risk::Amber)
            && p.tickable(r).next().is_some()
            // a part that frees nothing now isn't offered, whatever the AI said
            && self.part_true(r, p).0 > 0
    }

    /// Why the AI suggests part `p`: its own reason, else the suggestion's.
    fn sug_reason(&self, p: &PartRow) -> String {
        self.advisor.cl.sug_reasons.get(&p.key.to_lowercase()).cloned().or_else(|| self.advisor.cl.suggest.as_ref().map(|(_, why)| why.clone())).filter(|w| !w.is_empty()).unwrap_or_else(|| "no reason given".into())
    }

    /// Part `p` looks only in folders the tab's quick read left out (the component store,
    /// Windows.old): its size is known after a scan of the drive.
    fn part_left_out(&self, r: &Report, c: &ClRow, p: &PartRow) -> bool {
        if !p.findings.is_empty() || !self.own_active() {
            return false;
        }
        let out = self.cleaner_roots_all().1;
        let Some(d) = c.def.map(|k| &r.catalogue[k]) else { return false };
        d.parts.iter().find(|x| x.rule == p.key).is_some_and(|x| !x.paths.is_empty() && x.paths.iter().all(|path| sprawl_advisor::cleaners::left_out(path, &out)))
    }


    /// The right of the tab: the preview when there is one, else the part picked; then the AI.
    fn cleaners_tab_side(&self, r: &Report, rows: &[ClRow]) -> String {
        let mut x = String::new();
        if let Some(pv) = &self.advisor.cl.preview {
            x += &self.preview_xaml(pv);
        } else {
            let found = rows.iter().flat_map(|c| c.parts.iter().map(move |p| (c, p))).enumerate().find(|(_, (_, p))| self.advisor.cl.sel.as_deref() == Some(p.key.as_str()));
            match found {
                Some((n, (c, p))) => x += &self.part_details(r, c, p, n),
                None => {
                    x += r##"<Border Padding="14,12" CornerRadius="{$rc8}" Background="{$surface1}"><TextBlock Text="Pick a part to see what it is, what cleaning it costs, where it is and whether its application is open. Nothing goes before you review it and confirm." TextWrapping="Wrap" FontSize="{$fs12}" Foreground="{$text2}"/></Border>"##
                }
            }
        }
        x
    }

    /// The AI's own column: with AI off, one Set up AI button and nothing else; else the
    /// suggestion, the chat about cleaning, its status and Stop.
    fn ai_column(&self, r: &Report) -> String {
        let cfg = AiConfig::load();
        if cfg.provider == Provider::Off {
            return r##"<Border Padding="14,12" CornerRadius="{$rc8}" Background="{$surface1}"><StackPanel Spacing="8"><TextBlock Text="AI" FontSize="{$fs13}" FontWeight="SemiBold"/><Button x:Name="AdvClSetup" Content="Set up AI"/></StackPanel></Border>"##.into();
        }
        let mut x = self.suggest_card(r);
        x += &self.chat_card();
        x
    }

    /// A part in full: what it is, its cost, where it is, the application being open, the
    /// days it keeps, and its biggest files.
    fn part_details(&self, r: &Report, c: &ClRow, p: &PartRow, n: usize) -> String {
        let bytes = p.bytes(r);
        let label = |t: &str| format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" FontWeight="SemiBold" Foreground="{{$text2}}" Margin="0,4,0,0"/>"##, ui::esc(t));
        let body = |t: &str| format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text1}}" TextWrapping="Wrap" IsTextSelectionEnabled="True"/>"##, ui::esc(t));
        let mut x = String::from(r##"<Border Padding="14,12" CornerRadius="{$rc8}" Background="{$surface3}" BorderBrush="{$line2}" BorderThickness="1"><StackPanel Spacing="3">"##);
        x += &format!(
            r##"<Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><StackPanel><TextBlock Text="{}" FontSize="{{$fs15}}" FontWeight="SemiBold" TextWrapping="Wrap"/><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}"/></StackPanel><TextBlock Grid.Column="1" Text="{}" FontSize="{{$fs15}}" FontWeight="SemiBold"/></Grid>"##,
            ui::esc(&p.title),
            ui::esc(&format!("{} · {}", c.name, if p.info { "Tip" } else if p.class == Class::Privacy { "Privacy" } else { crate::advisor::risk_label(p.risk) })),
            match self.part_true(r, p) {
                _ if bytes == 0 => "-".into(),
                (t, false) => format!("up to {}", self.size(t)),
                (t, true) => self.size(t),
            }
        );
        if self.part_suggested(r, p) {
            x += &format!(r##"<TextBlock TextWrapping="Wrap" FontSize="{{$fs12}}" Margin="0,4,0,0"><Run Text="AI suggests: " FontWeight="SemiBold" Foreground="{{$link}}"/><Run Text="{}" Foreground="{{$text1}}"/></TextBlock>"##, ui::esc(&self.sug_reason(p)));
        }
        x += &label("What it is");
        x += &body(if p.why.is_empty() { "No description." } else { &p.why });
        x += &label("If it goes");
        x += &body(if !p.cost.is_empty() {
            &p.cost
        } else if p.info {
            "Sprawl doesn't clean this itself: it is a tip."
        } else if p.risk == Risk::Green {
            "Nothing you'd notice: the application makes it again when it needs it."
        } else {
            // never a reassurance that wasn't written for it
            "Its definition doesn't say. Read what it is above before ticking it."
        });
        x += &label("Where");
        if self.part_left_out(r, c, p) {
            x += &body(&format!("In a large Windows folder the quick read leaves out. Its size, and the way to clean it, show after a scan of {}.", sys_drive()));
            x += &format!(r##"<Button x:Name="AdvClScanSys" Content="{}"/>"##, ui::esc(&format!("Scan {}", sys_drive())));
        } else if p.findings.is_empty() {
            x += &body("Nothing of it here now.");
        }
        for &i in p.findings.iter().take(8) {
            x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" FontFamily="Consolas" Foreground="{{$text2}}" TextWrapping="Wrap" IsTextSelectionEnabled="True"/>"##, ui::esc(&r.findings[i].path.display().to_string()));
        }
        if p.findings.len() > 8 {
            x += &body(&format!("and {} more places", p.findings.len() - 8));
        }
        // the application being open
        let d = c.def.map(|k| &r.catalogue[k]);
        let open = p.findings.first().and_then(|&i| self.app_open(&r.findings[i]));
        x += &label("In use");
        x += &match (&open, d) {
            (Some((app, Closed::Must)), _) => format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$warn}}" TextWrapping="Wrap"/>"##, ui::esc(&format!("{app} is open, or that can't be told: close it first. This can't be cleaned while it runs."))),
            (Some((app, _)), _) => format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$warnquiet}}" TextWrapping="Wrap"/>"##, ui::esc(&format!("{app} is open: files it is using are skipped."))),
            (None, Some(d)) if !d.running.is_empty() && p.closed != Closed::No => body(&format!("{} isn't open.", d.name)),
            _ => body("Can be cleaned while the application runs; files in use are skipped."),
        };
        // keep the newest
        if p.clears && !p.info {
            let days = self.part_days(p);
            x += &label("Keeps");
            x += &format!(
                r##"<StackPanel Orientation="Horizontal" Spacing="8"><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text1}}" VerticalAlignment="Center"/><Button x:Name="AdvClK{n}" Content="Keep more days" Padding="8,1" FontSize="{{$fs12}}" CornerRadius="{{$rc4}}" ToolTipService.ToolTip="What changed lately stays. Press to keep more days."/></StackPanel>"##,
                ui::esc(&if days == 0 { "Everything in it goes.".to_string() } else { format!("What changed in the last {days} days stays.") })
            );
        }
        if bytes > 0 {
            x += &format!(
                r##"<StackPanel Orientation="Horizontal" Spacing="6" Margin="0,6,0,0"><Button x:Name="AdvClS{n}" Content="{}"/>{}</StackPanel>"##,
                if self.advisor.cl.show.as_deref() == Some(p.key.as_str()) { "Hide the biggest files" } else { "Show the biggest files" },
                // the "?" asks the AI about this part; its answer shows here
                if AiConfig::load().provider != Provider::Off {
                    format!(r##"<Button x:Name="AdvClQ{n}" Content="?" ToolTipService.ToolTip="What happens if I clean this? (asks the AI)" AutomationProperties.Name="What happens if I clean this?"/>"##)
                } else {
                    String::new()
                }
            );
            if self.advisor.cl.show.as_deref() == Some(p.key.as_str()) {
                x += &self.part_files(r, p);
            }
        }
        if self.advisor.cl.part_q.as_deref() == Some(p.key.as_str()) {
            x += &label("The AI says");
            x += r##"<StackPanel Orientation="Horizontal" Spacing="8"><ProgressRing IsActive="True" Width="14" Height="14"/><TextBlock Text="Asking..." FontSize="{$fs12}" Foreground="{$text2}"/></StackPanel>"##;
        } else if let Some(a) = self.advisor.cl.part_answers.get(&p.key) {
            x += &label("The AI says");
            x += &ui::markdown(a, 12, &crate::tokens::refer("text1"));
            x += crate::advisor::UNCHECKED_LINE;
        }
        if let Some(k) = c.def {
            let d = &r.catalogue[k];
            let showing = self.advisor.cl.def.contains(&c.id);
            let ci = self.cleaner_layout(r).iter().position(|o| o.id == c.id).unwrap_or(0);
            let forget = if d.builtin || sprawl_advisor::community::is_community(&d.id) {
                String::new()
            } else {
                format!(r##"<Button x:Name="AdvClF{ci}" Content="Forget this cleaner" Padding="8,1" FontSize="{{$fs12}}" CornerRadius="{{$rc4}}" Style="{{StaticResource SprawlSubtle}}" ToolTipService.ToolTip="Remove it from your rules (nothing on disk changes)"/>"##)
            };
            x += &format!(
                r##"<StackPanel Orientation="Horizontal" Spacing="8" Margin="0,6,0,0"><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" VerticalAlignment="Center"/><Button x:Name="AdvClDef{ci}" Content="{}" Padding="8,1" FontSize="{{$fs12}}" CornerRadius="{{$rc4}}" Style="{{StaticResource SprawlSubtle}}"/>{forget}</StackPanel>"##,
                ui::esc(&format!("From {}", d.pack)),
                if showing { "Hide the definition" } else { "Show the definition" }
            );
            if showing {
                x += &format!(
                    r##"<Border Padding="10,8" CornerRadius="{{$rc5}}" Background="{{$shade}}"><TextBlock FontFamily="Consolas" FontSize="{{$fs12}}" TextWrapping="Wrap" IsTextSelectionEnabled="True" Text="{}"/></Border>"##,
                    d.text.lines().map(ui::esc).collect::<Vec<_>>().join("&#10;")
                );
            }
        }
        x += "</StackPanel></Border>";
        x
    }

    /// Preview: the exact files the ticks would remove, counted by the clean-up's own tests.
    fn preview_xaml(&self, pv: &PreviewRun) -> String {
        let mut x = String::from(r##"<Border Padding="14,12" CornerRadius="{$rc8}" Background="{$accentwash}" BorderBrush="{$accentline}" BorderThickness="1"><StackPanel Spacing="4">"##);
        match pv {
            PreviewRun::Counting(_) => {
                x += r##"<StackPanel Orientation="Horizontal" Spacing="10"><ProgressRing IsActive="True" Width="18" Height="18"/><TextBlock Text="Counting what would go..." FontSize="{$fs14}" FontWeight="SemiBold" VerticalAlignment="Center"/></StackPanel><TextBlock Text="Nothing is deleted: this only looks." FontSize="{$fs12}" Foreground="{$text2}"/>"##;
            }
            PreviewRun::Done(items, keys) => {
                let bytes: u64 = items.iter().filter_map(|it| it.got.as_ref()).map(|g| g.bytes).sum();
                let files: u64 = items.iter().filter_map(|it| it.got.as_ref()).map(|g| g.files).sum();
                x += &format!(
                    r##"<Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBlock Text="{}" FontSize="{{$fs15}}" FontWeight="SemiBold" TextWrapping="Wrap"/><Button x:Name="AdvClPrevX" Grid.Column="1" Content="Close"/></Grid>"##,
                    ui::esc(&format!("{} {} would go, {}", crate::format::thousands(files), if files == 1 { "file" } else { "files" }, self.size(bytes)))
                );
                x += r##"<TextBlock Text="Counted now by the clean-up's own tests: what is protected, kept by you, or new since the read stays. Nothing was deleted." FontSize="{$fs12}" Foreground="{$text2}" TextWrapping="Wrap"/>"##;
                let mut now: Vec<String> = self.ticked_findings().into_iter().map(|(_, f)| key(&f.path)).collect();
                now.sort();
                if now != *keys {
                    x += r##"<TextBlock Text="The ticks changed since: preview again for these numbers." FontSize="{$fs12}" Foreground="{$warn}" TextWrapping="Wrap"/>"##;
                }
                for it in items {
                    let what = match &it.got {
                        Some(g) => format!("{} files, {}", crate::format::thousands(g.files), self.size(g.bytes)),
                        None => "its program decides".into(),
                    };
                    x += &format!(
                        r##"<Grid ColumnSpacing="8" Margin="0,2,0,0"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBlock Text="{}" FontSize="{{$fs12}}" TextTrimming="CharacterEllipsis"/><TextBlock Grid.Column="1" Text="{}" FontSize="{{$fs12}}" Foreground="{{$text2}}"/></Grid>"##,
                        ui::esc(&it.title),
                        ui::esc(&what)
                    );
                }
                let mut biggest: Vec<&(std::path::PathBuf, u64)> = items.iter().filter_map(|it| it.got.as_ref()).flat_map(|g| g.biggest.iter()).collect();
                biggest.sort_by_key(|(_, b)| std::cmp::Reverse(*b));
                if !biggest.is_empty() {
                    x += r##"<TextBlock Text="The biggest of them:" FontSize="{$fs12}" FontWeight="SemiBold" Foreground="{$text2}" Margin="0,6,0,0"/>"##;
                }
                for (p, b) in biggest.into_iter().take(PREVIEW_FILES) {
                    x += &format!(
                        r##"<Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><TextBlock Text="{}" FontSize="{{$fs11}}" FontFamily="Consolas" Foreground="{{$text2}}" TextTrimming="CharacterEllipsis" IsTextSelectionEnabled="True" ToolTipService.ToolTip="{0}"/><TextBlock Grid.Column="1" Text="{}" FontSize="{{$fs11}}" Foreground="{{$text2}}"/></Grid>"##,
                        ui::esc(&p.display().to_string()),
                        self.size(*b)
                    );
                }
            }
        }
        x += "</StackPanel></Border>";
        x
    }

    /// Preview the ticks: count, off the UI thread, what the run would remove from each.
    pub(crate) fn preview_start(&mut self) {
        let picked = self.ticked_findings();
        if picked.is_empty() {
            return;
        }
        let spare = self.spare();
        let work: Vec<(String, String, std::path::PathBuf, Option<sprawl_advisor::clean::Action>)> =
            picked.into_iter().map(|(_, f)| (f.title.clone(), key(&f.path), f.path.clone(), sprawl_advisor::clean::action(&f).map(|a| a.0))).collect();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let items = work
                .into_iter()
                .map(|(title, key, path, action)| {
                    let got = action.and_then(|a| sprawl_advisor::clean::preview(&path, &a, &spare, PREVIEW_FILES));
                    PrevItem { title, key, got }
                })
                .collect();
            let _ = tx.send(items);
        });
        self.advisor.cl.preview = Some(PreviewRun::Counting(rx));
    }

    /// The preview, when counted: its numbers are the review's too.
    pub(crate) fn preview_poll(&mut self) {
        let Some(PreviewRun::Counting(rx)) = &self.advisor.cl.preview else { return };
        let items = match rx.try_recv() {
            Ok(v) => v,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        for it in &items {
            if let Some(g) = &it.got {
                self.advisor.cleanup.measured.insert(it.key.clone(), g.bytes);
            }
        }
        let mut keys: Vec<String> = items.iter().map(|it| it.key.clone()).collect();
        keys.sort();
        self.advisor.cl.preview = Some(PreviewRun::Done(items, keys));
        if self.view == crate::state::View::Advisor {
            self.render_advisor();
        }
    }

    /// The Cleaners tab's ticks: when the tab opens, the user's own from last time are put
    /// back; after the user changes a tick, they are remembered. Never what Sprawl ticked by
    /// itself, and a clean-up clearing its ticks doesn't count as a change.
    pub(crate) fn cleaner_ticks_sync(&mut self) {
        if !self.advisor.cleaners_tab_on() || self.advisor.cleanup.running.is_some() || self.advice().is_none() {
            return;
        }
        self.advisor.memory_ready();
        let restore = std::mem::take(&mut self.advisor.cl.restore_pending);
        let changed = std::mem::take(&mut self.advisor.cl.user_ticked);
        // Settings > Cleaning, "Open Cleaners with my usual ticks": off puts none back
        if restore && self.settings.bool("clUsualTicks") == Some(false) {
            return;
        }
        if restore {
            let keys: Vec<String> = {
                let r = self.advice().unwrap();
                let want = &self.advisor.memory.cleaner_ticks;
                self.cleaner_layout(r).iter().flat_map(|c| c.parts.iter()).filter(|p| want.contains(&p.key)
                    && sprawl_advisor::cleaners::selection_fingerprint(&r.catalogue, &p.key).is_some_and(|fp| self.advisor.memory.cleaner_tick_definitions.get(&p.key) == Some(&fp)))
                    .flat_map(|p| p.tickable(r).filter(|&i| auto_takes(r, i)).map(|i| key(&r.findings[i].path))).collect()
            };
            self.advisor.cleanup.ticked.extend(keys);
            self.advisor.cleanup.own_ok = false;
            return;
        }
        if !changed {
            return;
        }
        let r = self.advice().unwrap();
        let rows = self.cleaner_layout(r);
        // the parts of this report as ticked now; remembered parts not here stay remembered
        let ticked = &self.advisor.cleanup.ticked;
        let here: HashSet<&str> = rows.iter().flat_map(|c| c.parts.iter()).filter(|p| p.tickable(r).next().is_some()).map(|p| p.key.as_str()).collect();
        let mut now: Vec<String> = self.advisor.memory.cleaner_ticks.iter().filter(|k| !here.contains(k.as_str())).cloned().collect();
        now.extend(
            rows.iter()
                .flat_map(|c| c.parts.iter())
                .filter(|p| {
                    let t: Vec<usize> = p.tickable(r).collect();
                    !t.is_empty() && t.iter().all(|&i| ticked.contains(&key(&r.findings[i].path)))
                })
                .map(|p| p.key.clone()),
        );
        now.sort();
        now.dedup();
        // Only explicit choices made for definitions present now receive a new binding.
        let mut definitions = self.advisor.memory.cleaner_tick_definitions.clone();
        definitions.retain(|id, _| now.contains(id));
        for id in now.iter().filter(|id| here.contains(id.as_str())) {
            if let Some(fp) = sprawl_advisor::cleaners::selection_fingerprint(&r.catalogue, id) { definitions.insert(id.clone(), fp); }
        }
        if now != self.advisor.memory.cleaner_ticks || definitions != self.advisor.memory.cleaner_tick_definitions {
            self.advisor.memory.cleaner_ticks = now;
            self.advisor.memory.cleaner_tick_definitions = definitions;
            self.save_memory();
        }
    }
}

impl State {
    /// The findings Sprawl can clean now that a cleaner found (a part's, a guess by shape, a
    /// Chromium cache): in the Advisor they are the Cleaners group, not rows of the list.
    pub(crate) fn cleaner_findings(&self, r: &Report) -> HashSet<usize> {
        use sprawl_advisor::finding::Group;
        self.cleaner_layout(r)
            .iter()
            .filter(|c| c.presence == Presence::InScan)
            .flat_map(|c| c.parts.iter().flat_map(|p| p.findings.iter().copied()))
            .filter(|&i| r.findings[i].group() == Group::Clean && !r.findings[i].in_active_project())
            .filter(|&i| !r.quiet.contains(&key(&r.findings[i].path)))
            // what can go now, as the headline counts it; what needs administrator stays a row
            .filter(|&i| untickable(&r.findings[i], r.counted[i]).is_none() && crate::cleanup::held_in_app(&r.findings[i]).is_none())
            .collect()
    }

    /// The Advisor's Cleaners group: the total, the biggest few applications (each ticked
    /// there as on the Cleaners tab), and the way to the tab. Its bytes are in the headline and
    /// in "tick enough" like any other finding's.
    pub(crate) fn cleaners_group(&self, r: &Report) -> String {
        let mine = self.cleaner_findings(r);
        if mine.is_empty() {
            return String::new();
        }
        let rows = self.cleaner_layout(r);
        let bytes_of = |c: &ClRow| -> u64 { c.parts.iter().flat_map(|p| p.findings.iter()).filter(|&&i| mine.contains(&i) && r.counted[i]).map(|&i| self.review_size(&r.findings[i]).0).sum() };
        let mut apps: Vec<(usize, u64)> = rows.iter().enumerate().map(|(ci, c)| (ci, bytes_of(c))).filter(|(_, b)| *b > 0).collect();
        apps.sort_by_key(|(_, b)| std::cmp::Reverse(*b));
        let total: u64 = apps.iter().map(|(_, b)| b).sum();
        let mut x = format!(
            r##"<Button x:Name="AdvClGrp" HorizontalAlignment="Stretch" HorizontalContentAlignment="Left" Padding="10,6" Margin="0,6,0,0" CornerRadius="{{$rc7}}" BorderThickness="0" Background="{{$surface2}}"><StackPanel Spacing="1"><TextBlock FontSize="{{$fs13}}"><Run Text="Cleaners" FontWeight="SemiBold"/><Run Text="   Caches of {} {}, {} ({})" Foreground="{{$text2}}"/></TextBlock><TextBlock Text="Caches, logs and crash reports the applications make again. Tick an application here for its caches and logs, or see each part in Cleaners." FontSize="{{$fs12}}" Foreground="{{$text3}}" TextWrapping="Wrap"/></StackPanel></Button>"##,
            apps.len(),
            if apps.len() == 1 { "application" } else { "applications" },
            self.size(total),
            if self.advisor.cl.hide_group { "show" } else { "hide" }
        );
        if self.advisor.cl.hide_group {
            return x;
        }
        let running = self.advisor.cleanup.running.is_some();
        let ticked = |i: usize| self.advisor.cleanup.ticked.contains(&key(&r.findings[i].path));
        x += r##"<StackPanel Spacing="2" Margin="10,0,0,0">"##;
        for &(ci, b) in apps.iter().take(GROUP_APPS) {
            let c = &rows[ci];
            let takes: Vec<usize> = c.parts.iter().filter(|p| app_tick_takes(p)).flat_map(|p| p.tickable(r)).filter(|&i| box_takes(r, i) && mine.contains(&i)).collect();
            let some = c.parts.iter().flat_map(|p| p.findings.iter()).any(|&i| ticked(i));
            let state = if takes.is_empty() {
                None
            } else if takes.iter().all(|&i| ticked(i)) {
                Some("True")
            } else if some {
                Some("{x:Null}")
            } else {
                Some("False")
            };
            let n = c.parts.iter().filter(|p| p.findings.iter().any(|i| mine.contains(i))).count();
            x += &format!(
                r##"<Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="34"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>{}<StackPanel Grid.Column="1" Spacing="1" VerticalAlignment="Center"><TextBlock Text="{}" FontSize="{{$fs13}}" TextTrimming="CharacterEllipsis"/><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}"/></StackPanel><TextBlock Grid.Column="2" Text="{}" FontSize="{{$fs13}}" VerticalAlignment="Center"/></Grid>"##,
                match state {
                    Some(s) => format!(
                        r##"<CheckBox x:Name="AdvClA{ci}" MinWidth="0" Padding="0" VerticalAlignment="Center" IsChecked="{s}" IsEnabled="{}" AutomationProperties.Name="Clean up {}" ToolTipService.ToolTip="Ticks its caches and logs. Privacy parts are ticked one by one, in Cleaners."/>"##,
                        if running { "False" } else { "True" },
                        ui::esc(&c.name)
                    ),
                    None => String::new(),
                },
                ui::esc(&c.name),
                ui::esc(&match c.parts.iter().filter(|p| p.findings.iter().any(|&i| ticked(i))).count() {
                    0 => format!("{n} {}", if n == 1 { "part" } else { "parts" }),
                    // how many a tick button took, as the Cleaners tab says it
                    k => format!("{n} {} · {k} ticked", if n == 1 { "part" } else { "parts" }),
                }),
                self.size(b)
            );
        }
        let more = apps.len().saturating_sub(GROUP_APPS);
        // ticks in the applications not listed here would be out of sight
        let hidden: usize = apps.iter().skip(GROUP_APPS).map(|&(ci, _)| rows[ci].parts.iter().filter(|p| p.findings.iter().any(|&i| ticked(i))).count()).sum();
        if hidden > 0 {
            x += &format!(r##"<TextBlock Text="{hidden} more ticked in applications not listed here: see them in Cleaners." FontSize="{{$fs12}}" Foreground="{{$text3}}" Margin="34,2,0,0" TextWrapping="Wrap"/>"##);
        }
        x += &format!(
            r##"<Button x:Name="AdvClAll" Content="{}" Margin="34,4,0,0"/></StackPanel>"##,
            if more > 0 { format!("See all in Cleaners ({more} more)") } else { "See all in Cleaners".to_string() }
        );
        x
    }
}

/// The Privacy group's wording, held to `privacy::PROMISE_WORDS` by a test.
pub(crate) const PV_HEAD: &str = "Privacy";
pub(crate) const PV_SUB: &str = "Cookies, history and what forms remember in this account's browsers, by profile, and the short lists Explorer keeps in this account's own registry. They free almost no space: each is ticked on its own, and nothing here is ticked for you.";
pub(crate) const PV_NONE: &str = "Nothing in this account's browsers or Explorer registry lists was found in what was read.";
pub(crate) const PV_HELD: &str = "held back by Sprawl's checks";

impl State {
    /// Settings > Cleaning, "Show privacy cleaners" (off unless switched on).
    pub(crate) fn privacy_on(&self) -> bool {
        self.settings.bool(sprawl_advisor::privacy::SETTING) == Some(true)
    }

    /// The Privacy group, after the space groups: one row per browser profile, by the name the
    /// browser gives it (its folder in the tooltip when they differ), each part with a tick of
    /// its own. No application tick, no size total. Nothing with the setting off.
    fn privacy_group(&self, r: &Report) -> String {
        if !r.privacy_on {
            return String::new();
        }
        let running = self.advisor.cleanup.running.is_some();
        let mut x = format!(
            r##"<TextBlock Text="{}" FontSize="{{$fs12}}" FontWeight="SemiBold" Foreground="{{$text2}}" Margin="0,8,0,0"/><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" TextWrapping="Wrap"/>"##,
            PV_HEAD,
            ui::esc(PV_SUB)
        );
        if r.privacy_rows.is_empty() {
            x += &format!(r##"<TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}" TextWrapping="Wrap" Margin="34,2,0,0"/>"##, ui::esc(PV_NONE));
        }
        for row in &r.privacy_rows {
            let name = &row.label;
            let tip = if row.tip.is_empty() { String::new() } else { format!(r##" ToolTipService.ToolTip="{}""##, ui::esc(&row.tip)) };
            x += &format!(r##"<StackPanel Spacing="1" Margin="34,2,0,4"><TextBlock Text="{}" FontSize="{{$fs13}}"{tip}/>"##, ui::esc(name));
            for &i in &row.parts {
                let f = &r.privacy[i];
                let browser = match &f.source {
                    Source::Rule(id) => sprawl_advisor::privacy::part(id),
                    _ => None,
                };
                let reg = match &f.source {
                    Source::Rule(id) => sprawl_advisor::regclean::part(id),
                    _ => None,
                };
                let ticked = self.advisor.cleanup.privacy_ticked.contains(&key(&f.path));
                // not the space labels ("your own files"): what it is, and whether it can go now
                let mut sub = match reg {
                    Some(p) => {
                        let n = r.registry_counts.get(p.id).copied().unwrap_or(0);
                        format!("Privacy · {}", if n == 1 { "1 value".to_string() } else { format!("{n} values") })
                    }
                    None => "Privacy".to_string(),
                };
                match self.app_open(f) {
                    Some((app, _)) => sub += &format!(" · close {app} first"),
                    None => {}
                }
                x += &format!(
                    r##"<Grid ColumnSpacing="6" Margin="0,2,0,0"><Grid.ColumnDefinitions><ColumnDefinition Width="30"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><CheckBox x:Name="AdvPvT{i}" MinWidth="0" Padding="0" VerticalAlignment="Top" IsChecked="{}" IsEnabled="{}" AutomationProperties.Name="{}"/><StackPanel Grid.Column="1" Spacing="1"><StackPanel Orientation="Horizontal" Spacing="8">{}<TextBlock Text="{}" FontSize="{{$fs13}}" ToolTipService.ToolTip="{}"/></StackPanel><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}"/><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{}" TextWrapping="Wrap"/></StackPanel></Grid>"##,
                    if ticked { "True" } else { "False" },
                    if running { "False" } else { "True" },
                    ui::esc(&f.title),
                    crate::advisor::risk_mark(f.risk, 8, crate::advisor::MID),
                    ui::esc(browser.map_or(f.title.as_str(), |p| p.title)),
                    ui::esc(browser.map_or_else(|| reg.map_or("", |p| p.why), |p| p.why)),
                    ui::esc(&sub),
                    ui::esc(browser.map_or_else(|| reg.map_or("", |p| p.cost), |p| p.cost)),
                    if ticked { crate::tokens::refer("warn") } else { crate::tokens::refer("text2") },
                );
            }
            x += "</StackPanel>";
        }
        if !r.privacy_held.is_empty() {
            let why: Vec<String> = r.privacy_held.iter().map(|(f, w)| format!("{}: {w}", f.title)).collect();
            x += &format!(
                r##"<TextBlock Text="{} {}" FontSize="{{$fs12}}" Foreground="{{$text3}}" Margin="34,2,0,0" ToolTipService.ToolTip="{}"/>"##,
                r.privacy_held.len(),
                PV_HELD,
                ui::esc(&why.join("&#10;"))
            );
        }
        x
    }
}

/// The system drive, "C:".
fn sys_drive() -> String {
    std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into())
}

/// What an application's own tick takes: its parts that free space and are safe or amber.
/// Privacy parts and the user's own files are ticked one by one.
fn app_tick_takes(p: &PartRow) -> bool {
    !p.info && p.class == Class::Space && p.risk <= Risk::Amber
}

/// The cleaners and where each one's application is, for a report (off the UI thread).
pub(crate) fn presences(tree: &sprawl_core::Tree, catalogue: &[CleanerDef], findings: &[Finding]) -> Vec<Presence> {
    let found: HashSet<&str> = findings.iter().filter_map(|f| if let Source::Rule(id) = &f.source { Some(id.as_str()) } else { None }).collect();
    catalogue.iter().map(|d| sprawl_advisor::cleaners::presence(tree, d, d.parts.iter().any(|p| found.contains(p.rule.as_str())))).collect()
}

#[cfg(test)]
mod own_failed_tests {
    use super::*;

    #[test]
    fn a_failed_read_drops_the_old_reads_ticks_only() {
        let mut c = crate::cleanup::Cleanup::default();
        c.ticked.insert("c:/x".into());
        c.ticked_for = Some(OWN_GEN + 3);
        c.review = true;
        c.privacy_ticked.insert("c:/y".into());
        own_failed_drops(&mut c);
        assert!(c.ticked.is_empty() && c.ticked_for.is_none() && !c.review && c.privacy_ticked.is_empty());
        // a scan's ticks aren't the tab's own read's
        let mut c = crate::cleanup::Cleanup::default();
        c.ticked.insert("c:/x".into());
        c.ticked_for = Some(7);
        own_failed_drops(&mut c);
        assert_eq!(c.ticked.len(), 1);
    }

    #[test]
    fn an_unopened_folder_says_why_without_guessing() {
        let line = unread_line(&[(std::path::PathBuf::from(r"C:\Windows\Temp"), "Access is denied. (os error 5)".into())]);
        assert_eq!(line, r"Left out, as they couldn't be opened: C:\Windows\Temp (Access is denied. (os error 5))");
        assert!(!line.contains("administrator"));
    }
}
