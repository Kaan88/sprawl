//! Creation times, read one path at a time: `dc:` in a search.
//!
//! A scan keeps a name, a size, an attribute word and the modified time; a creation time would
//! be another 8 bytes for every one of a drive's 3.4 million items (see `Node`'s memory note),
//! so it is not kept and `dc:` reads it for the matches it is asked about, as `da:` reads
//! last-access times ([`crate::used`]). Only a local fixed drive is read (a share would be
//! crawled), a folder skipped in Settings > Scanning is not read, and one answer reads at most
//! [`ANSWER_BUDGET`] of them.
//!
//! No file contents are read: the item is opened through its folder's handle (so a folder
//! replaced by a junction since the scan is refused, never followed) with attributes only and
//! every sharing mode, which asks Windows for the times of the item itself and can't act on it.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::used::{drive_of, Unseen};

/// What one path's creation time gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Born {
    /// When it was created.
    At(SystemTime),
    /// Not read: this answer's reading limit ran out first. Never take it for "not created".
    Unread,
    /// No time here: this drive isn't a local fixed disk, the path is skipped, gone, a link or a
    /// cloud file that isn't on this PC, or the view holds no times.
    Unknown,
}

/// The most creation times one answer reads: the same budget a `da:` answer has.
pub const ANSWER_BUDGET: usize = crate::used::ANSWER_BUDGET;

const NOT_FIXED: &str = "this drive isn't a local fixed disk, so its times aren't read";
const SKIPPED: &str = "it is in a folder skipped in Settings > Scanning";
const SKIPPED_UNKNOWN: &str = "the folders to skip in Settings > Scanning couldn't be read";
/// The only reason that means the item isn't there: an open that says NotFound.
const GONE: &str = "the item is gone";
/// Everything else an open refuses: a link or a cloud file is never followed or recalled, and a
/// name Windows can't take is never opened. None of these means the item is gone (times-13).
const NOT_READ: &str = "it is a link, a cloud file that isn't on this PC, a name Windows can't open, or under a link, so it isn't read";

/// Creation times for one answer: each path is one call, the folders to skip are read once, and
/// the work is bounded.
pub struct Created {
    /// Reads still allowed for this answer.
    left: usize,
    /// Folders to skip, read once for the answer.
    excluded: Vec<PathBuf>,
    /// Whether each drive letter looked at is a local fixed disk.
    drives: std::collections::HashMap<char, bool>,
    /// The folder last pinned: a search's matches are often siblings, and one open serves them
    /// all instead of opening the whole chain for each file.
    parent: Option<(PathBuf, sprawl_core::local::Directory)>,
    /// Nothing is read: a client that may only see the scan, or a view from a CSV file.
    unseen: Option<Unseen>,
    /// Why no time can be read, when that is the drive or a skip rather than the limit.
    why: Option<&'static str>,
}

impl Created {
    /// For an answer that may look at the disk.
    pub fn new() -> Created {
        Created { left: ANSWER_BUDGET, excluded: crate::exclusions::configured(), drives: Default::default(), parent: None, unseen: None, why: None }
    }

    /// For a view that holds no disk times at all (a client that may only see the scan, or a
    /// view loaded from a CSV file): nothing is read, and no note is added about the disk.
    pub fn off_for(unseen: Unseen) -> Created {
        Created { left: 0, excluded: Vec::new(), drives: Default::default(), parent: None, unseen: Some(unseen), why: None }
    }

    /// The same with a given reading limit, to see what running out says.
    #[cfg(test)]
    pub(crate) fn with_budget(left: usize) -> Created {
        Created { left, excluded: Vec::new(), drives: Default::default(), parent: None, unseen: None, why: None }
    }

    /// The same with given folders to skip, to see what a skipped folder says.
    #[cfg(test)]
    pub(crate) fn with_excluded(left: usize, excluded: Vec<PathBuf>) -> Created {
        Created { left, excluded, drives: Default::default(), parent: None, unseen: None, why: None }
    }

    /// When the item at `path` was created.
    pub fn of(&mut self, path: &Path) -> Born {
        if self.unseen.is_some() {
            return Born::Unknown;
        }
        if self.left == 0 {
            return Born::Unread;
        }
        // a folder the user skips is not read, and a list that can't be read refuses them all
        if crate::exclusions::unreadable(&self.excluded) {
            self.why.get_or_insert(SKIPPED_UNKNOWN);
            return Born::Unknown;
        }
        if self.excluded.iter().any(|p| crate::finding::within(path, p)) {
            self.why.get_or_insert(SKIPPED);
            return Born::Unknown;
        }
        let Some(drive) = drive_of(path) else {
            self.why.get_or_insert(NOT_FIXED);
            return Born::Unknown;
        };
        if !*self.drives.entry(drive).or_insert_with(|| crate::used::fixed_drive(drive)) {
            self.why.get_or_insert(NOT_FIXED);
            return Born::Unknown;
        }
        self.left -= 1;
        match self.time_of(path) {
            Ok(t) => Born::At(t),
            Err(e) => {
                // only NotFound means it is gone: a link, a cloud-only file, a name Windows
                // can't open or a folder that can't be pinned is a different answer
                self.why.get_or_insert(if e.kind() == std::io::ErrorKind::NotFound { GONE } else { NOT_READ });
                Born::Unknown
            }
        }
    }

    /// The creation time of `path`, opened as itself through its folder's handle with attributes
    /// only and full sharing (so a rename or delete is never held up), reusing the folder last
    /// pinned for the next sibling.
    fn time_of(&mut self, path: &Path) -> std::io::Result<SystemTime> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
            // a drive's root: nothing above it to pin
            return std::fs::symlink_metadata(path)?.created();
        };
        if !self.pin(parent) {
            return Err(std::io::Error::other("the folder couldn't be pinned"));
        }
        let Some((_, dir)) = self.parent.as_ref() else { return Err(std::io::Error::other("the folder couldn't be pinned")) };
        use sprawl_core::local::{Disposition, Kind, READ_ATTRIBUTES, SHARE_DELETE, SHARE_READ, SHARE_WRITE};
        let file = dir.open_in(name, READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE | SHARE_DELETE, Disposition::Open, Kind::Either, false)?;
        file.metadata()?.created()
    }

    /// The folder at `path` pinned, reusing the one already held when it is the same folder.
    fn pin(&mut self, path: &Path) -> bool {
        if self.parent.as_ref().is_some_and(|(p, _)| p == path) {
            return true;
        }
        self.parent = None;
        let Ok(dir) = sprawl_core::local::directory(path) else { return false };
        self.parent = Some((path.to_path_buf(), dir));
        true
    }

    /// Why no time can be read, when that is the drive rather than the limit.
    pub fn why(&self) -> Option<&'static str> {
        self.why
    }

    /// Reads still allowed for this answer.
    pub fn left(&self) -> usize {
        self.left
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file's own creation time, a path that is gone, a limit that ran out, a folder skipped
    /// in Settings (never read), and a path that isn't a plain `X:\` one.
    #[test]
    fn one_answers_creation_times() {
        let dir = std::env::temp_dir().join(format!("sprawl-born-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("skipped")).unwrap();
        std::fs::write(dir.join("kept.txt"), b"x").unwrap();
        std::fs::write(dir.join("skipped").join("secret.txt"), b"x").unwrap();
        // a plain list of folders to skip, not this PC's settings (a test must not depend on them)
        let mut c = Created::with_excluded(ANSWER_BUDGET, Vec::new());
        match c.of(&dir.join("kept.txt")) {
            Born::At(t) => assert!(t > SystemTime::UNIX_EPOCH),
            other => panic!("a file's own time: {other:?}"),
        }
        assert_eq!(c.of(&dir.join("gone.txt")), Born::Unknown);
        assert!(c.why().is_some());
        let mut none = Created::with_budget(0);
        assert_eq!(none.of(&dir.join("kept.txt")), Born::Unread, "a limit that ran out is not a no");
        assert_eq!(none.left(), 0);
        // a skipped folder is unknown, says why, and spends no read
        let mut skipped = Created::with_excluded(10, vec![dir.join("skipped")]);
        assert_eq!(skipped.of(&dir.join("skipped").join("secret.txt")), Born::Unknown);
        assert!(skipped.why().unwrap().contains("skipped in Settings > Scanning"));
        assert_eq!(skipped.left(), 10, "no read is spent on a skipped folder");
        // a list of folders to skip that can't be read refuses every path
        let mut unreadable = Created::with_excluded(10, vec![PathBuf::from(sprawl_core::progress::UNREADABLE_POLICY)]);
        assert_eq!(unreadable.of(&dir.join("kept.txt")), Born::Unknown);
        assert!(unreadable.why().unwrap().contains("couldn't be read"));
        assert_eq!(unreadable.left(), 10);
        // a relative or drive-relative path is never resolved against this folder
        let mut shape = Created::with_budget(10);
        assert_eq!(shape.of(Path::new(r"kept.txt")), Born::Unknown);
        assert_eq!(shape.of(Path::new(r"C:kept.txt")), Born::Unknown);
        assert_eq!(shape.left(), 10);
        // a view that holds no times reads nothing
        assert_eq!(Created::off_for(Unseen::ScanOnly).of(&dir.join("kept.txt")), Born::Unknown);
        // the folder last read stays pinned for its siblings: let it go before the folder goes
        drop(c);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A refused open says why it wasn't read and never that the item is gone; only a real
    /// `NotFound` says that (times-13).
    #[test]
    fn a_refused_open_is_not_said_to_be_gone() {
        let dir = std::env::temp_dir().join(format!("sprawl-born-refused-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 264 bytes of name (134 UTF-16 units): Windows can hold it, Sprawl's plain-name check
        // can't take it, and that is not "the item is gone"
        let long = format!("{}.txt", "\u{e9}".repeat(130));
        std::fs::write(dir.join(&long), b"x").unwrap();
        let mut refused = Created::with_budget(10);
        assert_eq!(refused.of(&dir.join(&long)), Born::Unknown);
        let why = refused.why().unwrap();
        assert!(!why.contains("gone"), "{why}");
        assert!(why.contains("isn't read"), "{why}");
        // a folder that can't be pinned is the same kind of answer
        let mut off = Created::with_budget(10);
        assert_eq!(off.of(&dir.join("no-such-folder").join("x.txt")), Born::Unknown);
        assert!(!off.why().unwrap().contains("gone"), "{}", off.why().unwrap());
        // only a leaf that is really gone says so
        let mut gone = Created::with_budget(10);
        assert_eq!(gone.of(&dir.join("gone.txt")), Born::Unknown);
        assert!(gone.why().unwrap().contains("is gone"), "{}", gone.why().unwrap());
        drop(refused);
        drop(off);
        drop(gone);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
