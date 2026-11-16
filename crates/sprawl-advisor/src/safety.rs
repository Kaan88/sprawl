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
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rejection::NotInTree => write!(f, "the path isn't in the scan"),
            Rejection::Missing => write!(f, "the path no longer exists"),
            Rejection::SizeMismatch { claimed, actual } => write!(f, "claimed {claimed} bytes but the scan has {actual}"),
            Rejection::Never(p) => write!(f, "{p} is on the never-touch list"),
            Rejection::ThroughLink(p) => write!(f, "{p} is a junction or link"),
        }
    }
}

/// Paths that must never be deleted or moved.
pub struct Never {
    patterns: Vec<(String, Pattern)>,
    user_data: Vec<Pattern>,
}

/// Never deleted or moved by hand. `**` covers everything below.
const NEVER: &[&str] = &[
    r"%SystemDrive%",
    r"%SystemRoot%",
    r"%SystemRoot%\**",
    r"%ProgramFiles%\**",
    r"%ProgramFiles(x86)%\**",
    r"%ProgramData%\Microsoft\**",
    r"%SystemDrive%\System Volume Information\**",
    r"%SystemDrive%\Recovery\**",
    r"%SystemDrive%\Boot\**",
    r"%SystemDrive%\$Recycle.Bin\**",
    r"%SystemDrive%\pagefile.sys",
    r"%SystemDrive%\hiberfil.sys",
    r"%SystemDrive%\swapfile.sys",
    r"%SystemDrive%\Users",
    r"%SystemDrive%\Users\*",
    r"%SystemDrive%\Users\*\AppData",
    r"%SystemDrive%\Users\*\AppData\Local",
    r"%SystemDrive%\Users\*\AppData\Roaming",
    r"%SystemDrive%\Users\*\NTUSER.DAT*",
    r"%SystemDrive%\Users\*\OneDrive*",
];

/// Folders that hold the user's own work.
const USER_DATA: &[&str] = &[
    r"%SystemDrive%\Users\*\Documents\**",
    r"%SystemDrive%\Users\*\Desktop\**",
    r"%SystemDrive%\Users\*\Pictures\**",
    r"%SystemDrive%\Users\*\Videos\**",
    r"%SystemDrive%\Users\*\Music\**",
    r"%SystemDrive%\Users\*\OneDrive*\**",
    r"%SystemDrive%\Users\*\source\**",
];

impl Never {
    pub fn default_list() -> Never {
        Never {
            patterns: NEVER.iter().map(|p| (p.to_string(), Pattern::new(p))).collect(),
            user_data: USER_DATA.iter().map(|p| Pattern::new(p)).collect(),
        }
    }

    /// The never-list entry `path` falls under, if any.
    pub fn hit(&self, path: &str) -> Option<&str> {
        self.patterns.iter().find(|(_, p)| p.matches(path)).map(|(s, _)| s.as_str())
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
pub fn check(f: &Finding, tree: &Tree, never: &Never) -> Result<Finding, Rejection> {
    let n = find_path(tree, &f.path).ok_or(Rejection::NotInTree)?;
    if std::fs::symlink_metadata(&f.path).is_err() {
        return Err(Rejection::Missing);
    }
    let actual = tree.nodes[n].size;
    if !close_enough(f.bytes, actual) {
        return Err(Rejection::SizeMismatch { claimed: f.bytes, actual });
    }
    let path = f.path.to_string_lossy();
    let destructive = matches!(f.remedy, Remedy::Delete | Remedy::Move);
    // Curated rules may run an official clean-up command on a protected path (DISM on WinSxS);
    // a model may not touch one at all.
    let from_model = matches!(f.source, crate::finding::Source::Model(_));
    if destructive || from_model {
        if let Some(p) = never.hit(&path) {
            return Err(Rejection::Never(p.to_string()));
        }
    }
    if destructive {
        // no junctions on the way down from the scan root
        let mut i = Some(n);
        while let Some(k) = i {
            if tree.nodes[k].is_reparse_point() {
                return Err(Rejection::ThroughLink(tree.path_of(k).display().to_string()));
            }
            i = tree.nodes[k].parent;
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
