//! Overwrite and delete (F9), and wiping a drive's free space: the words and the waiting.
//!
//! The engine for a file or a folder is `sprawl_advisor::overwrite`; free space is Windows' own
//! `cipher /w:`, run through the clean-up's tool runner. This module only works out what the
//! question must say for the items or drive picked, refuses what must never be offered, and runs
//! the work on its own thread. Nothing here ever claims the data is unrecoverable: the words say
//! "overwritten once" and "harder to recover".

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sprawl_core::ROOT;

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::Selector;
use crate::commands::{DelMsg, DeleteVerb};
use crate::state::{find, text, State, View};
use sprawl_advisor::overwrite;

/// The short line both questions always carry: what Sprawl can't see.
const OLDER_COPIES: &str = "System Restore, shadow copies and the Recycle Bin may hold older copies of files.";

impl State {
    // ------------------------------------------------------- overwrite and delete

    /// The tree's or File View's "Overwrite and delete...": checked, then asked. Several rows
    /// picked (Ctrl or Shift) go together, as a Delete's do.
    pub(crate) fn overwrite_ask(&mut self) {
        if !matches!(self.view, View::Tree | View::Files) {
            text(&self.root, "FootLeft", "Overwrite and delete works in Tree View and File View (on rows of this scan).");
            return;
        }
        // File View: what Delete takes there (the files listed and picked, or the map's tile
        // the menu was opened on); a pick the list no longer shows refuses, as Delete does
        let nodes = if self.view == View::Files {
            if self.scan_picks_hidden() {
                text(&self.root, "FootLeft", "Some of what is picked is not a file in this list, so nothing was overwritten. Pick the files again.");
                return;
            }
            self.scan_targets_top()
        } else {
            // Tree View: what Delete takes there (the selected node, a map tile among them, when
            // nothing is marked; otherwise the picked rows)
            self.tree_targets_top()
        };
        let Some(t) = self.tree.clone() else { return };
        if nodes.is_empty() {
            text(&self.root, "FootLeft", "Pick the file or folder to overwrite and delete first.");
            return;
        }
        let mut picked: Vec<(usize, PathBuf)> = Vec::new();
        for &n in &nodes {
            let path = t.path_of(n);
            if let Some(why) = self.overwrite_refusal(n, &path) {
                self.tell("Not overwritten", &why);
                return;
            }
            picked.push((n, path));
        }
        let total: u64 = nodes.iter().map(|&n| t.nodes[n].size).sum();
        let mut body = String::new();
        if picked.len() == 1 {
            let (n, path) = &picked[0];
            let node = &t.nodes[*n];
            let what = if node.is_dir {
                format!("{} files in {} folders, {}", node.files, node.folders, self.size(node.size))
            } else {
                self.size(node.size)
            };
            body.push_str(&format!("{}\n\n{what}\n\n", path.display()));
        } else {
            for (_, path) in picked.iter().take(12) {
                body.push_str(&format!("{}\n", path.display()));
            }
            if picked.len() > 12 {
                body.push_str(&format!("...and {} more\n", picked.len() - 12));
            }
            body.push_str(&format!("\n{} items, {}\n\n", picked.len(), self.size(total)));
        }
        body.push_str("Sprawl will:\n");
        body.push_str("- overwrite every file once with random data, from its start to the end of the space it takes on disk (a sparse file only where its data is);\n");
        body.push_str("- overwrite its named streams the same way, flush it, set its length to 0, give it a random name in the same folder, and delete it;\n");
        body.push_str("- empty the folders and remove them.\n\n");
        body.push_str("Nothing goes to the Recycle Bin. Files are left alone when they are in use, changed since the scan, kept or protected, links or junctions, cloud or offline, or have another name (a hard link) elsewhere: overwriting one of those names would empty the other too.\n\n");
        body.push_str("What this cannot promise:\n");
        let mut lines: Vec<String> = Vec::new();
        for (n, path) in &picked {
            for line in self.overwrite_limits(*n, path) {
                if !lines.contains(&line) {
                    lines.push(line);
                }
            }
        }
        for line in lines {
            body.push_str(&format!("- {line}\n"));
        }
        body.push_str(&format!("- {OLDER_COPIES}\n\n"));
        body.push_str("Overwriting once makes the data harder to recover; it does not make it unrecoverable.");
        let gen = self.tree_gen;
        self.ask("Overwrite and delete?", &body, "Overwrite and delete", true, move |s| s.overwrite_start(picked, gen));
    }

    /// Why `n` may not be overwritten and deleted from here, if it may not. The delete's own
    /// checks, plus what a whole drive means for an overwrite.
    fn overwrite_refusal(&self, n: usize, path: &Path) -> Option<String> {
        if let Some(why) = self.delete_refusal(n, path) {
            return Some(why);
        }
        if path.parent().is_none() {
            return Some("That is a whole drive. Pick a file or folder on it instead.".into());
        }
        None
    }

    /// The limits that apply to this very item, each detected: whether the volume is solid-state
    /// or copy-on-write, whether the files are marked compressed, sparse, encrypted or WOF, and
    /// whether they are in a cloud-synced folder. The shadow-copy line is added by the question.
    fn overwrite_limits(&self, n: usize, path: &Path) -> Vec<String> {
        let mut limits = Vec::new();
        let root = overwrite::drive_root(path);
        match root.as_deref().map(overwrite::solid_state) {
            Some(Some(true)) => limits.push("This drive is solid-state: it moves writes around, so the old data may stay in cells the overwrite never reaches. The drive maker's secure erase, or keeping the drive encrypted with BitLocker, is the stronger answer.".to_string()),
            Some(None) => limits.push("Sprawl couldn't tell whether this drive is solid-state. If it is, it moves writes around, so the old data may stay in cells the overwrite never reaches: the drive maker's secure erase, or BitLocker, is the stronger answer.".to_string()),
            _ => {}
        }
        if root.as_deref().and_then(overwrite::filesystem).is_some_and(|fs| fs.eq_ignore_ascii_case("ReFS")) {
            limits.push("This volume is ReFS, which copies data when it changes: the overwrite may land in new places, leaving the old data.".to_string());
        }
        let Some(t) = self.tree.as_ref() else { return limits };
        let (mut compressed, mut sparse, mut encrypted, mut wof, mut cloud) = (false, false, false, false, false);
        let mut stack = vec![n];
        while let Some(k) = stack.pop() {
            let node = &t.nodes[k];
            compressed |= node.attributes & 0x800 != 0;
            sparse |= node.attributes & 0x200 != 0;
            encrypted |= node.attributes & 0x4000 != 0;
            wof |= node.attributes & sprawl_core::attr::WOF != 0;
            cloud |= node.attributes & overwrite::NOT_LOCAL != 0;
            stack.extend(node.children.iter().copied());
            // all of them found: no need to look through the rest of a big folder
            if compressed && sparse && encrypted && wof && cloud {
                break;
            }
        }
        let mut marks = Vec::new();
        if compressed {
            marks.push("NTFS-compressed");
        }
        if sparse {
            marks.push("sparse");
        }
        if encrypted {
            marks.push("encrypted (EFS)");
        }
        if !marks.is_empty() {
            limits.push(format!(
                "Some of these files are {}: the overwrite may land in new places, leaving the old data.",
                marks.join(", ")
            ));
        }
        if wof {
            limits.push("Some of these files are Windows-compressed (WOF): they are written out in full while they are overwritten, and the compressed bytes already on the disk may not be overwritten, so the old data may stay there.".to_string());
        }
        if compressed || wof {
            limits.push("Overwriting a compressed file writes it out in full, so it takes its whole length on the disk until it is deleted; Sprawl leaves one alone when the drive hasn't room for the extra room it needs.".to_string());
        }
        if cloud || in_cloud_folder(path) {
            limits.push("Files in a OneDrive or other cloud-synced folder: older versions may stay online.".to_string());
        }
        limits.push("The old names can remain in the file system's own records (the change journal) even after the rename.".to_string());
        limits
    }

    /// "Yes": checked once more, then overwritten and deleted on its own thread.
    fn overwrite_start(&mut self, picked: Vec<(usize, PathBuf)>, gen: u64) {
        for (n, path) in &picked {
            if !self.still_same(gen, *n, path) {
                self.tell("Not overwritten", &format!("The scan changed while the question was open, so nothing was overwritten.\n\n{}", path.display()));
                return;
            }
            if let Some(why) = self.overwrite_refusal(*n, path) {
                self.tell("Not overwritten", &why);
                return;
            }
        }
        let total = self.tree.as_ref().map_or(0, |t| picked.iter().map(|(n, _)| t.nodes[*n].size).sum());
        self.manual_overwrite(picked.into_iter().map(|(_, p)| p).collect(), total);
    }

    /// The worker and its card, with the same checks as a Delete from the tree: the Keep list is
    /// read again per item, a path skipped in Settings is left, and each item is checked again
    /// after it is opened. `total` is what the scan said the items hold, for the card's bar;
    /// progress counts the same measure (each item's own bytes) as it goes.
    fn manual_overwrite(&mut self, paths: Vec<PathBuf>, total: u64) {
        use sprawl_advisor::clean::Spare;
        if self.advisor.deleting.is_some() {
            return;
        }
        let confirmed = self.advisor.memory.kept_paths();
        let skipped_at_confirmation = sprawl_advisor::exclusions::configured();
        let since = self.active_began();
        let seen = self.active_tree().map(sprawl_advisor::clean::seen_in);
        let stop = Arc::new(AtomicBool::new(false));
        self.advisor.delete_stop = Some(stop.clone());
        let at = std::time::Instant::now();
        self.own_deletes.extend(paths.iter().map(|p| (p.to_string_lossy().trim_end_matches(std::path::MAIN_SEPARATOR).to_lowercase(), at)));
        self.marked.clear();
        let (tx, rx) = std::sync::mpsc::channel();
        self.advisor.deleting = Some(rx);
        self.advisor.delete_total = (total, DeleteVerb::Overwrite, paths.len());
        crate::state::text(&self.root, "FootLeft", "Overwriting and deleting...");
        self.delete_card(Some(0));
        std::thread::spawn(move || {
            let (mut results, mut notes, mut damaged) = (Vec::new(), Vec::new(), Vec::new());
            let mut before = 0u64;
            let mut kept_seen = confirmed;
            for path in paths {
                if stop.load(Ordering::Relaxed) {
                    notes.push(format!("{}: not overwritten, the run was stopped", path.display()));
                    continue;
                }
                let skipped = sprawl_advisor::exclusions::configured();
                if sprawl_advisor::exclusions::intersects(&path, &skipped) {
                    notes.push(format!("{}: not overwritten, it contains or is inside a folder now skipped in Settings > Scanning", path.display()));
                    continue;
                }
                // a Keep added after the question must still count: an unreadable list stops it
                if let Err(e) = sprawl_advisor::memory::refresh_keeps(&mut kept_seen) {
                    notes.push(format!("{}: not overwritten, {e}", path.display()));
                    continue;
                }
                let mut keep = kept_seen.clone();
                keep.extend(skipped);
                keep.extend(skipped_at_confirmation.iter().cloned());
                let spare = Spare { since: Some(since), keep, seen: seen.clone() };
                let outcome = overwrite::run(&path, &spare, &stop, &mut |n| {
                    let _ = tx.send(DelMsg::Progress(before + n));
                });
                before += outcome.bytes;
                damaged.extend(outcome.damaged);
                notes.extend(outcome.problems);
                if outcome.more_problems > 0 {
                    notes.push(format!("and {} more were left alone", outcome.more_problems));
                }
                if outcome.kept_new + outcome.kept_chosen + outcome.kept_protected > 0 {
                    notes.push(format!(
                        "{}: kept {} new, {} chosen and {} protected items",
                        path.display(),
                        outcome.kept_new,
                        outcome.kept_chosen,
                        outcome.kept_protected
                    ));
                }
                let _ = tx.send(DelMsg::Progress(before));
                if let Some(now) = crate::shellmenu::read_now(&path) {
                    results.push((path, now));
                } else {
                    notes.push(format!("{} could not be read again; scan again to refresh it", path.display()));
                }
            }
            let _ = tx.send(DelMsg::Done(results, notes, damaged));
        });
    }

    // ------------------------------------------------------------- free space

    /// File > Wipe Free Space: checked, then asked about the drive on show.
    pub(crate) fn wipe_free_ask(&mut self) {
        if let Some(why) = self.delete_busy_refusal() {
            self.tell("Not wiped", &format!("{why}\n\nThe wipe waits until scanning, Check again and other file operations finish; screenshot runs cannot wipe."));
            return;
        }
        let root = match self.wipe_drive_root() {
            Ok(root) => root,
            Err(why) => {
                self.tell("Not wiped", &why);
                return;
            }
        };
        let drives = crate::win32::drives();
        let Some(drive) = drives.iter().find(|d| d.root.eq_ignore_ascii_case(&root.to_string_lossy())) else {
            self.tell("Not wiped", &format!("{} isn't a drive Sprawl can see.", root.display()));
            return;
        };
        if drive.kind != 3 {
            self.tell("Not wiped", &format!("{} isn't a fixed local drive: a network, removable or CD drive can't have its free space wiped from here.", root.display()));
            return;
        }
        let marker_dir = match overwrite::wipe_data_dir() {
            Ok(dir) => dir,
            Err(why) => {
                self.tell("Not wiped", &why);
                return;
            }
        };
        let review = match overwrite::review_wipe(&root) {
            Ok(review) => review,
            Err(why) => {
                self.tell("Not wiped", &why);
                return;
            }
        };
        let plan = match overwrite::wipe_plan(&review, &marker_dir) {
            Ok(plan) => plan,
            Err(why) => {
                self.tell("Not wiped", &why);
                return;
            }
        };
        if let Some(why) = plan.refusal() {
            self.tell("Free space wipe not started", &why);
            return;
        }
        let letter = root.to_string_lossy();
        let letter = letter.trim_end_matches('\\');
        let mut body = String::new();
        body.push_str(&format!("Windows' cipher will overwrite the free space on {letter}: zeros, then ones, then random data. Every file on the drive stays as it is; only space nothing is using is written over.\n\n"));
        body.push_str(&format!("This approval is for volume {}, not for whatever may later use {letter}. ", review.volume_path().display()));
        body.push_str(if review.uses_system_drive() {
            "This is the volume running Windows. Sprawl rechecks its identity before starting cipher.\n\n"
        } else {
            "Sprawl uses a private mount on a different fixed volume. If it can't verify the mount, nothing is wiped.\n\n"
        });
        body.push_str("It can take hours, and the drive fills up while it runs: other programs may have less room until it finishes.\n\n");
        body.push_str("What this cannot promise:\n");
        for line in wipe_limits(review.volume_path()) {
            body.push_str(&format!("- {line}\n"));
        }
        body.push_str(&format!("- If cipher leaves {} behind, Sprawl won't delete it. Once cipher has stopped, check the folder in File Explorer and only delete it if it contains cipher's temporary data.\n", plan.scratch.display()));
        body.push_str(&format!("- {OLDER_COPIES}\n\n"));
        body.push_str("Wiping the free space makes deleted data harder to recover; it does not make it unrecoverable.");
        self.ask(&format!("Wipe free space on {letter}?"), &body, "Wipe free space", true, move |s| s.wipe_free_start(review, marker_dir));
    }

    /// The drive the File menu's wipe acts on: the row on show or the scan root's drive, else the
    /// one picked in the drive box. A folder on a network or device path is refused rather than
    /// quietly replaced by some other drive.
    fn wipe_drive_root(&self) -> Result<PathBuf, String> {
        if let Some(p) = self.selected_path() {
            return overwrite::drive_root(&p).ok_or_else(|| format!("{} isn't on a local drive. Pick a local drive to wipe.", p.display()));
        }
        if let Some(t) = &self.tree {
            if t.nodes.get(ROOT).is_some_and(|n| n.name != sprawl_core::MULTIPLE) {
                let p = t.path_of(ROOT);
                return overwrite::drive_root(&p).ok_or_else(|| format!("{} isn't on a local drive. Pick a local drive to wipe.", p.display()));
            }
        }
        let idx = find::<Selector>(&self.root, "DriveBox").and_then(|c| c.SelectedIndex()).unwrap_or(0).max(0) as usize;
        crate::win32::drives().get(idx).map(|d| PathBuf::from(&d.root)).ok_or_else(|| "Pick a drive to scan first, or scan a folder on the drive to wipe.".to_string())
    }

    /// "Yes": checked once more, then Windows' cipher on its own thread.
    fn wipe_free_start(&mut self, review: overwrite::WipeReview, marker_dir: PathBuf) {
        if self.advisor.wiping.is_some() {
            return;
        }
        // the world may have moved while the question was open
        if let Some(why) = self.delete_busy_refusal() {
            self.tell("Not wiped", &format!("{why}\n\nNothing was wiped."));
            return;
        }
        let plan = match overwrite::wipe_plan(&review, &marker_dir) {
            Ok(plan) => plan,
            Err(why) => {
                self.tell("Not wiped", &why);
                return;
            }
        };
        if let Some(why) = plan.refusal() {
            self.tell("Free space wipe not started", &why);
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.advisor.delete_stop = Some(stop.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        self.advisor.wiping = Some((rx, stop.clone()));
        let letter = review.label.to_string_lossy().trim_end_matches('\\').to_string();
        crate::state::text(&self.root, "FootLeft", &format!("Free space wipe for the volume reviewed as {letter}..."));
        self.wipe_card(true);
        crate::state::text(&self.root, "DeleteTitle", &format!("Free space wipe for the volume reviewed as {letter}..."));
        std::thread::spawn(move || {
            let w = overwrite::wipe_free_space(&review, &marker_dir, &stop);
            let _ = tx.send(w);
        });
    }

    /// Each frame: a free-space wipe that has finished.
    pub(crate) fn wipe_free_poll(&mut self) {
        let Some((rx, _)) = &self.advisor.wiping else { return };
        match rx.try_recv() {
            Ok(w) => {
                self.advisor.wiping = None;
                self.advisor.delete_stop = None;
                self.wipe_card(false);
                crate::state::text(&self.root, "FootLeft", "");
                if self.advisor.close_after_wipe {
                    return self.exit_now();
                }
                let (title, body) = wipe_words(&w);
                self.tell(title, &body);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.advisor.wiping = None;
                self.advisor.delete_stop = None;
                self.wipe_card(false);
                if self.advisor.close_after_wipe {
                    return self.exit_now();
                }
                self.tell("Free space wipe ended unexpectedly", "The wipe stopped without an answer. Scan the drive to see what is there now (nothing the drive holds was deleted by it).");
            }
        }
    }

    /// The card over the window while cipher runs. It doesn't block anything; Stop ends it.
    fn wipe_card(&self, show: bool) {
        use crate::bindings::Microsoft::UI::Xaml::FrameworkElement;
        let Ok(card) = crate::state::find::<crate::bindings::Microsoft::UI::Xaml::UIElement>(&self.root, "DeleteCard") else { return };
        if !show {
            let _ = card.SetVisibility(crate::bindings::Microsoft::UI::Xaml::Visibility::Collapsed);
            return;
        }
        crate::state::text(&self.root, "DeleteTitle", "Wiping free space...");
        crate::state::text(&self.root, "DeleteSub", "Sprawl checks the reviewed volume before running Windows' cipher. The wipe can take hours.");
        if let Ok(bar) = crate::state::find::<FrameworkElement>(&self.root, "DeleteBar") {
            let _ = bar.SetWidth(0.0);
        }
        let _ = card.SetVisibility(crate::bindings::Microsoft::UI::Xaml::Visibility::Visible);
    }
}

/// The limits a free-space wipe can't promise, each detected: solid-state (or a drive Windows
/// can't tell about), a copy-on-write volume. The shadow-copy line is added by the question.
fn wipe_limits(root: &Path) -> Vec<String> {
    let mut limits = Vec::new();
    match overwrite::solid_state(root) {
        Some(true) => limits.push("This drive is solid-state: it moves writes around, so deleted files' data may stay in cells the free-space wipe never reaches. The drive maker's secure erase, or keeping the drive encrypted with BitLocker, is the stronger answer.".to_string()),
        None => limits.push("Sprawl couldn't tell whether this drive is solid-state. If it is, it moves writes around, so deleted files' data may stay in cells the free-space wipe never reaches: the drive maker's secure erase, or BitLocker, is the stronger answer.".to_string()),
        Some(false) => {}
    }
    if overwrite::filesystem(root).is_some_and(|fs| fs.eq_ignore_ascii_case("ReFS")) {
        limits.push("This volume is ReFS, which copies data when it changes: writing over the free space may land in new places, leaving the old data.".to_string());
    }
    limits
}

/// The card after a wipe: its title says what really happened, and the body is what cipher said.
fn wipe_words(w: &sprawl_advisor::overwrite::Wiped) -> (&'static str, String) {
    let o = &w.outcome;
    let mut title = "Free space wiped";
    let mut words = if o.tool_unconfirmed {
        title = "Free space wipe: end not confirmed";
        if o.stopped {
            "Stop was requested, but Windows hasn't confirmed that cipher ended. It may still be running."
        } else if o.timed_out {
            "The wipe ran out of time, but Windows hasn't confirmed that cipher ended. It may still be running."
        } else {
            "Windows hasn't confirmed that cipher ended. It may still be running."
        }.to_string()
    } else if w.left_before_start {
        title = "Free space wipe not started";
        "Windows' cipher was not started. Nothing was overwritten.".to_string()
    } else if o.stopped {
        title = "Free space wiping stopped";
        "Windows' cipher was stopped before it finished. What it had written stays written; the rest of the free space was not overwritten.".to_string()
    } else if o.timed_out {
        title = "Free space wipe ran out of time";
        "Windows' cipher was still running after the week Sprawl allows and was ended. What it had written stays written; the rest of the free space was not overwritten.".to_string()
    } else if w.ok {
        "Windows' cipher wrote over the drive's free space three times: zeros, then ones, then random data. Deleted data is harder to recover now; it is not unrecoverable.".to_string()
    } else {
        title = "Free space wipe didn't finish";
        "Windows' cipher stopped with a problem. Its answer is below. Every file the drive holds is as it was; the free space may be partly overwritten.".to_string()
    };
    if !o.notes.is_empty() {
        words.push_str("\n\n");
        words.push_str(&o.notes.join("\n"));
    }
    if !o.problems.is_empty() {
        words.push_str("\n\n");
        words.push_str(&o.problems.join("\n"));
    }
    if !o.output.is_empty() {
        words.push_str("\n\ncipher said:\n");
        words.push_str(&o.output.join("\n"));
    }
    (title, words)
}

/// `path` is in a folder Windows (or OneDrive) keeps in step with the cloud.
fn in_cloud_folder(path: &Path) -> bool {
    let low = path.to_string_lossy().to_lowercase();
    ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"].iter().any(|v| {
        std::env::var_os(v).is_some_and(|dir| {
            let dir = dir.to_string_lossy().to_lowercase();
            !dir.is_empty() && low.starts_with(dir.trim_end_matches('\\'))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sprawl_advisor::{clean::Outcome, overwrite::Wiped};

    #[test]
    fn unconfirmed_cipher_end_takes_precedence_over_stop_and_timeout() {
        for stopped in [true, false] {
            let mut w = Wiped {
                outcome: Outcome { stopped, timed_out: !stopped, tool_unconfirmed: true, ..Default::default() },
                ok: false,
                left_before_start: false,
            };
            let (title, body) = wipe_words(&w);
            assert_eq!(title, "Free space wipe: end not confirmed");
            assert!(body.contains(if stopped { "Stop was requested" } else { "ran out of time" }));
            assert!(body.contains("may still be running") && !body.contains("was stopped") && !body.contains("was ended"));
            w.outcome.tool_unconfirmed = false;
            let (_, body) = wipe_words(&w);
            assert!(body.contains(if stopped { "was stopped" } else { "was ended" }));
        }
    }

    #[test]
    fn scratch_refusal_card_says_cipher_never_started_and_shows_manual_steps() {
        let w = Wiped {
            outcome: Outcome { problems: vec![r"Leave E:\EFSTMPWP alone unless you recognise cipher's temporary files.".into()], ..Default::default() },
            ok: false,
            left_before_start: true,
        };
        let (title, body) = wipe_words(&w);
        assert_eq!(title, "Free space wipe not started");
        assert!(body.contains("Nothing was overwritten") && body.contains(r"E:\EFSTMPWP"));
    }

    #[test]
    fn occupied_scratch_is_a_preflight_refusal_even_without_another_wipe() {
        let mut plan = overwrite::WipePlan {
            marker: PathBuf::from("fixture marker"), scratch: PathBuf::from(r"E:\EFSTMPWP"),
            scratch_present: true, marker_refusal: None,
        };
        let refusal = plan.refusal().unwrap();
        assert!(refusal.contains(r"E:\EFSTMPWP") && refusal.contains("won't start a wipe"));
        plan.scratch_present = false;
        assert!(plan.refusal().is_none());
    }
}
