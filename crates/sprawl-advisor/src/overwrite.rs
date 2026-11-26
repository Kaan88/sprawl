//! Overwrite and delete (F9): overwrite a file's data once, then delete it; a folder's files
//! that way, then its empty folders.
//!
//! What it promises, and what it does not:
//! - One pass of random data over each file: the parts that are on the disk (a sparse file's
//!   holes are left out), its named streams, then its ordinary content; then each stream and the
//!   file are flushed, set to 0 bytes, the file renamed to a random name in its own folder, and
//!   only then deleted. A folder's own named streams are written over the same way before the
//!   folder goes. Nothing goes to the Recycle Bin.
//! - Nothing here claims the data is unrecoverable. On an SSD the drive moves writes around; a
//!   compressed, sparse, encrypted or WOF file and a copy-on-write volume may put the new data
//!   somewhere else; a cloud-synced folder may keep older versions online; old names can stay in
//!   the file system's own records (the change journal); and System Restore, shadow copies and
//!   the Recycle Bin may hold older copies. The window says each of these before anything runs.
//! - It is at least as careful as a delete: every item is opened once, through a handle with
//!   links not followed, and every check is made through that handle before anything about the
//!   file is changed: a path that leads elsewhere, a link or a junction, a cloud or offline file,
//!   a file with another name (a hard link: overwriting it would empty that name too), the
//!   never-list, anything created or changed after the scan, and the user's Keep list.
//!   Only then is a read-only mark cleared, and a failure puts it back. A file's length and
//!   last-write time are compared again just before the first byte, so one changed while the
//!   checks ran stays. A WOF-compressed file is written out in full like a compressed one. A file
//!   is left alone when writing over it would need more room than the drive has to spare.
//! - Stop finishes the file it is on (a half-overwritten file would be neither kept nor gone),
//!   then stops.
//!
//! Free-space wiping, which is Windows' own `cipher /w:` and not an overwrite of any file, lives
//! at the end of this module: [`review_wipe`], [`wipe_plan`], [`wipe_free_space`] and the
//! volume facts both features' questions use ([`filesystem`], [`solid_state`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::clean::{same_path, win, Spare};

const DIRECTORY: u32 = 0x10;
const READONLY: u32 = 0x1;
const SPARSE: u32 = 0x200;
const REPARSE: u32 = 0x400;
const COMPRESSED: u32 = 0x800;
/// IO_REPARSE_TAG_WOF: a Windows-compressed file is data behind a reparse point, not a link.
const WOF_TAG: u32 = 0x8000_0017;
/// Offline, recall-on-open (cloud placeholder) and sparse cloud bits: a file that isn't here.
pub const NOT_LOCAL: u32 = 0x1000 | 0x0004_0000 | 0x0040_0000;
/// Room kept free on the drive when writing over a file: a compressed one grows as it is
/// written, and the drive must not be filled to its last byte.
const FREE_MARGIN: u64 = 1 << 30;

/// What one overwrite left, in the same shape as a clean-up's outcome.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Bytes written over (what it actually wrote, for the record).
    pub overwritten: u64,
    /// Files overwritten and deleted.
    pub files: u64,
    /// Folders removed (their files having gone).
    pub folders: u64,
    /// The files' own lengths, for the window's progress and "what this frees" figure.
    pub bytes: u64,
    /// What was left, and why (the first few).
    pub problems: Vec<String>,
    /// Problems past the ones kept: "and N more were left alone".
    pub more_problems: u64,
    /// Files whose data is damaged by a failed attempt: partly overwritten, or overwritten but
    /// not deleted. Never cut: each one is a file the user must know about.
    pub damaged: Vec<String>,
    /// Items left because they were created or changed after the scan.
    pub kept_new: u64,
    /// Items left because the user chose to keep them.
    pub kept_chosen: u64,
    /// Items left because they are on the never-list.
    pub kept_protected: u64,
    pub stopped: bool,
}

impl Outcome {
    fn problem(&mut self, p: &Path, why: impl std::fmt::Display) {
        if self.problems.len() < 8 {
            self.problems.push(format!("{}: {why}", p.display()));
        } else {
            self.more_problems += 1;
        }
    }

    /// A file that is no longer what it was: it took some of the overwrite and stays where it is.
    fn damage(&mut self, p: &Path, why: impl std::fmt::Display) {
        self.damaged.push(format!("{}: {why}", p.display()));
    }
}

fn never() -> &'static crate::safety::Never {
    static N: std::sync::OnceLock<crate::safety::Never> = std::sync::OnceLock::new();
    N.get_or_init(crate::safety::Never::default_list)
}

/// Overwrite and delete `path` (a file or a folder), leaving what `spare` names. `progress` gets
/// the items' own bytes dealt with so far (the same measure as the window's total). See the
/// module comment for what it promises.
pub fn run(path: &Path, spare: &Spare, stop: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Outcome {
    let mut o = Outcome::default();
    if stop.load(Ordering::Relaxed) {
        o.stopped = true;
        return o;
    }
    // the never-list must be known in full: with a part of it unread nothing may go
    let never = never();
    if !never.complete() {
        o.problem(path, "left alone: Sprawl couldn't work out where Windows keeps its own folders");
        return o;
    }
    let path_buf = match sprawl_core::local::lexical(path) {
        Ok(p) => p,
        Err(e) => {
            o.problem(path, e);
            return o;
        }
    };
    let path = path_buf.as_path();
    // a drive's root as a whole is never offered, and its bookkeeping files stay
    if path.parent().is_none() {
        o.problem(path, "a whole drive cannot be overwritten");
        return o;
    }
    if let Some(p) = never.hit(&path.display().to_string()) {
        o.kept_protected += 1;
        o.problem(path, format!("left alone: it is protected ({p})"));
        return o;
    }
    if let Some(p) = never.path_hit(path) {
        o.kept_protected += 1;
        o.problem(path, format!("left alone: it is protected ({p})"));
        return o;
    }
    if odd_name(path) {
        o.problem(path, "skipped, its name ends in a dot or space");
        return o;
    }
    if spare.keeps(path) {
        o.kept_chosen += 1;
        return o;
    }
    // one handle for every check; a picked file goes through exactly the same ones as a file
    // found inside a folder does
    let (h, info) = match win::open_checked(path) {
        Ok(x) => x,
        Err(e) => {
            o.problem(path, format!("left alone: it couldn't be opened, so it may be in use ({e})"));
            return o;
        }
    };
    if info.attrs & DIRECTORY == 0 {
        scrub_file(path, h, info, spare, &mut o, progress);
        return o;
    }
    if spare.is_new(path, &info) {
        o.kept_new += 1;
        return o;
    }
    if info.attrs & REPARSE != 0 || info.attrs & DIRECTORY == 0 {
        o.problem(path, "left alone: it is not a plain folder any more");
        return o;
    }
    if info.attrs & NOT_LOCAL != 0 {
        o.problem(path, "left alone: a cloud or offline folder");
        return o;
    }
    match win::final_path(&h) {
        Some(real) if same_path(&real, path) => {}
        Some(real) => {
            o.problem(path, format!("now leads to {} instead; left alone", real.display()));
            return o;
        }
        None => {
            o.problem(path, "its real location couldn't be checked; left alone");
            return o;
        }
    }
    let (kids, unread) = match list(path) {
        Ok(x) => x,
        Err(e) => {
            o.problem(path, e);
            return o;
        }
    };
    if unread > 0 {
        o.problem(path, unreadable(unread));
    }
    // `left` says something in this folder is staying there: a kept item, one in use, a link, a
    // subfolder that stayed. A folder whose stream is written over and that then stays would be
    // damaged for nothing, so a folder with anything left in it is neither scrubbed nor deleted.
    let mut stack = vec![Frame { h, info, path: path.to_path_buf(), kids: kids.into_iter(), left: unread > 0 }];
    while let Some(top) = stack.last_mut() {
        if stop.load(Ordering::Relaxed) {
            o.stopped = true;
            return o;
        }
        let Some(k) = top.kids.next() else {
            let f = stack.pop().unwrap();
            let mut stays = f.left;
            if !stays {
                match scrub_folder_streams(&f.path, &f.h, &f.info, &mut o) {
                    FolderStreams::Left => stays = true,
                    done => match win::delete(&f.h) {
                        Ok(()) => o.folders += 1,
                        Err(e) => {
                            stays = true;
                            match done {
                                // its own streams are already written over: say so
                                FolderStreams::Written => o.damage(&f.path, format!("its own streams were overwritten once, but the folder couldn't be deleted ({e}); it stays")),
                                _ => o.problem(&f.path, format!("couldn't be deleted ({e})")),
                            }
                        }
                    },
                }
            }
            if stays {
                if let Some(parent) = stack.last_mut() {
                    parent.left = true;
                }
            }
            continue;
        };
        if odd_name(&k) {
            o.problem(&k, "skipped, its name ends in a dot or space");
            top.left = true;
            continue;
        }
        if let Some(p) = never.path_hit(&k) {
            o.kept_protected += 1;
            o.problem(&k, format!("left alone: it is protected ({p})"));
            top.left = true;
            continue;
        }
        if spare.keeps(&k) {
            o.kept_chosen += 1;
            top.left = true;
            continue;
        }
        // one handle for the item itself, links not followed, no other program writing it
        let (h, info) = match win::open_checked(&k) {
            Ok(x) => x,
            Err(e) => {
                o.problem(&k, format!("left alone: it couldn't be opened, so it may be in use ({e})"));
                top.left = true;
                continue;
            }
        };
        if spare.is_new(&k, &info) {
            o.kept_new += 1;
            top.left = true;
            continue;
        }
        if info.attrs & REPARSE != 0 && info.tag != WOF_TAG {
            // a link is never followed; a WOF file is data behind a hidden reparse point and is
            // written out like a compressed one
            o.problem(&k, "left alone: a link or junction");
            top.left = true;
            continue;
        }
        if info.attrs & DIRECTORY == 0 {
            let before = o.files;
            scrub_file(&k, h, info, spare, &mut o, progress);
            if o.files == before {
                top.left = true;
            }
            continue;
        }
        if info.attrs & NOT_LOCAL != 0 {
            o.problem(&k, "left alone: a cloud or offline folder");
            top.left = true;
            continue;
        }
        // it stays open (and so can't be swapped for a link) while its contents go
        match win::final_path(&h) {
            Some(real) if same_path(&real, &k) => {}
            Some(real) => {
                o.problem(&k, format!("now leads to {} instead; left alone", real.display()));
                top.left = true;
                continue;
            }
            None => {
                o.problem(&k, "its real location couldn't be checked; left alone");
                top.left = true;
                continue;
            }
        }
        match list(&k) {
            Ok((kids, unread)) => {
                if unread > 0 {
                    o.problem(&k, unreadable(unread));
                }
                stack.push(Frame { h, info, path: k, kids: kids.into_iter(), left: unread > 0 });
            }
            Err(e) => {
                o.problem(&k, e);
                top.left = true;
            }
        }
    }
    o
}

/// One open folder, its checked info and what is left to do in it. `left` is whether anything in
/// it stays (so it must not have its own streams written over, and must not be deleted).
struct Frame {
    h: win::Handle,
    info: win::Info,
    path: PathBuf,
    kids: std::vec::IntoIter<PathBuf>,
    left: bool,
}

/// What will be written over: the parts of the file on the disk (a sparse file's holes are left
/// out), or its whole length (a compressed file is written out in full as it is overwritten).
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    Ranges(Vec<(u64, u64)>),
    Whole(u64),
}

impl Plan {
    fn total(&self) -> u64 {
        match self {
            Plan::Ranges(r) => r.iter().map(|(_, n)| n).sum(),
            Plan::Whole(n) => *n,
        }
    }
}

/// Whether the file's data is kept behind an external backing provider (a Windows, WOF,
/// compressed file). An ordinary open hides its reparse point, so the checked handle is asked
/// as well as its tag.
fn wof(h: &win::Handle, info: &win::Info) -> bool {
    (info.attrs & REPARSE != 0 && info.tag == WOF_TAG) || win::wof_backed(h)
}

/// The parts of a file that will be written over, from the open, checked handle. A WOF file and
/// an NTFS-compressed one are written out in full (that is what turning them into plain data
/// costs); a sparse file only where its data is; a plain one over the space it already takes.
fn plan_for(h: &win::Handle, info: &win::Info) -> Result<Plan, String> {
    if wof(h, info) || info.attrs & COMPRESSED != 0 {
        Ok(Plan::Whole(info.size))
    } else if info.attrs & SPARSE != 0 {
        let ranges = win::allocated_ranges(h, info.size).map_err(|e| format!("left alone: the parts of it on the disk couldn't be read ({e})"))?;
        Ok(Plan::Ranges(ranges))
    } else {
        Ok(Plan::Whole(win::allocation(h, info.size)))
    }
}

/// The room a plan needs beyond what the file already takes: none for a plain or sparse file
/// (it is written where it already is), about its whole length for a compressed or WOF one,
/// which grows to its full size as it is written over.
fn extra_room(h: &win::Handle, plan: &Plan) -> u64 {
    let already = win::standard(h).map(|(alloc, _)| alloc).unwrap_or(0);
    plan.total().saturating_sub(already)
}

/// Why there isn't room for `extra` more bytes on the drive holding `p`, if there isn't: it must
/// fit, and a GB must stay free besides.
fn room_for(p: &Path, extra: u64) -> Option<String> {
    if extra == 0 {
        return None;
    }
    match free_space(p) {
        Some(free) if extra > free.saturating_sub(FREE_MARGIN) => Some(format!(
            "writing over it needs {} more and the drive has {} free, which isn't enough to spare a GB besides",
            bytes(extra),
            bytes(free)
        )),
        Some(_) => None,
        None => Some("how much room is left on its drive couldn't be read".to_string()),
    }
}

/// One file, end to end: every check through its one checked handle, then its streams and its
/// own data overwritten once, flushed, set to 0 bytes, given a random name and deleted. Left as
/// it is with the reason when anything refuses it; a failure once writing began is reported as
/// damage, and the file stays.
fn scrub_file(p: &Path, h: win::Handle, info: win::Info, spare: &Spare, o: &mut Outcome, progress: &mut dyn FnMut(u64)) {
    if info.attrs & DIRECTORY != 0 {
        o.problem(p, "left alone: it became a folder");
        return;
    }
    if info.attrs & REPARSE != 0 && info.tag != WOF_TAG {
        o.problem(p, "left alone: a link or junction");
        return;
    }
    if info.attrs & NOT_LOCAL != 0 {
        o.problem(p, "left alone: a cloud or offline file");
        return;
    }
    if spare.is_new(p, &info) {
        o.kept_new += 1;
        return;
    }
    if info.links != 1 {
        o.problem(p, "left alone: it has another name (a hard link) elsewhere; overwriting would empty that one too");
        return;
    }
    match win::final_path(&h) {
        Some(real) if same_path(&real, p) => {}
        Some(real) => {
            o.problem(p, format!("now leads to {} instead; left alone", real.display()));
            return;
        }
        None => {
            o.problem(p, "its real location couldn't be checked; left alone");
            return;
        }
    }
    let plan = match plan_for(&h, &info) {
        Ok(plan) => plan,
        Err(why) => {
            o.problem(p, why);
            return;
        }
    };
    // its named streams first: all of them are opened before any is written, so a stream that
    // can't be overwritten leaves the whole file as it was
    let names = match win::streams(p, &h) {
        Ok(n) => n,
        Err(e) => {
            o.problem(p, format!("left alone: its named streams couldn't be read ({e})"));
            return;
        }
    };
    let was_readonly = info.attrs & READONLY != 0;
    if was_readonly {
        if let Err(e) = win::clear_readonly(&h, info.attrs) {
            o.problem(p, format!("left alone: its read-only mark couldn't be cleared ({e})"));
            return;
        }
    }
    let mut streams: Vec<(String, win::Handle, win::Info, Plan)> = Vec::new();
    let mut extra = extra_room(&h, &plan);
    for name in names {
        let sp = stream_path(p, &name);
        match win::open_data(&sp) {
            Ok((sh, si)) => {
                // the stream is the same file's (never another's), and its own parts are
                // planned as the file's are
                if si.id != info.id || si.volume != info.volume {
                    if was_readonly {
                        let _ = win::set_readonly(&h, info.attrs);
                    }
                    o.problem(p, format!("left alone: its stream {name} isn't on the file Sprawl checked"));
                    return;
                }
                let plan = match plan_for(&sh, &si) {
                    Ok(plan) => plan,
                    Err(why) => {
                        if was_readonly {
                            let _ = win::set_readonly(&h, info.attrs);
                        }
                        o.problem(p, format!("left alone: its stream {name}: {why}"));
                        return;
                    }
                };
                extra += extra_room(&sh, &plan);
                streams.push((name, sh, si, plan));
            }
            Err(e) => {
                if was_readonly {
                    let _ = win::set_readonly(&h, info.attrs);
                }
                o.problem(p, format!("left alone: its stream {name} couldn't be opened ({e})"));
                return;
            }
        }
    }
    // the room the file and its streams need between them, once
    if let Some(why) = room_for(p, extra) {
        if was_readonly {
            let _ = win::set_readonly(&h, info.attrs);
        }
        o.problem(p, format!("left alone: {why}"));
        return;
    }
    let data = match reopen_data(p, &h, &info) {
        Ok(d) => d,
        Err(why) => {
            if was_readonly {
                let _ = win::set_readonly(&h, info.attrs);
            }
            o.problem(p, why);
            return;
        }
    };
    // the reopen is refused while another program holds it for writing; one that slipped a
    // write in just before is caught by its length and last-write time
    if let Err(why) = unchanged(&data, &info) {
        if was_readonly {
            let _ = win::set_readonly(&h, info.attrs);
        }
        o.problem(p, why);
        return;
    }
    let mut written = 0u64;
    for (name, sh, _si, plan) in &streams {
        if let Err(e) = wipe(sh, plan, &mut written) {
            if was_readonly {
                let _ = win::set_readonly(&h, info.attrs);
            }
            o.damage(p, format!("partly overwritten (its stream {name} couldn't be written: {e}); it is damaged and still takes space"));
            o.overwritten += written;
            return;
        }
        // its stream is made empty through the handle that could write it
        if let Err(e) = win::set_end(sh, 0).and_then(|()| win::flush(sh)) {
            if was_readonly {
                let _ = win::set_readonly(&h, info.attrs);
            }
            o.damage(p, format!("partly overwritten (its stream {name} couldn't be set to 0 bytes: {e}); it is damaged and still takes space"));
            o.overwritten += written;
            return;
        }
    }
    drop(streams);
    if let Err(e) = wipe(&data, &plan, &mut written) {
        if was_readonly {
            let _ = win::set_readonly(&h, info.attrs);
        }
        o.damage(p, format!("partly overwritten ({written} bytes written, then the data couldn't be written: {e}); it is damaged and still takes space"));
        o.overwritten += written;
        return;
    }
    // what was written is on the disk before the file is made short, so a crash can't leave the
    // old data on the disk under a file that says it is empty
    if let Err(e) = win::set_end(&data, 0).and_then(|()| win::flush(&data)) {
        if was_readonly {
            let _ = win::set_readonly(&h, info.attrs);
        }
        o.damage(p, format!("overwritten once, but it couldn't be set to 0 bytes and flushed ({e}); it is damaged and still takes space"));
        o.overwritten += written;
        return;
    }
    drop(data);
    let to = match random_name(p) {
        Some(to) => to,
        None => {
            if was_readonly {
                let _ = win::set_readonly(&h, info.attrs);
            }
            o.damage(p, "overwritten once and set to 0 bytes, but no random name could be made for it; it stays at this path");
            o.overwritten += written;
            return;
        }
    };
    if let Err(e) = win::rename(&h, &to) {
        if was_readonly {
            let _ = win::set_readonly(&h, info.attrs);
        }
        o.damage(p, format!("overwritten once and set to 0 bytes, but it couldn't be given a random name ({e}); it stays at this path"));
        o.overwritten += written;
        return;
    }
    match win::delete(&h) {
        Ok(()) => {
            o.files += 1;
            o.bytes += info.size;
            o.overwritten += written;
            progress(o.bytes);
        }
        // the name went with the delete as it was asked; nothing to report
        Err(e) if e.raw_os_error() == Some(145) => {
            o.files += 1;
            o.bytes += info.size;
            o.overwritten += written;
            progress(o.bytes);
        }
        Err(e) => {
            if was_readonly {
                let _ = win::set_readonly(&h, info.attrs);
            }
            o.overwritten += written;
            o.damage(&to, format!("overwritten once and set to 0 bytes, but it couldn't be deleted ({e}); it stays under this name"));
        }
    }
}

/// What a folder's own named streams came to.
enum FolderStreams {
    /// It carried none.
    None,
    /// It carried some, and all of them were written over.
    Written,
    /// One couldn't be read, opened, planned, given room or written: the folder stays (the
    /// reason is recorded).
    Left,
}

/// Write over the named streams a folder itself carries, while its checked handle is held.
fn scrub_folder_streams(path: &Path, h: &win::Handle, info: &win::Info, o: &mut Outcome) -> FolderStreams {
    let names = match win::streams(path, h) {
        Ok(n) => n,
        Err(e) => {
            o.problem(path, format!("left as it is: its named streams couldn't be read ({e})"));
            return FolderStreams::Left;
        }
    };
    let mut streams: Vec<(String, win::Handle, Plan)> = Vec::new();
    let mut extra = 0u64;
    for name in names {
        let sp = stream_path(path, &name);
        match win::open_data(&sp) {
            Ok((sh, si)) => {
                // the folder's own handle keeps it from being renamed or replaced while it is
                // held; the stream must still be on the very folder that was checked
                if si.id != info.id || si.volume != info.volume {
                    o.problem(path, format!("left as it is: its stream {name} isn't on the folder Sprawl checked"));
                    return FolderStreams::Left;
                }
                let plan = match plan_for(&sh, &si) {
                    Ok(plan) => plan,
                    Err(why) => {
                        o.problem(path, format!("left as it is: its stream {name}: {why}"));
                        return FolderStreams::Left;
                    }
                };
                extra += extra_room(&sh, &plan);
                streams.push((name, sh, plan));
            }
            Err(e) => {
                o.problem(path, format!("left as it is: its stream {name} couldn't be opened ({e})"));
                return FolderStreams::Left;
            }
        }
    }
    // the room the folder's streams need between them, checked once
    if let Some(why) = room_for(path, extra) {
        o.problem(path, format!("left as it is: {why}"));
        return FolderStreams::Left;
    }
    let mut written = 0u64;
    for (name, sh, plan) in &streams {
        if let Err(e) = wipe(sh, plan, &mut written) {
            o.overwritten += written;
            o.damage(path, format!("partly overwritten (its stream {name} couldn't be written: {e}); the folder is left"));
            return FolderStreams::Left;
        }
        if let Err(e) = win::set_end(sh, 0).and_then(|()| win::flush(sh)) {
            o.overwritten += written;
            o.damage(path, format!("partly overwritten (its stream {name} couldn't be set to 0 bytes: {e}); the folder is left"));
            return FolderStreams::Left;
        }
    }
    o.overwritten += written;
    if streams.is_empty() {
        FolderStreams::None
    } else {
        FolderStreams::Written
    }
}

/// Whether the file on the reopened data handle is still the one that was checked: its length,
/// and its last-write time (a WOF file's write-open does not move it, so it is compared
/// like any other).
fn unchanged(data: &win::Handle, info: &win::Info) -> Result<(), &'static str> {
    match (win::standard(data), win::written_time(data)) {
        (Ok((_, eof)), Ok(written)) if eof == info.size && written == info.written => Ok(()),
        (Ok((_, eof)), Ok(_)) if eof != info.size => Err("left alone: it changed while Sprawl was checking it (its length is not the one it had)"),
        (Ok(_), Ok(_)) => Err("left alone: it changed while Sprawl was checking it (it was written)"),
        _ => Err("left alone: it couldn't be checked again"),
    }
}

/// The same file, opened again for its data once every check passed. ReOpenFile keeps the file
/// object the checks were made on (the path can't be a different file); if Windows won't reopen
/// it, the path is opened instead and the file's id must be the one checked.
fn reopen_data(p: &Path, h: &win::Handle, info: &win::Info) -> Result<win::Handle, String> {
    if let Ok(d) = win::reopen_write(h) {
        return Ok(d);
    }
    let (d, di) = win::open_data(p).map_err(|e| format!("left alone: it couldn't be opened for writing, so it may be in use ({e})"))?;
    if di.id != info.id || di.volume != info.volume {
        return Err("left alone: a different file is at that path now".into());
    }
    Ok(d)
}

/// Write `plan` over the open file's data in one pass and flush it. `written` counts the bytes.
fn wipe(h: &win::Handle, plan: &Plan, written: &mut u64) -> std::io::Result<()> {
    let mut buf = vec![0u8; 1 << 20];
    match plan {
        Plan::Ranges(ranges) => {
            for &(off, len) in ranges {
                let mut left = len;
                let mut at = off;
                while left > 0 {
                    let n = usize::try_from(left.min(buf.len() as u64)).unwrap_or(buf.len());
                    random(&mut buf[..n])?;
                    win::seek(h, at)?;
                    win::write(h, &buf[..n])?;
                    at += n as u64;
                    left -= n as u64;
                    *written += n as u64;
                }
            }
        }
        Plan::Whole(len) => {
            win::seek_start(h)?;
            let mut left = *len;
            while left > 0 {
                let n = usize::try_from(left.min(buf.len() as u64)).unwrap_or(buf.len());
                random(&mut buf[..n])?;
                win::write(h, &buf[..n])?;
                left -= n as u64;
                *written += n as u64;
            }
        }
    }
    win::flush(h)
}

/// A random name in `p`'s own folder, never one already there. None when the system's random
/// numbers couldn't be read: the name must be unpredictable.
fn random_name(p: &Path) -> Option<PathBuf> {
    let folder = p.parent()?;
    for _ in 0..8 {
        let mut b = [0u8; 8];
        random(&mut b).ok()?;
        let to = folder.join(format!("sprawl-overwrite-{:016x}", u64::from_le_bytes(b)));
        // a rename never writes over a name that is there; a clash just tries another
        if !to.exists() {
            return Some(to);
        }
    }
    None
}

/// The file's path with one of its stream names after it (`C:\x\y.txt:name:$DATA`).
fn stream_path(p: &Path, name: &str) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(name);
    PathBuf::from(s)
}

fn bytes(n: u64) -> String {
    // plain and rounded, as the window says sizes ("4.2 GB")
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} bytes")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn odd_name(p: &Path) -> bool {
    p.file_name().map(|n| n.to_string_lossy()).is_some_and(|n| n.ends_with('.') || n.ends_with(' '))
}

/// The entries of folder `p`, and how many of them couldn't be read.
fn list(p: &Path) -> std::io::Result<(Vec<PathBuf>, usize)> {
    let (mut got, mut unread) = (Vec::new(), 0);
    for e in std::fs::read_dir(p)? {
        match e {
            Ok(e) => got.push(e.path()),
            Err(_) => unread += 1,
        }
    }
    Ok((got, unread))
}

fn unreadable(n: usize) -> String {
    format!("{n} item{} in it couldn't be read; left as {}", if n == 1 { "" } else { "s" }, if n == 1 { "it is" } else { "they are" })
}

// ---------------------------------------------------------------- random data and free space

/// Fill `buf` from the system's own random number generator. Not a made-up stream: the data
/// that replaces a file must not be predictable from another file's data or from the time.
fn random(buf: &mut [u8]) -> std::io::Result<()> {
    let mut left = buf;
    while !left.is_empty() {
        let n = left.len().min(u32::MAX as usize);
        // SAFETY: `left` is `n` bytes; only they are written.
        let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), left.as_mut_ptr(), n as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
        if status != 0 {
            return Err(std::io::Error::other(format!("the system's random numbers couldn't be read (0x{:08X})", status as u32)));
        }
        left = &mut left[n..];
    }
    Ok(())
}

/// The unused room on the drive holding `p`, in bytes.
pub fn free_space(p: &Path) -> Option<u64> {
    let root = drive_root(p)?;
    let mut free = 0u64;
    // SAFETY: the wide root is NUL-terminated; only `free` is written.
    let ok = unsafe { GetDiskFreeSpaceExW(wide(&root).as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}

// ---------------------------------------------------------------- volume facts and free space

/// The drive root (`C:\`) holding `path`, when `path` is a plain local drive path.
pub fn drive_root(path: &Path) -> Option<PathBuf> {
    match path.components().next()? {
        std::path::Component::Prefix(p) if matches!(p.kind(), std::path::Prefix::Disk(_)) => {
            Some(PathBuf::from(format!("{}\\", p.as_os_str().to_string_lossy())))
        }
        _ => None,
    }
}

/// The volume's file system name as Windows reports it ("NTFS", "ReFS"), from a path on it.
pub fn filesystem(path: &Path) -> Option<String> {
    let root = drive_root(path).or_else(|| Some(Volume::parse(path.to_str()?).ok()?.path().to_owned()))?;
    let mut label = [0u16; 64];
    let mut fs = [0u16; 64];
    // SAFETY: both buffers are 64 units; the wide root is NUL-terminated.
    let ok = unsafe {
        GetVolumeInformationW(
            wide(&root).as_ptr(),
            label.as_mut_ptr(),
            64,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            64,
        )
    };
    (ok != 0).then(|| from_wide(&fs)).filter(|s| !s.is_empty())
}

/// Whether Windows says the volume takes no seek penalty (a solid-state drive). None when it
/// can't tell: an old drive, a volume that doesn't answer, or one this account can't query.
pub fn solid_state(path: &Path) -> Option<bool> {
    let device = if let Some(root) = drive_root(path) {
        format!(r"\\.\{}", root.to_string_lossy().trim_end_matches('\\'))
    } else {
        let volume = Volume::parse(path.to_str()?).ok()?;
        volume.path().to_str()?.trim_end_matches('\\').to_owned()
    };
    // no access at all, shared with everything: a query, not a read of the drive
    // SAFETY: the device name is NUL-terminated.
    let h = unsafe { CreateFileW(wide(Path::new(&device)).as_ptr(), 0, 0x1 | 0x2 | 0x4, std::ptr::null_mut(), 3, 0, std::ptr::null_mut()) };
    if h as isize == -1 {
        return None;
    }
    // STORAGE_PROPERTY_QUERY { PropertyId: 7 (StorageDeviceSeekPenaltyProperty), QueryType: 0 }
    let mut query = [0u8; 12];
    query[0..4].copy_from_slice(&7u32.to_le_bytes());
    // DEVICE_SEEK_PENALTY_DESCRIPTOR { Version u32, Size u32, IncursSeekPenalty u8 }
    let mut out = [0u8; 32];
    let mut returned = 0u32;
    // SAFETY: the buffers are the sizes given; only `returned` and `out` are written.
    let ok = unsafe {
        DeviceIoControl(h, 0x002D_1400, query.as_ptr().cast(), 12, out.as_mut_ptr().cast(), 32, &mut returned, std::ptr::null_mut())
    };
    // SAFETY: the handle came from CreateFileW above and is closed once.
    unsafe { CloseHandle(h as isize) };
    (ok != 0 && returned >= 9).then(|| out[8] == 0)
}

// ---------------------------------------------------------------- wiping free space

#[path = "wipe_mount.rs"]
mod wipe_mount;
use wipe_mount::{Mount, NativeMount, SystemVolume, Volume};

/// Consent for one fixed volume. `label` is display only; only the verified system drive
/// can be addressed by letter, and only after rechecking its GUID at launch.
pub struct WipeReview {
    volume: Volume,
    system_root: Option<PathBuf>,
    pub label: PathBuf,
}

impl WipeReview {
    pub fn volume_path(&self) -> &Path { self.volume.path() }
    pub fn uses_system_drive(&self) -> bool { self.system_root.is_some() }
}

pub fn wipe_data_dir() -> Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Sprawl"))
        .ok_or_else(|| "Sprawl's local data folder could not be found".into())
}

fn ensure_wipe_allowed() -> Result<(), String> {
    crate::require_write().map_err(|e| e.to_string())?;
    match sprawl_core::elevation() {
        Ok(true) => Ok(()),
        Ok(false) => Err("Free-space wiping needs an administrator Sprawl. Nothing was wiped.".into()),
        Err(e) => Err(format!("Sprawl couldn't verify administrator rights ({e}). Nothing was wiped.")),
    }
}

pub fn review_wipe(root: &Path) -> Result<WipeReview, String> {
    ensure_wipe_allowed()?;
    let review = capture_wipe(root, |root| {
        if !wipe_mount::fixed(root) { return Err(std::io::Error::other("not a fixed local drive")); }
        let volume = wipe_mount::volume_at(root)?;
        if !wipe_mount::fixed(volume.path()) { return Err(std::io::Error::other("not a fixed local volume")); }
        Ok(volume)
    })?;
    let system = wipe_mount::system_volume().ok();
    bind_system_route(review, system.as_ref(), std::env::var_os("SystemDrive").as_deref())
        .map_err(|e| format!("Sprawl couldn't verify the Windows system drive ({e}). Nothing was wiped."))
}

fn capture_wipe(root: &Path, resolve: impl FnOnce(&Path) -> std::io::Result<Volume>) -> Result<WipeReview, String> {
    let label = drive_root(root).ok_or_else(|| "that isn't on a local drive".to_string())?;
    let volume = resolve(&label).map_err(|e| format!("Sprawl couldn't bind {} to a fixed volume ({e}). Nothing was wiped.", label.display()))?;
    Ok(WipeReview { volume, label, system_root: None })
}

fn checked_system_root(system: &SystemVolume, env_drive: Option<&std::ffi::OsStr>) -> std::io::Result<PathBuf> {
    let bad = || std::io::Error::other("SystemDrive doesn't match the drive holding the running Windows installation");
    let drive = env_drive.and_then(std::ffi::OsStr::to_str).ok_or_else(bad)?;
    let bytes = drive.as_bytes();
    if bytes.len() != 2 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return Err(bad());
    }
    let root = format!("{}:\\", (bytes[0] as char).to_ascii_uppercase());
    if !system.root.to_string_lossy().eq_ignore_ascii_case(&root) { return Err(bad()); }
    Ok(root.into())
}

fn bind_system_route(mut review: WipeReview, system: Option<&SystemVolume>, env_drive: Option<&std::ffi::OsStr>) -> std::io::Result<WipeReview> {
    if let Some(system) = system.filter(|s| s.volume == review.volume) {
        review.system_root = Some(checked_system_root(system, env_drive)?);
    }
    Ok(review)
}

fn system_cipher_args(review: &WipeReview, system: &SystemVolume, env_drive: Option<&std::ffi::OsStr>) -> std::io::Result<String> {
    let root = checked_system_root(system, env_drive)?;
    if review.system_root.as_ref() != Some(&root) || review.volume != system.volume {
        return Err(std::io::Error::other("the system drive no longer matches the reviewed volume GUID"));
    }
    // No quoting needed for the validated X:\ root (and no backslash before a closing quote).
    Ok(format!("/w:{}", root.display()))
}

/// Do not create a directory, touch normal profile paths, or detach anything in screenshot mode.
pub fn sweep_wipe_mounts() {
    if ensure_wipe_allowed().is_err() { return; }
    let system = wipe_mount::system_volume().ok();
    wipe_mount::sweep(wipe_data_dir().ok().as_deref(), system.as_ref());
}

/// What a wipe would find before it starts, for the question and for the run itself: the drive,
/// the file that says Sprawl is wiping it, cipher's working folder if one is there, and whether
/// another Sprawl still has a wipe running on it.
pub struct WipePlan {
    pub marker: PathBuf,
    pub scratch: PathBuf,
    pub scratch_present: bool,
    /// A Sprawl whose marker this is is still running, and still wiping this drive.
    pub live: Option<u32>,
}

impl WipePlan {
    /// The lines the question must carry about cipher's working folder.
    pub fn scratch_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(pid) = self.live {
            lines.push(format!("Another Sprawl (process {pid}) is wiping this drive's free space right now; wait for it to finish or stop it there."));
        }
        if self.scratch_present { lines.push(scratch_instructions(&self.scratch)); }
        lines
    }
}

fn scratch_instructions(path: &Path) -> String {
    format!("Sprawl leaves {} alone and won't start a wipe while it exists or can't be checked. Once cipher is no longer running, find that volume's current drive letter (don't assume the old letter is still right). Open it in File Explorer, show hidden and protected operating system files if needed, and inspect EFSTMPWP. Delete it yourself only if you recognise cipher's temporary data. Leave anything unfamiliar alone.", path.display())
}

fn scratch_present(path: &Path) -> bool {
    // Never inspect the contents or follow a link. An unreadable name also blocks the wipe.
    !std::fs::symlink_metadata(path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

/// Check the reviewed GUID itself, never the letter that was used to find it.
pub fn wipe_plan(review: &WipeReview, marker_dir: &Path) -> Result<WipePlan, String> {
    if !wipe_mount::fixed(review.volume_path()) {
        return Err(format!("The reviewed volume {} is no longer available as a fixed drive. Nothing was wiped.", review.volume_path().display()));
    }
    let marker = marker_dir.join(format!("wipe-{}.txt", review.volume.id()));
    Ok(wipe_plan_in(review.volume_path(), marker, review.volume_path()))
}

fn wipe_plan_in(root: &Path, marker: PathBuf, location: &Path) -> WipePlan {
    let ours = read_marker(&marker).filter(|m| m.root.eq_ignore_ascii_case(&root.to_string_lossy()));
    let scratch = location.join("EFSTMPWP");
    let live = ours.as_ref().filter(|m| !marker_is_this_run(m)).filter(|m| marker_live(m)).map(|m| m.pid);
    WipePlan {
        marker,
        scratch_present: scratch_present(&scratch),
        scratch,
        live,
    }
}

/// What a free-space wipe left. `ok` is whether cipher itself finished without a problem; what
/// it wrote stays written either way.
pub struct Wiped {
    pub outcome: crate::clean::Outcome,
    pub ok: bool,
    /// Cipher wasn't started.
    pub left_before_start: bool,
}

/// Wipe the approved volume: directly for the running Windows volume, privately mounted for
/// everything else. A failed system check never falls back to a mount or a different letter.
pub fn wipe_free_space(review: &WipeReview, data: &Path, stop: &AtomicBool) -> Wiped {
    if let Err(why) = ensure_wipe_allowed() { return not_wiped(why); }
    let plan = match wipe_plan(review, data) {
        Ok(plan) => plan,
        Err(why) => return not_wiped(why),
    };
    if review.uses_system_drive() {
        let check = || system_cipher_args(review, &wipe_mount::system_volume()?, std::env::var_os("SystemDrive").as_deref());
        return wipe_system_in(review, plan, stop, check, |_, marker, pins| {
            crate::clean::run_wipe_cipher(check, stop, pins, &[&marker.0])
        });
    }
    let system = wipe_mount::system_volume().ok();
    wipe_free_in(review, plan, stop, || NativeMount::prepare(data, system.as_ref(), &review.volume),
        |mount, args, marker, pins| {
            let mut share = mount.pins();
            share.push(&marker.0);
            crate::clean::run_wipe_cipher(|| Ok(args.to_owned()), stop, pins, &share)
        })
}

fn not_wiped(why: String) -> Wiped {
    let mut outcome = crate::clean::Outcome::default();
    outcome.problems.push(why);
    Wiped { outcome, ok: false, left_before_start: true }
}

fn begin_wipe(review: &WipeReview, plan: &WipePlan, stop: &AtomicBool) -> Result<MarkerHold, String> {
    if let Some(pid) = plan.live {
        return Err(format!("Not started: another Sprawl (process {pid}) is wiping this volume's free space."));
    }
    if plan.scratch_present { return Err(scratch_instructions(&plan.scratch)); }
    if stop.load(Ordering::Relaxed) { return Err("Stop was requested before cipher started.".into()); }
    let hold = claim_marker(&plan.marker, review.volume_path())
        .map_err(|e| format!("Not started: Sprawl couldn't note the wipe in {} ({e}).", plan.marker.display()))?;
    if scratch_present(&plan.scratch) { return Err(scratch_instructions(&plan.scratch)); }
    Ok(hold)
}

fn wipe_system_in(review: &WipeReview, plan: WipePlan, stop: &AtomicBool,
    verify: impl FnOnce() -> std::io::Result<String>,
    run: impl FnOnce(&str, &MarkerHold, crate::clean::ToolPins) -> crate::clean::Outcome,
) -> Wiped {
    let hold = match begin_wipe(review, &plan, stop) {
        Ok(hold) => std::sync::Arc::new(hold),
        Err(why) => return not_wiped(why),
    };
    let args = match verify() {
        Ok(args) => args,
        Err(e) => return not_wiped(format!("Sprawl couldn't recheck the Windows system drive ({e}).")),
    };
    let outcome = run(&args, &hold, crate::clean::ToolPins::hold(hold.clone()));
    wipe_finished(outcome, &plan.scratch)
}

struct MountGuard<M: Mount> {
    mount: M,
    cleanup_error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl<M: Mount> Drop for MountGuard<M> {
    fn drop(&mut self) {
        if let Err(e) = self.mount.cleanup() {
            let note = format!("Sprawl couldn't clean up its private mount {} ({e}). The next administrator launch will check for a leftover mount.", self.mount.path().display());
            eprintln!("{note}");
            *self.cleanup_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(note);
        }
    }
}

fn wipe_free_in<M: Mount>(review: &WipeReview, plan: WipePlan, stop: &AtomicBool,
    prepare: impl FnOnce() -> std::io::Result<M>,
    run: impl FnOnce(&M, &str, &MarkerHold, crate::clean::ToolPins) -> crate::clean::Outcome,
) -> Wiped {
    let hold = match begin_wipe(review, &plan, stop) {
        Ok(h) => h,
        Err(why) => return not_wiped(why),
    };
    let cleanup_error = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mount = match prepare() {
        Ok(mount) => mount,
        Err(e) => return not_wiped(format!("Sprawl couldn't prepare a private mount for the reviewed volume ({e}).")),
    };
    let mut mount = MountGuard { mount, cleanup_error: cleanup_error.clone() };
    let ready = (|| {
        mount.mount.attach()?;
        if mount.mount.verified_volume()? != review.volume {
            return Err(std::io::Error::other("the mounted volume GUID doesn't match the reviewed volume"));
        }
        wipe_mount::cipher_args(mount.mount.path())
    })();
    let mut result = match ready {
        Err(e) => {
            drop(mount);
            not_wiped(format!("Sprawl couldn't verify the private mount for the reviewed volume ({e})."))
        }
        Ok(_) if stop.load(Ordering::Relaxed) || scratch_present(&plan.scratch) => {
            let why = if stop.load(Ordering::Relaxed) { "Stop was requested before cipher started.".into() } else { scratch_instructions(&plan.scratch) };
            drop(mount);
            not_wiped(why)
        }
        Ok(args) => {
            let mount = std::sync::Arc::new(mount);
            let hold = std::sync::Arc::new(hold);
            let outcome = run(&mount.mount, &args, &hold, crate::clean::ToolPins::hold((hold.clone(), mount.clone())));
            drop(mount);
            wipe_finished(outcome, &plan.scratch)
        }
    };
    if let Some(note) = cleanup_error.lock().unwrap_or_else(|e| e.into_inner()).take() {
        result.outcome.notes.push(note);
    }
    result
}

fn wipe_finished(mut outcome: crate::clean::Outcome, scratch: &Path) -> Wiped {
    let ok = !outcome.stopped && !outcome.timed_out && !outcome.tool_unconfirmed
        && outcome.problems.is_empty() && outcome.more_problems == 0;
    if scratch_present(scratch) || outcome.tool_unconfirmed {
        outcome.notes.push(scratch_instructions(scratch));
    }
    Wiped { outcome, ok, left_before_start: false }
}

/// A marker Sprawl leaves while it wipes a drive: the drive, which process (and which run of it:
/// the process's own start time) is doing the wiping.
struct Marker {
    root: String,
    pid: u32,
    start: u64,
}

fn read_marker(p: &Path) -> Option<Marker> {
    let text = std::fs::read_to_string(p).ok()?;
    parse_marker(&text)
}

fn parse_marker(text: &str) -> Option<Marker> {
    let (mut root, mut pid, mut start) = (None, None, None);
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("root=") {
            root = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("pid=") {
            pid = v.trim().parse::<u32>().ok();
        } else if let Some(v) = line.strip_prefix("start=") {
            start = v.trim().parse::<u64>().ok();
        }
    }
    Some(Marker { root: root?, pid: pid?, start: start? })
}

/// Whether the Sprawl named in `m` is still running: its id and its own start time must both
/// match, so a reused id doesn't pass for a wipe that is long gone.
fn marker_live(m: &Marker) -> bool {
    process_alive(m.pid) && process_start(m.pid) == Some(m.start)
}

/// Whether the note names this very run: a wipe this process started, or the note it leaves
/// after a failed start. It is not "another Sprawl", and the user isn't told to wait for it.
fn marker_is_this_run(m: &Marker) -> bool {
    m.pid == std::process::id() && Some(m.start) == process_start(std::process::id())
}

/// A marker held open for the whole wipe, shared for reading only: another Sprawl can see that a
/// wipe is running, but can neither change nor remove the note. Windows retires this exact
/// file on close, including when the job's deferred teardown finally releases its pins.
pub(crate) struct MarkerHold(std::fs::File);

/// Claim the marker for this drive. The first Sprawl to make it wins; a marker left by a Sprawl
/// that is gone is removed and a fresh one made, and if another instance makes one in that
/// instant (or holds the old one open), this one gives up rather than wipe alongside it.
pub(crate) fn claim_marker(p: &Path, root: &Path) -> std::io::Result<MarkerHold> {
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::windows::fs::OpenOptionsExt;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let when = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let start = process_start(std::process::id()).unwrap_or(0);
    // Older Sprawl builds may still read this note. Never let them infer scratch ownership.
    let body = format!("root={}\npid={}\nstart={start}\nforeign=1\ntime={when}\n", root.display(), std::process::id());
    let make = || {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            // shared for reading only: another Sprawl sees the wipe running, and can neither
            // write the note nor remove it while it is held
            .share_mode(1)
            .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_DELETE_ON_CLOSE)
            .open(p)
    };
    let mut f = match make() {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            match read_marker(p) {
                Some(m) if !marker_is_this_run(&m) && marker_live(&m) => return Err(std::io::Error::other("another Sprawl is wiping this drive right now")),
                // a stale or unreadable note goes; one that can't be removed is being held by a
                // live wipe, which is a refusal, not a reason to carry on
                _ => {}
            }
            if let Err(e) = std::fs::remove_file(p) {
                return Err(if matches!(e.raw_os_error(), Some(5) | Some(32)) {
                    std::io::Error::other("another Sprawl is wiping this drive right now")
                } else {
                    e
                });
            }
            make().map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    std::io::Error::other("another Sprawl has just started wiping this drive")
                } else {
                    e
                }
            })?
        }
        Err(e) => return Err(e),
    };
    // written and read back through the same handle: it must be this run's note
    let checked = (|| -> std::io::Result<()> {
        f.write_all(body.as_bytes())?;
        f.seek(SeekFrom::Start(0))?;
        let mut read = String::new();
        f.read_to_string(&mut read)?;
        let m = parse_marker(&read).ok_or_else(|| std::io::Error::other("the wipe's own note couldn't be read back"))?;
        if m.pid != std::process::id() || m.start != start {
            return Err(std::io::Error::other("another Sprawl's note was found in place of this one's"));
        }
        Ok(())
    })();
    checked?; // A half-written note also retires on close, not by looking up its path again.
    Ok(MarkerHold(f))
}

/// Whether a process with this id is still running.
fn process_alive(pid: u32) -> bool {
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_TIMEOUT: u32 = 0x102;
    // SAFETY: a query about a process by its id; the handle is closed below.
    let h = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if h == 0 {
        return false;
    }
    let alive = unsafe { WaitForSingleObject(h as *mut std::ffi::c_void, 0) } == WAIT_TIMEOUT;
    unsafe { CloseHandle(h) };
    alive
}

/// When a process began, as a FILETIME: two runs never share both an id and a start time.
fn process_start(pid: u32) -> Option<u64> {
    const QUERY: u32 = 0x1000;
    let current = pid == std::process::id();
    // SAFETY: a query about a process by its id (or this one); the handle is closed below.
    let h = if current { unsafe { GetCurrentProcess() } } else { unsafe { OpenProcess(QUERY, 0, pid) as *mut std::ffi::c_void } };
    if h.is_null() {
        return None;
    }
    let (mut made, mut exit, mut kernel, mut user) = ([0u32; 2], [0u32; 2], [0u32; 2], [0u32; 2]);
    // SAFETY: four FILETIMEs are written into the arrays.
    let ok = unsafe { GetProcessTimes(h as isize, made.as_mut_ptr(), exit.as_mut_ptr(), kernel.as_mut_ptr(), user.as_mut_ptr()) } != 0;
    if !current {
        unsafe { CloseHandle(h as isize) };
    }
    ok.then(|| (made[1] as u64) << 32 | made[0] as u64)
}

fn wide(p: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;

#[link(name = "bcrypt")]
extern "system" {
    fn BCryptGenRandom(alg: *mut std::ffi::c_void, buf: *mut u8, len: u32, flags: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetVolumeInformationW(root: *const u16, label: *mut u16, label_len: u32, serial: *mut u32, max_len: *mut u32, flags: *mut u32, fs: *mut u16, fs_len: u32) -> i32;
    fn GetDiskFreeSpaceExW(dir: *const u16, free: *mut u64, total: *mut u64, total_free: *mut u64) -> i32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut std::ffi::c_void, disposition: u32, flags: u32, template: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn DeviceIoControl(h: *mut std::ffi::c_void, code: u32, inbuf: *const std::ffi::c_void, inlen: u32, outbuf: *mut std::ffi::c_void, outlen: u32, returned: *mut u32, overlapped: *mut std::ffi::c_void) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn GetProcessTimes(h: isize, made: *mut u32, exit: *mut u32, kernel: *mut u32, user: *mut u32) -> i32;
    fn WaitForSingleObject(h: *mut std::ffi::c_void, ms: u32) -> u32;
    #[link_name = "CloseHandle"]
    fn CloseHandle(h: isize) -> i32;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use super::*;

    /// A temporary folder for one test, removed when the test ends, even when it fails.
    pub(super) struct Temp(PathBuf);

    impl Temp {
        pub(super) fn join(&self, p: impl AsRef<Path>) -> PathBuf {
            self.0.join(p)
        }
    }

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

    pub(super) fn temp(name: &str) -> Temp {
        let base = std::env::temp_dir().join(format!("sprawl-overwrite-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        Temp(base)
    }

    fn zeros(dir: &Path) -> usize {
        std::fs::read_dir(dir).unwrap().flatten().count()
    }

    #[test]
    fn overwriting_replaces_the_content() {
        let base = temp("content");
        let f = base.join("notes.bin");
        let pattern = b"SPRAWL-KNOWN-PATTERN-0123456789".repeat(2000);
        std::fs::write(&f, &pattern).unwrap();
        let (h, info) = win::open_checked(&f).unwrap();
        let plan = plan_for(&h, &info).unwrap();
        let data = win::reopen_write(&h).unwrap();
        let mut written = 0u64;
        wipe(&data, &plan, &mut written).unwrap();
        assert_eq!(written, plan.total(), "the whole planned length was written");
        drop(data);
        drop(h);
        let after = std::fs::read(&f).unwrap();
        assert!(after.len() >= pattern.len(), "the file still has its bytes to read back");
        let hit = after.windows(pattern.len()).any(|w| w == pattern.as_slice());
        assert!(!hit, "none of the pattern is left after one pass");
        assert!(after.iter().any(|&b| b != 0), "the pass wrote data, not just zeros");
    }

    #[test]
    fn a_file_is_set_to_zero_and_renamed_before_it_is_deleted() {
        let base = temp("rename");
        let f = base.join("old-name.txt");
        std::fs::write(&f, vec![7u8; 4096]).unwrap();
        let stop = AtomicBool::new(false);
        let o = run(&f, &Spare::default(), &stop, &mut |_| {});
        assert_eq!(o.files, 1, "{:?} {:?}", o.problems, o.damaged);
        assert!(!f.exists(), "the old name is gone");
        assert_eq!(zeros(&base), 0, "nothing is left behind");
    }

    #[test]
    fn a_hard_link_stops_the_overwrite_and_both_names_keep_their_bytes() {
        let base = temp("hardlink");
        let a = base.join("a.bin");
        let b = base.join("b.bin");
        std::fs::write(&a, b"shared bytes").unwrap();
        std::fs::hard_link(&a, &b).unwrap();
        let stop = AtomicBool::new(false);
        let o = run(&a, &Spare::default(), &stop, &mut |_| {});
        assert_eq!(o.files, 0, "{:?}", o.problems);
        assert!(o.problems.iter().any(|p| p.contains("hard link")), "{:?}", o.problems);
        assert_eq!(std::fs::read(&a).unwrap(), b"shared bytes");
        assert_eq!(std::fs::read(&b).unwrap(), b"shared bytes");

        // the same in a folder: both names stay, and so does the folder
        let o = run(&base, &Spare::default(), &stop, &mut |_| {});
        assert_eq!(o.files, 0, "{:?}", o.problems);
        assert_eq!(std::fs::read(&a).unwrap(), b"shared bytes");
        assert_eq!(std::fs::read(&b).unwrap(), b"shared bytes");
        assert!(base.exists());
        std::fs::remove_file(&b).unwrap();
        std::fs::remove_file(&a).unwrap();
    }

    #[test]
    fn links_and_reparse_points_are_left_alone() {
        let base = temp("links");
        let keep = base.join("keep");
        std::fs::create_dir_all(keep.join("inner")).unwrap();
        std::fs::write(keep.join("inner").join("precious.txt"), b"keep me").unwrap();
        let folder = base.join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("gone.bin"), vec![3u8; 900]).unwrap();
        let link = folder.join("link");
        let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&keep).output().is_ok_and(|o| o.status.success());
        let stop = AtomicBool::new(false);
        let o = run(&folder, &Spare::default(), &stop, &mut |_| {});
        assert!(o.problems.iter().any(|p| p.contains("link") || p.contains("reparse")), "{:?}", o.problems);
        assert!(keep.join("inner").join("precious.txt").exists(), "the junction's target stays");
        assert_eq!(std::fs::read(keep.join("inner").join("precious.txt")).unwrap(), b"keep me");
        assert!(!folder.join("gone.bin").exists(), "the plain file went");
        if ok {
            assert!(link.exists(), "the junction itself stays: it is not followed and not overwritten");
            assert!(folder.exists(), "the folder stays, holding the link it must not remove");
        } else {
            assert!(!folder.exists(), "with no link made, the folder went too");
        }

        // the junction as the whole item: refused, and its target untouched
        let as_root = run(&link, &Spare::default(), &stop, &mut |_| {});
        assert!(!as_root.problems.is_empty() && as_root.files == 0, "{:?}", as_root.problems);
        assert!(keep.join("inner").join("precious.txt").exists());
    }

    #[test]
    fn a_write_between_the_check_and_the_reopen_leaves_the_file() {
        let base = temp("raced");
        let f = base.join("raced.bin");
        std::fs::write(&f, b"original!").unwrap();
        let (h, info) = win::open_checked(&f).unwrap();
        // Windows stamps a file's time from a clock that moves at the timer tick (about 15 ms),
        // so a write in the same tick would not be seen; the check is that a later one is.
        std::thread::sleep(Duration::from_millis(50));
        {
            // another program writes the same number of bytes and lets go
            let (w, wi) = win::open_data(&f).unwrap();
            assert_eq!(wi.size, info.size);
            win::write(&w, b"changed!!").unwrap();
            win::flush(&w).unwrap();
        }
        let data = win::reopen_write(&h).unwrap();
        assert!(unchanged(&data, &info).is_err(), "a same-length write is caught by its last-write time");
        drop(data);
        drop(h);
        assert_eq!(std::fs::read(&f).unwrap(), b"changed!!");
    }

    #[test]
    fn a_folder_that_stays_keeps_its_stream_and_its_parent_stays_too() {
        let base = temp("stays");
        let parent = base.join("parent");
        let child = parent.join("child");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("keep.txt"), b"mine").unwrap();
        std::fs::write(child.join("go.bin"), b"gone").unwrap();
        let child_stream = stream_path(&child, ":extra:$DATA");
        let parent_stream = stream_path(&parent, ":extra:$DATA");
        std::fs::write(&child_stream, b"CHILD-STREAM-KEEP").unwrap();
        std::fs::write(&parent_stream, b"PARENT-STREAM-KEEP").unwrap();
        let spare = Spare { since: None, keep: vec![child.join("keep.txt")], seen: None };
        let stop = AtomicBool::new(false);
        let o = run(&parent, &spare, &stop, &mut |_| {});
        assert_eq!(o.files, 1, "{:?} {:?}", o.problems, o.damaged);
        assert_eq!(std::fs::read(&child_stream).unwrap(), b"CHILD-STREAM-KEEP", "a folder that stays keeps its stream");
        assert_eq!(std::fs::read(&parent_stream).unwrap(), b"PARENT-STREAM-KEEP", "so does its parent");
        assert!(child.exists() && parent.exists());
        assert!(o.damaged.is_empty(), "{:?}", o.damaged);
    }

    #[test]
    fn a_file_changed_after_the_scan_is_left_alone() {
        let base = temp("changed");
        let f = base.join("changed.bin");
        std::fs::write(&f, b"before").unwrap();
        let cut = SystemTime::now();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&f, b"after the scan began").unwrap();
        let spare = Spare { since: Some(cut), keep: Vec::new(), seen: None };
        let stop = AtomicBool::new(false);
        let o = run(&f, &spare, &stop, &mut |_| {});
        assert_eq!((o.files, o.kept_new), (0, 1), "{:?}", o.problems);
        assert_eq!(std::fs::read(&f).unwrap(), b"after the scan began");
    }

    #[test]
    fn stop_finishes_the_file_it_is_on_and_leaves_the_rest() {
        let base = temp("stop");
        std::fs::write(base.join("a.bin"), vec![1u8; 2 << 20]).unwrap();
        std::fs::write(base.join("b.bin"), b"second").unwrap();
        let stop = AtomicBool::new(false);
        let o = run(&base, &Spare::default(), &stop, &mut |_| {
            stop.store(true, Ordering::Relaxed);
        });
        assert!(o.stopped, "it stopped");
        assert_eq!(o.files, 1, "the file it was on finished: {:?}", o.problems);
        let left: Vec<String> = std::fs::read_dir(&*base).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec!["b.bin".to_string()], "the rest stayed, and no random name was left behind");
    }

    #[test]
    fn a_folder_goes_file_by_file_and_then_its_empty_folders() {
        let base = temp("folder");
        let root = base.join("target");
        std::fs::create_dir_all(root.join("sub").join("deeper")).unwrap();
        std::fs::write(root.join("one.bin"), vec![9u8; 700]).unwrap();
        std::fs::write(root.join("sub").join("two.bin"), vec![9u8; 1300]).unwrap();
        std::fs::write(root.join("sub").join("deeper").join("three.bin"), vec![9u8; 100]).unwrap();
        let stop = AtomicBool::new(false);
        let o = run(&root, &Spare::default(), &stop, &mut |_| {});
        assert!(o.problems.is_empty(), "{:?}", o.problems);
        assert_eq!((o.files, o.bytes), (3, 2100));
        assert_eq!(o.folders, 3, "the folder, its sub and the folder inside it");
        assert!(!root.exists(), "the folder picked went too");
        assert_eq!(zeros(&base), 0, "nothing was left behind");
    }

    #[test]
    fn a_kept_file_keeps_its_folder() {
        let base = temp("kept");
        let root = base.join("target");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("go.bin"), vec![1u8; 10]).unwrap();
        let stay = root.join("stay.txt");
        std::fs::write(&stay, b"mine").unwrap();
        let spare = Spare { since: None, keep: vec![stay.clone()], seen: None };
        let stop = AtomicBool::new(false);
        let o = run(&root, &spare, &stop, &mut |_| {});
        assert_eq!((o.files, o.kept_chosen), (1, 1), "{:?}", o.problems);
        assert_eq!(std::fs::read(&stay).unwrap(), b"mine");
        assert!(root.exists(), "the folder stays, holding the kept file");
    }

    #[test]
    fn a_read_only_file_goes_too() {
        let base = temp("readonly");
        let f = base.join("read-only.bin");
        std::fs::write(&f, b"not for changing").unwrap();
        let mut perm = std::fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&f, perm).unwrap();
        let stop = AtomicBool::new(false);
        let o = run(&f, &Spare::default(), &stop, &mut |_| {});
        assert_eq!(o.files, 1, "{:?} {:?}", o.problems, o.damaged);
        assert!(!f.exists());
    }

    #[test]
    fn a_read_only_file_that_cannot_be_written_gets_its_mark_back() {
        let base = temp("readonly-busy");
        let f = base.join("busy.bin");
        std::fs::write(&f, b"held open").unwrap();
        // a program holding it for writing: untouched, and its read-only mark is put back
        let held = win::hold_for_writing(&f).unwrap();
        let mut perm = std::fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&f, perm).unwrap();
        let stop = AtomicBool::new(false);
        let o = run(&f, &Spare::default(), &stop, &mut |_| {});
        assert_eq!(o.files, 0, "{:?}", o.problems);
        assert!(!o.problems.is_empty());
        assert!(std::fs::metadata(&f).unwrap().permissions().readonly(), "the mark is put back when the write can't start");
        drop(held);
        let mut perm = std::fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(false);
        std::fs::set_permissions(&f, perm).unwrap();
    }

    #[test]
    fn a_sparse_file_has_only_its_allocated_range_written() {
        let base = temp("sparse");
        let f = base.join("sparse.bin");
        let (far, small) = (64u64 << 20, 4096u64);
        std::fs::write(&f, b"").unwrap();
        {
            let (h, _) = win::open_data(&f).unwrap();
            win::set_sparse(&h).unwrap();
            win::seek(&h, far).unwrap();
            win::write(&h, &[0x5A; 4096]).unwrap();
            win::set_end(&h, far + small).unwrap();
        }
        let (h, info) = win::open_checked(&f).unwrap();
        let plan = plan_for(&h, &info).unwrap();
        match &plan {
            Plan::Ranges(r) => {
                assert_eq!(r.len(), 1, "one range: {r:?}");
                assert!(r[0].0 <= far && r[0].1 >= small && r[0].1 <= small + 64 * 1024, "just the written part: {r:?}");
            }
            other => panic!("a sparse file is written by range, not {other:?}"),
        }
        let data = win::reopen_write(&h).unwrap();
        let mut written = 0u64;
        wipe(&data, &plan, &mut written).unwrap();
        assert_eq!(written, plan.total());
        let (alloc, eof) = win::standard(&h).unwrap();
        assert_eq!(eof, far + small, "its length is untouched by the pass");
        assert!(alloc < 1 << 20, "the pass did not fill its holes: {alloc} bytes allocated");
        drop(data);
        drop(h);

        let stop = AtomicBool::new(false);
        let o = run(&f, &Spare::default(), &stop, &mut |_| {});
        assert_eq!(o.files, 1, "{:?}", o.problems);
        assert!(!f.exists());
    }

    #[test]
    fn a_named_stream_is_overwritten_and_emptied_before_the_file_goes() {
        let base = temp("streams");
        let f = base.join("with-stream.bin");
        std::fs::write(&f, b"ordinary data").unwrap();
        let sp = stream_path(&f, ":extra:$DATA");
        let pattern = b"STREAM-KNOWN-PATTERN-0123456789";
        std::fs::write(&sp, pattern).unwrap();
        {
            let (hand, _info) = win::open_checked(&f).unwrap();
            assert_eq!(win::streams(&f, &hand).unwrap(), vec![":extra:$DATA".to_string()]);
        }

        // the stream itself: written over, then emptied, through the same calls the engine uses
        {
            let (h, _info) = win::open_checked(&f).unwrap();
            let (sh, si) = win::open_data(&sp).unwrap();
            assert_eq!(si.size, pattern.len() as u64, "the stream's own length is what is written over");
            let mut written = 0u64;
            wipe(&sh, &Plan::Whole(si.size), &mut written).unwrap();
            assert_eq!(written, si.size);
            drop(sh);
            let after = std::fs::read(&sp).unwrap();
            assert!(!after.windows(pattern.len()).any(|w| w == pattern.as_slice()), "none of the stream's pattern is left");
            let (sh, _) = win::open_data(&sp).unwrap();
            win::set_end(&sh, 0).unwrap();
            win::flush(&sh).unwrap();
            drop(sh);
            drop(h);
        }
        assert_eq!(std::fs::metadata(&sp).unwrap().len(), 0, "the stream is emptied before the file goes");

        let stop = AtomicBool::new(false);
        let o = run(&f, &Spare::default(), &stop, &mut |_| {});
        assert_eq!(o.files, 1, "{:?} {:?}", o.problems, o.damaged);
        assert!(!f.exists(), "the file and its stream went with it");
    }

    #[test]
    fn the_checked_handle_reopens_for_writing_the_same_file() {
        let base = temp("reopen");
        let f = base.join("reopen.bin");
        std::fs::write(&f, b"0123456789").unwrap();
        let (h, info) = win::open_checked(&f).unwrap();
        assert_eq!(info.links, 1);
        let d = win::reopen_write(&h).expect("the checked handle reopens for its data");
        let mut written = 0u64;
        wipe(&d, &Plan::Whole(10), &mut written).unwrap();
        drop(d);
        drop(h);
        let after = std::fs::read(&f).unwrap();
        assert_ne!(after, b"0123456789");
    }

    #[test]
    fn a_marker_says_which_drive_is_being_wiped_and_by_which_process() {
        let base = temp("marker");
        let volume = fixture_volume(1);
        let root = volume.path();
        let marker = base.join(format!("wipe-{}.txt", volume.id()));
        assert!(!marker.file_name().unwrap().to_string_lossy().contains(':'));
        assert!(read_marker(&marker).is_none(), "no marker, no wipe");
        let hold = claim_marker(&marker, root).unwrap();
        let seen = read_marker(&marker).expect("another Sprawl can read a held note and see the wipe");
        assert_eq!((seen.pid, seen.start), (std::process::id(), process_start(std::process::id()).unwrap()));
        assert!(std::fs::remove_file(&marker).is_err(), "the note can't be removed while it is held");
        assert!(std::fs::OpenOptions::new().write(true).open(&marker).is_err(), "nor written");
        assert!(claim_marker(&marker, root).is_err(), "a second wipe is refused");
        assert!(!process_alive(u32::MAX - 1), "a made-up id is not");

        // a note left by a run that is gone is replaced
        drop(hold);
        assert!(!marker.exists(), "the marker retires with its handle");
        std::fs::write(&marker, "root=C:\\\npid=4294967295\nstart=1\ntime=0\n").unwrap();
        let hold = claim_marker(&marker, root).unwrap();
        let m = read_marker(&marker).unwrap();
        assert_eq!(m.root, root.to_string_lossy());
        assert_eq!((m.pid, m.start), (std::process::id(), process_start(std::process::id()).unwrap()));
        assert!(std::fs::read_to_string(&marker).unwrap().contains("foreign=1"), "legacy readers must never infer ownership");
        assert!(marker_live(&m), "this very process is alive");
        let mut other = Marker { root: m.root.clone(), pid: m.pid, start: m.start ^ 1 };
        assert!(!marker_live(&other), "an id reused by another run is not this wipe");
        other.start = m.start;
        assert!(marker_live(&other));
        assert!(marker_is_this_run(&other), "this run's own note is not 'another Sprawl'");
        drop(hold);
        assert!(!marker.exists());
    }

    fn fixture_volume(n: u32) -> Volume {
        Volume::parse(&format!("\\\\?\\Volume{{00000000-0000-0000-0000-{n:012x}}}\\")).unwrap()
    }

    fn fixture_review(n: u32) -> WipeReview {
        capture_wipe(Path::new(r"E:\"), |_| Ok(fixture_volume(n))).unwrap()
    }

    #[test]
    fn reviewed_system_volume_launches_with_systemdrive_not_the_selected_alias() {
        let base = temp("wipe-system-root");
        let system = SystemVolume { root: r"D:\".into(), volume: fixture_volume(1) };
        let drive = Some(std::ffi::OsStr::new("d:"));
        let review = bind_system_route(fixture_review(1), Some(&system), drive).unwrap();
        assert!(review.uses_system_drive());
        assert_eq!(review.label, Path::new(r"E:\"));
        let marker = base.join("wipe-guid.txt");
        let plan = wipe_plan_in(review.volume_path(), marker.clone(), &base);
        let result = wipe_system_in(&review, plan, &AtomicBool::new(false),
            || system_cipher_args(&review, &system, drive), |args, _, _| {
                assert_eq!(args, r"/w:D:\");
                assert!(marker.exists());
                crate::clean::Outcome::default()
            });
        assert!(result.ok && !marker.exists());
        assert!(!bind_system_route(fixture_review(2), Some(&system), drive).unwrap().uses_system_drive());
        assert!(bind_system_route(fixture_review(1), Some(&system), Some(std::ffi::OsStr::new("E:"))).is_err());
    }

    #[test]
    fn system_guid_mismatch_after_review_refuses_before_launch() {
        let base = temp("wipe-system-mismatch");
        let mut system = SystemVolume { root: r"C:\".into(), volume: fixture_volume(1) };
        let drive = Some(std::ffi::OsStr::new("C:"));
        let review = bind_system_route(fixture_review(1), Some(&system), drive).unwrap();
        system.volume = fixture_volume(2);
        let marker = base.join("wipe-guid.txt");
        let plan = wipe_plan_in(review.volume_path(), marker.clone(), &base);
        let result = wipe_system_in(&review, plan, &AtomicBool::new(false),
            || system_cipher_args(&review, &system, drive),
            |_, _, _| panic!("a mismatching GUID must not reach cipher"));
        assert!(result.left_before_start && !result.ok && !marker.exists());
        assert!(result.outcome.problems[0].contains("no longer matches the reviewed volume GUID"));
    }

    type Trace = std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>;

    struct FakeMount {
        path: PathBuf,
        target: Volume,
        reported: Volume,
        attached: bool,
        fail_attach: bool,
        trace: Trace,
    }

    impl FakeMount {
        fn new(target: Volume, trace: Trace) -> Self {
            Self { path: fixture_volume(9).path().join(r"Sprawl\wipe-mounts-v1\mount-1-2-3"),
                reported: target.clone(), target, attached: false, fail_attach: false, trace }
        }
        fn event(&self, s: &'static str) { self.trace.lock().unwrap().push(s); }
    }

    impl Mount for FakeMount {
        fn attach(&mut self) -> std::io::Result<()> {
            self.event("mount");
            if self.fail_attach { return Err(std::io::Error::other("mount fixture failed")); }
            self.attached = true;
            Ok(())
        }
        fn verified_volume(&mut self) -> std::io::Result<Volume> {
            self.event("verify");
            Ok(self.reported.clone())
        }
        fn path(&self) -> &Path { &self.path }
        fn cleanup(&mut self) -> std::io::Result<()> {
            if self.attached { self.event("unmount"); }
            self.event("remove directory");
            Ok(())
        }
    }

    #[test]
    fn same_volume_replaced_scratch_survives_a_legacy_marker() {
        let base = temp("wipe-replaced-scratch");
        let marker = base.join("wipe-E.txt");
        let review = fixture_review(1);
        std::fs::write(&marker, "root=E:\\\npid=4294967295\nstart=1\nforeign=0\n").unwrap();
        let old = base.join("EFSTMPWP");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("fil1.tmp"), "original scratch").unwrap();
        std::fs::rename(&old, base.join("prior-scratch")).unwrap();
        std::fs::create_dir(&old).unwrap();
        let unrelated = old.join("ordinary.txt");
        std::fs::write(&unrelated, "keep on this volume").unwrap();
        let plan = wipe_plan_in(review.volume_path(), marker, &base);
        let lines = plan.scratch_lines().join("\n");
        assert!(lines.contains(&old.display().to_string()) && lines.contains("File Explorer") && lines.contains("Leave anything unfamiliar alone"));
        let result = wipe_free_in::<FakeMount>(&review, plan, &AtomicBool::new(false),
            || panic!("must not mount with an occupied scratch path"), |_, _, _, _| panic!("must not run"));
        assert!(result.left_before_start && !result.ok);
        assert_eq!(result.outcome.problems, [lines]);
        assert_eq!(std::fs::read_to_string(&unrelated).unwrap(), "keep on this volume");
    }

    #[test]
    fn rebound_letter_with_no_scratch_does_not_retarget_cipher() {
        let base = temp("wipe-rebound-letter");
        let letter = Path::new(r"E:\");
        let mut current_e = fixture_volume(1);
        let review = capture_wipe(letter, |p| { assert_eq!(p, letter); Ok(current_e.clone()) }).unwrap();
        current_e = fixture_volume(2); // simulated rebind while the confirmation is open
        let new_volume = base.join("replacement-volume");
        std::fs::create_dir(&new_volume).unwrap();
        assert!(!new_volume.join("EFSTMPWP").exists());
        let marker = base.join("wipe-guid.txt");
        let plan = wipe_plan_in(review.volume_path(), marker.clone(), &base);
        let trace = Trace::default();
        let result = wipe_free_in(&review, plan, &AtomicBool::new(false),
            || Ok(FakeMount::new(review.volume.clone(), trace.clone())),
            |mount, args, _marker, _pins| {
                assert_eq!(mount.target, fixture_volume(1));
                assert_ne!(mount.target, current_e, "B was never passed to the runner");
                assert_eq!(args, format!("\"/w:{}\"", mount.path.display()));
                assert!(!args.contains("E:"));
                mount.event("run");
                crate::clean::Outcome::default()
            });
        assert!(result.ok && !marker.exists());
        assert_eq!(*trace.lock().unwrap(), ["mount", "verify", "run", "unmount", "remove directory"]);
    }

    #[test]
    fn a_wipe_that_leaves_scratch_reports_it_and_does_not_remove_it() {
        let base = temp("wipe-after-end");
        let marker = base.join("wipe-E.txt");
        let review = fixture_review(1);
        let plan = wipe_plan_in(review.volume_path(), marker.clone(), &base);
        let scratch = plan.scratch.clone();
        let leftover = scratch.join("leftover.tmp");
        let result = wipe_free_in(&review, plan, &AtomicBool::new(false),
            || Ok(FakeMount::new(review.volume.clone(), Trace::default())), |_, _, _, _pins| {
            std::fs::create_dir(&scratch).unwrap();
            std::fs::write(&leftover, "fixture").unwrap();
            crate::clean::Outcome { stopped: true, ..Default::default() }
        });
        assert!(!result.ok && !result.left_before_start && result.outcome.stopped);
        assert!(result.outcome.notes[0].contains(&scratch.display().to_string()));
        assert_eq!(std::fs::read_to_string(leftover).unwrap(), "fixture");
        assert!(!marker.exists());
    }

    #[test]
    fn mount_mismatch_refuses_and_cleans_up_without_running_cipher() {
        let base = temp("wipe-guid-mismatch");
        let review = fixture_review(1);
        let marker = base.join("wipe-guid.txt");
        let plan = wipe_plan_in(review.volume_path(), marker.clone(), &base);
        let trace = Trace::default();
        let result = wipe_free_in(&review, plan, &AtomicBool::new(false), || {
            let mut mount = FakeMount::new(review.volume.clone(), trace.clone());
            mount.reported = fixture_volume(2);
            Ok(mount)
        }, |_, _, _, _| panic!("cipher must not run after a GUID mismatch"));
        assert!(result.left_before_start && result.outcome.problems[0].contains("doesn't match"));
        assert_eq!(*trace.lock().unwrap(), ["mount", "verify", "unmount", "remove directory"]);
        assert!(!marker.exists());
    }

    #[test]
    fn scratch_appearing_during_mount_still_refuses_cipher_and_cleans_up() {
        let base = temp("wipe-late-scratch");
        let review = fixture_review(1);
        let marker = base.join("wipe-guid.txt");
        let plan = wipe_plan_in(review.volume_path(), marker.clone(), &base);
        let scratch = plan.scratch.clone();
        let trace = Trace::default();
        let result = wipe_free_in(&review, plan, &AtomicBool::new(false), || {
            std::fs::create_dir(&scratch).unwrap();
            std::fs::write(scratch.join("ordinary.txt"), "untouched").unwrap();
            Ok(FakeMount::new(review.volume.clone(), trace.clone()))
        }, |_, _, _, _| panic!("the worker must recheck scratch"));
        assert!(result.left_before_start);
        assert_eq!(std::fs::read_to_string(scratch.join("ordinary.txt")).unwrap(), "untouched");
        assert_eq!(*trace.lock().unwrap(), ["mount", "verify", "unmount", "remove directory"]);
        assert!(!marker.exists());
    }

    #[test]
    fn private_mount_cleanup_covers_failed_mount_stop_timeout_failed_launch_and_deferred_end() {
        for mode in ["mount failed", "stop before launch", "stop", "timeout", "launch failed", "deferred"] {
            let base = temp("wipe-mount-lifetime");
            let review = fixture_review(1);
            let marker = base.join("wipe-guid.txt");
            let plan = wipe_plan_in(review.volume_path(), marker.clone(), &base);
            let trace = Trace::default();
            let stop = AtomicBool::new(false);
            let mut deferred = None;
            let result = wipe_free_in(&review, plan, &stop, || {
                let mut mount = FakeMount::new(review.volume.clone(), trace.clone());
                mount.fail_attach = mode == "mount failed";
                if mode == "stop before launch" { stop.store(true, Ordering::Relaxed); }
                Ok(mount)
            }, |mount, _, _, pins| {
                assert!(!matches!(mode, "mount failed" | "stop before launch"));
                mount.event("run");
                if mode == "deferred" { deferred = Some(pins); }
                let mut o = crate::clean::Outcome { stopped: mode == "stop", timed_out: mode == "timeout",
                    tool_unconfirmed: mode == "deferred", ..Default::default() };
                if mode == "launch failed" { o.problems.push("fixture launch failed".into()); }
                o
            });
            assert!(!result.ok, "{mode}");
            match mode {
                "mount failed" => assert_eq!(*trace.lock().unwrap(), ["mount", "remove directory"]),
                "stop before launch" => assert_eq!(*trace.lock().unwrap(), ["mount", "verify", "unmount", "remove directory"]),
                "deferred" => {
                    assert_eq!(*trace.lock().unwrap(), ["mount", "verify", "run"]);
                    assert!(marker.exists());
                    drop(deferred);
                    assert_eq!(*trace.lock().unwrap(), ["mount", "verify", "run", "unmount", "remove directory"]);
                },
                _ => assert_eq!(*trace.lock().unwrap(), ["mount", "verify", "run", "unmount", "remove directory"]),
            }
            assert!(!marker.exists(), "{mode}");
        }
    }

    #[test]
    fn a_folders_own_named_stream_is_written_over_before_it_goes() {
        let base = temp("folder-stream");
        let dir = base.join("with-stream");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("file.bin"), b"x").unwrap();
        let sp = stream_path(&dir, ":extra:$DATA");
        let pattern = b"FOLDER-STREAM-PATTERN-0123456789";
        std::fs::write(&sp, pattern).unwrap();

        // the folder's own stream, through the same calls the engine uses
        let (fh, finfo) = win::open_checked(&dir).unwrap();
        let mut o = Outcome::default();
        assert!(matches!(scrub_folder_streams(&dir, &fh, &finfo, &mut o), FolderStreams::Written), "{:?}", o.problems);
        drop(fh);
        let after = std::fs::read(&sp).unwrap();
        assert_eq!(after.len(), 0, "the folder's stream is emptied");
        assert!(!after.windows(pattern.len()).any(|w| w == pattern.as_slice()));

        // and the whole folder, with its file and its own stream, goes
        let stop = AtomicBool::new(false);
        let o = run(&dir, &Spare::default(), &stop, &mut |_| {});
        assert_eq!(o.files, 1, "{:?}", o.problems);
        assert!(o.problems.is_empty(), "{:?}", o.problems);
        assert!(!dir.exists());
    }

    #[test]
    fn a_volume_without_named_streams_is_not_an_error() {
        // exFAT and FAT32 answer FindFirstStreamW with these: "no streams" only when the
        // volume's own flags say it has none; on a volume that has them they are real errors
        for c in [1, 50, 87] {
            assert!(win::no_named_streams(Some(c), Some(false)), "{c}");
            assert!(!win::no_named_streams(Some(c), Some(true)), "{c} on a volume with streams");
            assert!(!win::no_named_streams(Some(c), None), "{c} with no flags read");
        }
        // a folder has no streams to list, whatever the volume
        assert!(win::no_named_streams(Some(38), Some(true)));
        assert!(win::no_named_streams(Some(38), None));
        for c in [2, 5, 234] {
            assert!(!win::no_named_streams(Some(c), Some(false)), "{c}");
        }
        assert!(!win::no_named_streams(None, Some(false)));
    }

    #[test]
    fn a_plain_file_needs_no_room_of_its_own_to_be_written_over() {
        let base = temp("room");
        let f = base.join("plain.bin");
        std::fs::write(&f, vec![4u8; 200_000]).unwrap();
        let (h, info) = win::open_checked(&f).unwrap();
        let plan = plan_for(&h, &info).unwrap();
        assert_eq!(extra_room(&h, &plan), 0, "plain data is written where it already is");
        assert!(room_for(&f, extra_room(&h, &plan)).is_none(), "so no free space is needed for it");
        // a compressed or WOF plan takes its whole length: that much more must be free
        let alloc = win::standard(&h).unwrap().0;
        assert_eq!(extra_room(&h, &Plan::Whole(alloc + 12_345)), 12_345);
        assert!(room_for(&f, extra_room(&h, &Plan::Whole(u64::MAX))).is_some(), "more than any drive has");
    }

    #[test]
    fn the_review_rejects_network_paths_and_cipher_rejects_drive_letters() {
        assert!(wipe_mount::cipher_args(Path::new(r"C:\")).is_err());
        assert!(wipe_mount::cipher_args(Path::new(r"D:\fixture\mount-1-2-3")).is_err());
        assert_eq!(drive_root(Path::new(r"C:\Users\x")).as_deref(), Some(Path::new(r"C:\")));
        assert_eq!(drive_root(Path::new(r"C:\")), Some(PathBuf::from(r"C:\")));
        assert!(drive_root(Path::new(r"\\server\share\a")).is_none());
        assert!(filesystem(&std::env::temp_dir()).is_some_and(|fs| !fs.is_empty()), "the temp folder's file system is known");
        // the answer may be None (an old drive, a VM, no rights); it must never panic
        let _ = solid_state(&std::env::temp_dir());
        assert!(capture_wipe(Path::new(r"\\server\share"), |_| panic!("must not call a volume API for a network path")).is_err());
    }
}
