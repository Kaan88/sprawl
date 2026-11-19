//! The Search box's helpers that need no window: the filter chips under the box, the
//! suggestions while a keyword is typed, the help list and the saved and recent searches.
//!
//! The box always holds the real query as text. A chip is lit from the words in the box
//! ([`sprawl_core::query::Query::words`]), never from a state of its own; picking an option
//! writes a word into the box, and taking a chip off removes its words from the box.

use std::ops::Range;
use std::path::PathBuf;

use sprawl_core::kinds;
use sprawl_core::query::{Kind, Query, Word, WordKey};

/// The dropdowns under the box, in their order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Chip {
    Kind,
    Type,
    Size,
    Modified,
    Where,
    More,
}

pub(crate) const CHIPS: [Chip; 6] = [Chip::Kind, Chip::Type, Chip::Size, Chip::Modified, Chip::Where, Chip::More];

/// What an option of a chip does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// The chip's words are replaced by this one.
    Term(String),
    /// The chip's words are replaced by this keyword, and the caret waits after it for a value.
    Prompt(&'static str),
    /// Ask for a folder, then `path:` it.
    Folder,
    /// `case:` or `exact:` in front of the last name word.
    Modifier(&'static str),
}

/// The Type options: a family of [`kinds`] and its name here.
pub(crate) const TYPES: [(u8, &str); 7] = [
    (kinds::IMAGES, "pictures"),
    (kinds::VIDEO, "video"),
    (kinds::AUDIO, "audio"),
    (kinds::DOCUMENTS, "documents"),
    (kinds::ARCHIVES, "archives"),
    (kinds::PROGRAMS, "programs"),
    (kinds::CODE, "code"),
];

const SIZES: [(&str, &str); 5] = [("size:empty", "empty"), ("size:<1mb", "under 1 MB"), ("size:1mb..100mb", "1 to 100 MB"), ("size:100mb..1gb", "100 MB to 1 GB"), ("size:>1gb", "over 1 GB")];

const DATES: [(&str, &str); 6] = [
    ("today", "today"),
    ("yesterday", "yesterday"),
    ("lastweek", "last 7 days"),
    ("lastmonth", "last month"),
    ("thismonth", "this month"),
    ("thisyear", "this year"),
];

/// The `ext:` word for a family.
pub(crate) fn type_term(f: u8) -> String {
    format!("ext:{}", kinds::extensions(f).join(";"))
}

impl Chip {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Chip::Kind => "Kind",
            Chip::Type => "Type",
            Chip::Size => "Size",
            Chip::Modified => "Modified",
            Chip::Where => "Where",
            Chip::More => "More",
        }
    }

    /// The options of its menu: what it says and what it does. `today` is "YYYY-MM-DD".
    pub(crate) fn options(self, today: &str) -> Vec<(String, Pick)> {
        let t = |s: &str| Pick::Term(s.to_string());
        match self {
            Chip::Kind => vec![("Files only".into(), t("file:")), ("Folders only".into(), t("folder:"))],
            Chip::Type => {
                let mut v: Vec<(String, Pick)> = TYPES.iter().map(|&(f, name)| (cap(name), Pick::Term(type_term(f)))).collect();
                v.push(("Other extensions...".into(), Pick::Prompt("ext:")));
                v
            }
            Chip::Size => {
                let mut v: Vec<(String, Pick)> = SIZES.iter().map(|&(term, name)| (cap(name), t(term))).collect();
                v.push(("Custom range...".into(), Pick::Prompt("size:")));
                v
            }
            Chip::Modified => vec![
                ("Today".into(), t("dm:today")),
                ("Last 7 days".into(), t("dm:lastweek")),
                ("Last month".into(), t("dm:lastmonth")),
                ("This year".into(), t("dm:thisyear")),
                ("A year or older".into(), Pick::Term(format!("dm:<{}", year_before(today)))),
                ("Custom dates...".into(), Pick::Prompt("dm:")),
            ],
            Chip::Where => vec![("Inside a folder...".into(), Pick::Folder), ("Path contains...".into(), Pick::Prompt("path:"))],
            Chip::More => vec![
                ("Compressed files and folders".into(), t("compressed:")),
                ("Whole name (exact:)".into(), Pick::Modifier("exact:")),
                ("Match case (case:)".into(), Pick::Modifier("case:")),
                ("Regular expression (regex:)".into(), Pick::Prompt("regex:")),
                ("At most 1 level deep".into(), t("depth:<=1")),
                ("At most 2 levels deep".into(), t("depth:<=2")),
                ("At most 3 levels deep".into(), t("depth:<=3")),
            ],
        }
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The `path:` word for "inside this folder": quoted, ending in `\` so the folder's own name
/// as a prefix of another folder's does not match (`C:\Data\` is not `C:\Database`).
pub(crate) fn folder_term(dir: &str) -> String {
    format!("path:\"{}\\\"", dir.trim_end_matches('\\'))
}

/// The same day a year earlier ("2025-11-17" for "2026-11-17"; 29 February becomes the 28th).
pub(crate) fn year_before(today: &str) -> String {
    let mut p = today.split('-').map(|x| x.parse::<u32>().unwrap_or(1));
    let (y, m, d) = (p.next().unwrap_or(1970), p.next().unwrap_or(1), p.next().unwrap_or(1));
    let d = if m == 2 && d == 29 { 28 } else { d };
    format!("{:04}-{:02}-{:02}", y.saturating_sub(1), m, d)
}

/// Today's date as "YYYY-MM-DD" (UTC: a day off around midnight does not matter here).
pub(crate) fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The chips a word lights. A word with `!` lights none (but its `folder:` still does).
pub(crate) fn chips_of(w: &Word) -> Vec<Chip> {
    let mut v = Vec::new();
    if w.kind != Kind::Any {
        v.push(Chip::Kind);
    }
    if w.not {
        return v;
    }
    match w.key {
        WordKey::Ext => v.push(Chip::Type),
        WordKey::Size | WordKey::Compare => v.push(Chip::Size),
        WordKey::Date => v.push(Chip::Modified),
        WordKey::Depth | WordKey::Regex | WordKey::Compressed => v.push(Chip::More),
        WordKey::Text => {
            if w.path {
                v.push(Chip::Where);
            }
            if w.case || w.exact {
                v.push(Chip::More);
            }
        }
        WordKey::None => {}
    }
    v
}

/// What each chip shows when its words are in the box (None: not lit), in [`CHIPS`] order.
pub(crate) fn lit(text: &str) -> [Option<String>; 6] {
    let words = Query::words(text);
    let mut out: [Option<String>; 6] = Default::default();
    let on = |c: Chip| words.iter().filter(move |w| chips_of(w).contains(&c));
    // the last kind word wins, as in the query
    if let Some(w) = on(Chip::Kind).last() {
        out[0] = Some(if w.kind == Kind::Files { "files".into() } else { "folders".into() });
    }
    let types: Vec<String> = on(Chip::Type).map(|w| type_label(&w.value)).collect();
    if !types.is_empty() {
        out[1] = Some(types.join(", "));
    }
    let sizes: Vec<String> = on(Chip::Size)
        .map(|w| {
            let term = if w.key == WordKey::Size { format!("size:{}", w.value) } else { w.value.clone() };
            SIZES.iter().find(|s| s.0.eq_ignore_ascii_case(&term)).map_or(w.value.clone(), |s| s.1.to_string())
        })
        .collect();
    if !sizes.is_empty() {
        out[2] = Some(sizes.join(" "));
    }
    let dates: Vec<String> = on(Chip::Modified).map(|w| date_label(&w.value)).collect();
    if !dates.is_empty() {
        out[3] = Some(dates.join(", "));
    }
    let wheres: Vec<String> = on(Chip::Where).map(|w| w.value.clone()).collect();
    if !wheres.is_empty() {
        out[4] = Some(wheres.join(", "));
    }
    let mut more: Vec<String> = Vec::new();
    for w in on(Chip::More) {
        let mut add = |s: String| {
            if !more.contains(&s) {
                more.push(s);
            }
        };
        match w.key {
            WordKey::Depth => add(format!("depth {}", w.value)),
            WordKey::Regex => add("regex".into()),
            WordKey::Compressed => add(if w.value.is_empty() { "compressed".into() } else { format!("compressed {}", w.value) }),
            _ => {
                if w.exact {
                    add("whole name".into());
                }
                if w.case {
                    add("match case".into());
                }
            }
        }
    }
    if !more.is_empty() {
        out[5] = Some(more.join(", "));
    }
    out
}

/// "pictures" for exactly the pictures' extensions, else ".jpg .png".
fn type_label(value: &str) -> String {
    let mut have: Vec<String> = value.split([';', '|']).map(|e| e.trim().trim_start_matches('.').to_lowercase()).filter(|e| !e.is_empty()).collect();
    have.sort();
    have.dedup();
    for &(f, name) in &TYPES {
        let mut want: Vec<&str> = kinds::extensions(f).to_vec();
        want.sort_unstable();
        if want.len() == have.len() && want.iter().zip(&have).all(|(a, b)| *a == b) {
            return name.to_string();
        }
    }
    let shown: Vec<String> = have.iter().take(4).map(|e| format!(".{e}")).collect();
    let more = if have.len() > 4 { format!(" +{}", have.len() - 4) } else { String::new() };
    format!("{}{more}", shown.join(" "))
}

fn date_label(value: &str) -> String {
    let v = value.trim();
    if let Some(d) = DATES.iter().find(|d| d.0.eq_ignore_ascii_case(v)) {
        return d.1.to_string();
    }
    if let Some(r) = v.strip_prefix("<=").or_else(|| v.strip_prefix('<')) {
        return format!("before {r}");
    }
    if let Some(r) = v.strip_prefix(">=").or_else(|| v.strip_prefix('>')) {
        return format!("after {r}");
    }
    v.to_string()
}

/// One change to the text: replace `range` with `with`.
type Edit = (Range<usize>, String);

/// `text` with `edits` (not overlapping) made, and where `at` (an offset in the old text) is now.
fn apply(text: &str, mut edits: Vec<Edit>, at: usize) -> (String, usize) {
    edits.sort_by_key(|e| std::cmp::Reverse(e.0.start));
    let mut s = text.to_string();
    let mut at = at;
    for (r, w) in edits {
        if at >= r.end {
            at = at + w.len() - (r.end - r.start);
        } else if at > r.start {
            at = r.start + w.len();
        }
        s.replace_range(r, &w);
    }
    (s, at)
}

/// The edit that takes word `span` out, with one space next to it.
fn drop_word(text: &str, span: &Range<usize>) -> Edit {
    let after = text[span.end..].chars().next().filter(|c| c.is_whitespace()).map_or(0, char::len_utf8);
    if after > 0 {
        return (span.start..span.end + after, String::new());
    }
    let before = text[..span.start].chars().next_back().filter(|c| c.is_whitespace()).map_or(0, char::len_utf8);
    (span.start - before..span.end, String::new())
}

/// The edits that take `chip`'s part of word `w` out.
fn chip_edit(text: &str, w: &Word, chip: Chip) -> Option<Edit> {
    if !chips_of(w).contains(&chip) {
        return None;
    }
    // the whole word but its `folder:`/`file:`
    let rest = || if w.kind != Kind::Any { (w.kind_end..w.span.end, String::new()) } else { drop_word(text, &w.span) };
    Some(match chip {
        Chip::Kind if w.kind_end >= w.span.end => drop_word(text, &w.span),
        Chip::Kind => (w.span.start..w.kind_end, String::new()),
        Chip::More if w.key == WordKey::Text => (w.prefix_end..w.value_at, if w.path { "path:".into() } else { String::new() }),
        _ => rest(),
    })
}

/// `text` without `chip`'s words.
pub(crate) fn remove(text: &str, chip: Chip) -> String {
    let words = Query::words(text);
    let edits: Vec<Edit> = words.iter().filter_map(|w| chip_edit(text, w, chip)).collect();
    let (s, _) = apply(text, edits, 0);
    s.trim().to_string()
}

/// `text` with `term` added at the end; the caret goes after it.
pub(crate) fn append(text: &str, term: &str) -> (String, usize) {
    let base = text.trim_end();
    let s = if base.is_empty() { term.to_string() } else { format!("{base} {term}") };
    let at = s.len();
    (s, at)
}

/// Picking `term` on `chip`: its words go, `term` is added. For More, only words of the same
/// sort go (a depth replaces a depth; a regex a regex).
pub(crate) fn put(text: &str, chip: Chip, term: &str) -> (String, usize) {
    let base = if chip == Chip::More {
        let key = if term.starts_with("depth:") { WordKey::Depth } else if term.starts_with("compressed:") { WordKey::Compressed } else { WordKey::Regex };
        let words = Query::words(text);
        let edits: Vec<Edit> = words
            .iter()
            .filter(|w| !w.not && w.key == key)
            .map(|w| if w.kind != Kind::Any { (w.kind_end..w.span.end, String::new()) } else { drop_word(text, &w.span) })
            .collect();
        apply(text, edits, 0).0.trim().to_string()
    } else {
        remove(text, chip)
    };
    append(&base, term)
}

/// `case:` or `exact:` in front of the last name word (after its other modifiers), or on its
/// own at the end when there is none; the caret goes after the word.
pub(crate) fn add_modifier(text: &str, m: &str) -> (String, usize) {
    let words = Query::words(text);
    let Some(w) = words.iter().rev().find(|w| w.key == WordKey::Text && !w.path && !w.value.is_empty()) else {
        return append(text, m);
    };
    let has = if m == "case:" { w.case } else { w.exact };
    if has {
        return (text.to_string(), w.span.end);
    }
    apply(text, vec![(w.prefix_end..w.prefix_end, m.to_string())], w.span.end)
}

// ------------------------------------------------------------ suggestions

/// The list under the box while a keyword is typed: what to put in place of `span`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Suggestions {
    pub(crate) span: Range<usize>,
    /// (shown, the word that replaces `span`)
    pub(crate) items: Vec<(String, String)>,
}

const KEYWORDS: [&str; 11] = ["ext:", "dm:", "size:", "depth:", "compressed:", "regex:", "path:", "case:", "exact:", "folder:", "file:"];
const MODIFIERS: [&str; 7] = ["folder:", "file:", "case:", "exact:", "wholename:", "path:", "!"];
const MAX_SUGGESTIONS: usize = 8;
/// After `ext:` the seven families come first, then the scan's own types.
const MAX_EXT_SUGGESTIONS: usize = 12;

/// The suggestions for the word that ends at `caret` (a byte offset), if it is a keyword
/// being typed. `exts` are extensions to offer after `ext:` (the scan's, biggest first).
pub(crate) fn suggest(text: &str, caret: usize, exts: &[String]) -> Option<Suggestions> {
    let caret = caret.min(text.len());
    if !text.is_char_boundary(caret) {
        return None;
    }
    let before = &text[..caret];
    // inside quotes: plain text
    if before.matches('"').count() % 2 == 1 {
        return None;
    }
    // only at the end of a word
    if text[caret..].chars().next().is_some_and(|c| !c.is_whitespace()) {
        return None;
    }
    let start = before.rfind(char::is_whitespace).map_or(0, |i| i + before[i..].chars().next().map_or(1, char::len_utf8));
    let word = &text[start..caret];
    if word.is_empty() {
        return None;
    }
    // the modifiers in front stay as they are
    let mut rest = word;
    'strip: loop {
        for m in MODIFIERS {
            if rest.len() > m.len() && rest.get(..m.len()).is_some_and(|p| p.eq_ignore_ascii_case(m)) {
                rest = &rest[m.len()..];
                continue 'strip;
            }
        }
        break;
    }
    let lead = &word[..word.len() - rest.len()];
    let mut items: Vec<(String, String)> = Vec::new();
    match rest.split_once(':') {
        Some((kw, value)) => {
            let kw_l = kw.to_ascii_lowercase();
            let head = format!("{lead}{kw}:");
            let v = value.to_ascii_lowercase();
            let mut offer = |shown: String, val: &str| {
                if val.to_ascii_lowercase().starts_with(&v) && !val.eq_ignore_ascii_case(value) && items.len() < MAX_SUGGESTIONS {
                    items.push((shown, format!("{head}{val}")));
                }
            };
            match kw_l.as_str() {
                "dm" | "datemodified" => {
                    for (d, name) in DATES {
                        offer(format!("{kw}:{d}   {name}"), d);
                    }
                }
                "size" => {
                    for (term, name) in SIZES {
                        let val = &term["size:".len()..];
                        offer(format!("{kw}:{val}   {name}"), val);
                    }
                    for val in [">100mb", ">10gb", "<10kb"] {
                        offer(format!("{kw}:{val}"), val);
                    }
                }
                "depth" => {
                    for (val, name) in [("<=1", "in the scanned folder"), ("<=2", "at most 2 levels down"), ("<=3", "at most 3 levels down")] {
                        offer(format!("{kw}:{val}   {name}"), val);
                    }
                }
                "ext" => {
                    // after `ext:jpg;` the next one
                    let cut = value.rfind([';', '|']).map_or(0, |i| i + 1);
                    let (done, part) = value.split_at(cut);
                    let p = part.to_ascii_lowercase();
                    let mut offer = |shown: String, val: String| {
                        if val.starts_with(&p) && val != p && items.len() < MAX_EXT_SUGGESTIONS && !items.iter().any(|i| i.1 == format!("{head}{done}{val}")) {
                            items.push((shown, format!("{head}{done}{val}")));
                        }
                    };
                    if done.is_empty() {
                        for &(f, name) in &TYPES {
                            if name.starts_with(&p) {
                                let all = kinds::extensions(f).join(";");
                                offer(format!("{kw}:{name}   ({})", kinds::extensions(f).iter().take(4).copied().collect::<Vec<_>>().join(" ")), all);
                            }
                        }
                    }
                    for e in exts.iter().filter(|e| !e.is_empty() && *e != "(none)") {
                        offer(format!("{kw}:{done}{e}"), e.to_ascii_lowercase());
                    }
                }
                _ => {}
            }
        }
        None => {
            let r = rest.to_ascii_lowercase();
            if r.len() >= 2 && r.bytes().all(|b| b.is_ascii_alphabetic()) {
                for k in KEYWORDS {
                    if k.starts_with(&r) && items.len() < MAX_SUGGESTIONS {
                        items.push((k.to_string(), format!("{lead}{k}")));
                    }
                }
            }
        }
    }
    (!items.is_empty()).then_some(Suggestions { span: start..caret, items })
}

/// Take suggestion `i`: the new text and the caret after it.
pub(crate) fn take(text: &str, s: &Suggestions, i: usize) -> Option<(String, usize)> {
    let (_, word) = s.items.get(i)?;
    let span = s.span.start.min(text.len())..s.span.end.min(text.len());
    if !text.is_char_boundary(span.start) || !text.is_char_boundary(span.end) {
        return None;
    }
    Some(apply(text, vec![(span.clone(), word.clone())], span.end))
}

/// Where the highlight goes for a key (38 up, 40 down) with `n` suggestions. None: nothing is
/// highlighted. Down with nothing highlighted is not the list's (it goes to the results).
pub(crate) fn move_highlight(cur: Option<usize>, key: i32, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    match (key, cur) {
        (40, Some(i)) => Some((i + 1).min(n - 1)),
        (38, Some(0)) => None,
        (38, Some(i)) => Some(i - 1),
        (38, None) => Some(n - 1),
        (_, c) => c,
    }
}

/// What a key in the box does while suggestions may be showing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BoxKey {
    /// Move the highlight to this.
    Highlight(Option<usize>),
    /// Take this suggestion.
    Take(usize),
    /// Close the list.
    Close,
    /// To the results, as without a list.
    Results,
    /// Not ours: the box keeps it.
    Pass,
}

/// `open`: how many suggestions show (0 for none); `cur`: the highlighted one.
pub(crate) fn box_key(key: i32, open: usize, cur: Option<usize>) -> BoxKey {
    let cur = cur.filter(|&i| i < open);
    match key {
        13 => match cur {
            Some(i) => BoxKey::Take(i),
            None => BoxKey::Results,
        },
        40 if open > 0 && cur.is_some() => BoxKey::Highlight(move_highlight(cur, 40, open)),
        40 => BoxKey::Results,
        38 if open > 0 => BoxKey::Highlight(move_highlight(cur, 38, open)),
        9 if open > 0 => BoxKey::Take(cur.unwrap_or(0)),
        27 if open > 0 => BoxKey::Close,
        _ => BoxKey::Pass,
    }
}

/// A byte offset in `s` from a caret position in UTF-16 units (what the text box counts).
pub(crate) fn byte_at(s: &str, utf16: usize) -> usize {
    let mut n = 0;
    for (i, c) in s.char_indices() {
        if n >= utf16 {
            return i;
        }
        n += c.len_utf16();
    }
    s.len()
}

/// The text box's position (UTF-16 units) of byte offset `b`.
pub(crate) fn utf16_at(s: &str, b: usize) -> usize {
    s[..b.min(s.len())].chars().map(char::len_utf16).sum()
}

// ------------------------------------------------------------ help

/// The help list: the examples of a line (each can be put in the box) and what it does.
pub(crate) fn help() -> Vec<(Vec<&'static str>, &'static str)> {
    sprawl_core::query::HELP
        .lines()
        .filter_map(|l| {
            let (ex, what) = l.split_once('\t')?;
            Some((ex.split("  ").map(str::trim).filter(|e| !e.is_empty()).collect(), what))
        })
        .collect()
}

// ------------------------------------------------------------ saved and recent searches

/// Recent searches kept.
pub(crate) const MAX_RECENT: usize = 10;

/// The searches the user pinned and the last ones run, newest first. Kept as text in the
/// settings folder, one per line: `pin<TAB>query` or `recent<TAB>query`.
#[derive(Default, Debug, PartialEq, Eq, Clone)]
pub(crate) struct History {
    pub(crate) pinned: Vec<String>,
    pub(crate) recent: Vec<String>,
}

fn clean(q: &str) -> String {
    q.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim().to_string()
}

impl History {
    pub(crate) fn parse(text: &str) -> History {
        let mut h = History::default();
        for line in text.trim_start_matches('\u{feff}').lines() {
            let Some((k, q)) = line.split_once('\t') else { continue };
            let q = clean(q);
            if q.is_empty() {
                continue;
            }
            match k {
                "pin" if !h.pinned.contains(&q) => h.pinned.push(q),
                "recent" if !h.recent.contains(&q) && h.recent.len() < MAX_RECENT => h.recent.push(q),
                _ => {}
            }
        }
        h
    }

    pub(crate) fn to_text(&self) -> String {
        let mut s = String::new();
        for q in &self.pinned {
            s += &format!("pin\t{q}\r\n");
        }
        for q in &self.recent {
            s += &format!("recent\t{q}\r\n");
        }
        s
    }

    /// A search was used: it goes first among the recent ones (a pinned one stays only pinned).
    pub(crate) fn note(&mut self, q: &str) -> bool {
        let q = clean(q);
        if q.is_empty() || self.pinned.contains(&q) || self.recent.first() == Some(&q) {
            return false;
        }
        self.recent.retain(|r| *r != q);
        self.recent.insert(0, q);
        self.recent.truncate(MAX_RECENT);
        true
    }

    pub(crate) fn pin(&mut self, q: &str) {
        let q = clean(q);
        if q.is_empty() || self.pinned.contains(&q) {
            return;
        }
        self.recent.retain(|r| *r != q);
        self.pinned.push(q);
    }

    pub(crate) fn unpin(&mut self, q: &str) {
        let q = clean(q);
        self.pinned.retain(|p| *p != q);
    }

    pub(crate) fn is_pinned(&self, q: &str) -> bool {
        self.pinned.contains(&clean(q))
    }
}

/// `%APPDATA%\Sprawl\searches.txt`, next to `settings.ini`.
pub(crate) fn history_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Sprawl").join("searches.txt"))
}

pub(crate) fn history_load() -> History {
    history_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| History::parse(&t)).unwrap_or_default()
}

/// Write it (nothing in a screenshot run); an empty history removes the file.
pub(crate) fn history_save(h: &History) {
    if crate::settings::READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let Some(p) = history_path() else { return };
    if h.pinned.is_empty() && h.recent.is_empty() {
        // through its handle, never by name: a link there is left alone
        let _ = sprawl_advisor::remove_local(&p);
        return;
    }
    let _ = sprawl_advisor::write_local(&p, h.to_text().as_bytes(), true);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Search's Type options are the views' families (sprawl_core::kinds), each once.
    #[test]
    fn types_are_the_views_families() {
        let mut fams: Vec<u8> = TYPES.iter().map(|t| t.0).collect();
        fams.sort_unstable();
        let mut views: Vec<u8> = sprawl_core::kinds::FAMILIES.iter().map(|f| f.0).collect();
        views.sort_unstable();
        assert_eq!(fams, views);
    }

    fn lit_at(text: &str, c: Chip) -> Option<String> {
        lit(text)[CHIPS.iter().position(|&x| x == c).unwrap()].clone()
    }

    #[test]
    fn a_picked_option_lights_its_chip_and_typing_it_does_too() {
        let today = "2026-11-17";
        for chip in CHIPS {
            for (label, pick) in chip.options(today) {
                let term = match pick {
                    Pick::Term(t) => t,
                    Pick::Prompt(_) | Pick::Folder => continue,
                    Pick::Modifier(m) => format!("{m}report"),
                };
                // picked: written into the box, which lights the chip
                let (text, at) = put("report", chip, &term);
                assert!(text.contains(&term), "{label}: {text}");
                assert_eq!(at, text.len());
                assert!(lit_at(&text, chip).is_some(), "{label}: {text}");
                assert!(Query::parse(&text).errors().is_empty(), "{label}: {text}");
                // typed by hand: the same
                assert_eq!(lit(&format!("report {term}")), lit(&text), "{label}");
                // taken off: the chip goes dark, the rest stays
                let off = remove(&text, chip);
                assert!(lit_at(&off, chip).is_none(), "{label}: {off}");
                assert!(off.contains("report"), "{label}: {off}");
            }
        }
    }

    #[test]
    fn chips_name_what_is_picked() {
        assert_eq!(lit_at(&type_term(kinds::IMAGES), Chip::Type).as_deref(), Some("pictures"));
        assert_eq!(lit_at("ext:jpg;png", Chip::Type).as_deref(), Some(".jpg .png"));
        assert_eq!(lit_at("size:>1gb", Chip::Size).as_deref(), Some("over 1 GB"));
        assert_eq!(lit_at(">1mb <10mb", Chip::Size).as_deref(), Some(">1mb <10mb"));
        assert_eq!(lit_at("dm:lastweek", Chip::Modified).as_deref(), Some("last 7 days"));
        assert_eq!(lit_at("dm:<2025-11-17", Chip::Modified).as_deref(), Some("before 2025-11-17"));
        assert_eq!(lit_at("folder:cache", Chip::Kind).as_deref(), Some("folders"));
        assert_eq!(lit_at("file: folder:", Chip::Kind).as_deref(), Some("folders"), "the last one counts, as in the query");
        assert_eq!(lit_at("path:\"C:\\My Dir\\\"", Chip::Where).as_deref(), Some("C:\\My Dir\\"));
        assert_eq!(lit_at("case:exact:README depth:<=2", Chip::More).as_deref(), Some("whole name, match case, depth <=2"));
        // a negated word is not a filter the chip can show
        assert_eq!(lit_at("!ext:tmp", Chip::Type), None);
        assert_eq!(lit_at("\"ext:tmp\"", Chip::Type), None, "quoted: text");
        assert_eq!(lit(""), <[Option<String>; 6]>::default());
    }

    #[test]
    fn picking_again_replaces_and_removing_keeps_the_rest() {
        let (t, _) = put("a ext:jpg b", Chip::Type, "ext:png");
        assert_eq!(t, "a b ext:png");
        let (t, _) = put("size:>1gb a >1mb", Chip::Size, "size:empty");
        assert_eq!(t, "a size:empty");
        assert_eq!(remove("folder:cache x", Chip::Kind), "cache x", "the word stays, its kind goes");
        assert_eq!(remove("folder: x", Chip::Kind), "x");
        assert_eq!(remove("folder:ext:jpg x", Chip::Type), "folder: x", "its kind stays");
        assert_eq!(remove("case:exact:path:steam x", Chip::More), "path:steam x");
        assert_eq!(remove("case:\"my file\"", Chip::More), "\"my file\"");
        assert_eq!(remove("path:steam x", Chip::Where), "x");
        assert_eq!(remove("x !ext:tmp", Chip::Type), "x !ext:tmp", "a negated word is not the chip's");
        assert_eq!(remove("\"a  b\" ext:jpg", Chip::Type), "\"a  b\"", "spaces in quotes stay");
        // More: a depth replaces a depth, the rest of More stays
        assert_eq!(put("exact:a depth:<=1", Chip::More, "depth:<=3").0, "exact:a depth:<=3");
        // modifiers go on the last name word
        assert_eq!(add_modifier("report ext:pdf", "exact:"), ("exact:report ext:pdf".to_string(), "exact:report".len()));
        assert_eq!(add_modifier("!tmp", "case:").0, "!case:tmp");
        assert_eq!(add_modifier("ext:pdf", "case:").0, "ext:pdf case:");
        assert_eq!(add_modifier("case:x", "case:").0, "case:x");
        // inside a folder: quoted, with its \
        let (t, _) = put("x", Chip::Where, &folder_term("C:\\My Data\\"));
        assert_eq!(t, "x path:\"C:\\My Data\\\"");
        assert_eq!(lit_at(&t, Chip::Where).as_deref(), Some("C:\\My Data\\"));
        assert!(Query::parse(&t).errors().is_empty());
        assert_eq!(folder_term("D:\\"), "path:\"D:\\\"");
        assert_eq!(year_before("2028-02-29"), "2027-02-28");
        assert_eq!(today().len(), 10);
    }

    #[test]
    fn suggestions_for_keywords() {
        let exts = vec!["jpg".to_string(), "json".to_string(), "(none)".to_string()];
        let s = suggest("report dm:", 10, &exts).unwrap();
        assert_eq!(s.span, 7..10);
        assert_eq!(s.items.len(), 6);
        assert_eq!(take("report dm:", &s, 2), Some(("report dm:lastweek".to_string(), 18)));
        // typing on filters
        let s = suggest("dm:last", 7, &exts).unwrap();
        assert_eq!(s.items.iter().map(|i| &i.1[..]).collect::<Vec<_>>(), ["dm:lastweek", "dm:lastmonth"]);
        // a finished value offers nothing
        assert_eq!(suggest("dm:today", 8, &exts), None);
        // ext: families first, then the scan's types; after ; the next one
        let s = suggest("ext:", 4, &exts).unwrap();
        assert!(s.items[0].1.starts_with("ext:jpg;jpeg;png"));
        assert!(s.items.iter().any(|i| i.1 == "ext:json"));
        assert!(!s.items.iter().any(|i| i.1.contains("none")));
        let s = suggest("ext:png;j", 9, &exts).unwrap();
        assert_eq!(s.items.iter().map(|i| &i.1[..]).collect::<Vec<_>>(), ["ext:png;jpg", "ext:png;json"]);
        // modifiers in front stay
        let s = suggest("!folder:si", 10, &exts).unwrap();
        assert_eq!(s.items, [("size:".to_string(), "!folder:size:".to_string())]);
        // the start of a keyword offers it; a plain word or one letter offers nothing
        assert_eq!(suggest("ex", 2, &exts).unwrap().items[0].1, "ext:");
        assert_eq!(suggest("e", 1, &exts), None);
        assert_eq!(suggest("report", 6, &exts), None);
        assert_eq!(suggest("", 0, &exts), None);
        // inside quotes, in the middle of a word, or after a space: nothing
        assert_eq!(suggest("\"dm:", 4, &exts), None);
        assert_eq!(suggest("dm:x", 2, &exts), None);
        assert_eq!(suggest("dm: ", 4, &exts), None);
        assert!(suggest("a size:", 7, &exts).is_some());
        // a caret off a character's edge is ignored
        assert_eq!(suggest("é", 1, &exts), None);
    }

    #[test]
    fn keys_in_the_box_never_take_plain_typing() {
        use BoxKey::*;
        // no list: Enter and Down go to the results, the rest is the box's
        assert_eq!(box_key(13, 0, None), Results);
        assert_eq!(box_key(40, 0, None), Results);
        assert_eq!(box_key(38, 0, None), Pass);
        assert_eq!(box_key(9, 0, None), Pass);
        assert_eq!(box_key(27, 0, None), Pass);
        assert_eq!(box_key(65, 3, None), Pass);
        // a list, nothing highlighted: Enter and Down still go to the results
        assert_eq!(box_key(13, 3, None), Results);
        assert_eq!(box_key(40, 3, None), Results);
        // Up comes into the list from below; Tab takes the first
        assert_eq!(box_key(38, 3, None), Highlight(Some(2)));
        assert_eq!(box_key(9, 3, None), Take(0));
        assert_eq!(box_key(27, 3, None), Close);
        // highlighted: Up and Down move, Enter and Tab take it
        assert_eq!(box_key(40, 3, Some(0)), Highlight(Some(1)));
        assert_eq!(box_key(40, 3, Some(2)), Highlight(Some(2)));
        assert_eq!(box_key(38, 3, Some(0)), Highlight(None));
        assert_eq!(box_key(13, 3, Some(1)), Take(1));
        assert_eq!(box_key(9, 3, Some(1)), Take(1));
        // a highlight left over from a longer list is not used
        assert_eq!(box_key(13, 2, Some(5)), Results);
    }

    #[test]
    fn caret_offsets() {
        let s = "a😀é b";
        assert_eq!(byte_at(s, 0), 0);
        assert_eq!(byte_at(s, 1), 1);
        assert_eq!(byte_at(s, 3), 5, "the emoji is two UTF-16 units");
        assert_eq!(byte_at(s, 99), s.len());
        for b in [0, 1, 5, 7, s.len()] {
            assert_eq!(byte_at(s, utf16_at(s, b)), b);
        }
    }

    #[test]
    fn help_has_examples_that_parse() {
        let h = help();
        assert!(h.len() > 10);
        for (ex, what) in &h {
            assert!(!ex.is_empty() && !what.is_empty());
            for e in ex {
                assert!(Query::parse(e).errors().is_empty(), "{e}");
            }
        }
        assert!(h.iter().any(|(ex, _)| ex == &["folder:", "file:"]));
        assert!(h.iter().any(|(ex, _)| ex == &["\\steam\\ *.vdf"]), "one space stays inside an example");
    }

    #[test]
    fn history_keeps_ten_and_the_pins() {
        let mut h = History::default();
        for i in 0..15 {
            assert!(h.note(&format!("q{i}")));
        }
        assert_eq!(h.recent.len(), MAX_RECENT);
        assert_eq!(h.recent[0], "q14");
        assert!(!h.note("q14"), "already first");
        h.note("q10");
        assert_eq!(h.recent[0], "q10");
        assert_eq!(h.recent.iter().filter(|r| *r == "q10").count(), 1);
        h.pin("q10");
        assert!(h.is_pinned("q10") && !h.recent.contains(&"q10".to_string()));
        assert!(!h.note("q10"), "a pinned search is not also recent");
        assert!(!h.note("   "));
        h.note("a\tb\nc");
        assert_eq!(h.recent[0], "a b c", "one line each");
        let back = History::parse(&h.to_text());
        assert_eq!(back, h);
        h.unpin("q10");
        assert!(h.pinned.is_empty());
        assert_eq!(History::parse("junk\nrecent\t\npin\tx\npin\tx\n"), History { pinned: vec!["x".into()], recent: vec![] });
    }
}
