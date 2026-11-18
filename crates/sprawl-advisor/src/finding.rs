//! A finding: one place where space could be won back, with what it costs and how to do it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What acting on a finding costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    /// Regenerable: caches, temp files, build output. Comes back by itself when needed.
    Green,
    /// Costs something: a re-download, a rebuild, a slower first start, lost history.
    Amber,
    /// The user's own data: only they can say whether it can go.
    Orange,
    /// Never: system files, things that break Windows or apps. Shown for information only.
    Red,
}

/// How the space is won back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Remedy {
    /// Delete the files (the command removes them).
    Delete,
    /// Run the owner's own clean-up command (`npm cache clean`, `DISM`, `Clear-RecycleBin` ...).
    Command,
    /// Uninstall the app that owns it.
    Uninstall,
    /// Move it to another drive.
    Move,
    /// Nothing to run: explanation only.
    Info,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind", content = "id")]
pub enum Source {
    /// A rule from a rule pack, by id.
    Rule(String),
    /// A language model, by model name.
    Model(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub path: PathBuf,
    /// The app or tool the space belongs to, when known.
    pub owner: Option<String>,
    /// Bytes won back (the tree's size of `path`).
    pub bytes: u64,
    pub risk: Risk,
    pub remedy: Remedy,
    /// A PowerShell command the user can copy and run.
    pub command: Option<String>,
    /// Short title for lists: "npm cache".
    pub title: String,
    /// Why this is safe (or what it costs), one point per line.
    pub evidence: Vec<String>,
    /// 0..1
    pub confidence: f32,
    pub source: Source,
    /// Set only for rules from the built-in pack that may run their owner's command on a
    /// never-list path (DISM on WinSxS). Never set for local packs or a model.
    #[serde(skip)]
    pub allow_protected: bool,
    /// The command comes from the built-in pack. Commands from anywhere else are refused.
    #[serde(skip)]
    pub trusted: bool,
    /// From Sprawl's own built-in rules. Only these are trusted on risk; a local pack's or a
    /// model's deletes are raised to at least Amber or Orange.
    #[serde(skip)]
    pub builtin: bool,
    /// The command acts on `path` itself (the rule's command has `{path}`, or it is the
    /// program's own delete), so link and scan-root checks apply even to exempt rules.
    #[serde(skip)]
    pub on_path: bool,
    /// What Sprawl does to `path` when this is cleaned up. Set only by the code that writes
    /// the finding (a rule, the program's own delete); `command` is the same thing written out
    /// for "Save as script". Never read back out of command text.
    #[serde(skip)]
    pub action: Option<Action>,
    /// The action needs administrator rights.
    #[serde(skip)]
    pub admin: bool,
    /// What Sprawl's clean-up leaves inside `path` because it is protected (a mail store, a disk
    /// image): already taken off `bytes`. Set by the safety check, never by the finding's source.
    #[serde(skip)]
    pub carved: Vec<Carve>,
    /// What acting on it would win back, when that isn't `bytes` (a tip: what compressing
    /// saves, what OneDrive frees). `bytes` stays the folder's size, which the safety checks
    /// compare with the tree.
    #[serde(skip)]
    pub gain: Option<u64>,
    /// The remedy it had before the user ticked it on a second click (R7c): it is then a
    /// delete of the whole path that always goes to the Recycle Bin. Set only by the app, at
    /// the user's click, after the deletion checks passed; never by a rule or a model.
    #[serde(skip)]
    pub forced_from: Option<Remedy>,
}

/// Where a finding is listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Sprawl can remove it (deletes, a tool's clean-up).
    Clean,
    /// Left behind by an application that is no longer installed (Sprawl can remove it, the
    /// user decides each).
    Leftover,
    /// The user can do it: compress, move, uninstall, clear from the app, pick between copies.
    Yourself,
    /// Information only: a folder bigger than usual, something never to touch.
    Know,
}

/// One protected item inside a finding that its clean-up leaves where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Carve {
    pub path: PathBuf,
    pub bytes: u64,
    /// The never-list entry it falls under.
    pub rule: String,
}

/// `p` is `base` or inside it (case and a trailing separator aside).
pub fn within(p: &std::path::Path, base: &std::path::Path) -> bool {
    let n = |p: &std::path::Path| PathBuf::from(p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase());
    let b = n(base);
    !b.as_os_str().is_empty() && n(p).starts_with(b)
}

/// `p` is in the Users folder of its drive (`X:\Users\...`, or the folder the user's own profile
/// sits in) and not inside `home`. The Users folder itself counts: it holds other profiles. A
/// network path (`\\server\...`) counts too: whose folder it is can't be told from here.
pub fn other_account(p: &std::path::Path, home: Option<&std::path::Path>) -> bool {
    let s = p.to_string_lossy().replace('/', "\\");
    if s.starts_with("\\\\") && !s.starts_with("\\\\?\\") || s.to_lowercase().starts_with("\\\\?\\unc\\") {
        return true;
    }
    let s = s.strip_prefix("\\\\?\\").unwrap_or(&s).to_string();
    let mut parts = s.trim_end_matches('\\').split('\\').filter(|x| !x.is_empty());
    let (Some(drive), Some(top)) = (parts.next(), parts.next()) else { return false };
    if !drive.ends_with(':') {
        return false;
    }
    // the name of the folder profiles live in: "Users", and whatever this user's sits in
    let users_dir = home.and_then(|h| h.parent()).and_then(|u| u.file_name()).map(|n| n.to_string_lossy().to_lowercase());
    let top = top.to_lowercase();
    let in_users = top == "users" || users_dir.is_some_and(|u| u == top);
    in_users && !home.is_some_and(|h| within(p, h))
}

/// What a clean-up does to a finding's path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The path itself (a folder with everything in it, or a file).
    Remove,
    /// What is inside the folder, leaving anything created or changed in the last `days`.
    Clear { days: u32 },
    /// Files matching a PowerShell-style pattern anywhere below the folder.
    Match { pattern: String },
    /// A built-in rule's own tool command, run by PowerShell without a window.
    Tool { command: String },
    /// A privacy part's files (`part` is its rule id, as `privacy.chrome/cookies`), each with its
    /// database side files (`-journal`, `-wal`, `-shm`), deleted all together or not at all, only
    /// while the browser is closed. Made only by [`crate::privacy`]; never written to a script.
    Set { part: String, files: Vec<PathBuf> },
    /// F4: each of `copies` replaced by a hard link to `keep`, after comparing it with `keep`
    /// byte for byte again; every path stays, the space of one copy is used. Made only by
    /// [`crate::dupes::find`] for copies on one volume, none in a protected place; never written
    /// to a script. The finding's path is `copies[0]`.
    Link { keep: PathBuf, copies: Vec<PathBuf> },
}

impl Finding {
    /// Found by its shape (a `Cache` folder under an unknown app, a folder an application that
    /// is gone left behind), not by a rule that names the program: a careful guess, never
    /// picked for the user nor written into a script.
    pub fn heuristic(&self) -> bool {
        matches!(&self.source, Source::Rule(id) if id == crate::cleaners::SHAPE || id == crate::leftovers::ID || id == crate::exe_installers::ID || id == crate::game_leftovers::ID)
    }

    /// Left behind by an application that is no longer installed.
    pub fn leftover(&self) -> bool {
        matches!(&self.source, Source::Rule(id) if id == crate::leftovers::ID || id == crate::game_leftovers::ID)
    }

    /// Its size is the whole folder's, while the clean-up takes only part of it (files a
    /// pattern names, what is older than some days, or what a tool decides): shown as "up to".
    pub fn estimate(&self) -> bool {
        matches!(&self.action, Some(Action::Match { .. }) | Some(Action::Clear { days: 1.. }) | Some(Action::Tool { .. }))
    }

    /// What acting on it wins back: its gain when it has one, else its size.
    pub fn expected_gain(&self) -> u64 {
        self.gain.unwrap_or(self.bytes)
    }

    /// Where it is listed: what Sprawl removes, what the user can do, what is worth knowing.
    pub fn group(&self) -> Group {
        // ticked on a second click: listed where it was
        match self.forced_from.unwrap_or(self.remedy) {
            // left behind: deletable, or shown only when it holds the user's work
            _ if self.leftover() => Group::Leftover,
            Remedy::Delete | Remedy::Command => Group::Clean,
            _ if self.risk == Risk::Red || matches!(&self.source, Source::Rule(id) if id.starts_with("anomaly.") || id.starts_with("growth.")) => Group::Know,
            _ => Group::Yourself,
        }
    }

    /// Inside another account's profile (Public counts as one): under the Users folder of any
    /// drive, but not inside `%USERPROFILE%`. Never green, never planned or ticked for the user.
    /// Everything else (this user's folders, Windows' own, a project on D:) keeps its rule's risk.
    pub fn other_account(&self) -> bool {
        other_account(&self.path, std::env::var_os("USERPROFILE").as_deref().map(std::path::Path::new))
    }

    /// The build output of a project the user is working on: shown on its own, never ticked,
    /// planned or counted in the headline for them.
    pub fn in_active_project(&self) -> bool {
        matches!(&self.source, Source::Rule(id) if id == crate::projects::ACTIVE)
    }

    /// An active project's output holding a program that ran lately: last in its group.
    pub fn holds_tool_in_use(&self) -> bool {
        self.in_active_project() && self.evidence.iter().any(|e| e.ends_with(crate::projects::TOOL_IN_USE))
    }

    /// "1 protected file kept", when the clean-up leaves something inside.
    pub fn carved_note(&self) -> Option<String> {
        match self.carved.len() {
            0 => None,
            1 => Some("1 protected item kept".into()),
            n => Some(format!("{n} protected items kept")),
        }
    }

    /// A privacy part (cookies, history): never ticked for the user, planned, written to a
    /// script or counted in what can be freed. Known by its action or its rule's id.
    pub fn privacy(&self) -> bool {
        matches!(self.action, Some(Action::Set { .. })) || matches!(&self.source, Source::Rule(id) if crate::privacy::is_privacy_id(id))
    }

    /// F4: copies made hard links to one of them: nothing is deleted or recycled.
    pub fn links(&self) -> bool {
        matches!(self.action, Some(Action::Link { .. }))
    }

    /// Give the finding its action, and the command text "Save as script" writes for it (none
    /// for a privacy part's set: those run only inside Sprawl).
    pub fn set_action(&mut self, action: Action, admin: bool) {
        self.command = (!matches!(action, Action::Set { .. } | Action::Link { .. })).then(|| crate::clean::script(&self.path, &action, admin));
        self.admin = admin;
        self.action = Some(action);
    }
}
