//! What a scan covered: the Details link on the header's scan line. It shows only what that
//! scan recorded: how it was read and as whom, what couldn't be read, the folders skipped by
//! the settings in force when it ran, and the drives it didn't include. A CSV says what it can't.

use std::path::{Path, PathBuf};

use sprawl_core::ScanMethod;

use crate::state::State;
use crate::ui::esc;

/// Recorded when a scan is published (the tree itself keeps only its method and error count).
pub(crate) struct Coverage {
    /// The folders scanned (several for a scan of several drives); empty for a CSV.
    pub(crate) roots: Vec<PathBuf>,
    /// The folders to skip that were in force when it ran.
    pub(crate) skips: Vec<PathBuf>,
    pub(crate) elevated: bool,
}

/// What the sheet needs besides the record, gathered by the window.
pub(crate) struct Seen<'a> {
    pub(crate) method: ScanMethod,
    pub(crate) errors: u64,
    pub(crate) when: String,
    pub(crate) secs: f64,
    /// How many of the scanned folders are on a network share or a mapped network drive.
    pub(crate) network: usize,
    /// The fixed drives on this PC, `C:\` style.
    pub(crate) drives: &'a [String],
    pub(crate) filter: &'a str,
}

/// `path` is `root` or inside it (letters' case aside).
fn inside(path: &Path, root: &Path) -> bool {
    let p = path.to_string_lossy().to_lowercase();
    let r = root.to_string_lossy().to_lowercase();
    let r = r.trim_end_matches('\\');
    p == r || p.strip_prefix(r).is_some_and(|rest| rest.starts_with('\\'))
}

/// The sheet's rows, (label, text): the same words a test can read.
pub(crate) fn rows(c: &Coverage, s: &Seen) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let csv = s.method == ScanMethod::Csv;
    let several = c.roots.len() > 1;
    let all_network = s.network > 0 && s.network >= c.roots.len();
    let who = if c.elevated { "administrator" } else { "this account" };
    if !c.roots.is_empty() {
        out.push(("Scanned", c.roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join("\n")));
    }
    let how = match s.method {
        ScanMethod::Csv => "Loaded from a CSV file, not read from the disk now. What was skipped or couldn't be read when the file was made isn't in it.".to_string(),
        // one method for the whole tree doesn't say how each folder was read
        _ if several => format!(
            "Several folders, scanned together. Each was read the way it allows: from its drive's file table (MFT) when Sprawl runs as administrator and it is on a local NTFS drive, otherwise folder by folder as {who}{}. The methods can differ between them, and so can how compressed files are counted.",
            if s.network > 0 { "; network folders are only listed" } else { "" }
        ),
        ScanMethod::Mft => "Read from the drive's own file table (MFT), as administrator: the files the table records under the scanned folder are counted, also in folders this account can't open (Windows' own metadata files aren't). Compressed files count about what they take on disk.".to_string(),
        ScanMethod::Walk => format!(
            "Folder by folder, as {who}. Files Windows compressed with compact may show their full size rather than what they take on disk{}.",
            if s.network > 0 || c.elevated { "" } else { "; a scan as administrator reads the drive's file table, which shows it" }
        ),
    };
    out.push(("How", how));
    out.push(("When", if csv { format!("File saved {}", s.when) } else { format!("{}, took {:.2} s", s.when, s.secs) }));
    if !csv {
        let unread = if s.errors == 0 && s.method == ScanMethod::Mft && !several {
            // the table read opens no folders, so "every folder could be read" isn't known
            "No errors recorded: the file table was read whole. This method doesn't open folders one by one.".to_string()
        } else if s.errors == 0 {
            "No errors recorded: every folder it reached could be read.".to_string()
        } else {
            let more = if !c.elevated && s.method == ScanMethod::Walk && s.network == 0 { " Restart as administrator (Options) to read them." } else { "" };
            format!(
                "{} {} couldn't be read (access denied, or in use). Sprawl counts them but doesn't keep their names.{more}",
                crate::format::thousands(s.errors),
                if s.errors == 1 { "item" } else { "items" }
            )
        };
        out.push(("Unreadable", unread));
        let skipped: Vec<&PathBuf> = c.skips.iter().filter(|k| c.roots.iter().any(|r| inside(k, r))).collect();
        out.push((
            "Skipped",
            if skipped.is_empty() && !(s.network > 0 && !c.skips.is_empty()) {
                "Nothing by your settings. Folders to skip are set in Options > Settings > Scanning.".to_string()
            } else {
                let mut t: Vec<String> = skipped.iter().map(|k| k.display().to_string()).collect();
                if s.network > 0 && !c.skips.is_empty() {
                    t.push("A folder skipped on a mapped drive is skipped on its share too.".into());
                }
                t.push("By your settings (Options > Settings > Scanning): neither they nor what is in them are counted.".into());
                t.join("\n")
            },
        ));
    }
    if all_network {
        out.push(("Network", if several { "Network folders: they are only listed. Nothing is run, compressed or deleted there." } else { "A network folder: it is only listed. Nothing is run, compressed or deleted there." }.into()));
    } else if s.network > 0 {
        out.push(("Network", format!("Mixed: {} of these {} folders {} on the network, only listed (nothing is run, compressed or deleted there); the rest are on this PC.", s.network, c.roots.len(), if s.network == 1 { "is" } else { "are" })));
    }
    // drives this scan didn't include: not missing files, just not asked for
    let letters: Vec<String> = c.roots.iter().map(|r| r.to_string_lossy().to_string()).filter(|r| r.as_bytes().get(1) == Some(&b':')).map(|r| r[..2].to_uppercase()).collect();
    let others: Vec<&str> = s.drives.iter().map(|d| d.trim_end_matches('\\')).filter(|d| !letters.iter().any(|l| l.eq_ignore_ascii_case(d))).collect();
    if !csv && !others.is_empty() && !letters.is_empty() {
        out.push(("Not in it", format!("Other drives on this PC: {}. Pick one beside Scan to read it.", others.join(", "))));
    }
    if !s.filter.trim().is_empty() {
        out.push(("Filter", format!("\"{}\" is on: the views show only what matches it. Empty the Filter box to see everything.", s.filter.trim())));
    }
    out
}

impl State {
    /// The Details link: what this scan covered, in a card.
    pub(crate) fn coverage_details(&mut self) {
        let (Some(t), Some(c)) = (self.tree.clone(), self.coverage.as_ref()) else { return };
        let remote = crate::win32::drives();
        let network = c.roots.iter().filter(|r| {
            let s = r.to_string_lossy();
            s.starts_with(r"\\") || remote.iter().any(|d| d.kind == 4 && s.to_uppercase().starts_with(&d.root[..2]))
        }).count();
        let drives: Vec<String> = crate::win32::drives().into_iter().filter(|d| d.kind == 3).map(|d| d.root).collect();
        let csv = t.method == ScanMethod::Csv;
        let seen = Seen {
            method: t.method,
            errors: t.errors,
            when: crate::format::fmt_time(if csv { self.scan_began } else { self.scan_taken }),
            secs: self.scan_secs,
            network,
            drives: &drives,
            filter: &self.filter,
        };
        let body: String = rows(c, &seen)
            .into_iter()
            .map(|(label, text)| {
                format!(
                    r##"<Grid ColumnSpacing="14" Margin="0,0,0,8"><Grid.ColumnDefinitions><ColumnDefinition Width="92"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><TextBlock Text="{}" FontSize="12" Foreground="#FF8E97AA"/><TextBlock Grid.Column="1" Text="{}" FontSize="13" Foreground="#FFD5DBE8" TextWrapping="Wrap" IsTextSelectionEnabled="True"/></Grid>"##,
                    esc(label),
                    // line by line: esc turns a line break into a replacement character
                    text.split('\n').map(esc).collect::<Vec<_>>().join("&#10;")
                )
            })
            .collect();
        self.tell_rich("What this scan covers", format!(r##"<StackPanel MaxWidth="620">{body}</StackPanel>"##));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen<'a>(method: ScanMethod, errors: u64, network: bool, drives: &'a [String]) -> Seen<'a> {
        Seen { method, errors, when: "2026-11-19 10:00".into(), secs: 1.5, network: usize::from(network), drives, filter: "" }
    }

    fn text(rows: &[(&str, String)], label: &str) -> String {
        rows.iter().find(|(l, _)| *l == label).map(|(_, t)| t.clone()).unwrap_or_default()
    }

    #[test]
    fn a_folder_scan_names_what_it_left_out() {
        let drives = vec![r"C:\".to_string(), r"D:\".to_string()];
        let c = Coverage { roots: vec![PathBuf::from(r"C:\")], skips: vec![PathBuf::from(r"C:\Work\Private"), PathBuf::from(r"D:\Games")], elevated: false };
        let r = rows(&c, &seen(ScanMethod::Walk, 12, false, &drives));
        assert!(text(&r, "How").contains("as this account"));
        assert!(text(&r, "Unreadable").starts_with("12 items couldn't be read") && text(&r, "Unreadable").contains("Restart as administrator"));
        let skipped = text(&r, "Skipped");
        assert!(skipped.contains(r"C:\Work\Private") && !skipped.contains("Games"), "only the skips inside this scan: {skipped}");
        assert_eq!(text(&r, "Not in it"), "Other drives on this PC: D:. Pick one beside Scan to read it.");
        assert!(text(&r, "Network").is_empty());
    }

    #[test]
    fn mft_and_csv_say_what_they_know() {
        let drives = vec![r"C:\".to_string()];
        let c = Coverage { roots: vec![PathBuf::from(r"C:\")], skips: vec![], elevated: true };
        let r = rows(&c, &seen(ScanMethod::Mft, 0, false, &drives));
        assert!(text(&r, "How").contains("MFT") && !text(&r, "How").contains("every file"), "only what the table records (Astra 1.0.2 S4 #4)");
        assert!(text(&r, "Unreadable").starts_with("No errors recorded") && !text(&r, "Unreadable").contains("every folder"));
        assert!(text(&r, "Skipped").starts_with("Nothing by your settings"));
        assert!(text(&r, "Not in it").is_empty(), "the only drive is scanned");
        let csv = Coverage { roots: vec![], skips: vec![], elevated: false };
        let r = rows(&csv, &seen(ScanMethod::Csv, 0, false, &drives));
        assert!(text(&r, "How").contains("isn't in it"));
        assert!(text(&r, "When").starts_with("File saved"));
        assert!(text(&r, "Unreadable").is_empty() && text(&r, "Skipped").is_empty() && text(&r, "Not in it").is_empty());
    }

    #[test]
    fn a_share_is_only_listed() {
        let drives = vec![r"C:\".to_string()];
        let c = Coverage { roots: vec![PathBuf::from(r"\\nas\media")], skips: vec![PathBuf::from(r"Z:\Old")], elevated: false };
        let r = rows(&c, &seen(ScanMethod::Walk, 3, true, &drives));
        assert!(text(&r, "Network").contains("only listed"));
        assert!(!text(&r, "Unreadable").contains("administrator"), "elevation doesn't help a share");
        assert!(text(&r, "Skipped").contains("mapped drive"));
        assert!(!text(&r, "How").contains("file table"));
        assert!(text(&r, "Not in it").is_empty(), "drives aren't missing from a share's scan");
    }

    #[test]
    fn several_folders_say_their_methods_can_differ() {
        let drives = vec![r"C:\".to_string(), r"D:\".to_string()];
        let c = Coverage { roots: vec![PathBuf::from(r"C:\Data"), PathBuf::from(r"\\nas\media")], skips: vec![], elevated: true };
        let mut s = seen(ScanMethod::Mft, 0, true, &drives);
        s.network = 1;
        let r = rows(&c, &s);
        assert!(text(&r, "How").starts_with("Several folders") && text(&r, "How").contains("can differ"), "{}", text(&r, "How"));
        assert!(text(&r, "Network").starts_with("Mixed: 1 of these 2 folders is on the network"), "{}", text(&r, "Network"));
        assert!(text(&r, "Unreadable").starts_with("No errors recorded"));
        // an elevated folder walk (a FAT drive, say) doesn't suggest running as administrator
        let one = Coverage { roots: vec![PathBuf::from(r"G:\")], skips: vec![], elevated: true };
        assert!(!text(&rows(&one, &seen(ScanMethod::Walk, 0, false, &drives)), "How").contains("a scan as administrator"));
    }

    #[test]
    fn inside_means_the_folder_or_below() {
        assert!(inside(Path::new(r"c:\work\x"), Path::new(r"C:\")));
        assert!(inside(Path::new(r"C:\Work"), Path::new(r"C:\work")));
        assert!(!inside(Path::new(r"C:\Workshop"), Path::new(r"C:\Work")));
    }
}
