//! Options that survive a restart, kept as `key=value` lines in
//! `%APPDATA%\Sprawl\settings.ini`.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Set for screenshot runs: nothing is written.
pub static READ_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Default)]
pub struct Settings {
    values: BTreeMap<String, String>,
}

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Sprawl").join("settings.ini"))
}

impl Settings {
    pub fn load() -> Self {
        let mut values = BTreeMap::new();
        if let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) {
            for line in text.trim_start_matches('\u{feff}').lines() {
                if let Some((k, v)) = line.split_once('=') {
                    values.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
        Settings { values }
    }

    /// Defaults that changed: a settings file from an older Sprawl keeps what it had (the old
    /// default), a new install gets the new one.
    pub fn keep_old_defaults(&mut self) {
        if self.values.is_empty() {
            return;
        }
        // read every fixed drive when Search opens: on before 0.2.6 (ticks them all in
        // Settings > Search once, then is set to 0)
        self.values.entry("oSearchDrives".into()).or_insert_with(|| "1".into());
        // scan history: kept unless switched off before 0.2.6; a new install starts with it off
        self.values.entry("snapshots".into()).or_insert_with(|| "1".into());
    }

    /// ONE-TIME, 1.0.7 only: puts the theme back to the defaults (the new look of 1.0.6 and
    /// "Like Windows"), once, on a settings file from before. The user's saved Custom picks
    /// (`look.custom`) stay. Remove this (and its call sites and test) in the release after
    /// 1.0.7, leaving the `lookReset107` key alone; see docs/plans/plan-1.0.5.md section L.
    /// Returns whether anything changed.
    pub fn reset_look_once(&mut self) -> bool {
        const DONE: &str = "lookReset107";
        if self.values.contains_key(DONE) {
            return false;
        }
        // a new install has nothing to reset; marked in memory only, so the file stays empty
        // (keep_old_defaults reads an empty file as a new install)
        if self.values.is_empty() {
            self.set_bool(DONE, true);
            return false;
        }
        self.values.retain(|k, _| k == "look.custom" || !(k.starts_with("look.") || k == "theme"));
        self.set_bool(DONE, true);
        true
    }

    pub fn save(&self) {
        // a screenshot run (and the dev steps it plays) leaves the user's settings alone
        if READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let Some(p) = path() else { return };
        let text: String = self.values.iter().map(|(k, v)| format!("{k}={v}\r\n")).collect();
        // a fresh temporary file swapped in through its handle: a crash mid-write can't leave
        // half a file, and an elevated Sprawl never writes through a link planted on the way
        let _ = sprawl_advisor::write_local(&p, text.as_bytes(), true);
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.get(key).map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    }

    pub fn num(&self, key: &str) -> Option<f64> {
        self.get(key)?.parse().ok()
    }

    pub fn set(&mut self, key: &str, value: impl ToString) {
        self.values.insert(key.to_string(), value.to_string());
    }

    pub fn set_bool(&mut self, key: &str, on: bool) {
        self.set(key, if on { "1" } else { "0" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_files_keep_their_defaults() {
        let mut new = Settings::default();
        new.keep_old_defaults();
        assert_eq!(new.bool("oSearchDrives"), None, "a new install takes the new default (off)");
        assert_eq!(new.bool("snapshots"), None, "and keeps no scan history until chosen");
        let mut old = Settings::default();
        old.set("units", "uDyn");
        old.keep_old_defaults();
        assert_eq!(old.bool("oSearchDrives"), Some(true));
        assert_eq!(old.bool("snapshots"), Some(true), "an older install keeps its history on");
        let mut chosen = Settings::default();
        chosen.set_bool("oSearchDrives", false);
        chosen.keep_old_defaults();
        assert_eq!(chosen.bool("oSearchDrives"), Some(false));
    }

    // remove with reset_look_once (the release after 1.0.7)
    #[test]
    fn look_resets_once() {
        let mut s = Settings::default();
        s.set("look.corners", "square");
        s.set("look.custom", "accent=iris;mode=2");
        s.set("theme", "2");
        s.set("units", "uDyn");
        assert!(s.reset_look_once());
        assert_eq!((s.get("look.corners"), s.get("theme")), (None, None), "back to the defaults");
        assert_eq!(s.get("look.custom"), Some("accent=iris;mode=2"), "the saved Custom picks stay");
        assert_eq!(s.get("units"), Some("uDyn"), "other settings stay");
        s.set("look.corners", "square");
        assert!(!s.reset_look_once(), "only once");
        assert_eq!(s.get("look.corners"), Some("square"), "a pick made after the reset stays");
        let mut new = Settings::default();
        assert!(!new.reset_look_once(), "a new install saves nothing here");
        assert!(!new.reset_look_once());
    }

    #[test]
    fn values_round_trip() {
        let mut s = Settings::default();
        s.set_bool("oTilt", false);
        s.set("lift", 1.5);
        assert_eq!(s.bool("oTilt"), Some(false));
        assert_eq!(s.num("lift"), Some(1.5));
        assert_eq!(s.bool("missing"), None);
    }
}
