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
    // what a program built on Chromium keeps for its user: sign-ins, site data, saved forms.
    // By Chromium's own layout, so a code folder called "cookies" isn't taken for one
    r"?:\**\Network\Cookies",
    r"?:\**\Default\Cookies",
    r"?:\**\Profile *\Cookies",
    r"?:\**\Local Storage\leveldb",
    r"?:\**\IndexedDB\*.indexeddb.leveldb",
    r"?:\**\IndexedDB\*.indexeddb.blob",
    r"?:\**\Login Data",
    r"?:\**\Web Data",
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

/// A cache folder *below* a never-list entry that a built-in cleaner part may clear: what
/// `docs/plans/cleaners.md` calls a "carve-out" (milestone 4). Not to be confused with
/// [`Finding::carved`] and [`Never::hits_inside`], which are protected items kept *inside* a
/// folder that is cleaned: those are left, these are allowed.
///
/// Exact and compiled in: `<user data>\<profile>\<leaf>`, where `<profile>` is `Default` or
/// `Profile <digits>` ([`chromium_profile_name`]) and `<leaf>` one of the cache folder names,
/// each matched whole. No `**`, no wildcard in the last two names. Only a part of the built-in
/// pack whose cleaner id is `cleaner`, clearing that folder (never removing it or anything
/// above it), may use one; see [`check`]. A pack on disk can't name or add one: it isn't
/// built-in, and its paths are held against the never-list when it is loaded.
#[derive(Debug)]
pub struct CacheCarveOut {
    pub cleaner: &'static str,
    /// The browser's `User Data` folder, any account (`?:\Users\*`) and this one (`%LOCALAPPDATA%`).
    pub user_data: &'static [&'static str],
    pub leaves: &'static [&'static str],
}

/// What Chromium keeps in a profile as caches only: pages, images and scripts (`Cache`), compiled
/// scripts (`Code Cache`), compiled graphics shaders (`GPUCache`).
const CHROMIUM_CACHES: &[&str] = &["Cache", "Code Cache", "GPUCache"];

pub const CACHE_CARVE_OUTS: &[CacheCarveOut] = &[
    CacheCarveOut { cleaner: "app.chrome", user_data: &[r"?:\Users\*\AppData\Local\Google\Chrome\User Data", r"%LOCALAPPDATA%\Google\Chrome\User Data"], leaves: CHROMIUM_CACHES },
    CacheCarveOut { cleaner: "app.edge", user_data: &[r"?:\Users\*\AppData\Local\Microsoft\Edge\User Data", r"%LOCALAPPDATA%\Microsoft\Edge\User Data"], leaves: CHROMIUM_CACHES },
    CacheCarveOut { cleaner: "app.brave", user_data: &[r"?:\Users\*\AppData\Local\BraveSoftware\Brave-Browser\User Data", r"%LOCALAPPDATA%\BraveSoftware\Brave-Browser\User Data"], leaves: CHROMIUM_CACHES },
];

/// A Chromium profile folder's name as Chromium makes them: `Default`, or `Profile ` and a number
/// (`Profile 1`, `Profile 12`). Not `Guest Profile`, `System Profile`, `Profile Backup` or
/// anything else that only starts the same.
pub fn chromium_profile_name(name: &str) -> bool {
    if name.eq_ignore_ascii_case("Default") {
        return true;
    }
    let Some(n) = name.get(..8).filter(|p| p.eq_ignore_ascii_case("Profile ")).map(|_| &name[8..]) else { return false };
    !n.is_empty() && n.len() <= 4 && n.bytes().all(|b| b.is_ascii_digit())
}

/// Every carve-out as one pattern per line, `<profile>` standing for [`chromium_profile_name`]:
/// for the report and the tests.
pub fn cache_carve_out_patterns() -> Vec<(&'static str, String)> {
    CACHE_CARVE_OUTS.iter().flat_map(|c| c.user_data.iter().flat_map(move |u| c.leaves.iter().map(move |l| (c.cleaner, format!(r"{u}\<profile>\{l}"))))).collect()
}

/// The carve-out `path` is exactly (the folder itself, not anything inside it), if any.
pub fn cache_carve_out(path: &str) -> Option<&'static CacheCarveOut> {
    cache_carve_out_in(path, CACHE_CARVE_OUTS)
}

fn cache_carve_out_in<'a>(path: &str, list: &'a [CacheCarveOut]) -> Option<&'a CacheCarveOut> {
    // a plain local path only (`X:\...`): never a network or device path, which the never-list
    // reads in several ways to be on the safe side
    let b = path.as_bytes();
    if !(b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\') || path.contains('/') {
        return None;
    }
    let forms = drive_forms(path)?;
    for form in forms {
        let segs: Vec<&str> = form.split('\\').filter(|s| !s.is_empty()).collect();
        if segs.len() < 4 {
            continue;
        }
        let (leaf, profile) = (segs[segs.len() - 1], segs[segs.len() - 2]);
        let parent = segs[..segs.len() - 2].join("\\");
        if !chromium_profile_name(profile) {
            continue;
        }
        if let Some(c) = list.iter().find(|c| c.leaves.iter().any(|l| l.eq_ignore_ascii_case(leaf)) && c.user_data.iter().any(|u| Pattern::new(u).matches(&parent))) {
            return Some(c);
        }
    }
    None
}

/// Whether finding `f` at `path` (on the never-list) may go ahead as a carve-out: a part of the
/// built-in pack, of the cleaner the carve-out names, clearing exactly that folder in this
/// account's profile, and every never-list entry `path` falls under is a whole folder it lies
/// below (a browser's `User Data\**`, `AppData\Local\Microsoft\**`): never one found by its
/// name (`Login Data`, `*.kdbx`, `Network\Cookies`), which no carve-out waives.
fn cache_carve_out_applies(f: &Finding, path: &str, never: &Never, list: &[CacheCarveOut], home: Option<&std::path::Path>) -> bool {
    let Some(c) = cache_carve_out_in(path, list) else { return false };
    let part_of_cleaner = matches!(&f.source, crate::finding::Source::Rule(id) if id.len() > c.cleaner.len() + 1 && id.starts_with(c.cleaner) && id.as_bytes()[c.cleaner.len()] == b'/');
    let clears = matches!(f.action, Some(crate::finding::Action::Clear { .. })) && matches!(crate::clean::action(f), Some((crate::finding::Action::Clear { .. }, _)));
    let hits = never.all_hits(path);
    part_of_cleaner
        && clears
        && f.builtin
        && f.trusted
        && f.remedy == Remedy::Delete
        && !crate::finding::other_account(&f.path, home)
        && !hits.is_empty()
        && hits.iter().all(|h| is_folder_cover(h))
}

/// A never-list entry that protects a whole folder by where it is (`...\User Data\**`), as
/// opposed to one that names an item found anywhere (`?:\**\Login Data`) or one exact path.
fn is_folder_cover(entry: &str) -> bool {
    entry.ends_with(r"\**") && !entry.starts_with(r"?:\**")
}

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

    /// Every never-list entry `path` falls under or holds (as [`Never::hit`], not only the first).
    pub fn all_hits(&self, path: &str) -> Vec<&str> {
        let Some(forms) = drive_forms(path) else { return vec!["(a path Sprawl can't check)"] };
        self.patterns.iter().filter(|(_, pat)| forms.iter().any(|p| pat.matches(p) || pat.is_above(p))).map(|(s, _)| s.as_str()).collect()
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
    let home = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from);
    check_in(f, tree, never, CACHE_CARVE_OUTS, home.as_deref())
}

/// [`check`] with the cache carve-outs given (the tests' fixtures can't sit in `X:\Users\...`).
fn check_in(f: &Finding, tree: &Tree, never: &Never, carve_outs: &[CacheCarveOut], home: Option<&std::path::Path>) -> Result<Finding, Rejection> {
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
            // a cache folder inside a browser profile, for a built-in part (milestone 4): only
            // this check is waived; everything below (links, never-list names inside, the
            // scan's root) still applies
            if from_model || !cache_carve_out_applies(f, &path, never, carve_outs, home) {
                return Err(Rejection::Never(p.to_string()));
            }
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
            gain: None,
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

    /// Milestone 4: the whole list of cache carve-outs, written out; and nothing a browser
    /// profile keeps for its user (nor anything else in `User Data`) can be one of them.
    #[test]
    fn cache_carve_outs_are_exactly_these_caches() {
        let got: Vec<String> = cache_carve_out_patterns().into_iter().map(|(c, p)| format!("{c}: {p}")).collect();
        let mut want = Vec::new();
        for (c, base) in [("app.chrome", r"Google\Chrome"), ("app.edge", r"Microsoft\Edge"), ("app.brave", r"BraveSoftware\Brave-Browser")] {
            for root in [format!(r"?:\Users\*\AppData\Local\{base}\User Data"), format!(r"%LOCALAPPDATA%\{base}\User Data")] {
                for leaf in ["Cache", "Code Cache", "GPUCache"] {
                    want.push(format!(r"{c}: {root}\<profile>\{leaf}"));
                }
            }
        }
        assert_eq!(got, want);
        for (_, p) in cache_carve_out_patterns() {
            assert!(!p.contains("**"), "{p}");
            let segs: Vec<&str> = p.split('\\').collect();
            // the profile and the leaf are named; no wildcard after `Users\*`
            assert!(!segs[segs.len() - 1].contains(['*', '?']) && segs[segs.len() - 2] == "<profile>", "{p}");
            assert!(segs[4..].iter().all(|s| !s.contains(['*', '?'])), "{p}");
            assert!(segs[segs.len() - 1].contains("Cache"), "{p}");
        }
        // what a profile keeps for its user, and what else sits in a profile or in User Data
        const KEPT: &[&str] = &[
            "Login Data", "Login Data-journal", "Login Data For Account", "Bookmarks", "Bookmarks.bak", "Cookies", "Cookies-journal", "Network", r"Network\Cookies",
            "History", "History-journal", "Web Data", "Preferences", "Secure Preferences", "Local Storage", r"Local Storage\leveldb", "IndexedDB", "Session Storage", "Sessions",
            "Current Session", "Current Tabs", "Last Session", "Last Tabs", "Extensions", "Local Extension Settings", "Extension State", "Sync Data", "Sync Extension Settings",
            "Service Worker", r"Service Worker\CacheStorage", r"Service Worker\ScriptCache", "File System", "Favicons", "Top Sites", "Visited Links", "Shortcuts", "databases",
            "Storage", "Wallet", "Accounts", "shared_proto_db", "Local State", "Safe Browsing", "Crashpad", "GrShaderCache", "ShaderCache", "DawnCache", "component_crx_cache",
            "Collections", "EdgeCoupons", "Cache2", "Caches", "cache.bak", "MyCache",
        ];
        let never = Never::default_list();
        for root in [r"C:\Users\sam\AppData\Local\Google\Chrome\User Data", r"D:\Users\x\AppData\Local\Microsoft\Edge\User Data", r"C:\Users\sam\AppData\Local\BraveSoftware\Brave-Browser\User Data"] {
            for prof in ["Default", "default", "Profile 1", "Profile 23", "Guest Profile", "System Profile", "Profile Backup", "Profile 1a", "Profile ", "Profile 12345", "Default.", "..", "x", "Default Cache"] {
                for k in KEPT {
                    assert!(cache_carve_out(&format!(r"{root}\{prof}\{k}")).is_none(), r"{root}\{prof}\{k}");
                }
                for leaf in CHROMIUM_CACHES {
                    let p = format!(r"{root}\{prof}\{leaf}");
                    assert_eq!(cache_carve_out(&p).is_some(), chromium_profile_name(prof), "{p}");
                    // never what is inside one
                    for inside in ["Cache_Data", "js", "index", r"Cache_Data\f_000001", "Login Data"] {
                        assert!(cache_carve_out(&format!(r"{p}\{inside}")).is_none(), r"{p}\{inside}");
                    }
                }
            }
            for p in [root.to_string(), format!(r"{root}\Cache"), format!(r"{root}\GPUCache"), format!(r"{root}\ShaderCache"), format!(r"{root}\Default"), root.trim_end_matches(r"\User Data").to_string()] {
                assert!(cache_carve_out(&p).is_none(), "{p}");
            }
            // on the never-list only by the browser's (and AppData's) whole folders: nothing
            // named there is waived
            let p = format!(r"{root}\Default\Cache");
            let hits = never.all_hits(&p);
            assert!(!hits.is_empty() && hits.iter().all(|h| is_folder_cover(h)), "{p}: {hits:?}");
            assert!(never.all_hits(&format!(r"{root}\Default\Login Data")).iter().any(|h| !is_folder_cover(h)));
            assert!(never.all_hits(&format!(r"{root}\Default\Network\Cookies")).iter().any(|h| !is_folder_cover(h)));
        }
        // other browsers, other channels, other folders: none
        for p in [
            r"C:\Users\sam\AppData\Local\Vivaldi\User Data\Default\Cache",
            r"C:\Users\sam\AppData\Local\Chromium\User Data\Default\Cache",
            r"C:\Users\sam\AppData\Roaming\Opera Software\Opera Stable\Cache",
            r"C:\Users\sam\AppData\Local\Google\Chrome Beta\User Data\Default\Cache",
            r"C:\Users\sam\AppData\Local\Google\Chrome SxS\User Data\Default\Cache",
            r"C:\Users\sam\AppData\Local\Google\Chrome\User Data2\Default\Cache",
            r"C:\Users\sam\AppData\Local\Google\Chrome\Default\Cache",
            r"C:\Users\sam\AppData\Roaming\Google\Chrome\User Data\Default\Cache",
            r"C:\Users\sam\AppData\Local\Google\Chrome\User Data\Default\Default\Cache",
            r"C:\Users\sam\AppData\Roaming\Mozilla\Firefox\Profiles\x.default\cache2",
            r"C:\Users\sam\AppData\Local\Microsoft\Windows\Explorer\thumbcache_32.db",
            r"C:\Google\Chrome\User Data\Default\Cache",
            r"\\server\share\Users\sam\AppData\Local\Google\Chrome\User Data\Default\Cache",
        ] {
            assert!(cache_carve_out(p).is_none(), "{p}");
        }
        assert!(chromium_profile_name("Profile 9999") && !chromium_profile_name("Profile 0x1") && !chromium_profile_name("Profile -1"));
    }

    /// A carve-out lets only its own cleaner's built-in part clear exactly that folder; every
    /// other check still applies inside it (a protected name found there stays), and the run
    /// leaves the profile's other files alone.
    #[test]
    fn a_cache_carve_out_is_for_its_own_built_in_part_only() {
        use crate::finding::{Action, Source};
        let base = std::env::temp_dir().join(format!("sprawl-carveout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let ud = base.join("Browser").join("User Data");
        let write = |p: std::path::PathBuf, n: usize| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, vec![1u8; n]).unwrap();
        };
        write(ud.join("Default").join("Cache").join("Cache_Data").join("f_000001"), 3000);
        write(ud.join("Default").join("Cache").join("Login Data"), 10); // a protected name, somehow inside
        write(ud.join("Default").join("Login Data"), 100);
        write(ud.join("Default").join("Bookmarks"), 100);
        write(ud.join("Default").join("Network").join("Cookies"), 100);
        write(ud.join("Profile 2").join("Code Cache").join("js").join("a"), 500);
        write(ud.join("Guest Profile").join("Cache").join("x"), 500);
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let udp: &'static str = Box::leak(ud.display().to_string().into_boxed_str());
        let list = [CacheCarveOut { cleaner: "app.test", user_data: Box::leak(vec![udp].into_boxed_slice()), leaves: CHROMIUM_CACHES }];
        // the browser's folder on the never-list as a whole, as the real ones are
        let mut never = Never::default_list();
        let cover = format!(r"{udp}\**");
        never.patterns.push((cover.clone(), Pattern::new(&cover)));
        never.incomplete = false;
        let home = Some(base.as_path());
        let finding = |path: std::path::PathBuf, id: &str, action: Option<Action>| {
            let n = crate::find_path(&tree, &path).unwrap();
            let mut f = Finding {
                path,
                owner: None,
                bytes: tree.nodes[n].size,
                risk: Risk::Amber,
                remedy: Remedy::Delete,
                command: None,
                title: "t".into(),
                evidence: vec![],
                confidence: 1.0,
                source: Source::Rule(id.into()),
                allow_protected: false,
                trusted: true,
                builtin: true,
                on_path: true,
                action: None,
                admin: false,
                carved: Vec::new(),
                gain: None,
            };
            if let Some(a) = action {
                f.set_action(a, false);
            }
            f
        };
        let clear = Some(Action::Clear { days: 0 });
        let cache = ud.join("Default").join("Cache");
        let good = finding(cache.clone(), "app.test/cache", clear.clone());
        let ok = check_in(&good, &tree, &never, &list, home).expect("its own part, clearing the cache");
        assert_eq!(ok.carved.len(), 1, "the protected name inside is carved out");
        assert_eq!(ok.bytes, 3000);
        assert!(check_in(&finding(ud.join("Profile 2").join("Code Cache"), "app.test/cache", clear.clone()), &tree, &never, &list, home).is_ok());
        // everything else stays held back by the never-list
        let never_hit = |f: &Finding| matches!(check_in(f, &tree, &never, &list, home), Err(Rejection::Never(_)));
        for (id, action) in [("app.other/cache", clear.clone()), ("app.testx/cache", clear.clone()), ("app.test", clear.clone()), ("app.test/cache", Some(Action::Remove)), ("app.test/cache", Some(Action::Match { pattern: "*".into() })), ("app.test/cache", None)] {
            assert!(never_hit(&finding(cache.clone(), id, action.clone())), "{id} {action:?}");
        }
        assert!(never_hit(&Finding { builtin: false, ..good.clone() }), "a pack on disk");
        assert!(check_in(&Finding { trusted: false, ..good.clone() }, &tree, &never, &list, home).is_err());
        assert!(never_hit(&Finding { source: Source::Model("m".into()), ..good.clone() }), "a model");
        assert!(never_hit(&Finding { remedy: Remedy::Command, ..good.clone() }));
        assert!(matches!(check_in(&good, &tree, &never, &list, Some(std::path::Path::new(r"C:\Users\someone-else"))), Err(Rejection::Never(_))), "another account's");
        assert!(matches!(check_in(&good, &tree, &never, &[], home), Err(Rejection::Never(_))), "no carve-out: held back as before");
        for p in [ud.join("Default"), ud.join("Guest Profile").join("Cache"), ud.join("Default").join("Login Data"), ud.join("Default").join("Network")] {
            assert!(never_hit(&finding(p.clone(), "app.test/cache", clear.clone())), "{}", p.display());
            assert!(never_hit(&finding(p.clone(), "app.test/cache", Some(Action::Remove))), "{}", p.display());
        }
        // a carve-out that is a junction is still held back as one
        std::fs::create_dir_all(ud.join("Profile 3")).unwrap();
        std::fs::create_dir_all(base.join("elsewhere")).unwrap();
        if std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(ud.join("Profile 3").join("GPUCache")).arg(base.join("elsewhere")).output().is_ok_and(|o| o.status.success()) {
            let t2 = sprawl_core::scan(&base, |_| {}).unwrap();
            let p = ud.join("Profile 3").join("GPUCache");
            let mut f = good.clone();
            f.path = p.clone();
            f.bytes = 0;
            f.set_action(Action::Clear { days: 0 }, false);
            assert!(matches!(check_in(&f, &t2, &never, &list, home), Err(Rejection::ThroughLink(_))));
        }
        // the run: the cache goes, the protected name inside and the profile's files stay
        let o = crate::clean::run(&cache, &Action::Clear { days: 0 }, &crate::clean::Spare::default(), &std::sync::atomic::AtomicBool::new(false), &mut |_| {});
        assert_eq!((o.freed, o.kept_protected), (3000, 1), "{:?}", o.problems);
        assert!(cache.exists() && cache.join("Login Data").exists() && !cache.join("Cache_Data").exists());
        for k in [r"Default\Login Data", r"Default\Bookmarks", r"Default\Network\Cookies", r"Profile 2\Code Cache\js\a", r"Guest Profile\Cache\x"] {
            assert!(ud.join(k).exists(), "{k}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
