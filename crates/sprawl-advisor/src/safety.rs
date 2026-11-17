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
    // anywhere: disk images, password vaults, mail stores, wallets
    r"?:\**\*.vhdx",
    r"?:\**\*.vhd",
    r"?:\**\*.avhdx",
    r"?:\**\*.vmdk",
    r"?:\**\*.vdi",
    r"?:\**\*.kdbx",
    r"?:\**\*.pst",
    r"?:\**\wallet.dat",
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

    /// A never-list entry found anywhere below node `n` (a disk image, a vault) by name.
    pub fn hit_inside(&self, tree: &Tree, n: usize) -> Option<(String, &str)> {
        let names: Vec<(&str, &str)> = self.patterns.iter().filter_map(|(s, p)| p.any_depth_name().map(|m| (m, s.as_str()))).collect();
        // what each name must end with (`*.vhdx` -> `.vhdx`), to skip the glob for most files
        let tails: Vec<&str> = names.iter().map(|(m, _)| m.trim_start_matches('*')).collect();
        let mut stack = tree.nodes[n].children.clone();
        while let Some(k) = stack.pop() {
            let raw = &tree.nodes[k].name;
            stack.extend_from_slice(&tree.nodes[k].children);
            if !tails.iter().any(|t| raw.len() >= t.len() && raw.is_char_boundary(raw.len() - t.len()) && raw[raw.len() - t.len()..].eq_ignore_ascii_case(t)) {
                continue;
            }
            let name = raw.to_lowercase();
            if let Some((_, s)) = names.iter().find(|(m, _)| crate::rules::seg_match(m, &name)) {
                return Some((tree.path_of(k).display().to_string(), s));
            }
        }
        None
    }

    /// The never-list entry a file or folder called `name` falls under anywhere (a disk image, a
    /// vault): for what appears after the scan, which [`Never::hit_inside`] can't see.
    pub fn name_hit(&self, name: &str) -> Option<&str> {
        let name = name.to_lowercase();
        self.patterns.iter().find_map(|(s, p)| p.any_depth_name().filter(|m| crate::rules::seg_match(m, &name)).map(|_| s.as_str()))
    }

    pub fn is_user_data(&self, path: &str) -> bool {
        let forms = drive_forms(path).unwrap_or_default();
        self.user_data.iter().any(|pat| forms.iter().any(|p| pat.matches(p)))
    }
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
    if !close_enough(f.bytes, actual) {
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
        if let Some((at, p)) = never.hit_inside(tree, n) {
            return Err(Rejection::Never(format!("{at} ({p})")));
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
                return Err(Rejection::LinksInside(tree.path_of(k).display().to_string()));
            }
            stack.extend_from_slice(&tree.nodes[k].children);
        }
    }
    let mut out = f.clone();
    out.bytes = actual;
    // Risk is only taken on trust from the built-in rules: anything else that removes files is
    // at least Amber (a local pack) or Orange (a model), whatever it claims.
    if removes && !f.builtin {
        let floor = if from_model { Risk::Orange } else { Risk::Amber };
        if out.risk < floor {
            out.risk = floor;
            out.evidence.push("Not from Sprawl's built-in rules, so check it yourself".into());
        }
    }
    // A build output found by its project's marker file is regenerable wherever it lives
    // (projects usually sit in Documents or source); anything else in the user's folders is theirs.
    let build_output = f.builtin && matches!(&f.source, crate::finding::Source::Rule(id) if id == "project.build-output");
    if never.is_user_data(&path) && out.risk < Risk::Orange && !build_output {
        out.risk = Risk::Orange;
        out.evidence.push("Inside your own files, so only you can decide".into());
    }
    let _ = segments;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
