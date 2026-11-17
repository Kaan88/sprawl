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
}

/// `p` is `base` or inside it (case and a trailing separator aside).
pub fn within(p: &std::path::Path, base: &std::path::Path) -> bool {
    let n = |p: &std::path::Path| PathBuf::from(p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase());
    let b = n(base);
    !b.as_os_str().is_empty() && n(p).starts_with(b)
}

/// `p` is in the Users folder of its drive (`X:\Users\...`, or the folder the user's own profile
/// sits in) and not inside `home`. The Users folder itself counts: it holds other profiles.
pub fn other_account(p: &std::path::Path, home: Option<&std::path::Path>) -> bool {
    let s = p.to_string_lossy().replace('/', "\\");
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
}

impl Finding {
    /// Found by its shape (a `Cache` folder under an unknown app), not by a rule that names the
    /// program: a careful guess, never picked for the user nor written into a script.
    pub fn heuristic(&self) -> bool {
        matches!(&self.source, Source::Rule(id) if id == crate::cleaners::SHAPE)
    }

    /// Its size is the whole folder's, while the clean-up takes only part of it (files a
    /// pattern names, or what is older than some days): shown as "up to".
    pub fn estimate(&self) -> bool {
        matches!(&self.action, Some(Action::Match { .. }) | Some(Action::Clear { days: 1.. }))
    }

    /// Inside another account's profile (Public counts as one): under the Users folder of any
    /// drive, but not inside `%USERPROFILE%`. Never green, never planned or ticked for the user.
    /// Everything else (this user's folders, Windows' own, a project on D:) keeps its rule's risk.
    pub fn other_account(&self) -> bool {
        other_account(&self.path, std::env::var_os("USERPROFILE").as_deref().map(std::path::Path::new))
    }

    /// Give the finding its action, and the command text "Save as script" writes for it.
    pub fn set_action(&mut self, action: Action, admin: bool) {
        self.command = Some(crate::clean::script(&self.path, &action, admin));
        self.admin = admin;
        self.action = Some(action);
    }
}
