//! Checks every finding must pass before it is shown, whoever produced it (a rule or a model).
//!
//! 1. The path must be in the scanned tree and still exist on disk.
//! 2. Its size must match the tree's (a model can't invent or inflate numbers).
//! 3. Nothing on the never-touch list may be deleted or moved, whatever the reasoning. Running
//!    the owner's own clean-up command (DISM, `Clear-RecycleBin`, ...) from a curated rule is still
//!    allowed; a model's finding on a protected path is always refused.
//! 4. Never through a junction or symbolic link: deleting through one removes the target's files.
//! 5. The user's own folders (Documents, Desktop, Pictures, ...) are at least `Orange`.

use std::fmt;

use sprawl_core::Tree;

use crate::finding::{Finding, Remedy, Risk};
use crate::rules::Pattern;
use crate::{find_path, segments};

#[derive(Clone, Debug, PartialEq)]
pub enum Rejection {
    NotInTree,
    Missing,
    SizeMismatch { claimed: u64, actual: u64 },
    Never(String),
    ThroughLink(String),
    /// A junction or link somewhere inside: a recursive delete could reach its target.
    LinksInside(String),
    /// The scan's own root (a whole drive or the folder the user picked).
    ScanRoot,
    /// Command text from a model or a local pack. Only the built-in pack's commands are run.
    UntrustedCommand,
    /// Red means information only.
    RedWithAction,
    /// A CSV import isn't the disk: nothing is offered from one.
    NotAScan,
    /// The protected folders couldn't all be worked out (an environment variable is missing),
    /// so nothing destructive is offered.
    NeverListIncomplete,
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rejection::NotInTree => write!(f, "the path isn't in the scan"),
            Rejection::Missing => write!(f, "the path no longer exists"),
            Rejection::SizeMismatch { claimed, actual } => write!(f, "claimed {claimed} bytes but the scan has {actual}"),
            Rejection::Never(p) => write!(f, "{p} is on the never-touch list"),
            Rejection::ThroughLink(p) => write!(f, "{p} is a junction or link"),
            Rejection::LinksInside(p) => write!(f, "{p} contains a junction or link"),
            Rejection::ScanRoot => write!(f, "it is the whole scanned folder or drive"),
            Rejection::UntrustedCommand => write!(f, "its command didn't come from the built-in rules"),
            Rejection::NotAScan => write!(f, "the tree was loaded from a CSV file, not scanned"),
            Rejection::RedWithAction => write!(f, "it is marked never-touch but carries an action"),
            Rejection::NeverListIncomplete => write!(f, "the protected Windows folders couldn't be worked out"),
        }
    }
}

/// Paths that must never be deleted or moved.
pub struct Never {
    patterns: Vec<(String, Pattern)>,
    user_data: Vec<Pattern>,
    /// A pattern kept an unexpanded `%VAR%`: fail closed.
    incomplete: bool,
}

/// Never deleted or moved by hand, nor anything that holds one of them. `**` covers everything
/// below. `?:` is any drive, so the system folders of a second drive or a second Windows are
/// covered too; the `%VAR%` entries catch relocated folders and profiles. `?:\**\<name>` entries
/// are also looked for inside any folder about to go.
const NEVER: &[&str] = &[
    r"?:",
    r"?:\Windows",
    r"?:\Windows\**",
    r"%SystemRoot%",
    r"%SystemRoot%\**",
    r"?:\Program Files",
    r"?:\Program Files\**",
    r"?:\Program Files (x86)",
    r"?:\Program Files (x86)\**",
    r"%ProgramFiles%\**",
    r"%ProgramFiles(x86)%\**",
    r"%ProgramW6432%\**",
    r"?:\ProgramData",
    r"?:\ProgramData\Microsoft\**",
    r"?:\ProgramData\Package Cache\**",
    r"%ProgramData%",
    r"%ProgramData%\Microsoft\**",
    r"?:\System Volume Information",
    r"?:\System Volume Information\**",
    r"?:\Recovery\**",
    r"?:\Boot\**",
    r"?:\EFI\**",
    r"?:\bootmgr",
    r"?:\BOOTNXT",
    r"?:\$WinREAgent\**",
    r"?:\$Recycle.Bin",
    r"?:\$Recycle.Bin\**",
    r"?:\pagefile.sys",
    r"?:\hiberfil.sys",
    r"?:\swapfile.sys",
    r"?:\DumpStack.log*",
    r"?:\Users",
    r"?:\Users\*",
    r"?:\Users\*\AppData",
    r"?:\Users\*\AppData\Local",
    r"?:\Users\*\AppData\LocalLow",
    r"?:\Users\*\AppData\Roaming",
    r"?:\Users\*\AppData\Local\Microsoft\**",
    r"?:\Users\*\AppData\Local\Packages\**",
    r"?:\Users\*\AppData\Roaming\Microsoft\**",
    r"?:\Users\*\NTUSER.DAT*",
    r"?:\Users\*\ntuser.ini",
    r"?:\Users\*\OneDrive*",
    r"?:\Users\*\.ssh\**",
    r"?:\Users\*\.gnupg\**",
    r"?:\Users\*\.aws\**",
    r"?:\Users\*\.azure\**",
    r"?:\Users\*\.kube\**",
    r"?:\Users\*\.docker\**",
    r"?:\Users\*\AppData\Local\Google\Chrome\User Data\**",
    r"?:\Users\*\AppData\Local\Microsoft\Edge\User Data\**",
    r"?:\Users\*\AppData\Local\BraveSoftware\Brave-Browser\User Data\**",
    r"?:\Users\*\AppData\Roaming\Mozilla\Firefox\Profiles\**",
    r"?:\Users\*\AppData\Roaming\Thunderbird\Profiles\**",
    r"?:\Users\*\AppData\Roaming\Bitcoin\**",
    r"?:\Users\*\AppData\Roaming\Ethereum\**",
    r"?:\Users\*\AppData\Roaming\Exodus\**",
    r"?:\Users\*\AppData\Roaming\Electrum\**",
    // a relocated profile
    r"%USERPROFILE%",
    r"%USERPROFILE%\AppData",
    r"%USERPROFILE%\NTUSER.DAT*",
    r"%USERPROFILE%\.ssh\**",
    r"%USERPROFILE%\.gnupg\**",
    r"%LOCALAPPDATA%",
    r"%LOCALAPPDATA%\Microsoft\**",
    r"%LOCALAPPDATA%\Packages\**",
    r"%APPDATA%",
    r"%APPDATA%\Microsoft\**",
    // password managers
    r"?:\Users\*\AppData\Roaming\Bitwarden\**",
    r"?:\Users\*\AppData\Local\1Password\**",
    // messengers: Signal's messages, attachments and key; Telegram's data; WhatsApp
    r"?:\Users\*\AppData\Roaming\Signal\sql\**",
    r"?:\Users\*\AppData\Roaming\Signal\attachments.noindex\**",
    r"?:\Users\*\AppData\Roaming\Signal\config.json",
    r"?:\Users\*\AppData\Roaming\WhatsApp\**",
    r"?:\Users\*\AppData\Local\WhatsApp\**",
    // other Chromium browsers' profiles
    r"?:\Users\*\AppData\Roaming\Opera Software\**",
    r"?:\Users\*\AppData\Local\Vivaldi\User Data\**",
    r"?:\Users\*\AppData\Local\Chromium\User Data\**",
    r"?:\Users\*\AppData\Local\Yandex\YandexBrowser\User Data\**",
    r"?:\Users\*\AppData\Local\Arc\User Data\**",
    // anywhere (a name, or the last folders of a path): disk images, password vaults, mail
    // stores, wallets, messengers' data
    r"?:\**\*.vhdx",
    r"?:\**\*.vhd",
    r"?:\**\*.avhdx",
    r"?:\**\*.vmdk",
    r"?:\**\*.vdi",
    r"?:\**\*.kdbx",
    r"?:\**\*.kdb",
    r"?:\**\*.keyx",
    r"?:\**\*.opvault",
    r"?:\**\*.agilekeychain",
    r"?:\**\*.1pux",
    r"?:\**\*.pst",
    r"?:\**\*.ost",
    r"?:\**\*.mbox",
    r"?:\**\wallet.dat",
    r"?:\**\Thunderbird\Profiles",
    r"?:\**\Thunderbird\Profiles\**",
    r"?:\**\ThunderbirdPortable\Data\profile",
    r"?:\**\ThunderbirdPortable\Data\profile\**",
    r"?:\**\Telegram Desktop\tdata",
    r"?:\**\Telegram Desktop\tdata\**",
];

/// Folders that hold the user's own work.
const USER_DATA: &[&str] = &[
    r"?:\Users\*\Documents\**",
    r"?:\Users\*\Desktop\**",
    r"?:\Users\*\Pictures\**",
    r"?:\Users\*\Videos\**",
    r"?:\Users\*\Music\**",
    r"?:\Users\*\Downloads\**",
    r"?:\Users\*\Saved Games\**",
    r"?:\Users\*\Favorites\**",
    r"?:\Users\*\Contacts\**",
    r"?:\Users\*\OneDrive*\**",
    r"?:\Users\*\Dropbox*\**",
    r"?:\Users\*\iCloud*\**",
    r"?:\Users\*\Google Drive*\**",
    r"?:\Users\*\source\**",
    // a relocated profile or redirected folders (unset variables simply match nothing)
    r"%USERPROFILE%\Documents\**",
    r"%USERPROFILE%\Desktop\**",
    r"%USERPROFILE%\Pictures\**",
    r"%USERPROFILE%\Videos\**",
    r"%USERPROFILE%\Music\**",
    r"%USERPROFILE%\Downloads\**",
    r"%OneDrive%\**",
    r"%OneDriveConsumer%\**",
    r"%OneDriveCommercial%\**",
];

/// Every `X:\...` spelling a path could stand for, with backslashes. `\\?\` and `\\.\` are
/// dropped. A network path `\\server\share\rest` is checked both as a drive root (`U:\rest`)
/// and as a folder on one (`U:\share\rest`, so a share named `Users` is still `Users`); an
/// admin share `C$` is `C:\rest` and `ADMIN$` is `C:\Windows\rest`. `None` (treated as
/// protected) for anything else, including `.` and `..` segments and names ending in a dot or
/// space, which Windows would resolve to some other path.
fn drive_forms(path: &str) -> Option<Vec<String>> {
    let p = path.replace('/', "\\");
    let p = if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = p.strip_prefix(r"\\?\").or_else(|| p.strip_prefix(r"\\.\")) {
        rest.to_string()
    } else {
        p
    };
    let out = if let Some(unc) = p.strip_prefix(r"\\") {
        let mut it = unc.splitn(3, '\\');
        let (server, share) = (it.next()?, it.next()?);
        let rest = it.next().unwrap_or("");
        if server.is_empty() || share.is_empty() {
            return None;
        }
        let sb = share.as_bytes();
        if share.eq_ignore_ascii_case("admin$") {
            vec![format!(r"C:\Windows\{rest}")]
        } else if sb.len() == 2 && sb[0].is_ascii_alphabetic() && sb[1] == b'$' {
            vec![format!(r"{}:\{rest}", sb[0] as char)]
        } else if share.ends_with('$') {
            return None; // IPC$, print$ and other hidden shares
        } else {
            vec![format!(r"U:\{rest}"), format!(r"U:\{share}\{rest}")]
        }
    } else {
        let b = p.as_bytes();
        if !(b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'\\')) {
            return None;
        }
        vec![p]
    };
    for f in &out {
        if f[2..].split('\\').filter(|s| !s.is_empty()).any(|s| s == "." || s == ".." || s.ends_with('.') || s.ends_with(' ')) {
            return None;
        }
    }
    Some(out)
}

impl Never {
    pub fn default_list() -> Never {
        let patterns: Vec<(String, Pattern)> = NEVER.iter().map(|p| (p.to_string(), Pattern::new(p))).collect();
        // %VAR% entries that couldn't be expanded protect nothing: fail closed. (The x86 and
        // W6432 Program Files variables may legitimately be missing.)
        let incomplete = NEVER
            .iter()
            .filter(|p| !p.contains("(x86)") && !p.contains("W6432"))
            .any(|p| crate::rules::expand(p).contains('%'));
        Never { patterns, user_data: USER_DATA.iter().map(|p| Pattern::new(p)).collect(), incomplete }
    }

    /// Every `%VAR%` entry could be worked out. When not, callers refuse destructive actions.
    pub fn complete(&self) -> bool {
        !self.incomplete
    }

    /// The never-list entry `path` falls under or holds, if any. A path in a form that isn't a
    /// drive or network path is refused outright.
    pub fn hit(&self, path: &str) -> Option<&str> {
        let Some(forms) = drive_forms(path) else { return Some("(a path Sprawl can't check)") };
        self.patterns.iter().find(|(_, pat)| forms.iter().any(|p| pat.matches(p) || pat.is_above(p))).map(|(s, _)| s.as_str())
    }

    /// The `?:\**\<tail>` entries: (tail segments, the entry, what a name must end with to be
    /// worth the glob: `*.vhdx` -> `.vhdx`, or "" when the last segment has other wildcards).
    fn tails(&self) -> Vec<(&[String], &str, &str)> {
        self.patterns
            .iter()
            .filter_map(|(s, p)| {
                let t = p.any_depth_tail()?;
                let last = t.last()?.trim_start_matches('*');
                let quick = if last.contains(['*', '?']) { "" } else { last };
                Some((t, s.as_str(), quick))
            })
            .collect()
    }

    /// Every never-list item below node `n` found by the end of its path (a disk image, a vault,
    /// a `Thunderbird\Profiles` folder), not looking inside one found: node and the entry. What
    /// Sprawl's own clean-up skips by [`Never::path_hit`].
    pub fn hits_inside(&self, tree: &Tree, n: usize) -> Vec<(usize, &str)> {
        let tails = self.tails();
        let mut out = Vec::new();
        let mut stack = tree.nodes[n].children.clone();
        while let Some(k) = stack.pop() {
            let raw = &tree.nodes[k].name;
            let maybe = tails.iter().any(|(_, _, q)| q.is_empty() || (raw.len() >= q.len() && raw.is_char_boundary(raw.len() - q.len()) && raw[raw.len() - q.len()..].eq_ignore_ascii_case(q)));
            if maybe {
                // the node's name and those of the folders above it, last first
                let mut names = Vec::new();
                let mut i = Some(k);
                while let Some(j) = i.filter(|_| names.len() < 4) {
                    names.push(tree.nodes[j].name.to_lowercase());
                    i = tree.nodes[j].parent;
                }
                if let Some((_, s, _)) = tails.iter().find(|(t, _, _)| ends_with(&names, t)) {
                    out.push((k, *s));
                    continue;
                }
            }
            stack.extend_from_slice(&tree.nodes[k].children);
        }
        out
    }

    /// The first never-list item below node `n` (see [`Never::hits_inside`]), by path.
    pub fn hit_inside(&self, tree: &Tree, n: usize) -> Option<(String, &str)> {
        self.hits_inside(tree, n).first().map(|&(k, s)| (tree.path_of(k).display().to_string(), s))
    }

    /// The never-list entry the item at `path` falls under by the end of its path (a disk image,
    /// a vault, a `Thunderbird\Profiles` folder): for what Sprawl's clean-up meets on the way,
    /// including what appeared after the scan, which [`Never::hits_inside`] can't see.
    pub fn path_hit(&self, path: &std::path::Path) -> Option<&str> {
        let mut names: Vec<String> = Vec::new();
        for c in path.components().rev().take(4) {
            names.push(c.as_os_str().to_string_lossy().to_lowercase());
        }
        self.tails().into_iter().find(|(t, _, _)| ends_with(&names, t)).map(|(_, s, _)| s)
    }

    pub fn is_user_data(&self, path: &str) -> bool {
        let forms = drive_forms(path).unwrap_or_default();
        self.user_data.iter().any(|pat| forms.iter().any(|p| pat.matches(p)))
    }
}

/// `names` (an item's name, then the folders above it) ends in `tail` (segments, the item last).
fn ends_with(names: &[String], tail: &[String]) -> bool {
    tail.len() <= names.len() && tail.iter().rev().zip(names).all(|(t, n)| crate::rules::seg_match(t, n))
}

/// How far a claimed size may be from the scan's: files change between the scan and the check.
fn close_enough(claimed: u64, actual: u64) -> bool {
    claimed.abs_diff(actual) <= (actual / 100).max(1 << 20)
}

/// Check one finding against the tree and the disk. Returns it (with its risk raised if it
/// touches the user's own data or doesn't come from the built-in rules) or the reason it was
/// held back.
///
/// `tree` must be the full scan (not a filtered view): sizes and links are checked against it.
pub fn check(f: &Finding, tree: &Tree, never: &Never) -> Result<Finding, Rejection> {
    if tree.method == sprawl_core::ScanMethod::Csv {
        return Err(Rejection::NotAScan);
    }
    let n = find_path(tree, &f.path).ok_or(Rejection::NotInTree)?;
    if std::fs::symlink_metadata(&f.path).is_err() {
        return Err(Rejection::Missing);
    }
    let actual = tree.nodes[n].size;
    // checked again (just before a clean-up): its size already leaves out what was carved
    let carved_before: u64 = f.carved.iter().map(|c| c.bytes).sum();
    if !close_enough(f.bytes, actual) && !(carved_before > 0 && close_enough(f.bytes, actual.saturating_sub(carved_before))) {
        return Err(Rejection::SizeMismatch { claimed: f.bytes, actual });
    }
    // Command text is only ever taken from the built-in pack; a model or a local pack names a
    // remedy and the program writes the command.
    if f.command.is_some() && !f.trusted {
        return Err(Rejection::UntrustedCommand);
    }
    if f.risk == Risk::Red && (f.command.is_some() || !matches!(f.remedy, Remedy::Info)) {
        return Err(Rejection::RedWithAction);
    }
    let path = f.path.to_string_lossy();
    let from_model = matches!(f.source, crate::finding::Source::Model(_));
    let removes = matches!(f.remedy, Remedy::Delete | Remedy::Move);
    // A curated built-in rule may run its owner's official clean-up (DISM on WinSxS) on a
    // protected path; the tool decides what goes, not a recursive delete.
    let exempt = f.allow_protected && f.builtin && f.trusted && f.remedy == Remedy::Command && !from_model;
    // Every other command is treated as acting on the files at `path`, and so is an exempt
    // one that names `path` itself (a recursive Remove-Item): only the never-list is waived.
    let destructive = removes || (f.command.is_some() && (!exempt || f.on_path));
    // Sprawl's own clean-up of a rule's finding: it skips never-list names at any depth and
    // removes links as links, so protected items inside are carved out rather than vetoing the
    // finding (a rule's "command" remedy with a typed clear is Sprawl's own run too). A move, a
    // tool's command or a model's finding keeps the veto.
    let own_run = matches!(f.remedy, Remedy::Delete | Remedy::Command)
        && !from_model
        && matches!(f.action, Some(crate::finding::Action::Remove | crate::finding::Action::Clear { .. } | crate::finding::Action::Match { .. }))
        && crate::clean::action(f).is_some();
    let mut carved = Vec::new();
    let mut links = 0usize;
    if (destructive || from_model) && !exempt {
        if never.incomplete {
            return Err(Rejection::NeverListIncomplete);
        }
        if let Some(p) = never.hit(&path) {
            return Err(Rejection::Never(p.to_string()));
        }
    }
    if destructive {
        let multi_top = tree.nodes[sprawl_core::ROOT].name == sprawl_core::MULTIPLE && tree.nodes[n].parent == Some(sprawl_core::ROOT);
        if n == sprawl_core::ROOT || multi_top {
            return Err(Rejection::ScanRoot);
        }
        for (k, p) in never.hits_inside(tree, n) {
            if !own_run {
                return Err(Rejection::Never(format!("{} ({p})", tree.path_of(k).display())));
            }
            carved.push(crate::finding::Carve { path: tree.path_of(k), bytes: tree.nodes[k].size, rule: p.to_string() });
        }
        // no junctions on the way down from the scan root
        let mut i = Some(n);
        while let Some(k) = i {
            if tree.nodes[k].is_reparse_point() {
                return Err(Rejection::ThroughLink(tree.path_of(k).display().to_string()));
            }
            i = tree.nodes[k].parent;
        }
        // nor anywhere inside: a recursive delete may follow one into its target
        let mut stack = tree.nodes[n].children.clone();
        while let Some(k) = stack.pop() {
            if tree.nodes[k].is_reparse_point() {
                if !own_run {
                    return Err(Rejection::LinksInside(tree.path_of(k).display().to_string()));
                }
                links += 1;
            }
            stack.extend_from_slice(&tree.nodes[k].children);
        }
    }
    let mut out = f.clone();
    let kept: u64 = carved.iter().map(|c| c.bytes).sum();
    out.bytes = actual.saturating_sub(kept);
    // a check made again doesn't repeat its own lines
    out.evidence.retain(|e| !e.starts_with(CARVED_LINE) && !e.ends_with(LINKS_LINE));
    if !carved.is_empty() {
        let first = &carved[0];
        let more = if carved.len() > 1 { format!(" and {} more", carved.len() - 1) } else { String::new() };
        out.evidence.push(format!(
            "{CARVED_LINE} ({} in all): {}{more}, on the never-list as {}",
            sprawl_core::format_size(kept),
            first.path.display(),
            first.rule
        ));
    }
    out.carved = carved;
    if links > 0 {
        out.evidence.push(format!("{links} junction or link{} inside{LINKS_LINE}", if links == 1 { "" } else { "s" }));
    }
    // Risk is only taken on trust from the built-in rules: anything else that removes files is
    // at least Amber (a local pack) or Orange (a model), whatever it claims.
    if removes && !f.builtin {
        let floor = if from_model { Risk::Orange } else { Risk::Amber };
        if out.risk < floor {
            out.risk = floor;
            out.evidence.push("Not from Sprawl's built-in rules, so check it yourself".into());
        }
    }
    // Inside another account's profile: never green, and never ticked or planned for the user
    // (see `Finding::other_account`)
    other_account_floor(&mut out, f.other_account());
    // A build output found by its project's marker file is regenerable wherever it lives
    // (projects usually sit in Documents or source); anything else in the user's folders is theirs.
    let build_output = f.builtin && matches!(&f.source, crate::finding::Source::Rule(id) if crate::projects::is_build_output(id));
    if never.is_user_data(&path) && out.risk < Risk::Orange && !build_output {
        out.risk = Risk::Orange;
        out.evidence.push("Inside your own files, so only you can decide".into());
    }
    let _ = segments;
    Ok(out)
}

const CARVED_LINE: &str = "Keeps what is protected inside";
const LINKS_LINE: &str = ": removed as links, what they point to is not touched";

/// A delete or command inside another account's profile is at least Amber.
fn other_account_floor(out: &mut Finding, other_account: bool) {
    let acts = matches!(out.remedy, Remedy::Delete | Remedy::Move | Remedy::Command);
    if acts && other_account && out.risk < Risk::Amber {
        out.risk = Risk::Amber;
        out.evidence.push("In another account's folder, so Sprawl doesn't tick it for you".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_account_is_never_green() {
        let mut f = Finding {
            path: r"Z:\Users\other\AppData\Local\Temp".into(),
            owner: None,
            bytes: 1,
            risk: Risk::Green,
            remedy: Remedy::Delete,
            command: None,
            title: String::new(),
            evidence: vec![],
            confidence: 1.0,
            source: crate::finding::Source::Rule("t".into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: true,
            action: None,
            admin: false,
            carved: Vec::new(),
        };
        assert!(f.other_account());
        let mut out = f.clone();
        other_account_floor(&mut out, f.other_account());
        assert_eq!(out.risk, Risk::Amber);
        // not another account's (own folders, Windows', a project on D:): untouched
        let mut mine = f.clone();
        other_account_floor(&mut mine, false);
        assert_eq!(mine.risk, Risk::Green);
        f.remedy = Remedy::Info;
        let mut info = f.clone();
        other_account_floor(&mut info, true);
        assert_eq!(info.risk, Risk::Green);
    }

    #[test]
    fn mail_vaults_messengers_and_browsers_are_protected() {
        let never = Never::default_list();
        for p in [
            r"C:\Users\sam\AppData\Local\Temp\x\archive.ost",
            r"D:\Mail\export.mbox",
            r"C:\Users\sam\AppData\Local\Temp\old.kdb",
            r"E:\keys\me.keyx",
            r"C:\Users\sam\Dropbox\Me.opvault",
            r"C:\Users\sam\Downloads\backup.1pux",
            r"D:\Portable\Thunderbird\Profiles",
            r"D:\Portable\Thunderbird\Profiles\abc.default\Mail\Inbox",
            r"D:\Apps\ThunderbirdPortable\Data\profile\prefs.js",
            r"D:\Apps\Telegram Desktop\tdata",
            r"C:\Users\sam\AppData\Roaming\Telegram Desktop\tdata\user_data",
            r"C:\Users\sam\AppData\Roaming\Bitwarden\data.json",
            r"C:\Users\sam\AppData\Local\1Password\data\1password.sqlite",
            r"C:\Users\sam\AppData\Roaming\Signal\sql\db.sqlite",
            r"C:\Users\sam\AppData\Roaming\Signal\attachments.noindex\ab\cd",
            r"C:\Users\sam\AppData\Roaming\Signal\config.json",
            r"C:\Users\sam\AppData\Roaming\Signal",
            r"C:\Users\sam\AppData\Roaming\WhatsApp\IndexedDB",
            r"C:\Users\sam\AppData\Roaming\Opera Software\Opera Stable\Cookies",
            r"C:\Users\sam\AppData\Local\Vivaldi\User Data\Default\Login Data",
            r"C:\Users\sam\AppData\Local\Chromium\User Data\Default",
            r"C:\Users\sam\AppData\Local\Arc\User Data\Default",
        ] {
            assert!(never.hit(p).is_some(), "{p}");
        }
        // their caches, and look-alikes, are not
        for p in [r"C:\Users\sam\AppData\Roaming\Signal\Cache", r"C:\Users\sam\AppData\Local\Opera Software\Opera Stable\Cache", r"D:\x\Profiles", r"D:\x\tdata", r"D:\x\notes.db"] {
            assert!(never.hit(p).is_none(), "{p}");
        }
        // found inside a folder by the end of their path, and skipped by the clean-up
        assert!(never.path_hit(std::path::Path::new(r"D:\Portable\Thunderbird\Profiles")).is_some());
        assert!(never.path_hit(std::path::Path::new(r"D:\x\Profiles")).is_none());
        assert!(never.path_hit(std::path::Path::new(r"D:\x\mail.OST")).is_some());
    }

    #[test]
    fn never_list() {
        std::env::set_var("SystemDrive", "C:");
        std::env::set_var("SystemRoot", r"C:\Windows");
        let n = Never::default_list();
        assert!(n.hit(r"C:\Windows\System32\drivers").is_some());
        assert!(n.hit(r"C:\").is_some());
        assert!(n.hit(r"C:\Users\sam").is_some());
        assert!(n.hit(r"C:\Users\sam\AppData\Local").is_some());
        assert!(n.hit(r"C:\Users\sam\AppData\Local\npm-cache").is_none());
        assert!(n.is_user_data(r"C:\Users\sam\Documents\big.iso"));
        assert!(!n.is_user_data(r"C:\Users\sam\AppData\Local\Temp"));
    }

    #[test]
    fn size_tolerance() {
        assert!(close_enough(100, 100));
        assert!(close_enough(1_000_000_000, 1_005_000_000));
        assert!(!close_enough(2_000_000_000, 1_000_000_000));
    }
}
