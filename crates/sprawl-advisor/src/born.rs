//! Creation times, read one path at a time: `dc:` in a search.
//!
//! A scan keeps a name, a size, an attribute word and the modified time; a creation time would
//! be another 8 bytes for every one of a drive's 3.4 million items (see `Node`'s memory note),
//! so it is not kept and `dc:` reads it for the matches it is asked about, as `da:` reads
//! last-access times ([`crate::used`]). Only a local fixed drive is read (a share would be
//! crawled), and one answer reads at most [`ANSWER_BUDGET`] of them.
//!
//! No file is opened: [`std::fs::symlink_metadata`] asks Windows for the attributes and times
//! of the path itself (a link's own time, never its target's), which is one call and can't act
//! on anything.

use std::path::Path;
use std::time::SystemTime;

/// What one path's creation time gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Born {
    /// When it was created.
    At(SystemTime),
    /// Not read: this answer's reading limit ran out first. Never take it for "not created".
    Unread,
    /// No time here: this drive isn't a local fixed disk, or the path is gone.
    Unknown,
}

/// The most creation times one answer reads: the same budget a `da:` answer has.
pub const ANSWER_BUDGET: usize = crate::used::ANSWER_BUDGET;

const NOT_FIXED: &str = "this drive isn't a local fixed disk, so its times aren't read";
const NO_TIME: &str = "the item is gone, or Windows keeps no creation time for it";

/// Creation times for one answer: each path is one call, and the work is bounded.
pub struct Created {
    /// Reads still allowed for this answer.
    left: usize,
    /// Whether each drive letter looked at is a local fixed disk.
    drives: std::collections::HashMap<char, bool>,
    /// Why no time can be read, when that is the drive rather than the limit.
    why: Option<&'static str>,
}

impl Created {
    /// For an answer that may look at the disk.
    pub fn new() -> Created {
        Created { left: ANSWER_BUDGET, drives: Default::default(), why: None }
    }

    /// The same with a given reading limit, to see what running out says.
    #[cfg(test)]
    pub(crate) fn with_budget(left: usize) -> Created {
        Created { left, drives: Default::default(), why: None }
    }

    /// When the item at `path` was created.
    pub fn of(&mut self, path: &Path) -> Born {
        if self.left == 0 {
            return Born::Unread;
        }
        let Some(drive) = path.to_string_lossy().chars().next().map(|c| c.to_ascii_uppercase()).filter(|c| c.is_ascii_alphabetic()) else {
            self.why.get_or_insert(NOT_FIXED);
            return Born::Unknown;
        };
        if !*self.drives.entry(drive).or_insert_with(|| crate::used::fixed_drive(drive)) {
            self.why.get_or_insert(NOT_FIXED);
            return Born::Unknown;
        }
        self.left -= 1;
        match std::fs::symlink_metadata(path).and_then(|m| m.created()) {
            Ok(t) => Born::At(t),
            Err(_) => {
                self.why.get_or_insert(NO_TIME);
                Born::Unknown
            }
        }
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

    /// A file's own creation time, a path that is gone, and a limit that ran out.
    #[test]
    fn one_answers_creation_times() {
        let dir = std::env::temp_dir().join(format!("sprawl-born-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("kept.txt");
        std::fs::write(&file, b"x").unwrap();
        let mut c = Created::new();
        match c.of(&file) {
            Born::At(t) => assert!(t > SystemTime::UNIX_EPOCH),
            other => panic!("a file's own time: {other:?}"),
        }
        assert_eq!(c.of(&dir.join("gone.txt")), Born::Unknown);
        assert!(c.why().is_some());
        let mut none = Created::with_budget(0);
        assert_eq!(none.of(&file), Born::Unread, "a limit that ran out is not a no");
        assert_eq!(none.left(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
