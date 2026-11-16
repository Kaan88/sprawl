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

/// Never deleted or moved by hand. `**` covers everything below. `?:` is any drive, so the
/// system folders of a second drive or a second Windows are covered too; the `%VAR%` entries
/// catch relocated folders.
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
    r"?:\Users\*\AppData\Local\Google\Chrome\User Data\**",
    r"?:\Users\*\AppData\Local\Microsoft\Edge\User Data\**",
    r"?:\Users\*\AppData\Roaming\Mozilla\Firefox\Profiles\**",
    r"?:\**\*.vhdx",
    r"%USERPROFILE%",
    r"%LOCALAPPDATA%",
    r"%APPDATA%",
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
];

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

    /// The never-list entry `path` falls under, if any.
    pub fn hit(&self, path: &str) -> Option<&str> {
        self.patterns.iter().find(|(_, p)| p.matches(path)).map(|(s, _)| s.as_str())
    }

    /// Every `%VAR%` entry could be worked out. When not, callers refuse destructive actions.
    pub fn complete(&self) -> bool {
        !self.incomplete
    }

    pub fn is_user_data(&self, path: &str) -> bool {
        self.user_data.iter().any(|p| p.matches(path))
    }
}

/// How far a claimed size may be from the scan's: files change between the scan and the check.
fn close_enough(claimed: u64, actual: u64) -> bool {
    claimed.abs_diff(actual) <= (actual / 100).max(1 << 20)
}

/// Check one finding against the tree and the disk. Returns it (with its risk raised if it
/// touches the user's own data) or the reason it was held back.
///
/// `tree` must be the full scan (not a filtered view, not a CSV import): sizes and links are
/// checked against it.
pub fn check(f: &Finding, tree: &Tree, never: &Never) -> Result<Finding, Rejection> {
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
    let on_path = f.command.as_deref().is_some_and(|c| c.contains(&crate::rules::ps_quote(&path)));
    // Acts on the files at `path` (rather than asking their owner to clean up).
    let removes = matches!(f.remedy, Remedy::Delete | Remedy::Move);
    let destructive = removes || on_path;
    let from_model = matches!(f.source, crate::finding::Source::Model(_));
    if destructive || from_model {
        if never.incomplete {
            return Err(Rejection::NeverListIncomplete);
        }
        // Curated built-in rules may run an official clean-up command on a protected path
        // (DISM on WinSxS); nothing else may touch one.
        let exempt = f.allow_protected && !removes && !from_model;
        if !exempt {
            if let Some(p) = never.hit(&path) {
                return Err(Rejection::Never(p.to_string()));
            }
        }
    }
    if destructive {
        let multi_top = tree.nodes[sprawl_core::ROOT].name == sprawl_core::MULTIPLE && tree.nodes[n].parent == Some(sprawl_core::ROOT);
        if n == sprawl_core::ROOT || multi_top {
            return Err(Rejection::ScanRoot);
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
    if never.is_user_data(&path) && out.risk < Risk::Orange {
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
