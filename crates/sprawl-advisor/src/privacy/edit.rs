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
//!    original byte for byte. If any of that fails, nothing is edited. Copies are named in UTC
//!    (so the newest sort last), the last [`KEEP_BACKUPS`] per part are kept, and copies older
//!    than 30 days are dropped; a copy that brought no edit is removed.
//! 3. the format is checked on a read-only connection first: `PRAGMA integrity_check` answers
//!    ok, the tables and columns Sprawl uses are there, and for Firefox the schema version is
//!    one Sprawl was written for. A database refused here is never opened for writing, which
//!    is what would let SQLite finish a log or journal the browser left behind.
//! 4. then the database is opened read-write and never created, and every delete for it goes in
//!    one transaction. If nothing was taken out, the transaction is dropped rather than kept, so
//!    a run with nothing to do writes nothing. Inside it, `PRAGMA quick_check` reads what was
//!    written (through the log the read-only check could not see), and `PRAGMA foreign_key_check`
//!    refuses a schema with a table Sprawl doesn't know still pointing at a row that went. The
//!    browser is checked again once the transaction is open and once more before it is kept
//!    (Firefox's `parent.lock`, Chromium's `lockfile`): if it started meanwhile, the edit is
//!    rolled back. No VACUUM (the browser does its own), so the file may not get smaller.
//!
//! The side files Sprawl held are released before SQLite opens them by path: the browser's own
//! `-journal`/`-wal` are then SQLite's to read and finish, not Sprawl's to hold (residual risk:
//! SQLite may complete a journal the browser left, and the browser-closed checks above are what
//! keep that from mattering).
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

/// Firefox's `places.sqlite` schema versions this code was written for: 52, which Firefox itself
/// still opens (below it, Places replaces the database), through 89, `DATABASE_SCHEMA_VERSION` in
/// nsINavHistoryService.idl (mozilla-firefox/firefox main, fetched 2026-11-23; the tables in
/// `firefox_places.sql` are the same revision's). Anything else is refused untouched.
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
/// checkpointed its log, so the file itself is what the edit will see; after a crash the log may
/// hold rows this count doesn't, which is why the review says "at least".
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
    // Firefox history: the tables only some versions have, and the columns that decide how
    // Firefox is asked to rebuild a kept page's frecency.
    has_origins: bool,
    has_annos: bool,
    has_inputhistory: bool,
    has_places_extra: bool,
    has_visits_extra: bool,
    has_metadata: bool,
    has_search_queries: bool,
    has_newtab: bool,
    has_meta: bool,
    has_foreign_count: bool,
    places_recalc: bool,
    places_recalc_alt: bool,
    origins_recalc: bool,
    origins_recalc_alt: bool,
}

impl Schema {
    const NONE: Schema = Schema {
        has_origins: false,
        has_annos: false,
        has_inputhistory: false,
        has_places_extra: false,
        has_visits_extra: false,
        has_metadata: false,
        has_search_queries: false,
        has_newtab: false,
        has_meta: false,
        has_foreign_count: false,
        places_recalc: false,
        places_recalc_alt: false,
        origins_recalc: false,
        origins_recalc_alt: false,
    };
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
                need("moz_keywords", &["place_id"])?;
                let places = columns(conn, "moz_places").map_err(|e| e.to_string())?;
                let has = |c: &str| places.iter().any(|p| p.eq_ignore_ascii_case(c));
                let mut s = Schema { has_foreign_count: has("foreign_count"), places_recalc: has("recalc_frecency"), places_recalc_alt: has("recalc_alt_frecency"), ..Schema::NONE };
                s.has_origins = has("origin_id") && has_table(conn, "moz_origins")?;
                if s.has_origins {
                    need("moz_origins", &["id", "frecency"])?;
                    let origins = columns(conn, "moz_origins").map_err(|e| e.to_string())?;
                    s.origins_recalc = origins.iter().any(|c| c.eq_ignore_ascii_case("recalc_frecency"));
                    s.origins_recalc_alt = origins.iter().any(|c| c.eq_ignore_ascii_case("recalc_alt_frecency"));
                }
                s.has_annos = has_table(conn, "moz_annos")?;
                if s.has_annos {
                    need("moz_annos", &["place_id"])?;
                }
                s.has_inputhistory = has_table(conn, "moz_inputhistory")?;
                if s.has_inputhistory {
                    need("moz_inputhistory", &["place_id"])?;
                }
                s.has_places_extra = has_table(conn, "moz_places_extra")?;
                if s.has_places_extra {
                    need("moz_places_extra", &["place_id"])?;
                }
                s.has_visits_extra = has_table(conn, "moz_historyvisits_extra")?;
                if s.has_visits_extra {
                    need("moz_historyvisits_extra", &["visit_id"])?;
                }
                s.has_metadata = has_table(conn, "moz_places_metadata")?;
                if s.has_metadata {
                    need("moz_places_metadata", &["place_id", "referrer_place_id", "search_query_id"])?;
                    s.has_search_queries = has_table(conn, "moz_places_metadata_search_queries")?;
                    if s.has_search_queries {
                        need("moz_places_metadata_search_queries", &["id"])?;
                    }
                }
                s.has_newtab = has_table(conn, "moz_newtab_shortcuts_interaction")?;
                if s.has_newtab {
                    need("moz_newtab_shortcuts_interaction", &["place_id"])?;
                }
                s.has_meta = has_table(conn, "moz_meta")?;
                if s.has_meta {
                    need("moz_meta", &["key"])?;
                }
                Ok(s)
            }
            Edit::FirefoxForms => {
                if !FORMS_VERSIONS.contains(&version) {
                    return Err(wrong());
                }
                // moz_deleted_formhistory is Firefox's own sync tombstones (what other devices
                // removed), and is left alone: deleting it would make sync take them back
                need("moz_formhistory", &["id", "fieldname", "value"])?;
                Ok(Schema::NONE)
            }
            Edit::ChromiumAutofill => {
                // Chromium's `meta` table carries the version and isn't checked: the columns
                // Sprawl names are what it writes, and a changed table is refused below.
                need("autofill", &["name", "value"])?;
                Ok(Schema::NONE)
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

    /// The places that stay: bookmarked, used by a keyword, or held by something else Firefox
    /// counts in `foreign_count`. Firefox's triggers bump that for bookmarks and keywords; an
    /// annotation alone does not count, so a download's page goes and its annotation with it.
    /// Qualified with the table name, so it reads the same inside a subquery.
    fn stays(&self) -> String {
        let mut s = String::from("(moz_places.id IN (SELECT fk FROM moz_bookmarks WHERE fk IS NOT NULL) OR moz_places.id IN (SELECT place_id FROM moz_keywords WHERE place_id IS NOT NULL))");
        if self.has_foreign_count {
            s = format!("({s} OR moz_places.foreign_count != 0)");
        }
        s
    }

    /// Firefox history's deletes, in the order they must run, each with the SELECT that counts
    /// exactly what it would take from the database as it is now (before any of them run).
    fn history_ops(&self) -> Vec<(String, String)> {
        let stays = self.stays();
        let deletable = format!("NOT {stays}");
        let mut v = vec![
            // every visit goes, bookmarked pages' included: the part says Firefox forgets the
            // pages you visited, and a visit to a bookmarked page is one of them
            ("DELETE FROM moz_historyvisits".to_string(), "SELECT COUNT(*) FROM moz_historyvisits".to_string()),
            // a page goes only when nothing else of Firefox's holds it: no bookmark, no keyword,
            // and (when the column is there) no annotation or other holder
            (format!("DELETE FROM moz_places WHERE {deletable}"), format!("SELECT COUNT(*) FROM moz_places WHERE {deletable}")),
        ];
        // the visit records of newer versions, keyed to the visits that went
        if self.has_visits_extra {
            v.push((
                "DELETE FROM moz_historyvisits_extra WHERE visit_id NOT IN (SELECT id FROM moz_historyvisits)".to_string(),
                "SELECT COUNT(*) FROM moz_historyvisits_extra".to_string(),
            ));
        }
        // rows keyed to a page, for the pages that went (none of these declare a foreign key)
        if self.has_annos {
            v.push((
                "DELETE FROM moz_annos WHERE place_id NOT IN (SELECT id FROM moz_places)".to_string(),
                format!("SELECT COUNT(*) FROM moz_annos WHERE place_id IN (SELECT id FROM moz_places WHERE {deletable})"),
            ));
        }
        if self.has_inputhistory {
            v.push((
                "DELETE FROM moz_inputhistory WHERE place_id NOT IN (SELECT id FROM moz_places)".to_string(),
                format!("SELECT COUNT(*) FROM moz_inputhistory WHERE place_id IN (SELECT id FROM moz_places WHERE {deletable})"),
            ));
        }
        if self.has_places_extra {
            v.push((
                "DELETE FROM moz_places_extra WHERE place_id NOT IN (SELECT id FROM moz_places)".to_string(),
                format!("SELECT COUNT(*) FROM moz_places_extra WHERE place_id IN (SELECT id FROM moz_places WHERE {deletable})"),
            ));
        }
        if self.has_metadata {
            // Firefox's own History.clear() removes every interaction record, not only those
            // keyed to a page that goes: typed text, view times and search terms are history too
            v.push((
                "DELETE FROM moz_places_metadata".to_string(),
                "SELECT COUNT(*) FROM moz_places_metadata".to_string(),
            ));
        }
        if self.has_search_queries {
            v.push((
                "DELETE FROM moz_places_metadata_search_queries".to_string(),
                "SELECT COUNT(*) FROM moz_places_metadata_search_queries".to_string(),
            ));
        }
        if self.has_newtab {
            v.push((
                "DELETE FROM moz_newtab_shortcuts_interaction".to_string(),
                "SELECT COUNT(*) FROM moz_newtab_shortcuts_interaction".to_string(),
            ));
        }
        if self.has_origins {
            v.push((
                "DELETE FROM moz_origins WHERE id NOT IN (SELECT origin_id FROM moz_places WHERE origin_id IS NOT NULL)".to_string(),
                format!("SELECT COUNT(*) FROM moz_origins WHERE NOT EXISTS (SELECT 1 FROM moz_places WHERE moz_places.origin_id = moz_origins.id AND {stays})"),
            ));
        }
        v
    }

    fn count_sql(&self, kind: Edit) -> Vec<String> {
        match kind {
            Edit::FirefoxHistory => self.history_ops().into_iter().map(|(_, c)| c).collect(),
            Edit::FirefoxForms => vec!["SELECT COUNT(*) FROM moz_formhistory".to_string()],
            Edit::ChromiumAutofill => vec!["SELECT COUNT(*) FROM autofill".to_string()],
        }
    }

    /// The deletes, in the order they must run, and whether each counts as rows taken out (the
    /// bookkeeping UPDATEs do not).
    fn statements(&self, kind: Edit) -> Vec<(String, bool)> {
        match kind {
            Edit::FirefoxHistory => {
                let mut v: Vec<(String, bool)> = self.history_ops().into_iter().map(|(d, _)| (d, true)).collect();
                // what stays keeps its place but no history: Firefox's own counts say none
                v.push(("UPDATE moz_places SET visit_count = 0, last_visit_date = NULL".to_string(), false));
                // and its ranking is asked to be worked out again from nothing, rather than kept
                // from the visits that are gone
                if self.places_recalc {
                    v.push(("UPDATE moz_places SET recalc_frecency = 1".to_string(), false));
                }
                if self.places_recalc_alt {
                    v.push(("UPDATE moz_places SET recalc_alt_frecency = 1".to_string(), false));
                }
                if !self.places_recalc && !self.places_recalc_alt {
                    v.push(("UPDATE moz_places SET frecency = -1".to_string(), false));
                }
                if self.has_origins {
                    if self.origins_recalc {
                        v.push(("UPDATE moz_origins SET recalc_frecency = 1".to_string(), false));
                    }
                    if self.origins_recalc_alt {
                        v.push(("UPDATE moz_origins SET recalc_alt_frecency = 1".to_string(), false));
                    }
                    if !self.origins_recalc && !self.origins_recalc_alt {
                        // an origin's own "work this out again" is 0 (there is no -1 for it)
                        v.push(("UPDATE moz_origins SET frecency = 0".to_string(), false));
                    }
                }
                // the cached origin frecency sums are rebuilt from the pages that are left
                if self.has_meta {
                    v.push(("DELETE FROM moz_meta WHERE key LIKE 'origin_%frecency%'".to_string(), false));
                }
                v
            }
            Edit::FirefoxForms => vec![("DELETE FROM moz_formhistory".to_string(), true)],
            Edit::ChromiumAutofill => vec![("DELETE FROM autofill".to_string(), true)],
        }
    }
}

fn integrity(conn: &Connection) -> Result<(), String> {
    passes(conn, "integrity_check", "the database")
}

/// The same, quickly, on the connection that did the writing: it reads pages through the log the
/// read-only check couldn't see.
fn quick(conn: &Connection) -> Result<(), String> {
    passes(conn, "quick_check", "the edited database")
}

fn passes(conn: &Connection, pragma: &str, who: &str) -> Result<(), String> {
    let bad = |e: rusqlite::Error| format!("{who} didn't pass its own check ({e})");
    let mut stmt = conn.prepare(&format!("PRAGMA {pragma}")).map_err(bad)?;
    let mut rows = stmt.query([]).map_err(bad)?;
    let mut first: Option<String> = None;
    let mut more = 0u64;
    while let Some(r) = rows.next().map_err(bad)? {
        let line: String = r.get(0).map_err(bad)?;
        if first.is_none() {
            first = Some(line);
        } else {
            more += 1;
        }
    }
    match first.as_deref() {
        Some("ok") if more == 0 => Ok(()),
        Some("ok") => Err(format!("{who} didn't pass its own check, and {more} more things after its first line")),
        Some(why) => Err(format!("{who} didn't pass its own check ({why})")),
        None => Err(format!("{who} didn't pass its own check (it gave no answer)")),
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
#[cfg(test)]
fn apply(kind: Edit, db: &Path, sides: &[PathBuf], folder: &Path, browser: &str, spare: &Spare) -> Result<u64, String> {
    let mut notes = Vec::new();
    apply_guarded(kind, db, sides, folder, browser, spare, &|| Ok(()), &mut notes)
}

/// [`apply`], with `guard` run once the transaction is open and again just before it is
/// committed: the browser may start between the check before the edit and the commit, and the
/// edit must not land if it did. Anything that goes wrong after the copy removes the copy and
/// leaves the database as SQLite found it; anything that couldn't be removed is a note.
#[allow(clippy::too_many_arguments)]
fn apply_guarded(kind: Edit, db: &Path, sides: &[PathBuf], folder: &Path, browser: &str, spare: &Spare, guard: &dyn Fn() -> Result<(), String>, notes: &mut Vec<String>) -> Result<u64, String> {
    let held = crate::clean::hold_plain(db, spare)?;
    let mut held_sides = Vec::new();
    for s in sides {
        match crate::clean::hold_plain(s, spare) {
            Ok(h) => held_sides.push((s.clone(), h)),
            Err(e) => return Err(format!("{}: {e}", s.display())),
        }
    }
    let name = |p: &Path| p.file_name().and_then(|n| n.to_str()).map(str::to_string).ok_or_else(|| "a file's name couldn't be read".to_string());
    let Some(part_dir) = folder.parent() else { return Err("the backup folder's place couldn't be read".into()) };
    let copied: Result<(), String> = (|| {
        let dir = crate::store::ensure_dir(folder).map_err(|e| format!("a backup folder couldn't be made ({e})"))?;
        copy_checked(&held, &dir, &name(db)?)?;
        for (s, h) in &held_sides {
            copy_checked(h, &dir, &name(s)?)?;
        }
        Ok(())
    })();
    if let Err(e) = copied {
        // no checked copy, no edit: whatever was written is removed
        if let Some(n) = remove_copy(part_dir, folder) {
            notes.push(n);
        }
        return Err(e);
    }
    // the side files are copied and released: SQLite opens them itself, by path, from here on
    drop(held_sides);
    match edit_database(kind, db, browser, guard) {
        Ok(0) => {
            // nothing was edited: a copy of it would only push a real one out
            if let Some(n) = remove_copy(part_dir, folder) {
                notes.push(n);
            }
            Ok(0)
        }
        Ok(rows) => {
            let keep = folder.file_name().and_then(|n| n.to_str()).unwrap_or("");
            notes.extend(prune(part_dir, keep));
            Ok(rows)
        }
        Err(e) => {
            // a failed try keeps no copy: it must not push a good one out
            if let Some(n) = remove_copy(part_dir, folder) {
                notes.push(n);
            }
            Err(e)
        }
    }
}

/// The checks, then the deletes. The format is checked on a read-only connection first, so a
/// database Sprawl refuses is never opened for writing (a read-write open would let SQLite
/// finish a write-ahead log or a journal the browser left behind).
fn edit_database(kind: Edit, db: &Path, browser: &str, guard: &dyn Fn() -> Result<(), String>) -> Result<u64, String> {
    {
        let ro = open_read(db).map_err(|e| format!("the database couldn't be read ({e})"))?;
        integrity(&ro)?;
        Schema::check(&ro, kind, browser)?;
    }
    let mut conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("the database couldn't be opened for writing ({e})"))?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
    // every row keyed to a page that goes is removed by a statement of its own below, and each
    // one is counted; SQLite's own cascades would take some of them silently, so they are off
    conn.execute_batch("PRAGMA foreign_keys = OFF").map_err(|e| format!("the database couldn't be prepared ({e})"))?;
    let schema = Schema::check(&conn, kind, browser)?;
    // one transaction for every delete: any failure rolls all of it back
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| format!("the edit couldn't be started ({e})"))?;
    // the write lock is ours now: a browser that started meanwhile is caught before a page is
    // written, and again before what was written is kept
    guard()?;
    let mut rows = 0u64;
    for (sql, counts) in schema.statements(kind) {
        let n = tx.execute(&sql, []).map_err(|e| format!("the edit was rolled back ({e})"))? as u64;
        if counts {
            rows += n;
        }
    }
    if rows == 0 {
        // nothing was taken out: drop the transaction, so its bookkeeping UPDATEs are not kept
        // (a copy of a database nothing changed is not worth keeping either)
        return Ok(0);
    }
    // the write connection can see a log the read-only check could not: check what was written
    quick(&tx)?;
    // anything the edit left pointing at a row that went is a schema Sprawl doesn't know
    if foreign_key_orphan(&tx)? {
        return Err(unknown(browser));
    }
    guard()?;
    tx.commit().map_err(|e| format!("the edit was rolled back ({e})"))?;
    Ok(rows)
}

/// Whether any foreign key in the database points at a row that isn't there (it works with
/// enforcement off, which is how the edit runs).
fn foreign_key_orphan(conn: &Connection) -> Result<bool, String> {
    let mut stmt = conn.prepare("PRAGMA foreign_key_check").map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    Ok(rows.next().map_err(|e| e.to_string())?.is_some())
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

/// Keep the newest [`KEEP_BACKUPS`] copies in `part_dir`, and drop any older than 30 days; the
/// copy just written is never one of them, and each goes through the pinned folder's handle
/// (only folders named like a copy are considered, and never a link). A copy that can't be
/// removed is reported and does not count as one of the copies kept.
fn prune(part_dir: &Path, keep: &str) -> Vec<String> {
    let mut notes = Vec::new();
    let Ok(dir) = crate::store::ensure_dir(part_dir) else { return notes };
    let Ok(names) = dir.names() else { return notes };
    let cutoff = stamp_before(30 * 86_400);
    let mut old: Vec<String> = names.into_iter().filter(|n| is_stamp(n) && n != keep).collect();
    old.sort();
    let excess = old.len().saturating_sub(KEEP_BACKUPS.saturating_sub(1));
    let mut removed = 0usize;
    for n in &old {
        let too_old = cutoff.as_deref().is_some_and(|c| n.as_str() < c);
        if removed < excess || too_old {
            match dir.remove_tree(n) {
                Ok(()) => removed += 1,
                // it stays and is not counted as kept: the next one goes instead
                Err(e) => notes.push(format!("an old copy couldn't be removed ({}: {e})", part_dir.join(n).display())),
            }
        }
    }
    notes
}

/// Where the browser keeps the file it holds while it runs: Firefox's `parent.lock` in the
/// profile, Chromium's `lockfile` in the user-data folder one above the profile.
fn lock_path(kind: crate::privacy::Kind, db: &Path) -> Option<PathBuf> {
    match kind {
        crate::privacy::Kind::Firefox => db.parent().map(|p| p.join("parent.lock")),
        crate::privacy::Kind::Chromium => db.parent().and_then(|p| p.parent()).map(|p| p.join("lockfile")),
    }
}

/// Firefox holds `parent.lock` in its profile while it runs, Chromium `lockfile` in its user-data
/// folder: one that exists and can't be opened over means the browser is (or is starting) and
/// nothing may be edited. The file is only opened when it is already there, so a browser starting
/// in that instant is not held up by Sprawl.
fn browser_lock_free(lock: &Path, who: &str) -> Result<(), String> {
    use std::os::windows::fs::OpenOptionsExt;
    if let Err(e) = std::fs::symlink_metadata(lock) {
        return if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(format!("{who}'s lock couldn't be checked ({e})")) };
    }
    match std::fs::OpenOptions::new().read(true).share_mode(0).custom_flags(0x0020_0000).open(lock) {
        Ok(f) => {
            drop(f);
            Ok(())
        }
        Err(_) => Err(format!("{who}'s lock is held, so it may have started")),
    }
}

/// Remove the copy at `folder` (the one just made), through the pinned part folder's handle.
/// Remove the copy at `folder` (the one just made), through the pinned part folder's handle. A
/// copy that can't be removed is reported: it stays where it is.
fn remove_copy(part_dir: &Path, folder: &Path) -> Option<String> {
    let name = folder.file_name().and_then(|n| n.to_str())?;
    let dir = crate::store::ensure_dir(part_dir).ok()?;
    dir.remove_tree(name).err().map(|e| format!("a copy of the database couldn't be removed ({}: {e})", folder.display()))
}

/// Whether `name` is what [`stamp`] writes: `2026-11-23-03-50-12-123456`.
fn is_stamp(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 26 && b.iter().enumerate().all(|(i, c)| if matches!(i, 4 | 7 | 10 | 13 | 16 | 19) { *c == b'-' } else { c.is_ascii_digit() })
}

/// The name of one copy's folder: UTC date and time, fixed width so the newest sort last and
/// [`prune`] can tell a copy's folder from anything else. None when the clock can't be read
/// (never a made-up name that would sort first and be pruned in place of a real copy).
pub(crate) fn stamp_at(t: std::time::SystemTime) -> Option<String> {
    let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    let [y, m, day, h, mi, s] = utc_parts(d.as_secs())?;
    Some(format!("{y:04}-{m:02}-{day:02}-{h:02}-{mi:02}-{s:02}-{:06}", d.subsec_micros()))
}

fn stamp_before(secs: u64) -> Option<String> {
    stamp_at(std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(secs))?)
}

pub(crate) fn stamp() -> Result<String, String> {
    stamp_at(std::time::SystemTime::now()).ok_or_else(|| "the time couldn't be read, so nothing was edited".to_string())
}

/// UTC date and time from seconds since 1970, by the civil-date algorithm that needs no table
/// (Howard Hinnant's `civil_from_days`).
fn utc_parts(secs: u64) -> Option<[i64; 6]> {
    let secs = i64::try_from(secs).ok()?;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Some([year, month, day, rem / 3_600, rem % 3_600 / 60, rem % 60])
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
    let stamp = match stamp() {
        Ok(s) => s,
        Err(e) => return o.problem(file, format!("{e}; {whole}")),
    };
    let folder = root.join(part.backup_name()).join(&stamp);
    // the browser may start between the check just made and the commit: the guard runs after
    // the transaction is open and again before it is kept
    let b = part.browser();
    let lock = lock_path(b.kind, file);
    let guard = || -> Result<(), String> {
        match crate::procs::any_running(&exes, crate::procs::running().as_ref()) {
            Some(false) => {}
            Some(true) => return Err(format!("{} started while the database was being edited", b.short)),
            None => return Err(format!("Windows couldn't say whether {} is open", b.short)),
        }
        // a browser that could have been started and stopped again is caught by the lock it
        // holds while it runs; one it can't be opened over means it is back
        if let Some(lock) = &lock {
            browser_lock_free(lock, b.short)?;
        }
        Ok(())
    };
    match apply_guarded(kind, file, &sides, &folder, browser, spare, &guard, &mut o.notes) {
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

    /// A folder under the temp folder, removed when the test ends (after everything opened in
    /// it has been dropped, which is how the test's own locals are ordered).
    struct Temp(PathBuf);
    impl std::ops::Deref for Temp {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn dir(name: &str) -> Temp {
        let p = std::env::temp_dir().join(format!("sprawl-dbedit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Temp(p)
    }

    fn open(path: &Path) -> Connection {
        Connection::open(path).unwrap()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// Firefox's places.sqlite tables and indexes, as Firefox itself creates them (see the file's
    /// own header: nsPlacesTables.h and nsPlacesIndexes.h, mozilla-central, fetched 2026-11-23).
    const PLACES: &str = include_str!("firefox_places.sql");

    /// A bookmarked page (with a visit, an annotation, an input-history row, a newtab row and a
    /// metadata row that refers to the page that goes); a page only a keyword holds; a page only
    /// an annotation holds (a download); two pages nothing holds; origins for all of them; a
    /// metadata search query used by one page only; and Firefox's cached origin frecency keys.
    fn places_db(path: &Path, version: i64) -> Connection {
        let c = open(path);
        c.execute_batch(PLACES).unwrap();
        c.pragma_update(None, "user_version", version).unwrap();
        c.execute_batch(
            r#"
INSERT INTO moz_origins (id, prefix, host, frecency) VALUES
  (1, 'https://', 'kept.example', 100),
  (2, 'https://', 'gone.example', 50),
  (3, 'https://', 'keyword.example', 40),
  (4, 'https://', 'download.example', 30);
INSERT INTO moz_places (id, url, title, visit_count, last_visit_date, origin_id, foreign_count) VALUES
  (1, 'https://kept.example/a', 'Kept', 2, 200, 1, 1),
  (2, 'https://gone.example/b', 'Gone', 3, 500, 2, 0),
  (3, 'https://gone.example/c', 'Typed', 0, NULL, 2, 0),
  (4, 'https://keyword.example/d', 'Keyword', 1, 600, 3, 1),
  (5, 'https://download.example/file', 'Download', 1, 700, 4, 0);
INSERT INTO moz_historyvisits (id, place_id, visit_date, visit_type) VALUES
  (1, 1, 100, 1), (2, 1, 200, 1),
  (3, 2, 300, 1), (4, 2, 400, 1), (5, 2, 500, 1),
  (6, 4, 600, 2), (7, 5, 700, 7);
INSERT INTO moz_bookmarks (id, type, fk, parent, position, title) VALUES (3, 2, NULL, 0, 0, 'Toolbar'), (10, 1, 1, 3, 0, 'Kept');
INSERT INTO moz_keywords (id, keyword, place_id) VALUES (1, 'kw', 4);
INSERT INTO moz_anno_attributes (id, name) VALUES (1, 'bookmarkProperties/description'), (2, 'downloads/destinationFileURI');
INSERT INTO moz_annos (id, place_id, anno_attribute_id, content) VALUES
  (1, 1, 1, 'a note on the page that stays'),
  (2, 2, 1, 'a note on a page that goes'),
  (3, 5, 2, 'C:\Users\sam\Downloads\file.zip');
INSERT INTO moz_inputhistory (place_id, input, use_count) VALUES (1, 'ke', 2), (2, 'go', 3);
INSERT INTO moz_places_extra (place_id, sync_json) VALUES (1, '{}'), (2, '{}');
INSERT INTO moz_places_metadata_search_queries (id, terms) VALUES (1, 'gone query'), (2, 'kept query');
INSERT INTO moz_places_metadata (id, place_id, referrer_place_id, created_at, search_query_id) VALUES
  (1, 2, NULL, 1, 1),
  (2, 1, 2, 2, NULL),
  (3, 1, NULL, 3, 2);
INSERT INTO moz_newtab_shortcuts_interaction (id, place_id, event_type, timestamp_s, pinned, tile_position) VALUES
  (1, 1, 0, 1000, 0, 0),
  (2, 2, 0, 1001, 0, 1);
INSERT INTO moz_meta (key, value) VALUES ('origin_frecency_count', '4'), ('origin_frecency_sum', '220'), ('origin_alt_frecency_sum', '5'), ('other', 'stays');
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

    /// Firefox history: every visit goes; the pages that aren't bookmarked, keyed by a keyword or
    /// held by anything else (a download's annotation) go with them; the rows keyed to those
    /// pages go too; and what stays is left with no history and with its ranking to be rebuilt.
    #[test]
    fn firefox_history_rows_go_and_what_holds_a_page_stays() {
        let base = dir("history");
        let db_path = base.join("places.sqlite");
        places_db(&db_path, 89);
        let before = std::fs::read(&db_path).unwrap();
        let backups = base.join("backups").join("privacy.firefox-history").join(stamp().unwrap());
        let rows = apply(Edit::FirefoxHistory, &db_path, &[], &backups, "Firefox", &Spare::default()).unwrap();
        assert_eq!(rows, 23, "7 visits, 3 pages, and the rows keyed to them");
        let c = open(&db_path);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_historyvisits"), 0);
        let left: Vec<i64> = { let mut s = c.prepare("SELECT id FROM moz_places ORDER BY id").unwrap(); s.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect() };
        assert_eq!(left, [1, 4], "the bookmarked and the keyworded page stay; a download's page is held by nothing Firefox counts");
        let (visits, last): (i64, Option<i64>) = c.query_row("SELECT visit_count, last_visit_date FROM moz_places WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((visits, last), (0, None), "the kept page keeps no history count");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_bookmarks WHERE fk = 1"), 1, "the bookmark stays");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_bookmarks WHERE type = 2"), 1, "the folder stays");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_keywords WHERE place_id = 4"), 1, "the keyword stays");
        let notes: Vec<String> = { let mut s = c.prepare("SELECT content FROM moz_annos ORDER BY id").unwrap(); s.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect() };
        assert_eq!(notes, ["a note on the page that stays"], "the kept page's note stays; the download's destination went with its page");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_places_metadata"), 0, "no page-interaction record survives");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_places_metadata_search_queries"), 0, "no search term survives");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_newtab_shortcuts_interaction"), 0, "no newtab click survives");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_inputhistory"), 1, "the kept page's typed text stays (Firefox's clear keeps it too)");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_origins"), 2, "only origins a remaining page uses");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_places WHERE recalc_frecency = 1 AND recalc_alt_frecency = 1"), 2, "kept pages ask for their ranking to be worked out again");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_origins WHERE recalc_frecency = 1 AND recalc_alt_frecency = 1"), 2, "and so do their origins");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_meta WHERE key LIKE 'origin_%frecency%'"), 0, "Firefox rebuilds its origin sums");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_meta"), 1, "and leaves other keys");
        drop(c);
        // the copy is the database exactly as it was, and it is kept
        assert_eq!(std::fs::read(backups.join("places.sqlite")).unwrap(), before, "the copy is byte for byte the original");
        // and running it again takes out nothing, and writes nothing
        let after = std::fs::read(&db_path).unwrap();
        assert_eq!(apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups").join("privacy.firefox-history").join(stamp().unwrap()), "Firefox", &Spare::default()).unwrap(), 0);
        assert_eq!(std::fs::read(&db_path).unwrap(), after, "a run with nothing to take out leaves the file's bytes as they were");
    }

    /// No row keyed to a place is left pointing at a page that is gone, in any table that has
    /// one, and the bookmarked and keyworded pages are still there.
    #[test]
    fn no_row_keyed_to_a_missing_place_is_left() {
        let base = dir("orphans");
        let db_path = base.join("places.sqlite");
        places_db(&db_path, 89);
        apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups"), "Firefox", &Spare::default()).unwrap();
        let c = open(&db_path);
        let tables: Vec<String> = { let mut s = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").unwrap(); s.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect() };
        for t in tables {
            let cols: Vec<String> = { let mut s = c.prepare(&format!("PRAGMA table_info({t})")).unwrap(); s.query_map([], |r| r.get::<_, String>(1)).unwrap().map(|r| r.unwrap()).collect() };
            for (col, parent) in [("place_id", "moz_places"), ("visit_id", "moz_historyvisits"), ("origin_id", "moz_origins")] {
                if cols.iter().any(|x| x == col) {
                    assert_eq!(count(&c, &format!("SELECT COUNT(*) FROM {t} WHERE {col} NOT IN (SELECT id FROM {parent})")), 0, "{t}.{col} left a row pointing at a missing row");
                }
            }
            if t == "moz_bookmarks" {
                assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_bookmarks WHERE fk IS NOT NULL AND fk NOT IN (SELECT id FROM moz_places)"), 0, "a bookmark pointing at a missing page");
            }
            if t == "moz_places_metadata" {
                assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_places_metadata WHERE referrer_place_id IS NOT NULL AND referrer_place_id NOT IN (SELECT id FROM moz_places)"), 0, "a referrer pointing at a missing page");
            }
        }
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_places WHERE id IN (1, 4)"), 2);
    }

    /// Firefox form history: every entry goes, and the deleted-entry table (Firefox's own sync
    /// tombstones) is left alone.
    #[test]
    fn firefox_form_history_is_emptied() {
        let base = dir("forms");
        let db_path = base.join("formhistory.sqlite");
        forms_db(&db_path);
        let backups = base.join("backups");
        let rows = apply(Edit::FirefoxForms, &db_path, &[], &backups, "Firefox", &Spare::default()).unwrap();
        assert_eq!(rows, 2, "the two entries; the tombstones are not rows this part removes");
        let c = open(&db_path);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_formhistory"), 0);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_deleted_formhistory"), 1, "Firefox's sync tombstones stay");
        // without the deleted table the part still works
        drop(c);
        let plain = base.join("plain.sqlite");
        { let c = open(&plain); c.execute_batch("CREATE TABLE moz_formhistory (id INTEGER PRIMARY KEY, fieldname TEXT NOT NULL, value TEXT NOT NULL);").unwrap(); c.pragma_update(None, "user_version", 4).unwrap(); c.execute_batch("INSERT INTO moz_formhistory (fieldname, value) VALUES ('a', 'b')").unwrap(); }
        assert_eq!(apply(Edit::FirefoxForms, &plain, &[], &base.join("bk2"), "Firefox", &Spare::default()).unwrap(), 1);
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

    }

    /// Without a checked copy nothing is edited: a backup folder that can't be made stops it,
    /// and the database is exactly as it was.
    #[test]
    fn no_backup_no_edit() {
        let base = dir("nobackup");
        let db_path = base.join("places.sqlite");
        places_db(&db_path, 89);
        let before = std::fs::read(&db_path).unwrap();
        // a file where the backup folder should be
        std::fs::write(base.join("backups"), b"not a folder").unwrap();
        let err = apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups").join("part"), "Firefox", &Spare::default()).unwrap_err();
        assert!(err.contains("backup folder"), "{err}");
        assert_eq!(std::fs::read(&db_path).unwrap(), before);
        assert_eq!(open(&db_path).query_row("SELECT COUNT(*) FROM moz_historyvisits", [], |r| r.get::<_, i64>(0)).unwrap(), 7);

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
        assert!(apply(Edit::FirefoxHistory, &base.join("new.sqlite"), &[], &base.join("bk82"), "Firefox", &Spare::default()).is_ok());
        // one below each end is refused too
        places_db(&base.join("too-old.sqlite"), 51);
        let before = std::fs::read(base.join("too-old.sqlite")).unwrap();
        let err = apply(Edit::FirefoxHistory, &base.join("too-old.sqlite"), &[], &base.join("bk51"), "Firefox", &Spare::default()).unwrap_err();
        assert!(err.contains("isn't one Sprawl knows how to edit"), "{err}");
        assert_eq!(std::fs::read(base.join("too-old.sqlite")).unwrap(), before);
        places_db(&base.join("too-new.sqlite"), 90);
        assert!(apply(Edit::FirefoxHistory, &base.join("too-new.sqlite"), &[], &base.join("bk83"), "Firefox", &Spare::default()).is_err());

    }

    /// A failure part-way through the deletes rolls the whole transaction back.
    #[test]
    fn a_failure_mid_way_rolls_back() {
        let base = dir("rollback");
        let db_path = base.join("places.sqlite");
        {
            let c = places_db(&db_path, 89);
            // the last statement (on moz_origins) fails after the visits and pages have gone
            c.execute_batch("CREATE TRIGGER stop_origins BEFORE DELETE ON moz_origins BEGIN SELECT RAISE(ABORT, 'no'); END;").unwrap();
        }
        let err = apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups"), "Firefox", &Spare::default()).unwrap_err();
        assert!(err.contains("rolled back"), "{err}");
        let c = open(&db_path);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_historyvisits"), 7);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_places"), 5);
        assert_eq!(c.query_row("SELECT visit_count FROM moz_places WHERE id = 1", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_annos"), 3);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_origins"), 4);

    }

    /// A file with another name (a hard link) is refused, and one that changed since the scan.
    #[test]
    fn a_file_with_another_name_or_changed_is_left_alone() {
        let base = dir("links");
        let db_path = base.join("places.sqlite");
        places_db(&db_path, 89);
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

    }

    /// Measuring for the review is read-only and agrees with what the edit does, and it leaves
    /// the database's bytes as they were.
    #[test]
    fn counting_is_read_only_and_agrees_with_the_edit() {
        let base = dir("count");
        let db_path = base.join("places.sqlite");
        places_db(&db_path, 89);
        let before = std::fs::read(&db_path).unwrap();
        assert_eq!(rows(Edit::FirefoxHistory, &db_path), Some(23));
        assert_eq!(std::fs::read(&db_path).unwrap(), before, "measuring wrote nothing");
        assert_eq!(rows(Edit::FirefoxHistory, &base.join("missing.sqlite")), None);
        // and it agrees with the edit
        assert_eq!(apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups"), "Firefox", &Spare::default()).unwrap(), 23);

    }

    /// A database in WAL mode (as Firefox and Chromium keep theirs) is edited the same way: the
    /// write-ahead log is SQLite's to make, and the deletes still go in one transaction.
    #[test]
    fn a_write_ahead_log_database_is_edited() {
        let base = dir("wal");
        let db_path = base.join("places.sqlite");
        {
            let c = places_db(&db_path, 89);
            c.pragma_update(None, "journal_mode", "WAL").unwrap();
        }
        let rows = apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups"), "Firefox", &Spare::default()).unwrap();
        assert_eq!(rows, 23);
        let c = open(&db_path);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_historyvisits"), 0);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_places"), 2);

    }

    /// Measuring a WAL database read-only works and leaves the profile as it was (no `-shm` or
    /// `-wal` made by the review), so the scan's "changed since" check isn't tripped by Sprawl.
    #[test]
    fn measuring_a_wal_database_touches_nothing() {
        let base = dir("walcount");
        let db_path = base.join("places.sqlite");
        {
            let c = places_db(&db_path, 89);
            c.pragma_update(None, "journal_mode", "WAL").unwrap();
        }
        let names = |p: &Path| { let mut v: Vec<String> = std::fs::read_dir(p).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect(); v.sort(); v };
        let before = names(&base);
        assert_eq!(rows(Edit::FirefoxHistory, &db_path), Some(23), "the main file is read");
        assert_eq!(names(&base), before, "nothing was made beside it");

    }

    /// The review's finding for an edit: rows, no bytes, the action names the database, the
    /// preview lists it, and the safety check takes it as the part's own file only.
    #[test]
    fn the_review_offers_rows_not_bytes() {
        let (base, places) = crate::privacy::tests::fixture("editfind");
        let fp = places.roaming.join(r"Mozilla\Firefox\Profiles\abcd1234.default-release");
        let db = fp.join("places.sqlite");
        std::fs::remove_file(&db).unwrap();
        places_db(&db, 89);
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let found = crate::privacy::find_in(&tree, &places);
        let f = found.iter().find(|f| f.title == "Firefox: Browsing history (default-release)").expect("the history part");
        assert_eq!(f.bytes, 0, "a privacy edit is never counted as space");
        assert!(
            matches!(&f.action, Some(crate::finding::Action::Edit { part, file, rows }) if part == "privacy.firefox/history" && file == &db && *rows == 23),
            "{:?}",
            f.action
        );
        assert!(f.evidence.iter().any(|e| e.contains("At least 23 rows would go")), "{:?}", f.evidence);
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
        assert_eq!((preview.bytes, preview.files, preview.rows), (0, 1, 23));
        assert_eq!(preview.biggest[0].0, db);

    }

    /// A database from before Firefox tracked recalculation: the ranking is set to -1, which is
    /// Firefox's own "work this out again".
    #[test]
    fn without_recalc_columns_frecency_is_set_to_rebuild() {
        let base = dir("frecency");
        let db_path = base.join("places.sqlite");
        {
            let c = places_db(&db_path, 89);
            c.execute_batch("ALTER TABLE moz_places DROP COLUMN recalc_frecency; ALTER TABLE moz_places DROP COLUMN recalc_alt_frecency; ALTER TABLE moz_origins DROP COLUMN recalc_frecency; ALTER TABLE moz_origins DROP COLUMN recalc_alt_frecency;").unwrap();
        }
        apply(Edit::FirefoxHistory, &db_path, &[], &base.join("backups"), "Firefox", &Spare::default()).unwrap();
        let c = open(&db_path);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_places WHERE frecency = -1"), 2, "every kept page");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_origins WHERE frecency = 0"), 2, "every kept origin asks to be worked out again");
    }

    /// The browser's lock (Firefox's `parent.lock`, Chromium's `lockfile`): free when the file is
    /// missing or no one holds it, held otherwise.
    #[test]
    fn the_browser_lock_is_read_as_the_browser_holds_it() {
        use std::os::windows::fs::OpenOptionsExt;
        let base = dir("lock");
        for name in ["parent.lock", "lockfile"] {
            let lock = base.join(name);
            let who = if name == "parent.lock" { "Firefox" } else { "Chrome" };
            assert!(browser_lock_free(&lock, who).is_ok(), "{name}: no lock file at all");
            std::fs::write(&lock, b"").unwrap();
            assert!(browser_lock_free(&lock, who).is_ok(), "{name}: a lock file nobody holds");
            let held = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&lock).unwrap();
            assert!(browser_lock_free(&lock, who).is_err(), "{name}: a lock the browser holds");
            drop(held);
            assert!(browser_lock_free(&lock, who).is_ok(), "{name}: the lock let go");
            std::fs::remove_file(&lock).unwrap();
        }
    }

    /// The lock file each browser holds while it runs, found from the database's own place.
    #[test]
    fn the_lock_is_looked_for_where_its_browser_keeps_it() {
        let ff = Path::new(r"C:\Users\sam\AppData\Roaming\Mozilla\Firefox\Profiles\abc.default\places.sqlite");
        assert_eq!(lock_path(crate::privacy::Kind::Firefox, ff).unwrap(), Path::new(r"C:\Users\sam\AppData\Roaming\Mozilla\Firefox\Profiles\abc.default\parent.lock"));
        let ch = Path::new(r"C:\Users\sam\AppData\Local\Google\Chrome\User Data\Default\Web Data");
        assert_eq!(lock_path(crate::privacy::Kind::Chromium, ch).unwrap(), Path::new(r"C:\Users\sam\AppData\Local\Google\Chrome\User Data\lockfile"));
    }

    /// A table Sprawl doesn't know, with a foreign key to a page that goes: the edit rolls back
    /// and the database is left as it was.
    #[test]
    fn a_table_keyed_to_a_page_that_is_not_cleaned_rolls_back() {
        let base = dir("fkcheck");
        let db_path = base.join("places.sqlite");
        {
            let c = places_db(&db_path, 89);
            c.execute_batch("CREATE TABLE extra_refs (id INTEGER PRIMARY KEY, place_id INTEGER REFERENCES moz_places(id)); INSERT INTO extra_refs (id, place_id) VALUES (1, 2);").unwrap();
        }
        let before = std::fs::read(&db_path).unwrap();
        let backups = base.join("backups");
        let err = apply(Edit::FirefoxHistory, &db_path, &[], &backups, "Firefox", &Spare::default()).unwrap_err();
        assert!(err.contains("this version of Firefox isn't one Sprawl knows how to edit"), "{err}");
        assert_eq!(std::fs::read(&db_path).unwrap(), before, "the transaction rolled back");
        assert!(!backups.exists(), "the copy of the failed try is gone");
    }

    /// A copy that can't be removed is reported, not silently left, and the item still fails
    /// for its own reason.
    #[test]
    fn a_copy_that_cannot_be_removed_is_reported() {
        use std::os::windows::fs::OpenOptionsExt;
        let base = dir("stuck");
        let db_path = base.join("places.sqlite");
        places_db(&db_path, 89);
        let folder = base.join("copy");
        std::fs::create_dir_all(&folder).unwrap();
        let stuck = folder.join("stuck");
        std::fs::write(&stuck, b"x").unwrap();
        // held with no sharing: the folder cannot be emptied, so it cannot be removed
        let held = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&stuck).unwrap();
        let mut notes = Vec::new();
        let guard = || Err::<(), String>("Firefox started while the database was being edited".into());
        let err = apply_guarded(Edit::FirefoxHistory, &db_path, &[], &folder, "Firefox", &Spare::default(), &guard, &mut notes).unwrap_err();
        assert!(err.contains("started"), "{err}");
        assert!(notes.iter().any(|n| n.contains("couldn't be removed")), "{notes:?}");
        drop(held);
    }

    /// Copies are named so they sort by when they were made, and only such folders are pruned.
    #[test]
    fn copy_folders_are_named_so_the_newest_sort_last() {
        let name = stamp().unwrap();
        assert_eq!(name.len(), 26, "{name}");
        assert!(is_stamp(&name));
        assert!(!is_stamp("2026-11-23") && !is_stamp("a folder") && !is_stamp(""));
        assert!("2026-11-23-00-00-00-000001" < "2026-11-23-00-00-00-000002");
    }

    /// Only the newest few copies are kept, and the copy just made is never one of the ones
    /// that go - even when every older copy is stamped in the future (a clock that ran ahead
    /// once, then was put right).
    #[test]
    fn only_the_newest_copies_are_kept() {
        let base = dir("prune");
        let db_path = base.join("formhistory.sqlite");
        forms_db(&db_path);
        let part_dir = base.join("backups").join("privacy.firefox-forms");
        for i in 0..KEEP_BACKUPS {
            let n = format!("2999-01-01-00-00-{i:02}-000000");
            std::fs::create_dir_all(part_dir.join(&n)).unwrap();
            std::fs::write(part_dir.join(&n).join("formhistory.sqlite"), b"an old copy").unwrap();
        }
        let now = stamp().unwrap();
        apply(Edit::FirefoxForms, &db_path, &[], &part_dir.join(&now), "Firefox", &Spare::default()).unwrap();
        let mut left: Vec<String> = std::fs::read_dir(&part_dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert!(left.contains(&now), "the copy just made is kept");
        assert_eq!(left.len(), KEEP_BACKUPS, "the oldest copy went");
        assert!(!left.contains(&"2999-01-01-00-00-00-000000".to_string()), "the oldest was the future one");
    }

    /// A copy older than 30 days goes even when there are only a few.
    #[test]
    fn copies_older_than_thirty_days_go() {
        let base = dir("stale");
        let db_path = base.join("formhistory.sqlite");
        forms_db(&db_path);
        let part_dir = base.join("backups").join("privacy.firefox-forms");
        let old = stamp_before(40 * 86_400).unwrap();
        std::fs::create_dir_all(part_dir.join(&old)).unwrap();
        std::fs::write(part_dir.join(&old).join("formhistory.sqlite"), b"a stale copy").unwrap();
        let now = stamp().unwrap();
        apply(Edit::FirefoxForms, &db_path, &[], &part_dir.join(&now), "Firefox", &Spare::default()).unwrap();
        assert!(!part_dir.join(&old).exists(), "a copy 40 days old is dropped");
        assert!(part_dir.join(&now).exists());
    }

    /// A browser that starts once the edit has begun is caught twice - after the transaction
    /// opens and before it is kept - and either time the database is left as it was, with no
    /// copy of the failed try left behind.
    #[test]
    fn a_browser_that_starts_mid_edit_rolls_back() {
        let base = dir("guard");
        let db_path = base.join("places.sqlite");
        places_db(&db_path, 89);
        let before = std::fs::read(&db_path).unwrap();
        let calls = std::cell::Cell::new(0);
        let started = || {
            calls.set(calls.get() + 1);
            Err("Firefox started while the database was being edited".to_string())
        };
        let first = base.join("copy1");
        let mut notes = Vec::new();
        let err = apply_guarded(Edit::FirefoxHistory, &db_path, &[], &first, "Firefox", &Spare::default(), &started, &mut notes).unwrap_err();
        assert!(err.contains("started"), "{err}");
        assert_eq!(std::fs::read(&db_path).unwrap(), before, "nothing was written");
        assert!(!first.exists(), "the copy of the failed try is gone");
        // the second check, just before the commit: the deletes are rolled back
        let calls = std::cell::Cell::new(0);
        let late = || {
            calls.set(calls.get() + 1);
            if calls.get() == 1 { Ok(()) } else { Err("Firefox started while the database was being edited".to_string()) }
        };
        let second = base.join("copy2");
        let mut notes = Vec::new();
        let err = apply_guarded(Edit::FirefoxHistory, &db_path, &[], &second, "Firefox", &Spare::default(), &late, &mut notes).unwrap_err();
        assert!(err.contains("started"), "{err}");
        assert!(!second.exists(), "the copy of the failed try is gone");
        let c = open(&db_path);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_historyvisits"), 7, "the visits are all back");
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_places"), 5);
    }

    /// A database the browser was killed on: its log holds rows that never reached the file, and
    /// the copy of it (with the log and the shared index) is read, edited and backed up whole.
    #[test]
    fn a_live_write_ahead_log_is_read_and_backed_up() {
        let base = dir("livewal");
        let src = base.join("live");
        std::fs::create_dir_all(&src).unwrap();
        let db = src.join("places.sqlite");
        let c = open(&db);
        c.execute_batch(PLACES).unwrap();
        c.pragma_update(None, "user_version", 61).unwrap();
        c.pragma_update(None, "journal_mode", "WAL").unwrap();
        c.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        c.execute_batch(
            "INSERT INTO moz_places (id, url, title) VALUES (1, 'https://kept.example/a', 'Kept'), (2, 'https://gone.example/b', 'Gone');
             INSERT INTO moz_historyvisits (id, place_id, visit_date) VALUES (1, 1, 100), (2, 2, 200);
             INSERT INTO moz_bookmarks (id, type, fk, parent, position, title) VALUES (10, 1, 1, 0, 0, 'Kept');",
        )
        .unwrap();
        // copied while the connection is still open: the browser "was killed", so nothing was
        // checkpointed and the log is all there is
        let dst = base.join("copy");
        std::fs::create_dir_all(&dst).unwrap();
        for sfx in ["", "-wal", "-shm"] {
            std::fs::copy(src.join(format!("places.sqlite{sfx}")), dst.join(format!("places.sqlite{sfx}"))).unwrap();
        }
        let wal_before = std::fs::read(dst.join("places.sqlite-wal")).unwrap();
        assert!(!wal_before.is_empty(), "the log holds the rows");
        drop(c);
        let copy_db = dst.join("places.sqlite");
        let sides = vec![dst.join("places.sqlite-wal"), dst.join("places.sqlite-shm")];
        let backups = base.join("backups");
        let rows = apply(Edit::FirefoxHistory, &copy_db, &sides, &backups, "Firefox", &Spare::default()).unwrap();
        assert_eq!(rows, 3, "the log's two visits and the one page it held");
        let c = open(&copy_db);
        assert_eq!(count(&c, "SELECT COUNT(*) FROM moz_historyvisits"), 0, "the log's visits went");
        let left: Vec<i64> = { let mut s = c.prepare("SELECT id FROM moz_places").unwrap(); s.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect() };
        assert_eq!(left, [1], "the bookmarked page from the log stays");
        drop(c);
        assert_eq!(std::fs::read(backups.join("places.sqlite-wal")).unwrap(), wal_before, "the copy holds the log byte for byte");
        assert!(backups.join("places.sqlite-shm").exists(), "and the shared index");
    }

    /// A corrupted page is refused on the read-only check, before any write handle is taken, and
    /// the file is left exactly as it was.
    #[test]
    fn a_corrupted_database_is_refused_untouched() {
        use std::io::{Seek, SeekFrom, Write};
        let base = dir("corrupt");
        let db_path = base.join("places.sqlite");
        let page = { let c = places_db(&db_path, 89); let p: i64 = c.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap(); drop(c); p as u64 };
        {
            let mut f = std::fs::OpenOptions::new().write(true).open(&db_path).unwrap();
            for n in 1..=2u64 {
                f.seek(SeekFrom::Start(page * n)).unwrap();
                f.write_all(&[0xFF; 32]).unwrap();
            }
            f.sync_all().unwrap();
        }
        let before = std::fs::read(&db_path).unwrap();
        let backups = base.join("backups");
        let err = apply(Edit::FirefoxHistory, &db_path, &[], &backups, "Firefox", &Spare::default()).unwrap_err();
        assert!(err.contains("didn't pass"), "{err}");
        assert_eq!(std::fs::read(&db_path).unwrap(), before, "the file is as it was");
        assert!(!backups.exists(), "the failed try keeps no copy");
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

    }
}