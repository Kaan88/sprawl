//! F2 on the Search tab: rename the one picked result, and Ctrl+C: copy the picked results as files.
//!
//! A rename changes the user's files, so it is careful:
//!
//! - one result only, a file or a folder that is still there as that kind;
//! - refused (before it asks) where Delete would refuse: protected paths, a scan loaded from CSV,
//!   a scanned folder itself, a folder holding something protected, and while a scan or clean-up
//!   runs; and for results from a saved scan, which show how a place was and not what is there;
//! - the new name is checked for what Windows doesn't allow and never replaces what is already
//!   there (`win32::move_no_replace`, not `fs::rename`, which would overwrite a file);
//! - a file keeps its extension unless the new name has one of its own;
//! - Windows' own error is shown as it is.
//!
//! The question is a card with a box to type in (`ask_text`) rather than a box in the row: the
//! rows are drawn again whenever results arrive, which would drop a box being typed in.
//! Afterwards a result of the scan on screen is read again with its folder (as after the Windows
//! menu's own Rename); a result from another drive leaves the list, since that drive's read
//! still has the old name (Refresh reads it again).

use std::path::{Path, PathBuf};

use crate::search::picked_in_order;
use crate::state::{text, State, View};

/// Names Windows keeps for devices, with or without an extension.
const DEVICES: [&str; 30] = [
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "COM\u{b9}", "COM\u{b2}", "COM\u{b3}", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6",
    "LPT7", "LPT8", "LPT9", "LPT\u{b9}", "LPT\u{b2}", "LPT\u{b3}",
];

/// Why `name` can't be a file or folder name on Windows.
pub(crate) fn invalid_name(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Type a name.".into());
    }
    // Windows counts UTF-16 units
    if name.encode_utf16().count() > 255 {
        return Some("A name can have at most 255 characters.".into());
    }
    if let Some(c) = name.chars().find(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || (*c as u32) < 32) {
        let shown = if (c as u32) < 32 { "a control character".to_string() } else { format!("\u{201c}{c}\u{201d}") };
        return Some(format!("A name can't contain {shown}. None of < > : \" / \\ | ? * is allowed."));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some("A name can't end with a dot or a space (Windows would drop it).".into());
    }
    let stem = name.split('.').next().unwrap_or("").trim_end();
    if DEVICES.iter().any(|d| d.eq_ignore_ascii_case(stem)) {
        return Some(format!("\u{201c}{stem}\u{201d} is a name Windows keeps for a device."));
    }
    None
}

/// The extension of file name `name`: what follows its last dot, if both parts are not empty
/// and it looks like an extension (letters and digits, at most 10; "Report v1.2 final" has none).
fn extension(name: &str) -> Option<&str> {
    name.rsplit_once('.')
        .filter(|(a, b)| !a.is_empty() && !b.is_empty() && b.chars().count() <= 10 && b.chars().all(char::is_alphanumeric))
        .map(|(_, e)| e)
}

/// What `typed` makes of the name `old`: a file keeps its extension unless what was typed has a
/// dot of its own. Err says what is wrong with it.
pub(crate) fn new_name(old: &str, is_dir: bool, typed: &str) -> Result<String, String> {
    // untouched (or typed the same) is no change, even when trimming would make one
    if typed == old {
        return Err("That is already its name.".into());
    }
    let typed = typed.trim();
    if typed.is_empty() {
        return Err("Type a name.".into());
    }
    let name = match extension(old) {
        Some(ext) if !is_dir && !typed.contains('.') => format!("{typed}.{ext}"),
        _ => typed.to_string(),
    };
    if let Some(why) = invalid_name(&name) {
        return Err(why);
    }
    if name == old {
        return Err("That is already its name.".into());
    }
    Ok(name)
}

/// Why a rename didn't happen: the name can be tried again, or the rename stops here.
#[derive(Debug, PartialEq)]
pub(crate) enum Bad {
    Name(String),
    Stop(String),
}

/// Rename `old` to `name` in its folder, now. Never onto a name that is taken (a name that
/// differs only in case is the same file: that is allowed). `refusal` is the protected-path
/// check, applied to the new path too. Returns the new path.
pub(crate) fn rename_on_disk(old: &Path, name: &str, refusal: &dyn Fn(&Path) -> Option<String>) -> Result<PathBuf, Bad> {
    let Some(dir) = old.parent() else { return Err(Bad::Stop("It has no folder to be renamed in.".into())) };
    // the folder and every folder above it are held as themselves (no junction, no link, nothing
    // that resolves elsewhere) until the rename is done, so none can be swapped for a link to
    // somewhere else between the checks and the move, as Delete does
    let _held = sprawl_core::local::directory(dir).map_err(|e| Bad::Stop(format!("Sprawl couldn't hold its folder safely ({e}), so nothing was renamed.")))?;
    let to = dir.join(name);
    if let Some(why) = refusal(&to) {
        return Err(Bad::Stop(why));
    }
    let same_but_case = old.file_name().is_some_and(|o| o.to_string_lossy().to_lowercase() == name.to_lowercase());
    if !same_but_case && std::fs::symlink_metadata(&to).is_ok() {
        return Err(Bad::Name(format!("\u{201c}{name}\u{201d} is already in that folder. Choose another name.")));
    }
    crate::win32::move_no_replace(old, &to).map_err(|e| Bad::Stop(format!("Windows said: {e}")))?;
    Ok(to)
}

/// Why `path` may not be renamed from or to, wherever it is: not a local drive path (a network
/// share is shown read only, as for every other action), protected, excluded in Settings, or
/// one the user told Sprawl to Keep (or holding or inside one: a rename would orphan the Keep).
pub(crate) fn path_refusal_for_rename(path: &Path, kept: &[PathBuf]) -> Option<String> {
    if sprawl_core::local::lexical(path).is_err() {
        return Some(format!("{}\n\nisn't on a local drive. Sprawl shows network folders read only and renames only on local drives; rename it in Explorer.", path.display()));
    }
    if let Some(why) = crate::commands::path_refusal(path) {
        return Some(why);
    }
    kept_refusal(path, kept)
}

/// The Keep decision (what clean-ups spare) that a rename of `path` would orphan, if any.
pub(crate) fn kept_refusal(path: &Path, kept: &[PathBuf]) -> Option<String> {
    use crate::search_more::{norm, within};
    let a = norm(path);
    kept.iter()
        .find(|k| {
            let b = norm(k);
            !b.is_empty() && (within(&a, &b) || within(&b, &a))
        })
        .map(|k| format!("You told Sprawl to Keep {}, which this is, holds or is inside. A rename would leave the Keep naming a path that is gone, and clean-ups would no longer spare the item. Forget that Keep first.", k.display()))
}

/// Every path the user chose to Keep (saved, and decided since); Err when the saved list can't
/// be read, which stops the rename.
fn kept_list(live: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    let mut kept = sprawl_advisor::memory::Memory::load_checked()
        .map_err(|e| format!("Sprawl couldn't read what you told it to Keep ({e}), so it won't rename anything now."))?
        .kept_paths();
    kept.extend(live);
    Ok(kept)
}

/// Why folder item `id` of `place` (as `path`) can't be renamed for what it holds: the folder
/// changed after its drive was read, or something in it (or it) is protected by the end of its
/// path (a disk image, a mail profile, a vault), which `Never::hit` on the folder doesn't see.
/// The change check looks only at the folder's own modified time, which a file added deeper
/// down doesn't move: what is below is as the drive's read found it.
fn more_folder_refusal(place: &crate::search_more::Place, id: usize, path: &Path) -> Option<String> {
    let ix = &place.ix;
    let id = id as u32;
    if std::fs::metadata(path).and_then(|m| m.modified()).map_or(true, |m| m > place.read_at) {
        return Some("The folder changed since its drive was read, so what it holds isn't known. Refresh the drive's results (or scan the folder) and try again.".into());
    }
    let never = sprawl_advisor::safety::Never::default_list();
    // the end-of-path entries taken once, and each item's name tested against their endings
    // before any path is built: a folder of a million items takes a fraction of a second
    let matcher = never.tail_matcher();
    let d = ix.depth(id);
    let n = ix.len() as u32;
    let mut j = id;
    while j < n && (j == id || inside(ix, j, id, d)) {
        let raw = ix.name(j);
        if matcher.maybe(&raw) {
            // the item's last names (lower case, last first), as far as a never-list entry looks
            let mut names = vec![raw.to_lowercase()];
            let mut at = ix.parent(j);
            while let Some(i) = at.filter(|_| names.len() < 4) {
                names.push(ix.name(i).to_lowercase());
                at = ix.parent(i);
            }
            if let Some(p) = matcher.hit(&names) {
                let held = ix.path_of(j);
                return Some(format!("This folder holds
{}

which is protected ({p}), so it can't be renamed.", held.display()));
            }
        }
        j += 1;
    }
    None
}

/// Whether item `j` (after folder `id`, which is at depth `d`) is inside it. The index's depth
/// stops at 255, so below a folder that deep the depth can't tell what is inside from what
/// follows: the parent chain does.
fn inside(ix: &sprawl_core::compact::CompactIndex, j: u32, id: u32, d: u32) -> bool {
    if d < 255 {
        return ix.depth(j) > d;
    }
    let mut k = j;
    while k > id {
        match ix.parent(k) {
            None => return false,
            Some(p) if p == id => return true,
            Some(p) => k = p,
        }
    }
    false
}

/// What a rename is for.
#[derive(Clone, Copy)]
enum Target {
    /// A node of the scan on screen, in that tree generation.
    Scan(usize, u64),
    /// Item `id` of place `k`, in that places generation.
    More(usize, usize, u64),
}

impl State {
    /// Why no rename may start now: a scan, clean-up or other file operation runs.
    fn rename_busy(&self) -> Option<String> {
        self.delete_busy_refusal().map(|_| "Renaming waits until scanning, Check again and other file operations finish.".to_string())
    }

    /// Why result `target` at `path` may not be renamed, if it may not (the checks Delete makes
    /// before it asks, and the saved scan's).
    fn rename_refusal(&self, target: Target, path: &Path) -> Option<String> {
        if let Some(why) = self.rename_busy() {
            return Some(why);
        }
        let kept = match kept_list(self.advisor.memory.kept_paths()) {
            Ok(k) => k,
            Err(why) => return Some(why),
        };
        if let Some(why) = path_refusal_for_rename(path, &kept) {
            return Some(why);
        }
        let is_dir = match target {
            Target::Scan(n, _) => {
                let t = self.tree.as_ref()?;
                if t.method == sprawl_core::ScanMethod::Csv {
                    return Some("This view was loaded from a CSV file, so it may not match the disk. Scan the folder to rename from it.".into());
                }
                if t.nodes[sprawl_core::ROOT].name == sprawl_core::MULTIPLE && t.nodes[n].parent == Some(sprawl_core::ROOT) {
                    return Some("That is one of the scanned folders itself.".into());
                }
                let never = sprawl_advisor::safety::Never::default_list();
                // with a filter on, the tree on screen shows only part of a folder: what it
                // holds is looked for in the whole scan
                let (whole, inside) = match &self.full_tree {
                    Some(full) if t.nodes[n].is_dir => match sprawl_advisor::find_path(full, path) {
                        Some(k) => (&**full, k),
                        None => return Some("A filter is on and this folder isn't in the whole scan, so what it holds isn't known. Clear the filter first.".into()),
                    },
                    _ => (&**t, n),
                };
                if let Some((at, p)) = never.hit_inside(whole, inside) {
                    return Some(format!("This folder holds\n{at}\n\nwhich is protected ({p}), so it can't be renamed."));
                }
                t.nodes[n].is_dir
            }
            Target::More(k, id, _) => {
                let m = &self.search.more;
                let Some(place) = m.place(k) else { return Some("That result is gone from the list.".into()) };
                if place.as_of.is_some() {
                    return Some("This result comes from a saved scan, which shows how a place was and not what is there now. Rename it from Explorer, or scan the folder first.".into());
                }
                if self.overlaps_scan(path) {
                    return Some("It is, or holds, the scan on screen, which a rename here would put out of step with the disk. Rename it in Explorer, or scan its folder on its own first.".into());
                }
                let Some((ix, i)) = m.item(k, id) else { return Some("That result is gone from the list.".into()) };
                if ix.is_dir(i) {
                    if let Some(why) = more_folder_refusal(place, id, path) {
                        return Some(why);
                    }
                }
                ix.is_dir(i)
            }
        };
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() == is_dir => None,
            Ok(m) if m.file_type().is_symlink() => None,
            Ok(_) => Some("It changed since it was found (a file became a folder or the other way round). Scan again first.".into()),
            Err(_) => Some("It isn't there any more. Scan again to refresh the results.".into()),
        }
    }

    /// F2 in Search: rename the one picked result.
    pub(crate) fn search_rename(&mut self) {
        if self.view != View::Search {
            return;
        }
        let scan = picked_in_order(&self.rows, &self.marked, self.selected);
        let m = &self.search.more;
        let more = crate::search_delete::count(&m.picks, m.sel.is_some_and(|(k, id)| m.item(k, id).is_some()));
        match scan.len() + more {
            0 => return text(&self.root, "SearchInfo", "Pick the result to rename first."),
            1 => {}
            _ => return text(&self.root, "SearchInfo", "F2 renames one result at a time: pick just one."),
        }
        let found = if let Some(&n) = scan.first() {
            self.tree.as_ref().map(|t| (Target::Scan(n, self.tree_gen), t.path_of(n), t.nodes[n].is_dir))
        } else {
            let at = m.picks.iter().next().map(|p| (p.place, p.id)).or(m.sel);
            at.and_then(|(k, id)| m.item(k, id).map(|(ix, i)| (Target::More(k, id, m.gen), ix.path_of(i), ix.is_dir(i))))
        };
        let Some((target, path, is_dir)) = found else { return };
        if let Some(why) = self.rename_refusal(target, &path) {
            self.tell("Not renamed", &why);
            return;
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.rename_ask(target, path, is_dir, &name, "");
    }

    /// The question: the name in the box with its stem selected (a file's extension is kept
    /// unless the new name has its own); `note` says what was wrong with the last try.
    fn rename_ask(&mut self, target: Target, path: PathBuf, is_dir: bool, initial: &str, note: &str) {
        let stem = if is_dir { initial } else { extension(initial).map_or(initial, |e| &initial[..initial.len() - e.len() - 1]) };
        let select = (0, stem.encode_utf16().count() as i32);
        let what = if note.is_empty() { String::new() } else { format!("{note}\n\n") };
        let body = format!("{what}New name for\n{}", path.display());
        let p = path.clone();
        self.ask_text("Rename", &body, initial, select, "Rename", move |s, typed| s.rename_typed(target, p, is_dir, typed));
    }

    /// "Rename": everything is looked at again (the results may have changed behind the
    /// question), then the rename runs.
    fn rename_typed(&mut self, target: Target, path: PathBuf, is_dir: bool, typed: String) {
        let same = match target {
            Target::Scan(n, gen) => self.still_same(gen, n, &path),
            Target::More(k, id, gen) => self.search.more.gen == gen && self.search.more.item(k, id).is_some_and(|(ix, i)| ix.path_of(i) == path),
        };
        if !same {
            self.tell("Not renamed", &format!("The results changed while the question was open, so nothing was renamed.\n\n{}", path.display()));
            return;
        }
        if let Some(why) = self.rename_refusal(target, &path) {
            self.tell("Not renamed", &why);
            return;
        }
        let old_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name = match new_name(&old_name, is_dir, &typed) {
            Ok(n) => n,
            Err(why) => return self.rename_ask(target, path, is_dir, &typed, &why),
        };
        let kept = match kept_list(self.advisor.memory.kept_paths()) {
            Ok(k) => k,
            Err(why) => return self.tell("Not renamed", &why),
        };
        match rename_on_disk(&path, &name, &|p| path_refusal_for_rename(p, &kept)) {
            Ok(_) => {}
            Err(Bad::Name(why)) => return self.rename_ask(target, path, is_dir, &name, &why),
            Err(Bad::Stop(why)) => return self.tell("Not renamed", &format!("{why}\n\n{}", path.display())),
        };
        let done = format!("Renamed {old_name} to {name}");
        match target {
            Target::Scan(n, _) => {
                // the scan on screen: the row has its new name at once, and its folder is read
                // again, as after the Windows menu's own Rename (Delete and the like wait for
                // that to finish). The read is of the new path: a row directly in the scan's
                // folder is read as itself, which the old name no longer is.
                let to = path.with_file_name(&name);
                let Some(root) = self.scan_renamed(n, &path, &name) else { return };
                let again = crate::shellmenu::rescan_target(&to, &root);
                let (tx, rx) = std::sync::mpsc::channel();
                self.shell_rescans.push(rx);
                std::thread::spawn(move || {
                    if let Some(now) = crate::shellmenu::read_now(&again) {
                        let _ = tx.send((again, now));
                    }
                });
                text(&self.root, "SearchInfo", &format!("{done}; reading its folder again..."));
            }
            Target::More(..) => {
                // that drive's read still has the old name: the result leaves the list
                self.more_forget_deleted(std::slice::from_ref(&path));
                let case_only = old_name.to_lowercase() == name.to_lowercase();
                let line = if case_only { "The list may still show the old spelling until the drive is read again" } else { "It shows under its new name once the drive is read again" };
                text(&self.root, "SearchInfo", &format!("{done}. {line} (Refresh under the results)."));
            }
        }
    }

    /// Node `n` (at `old`) is now called `name` on the disk: the tree on screen (and the whole
    /// scan behind a filter) says so. Returns the scanned folder the node belongs to, for the
    /// read that follows: the scan's root, or for a scan of several folders the one it is in.
    fn scan_renamed(&mut self, n: usize, old: &Path, name: &str) -> Option<PathBuf> {
        if let Some(full) = self.full_tree.as_mut().map(std::sync::Arc::make_mut) {
            if let Some(k) = sprawl_advisor::find_path(full, old).filter(|&k| k != sprawl_core::ROOT) {
                full.nodes[k].name = name.to_string();
            }
        }
        let t = std::sync::Arc::make_mut(self.tree.as_mut()?);
        if n >= t.nodes.len() || n == sprawl_core::ROOT {
            return None;
        }
        t.nodes[n].name = name.to_string();
        let root = if t.nodes[sprawl_core::ROOT].name == sprawl_core::MULTIPLE {
            let mut k = n;
            while let Some(p) = t.nodes[k].parent.filter(|&p| p != sprawl_core::ROOT) {
                k = p;
            }
            t.path_of(k)
        } else {
            t.path_of(sprawl_core::ROOT)
        };
        self.order_gen += 1;
        self.tree_gen += 1;
        Some(root)
    }

    /// Ctrl+C in Search: the picked results go on the clipboard as files, so Paste in Explorer
    /// copies them.
    pub(crate) fn search_copy_files(&mut self) {
        let paths = self.menu_paths();
        if paths.is_empty() {
            text(&self.root, "SearchInfo", "Pick the results to copy first.");
            return;
        }
        let n = paths.len();
        let note = if crate::win32::copy_files(self.hwnd, &paths) {
            format!("{} copied as {}: Paste in Explorer to copy {}.", crate::format::thousands(n as u64), if n == 1 { "a file" } else { "files" }, if n == 1 { "it" } else { "them" })
        } else {
            "Couldn't put the results on the clipboard (another program has it, or there isn't enough memory). If the clipboard had been emptied, what was on it is gone.".to_string()
        };
        text(&self.root, "SearchInfo", &note);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new(name: &str) -> Temp {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let p = std::env::temp_dir().join(format!("sprawl-rename-{name}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&p).unwrap();
            Temp(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn allow(_: &Path) -> Option<String> {
        None
    }

    #[test]
    fn a_file_keeps_its_extension_unless_the_new_name_has_one() {
        assert_eq!(new_name("report.txt", false, "final"), Ok("final.txt".into()));
        assert_eq!(new_name("report.txt", false, "final.md"), Ok("final.md".into()));
        assert_eq!(new_name("report.txt", false, "  final  "), Ok("final.txt".into()), "spaces round it go");
        assert_eq!(new_name("archive.tar.gz", false, "backup"), Ok("backup.gz".into()));
        // no extension to keep, and a dotfile has none
        assert_eq!(new_name("README", false, "NOTES"), Ok("NOTES".into()));
        assert_eq!(new_name(".gitignore", false, "ignore"), Ok("ignore".into()));
        // a folder's name is the whole name
        assert_eq!(new_name("v1.2", true, "v2"), Ok("v2".into()));
        assert_eq!(new_name("Photos", true, "Pictures"), Ok("Pictures".into()));
    }

    #[test]
    fn names_windows_doesnt_allow_are_refused_with_the_reason() {
        for bad in ["", "   ", "a/b", "a\\b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", "name.", "CON", "con.txt", "Nul", "LPT3.log", "a\u{1}b"] {
            assert!(new_name("old.txt", false, bad).is_err(), "{bad:?}");
        }
        assert!(new_name("old", true, "ends with a dot.").is_err());
        assert!(new_name("old", true, &"x".repeat(256)).is_err());
        assert_eq!(new_name("old", true, &"x".repeat(255)), Ok("x".repeat(255)));
        // not a device: only the whole stem is
        assert!(new_name("old", true, "console").is_ok());
        assert!(new_name("old", true, "COM10").is_ok());
        assert!(new_name("a.txt", false, "a.txt").is_err(), "the same name is nothing to rename");
        assert!(new_name("a.txt", false, "a").is_err(), "it comes to the same name");
    }

    #[test]
    fn a_rename_never_replaces_what_is_there() {
        let t = Temp::new("replace");
        let (a, b) = (t.0.join("a.txt"), t.0.join("b.txt"));
        std::fs::write(&a, b"AAA").unwrap();
        std::fs::write(&b, b"BBB").unwrap();
        // onto a taken name: refused, and both files are as they were
        assert!(matches!(rename_on_disk(&a, "b.txt", &allow), Err(Bad::Name(_))));
        assert_eq!(std::fs::read(&a).unwrap(), b"AAA");
        assert_eq!(std::fs::read(&b).unwrap(), b"BBB");
        // a name that differs in case is taken too (b.txt is there as B.TXT)
        assert!(matches!(rename_on_disk(&a, "B.TXT", &allow), Err(Bad::Name(_))));
        // a free name: renamed, content kept
        let to = rename_on_disk(&a, "c.txt", &allow).unwrap();
        assert_eq!(to, t.0.join("c.txt"));
        assert!(!a.exists());
        assert_eq!(std::fs::read(&to).unwrap(), b"AAA");
        // a folder
        let d = t.0.join("dir");
        std::fs::create_dir(&d).unwrap();
        std::fs::write(d.join("in.txt"), b"x").unwrap();
        let moved = rename_on_disk(&d, "dir2", &allow).unwrap();
        assert!(moved.join("in.txt").exists() && !d.exists());
        // onto a folder's name: refused too
        assert!(matches!(rename_on_disk(&to, "dir2", &allow), Err(Bad::Name(_))));
    }

    #[test]
    fn only_the_case_can_change_and_a_missing_file_says_so() {
        let t = Temp::new("case");
        let a = t.0.join("report.txt");
        std::fs::write(&a, b"x").unwrap();
        let to = rename_on_disk(&a, "Report.TXT", &allow).unwrap();
        let listed: Vec<String> = std::fs::read_dir(&t.0).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(listed, ["Report.TXT"], "{to:?}");
        // gone: Windows' own words, not ours
        match rename_on_disk(&t.0.join("nothing.txt"), "x.txt", &allow) {
            Err(Bad::Stop(why)) => assert!(why.starts_with("Windows said: "), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_protected_new_path_stops_the_rename() {
        let t = Temp::new("protected");
        let a = t.0.join("a.txt");
        std::fs::write(&a, b"x").unwrap();
        let deny = |p: &Path| p.file_name().is_some_and(|n| n == "keep.txt").then(|| "protected".to_string());
        assert_eq!(rename_on_disk(&a, "keep.txt", &deny), Err(Bad::Stop("protected".into())));
        assert!(a.exists());
    }

    #[test]
    fn a_dotted_name_that_is_no_extension_is_taken_whole() {
        // "Report v1.2 final" is a name, not "Report v1" and an extension
        assert_eq!(new_name("Report v1.2 final", false, "Report final"), Ok("Report final".into()));
        assert_eq!(new_name("notes.backup of monday", false, "notes"), Ok("notes".into()));
        assert_eq!(new_name("a.verylongextension", false, "b"), Ok("b".into()));
        assert_eq!(new_name("photo.JPEG", false, "holiday"), Ok("holiday.JPEG".into()));
        assert_eq!(extension("Report v1.2 final"), None);
        assert_eq!(extension("x.tar"), Some("tar"));
    }

    #[test]
    fn every_device_name_and_the_utf16_length_are_refused() {
        for bad in ["CONIN$", "conout$", "COM\u{b9}", "LPT\u{b3}.txt", "AUX.log"] {
            assert!(invalid_name(bad).is_some(), "{bad:?}");
        }
        // 128 characters of two UTF-16 units each are 256 units: too long for Windows
        assert!(invalid_name(&"\u{1F600}".repeat(128)).is_some());
        assert!(invalid_name(&"\u{1F600}".repeat(127)).is_none());
    }

    #[test]
    fn a_kept_path_is_never_orphaned_by_a_rename() {
        let kept = vec![PathBuf::from(r"D:\Projects\old-build")];
        // the kept folder, one above it and one inside it
        for p in [r"D:\Projects\old-build", r"d:\projects", r"D:\Projects\old-build\src\a.rs"] {
            assert!(kept_refusal(Path::new(p), &kept).is_some(), "{p}");
        }
        // beside it, or a name that merely starts alike
        for p in [r"D:\Projects\other", r"D:\Projects\old-build-2", r"D:\Work"] {
            assert!(kept_refusal(Path::new(p), &kept).is_none(), "{p}");
        }
        assert!(kept_refusal(Path::new(r"D:\x"), &[]).is_none());
    }

    #[test]
    fn a_network_path_is_refused_before_anything_else() {
        for p in [r"\\nas\share\a.txt", r"\\?\UNC\nas\share\a.txt", r"\\.\C:\a.txt"] {
            let why = path_refusal_for_rename(Path::new(p), &[]).unwrap_or_default();
            assert!(why.contains("local drive"), "{p}: {why}");
        }
    }

    #[test]
    fn nothing_is_renamed_through_a_junction() {
        let t = Temp::new("junction");
        let real = t.0.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("a.txt"), b"x").unwrap();
        let link = t.0.join("link");
        let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(&real).output().map(|o| o.status.success()).unwrap_or(false);
        assert!(made, "the test needs to make a junction");
        // the spelling is a plain local path, but a folder above the item is a junction
        match rename_on_disk(&link.join("a.txt"), "b.txt", &allow) {
            Err(Bad::Stop(why)) => assert!(why.contains("hold its folder safely"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert!(real.join("a.txt").exists() && !real.join("b.txt").exists());
    }

    #[test]
    fn a_folder_holding_something_protected_is_not_renamed_from_another_drive() {
        use sprawl_core::{Node, Tree};
        let t = Temp::new("more");
        std::fs::create_dir_all(t.0.join("vms")).unwrap();
        std::fs::create_dir_all(t.0.join("docs")).unwrap();
        let root = t.0.display().to_string();
        let mut nodes = vec![Node { name: root.clone(), is_dir: true, ..Default::default() }];
        for (name, dir, parent) in [("vms", true, 0), ("win11.vhdx", false, 1), ("docs", true, 0), ("notes.txt", false, 3)] {
            let i = nodes.len();
            nodes.push(Node { name: name.into(), is_dir: dir, parent: Some(parent), ..Default::default() });
            nodes[parent].children.push(i);
        }
        let mut tree = Tree { volumes: Vec::new(), root_path: PathBuf::from(&root), nodes, errors: 0, method: Default::default() };
        tree.compute_totals();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        let place = |read_at| crate::search_more::Place { ix: std::sync::Arc::new(sprawl_core::compact::CompactIndex::build(&tree)), as_of: None, read_at, errors: 0, drive: Some("D:\\".into()) };
        let p = place(later);
        // (the index keeps its items in its own order: look the folders up by name)
        let id = |name: &str| (0..p.ix.len() as u32).find(|&i| p.ix.name(i) == name).unwrap() as usize;
        let (vms, docs) = (id("vms"), id("docs"));
        let why = more_folder_refusal(&p, vms, &t.0.join("vms")).unwrap_or_default();
        assert!(why.contains("win11.vhdx") && why.contains("protected"), "{why}");
        assert_eq!(more_folder_refusal(&p, docs, &t.0.join("docs")), None);
        // a read older than the folder says nothing about what is in it now
        let old = place(std::time::UNIX_EPOCH);
        assert!(more_folder_refusal(&old, docs, &t.0.join("docs")).is_some());
        // a relative spelling is refused too
        assert!(path_refusal_for_rename(Path::new("relative.txt"), &[]).is_some());
    }

    #[test]
    fn an_untouched_name_is_no_rename_even_when_trimming_would_change_it() {
        assert_eq!(new_name(" notes.txt", false, " notes.txt"), Err("That is already its name.".into()));
        assert_eq!(new_name(" notes.txt", false, "notes.txt"), Ok("notes.txt".into()), "typing it trimmed is a choice");
    }

    #[test]
    fn a_path_of_more_than_260_characters_is_renamed() {
        let t = Temp::new("long");
        let mut dir = t.0.clone();
        for i in 0..7 {
            dir.push(format!("{i}{}", "d".repeat(49)));
        }
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        std::fs::write(&a, b"x").unwrap();
        assert!(a.as_os_str().len() > 260, "{}", a.as_os_str().len());
        let to = rename_on_disk(&a, "b.txt", &allow).unwrap();
        assert!(std::fs::metadata(&to).is_ok() && std::fs::metadata(&a).is_err());
        // and a new name that is what pushes it past 259
        let c = rename_on_disk(&to, &format!("{}.txt", "n".repeat(240)), &allow).unwrap();
        assert!(std::fs::metadata(&c).is_ok());
        // the verbatim spelling itself
        let w = |p: &Path| String::from_utf16(&crate::win32::verbatim(p)).unwrap();
        assert_eq!(w(Path::new(r"C:\x\y")), "\\\\?\\C:\\x\\y\0");
        assert_eq!(w(Path::new(r"\\srv\share\x")), "\\\\?\\UNC\\srv\\share\\x\0");
        assert_eq!(w(Path::new(r"\\?\C:\x")), "\\\\?\\C:\\x\0");
    }

    /// A tree as an index: (name, is_dir, parent index) in order.
    fn index_of(root: &str, items: &[(String, bool, usize)]) -> std::sync::Arc<sprawl_core::compact::CompactIndex> {
        use sprawl_core::{Node, Tree};
        let mut nodes = vec![Node { name: root.into(), is_dir: true, ..Default::default() }];
        for (name, dir, parent) in items {
            let i = nodes.len();
            nodes.push(Node { name: name.clone(), is_dir: *dir, parent: Some(*parent), ..Default::default() });
            nodes[*parent].children.push(i);
        }
        let mut tree = Tree { volumes: Vec::new(), root_path: PathBuf::from(root), nodes, errors: 0, method: Default::default() };
        tree.compute_totals();
        std::sync::Arc::new(sprawl_core::compact::CompactIndex::build(&tree))
    }

    #[test]
    fn a_big_folder_is_looked_through_quickly_and_a_very_deep_one_completely() {
        let t = Temp::new("big");
        let root = t.0.display().to_string();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        let place = |ix| crate::search_more::Place { ix, as_of: None, read_at: later, errors: 0, drive: Some("D:\\".into()) };
        // 200,000 items in one folder and none protected
        let mut items = vec![("big".to_string(), true, 0)];
        items.extend((0..200_000).map(|i| (format!("file{i}.dat"), false, 1)));
        let p = place(index_of(&root, &items));
        let big = (0..p.ix.len() as u32).find(|&i| p.ix.name(i) == "big").unwrap() as usize;
        let start = std::time::Instant::now();
        assert_eq!(more_folder_refusal(&p, big, &t.0), None);
        assert!(start.elapsed() < std::time::Duration::from_secs(3), "{:?}", start.elapsed());
        // a folder 256 levels down: below depth 255 the index's depth no longer separates what
        // is inside from what follows, and a disk image at the bottom is still found
        let mut items = Vec::new();
        for i in 0..260 {
            items.push((format!("d{i}"), true, i));
        }
        items.push(("win11.vhdx".to_string(), false, 260));
        let p = place(index_of(&root, &items));
        let at = (0..p.ix.len() as u32).find(|&i| p.ix.name(i) == "d256").unwrap();
        assert_eq!(p.ix.depth(at), 255, "depth is saturated here");
        let why = more_folder_refusal(&p, at as usize, &t.0).unwrap_or_default();
        assert!(why.contains("win11.vhdx"), "{why}");
    }

    #[test]
    fn files_go_on_the_clipboard_as_a_drop_list() {
        let b = crate::win32::hdrop_bytes(&[PathBuf::from(r"C:\a.txt"), PathBuf::from(r"D:\x y\b")]);
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        assert_eq!(u32_at(0), 20, "the names start after the 20-byte header");
        assert_eq!(u32_at(16), 1, "wide names");
        let names: Vec<u16> = b[20..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let text = String::from_utf16(&names).unwrap();
        assert_eq!(text, "C:\\a.txt\0D:\\x y\\b\0\0");
    }
}
