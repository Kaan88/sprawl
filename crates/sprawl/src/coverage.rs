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
    /// How many of the roots were on a share or a network drive, as the scan that opened each
    /// one told (a letter mapped since, or during the scan, doesn't relabel it).
    pub(crate) network: usize,
    /// How many roots that couldn't be told for (or weren't reached: a stopped scan).
    pub(crate) unknown: usize,
}

/// (on the network, couldn't be told) for a scan of `roots`, from what its workers recorded
/// when they opened each root ([`sprawl_core::Progress::note_root`]). Nothing is asked of
/// any drive here (Astra 1.0.2 round 3, S4 #4).
pub(crate) fn network_roots(roots: &[PathBuf], seen: &[(PathBuf, Option<bool>)]) -> (usize, usize) {
    let network = seen.iter().filter(|(_, n)| *n == Some(true)).count();
    let told = seen.iter().filter(|(_, n)| n.is_some()).count();
    (network, roots.len().max(seen.len()) - told)
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
            "Several folders, scanned together. Each was read either from its drive's file table (MFT) or folder by folder as {who}, depending on its drive and Sprawl's rights{}. The methods can differ between them, and so can how compressed files are counted.",
            if s.network > 0 { "; network folders are only listed" } else { "" }
        ),
        ScanMethod::Mft => "Read from the drive's own file table (MFT), as administrator: the files the table records under the scanned folder are counted, also in folders this account can't open (Windows' own metadata files aren't). Compressed files count about what they take on disk.".to_string(),
        ScanMethod::Walk => format!(
            "Folder by folder, as {who}. Files Windows compressed with compact may show their full size rather than what they take on disk{}.",
            if s.network > 0 || c.elevated { "" } else { "; on a local NTFS drive, a scan as administrator may read the drive's file table, which shows it" }
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
            let more = if !c.elevated && s.method == ScanMethod::Walk && s.network == 0 { " Restarting as administrator (Options) may let Sprawl read some of them." } else { "" };
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
    let mut net = if all_network {
        if several { "Network folders: they are only listed. Nothing is run, compressed or deleted there." } else { "A network folder: it is only listed. Nothing is run, compressed or deleted there." }.to_string()
    } else if s.network > 0 {
        // each count said once: on the network, on this PC, couldn't be told (Astra 1.0.2
        // round 4, S4 #4: "the rest are on this PC" contradicted an unknown one)
        let unknown = if csv { 0 } else { c.unknown };
        let local = c.roots.len().saturating_sub(s.network + unknown);
        let mut t = format!("Mixed: {} of these {} folders {} on the network, only listed (nothing is run, compressed or deleted there)", s.network, c.roots.len(), if s.network == 1 { "is" } else { "are" });
        if local > 0 {
            t += &format!("; {local} {} on this PC", if local == 1 { "is" } else { "are" });
        }
        if unknown > 0 {
            t += &format!("; for {unknown}, whether {} on the network couldn't be told when the scan opened {}", if unknown == 1 { "it is" } else { "they are" }, if unknown == 1 { "it" } else { "them" });
        }
        t + "."
    } else {
        String::new()
    };
    if c.unknown > 0 && !csv && s.network == 0 {
        let which = if c.roots.len() == 1 { "this folder is".to_string() } else { format!("{} of these folders {}", c.unknown, if c.unknown == 1 { "is" } else { "are" }) };
        let gap = if net.is_empty() { "" } else { " " };
        net = format!("{net}{gap}Whether {which} on the network couldn't be told when the scan opened {}.", if c.unknown == 1 { "it" } else { "them" });
    }
    if !net.is_empty() { out.push(("Network", net)); }
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
        let network = c.network;
        // kinds only: no drive is asked for its name or anything else (a network one may hang)
        let drives = crate::win32::fixed_drives();
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
                    r##"<Grid ColumnSpacing="14" Margin="0,0,0,8"><Grid.ColumnDefinitions><ColumnDefinition Width="92"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><TextBlock Text="{}" FontSize="{{$fs12}}" Foreground="{{$text3}}"/><TextBlock Grid.Column="1" Text="{}" FontSize="{{$fs13}}" Foreground="{{$text1}}" TextWrapping="Wrap" IsTextSelectionEnabled="True"/></Grid>"##,
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
        let c = Coverage { roots: vec![PathBuf::from(r"C:\")], skips: vec![PathBuf::from(r"C:\Work\Private"), PathBuf::from(r"D:\Games")], elevated: false, network: 0, unknown: 0 };
        let r = rows(&c, &seen(ScanMethod::Walk, 12, false, &drives));
        assert!(text(&r, "How").contains("as this account"));
        assert!(text(&r, "Unreadable").starts_with("12 items couldn't be read") && text(&r, "Unreadable").contains("may let Sprawl read some"));
        assert!(text(&r, "How").contains("may read"), "elevation may, not will, use the file table (re-review S4 #4)");
        let skipped = text(&r, "Skipped");
        assert!(skipped.contains(r"C:\Work\Private") && !skipped.contains("Games"), "only the skips inside this scan: {skipped}");
        assert_eq!(text(&r, "Not in it"), "Other drives on this PC: D:. Pick one beside Scan to read it.");
        assert!(text(&r, "Network").is_empty());
    }

    #[test]
    fn mft_and_csv_say_what_they_know() {
        let drives = vec![r"C:\".to_string()];
        let c = Coverage { roots: vec![PathBuf::from(r"C:\")], skips: vec![], elevated: true, network: 0, unknown: 0 };
        let r = rows(&c, &seen(ScanMethod::Mft, 0, false, &drives));
        assert!(text(&r, "How").contains("MFT") && !text(&r, "How").contains("every file"), "only what the table records (Astra 1.0.2 S4 #4)");
        assert!(text(&r, "Unreadable").starts_with("No errors recorded") && !text(&r, "Unreadable").contains("every folder"));
        assert!(text(&r, "Skipped").starts_with("Nothing by your settings"));
        assert!(text(&r, "Not in it").is_empty(), "the only drive is scanned");
        let csv = Coverage { roots: vec![], skips: vec![], elevated: false, network: 0, unknown: 0 };
        let r = rows(&csv, &seen(ScanMethod::Csv, 0, false, &drives));
        assert!(text(&r, "How").contains("isn't in it"));
        assert!(text(&r, "When").starts_with("File saved"));
        assert!(text(&r, "Unreadable").is_empty() && text(&r, "Skipped").is_empty() && text(&r, "Not in it").is_empty());
    }

    #[test]
    fn a_share_is_only_listed() {
        let drives = vec![r"C:\".to_string()];
        let c = Coverage { roots: vec![PathBuf::from(r"\\nas\media")], skips: vec![PathBuf::from(r"Z:\Old")], elevated: false, network: 1, unknown: 0 };
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
        let c = Coverage { roots: vec![PathBuf::from(r"C:\Data"), PathBuf::from(r"\\nas\media")], skips: vec![], elevated: true, network: 0, unknown: 0 };
        let mut s = seen(ScanMethod::Mft, 0, true, &drives);
        s.network = 1;
        let r = rows(&c, &s);
        assert!(text(&r, "How").starts_with("Several folders") && text(&r, "How").contains("can differ"), "{}", text(&r, "How"));
        assert!(text(&r, "Network").starts_with("Mixed: 1 of these 2 folders is on the network"), "{}", text(&r, "Network"));
        assert!(text(&r, "Unreadable").starts_with("No errors recorded"));
        // an elevated folder walk (a FAT drive, say) doesn't suggest running as administrator
        let one = Coverage { roots: vec![PathBuf::from(r"G:\")], skips: vec![], elevated: true, network: 0, unknown: 0 };
        assert!(!text(&rows(&one, &seen(ScanMethod::Walk, 0, false, &drives)), "How").contains("a scan as administrator"));
    }

    #[test]
    fn the_network_label_comes_from_the_scan_that_opened_each_root() {
        let roots = [PathBuf::from(r"C:\Data"), PathBuf::from(r"Y:\media"), PathBuf::from(r"D:\x")];
        // as recorded: C: here, Y: a share, D: couldn't be told
        let seen_roots = [(roots[0].clone(), Some(false)), (roots[1].clone(), Some(true)), (roots[2].clone(), None)];
        assert_eq!(network_roots(&roots, &seen_roots), (1, 1));
        // a stopped scan that never opened the last root: not told either
        assert_eq!(network_roots(&roots, &seen_roots[..2]), (1, 1));
        let drives = vec![r"C:\".to_string()];
        let c = Coverage { roots: roots.to_vec(), skips: vec![], elevated: false, network: 1, unknown: 1 };
        let mut s = seen(ScanMethod::Walk, 0, true, &drives);
        s.network = 1;
        let net = text(&rows(&c, &s), "Network");
        // each count once, none contradicting another (round 4, S4 #4)
        assert_eq!(net, "Mixed: 1 of these 3 folders is on the network, only listed (nothing is run, compressed or deleted there); 1 is on this PC; for 1, whether it is on the network couldn't be told when the scan opened it.");
        assert!(!net.contains("the rest"));
        // two on this PC, none unknown
        let c4 = Coverage { roots: vec![roots[0].clone(), roots[1].clone(), roots[2].clone()], skips: vec![], elevated: false, network: 1, unknown: 0 };
        assert!(text(&rows(&c4, &s), "Network").ends_with("; 2 are on this PC."));
        // a real scan records its root from the handle it opened: this PC's temp folder
        let p = sprawl_core::Progress::new();
        assert!(sprawl_core::walk::inventory(&std::env::temp_dir(), &p).is_ok());
        assert_eq!(p.roots().len(), 1);
        assert_eq!(p.roots()[0].1, Some(false), "{:?}", p.roots());
    }

    #[test]
    fn inside_means_the_folder_or_below() {
        assert!(inside(Path::new(r"c:\work\x"), Path::new(r"C:\")));
        assert!(inside(Path::new(r"C:\Work"), Path::new(r"C:\work")));
        assert!(!inside(Path::new(r"C:\Workshop"), Path::new(r"C:\Work")));
    }
}
