//! Options that survive a restart, kept as `key=value` lines in
//! `%APPDATA%\Sprawl\settings.ini`.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// Set for screenshot runs: nothing is written.
pub static READ_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn read_only() -> bool {
    READ_ONLY.load(std::sync::atomic::Ordering::Relaxed)
}

const SCHEMA: &str = "settingsSchema";
const VERSION: &str = "1";
type Values = BTreeMap<String, String>;

#[derive(Clone, Default)]
pub struct Settings {
    values: Values,
    pending: BTreeMap<String, Option<String>>,
}

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Sprawl").join("settings.ini"))
}

fn read(path: &Path) -> io::Result<Values> {
    let root = path.parent().and_then(Path::parent).ok_or_else(|| io::Error::other("No settings folder"))?;
    let bytes = sprawl_core::local::read_checked(root, &["Sprawl"], "settings.ini", 1 << 20)?;
    let Some(bytes) = bytes else { return Ok(Values::new()) };
    let text = String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut values = Values::new();
    for line in text.trim_start_matches('\u{feff}').lines().map(str::trim) {
        if line.is_empty() || line.starts_with([';', '#']) { continue; }
        if let Some((k, v)) = line.split_once('=').filter(|(k, _)| !k.trim().is_empty()) {
            values.insert(k.trim().to_string(), v.trim().to_string());
        } else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "The settings file has a malformed line"));
        }
    }
    Ok(values)
}

/// A machine-wide mutex for one profile file, including separate Sprawl instances. It isn't
/// the settings file (which is atomically replaced), and there is no lock file to get stranded.
struct SettingsLock(windows::Win32::Foundation::HANDLE);

impl SettingsLock {
    fn take(path: &Path) -> io::Result<Self> {
        use windows::Win32::Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
        let spelling = path.to_string_lossy().replace('/', "\\").to_lowercase();
        let hash = spelling.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
        let name = windows::core::HSTRING::from(format!("Global\\Sprawl.Settings.{hash:016x}"));
        let handle = unsafe { CreateMutexW(None, false, &name) }.map_err(io::Error::other)?;
        if matches!(unsafe { WaitForSingleObject(handle, 3000) }, WAIT_OBJECT_0 | WAIT_ABANDONED) {
            Ok(Self(handle))
        } else {
            let _ = unsafe { CloseHandle(handle) };
            Err(io::Error::other("Couldn't lock the settings file; try saving again"))
        }
    }
}

impl Drop for SettingsLock {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::System::Threading::ReleaseMutex(self.0) };
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

impl Settings {
    pub fn load() -> Self {
        if read_only() { return Self::default(); }
        match path().map(|p| read(&p)).transpose() {
            Ok(values) => Self { values: values.unwrap_or_default(), pending: BTreeMap::new() },
            Err(e) => {
                eprintln!("Couldn't read Sprawl settings: {e}");
                Self::default()
            }
        }
    }

    /// A missing key is never evidence of consent. Only the explicit old all-drive switch
    /// can migrate to an enabled list; never replace a current list with that legacy choice.
    pub fn migrate_search(&mut self, fixed: impl FnOnce() -> Vec<String>) -> bool {
        if self.get(SCHEMA).is_some() || self.bool("oSearchDrives") != Some(true) {
            return false;
        }
        if self.get("searchDrivesOn").is_none() {
            let off = self.get("searchDrivesOff").unwrap_or("").split(',').map(str::trim);
            let off: Vec<_> = off.collect();
            let on: Vec<_> = fixed().into_iter().filter(|d| !off.iter().any(|o| o.eq_ignore_ascii_case(d))).collect();
            self.set("searchDrivesOn", on.join(","));
        }
        self.set_bool("oSearchDrives", false);
        true
    }

    pub fn save(&mut self) -> io::Result<()> {
        self.save_at_controlled(path, read_only())
    }

    fn save_at_controlled(&mut self, locate: impl FnOnce() -> Option<PathBuf>, sandbox: bool) -> io::Result<()> {
        if sandbox { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Screenshot mode does not save settings")); }
        self.save_at(&locate().ok_or_else(|| io::Error::other("APPDATA is not set"))?)
    }

    fn save_at(&mut self, path: &Path) -> io::Result<()> {
        if self.pending.is_empty() { return Ok(()); }
        let _locked = SettingsLock::take(path)?;
        let mut disk = read(path)?;
        disk.entry(SCHEMA.into()).or_insert_with(|| VERSION.into());
        for (key, value) in &self.pending {
            match value {
                Some(v) if !key.contains(['=', '\r', '\n']) && !v.contains(['\r', '\n']) => { disk.insert(key.clone(), v.clone()); }
                Some(_) => return Err(io::Error::new(io::ErrorKind::InvalidInput, "A setting contains a line break")),
                None => { disk.remove(key); }
            }
        }
        let text: String = disk.iter().map(|(k, v)| format!("{k}={v}\r\n")).collect();
        sprawl_advisor::write_local(path, text.as_bytes(), true)?;
        self.pending.clear();
        Ok(())
    }

    pub(crate) fn disk_enabled(key: &str) -> bool {
        if read_only() { return false; }
        path().and_then(|p| read(&p).ok()).and_then(|v| v.get(key).cloned())
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(|v| match v {
            "1" => Some(true),
            "0" => Some(false),
            _ if v.eq_ignore_ascii_case("true") => Some(true),
            _ if v.eq_ignore_ascii_case("false") => Some(false),
            _ => None,
        })
    }

    pub fn num(&self, key: &str) -> Option<f64> {
        self.get(key)?.parse::<f64>().ok().filter(|v| v.is_finite())
    }

    pub fn set(&mut self, key: &str, value: impl ToString) {
        let value = value.to_string();
        if self.get(key) != Some(&value) {
            self.values.insert(key.to_string(), value.clone());
            self.pending.insert(key.to_string(), Some(value));
        }
    }

    pub(crate) fn remove(&mut self, key: &str) {
        self.values.remove(key);
        self.pending.insert(key.to_string(), None);
    }

    pub fn set_bool(&mut self, key: &str, on: bool) {
        self.set(key, if on { "1" } else { "0" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_requires_an_explicit_legacy_choice() {
        let mut s = Settings::default();
        s.set("window", "placed");
        assert!(!s.migrate_search(|| panic!("no drive enumeration for ambiguous files")));
        assert_eq!(s.bool("snapshots"), None);
        s.set_bool("oSearchDrives", true);
        s.set("searchDrivesOff", "d:\\");
        assert!(s.migrate_search(|| vec!["C:\\".into(), "D:\\".into()]));
        assert_eq!(s.get("searchDrivesOn"), Some("C:\\"));
        assert!(!s.migrate_search(Vec::new));
    }

    #[test]
    fn stale_writers_merge_and_stamp_first_save() {
        let dir = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-settings").unwrap();
        let p = dir.path().join("Sprawl").join("settings.ini");
        let (mut a, mut b) = (Settings::default(), Settings::default());
        a.set("scanExclude", "D:\\Private");
        a.save_at(&p).unwrap();
        b.set("window", "placed");
        b.save_at(&p).unwrap();
        let disk = read(&p).unwrap();
        assert_eq!((disk[SCHEMA].as_str(), disk["scanExclude"].as_str(), disk["window"].as_str()), (VERSION, "D:\\Private", "placed"));
        assert!(!disk.contains_key("snapshots"));
        assert!(!disk.contains_key("searchDrivesOn"));
    }

    #[test]
    fn a_malformed_file_is_not_replaced_by_defaults() {
        let dir = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-settings").unwrap();
        let p = dir.path().join("Sprawl").join("settings.ini");
        sprawl_advisor::write_local(&p, b"not a setting\n", false).unwrap();
        let mut s = Settings::default();
        s.set_bool("snapshots", false);
        assert!(s.save_at(&p).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"not a setting\n");
        assert!(s.pending.contains_key("snapshots"));
    }

    #[test]
    fn screenshot_save_is_refused_before_locating_any_profile_path() {
        let mut s = Settings::default();
        s.set("scanExclude", "D:\\Private");
        let r = s.save_at_controlled(|| panic!("screenshot cannot inspect the profile"), true);
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
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
