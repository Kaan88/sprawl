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
const DEVICES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Why `name` can't be a file or folder name on Windows.
pub(crate) fn invalid_name(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Type a name.".into());
    }
    if name.chars().count() > 255 {
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

/// The extension of file name `name` (what follows its last dot, if both parts are not empty).
fn extension(name: &str) -> Option<&str> {
    name.rsplit_once('.').filter(|(a, b)| !a.is_empty() && !b.is_empty()).map(|(_, e)| e)
}

/// What `typed` makes of the name `old`: a file keeps its extension unless what was typed has a
/// dot of its own. Err says what is wrong with it.
pub(crate) fn new_name(old: &str, is_dir: bool, typed: &str) -> Result<String, String> {
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
        if let Some(why) = crate::commands::path_refusal(path) {
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
                if let Some((at, p)) = never.hit_inside(t, n) {
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
                    return Some("It is, or holds, the scan on screen; rename it from the Tree or Files tab.".into());
                }
                let Some((ix, i)) = m.item(k, id) else { return Some("That result is gone from the list.".into()) };
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
        match rename_on_disk(&path, &name, &crate::commands::path_refusal) {
            Ok(_) => {}
            Err(Bad::Name(why)) => return self.rename_ask(target, path, is_dir, &name, &why),
            Err(Bad::Stop(why)) => return self.tell("Not renamed", &format!("{why}\n\n{}", path.display())),
        };
        let done = format!("Renamed {old_name} to {name}");
        match target {
            Target::Scan(..) => {
                // the scan on screen: its folder is read again, as after the Windows menu's own
                // Rename (Delete and the like wait for that to finish)
                let Some(root) = self.tree.as_ref().map(|t| t.path_of(sprawl_core::ROOT)) else { return };
                let again = crate::shellmenu::rescan_target(&path, &root);
                let (tx, rx) = std::sync::mpsc::channel();
                self.shell_rescans = Some(rx);
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
                text(&self.root, "SearchInfo", &format!("{done}. It shows under its new name once the drive is read again (Refresh under the results)."));
            }
        }
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
            "Couldn't put the results on the clipboard (another program has it).".to_string()
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
