//! The thresholds a user may change in Settings > Advanced, each with its default, its floor and
//! its ceiling. A value is always held between them: lowering "idle after" below its floor, or
//! a size that can list more safe findings below its default, isn't possible, so a changed
//! threshold never makes anything green or ticked that wouldn't be otherwise.
//!
//! The app sets them at start and when Settings is saved; everything else reads them here.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// One threshold.
#[derive(Debug, PartialEq)]
pub struct Knob {
    /// Its key in settings.ini.
    pub key: &'static str,
    pub default: u64,
    /// The lowest value allowed.
    pub floor: u64,
    /// The highest value allowed.
    pub max: u64,
}

/// A project's build output counts as idle after this many days without a change.
pub const PROJECT_IDLE_DAYS: Knob = Knob { key: "tuneProjectIdleDays", default: 30, floor: 7, max: 3650 };
/// An application counts as unused after this many days.
pub const UNUSED_APP_DAYS: Knob = Knob { key: "tuneUnusedAppDays", default: 180, floor: 30, max: 3650 };
/// A leftover needs no change for this many days.
pub const LEFTOVER_IDLE_DAYS: Knob = Knob { key: "tuneLeftoverIdleDays", default: 180, floor: 30, max: 3650 };
/// A leftover's smallest size, MB.
pub const LEFTOVER_MIN_MB: Knob = Knob { key: "tuneLeftoverMinMb", default: 50, floor: 10, max: 100_000 };
/// The smallest finding listed, MB. Only raised: lower would list more safe findings.
pub const FINDING_MIN_MB: Knob = Knob { key: "tuneFindingMinMb", default: 50, floor: 50, max: 100_000 };
/// The smallest cleaner part listed, KB. Only raised, for the same reason.
pub const PART_MIN_KB: Knob = Knob { key: "tunePartMinKb", default: 1024, floor: 1024, max: 1_048_576 };
/// "Not now" hides a finding for this many days.
pub const LATER_DAYS: Knob = Knob { key: "tuneLaterDays", default: 30, floor: 1, max: 365 };
/// Duplicates: the smallest file compared, MB.
pub const DUPES_MIN_MB: Knob = Knob { key: "tuneDupesMinMb", default: 50, floor: 1, max: 100_000 };
/// Duplicates: how much is read to compare, GB.
pub const DUPES_BUDGET_GB: Knob = Knob { key: "tuneDupesBudgetGb", default: 40, floor: 1, max: 2000 };
/// Community cleaners are fetched again after this many days.
pub const COMMUNITY_DAYS: Knob = Knob { key: "tuneCommunityDays", default: 7, floor: 1, max: 90 };
/// Full snapshots kept per folder in History.
pub const KEEP_FULL: Knob = Knob { key: "tuneKeepFull", default: 3, floor: 1, max: 20 };
/// Saved scans Search may use.
pub const SEARCH_SNAPSHOTS: Knob = Knob { key: "searchSnapshots", default: 3, floor: 0, max: 10 };
/// Most results shown from other drives and saved scans.
pub const SEARCH_MAX_HITS: Knob = Knob { key: "searchMaxHits", default: 100_000, floor: 1_000, max: 1_000_000 };
/// A clean-up warns when its scan is older than this, minutes.
pub const OLD_SCAN_MINUTES: Knob = Knob { key: "oldScanMinutes", default: 60, floor: 5, max: 1440 };

pub const ALL: [&Knob; 14] = [
    &PROJECT_IDLE_DAYS,
    &UNUSED_APP_DAYS,
    &LEFTOVER_IDLE_DAYS,
    &LEFTOVER_MIN_MB,
    &FINDING_MIN_MB,
    &PART_MIN_KB,
    &LATER_DAYS,
    &DUPES_MIN_MB,
    &DUPES_BUDGET_GB,
    &COMMUNITY_DAYS,
    &KEEP_FULL,
    &SEARCH_SNAPSHOTS,
    &SEARCH_MAX_HITS,
    &OLD_SCAN_MINUTES,
];

fn values() -> &'static RwLock<HashMap<&'static str, u64>> {
    static V: OnceLock<RwLock<HashMap<&'static str, u64>>> = OnceLock::new();
    V.get_or_init(Default::default)
}

impl Knob {
    /// `v` held between the floor and the ceiling.
    pub fn clamp(&self, v: u64) -> u64 {
        v.clamp(self.floor, self.max)
    }
}

/// The value in use.
pub fn get(k: &Knob) -> u64 {
    values().read().ok().and_then(|m| m.get(k.key).copied()).unwrap_or(k.default)
}

/// Use `v` (held between the floor and the ceiling); returns what is used.
pub fn set(k: &Knob, v: u64) -> u64 {
    let v = k.clamp(v);
    if let Ok(mut m) = values().write() {
        m.insert(k.key, v);
    }
    v
}

/// The knob whose settings key is `key`.
pub fn by_key(key: &str) -> Option<&'static Knob> {
    ALL.iter().copied().find(|k| k.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults are the documented ones (handoff-opus.md, Settings), and each sits between
    /// its floor and ceiling.
    #[test]
    fn defaults_are_the_documented_ones() {
        let documented: [(&str, u64); 14] = [
            ("tuneProjectIdleDays", 30),
            ("tuneUnusedAppDays", 180),
            ("tuneLeftoverIdleDays", 180),
            ("tuneLeftoverMinMb", 50),
            ("tuneFindingMinMb", 50),
            ("tunePartMinKb", 1024),
            ("tuneLaterDays", 30),
            ("tuneDupesMinMb", 50),
            ("tuneDupesBudgetGb", 40),
            ("tuneCommunityDays", 7),
            ("tuneKeepFull", 3),
            ("searchSnapshots", 3),
            ("searchMaxHits", 100_000),
            ("oldScanMinutes", 60),
        ];
        for (k, d) in documented {
            let knob = by_key(k).unwrap_or_else(|| panic!("{k}"));
            assert_eq!(knob.default, d, "{k}");
            assert!(knob.floor <= knob.default && knob.default <= knob.max, "{k}");
        }
        // the code's own constants say the same
        assert_eq!(PROJECT_IDLE_DAYS.default, crate::PROJECT_IDLE_DAYS);
        assert_eq!(UNUSED_APP_DAYS.default, crate::UNUSED_DAYS);
        assert_eq!(LEFTOVER_IDLE_DAYS.default, crate::leftovers::IDLE_DAYS);
        assert_eq!(LEFTOVER_MIN_MB.default << 20, crate::leftovers::MIN_BYTES);
        assert_eq!(FINDING_MIN_MB.default << 20, crate::rules::DEFAULT_MIN);
        assert_eq!(PART_MIN_KB.default << 10, crate::cleaners::PART_MIN);
        assert_eq!(LATER_DAYS.default, crate::memory::LATER_DAYS);
        assert_eq!(COMMUNITY_DAYS.default * 86_400, crate::community::EVERY);
        assert_eq!(KEEP_FULL.default as usize, crate::history::KEEP_FULL);
    }

    /// The floors: idle times never below 7 days for projects and 30 for applications and
    /// leftovers, and the sizes that could list more safe findings never below their default.
    #[test]
    fn floors_hold() {
        assert_eq!(PROJECT_IDLE_DAYS.clamp(1), 7);
        assert_eq!(UNUSED_APP_DAYS.clamp(0), 30);
        assert_eq!(LEFTOVER_IDLE_DAYS.clamp(5), 30);
        assert_eq!(FINDING_MIN_MB.clamp(1), 50);
        assert_eq!(PART_MIN_KB.clamp(1), 1024);
        assert_eq!(OLD_SCAN_MINUTES.clamp(0), 5);
        assert_eq!(OLD_SCAN_MINUTES.clamp(100_000), 1440);
        // a knob nobody set reads its default
        assert_eq!(get(&DUPES_BUDGET_GB), 40);
    }
}
