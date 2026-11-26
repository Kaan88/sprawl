//! The Search tab beyond the scan: other fixed drives the user switches on (read from the MFT
//! as administrator, else folder by folder), and saved scans of places no drive read covers.
//! Their matches sit in the same list as the scan's.
//!
//! What was read is kept as a [`CompactIndex`] (names, sizes, times; no [`sprawl_core::Tree`]):
//! its item ids are its own, not node numbers of the scan on screen. A row from here is
//! `(id, place + 1)` and is never put in `selected` or `marked`, never looked up in `self.tree`,
//! and gets the table's row menu (the items that need the scan on screen shown, off). Several can
//! be picked (`picks`, keyed by the row's full path, so a re-sort can't move a pick to another
//! item) and deleted by path: see `search_delete.rs`, which checks each item on the disk as it is
//! now instead of against a scan. Places live in slots that are never reused, so a row's place
//! number always means the same place (or none, once it is gone).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use sprawl_core::compact::{CompactIndex, SortBy};
use sprawl_core::search::Query;

use crate::bindings::Microsoft::UI::Xaml::Controls::Border;
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::ButtonBase;
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement, Visibility};
use crate::format::{fmt_time, thousands};
use crate::menu::{Item, Row};
use crate::state::{find, keep, load, with, State, View};
use crate::ui;

/// Matches from other drives kept at most (the first ones in the sort order): 100,000 unless
/// the user changed it (Settings > Search, `searchMaxHits`).
pub(crate) fn max_hits() -> usize {
    sprawl_advisor::tuning::get(&sprawl_advisor::tuning::SEARCH_MAX_HITS) as usize
}
/// What the status adds when other drives have more matches than are listed: the list is the
/// first `shown` of all `total` in the sort order on screen (a sort orders every match before the
/// cap is taken, so another sort lists others).
pub(crate) fn capped_note(shown: usize, total: usize) -> String {
    if shown > 0 && total > shown {
        format!("; showing the first {} of {} on other drives", thousands(shown as u64), thousands(total as u64))
    } else {
        String::new()
    }
}

/// A place that isn't the scan on screen, as a search-only index.
pub(crate) struct Place {
    pub(crate) ix: Arc<CompactIndex>,
    /// From a saved scan: its date.
    pub(crate) as_of: Option<SystemTime>,
    /// When it was read (or loaded).
    pub(crate) read_at: SystemTime,
    /// Folders it could not read.
    pub(crate) errors: u64,
    /// The drive it is ("D:\"); None for a saved scan.
    pub(crate) drive: Option<String>,
}

impl Place {
    /// The item `id`, if it is one of this place's.
    fn has(&self, id: usize) -> bool {
        id < self.ix.len()
    }
}

/// A drive or saved scan being read.
struct Reading {
    label: String,
    drive: Option<String>,
    progress: Option<Arc<sprawl_core::Progress>>,
    rx: Receiver<Result<Place, String>>,
}

/// (places gen, query, sort, every slot's matches sorted, the first MAX_HITS merged, total)
type Found = (u64, String, (usize, bool), Option<(Vec<Vec<u32>>, Vec<(usize, usize)>, usize)>);

#[derive(Default)]
pub(crate) struct More {
    /// Never reused: a row's place number always means the same place (None: gone).
    places: Vec<Option<Arc<Place>>>,
    /// Bumped when `places` changes.
    pub(crate) gen: u64,
    snapshots_tried: bool,
    reading: Vec<Reading>,
    /// Reads that failed: (drive or label, why).
    failed: Vec<(String, String)>,
    /// Drives switched on (read, being read, or failed) and the ones switched off; both
    /// remembered between runs (`searchDrivesOn`, `searchDrivesOff`).
    drives_on: BTreeSet<String>,
    drives_off: BTreeSet<String>,
    remembered: bool,
    running: Option<Receiver<Found>>,
    /// Every slot's matches (what the scan on screen covers left out), in `hits_sort` order:
    /// the cap is taken from them again when the sort changes.
    all: Arc<Vec<Vec<u32>>>,
    /// (place, id) in the Search sort order, at most MAX_HITS; and how many there were.
    pub(crate) hits: Vec<(usize, usize)>,
    pub(crate) total: usize,
    hits_gen: u64,
    hits_query: String,
    hits_sort: (usize, bool),
    resorting: Option<Receiver<Found>>,
    /// The row from another drive that is picked (place, id): the one the keyboard is on.
    pub(crate) sel: Option<(usize, usize)>,
    /// The rows picked with Ctrl/Shift, by full path (never mixed with the scan's `marked`).
    pub(crate) picks: crate::search_delete::Picks,
    /// What was deleted from the places, as (slot, run of ids): gone from the rows and from
    /// every later search, though the places keep what they read.
    gone: Vec<(usize, std::ops::Range<u32>)>,
    /// A Delete measuring its folders before it asks.
    pub(crate) prep: Option<crate::search_delete::Prep>,
    /// When the strip last showed the reading progress.
    drawn: Option<Instant>,
    /// The drive toggles and the strip as built: while they stay the same only texts change,
    /// so buttons don't move or flicker.
    toggles_shape: String,
    strip_shape: String,
}

impl More {
    /// The page was made again (the theme changed): the strip and the toggles are built again.
    pub(crate) fn restyle(&mut self) {
        self.toggles_shape.clear();
        self.strip_shape.clear();
    }

    /// The matches for the places as they are now (none while they changed under them).
    pub(crate) fn current_hits(&self) -> &[(usize, usize)] {
        if self.hits_gen == self.gen {
            &self.hits
        } else {
            &[]
        }
    }

    pub(crate) fn place(&self, k: usize) -> Option<&Place> {
        self.places.get(k).and_then(|p| p.as_deref())
    }

    /// Place `k` and its item `id`, if both still exist.
    pub(crate) fn item(&self, k: usize, id: usize) -> Option<(&CompactIndex, u32)> {
        let p = self.place(k).filter(|p| p.has(id))?;
        if self.is_gone(k, id as u32) {
            return None;
        }
        Some((&p.ix, id as u32))
    }

    /// The item was deleted from the disk by Sprawl since the place was read.
    fn is_gone(&self, k: usize, id: u32) -> bool {
        self.gone.iter().any(|(pk, r)| *pk == k && r.contains(&id))
    }

    /// Items in every place (their roots not counted).
    pub(crate) fn items_searched(&self) -> u64 {
        self.places.iter().flatten().map(|p| p.ix.len().saturating_sub(1) as u64).sum()
    }

    /// Other drives' rows are being sorted again after a sort change.
    pub(crate) fn is_resorting(&self) -> bool {
        self.resorting.is_some()
    }

    /// The other drives switched on, `D:` style.
    pub(crate) fn on_list(&self) -> Vec<String> {
        self.drives_on.iter().map(|d| d.trim_end_matches('\\').to_string()).collect()
    }

    pub(crate) fn is_reading(&self) -> bool {
        !self.reading.is_empty()
    }

    /// The drives and saved scans being read now, by name (for the activity indicator).
    pub(crate) fn reading_now(&self) -> Vec<String> {
        self.reading.iter().map(|r| r.label.clone()).collect()
    }

    /// Matches from other drives being sorted again on a worker.
    pub(crate) fn sorting_now(&self) -> bool {
        self.resorting.is_some()
    }

    pub(crate) fn has_places(&self) -> bool {
        self.places.iter().any(Option::is_some)
    }

    /// Nothing is picked and the keyboard is on no row from here.
    pub(crate) fn drop_picks(&mut self) {
        self.sel = None;
        self.picks.clear();
    }

    /// Drop the picks (and the keyboard's row) that are not among the listed matches any more;
    /// how many picks went.
    pub(crate) fn prune_picks(&mut self) -> usize {
        let listed: std::collections::HashSet<(usize, usize)> = self.hits.iter().copied().collect();
        if self.sel.is_some_and(|s| !listed.contains(&s)) {
            self.sel = None;
        }
        self.picks.retain(|p| listed.contains(&(p.place, p.id)))
    }

    /// The places changed: matches and the picked row are for the old ones.
    fn bump(&mut self) {
        self.gen += 1;
        self.sel = None;
        self.picks.clear();
        self.resorting = None;
    }
}

/// "D:\ 1,234,567 items", or "C:\ (saved 12 Nov)".
fn reading_label(r: &Reading) -> String {
    match &r.progress {
        Some(p) if p.items() > 0 => format!("{} {} items", r.label, thousands(p.items())),
        Some(_) => format!("{} starting", r.label),
        None => r.label.clone(),
    }
}

pub(crate) fn norm(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches('\\').to_lowercase()
}

/// `a` is `b` or inside it.
pub(crate) fn within(a: &str, b: &str) -> bool {
    a == b || a.strip_prefix(b).is_some_and(|r| r.starts_with('\\')) || (b.len() == 2 && a.starts_with(b))
}

/// The items of `ix` inside folder `cur` (lower case, no trailing `\`), as an id range (a
/// folder and what is inside it are one run of ids); None when `cur` is not in it.
pub(crate) fn covered(ix: &CompactIndex, cur: &str) -> Option<std::ops::Range<u32>> {
    let root = norm(ix.root_path());
    if !within(cur, &root) {
        return None;
    }
    let n = ix.len() as u32;
    let rel = cur.get(root.len()..).unwrap_or("").trim_start_matches('\\');
    let mut id = 0u32;
    for comp in rel.split('\\').filter(|c| !c.is_empty()) {
        let d = ix.depth(id);
        let mut j = id + 1;
        let mut found = None;
        while j < n && ix.depth(j) > d {
            if ix.parent(j) == Some(id) && ix.name(j).to_lowercase() == comp {
                found = Some(j);
                break;
            }
            j += 1;
        }
        id = found?;
    }
    let d = ix.depth(id);
    let mut end = id + 1;
    while end < n && ix.depth(end) > d {
        end += 1;
    }
    Some(id..end)
}

/// The Search column as a compact index sorts.
pub(crate) fn sort_by(col: usize) -> SortBy {
    match crate::search::COLS[col.min(crate::search::COLS.len() - 1)].key() {
        crate::tables::Key::Name => SortBy::Name,
        crate::tables::Key::Modified => SortBy::Modified,
        crate::tables::Key::Folder => SortBy::Folder,
        _ => SortBy::Size,
    }
}

/// A row's value for the sort, comparable across places (and with the scan's rows), the way
/// [`CompactIndex::sort`] orders one place.
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone)]
pub(crate) enum SortVal {
    Num(u64),
    Time(Option<SystemTime>),
    /// (the folder's path, then the name), lower case
    Text(String, String),
    /// By name: folders first whichever way round (see [`name_group`]), then the name, lower case
    Name(u8, String),
}

/// The part of a name sort's key that puts folders before files. The sort takes the biggest key
/// first when `desc`, so a folder is the biggest group then; the names inside a group run A-Z or
/// Z-A as the key does.
pub(crate) fn name_group(is_dir: bool, desc: bool) -> u8 {
    u8::from(is_dir == desc)
}

pub(crate) fn compact_val(ix: &CompactIndex, id: u32, by: SortBy, desc: bool) -> SortVal {
    match by {
        SortBy::Size => SortVal::Num(ix.size(id)),
        SortBy::Modified => SortVal::Time(ix.modified(id)),
        SortBy::Name => SortVal::Name(name_group(ix.is_dir(id), desc), ix.name(id).to_lowercase()),
        SortBy::Folder => SortVal::Text(ix.parent(id).map(|p| ix.path_of(p).to_string_lossy().to_lowercase()).unwrap_or_default(), ix.name(id).to_lowercase()),
    }
}

/// The same for a node of the scan on screen, so its rows and other drives' rows merge.
pub(crate) fn scan_val(t: &sprawl_core::Tree, n: usize, by: SortBy, desc: bool) -> SortVal {
    let x = &t.nodes[n];
    match by {
        SortBy::Size => SortVal::Num(x.size),
        SortBy::Modified => SortVal::Time(x.modified),
        SortBy::Name => SortVal::Name(name_group(x.is_dir, desc), x.name.to_lowercase()),
        SortBy::Folder => SortVal::Text(x.parent.map(|p| t.path_of(p).to_string_lossy().to_lowercase()).unwrap_or_default(), x.name.to_lowercase()),
    }
}

/// The first `cap` of several sorted lists, merged in the same order (`desc`: biggest key
/// first). Equal keys keep the list order.
pub(crate) fn merge_capped<K: Ord>(lists: &[Vec<u32>], key: impl Fn(usize, u32) -> K, desc: bool, cap: usize) -> Vec<(usize, usize)> {
    let mut pos = vec![0usize; lists.len()];
    let mut heads: Vec<Option<K>> = lists.iter().enumerate().map(|(k, l)| l.first().map(|&id| key(k, id))).collect();
    let mut out = Vec::new();
    while out.len() < cap {
        let mut best: Option<usize> = None;
        for k in 0..lists.len() {
            let Some(h) = &heads[k] else { continue };
            let better = match best.and_then(|b| heads[b].as_ref()) {
                None => true,
                Some(b) => {
                    if desc {
                        h > b
                    } else {
                        h < b
                    }
                }
            };
            if better {
                best = Some(k);
            }
        }
        let Some(k) = best else { break };
        out.push((k, lists[k][pos[k]] as usize));
        pos[k] += 1;
        heads[k] = lists[k].get(pos[k]).map(|&id| key(k, id));
    }
    out
}

/// Every slot's matches sorted, and the first MAX_HITS of them merged.
fn sort_and_cap(places: &[Option<Arc<Place>>], mut all: Vec<Vec<u32>>, (c, desc): (usize, bool), cancel: &AtomicBool) -> Option<(Vec<Vec<u32>>, Vec<(usize, usize)>)> {
    let by = sort_by(c);
    for (k, hits) in all.iter_mut().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        if let Some(p) = places.get(k).and_then(|p| p.as_ref()) {
            p.ix.sort(hits, by, desc);
        }
    }
    let shown = merge_capped(&all, |k, id| places[k].as_ref().map(|p| compact_val(&p.ix, id, by, desc)), desc, max_hits());
    Some((all, shown))
}

/// The drives to read when Search opens: those switched on, or every fixed drive when "every
/// drive" is on, except those switched off, the scan's own drive, and those already read,
/// being read or failed (`busy`).
pub(crate) fn drives_to_read(fixed: &[String], cur: Option<&str>, every: bool, on: &BTreeSet<String>, off: &BTreeSet<String>, busy: &dyn Fn(&str) -> bool) -> Vec<String> {
    fixed
        .iter()
        .filter(|d| cur.is_none_or(|c| norm(Path::new(d)) != c))
        .filter(|d| on.contains(*d) || (every && !off.contains(*d)))
        .filter(|d| !busy(d))
        .cloned()
        .collect()
}

fn set_list(s: &str) -> BTreeSet<String> {
    s.split(',').map(str::trim).filter(|d| !d.is_empty()).map(str::to_ascii_uppercase).collect()
}

/// Reconcile the data already held by Search before starting new work. Dropping a saved
/// scan's receiver ensures an old load cannot publish after a scope change.
fn reconcile_scope(m: &mut More, on: BTreeSet<String>, off: BTreeSet<String>) {
    let disabled: Vec<_> = m.drives_on.difference(&on).cloned().collect();
    for d in disabled { more_drop_drive(m, &d); }
    m.drives_on = on;
    m.drives_off = off;
    m.reading.retain(|r| r.drive.is_some());
    for p in &mut m.places {
        if p.as_ref().is_some_and(|p| p.drive.is_none()) { *p = None; }
    }
    m.snapshots_tried = false;
    m.running = None;
    m.hits.clear();
    m.all = Arc::new(Vec::new());
    m.total = 0;
    m.bump();
}

impl State {
    fn current_root(&self) -> Option<String> {
        self.tree.as_ref().filter(|t| t.root_path.as_os_str() != sprawl_core::MULTIPLE).map(|t| norm(&t.root_path))
    }

    /// The fixed drives (removable and network ones are not offered).
    fn fixed_drives() -> Vec<String> {
        crate::win32::drives().into_iter().filter(|d| d.kind == 3).map(|d| d.root).collect()
    }

    /// The fixed drives, for Settings.
    pub(crate) fn fixed_drives_pub() -> Vec<String> {
        Self::fixed_drives()
    }

    /// The Settings dialog and the Search menu both reconcile the same live scope.
    pub(crate) fn more_settings_changed(&mut self) {
        let on = set_list(self.settings.get("searchDrivesOn").unwrap_or(""));
        let off = set_list(self.settings.get("searchDrivesOff").unwrap_or(""));
        self.search.more.remembered = true;
        reconcile_scope(&mut self.search.more, on, off);
        self.search.stamp += 1;
        self.cancel_prep();
        if self.view == crate::state::View::Search && !crate::settings::read_only() {
            self.more_open();
            self.search_run();
        }
    }

    /// The tab opened: read the drives switched on, and load the saved scans (once a session)
    /// that the current scan and those drives don't cover.
    pub(crate) fn more_open(&mut self) {
        if crate::settings::read_only() { return; }
        if !self.search.more.remembered {
            self.search.more.remembered = true;
            let m = &mut self.search.more;
            m.drives_on = set_list(self.settings.get("searchDrivesOn").unwrap_or(""));
            m.drives_off = set_list(self.settings.get("searchDrivesOff").unwrap_or(""));
        }
        self.more_forget_covered();
        self.more_all_drives();
        let count = sprawl_advisor::tuning::get(&sprawl_advisor::tuning::SEARCH_SNAPSHOTS) as usize;
        if self.search.more.snapshots_tried || self.settings.bool("snapshots") != Some(true) || count == 0 {
            self.search.more.snapshots_tried = true;
            self.render_more();
            return;
        }
        self.search.more.snapshots_tried = true;
        let Some(base) = sprawl_advisor::history::base_dir() else { return };
        let cur = self.current_root();
        // the newest full snapshot of each root, newest roots first
        let mut newest: Vec<sprawl_advisor::history::Entry> = Vec::new();
        for e in sprawl_advisor::history::list(&base).into_iter().rev().filter(|e| e.full) {
            if newest.iter().any(|n| norm(&n.root) == norm(&e.root)) {
                continue;
            }
            let r = norm(&e.root);
            if cur.as_ref().is_some_and(|c| within(c, &r) || within(&r, c)) {
                continue;
            }
            // a drive read now is newer than any saved scan on it
            if self.search.more.drives_on.iter().any(|d| within(&r, &norm(Path::new(d)))) {
                continue;
            }
            newest.push(e);
        }
        newest.truncate(count);
        for e in newest {
            let (tx, rx) = channel();
            let label = format!("{} (saved {})", e.root.display(), fmt_time(e.taken));
            std::thread::spawn(move || {
                let r = sprawl_advisor::history::load(&e).map_err(|x| x.to_string()).map(|t| {
                    let ix = Arc::new(CompactIndex::build(&t));
                    Place { ix, as_of: Some(e.taken), read_at: SystemTime::now(), errors: t.errors, drive: None }
                });
                let _ = tx.send(r);
            });
            self.search.more.reading.push(Reading { label, drive: None, progress: None, rx });
        }
        self.render_more();
    }

    fn more_all_drives(&mut self) {
        let cur = self.current_root();
        let m = &self.search.more;
        let busy = |d: &str| {
            m.reading.iter().any(|r| r.drive.as_deref() == Some(d))
                || m.places.iter().flatten().any(|p| p.drive.as_deref() == Some(d))
                || m.failed.iter().any(|f| f.0 == d)
        };
        let want = drives_to_read(&Self::fixed_drives(), cur.as_deref(), false, &m.drives_on, &m.drives_off, &busy);
        for d in want {
            self.search.more.drives_on.insert(d.clone());
            self.more_read(d);
        }
    }

    /// Read drive `root` (again): the place it had stays searchable until the new one arrives.
    fn more_read(&mut self, root: String) {
        let m = &mut self.search.more;
        if m.reading.iter().any(|r| r.drive.as_deref() == Some(&root)) {
            return;
        }
        m.failed.retain(|f| f.0 != root);
        let progress = sprawl_advisor::exclusions::progress();
        let (tx, rx) = channel();
        let (p2, r2) = (progress.clone(), root.clone());
        std::thread::spawn(move || {
            let r = match sprawl_core::scan_auto(Path::new(&r2), &p2) {
                // switched off meanwhile: nobody waits for it
                _ if p2.is_cancelled() => return,
                Ok(t) => {
                    let ix = Arc::new(CompactIndex::build(&t));
                    Ok(Place { ix, as_of: None, read_at: SystemTime::now(), errors: t.errors, drive: Some(r2) })
                }
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(r);
        });
        m.reading.push(Reading { label: root.clone(), drive: Some(root), progress: Some(progress), rx });
        self.render_more();
    }

    /// A drive toggle: switched on, it is read; switched off, its read stops (what it would
    /// have found is dropped) and its place goes.
    pub(crate) fn more_drive(&mut self, letter: char) {
        if crate::settings::read_only() { return; }
        let root = format!("{letter}:\\");
        let mut on = self.search.more.drives_on.clone();
        let mut off = self.search.more.drives_off.clone();
        if on.remove(&root) {
            off.insert(root);
        } else {
            off.remove(&root);
            on.insert(root);
        }
        let mut staged = self.settings.clone();
        staged.set("searchDrivesOn", on.into_iter().collect::<Vec<_>>().join(","));
        staged.set("searchDrivesOff", off.into_iter().collect::<Vec<_>>().join(","));
        match staged.save() {
            Ok(()) => { self.settings = staged; self.more_settings_changed(); }
            Err(e) => crate::state::text(&self.root, "SearchInfo", &format!("Couldn't save Search scope: {e}")),
        }
    }

    /// Read a drive again (Refresh, or Try again after a failure).
    pub(crate) fn more_refresh(&mut self, root: String) {
        if self.search.more.drives_on.contains(&root) {
            self.more_read(root);
        }
    }

    /// A scan replaced the tree: saved scans it now covers go (they would only repeat it,
    /// older). A whole drive read now still adds the rest of the volume.
    pub(crate) fn more_forget_covered(&mut self) {
        let Some(cur) = self.current_root() else { return };
        let m = &mut self.search.more;
        let mut gone = false;
        for slot in m.places.iter_mut() {
            if let Some(p) = slot {
                let r = norm(p.ix.root_path());
                if p.as_of.is_some() && (within(&cur, &r) || within(&r, &cur)) {
                    *slot = None;
                    gone = true;
                }
            }
        }
        if gone {
            m.bump();
            self.search.stamp += 1;
        }
    }

    /// Search the places for the current words (called with the main search).
    pub(crate) fn more_run(&mut self, cancel: Arc<AtomicBool>) {
        let cur = self.current_root();
        let m = &mut self.search.more;
        m.running = None;
        m.resorting = None;
        let q = Query::parse(&self.search.reads.plain);
        // `dc:`/`da:` read each match's own time, which only the scan on screen can do: other
        // drives and saved scans are not searched at all for such a query (search::places_searched)
        if q.is_empty()
            || !q.errors().is_empty()
            || self.search.reads.error.is_some()
            || !crate::search::places_searched(&self.search.reads.dates)
            || !m.has_places()
        {
            m.hits.clear();
            m.all = Arc::new(Vec::new());
            m.total = 0;
            m.hits_gen = m.gen;
            m.hits_query = self.search.query.clone();
            // nothing is listed now: nothing is picked
            m.drop_picks();
            self.cancel_prep();
            self.render_more();
            return;
        }
        let places = m.places.clone();
        let gone = m.gone.clone();
        let gen = m.gen;
        let text_q = self.search.query.clone();
        let sort = self.search.sort;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut all: Vec<Vec<u32>> = Vec::with_capacity(places.len());
            for (slot, p) in places.iter().enumerate() {
                let Some(p) = p else {
                    all.push(Vec::new());
                    continue;
                };
                let Some(mut v) = p.ix.search(&q, &cancel) else {
                    let _ = tx.send((gen, text_q, sort, None));
                    return;
                };
                // what the scan on screen already shows is left out
                if let Some(r) = cur.as_deref().and_then(|c| covered(&p.ix, c)) {
                    v.retain(|id| !r.contains(id));
                }
                // what Sprawl deleted since the place was read
                if gone.iter().any(|(k, _)| *k == slot) {
                    v.retain(|id| !gone.iter().any(|(k, r)| *k == slot && r.contains(id)));
                }
                all.push(v);
            }
            let total = all.iter().map(Vec::len).sum();
            let r = sort_and_cap(&places, all, sort, &cancel).map(|(all, shown)| (all, shown, total));
            let _ = tx.send((gen, text_q, sort, r));
        });
        m.running = Some(rx);
    }

    /// Each frame: places that arrived, results that arrived.
    pub(crate) fn more_poll(&mut self) {
        let mut changed = false;
        let mut i = 0;
        while i < self.search.more.reading.len() {
            let m = &mut self.search.more;
            match m.reading[i].rx.try_recv() {
                Err(TryRecvError::Empty) => i += 1,
                got => {
                    let r = m.reading.remove(i);
                    let key = r.drive.clone().unwrap_or_else(|| r.label.clone());
                    match got {
                        Ok(Ok(p)) => {
                            // a drive read again replaces what it had
                            if let Some(d) = &p.drive {
                                for slot in m.places.iter_mut() {
                                    if slot.as_ref().is_some_and(|q| q.drive.as_ref() == Some(d)) {
                                        *slot = None;
                                    }
                                }
                            }
                            m.places.push(Some(Arc::new(p)));
                        }
                        Ok(Err(why)) => m.failed.push((key, why)),
                        Err(_) => m.failed.push((key, "the read stopped".into())),
                    }
                    m.bump();
                    changed = true;
                }
            }
        }
        if changed {
            self.cancel_prep();
            self.search.stamp += 1;
            self.more_forget_covered();
            // a drive that finished reading only matters to a query that searches it: `dc:`/`da:`
            // search the scan on screen, and re-running would read every match's time again
            if self.view == View::Search && crate::search::places_searched(&self.search.reads.dates) {
                self.search_run();
            }
            self.render_more();
        } else if self.search.more.is_reading() && self.view == View::Search && self.search.more.drawn.is_none_or(|t| t.elapsed().as_millis() > 500) {
            // the counts going up
            self.render_more();
        }
        for which in [false, true] {
            let m = &self.search.more;
            let rx = if which { &m.resorting } else { &m.running };
            let Some(Ok((gen, q, sort, r))) = rx.as_ref().map(|rx| rx.try_recv()) else { continue };
            let m = &mut self.search.more;
            if which {
                m.resorting = None;
            } else {
                m.running = None;
            }
            if let Some((all, hits, total)) = r.filter(|_| gen == m.gen && q == self.search.query) {
                m.all = Arc::new(all);
                m.hits = hits;
                m.total = total;
                m.hits_gen = gen;
                m.hits_query = q;
                m.hits_sort = sort;
                let mut dropped = 0;
                if !which {
                    // a new list of results: what was picked is for the old one (a re-sort
                    // keeps the picks, which are by path)
                    m.sel = None;
                    m.picks.clear();
                } else {
                    // a re-sort takes the cap again: a pick the new order cut off is not on the
                    // list any more, so it is not picked any more
                    dropped = m.prune_picks();
                }
                // a Delete still measuring went with the picks it was for
                if !which || dropped > 0 {
                    self.cancel_prep();
                }
                if sort != self.search.sort {
                    self.more_sort();
                }
                self.search.stamp += 1;
                if self.view == View::Search {
                    self.refresh_rows();
                    // the scan's own rows may still be sorting
                    let info = if self.search.sorting.is_some() {
                        "Sorting...".to_string()
                    } else if dropped > 0 {
                        format!("{} picked result{} left the list when it was sorted again (it lists only the first {} from other drives)", thousands(dropped as u64), if dropped == 1 { "" } else { "s" }, thousands(self.search.more.hits.len() as u64))
                    } else {
                        self.search_count()
                    };
                    crate::state::text(&self.root, "SearchInfo", &info);
                }
            }
        }
    }

    /// The sort changed (a click on a column): the matches from other drives are sorted again
    /// and the first MAX_HITS taken again in the new order, on a worker.
    pub(crate) fn more_sort(&mut self) {
        let sort = self.search.sort;
        let m = &mut self.search.more;
        if m.hits_gen != m.gen || m.hits_query != self.search.query || m.hits_sort == sort || m.all.is_empty() {
            return;
        }
        let places = m.places.clone();
        let all = m.all.clone();
        let (gen, q, total) = (m.gen, m.hits_query.clone(), m.total);
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let r = sort_and_cap(&places, (*all).clone(), sort, &AtomicBool::new(false)).map(|(all, shown)| (all, shown, total));
            let _ = tx.send((gen, q, sort, r));
        });
        m.resorting = Some(rx);
        if self.view == View::Search {
            crate::state::text(&self.root, "SearchInfo", "Sorting...");
        }
    }

    /// A click on a row from another drive (the table's row `row`): picked (never part of the
    /// scan's picks); Ctrl adds or takes away a row, Shift picks the run from the row the
    /// keyboard is on; a double-click opens it (a file with its program, a folder in Explorer).
    pub(crate) fn more_tapped(&mut self, row: usize, k: usize, id: usize, double: bool) {
        if self.search.more.item(k, id).is_none() {
            return;
        }
        let (ctrl, shift) = if double { (false, false) } else { crate::win32::ctrl_shift_down() };
        let before = self.search.more.sel;
        // the scan's rows and these never share a pick
        self.selected = None;
        self.marked.clear();
        if shift {
            let from = before.and_then(|(p, n)| self.rows.iter().position(|r| r.1 == p + 1 && r.0 == n)).unwrap_or(row);
            self.more_pick_run(from.min(row), from.max(row));
            self.search.more.sel = Some((k, id));
        } else if ctrl {
            if self.search.more.picks.is_empty() {
                if let Some((p, n)) = before {
                    self.more_pick(p, n);
                }
            }
            let now = self.search.more.item(k, id).map(|(ix, i)| ix.path_of(i)).map(|p| crate::search_delete::key(&p)).unwrap_or_default();
            if self.search.more.picks.contains_key(&now) {
                self.search.more.picks.remove(&now);
            } else {
                self.more_pick(k, id);
            }
            self.search.more.sel = Some((k, id));
        } else {
            self.search.more.picks.clear();
            self.search.more.sel = Some((k, id));
        }
        if ctrl || shift {
            self.more_picked_note();
            self.focus_main();
            self.refresh_rows();
            return;
        }
        if double {
            self.more_command('O', k, id);
        } else {
            self.more_show_path(k, id);
        }
        self.focus_main();
        self.refresh_rows();
    }

    /// Pick the item `id` of place `k`, by its full path, with what the row shows of it (size,
    /// date, file or folder): a Delete holds it to that. False when it is not an item or the
    /// picks are full.
    pub(crate) fn more_pick(&mut self, k: usize, id: usize) -> bool {
        let m = &self.search.more;
        let Some((ix, i)) = m.item(k, id) else { return false };
        let pick = crate::search_delete::Pick::of(ix, i, k, id, m.gen);
        self.search.more.picks.add(pick)
    }

    /// Pick every row from other drives among the table's rows `a..=b`.
    pub(crate) fn more_pick_run(&mut self, a: usize, b: usize) {
        let run: Vec<(usize, usize)> = self.rows[a.min(self.rows.len())..(b + 1).min(self.rows.len())].iter().filter(|r| r.1 > 0).map(|r| (r.1 - 1, r.0)).collect();
        for (k, id) in run {
            if !self.more_pick(k, id) {
                break;
            }
        }
    }

    /// Say how many are picked (and that the picks are full).
    pub(crate) fn more_picked_note(&self) {
        // (only the picks: the row the keyboard is on counts only when nothing is picked, and
        // then there is nothing to say)
        let n = self.search.more.picks.len();
        let full = if n >= crate::search_delete::MAX_PICKS { format!(" (the most Sprawl picks at once: {})", crate::search_delete::MAX_PICKS) } else { String::new() };
        let note = match n {
            0 => String::new(),
            1 => format!("1 result picked{full}"),
            n => format!("{} results picked{full}", thousands(n as u64)),
        };
        crate::state::text(&self.root, "SearchInfo", &if note.is_empty() { self.search_count() } else { note });
    }

    /// Paths a Delete went through: those no longer on the disk leave the results (and every
    /// later search). A folder that is still there, even if only part of it went, stays.
    pub(crate) fn more_forget_deleted(&mut self, paths: &[std::path::PathBuf]) {
        let mut any = false;
        for path in paths {
            if !matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound) {
                continue;
            }
            let cur = norm(path);
            let m = &mut self.search.more;
            for k in 0..m.places.len() {
                let Some(r) = m.place(k).and_then(|p| covered(&p.ix, &cur)) else { continue };
                // (a root is never an item of its own to delete: covered() of a drive root is all)
                if r.len() == m.place(k).map_or(0, |p| p.ix.len()) {
                    continue;
                }
                any = true;
                let r2 = r.clone();
                let all = Arc::make_mut(&mut m.all);
                if let Some(list) = all.get_mut(k) {
                    list.retain(|id| !r2.contains(id));
                }
                m.hits.retain(|&(pk, id)| !(pk == k && r.contains(&(id as u32))));
                m.gone.push((k, r));
            }
        }
        if !any {
            return;
        }
        let m = &mut self.search.more;
        m.total = m.all.iter().map(Vec::len).sum();
        m.sel = None;
        m.picks.clear();
        self.search.stamp += 1;
        if self.view == View::Search {
            self.refresh_rows();
            let info = self.search_count();
            crate::state::text(&self.root, "SearchInfo", &info);
        }
    }

    /// The full path of a row from another drive, under the table.
    pub(crate) fn more_show_path(&self, k: usize, id: usize) {
        if let Some((ix, i)) = self.search.more.item(k, id) {
            let p = &self.search.more.place(k);
            let when = p.map(|p| p.as_of.unwrap_or(p.read_at)).map(fmt_time).unwrap_or_default();
            crate::state::text(&self.root, "FootLeft", &format!("{}   -   {}   (read {when})", ix.path_of(i).display(), self.size(ix.size(i))));
        }
    }

    /// A row from another drive, in the same columns as the scan's.
    pub(crate) fn more_row(&self, row: usize, k: usize, id: usize) -> String {
        let Some(p) = self.search.more.place(k) else { return String::new() };
        let Some((ix, i)) = self.search.more.item(k, id) else { return String::new() };
        let name = ix.name(i);
        let full = ix.path_of(i).display().to_string();
        let picked = crate::search_delete::counts(self.search.more.sel, &self.search.more.picks, (k, id), Path::new(&full));
        let (bg, sel_bar) = (self.row_bg(row, picked), crate::tables::row_bar(picked));
        let folder = ix.parent(i).map(|q| ix.path_of(q).display().to_string()).unwrap_or_default();
        let ext = name.rsplit_once('.').filter(|(a, b)| !a.is_empty() && !b.is_empty()).map(|(_, e)| e.to_lowercase()).unwrap_or_default();
        let (glyph, gc) = if ix.is_dir(i) { ("&#xE8B7;", crate::tokens::refer("folder")) } else { ("&#xE7C3;", self.ext_colour_hex(&ext)) };
        let saved = match p.as_of {
            Some(t) => format!(
                r##"<Border Background="{{$warnwash}}" CornerRadius="{{$rc3}}" Padding="5,0" VerticalAlignment="Center" ToolTipService.ToolTip="From a saved scan: how it was then, not checked now"><TextBlock Text="saved {}" FontSize="{{$fs11}}" Foreground="{{$warn}}"/></Border>"##,
                ui::esc(&fmt_time(t))
            ),
            None => String::new(),
        };
        format!(
            r##"<Grid Height="{height}" Background="{bg}">{cols}{sel_bar}<StackPanel Orientation="Horizontal" Spacing="8" Margin="10,0,0,0" ToolTipService.ToolTip="{full}"><FontIcon Glyph="{glyph}" FontSize="{{$fs13}}" Foreground="{gc}"/><TextBlock VerticalAlignment="Center" FontSize="{{$fs12}}" TextTrimming="CharacterEllipsis">{runs}</TextBlock>{saved}</StackPanel>{c1}{c2}{c3}<TextBlock Grid.Column="4" Visibility="{fv}" Text="{short}" FontSize="{{$fs12}}" Foreground="{{$text3}}" VerticalAlignment="Center" Margin="10,0" TextTrimming="CharacterEllipsis" ToolTipService.ToolTip="{folder}"/></Grid>"##,
            height = crate::state::row_height(),
            short = ui::esc(&crate::format::mid_path(&folder, self.tables.folder_chars)),
            cols = crate::tables::col_defs(&self.search_cols()),
            runs = self.name_runs(&name),
            fv = if self.search_col_shown(4) { "Visible" } else { "Collapsed" },
            c1 = if self.search_col_shown(1) { crate::tables::cell(1, &self.size(ix.size(i))) } else { String::new() },
            c2 = if self.search_col_shown(2) { crate::tables::cell(2, "") } else { String::new() },
            c3 = if self.search_col_shown(3) { crate::tables::cell(3, &ix.modified(i).map(fmt_time).unwrap_or_default()) } else { String::new() },
            folder = ui::esc(&folder),
            full = ui::esc(&full),
        )
    }

    pub(crate) fn more_command(&mut self, kind: char, k: usize, id: usize) {
        let Some(path) = self.search.more.item(k, id).map(|(ix, i)| ix.path_of(i)) else { return };
        match kind {
            'E' => {
                // show it in Explorer if it is still there, else its nearest folder that is
                let target = path.ancestors().find(|p| p.exists()).map(Path::to_path_buf);
                match target {
                    Some(t) if t == path => {
                        crate::win32::reveal(&t);
                    }
                    Some(t) => {
                        crate::win32::reveal(&t);
                        crate::state::text(&self.root, "SearchInfo", "That is gone now; its nearest folder is open.");
                    }
                    None => crate::state::text(&self.root, "SearchInfo", "That path is gone."),
                }
            }
            'O' => self.search_open(&path),
            // a row from another drive: not part of the scan on screen, nothing to read again
            'W' => self.shell_menu(path, false),
            _ => {}
        }
    }

    /// The drive toggles beside the chips, and the strip under the results: what is included,
    /// when each drive was read (with Refresh), what is being read or failed, and what is not.
    pub(crate) fn render_more(&mut self) {
        crate::bench::note("search drives panel drawn");
        self.render_drive_toggles();
        let Ok(host) = find::<Border>(&self.root, "SearchMore") else { return };
        let cur = self.current_root();
        let offer: Vec<String> = Self::fixed_drives().into_iter().filter(|d| cur.as_ref().is_none_or(|c| norm(Path::new(d)) != *c)).collect();
        self.search.more.drawn = Some(Instant::now());
        let m = &self.search.more;
        // (kind, text, the drive a button acts on)
        let mut entries: Vec<(&str, String, Option<String>)> = Vec::new();
        if let Some(t) = &self.tree {
            entries.push(("scan", format!("This scan: {}", t.root_path.display()), None));
        }
        for p in m.places.iter().flatten() {
            let root = p.ix.root_path().display().to_string();
            let errs = if p.errors > 0 { format!(", {} unreadable", thousands(p.errors)) } else { String::new() };
            let items = thousands(p.ix.len().saturating_sub(1) as u64);
            match (&p.as_of, &p.drive) {
                (Some(t), _) => entries.push(("saved", format!("{root}: saved scan of {} ({items} items)", fmt_time(*t)), None)),
                (None, Some(d)) => {
                    let again = m.reading.iter().any(|r| r.drive.as_ref() == Some(d));
                    let kind = if again { "rereading" } else { "read" };
                    entries.push((kind, format!("{root}: read {} ({items} items{errs})", fmt_time(p.read_at)), Some(d.clone())));
                }
                (None, None) => {}
            }
        }
        // the drives being read, on one line
        let reading: Vec<String> = m
            .reading
            .iter()
            // a drive read again shows on its entry above
            .filter(|r| !r.drive.as_ref().is_some_and(|d| m.places.iter().flatten().any(|p| p.drive.as_ref() == Some(d))))
            .map(reading_label)
            .collect();
        if !reading.is_empty() {
            entries.push(("reading", format!("Reading {}", reading.join("   ·   ")), None));
        }
        for (what, why) in &m.failed {
            let d = m.drives_on.contains(what).then(|| what.clone());
            entries.push(("failed", format!("{what} could not be read: {why}"), d));
        }
        let left_out: Vec<&str> = offer.iter().filter(|d| !m.drives_on.contains(*d)).map(String::as_str).collect();
        let mut note = if left_out.is_empty() { String::new() } else { format!("Not included: {}.", left_out.join(", ")) };
        note += "  Removable and network drives are not offered. New files show after a drive is read again.";
        if m.is_reading() && !sprawl_core::is_elevated() {
            note += "  Read folder by folder; as administrator it takes seconds.";
        }
        if entries.is_empty() && offer.is_empty() {
            let _ = host.cast::<UIElement>().and_then(|u| u.SetVisibility(Visibility::Collapsed));
            return;
        }
        let shape = entries.iter().map(|e| format!("{}|{}", e.0, e.2.as_deref().unwrap_or(""))).collect::<Vec<_>>().join(";");
        let built = host.Child().ok().and_then(|c| c.cast::<FrameworkElement>().ok());
        if let Some(fe) = built.filter(|_| shape == self.search.more.strip_shape) {
            for (i, e) in entries.iter().enumerate() {
                crate::state::text(&fe, &format!("SxE{i}"), &e.1);
            }
            crate::state::text(&fe, "SxNote", &note);
            let _ = host.cast::<UIElement>().and_then(|u| u.SetVisibility(Visibility::Visible));
            return;
        }
        let mut x = format!(r##"<StackPanel {} Spacing="2" Padding="8,4,8,6">"##, ui::NS);
        for (i, (kind, line, drive)) in entries.iter().enumerate() {
            let (glyph, colour) = match *kind {
                "scan" => ("&#xE8B7;", crate::tokens::refer("text2")),
                "saved" => ("&#xE81C;", crate::tokens::refer("warn")),
                "failed" => ("&#xE783;", crate::tokens::refer("fail")),
                _ => ("&#xEDA2;", crate::tokens::refer("text2")),
            };
            x += r##"<StackPanel Orientation="Horizontal" Spacing="8">"##;
            if matches!(*kind, "reading" | "rereading") {
                x += r##"<ProgressRing IsActive="True" Width="12" Height="12" MinWidth="0" MinHeight="0" VerticalAlignment="Center"/>"##;
            } else {
                x += &format!(r##"<FontIcon Glyph="{glyph}" FontSize="{{$fs12}}" Foreground="{colour}" VerticalAlignment="Center"/>"##);
            }
            x += &format!(r##"<TextBlock x:Name="SxE{i}" Text="{}" {} FontSize="{{$fs12}}" Foreground="{{$text3}}" VerticalAlignment="Center" TextTrimming="CharacterEllipsis" MaxWidth="700"/>"##, ui::esc(line), crate::look::NUM);
            if drive.is_some() && *kind != "rereading" {
                let label = if *kind == "failed" { "Try again" } else { "Refresh" };
                x += &format!(r##"<Button x:Name="SxR{i}" Content="{label}" FontSize="{{$fs11}}" Padding="8,1" ToolTipService.ToolTip="Read this drive again: new files show, deleted ones go"/>"##);
            }
            if *kind == "rereading" {
                x += r##"<TextBlock Text="reading again..." FontSize="{$fs12}" Foreground="{$faint}" VerticalAlignment="Center"/>"##;
            }
            x += "</StackPanel>";
        }
        x += &format!(r##"<TextBlock x:Name="SxNote" Text="{}" FontSize="{{$fs11}}" Foreground="{{$faint}}" TextWrapping="Wrap"/>"##, ui::esc(&note));
        x += "</StackPanel>";
        let Ok(content) = load::<UIElement>(&x) else { return };
        let _ = host.SetChild(&content);
        self.search.more.strip_shape = shape;
        let _ = host.cast::<UIElement>().and_then(|u| u.SetVisibility(Visibility::Visible));
        let Ok(fe) = content.cast::<FrameworkElement>() else { return };
        for (i, (_, _, drive)) in entries.into_iter().enumerate() {
            let Some(d) = drive else { continue };
            if let Ok(b) = find::<ButtonBase>(&fe, &format!("SxR{i}")) {
                if let Ok(t) = b.Click(move |_, _| {
                    let d = d.clone();
                    with(|s| s.more_refresh(d));
                }) {
                    keep(t);
                }
            }
        }
    }

    /// "Search in: this scan + D:" beside the chips: its menu switches each fixed drive (not
    /// the scan's own) on or off.
    fn render_drive_toggles(&mut self) {
        let Ok(host) = find::<Border>(&self.root, "SearchDrives") else { return };
        let cur = self.current_root();
        let offer: Vec<String> = Self::fixed_drives().into_iter().filter(|d| cur.as_ref().is_none_or(|c| norm(Path::new(d)) != *c)).collect();
        let m = &self.search.more;
        let shape = offer.iter().map(|d| format!("{d}{}", m.drives_on.contains(d))).collect::<Vec<_>>().join(",") + if self.tree.is_some() { ";scan" } else { "" };
        if shape == m.toggles_shape && host.Child().is_ok() {
            return;
        }
        let on: Vec<String> = offer.iter().filter(|d| m.drives_on.contains(*d)).map(|d| d.trim_end_matches('\\').to_string()).collect();
        let label = match (self.tree.is_some(), on.is_empty()) {
            (true, true) => "this scan".to_string(),
            (true, false) => format!("this scan + {}", on.join(", ")),
            (false, false) => on.join(", "),
            (false, true) => "nothing yet".to_string(),
        };
        // drawn and run as every menu (menu.rs); a drive's check is its click's to flip
        let mut rows = Vec::new();
        if self.tree.is_some() {
            rows.push(Row::Item(Item::new("", "This scan: always searched").off(true)));
            rows.push(Row::Sep);
        }
        for d in &offer {
            let letter = d.chars().next().unwrap_or('C');
            rows.push(Row::Toggle {
                name: format!("SxD{letter}"),
                text: format!("Also drive {letter}:"),
                checked: m.drives_on.contains(d),
                key: String::new(),
                mods: String::new(),
                tip: format!("Search {} too: it is read now and kept in memory until Sprawl closes (remembered for next time). Choose again to leave it out.", ui::esc(d)),
            });
        }
        if offer.is_empty() {
            rows.push(Row::Item(Item::new("", "No other fixed drive to add").off(true)));
        }
        let x = format!(
            r##"<DropDownButton {} x:Name="SxDrives" FontSize="{{$fs12}}" Padding="10,3,6,3" ToolTipService.ToolTip="Where Search looks: this scan, and the other drives you switch on"><TextBlock Text="Search in: {}" FontSize="{{$fs12}}"/><DropDownButton.Flyout>{}</DropDownButton.Flyout></DropDownButton>"##,
            ui::NS,
            ui::esc(&label),
            crate::menu::xaml("SxDrivesFly", "SxDrives", "Bottom", &rows)
        );
        let Ok(content) = load::<UIElement>(&x) else { return };
        let _ = host.SetChild(&content);
        self.search.more.toggles_shape = shape;
        let Ok(fe) = content.cast::<FrameworkElement>() else { return };
        if let Err(e) = crate::menu::bind(&fe, "SxDrivesFly", crate::menu::How::Button) {
            eprintln!("search in: {e:?}");
        }
        // a drive's check, however it is flipped (a click, Enter, Space, a screen reader's
        // Toggle), switches the drive to match it: once, and not at all when it is so already
        for d in &offer {
            let letter = d.chars().next().unwrap_or('C');
            let root = d.clone();
            let _ = crate::menu::on_mark(&fe, &format!("SxD{letter}"), move |on| {
                with(|s| {
                    if s.search.more.drives_on.contains(&root) != on {
                        s.more_drive(letter);
                    }
                });
            });
        }
    }
}

/// Drive `root` switched off: its read is stopped and forgotten, its place and failure go.
fn more_drop_drive(m: &mut More, root: &str) {
    for r in m.reading.iter().filter(|r| r.drive.as_deref() == Some(root)) {
        if let Some(p) = &r.progress {
            p.cancel();
        }
    }
    // the receiver goes with it: whatever the read sends now is dropped
    m.reading.retain(|r| r.drive.as_deref() != Some(root));
    m.failed.retain(|f| f.0 != root);
    for slot in m.places.iter_mut() {
        if slot.as_ref().is_some_and(|p| p.drive.as_deref() == Some(root)) {
            *slot = None;
        }
    }
    m.bump();
}

use windows_core::Interface;

#[cfg(test)]
mod tests {
    use super::*;
    use sprawl_core::{Node, Tree};
    use std::path::PathBuf;

    fn tree(root: &str, names: &[(&str, bool, usize, u64)]) -> Tree {
        let mut nodes = vec![Node { name: root.into(), is_dir: true, ..Default::default() }];
        for &(name, dir, parent, size) in names {
            let i = nodes.len();
            nodes.push(Node { name: name.into(), is_dir: dir, size, parent: Some(parent), ..Default::default() });
            nodes[parent].children.push(i);
        }
        let mut t = Tree { volumes: Vec::new(), root_path: PathBuf::from(root), nodes, errors: 0, method: Default::default() };
        t.compute_totals();
        t
    }

    fn place(t: &Tree, drive: &str) -> Option<Arc<Place>> {
        Some(Arc::new(Place { ix: Arc::new(CompactIndex::build(t)), as_of: None, read_at: SystemTime::now(), errors: 0, drive: Some(drive.into()) }))
    }

    #[test]
    fn a_scope_change_cancels_removed_drives_and_discards_old_saved_scans() {
        let t = tree("D:\\", &[("one", false, 1, 1)]);
        let mut m = More::default();
        m.drives_on = set_list("d:\\,E:\\");
        m.places = vec![place(&t, "D:\\"), place(&t, "E:\\")];
        let (tx, rx) = channel();
        let (saved_tx, saved_rx) = channel();
        let progress = sprawl_core::Progress::new();
        m.reading.push(Reading { label: "D".into(), drive: Some("D:\\".into()), progress: Some(progress.clone()), rx });
        m.reading.push(Reading { label: "Saved".into(), drive: None, progress: None, rx: saved_rx });
        m.snapshots_tried = true;
        reconcile_scope(&mut m, set_list("E:\\"), set_list("D:\\"));
        assert!(progress.is_cancelled() && m.places[0].is_none() && m.places[1].is_some());
        assert!(!m.snapshots_tried && m.reading.is_empty());
        assert!(tx.send(Err("old drive".into())).is_err());
        assert!(saved_tx.send(Err("old snapshot".into())).is_err());
    }

    #[test]
    fn the_first_rows_are_taken_again_in_each_order() {
        // the cap must come from all the matches in the order asked for, not from the rows
        // kept under another order (SRCH-6)
        let d = tree("D:\\", &[("a.txt", false, 0, 5), ("b.txt", false, 0, 50), ("z.txt", false, 0, 1)]);
        let e = tree("E:\\", &[("c.txt", false, 0, 40), ("y.txt", false, 0, 2)]);
        let places = vec![place(&d, "D:\\"), None, place(&e, "E:\\")];
        let all: Vec<Vec<u32>> = places.iter().map(|p| p.as_ref().map_or(Vec::new(), |p| p.ix.search(&Query::parse("file:"), &AtomicBool::new(false)).unwrap())).collect();
        let names = |shown: &[(usize, usize)]| -> Vec<String> { shown.iter().map(|&(k, id)| places[k].as_ref().unwrap().ix.name(id as u32)).collect() };
        let col = |k: crate::tables::Key| crate::search::COLS.iter().position(|c| c.key() == k).unwrap();
        // biggest first: the biggest two
        let (all, shown) = sort_and_cap(&places, all, (col(crate::tables::Key::Size), true), &AtomicBool::new(false)).unwrap();
        assert_eq!(names(&shown), ["b.txt", "c.txt", "a.txt", "y.txt", "z.txt"]);
        let capped = merge_capped(&all, |k, id| places[k].as_ref().map(|p| compact_val(&p.ix, id, SortBy::Size, true)), true, 2);
        assert_eq!(names(&capped), ["b.txt", "c.txt"]);
        // by name: the first two names of all of them, not of the biggest two
        let (all, _) = sort_and_cap(&places, all, (col(crate::tables::Key::Name), false), &AtomicBool::new(false)).unwrap();
        let capped = merge_capped(&all, |k, id| places[k].as_ref().map(|p| compact_val(&p.ix, id, SortBy::Name, false)), false, 2);
        assert_eq!(names(&capped), ["a.txt", "b.txt"]);
        // by folder: D:\ then E:\, names within
        let (_, shown) = sort_and_cap(&places, all, (col(crate::tables::Key::Folder), false), &AtomicBool::new(false)).unwrap();
        assert_eq!(names(&shown), ["a.txt", "b.txt", "z.txt", "c.txt", "y.txt"]);
    }

    #[test]
    fn by_name_every_folder_is_above_every_file_in_both_directions() {
        let d = tree("D:\\", &[("b.txt", false, 0, 1), ("Zed", true, 0, 0), ("a.txt", false, 0, 1), ("Alpha", true, 0, 0)]);
        let e = tree("E:\\", &[("c.txt", false, 0, 1), ("Mid", true, 0, 0), ("z.txt", false, 0, 1)]);
        let places = vec![place(&d, "D:\\"), place(&e, "E:\\")];
        let all: Vec<Vec<u32>> = places.iter().map(|p| p.as_ref().map_or(Vec::new(), |p| (1..p.ix.len() as u32).collect())).collect();
        let names = |shown: &[(usize, usize)]| -> Vec<String> { shown.iter().map(|&(k, id)| places[k].as_ref().unwrap().ix.name(id as u32)).collect() };
        let name_col = crate::search::COLS.iter().position(|c| c.key() == crate::tables::Key::Name).unwrap();
        let (all, shown) = sort_and_cap(&places, all, (name_col, false), &AtomicBool::new(false)).unwrap();
        assert_eq!(names(&shown), ["Alpha", "Mid", "Zed", "a.txt", "b.txt", "c.txt", "z.txt"]);
        let (_, shown) = sort_and_cap(&places, all, (name_col, true), &AtomicBool::new(false)).unwrap();
        assert_eq!(names(&shown), ["Zed", "Mid", "Alpha", "z.txt", "c.txt", "b.txt", "a.txt"], "folders still first, each group reversed");
        // the scan's rows merge with them by the same key: a scan folder between other drives' folders
        let t = tree("C:\\", &[("Beta", true, 0, 0)]);
        for desc in [false, true] {
            let a = scan_val(&t, 1, SortBy::Name, desc);
            let f = compact_val(&places[0].as_ref().unwrap().ix, 1, SortBy::Name, desc);
            let dir_id = (1..places[0].as_ref().unwrap().ix.len() as u32).find(|&i| places[0].as_ref().unwrap().ix.is_dir(i)).unwrap();
            let d = compact_val(&places[0].as_ref().unwrap().ix, dir_id, SortBy::Name, desc);
            // (a folder against a file: the folder is the one that comes first in this direction)
            let folder_first = if desc { a > f } else { a < f };
            assert!(folder_first, "a scan folder is ahead of another drive's file (desc {desc})");
            assert!(if desc { d > f } else { d < f }, "and so is another drive's");
        }
    }

    fn pick(place: usize, id: usize, path: &str) -> crate::search_delete::Pick {
        crate::search_delete::Pick { path: path.into(), is_dir: false, size: 1, secs: None, place, id, gen: 0 }
    }

    #[test]
    fn a_re_sort_that_cuts_the_list_drops_the_picks_it_cut() {
        // A, B, C picked; the new order lists only A and B: C is no longer picked (SRCH finding 1)
        let mut m = More::default();
        m.picks.add(pick(0, 1, r"D:\a"));
        m.picks.add(pick(0, 2, r"D:\b"));
        m.picks.add(pick(0, 3, r"D:\c"));
        m.sel = Some((0, 3));
        m.hits = vec![(0, 1), (0, 2), (0, 9)];
        assert_eq!(m.prune_picks(), 1);
        assert_eq!(m.picks.len(), 2);
        assert!(m.picks.contains_key(&crate::search_delete::key(Path::new(r"D:\a"))) && !m.picks.contains_key(&crate::search_delete::key(Path::new(r"D:\c"))));
        assert_eq!(m.sel, None, "the keyboard's row left the list too");
        // nothing cut: nothing changes
        m.sel = Some((0, 2));
        assert_eq!(m.prune_picks(), 0);
        assert_eq!(m.sel, Some((0, 2)));
        // an emptied or unparsable query lists nothing: nothing stays picked
        m.drop_picks();
        assert!(m.picks.is_empty() && m.sel.is_none());
    }

    #[test]
    fn the_status_says_how_many_of_how_many_are_listed() {
        assert_eq!(capped_note(100_000, 250_000), "; showing the first 100,000 of 250,000 on other drives");
        assert_eq!(capped_note(5, 5), "");
        assert_eq!(capped_note(0, 0), "");
    }

    #[test]
    fn merging_keeps_order_and_ties() {
        let lists = vec![vec![1, 4, 9], vec![], vec![2, 4, 5]];
        let key = |k: usize, id: u32| (lists[k].len() as u32 * 0 + id, ());
        let m = merge_capped(&lists, |k, id| key(k, id), false, 10);
        assert_eq!(m, [(0, 1), (2, 2), (0, 4), (2, 4), (2, 5), (0, 9)], "a tie keeps the list order");
        assert_eq!(merge_capped(&lists, |k, id| key(k, id), false, 3).len(), 3);
        let rev = vec![vec![9, 4, 1], vec![5, 4, 2]];
        assert_eq!(merge_capped(&rev, |_, id| id, true, 4), [(0, 9), (1, 5), (0, 4), (1, 4)]);
        assert!(merge_capped::<u32>(&[], |_, id| id, false, 5).is_empty());
    }

    #[test]
    fn what_the_scan_covers_is_one_run_of_ids() {
        let t = tree("D:\\", &[("Games", true, 0, 0), ("Steam", true, 1, 0), ("x.vdf", false, 2, 1), ("y.txt", false, 1, 1), ("top.txt", false, 0, 1)]);
        let ix = CompactIndex::build(&t);
        let r = covered(&ix, "d:\\games").unwrap();
        let mut inside: Vec<String> = r.clone().map(|i| ix.name(i)).collect();
        inside.sort();
        assert_eq!(inside, ["Games", "Steam", "x.vdf", "y.txt"]);
        let r = covered(&ix, "d:\\games\\steam").unwrap();
        assert_eq!(r.clone().map(|i| ix.name(i)).collect::<Vec<_>>(), ["Steam", "x.vdf"]);
        assert_eq!(covered(&ix, "d:").unwrap(), 0..ix.len() as u32, "the whole drive");
        assert_eq!(covered(&ix, "d:\\nothere"), None);
        assert_eq!(covered(&ix, "e:\\games"), None);
        assert_eq!(covered(&ix, "d:\\gam"), None, "a name, not a prefix of one");
    }

    #[test]
    fn which_drives_are_read_when_search_opens() {
        let fixed: Vec<String> = ["C:\\", "D:\\", "E:\\", "F:\\"].iter().map(|s| s.to_string()).collect();
        let set = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<BTreeSet<String>>();
        let none = |_: &str| false;
        // only those switched on, never the scan's own drive
        assert_eq!(drives_to_read(&fixed, Some("c:"), false, &set(&["C:\\", "E:\\"]), &set(&[]), &none), ["E:\\"]);
        // every drive: but not those switched off
        assert_eq!(drives_to_read(&fixed, Some("c:"), true, &set(&[]), &set(&["E:\\"]), &none), ["D:\\", "F:\\"]);
        // a scan of a folder: its drive can be added (the rest of the volume)
        assert_eq!(drives_to_read(&fixed, Some("c:\\users"), false, &set(&["C:\\"]), &set(&[]), &none), ["C:\\"]);
        // already read, being read or failed: not again
        let busy = |d: &str| d == "D:\\";
        assert_eq!(drives_to_read(&fixed, None, true, &set(&[]), &set(&[]), &busy), ["C:\\", "E:\\", "F:\\"]);
        // a remembered drive that is no longer there is not offered
        assert_eq!(drives_to_read(&fixed, None, false, &set(&["Z:\\"]), &set(&[]), &none), Vec::<String>::new());
        assert_eq!(set_list(" D:\\, ,E:\\"), set(&["D:\\", "E:\\"]));
    }

    #[test]
    fn a_drive_switched_off_while_read_is_dropped() {
        // SRCH-5: switched off mid-read, the read is cancelled and what it sends is dropped
        let mut m = More::default();
        let (tx, rx) = channel();
        let progress = sprawl_core::Progress::new();
        m.reading.push(Reading { label: "D:\\".into(), drive: Some("D:\\".into()), progress: Some(progress.clone()), rx });
        m.drives_on.insert("D:\\".into());
        let t = tree("D:\\", &[("a", false, 0, 1)]);
        m.places.push(place(&t, "D:\\"));
        m.sel = Some((0, 1));
        let gen = m.gen;
        more_drop_drive(&mut m, "D:\\");
        assert!(progress.is_cancelled());
        assert!(m.reading.is_empty());
        assert!(m.place(0).is_none(), "its slot is empty, not reused");
        assert!(m.sel.is_none() && m.gen != gen);
        assert!(tx.send(Ok(Place { ix: Arc::new(CompactIndex::build(&t)), as_of: None, read_at: SystemTime::now(), errors: 0, drive: Some("D:\\".into()) })).is_err(), "nobody takes the result");
        // a row of a slot that is gone, or an id past its index, is no item
        assert!(m.item(0, 1).is_none());
        m.places.push(place(&t, "D:\\"));
        assert!(m.item(1, 1).is_some());
        assert!(m.item(1, 99).is_none());
        assert!(m.item(7, 0).is_none());
        assert_eq!(m.items_searched(), 1);
    }
}
