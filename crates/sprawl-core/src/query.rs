//! The search box language: [`Query::parse`] reads what was typed, [`crate::search::search`]
//! and [`crate::compact::CompactIndex::search`] find what matches. [`HELP`] lists it for the
//! app's empty state.
//!
//! A query is words separated by spaces; every word must match ("and"). A word is:
//!
//! * text, found anywhere in the name, any case: `report`; with `*` and `?` it is a pattern
//!   for the whole name: `*.iso`, `img_????.jpg`; with `\` (or `/`) it is matched against the
//!   full path, where folders end in `\`: `\steam\ *.vdf`;
//! * alternatives: `a|b` or `a;b`; groups: `<a b>|c` is both words of the group or `c`, groups
//!   nest and `!<a b>` is not both of them;
//! * `!word`: not;
//! * `>1gb`, `<10kb`, `>=5mb`, `<=1k`: size (1 kb = 1024 bytes; a folder's size is its total);
//! * `folder:` or `file:` in front: one kind only (`folder:cache`);
//! * modifiers in front of text, in any order: `case:` (match case), `exact:` or
//!   `wholename:` (the whole name, not part of it), `path:` (the full path even without `\`);
//! * keywords: `ext:jpg;png`, `dm:2026-11` (modified; see [`HELP`]), `size:100mb..2gb`,
//!   `depth:<=3`, `regex:^ab+c$`, `parent:C:\Games` (directly inside), `child:*.tmp` (folders
//!   holding one), `childcount:>10000`, `childfilecount:>1000`, `attrib:hs`, `len:>260` (path
//!   length), `dupe:` and `sizedupe:` (another match with the same name or size);
//! * type macros: `audio:`, `video:`, `pic:`, `doc:`, `exe:`, `zip:` (the Type filter's lists).
//!
//! `dc:` (created) and `da:` (last opened) are not in the scan: [`split_read_dates`] takes them
//! out and gives the times to read for each match.
//!
//! Quotes keep spaces, and whatever is inside them is plain text: `"!x"`, `"a;b"`, `"dm:x"`.
//! A word whose `!`, keyword or `|`/`;` would otherwise mean something can be quoted to be
//! looked for as it is (`*` and `?` stay wildcards: a name cannot hold them).
//!
//! In `a|b`, an alternative without a keyword of its own takes the first one's: `ext:jpg|png`
//! is two extensions, `foo|ext:iso` is the text `foo` or the extension `iso`. A `regex:` takes
//! the rest of the word as it is (its `|` is the regex's).
//!
//! Something unknown before a `:` (`C:`, `foo:bar`) is text. A keyword whose value cannot be
//! read (`dm:2026-13`, `size:lots`, `regex:(`) is an error: [`Query::errors`] says what, and
//! the query finds nothing until it is fixed. A keyword with nothing after it (`ext:` while it
//! is being typed) is ignored.

use std::borrow::Cow;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::bytes::{Regex, RegexBuilder};

use crate::filter::{wildcard, wildcard_case};

/// What the search box understands, one feature per line: an example, a tab, what it does.
pub const HELP: &str = "\
report 2024\tboth words, anywhere in the name, any case (a space means and)
\"my file\"\tquotes keep spaces; inside them ! | ; and keywords are plain text
*.iso  img_????.jpg\t* is any run of characters, ? one character, for the whole name
*.log|*.tmp\teither one (also *.log;*.tmp)
<a b>|c\tall of a group's words, or c: <report 2024>|draft (groups nest; !<temp cache> is not both)
!tmp\tnot
\\steam\\ *.vdf\ta word with \\ or / is matched against the full path (folders end in \\)
path:steam\tmatch against the full path, without needing a \\
>1gb  <10kb  >=5mb\tsize (1 kb = 1024 bytes; a folder's size is its total)
size:100mb..2gb\tsize range, both ends included; also size:>1gb, size:empty
ext:jpg;png\tfiles with one of these extensions
dm:2026-11\tmodified in that month; also dm:2026 or dm:2026-01-15 (local time)
dm:>2026-01-15\tmodified after that day; also <, >= and <=
dm:2026-01..2026-03\tmodified from the start of the first to the end of the last
dm:today\talso yesterday, lastweek (7 days), lastmonth, thismonth, thisyear
dc:2026-11\tcreated in that month (same forms as dm:; give it another word too)
da:>2026-01-15\tlast opened after that day (dc: and da: read each match's own time, so narrow the search)
regex:^log\\d+\\.txt$\ta regular expression on the name (on the full path if it has \\\\)
case:README\tmatch upper and lower case exactly
exact:desktop.ini\tthe whole name, not part of it (also wholename:)
depth:<=2\tat most two levels below the scanned folder
parent:C:\\Games\\Steam\tonly what is directly inside that folder (parent:steam: a folder named steam)
child:*.tmp\tfolders that directly hold a *.tmp file or folder
childcount:>10000\tfolders directly holding more than 10,000 files and folders
childfilecount:>10000\tfolders directly holding more than 10,000 files
dupe:\tfiles and folders with another match of the same name (any case)
sizedupe:\tfiles with another match of the same size
attrib:hs\thidden and system: h hidden, s system, r read-only, l link or junction, c compressed
len:>260\ta full path longer than 260 characters (also len:<=200, len:100..260)
audio:  video:  pic:\tfiles of that kind, the same lists as the Type filter
doc:  exe:  zip:\tdocuments, programs and archives, the same lists as the Type filter
compressed:\tcompression recorded by this scan; fast folder scans may omit hidden WOF backing (MFT and detailed checks retain it); compressed:no for others
folder:  file:\tfolders only, files only (folder:cache)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Any,
    Files,
    Folders,
}

/// One alternative of a word. Text is lower-case unless it is a `Case…` one.
#[derive(Debug, Clone)]
pub(crate) enum Pat {
    /// Anywhere in the name.
    Sub(Vec<u8>),
    /// The whole name, with `*` and `?`.
    Glob(Vec<char>),
    /// Anywhere in the full path.
    PathSub(Vec<u8>),
    /// The whole path, with `*` and `?`.
    PathGlob(Vec<char>),
    /// The whole name.
    Exact(Vec<u8>),
    /// The whole path (a trailing `\` ignored on both sides).
    PathExact(Vec<u8>),
    CaseSub(Vec<u8>),
    CaseGlob(Vec<char>),
    CaseExact(Vec<u8>),
    CasePathSub(Vec<u8>),
    CasePathGlob(Vec<char>),
    CasePathExact(Vec<u8>),
    /// A file's extension, without the dot.
    Ext(Vec<u8>),
    ExtGlob(Vec<char>),
    Regex { re: Box<Regex>, path: bool, case: bool },
    /// Bytes, both ends included.
    Size(u64, u64),
    /// Modified, seconds since 1970 (UTC): from the first, up to but not including the second.
    Date(i64, i64),
    /// Levels below the root (its children are 1), both ends included.
    Depth(u32, u32),
    Compressed(bool),
    /// Folders only or files only, as a word of a group (`<folder:a b>`): at the top level the
    /// whole query takes `folder:` instead.
    Kind(Kind),
    /// `parent:`: the item is directly inside that folder (lower case; a name with no `\`
    /// matches a folder of that name anywhere).
    Parent { path: Vec<u8>, case: bool },
    /// `child:`: a folder whose direct children include one matching these words.
    Child(Vec<Term>),
    /// Direct children (files and folders), both ends included.
    ChildCount(u32, u32),
    /// Direct child files, both ends included.
    ChildFiles(u32, u32),
    /// `attrib:hsrl`: the `FILE_ATTRIBUTE_*` bits `mask` all set, and compressed when asked.
    Attrib { mask: u32, compressed: bool },
    /// The full path's length in characters, both ends included.
    Len(u32, u32),
}

/// What a pattern needs to look at.
pub(crate) const NEED_NAME: u8 = 1;
pub(crate) const NEED_PATH: u8 = 2;
pub(crate) const NEED_CASE_NAME: u8 = 4;
pub(crate) const NEED_CASE_PATH: u8 = 8;

impl Pat {
    fn needs(&self) -> u8 {
        match self {
            Pat::Sub(_) | Pat::Glob(_) | Pat::Exact(_) | Pat::Ext(_) | Pat::ExtGlob(_) => NEED_NAME,
            Pat::PathSub(_) | Pat::PathGlob(_) | Pat::PathExact(_) => NEED_PATH,
            Pat::CaseSub(_) | Pat::CaseGlob(_) | Pat::CaseExact(_) => NEED_CASE_NAME,
            Pat::CasePathSub(_) | Pat::CasePathGlob(_) | Pat::CasePathExact(_) => NEED_CASE_PATH,
            Pat::Regex { path, case, .. } => match (path, case) {
                (false, false) => NEED_NAME,
                (false, true) => NEED_CASE_NAME,
                (true, false) => NEED_PATH,
                (true, true) => NEED_CASE_PATH,
            },
            Pat::Size(..) | Pat::Date(..) | Pat::Depth(..) | Pat::Compressed(_) | Pat::Kind(_) => 0,
            Pat::Child(_) | Pat::ChildCount(..) | Pat::ChildFiles(..) => 0,
            Pat::Attrib { .. } => 0,
            Pat::Parent { case, .. } => if *case { NEED_CASE_PATH } else { NEED_PATH },
            Pat::Len(..) => NEED_CASE_PATH,
        }
    }

    #[inline]
    fn matches<I: Item>(&self, it: &I) -> bool {
        match self {
            Pat::Sub(s) => contains(it.name(), s),
            Pat::Glob(g) => std::str::from_utf8(it.name()).is_ok_and(|n| wildcard(g, n)),
            Pat::PathSub(s) => contains(it.path(), s),
            Pat::PathGlob(g) => std::str::from_utf8(it.path()).is_ok_and(|n| wildcard(g, n)),
            Pat::Exact(s) => it.name() == &s[..],
            Pat::PathExact(s) => trim_sep(it.path()) == &s[..],
            Pat::CaseSub(s) => contains(&it.orig_name(), s),
            Pat::CaseGlob(g) => std::str::from_utf8(&it.orig_name()).is_ok_and(|n| wildcard_case(g, n)),
            Pat::CaseExact(s) => *it.orig_name() == s[..],
            Pat::CasePathSub(s) => contains(it.orig_path(), s),
            Pat::CasePathGlob(g) => std::str::from_utf8(it.orig_path()).is_ok_and(|n| wildcard_case(g, n)),
            Pat::CasePathExact(s) => trim_sep(it.orig_path()) == &s[..],
            Pat::Ext(e) => !it.is_dir() && extension(it.name()) == Some(&e[..]),
            Pat::ExtGlob(g) => {
                !it.is_dir() && extension(it.name()).and_then(|x| std::str::from_utf8(x).ok()).is_some_and(|x| wildcard(g, x))
            }
            Pat::Regex { re, path, case } => match (path, case) {
                (false, false) => re.is_match(it.name()),
                (false, true) => re.is_match(&it.orig_name()),
                (true, false) => re.is_match(it.path()),
                (true, true) => re.is_match(it.orig_path()),
            },
            Pat::Size(a, b) => (*a..=*b).contains(&it.size()),
            Pat::Date(a, b) => it.modified().is_some_and(|m| *a <= m && m < *b),
            Pat::Depth(a, b) => (*a..=*b).contains(&it.depth()),
            Pat::Compressed(yes) => it.compressed() == *yes,
            Pat::Kind(k) => match k {
                Kind::Any => true,
                Kind::Files => !it.is_dir(),
                Kind::Folders => it.is_dir(),
            },
            Pat::Parent { path, case } => {
                let p = if *case { parent_path(it.orig_path()) } else { parent_path(it.path()) };
                p == &path[..] || (!path.is_empty() && p.len() > path.len() && p.ends_with(path) && p[p.len() - path.len() - 1] == b'\\')
            }
            Pat::Child(ts) => it.child_matches(ts),
            // counts are of a folder's children: a file has none, and `childcount:0` means the
            // empty folders, not every file
            Pat::ChildCount(a, b) => it.is_dir() && (*a..=*b).contains(&it.children()),
            Pat::ChildFiles(a, b) => it.is_dir() && (*a..=*b).contains(&it.child_files()),
            Pat::Attrib { mask, compressed } => it.attributes() & mask == *mask && (!*compressed || it.compressed()),
            Pat::Len(a, b) => {
                // the path as it is on disk, in UTF-16 units as Windows counts it (lower-casing
                // changes the length of some letters: a Turkish İ becomes two characters)
                let p = trim_sep(it.orig_path());
                let n = std::str::from_utf8(p).map_or(p.len(), |s| s.encode_utf16().count());
                (*a as usize..=*b as usize).contains(&n)
            }
        }
    }
}

/// The folder an item's path is in: without a trailing `\` and, for something at the top of a
/// volume, `c:` (the folder the query's `c:\` is trimmed to as well).
fn parent_path(p: &[u8]) -> &[u8] {
    let p = trim_sep(p);
    match p.iter().rposition(|&b| b == b'\\') {
        Some(i) => &p[..i],
        None => p,
    }
}

fn trim_sep(p: &[u8]) -> &[u8] {
    p.strip_suffix(b"\\").unwrap_or(p)
}

/// The part after the last dot, as [`crate::extension_of`] has it: none for `.gitignore` or
/// `name.`.
fn extension(name: &[u8]) -> Option<&[u8]> {
    let i = name.iter().rposition(|&b| b == b'.')?;
    (i > 0 && i + 1 < name.len()).then(|| &name[i + 1..])
}

/// What a word is matched against. `path` and `orig_path` are only asked for words that need
/// them (see [`Term::on_path`]); the others are always there.
///
/// Cost: a word is only ever checked when it is in the query, so a query without the newer
/// keywords does exactly the work it did before. `parent:` and `len:` need the full path (as
/// `path:` does, so the walk builds it for the candidates only); `child:`, `childcount:` and
/// `childfilecount:` look at a folder's children, which the scan already holds (the compact
/// index finds them in its own run of ids); `dc:` and `da:` are not a [`Pat`] at all: they are
/// read for each match by the caller ([`split_read_dates`]).
pub(crate) trait Item {
    /// The name, lower-case.
    fn name(&self) -> &[u8];
    /// The name as it is on disk.
    fn orig_name(&self) -> Cow<'_, [u8]>;
    /// The full path, lower-case, folders ending in `\`.
    fn path(&self) -> &[u8];
    /// The full path as it is on disk, folders ending in `\`.
    fn orig_path(&self) -> &[u8];
    fn is_dir(&self) -> bool;
    fn size(&self) -> u64;
    /// Seconds since 1970 (UTC).
    fn modified(&self) -> Option<i64>;
    fn depth(&self) -> u32;
    fn compressed(&self) -> bool {
        false
    }
    /// Direct children, files and folders (a file has none).
    fn children(&self) -> u32 {
        0
    }
    /// Of the direct children, the files.
    fn child_files(&self) -> u32 {
        0
    }
    /// Whether one direct child matches `t` (`child:`).
    fn child_matches(&self, _ts: &[Term]) -> bool {
        false
    }
    /// The `FILE_ATTRIBUTE_*` bits the scan kept (0 when it keeps none).
    fn attributes(&self) -> u32 {
        0
    }
}

/// One alternative of a word: a pattern, or a group `<...>` whose words must all match.
#[derive(Debug, Clone)]
pub(crate) enum Alt {
    Pat(Pat),
    Group(Vec<Term>),
}

impl Alt {
    fn needs(&self) -> u8 {
        match self {
            Alt::Pat(p) => p.needs(),
            Alt::Group(ts) => ts.iter().fold(0, |a, t| a | t.needs),
        }
    }

    #[inline]
    fn matches<I: Item>(&self, it: &I) -> bool {
        match self {
            Alt::Pat(p) => p.matches(it),
            Alt::Group(ts) => ts.iter().all(|t| t.matches(it)),
        }
    }

    /// A date, or a group of nothing but dated words (`!<dm:2026>` skips an item with no time
    /// as `!dm:2026` does).
    fn is_dated(&self) -> bool {
        match self {
            Alt::Pat(p) => matches!(p, Pat::Date(..)),
            Alt::Group(ts) => !ts.is_empty() && ts.iter().all(|t| t.dated),
        }
    }

    /// The plain word pattern of this alternative, if it is one.
    fn plain(&self) -> Option<&Pat> {
        match self {
            Alt::Pat(p) => Some(p),
            Alt::Group(_) => None,
        }
    }

    /// Whether this alternative walks a folder's children (`child:`, `childcount:`), which is
    /// work worth doing after every cheap word has passed.
    fn child_walk(&self) -> bool {
        match self {
            Alt::Pat(p) => matches!(p, Pat::Child(_) | Pat::ChildCount(..) | Pat::ChildFiles(..)),
            Alt::Group(ts) => ts.iter().any(Term::child_walk),
        }
    }
}

/// A word: any of its alternatives, or none of them with `!`.
#[derive(Debug, Clone)]
pub(crate) struct Term {
    not: bool,
    alts: Vec<Alt>,
    /// `NEED_*` bits of all the alternatives.
    pub(crate) needs: u8,
    /// Every alternative is a date: an item without a modified time never matches, `!` or not.
    dated: bool,
}

impl Term {
    #[inline]
    pub(crate) fn matches<I: Item>(&self, it: &I) -> bool {
        if self.dated && it.modified().is_none() {
            return false;
        }
        let hit = self.alts.iter().any(|p| p.matches(it));
        hit != self.not
    }

    pub(crate) fn on_path(&self) -> bool {
        self.needs & (NEED_PATH | NEED_CASE_PATH) != 0
    }

    /// Whether the word looks at a folder's children.
    fn child_walk(&self) -> bool {
        self.alts.iter().any(Alt::child_walk)
    }

    /// The order [`Query::split_terms`] checks words in: sizes, dates, depths and kinds (0)
    /// before names (1), and both before a word that walks children (2).
    fn cost(&self) -> u8 {
        if self.child_walk() {
            2
        } else if self.needs != 0 {
            1
        } else {
            0
        }
    }
}

#[inline]
pub(crate) fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > hay.len() {
        return false;
    }
    let first = needle[0];
    let last = hay.len() - needle.len();
    let mut i = 0;
    while i <= last {
        match hay[i..=last].iter().position(|&b| b == first) {
            None => return false,
            Some(k) => {
                i += k;
                if &hay[i..i + needle.len()] == needle {
                    return true;
                }
                i += 1;
            }
        }
    }
    false
}

/// Seconds since 1970, rounded down (before 1970: negative).
/// Seconds since 1970 (before it, negative): what a `dm:` or `da:` value is compared with.
pub fn unix_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs().min(i64::MAX as u64) as i64,
        Err(e) => {
            let d = e.duration();
            -(d.as_secs().min(i64::MAX as u64 - 1) as i64) - i64::from(d.subsec_nanos() > 0)
        }
    }
}

/// A parsed search.
#[derive(Debug, Clone, Default)]
pub struct Query {
    terms: Vec<Term>,
    pub kind: Kind,
    /// The smallest size, in bytes, included (from `>` and `>=` words).
    pub min: Option<u64>,
    /// The largest size, in bytes, included (from `<` and `<=` words). Larger than `max` when
    /// nothing can match (`<0`).
    pub max: Option<u64>,
    /// `dupe:`: only matches whose name another match has too (any case).
    pub dupe: bool,
    /// `sizedupe:`: only files whose size another file match has too.
    pub size_dupe: bool,
    errors: Vec<String>,
}

/// What a `dupe:` or `sizedupe:` search groups its matches by, so copies can be listed next to
/// each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    /// The name, any case.
    Name,
    /// Bytes (files only).
    Size,
}

/// One match's group key in a `dupe:`/`sizedupe:` search: what it shares with its copies.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GroupKey {
    Name(Vec<u8>),
    Size(u64),
}

impl Query {
    /// How the query's matches group for [`crate::search::group_key`] (`None`: no `dupe:` or
    /// `sizedupe:`).
    pub fn group_by(&self) -> Option<GroupBy> {
        if self.dupe {
            Some(GroupBy::Name)
        } else if self.size_dupe {
            Some(GroupBy::Size)
        } else {
            None
        }
    }
}

/// Keep the ones among `len` whose key another one shares: `dupe:` and `sizedupe:` leave only
/// the copies. An item whose key is `None` (a file, for `sizedupe:`) never matches.
pub(crate) fn keep_shared<K: std::hash::Hash + Eq + Clone>(len: usize, key: impl Fn(usize) -> Option<K>) -> Vec<usize> {
    let mut count: std::collections::HashMap<K, u32> = std::collections::HashMap::new();
    let mut keys: Vec<Option<K>> = Vec::with_capacity(len);
    for i in 0..len {
        let k = key(i);
        if let Some(k) = &k {
            *count.entry(k.clone()).or_insert(0) += 1;
        }
        keys.push(k);
    }
    (0..len).filter(|&i| keys[i].as_ref().is_some_and(|k| count[k] > 1)).collect()
}

/// The type macros: `audio:` is `ext:mp3;wav;...`, the same list the Type filter offers.
fn macro_family(w: &str) -> Option<u8> {
    Some(match w.to_ascii_lowercase().as_str() {
        "audio:" => crate::kinds::AUDIO,
        "video:" => crate::kinds::VIDEO,
        "pic:" => crate::kinds::IMAGES,
        "doc:" => crate::kinds::DOCUMENTS,
        "exe:" => crate::kinds::PROGRAMS,
        "zip:" => crate::kinds::ARCHIVES,
        _ => return None,
    })
}

/// `1gb`, `10 kb`, `1.5m`, `500` (bytes).
pub(crate) fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let v: f64 = num.parse().ok()?;
    let mul = match unit.trim() {
        "" | "b" => 1u64,
        "k" | "kb" | "kib" => 1 << 10,
        "m" | "mb" | "mib" => 1 << 20,
        "g" | "gb" | "gib" => 1 << 30,
        "t" | "tb" | "tib" => 1 << 40,
        _ => return None,
    };
    let bytes = v * mul as f64;
    bytes.is_finite().then_some(bytes as u64)
}

/// `>v`, `>=v`, `<v`, `<=v` as an inclusive range; `None` for anything else. `<0` is
/// `(1, 0)`: nothing.
fn compare<T: Copy>(s: &str, parse: impl Fn(&str) -> Option<T>, step: impl Fn(T, i8) -> Option<T>, lo: T, hi: T) -> Option<(T, T)> {
    if let Some(r) = s.strip_prefix(">=") {
        Some((parse(r)?, hi))
    } else if let Some(r) = s.strip_prefix('>') {
        // nothing is bigger than the largest value: an empty range
        Some(match step(parse(r)?, 1) {
            Some(v) => (v, hi),
            None => (hi, lo),
        })
    } else if let Some(r) = s.strip_prefix("<=") {
        Some((lo, parse(r)?))
    } else if let Some(r) = s.strip_prefix('<') {
        Some(match step(parse(r)?, -1) {
            Some(v) => (lo, v),
            None => (hi, lo),
        })
    } else {
        None
    }
}

fn step_u64(v: u64, d: i8) -> Option<u64> {
    if d > 0 {
        v.checked_add(1)
    } else {
        v.checked_sub(1)
    }
}

/// A bare `>1gb`: `None` when it is not one (then it is text).
fn size_compare(s: &str) -> Option<(u64, u64)> {
    compare(s, parse_size, step_u64, 0, u64::MAX)
}

/// `size:` values: a comparison, `a..b`, `empty`, or one size exactly.
fn size_value(s: &str) -> Option<(u64, u64)> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("empty") {
        return Some((0, 0));
    }
    if let Some(r) = size_compare(s) {
        return Some(r);
    }
    if let Some((a, b)) = s.split_once("..") {
        let a = if a.trim().is_empty() { 0 } else { parse_size(a)? };
        let b = if b.trim().is_empty() { u64::MAX } else { parse_size(b)? };
        return Some((a, b));
    }
    let v = parse_size(s)?;
    Some((v, v))
}

/// `depth:` values: a comparison, `a..b` or one number.
fn depth_value(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    let num = |t: &str| t.trim().parse::<u32>().ok();
    let step = |v: u32, d: i8| if d > 0 { v.checked_add(1) } else { v.checked_sub(1) };
    if let Some(r) = compare(s, num, step, 0, u32::MAX) {
        return Some(r);
    }
    if let Some((a, b)) = s.split_once("..") {
        let a = if a.trim().is_empty() { 0 } else { num(a)? };
        let b = if b.trim().is_empty() { u32::MAX } else { num(b)? };
        return Some((a, b));
    }
    let v = num(s)?;
    Some((v, v))
}

// -------------------------------------------------------------------- dates (local time)

/// Days since 1970-01-01 of a date in the proleptic Gregorian calendar.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_in_month(y: i64, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    (days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)) as u32
}

/// A day as days since 1970, so that "the day after" and "a month before" are plain sums.
type Day = i64;

/// Local midnight at the start of `day`, as seconds since 1970 (UTC), with the daylight saving
/// time of that date.
fn local_midnight(day: Day) -> Option<i64> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{SystemTimeToFileTime, TzSpecificLocalTimeToSystemTime};
    let (y, m, d) = civil_from_days(day);
    if !(1601..=30000).contains(&y) {
        return None;
    }
    let local = SYSTEMTIME { wYear: y as u16, wMonth: m as u16, wDayOfWeek: 0, wDay: d as u16, wHour: 0, wMinute: 0, wSecond: 0, wMilliseconds: 0 };
    // SAFETY: valid in/out structs; a null time zone means the current one.
    unsafe {
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        if TzSpecificLocalTimeToSystemTime(std::ptr::null(), &local, &mut utc) == 0 {
            return None;
        }
        let mut ft = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        if SystemTimeToFileTime(&utc, &mut ft) == 0 {
            return None;
        }
        let ticks = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
        Some((ticks / 10_000_000) as i64 - 11_644_473_600)
    }
}

/// The local date of a moment.
fn local_day(secs: i64) -> Option<Day> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let ticks = u64::try_from(secs.checked_add(11_644_473_600)?).ok()?.checked_mul(10_000_000)?;
    let f = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    // SAFETY: valid in/out structs.
    unsafe {
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        let mut local: SYSTEMTIME = std::mem::zeroed();
        if FileTimeToSystemTime(&f, &mut utc) == 0 || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0 {
            return None;
        }
        Some(days_from_civil(i64::from(local.wYear), u32::from(local.wMonth), u32::from(local.wDay)))
    }
}

/// The same day `months` months away, the day clamped to the month's length.
fn add_months(day: Day, months: i64) -> Day {
    let (y, m, d) = civil_from_days(day);
    let k = y * 12 + i64::from(m) - 1 + months;
    let (y, m) = (k.div_euclid(12), k.rem_euclid(12) as u32 + 1);
    days_from_civil(y, m, d.min(days_in_month(y, m)))
}

/// A named or written period as `[first day, day after the last)`.
fn period(s: &str, today: Option<Day>) -> Option<(Day, Day)> {
    let s = s.trim().to_ascii_lowercase();
    let t = || today;
    match s.as_str() {
        "today" => return Some((t()?, t()? + 1)),
        "yesterday" => return Some((t()? - 1, t()?)),
        "lastweek" => return Some((t()? - 7, t()? + 1)),
        "lastmonth" => return Some((add_months(t()?, -1), t()? + 1)),
        "thismonth" => {
            let (y, m, _) = civil_from_days(t()?);
            return Some((days_from_civil(y, m, 1), add_months(days_from_civil(y, m, 1), 1)));
        }
        "thisyear" => {
            let (y, _, _) = civil_from_days(t()?);
            return Some((days_from_civil(y, 1, 1), days_from_civil(y + 1, 1, 1)));
        }
        _ => {}
    }
    let parts: Vec<&str> = s.split(['-', '/', '.']).collect();
    let num = |p: &str, digits: std::ops::RangeInclusive<usize>| -> Option<u32> {
        (digits.contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse().ok()).flatten()
    };
    let y = i64::from(num(parts[0], 4..=4)?);
    if !(1601..=9999).contains(&y) {
        return None;
    }
    match parts[1..] {
        [] => Some((days_from_civil(y, 1, 1), days_from_civil(y + 1, 1, 1))),
        [m] => {
            let m = num(m, 1..=2)?;
            if !(1..=12).contains(&m) {
                return None;
            }
            let first = days_from_civil(y, m, 1);
            Some((first, add_months(first, 1)))
        }
        [m, d] => {
            let (m, d) = (num(m, 1..=2)?, num(d, 1..=2)?);
            if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
                return None;
            }
            let day = days_from_civil(y, m, d);
            Some((day, day + 1))
        }
        _ => None,
    }
}

/// A `dm:` value as seconds `[from, to)`.
fn date_value(s: &str, today: Option<Day>) -> Option<(i64, i64)> {
    let s = s.trim();
    let (lo, hi) = (i64::MIN, i64::MAX);
    let start = |p: &str| local_midnight(period(p, today)?.0);
    let end = |p: &str| local_midnight(period(p, today)?.1);
    if let Some(r) = s.strip_prefix(">=") {
        Some((start(r)?, hi))
    } else if let Some(r) = s.strip_prefix('>') {
        Some((end(r)?, hi))
    } else if let Some(r) = s.strip_prefix("<=") {
        Some((lo, end(r)?))
    } else if let Some(r) = s.strip_prefix('<') {
        Some((lo, start(r)?))
    } else if let Some((a, b)) = s.split_once("..") {
        let a = if a.trim().is_empty() { lo } else { start(a)? };
        let b = if b.trim().is_empty() { hi } else { end(b)? };
        Some((a, b))
    } else {
        let s = s.strip_prefix('=').unwrap_or(s);
        Some((start(s)?, end(s)?))
    }
}

// -------------------------------------------------------------------- parsing

/// Which time a `dc:` or `da:` word asks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadKind {
    /// `dc:`: when it was created.
    Created,
    /// `da:`: when it was last opened.
    Accessed,
}

/// The `dc:` (created) and `da:` (last opened) words of a query, as seconds since 1970
/// `[from, to)`. A scan doesn't hold either time, so the caller reads them for each match (see
/// [`ReadDates::is_empty`]): only a local fixed drive is read, and there is a limit on how many
/// matches one answer reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadDates {
    pub created: Option<(i64, i64)>,
    pub accessed: Option<(i64, i64)>,
}

impl ReadDates {
    /// Nothing to read: the query asks for neither time.
    pub fn is_empty(&self) -> bool {
        self.created.is_none() && self.accessed.is_none()
    }
}

/// A `dc:`/`da:` word (already lower-case), the time it asks about and what follows it.
fn read_keyword(low: &str) -> Option<(ReadKind, &str)> {
    for (kw, kind) in [("datecreated:", ReadKind::Created), ("dateaccessed:", ReadKind::Accessed), ("dc:", ReadKind::Created), ("da:", ReadKind::Accessed)] {
        if let Some(v) = low.strip_prefix(kw) {
            return Some((kind, v));
        }
    }
    None
}

/// Whether a `dc:`/`da:` word starts at `i` of `c` (a word's start).
fn read_word_at(c: &[Ch], i: usize) -> bool {
    let low: String = c[i..].iter().filter(|&&(_, q)| !q).take(14).map(|&(ch, _)| ch.to_ascii_lowercase()).collect();
    let low = low.strip_prefix('!').unwrap_or(&low);
    read_keyword(low).is_some()
}

/// Whether a group or an alternative holds a `dc:`/`da:` word of its own (`<a da:2024>`), which
/// can't be taken out of the text without changing what the group means.
fn read_inside(c: &[Ch]) -> bool {
    let mut start = true;
    for (i, &(ch, quoted)) in c.iter().enumerate() {
        if quoted {
            start = false;
        } else if ch.is_whitespace() || ch == '|' || ch == ';' || ch == '<' || ch == '>' {
            start = true;
        } else if ch == '!' && start {
        } else {
            if start && read_word_at(c, i) {
                return true;
            }
            start = false;
        }
    }
    false
}

/// The `dc:` and `da:` words of `q` taken out, as the times to read for each match: only the
/// kinds in `kinds` are taken (the others stay words to look for). Returns the query without
/// them, and `Err` saying what is wrong with a value (`da:2026-13`) or where the word is (not at
/// the top level, negated).
fn split_times(q: &str, now: SystemTime, kinds: &[ReadKind]) -> (String, Result<ReadDates, String>) {
    if too_long(q) {
        return (q.to_string(), Err(TOO_LONG.to_string()));
    }
    let today = || local_day(unix_secs(now));
    let mut kept: Vec<&str> = Vec::new();
    let mut out = ReadDates::default();
    let mut error: Option<String> = None;
    for t in token_spans(q) {
        let text = &q[t.start..t.end];
        let plain = t.chars.iter().all(|&(_, quoted)| !quoted);
        if !plain {
            kept.push(text);
            continue;
        }
        // inside a group or an alternative it can't be lifted out on its own
        let parts = marks(&t.chars).contains(&MARK_OPEN) || t.chars.iter().any(|&(c, quoted)| !quoted && (c == '|' || c == ';'));
        if parts {
            if read_inside(&t.chars) {
                error.get_or_insert_with(|| "dc: and da: go as words of their own, not inside <...> or a|b".to_string());
                continue;
            }
            kept.push(text);
            continue;
        }
        let low: String = t.chars.iter().map(|&(c, _)| c).collect::<String>().to_ascii_lowercase();
        let (negated, low) = match low.strip_prefix('!') {
            Some(r) => (true, r),
            None => (false, low.as_str()),
        };
        let kw = read_keyword(low).filter(|(k, _)| kinds.contains(k));
        // never negated: the time of a match that wasn't read can't mean "not in range"
        if negated {
            if let Some((k, _)) = kw {
                let kw = if k == ReadKind::Created { "dc:" } else { "da:" };
                error.get_or_insert_with(|| format!("{kw} can't be negated"));
            }
            kept.push(text);
            continue;
        }
        let Some((kind, v)) = kw else {
            kept.push(text);
            continue;
        };
        // a keyword with nothing after it is being typed: ignored, as the others are
        if v.is_empty() {
            continue;
        }
        match date_value(v, today()) {
            Some((a, b)) => {
                let slot = if kind == ReadKind::Created { &mut out.created } else { &mut out.accessed };
                *slot = Some(slot.map_or((a, b), |(a0, b0)| (a.max(a0), b.min(b0))));
            }
            None => {
                let kw = if kind == ReadKind::Created { "dc" } else { "da" };
                error.get_or_insert_with(|| format!("{kw}:{v} isn't a date Sprawl understands (try {kw}:2026-01-15, {kw}:2026-11 or {kw}:lastweek)"));
            }
        }
    }
    (kept.join(" "), error.map_or(Ok(out), Err))
}

/// The `dc:` and `da:` words of a query taken out, as the times to read for each match (see
/// [`ReadDates`]). Returns the query without them, and `Err` saying what is wrong with a value
/// (`da:2026-13`).
pub fn split_read_dates(q: &str, now: SystemTime) -> (String, Result<ReadDates, String>) {
    split_times(q, now, &[ReadKind::Created, ReadKind::Accessed])
}

/// `da:` (date accessed) words taken out of an AI search, as seconds `[from, to)`: a scan
/// doesn't hold access times, so the caller reads them for the matches it keeps. Returns the
/// query without those words, and `Err` saying what is wrong with a value (`da:2026-13`).
pub fn split_accessed(q: &str, now: SystemTime) -> (String, Result<Option<(i64, i64)>, String>) {
    let (text, r) = split_times(q, now, &[ReadKind::Accessed]);
    (text, r.map(|d| d.accessed))
}

/// A character of the query and whether it was inside quotes.
type Ch = (char, bool);

#[cfg(test)]
fn tokens(q: &str) -> Vec<Vec<Ch>> {
    token_spans(q).into_iter().map(|t| t.chars).collect()
}

/// A character of a query outside its quotes, with whether it was inside them.
fn chars_of(q: &str) -> Vec<Ch> {
    let mut out = Vec::with_capacity(q.len());
    let mut quoted = false;
    for c in q.chars() {
        if c == '"' {
            quoted = !quoted;
            continue;
        }
        out.push((c, quoted));
    }
    out
}

/// A group's `<` (1) or `>` (2) in a word; 0 is an ordinary character.
const MARK_OPEN: u8 = 1;
const MARK_CLOSE: u8 = 2;

/// Which `<` and `>` of `c` are a group's. A `<` opens one where a word starts (after a space,
/// a `|`, a `;` or a `!`), when a `>` that ends a word closes it: `<-5` has none, so it stays
/// text. A bare size keeps its `<`: `<10mb >1mb` is two sizes being typed, not a group.
///
/// The pairs are matched in one pass (a stack of open `<`), so nothing here costs more than one
/// scan of the query however many `<` it holds: 30 unclosed ones or a pasted address list used
/// to take seconds to minutes, and it runs on every text change.
fn marks(c: &[Ch]) -> Vec<u8> {
    let mut m = vec![0u8; c.len()];
    let mut open: Vec<usize> = Vec::new();
    let mut start = true;
    for (i, &(ch, quoted)) in c.iter().enumerate() {
        if quoted {
            start = false;
        } else if ch == '|' || ch == ';' || ch.is_whitespace() {
            start = true;
        } else if ch == '!' && start {
            // `!<a b>` negates the group: the `<` still starts a word
        } else if ch == '<' && start {
            if bare_size_word(c, i) {
                // not a group: an ordinary character, as before
                start = false;
            } else {
                // a group's `<` still leaves a word start behind it, so groups nest at once
                open.push(i);
            }
        } else if ch == '>' && after_word(c, i + 1, c.len()) {
            if let Some(o) = open.pop() {
                m[o] = MARK_OPEN;
                m[i] = MARK_CLOSE;
            }
            start = true;
        } else {
            start = false;
        }
    }
    m
}

/// Whether the word starting at `i` (a `<` up to the next space, `|` or `;`) is a bare size
/// comparison, `<10mb`, `<=5kb` or `<0`: such a `<` is never a group's.
fn bare_size_word(c: &[Ch], i: usize) -> bool {
    // a size starts with a digit or a dot (after `<` or `<=`): most `<` are not worth a scan
    let mut at = i + 1;
    if matches!(c.get(at), Some(&('=', false))) {
        at += 1;
    }
    if !matches!(c.get(at), Some(&(d, false)) if d.is_ascii_digit() || d == '.') {
        return false;
    }
    let mut s = String::new();
    for &(ch, quoted) in &c[i..] {
        if !quoted && (ch.is_whitespace() || ch == '|' || ch == ';') {
            break;
        }
        s.push(ch);
        if s.len() > 64 {
            return false;
        }
    }
    // the last word of a group ends in the group's `>`: `<a <1gb>`
    size_compare(s.trim_end_matches('>')).is_some()
}

/// Whether position `i` is at a word's start (the end, a space, `|`, `;` or a group's `>`).
fn after_word(c: &[Ch], i: usize, b: usize) -> bool {
    match c.get(i) {
        Some(&(ch, false)) => i < b && (ch.is_whitespace() || ch == '|' || ch == ';' || ch == '>'),
        Some(_) => false,
        None => true,
    }
}

/// A word of the query with where it is: `start..end` in the text (quotes included), and the
/// byte offset in the text of each of its characters.
struct Token {
    start: usize,
    end: usize,
    chars: Vec<Ch>,
    at: Vec<usize>,
}

fn token_spans(q: &str) -> Vec<Token> {
    let chars = chars_of(q);
    let m = marks(&chars);
    let mut out = Vec::new();
    let mut cur = Token { start: 0, end: 0, chars: Vec::new(), at: Vec::new() };
    let mut quoted = false;
    // where the word began (its first quote counts), if one has
    let mut begun: Option<usize> = None;
    // a group's spaces are part of its word: `a <b c> d` is three words
    let mut depth = 0usize;
    let mut k = 0usize;
    for (i, c) in q.char_indices() {
        if c == '"' {
            quoted = !quoted;
            begun.get_or_insert(i);
            continue;
        }
        let mark = m[k];
        k += 1;
        match mark {
            MARK_OPEN => depth += 1,
            MARK_CLOSE => depth = depth.saturating_sub(1),
            _ => {}
        }
        if c.is_whitespace() && !quoted && depth == 0 {
            if let Some(s) = begun.take() {
                if !cur.chars.is_empty() {
                    cur.start = s;
                    cur.end = i;
                    out.push(std::mem::replace(&mut cur, Token { start: 0, end: 0, chars: Vec::new(), at: Vec::new() }));
                }
            }
        } else {
            begun.get_or_insert(i);
            cur.chars.push((c, quoted));
            cur.at.push(i);
        }
    }
    if let Some(s) = begun {
        if !cur.chars.is_empty() {
            cur.start = s;
            cur.end = q.len();
            out.push(cur);
        }
    }
    out
}

/// Which keyword a word of the box uses, for the app's filter chips ([`Query::words`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordKey {
    /// A name or path word (with its modifiers in [`Word`]).
    Text,
    Ext,
    Date,
    /// `dc:`/`da:`: a time read for each match, not a name word.
    ReadDate,
    /// `size:`.
    Size,
    /// A bare `>1gb`, `<=10kb`.
    Compare,
    Depth,
    Compressed,
    Regex,
    /// `parent:`.
    Parent,
    Child,
    ChildCount,
    ChildFiles,
    /// `attrib:`.
    Attrib,
    /// `len:`.
    Len,
    /// `dupe:`.
    Dupe,
    /// `sizedupe:`.
    SizeDupe,
    /// Only `folder:` or `file:`, nothing after it.
    None,
}

/// A word of the box as [`Query::parse`] reads it, and where it is in the text, so the app can
/// light a chip for it or take it out. Byte offsets are into the text given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    /// The whole word, quotes included.
    pub span: std::ops::Range<usize>,
    /// `folder:` or `file:` in front of it.
    pub kind: Kind,
    /// Where that `folder:`/`file:` ends (`span.start` without one).
    pub kind_end: usize,
    /// `!` after it.
    pub not: bool,
    /// Where the `!` ends (`kind_end` without one): the modifiers and keyword start here.
    pub prefix_end: usize,
    /// The first alternative's keyword and modifiers.
    pub key: WordKey,
    pub case: bool,
    pub exact: bool,
    pub path: bool,
    /// Where what follows the modifiers and keyword starts (an opening quote included).
    pub value_at: usize,
    /// That value as the query reads it (without quotes).
    pub value: String,
}

impl Query {
    /// The words of `q` and the keyword each one uses, for showing them as filters. Reads the
    /// text the way [`Query::parse`] does; whether a value can be understood is not checked
    /// (see [`Query::errors`]).
    pub fn words(q: &str) -> Vec<Word> {
        if too_long(q) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for tok in token_spans(q) {
            let n = tok.chars.len();
            // a word with a group `<a b>|c` is no single filter: the chips leave it alone
            if marks(&tok.chars).contains(&MARK_OPEN) {
                out.push(Word {
                    span: tok.start..tok.end,
                    kind: Kind::Any,
                    kind_end: tok.start,
                    not: false,
                    prefix_end: tok.start,
                    key: WordKey::None,
                    case: false,
                    exact: false,
                    path: false,
                    value_at: tok.start,
                    value: String::new(),
                });
                continue;
            }
            // the text offset just after the first `k` characters of the word (a quote that
            // follows them is not part of them)
            let after = |k: usize| if k == 0 { tok.start } else { tok.at[k - 1] + tok.chars[k - 1].0.len_utf8() };
            let mut t: &[Ch] = &tok.chars;
            let mut kind = Kind::Any;
            for (kw, k) in [("folder:", Kind::Folders), ("file:", Kind::Files)] {
                if let Some(r) = strip_kw(t, kw) {
                    kind = k;
                    t = r;
                    break;
                }
            }
            let kind_end = after(n - t.len());
            let mut w = Word {
                span: tok.start..tok.end,
                kind,
                kind_end,
                not: false,
                prefix_end: kind_end,
                key: WordKey::None,
                case: false,
                exact: false,
                path: false,
                value_at: kind_end,
                value: String::new(),
            };
            if t.is_empty() {
                out.push(w);
                continue;
            }
            if matches!(t[0], ('>' | '<', false)) && t.iter().all(|c| !c.1) && size_compare(&text(t)).is_some() {
                w.key = WordKey::Compare;
                w.value = text(t);
                out.push(w);
                continue;
            }
            if t[0] == ('!', false) {
                w.not = true;
                t = &t[1..];
            }
            w.prefix_end = after(n - t.len());
            let (p, rest) = prefix(t);
            w.key = match p.key {
                Key::Text => WordKey::Text,
                Key::Ext => WordKey::Ext,
                Key::Date => WordKey::Date,
                Key::Size => WordKey::Size,
                Key::Depth => WordKey::Depth,
                Key::Compressed => WordKey::Compressed,
                Key::Regex => WordKey::Regex,
                Key::Parent => WordKey::Parent,
                Key::Child => WordKey::Child,
                Key::ChildCount => WordKey::ChildCount,
                Key::ChildFiles => WordKey::ChildFiles,
                Key::Attrib => WordKey::Attrib,
                Key::Len => WordKey::Len,
                Key::Dupe => WordKey::Dupe,
                Key::SizeDupe => WordKey::SizeDupe,
            };
            w.case = p.case;
            w.exact = p.exact;
            w.path = p.path;
            w.value_at = after(n - rest.len());
            w.value = text(rest);
            // whatever is inside quotes is plain text, so `"audio:"` is not the macro and a
            // `dc:` path inside quotes is not read
            let quoted = rest.iter().any(|&(_, q)| q);
            // `dc:`/`da:` are taken out and read, not searched as text: no modifier may land on
            // them (`exact:dc:2026` would stop being a date)
            if w.kind == Kind::Any
                && w.key == WordKey::Text
                && !w.case
                && !w.exact
                && !w.path
                && !quoted
                && read_keyword(&w.value.trim().to_ascii_lowercase()).is_some()
            {
                w.key = WordKey::ReadDate;
            }
            // a type macro is the Type filter's own list (`audio:` lights Type as audio)
            if w.key == WordKey::Text && !w.case && !w.exact && !w.path && !quoted {
                if let Some(f) = macro_family(w.value.trim()) {
                    w.key = WordKey::Ext;
                    w.value = crate::kinds::extensions(f).join(";");
                }
            }
            out.push(w);
        }
        out
    }
}

fn text(t: &[Ch]) -> String {
    t.iter().map(|c| c.0).collect()
}

/// `t` without the unquoted keyword `kw` (any case) in front.
fn strip_kw<'a>(t: &'a [Ch], kw: &str) -> Option<&'a [Ch]> {
    let n = kw.chars().count();
    if t.len() < n {
        return None;
    }
    let ok = t[..n].iter().zip(kw.chars()).all(|(&(c, q), k)| !q && c.eq_ignore_ascii_case(&k));
    ok.then(|| &t[n..])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Key {
    #[default]
    Text,
    Ext,
    Date,
    Size,
    Depth,
    Compressed,
    Regex,
    Parent,
    Child,
    ChildCount,
    ChildFiles,
    Attrib,
    Len,
    Dupe,
    SizeDupe,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Prefix {
    case: bool,
    exact: bool,
    path: bool,
    key: Key,
}

impl Prefix {
    fn is_some(&self) -> bool {
        *self != Prefix::default()
    }
}

/// The modifiers and keyword in front of `t`, and what follows them.
fn prefix(mut t: &[Ch]) -> (Prefix, &[Ch]) {
    let mut p = Prefix::default();
    'outer: loop {
        for (kw, set) in [("case:", 0), ("exact:", 1), ("wholename:", 1), ("path:", 2)] {
            if let Some(r) = strip_kw(t, kw) {
                match set {
                    0 => p.case = true,
                    1 => p.exact = true,
                    _ => p.path = true,
                }
                t = r;
                continue 'outer;
            }
        }
        for (kw, key) in [
            ("ext:", Key::Ext),
            ("dm:", Key::Date),
            ("datemodified:", Key::Date),
            ("size:", Key::Size),
            ("depth:", Key::Depth),
            ("compressed:", Key::Compressed),
            ("regex:", Key::Regex),
            ("parent:", Key::Parent),
            ("childfilecount:", Key::ChildFiles),
            ("childcount:", Key::ChildCount),
            ("child:", Key::Child),
            ("attrib:", Key::Attrib),
            ("len:", Key::Len),
            ("sizedupe:", Key::SizeDupe),
            ("dupe:", Key::Dupe),
        ] {
            if let Some(r) = strip_kw(t, kw) {
                p.key = key;
                t = r;
                break;
            }
        }
        return (p, t);
    }
}

/// Split on unquoted `|` and `;`.
fn split_alts(t: &[Ch]) -> Vec<&[Ch]> {
    t.split(|&(c, q)| !q && (c == '|' || c == ';')).collect()
}

// ------------------------------------------------------------ words and groups

/// The most nesting read: groups inside groups, and `child:` inside `child:`. Deeper is refused
/// with an error instead of recursing until the stack is gone.
const MAX_DEPTH: u32 = 16;

/// The longest query read at all: the search box and the AI take any text, and nothing past a
/// few thousand characters is a search anyone meant.
const MAX_QUERY_CHARS: usize = 4096;
/// What a query over [`MAX_QUERY_CHARS`] is answered with.
const TOO_LONG: &str = "the query is too long: keep it under 4,096 characters";

impl Query {
    /// Note that the query nests deeper than [`MAX_DEPTH`], once.
    fn too_deep(&mut self) {
        if !self.errors.iter().any(|e| e == TOO_DEEP) {
            self.errors.push(TOO_DEEP.to_string());
        }
    }

    /// Whether the query has a group (`<a b>|c`): what a filter box can't take.
    pub fn has_group(q: &str) -> bool {
        !too_long(q) && marks(&chars_of(q)).contains(&MARK_OPEN)
    }
}

/// What a query nested too deeply is answered with.
const TOO_DEEP: &str = "too many nested groups: keep them under 16 levels";

/// Whether `q` is longer than [`MAX_QUERY_CHARS`].
fn too_long(q: &str) -> bool {
    q.chars().count() > MAX_QUERY_CHARS
}

/// The terms of a word list: its words, all of which must match (a group's inside, or the whole
/// query). A group's `<>` keeps its spaces inside one word.
fn parse_words(c: &[Ch], top: bool, depth: u32, q: &mut Query, today: &mut Option<Option<Day>>, now: SystemTime) -> Vec<Term> {
    if depth > MAX_DEPTH {
        q.too_deep();
        return Vec::new();
    }
    let m = marks(c);
    let mut out = Vec::new();
    let mut nest = 0usize;
    let mut start = 0usize;
    for i in 0..c.len() {
        match m[i] {
            MARK_OPEN => nest += 1,
            MARK_CLOSE => nest = nest.saturating_sub(1),
            _ if nest == 0 && !c[i].1 && c[i].0.is_whitespace() => {
                if i > start {
                    out.extend(terms_of(&c[start..i], top, depth, q, today, now));
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < c.len() {
        out.extend(terms_of(&c[start..], top, depth, q, today, now));
    }
    out
}

/// The terms of one word: `!`, then its alternatives (`a|b`, `a;b`), any of which may be a
/// group `<...>`: `<a b>|c` is all of a and b, or c.
fn terms_of(t: &[Ch], top: bool, depth: u32, q: &mut Query, today: &mut Option<Option<Day>>, now: SystemTime) -> Vec<Term> {
    let m = marks(t);
    if !m.contains(&MARK_OPEN) {
        return plain_term(t, top, depth, q, today, now);
    }
    let mut i = 0usize;
    let not = t.first() == Some(&('!', false));
    if not {
        i = 1;
    }
    // the alternatives as typed: a group's inside, or a run of plain characters
    let mut raws: Vec<(Option<&[Ch]>, &[Ch])> = Vec::new();
    while i < t.len() {
        if m[i] == MARK_OPEN {
            let mut d = 0isize;
            let mut e = i;
            while e < t.len() {
                if m[e] == MARK_OPEN {
                    d += 1;
                } else if m[e] == MARK_CLOSE {
                    d -= 1;
                    if d == 0 {
                        break;
                    }
                }
                e += 1;
            }
            if e >= t.len() {
                break;
            }
            raws.push((Some(&t[i + 1..e]), &[]));
            i = e + 1;
        } else {
            let start = i;
            while i < t.len() && !(m[i] == 0 && !t[i].1 && (t[i].0 == '|' || t[i].0 == ';')) {
                i += 1;
            }
            raws.push((None, &t[start..i]));
        }
        if i < t.len() && m[i] == 0 && !t[i].1 && (t[i].0 == '|' || t[i].0 == ';') {
            i += 1;
            continue;
        }
        break;
    }
    if i < t.len() {
        // whatever follows a group's `>` in its word is never read: say so
        let rest: String = t[i..].iter().map(|&(c, _)| c).collect();
        q.errors.push(format!("{rest}: put a space, | or ; after a group's >"));
    }
    // a plain alternative without a keyword of its own takes the first plain one's (`ext:a|b`)
    let first = raws.iter().find(|(g, _)| g.is_none()).map(|(_, c)| prefix(c).0);
    let mut alts: Vec<Alt> = Vec::new();
    let mut plain_seen = false;
    for (group, chars) in raws {
        let Some(inner) = group else {
            let (own, rest) = prefix(chars);
            let (p, rest) = if own.is_some() || !plain_seen { (own, rest) } else { (first.unwrap_or_default(), chars) };
            plain_seen = true;
            // a `dupe:` only means something as a top-level word of its own
            match build(p, rest, chars, false, depth, q, today, now) {
                Ok(pats) => alts.extend(pats.into_iter().map(Alt::Pat)),
                Err(e) => q.errors.push(e),
            }
            continue;
        };
        let ts = parse_words(inner, false, depth + 1, q, today, now);
        if !ts.is_empty() {
            alts.push(Alt::Group(ts));
        }
    }
    if alts.is_empty() {
        return Vec::new();
    }
    let needs = alts.iter().fold(0, |a, p| a | p.needs());
    let dated = alts.iter().all(Alt::is_dated);
    vec![Term { not, alts, needs, dated }]
}

/// The terms of a word with no group in it: `folder:`, a bare size, `!` and `a|b`. At the top
/// level of the query a kind and a bare size belong to the whole query; inside a group they are
/// words of it.
fn plain_term(t: &[Ch], top: bool, depth: u32, q: &mut Query, today: &mut Option<Option<Day>>, now: SystemTime) -> Vec<Term> {
    let mut t: &[Ch] = t;
    let mut kind = Kind::Any;
    for (kw, k) in [("folder:", Kind::Folders), ("file:", Kind::Files)] {
        if let Some(r) = strip_kw(t, kw) {
            kind = k;
            t = r;
            break;
        }
    }
    let mut out = Vec::new();
    if kind != Kind::Any {
        if top {
            q.kind = kind;
        } else {
            out.push(Term { not: false, alts: vec![Alt::Pat(Pat::Kind(kind))], needs: 0, dated: false });
        }
    }
    if t.is_empty() {
        return out;
    }
    // a bare size: `>1gb` (`>abc` is text)
    if matches!(t[0], ('>' | '<', false)) && t.iter().all(|c| !c.1) {
        if let Some((a, b)) = size_compare(&text(t)) {
            if top {
                q.min = Some(q.min.map_or(a, |m| m.max(a)));
                q.max = Some(q.max.map_or(b, |m| m.min(b)));
            } else {
                out.push(Term { not: false, alts: vec![Alt::Pat(Pat::Size(a, b))], needs: 0, dated: false });
            }
            return out;
        }
    }
    let not = t[0] == ('!', false);
    if not {
        t = &t[1..];
    }
    if t.is_empty() {
        return out;
    }
    let (first, _) = prefix(t);
    // a regex keeps its `|`
    let alts_raw = if first.key == Key::Regex { vec![t] } else { split_alts(t) };
    // `dupe:` only as a word of its own at the top: not negated, not one of several alternatives
    let own_word = top && !not && alts_raw.len() == 1;
    let mut alts = Vec::new();
    for (k, alt) in alts_raw.into_iter().enumerate() {
        let (own, rest) = prefix(alt);
        let p = if own.is_some() || k == 0 { own } else { first };
        let rest = if own.is_some() || k == 0 { rest } else { alt };
        match build(p, rest, alt, own_word, depth, q, today, now) {
            Ok(pats) => alts.extend(pats.into_iter().map(Alt::Pat)),
            Err(e) => q.errors.push(e),
        }
    }
    if !alts.is_empty() {
        let needs = alts.iter().fold(0, |a, p| a | p.needs());
        let dated = alts.iter().all(Alt::is_dated);
        out.push(Term { not, alts, needs, dated });
    }
    out
}

impl Query {
    /// Read the search box. "Now" is the clock's (for `dm:today` and the like).
    pub fn parse(q: &str) -> Query {
        Query::parse_at(q, SystemTime::now())
    }

    /// [`Query::parse`] with `now` given, for words like `dm:today`.
    pub fn parse_at(q: &str, now: SystemTime) -> Query {
        let mut query = Query::default();
        if too_long(q) {
            query.errors.push(TOO_LONG.to_string());
            return query;
        }
        let mut today: Option<Option<Day>> = None;
        let terms = parse_words(&chars_of(q), true, 0, &mut query, &mut today, now);
        query.terms = terms;
        query
    }
    
    /// Nothing to look for: no words, no size, no kind, no mistakes.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.min.is_none()
            && self.max.is_none()
            && self.kind == Kind::Any
            && !self.dupe
            && !self.size_dupe
            && self.errors.is_empty()
    }

    /// What could not be understood, one message per word (`dm:2026-13: not a date ...`).
    /// While there are any, the query finds nothing.
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// Some words need the full path (slower: the paths are built as the tree is walked).
    pub fn uses_paths(&self) -> bool {
        self.terms.iter().any(Term::on_path)
    }

    /// The words to check before any path is built, cheapest first (sizes, dates and depths,
    /// then names, then the words that walk a folder's children), and the words that need the
    /// path.
    pub(crate) fn split_terms(&self) -> (Vec<&Term>, Vec<&Term>) {
        let (mut early, path): (Vec<&Term>, Vec<&Term>) = self.terms.iter().partition(|t| !t.on_path());
        early.sort_by_key(|t| t.cost());
        (early, path)
    }

    /// Does an item's size pass the `>`/`<` words?
    #[inline]
    pub(crate) fn size_ok(&self, size: u64) -> bool {
        !(self.min.is_some_and(|m| size < m) || self.max.is_some_and(|m| size > m))
    }

    /// Where the first plain word is in `name` (byte range), to show it in bold.
    pub fn highlight(&self, name: &str) -> Option<(usize, usize)> {
        // the lowercased name's byte offsets are the name's only when every character keeps its
        // width (a whole length that matches can still hide one that grew and one that shrank)
        let same = name.chars().all(|c| {
            let mut l = c.to_lowercase();
            l.next().is_some_and(|x| x.len_utf8() == c.len_utf8()) && l.next().is_none()
        });
        if !same {
            return None;
        }
        let lower = name.to_lowercase();
        if lower.len() != name.len() {
            return None;
        }
        self.terms.iter().filter(|t| !t.not).flat_map(|t| &t.alts).filter_map(Alt::plain).find_map(|p| match p {
            Pat::Sub(s) => {
                let s = std::str::from_utf8(s).ok()?;
                lower.find(s).map(|i| (i, i + s.len()))
            }
            Pat::CaseSub(s) => {
                let s = std::str::from_utf8(s).ok()?;
                name.find(s).map(|i| (i, i + s.len()))
            }
            _ => None,
        })
    }
}

/// One alternative: `p` says how to read `rest`; `whole` is the alternative as typed, for
/// messages. `own_word` says it stands alone at the top level of the query (what `dupe:` needs);
/// `depth` is how many groups and `child:` words it is inside.
fn build(p: Prefix, rest: &[Ch], whole: &[Ch], own_word: bool, depth: u32, q: &mut Query, today: &mut Option<Option<Day>>, now: SystemTime) -> Result<Vec<Pat>, String> {
    let value = text(rest);
    if value.trim().is_empty() && !matches!(p.key, Key::Compressed | Key::Dupe | Key::SizeDupe) {
        return Ok(Vec::new());
    }
    let shown = text(whole);
    match p.key {
        Key::Compressed => match value.to_ascii_lowercase().as_str() {
            "" | "yes" | "true" | "1" => Ok(vec![Pat::Compressed(true)]),
            "no" | "false" | "0" => Ok(vec![Pat::Compressed(false)]),
            _ => Err(format!("{shown}: use compressed:, compressed:yes or compressed:no")),
        },
        Key::Ext => {
            let v = value.trim();
            // a macro keeps its keyword as an alternative: `ext:jpg|audio:` is both lists
            let quoted = rest.iter().any(|&(_, q)| q);
            if !quoted && !p.case && !p.exact && !p.path {
                if let Some(f) = macro_family(v) {
                    return Ok(crate::kinds::extensions(f).iter().map(|e| Pat::Ext(e.as_bytes().to_vec())).collect());
                }
            }
            let v = v.strip_prefix('.').unwrap_or(v).to_lowercase();
            if v.is_empty() {
                return Ok(Vec::new());
            }
            Ok(vec![if v.contains(['*', '?']) { Pat::ExtGlob(v.chars().collect()) } else { Pat::Ext(v.into_bytes()) }])
        }
        Key::Size => match size_value(&value) {
            Some((a, b)) => Ok(vec![Pat::Size(a, b)]),
            None => Err(format!("{shown}: not a size (try size:>1gb, size:100mb..2gb or size:empty)")),
        },
        Key::Depth => match depth_value(&value) {
            Some((a, b)) => Ok(vec![Pat::Depth(a, b)]),
            None => Err(format!("{shown}: not a depth (try depth:<=3, depth:2 or depth:1..4)")),
        },
        Key::Parent => {
            let v = value.trim().replace('/', "\\");
            let v = v.trim_end_matches('\\');
            if v.is_empty() {
                return Ok(Vec::new());
            }
            let v = if p.case { v.to_string() } else { v.to_lowercase() };
            Ok(vec![Pat::Parent { path: v.into_bytes(), case: p.case }])
        }
        Key::Child => {
            let ts = parse_words(rest, false, depth + 1, q, today, now);
            // a case word on the path is a path word too: the children's own paths are not built
            if ts.iter().any(Term::on_path) {
                return Err(format!("{shown}: child: looks at the names of the items directly inside, so a word with \\ or path: can't go in it"));
            }
            if ts.is_empty() {
                return Ok(Vec::new());
            }
            Ok(vec![Pat::Child(ts)])
        }
        Key::ChildCount => match depth_value(&value) {
            Some((a, b)) => Ok(vec![Pat::ChildCount(a, b)]),
            None => Err(format!("{shown}: not a count (try childcount:>10000, childcount:0 or childcount:10..100)")),
        },
        Key::ChildFiles => match depth_value(&value) {
            Some((a, b)) => Ok(vec![Pat::ChildFiles(a, b)]),
            None => Err(format!("{shown}: not a count (try childfilecount:>10000, childfilecount:0 or childfilecount:10..100)")),
        },
        Key::Attrib => {
            let mut mask = 0u32;
            let mut compressed = false;
            for c in value.trim().to_ascii_lowercase().chars() {
                match c {
                    'h' => mask |= crate::attr::HIDDEN,
                    's' => mask |= crate::attr::SYSTEM,
                    'r' => mask |= crate::attr::READONLY,
                    'l' => mask |= crate::attr::REPARSE_POINT,
                    'c' => compressed = true,
                    _ => {
                        return Err(format!("{shown}: not an attribute (use attrib:h, s, r, l or c, as in attrib:hs)"));
                    }
                }
            }
            Ok(vec![Pat::Attrib { mask, compressed }])
        }
        Key::Len => match depth_value(&value) {
            Some((a, b)) => Ok(vec![Pat::Len(a, b)]),
            None => Err(format!("{shown}: not a path length (try len:>260, len:<=200 or len:100..260)")),
        },
        Key::Dupe | Key::SizeDupe => {
            if !value.trim().is_empty() {
                return Err(format!("{shown}: takes no value: give dupe: or sizedupe: as a word of its own"));
            }
            // a flag of the whole query: inside a group, child: or a|b it would quietly change
            // what the other words mean
            if !own_word {
                return Err(format!("{shown}: goes as a word of its own (not with !, in <...>, child: or a|b)"));
            }
            if p.key == Key::Dupe {
                q.dupe = true;
            } else {
                q.size_dupe = true;
            }
            Ok(Vec::new())
        }
        Key::Date => {
            let words = ["today", "yesterday", "lastweek", "lastmonth", "thismonth", "thisyear"];
            let needs_today = words.iter().any(|w| value.to_ascii_lowercase().contains(w));
            let day = if needs_today { *today.get_or_insert_with(|| local_day(unix_secs(now))) } else { None };
            match date_value(&value, day) {
                Some((a, b)) => Ok(vec![Pat::Date(a, b)]),
                None => Err(format!(
                    "{shown}: not a date (try dm:2026-01-15, dm:2026-11, dm:2026, dm:>2026-01-15, dm:2026-01..2026-03 or dm:lastweek)"
                )),
            }
        }
        Key::Regex => {
            let path = p.path || value.contains("\\\\");
            let re = RegexBuilder::new(&value).case_insensitive(!p.case).size_limit(1 << 22).dfa_size_limit(1 << 22).build();
            match re {
                Ok(re) => Ok(vec![Pat::Regex { re: Box::new(re), path, case: p.case }]),
                Err(e) => {
                    let e = e.to_string();
                    let why = e.lines().rev().find_map(|l| l.trim().strip_prefix("error:")).unwrap_or(e.lines().next().unwrap_or("")).trim().to_string();
                    Err(format!("{shown}: not a regular expression ({why})"))
                }
            }
        }
        Key::Text => {
            // the type macros: `audio:` is the Type filter's audio extensions, one list for both;
            // quotes make plain text of them too (`"audio:"`)
            let quoted = rest.iter().any(|&(_, q)| q);
            if !p.case && !p.exact && !p.path && !quoted {
                if let Some(f) = macro_family(value.trim()) {
                    return Ok(crate::kinds::extensions(f).iter().map(|e| Pat::Ext(e.as_bytes().to_vec())).collect());
                }
                if let Some((kw, _)) = value.trim().split_once(':') {
                    if macro_family(&format!("{kw}:").to_lowercase()).is_some() {
                        return Err(format!("{shown}: a type takes no value (audio: on its own is every audio file; use ext:mp3 for one)"));
                    }
                }
            }
            // as before, an alternative's spaces at either end do not count (`"report "`)
            // `/` is a path separator too: names cannot hold it
            let v = value.trim().replace('/', "\\");
            let v = if p.case { v } else { v.to_lowercase() };
            let path = p.path || v.contains('\\');
            let glob = v.contains(['*', '?']);
            Ok(vec![match (p.case, path, glob, p.exact) {
                (false, false, false, false) => Pat::Sub(v.into_bytes()),
                (false, false, false, true) => Pat::Exact(v.into_bytes()),
                (false, false, true, _) => Pat::Glob(v.chars().collect()),
                (false, true, false, false) => Pat::PathSub(v.into_bytes()),
                (false, true, false, true) => Pat::PathExact(trim_sep(v.as_bytes()).to_vec()),
                (false, true, true, _) => Pat::PathGlob(v.chars().collect()),
                (true, false, false, false) => Pat::CaseSub(v.into_bytes()),
                (true, false, false, true) => Pat::CaseExact(v.into_bytes()),
                (true, false, true, _) => Pat::CaseGlob(v.chars().collect()),
                (true, true, false, false) => Pat::CasePathSub(v.into_bytes()),
                (true, true, false, true) => Pat::CasePathExact(trim_sep(v.as_bytes()).to_vec()),
                (true, true, true, _) => Pat::CasePathGlob(v.chars().collect()),
            }])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// An item for testing words one at a time.
    struct T {
        name: String,
        path: String,
        dir: bool,
        size: u64,
        modified: Option<i64>,
        depth: u32,
        /// Its direct children: (name, is a folder).
        kids: Vec<(String, bool)>,
        attrs: u32,
    }

    impl Item for T {
        fn name(&self) -> &[u8] {
            // the tests use ASCII names here
            unreachable_lower(&self.name)
        }
        fn orig_name(&self) -> Cow<'_, [u8]> {
            Cow::Borrowed(self.name.as_bytes())
        }
        fn path(&self) -> &[u8] {
            unreachable_lower(&self.path)
        }
        fn orig_path(&self) -> &[u8] {
            self.path.as_bytes()
        }
        fn is_dir(&self) -> bool {
            self.dir
        }
        fn size(&self) -> u64 {
            self.size
        }
        fn modified(&self) -> Option<i64> {
            self.modified
        }
        fn depth(&self) -> u32 {
            self.depth
        }
        fn children(&self) -> u32 {
            self.kids.len() as u32
        }
        fn child_files(&self) -> u32 {
            self.kids.iter().filter(|k| !k.1).count() as u32
        }
        fn attributes(&self) -> u32 {
            self.attrs
        }
        fn compressed(&self) -> bool {
            self.attrs & (crate::attr::COMPRESSED | crate::attr::WOF) != 0
        }
        fn child_matches(&self, ts: &[Term]) -> bool {
            self.kids.iter().any(|(name, dir)| {
                let it = T { name: name.clone(), path: String::new(), dir: *dir, size: 0, modified: None, depth: 0, kids: Vec::new(), attrs: 0 };
                ts.iter().all(|t| t.matches(&it))
            })
        }
    }

    /// Lower-case copies leaked for the test's lifetime (tests only).
    fn unreachable_lower(s: &str) -> &'static [u8] {
        Box::leak(s.to_lowercase().into_boxed_str()).as_bytes()
    }

    fn file(name: &str) -> T {
        T { name: name.into(), path: format!("C:\\Data\\{name}"), dir: false, size: 100, modified: None, depth: 1, kids: Vec::new(), attrs: 0 }
    }

    fn hit(q: &Query, it: &T) -> bool {
        q.errors.is_empty() && q.terms.iter().all(|t| t.matches(it)) && q.size_ok(it.size) && match q.kind {
            Kind::Any => true,
            Kind::Files => !it.dir,
            Kind::Folders => it.dir,
        }
    }

    fn m(q: &str, it: &T) -> bool {
        hit(&Query::parse(q), it)
    }

    /// `da:` is taken out of an AI search as a range, not searched as text; quoted is text.
    #[test]
    fn access_dates_are_taken_out_of_a_query() {
        let now = UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let jan = local_midnight(days_from_civil(2026, 1, 1)).unwrap();
        let feb = local_midnight(days_from_civil(2026, 2, 1)).unwrap();
        let (text, r) = split_accessed("*.mp4 da:2026-01", now);
        assert_eq!(text, "*.mp4");
        assert_eq!(r.unwrap(), Some((jan, feb)));
        assert_eq!(split_accessed("da:2026-01 *.mp4", now).0, "*.mp4");
        assert_eq!(split_accessed("x dateaccessed:2026-01 y", now).0, "x y");
        // quoted, it is a word to look for; nothing is read for it
        let (text, r) = split_accessed("\"da:2026-01\" x", now);
        assert_eq!(text, "\"da:2026-01\" x");
        assert_eq!(r.unwrap(), None);
        // a value that isn't a date is said, never searched as text
        assert!(split_accessed("x da:2026-13", now).1.unwrap_err().contains("isn't a date"));
        // two of them: the part both allow
        let (a, b) = split_accessed("da:>=2026-01-10 da:<=2026-01-20", now).1.unwrap().unwrap();
        assert_eq!(a, local_midnight(days_from_civil(2026, 1, 10)).unwrap());
        assert_eq!(b, local_midnight(days_from_civil(2026, 1, 21)).unwrap());
        // being typed: ignored
        assert_eq!(split_accessed("x da:", now).1.unwrap(), None);
        // never negated
        assert!(split_accessed("x !da:2026-01", now).1.is_err());
    }

    #[test]
    fn sizes_and_empty() {
        assert_eq!(parse_size("1gb"), Some(1 << 30));
        assert_eq!(parse_size("1.5k"), Some(1536));
        assert_eq!(parse_size("12"), Some(12));
        assert_eq!(parse_size("1xb"), None);
        assert_eq!(parse_size("."), None);
        assert!(Query::parse("  ").is_empty());
        assert!(Query::parse("!").is_empty());
        // `>` with no number is a word
        assert!(!Query::parse(">abc").terms.is_empty());
        assert!(Query::parse(">abc").errors().is_empty());
    }

    #[test]
    fn highlight() {
        let q = Query::parse("!x 2024");
        assert_eq!(q.highlight("Report 2024.PDF"), Some((7, 11)));
        assert_eq!(Query::parse("*.pdf").highlight("a.pdf"), None);
        assert_eq!(Query::parse("case:PDF").highlight("a.pdf.PDF"), Some((6, 9)));
        // the Ohm sign lowercases one byte shorter, Ⱥ one byte longer: the same length in all,
        // but the offsets would cut a character (and panic where the name is drawn)
        assert_eq!(Query::parse("a").highlight("\u{2126}a\u{023A}"), None);
    }

    #[test]
    fn bare_sizes_include_their_edges() {
        let mut f = file("a.txt");
        f.size = 0;
        assert!(m(">=0", &f), ">=0 includes empty files");
        assert!(!m(">0", &f));
        assert!(m("<=0", &f));
        assert!(!m("<0", &f), "nothing is smaller than 0");
        f.size = 5 << 20;
        assert!(m(">=5mb", &f));
        assert!(!m(">5mb", &f));
        assert!(m("<=5mb", &f));
        assert!(!m("<5mb", &f));
        assert!(m(">1mb <10mb", &f));
        assert!(!m(">1mb <2mb", &f));
    }

    #[test]
    fn size_keyword() {
        let mut f = file("a.iso");
        f.size = 2 << 30;
        assert!(m("size:>1gb", &f));
        assert!(m("size:100mb..2gb", &f), "both ends included");
        assert!(!m("size:100mb..1gb", &f));
        assert!(m("size:1gb..", &f));
        assert!(m("size:..2gb", &f));
        assert!(!m("size:empty", &f));
        assert!(m("size:2gb", &f));
        assert!(m("!size:empty", &f));
        assert!(m("size:<1kb|>1gb", &f), "alternatives keep the keyword");
        f.size = 0;
        assert!(m("size:empty", &f));
        assert!(m("SIZE:EMPTY", &f));
        let q = Query::parse("size:lots");
        assert_eq!(q.errors().len(), 1);
        assert!(q.errors()[0].starts_with("size:lots:"), "{:?}", q.errors());
        assert!(!q.is_empty());
        assert!(!hit(&q, &f), "a query with a mistake finds nothing");
    }

    #[test]
    fn ext_keyword() {
        assert!(m("ext:iso", &file("Disk.ISO")));
        assert!(m("ext:.iso", &file("disk.iso")));
        assert!(!m("ext:is", &file("disk.iso")), "the whole extension");
        assert!(!m("ext:iso", &file("iso.txt")));
        assert!(m("ext:jpg;png", &file("a.png")));
        assert!(m("ext:jpg|png", &file("a.jpg")));
        assert!(!m("ext:jpg;png", &file("a.gif")));
        assert!(m("ext:mp*", &file("a.mp4")));
        assert!(!m("ext:gitignore", &file(".gitignore")), "as the Types table: no extension");
        assert!(m("!ext:tmp", &file("a.txt")));
        assert!(m("foo|ext:iso", &file("a.iso")));
        assert!(m("foo|ext:iso", &file("foo.txt")));
        let mut d = file("photos.jpg");
        d.dir = true;
        assert!(!m("ext:jpg", &d), "files only");
        assert!(Query::parse("ext:").is_empty(), "nothing typed yet");
    }

    #[test]
    fn path_keyword_and_slashes() {
        let mut f = file("config.vdf");
        f.path = "C:\\Games\\Steam\\config.vdf".into();
        assert!(m("path:steam", &f));
        assert!(!m("steam", &f), "a plain word looks at the name");
        assert!(m("path:steam\\config", &f));
        assert!(m("/steam/ *.vdf", &f), "/ is a separator");
        assert!(m("path:games/steam", &f));
        assert!(m("path:*\\steam\\*.vdf", &f));
        assert!(m("path:exact:c:\\games\\steam\\config.vdf", &f));
        assert!(!m("path:exact:c:\\games\\steam", &f));
    }

    #[test]
    fn case_and_exact() {
        let f = file("README.md");
        assert!(m("case:README", &f));
        assert!(!m("case:readme", &f));
        assert!(m("case:READ*.md", &f));
        assert!(!m("case:read*.md", &f));
        assert!(m("exact:readme.md", &f));
        assert!(m("wholename:README.MD", &f));
        assert!(!m("exact:readme", &f));
        assert!(m("case:exact:README.md", &f));
        assert!(m("exact:case:README.md", &f));
        assert!(!m("case:exact:README.MD", &f));
        assert!(m("case:\\Data\\", &f), "case on the path");
        assert!(!m("case:\\data\\", &f));
        assert!(m("Case:Exact:README.md", &f), "keywords in any case");
    }

    #[test]
    fn regex_keyword() {
        assert!(m("regex:^read.*\\.md$", &file("README.md")), "any case by default");
        assert!(!m("case:regex:^read", &file("README.md")));
        assert!(m("case:regex:^READ", &file("README.md")));
        assert!(m("regex:^log\\d+\\.txt$", &file("log12.txt")));
        assert!(m("regex:a|b", &file("b.txt")), "| belongs to the regex");
        assert!(m("regex:\\\\data\\\\read", &file("README.md")), "\\\\ means the path");
        assert!(!m("regex:\\\\nothere\\\\", &file("README.md")));
        assert!(m("!regex:^x", &file("README.md")));
        for bad in ["regex:(", "regex:[a-", "regex:a{99999999}"] {
            let q = Query::parse(bad);
            assert_eq!(q.errors().len(), 1, "{bad}");
            assert!(q.errors()[0].starts_with(bad), "{:?}", q.errors());
        }
    }

    #[test]
    fn depth_keyword() {
        let mut f = file("a");
        f.depth = 3;
        assert!(m("depth:<=3", &f));
        assert!(!m("depth:<3", &f));
        assert!(m("depth:3", &f));
        assert!(m("depth:2..4", &f));
        assert!(m("depth:>2", &f));
        assert!(!Query::parse("depth:x").errors().is_empty());
    }

    #[test]
    fn quotes_make_text() {
        // a quoted `!` is a letter
        assert!(m("\"!x\"", &file("a!x.txt")));
        assert!(!m("\"!x\"", &file("abc.txt")));
        // a `!` before quotes still negates
        assert!(m("!\"my file\"", &file("other.txt")));
        assert!(!m("!\"my file\"", &file("my file.txt")));
        // quoted `;` and `|` are letters
        assert!(m("\"a;b\"", &file("a;b.txt")));
        assert!(!m("\"a;b\"", &file("a.txt")));
        assert!(!m("\"a|b\"", &file("b.txt")));
        assert!(m("\"my file\"|other", &file("other.txt")));
        // a quoted keyword is text
        assert!(m("\"dm:x\"", &file("dm:x")));
        assert!(Query::parse("\"dm:x\"").errors().is_empty());
        assert!(m("\">1\"", &file("a>1")));
        // spaces at the ends of a quoted word do not count, as before
        assert!(m("\"report \"", &file("report.txt")));
        assert!(m("\" report\"|zzz", &file("report.txt")));
        // a keyword's value may be quoted
        assert!(m("path:\"my dir\"", &T { path: "C:\\my dir\\a".into(), ..file("a") }));
    }

    #[test]
    fn unknown_keywords_are_text() {
        let q = Query::parse("C: foo:bar http://x");
        assert!(q.errors().is_empty());
        assert_eq!(q.terms.len(), 3);
        assert!(m("c:", &T { path: "C:\\a".into(), ..file("C:") }));
        assert!(m("foo:bar", &file("foo:bar")));
        // and `c:\` is a path word, as before
        assert!(m("c:\\data\\", &file("x")));
    }

    /// `<a b>|c`: all of the words of a group, or c. Groups nest; `!` before one negates it;
    /// quotes keep `<` and `>` plain; sizes and keywords still work where they did.
    #[test]
    fn groups_are_all_of_a_word_or_one_of_its_others() {
        assert!(m("<report 2024>|draft", &file("Report 2024.pdf")));
        assert!(m("<report 2024>|draft", &file("draft.txt")));
        assert!(!m("<report 2024>|draft", &file("report.pdf")), "the group needs both words");
        assert!(!m("<report 2024>|draft", &file("2024.txt")));
        // `|` inside a group is an alternative of the word it follows
        assert!(m("<report 2024|draft>", &file("Report 2024.pdf")));
        assert!(m("<report 2024|draft>", &file("report draft.pdf")));
        assert!(!m("<report 2024|draft>", &file("2024.pdf")), "a and (b or c), not (a and b) or c");
        // nested: a and ((b and c) or d)
        assert!(m("<a <b c>|d>", &file("a b c.txt")));
        assert!(m("<a <b c>|d>", &file("a d.txt")));
        assert!(!m("<a <b c>|d>", &file("a b.txt")));
        // an inner group may come first, or follow a `!`
        assert!(m("<<a b>|c d>", &file("a b d.txt")));
        assert!(m("<<a b>|c d>", &file("c d.txt")));
        assert!(!m("<<a b>|c d>", &file("a b.txt")));
        assert!(m("<<a b> c>", &file("a b c.txt")));
        assert!(!m("<<a b> c>", &file("a b.txt")));
        assert!(m("x|<<a b>|c d>", &file("c d.txt")));
        assert!(m("<!<a b> c>", &file("c.txt")));
        assert!(!m("<!<a b> c>", &file("a b c.txt")));
        assert!(!m("!<<temp cache>|backup>", &file("temp cache.txt")));
        assert!(m("!<<temp cache>|backup>", &file("temp.txt")));
        assert!(!m("!<<a b>>", &file("a b.txt")));
        assert!(m("!<<a b>>", &file("a.txt")));
        // a size as the last word of a group is a size all the same
        let mut five = file("a.txt");
        five.size = 5;
        assert!(m("<a <1gb>", &five));
        assert!(m("<a <=1gb>", &five));
        assert!(!m("<a >1gb>", &five));
        let mut log = file("x.log");
        log.size = 5;
        assert!(m("<*.log <1mb>|*.tmp", &log));
        assert!(m("<*.log <1mb>|*.tmp", &file("x.tmp")));
        // `;` splits as `|` does, in a group too
        assert!(m("<a b;c>", &file("a c.txt")));
        assert!(!m("<a b;c>", &file("b c.txt")));
        // not a group
        assert!(m("!<temp cache>", &file("other.txt")));
        assert!(m("!<temp cache>", &file("my temp file.txt")), "not both of them");
        assert!(!m("!<temp cache>", &file("my temp cache.txt")));
        assert!(m("!<temp cache>", &file("my cache.txt")), "one word of the two is not both");
        assert!(m("!<temp cache>", &file("temp.txt")), "one word of the group is not all of it");
        // sizes, dates and kinds are words of a group
        let mut big = file("a b.iso");
        big.size = 2 << 30;
        assert!(m("<a >1gb>", &big));
        assert!(!m("<a >1gb>", &T { size: 5, ..file("b a.txt") }));
        let mut d = file("cache");
        d.dir = true;
        assert!(!m("<folder:cache>", &file("cache.txt")));
        assert!(m("<folder:cache>", &d));
        assert!(m("<size:>1gb ext:iso>", &T { size: 3 << 30, ..file("x.iso") }));
        // quotes keep < > plain
        assert!(m("\"<a b>\"", &file("<a b>.txt")));
        assert!(!m("\"<a b>\"", &file("a b.txt")));
        // a `<` with no group around it is text, as before
        assert!(m("<abc", &file("<abc.txt")));
        assert!(m("a>b", &file("a>b.txt")));
        assert!(m("<10kb", &T { size: 5, ..file("x") }), "a size, not a group");
        assert!(Query::parse("<>").is_empty(), "an empty group asks nothing");
        assert_eq!(Query::parse("<a").terms.len(), 1, "an unclosed < is text");
        // a group's words are query words all the same: a path word inside one still works
        let mut deep = file("x.log");
        deep.path = "C:\\Data\\logs\\x.log".into();
        assert!(m("<\\logs\\ *.log>", &deep));
    }

    /// Whatever follows a group's `>` in its word is never read, so it is an error, not a
    /// silently widened list.
    #[test]
    fn text_glued_to_a_groups_end_is_an_error() {
        for q in ["<a b>>1gb", "<a b>>=5mb", "<a b>>"] {
            assert_eq!(Query::parse(q).errors().len(), 1, "{q}");
        }
        assert!(Query::parse("<a b> >1gb").errors().is_empty());
        assert!(Query::parse("<a b>|c").errors().is_empty());
    }

    /// `!<dm:2026>` skips items with no modified time as `!dm:2026` does; a group with another
    /// word in it still follows the usual `!`.
    #[test]
    fn negated_group_of_dates_skips_untimed_items() {
        let now = local(2026, 6, 1, 12);
        let none = file("a.txt");
        assert!(!md("!dm:2026", now, &none));
        assert!(!md("!<dm:2026>", now, &none));
        assert!(md("!<dm:2026>", now, &dated(local(2025, 3, 1, 12))));
        assert!(!md("!<dm:2026>", now, &dated(local(2026, 3, 1, 12))));
        assert!(md("!<dm:2026 a>", now, &none));
    }

    /// A `<` costs a short look, not a scan of the rest of the word, so a run of them is cheap.
    #[test]
    fn many_open_brackets_are_cheap() {
        let start = std::time::Instant::now();
        let _ = Query::parse(&"<".repeat(4096));
        let _ = Query::parse(&"<=".repeat(2048));
        let _ = Query::parse(&"<1".repeat(2048));
        assert!(start.elapsed() < Duration::from_secs(1), "{:?}", start.elapsed());
    }

    /// A `<` with no `>` after it is read in one pass whatever the text holds: the old matcher
    /// tried again for every `<` before it, so 30 of them took half a minute and an address
    /// list could take hours (this runs on every text change, in the window's thread).
    #[test]
    fn unclosed_groups_and_address_lists_are_cheap() {
        let start = std::time::Instant::now();
        let q = Query::parse(&format!("{}x", "<a ".repeat(200)));
        assert!(q.errors().is_empty(), "{:?}", q.errors());
        assert!(!q.terms.is_empty());
        let list: String = (0..40).map(|i| format!("Ann Lee <ann{i}@example.com>, ")).collect();
        let q = Query::parse(&list);
        assert!(q.errors().is_empty(), "{:?}", q.errors());
        assert!(start.elapsed() < Duration::from_secs(1), "{:?}", start.elapsed());
    }

    /// Groups and `child:` deeper than [`MAX_DEPTH`], or a query past [`MAX_QUERY_CHARS`], are
    /// refused with an error: they are read for every keystroke and from the AI, and must never
    /// recurse until the stack is gone (an abort, not a panic).
    #[test]
    fn deep_or_long_queries_are_refused() {
        let nested = format!("{}a{}", "<a ".repeat(1000), ">".repeat(1000));
        assert!(Query::parse(&nested).errors().iter().any(|e| e.contains("too many nested")), "{:?}", Query::parse(&nested).errors());
        let child = format!("{}a", "child:".repeat(500));
        assert!(Query::parse(&child).errors().iter().any(|e| e.contains("too many nested")), "{:?}", Query::parse(&child).errors());
        // the deepest allowed still reads
        let ok = format!("{}a{}", "<a ".repeat(MAX_DEPTH as usize), ">".repeat(MAX_DEPTH as usize));
        assert!(Query::parse(&ok).errors().is_empty(), "{:?}", Query::parse(&ok).errors());
        // 100 000: past the length limit, so nothing is scanned at all
        let huge = format!("{}a{}", "<".repeat(100_000), ">".repeat(100_000));
        let q = Query::parse(&huge);
        assert!(q.errors().iter().any(|e| e.contains("too long")), "{:?}", q.errors());
        let q = Query::parse(&format!("{}a", "child:".repeat(100_000)));
        assert!(q.errors().iter().any(|e| e.contains("too long")), "{:?}", q.errors());
        // `words()` and the date splitter stop at the same limit
        assert!(Query::words(&huge).is_empty());
        let now = UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        assert!(split_read_dates(&huge, now).1.is_err());
    }

    /// A `<` that starts a bare size stays a size while the next comparison is typed: `<10mb >`
    /// kept its limit before the group syntax, and `dm:>` was still reported as not a date.
    #[test]
    fn a_bare_size_never_opens_a_group() {
        let q = Query::parse("<10mb >");
        assert_eq!(q.max, Some((10u64 << 20) - 1), "{:?}", q.errors());
        assert!(q.errors().is_empty(), "{:?}", q.errors());
        let q = Query::parse("<10mb >1mb");
        assert_eq!((q.min, q.max), (Some((1 << 20) + 1), Some((10u64 << 20) - 1)));
        let q = Query::parse("<10mb dm:>");
        assert_eq!(q.max, Some((10u64 << 20) - 1));
        assert!(q.errors().iter().any(|e| e.contains("dm:>")), "{:?}", q.errors());
        let q = Query::parse("<=10 regex:^c a>");
        assert_eq!(q.max, Some(10));
        let q = Query::parse("<0 x>");
        assert!(!q.size_ok(0) && !q.size_ok(1 << 40), "<0 is nothing, not a group");
        // a group whose first word is such text can be quoted
        assert!(m("\"<10mb\"", &file("<10mb")));
    }

    /// `child:` and the counts walk a folder's children: they are checked after the names, so a
    /// query that uses them does that walk only for the items the cheap words passed.
    #[test]
    fn child_words_are_checked_after_names() {
        let q = Query::parse("child:*.tmp size:>1mb report");
        let (early, path) = q.split_terms();
        assert!(path.is_empty());
        assert_eq!(early.iter().map(|t| t.cost()).collect::<Vec<_>>(), [0, 1, 2]);
        assert!(format!("{:?}", early[2]).contains("Child"), "{:?}", early[2]);
        // inside a group too
        let q = Query::parse("<child:*.tmp report>");
        let (early, _) = q.split_terms();
        assert_eq!(early[0].cost(), 2);
    }

    /// `parent:` is the folder directly above; `child:` a folder holding a match; the counts are
    /// of direct children.
    #[test]
    fn folder_shape() {
        let mut steam = file("Steam");
        steam.dir = true;
        steam.path = "C:\\Games\\Steam".into();
        steam.kids = vec![("config.vdf".into(), false), ("Report.pdf".into(), false), ("cache".into(), true)];
        let mut vdf = file("config.vdf");
        vdf.path = "C:\\Games\\Steam\\config.vdf".into();
        let mut cache = file("cache");
        cache.dir = true;
        cache.path = "C:\\Games\\Steam\\cache".into();
        cache.kids = vec![("old.tmp".into(), false)];
        // parent:
        assert!(m("parent:C:/Games/Steam", &vdf));
        assert!(m("parent:c:\\games\\steam\\", &vdf), "slashes, case and a trailing \\ do not count");
        assert!(m("parent:steam", &vdf), "a bare name is a folder of that name");
        assert!(m("parent:steam", &cache), "the folder's own parent is the folder above it");
        assert!(!m("parent:steam", &T { path: "C:\\Games\\Steam\\cache\\old.tmp".into(), ..file("old.tmp") }), "not what is deeper");
        assert!(!m("parent:c:\\games", &vdf), "direct children only");
        assert!(m("parent:c:\\games", &steam));
        assert!(m("file: parent:steam", &vdf));
        assert!(!m("folder: parent:steam", &vdf));
        // a file at the top of a drive is in `c:`
        let top = T { path: "C:\\a.txt".into(), ..file("a.txt") };
        assert!(m("parent:C:\\", &top));
        // child:
        assert!(m("child:*.vdf", &steam));
        assert!(m("child:cache", &steam), "a folder among the children counts");
        assert!(!m("child:*.iso", &steam));
        assert!(m("child:old.tmp", &cache));
        assert!(!m("child:old.tmp", &vdf), "a file has no children");
        assert!(m("child:ext:vdf|pdf", &steam));
        // counts are of a folder's children: a file has none to count
        assert!(m("childcount:3", &steam));
        assert!(m("childcount:>2", &steam));
        assert!(!m("childcount:>3", &steam));
        assert!(m("childfilecount:2", &steam));
        assert!(m("childfilecount:1..2", &steam));
        assert!(!m("childfilecount:>2", &steam));
        let mut empty = file("empty");
        empty.dir = true;
        empty.path = "C:\\Games\\empty".into();
        assert!(m("childcount:0", &empty), "an empty folder");
        assert!(m("childfilecount:0", &empty));
        assert!(!m("childcount:0", &vdf), "a file is not an empty folder");
        assert!(!m("childfilecount:0", &vdf));
        assert!(!m("childcount:0", &steam));
        // pinned: `!childcount:>1` is every file and every folder with at most one child
        assert!(m("!childcount:>1", &empty));
        assert!(m("!childcount:>1", &cache));
        assert!(m("!childcount:>1", &vdf));
        assert!(!m("!childcount:>1", &steam));
        assert!(m("childcount:>0 folder:", &steam));
        assert!(!m("childcount:>0 file:", &steam));
        // mistakes say what is wrong
        for bad in ["childcount:x", "childfilecount:lots", "child:\\steam\\", "child:case:path:x", "child:case:a\\b"] {
            let q = Query::parse(bad);
            assert_eq!(q.errors().len(), 1, "{bad}");
            assert!(q.errors()[0].starts_with("child"), "{:?}", q.errors());
        }
        // nothing typed after the keyword is ignored
        for q in ["parent:", "child:", "childcount:", "childfilecount:"] {
            assert!(Query::parse(q).is_empty(), "{q}");
        }
    }

    #[test]
    fn attributes_and_path_length() {
        let mut hidden = file("a.txt");
        hidden.attrs = crate::attr::HIDDEN | crate::attr::SYSTEM;
        assert!(m("attrib:h", &hidden));
        assert!(m("attrib:hs", &hidden));
        assert!(m("attrib:sh", &hidden), "in any order");
        assert!(!m("attrib:r", &hidden));
        assert!(m("!attrib:r", &hidden));
        let mut link = file("l");
        link.attrs = crate::attr::REPARSE_POINT;
        assert!(m("attrib:l", &link));
        assert!(!m("attrib:hs", &link));
        let mut comp = file("z");
        comp.attrs = crate::attr::COMPRESSED;
        assert!(m("attrib:c", &comp));
        assert!(m("compressed:", &comp), "c is the same as compressed:");
        assert!(!m("attrib:c", &hidden));
        // len: the full path in characters, a folder's without its trailing \
        let long = T { path: format!("C:\\{}", "a".repeat(260)), ..file("x") };
        assert!(m("len:>260", &long));
        assert!(m("len:263", &long));
        assert!(!m("len:<=260", &long));
        assert!(m("len:..270", &long));
        assert!(m("len:<14", &file("a.txt")), "C:\\Data\\a.txt is 13 characters");
        assert!(!m("len:<13", &file("a.txt")));
        assert!(m("len:13", &file("a.txt")));
        assert!(!m("len:13 folder:", &file("a.txt")));
        // a Turkish İ lower-cases to two characters; the path is counted as it is on disk
        let dotted = T { path: r"C:\İİİ.txt".into(), ..file("x") };
        assert!(m("len:10", &dotted));
        assert!(!m("len:13", &dotted));
        // characters, not bytes
        let nonascii = T { path: "C:\\é\\b.txt".into(), ..file("b.txt") };
        assert!(m("len:10", &nonascii), "C:\\é\\b.txt is 10 characters, {} bytes", "C:\\é\\b.txt".len());
        assert!(!m("len:11", &nonascii), "not counted in bytes");
        for bad in ["attrib:x", "attrib:hx", "len:lots"] {
            let q = Query::parse(bad);
            assert_eq!(q.errors().len(), 1, "{bad}");
        }
        for q in ["attrib:", "len:"] {
            assert!(Query::parse(q).is_empty(), "{q}");
        }
    }

    /// `dupe:` and `sizedupe:` are flags of the whole query: among the matches of the other
    /// words, only the ones with a copy.
    #[test]
    fn duplicates_are_flags() {
        let q = Query::parse("dupe:");
        assert!(q.dupe && !q.size_dupe && q.group_by() == Some(GroupBy::Name));
        assert!(!q.is_empty());
        let q = Query::parse("sizedupe: report");
        assert!(q.size_dupe && q.group_by() == Some(GroupBy::Size));
        assert_eq!(q.terms.len(), 1, "the word after it still counts");
        assert_eq!(Query::parse("report").group_by(), None);
        // they take no value
        for bad in ["dupe:yes", "sizedupe:1gb"] {
            let q = Query::parse(bad);
            assert_eq!(q.errors().len(), 1, "{bad}");
        }
        // and they go as a word of their own at the top: not negated, in a group, in child: or
        // one of several alternatives (where they would silently change the other words)
        for bad in ["!dupe:", "<a dupe:>", "<readme dupe:>", "child:dupe:", "a|dupe:", "dupe:|a", "!sizedupe:", "child:sizedupe:"] {
            let q = Query::parse(bad);
            assert!(!q.errors().is_empty(), "{bad}");
            assert!(q.errors().iter().any(|e| e.contains("word of its own")), "{bad}: {:?}", q.errors());
            assert!(!q.dupe && !q.size_dupe, "{bad} is not a duplicates search");
        }
        // quoted, they are text
        assert!(m("\"dupe:\"", &file("dupe:")));
        // keep_shared: a name or size two of them have, once
        let names = ["a.txt", "b.txt", "a.txt", "c.txt"];
        assert_eq!(keep_shared(4, |i| Some(names[i])), [0, 2]);
        let sizes = [1u64, 2, 1, 3];
        assert_eq!(keep_shared(4, |i| Some(sizes[i])), [0, 2]);
        assert!(keep_shared(4, |_| None::<u64>).is_empty());
    }

    /// The type macros are the Type filter's own lists, one list for both.
    #[test]
    fn type_macros_are_the_type_filters_lists() {
        for (mac, f) in [("audio:", crate::kinds::AUDIO), ("video:", crate::kinds::VIDEO), ("pic:", crate::kinds::IMAGES),
            ("doc:", crate::kinds::DOCUMENTS), ("exe:", crate::kinds::PROGRAMS), ("zip:", crate::kinds::ARCHIVES)] {
            assert_eq!(Query::parse(mac).errors().len(), 0, "{mac}");
            for e in crate::kinds::extensions(f) {
                assert!(m(mac, &file(&format!("a.{e}"))), "{mac} {e}");
            }
            assert!(!m(mac, &file(&format!("a.{}", "qwe"))), "{mac}");
            assert!(m(&format!("!{mac}"), &file("a.qwe")), "{mac}");
            assert!(!m(&format!("!{mac}"), &file(&format!("a.{}", crate::kinds::extensions(f)[0]))), "{mac}");
            // the box's Type chip names it
            let w = Query::words(mac);
            assert_eq!((w[0].key, &w[0].value[..]), (WordKey::Ext, crate::kinds::extensions(f).join(";").as_str()), "{mac}");
        }
        // a macro takes no value, and that is said
        let q = Query::parse("audio:mp3");
        assert_eq!(q.errors().len(), 1);
        assert!(q.errors()[0].contains("takes no value"), "{:?}", q.errors());
        // a macro keeps an inherited keyword: `ext:jpg|audio:` is both lists
        assert!(m("ext:jpg|audio:", &file("a.mp3")));
        assert!(m("ext:jpg|audio:", &file("a.jpg")));
        assert!(!m("ext:jpg|audio:", &file("a.txt")));
        // quotes make plain text of it: `"audio:"` is a name with that text
        assert!(m("\"audio:\"", &file("audio:")));
        assert!(!m("\"audio:\"", &file("a.mp3")));
        // and `words()` reports a macro only where it is the type word itself
        assert_eq!(Query::words("regex:zip:")[0].key, WordKey::Regex);
        assert_eq!(Query::words("child:audio:")[0].key, WordKey::Child);
        assert_eq!(Query::words("ext:audio:")[0].key, WordKey::Ext);
        assert_eq!(Query::words("\"audio:\"")[0].key, WordKey::Text);
        // folders are not files
        let mut d = file("a.mp3");
        d.dir = true;
        assert!(!m("audio:", &d));
        // alternatives and groups keep them
        assert!(m("audio:|ext:iso", &file("a.mp3")));
        assert!(m("<audio: >1mb>", &T { size: 2 << 20, ..file("a.mp3") }));
    }

    /// `dc:` and `da:` are taken out as the two times to read for each match: a value that isn't
    /// a date, one inside a group or an alternative, and `!` are all said.
    #[test]
    fn created_and_accessed_dates_are_read_per_match() {
        let now = UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let jan = local_midnight(days_from_civil(2026, 1, 1)).unwrap();
        let feb = local_midnight(days_from_civil(2026, 2, 1)).unwrap();
        let (text, r) = split_read_dates("*.mp4 dc:2026-01", now);
        assert_eq!(text, "*.mp4");
        let d = r.unwrap();
        assert_eq!(d.created, Some((jan, feb)));
        assert_eq!(d.accessed, None);
        // both at once, in any order
        let (text, d) = split_read_dates("da:>2026-01-31 dc:2026-01 x", now);
        assert_eq!(text, "x");
        let d = d.unwrap();
        assert!(d.created.is_some() && d.accessed.is_some());
        assert!(!d.is_empty());
        // the long names and any case
        assert_eq!(split_read_dates("x DATECREATED:2026-01", now).0, "x");
        assert_eq!(split_read_dates("x DateAccessed:2026-01", now).0, "x");
        // quoted, it is a word to look for; nothing is read for it
        assert_eq!(split_read_dates("\"dc:2026\" x", now).1.unwrap(), ReadDates::default());
        // two of one kind: the part both allow
        let d = split_read_dates("dc:>=2026-01-10 dc:<=2026-01-20", now).1.unwrap();
        assert_eq!(
            d.created,
            Some((local_midnight(days_from_civil(2026, 1, 10)).unwrap(), local_midnight(days_from_civil(2026, 1, 21)).unwrap()))
        );
        // mistakes say what is wrong: the value, the negation, or where the word is
        for (q, want) in [
            ("x dc:2026-13", "dc:2026-13 isn't a date Sprawl understands"),
            ("x da:soon", "da:soon isn't a date Sprawl understands"),
            ("x !da:2026", "da: can't be negated"),
            ("x !dc:2026", "dc: can't be negated"),
            ("<a da:2026>", "words of their own"),
            ("a|da:2026", "words of their own"),
            ("<dc:2026 x>", "words of their own"),
        ] {
            let e = split_read_dates(q, now).1.unwrap_err();
            assert!(e.contains(want), "{q}: {e}");
        }
        // being typed: ignored
        assert_eq!(split_read_dates("x dc:", now).1.unwrap(), ReadDates::default());
        assert_eq!(split_read_dates("x da:", now).1.unwrap(), ReadDates::default());
        // the AI's split takes only `da:` out: a `dc:` word stays a word there
        assert_eq!(split_accessed("x dc:2026", now).0, "x dc:2026");
        assert_eq!(split_accessed("x da:2026", now).0, "x");
    }

    #[test]
    fn never_panics() {
        for q in [
            "", "!", "\"", "\"\"", "!\"", "|", ";;", "a||b", ":", "dm:", "dm:..", "dm:>", "dm:<=", "size:..", "size:>", "depth:..",
            "regex:", "regex:\\", "ext:.", "case:", "exact:", "path:", "case:exact:path:", ">", ">=", "<", "<=", "size:1e999",
            ">99999999999999999999999tb", "dm:0000", "dm:99999", "dm:2026-02-30", "dm:2026-1-1", "dm:-1", "depth:99999999999",
            "dm:today..yesterday", "folder:", "file:!", "!folder:", "😀 é *ü?", "dm:1601", "dm:9999-12-31", "\\", "/", "*", "?",
            "<", ">", "<>", "<a", "a>", "<<a>>", "!<>", "<a|<b>", "<a b", "a b>", "<", "><", "<->", "<a><b>", "<|>",
            "<1mb <1mb >", "child:child:child:a", "<a <b <c>>>", "<=1 <=2 >", "<\"a b\">", "a|;|b", "<!>", "><<>>",
        ] {
            let q = Query::parse(q);
            let _ = hit(&q, &file("a.txt"));
        }
    }

    // ------------------------------------------------------------ dates

    fn local(y: i64, mo: u32, d: u32, h: i64) -> i64 {
        local_midnight(days_from_civil(y, mo, d)).unwrap() + h * 3600
    }

    fn at(secs: i64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs as u64)
    }

    fn dated(secs: i64) -> T {
        T { modified: Some(secs), ..file("a.txt") }
    }

    fn md(q: &str, now: i64, it: &T) -> bool {
        hit(&Query::parse_at(q, at(now)), it)
    }

    #[test]
    fn calendar() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        for z in [-800_000, -1, 0, 59, 365, 11_000, 20_000, 2_900_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 12), 31);
        assert_eq!(add_months(days_from_civil(2026, 3, 31), -1), days_from_civil(2026, 2, 28));
        assert_eq!(add_months(days_from_civil(2026, 1, 15), -1), days_from_civil(2025, 12, 15));
        assert_eq!(local_day(local(2026, 11, 17, 13)), Some(days_from_civil(2026, 11, 17)));
        assert_eq!(local_day(local(2026, 7, 1, 0)), Some(days_from_civil(2026, 7, 1)), "summer time too");
    }

    #[test]
    fn written_dates() {
        let now = local(2026, 11, 17, 15);
        let jan15 = dated(local(2026, 1, 15, 12));
        assert!(md("dm:2026-01-15", now, &jan15));
        assert!(md("dm:2026/01/15", now, &jan15));
        assert!(md("dm:2026-1-15", now, &jan15));
        assert!(!md("dm:2026-01-16", now, &jan15));
        assert!(md("dm:2026-01", now, &jan15));
        assert!(md("dm:2026", now, &jan15));
        assert!(!md("dm:2025", now, &jan15));
        assert!(!md("dm:>2026-01-15", now, &jan15), "after that day");
        assert!(md("dm:>2026-01-14", now, &jan15));
        assert!(md("dm:>=2026-01-15", now, &jan15));
        assert!(md("dm:<=2026-01-15", now, &jan15));
        assert!(!md("dm:<2026-01-15", now, &jan15));
        assert!(md("dm:<2026-01-16", now, &jan15));
        assert!(md("dm:<2027", now, &jan15));
        assert!(md("dm:2025-12..2026-03", now, &jan15));
        assert!(!md("dm:2026-02..2026-03", now, &jan15));
        assert!(md("dm:2026-01..", now, &jan15));
        assert!(md("dm:..2026-01-15", now, &jan15));
        assert!(md("datemodified:2026-01", now, &jan15));
        assert!(md("!dm:2025", now, &jan15));
        // the edges of a day: local midnight belongs to the day it starts
        assert!(md("dm:2026-01-15", now, &dated(local(2026, 1, 15, 0))));
        assert!(!md("dm:2026-01-15", now, &dated(local(2026, 1, 16, 0))));
        assert!(md("dm:2026-01-15", now, &dated(local(2026, 1, 16, 0) - 1)));
        // a date with no time never matches, not even with !
        let none = file("a.txt");
        assert!(!md("dm:2026", now, &none));
        assert!(!md("!dm:2026", now, &none));
        for bad in ["dm:2026-13", "dm:2026-02-30", "dm:26-01-01", "dm:soon", "dm:>x", "dm:2026-01..x"] {
            let q = Query::parse_at(bad, at(now));
            assert_eq!(q.errors().len(), 1, "{bad}");
        }
    }

    #[test]
    fn named_dates() {
        let now = local(2026, 11, 17, 15);
        let today = dated(local(2026, 11, 17, 1));
        let yesterday = dated(local(2026, 11, 16, 23));
        let six_days = dated(local(2026, 11, 11, 1));
        let ten_days = dated(local(2026, 11, 7, 1));
        let october = dated(local(2026, 10, 20, 1));
        let september = dated(local(2026, 9, 20, 1));
        let last_year = dated(local(2025, 12, 31, 23));
        assert!(md("dm:today", now, &today));
        assert!(!md("dm:today", now, &yesterday));
        assert!(md("dm:yesterday", now, &yesterday));
        assert!(!md("dm:yesterday", now, &today));
        assert!(md("dm:lastweek", now, &today));
        assert!(md("dm:lastweek", now, &six_days));
        assert!(!md("dm:lastweek", now, &ten_days));
        assert!(md("dm:lastmonth", now, &october));
        assert!(!md("dm:lastmonth", now, &september));
        assert!(md("dm:thismonth", now, &ten_days));
        assert!(!md("dm:thismonth", now, &october));
        assert!(md("dm:thisyear", now, &september));
        assert!(!md("dm:thisyear", now, &last_year));
        assert!(md("dm:today|yesterday", now, &yesterday));
        assert!(md("dm:>=yesterday", now, &today));
        assert!(md("DM:TODAY", now, &today));
        // "now" is the one given
        let later = local(2027, 3, 1, 12);
        assert!(!md("dm:today", later, &today));
    }

    #[test]
    fn words_say_where_and_what() {
        let q = "folder:cache !ext:tmp  size:>1gb >1mb \"dm:x\" case:exact:README dm: file: path:\"my dir\" ünï regex:a|b";
        let w = Query::words(q);
        let at = |i: usize| &q[w[i].span.clone()];
        assert_eq!(w.len(), 11);
        assert_eq!(at(0), "folder:cache");
        assert_eq!((w[0].kind, w[0].key, &w[0].value[..]), (Kind::Folders, WordKey::Text, "cache"));
        assert_eq!(&q[w[0].span.start..w[0].kind_end], "folder:");
        assert_eq!(&q[w[0].value_at..w[0].span.end], "cache");
        assert_eq!(at(1), "!ext:tmp");
        assert!(w[1].not);
        assert_eq!((w[1].key, &w[1].value[..]), (WordKey::Ext, "tmp"));
        assert_eq!(&q[w[1].prefix_end..w[1].value_at], "ext:");
        assert_eq!((w[2].key, &w[2].value[..]), (WordKey::Size, ">1gb"));
        assert_eq!((w[3].key, &w[3].value[..]), (WordKey::Compare, ">1mb"));
        assert_eq!(at(4), "\"dm:x\"");
        assert_eq!((w[4].key, &w[4].value[..]), (WordKey::Text, "dm:x"), "a quoted keyword is text");
        assert!(w[5].case && w[5].exact && !w[5].path);
        assert_eq!(&q[w[5].prefix_end..w[5].value_at], "case:exact:");
        assert_eq!((w[6].key, &w[6].value[..]), (WordKey::Date, ""));
        assert_eq!((w[7].kind, w[7].key), (Kind::Files, WordKey::None));
        assert_eq!(at(8), "path:\"my dir\"");
        assert!(w[8].path);
        assert_eq!(w[8].value, "my dir");
        assert_eq!(&q[w[8].value_at..w[8].span.end], "\"my dir\"", "the value's opening quote is the value's");
        assert_eq!(at(9), "ünï");
        assert_eq!((w[10].key, &w[10].value[..]), (WordKey::Regex, "a|b"));
        // `dc:` and `da:` are read for each match, not name words: no modifier may land on them
        let q = "report dc:2026 da:>2026-01-15";
        let w = Query::words(q);
        assert_eq!((w[0].key, w[1].key, w[2].key), (WordKey::Text, WordKey::ReadDate, WordKey::ReadDate));
        assert_eq!(w[1].value, "dc:2026");
        // quoted, `dc:` is a name again
        assert_eq!(Query::words("\"dc:2026\"")[0].key, WordKey::Text);
        // the same words as the parser reads
        for q in ["", "  ", "\"\"", "a\" \"b c", "!", "folder:", "\"a b\"|c d", "<a b>|c", "<a <b c> d>|e", "!<a b>|c", "<a|b> c"] {
            let n = tokens(q).len();
            assert_eq!(Query::words(q).len(), n, "{q:?}");
            for (t, w) in tokens(q).iter().zip(Query::words(q)) {
                let mut again = tokens(&q[w.span.clone()]);
                assert_eq!(again.len(), 1, "{q:?}");
                assert_eq!(again.pop().unwrap(), *t);
            }
        }
    }

    #[test]
    fn help_lists_every_keyword() {
        for k in [
            "ext:", "dm:", "path:", "size:", "regex:", "case:", "exact:", "depth:", "folder:", "!", "|", ">1gb", "\"", "<a b>|c",
            "parent:", "child:", "childcount:", "childfilecount:", "dupe:", "sizedupe:", "attrib:", "len:", "audio:", "dc:", "da:",
        ] {
            assert!(HELP.contains(k), "{k}");
        }
        assert!(HELP.lines().all(|l| l.split('\t').count() == 2), "example, tab, description");
    }
}
