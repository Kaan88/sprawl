//! The search box language: [`Query::parse`] reads what was typed, [`crate::search::search`]
//! and [`crate::compact::CompactIndex::search`] find what matches. [`HELP`] lists it for the
//! app's empty state.
//!
//! A query is words separated by spaces; every word must match ("and"). A word is:
//!
//! * text, found anywhere in the name, any case: `report`; with `*` and `?` it is a pattern
//!   for the whole name: `*.iso`, `img_????.jpg`; with `\` (or `/`) it is matched against the
//!   full path, where folders end in `\`: `\steam\ *.vdf`;
//! * alternatives: `a|b` or `a;b`;
//! * `!word`: not;
//! * `>1gb`, `<10kb`, `>=5mb`, `<=1k`: size (1 kb = 1024 bytes; a folder's size is its total);
//! * `folder:` or `file:` in front: one kind only (`folder:cache`);
//! * modifiers in front of text, in any order: `case:` (match case), `exact:` or
//!   `wholename:` (the whole name, not part of it), `path:` (the full path even without `\`);
//! * keywords: `ext:jpg;png`, `dm:2026-11` (modified; see [`HELP`]), `size:100mb..2gb`,
//!   `depth:<=3`, `regex:^ab+c$`.
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
regex:^log\\d+\\.txt$\ta regular expression on the name (on the full path if it has \\\\)
case:README\tmatch upper and lower case exactly
exact:desktop.ini\tthe whole name, not part of it (also wholename:)
depth:<=2\tat most two levels below the scanned folder
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
            Pat::Size(..) | Pat::Date(..) | Pat::Depth(..) => 0,
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
        }
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
}

/// A word: any of its alternatives, or none of them with `!`.
#[derive(Debug, Clone)]
pub(crate) struct Term {
    not: bool,
    alts: Vec<Pat>,
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
pub(crate) fn unix_secs(t: SystemTime) -> i64 {
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
    errors: Vec<String>,
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

/// A character of the query and whether it was inside quotes.
type Ch = (char, bool);

fn tokens(q: &str) -> Vec<Vec<Ch>> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut quoted = false;
    for c in q.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push((c, quoted)),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
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
    Regex,
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
            ("regex:", Key::Regex),
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

impl Query {
    /// Read the search box. "Now" is the clock's (for `dm:today` and the like).
    pub fn parse(q: &str) -> Query {
        Query::parse_at(q, SystemTime::now())
    }

    /// [`Query::parse`] with `now` given, for words like `dm:today`.
    pub fn parse_at(q: &str, now: SystemTime) -> Query {
        let mut query = Query::default();
        let mut today: Option<Option<Day>> = None;
        for tok in tokens(q) {
            let mut t: &[Ch] = &tok;
            for (kw, kind) in [("folder:", Kind::Folders), ("file:", Kind::Files)] {
                if let Some(r) = strip_kw(t, kw) {
                    query.kind = kind;
                    t = r;
                    break;
                }
            }
            if t.is_empty() {
                continue;
            }
            // a bare size: `>1gb` (`>abc` is text)
            if matches!(t[0], ('>' | '<', false)) && t.iter().all(|c| !c.1) {
                if let Some((a, b)) = size_compare(&text(t)) {
                    query.min = Some(query.min.map_or(a, |m| m.max(a)));
                    query.max = Some(query.max.map_or(b, |m| m.min(b)));
                    continue;
                }
            }
            let not = t[0] == ('!', false);
            if not {
                t = &t[1..];
            }
            if t.is_empty() {
                continue;
            }
            let (first, _) = prefix(t);
            // a regex keeps its `|`
            let alts = if first.key == Key::Regex { vec![t] } else { split_alts(t) };
            let mut pats = Vec::new();
            for (k, alt) in alts.into_iter().enumerate() {
                let (own, rest) = prefix(alt);
                let p = if own.is_some() || k == 0 { own } else { first };
                let rest = if own.is_some() || k == 0 { rest } else { alt };
                let today = || *today.get_or_insert_with(|| local_day(unix_secs(now)));
                match build(p, rest, alt, today) {
                    Ok(Some(pat)) => pats.push(pat),
                    Ok(None) => {}
                    Err(e) => query.errors.push(e),
                }
            }
            if !pats.is_empty() {
                let needs = pats.iter().fold(0, |a, p| a | p.needs());
                let dated = pats.iter().all(|p| matches!(p, Pat::Date(..)));
                query.terms.push(Term { not, alts: pats, needs, dated });
            }
        }
        query
    }

    /// Nothing to look for: no words, no size, no kind, no mistakes.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.min.is_none() && self.max.is_none() && self.kind == Kind::Any && self.errors.is_empty()
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

    /// The words to check before any path is built, cheapest first (sizes, dates and depths
    /// before names), and the words that need the path.
    pub(crate) fn split_terms(&self) -> (Vec<&Term>, Vec<&Term>) {
        let (mut early, path): (Vec<&Term>, Vec<&Term>) = self.terms.iter().partition(|t| !t.on_path());
        early.sort_by_key(|t| t.needs != 0);
        (early, path)
    }

    /// Does an item's size pass the `>`/`<` words?
    #[inline]
    pub(crate) fn size_ok(&self, size: u64) -> bool {
        !(self.min.is_some_and(|m| size < m) || self.max.is_some_and(|m| size > m))
    }

    /// Where the first plain word is in `name` (byte range), to show it in bold.
    pub fn highlight(&self, name: &str) -> Option<(usize, usize)> {
        let lower = name.to_lowercase();
        if lower.len() != name.len() {
            return None;
        }
        self.terms.iter().filter(|t| !t.not).flat_map(|t| &t.alts).find_map(|p| match p {
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
/// messages.
fn build(p: Prefix, rest: &[Ch], whole: &[Ch], today: impl FnOnce() -> Option<Day>) -> Result<Option<Pat>, String> {
    let value = text(rest);
    if value.trim().is_empty() {
        return Ok(None);
    }
    let shown = text(whole);
    match p.key {
        Key::Ext => {
            let v = value.trim();
            let v = v.strip_prefix('.').unwrap_or(v).to_lowercase();
            if v.is_empty() {
                return Ok(None);
            }
            Ok(Some(if v.contains(['*', '?']) { Pat::ExtGlob(v.chars().collect()) } else { Pat::Ext(v.into_bytes()) }))
        }
        Key::Size => match size_value(&value) {
            Some((a, b)) => Ok(Some(Pat::Size(a, b))),
            None => Err(format!("{shown}: not a size (try size:>1gb, size:100mb..2gb or size:empty)")),
        },
        Key::Depth => match depth_value(&value) {
            Some((a, b)) => Ok(Some(Pat::Depth(a, b))),
            None => Err(format!("{shown}: not a depth (try depth:<=3, depth:2 or depth:1..4)")),
        },
        Key::Date => {
            let words = ["today", "yesterday", "lastweek", "lastmonth", "thismonth", "thisyear"];
            let needs_today = words.iter().any(|w| value.to_ascii_lowercase().contains(w));
            let today = if needs_today { today() } else { None };
            match date_value(&value, today) {
                Some((a, b)) => Ok(Some(Pat::Date(a, b))),
                None => Err(format!(
                    "{shown}: not a date (try dm:2026-01-15, dm:2026-11, dm:2026, dm:>2026-01-15, dm:2026-01..2026-03 or dm:lastweek)"
                )),
            }
        }
        Key::Regex => {
            let path = p.path || value.contains("\\\\");
            let re = RegexBuilder::new(&value).case_insensitive(!p.case).size_limit(1 << 22).dfa_size_limit(1 << 22).build();
            match re {
                Ok(re) => Ok(Some(Pat::Regex { re: Box::new(re), path, case: p.case })),
                Err(e) => {
                    let e = e.to_string();
                    let why = e.lines().rev().find_map(|l| l.trim().strip_prefix("error:")).unwrap_or(e.lines().next().unwrap_or("")).trim().to_string();
                    Err(format!("{shown}: not a regular expression ({why})"))
                }
            }
        }
        Key::Text => {
            // as before, an alternative's spaces at either end do not count (`"report "`)
            // `/` is a path separator too: names cannot hold it
            let v = value.trim().replace('/', "\\");
            let v = if p.case { v } else { v.to_lowercase() };
            let path = p.path || v.contains('\\');
            let glob = v.contains(['*', '?']);
            Ok(Some(match (p.case, path, glob, p.exact) {
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
            }))
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
    }

    /// Lower-case copies leaked for the test's lifetime (tests only).
    fn unreachable_lower(s: &str) -> &'static [u8] {
        Box::leak(s.to_lowercase().into_boxed_str()).as_bytes()
    }

    fn file(name: &str) -> T {
        T { name: name.into(), path: format!("C:\\Data\\{name}"), dir: false, size: 100, modified: None, depth: 1 }
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

    #[test]
    fn never_panics() {
        for q in [
            "", "!", "\"", "\"\"", "!\"", "|", ";;", "a||b", ":", "dm:", "dm:..", "dm:>", "dm:<=", "size:..", "size:>", "depth:..",
            "regex:", "regex:\\", "ext:.", "case:", "exact:", "path:", "case:exact:path:", ">", ">=", "<", "<=", "size:1e999",
            ">99999999999999999999999tb", "dm:0000", "dm:99999", "dm:2026-02-30", "dm:2026-1-1", "dm:-1", "depth:99999999999",
            "dm:today..yesterday", "folder:", "file:!", "!folder:", "😀 é *ü?", "dm:1601", "dm:9999-12-31", "\\", "/", "*", "?",
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
    fn help_lists_every_keyword() {
        for k in ["ext:", "dm:", "path:", "size:", "regex:", "case:", "exact:", "depth:", "folder:", "!", "|", ">1gb", "\""] {
            assert!(HELP.contains(k), "{k}");
        }
        assert!(HELP.lines().all(|l| l.split('\t').count() == 2), "example, tab, description");
    }
}
