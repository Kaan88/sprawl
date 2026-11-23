//! Editing inside a browser database (F10): Firefox's browsing history and form history, and
//! Chromium's autofill entries. Reached only from `clean::run`, after the part's own checks
//! (this account's profile, the browser closed, nothing else has the files open).
//!
//! In order, and as far as anything goes, the rest is left exactly as it was:
//! 1. the database and its side files are opened as themselves, through handles: plain local
//!    files, one name only (a hard link is refused), where their path says, not changed since
//!    the scan. The database stays held while it is edited, so nothing can rename, replace or
//!    remove it meanwhile; its side files are held while they are copied, then let go.
//! 2. a copy of each goes into Sprawl's backup folder, made through folders Sprawl opens and
//!    pins itself (never through a link), and the copy is read back and compared with the
//!    original byte for byte. If any of that fails, nothing is edited.
//! 3. the database is opened read-write and never created, and its format is checked first:
//!    `PRAGMA integrity_check` answers ok, the tables and columns Sprawl uses are there, and for
//!    Firefox the schema version is one Sprawl was written for.
//! 4. every delete for that database goes in one transaction; any failure rolls it back. No
//!    VACUUM (the browser does its own), so the file may not get smaller.
//!
//! Reading for the review ([`rows`]) opens the database read-only and immutable: even a
//! read-only SQLite open of a WAL database makes a `-shm` and a `-wal` beside it, and Sprawl
//! leaves nothing in a browser's profile.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::clean::{Outcome, Spare};
use crate::privacy::{Edit, Places, PrivacyPart, SIDE_FILES};

/// Firefox's `places.sqlite` schema versions this code was written for: 52, which Firefox 62
/// wrote and the oldest Firefox itself still opens, through 89, the current main branch in 2026
/// (`nsINavHistoryService::DATABASE_SCHEMA_VERSION`). Anything else is refused untouched.
pub const PLACES_VERSIONS: std::ops::RangeInclusive<i64> = 52..=89;
/// `formhistory.sqlite`: 5 since Firefox 57; Firefox itself refuses anything below 3.
pub const FORMS_VERSIONS: std::ops::RangeInclusive<i64> = 3..=5;

/// How many copies of one part's database are kept; the oldest goes as a new one is made.
pub const KEEP_BACKUPS: usize = 5;

/// `%LOCALAPPDATA%\Sprawl\backups\privacy`: where a database is copied before Sprawl edits it.
/// Nothing else of Sprawl's lives here; the copy is kept for the user to put back by hand.
pub fn backup_root() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Sprawl").join("backups").join("privacy"))
}

/// What "isn't one Sprawl knows how to edit" says, with the browser's name.
fn unknown(browser: &str) -> String {
    format!("this version of {browser} isn't one Sprawl knows how to edit")
}

/// How many rows `kind` would take out of `db` now, read-only: None when the database can't be
/// read or isn't a format Sprawl knows.
///
/// The read is `immutable`: even a read-only SQLite open of a database in WAL mode makes a
/// `-shm` and a `-wal` beside it, and Sprawl must leave nothing in a browser's profile (those
/// would also count as new files when the edit is checked). A browser that closed cleanly has
/// checkpointed its log, so the file itself is what the edit will see.
pub(crate) fn rows(kind: Edit, db: &Path) -> Option<u64> {
    let conn = open_read(db).ok()?;
    conn.execute_batch("PRAGMA query_only = 1").ok()?;
    let schema = Schema::check(&conn, kind, "").ok()?;
    schema.count(&conn, kind).ok()
}

fn open_read(db: &Path) -> rusqlite::Result<Connection> {
    // a URI, so SQLite reads the main file and leaves the browser's `-shm` and `-wal` alone
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
    Connection::open_with_flags(uri(db), flags)
}

/// `file:C:/.../places.sqlite?immutable=1`.
fn uri(db: &Path) -> String {
    let mut s = String::from("file:");
    for b in db.to_string_lossy().replace('\\', "/").bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => s.push(b as char),
            _ => s.push_str(&format!("%{b:02X}")),
        }
    }
    s.push_str("?immutable=1");
    s
}

/// The tables and columns a database of one kind must have, and the optional ones the code can
/// use when they are there.
struct Schema {
    has_annotations: bool,
    has_origins: bool,
    has_deleted_forms: bool,
}

fn columns(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(1)).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn has_table(conn: &Connection, table: &str) -> Result<bool, String> {
    conn.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [table], |_| Ok(true))
        .optional()
        .map(|o| o.is_some())
        .map_err(|e| e.to_string())
}

impl Schema {
    /// The format Sprawl expects, or the message "this version of <browser> ..." (`browser` is
    /// empty while measuring for the review, when no message is shown).
    fn check(conn: &Connection, kind: Edit, browser: &str) -> Result<Schema, String> {
        let wrong = || unknown(browser);
        let need = |table: &str, cols: &[&str]| -> Result<(), String> {
            let got = columns(conn, table).map_err(|_| wrong())?;
            if got.is_empty() || cols.iter().any(|c| !got.iter().any(|g| g.eq_ignore_ascii_case(c))) {
                return Err(wrong());
            }
            Ok(())
        };
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0);
        match kind {
            Edit::FirefoxHistory => {
                if !PLACES_VERSIONS.contains(&version) {
                    return Err(wrong());
                }
                need("moz_places", &["id", "visit_count", "last_visit_date"])?;
                need("moz_historyvisits", &["id", "place_id", "visit_date"])?;
                need("moz_bookmarks", &["id", "fk"])?;
                let places = columns(conn, "moz_places").map_err(|e| e.to_string())?;
                let has_origin_id = places.iter().any(|c| c.eq_ignore_ascii_case("origin_id"));
                let has_origins = has_table(conn, "moz_origins")?;
                if has_origins && has_origin_id {
                    need("moz_origins", &["id"])?;
                }
                let has_annotations = has_table(conn, "moz_annotations")?;
                if has_annotations {
                    need("moz_annotations", &["place_id"])?;
                }
                Ok(Schema { has_annotations, has_origins: has_origins && has_origin_id, has_deleted_forms: false })
            }
            Edit::FirefoxForms => {
                if !FORMS_VERSIONS.contains(&version) {
                    return Err(wrong());
                }
                need("moz_formhistory", &["id", "fieldname", "value"])?;
                let has_deleted_forms = has_table(conn, "moz_deleted_formhistory")?;
                if has_deleted_forms {
                    need("moz_deleted_formhistory", &["guid"])?;
                }
                Ok(Schema { has_annotations: false, has_origins: false, has_deleted_forms })
            }
            Edit::ChromiumAutofill => {
                // Chromium's `meta` table carries the version and isn't checked: the columns
                // Sprawl names are what it writes, and a changed table is refused below.
                need("autofill", &["name", "value"])?;
                Ok(Schema { has_annotations: false, has_origins: false, has_deleted_forms: false })
            }
        }
    }

    /// How many rows each delete would take, with the same conditions the delete uses.
    fn count(&self, conn: &Connection, kind: Edit) -> Result<u64, String> {
        let mut total = 0u64;
        for sql in self.count_sql(kind) {
            total += conn.query_row(&sql, [], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())? as u64;
        }
        Ok(total)
    }

    fn count_sql(&self, kind: Edit) -> Vec<String> {
        match kind {
            Edit::FirefoxHistory => {
                let mut v = vec![
                    "SELECT COUNT(*) FROM moz_historyvisits".to_string(),
                    "SELECT COUNT(*) FROM moz_places WHERE id NOT IN (SELECT fk FROM moz_bookmarks WHERE fk IS NOT NULL)".to_string(),
                ];
                if self.has_annotations {
                    v.push("SELECT COUNT(*) FROM moz_annotations WHERE place_id NOT IN (SELECT id FROM moz_places WHERE id IN (SELECT fk FROM moz_bookmarks WHERE fk IS NOT NULL))".into());
                }
                if self.has_origins {
                    v.push("SELECT COUNT(*) FROM moz_origins WHERE id NOT IN (SELECT origin_id FROM moz_places WHERE origin_id IS NOT NULL AND id IN (SELECT fk FROM moz_bookmarks WHERE fk IS NOT NULL))".into());
                }
                v
            }
            Edit::FirefoxForms => {
                let mut v = vec!["SELECT COUNT(*) FROM moz_formhistory".to_string()];
                if self.has_deleted_forms {
                    v.push("SELECT COUNT(*) FROM moz_deleted_formhistory".into());
                }
                v
            }
            Edit::ChromiumAutofill => vec!["SELECT COUNT(*) FROM autofill".to_string()],
        }
    }

    /// The deletes, in the order they must run, and whether each counts as rows taken out (the
    /// one bookkeeping UPDATE does not).
    fn statements(&self, kind: Edit) -> Vec<(String, bool)> {
        match kind {
            Edit::FirefoxHistory => {
                let mut v = vec![
                    // every visit goes, bookmarked pages' included: the part says Firefox forgets
                    // the pages you visited, and a visit to a bookmarked page is one of them
                    ("DELETE FROM moz_historyvisits".to_string(), true),
                    // a page without a bookmark row goes with them
                    ("DELETE FROM moz_places WHERE id NOT IN (SELECT fk FROM moz_bookmarks WHERE fk IS NOT NULL)".to_string(), true),
                    // what stays (the bookmarked pages) is left agreeing with the visits it has
                    ("UPDATE moz_places SET visit_count = (SELECT COUNT(*) FROM moz_historyvisits v WHERE v.place_id = moz_places.id), last_visit_date = (SELECT MAX(v.visit_date) FROM moz_historyvisits v WHERE v.place_id = moz_places.id) WHERE id IN (SELECT fk FROM moz_bookmarks WHERE fk IS NOT NULL)".to_string(), false),
                ];
                // notes of the pages that went are notes of nothing
                if self.has_annotations {
                    v.push(("DELETE FROM moz_annotations WHERE place_id NOT IN (SELECT id FROM moz_places)".into(), true));
                }
                // an origin no remaining page uses goes too
                if self.has_origins {
                    v.push(("DELETE FROM moz_origins WHERE id NOT IN (SELECT origin_id FROM moz_places WHERE origin_id IS NOT NULL)".into(), true));
                }
                v
            }
            Edit::FirefoxForms => {
                let mut v = vec![("DELETE FROM moz_formhistory".to_string(), true)];
                if self.has_deleted_forms {
                    v.push(("DELETE FROM moz_deleted_formhistory".into(), true));
                }
                v
            }
            Edit::ChromiumAutofill => vec![("DELETE FROM autofill".to_string(), true)],
        }
    }
}

fn integrity(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn.prepare("PRAGMA integrity_check").map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let mut first: Option<String> = None;
    let mut more = 0u64;
    while let Some(r) = rows.next().map_err(|e| e.to_string())? {
        let line: String = r.get(0).map_err(|e| e.to_string())?;
        if first.is_none() {
            first = Some(line);
        } else {
            more += 1;
        }
    }
    match first.as_deref() {
        Some("ok") if more == 0 => Ok(()),
        Some("ok") => Err(format!("the database didn't pass its own check, and {more} more things after its first line")),
        Some(why) => Err(format!("the database didn't pass its own check ({why})")),
        None => Err("the database didn't pass its own check (it gave no answer)".into()),
    }
}

/// `p` with `suffix` added to its last name (`places.sqlite` and `-wal`), without going through
/// text: a name that isn't valid Unicode keeps its own bytes.
fn side_path(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// The whole edit: the copy first, then the format check, then one transaction.
fn apply(kind: Edit, db: &Path, sides: &[PathBuf], folder: &Path, browser: &str, spare: &Spare) -> Result<u64, String> {
    let held = crate::clean::hold_plain(db, spare)?;
    let mut held_sides = Vec::new();
    for s in sides {
        match crate::clean::hold_plain(s, spare) {
            Ok(h) => held_sides.push((s.clone(), h)),
            Err(e) => return Err(format!("{}: {e}", s.display())),
        }
    }
    let name = |p: &Path| p.file_name().and_then(|n| n.to_str()).map(str::to_string).ok_or_else(|| "a file's name couldn't be read".to_string());
    let copied: Result<(), String> = (|| {
        let dir = crate::store::ensure_dir(folder).map_err(|e| format!("a backup folder couldn't be made ({e})"))?;
        copy_checked(&held, &dir, &name(db)?)?;
        for (s, h) in &held_sides {
            copy_checked(h, &dir, &name(s)?)?;
        }
        Ok(())
    })();    if let Err(e) = copied {
        // no checked copy, no edit: whatever was written is removed
        let _ = std::fs::remove_dir_all(folder);
        return Err(e);
    }
    // the side files are copied: SQLite is let to open them as it needs while it works
    drop(held_sides);
    if let Some(part_dir) = folder.parent() {
        prune(part_dir);
    }
    let mut conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("the database couldn't be opened for writing ({e})"))?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
    integrity(&conn)?;
    let schema = Schema::check(&conn, kind, browser)?;
    // one transaction for every delete: any failure rolls all of it back
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| format!("the edit couldn't be started ({e})"))?;
    let mut rows = 0u64;
    for (sql, counts) in schema.statements(kind) {
        let n = tx.execute(&sql, []).map_err(|e| format!("the edit was rolled back ({e})"))? as u64;
        if counts {
            rows += n;
        }
    }
    tx.commit().map_err(|e| format!("the edit was rolled back ({e})"))?;
    drop(conn);
    drop(held);
    Ok(rows)
}

/// Copy `src` (an open, checked file) into `dir` under `name`, then read it back and compare it
/// with the original byte for byte.
fn copy_checked(src: &crate::clean::HeldFile, dir: &sprawl_core::local::Directory, name: &str) -> Result<(), String> {
    use sprawl_core::local::{Disposition, Kind, READ_DATA, SHARE_READ, SHARE_WRITE, WRITE_DATA};
    let mut out = dir
        .open_in(name, WRITE_DATA, SHARE_READ, Disposition::OpenOrCreate, Kind::File, true)
        .map_err(|e| format!("a copy of {name} couldn't be made ({e})"))?;
    // a name already there (an earlier partial copy) is emptied first: the copy is exactly the
    // original, never its start with the old end left behind
    out.set_len(0).map_err(|e| format!("a copy of {name} couldn't be made ({e})"))?;
    src.copy_to(&mut out).map_err(|e| format!("a copy of {name} couldn't be written ({e})"))?;
    out.sync_all().map_err(|e| format!("a copy of {name} couldn't be saved ({e})"))?;
    drop(out);
    let mut back = dir
        .open_in(name, READ_DATA, SHARE_READ | SHARE_WRITE, Disposition::Open, Kind::File, false)
        .map_err(|e| format!("the copy of {name} couldn't be read again ({e})"))?;
    match src.equals(&mut back) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!("the copy of {name} doesn't match the original")),
        Err(e) => Err(format!("the copy of {name} couldn't be compared ({e})")),
    }
}

/// Keep the newest [`KEEP_BACKUPS`] copies in `part_dir`; the rest go. Only folders named like a
/// copy ([`stamp`]) and plain (never a link) are removed, so nothing else in the folder is
/// touched, whatever it holds.
fn prune(part_dir: &Path) {
    let Ok(rd) = std::fs::read_dir(part_dir) else { return };
    let mut names: Vec<String> = Vec::new();
    for e in rd.flatten() {
        let Ok(m) = std::fs::symlink_metadata(e.path()) else { continue };
        // a link (or a folder that leads elsewhere) is not Sprawl's to remove
        use std::os::windows::fs::MetadataExt;
        if !m.is_dir() || m.file_attributes() & 0x400 != 0 {
            continue;
        }
        if let Some(n) = e.file_name().to_str().filter(|n| is_stamp(n)) {
            names.push(n.to_string());
        }
    }
    names.sort();
    let over = names.len().saturating_sub(KEEP_BACKUPS);
    for old in names.into_iter().take(over) {
        let _ = std::fs::remove_dir_all(part_dir.join(old));
    }
}

/// Whether `name` is what [`stamp`] writes: `2026-11-23-03-50-12-123456`.
fn is_stamp(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 26 && b.iter().enumerate().all(|(i, c)| if matches!(i, 4 | 7 | 10 | 13 | 16 | 19) { *c == b'-' } else { c.is_ascii_digit() })
}

/// The name of one copy's folder: local date and time, with microseconds so two in one second
/// sort in the order they were made. Fixed width, so the newest sort last and [`prune`] can
/// tell a copy's folder from anything else.
pub(crate) fn stamp() -> String {
    let now = std::time::SystemTime::now();
    let micros = now.duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_micros()).unwrap_or(0);
    match sprawl_core::local_parts(now) {
        Some([y, m, d, h, mi, s]) => format!("{y:04}-{m:02}-{d:02}-{h:02}-{mi:02}-{s:02}-{micros:06}"),
        None => format!("0000-00-00-00-00-00-{micros:06}"),
    }
}

/// Edit `part`'s database, as `clean::run` reaches it: the checks a set has (this account's
/// profile, no link above, not kept, the browser closed, nothing else using the files), then
/// [`apply`]. Nothing is touched at the first "no".
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    part: &PrivacyPart,
    file: &Path,
    spare: &Spare,
    stop: &AtomicBool,
    places: &Places,
    now: Option<&HashSet<String>>,
    in_use: &dyn Fn(&[PathBuf]) -> Option<Vec<String>>,
    o: &mut Outcome,
    progress: &mut dyn FnMut(u64),
) {
    let browser = part.browser().short;
    let whole = "nothing in this database was edited";
    let Some(kind) = part.edit else {
        return o.problem(file, format!("not a part Sprawl edits; {whole}"));
    };
    let Some(m) = crate::privacy::member(part, file, places) else {
        return o.problem(file, format!("not one of this part's files; {whole}"));
    };
    if m.side.is_some() || m.file != 0 || part.files.len() != 1 {
        return o.problem(file, format!("not the one database this part edits; {whole}"));
    }
    if spare.keeps(file) {
        o.kept_chosen += 1;
        return o.problem(file, format!("you chose to keep it; {whole}"));
    }
    if crate::clean::has_link_above(file, places) {
        return o.problem(file, format!("a folder above it is a junction or link; {whole}"));
    }
    // the side files that will be copied with it: a link where one belongs holds the whole part
    let mut sides = Vec::new();
    for sfx in SIDE_FILES {
        let side = side_path(file, sfx);
        match std::fs::symlink_metadata(&side) {
            Ok(_) => sides.push(side),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return o.problem(&side, format!("{e}; {whole}")),
        }
    }
    let exes: Vec<String> = part.browser().running.iter().map(|r| r.to_string()).collect();
    match crate::procs::any_running(&exes, now) {
        Some(false) => {}
        Some(true) => return o.problem(file, format!("{} is open; {whole}", part.browser().short)),
        None => return o.problem(file, format!("Windows couldn't say whether {} is open; {whole}", part.browser().short)),
    }
    let mut all = vec![file.to_path_buf()];
    all.extend(sides.iter().cloned());
    match in_use(&all) {
        Some(names) if names.is_empty() => {}
        Some(names) => return o.problem(file, format!("in use by {}; {whole}", names.join(", "))),
        None => return o.problem(file, format!("Windows couldn't say which programs use these files; {whole}")),
    }
    if stop.load(std::sync::atomic::Ordering::Relaxed) {
        o.stopped = true;
        return;
    }
    let Some(root) = backup_root() else {
        return o.problem(file, format!("Sprawl's data folder couldn't be worked out; {whole}"));
    };
    let folder = root.join(part.backup_name()).join(stamp());
    match apply(kind, file, &sides, &folder, browser, spare) {
        Ok(rows) => {
            o.edited = Some(rows);
            progress(0);
        }
        Err(e) => o.problem(file, format!("{e}; {whole}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn dir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("sprawl-dbedit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn open(path: &Path) -> Connection {
        Connection::open(path).unwrap()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// Firefox's places.sqlite tables this code names, as Firefox writes them (Database.cpp and
    /// nsPlacesTables.h): the columns matter, so they are the real ones.
    const PLACES: &str = r#"
CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR, rev_host LONGVARCHAR, visit_count INTEGER DEFAULT 0, hidden INTEGER DEFAULT 0 NOT NULL, typed INTEGER DEFAULT 0 NOT NULL, frecency INTEGER DEFAULT -1 NOT NULL, last_visit_date INTEGER, guid TEXT, foreign_count INTEGER DEFAULT 0 NOT NULL, url_hash INTEGER DEFAULT 0 NOT NULL, description TEXT, preview_image_url TEXT, site_name TEXT, origin_id INTEGER, recalc_frecency INTEGER NOT NULL DEFAULT 0, alt_frecency INTEGER, recalc_alt_frecency INTEGER NOT NULL DEFAULT 0);
CREATE TABLE moz_historyvisits (id INTEGER PRIMARY KEY, from_visit INTEGER, place_id INTEGER, visit_date INTEGER, visit_type INTEGER, session INTEGER);
CREATE TABLE moz_bookmarks (id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER DEFAULT NULL, parent INTEGER, position INTEGER, title LONGVARCHAR, keyword_id INTEGER, folder_type TEXT, dateAdded INTEGER, lastModified INTEGER, guid TEXT, syncStatus INTEGER NOT NULL DEFAULT 0, syncChangeCounter INTEGER NOT NULL DEFAULT 1);
CREATE TABLE moz_origins (id INTEGER PRIMARY KEY, prefix TEXT NOT NULL, host TEXT NOT NULL, frecency INTEGER NOT NULL, recalc_frecency INTEGER NOT NULL DEFAULT 0, alt_frecency INTEGER, recalc_alt_frecency INTEGER NOT NULL DEFAULT 0, UNIQUE (prefix, host));
CREATE TABLE moz_keywords (id INTEGER PRIMARY KEY AUTOINCREMENT, keyword TEXT UNIQUE, place_id INTEGER, post_data TEXT);
CREATE TABLE moz_annotations (id INTEGER PRIMARY KEY, place_id INTEGER, name TEXT NOT NULL, value TEXT, date_added INTEGER, last_modified INTEGER, guid TEXT, syncStatus INTEGER NOT NULL DEFAULT 0, syncChangeCounter INTEGER NOT NULL DEFAULT 1);
"#;

    /// A bookmarked page with two visits, a note and a keyword; a visited page that is not
    /// bookmarked, with a note; a page never visited; two origins; a bookmark folder.
    fn places_db(path: &Path, version: i64) -> Connection {
        let c = open(path);
        c.execute_batch(PLACES).unwrap();
        c.pragma_update(None, "user_version", version).unwrap();
        c.execute_batch(
            r#"
INSERT INTO moz_origins (id, prefix, host, frecency) VALUES (1, 'https://', 'kept.example', 100), (2, 'https://', 'gone.example', 50);
INSERT INTO moz_places (id, url, title, visit_count, last_visit_date, origin_id) VALUES
  (1, 'https://kept.example/a', 'Kept', 2, 200, 1),
  (2, 'https://gone.example/b', 'Gone', 3, 500, 2),
  (3, 'https://gone.example/c', 'Typed', 0, NULL, 2);
INSERT INTO moz_historyvisits (id, place_id, visit_date, visit_type) VALUES (1, 1, 100, 1), (2, 1, 200, 1), (3, 2, 300, 1), (4, 2, 400, 1), (5, 2, 500, 1);
INSERT INTO moz_bookmarks (id, type, fk, parent, position, title) VALUES (3, 2, NULL, 0, 0, 'Toolbar'), (10, 1, 1, 3, 0, 'Kept');
INSERT INTO moz_keywords (id, keyword, place_id) VALUES (1, 'kept', 1);
INSERT INTO moz_annotations (id, place_id, name, value) VALUES (1, 1, 'bookmarkProperties/description', 'a note'), (2, 2, 'bookmarkProperties/description', 'a note on a page that goes');
"#,
        )
        .unwrap();
        c
    }

    const FORMS: &str = r#"
CREATE TABLE moz_formhistory (id INTEGER PRIMARY KEY, fieldname TEXT NOT NULL, value TEXT NOT NULL, timesUsed INTEGER, firstUsed INTEGER, lastUsed INTEGER, guid TEXT);
CREATE TABLE moz_deleted_formhistory (id INTEGER PRIMARY KEY, timeDeleted INTEGER, guid TEXT);
"#;

    fn forms_db(path: &Path) -> Connection {
        let c = open(path);
        c.execute_batch(FORMS).unwrap();
        c.pragma_update(None, "user_version", 5).unwrap();
        c.execute_batch(
            r#"
INSERT INTO moz_formhistory (id, fieldname, value, timesUsed, firstUsed, lastUsed, guid) VALUES (1, 'email', 'sam@example.com', 3, 1, 2, 'a'), (2, 'name', 'Sam', 1, 1, 1, 'b');
INSERT INTO moz_deleted_formhistory (id, timeDeleted, guid) VALUES (1, 2, 'c');
"#,
        )
        .unwrap();
        c
    }

    const WEB_DATA: &str = r#"
CREATE TABLE autofill (name VARCHAR NOT NULL, value VARCHAR NOT NULL, value_lower VARCHAR NOT NULL, date_created INTEGER DEFAULT 0, date_last_used INTEGER DEFAULT 0, count INTEGER DEFAULT 1, PRIMARY KEY (name, value));
CREATE TABLE autofill_profiles (guid VARCHAR PRIMARY KEY, company_name VARCHAR, street_address VARCHAR, dependent_locality VARCHAR, city VARCHAR, state VARCHAR, zipcode VARCHAR, sorting_code VARCHAR, country_code VARCHAR, use_count INTEGER, use_date INTEGER, date_modified INTEGER NOT NULL DEFAULT 0, origin VARCHAR, language_code VARCHAR);
CREATE TABLE credit_cards (guid VARCHAR PRIMARY KEY, name_on_card VARCHAR, expiration_month INTEGER, expiration_year INTEGER, card_number_encrypted BLOB, date_modified INTEGER NOT NULL DEFAULT 0, origin VARCHAR, use_count INTEGER NOT NULL DEFAULT 0, use_date INTEGER NOT NULL DEFAULT 0, nickname VARCHAR, billing_address_id VARCHAR);
CREATE TABLE keywords (id INTEGER PRIMARY KEY, short_name VARCHAR NOT NULL DEFAULT '', keyword VARCHAR NOT NULL DEFAULT '', favicon_url VARCHAR NOT NULL DEFAULT '', url VARCHAR NOT NULL DEFAULT '', safe_for_autoreplace INTEGER NOT NULL DEFAULT 0, originating_url VARCHAR, date_created INTEGER DEFAULT 0, usage_count INTEGER DEFAULT 0, input_encodings VARCHAR, display_url VARCHAR, prepopulate_id INTEGER NOT NULL DEFAULT 0, created_by_policy INTEGER, last_modified INTEGER DEFAULT 0, sync_guid VARCHAR, alternate_urls VARCHAR, image_url VARCHAR, search_url_post_params VARCHAR, suggestions_url_post_params VARCHAR, image_url_post_params VARCHAR, new_tab_url VARCHAR, last_visited INTEGER DEFAULT 0, created_from_metadata INTEGER NOT NULL DEFAULT 0);
CREATE TABLE meta (key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);
"#;

    fn web_data_db(path: &Path) -> Connection {
        let c = open(path);
        c.execute_batch(WEB_DATA).unwrap();
        c.execute_batch(
            r#"
INSERT INTO autofill (name, value, value_lower) VALUES ('email', 'sam@example.com', 'sam@example.com'), ('name', 'Sam', 'sam'), ('phone', '555', '555');
INSERT INTO autofill_profiles (guid, city, country_code) VALUES ('p1', 'Ankara', 'TR');
INSERT INTO credit_cards (guid, name_on_card, expiration_month, expiration_year) VALUES ('c1', 'SAM', 1, 2030);
INSERT INTO keywords (id, short_name, keyword, url) VALUES (1, 'Search', 'go', 'https://search.example/?q=%s');
INSERT INTO meta (key, value) VALUES ('version', '90');
"#,
        )
        .unwrap();
        c
    }

    /// Firefox history: every visit goes, the pages that aren't bookmarked go with them, and the
    /// bookmarked page stays with counts that agree (none left); its keyword and note stay.
    #[test]
    fn firefox_history_rows_go_and_bookmarks_stay() {
        let base = dir("history");
        let db_path = base.join("places.sqlite");
        places_db(&db_path, 61);
        let before = std::fs::read(&db_path).unwrap();
        let backups = base.join("backups").join("privacy.firefox-history");
        let rows = apply(Edit::FirefoxHistory, &db_path, &[], &backups, "Firefox", &Spare::default()).unwrap();
        assert_eq!(rows, 9, "5 visits, 2 pages, 1 note and 1 origin");
        let c = open(&db_path);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_historyvisits"), 0);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_places"), 1);
        let (url, visits, last): (String, i64, Option<i64>) = c.query_row("SELECT url, visit_count, last_visit_date FROM moz_places", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!((url.as_str(), visits, last), ("https://kept.example/a", 0, None), "the kept page's counts agree with the visits left");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_bookmarks WHERE fk = 1"), 1, "the bookmark stays");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_bookmarks WHERE type = 2"), 1, "the folder stays");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_keywords WHERE place_id = 1"), 1, "the keyword stays");
        let notes: Vec<String> = { let mut s = c.prepare("SELECT value FROM moz_annotations").unwrap(); s.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect() };
        assert_eq!(notes, ["a note"], "the note of the page that stayed; the other page's note went");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_origins"), 1, "only the origin a remaining page uses");
        drop(c);
        // the copy is the database exactly as it was, and it is kept
        assert_eq!(std::fs::read(backups.join("places.sqlite")).unwrap(), before, "the copy is byte for byte the original");
        // and running it again takes out nothing
        assert_eq!(apply(Edit::FirefoxHistory, &db_path, &[], &backups, "Firefox", &Spare::default()).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Firefox form history: every entry goes, the deleted-entry table with it.
    #[test]
    fn firefox_form_history_is_emptied() {
        let base = dir("forms");
        let db_path = base.join("formhistory.sqlite");
        forms_db(&db_path);
        let backups = base.join("backups");
        let rows = apply(Edit::FirefoxForms, &db_path, &[], &backups, "Firefox", &Spare::default()).unwrap();
        assert_eq!(rows, 3, "two entries and one deleted entry");
        let c = open(&db_path);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_formhistory"), 0);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_deleted_formhistory"), 0);
        // without the deleted table the part still works
        drop(c);
        let plain = base.join("plain.sqlite");
        { let c = open(&plain); c.execute_batch("CREATE TABLE moz_formhistory (id INTEGER PRIMARY KEY, fieldname TEXT NOT NULL, value TEXT NOT NULL);").unwrap(); c.pragma_update(None, "user_version", 4).unwrap(); c.execute_batch("INSERT INTO moz_formhistory (fieldname, value) VALUES ('a', 'b')").unwrap(); }
        assert_eq!(apply(Edit::FirefoxForms, &plain, &[], &base.join("bk2"), "Firefox", &Spare::default()).unwrap(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Chromium autofill: the `autofill` rows go, and addresses, cards, passwords' neighbour
    /// tables (never opened) and search engines stay.
    #[test]
    fn chromium_autofill_rows_go_and_the_rest_stays() {
        let base = dir("autofill");
        let db_path = base.join("Web Data");
        web_data_db(&db_path);
        let rows = apply(Edit::ChromiumAutofill, &db_path, &[], &base.join("backups"), "Chrome", &Spare::default()).unwrap();
        assert_eq!(rows, 3);
        let c = open(&db_path);
        for (table, n) in [("autofill", 0), ("autofill_profiles", 1), ("credit_cards", 1), ("keywords", 1)] {
            assert_eq!(count(&c, &format!("SELECT COUNT(*) FROM {table}")), n, "{table}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Without a checked copy nothing is edited: a backup folder that can't be made stops it,
    /// and the database is exactly as it was.
    #[test]
    fn no_backup_no_edit() {
        let base = dir("nobackup");
        let db_path = base.join("places.sqlite");
        places_db(&db_path, 61);
        let before = std::fs::read(&db_path).unwrap();
        // a file where the backup folder should be
        std::fs::write(base.join("backups"), b"not a folder").unwrap();
        let err = apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups").join("part"), "Firefox", &Spare::default()).unwrap_err();
        assert!(err.contains("backup folder"), "{err}");
        assert_eq!(std::fs::read(&db_path).unwrap(), before);
        assert_eq!(open(&db_path).query_row("SELECT COUNT(*) FROM moz_historyvisits", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A schema version Sprawl wasn't written for is refused, and nothing is written.
    #[test]
    fn an_unknown_schema_version_is_refused_untouched() {
        let base = dir("version");
        let db_path = base.join("places.sqlite");
        places_db(&db_path, 999);
        let before = std::fs::read(&db_path).unwrap();
        let err = apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups"), "Firefox", &Spare::default()).unwrap_err();
        assert!(err.contains("this version of Firefox isn't one Sprawl knows how to edit"), "{err}");
        assert_eq!(std::fs::read(&db_path).unwrap(), before);
        // the oldest Firefox itself opens (52) is taken, and the newest Sprawl was written for
        places_db(&base.join("old.sqlite"), 52);
        assert!(apply(Edit::FirefoxHistory, &base.join("old.sqlite"), &[], &base.join("bk52"), "Firefox", &Spare::default()).is_ok());
        places_db(&base.join("new.sqlite"), 89);
        assert!(apply(Edit::FirefoxHistory, &base.join("new.sqlite"), &[], &base.join("bk89"), "Firefox", &Spare::default()).is_ok());
        // one below each end is refused too
        places_db(&base.join("too-old.sqlite"), 51);
        let before = std::fs::read(base.join("too-old.sqlite")).unwrap();
        let err = apply(Edit::FirefoxHistory, &base.join("too-old.sqlite"), &[], &base.join("bk51"), "Firefox", &Spare::default()).unwrap_err();
        assert!(err.contains("isn't one Sprawl knows how to edit"), "{err}");
        assert_eq!(std::fs::read(base.join("too-old.sqlite")).unwrap(), before);
        places_db(&base.join("too-new.sqlite"), 90);
        assert!(apply(Edit::FirefoxHistory, &base.join("too-new.sqlite"), &[], &base.join("bk90"), "Firefox", &Spare::default()).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A failure part-way through the deletes rolls the whole transaction back.
    #[test]
    fn a_failure_mid_way_rolls_back() {
        let base = dir("rollback");
        let db_path = base.join("places.sqlite");
        {
            let c = places_db(&db_path, 61);
            // the last statement (on moz_origins) fails after the visits and pages have gone
            c.execute_batch("CREATE TRIGGER stop_origins BEFORE DELETE ON moz_origins BEGIN SELECT RAISE(ABORT, 'no'); END;").unwrap();
        }
        let err = apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups"), "Firefox", &Spare::default()).unwrap_err();
        assert!(err.contains("rolled back"), "{err}");
        let c = open(&db_path);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_historyvisits"), 5);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_places"), 3);
        assert_eq!(c.query_row("SELECT visit_count FROM moz_places WHERE id = 1", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_annotations"), 2);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_origins"), 2);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A file with another name (a hard link) is refused, and one that changed since the scan.
    #[test]
    fn a_file_with_another_name_or_changed_is_left_alone() {
        let base = dir("links");
        let db_path = base.join("places.sqlite");
        places_db(&db_path, 61);
        std::fs::hard_link(&db_path, base.join("same file")).unwrap();
        let err = apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups"), "Firefox", &Spare::default()).unwrap_err();
        assert!(err.contains("another name"), "{err}");
        std::fs::remove_file(base.join("same file")).unwrap();
        let old = Spare { since: Some(std::time::SystemTime::UNIX_EPOCH), keep: Vec::new(), seen: None };
        let err = apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups"), "Firefox", &old).unwrap_err();
        assert!(err.contains("changed since the scan"), "{err}");
        // a symbolic link at the file itself, when Windows lets a test make one
        let target = base.join("target.sqlite");
        std::fs::copy(&db_path, &target).unwrap();
        let linked = base.join("linked.sqlite");
        if std::os::windows::fs::symlink_file(&target, &linked).is_ok() {
            let err = apply(Edit::FirefoxHistory, &linked, &[], &base.join("backups"), "Firefox", &Spare::default()).unwrap_err();
            assert!(err.contains("plain local file") || err.contains("doesn't lead"), "{err}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The checks `clean::run` makes before it edits: a browser that is open, and a profile
    /// reached through a junction, both leave the database as it was.
    #[test]
    fn an_open_browser_or_a_link_above_holds_the_edit() {
        let base = dir("hold");
        let local = base.join("Local");
        let roaming = base.join("Roaming");
        let profile = local.join(r"Google\Chrome\User Data\Default");
        std::fs::create_dir_all(&profile).unwrap();
        let db_path = profile.join("Web Data");
        web_data_db(&db_path);
        let before = std::fs::read(&db_path).unwrap();
        let places = Places { local: local.clone(), roaming };
        let part = crate::privacy::part("privacy.chrome/autofill").unwrap();
        let mut o = crate::clean::Outcome::default();
        let open: HashSet<String> = ["chrome.exe".to_string()].into();
        let nobody = |_: &[PathBuf]| Some(Vec::new());
        let stop = AtomicBool::new(false);
        let mut progress = |_| {};
        run(part, &db_path, &Spare::default(), &stop, &places, Some(&open), &nobody, &mut o, &mut progress);
        assert!(o.problems[0].contains("Chrome is open"), "{:?}", o.problems);
        assert_eq!(std::fs::read(&db_path).unwrap(), before);
        // a folder above it swapped for a junction
        let real = base.join("elsewhere");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::rename(&profile, &real.join("Default")).unwrap();
        let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&profile).arg(real.join("Default")).output().is_ok_and(|o| o.status.success());
        if ok {
            let mut o = crate::clean::Outcome::default();
            let closed: HashSet<String> = ["explorer.exe".to_string()].into();
            run(part, &db_path, &Spare::default(), &stop, &places, Some(&closed), &nobody, &mut o, &mut progress);
            assert!(o.problems[0].contains("junction or link"), "{:?}", o.problems);
            assert_eq!(std::fs::read(&db_path).unwrap(), before);
            let _ = std::fs::remove_dir(&profile);
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Measuring for the review is read-only and agrees with what the edit does, and it leaves
    /// the database's bytes as they were.
    #[test]
    fn counting_is_read_only_and_agrees_with_the_edit() {
        let base = dir("count");
        let db_path = base.join("places.sqlite");
        places_db(&db_path, 61);
        let before = std::fs::read(&db_path).unwrap();
        assert_eq!(rows(Edit::FirefoxHistory, &db_path), Some(9));
        assert_eq!(std::fs::read(&db_path).unwrap(), before, "measuring wrote nothing");
        assert_eq!(rows(Edit::FirefoxHistory, &base.join("missing.sqlite")), None);
        // and it agrees with the edit
        assert_eq!(apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups"), "Firefox", &Spare::default()).unwrap(), 9);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A database in WAL mode (as Firefox and Chromium keep theirs) is edited the same way: the
    /// write-ahead log is SQLite's to make, and the deletes still go in one transaction.
    #[test]
    fn a_write_ahead_log_database_is_edited() {
        let base = dir("wal");
        let db_path = base.join("places.sqlite");
        {
            let c = places_db(&db_path, 61);
            c.pragma_update(None, "journal_mode", "WAL").unwrap();
        }
        let rows = apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups"), "Firefox", &Spare::default()).unwrap();
        assert_eq!(rows, 9);
        let c = open(&db_path);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_historyvisits"), 0);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_places"), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Measuring a WAL database read-only works and leaves the profile as it was (no `-shm` or
    /// `-wal` made by the review), so the scan's "changed since" check isn't tripped by Sprawl.
    #[test]
    fn measuring_a_wal_database_touches_nothing() {
        let base = dir("walcount");
        let db_path = base.join("places.sqlite");
        {
            let c = places_db(&db_path, 61);
            c.pragma_update(None, "journal_mode", "WAL").unwrap();
        }
        let names = |p: &Path| { let mut v: Vec<String> = std::fs::read_dir(p).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect(); v.sort(); v };
        let before = names(&base);
        assert_eq!(rows(Edit::FirefoxHistory, &db_path), Some(9), "the main file is read");
        assert_eq!(names(&base), before, "nothing was made beside it");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The review's finding for an edit: rows, no bytes, the action names the database, the
    /// preview lists it, and the safety check takes it as the part's own file only.
    #[test]
    fn the_review_offers_rows_not_bytes() {
        let (base, places) = crate::privacy::tests::fixture("editfind");
        let fp = places.roaming.join(r"Mozilla\Firefox\Profiles\abcd1234.default-release");
        let db = fp.join("places.sqlite");
        std::fs::remove_file(&db).unwrap();
        places_db(&db, 61);
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let found = crate::privacy::find_in(&tree, &places);
        let f = found.iter().find(|f| f.title == "Firefox: Browsing history (default-release)").expect("the history part");
        assert_eq!(f.bytes, 0, "a privacy edit is never counted as space");
        assert!(
            matches!(&f.action, Some(crate::finding::Action::Edit { part, file, rows }) if part == "privacy.firefox/history" && file == &db && *rows == 9),
            "{:?}",
            f.action
        );
        assert!(f.evidence.iter().any(|e| e.contains("9 rows would go")), "{:?}", f.evidence);
        // the fixture's other databases aren't readable as their formats, so their parts aren't offered
        assert!(!found.iter().any(|f| f.title.contains("Form history") || f.title.contains("Autofill")), "{found:?}");
        // the safety check takes it, with no space claimed
        let never = crate::safety::Never::default_list();
        let home = Some(base.as_path());
        let ok = crate::safety::check_in(f, &tree, &never, crate::safety::CACHE_CARVE_OUTS, home, Some(&places)).unwrap();
        assert_eq!(ok.bytes, 0);
        // a finding whose part isn't the file's own is refused
        let mut bad = f.clone();
        bad.action = Some(crate::finding::Action::Edit { part: "privacy.chrome/autofill".into(), file: db.clone(), rows: 1 });
        assert!(crate::safety::check_in(&bad, &tree, &never, crate::safety::CACHE_CARVE_OUTS, home, Some(&places)).is_err());
        // the review's preview: the database, its size and the rows, never bytes to free
        let preview = crate::clean::edit_preview(crate::privacy::part("privacy.firefox/history").unwrap(), &db, &crate::clean::Spare::default(), &places);
        assert_eq!((preview.bytes, preview.files, preview.rows), (0, 1, 9));
        assert_eq!(preview.biggest[0].0, db);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Copies are named so they sort by when they were made, and only such folders are pruned.
    #[test]
    fn copy_folders_are_named_so_the_newest_sort_last() {
        let name = stamp();
        assert_eq!(name.len(), 26, "{name}");
        assert!(is_stamp(&name));
        assert!(!is_stamp("2026-11-23") && !is_stamp("a folder") && !is_stamp(""));
        assert!("2026-11-23-00-00-00-000001" < "2026-11-23-00-00-00-000002");
    }

    /// Only the newest few copies are kept, so a part cleaned often doesn't fill the disk.
    #[test]
    fn only_the_newest_copies_are_kept() {
        let base = dir("prune");
        let db_path = base.join("formhistory.sqlite");
        forms_db(&db_path);
        let part_dir = base.join("backups").join("privacy.firefox-forms");
        for i in 0..(KEEP_BACKUPS + 3) {
            let folder = part_dir.join(format!("2026-11-23-00-00-{i:02}-000000"));
            let c = open(&db_path);
            c.execute_batch("INSERT INTO moz_formhistory (fieldname, value) VALUES ('a', 'b')").unwrap();
            drop(c);
            apply(Edit::FirefoxForms, &db_path, &[], &folder, "Firefox", &Spare::default()).unwrap();
        }
        let left = std::fs::read_dir(&part_dir).unwrap().count();
        assert_eq!(left, KEEP_BACKUPS, "the oldest copies went");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The journal and write-ahead log of the database are copied with it.
    #[test]
    fn the_side_files_are_copied_too() {
        let base = dir("sides");
        let db_path = base.join("formhistory.sqlite");
        forms_db(&db_path);
        std::fs::write(format!("{}-wal", db_path.display()), b"a wal").unwrap();
        std::fs::write(format!("{}-shm", db_path.display()), b"an shm").unwrap();
        let sides: Vec<PathBuf> = SIDE_FILES.iter().map(|s| PathBuf::from(format!("{}{s}", db_path.display()))).filter(|p| p.exists()).collect();
        let backups = base.join("backups");
        apply(Edit::FirefoxForms, &db_path, &sides, &backups, "Firefox", &Spare::default()).unwrap();
        assert_eq!(std::fs::read(backups.join("formhistory.sqlite-wal")).unwrap(), b"a wal");
        assert_eq!(std::fs::read(backups.join("formhistory.sqlite-shm")).unwrap(), b"an shm");
        let _ = std::fs::remove_dir_all(&base);
    }
}