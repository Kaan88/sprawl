//! Every setting, described once: its key, section, words, default and kind. The setup wizard
//! and Settings both build their rows from this list, so they always say the same thing and
//! read and write the same keys.

use sprawl_advisor::tuning::{self, Knob};

/// A section of Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sec {
    General,
    Theme,
    Scanning,
    Search,
    Map,
    Cleaning,
    History,
    Ai,
    Data,
}

impl Sec {
    pub const ALL: [Sec; 9] = [Sec::General, Sec::Theme, Sec::Scanning, Sec::Search, Sec::Map, Sec::Cleaning, Sec::History, Sec::Ai, Sec::Data];

    pub fn title(self) -> &'static str {
        match self {
            Sec::General => "General",
            Sec::Theme => "Theme",
            Sec::Ai => "AI and connections",
            Sec::Scanning => "Scanning",
            Sec::Search => "Search",
            Sec::Map => "Treemap",
            Sec::Cleaning => "Cleaning",
            Sec::History => "History and alerts",
            Sec::Data => "Data and recovery",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Sec::General => "&#xE713;",
            Sec::Theme => "&#xE771;",
            Sec::Ai => "&#xE945;",
            Sec::Scanning => "&#xEDA2;",
            Sec::Search => "&#xE71E;",
            Sec::Map => "&#xE790;",
            Sec::Cleaning => "&#xE74D;",
            Sec::History => "&#xE81C;",
            Sec::Data => "&#xE8B7;",
        }
    }

    /// The section a link names (`"ai"`, `"history"`...).
    pub fn by_id(id: &str) -> Option<Sec> {
        if id.eq_ignore_ascii_case("advisor") { return Some(Sec::Cleaning); }
        if id.eq_ignore_ascii_case("network") { return Some(Sec::Ai); }
        Sec::ALL.into_iter().find(|s| format!("{s:?}").eq_ignore_ascii_case(id))
    }

    pub fn groups(self) -> &'static [(&'static str, &'static [&'static str])] {
        match self {
            Sec::General => &[("Startup and navigation", &["startTab", "afterScan"]), ("Sizes and panels", &["units", "decimal", "oHeader", "oTypes", "oTreemap"])],
            Sec::Theme => &[],
            Sec::Scanning => &[("Folders", &["scanExclude", "programFolders"]), ("Drive selection and Explorer", &["oAuto", "oExplorer"]), ("Administrator permissions", &["oAlwaysAdmin", "adminShellMenu", "adminHandOff", "adminClaudeCode", "adminUserTools", "adminMcpDisk"])],
            Sec::Search => &[("Where Search looks", &["searchDrivesOn", "searchSnapshots"]), ("Saved and recent searches", &["searchHistory", "recentSearches"]), ("Results", &["searchColumns", "searchMaxHits"])],
            Sec::Map => &[("Colours and tiles", &["palette", "oNames", "oFree", "oAlloc"]), ("Motion and 3D", &["animate", "oPulses", "oTilt", "oMirror", "aa"]), ("Tile details", &["oCushion", "lift", "cushion", "tilt"])],
            Sec::Cleaning => &[("Cleanup choices", &["recycleOwn", "clUsualTicks", "oldScanMinutes"]), ("Keep list", &["neverTouch"]), ("Privacy and backups", &["privacyCleaners", "registryBackups"]), ("Advisor rules", &["uninstaller", "tuneFindingMinMb", "tunePartMinKb", "tuneLaterDays"]), ("Idle applications and projects", &["tuneProjectIdleDays", "tuneUnusedAppDays", "tuneLeftoverIdleDays", "tuneLeftoverMinMb"]), ("Duplicates", &["tuneDupesMinMb", "tuneDupesBudgetGb"])],
            Sec::History => &[("Scan history", &["snapshots", "snapshot_cap_mb", "tuneKeepFull", "historyDisk"]), ("Low space", &["alertLow", "alertFreeGb"]), ("Folder growth", &["alertGrowth", "alertGrowGb", "alertDailyScan"])],
            Sec::Ai => &[("Provider and permissions", &["ai", "aiWebLookup"]), ("Logs and integrations", &["aiLog", "mcp"]), ("Offline and downloads", &["offline", "oUpdates", "communityFetch", "tuneCommunityDays"])],
            Sec::Data => &[("Data and recovery", &["dataFolder"])],
        }
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
    /// Percentage in a bounded slider; persisted as the renderer's own scale.
    Range(u32, u32),
    /// Built by its own code: a list, buttons, the AI provider.
    Custom,
}

pub struct Pref {
    pub key: &'static str,
    pub sec: Sec,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: Kind,
}

const fn p(key: &'static str, sec: Sec, label: &'static str, help: &'static str, kind: Kind) -> Pref {
    Pref { key, sec, label, help, kind }
}

pub const UNITS: [&str; 6] = ["Best fit", "Bytes", "KB", "MB", "GB", "TB"];
pub const UNIT_ITEMS: [&str; 6] = ["uDyn", "uB", "uKB", "uMB", "uGB", "uTB"];
pub const AA: [&str; 4] = ["Off", "Low (2x)", "Medium (4x)", "High (8x)"];
pub const AA_ITEMS: [&str; 4] = ["aaOff", "aaLow", "aaMedium", "aaHigh"];
pub const CAPS: [&str; 4] = ["500 MiB", "1 GiB", "2 GiB", "5 GiB"];
pub const CAPS_MB: [u32; 4] = sprawl_advisor::history::Retention::CAPS_MB;
pub const AFTER_SCAN: [&str; 2] = ["Stay where I am", "Open the Advisor"];
/// "Open Sprawl on": the tab used last, or one of the tabs (look::start_tab reads the index).
/// "Warn me below": the free space a drive may fall to before the low-space notice.
pub const ALERT_AT: [&str; 4] = ["5 GiB", "10 GiB", "20 GiB", "50 GiB"];
pub const ALERT_GB: [u64; 4] = [5, 10, 20, 50];
/// "Grew by more than": how much a folder may grow in about a week before the growth notice.
pub const GROW_AT: [&str; 4] = ["2 GiB", "5 GiB", "10 GiB", "20 GiB"];
pub const GROW_GB: [u64; 4] = [2, 5, 10, 20];
pub const OPEN_ON: [&str; 6] = ["The tab used last", "Tree View", "File View", "Advisor", "Cleaners", "Search"];

/// The Options menu switches a setting stands for: read from and applied through the menu.
pub const MENU: [&str; 14] = ["oUpdates", "oAlwaysAdmin", "oExplorer", "oAuto", "oPulses", "oTilt", "oNames", "oCushion", "oMirror", "oFree", "oAlloc", "oHeader", "oTypes", "oTreemap"];
pub const ANIMATE: [&str; 3] = ["Follow Windows", "On", "Off"];

pub static PREFS: &[Pref] = &[
    // General
    p("scanExclude", Sec::Scanning, "Folders to skip", "Full paths separated by semicolons. Rescan to update results. Sprawl also refuses a program's cleanup command if its scope overlaps a skipped folder or can't be verified.", Kind::Text("D:\\Games; C:\\Work\\Private")),
    p("programFolders", Sec::Scanning, "Program folders", "Folders where installed programs live, beyond Program Files. The Advisor checks the large folders directly inside each entry for Windows compression. Nothing is deleted.", Kind::Custom),
    p("theme", Sec::Theme, "Light or dark", "Light or dark. Like Windows follows Settings > Personalisation > Colours. A change shows as soon as you save.", Kind::Choice(&crate::theme::CHOICES, 0)),
    p("units", Sec::General, "Sizes in", "Best fit or a fixed unit. The base below determines GB versus GiB. Search sizes, goals and alert thresholds keep their stated binary meaning.", Kind::Choice(&UNITS, 0)),
    p("decimal", Sec::General, "Use decimal sizes (GB)", "Off: binary sizes, 1 GiB = 1,024 MiB. On: decimal sizes, 1 GB = 1,000 MB. Changes size displays, not byte counts or search and cleanup thresholds.", Kind::Switch(false)),
    p(
        "startTab",
        Sec::General,
        "Open Sprawl on",
        "Takes effect next launch. Opening a folder from Explorer or the command line still starts a scan in Tree View.",
        Kind::Choice(&OPEN_ON, 0),
    ),
    p(
        "alertLow",
        Sec::History,
        "Tell me when a drive is nearly full",
        "Every 4 hours a scheduled task of your account looks at how much space each drive has free, nothing more, and shows a Windows notice when one is below the limit: at most once a week per drive. A click opens Sprawl on it. Nothing is scanned or cleaned by it.",
        Kind::Switch(false),
    ),
    p("alertFreeGb", Sec::History, "Nearly full means less free than", "", Kind::Choice(&ALERT_AT, 1)),
    p(
        "alertGrowth",
        Sec::History,
        "Tell me when a folder grows fast",
        "Uses existing scans in History. Turn on Keep scan history to include future scans.",
        Kind::Switch(false),
    ),
    p("alertGrowGb", Sec::History, "Fast means it grew by more than", "", Kind::Choice(&GROW_AT, 1)),
    p(
        "alertDailyScan",
        Sec::History,
        "Also read my user folder once a day for it",
        "Requires growth alerts and Keep scan history. Saves folder sizes only, at low priority.",
        Kind::Switch(false),
    ),
    p("dataFolder", Sec::Data, "Your data", "Settings and logs are in Roaming. Scan history and backups are in Local.", Kind::Custom),
    p("oHeader", Sec::General, "Show header", "", Kind::Switch(true)),
    p("oTypes", Sec::General, "Show file types", "", Kind::Switch(true)),
    p("oTreemap", Sec::General, "Show treemap", "", Kind::Switch(true)),
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
    p("oUpdates", Sec::Ai, "Check for updates automatically", "At startup and once a day while open, check GitHub for a signed release.", Kind::Switch(true)),
    p(
        "communityFetch",
        Sec::Ai,
        "Fetch community cleaners",
        "Download winapp2.ini and BleachBit definitions from GitHub. Each cleaner stays off until enabled.",
        Kind::Switch(false),
    ),
    p("offline", Sec::Ai, "Work offline", "Save to block new update, community and AI requests. Requests already sent may finish.", Kind::Switch(false)),
    // Scanning
    p(
        "oAlwaysAdmin",
        Sec::Scanning,
        "Always run as administrator",
        "Takes effect next launch, from a copy installed for all users. Windows asks before granting administrator rights.",
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
    p("oPulses", Sec::Map, "Watch for changes after a scan", "Tiles pulse when their files change (Live Change Pulses).", Kind::Switch(true)),
    p("afterScan", Sec::General, "After a scan", "Where Sprawl takes you when a scan finishes.", Kind::Choice(&AFTER_SCAN, 0)),
    // Search
    p("searchDrivesOn", Sec::Search, "Other drives Search may read", "A ticked drive is read when Search opens, so Search finds files there too. None is ticked until you choose.", Kind::Custom),
    p("searchHistory", Sec::Search, "Keep recent searches", "Turning it off clears recent searches. Pinned searches stay.", Kind::Switch(true)),
    p("recentSearches", Sec::Search, "Recent searches", "Pinned searches are managed from the Saved and recent searches menu.", Kind::Custom),
    p("searchColumns", Sec::Search, "Search columns", "Name is always shown.", Kind::Custom),
    p("searchSnapshots", Sec::Search, "Saved scans Search may use", "Search also looks in the newest saved scans of History that nothing else covers.", Kind::Number(&tuning::SEARCH_SNAPSHOTS, "scans")),
    p("searchMaxHits", Sec::Search, "Most results shown from other drives", "More takes more memory and time to sort.", Kind::Number(&tuning::SEARCH_MAX_HITS, "rows")),
    // Map and look
    p("palette", Sec::Map, "Colours", "The colours of the map and the file types.", Kind::Choice(&crate::paint::PALETTES, 0)),
    p("animate", Sec::Map, "Map animations", "Follow Windows uses its Animation effects preference.", Kind::Choice(&ANIMATE, 0)),
    p("oTilt", Sec::Map, "3D and tilt", "The map as lit 3D tiles, tilted.", Kind::Switch(true)),
    p("oNames", Sec::Map, "Folder and file names", "Names on the map's tiles.", Kind::Switch(true)),
    p("oCushion", Sec::Map, "Shading", "Cushion shading on the tiles.", Kind::Switch(true)),
    p("oMirror", Sec::Map, "Mirror floor", "A glossy reflection under the map.", Kind::Switch(true)),
    p("oFree", Sec::Map, "Free space", "The drive's free space as a tile.", Kind::Switch(false)),
    p("oAlloc", Sec::Map, "Size by allocated space", "Tiles sized by the space files take on disk, not their length.", Kind::Switch(false)),
    p("aa", Sec::Map, "Smooth 3D edges", "Higher uses more graphics power.", Kind::Choice(&AA, 2)),
    p("lift", Sec::Map, "Tile height on hover", "", Kind::Range(250, 100)),
    p("cushion", Sec::Map, "Shading strength", "", Kind::Range(200, 100)),
    p("tilt", Sec::Map, "Tilt amount", "", Kind::Range(100, 55)),
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
        "Adds a Privacy group to Cleaners: cookies, history and what forms remember in this account's browsers, by profile, and the short lists Explorer keeps in this account's own registry. They free almost no space, each is ticked on its own, and a browser database or a registry list is copied before it is edited. Off, Sprawl doesn't look at them.",
        Kind::Switch(false),
    ),
    p("registryBackups", Sec::Cleaning, "Registry backups", "The backups Sprawl wrote before clearing Explorer's registry lists in Cleaners. Put back writes the removed values back; values written since stay. The last ten are kept, in %LOCALAPPDATA%\\Sprawl\\registry-backups.", Kind::Custom),
    p("neverTouch", Sec::Cleaning, "Keep during cleanup", "Sprawl refuses a program's cleanup command if its scope overlaps a kept path or can't be verified. The review says why. To omit a folder from new scans, use Scanning > Folders to skip.", Kind::Custom),
    p("clUsualTicks", Sec::Cleaning, "Open Cleaners with my usual ticks", "Cleaners opens with the parts you ticked last time. Sprawl never ticks anything for you.", Kind::Switch(true)),
    p("tuneCommunityDays", Sec::Ai, "Fetch community definitions every", "Only when community fetching is on.", Kind::Number(&tuning::COMMUNITY_DAYS, "days")),
    p("oldScanMinutes", Sec::Cleaning, "Old scan warning after", "A clean-up says so when its scan is older than this.", Kind::Number(&tuning::OLD_SCAN_MINUTES, "minutes")),
    // Advisor
    p("uninstaller", Sec::Cleaning, "Uninstaller", "\"Hand to...\" in Advisor uses this instead of Windows' own uninstaller.", Kind::Text("Full path to an .exe (optional)")),
    p("mcp", Sec::Ai, "Use Sprawl from your own Claude Code", "Run this once in a terminal; Claude Code can then ask Sprawl about your disk while Sprawl is open.", Kind::Custom),
    p("tuneFindingMinMb", Sec::Cleaning, "Smallest finding listed", "", Kind::Number(&tuning::FINDING_MIN_MB, "MiB")),
    p("tunePartMinKb", Sec::Cleaning, "Smallest cleaner part listed", "", Kind::Number(&tuning::PART_MIN_KB, "KiB")),
    p("tuneProjectIdleDays", Sec::Cleaning, "A project counts as idle after", "Its build output is offered once nothing in the project changed for this long.", Kind::Number(&tuning::PROJECT_IDLE_DAYS, "days")),
    p("tuneUnusedAppDays", Sec::Cleaning, "An application counts as unused after", "Not started for this long (and 1 GiB or more).", Kind::Number(&tuning::UNUSED_APP_DAYS, "days")),
    p("tuneLeftoverIdleDays", Sec::Cleaning, "A leftover needs no change for", "A folder of an application that is gone.", Kind::Number(&tuning::LEFTOVER_IDLE_DAYS, "days")),
    p("tuneLeftoverMinMb", Sec::Cleaning, "A leftover's smallest size", "", Kind::Number(&tuning::LEFTOVER_MIN_MB, "MiB")),
    p("tuneLaterDays", Sec::Cleaning, "\"Not now\" hides a finding for", "", Kind::Number(&tuning::LATER_DAYS, "days")),
    p("tuneDupesMinMb", Sec::Cleaning, "Duplicates: smallest file", "", Kind::Number(&tuning::DUPES_MIN_MB, "MiB")),
    p("tuneDupesBudgetGb", Sec::Cleaning, "Duplicates: how much to read", "Files are read to compare them; more finds more and takes longer.", Kind::Number(&tuning::DUPES_BUDGET_GB, "GiB")),
    // History
    p(
        "snapshots",
        Sec::History,
        "Keep scan history",
        "After each scan Sprawl saves a snapshot of it: every file name of each scanned folder, with sizes and dates, on this PC only (%LOCALAPPDATA%\\Sprawl\\snapshots). About 5 MB for a whole system drive, the newest in full detail. It powers growth over time, the time-lapse and comparing scans.",
        Kind::Switch(false),
    ),
    p("snapshot_cap_mb", Sec::History, "Space for history", "Applied the next time a snapshot is saved; the oldest ones are thinned out first.", Kind::Choice(&CAPS, 1)),
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
            Kind::Range(_, start) => Some(start.to_string()),
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

    #[test]
    fn preferences_have_one_home_and_program_folders_stay_by_exclusions() {
        for p in PREFS.iter().filter(|p| p.sec != Sec::Theme) {
            let places: Vec<_> = Sec::ALL.into_iter()
                .filter(|sec| sec.groups().iter().any(|(_, keys)| keys.contains(&p.key))).collect();
            assert_eq!(places, [p.sec], "{}", p.key);
            assert_eq!(p.sec.groups().iter().filter(|(_, keys)| keys.contains(&p.key)).count(), 1, "{}", p.key);
        }
        assert_eq!(Sec::Scanning.groups()[0].1, &["scanExclude", "programFolders"]);
    }

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
            ("animate", "0"),
            ("oHeader", "1"),
            ("oTypes", "1"),
            ("oTreemap", "1"),
            ("lift", "100"),
            ("cushion", "100"),
            ("tilt", "55"),
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
