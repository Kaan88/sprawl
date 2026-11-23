//! Every setting, described once: its key, section, words, default and kind. The setup wizard
//! and Settings both build their rows from this list, so they always say the same thing and
//! read and write the same keys.

use sprawl_advisor::tuning::{self, Knob};

/// A section of Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sec {
    General,
    Theme,
    Ai,
    Network,
    Scanning,
    Search,
    Map,
    Cleaning,
    Advisor,
    History,
}

impl Sec {
    pub const ALL: [Sec; 10] = [Sec::General, Sec::Theme, Sec::Ai, Sec::Network, Sec::Scanning, Sec::Search, Sec::Map, Sec::Cleaning, Sec::Advisor, Sec::History];

    pub fn title(self) -> &'static str {
        match self {
            Sec::General => "General",
            Sec::Theme => "Theme",
            Sec::Ai => "AI and privacy",
            Sec::Network => "Network",
            Sec::Scanning => "Scanning",
            Sec::Search => "Search",
            Sec::Map => "Map and look",
            Sec::Cleaning => "Cleaning",
            Sec::Advisor => "Advisor",
            Sec::History => "History",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Sec::General => "&#xE713;",
            Sec::Theme => "&#xE771;",
            Sec::Ai => "&#xE945;",
            Sec::Network => "&#xE774;",
            Sec::Scanning => "&#xEDA2;",
            Sec::Search => "&#xE71E;",
            Sec::Map => "&#xE790;",
            Sec::Cleaning => "&#xE74D;",
            Sec::Advisor => "&#xE82F;",
            Sec::History => "&#xE81C;",
        }
    }

    /// The section a link names (`"ai"`, `"history"`...).
    pub fn by_id(id: &str) -> Option<Sec> {
        Sec::ALL.into_iter().find(|s| format!("{s:?}").eq_ignore_ascii_case(id))
    }
}

/// How a setting is shown and kept.
pub enum Kind {
    /// On or off, with its default.
    Switch(bool),
    /// A number held between a tuning knob's floor and ceiling, in this unit.
    Number(&'static Knob, &'static str),
    /// One of these, with the default's index.
    Choice(&'static [&'static str], usize),
    /// Free text (a path), with a placeholder.
    Text(&'static str),
    /// Built by its own code: a list, buttons, the AI provider.
    Custom,
}

pub struct Pref {
    pub key: &'static str,
    pub sec: Sec,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: Kind,
    /// Under "Advanced" in its section.
    pub advanced: bool,
}

const fn p(key: &'static str, sec: Sec, label: &'static str, help: &'static str, kind: Kind) -> Pref {
    Pref { key, sec, label, help, kind, advanced: false }
}

const fn adv(key: &'static str, sec: Sec, label: &'static str, help: &'static str, kind: Kind) -> Pref {
    Pref { key, sec, label, help, kind, advanced: true }
}

pub const UNITS: [&str; 6] = ["Best fit", "Bytes", "KB", "MB", "GB", "TB"];
pub const UNIT_ITEMS: [&str; 6] = ["uDyn", "uB", "uKB", "uMB", "uGB", "uTB"];
pub const AA: [&str; 4] = ["Off", "Low (2x)", "Medium (4x)", "High (8x)"];
pub const AA_ITEMS: [&str; 4] = ["aaOff", "aaLow", "aaMedium", "aaHigh"];
pub const CAPS: [&str; 4] = ["500 MB", "1 GB", "2 GB", "5 GB"];
pub const CAPS_MB: [u32; 4] = [500, 1024, 2048, 5120];
pub const AFTER_SCAN: [&str; 2] = ["Stay where I am", "Open the Advisor"];
/// "Open Sprawl on": the tab used last, or one of the tabs (look::start_tab reads the index).
/// "Warn me below": the free space a drive may fall to before the low-space notice.
pub const ALERT_AT: [&str; 4] = ["5 GB", "10 GB", "20 GB", "50 GB"];
pub const ALERT_GB: [u64; 4] = [5, 10, 20, 50];
/// "Grew by more than": how much a folder may grow in about a week before the growth notice.
pub const GROW_AT: [&str; 4] = ["2 GB", "5 GB", "10 GB", "20 GB"];
pub const GROW_GB: [u64; 4] = [2, 5, 10, 20];
pub const OPEN_ON: [&str; 6] = ["The tab used last", "Tree View", "File View", "Advisor", "Cleaners", "Search"];

/// The Options menu switches a setting stands for: read from and applied through the menu.
pub const MENU: [&str; 11] = ["oUpdates", "oAlwaysAdmin", "oExplorer", "oAuto", "oPulses", "oTilt", "oNames", "oCushion", "oMirror", "oFree", "oAlloc"];

pub static PREFS: [Pref; 61] = [
    // General
    p("scanExclude", Sec::Scanning, "Folders to skip", "Full paths separated by semicolons, for example D:\\Games; C:\\Work\\Private. New app, Search, Cleaners and MCP scans omit them. Fast MFT scans still work: skipped folders are left out as the tree is built. Cleanup actions covering a skipped folder are refused. Existing History and CSV imports remain unchanged; rescan to replace old results.", Kind::Text("D:\\Games; C:\\Work\\Private")),
    p("theme", Sec::Theme, "Light or dark", "Light or dark. Like Windows follows Settings > Personalisation > Colours. A change shows as soon as you save.", Kind::Choice(&crate::theme::CHOICES, 0)),
    p("units", Sec::General, "Sizes in", "Best fit or a fixed unit. The base below determines GB versus GiB. Search sizes, goals and alert thresholds keep their stated binary meaning.", Kind::Choice(&UNITS, 0)),
    p("decimal", Sec::General, "Use decimal sizes (GB)", "Off: binary sizes, 1 GiB = 1,024 MiB. On: decimal sizes, 1 GB = 1,000 MB. Changes size displays, not byte counts or search and cleanup thresholds.", Kind::Switch(false)),
    p(
        "startTab",
        Sec::General,
        "Open Sprawl on",
        "The tab shown when Sprawl starts; nothing is scanned because of it. A folder opened from Explorer or the command line is scanned and shown in Tree View whatever this says.",
        Kind::Choice(&OPEN_ON, 0),
    ),
    p(
        "alertLow",
        Sec::General,
        "Tell me when a drive is nearly full",
        "Every 4 hours a scheduled task of your account looks at how much space each drive has free, nothing more, and shows a Windows notice when one is below the limit: at most once a week per drive. A click opens Sprawl on it. Nothing is scanned or cleaned by it.",
        Kind::Switch(false),
    ),
    p("alertFreeGb", Sec::General, "Nearly full means less free than", "", Kind::Choice(&ALERT_AT, 1)),
    p(
        "alertGrowth",
        Sec::General,
        "Tell me when a folder grows fast",
        "The same check compares your scans in History (keep scan history must be on) and shows a notice when a folder grew by more than the amount below in about a week: once per scan, at most once a week per folder.",
        Kind::Switch(false),
    ),
    p("alertGrowGb", Sec::General, "Fast means it grew by more than", "", Kind::Choice(&GROW_AT, 1)),
    p(
        "alertDailyScan",
        Sec::General,
        "Also read my user folder once a day for it",
        "Off: only the scans you make count. On: once a day the check also reads your user folder, folder sizes only, with your own rights and at low priority, and keeps it in History.",
        Kind::Switch(false),
    ),
    p("dataFolder", Sec::General, "Your data", "Settings, what the Advisor remembers, scan history and logs: all on this PC, in your own folders.", Kind::Custom),
    // AI and privacy
    p("ai", Sec::Ai, "AI provider", "", Kind::Custom),
    p(
        "aiWebLookup",
        Sec::Ai,
        "Let the AI search the web (asks each time)",
        "Off: the AI is never offered a web search. On: it may ask to; you see the exact query, with paths and names taken out, and allow it once or for the session.",
        Kind::Switch(false),
    ),
    p("aiLog", Sec::Ai, "AI request log", "Every request to an AI provider and every web lookup, kept on this PC. Forget removes the names the AI gave folders and the rules and cleaners it proposed that you haven't approved.", Kind::Custom),
    // Network
    p("oUpdates", Sec::Network, "Check for updates automatically", "When it starts, and once a day while it stays open, Sprawl asks GitHub for a newer signed release. Nothing about this PC is sent.", Kind::Switch(true)),
    p(
        "communityFetch",
        Sec::Network,
        "Fetch community cleaners",
        "Once a week, winapp2.ini and BleachBit's cleaners are downloaded from GitHub (a plain download; nothing about this PC is sent). Each does nothing until you turn it on in Cleaners.",
        Kind::Switch(false),
    ),
    p("offline", Sec::Network, "Work offline: no update check, no community download, no AI", "While on, nothing leaves this PC, whatever else is set.", Kind::Switch(false)),
    // Scanning
    p(
        "oAlwaysAdmin",
        Sec::Scanning,
        "Always run as administrator",
        "Reads the NTFS master file table directly: a whole drive in seconds instead of a minute or more. Windows asks each time Sprawl starts. As administrator, the four things in the box below are off unless you allow them.",
        Kind::Switch(false),
    ),
    // While Sprawl runs as administrator, also allow (off by default; never on the wizard)
    p("adminShellMenu", Sec::Scanning, "The Windows menu (as administrator)", "", Kind::Switch(false)),
    p("adminHandOff", Sec::Scanning, "Handing an application to your uninstaller (as administrator)", "", Kind::Switch(false)),
    p("adminClaudeCode", Sec::Scanning, "Claude Code and Codex (as administrator)", "", Kind::Switch(false)),
    p("adminUserTools", Sec::Scanning, "Clean-up commands from your own folders (as administrator)", "", Kind::Switch(false)),
    p("adminMcpDisk", Sec::Scanning, "MCP tools reading the disk (as administrator)", "", Kind::Switch(false)),
    p("oExplorer", Sec::Scanning, "Add \"Scan with Sprawl\" to Explorer", "Right-click any folder or drive in Explorer to scan it. Only for your account; no administrator needed.", Kind::Switch(false)),
    p("oAuto", Sec::Scanning, "Scan as soon as a drive is picked", "Picking a drive in the list starts its scan.", Kind::Switch(true)),
    p("oPulses", Sec::Scanning, "Watch for changes after a scan", "Tiles pulse when their files change (Live Change Pulses).", Kind::Switch(true)),
    p("afterScan", Sec::Scanning, "After a scan", "Where Sprawl takes you when a scan finishes.", Kind::Choice(&AFTER_SCAN, 0)),
    // Search
    p("searchDrivesOn", Sec::Search, "Other drives Search may read", "A ticked drive is read when Search opens, so Search finds files there too. None is ticked until you choose.", Kind::Custom),
    p("searchHistory", Sec::Search, "Keep recent searches", "Your recent and saved searches, in %APPDATA%\\Sprawl\\searches.txt on this PC.", Kind::Switch(true)),
    p("searchSnapshots", Sec::Search, "Saved scans Search may use", "Search also looks in the newest saved scans of History that nothing else covers.", Kind::Number(&tuning::SEARCH_SNAPSHOTS, "scans")),
    adv("searchMaxHits", Sec::Search, "Most results shown from other drives", "More takes more memory and time to sort.", Kind::Number(&tuning::SEARCH_MAX_HITS, "rows")),
    // Map and look
    p("palette", Sec::Map, "Colours", "The colours of the map and the file types.", Kind::Choice(&crate::paint::PALETTES, 0)),
    p("animate", Sec::Map, "Animations", "Turn off for a calmer, lighter map. Until you choose, it follows Windows' Animation effects (Accessibility > Visual effects).", Kind::Switch(true)),
    p("oTilt", Sec::Map, "3D and tilt", "The map as lit 3D tiles, tilted.", Kind::Switch(true)),
    p("oNames", Sec::Map, "Folder and file names", "Names on the map's tiles.", Kind::Switch(true)),
    p("oCushion", Sec::Map, "Shading", "Cushion shading on the tiles.", Kind::Switch(true)),
    p("oMirror", Sec::Map, "Mirror floor", "A glossy reflection under the map.", Kind::Switch(true)),
    p("oFree", Sec::Map, "Free space", "The drive's free space as a tile.", Kind::Switch(false)),
    p("oAlloc", Sec::Map, "Size by allocated space", "Tiles sized by the space files take on disk, not their length.", Kind::Switch(false)),
    p("aa", Sec::Map, "Antialiasing", "Smoother 3D edges; higher takes more of the graphics card.", Kind::Choice(&AA, 2)),
    // Cleaning
    p(
        "recycleOwn",
        Sec::Cleaning,
        "Send my own files to the Recycle Bin instead of deleting them",
        "Your own files and leftovers (orange) go to the Recycle Bin, where Explorer can put them back; their space is freed when the bin is emptied. Caches are still deleted: in the bin they would free nothing. What can't go to the bin is left as it is, never deleted instead.",
        Kind::Switch(true),
    ),
    p(
        "privacyCleaners",
        Sec::Cleaning,
        "Show privacy cleaners",
        "Adds a Privacy group to Cleaners: cookies, history and what forms remember in this account's browsers, by profile. They free almost no space, each is ticked on its own, and a browser database is copied before it is edited. Off, Sprawl doesn't look at them.",
        Kind::Switch(false),
    ),
    p("neverTouch", Sec::Cleaning, "Never touch", "Folders and files you chose to keep: Sprawl never offers them and a clean-up leaves them. Add one, or take one off the list.", Kind::Custom),
    p("clUsualTicks", Sec::Cleaning, "Open Cleaners with my usual ticks", "Cleaners opens with the parts you ticked last time. Sprawl never ticks anything for you.", Kind::Switch(true)),
    p("tuneCommunityDays", Sec::Cleaning, "Community cleaners: fetch every", "When fetching is on (Network).", Kind::Number(&tuning::COMMUNITY_DAYS, "days")),
    adv("oldScanMinutes", Sec::Cleaning, "Old scan warning after", "A clean-up says so when its scan is older than this.", Kind::Number(&tuning::OLD_SCAN_MINUTES, "minutes")),
    // Advisor
    p("uninstaller", Sec::Advisor, "Uninstaller", "\"Hand to...\" in the Advisor hands an application to it (Revo, Bulk Crap Uninstaller and the like) instead of Windows' own uninstaller.", Kind::Text("A full path to its .exe (optional)")),
    p("mcp", Sec::Advisor, "Use Sprawl from your own Claude Code", "Run this once in a terminal; then your own Claude Code can ask Sprawl about your disk while Sprawl is open.", Kind::Custom),
    adv("tuneFindingMinMb", Sec::Advisor, "Smallest finding listed", "Can only be raised: lower would list more safe findings.", Kind::Number(&tuning::FINDING_MIN_MB, "MB")),
    adv("tunePartMinKb", Sec::Advisor, "Smallest cleaner part listed", "Can only be raised, for the same reason.", Kind::Number(&tuning::PART_MIN_KB, "KB")),
    adv("tuneProjectIdleDays", Sec::Advisor, "A project counts as idle after", "Its build output is offered once nothing in the project changed for this long.", Kind::Number(&tuning::PROJECT_IDLE_DAYS, "days")),
    adv("tuneUnusedAppDays", Sec::Advisor, "An application counts as unused after", "Not started for this long (and 1 GB or more).", Kind::Number(&tuning::UNUSED_APP_DAYS, "days")),
    adv("tuneLeftoverIdleDays", Sec::Advisor, "A leftover needs no change for", "A folder of an application that is gone.", Kind::Number(&tuning::LEFTOVER_IDLE_DAYS, "days")),
    adv("tuneLeftoverMinMb", Sec::Advisor, "A leftover's smallest size", "", Kind::Number(&tuning::LEFTOVER_MIN_MB, "MB")),
    adv("tuneLaterDays", Sec::Advisor, "\"Not now\" hides a finding for", "", Kind::Number(&tuning::LATER_DAYS, "days")),
    adv("tuneDupesMinMb", Sec::Advisor, "Duplicates: smallest file", "", Kind::Number(&tuning::DUPES_MIN_MB, "MB")),
    adv("tuneDupesBudgetGb", Sec::Advisor, "Duplicates: how much to read", "Files are read to compare them; more finds more and takes longer.", Kind::Number(&tuning::DUPES_BUDGET_GB, "GB")),
    // History
    p(
        "snapshots",
        Sec::History,
        "Keep scan history",
        "After each scan Sprawl saves a snapshot of it: every file name of each scanned folder, with sizes and dates, on this PC only (%LOCALAPPDATA%\\Sprawl\\snapshots). About 5 MB for a whole system drive, the newest in full detail. It powers growth over time, the time-lapse and comparing scans.",
        Kind::Switch(false),
    ),
    p("snapshot_cap_mb", Sec::History, "Space for history", "The oldest snapshots are thinned out first when it is full.", Kind::Choice(&CAPS, 1)),
    p("historyDisk", Sec::History, "History on disk", "Clear deletes every saved snapshot; scans you make later are kept again if history is on.", Kind::Custom),
    p("tuneKeepFull", Sec::History, "Full snapshots kept per folder", "Older ones keep only folder totals.", Kind::Number(&tuning::KEEP_FULL, "snapshots")),
];

/// The wizard: only what matters on the first day. Each page: title, glyph, line, settings.
pub const WIZARD: [(&str, &str, &str, &[&str]); 7] = [
    ("Welcome", "&#xE7BE;", "Let's set it up the way you like it", &[]),
    ("AI", "&#xE945;", "Optional: who should explain your disk?", &["ai", "aiWebLookup"]),
    ("Network", "&#xE774;", "What Sprawl may fetch, and when", &["oUpdates", "communityFetch", "offline"]),
    ("Scanning and search", "&#xEDA2;", "How Sprawl reads your drives", &["oAlwaysAdmin", "oExplorer", "searchDrivesOn", "searchHistory"]),
    ("History", "&#xE81C;", "See how your disk changes over time", &["snapshots", "snapshot_cap_mb"]),
    ("Cleaning", "&#xE74D;", "What happens to your own files", &["recycleOwn", "privacyCleaners"]),
    ("Look", "&#xE790;", "How the map is drawn", &["palette", "animate"]),
];

pub fn get(key: &str) -> Option<&'static Pref> {
    PREFS.iter().find(|p| p.key == key)
}

impl Pref {
    /// The default, as the draft holds it: "1"/"0", a number, a choice's index, text.
    pub fn default_value(&self) -> Option<String> {
        match &self.kind {
            Kind::Switch(on) => Some(if *on { "1" } else { "0" }.into()),
            Kind::Number(k, _) => Some(k.default.to_string()),
            Kind::Choice(_, i) => Some(i.to_string()),
            Kind::Text(_) => Some(String::new()),
            Kind::Custom => None,
        }
    }

    /// The help line, with a number's default, floor and ceiling.
    pub fn help_line(&self) -> String {
        match &self.kind {
            Kind::Number(k, unit) => {
                let one = |n: u64| if n == 1 { unit.trim_end_matches('s') } else { unit };
                let lim = if k.floor == 0 {
                    format!("Default {} {}; 0 turns this off.", k.default, one(k.default))
                } else if k.floor == k.default { format!("Default {} {}; it can't go lower.", k.default, one(k.default)) } else { format!("Default {} {}; at least {} {}.", k.default, one(k.default), k.floor, one(k.floor)) };
                if self.help.is_empty() {
                    lim
                } else {
                    format!("{} {lim}", self.help)
                }
            }
            _ => self.help.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each setting's default is the documented one (handoff-opus.md, "Settings and the wizard").
    #[test]
    fn defaults_are_the_documented_ones() {
        let documented: &[(&str, &str)] = &[
            ("units", "0"),
            ("decimal", "0"),
            ("scanExclude", ""),
            ("startTab", "0"),
            ("theme", "0"),
            ("aiWebLookup", "0"),
            ("oUpdates", "1"),
            ("communityFetch", "0"),
            ("offline", "0"),
            ("oAlwaysAdmin", "0"),
            ("oExplorer", "0"),
            ("oAuto", "1"),
            ("oPulses", "1"),
            ("afterScan", "0"),
            ("searchHistory", "1"),
            ("searchSnapshots", "3"),
            ("searchMaxHits", "100000"),
            ("palette", "0"),
            ("animate", "1"),
            ("clUsualTicks", "1"),
            ("recycleOwn", "1"),
            ("privacyCleaners", "0"),
            ("adminShellMenu", "0"),
            ("adminHandOff", "0"),
            ("adminClaudeCode", "0"),
            ("adminUserTools", "0"),
            ("adminMcpDisk", "0"),
            ("tuneCommunityDays", "7"),
            ("oldScanMinutes", "60"),
            ("tuneFindingMinMb", "50"),
            ("tunePartMinKb", "1024"),
            ("tuneProjectIdleDays", "30"),
            ("tuneUnusedAppDays", "180"),
            ("tuneLeftoverIdleDays", "180"),
            ("tuneLeftoverMinMb", "50"),
            ("tuneLaterDays", "30"),
            ("tuneDupesMinMb", "50"),
            ("tuneDupesBudgetGb", "40"),
            ("snapshots", "0"),
            ("snapshot_cap_mb", "1"),
            ("tuneKeepFull", "3"),
            ("uninstaller", ""),
            // the alerts: all off; 10 GB free, 5 GB of growth
            ("alertLow", "0"),
            ("alertFreeGb", "1"),
            ("alertGrowth", "0"),
            ("alertGrowGb", "1"),
            ("alertDailyScan", "0"),
        ];
        assert_eq!(ALERT_AT.len(), ALERT_GB.len());
        assert_eq!(GROW_AT.len(), GROW_GB.len());
        for (k, d) in documented {
            assert_eq!(get(k).unwrap_or_else(|| panic!("{k}")).default_value().as_deref(), Some(*d), "{k}");
        }
        // every setting that has a default is in the table above
        for p in PREFS.iter().filter(|p| p.default_value().is_some()) {
            if !["oTilt", "oNames", "oCushion", "oMirror", "oFree", "oAlloc", "aa"].contains(&p.key) {
                assert!(documented.iter().any(|(k, _)| *k == p.key), "{} has no documented default", p.key);
            }
        }
        // the map's switches keep the Options menu's defaults
        for (k, on) in [("oTilt", "1"), ("oNames", "1"), ("oCushion", "1"), ("oMirror", "1"), ("oFree", "0"), ("oAlloc", "0"), ("aa", "2")] {
            assert_eq!(get(k).unwrap().default_value().as_deref(), Some(on), "{k}");
        }
        assert_eq!(CAPS.len(), CAPS_MB.len());
        assert_eq!(UNITS.len(), UNIT_ITEMS.len());
        assert_eq!(AA.len(), AA_ITEMS.len());
    }

    /// The wizard asks nothing Settings doesn't have, and turns nothing on that sends or keeps
    /// data: what it shows comes from the same descriptions.
    #[test]
    fn the_wizard_is_a_part_of_settings() {
        let mut keys = std::collections::HashSet::new();
        for p in PREFS.iter() {
            assert!(keys.insert(p.key), "{} twice", p.key);
        }
        for (_, _, _, page) in WIZARD {
            for k in page {
                assert!(get(k).is_some(), "the wizard's {k} isn't a setting");
            }
        }
        for k in ["aiWebLookup", "communityFetch", "offline", "snapshots", "oAlwaysAdmin", "oExplorer"] {
            assert_eq!(get(k).unwrap().default_value().as_deref(), Some("0"), "{k} starts off");
        }
        // the uninstaller and the MCP line are in Settings only
        assert!(!WIZARD.iter().any(|w| w.3.contains(&"uninstaller") || w.3.contains(&"mcp")));
        // and every Options menu switch it stands for is a setting
        for m in MENU {
            assert!(matches!(get(m).map(|p| &p.kind), Some(Kind::Switch(_))), "{m}");
        }
    }

    /// No safety limit is a setting: the never-list, risk floors and size caps aren't here, and
    /// every number has a floor.
    #[test]
    fn safety_limits_are_not_settings() {
        for p in PREFS.iter() {
            let k = p.key.to_lowercase();
            for word in ["never", "risk", "confidence", "cap", "delay", "protect"] {
                if word == "never" && p.key == "neverTouch" {
                    continue; // the user's own keep-list: they may add and remove their own
                }
                if word == "cap" && p.key == "snapshot_cap_mb" {
                    continue; // space for history, not a safety cap
                }
                assert!(!k.contains(word), "{} looks like a safety limit", p.key);
            }
            if let Kind::Number(knob, _) = p.kind {
                assert_eq!(knob.key, p.key, "a number setting is its knob");
            }
        }
    }
}
