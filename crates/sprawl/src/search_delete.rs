//! Delete for the Search tab's rows from other drives and saved scans.
//!
//! Those rows come from a [`CompactIndex`] that is not the scan on screen, so the scan's way of
//! judging "new or changed since the scan" can't be used. Here every item is judged by the disk
//! as it is now, twice: when the question is made, and again in the worker immediately before
//! each item goes. An item is left alone, and listed, unless it
//!
//! - is still there, and still the same kind (file or folder);
//! - is not a link, junction or other reparse point (checked without following it);
//! - is not protected (the same refusal as every other Delete: `commands::path_refusal`);
//! - for a file, still has the size and last-write time (to the second, as the index keeps it)
//!   the result row showed.
//!
//! A folder is walked first, off the window's thread with a time limit, so the question gives
//! what it holds now next to what the saved read said. The deletion itself is
//! `State::manual_delete_with`, the same worker as the scan's Delete (`clean::run_with`, the
//! Recycle Bin or permanent), with `since` set to when the fresh check began, so whatever was
//! created or changed after it stays too.
//!
//! Picks are by full path ([`Picks`]): a re-sort or a new list can't make one point at another
//! item. Picks from the scan's rows and from these never mix: picking one kind drops the other.

use std::collections::BTreeMap;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sprawl_core::compact::CompactIndex;
use sprawl_core::{Progress, ROOT};

use crate::format::{fmt_time, thousands};
use crate::state::{text, State, View};

/// The most results that can be picked at once.
pub(crate) const MAX_PICKS: usize = 5000;
/// How long measuring the picked folders may take before the Delete is refused.
const MEASURE_LIMIT: Duration = Duration::from_secs(60);
const REPARSE: u32 = 0x400;

/// A picked result, as the row showed it.
#[derive(Clone, Debug)]
pub(crate) struct Pick {
    pub path: PathBuf,
    pub is_dir: bool,
    /// A file's size in bytes, as shown.
    pub size: u64,
    /// The last-write time shown, in whole seconds since 1970.
    pub secs: Option<i64>,
    /// Where it was listed (to see the list is still the one it was picked from).
    pub place: usize,
    pub id: usize,
    pub gen: u64,
}

impl Pick {
    pub(crate) fn of(ix: &CompactIndex, i: u32, place: usize, id: usize, gen: u64) -> Pick {
        Pick { path: ix.path_of(i), is_dir: ix.is_dir(i), size: ix.size(i), secs: ix.modified(i).map(secs), place, id, gen }
    }
}

/// Seconds since 1970, rounded down: how the index keeps a time.
pub(crate) fn secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs().min(i64::MAX as u64) as i64,
        Err(e) => {
            let d = e.duration();
            -(d.as_secs().min(i64::MAX as u64 - 1) as i64) - i64::from(d.subsec_nanos() > 0)
        }
    }
}

/// A path as a key: lower case, no trailing `\`.
pub(crate) fn key(p: &Path) -> String {
    crate::search_more::norm(p)
}

/// The picked rows from other drives, by full path.
#[derive(Clone, Default)]
pub(crate) struct Picks {
    map: BTreeMap<String, Pick>,
}

impl Picks {
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    pub(crate) fn clear(&mut self) {
        self.map.clear();
    }

    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    pub(crate) fn remove(&mut self, key: &str) {
        self.map.remove(key);
    }

    /// Keep the picks `keep` says yes to; how many went.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&Pick) -> bool) -> usize {
        let before = self.map.len();
        self.map.retain(|_, p| keep(p));
        before - self.map.len()
    }

    /// Pick it; false when it is not already picked and the most are.
    pub(crate) fn add(&mut self, p: Pick) -> bool {
        let k = key(&p.path);
        if !self.map.contains_key(&k) && self.map.len() >= MAX_PICKS {
            return false;
        }
        self.map.insert(k, p);
        true
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Pick> {
        self.map.values()
    }

    /// The picks that are not inside another pick (a folder takes what is in it along).
    pub(crate) fn top_most(&self) -> Vec<Pick> {
        self.map.values().filter(|p| !p.path.ancestors().skip(1).any(|a| self.map.contains_key(&key(a)))).cloned().collect()
    }
}

/// Whether the result `this` (place, id) at `path` is one the keyboard and the row menu act on:
/// one of the picks, or, with no picks, the row the keyboard is on (`sel`). With picks, `sel`
/// counts only if it is one of them (Ctrl+click took it away, say). The one rule for Delete, the
/// row menu, the highlight and the count.
pub(crate) fn counts(sel: Option<(usize, usize)>, picks: &Picks, this: (usize, usize), path: &Path) -> bool {
    picks.contains_key(&key(path)) || (picks.is_empty() && sel == Some(this))
}

/// How many results the keyboard and the row menu act on: the picks, or with none the row the
/// keyboard is on (`sel_listed`: it is still an item).
pub(crate) fn count(picks: &Picks, sel_listed: bool) -> usize {
    if picks.is_empty() {
        usize::from(sel_listed)
    } else {
        picks.len()
    }
}

/// Every pick `(place, id)` is one of the rows `(id, place + 1)` the table shows now.
pub(crate) fn all_listed(rows: &[(usize, usize)], picks: &[(usize, usize)]) -> bool {
    if picks.is_empty() {
        return true;
    }
    let shown: std::collections::HashSet<(usize, usize)> = rows.iter().copied().collect();
    picks.iter().all(|&(place, id)| shown.contains(&(id, place + 1)))
}

/// Every pick `(place, id)` is still among the matches from other drives the list holds.
pub(crate) fn picks_still_hit(hits: &[(usize, usize)], picks: &[(usize, usize)]) -> bool {
    if picks.is_empty() {
        return true;
    }
    let listed: std::collections::HashSet<(usize, usize)> = hits.iter().copied().collect();
    picks.iter().all(|p| listed.contains(p))
}

/// A right-click on the Search result at `path` (or the menu key on it): a row that is picked
/// keeps every pick, because the menu acts on them all; any other row is acted on alone, so the
/// picks go. True when they stayed. The menu key (`by_key`) on a row that was just un-picked keeps
/// them too, as Del does: only a pointer right-click on another row acts on that row alone.
pub(crate) fn menu_keeps_picks(picks: &mut Picks, path: &Path, by_key: bool) -> bool {
    if picks.contains_key(&key(path)) || (by_key && !picks.is_empty()) {
        return true;
    }
    picks.clear();
    false
}

/// Why the item may not be deleted as it is on the disk now (`refusal` is the protected-path
/// refusal every Delete uses). Looks at the item itself without following a link.
pub(crate) fn check_now(p: &Pick, refusal: &dyn Fn(&Path) -> Option<String>) -> Result<(), String> {
    if let Some(why) = refusal(&p.path) {
        return Err(why);
    }
    let m = match std::fs::symlink_metadata(&p.path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err("it isn't there any more".into()),
        Err(e) => return Err(format!("it can't be read now ({e})")),
    };
    if m.file_attributes() & REPARSE != 0 {
        return Err("it is a link, junction or cloud placeholder (a reparse point), which Sprawl won't delete from a search result".into());
    }
    if m.is_dir() != p.is_dir {
        return Err("it changed since it was found (a file became a folder or the other way round)".into());
    }
    if !p.is_dir && (m.len() != p.size || m.modified().ok().map(secs) != p.secs) {
        return Err("it changed since it was found (its size or date is not what the result showed)".into());
    }
    Ok(())
}

/// What a folder holds now.
#[derive(Clone, Debug, Default)]
pub(crate) struct Measured {
    pub size: u64,
    pub files: u64,
    /// Folders inside it that could not be read (the total is then short).
    pub unreadable: u64,
}

enum Measure {
    TooLong,
    Refused(String),
}

/// Walk folder `path` (links are not followed; the walker checks each folder through a handle)
/// and total it. A protected item inside refuses it, as for the scan's rows.
fn measure(path: &Path, progress: &Progress) -> Result<Measured, Measure> {
    match sprawl_core::walk::inventory(path, progress) {
        Ok(t) => {
            let never = sprawl_advisor::safety::Never::default_list();
            if let Some((at, p)) = never.hit_inside(&t, ROOT) {
                return Err(Measure::Refused(format!("it holds {at}, which is protected ({p}); delete the things around it one by one instead")));
            }
            Ok(Measured { size: t.nodes[ROOT].size, files: t.nodes[ROOT].files, unreadable: t.errors })
        }
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => Err(Measure::TooLong),
        Err(e) => Err(Measure::Refused(format!("it can't be read now ({e})"))),
    }
}

/// What the question needs about a pick.
pub(crate) struct PrepIn {
    pub pick: Pick,
    /// Files a folder held when it was read.
    pub saved_files: u64,
    /// "the saved scan from 12 Nov" / "the read of D:\ at ...".
    pub saved_when: String,
}

/// A pick that passed the fresh check, with what is there now.
#[derive(Clone)]
pub(crate) struct Ready {
    pub pick: Pick,
    /// Bytes now: the file's size, or what the folder holds.
    pub bytes: u64,
    pub measured: Option<Measured>,
    pub saved_files: u64,
    pub saved_when: String,
}

pub(crate) struct Prepared {
    /// When the check began: anything created or changed after stays.
    pub since: SystemTime,
    pub ready: Vec<Ready>,
    pub refused: Vec<String>,
    pub timed_out: bool,
}

/// The fresh check of every pick, folders measured: runs on a worker. `refusal` is the
/// protected-path refusal. A folder not measured within `limit` ends it with `timed_out`.
pub(crate) fn prepare(inputs: Vec<PrepIn>, progress: &Arc<Progress>, refusal: &(dyn Fn(&Path) -> Option<String> + Sync), limit: Duration) -> Prepared {
    let since = SystemTime::now();
    let done = Arc::new(AtomicBool::new(false));
    {
        let (p, d) = (progress.clone(), done.clone());
        std::thread::spawn(move || {
            let t0 = Instant::now();
            while !d.load(Ordering::Relaxed) {
                if t0.elapsed() >= limit {
                    p.cancel();
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
    }
    let mut out = Prepared { since, ready: Vec::new(), refused: Vec::new(), timed_out: false };
    for inp in inputs {
        if progress.is_cancelled() {
            out.timed_out = true;
            break;
        }
        let what = inp.pick.path.display().to_string();
        if let Err(why) = check_now(&inp.pick, refusal) {
            out.refused.push(format!("{what}: {why}"));
            continue;
        }
        if !inp.pick.is_dir {
            out.ready.push(Ready { bytes: inp.pick.size, pick: inp.pick, measured: None, saved_files: 0, saved_when: inp.saved_when });
            continue;
        }
        match measure(&inp.pick.path, progress) {
            Ok(m) => out.ready.push(Ready { bytes: m.size, measured: Some(m), saved_files: inp.saved_files, saved_when: inp.saved_when, pick: inp.pick }),
            Err(Measure::TooLong) => {
                out.timed_out = true;
                break;
            }
            Err(Measure::Refused(why)) => out.refused.push(format!("{what}: {why}")),
        }
    }
    done.store(true, Ordering::Relaxed);
    out
}

/// A Delete measuring its folders before it asks.
pub(crate) struct Prep {
    rx: Receiver<Prepared>,
    progress: Arc<Progress>,
    label: String,
    /// The results list it was started on: a change of places drops it.
    gen: u64,
    permanent: bool,
    /// Refused before the check (overlaps the scan on screen).
    refused: Vec<String>,
}

/// Files a folder held when its place was read: the items in its run of ids that are not folders.
fn saved_files(ix: &CompactIndex, id: u32) -> u64 {
    let (d, n) = (ix.depth(id), ix.len() as u32);
    let (mut j, mut c) = (id + 1, 0);
    while j < n && ix.depth(j) > d {
        if !ix.is_dir(j) {
            c += 1;
        }
        j += 1;
    }
    c
}

impl State {
    /// The scan on screen is, or holds, `path`: it must be deleted from that scan.
    pub(crate) fn overlaps_scan(&self, path: &Path) -> bool {
        use crate::search_more::within;
        let Some(t) = &self.tree else { return false };
        let roots: Vec<PathBuf> = if t.nodes[ROOT].name == sprawl_core::MULTIPLE {
            t.nodes[ROOT].children.iter().map(|&c| t.path_of(c)).collect()
        } else {
            vec![t.root_path.clone()]
        };
        let a = key(path);
        roots.iter().any(|r| {
            let b = key(r);
            within(&a, &b) || within(&b, &a)
        })
    }

    /// Delete (Del: to the Recycle Bin; Shift+Del: for good) with rows from other drives or saved
    /// scans picked: every item is checked on the disk now, folders are measured (on a worker),
    /// and then one question covers them all. Never mixed with the scan's rows.
    pub(crate) fn delete_more(&mut self, permanent: bool) {
        if self.search.more.prep.is_some() {
            text(&self.root, "SearchInfo", "Still checking the folders for the last Delete...");
            return;
        }
        if !self.marked.is_empty() || self.selected.is_some_and(|s| self.rows.iter().any(|r| r.0 == s && r.1 == 0)) {
            self.tell("Not deleted", "Results from the scan on screen and results from other drives or saved scans are picked together. Pick one kind at a time. Nothing was deleted.");
            return;
        }
        if let Some(why) = self.delete_busy_refusal() {
            self.tell("Not deleted", &why);
            return;
        }
        let m = &self.search.more;
        let mut all = m.picks.clone();
        // with picks, the row the keyboard is on counts only if it is one of them
        if let (Some((k, id)), true) = (m.sel, m.picks.is_empty()) {
            if let Some((ix, i)) = m.item(k, id) {
                all.add(Pick::of(ix, i, k, id, m.gen));
            }
        }
        // each pick is still the row it was picked from (same list, same place, same path), and
        // still on the list (a sort that cut the list short can drop a row that was picked)
        let at: Vec<(usize, usize)> = all.iter().map(|p| (p.place, p.id)).collect();
        let listed = all_listed(&self.rows, &at) && all.iter().all(|p| p.gen == m.gen && m.item(p.place, p.id).is_some_and(|(ix, i)| key(&ix.path_of(i)) == key(&p.path)));
        if !listed {
            self.tell("Not deleted", "The results changed since they were picked, so nothing was deleted. Pick them again.");
            return;
        }
        let top = all.top_most();
        if top.is_empty() {
            text(&self.root, "SearchInfo", "Pick the results to delete first (Ctrl+click or Shift+click for several).");
            return;
        }
        let mut refused = Vec::new();
        let mut inputs = Vec::new();
        for p in top {
            if self.overlaps_scan(&p.path) {
                refused.push(format!("{}: it is, or holds, the scan on screen; delete it from the Tree or Files tab", p.path.display()));
                continue;
            }
            let place = self.search.more.place(p.place);
            let when = place.map(|pl| pl.as_of.unwrap_or(pl.read_at)).map(fmt_time).unwrap_or_default();
            let saved_when = if place.is_some_and(|pl| pl.as_of.is_some()) { format!("the saved scan from {when}") } else { format!("the read of {when}") };
            let saved_files = match self.search.more.item(p.place, p.id) {
                Some((ix, i)) if p.is_dir => saved_files(ix, i),
                _ => 0,
            };
            inputs.push(PrepIn { pick: p, saved_files, saved_when });
        }
        let label = if inputs.len() == 1 { inputs[0].pick.path.display().to_string() } else { format!("{} items", inputs.len()) };
        let gen = self.search.more.gen;
        let progress = Progress::new();
        let (tx, rx) = channel();
        let p2 = progress.clone();
        std::thread::spawn(move || {
            let r = prepare(inputs, &p2, &crate::commands::path_refusal, MEASURE_LIMIT);
            let _ = tx.send(r);
        });
        text(&self.root, "FootLeft", &format!("Checking {label}..."));
        self.search.more.prep = Some(Prep { rx, progress, label, gen, permanent, refused });
    }

    /// The list changed under a Delete that is still measuring: stop it and say so.
    pub(crate) fn cancel_prep(&mut self) {
        if let Some(p) = self.search.more.prep.take() {
            p.progress.cancel();
            text(&self.root, "FootLeft", "");
            self.tell("Not deleted", "The results changed while Sprawl was checking, so nothing was deleted.");
        }
    }

    /// Each frame: the fresh check of a Delete came back (or is still measuring).
    pub(crate) fn more_prep_poll(&mut self) {
        let Some(p) = &self.search.more.prep else { return };
        match p.rx.try_recv() {
            Err(TryRecvError::Empty) => {
                let n = p.progress.items();
                let msg = if n > 0 { format!("Checking {}... {} items so far", p.label, thousands(n)) } else { format!("Checking {}...", p.label) };
                text(&self.root, "FootLeft", &msg);
            }
            Err(TryRecvError::Disconnected) => {
                self.search.more.prep = None;
                text(&self.root, "FootLeft", "");
                self.tell("Not deleted", "The check stopped unexpectedly, so nothing was deleted.");
            }
            Ok(done) => {
                let Some(prep) = self.search.more.prep.take() else { return };
                text(&self.root, "FootLeft", "");
                self.more_prepared(prep, done);
            }
        }
    }

    /// The fresh check is done: one question for what passed.
    fn more_prepared(&mut self, prep: Prep, done: Prepared) {
        if prep.gen != self.search.more.gen {
            self.tell("Not deleted", "The results changed while Sprawl was checking, so nothing was deleted.");
            return;
        }
        // a new list or a re-sort that cut the list short can leave a measured folder off it
        let at: Vec<(usize, usize)> = done.ready.iter().map(|r| (r.pick.place, r.pick.id)).collect();
        if !picks_still_hit(&self.search.more.hits, &at) {
            self.tell("Not deleted", "The results changed while Sprawl was checking, so nothing was deleted.");
            return;
        }
        if done.timed_out {
            self.tell("Not deleted", &format!("Measuring {} took longer than a minute, so nothing was deleted.\n\nUse Scan This Folder on it first, then delete it from that scan.", prep.label));
            return;
        }
        let mut refused = prep.refused;
        refused.extend(done.refused);
        let (mut with_bin, without): (Vec<Ready>, Vec<Ready>) = done.ready.into_iter().partition(|r| prep.permanent || sprawl_advisor::recycle::no_bin(&r.pick.path).is_none());
        if with_bin.is_empty() && without.is_empty() {
            let why = if refused.is_empty() { String::new() } else { format!("\n\n{}", refused.join("\n")) };
            self.tell("Not deleted", &format!("Nothing can be deleted from these results.{why}"));
            return;
        }
        // No Recycle Bin there: Del never turns into a permanent delete on its own. When that
        // is all there is, it is asked as a permanent delete in so many words; otherwise those
        // items are left out and said so.
        let mut permanent = prep.permanent;
        let mut intro = String::new();
        let mut left_out = String::new();
        if !without.is_empty() {
            let why = sprawl_advisor::recycle::no_bin(&without[0].pick.path).unwrap_or_default();
            if with_bin.is_empty() {
                permanent = true;
                intro = format!("There is no Recycle Bin for this: {why}.\nDeleting it removes it for good. It will not go to the Recycle Bin and can't be brought back.\n\n");
                with_bin = without;
            } else {
                let names: String = without.iter().take(6).map(|r| format!("{}\n", r.pick.path.display())).collect();
                left_out = format!("\n\nNot included, because there is no Recycle Bin there ({why}). Shift+Del deletes these for good:\n{names}");
            }
        }
        let ready = with_bin;
        let total: u64 = ready.iter().map(|r| r.bytes).sum();
        let one = ready.len() == 1;
        let line = |r: &Ready| -> String {
            match &r.measured {
                Some(m) => {
                    let short = if m.unreadable > 0 { format!(" ({} folders could not be read)", thousands(m.unreadable)) } else { String::new() };
                    format!("now {} in {} files{short}; {} said {} in {} files", self.size(m.size), thousands(m.files), r.saved_when, self.size(r.pick.size), thousands(r.saved_files))
                }
                None => self.size(r.bytes),
            }
        };
        let what = if one {
            let r = &ready[0];
            let kind = if r.pick.is_dir { "folder" } else { "file" };
            let head = match (r.pick.is_dir, permanent) {
                (_, true) => format!("Delete this {kind} permanently? It can't be brought back."),
                (_, false) => format!("Move this {kind} to the Recycle Bin?"),
            };
            format!("{head}\n\n{}\n\n{}", line(r), r.pick.path.display())
        } else {
            let head = if permanent {
                format!("Delete these {} items permanently? They can't be brought back.", ready.len())
            } else {
                format!("Move these {} items to the Recycle Bin?", ready.len())
            };
            let list: String = ready.iter().take(12).map(|r| format!("{}\n    {}\n", r.pick.path.display(), line(r))).collect();
            let more = if ready.len() > 12 { format!("...and {} more\n", ready.len() - 12) } else { String::new() };
            format!("{head}\n\nNow {} in all.\n\n{list}{more}", self.size(total))
        };
        let left_alone = if refused.is_empty() {
            String::new()
        } else {
            let list: String = refused.iter().take(6).map(|s| format!("{s}\n")).collect();
            let more = if refused.len() > 6 { format!("...and {} more\n", refused.len() - 6) } else { String::new() };
            format!("\n\nLeft alone:\n{list}{more}")
        };
        let text = format!("{intro}{what}{left_alone}{left_out}\n\nItems that changed since this check, links and protected items are left alone; each item is checked again just before it goes.");
        let (title, yes) = if permanent { ("Delete permanently?", "Delete for good") } else { ("Move to the Recycle Bin?", "Move to the Recycle Bin") };
        let since = done.since;
        self.ask(title, &text, yes, permanent, move |s| s.delete_more_now(ready, permanent, since));
    }

    /// "Yes": every item is looked at again (the scan on screen may have changed meanwhile), then
    /// the worker deletes them, looking at each once more immediately before it goes.
    fn delete_more_now(&mut self, ready: Vec<Ready>, permanent: bool, since: SystemTime) {
        if let Some(why) = self.delete_busy_refusal() {
            self.tell("Not deleted", &why);
            return;
        }
        let (mut paths, mut notes, mut total) = (Vec::new(), Vec::new(), 0u64);
        let mut picks = std::collections::HashMap::new();
        for r in ready {
            let what = r.pick.path.display().to_string();
            if self.overlaps_scan(&r.pick.path) {
                notes.push(format!("{what}: it is, or holds, the scan on screen"));
            } else if let Err(why) = check_now(&r.pick, &crate::commands::path_refusal) {
                notes.push(format!("{what}: not deleted, {why}"));
            } else {
                total += r.bytes;
                paths.push(r.pick.path.clone());
                picks.insert(key(&r.pick.path), r.pick);
            }
        }
        if paths.is_empty() {
            self.tell("Not deleted", &notes.into_iter().take(12).collect::<Vec<_>>().join("\n"));
            return;
        }
        if !notes.is_empty() {
            self.tell("Some items were left alone", &notes.into_iter().take(12).collect::<Vec<_>>().join("\n"));
        }
        let picks = Arc::new(picks);
        let precheck = move |p: &Path| -> Option<String> {
            match picks.get(&key(p)) {
                Some(pick) => check_now(pick, &crate::commands::path_refusal).err(),
                None => Some("it is not one of the items that were asked about".into()),
            }
        };
        self.search.more.picks.clear();
        self.search.more.sel = None;
        let basis = crate::commands::DeleteBasis { since, seen: None, total, precheck: Some(Arc::new(precheck)), more: true };
        self.manual_delete_with(paths, permanent, basis);
        if self.view == View::Search {
            self.refresh_rows();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sprawl_core::compact::SortBy;
    use sprawl_core::{Node, Tree};

    /// A throwaway folder under the temp folder, removed (only it) when dropped.
    struct Temp(PathBuf);
    impl Temp {
        fn new(name: &str) -> Temp {
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let p = std::env::temp_dir().join(format!("sprawl-searchdel-{name}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&p).unwrap();
            Temp(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn none(_: &Path) -> Option<String> {
        None
    }

    /// The pick a row of a saved scan would give for the file as it is now.
    fn pick_of_disk(path: &Path) -> Pick {
        let m = std::fs::symlink_metadata(path).unwrap();
        Pick { path: path.to_path_buf(), is_dir: m.is_dir(), size: m.len(), secs: m.modified().ok().map(secs), place: 0, id: 1, gen: 0 }
    }

    #[test]
    fn a_measured_pick_must_still_be_on_the_list() {
        let hits = [(0, 1), (0, 2), (1, 5)];
        assert!(super::picks_still_hit(&hits, &[(0, 2), (1, 5)]));
        assert!(!super::picks_still_hit(&hits, &[(0, 2), (1, 6)]), "cut off by a re-sort");
        assert!(super::picks_still_hit(&[], &[]));
        assert!(!super::picks_still_hit(&[], &[(0, 1)]), "a new, empty list");
    }

    #[test]
    fn an_unchanged_file_passes() {
        let t = Temp::new("same");
        let f = t.0.join("a.txt");
        std::fs::write(&f, b"hello").unwrap();
        let p = pick_of_disk(&f);
        assert_eq!(check_now(&p, &none), Ok(()));
        let d = pick_of_disk(&t.0);
        assert!(d.is_dir);
        assert_eq!(check_now(&d, &none), Ok(()), "a plain folder passes, with no size to match");
    }

    #[test]
    fn a_file_whose_size_or_date_changed_is_refused() {
        let t = Temp::new("changed");
        let f = t.0.join("a.txt");
        std::fs::write(&f, b"hello").unwrap();
        let p = pick_of_disk(&f);
        // the size changed
        std::fs::write(&f, b"hello world").unwrap();
        let e = check_now(&p, &none).unwrap_err();
        assert!(e.contains("changed since it was found"), "{e}");
        // the same size, a different date
        let f2 = t.0.join("b.txt");
        std::fs::write(&f2, b"12345").unwrap();
        let p2 = pick_of_disk(&f2);
        assert_eq!(check_now(&p2, &none), Ok(()));
        let file = std::fs::File::options().write(true).open(&f2).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(3 * 86_400)).unwrap();
        drop(file);
        let e = check_now(&p2, &none).unwrap_err();
        assert!(e.contains("changed since it was found"), "{e}");
        // a row with no date can't be matched
        let mut p3 = pick_of_disk(&f2);
        p3.secs = None;
        assert!(check_now(&p3, &none).is_err());
    }

    #[test]
    fn a_missing_item_or_another_kind_is_refused() {
        let t = Temp::new("missing");
        let f = t.0.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        let p = pick_of_disk(&f);
        std::fs::remove_file(&f).unwrap();
        assert!(check_now(&p, &none).unwrap_err().contains("isn't there"));
        // a file replaced by a folder of the same name
        std::fs::create_dir(&f).unwrap();
        assert!(check_now(&p, &none).unwrap_err().contains("file became a folder"));
        // a folder replaced by a file
        let d = t.0.join("d");
        std::fs::create_dir(&d).unwrap();
        let pd = pick_of_disk(&d);
        std::fs::remove_dir(&d).unwrap();
        std::fs::write(&d, b"x").unwrap();
        assert!(check_now(&pd, &none).unwrap_err().contains("folder"));
    }

    #[test]
    fn a_junction_is_refused() {
        let t = Temp::new("junction");
        let target = t.0.join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("keep.txt"), b"x").unwrap();
        let link = t.0.join("link");
        let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(&target).output();
        if !made.is_ok_and(|o| o.status.success()) || !link.exists() {
            eprintln!("skipped: no junction could be made here");
            return;
        }
        // the row said "a folder" (a saved scan lists a junction as one)
        let p = Pick { path: link.clone(), is_dir: true, size: 0, secs: None, place: 0, id: 1, gen: 0 };
        let e = check_now(&p, &none).unwrap_err();
        assert!(e.contains("reparse point"), "{e}");
        // it is also refused when measuring, and what it points at is untouched
        let done = prepare(vec![PrepIn { pick: p, saved_files: 0, saved_when: "x".into() }], &Progress::new(), &none, Duration::from_secs(20));
        assert!(done.ready.is_empty() && done.refused.len() == 1 && !done.timed_out);
        assert!(target.join("keep.txt").exists());
        let _ = std::fs::remove_dir(&link);
    }

    #[test]
    fn a_protected_path_is_refused() {
        // nothing is touched: the refusal comes before the disk is looked at
        for path in [r"C:\Windows\System32", r"C:\Windows", r"C:\"] {
            let p = Pick { path: PathBuf::from(path), is_dir: true, size: 0, secs: None, place: 0, id: 1, gen: 0 };
            assert!(check_now(&p, &crate::commands::path_refusal).is_err(), "{path}");
        }
        // a path written oddly is refused too
        let p = Pick { path: PathBuf::from(r"C:\temp\..\Windows"), is_dir: true, size: 0, secs: None, place: 0, id: 1, gen: 0 };
        assert!(check_now(&p, &crate::commands::path_refusal).is_err());
        // and an ordinary file under the temp folder is not
        let t = Temp::new("plain");
        let f = t.0.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(check_now(&pick_of_disk(&f), &crate::commands::path_refusal), Ok(()));
    }

    #[test]
    fn a_folder_is_measured_as_it_is_now() {
        let t = Temp::new("measure");
        let d = t.0.join("d");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(d.join("sub").join("b.bin"), vec![0u8; 2000]).unwrap();
        let mut p = pick_of_disk(&d);
        p.size = 500; // what the saved read said
        let done = prepare(vec![PrepIn { pick: p, saved_files: 1, saved_when: "the saved scan".into() }], &Progress::new(), &none, Duration::from_secs(20));
        assert!(done.refused.is_empty() && !done.timed_out);
        let r = &done.ready[0];
        let m = r.measured.as_ref().unwrap();
        assert_eq!((m.size, m.files, r.bytes), (3000, 2, 3000));
        assert_eq!(r.pick.size, 500, "the saved size is kept for the question to show beside it");
    }

    #[test]
    fn nested_picks_go_with_their_folder() {
        let mut picks = Picks::default();
        for (path, dir) in [(r"D:\a", true), (r"D:\a\b.txt", false), (r"D:\a\c", true), (r"D:\ab", true), (r"E:\x.txt", false)] {
            picks.add(Pick { path: PathBuf::from(path), is_dir: dir, size: 1, secs: Some(1), place: 0, id: 1, gen: 0 });
        }
        let top: Vec<String> = picks.top_most().iter().map(|p| p.path.display().to_string()).collect();
        assert_eq!(top, [r"D:\a", r"D:\ab", r"E:\x.txt"], "D:\\ab is not inside D:\\a");
        // the same path in another case is the same pick
        assert!(picks.contains_key(&key(Path::new(r"d:\A\B.TXT"))));
    }

    #[test]
    fn a_right_click_keeps_the_picks_on_a_picked_row_and_drops_them_on_another() {
        let mut picks = Picks::default();
        for path in [r"D:\a.txt", r"D:\b.txt", r"E:\saved\c.txt"] {
            picks.add(Pick { path: PathBuf::from(path), is_dir: false, size: 1, secs: Some(1), place: 0, id: 1, gen: 0 });
        }
        // the same row (a path is compared as the picks keep it)
        assert!(menu_keeps_picks(&mut picks, Path::new(r"d:\A.TXT"), false));
        assert_eq!(picks.len(), 3, "a picked row leaves every pick for the menu to act on");
        // another row: it alone is acted on
        assert!(!menu_keeps_picks(&mut picks, Path::new(r"E:\other.txt"), false));
        assert!(picks.is_empty());
        // and with nothing picked, nothing is kept
        assert!(!menu_keeps_picks(&mut picks, Path::new(r"D:\a.txt"), false));
        assert!(!menu_keeps_picks(&mut picks, Path::new(r"D:\a.txt"), true));
        // the menu key on a row just taken out of the picks: the picks stay, as for Del
        picks.add(Pick { path: PathBuf::from(r"D:\a.txt"), is_dir: false, size: 1, secs: Some(1), place: 0, id: 1, gen: 0 });
        assert!(menu_keeps_picks(&mut picks, Path::new(r"D:\c.txt"), true));
        assert_eq!(picks.len(), 1);
        // a pointer right-click on that row acts on it alone
        assert!(!menu_keeps_picks(&mut picks, Path::new(r"D:\c.txt"), false));
        assert!(picks.is_empty());
    }

    fn tree(names: &[(&str, bool, usize, u64)]) -> Tree {
        let mut nodes = vec![Node { name: "D:\\".into(), is_dir: true, ..Default::default() }];
        for &(name, dir, parent, size) in names {
            let i = nodes.len();
            nodes.push(Node { name: name.into(), is_dir: dir, size, parent: Some(parent), ..Default::default() });
            nodes[parent].children.push(i);
        }
        let mut t = Tree { volumes: Vec::new(), root_path: PathBuf::from("D:\\"), nodes, errors: 0, method: Default::default() };
        t.compute_totals();
        t
    }

    #[test]
    fn a_pick_by_path_survives_a_re_sort() {
        let t = tree(&[("a.txt", false, 0, 5), ("b.txt", false, 0, 50), ("c.txt", false, 0, 1), ("d.txt", false, 0, 20)]);
        let ix = CompactIndex::build(&t);
        let mut picks = Picks::default();
        // picked from the list as sorted by size, biggest first: b.txt and c.txt
        let mut rows: Vec<u32> = (1..ix.len() as u32).collect();
        ix.sort(&mut rows, SortBy::Size, true);
        for &i in &[rows[0], rows[3]] {
            picks.add(Pick::of(&ix, i, 0, i as usize, 0));
        }
        let picked = |rows: &[u32]| -> Vec<String> { rows.iter().filter(|&&i| picks.contains_key(&key(&ix.path_of(i)))).map(|&i| ix.name(i)).collect() };
        let before = picked(&rows);
        assert_eq!(before, ["b.txt", "c.txt"]);
        // sorted by name instead: the rows moved, the same two items are still the picked ones
        let mut by_name = rows.clone();
        ix.sort(&mut by_name, SortBy::Name, true);
        assert_ne!(rows, by_name);
        let mut after = picked(&by_name);
        after.sort();
        assert_eq!(after, ["b.txt", "c.txt"]);
        // and the picks name them by path, with the size and date the rows showed
        let kept: Vec<String> = picks.iter().map(|p| p.path.display().to_string()).collect();
        assert_eq!(kept, [r"D:\b.txt", r"D:\c.txt"]);
        assert_eq!(picks.top_most().len(), 2);
    }

    #[test]
    fn with_picks_the_keyboards_row_counts_only_if_it_is_picked() {
        let mut picks = Picks::default();
        let p = |n: &str| PathBuf::from(format!(r"D:\{n}"));
        let pk = |n: &str, id: usize| Pick { path: p(n), is_dir: false, size: 1, secs: None, place: 0, id, gen: 0 };
        for (n, id) in [("a", 1), ("b", 2), ("c", 3)] {
            picks.add(pk(n, id));
        }
        // Ctrl+click C again takes it away, and the keyboard stays on C
        picks.remove(&key(&p("c")));
        let sel = Some((0, 3));
        assert!(counts(sel, &picks, (0, 1), &p("a")));
        assert!(!counts(sel, &picks, (0, 3), &p("c")), "C is the keyboard's row but not picked: it doesn't count");
        assert_eq!(count(&picks, true), 2, "so Delete and the menu act on 2");
        // nothing picked: the keyboard's row is the one
        let none = Picks::default();
        assert!(counts(sel, &none, (0, 3), &p("c")));
        assert!(!counts(sel, &none, (0, 4), &p("d")));
        assert_eq!((count(&none, true), count(&none, false)), (1, 0));
    }

    #[test]
    fn a_pick_that_is_not_on_the_list_refuses_the_delete() {
        // rows are (id, place + 1); picks are (place, id)
        let rows = [(1, 1), (2, 1), (7, 2), (5, 0)];
        assert!(all_listed(&rows, &[(0, 1), (0, 2), (1, 7)]));
        assert!(!all_listed(&rows, &[(0, 1), (0, 3)]), "id 3 fell off the list");
        assert!(!all_listed(&rows, &[(1, 1)]), "id 1 is listed for place 0, not place 1");
        assert!(!all_listed(&rows, &[(0, 5)]), "a row of the scan on screen is not another drive's");
        assert!(all_listed(&[], &[]), "nothing picked: nothing to refuse");
        assert!(!all_listed(&[], &[(0, 1)]), "the list was emptied");
    }

    #[test]
    fn retain_says_how_many_picks_went() {
        let mut picks = Picks::default();
        for i in 0..4 {
            picks.add(Pick { path: PathBuf::from(format!(r"D:\f{i}")), is_dir: false, size: 1, secs: None, place: 0, id: i, gen: 0 });
        }
        assert_eq!(picks.retain(|p| p.id % 2 == 0), 2);
        assert_eq!(picks.len(), 2);
    }

    #[test]
    fn the_most_picks_is_held() {
        let mut picks = Picks::default();
        for i in 0..MAX_PICKS + 10 {
            let ok = picks.add(Pick { path: PathBuf::from(format!(r"D:\f{i}")), is_dir: false, size: 1, secs: Some(1), place: 0, id: i, gen: 0 });
            assert_eq!(ok, i < MAX_PICKS);
        }
        assert_eq!(picks.len(), MAX_PICKS);
        // one already picked can be added again (it is the same pick)
        assert!(picks.add(Pick { path: PathBuf::from(r"D:\F0"), is_dir: false, size: 1, secs: Some(1), place: 0, id: 0, gen: 0 }));
    }
}
