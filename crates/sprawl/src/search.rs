//! The Search tab: find files and folders by name in the scan, see them on the map, act on them
//! with the same commands as everywhere else. The index (`sprawl_core::search`) is built on
//! another thread the first time the tab is used after a scan; each keystroke starts a new
//! search on other threads and cancels the one before. The tree, the selection and the map's
//! layout are left as they were.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use sprawl_core::search::{search, Index, Kind, Query};

use crate::format::{fmt_time, thousands};
use crate::state::{find, text, State, View};
use crate::tables::{cell, Col, FILES_LIKE};
use crate::ui;

/// The map colour code of a match (see `advisor::lens_colour`).
pub(crate) const LENS_MATCH: u8 = 20;

/// The row menu's items that need the scan on screen: (item, its tip on the scan's row, its tip
/// on a row from another drive or a saved scan).
pub(crate) const SCAN_ONLY: &[(&str, &str, &str)] = &[
    ("cInTree", "Show it in the tree", "Only a result of the scan on screen is in the tree."),
    ("cMapHere", "Show where it sits on the treemap", "Only a result of the scan on screen is on the treemap."),
    ("cExpandAll", "Expand everything inside it", "Only a result of the scan on screen can be expanded."),
    ("cCopyInfo", "Copy the rows on screen as text", "Copy Name and Size copies the scan on screen's rows."),
];

/// The row menu's "Overwrite and delete": Tree View and File View only, off in Search.
pub(crate) const OVERWRITE_ONLY: &str = "Overwrite and delete works in Tree View and File View.";

/// The `dc:` and `da:` words of the query: what they ask, what the last search's reads left out,
/// and why they can't be used.
#[derive(Default, Clone)]
pub(crate) struct Reads {
    /// The query without those words: what is searched.
    pub(crate) plain: String,
    pub(crate) dates: sprawl_core::query::ReadDates,
    /// A value not understood, no other word to narrow the search with, no scan, or a CSV view.
    pub(crate) error: Option<String>,
    /// The last search's reads: matches the reading limit stopped and matches with no time to
    /// read, for the one-line count, and the same said whole for the no-match panel.
    pub(crate) unread: usize,
    pub(crate) unknown: usize,
    /// Matches read only in part (a folder inside was skipped or couldn't be read): never
    /// decided either way, so never dropped without a word.
    pub(crate) partial: usize,
    /// More matches than one answer reads: none of them was checked, and this is how many.
    pub(crate) too_many: usize,
    /// Matches whose times were read and gave a yes or a no: what separates "none of them is in
    /// the range" from "nothing could be checked" (times-20).
    pub(crate) decided: usize,
    pub(crate) notes: Vec<String>,
}

/// The count line while a `dc:`/`da:` search reads the matches' times.
const READING_NOTE: &str = "Reading when the matches were created / last opened...";

/// What one search's `dc:`/`da:` reads said, for the frame that collects them.
#[derive(Default, Clone)]
pub(crate) struct Said {
    pub(crate) notes: Vec<String>,
    pub(crate) unread: usize,
    pub(crate) unknown: usize,
    pub(crate) partial: usize,
    /// Matches too many to read: nothing was checked, and this is how many there were.
    pub(crate) too_many: usize,
    /// Matches read and decided, yes or no.
    pub(crate) decided: usize,
}

/// The most matches a `dc:`/`da:` search reads times for: past it, the words are not checked and
/// the status line says so instead of showing a wrong list (one answer's reading budget, as the
/// AI's own `da:` search has).
const READ_MOST: usize = sprawl_advisor::used::ANSWER_BUDGET;

/// What the times read for the matches left: the matches inside the ranges, and what wasn't read.
enum ReadOutcome {
    Kept { kept: Vec<usize>, notes: Vec<String>, unread: usize, partial: usize, unknown: usize, decided: usize },
    /// Too many matches to read without narrowing: how many there were.
    TooMany(usize),
}

/// The matches of the scan whose creation (and last-opened) time is inside the ranges. Runs on
/// the search's own thread: each path is one reading (a folder's last-opened time is its files'),
/// nothing that wasn't read can be taken for a "no", and a newer search stops it (`None`) so a
/// stale query never keeps reading the disk.
fn read_filter(tree: &sprawl_core::Tree, hits: Vec<usize>, dates: &sprawl_core::query::ReadDates, cancel: &AtomicBool) -> Option<ReadOutcome> {
    read_filter_with(tree, hits, dates, cancel, None)
}

/// [`read_filter`], with the `da:` reads already made when a test needs a given reading limit.
fn read_filter_with(
    tree: &sprawl_core::Tree,
    hits: Vec<usize>,
    dates: &sprawl_core::query::ReadDates,
    cancel: &AtomicBool,
    opened: Option<sprawl_advisor::used::Accessed>,
) -> Option<ReadOutcome> {
    use sprawl_advisor::used::InRange;
    let mut opened = dates.accessed.map(|_| opened.unwrap_or_else(sprawl_advisor::used::Accessed::new));
    // a PC that keeps no last-access times can't answer `da:` however far the search is narrowed,
    // so that is said before "too many to read" (times-18); nothing is read for such a search
    let no_access = opened.as_ref().is_some_and(|a| !a.kept());
    // a scan of a drive whose times are never read (a share, a removable disk, not NTFS for
    // `da:`) says that for each match, not "too many to read": narrowing could never help
    let first = hits.first().map(|&n| tree.path_of(n));
    let never_read = hits.len() > READ_MOST
        && first.as_ref().is_some_and(|p| {
            opened.as_mut().is_some_and(|a| !a.reads_drive(p)) || (dates.created.is_some() && !sprawl_advisor::born::Created::new().reads_drive(p))
        });
    if hits.len() > READ_MOST && !no_access && !never_read {
        return Some(ReadOutcome::TooMany(hits.len()));
    }
    let mut born = dates.created.filter(|_| !no_access).map(|_| sprawl_advisor::born::Created::new());
    let (mut unread_c, mut unknown_c, mut unread_a, mut partial_a, mut unknown_a) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut kept = Vec::with_capacity(hits.len());
    let total = hits.len();
    for n in hits {
        // a newer search started: this one's answer would be thrown away, so stop reading
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let path = tree.path_of(n);
        let mut ok = true;
        if let (Some(r), Some(b)) = (dates.created, born.as_mut()) {
            match b.of(&path) {
                sprawl_advisor::born::Born::At(t) => {
                    let s = sprawl_core::query::unix_secs(t);
                    ok = r.0 <= s && s < r.1;
                }
                sprawl_advisor::born::Born::Unread => {
                    unread_c += 1;
                    ok = false;
                }
                sprawl_advisor::born::Born::Unknown => {
                    unknown_c += 1;
                    ok = false;
                }
            }
        }
        if ok {
            if let (Some(r), Some(a)) = (dates.accessed, opened.as_mut()) {
                // every state has its own count: a limit or a partly read folder is never
                // dropped as if it had been decided (times-12)
                match sprawl_advisor::used::in_range(a.of(&path), r.0, r.1) {
                    InRange::Yes => {}
                    InRange::No => ok = false,
                    InRange::Unread => {
                        unread_a += 1;
                        ok = false;
                    }
                    InRange::Partial => {
                        partial_a += 1;
                        ok = false;
                    }
                    InRange::Unknown => {
                        unknown_a += 1;
                        ok = false;
                    }
                }
            }
        }
        if ok {
            kept.push(n);
        }
    }
    let count = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut notes = Vec::new();
    if unread_c > 0 {
        notes.push(format!(
            "{} not checked for its created time: the reading limit ran out (a smaller folder or a narrower search gives more).",
            count(unread_c, "match", "matches")
        ));
    }
    if let Some(b) = born.as_ref() {
        notes.extend(b.notes("match", "matches"));
    }
    if unread_a > 0 {
        notes.push(format!(
            "{} not checked for when it was last opened: the reading limit ran out (a smaller folder or a narrower search gives more).",
            count(unread_a, "match", "matches")
        ));
    }
    if let Some(note) = sprawl_advisor::used::partial_note(partial_a, "match", "matches") {
        notes.push(note);
    }
    if unknown_a > 0 {
        notes.extend(sprawl_advisor::used::notes(0, unknown_a, "match", "matches", opened.as_ref().expect("one was made")));
    }
    let missed = unread_c + unread_a + partial_a + unknown_c + unknown_a;
    Some(ReadOutcome::Kept { kept, notes, unread: unread_c + unread_a, partial: partial_a, unknown: unknown_c + unknown_a, decided: total.saturating_sub(missed) })
}

/// Whether other drives and saved scans are searched for a query: `dc:` and `da:` read each
/// match's own time, which only the scan on screen can do (one path at a time), so a query with
/// either searches nothing else. Their rows would otherwise be listed without the dates, mixed
/// into the scan's filtered list and ready to be picked and deleted.
pub(crate) fn places_searched(dates: &sprawl_core::query::ReadDates) -> bool {
    dates.is_empty()
}

/// The reads for `query`: the query without a `dc:`/`da:` word, the times to read, and why they
/// can't be used here (a value not understood, nothing to narrow them with, no scan, a CSV view).
/// What the last search said about the same words is carried over, so a redraw of the box (Ctrl+F,
/// the tab button, a theme change) can't wipe a reading limit or an unchecked match into "no match".
fn reads_for(query: &str, now: std::time::SystemTime, tree: Option<&sprawl_core::Tree>, old: &Reads) -> Reads {
    let (plain, r) = sprawl_core::query::split_read_dates(query, now);
    let mut reads = Reads { plain, ..Default::default() };
    match r {
        Err(e) => reads.error = Some(e),
        Ok(d) if !d.is_empty() => {
            if reads.plain.trim().is_empty() {
                reads.error = Some(
                    "dc: and da: read each match's own time, so give them another word to narrow the search first (a name, ext:, size: or a folder)."
                        .to_string(),
                );
            } else {
                reads.dates = d;
            }
        }
        Ok(_) => {}
    }
    if !reads.dates.is_empty() {
        // the times are read only from the scan on screen, and never from a CSV view
        match tree {
            None => reads.error = Some("dc: and da: read each match's own time, which needs a scan: scan a folder or a drive first.".to_string()),
            Some(t) if t.method == sprawl_core::ScanMethod::Csv => {
                reads.error =
                    Some("dc: and da: read each match's own time, which this view doesn't have: it came from a CSV file, not the disk.".to_string())
            }
            Some(_) => {}
        }
    }
    if reads.plain == old.plain && reads.dates == old.dates && reads.error == old.error {
        reads.notes = old.notes.clone();
        reads.unread = old.unread;
        reads.unknown = old.unknown;
        reads.partial = old.partial;
        reads.too_many = old.too_many;
        reads.decided = old.decided;
    }
    reads
}

/// The first line of the no-match panel: what a `dc:`/`da:` search read is said first, because
/// "Nothing matches" would pass matches that were never checked off as a no.
fn none_headline(q: &str, place: &str, reads: &Reads) -> String {
    let missed = reads.unread + reads.unknown + reads.partial + reads.too_many;
    if reads.notes.is_empty() && missed == 0 {
        format!("Nothing matches \u{201c}{q}\u{201d} in {place}.")
    } else if reads.decided > 0 {
        // some were read and are out of the range: only the rest couldn't be checked (times-20)
        format!(
            "None of the {} read for \u{201c}{q}\u{201d} in {place} is in the range; {missed} couldn't be checked.",
            thousands(reads.decided as u64)
        )
    } else {
        format!("Nothing could be checked for \u{201c}{q}\u{201d} in {place}.")
    }
}

/// What the count line adds about a `dc:`/`da:` search's reads: nothing when all were read.
fn count_suffix(reads: &Reads) -> String {
    let mut s = String::new();
    if reads.too_many > 0 {
        s += &format!("; {} not checked (too many to read: narrow the search)", thousands(reads.too_many as u64));
    } else if reads.unread > 0 {
        s += &format!("; {} not read (reading limit)", thousands(reads.unread as u64));
    }
    if reads.partial > 0 {
        s += &format!("; {} not fully read", thousands(reads.partial as u64));
    }
    if reads.unknown > 0 {
        s += &format!("; {} with no time to read", thousands(reads.unknown as u64));
    }
    s
}

/// The most results the row menu opens at once: Explorer starts a program for each result and a
/// window for each folder holding picked files or picked itself.
pub(crate) const MAX_OPEN: usize = 10;

/// Whether Open (or Show in Explorer) may go ahead for `n` results or windows at once.
pub(crate) fn can_open(n: usize) -> bool {
    n <= MAX_OPEN
}

/// Whether the row menu's `item` can act on a Search result: `scan` is true for a row of the scan
/// on screen, and the items that need the scan are off for the rows that come from elsewhere.
pub(crate) fn item_applies(item: &str, scan: bool) -> bool {
    if item == "cOverwrite" {
        return false;
    }
    scan || !SCAN_ONLY.iter().any(|&(name, _, _)| name == item)
}

/// (tree generation, query, sort, matches sorted, the folder ranks it built, what the `dc:`/`da:`
/// reads said)
type Found = (u64, String, (usize, bool), Option<Vec<usize>>, Option<Arc<Vec<u32>>>, Said);
/// (tree generation, sort, the matches sorted again, the folder ranks it built)
type Sorted = (u64, (usize, bool), Vec<usize>, Option<Arc<Vec<u32>>>);

/// How long typing pauses before the search runs: short enough to feel immediate, long enough
/// that a word typed quickly doesn't start (and cancel) a search per letter.
const TYPING_PAUSE_MS: u128 = 50;

/// More matches than this are sorted again on a worker (a header click), not on the UI thread.
const SORT_HERE: usize = 20_000;

#[derive(Default)]
pub(crate) struct Search {
    /// What is typed.
    pub(crate) query: String,
    /// The `dc:`/`da:` words of what is typed, and what their reads left out.
    pub(crate) reads: Reads,
    /// The map is shown under the results (the Map button; off at first: results take the room).
    pub(crate) map: bool,
    /// The map's tile the row menu was opened on (it counts only while it is still `selected`).
    pub(crate) tile: Option<usize>,
    /// What the no-match message says now ("" hidden; `None` not drawn yet).
    pub(crate) none: Option<String>,
    /// The last tap on a result: (row, when, already taken as a double-click).
    last_tap: Option<((usize, usize), std::time::Instant, bool)>,
    /// The index and the `tree_gen` it was built for.
    index: Option<(u64, Arc<Index>)>,
    pub(crate) building: Option<(u64, Receiver<Index>)>,
    cancel: Arc<AtomicBool>,
    running: Option<Receiver<Found>>,
    /// The search to run once the index is ready.
    pending: bool,
    /// Matches, sorted, for `results_gen`.
    pub(crate) results: Vec<usize>,
    pub(crate) results_gen: u64,
    pub(crate) sort: (usize, bool),
    /// Matches lit on the map.
    pub(crate) lens: Option<Arc<Vec<u8>>>,
    /// Nodes where the watcher saw a change since the scan, and the `tree_gen` they are for.
    changed: (u64, std::collections::HashSet<usize>),
    /// Saved scans and other drives.
    pub(crate) more: crate::search_more::More,
    /// When the box last changed: the search runs once typing pauses.
    typed: Option<std::time::Instant>,
    /// Bumped whenever the scan's matches or the other drives' matches change or are re-sorted.
    pub(crate) stamp: u64,
    /// Every folder's place in path order (`folder_ranks`), for the `tree_gen` it was built for.
    ranks: Option<(u64, Arc<Vec<u32>>)>,
    /// A sort of many matches on a worker.
    pub(crate) sorting: Option<Receiver<Sorted>>,
    /// The two lists merged into the table's rows, for (`stamp`, `tree_gen`).
    merged: std::cell::RefCell<Option<((u64, u64), Vec<(usize, usize)>)>>,
    /// The help and the empty state are made (once, when the tab first opens).
    pub(crate) ui_built: bool,
    /// The filter chips as made (their names are looked up in here).
    pub(crate) chips: Option<crate::bindings::Microsoft::UI::Xaml::FrameworkElement>,
    /// The suggestions under the box and the highlighted one.
    pub(crate) suggest: Option<crate::search_query::Suggestions>,
    pub(crate) highlight: Option<usize>,
    /// Saved and recent searches (read when first needed), and what their menu was made for.
    pub(crate) history: Option<crate::search_query::History>,
    pub(crate) saved_shape: String,
    /// The columns hidden (bit i: column i of `COLS`; Name never), remembered in settings.
    pub(crate) cols_hidden: u8,
    cols_loaded: bool,
}

/// The settings words for `COLS`, in order (`searchColsHidden` lists the hidden ones).
pub(crate) const COL_KEYS: [&str; 5] = ["name", "size", "alloc", "modified", "folder"];

/// The hidden columns' bits for a `searchColsHidden` value; Name can't be hidden and unknown
/// words are ignored.
pub(crate) fn hidden_from(s: &str) -> u8 {
    s.split(',').filter_map(|w| COL_KEYS.iter().position(|k| *k == w.trim())).filter(|&i| i > 0).fold(0, |m, i| m | (1 << i))
}

/// The `searchColsHidden` value for the hidden columns' bits.
pub(crate) fn hidden_to(mask: u8) -> String {
    COL_KEYS.iter().enumerate().filter(|&(i, _)| i > 0 && mask & (1 << i) != 0).map(|(_, k)| *k).collect::<Vec<_>>().join(",")
}

/// The sort to keep when the columns in `hidden` are hidden: a sort by a column that isn't shown
/// has no title to carry its arrow, so the list goes back to Name, A to Z.
pub(crate) fn sort_for_hidden(sort: (usize, bool), hidden: u8) -> (usize, bool) {
    if sort.0 > 0 && sort.0 < COL_KEYS.len() && hidden & (1 << sort.0) != 0 { (0, false) } else { sort }
}

/// The columns' menu: (the row's name, its words, the column).
pub(crate) const COL_MENU: [(&str, &str, usize); 4] = [("scSize", "Size", 1), ("scAlloc", "Allocated", 2), ("scMod", "Modified", 3), ("scFolder", "Folder", 4)];

pub(crate) const COLS: &[Col] = FILES_LIKE;

impl Search {
    /// The watcher saw a change at node `n` (the nearest part of the tree that still exists).
    pub(crate) fn changed(&mut self, gen: u64, n: usize) {
        if self.changed.0 != gen {
            self.changed = (gen, Default::default());
        }
        if self.changed.1.len() < 100_000 {
            self.changed.1.insert(n);
        }
    }

    /// Node `n`, or the folder it is in, changed since the scan.
    fn is_changed(&self, gen: u64, n: usize, parent: Option<usize>) -> bool {
        self.changed.0 == gen && (self.changed.1.contains(&n) || parent.is_some_and(|p| self.changed.1.contains(&p)))
    }

    /// Biggest first.
    pub(crate) fn new() -> Search {
        Search { sort: (1, true), ..Default::default() }
    }
}

impl State {
    /// The columns as shown now: the hidden ones at width 0.
    pub(crate) fn search_cols(&self) -> std::borrow::Cow<'static, [Col]> {
        if self.search.cols_hidden == 0 {
            std::borrow::Cow::Borrowed(COLS)
        } else {
            std::borrow::Cow::Owned(crate::tables::shown_cols(COLS, self.search.cols_hidden))
        }
    }

    /// Whether column `i` is shown.
    pub(crate) fn search_col_shown(&self, i: usize) -> bool {
        i == 0 || self.search.cols_hidden & (1 << i) == 0
    }

    /// The remembered choice of columns is read (once, before the first rows are made).
    pub(crate) fn search_cols_load(&mut self) {
        if !self.search.cols_loaded {
            self.search.cols_loaded = true;
            self.search.cols_hidden = hidden_from(self.settings.get("searchColsHidden").unwrap_or(""));
            self.search_sort_visible();
        }
    }

    /// The list is sorted by a column that is shown: if its column is hidden it is sorted by
    /// Name instead.
    fn search_sort_visible(&mut self) {
        let sort = sort_for_hidden(self.search.sort, self.search.cols_hidden);
        if sort != self.search.sort {
            self.search.sort = sort;
            self.search_sort();
        }
    }

    /// A right-click on the list's header: the columns, each with a check; Name stays.
    pub(crate) fn search_cols_menu(&mut self, at: crate::bindings::Windows::Foundation::Point) {
        if self.view != View::Search {
            return;
        }
        self.search_cols_load();
        let Ok(target) = find::<crate::bindings::Microsoft::UI::Xaml::UIElement>(&self.root, "TreeHeader") else { return };
        for (name, _, i) in COL_MENU {
            let _ = crate::menubar::set_checked(&self.root, name, self.search_col_shown(i));
        }
        crate::menu::show_at("ColsMenu", &target, at, false);
    }

    /// The column's check changed: show or hide it, and remember it. Nothing when it is so
    /// already (the check was set by the menu opening).
    pub(crate) fn search_col_set(&mut self, i: usize, shown: bool) {
        self.search_cols_load();
        if i == 0 || i >= COL_KEYS.len() || self.search_col_shown(i) == shown {
            return;
        }
        if shown {
            self.search.cols_hidden &= !(1 << i);
        } else {
            self.search.cols_hidden |= 1 << i;
        }
        self.search_sort_visible();
        let v = hidden_to(self.search.cols_hidden);
        self.settings.set("searchColsHidden", v);
        self.settings.save();
        if self.view == View::Search {
            // the folder paths take the width the shown columns leave
            if let Some(w) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::ScrollViewer>(&self.root, "TreeScroll").ok().and_then(|s| s.ViewportWidth().ok()).filter(|w| *w > 0.0) {
                self.tables.folder_chars = crate::tables::folder_chars_in(w, &self.search_cols());
            }
            self.refresh_rows();
        }
    }

    /// The text in the search box changed.
    pub(crate) fn search_changed(&mut self, q: String) {
        if q == self.search.query && self.search.results_gen == self.tree_gen {
            return;
        }
        self.search.query = q;
        self.search.typed = Some(std::time::Instant::now());
        self.search_box_state();
    }

    /// Start a search for what is typed (or wait for the index).
    pub(crate) fn search_run(&mut self) {
        // whatever is running is about something older
        self.search.cancel.store(true, Ordering::Relaxed);
        self.search.running = None;
        self.search.sorting = None;
        self.search.reads.notes.clear();
        // `dc:`/`da:` read each match's own time, which needs a scan and can't be done for a
        // CSV view: the error is set here, once. A scan finishing (or a CSV view opening) while
        // the tab shows such a query changes it, so the hint line is redrawn with it and never
        // contradicts the list (times-14).
        let before = self.search.reads.error.clone();
        self.search_reads();
        if self.search.reads.error != before {
            self.search_hint_state();
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.search.cancel = cancel.clone();
        // outside the scan: saved scans and other drives
        self.more_forget_covered();
        self.more_run(cancel.clone());
        let Some(tree) = self.tree.clone() else {
            let m = &self.search.more;
            let note = if m.is_reading() {
                "Reading your drives; results come in as each one is ready."
            } else if !m.has_places() {
                "Scan a folder, or turn on Options > Search Every Drive When Search Opens."
            } else if self.search.query.trim().is_empty() {
                "Type to search every drive."
            } else {
                ""
            };
            let note = if note.is_empty() { self.search_count() } else { note.to_string() };
            self.search_show(Vec::new(), note, false);
            return;
        };
        let q = Query::parse(&self.search.reads.plain);
        // a word that can't be understood, or a `dc:`/`da:` word with nothing to narrow it:
        // nothing is searched (the hint line says why)
        if q.is_empty() || !q.errors().is_empty() || self.search.reads.error.is_some() {
            self.search.pending = false;
            self.search_show(Vec::new(), String::new(), false);
            return;
        }
        let gen = self.tree_gen;
        let index = match &self.search.index {
            Some((g, ix)) if *g == gen => ix.clone(),
            _ => {
                // build it (once) and search when it is ready
                self.search.pending = true;
                if self.search.building.as_ref().is_none_or(|(g, _)| *g != gen) {
                    self.search.index = None;
                    let (tx, rx) = channel();
                    let t = tree.clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(Index::build(&t));
                    });
                    self.search.building = Some((gen, rx));
                }
                text(&self.root, "SearchInfo", "Getting the names ready...");
                return;
            }
        };
        self.search.pending = false;
        let (tx, rx) = channel();
        let text_q = self.search.query.clone();
        // sorted on the worker too: a million matches sort slowly
        let (c, desc) = self.search.sort;
        let key = COLS[c.min(COLS.len() - 1)].key();
        let alloc = if key == crate::tables::Key::Alloc { self.alloc.clone() } else { Vec::new() };
        let ranks = self.ranks_for(gen);
        let dates = self.search.reads.dates;
        // a `dc:`/`da:` read takes seconds: the older query's rows are never left on screen, ready
        // to be picked and deleted, while the times are read (times-23)
        if !dates.is_empty() {
            self.search.reads.decided = 0;
            self.search_show(Vec::new(), READING_NOTE.to_string(), false);
        }
        std::thread::spawn(move || {
            let mut r = search(&tree, &index, &q, &cancel);
            let mut said = Said::default();
            // `dc:`/`da:`: the times are read for the matches the other words left, here and
            // never on the window's thread; a newer search stops it and nothing is shown
            if !dates.is_empty() {
                r = match r {
                    Some(hits) => match read_filter(&tree, hits, &dates, &cancel) {
                        Some(ReadOutcome::Kept { kept, notes, unread, partial, unknown, decided }) => {
                            said = Said { notes, unread, partial, unknown, too_many: 0, decided };
                            Some(kept)
                        }
                        Some(ReadOutcome::TooMany(n)) => {
                            said = Said {
                                notes: vec![format!(
                                    "dc: and da: weren't checked: {} matches would each have their own time read; add a word that narrows the search (a name, ext:, size: or a folder).",
                                    crate::format::thousands(n as u64)
                                )],
                                unread: n,
                                unknown: 0,
                                partial: 0,
                                too_many: n,
                                decided: 0,
                            };
                            Some(Vec::new())
                        }
                        None => None,
                    },
                    None => None,
                };
            }
            let mut built = None;
            if let Some(v) = &mut r {
                let ranks = folder_ranks_if(&tree, key, v.len(), ranks, &mut built);
                crate::tables::sort_nodes(&tree, &alloc, key, desc, v, ranks.as_deref().map_or(&[], |r| &r[..]));
            }
            let _ = tx.send((gen, text_q, (c, desc), r, built, said));
        });
        self.search.running = Some(rx);
    }

    /// Read the `dc:`/`da:` words of what is typed into [`Search::reads`]: the query without
    /// them, the two times to read, and why they can't be used here.
    pub(crate) fn search_reads(&mut self) {
        self.search.reads = reads_for(&self.search.query, std::time::SystemTime::now(), self.tree.as_deref(), &self.search.reads);
    }

    /// Each frame: the index, the results, and a tree that changed under the results.
    /// A search that found nothing says so where the results would be, with what was searched
    /// and the ways to widen it (the query stays in the box).
    fn search_none_state(&mut self) {
        let m = &self.search.more;
        let q = self.search.query.trim();
        // a `dc:`/`da:` query doesn't use other drives, so their reading doesn't hold the answer
        // back (and the panel says they are left out)
        let reads_places = places_searched(&self.search.reads.dates);
        let settled = self.search.running.is_none()
            && self.search.sorting.is_none()
            && !self.search.pending
            && self.search.typed.is_none()
            && (!m.is_reading() || !reads_places);
        let none = self.view == View::Search
            && !q.is_empty()
            && settled
            && self.search.reads.error.is_none()
            && Query::parse(&self.search.reads.plain).errors().is_empty()
            && self.search.results_gen == self.tree_gen
            && self.search.results.is_empty()
            && m.current_hits().is_empty();
        let msg = if none {
            let drives = m.on_list();
            // `dc:`/`da:` search only the scan on screen: naming other drives as searched would
            // say the dates had been checked there too
            let place = match (self.tree.is_some(), drives.is_empty() || !places_searched(&self.search.reads.dates)) {
                (true, true) => "this scan".to_string(),
                (true, false) => format!("this scan and {}", drives.join(", ")),
                _ => drives.join(", "),
            };
            let mut ways = vec!["check the words", "take a filter off (a lit button under the box, or its words in the box)"];
            if drives.is_empty() && places_searched(&self.search.reads.dates) {
                ways.push("add a drive with Search in");
            }
            let mut s = none_headline(q, &place, &self.search.reads);
            s += &format!("\nTo widen it: {}.", ways.join("; "));
            // what a `dc:`/`da:` search couldn't read is said here too: never as a plain "no"
            if !places_searched(&self.search.reads.dates) {
                s += "\ndc: and da: read each match's own time, so only the scan on screen is searched: other drives and saved scans are left out.";
            }
            if !self.search.reads.notes.is_empty() {
                s += &format!("\n{}", self.search.reads.notes.join("\n"));
            }
            if self.full_tree.is_some() {
                s += &format!("\nThe scan is filtered by \u{201c}{}\u{201d}: only what matches it is searched. Empty the Filter box on the Tree tab to search everything.", self.filter);
            }
            s
        } else {
            String::new()
        };
        if self.search.none.as_deref() == Some(msg.as_str()) {
            return;
        }
        text(&self.root, "SearchNoneText", &msg);
        if let Ok(e) = find::<crate::bindings::Microsoft::UI::Xaml::UIElement>(&self.root, "SearchNone") {
            let _ = e.SetVisibility(if msg.is_empty() { crate::bindings::Microsoft::UI::Xaml::Visibility::Collapsed } else { crate::bindings::Microsoft::UI::Xaml::Visibility::Visible });
        }
        self.search.none = Some(msg);
    }

    pub(crate) fn search_poll(&mut self) {
        self.search_none_state();
        if self.search.typed.is_some_and(|t| t.elapsed().as_millis() >= TYPING_PAUSE_MS) {
            self.search.typed = None;
            self.search_run();
        }
        self.more_poll();
        if let Some((g, rx)) = &self.search.building {
            if let Ok(ix) = rx.try_recv() {
                let g = *g;
                self.search.building = None;
                if g == self.tree_gen {
                    self.search.index = Some((g, Arc::new(ix)));
                    if self.search.pending {
                        self.search_run();
                    }
                }
            }
        }
        if let Some(rx) = &self.search.sorting {
            if let Ok((gen, sort, v, built)) = rx.try_recv() {
                self.search.sorting = None;
                self.keep_ranks(gen, built);
                if gen == self.tree_gen && self.search.results_gen == gen && sort == self.search.sort {
                    self.search.results = v;
                    self.search.stamp += 1;
                    // other drives' rows may still be sorting
                    let info = if self.search.more.is_resorting() { "Sorting...".to_string() } else { self.search_count() };
                    text(&self.root, "SearchInfo", &info);
                    if self.view == View::Search {
                        self.refresh_rows();
                    }
                }
            }
        }
        if let Some(rx) = &self.search.running {
            if let Ok((gen, q, sort, r, built, said)) = rx.try_recv() {
                self.search.running = None;
                self.keep_ranks(gen, built);
                if r.is_some() && gen == self.tree_gen && q == self.search.query {
                    self.search.reads.notes = said.notes;
                    self.search.reads.unread = said.unread;
                    self.search.reads.unknown = said.unknown;
                    self.search.reads.partial = said.partial;
                    self.search.reads.too_many = said.too_many;
                    self.search.reads.decided = said.decided;
                }
                if let Some(found) = r.filter(|_| gen == self.tree_gen && q == self.search.query) {
                    let resort = sort != self.search.sort;
                    self.search_show(found, String::new(), resort);
                }
            }
        }
        // a delete or a branch rescan changed the tree: search it again
        if self.view == View::Search
            && self.search.results_gen != self.tree_gen
            && !self.search.query.trim().is_empty()
            && self.search.running.is_none()
            && !self.search.pending
        {
            self.search_run();
        }
    }

    /// Show `found` (sorted here), or `note` when there is nothing to show.
    fn search_show(&mut self, found: Vec<usize>, note: String, sort: bool) {
        self.search.results = found;
        self.search.tile = None;
        self.search.results_gen = self.tree_gen;
        if sort {
            self.search_sort();
        }
        let t = self.tree.as_ref();
        self.search.stamp += 1;
        let info = if !note.is_empty() { note } else { self.search_count() };
        text(&self.root, "SearchInfo", &info);
        // the map: matches lit, the rest dimmed
        self.search.lens = match t {
            Some(t) if !self.search.results.is_empty() => {
                let mut v = vec![0u8; t.nodes.len()];
                for &n in &self.search.results {
                    v[n] = LENS_MATCH;
                }
                Some(Arc::new(v))
            }
            _ => None,
        };
        if self.view == View::Search {
            self.refresh_rows();
            self.prune_marked();
            self.map.drawn_once = false;
        }
        if crate::bench::on() {
            crate::bench::note(&format!("search results shown ({})", self.search.results.len()));
        }
    }

    pub(crate) fn search_sort(&mut self) {
        let (c, desc) = self.search.sort;
        let key = COLS[c.min(COLS.len() - 1)].key();
        let gen = self.tree_gen;
        let ranks = self.ranks_for(gen);
        // results of an older tree hold indices that tree doesn't have: the search that is owed
        // sorts by this column when it has run
        if let Some(t) = self.tree.clone().filter(|_| self.search.results_gen == self.tree_gen) {
            if self.search.results.len() <= SORT_HERE {
                let mut v = std::mem::take(&mut self.search.results);
                crate::tables::sort_nodes(&t, &self.alloc, key, desc, &mut v, ranks.as_deref().map_or(&[], |r| &r[..]));
                self.search.results = v;
            } else {
                // many: on a worker; the rows keep their order until it is done
                let v = self.search.results.clone();
                let alloc = if key == crate::tables::Key::Alloc { self.alloc.clone() } else { Vec::new() };
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    let mut v = v;
                    let mut built = None;
                    let ranks = folder_ranks_if(&t, key, v.len(), ranks, &mut built);
                    crate::tables::sort_nodes(&t, &alloc, key, desc, &mut v, ranks.as_deref().map_or(&[], |r| &r[..]));
                    let _ = tx.send((gen, (c, desc), v, built));
                });
                self.search.sorting = Some(rx);
                text(&self.root, "SearchInfo", "Sorting...");
            }
        }
        self.more_sort();
        self.search.stamp += 1;
    }

    /// The folder ranks of tree `gen`, if they were built.
    fn ranks_for(&self, gen: u64) -> Option<Arc<Vec<u32>>> {
        self.search.ranks.as_ref().filter(|(g, _)| *g == gen).map(|(_, r)| r.clone())
    }

    /// Keep folder ranks a worker built, if they are for the tree on screen.
    fn keep_ranks(&mut self, gen: u64, built: Option<Arc<Vec<u32>>>) {
        if let Some(r) = built.filter(|_| gen == self.tree_gen) {
            self.search.ranks = Some((gen, r));
        }
    }

    /// "12 files (3.1 GB), 4 folders": the scan's matches and the other drives' together, and
    /// what a `dc:`/`da:` search couldn't read.
    pub(crate) fn search_count(&self) -> String {
        if self.search.query.trim().is_empty() {
            return String::new();
        }
        if !Query::parse(&self.search.reads.plain).errors().is_empty() {
            return "Nothing searched: a word can't be understood (see below the box)".to_string();
        }
        if self.search.reads.error.is_some() {
            return "Nothing searched (see below the box)".to_string();
        }
        let (mut files, mut folders, mut bytes) = (0u64, 0u64, 0u64);
        let mut add = |node: &sprawl_core::Node| {
            if node.is_dir {
                folders += 1;
            } else {
                files += 1;
                bytes += node.size;
            }
        };
        if let Some(t) = self.tree.as_ref().filter(|_| self.search.results_gen == self.tree_gen) {
            for &n in &self.search.results {
                add(&t.nodes[n]);
            }
        }
        let m = &self.search.more;
        let others = m.current_hits();
        for &(k, n) in others {
            if let Some((ix, i)) = m.item(k, n) {
                if ix.is_dir(i) {
                    folders += 1;
                } else {
                    files += 1;
                    bytes += ix.size(i);
                }
            }
        }
        let mut s = match (files, folders) {
            // other drives aren't searched when `dc:`/`da:` is in the query, so reading them is
            // not "nothing yet"
            (0, 0) if m.is_reading() && places_searched(&self.search.reads.dates) => "Nothing yet; still reading drives".to_string(),
            (0, 0) => "Nothing found".to_string(),
            (f, 0) => format!("{} file{}, {}", thousands(f), if f == 1 { "" } else { "s" }, self.size(bytes)),
            (0, d) => format!("{} folder{}", thousands(d), if d == 1 { "" } else { "s" }),
            (f, d) => format!("{} file{} ({}), {} folder{}", thousands(f), if f == 1 { "" } else { "s" }, self.size(bytes), thousands(d), if d == 1 { "" } else { "s" }),
        };
        s += &crate::search_more::capped_note(others.len(), m.total);
        if self.full_tree.is_some() {
            s += "; the scan's part is the filtered view";
        }
        // what a `dc:`/`da:` search read or couldn't, short for this one line (the whole wording
        // is in the no-match panel), and never passed off as a plain "no match"
        s += &count_suffix(&self.search.reads);
        if !places_searched(&self.search.reads.dates) && m.has_places() {
            s += "; dc: and da: search only this scan";
        }
        s
    }

    /// The rows of the Search tab (none while the tree has changed under the results).
    /// The scan's matches are `(node, 0)`; another drive's are `(node, place + 1)`.
    pub(crate) fn search_rows(&self) -> Vec<(usize, usize)> {
        let at = (self.search.stamp, self.tree_gen);
        if let Some((s, rows)) = self.search.merged.borrow().as_ref() {
            if *s == at {
                return rows.clone();
            }
        }
        let len = self.tree.as_ref().map_or(0, |t| t.nodes.len());
        let mine: &[usize] = if self.search.results_gen == self.tree_gen { &self.search.results } else { &[] };
        let mine: Vec<usize> = mine.iter().copied().filter(|&n| n < len).collect();
        let m = &self.search.more;
        let others = m.current_hits();
        let mut rows = Vec::with_capacity(mine.len() + others.len());
        match &self.tree {
            Some(t) if !others.is_empty() && !mine.is_empty() => {
                let (c, desc) = self.search.sort;
                let by = crate::search_more::sort_by(c);
                let (mut i, mut j) = (0, 0);
                // by folder: both lists are in folder order, so a folder's path is built once
                // for its run of rows, not for every comparison
                let mut mine_dir: (Option<usize>, String) = (None, String::new());
                let mut other_dir: (Option<(usize, u32)>, String) = (None, String::new());
                while i < mine.len() && j < others.len() {
                    let (k, n) = others[j];
                    let Some((ix, id)) = m.item(k, n) else {
                        j += 1;
                        continue;
                    };
                    let a = &t.nodes[mine[i]];
                    let o = if by == sprawl_core::compact::SortBy::Folder {
                        if mine_dir.0 != a.parent || mine_dir.1.is_empty() {
                            mine_dir = (a.parent, a.parent.map(|p| t.path_of(p).to_string_lossy().into_owned()).unwrap_or_default());
                        }
                        let here = ix.parent(id).map(|p| (k, p));
                        if other_dir.0 != here || other_dir.1.is_empty() {
                            other_dir = (here, ix.parent(id).map(|p| ix.path_of(p).to_string_lossy().into_owned()).unwrap_or_default());
                        }
                        // the one comparison both sorts use
                        crate::tables::cmp_names(&mine_dir.1, &other_dir.1).then_with(|| crate::tables::cmp_names(&a.name, &ix.name(id)))
                    } else if by == sprawl_core::compact::SortBy::Name {
                        // folders first (the same key the lists were sorted by), then the name
                        let group = |dir| crate::search_more::name_group(dir, desc);
                        group(a.is_dir).cmp(&group(ix.is_dir(id))).then_with(|| crate::tables::cmp_names(&a.name, &ix.name(id)))
                    } else {
                        crate::search_more::scan_val(t, mine[i], by, desc).cmp(&crate::search_more::compact_val(ix, id, by, desc))
                    };
                    let mine_first = if desc { o != std::cmp::Ordering::Less } else { o != std::cmp::Ordering::Greater };
                    if mine_first {
                        rows.push((mine[i], 0));
                        i += 1;
                    } else {
                        rows.push((n, k + 1));
                        j += 1;
                    }
                }
                rows.extend(mine[i..].iter().map(|&n| (n, 0)));
                rows.extend(others[j..].iter().filter(|&&(k, n)| m.item(k, n).is_some()).map(|&(k, n)| (n, k + 1)));
            }
            _ => {
                rows.extend(mine.iter().map(|&n| (n, 0)));
                rows.extend(others.iter().filter(|&&(k, n)| m.item(k, n).is_some()).map(|&(k, n)| (n, k + 1)));
            }
        }
        *self.search.merged.borrow_mut() = Some((at, rows.clone()));
        rows
    }

    /// The matched part of `name` in bold.
    pub(crate) fn name_runs(&self, name: &str) -> String {
        let q = Query::parse(&self.search.query);
        // (a cut inside a character would panic on the window's thread: plain text instead)
        match q.highlight(name).filter(|&(a, b)| a <= b && name.is_char_boundary(a) && name.is_char_boundary(b)) {
            Some((a, b)) => format!(
                r##"<Run Text="{}"/><Run Text="{}" FontWeight="Bold" Foreground="{{$match}}"/><Run Text="{}"/>"##,
                ui::esc(&name[..a]),
                ui::esc(&name[a..b]),
                ui::esc(&name[b..])
            ),
            None => format!(r##"<Run Text="{}"/>"##, ui::esc(name)),
        }
    }

    pub(crate) fn search_row(&self, k: usize) -> String {
        let (n, d) = self.rows[k];
        if d > 0 {
            return self.more_row(k, d - 1, n);
        }
        let t = self.tree.as_ref().unwrap();
        let node = &t.nodes[n];
        let sel = Some(n) == self.selected || self.marked.contains(&n);
        let (bg, sel_bar) = (self.row_bg(k, sel), crate::tables::row_bar(sel));
        let modified = node.modified.map(fmt_time).unwrap_or_default();
        let folder = node.parent.map(|p| t.path_of(p).display().to_string()).unwrap_or_default();
        let (glyph, gc) = if node.is_dir { ("&#xE8B7;", crate::tokens::refer("folder")) } else { ("&#xE7C3;", self.ext_colour_hex(&node.extension())) };
        let runs = self.name_runs(&node.name);
        let full = ui::esc(&t.path_of(n).display().to_string());
        let changed = if self.search.is_changed(self.tree_gen, n, node.parent) {
            r##"<FontIcon Glyph="&#xE895;" FontSize="{$fs11}" Foreground="{$warn}" ToolTipService.ToolTip="Changed since the scan: check it is still there"/>"##
        } else {
            ""
        };
        format!(
            r##"<Grid Height="24" Background="{bg}">{cols}{sel_bar}<StackPanel Orientation="Horizontal" Spacing="8" Margin="10,0,0,0" ToolTipService.ToolTip="{full}"><FontIcon Glyph="{glyph}" FontSize="{{$fs13}}" Foreground="{gc}"/><TextBlock VerticalAlignment="Center" FontSize="{{$fs12}}" TextTrimming="CharacterEllipsis">{runs}</TextBlock>{changed}</StackPanel>{c1}{c2}{c3}<TextBlock Grid.Column="4" Visibility="{fv}" Text="{short}" FontSize="{{$fs12}}" Foreground="{{$text3}}" VerticalAlignment="Center" Margin="10,0" TextTrimming="CharacterEllipsis" ToolTipService.ToolTip="{folder}"/></Grid>"##,
            short = ui::esc(&crate::format::mid_path(&folder, self.tables.folder_chars)),
            cols = crate::tables::col_defs(&self.search_cols()),
            fv = if self.search_col_shown(4) { "Visible" } else { "Collapsed" },
            c1 = if self.search_col_shown(1) { cell(1, &self.size(node.size)) } else { String::new() },
            c2 = if self.search_col_shown(2) { cell(2, &self.size(self.alloc[n])) } else { String::new() },
            c3 = if self.search_col_shown(3) { cell(3, &modified) } else { String::new() },
            folder = ui::esc(&folder),
        )
    }

    /// Esc in the results: back to the search box.
    pub(crate) fn search_box_focus(&mut self) {
        self.search_ui_build();
        self.search_box_take_focus();
    }

    /// The focus to the Search box, now if it can take it, else on the next frames
    /// ([`State::search_focus_frame`]).
    pub(crate) fn search_box_take_focus(&mut self) {
        if self.search_box_focus_now() {
            crate::bench::note("search box focused");
        }
        // watched for about half a second: the box may not be laid out yet, and the tab button
        // a click came from may take the focus back after its Click
        self.search_focus_wait = 30;
    }

    /// Whether the Search box has the focus after asking for it (false while it is hidden or
    /// not yet laid out).
    fn search_box_focus_now(&self) -> bool {
        use crate::bindings::Microsoft::UI::Xaml::{Controls::TextBox, FocusState};
        let Ok(b) = find::<TextBox>(&self.root, "SearchBox") else { return false };
        b.Focus(FocusState::Programmatic).unwrap_or(false)
    }

    /// Each frame, for a moment after the Search tab opened: the Search box takes the focus if
    /// nothing has it yet or a tab button still does (never from anything the user went to).
    pub(crate) fn search_focus_frame(&mut self) {
        if self.search_focus_wait == 0 {
            return;
        }
        if self.view != View::Search {
            self.search_focus_wait = 0;
            return;
        }
        self.search_focus_wait -= 1;
        use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement};
        use windows_core::Interface;
        let Ok(root) = self.root.cast::<UIElement>() else { return };
        let name = crate::menu::focused(&root).and_then(|f| f.cast::<FrameworkElement>().ok()).and_then(|f| f.Name().ok()).map(|n| n.to_string_lossy());
        // a menu title holds it when the window first opens (WinUI's first focus), as long as no
        // menu is open
        let free = match name.as_deref() {
            None => true,
            Some(n) => n.starts_with("Tab") || n.starts_with("Menu") && !crate::menu::any_open(),
        };
        if free && self.search_box_focus_now() {
            crate::bench::note(&format!("search box focused (from {})", name.as_deref().unwrap_or("nothing")));
        }
    }

    /// Ctrl+F: the Search tab, its box focused.
    pub(crate) fn search_focus(&mut self) {
        if self.view != View::Search {
            self.set_view(View::Search);
        }
        self.search_ui_build();
        self.search_box_state();
        self.more_open();
        self.search_box_take_focus();
    }

    /// Enter or a double-click on a result: a file opens with its program, a folder in
    /// Explorer, if it is still there.
    pub(crate) fn search_open(&mut self, path: &std::path::Path) {
        let Ok(m) = std::fs::metadata(path) else {
            text(&self.root, "StatusText", &format!("{} isn't there any more. Scan again to update the results.", path.display()));
            return;
        };
        self.search_note();
        let explorer = std::env::var_os("SystemRoot").map(|w| std::path::PathBuf::from(w).join("explorer.exe")).unwrap_or_else(|| "explorer.exe".into());
        // Explorer opens a folder in a window and a file with its default program
        let _ = std::process::Command::new(explorer).arg(path).spawn();
        text(&self.root, "StatusText", &format!("{} {}", if m.is_dir() { "Opened" } else { "Opening" }, path.display()));
    }

    /// Whether this tap on `row` is a double-click. WinUI's own double-tap is lost when the
    /// results are redrawn between the two clicks (other drives arriving every half second), so
    /// two taps on the same row within the double-click time count too; `gesture` is WinUI's,
    /// which is not counted again when the second tap already was.
    pub(crate) fn tap_is_double(&mut self, row: (usize, usize), gesture: bool) -> bool {
        let window = std::time::Duration::from_millis(crate::win32::double_click_ms());
        let (double, last) = second_tap(self.search.last_tap, row, std::time::Instant::now(), window, gesture);
        self.search.last_tap = Some(last);
        double
    }

    /// Ctrl+Enter or "Show in the Tree" on a result: show it in the tree.
    pub(crate) fn search_reveal(&mut self, n: usize) {
        self.search_note();
        self.set_view(View::Tree);
        self.reveal(n);
    }

    /// How many results [`State::menu_paths`] would give, without building them.
    pub(crate) fn menu_target_count(&self) -> usize {
        let more = &self.search.more;
        if self.view != View::Search {
            self.selected.map_or(0, |_| 1)
        } else if more.sel.is_none() && more.picks.is_empty() {
            self.scan_targets().len()
        } else {
            more.picks.len().max(1)
        }
    }

    /// The full paths of the results the row menu acts on, in the order shown: the scan's picked
    /// rows, or the rows from another drive or a saved scan that are picked (or were right-clicked).
    /// Elsewhere, the selected row.
    pub(crate) fn menu_paths(&self) -> Vec<std::path::PathBuf> {
        if self.view != View::Search {
            return self.selected_path().into_iter().collect();
        }
        let more = &self.search.more;
        if more.sel.is_none() && more.picks.is_empty() {
            let t = self.tree.as_ref();
            return self.scan_targets().iter().filter_map(|&n| t.filter(|t| n < t.nodes.len()).map(|t| t.path_of(n))).collect();
        }
        self.rows
            .iter()
            .filter_map(|&(n, d)| {
                if d == 0 {
                    return None;
                }
                let (k, id) = (d - 1, n);
                let (ix, i) = more.item(k, id)?;
                let p = ix.path_of(i);
                crate::search_delete::counts(more.sel, &more.picks, (k, id), &p).then_some(p)
            })
            .collect()
    }

    /// The scan's nodes Delete, Open, Show in Explorer and Copy Path act on in Search: the picked
    /// rows that are listed, and the map's tile the menu was opened on. One rule for all of them.
    pub(crate) fn scan_targets(&self) -> Vec<usize> {
        scan_targets(&self.rows, &self.marked, self.selected, self.search.tile)
    }

    /// Whether `marked` or `selected` hold a scan node that is not one of [`State::scan_targets`]
    /// (picked under an earlier list, or left behind by a rescan): Delete then refuses.
    pub(crate) fn scan_picks_hidden(&self) -> bool {
        picks_hidden(&self.scan_targets(), &self.marked)
    }

    /// Picks that are not rows of the list now are dropped (a new list, the keyboard coming down
    /// from the box), so nothing picked is out of sight.
    pub(crate) fn prune_marked(&mut self) {
        if self.view != View::Search {
            return;
        }
        let shown: std::collections::HashSet<usize> = self.rows.iter().filter(|r| r.1 == 0).map(|r| r.0).collect();
        self.marked.retain(|n| shown.contains(n));
    }

    /// "Use as filter": the words go to the filter box (which narrows the whole analysis).
    pub(crate) fn search_to_filter(&mut self) {
        let f = match filter_word(&self.search.query) {
            Ok(f) => f,
            Err(why) => {
                text(&self.root, "SearchInfo", why);
                return;
            }
        };
        if let Ok(b) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::TextBox>(&self.root, "FilterBox") {
            let _ = b.SetText(&f.into());
        }
        self.set_view(View::Files);
    }
}

/// The word "Use as filter" would put in the filter box, or what the Search line says instead.
fn filter_word(query: &str) -> Result<String, &'static str> {
    let q = Query::parse(query);
    // a group is not one pattern: its `<` and `>` would become letters of a filter meant
    // something else, and half of it would be sent
    let words: Vec<&str> = query.split_whitespace().filter(|w| !w.starts_with(['>', '<', '!']) && !w.contains(':')).collect();
    if words.is_empty() || q.kind == Kind::Folders || Query::has_group(query) {
        return Err("Only names and patterns can become a filter (not sizes, ! or folder:).");
    }
    // a search's space means "and", which a filter can't say
    if words.len() > 1 {
        return Err("A filter takes one pattern (or a|b); it can't say \"this and that\".");
    }
    Ok(words[0].to_string())
}

type Tap = ((usize, usize), std::time::Instant, bool);

/// Whether a tap on `row` at `now` is the second of a double-click, and the tap to remember.
fn second_tap(last: Option<Tap>, row: (usize, usize), now: std::time::Instant, window: std::time::Duration, gesture: bool) -> (bool, Tap) {
    let recent = last.filter(|(r, t, _)| *r == row && now.duration_since(*t) <= window);
    let taken = recent.is_some_and(|(_, _, taken)| taken);
    let double = if gesture { !taken } else { recent.is_some() && !taken };
    (double, (row, now, double || (gesture && taken)))
}

#[cfg(test)]
mod filter_tests {
    use super::filter_word;

    #[test]
    fn a_group_is_not_a_filter() {
        // the group's halves must not be cut into a pattern
        assert!(filter_word("<report 2024>|draft").is_err());
        assert!(filter_word("<a <b c> d>|e").is_err());
        // a pattern on its own still goes
        assert_eq!(filter_word("*.iso").unwrap(), "*.iso");
        assert_eq!(filter_word("*.log|*.tmp").unwrap(), "*.log|*.tmp");
        // a bare size is still a size, not a group
        assert!(filter_word(">1gb").is_err());
        assert!(filter_word("<10mb").is_err());
        assert!(filter_word("report 2024").is_err());
        assert!(filter_word("folder:cache").is_err());
        assert!(filter_word("").is_err());
    }
}

#[cfg(test)]
mod col_tests {
    use super::{hidden_from, hidden_to, sort_for_hidden, COLS, COL_KEYS, COL_MENU};

    /// Hiding the column the list is sorted by moves the sort to Name; any other hidden column
    /// leaves it.
    #[test]
    fn a_sort_never_stays_on_a_hidden_column() {
        assert_eq!(sort_for_hidden((1, true), 0b00010), (0, false), "Size hidden");
        assert_eq!(sort_for_hidden((4, false), 0b10000), (0, false), "Folder hidden");
        assert_eq!(sort_for_hidden((1, true), 0b10100), (1, true), "another column hidden");
        assert_eq!(sort_for_hidden((0, true), 0b11110), (0, true), "Name is never hidden");
        assert_eq!(sort_for_hidden((9, true), 0b11111), (9, true), "no such column: untouched");
    }

    #[test]
    fn the_hidden_columns_are_remembered_as_words() {
        assert_eq!(hidden_from(""), 0);
        assert_eq!(hidden_from("alloc,folder"), 0b10100);
        assert_eq!(hidden_to(0b10100), "alloc,folder");
        // Name can't be hidden, unknown words and spaces don't matter
        assert_eq!(hidden_from("name, size ,nonsense"), 0b10);
        assert_eq!(hidden_to(0b00001), "");
        for mask in 0..32u8 {
            assert_eq!(hidden_from(&hidden_to(mask)), mask & !1, "{mask:05b}");
        }
    }

    #[test]
    fn the_menu_lists_every_column_but_name() {
        assert_eq!(COL_KEYS.len(), COLS.len());
        assert_eq!(COL_MENU.iter().map(|m| m.2).collect::<Vec<_>>(), [1, 2, 3, 4]);
    }
}

#[cfg(test)]
mod places_tests {
    use super::places_searched;
    use sprawl_core::query::ReadDates;

    /// `dc:` or `da:` in the query: no other drive or saved scan is searched, so its rows can't
    /// be listed without the dates. Neither word: they are, as before.
    #[test]
    fn a_date_word_keeps_the_search_to_the_scan() {
        assert!(places_searched(&ReadDates::default()));
        assert!(!places_searched(&ReadDates { created: Some((-1, i64::MAX)), accessed: None }));
        assert!(!places_searched(&ReadDates { created: None, accessed: Some((-1, i64::MAX)) }));
        assert!(!places_searched(&ReadDates { created: Some((-1, 0)), accessed: Some((0, 1)) }));
    }
}

#[cfg(test)]
mod reads_tests {
    use super::{count_suffix, none_headline, read_filter, read_filter_with, reads_for, ReadOutcome, Reads};
    use std::sync::atomic::AtomicBool;
    use std::time::SystemTime;

    fn csv_tree() -> sprawl_core::Tree {
        sprawl_core::Tree {
            volumes: Vec::new(),
            root_path: std::path::PathBuf::from(r"C:\"),
            nodes: vec![sprawl_core::Node { name: r"C:\".into(), is_dir: true, ..Default::default() }],
            errors: 0,
            method: sprawl_core::ScanMethod::Csv,
        }
    }

    /// What a reading limit left out survives a redraw of the same words, and goes when the words
    /// change (times-3).
    #[test]
    fn a_redraw_does_not_wipe_what_the_last_search_said() {
        let now = SystemTime::now();
        let tree = csv_tree();
        let mut with_scan = tree.clone();
        with_scan.method = sprawl_core::ScanMethod::Walk;
        let mut old = reads_for("*.mkv da:<2024", now, Some(&with_scan), &Reads::default());
        assert!(old.error.is_none(), "{:?}", old.error);
        assert!(old.dates.accessed.is_some());
        old.notes = vec!["300 matches not checked for when it was last opened: the reading limit ran out".into()];
        old.unread = 300;
        old.unknown = 2;
        old.partial = 5;
        let again = reads_for("*.mkv da:<2024", now, Some(&with_scan), &old);
        assert_eq!(again.notes, old.notes, "a redraw keeps the notes");
        assert_eq!((again.unread, again.unknown, again.partial), (300, 2, 5));
        // another word: the old search's counts are about other matches
        let changed = reads_for("*.mkv da:<2024 holiday", now, Some(&with_scan), &old);
        assert!(changed.notes.is_empty() && changed.unread == 0 && changed.unknown == 0 && changed.partial == 0);
    }

    /// `dc:`/`da:` with no scan says so in the reads themselves, so the hint under the box and
    /// the count line see it and a redraw keeps it (times-6), and a CSV view says where it came
    /// from and reads nothing (times-9).
    #[test]
    fn a_view_that_has_no_times_says_so_before_anything_is_read() {
        let now = SystemTime::now();
        let none = reads_for("*.mkv da:<2024", now, None, &Reads::default());
        assert!(none.error.as_deref().unwrap().contains("needs a scan"), "{:?}", none.error);
        assert!(none.dates.accessed.is_some(), "the words are understood; the view is what refuses");
        let redraw = reads_for("*.mkv da:<2024", now, None, &none);
        assert!(redraw.error.is_some(), "a redraw keeps the error");
        let csv = reads_for("*.mkv dc:2024", now, Some(&csv_tree()), &Reads::default());
        assert!(csv.error.as_deref().unwrap().contains("CSV"), "{:?}", csv.error);
        // an ordinary scan, and no date word, have no error
        let mut walk = csv_tree();
        walk.method = sprawl_core::ScanMethod::Walk;
        assert!(reads_for("*.mkv dc:2024", now, Some(&walk), &Reads::default()).error.is_none());
        assert!(reads_for("*.mkv", now, None, &Reads::default()).error.is_none());
    }

    /// A panel with nothing in it says "couldn't be checked" when matches were never read, and
    /// "Nothing matches" only when they really were all checked (times-5).
    #[test]
    fn a_no_match_panel_never_passes_off_an_unchecked_match() {
        assert_eq!(none_headline("q", "this scan", &Reads::default()), "Nothing matches \u{201c}q\u{201d} in this scan.");
        let noted = Reads { notes: vec!["dc: and da: weren't checked: 52,000 matches".into()], too_many: 52_000, unread: 52_000, ..Default::default() };
        assert_eq!(none_headline("q", "this scan", &noted), "Nothing could be checked for \u{201c}q\u{201d} in this scan.");
        let partly = Reads { notes: vec!["1 match not fully read".into()], ..Default::default() };
        assert!(none_headline("q", "this scan", &partly).starts_with("Nothing could be checked"));
    }

    /// A search the user has moved on from stops reading the disk (times-4).
    #[test]
    fn a_newer_search_stops_an_older_ones_reading() {
        let dir = std::env::temp_dir().join(format!("sprawl-read-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        std::fs::write(dir.join("sub").join("b.txt"), b"x").unwrap();
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let hits: Vec<usize> = tree.nodes.iter().enumerate().filter(|&(i, n)| i != sprawl_core::ROOT && !n.is_dir).map(|(i, _)| i).collect();
        assert_eq!(hits.len(), 2);
        let dates = sprawl_core::query::ReadDates { created: Some((0, i64::MAX)), accessed: None };
        assert!(read_filter(&tree, hits.clone(), &dates, &AtomicBool::new(true)).is_none(), "a cancelled search reads nothing and says nothing");
        assert!(read_filter(&tree, hits, &dates, &AtomicBool::new(false)).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A search too big to read says how many matches weren't checked, never "not read (reading
    /// limit)" and never a plain "Nothing found" (times-5).
    #[test]
    fn a_search_too_big_to_read_says_how_many() {
        let too_many = Reads { too_many: 52_000, unread: 52_000, ..Default::default() };
        let line = count_suffix(&too_many);
        assert!(line.contains("52,000 not checked (too many to read: narrow the search)"), "{line}");
        assert!(!line.contains("reading limit"), "{line}");
        assert!(count_suffix(&Reads { unread: 3, ..Default::default() }).contains("3 not read (reading limit)"));
        assert!(count_suffix(&Reads { unknown: 2, ..Default::default() }).contains("2 with no time to read"));
        assert!(count_suffix(&Reads::default()).is_empty());
    }

    /// A folder the reading limit stopped is not dropped from the list without a word, and is
    /// the reading limit, not a partial read; a partly read match has its own count in the line
    /// (times-12).
    #[test]
    fn a_match_read_only_in_part_is_counted_and_said() {
        use sprawl_advisor::used::{Accessed, ANSWER_BUDGET, PER_PATH};
        let dir = std::env::temp_dir().join(format!("sprawl-read-part-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let big = dir.join("big");
        std::fs::create_dir_all(&big).unwrap();
        // one more than the most entries read for one path
        for i in 0..=PER_PATH {
            std::fs::write(big.join(format!("f{i}.txt")), b"x").unwrap();
        }
        std::fs::write(dir.join("small.txt"), b"x").unwrap();
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let node = |name: &str| tree.nodes.iter().position(|n| n.name == name).unwrap_or_else(|| panic!("{name} in the scan"));
        let hits = vec![node("big"), node("small.txt")];
        let dates = sprawl_core::query::ReadDates { created: None, accessed: Some((0, i64::MAX)) };
        let outcome = read_filter_with(&tree, hits, &dates, &AtomicBool::new(false), Some(Accessed::with_budget(ANSWER_BUDGET)));
        let Some(ReadOutcome::Kept { kept, unread, partial, .. }) = outcome else { panic!("an answer") };
        assert_eq!(kept, vec![node("small.txt")], "the small match is still listed");
        assert_eq!((unread, partial), (1, 0), "the folder the limit stopped is the reading limit, not a partial read");
        let line = count_suffix(&Reads { unread, partial, ..Default::default() });
        assert!(line.contains("1 not read (reading limit)"), "{line}");
        // a part skipped or unreadable has its own count, and the panel never calls it a no
        let line = count_suffix(&Reads { partial: 20, ..Default::default() });
        assert!(line.contains("; 20 not fully read"), "{line}");
        assert!(!line.contains("reading limit"), "{line}");
        assert!(none_headline("q", "this scan", &Reads { partial: 2, ..Default::default() }).starts_with("Nothing could be checked"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Matches read and out of the range are said as such; only the unread are "not checked"
    /// (times-20).
    #[test]
    fn a_range_with_no_match_says_how_many_were_read() {
        let dir = std::env::temp_dir().join(format!("sprawl-read-decided-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["a.txt", "b.txt", "gone.txt"] {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let hits: Vec<usize> = tree.nodes.iter().enumerate().filter(|&(i, n)| i != sprawl_core::ROOT && !n.is_dir).map(|(i, _)| i).collect();
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        let dates = sprawl_core::query::ReadDates { created: Some((0, 86_400)), accessed: None };
        let Some(ReadOutcome::Kept { kept, notes, unknown, decided, .. }) = read_filter(&tree, hits, &dates, &AtomicBool::new(false)) else { panic!("an answer") };
        assert!(kept.is_empty());
        assert_eq!((unknown, decided), (1, 2));
        let reads = Reads { notes, unknown, decided, ..Default::default() };
        let line = none_headline("q", "this scan", &reads);
        assert!(line.starts_with("None of the 2 read for") && line.contains("1 couldn't be checked"), "{line}");
        // nothing decided stays "could be checked"
        let unread = Reads { unread: 3, notes: vec!["3 not read".into()], ..Default::default() };
        assert!(none_headline("q", "this scan", &unread).starts_with("Nothing could be checked"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A drive whose times are never read says so for each match, not "too many to read"
    /// (times-18).
    #[test]
    fn a_drive_that_is_never_read_is_said_before_too_many() {
        use sprawl_advisor::used::Accessed;
        let nodes = vec![sprawl_core::Node { name: "x".into(), ..Default::default() }];
        let tree = sprawl_core::Tree { volumes: Vec::new(), root_path: format!("{b}{b}server{b}share", b = char::from(92)).into(), nodes, errors: 0, method: Default::default() };
        for dates in [
            sprawl_core::query::ReadDates { created: None, accessed: Some((0, i64::MAX)) },
            sprawl_core::query::ReadDates { created: Some((0, i64::MAX)), accessed: None },
        ] {
            let hits = vec![0usize; super::READ_MOST + 1];
            match read_filter_with(&tree, hits, &dates, &AtomicBool::new(false), Some(Accessed::with_budget(10))) {
                Some(ReadOutcome::Kept { unknown, .. }) => assert_eq!(unknown, super::READ_MOST + 1),
                _ => panic!("the drive's reason, not too many"),
            }
        }
    }

    /// A PC that keeps no last-access times says so before "too many to read": narrowing the
    /// search could never help (times-18).
    #[test]
    fn no_access_times_is_said_before_too_many() {
        use sprawl_advisor::used::Accessed;
        let mut a = Accessed::with_budget(10);
        a.set_kept_for_test(false);
        let dates = sprawl_core::query::ReadDates { created: None, accessed: Some((0, i64::MAX)) };
        let nodes = vec![sprawl_core::Node { name: "x".into(), ..Default::default() }];
        let tree = sprawl_core::Tree { volumes: Vec::new(), root_path: "C:\\".into(), nodes, errors: 0, method: Default::default() };
        let hits = vec![0usize; super::READ_MOST + 1];
        match read_filter_with(&tree, hits, &dates, &AtomicBool::new(false), Some(a)) {
            Some(ReadOutcome::Kept { notes, unknown, .. }) => {
                assert_eq!(unknown, super::READ_MOST + 1);
                assert!(notes.iter().any(|n| n.contains("last-access updates turned off")), "{notes:?}");
            }
            _ => panic!("the machine's reason, not too many"),
        }
    }
}

#[cfg(test)]
mod tap_tests {
    use super::second_tap;
    use std::time::{Duration, Instant};

    #[test]
    fn two_taps_on_a_row_are_one_double_click() {
        let w = Duration::from_millis(500);
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        // WinUI lost the gesture: two plain taps
        let (d, last) = second_tap(None, (3, 0), t0, w, false);
        assert!(!d);
        let (d, last) = second_tap(Some(last), (3, 0), ms(200), w, false);
        assert!(d);
        // a third tap starts over, not a second double-click
        let (d, _) = second_tap(Some(last), (3, 0), ms(300), w, false);
        assert!(!d);
        // WinUI's own double-tap after one tap: once
        let (_, last) = second_tap(None, (3, 0), t0, w, false);
        let (d, last) = second_tap(Some(last), (3, 0), ms(200), w, true);
        assert!(d);
        let (d, _) = second_tap(Some(last), (3, 0), ms(210), w, true);
        assert!(!d, "the same double-click is not taken twice");
        // another row, or too slow: not a double-click
        let (_, last) = second_tap(None, (3, 0), t0, w, false);
        assert!(!second_tap(Some(last), (4, 0), ms(100), w, false).0);
        assert!(!second_tap(Some(last), (3, 0), ms(900), w, false).0);
    }
}

/// The folder ranks to sort `rows` matches by `key`: the ones given, or (by folder, for many
/// matches) built now and put in `built` to keep. Few matches don't need them.
fn folder_ranks_if(tree: &sprawl_core::Tree, key: crate::tables::Key, rows: usize, given: Option<Arc<Vec<u32>>>, built: &mut Option<Arc<Vec<u32>>>) -> Option<Arc<Vec<u32>>> {
    if key != crate::tables::Key::Folder || given.is_some() || rows <= SORT_HERE {
        return given;
    }
    let r = Arc::new(sprawl_core::search::folder_ranks(tree));
    *built = Some(r.clone());
    Some(r)
}

/// The scan's rows that are picked (`marked` and `selected`), in the order shown. A row from
/// another drive or a saved scan is never one of them, whatever its number.
pub(crate) fn picked_in_order(rows: &[(usize, usize)], marked: &std::collections::BTreeSet<usize>, selected: Option<usize>) -> Vec<usize> {
    let mut seen = std::collections::BTreeSet::new();
    rows.iter().filter(|r| r.1 == 0 && (marked.contains(&r.0) || selected == Some(r.0)) && seen.insert(r.0)).map(|r| r.0).collect()
}

/// Each of `paths` as Show in Explorer takes it: itself when `exists`, else its nearest folder that
/// does (each folder once). Also how many were mapped to a folder, and how many have none.
pub(crate) fn nearest_existing(paths: &[std::path::PathBuf], exists: impl Fn(&std::path::Path) -> bool) -> (Vec<std::path::PathBuf>, usize, usize) {
    let (mut out, mut seen, mut gone, mut nowhere) = (Vec::new(), std::collections::HashSet::new(), 0, 0);
    for p in paths {
        if exists(p) {
            if seen.insert(p.clone()) {
                out.push(p.clone());
            }
            continue;
        }
        match p.ancestors().skip(1).find(|a| exists(a)) {
            Some(a) => {
                gone += 1;
                if seen.insert(a.to_path_buf()) {
                    out.push(a.to_path_buf());
                }
            }
            None => nowhere += 1,
        }
    }
    (out, gone, nowhere)
}

/// What Show in Explorer says when some of the `total` results were gone.
pub(crate) fn gone_note(total: usize, gone: usize, nowhere: usize) -> String {
    match (total, gone, nowhere) {
        (1, 1, _) => "That is gone now; its nearest folder is open.".into(),
        (1, _, _) => "That path is gone.".into(),
        (_, g, 0) => format!("{g} are gone now; their nearest folders are open."),
        (_, 0, n) => format!("{n} paths are gone."),
        (_, g, n) => format!("{g} are gone now; their nearest folders are open. {n} paths are gone."),
    }
}

/// What Search's menu and Delete act on in the scan: [`picked_in_order`], then the map's `tile`
/// when it is the selected node and not a row already.
pub(crate) fn scan_targets(rows: &[(usize, usize)], marked: &std::collections::BTreeSet<usize>, selected: Option<usize>, tile: Option<usize>) -> Vec<usize> {
    let mut v = picked_in_order(rows, marked, selected);
    if let Some(t) = tile.filter(|&t| selected == Some(t) && !v.contains(&t)) {
        v.push(t);
    }
    v
}

/// A picked node (in `marked`) that is not among `targets`. A stale `selected` that is not a
/// target is simply not deleted, so it does not count. Linear in the picks.
pub(crate) fn picks_hidden(targets: &[usize], marked: &std::collections::BTreeSet<usize>) -> bool {
    let set: std::collections::HashSet<usize> = targets.iter().copied().collect();
    marked.iter().any(|n| !set.contains(n))
}

#[cfg(test)]
mod picked_tests {
    use super::{picked_in_order, picks_hidden, scan_targets};

    #[test]
    fn a_gone_result_shows_its_nearest_folder_and_says_so() {
        use super::{gone_note, nearest_existing};
        use std::path::{Path, PathBuf};
        let there = |p: &Path| p == Path::new(r"D:\a") || p == Path::new(r"D:\a\x.txt") || p == Path::new(r"D:\");
        let ps: Vec<PathBuf> = [r"D:\a\x.txt", r"D:\a\old\y.txt", r"D:\a\old\z.txt", r"Q:\no\w.txt"].iter().map(PathBuf::from).collect();
        let (out, gone, nowhere) = nearest_existing(&ps, there);
        assert_eq!(out, [PathBuf::from(r"D:\a\x.txt"), PathBuf::from(r"D:\a")]);
        assert_eq!((gone, nowhere), (2, 1));
        assert_eq!(gone_note(1, 1, 0), "That is gone now; its nearest folder is open.");
        assert_eq!(gone_note(1, 0, 1), "That path is gone.");
        assert!(gone_note(4, 2, 1).contains("2 are gone"));
    }

    #[test]
    fn delete_and_the_menu_share_one_set_of_targets() {
        let rows = [(9, 0), (5, 1), (5, 0), (2, 0)];
        let m = |v: &[usize]| v.iter().copied().collect::<std::collections::BTreeSet<_>>();
        // a node that is not a row (a parent folder after a rescan, a folder picked in Tree View)
        let t = scan_targets(&rows, &m(&[]), Some(77), None);
        assert!(t.is_empty());
        // ... nor does a stale selected node hide or join real picks (Ctrl+click after ROOT)
        let t = scan_targets(&rows, &m(&[9, 2]), Some(0), None);
        assert_eq!(t, [9, 2]);
        assert!(!picks_hidden(&t, &m(&[9, 2])), "a stale selected is not a target and not refused");
        assert!(picks_hidden(&t, &m(&[9, 2, 0])), "a non-row in marked still refuses");
        // a row picked earlier that the new list does not show
        let t = scan_targets(&rows, &m(&[2, 40]), Some(2), None);
        assert_eq!(t, [2]);
        assert!(picks_hidden(&t, &m(&[2, 40])));
        // everything picked is listed: nothing hidden
        let t = scan_targets(&rows, &m(&[9, 2]), Some(2), None);
        assert!(!picks_hidden(&t, &m(&[9, 2])));
        // the map's tile is a target while it is the selected node, and only then
        assert_eq!(scan_targets(&rows, &m(&[]), Some(77), Some(77)), [77]);
        assert!(!picks_hidden(&[77], &m(&[])));
        assert!(scan_targets(&rows, &m(&[]), Some(5), Some(77)).len() == 1, "the tile is stale once something else is selected");
        assert_eq!(scan_targets(&rows, &m(&[]), Some(5), Some(5)), [5]);
    }

    #[test]
    fn only_the_scans_rows_in_the_order_shown() {
        // node 5 of the scan and item 5 of another drive (place 0) are different things
        let rows = [(9, 0), (5, 1), (5, 0), (2, 0), (7, 2)];
        let marked = [2, 5, 7].into_iter().collect();
        assert_eq!(picked_in_order(&rows, &marked, Some(9)), [9, 5, 2]);
        assert_eq!(picked_in_order(&rows, &Default::default(), Some(7)), Vec::<usize>::new(), "7 is only another drive's row");
        assert_eq!(picked_in_order(&rows, &Default::default(), None), Vec::<usize>::new());
    }
}

#[cfg(test)]
mod menu_tests {
    use super::{can_open, item_applies, MAX_OPEN, SCAN_ONLY};

    /// Open starts a program or an Explorer window for each result, so only up to the limit at
    /// once; one more is refused.
    #[test]
    fn open_goes_up_to_the_limit_and_no_further() {
        assert!(can_open(0) && can_open(1) && can_open(MAX_OPEN));
        assert!(!can_open(MAX_OPEN + 1));
        assert!(!can_open(MAX_OPEN * 2));
    }

    /// The items that go by path stay on for a row from another drive or a saved scan; the ones
    /// that need the scan on screen are off there, and every one of them says why.
    #[test]
    fn what_the_row_menu_can_do_with_another_drives_row() {
        for &(name, on, off) in SCAN_ONLY {
            assert!(item_applies(name, true), "{name}");
            assert!(!item_applies(name, false), "{name}");
            assert!(!on.is_empty() && !off.is_empty(), "{name}");
        }
        for name in ["cOpen", "cExplore", "cCmd", "cCopyPath", "cDelete", "cDeleteForever", "cShell", "cScan"] {
            assert!(item_applies(name, true), "{name}");
            assert!(item_applies(name, false), "{name}");
        }
        assert!(!item_applies("cOverwrite", true) && !item_applies("cOverwrite", false), "Search offers no overwrite");
    }

    /// Every row of the right-click menu is in one list or the other: a new item has to choose
    /// whether it works by path or needs the scan on screen.
    #[test]
    fn every_row_menu_item_is_known_here() {
        for &(item, _) in crate::ROW_MENU {
            match item {
                "cOverwrite" => assert!(!item_applies(item, true)),
                "cOpen" | "cExplore" | "cCmd" | "cCopyPath" | "cDelete" | "cDeleteForever" | "cShell" | "cScan" => assert!(item_applies(item, false), "{item}"),
                _ => assert!(SCAN_ONLY.iter().any(|&(n, _, _)| n == item), "{item} is neither by path nor listed as scan-only"),
            }
        }
    }
}
