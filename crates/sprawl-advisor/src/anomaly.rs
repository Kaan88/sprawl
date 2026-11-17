//! Anomaly sense: well-known folders that are far bigger than they usually are ("the Chrome
//! profile is 22 GB; it is usually 1 to 3 GB"). Information only: what usually makes it big and
//! where to look. Nothing here deletes anything.

use sprawl_core::{format_size, Tree};

use crate::finding::{Finding, Remedy, Risk, Source};
use crate::rules::{expand, Pattern};

struct Typical {
    id: &'static str,
    what: &'static str,
    /// Patterns like the rule packs' (`%VAR%`, `*`, `**`, `?:` for any drive).
    paths: &'static [&'static str],
    /// Usual range, bytes.
    low: u64,
    high: u64,
    /// What usually makes it big, and what to do.
    hint: &'static str,
}

const GB: u64 = 1 << 30;
const MB: u64 = 1 << 20;

const TYPICAL: &[Typical] = &[
    Typical { id: "chrome-profile", what: "Chrome profile", paths: &[r"%LOCALAPPDATA%\Google\Chrome\User Data\Default", r"%LOCALAPPDATA%\Google\Chrome\User Data\Profile *"], low: GB, high: 3 * GB,
        hint: "Usually Service Worker caches of web apps, IndexedDB of sites, and extensions. chrome://settings/content/all lists what each site stores." },
    Typical { id: "edge-profile", what: "Edge profile", paths: &[r"%LOCALAPPDATA%\Microsoft\Edge\User Data\Default", r"%LOCALAPPDATA%\Microsoft\Edge\User Data\Profile *"], low: GB, high: 3 * GB,
        hint: "Usually Service Worker caches and site data. edge://settings/content/all lists what each site stores." },
    Typical { id: "firefox-profile", what: "Firefox profile", paths: &[r"%APPDATA%\Mozilla\Firefox\Profiles\*"], low: 200 * MB, high: 2 * GB,
        hint: "Usually site storage (storage\\default) and session history. about:preferences#privacy > Manage Data shows it by site." },
    Typical { id: "outlook-data", what: "Outlook mail cache", paths: &[r"%LOCALAPPDATA%\Microsoft\Outlook"], low: GB, high: 10 * GB,
        hint: "The offline copy of your mailbox (.ost). Outlook > Account Settings > Change > the 'Download email for the past' slider keeps less offline." },
    Typical { id: "teams-cache", what: "Teams cache", paths: &[r"%LOCALAPPDATA%\Packages\MSTeams_*\LocalCache", r"%APPDATA%\Microsoft\Teams"], low: 300 * MB, high: 2 * GB,
        hint: "Cached media and chat data. Settings > Privacy (or quitting Teams and clearing its cache folder) resets it; it fills again with use." },
    Typical { id: "discord", what: "Discord data", paths: &[r"%APPDATA%\discord"], low: 100 * MB, high: GB,
        hint: "Mostly Cache and Code Cache: quit Discord and they can be cleared; they fill again." },
    Typical { id: "slack", what: "Slack data", paths: &[r"%APPDATA%\Slack"], low: 200 * MB, high: 2 * GB,
        hint: "Mostly caches. Help > Troubleshooting > Clear Cache and Restart." },
    Typical { id: "spotify", what: "Spotify data", paths: &[r"%LOCALAPPDATA%\Spotify"], low: 300 * MB, high: 3 * GB,
        hint: "Mostly the streaming cache (Storage). Settings > Storage sets a limit or clears it; downloads for offline use are separate." },
    Typical { id: "vscode-workspaces", what: "VS Code workspace storage", paths: &[r"%APPDATA%\Code\User\workspaceStorage"], low: 100 * MB, high: 2 * GB,
        hint: "Per-folder state of extensions (language servers, indexes) for every folder ever opened. Subfolders of projects you no longer open can go." },
    Typical { id: "whatsapp", what: "WhatsApp data", paths: &[r"%LOCALAPPDATA%\Packages\*WhatsApp*\LocalState"], low: 500 * MB, high: 5 * GB,
        hint: "Mostly received media. Settings > Storage shows the largest chats." },
    Typical { id: "windows-installer", what: "Windows Installer cache", paths: &[r"%SystemRoot%\Installer"], low: 2 * GB, high: 15 * GB,
        hint: "Copies of installers Windows needs to repair and uninstall programs. Never delete files from it by hand: uninstalling old programs and updates is the safe way to shrink it." },
    Typical { id: "windows-logs", what: "Windows logs", paths: &[r"%SystemRoot%\Logs"], low: 100 * MB, high: 2 * GB,
        hint: "A runaway CBS or DISM log is the usual cause (Logs\\CBS). Old .log and .cab files there can go as administrator; the newest CBS.log is in use." },
    Typical { id: "steam-html", what: "Steam web cache", paths: &[r"%LOCALAPPDATA%\Steam\htmlcache"], low: 100 * MB, high: GB,
        hint: "The store and community pages' cache. Steam > Settings > In-Game > Delete Web Browser Data." },
    Typical { id: "hiberfil", what: "Hibernation file", paths: &[r"?:\hiberfil.sys"], low: 2 * GB, high: 8 * GB,
        hint: "As big as a share of your memory. 'powercfg /h /type reduced' (as administrator) keeps Fast Startup with a smaller file; 'powercfg /h off' removes it and hibernation." },
];

/// Folders far bigger than usual: more than twice their usual top. Information findings.
pub fn find(tree: &Tree) -> Vec<Finding> {
    let mut out = Vec::new();
    for t in TYPICAL {
        let patterns: Vec<Pattern> = t.paths.iter().map(|p| Pattern::new(&expand(p))).collect();
        for n in crate::rules::matching_nodes(tree, &patterns) {
            let node = &tree.nodes[n];
            if node.size <= t.high.saturating_mul(2) {
                continue;
            }
            out.push(Finding {
                path: tree.path_of(n),
                owner: None,
                bytes: node.size,
                risk: Risk::Amber,
                remedy: Remedy::Info,
                command: None,
                title: format!("{} is {}", t.what, format_size(node.size)),
                evidence: vec![format!("Usually {} to {}", format_size(t.low), format_size(t.high)), t.hint.to_string()],
                confidence: 0.8,
                source: Source::Rule(format!("anomaly.{}", t.id)),
                allow_protected: false,
                trusted: true,
                builtin: true,
                on_path: false,
                action: None,
                admin: false,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn patterns_find_profile_folders() {
        let dir = std::env::temp_dir().join(format!("sprawl-anomaly-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for p in ["User Data\\Default", "User Data\\Profile 2", "User Data\\Other"] {
            std::fs::create_dir_all(dir.join(p)).unwrap();
        }
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let root = tree.path_of(sprawl_core::ROOT).display().to_string();
        let pats: Vec<crate::rules::Pattern> = [format!(r"{root}\User Data\Default"), format!(r"{root}\User Data\Profile *")].iter().map(|p| crate::rules::Pattern::new(p)).collect();
        let mut names: Vec<String> = crate::rules::matching_nodes(&tree, &pats).into_iter().map(|n| tree.nodes[n].name.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["Default", "Profile 2"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn table_is_sane() {
        for t in super::TYPICAL {
            assert!(t.low < t.high && !t.paths.is_empty() && !t.hint.is_empty(), "{}", t.id);
        }
    }
}
