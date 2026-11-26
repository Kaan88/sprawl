//! Folders where programs are installed, beyond Windows' own Program Files folders. The
//! compression rule reads this list; it is not a scan exclusion and nothing in it is skipped.
//!
//! The list is kept in the app's settings (`programFolders`) and loaded once into this process
//! at startup, so a rule never reads the settings file itself. Settings replaces the snapshot
//! when the list is saved.
use std::path::{Path, PathBuf};

/// The settings key the list is kept under.
pub const SETTING: &str = "programFolders";

/// The list in use by this process. Empty until the app loads its settings.
static CONFIGURED: std::sync::RwLock<Vec<PathBuf>> = std::sync::RwLock::new(Vec::new());

/// A semicolon-separated list of full paths, with the same validation as the folders to skip.
pub fn parse(text: &str) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for value in text.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let value = value.trim_matches('"').replace('/', "\\");
        let b = value.as_bytes();
        if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\'
            || value.contains(['*', '?', '%', '\r', '\n', '\0', '"']) || value[2..].contains(':')
            || value[3..].split('\\').any(|part| matches!(part, "." | "..") || part.ends_with('.') || part.ends_with(' ')) {
            return Err("Program folders must be full absolute paths separated by semicolons, without wildcards, variables or ..".into());
        }
        let p: PathBuf = Path::new(&value).components().collect();
        if !paths.contains(&p) { paths.push(p); }
        if paths.len() > 100 { return Err("Use at most 100 program folders.".into()); }
    }
    Ok(paths)
}

/// The list in use now, which the compression rule reads. No disk is touched.
pub fn configured() -> Vec<PathBuf> {
    CONFIGURED.read().map(|paths| paths.clone()).unwrap_or_default()
}

/// Replace the list in use now. The caller stores it in the app's settings separately.
pub fn replace(paths: Vec<PathBuf>) {
    if let Ok(mut list) = CONFIGURED.write() { *list = paths; }
}

/// Whether a folder's spelling can be one entry in the semicolon-separated setting. A Windows
/// folder name may itself contain `;`; such a name can't round-trip here, so it is refused
/// rather than written as two entries.
pub fn storable(path: &Path) -> bool {
    !path.to_string_lossy().contains(';')
}

pub(crate) fn allowed(path: &Path, home: Option<&Path>, never: &crate::safety::Never) -> bool {
    path.is_absolute() && path.parent().is_some() && never.complete()
        && !home.is_some_and(|h| crate::finding::within(h, path))
        && path.to_str().is_some_and(|s| !never.is_user_data(s))
}

/// Validate a new entry after opening it by its long local name. The same rule
/// is applied when a previously saved entry is used for compression.
pub fn validate_library(path: &Path) -> Result<(), String> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let never = crate::safety::Never::for_compression(home.as_deref().and_then(Path::to_str));
    allowed(path, home.as_deref(), &never).then_some(())
        .ok_or_else(|| "Choose a program library, not a personal folder (such as Documents, Desktop or Pictures) or a folder that contains one.".into())
}

/// The settings value for `paths` (each one [`storable`]): the spelling [`parse`] reads back.
pub fn value(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("; ")
}

/// Load the settings text into this process. Invalid text leaves an empty list; the Settings
/// page validates the text before it is saved.
pub fn load(text: &str) {
    replace(parse(text).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_paths_and_validation() {
        let paths = parse(r"C:\Programs; D:/Programs/More").unwrap();
        assert_eq!(paths, [PathBuf::from(r"C:\Programs"), PathBuf::from(r"D:\Programs\More")]);
        assert_eq!(parse(r"C:\Programs; C:\Programs").unwrap(), vec![PathBuf::from(r"C:\Programs")]);
        for bad in [r"relative", r"C:\Programs\*", r"C:\Programs\..\Other", r"%ProgramFiles%\App", r"C:\Programs;Tools"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn the_setting_value_round_trips() {
        let paths = parse(r"C:\Programs\One; D:\Programs\Two").unwrap();
        assert_eq!(parse(&value(&paths)).unwrap(), paths);
        assert_eq!(value(&[]), "");
    }

    #[test]
    fn a_semicolon_in_a_folder_name_is_refused() {
        assert!(!storable(Path::new(r"C:\Programs;Tools")));
        assert!(storable(Path::new(r"C:\Programs")));
    }
}
