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

use crate::used::{drive_of, refusal_of, tally_reason, Unseen};

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
    /// How many answers had no time for each reason, each said with its own.
    reasons: Vec<(&'static str, usize)>,
}

impl Created {
    /// For an answer that may look at the disk.
    pub fn new() -> Created {
        Created { left: ANSWER_BUDGET, excluded: crate::exclusions::configured(), drives: Default::default(), parent: None, unseen: None, reasons: Vec::new() }
    }

    /// For a view that holds no disk times at all (a client that may only see the scan, or a
    /// view loaded from a CSV file): nothing is read, and no note is added about the disk.
    pub fn off_for(unseen: Unseen) -> Created {
        Created { left: 0, excluded: Vec::new(), drives: Default::default(), parent: None, unseen: Some(unseen), reasons: Vec::new() }
    }

    /// The same with a given reading limit, to see what running out says.
    #[cfg(test)]
    pub(crate) fn with_budget(left: usize) -> Created {
        Created { left, excluded: Vec::new(), drives: Default::default(), parent: None, unseen: None, reasons: Vec::new() }
    }

    /// The same with given folders to skip, to see what a skipped folder says.
    #[cfg(test)]
    pub(crate) fn with_excluded(left: usize, excluded: Vec<PathBuf>) -> Created {
        Created { left, excluded, drives: Default::default(), parent: None, unseen: None, reasons: Vec::new() }
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
            self.tally(SKIPPED_UNKNOWN);
            return Born::Unknown;
        }
        if self.excluded.iter().any(|p| crate::finding::within(path, p)) {
            self.tally(SKIPPED);
            return Born::Unknown;
        }
        let Some(drive) = drive_of(path) else {
            self.tally(NOT_FIXED);
            return Born::Unknown;
        };
        if !*self.drives.entry(drive).or_insert_with(|| crate::used::fixed_drive(drive)) {
            self.tally(NOT_FIXED);
            return Born::Unknown;
        }
        self.left -= 1;
        match self.time_of(path) {
            Ok(t) => Born::At(t),
            Err(e) => {
                // only NotFound (of the item or of its folder) means it is gone: a link, a
                // cloud-only file or a name Sprawl doesn't open is a different answer
                self.tally(refusal_of(&e));
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
        self.pin(parent)?;
        let Some((_, dir)) = self.parent.as_ref() else { return Err(std::io::Error::other("the folder couldn't be pinned")) };
        use sprawl_core::local::{Disposition, Kind, READ_ATTRIBUTES, SHARE_DELETE, SHARE_READ, SHARE_WRITE};
        let file = dir.open_in(name, READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE | SHARE_DELETE, Disposition::Open, Kind::Either, false)?;
        file.metadata()?.created()
    }

    /// The folder at `path` pinned, reusing the one already held when it is the same folder.
    fn pin(&mut self, path: &Path) -> std::io::Result<()> {
        if self.parent.as_ref().is_some_and(|(p, _)| p == path) {
            return Ok(());
        }
        self.parent = None;
        let dir = sprawl_core::local::directory(path)?;
        self.parent = Some((path.to_path_buf(), dir));
        Ok(())
    }

    fn tally(&mut self, why: &'static str) {
        tally_reason(&mut self.reasons, why);
    }

    /// Whether `path` is on a local fixed disk, whose creation times are read at all.
    pub fn reads_drive(&mut self, path: &Path) -> bool {
        match drive_of(path) {
            Some(d) => *self.drives.entry(d).or_insert_with(|| crate::used::fixed_drive(d)),
            None => false,
        }
    }

    /// The first reason no time could be read (a test's shorthand).
    #[cfg(test)]
    fn why(&self) -> Option<&'static str> {
        self.reasons.first().map(|r| r.0)
    }

    /// One line per reason a match had no creation time.
    pub fn notes(&self, one: &str, many: &str) -> Vec<String> {
        crate::used::reason_lines(&self.reasons, one, many, "with no creation time to check")
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
        // a name Sprawl's plain-name check refuses (a trailing dot): not "the item is gone"
        let long = "bad.";
        let mut refused = Created::with_budget(10);
        assert_eq!(refused.of(&dir.join(long)), Born::Unknown);
        let why = refused.why().unwrap();
        assert!(!why.contains("gone"), "{why}");
        assert!(why.contains("isn't read"), "{why}");
        // a folder that can't be pinned is the same kind of answer
        let mut off = Created::with_budget(10);
        assert_eq!(off.of(&dir.join("no-such-folder").join("x.txt")), Born::Unknown);
        // a folder that is gone since the scan means the item is gone with it
        assert!(off.why().unwrap().contains("is gone"), "{}", off.why().unwrap());
        // only a leaf that is really gone says so
        let mut gone = Created::with_budget(10);
        assert_eq!(gone.of(&dir.join("gone.txt")), Born::Unknown);
        assert!(gone.why().unwrap().contains("is gone"), "{}", gone.why().unwrap());
        drop(refused);
        drop(off);
        drop(gone);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// One answer over a deleted file, a refused name and a file under a deleted folder gives a
    /// count for each reason: only the vanished ones say gone, and a long non-Latin name (an
    /// ordinary NTFS name) is read (times-17, times-19).
    #[test]
    fn each_reason_has_its_own_line() {
        let dir = std::env::temp_dir().join(format!("sprawl-born-lines-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let long = format!("{}.txt", "\u{4e2d}".repeat(100));
        std::fs::write(dir.join(&long), b"x").unwrap();
        let mut c = Created::with_budget(10);
        assert!(matches!(c.of(&dir.join(&long)), Born::At(_)), "a 100 character CJK name is read");
        assert_eq!(c.of(&dir.join("gone.txt")), Born::Unknown);
        assert_eq!(c.of(&dir.join("bad.")), Born::Unknown);
        assert_eq!(c.of(&dir.join("no-folder").join("x.txt")), Born::Unknown);
        let lines = c.notes("match", "matches");
        let said = lines.join("\n");
        assert_eq!(lines.len(), 2, "{said}");
        assert!(lines.iter().any(|l| l.starts_with("2 matches") && l.contains("the item is gone")), "{said}");
        assert!(lines.iter().any(|l| l.starts_with("1 match ") && l.contains("isn't read") && !l.contains("gone")), "{said}");
        drop(c);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
