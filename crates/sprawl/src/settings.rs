//! Options that survive a restart, kept as `key=value` lines in
//! `%APPDATA%\Sprawl\settings.ini`.

use std::collections::BTreeMap;
use std::path::PathBuf;

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

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text: String = self.values.iter().map(|(k, v)| format!("{k}={v}\r\n")).collect();
        // write a temporary file and swap it in, so a crash mid-write can't leave half a file
        let tmp = p.with_extension("ini.tmp");
        if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &p).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
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
    fn values_round_trip() {
        let mut s = Settings::default();
        s.set_bool("oTilt", false);
        s.set("lift", 1.5);
        assert_eq!(s.bool("oTilt"), Some(false));
        assert_eq!(s.num("lift"), Some(1.5));
        assert_eq!(s.bool("missing"), None);
    }
}
