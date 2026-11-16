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
}
