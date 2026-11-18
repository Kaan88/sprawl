//! EXE installers (A2). Version resources are evidence, not an installer registry: require
//! product AND publisher to match a registered app, plus installer wording in both the
//! original filename and description. Always a labelled guess, orange and never scripted.

use std::path::Path;
use crate::facts::{AppKind, Facts};
use crate::finding::{Action, Finding, Remedy, Risk, Source};
use sprawl_core::{Tree, ROOT};

pub const ID: &str = "kind.installed-exe";

#[derive(Default)]
struct Version {
    product: String,
    company: String,
    description: String,
    original: String,
}

fn norm(s: &str) -> String { s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase() }

fn owner<'a>(v: &Version, facts: &'a Facts) -> Option<&'a crate::facts::App> {
    let (product, company, description, original) = (norm(&v.product), norm(&v.company), norm(&v.description), norm(&v.original));
    if product.chars().count() < 4 || company.chars().count() < 3 || original.contains(['\\', '/', ':'])
        || original.contains("unins") || description.contains("uninstall")
        || !original.ends_with(".exe") || !(original.contains("setup") || original.contains("install"))
        || !(description.contains("setup") || description.contains("installer") || description.contains("installation")) {
        return None;
    }
    let mut matches = facts.apps.iter().filter(|a| a.kind == AppKind::Installed && norm(&a.name) == product
        && a.publisher.as_deref().is_some_and(|p| norm(p) == company));
    let app = matches.next()?;
    // Ambiguous registrations do not identify a spare installer confidently enough.
    matches.next().is_none().then_some(app)
}

pub fn find(tree: &Tree, facts: &Facts, min: u64) -> Vec<Finding> {
    let home = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from);
    find_with(tree, facts, min, home.as_deref(), version)
}

fn find_with(tree: &Tree, facts: &Facts, min: u64, home: Option<&Path>, mut read: impl FnMut(&Path) -> Option<Version>) -> Vec<Finding> {
    let never = crate::safety::Never::default_list();
    if !never.complete() || tree.method == sprawl_core::ScanMethod::Csv { return Vec::new(); }
    let mut stack = vec![ROOT];
    let mut out = Vec::new();
    let mut reads = 0;
    while let Some(n) = stack.pop() {
        let node = &tree.nodes[n];
        if node.is_reparse_point() || node.attributes & crate::dupes::NOT_LOCAL != 0
            || !crate::scan_name_ok(tree, n) { continue; }
        if node.is_dir { stack.extend_from_slice(&node.children); continue; }
        if n == ROOT || node.size < min || !node.name.to_ascii_lowercase().ends_with(".exe") { continue; }
        let path = tree.path_of(n);
        if never.hit(&path.to_string_lossy()).is_some() || facts.owner_of(&path).is_some()
            || crate::finding::other_account(&path, home) { continue; }
        if reads == 500 { break; }
        reads += 1;
        let Some(v) = read(&path) else { continue };
        let Some(app) = owner(&v, facts) else { continue };
        let mut f = Finding {
            path, owner: Some(app.name.clone()), bytes: node.size, risk: Risk::Orange,
            remedy: Remedy::Delete, command: None,
            title: format!("{} (probably an installer of {})", node.name, app.name),
            evidence: vec![
                format!("The file's ProductName and CompanyName match the installed registration for {}", app.name),
                format!("Its own description says {}; original filename {}", crate::facts::clean_text(&v.description), crate::facts::clean_text(&v.original)),
                "These are the file's claims, not proof of an identical version or a spare copy. Check before removing it".into(),
                "Only this EXE would go. Keep offline installers, repair copies and installers for another PC".into(),
            ],
            confidence: 0.6, source: Source::Rule(ID.into()), allow_protected: false,
            trusted: true, builtin: true, on_path: true, action: None, admin: false,
            carved: Vec::new(), gain: None, forced_from: None,
        };
        f.set_action(Action::Remove, false);
        out.push(f);
    }
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out.truncate(50);
    out
}

#[cfg(not(windows))]
fn version(_: &Path) -> Option<Version> { None }

#[cfg(windows)]
fn version(path: &Path) -> Option<Version> {
    use std::os::windows::ffi::OsStrExt;
    use core::ffi::c_void;
    #[link(name = "version")]
    extern "system" {
        fn GetFileVersionInfoSizeW(path: *const u16, handle: *mut u32) -> u32;
        fn GetFileVersionInfoW(path: *const u16, handle: u32, len: u32, data: *mut c_void) -> i32;
        fn VerQueryValueW(data: *const c_void, key: *const u16, value: *mut *mut c_void, len: *mut u32) -> i32;
    }
    fn query<'a>(buf: &'a [u8], key: &str, unit: usize) -> Option<&'a [u8]> {
        let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
        let (mut ptr, mut len) = (std::ptr::null_mut(), 0);
        if unsafe { VerQueryValueW(buf.as_ptr().cast(), key.as_ptr(), &mut ptr, &mut len) } == 0 { return None; }
        let offset = (ptr as usize).checked_sub(buf.as_ptr() as usize)?;
        buf.get(offset..offset.checked_add((len as usize).checked_mul(unit)?)?)
    }
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if path[..path.len() - 1].contains(&0) { return None; }
    let mut unused = 0;
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), &mut unused) };
    if size == 0 || size > 1 << 20 { return None; }
    let mut buf = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(path.as_ptr(), 0, size, buf.as_mut_ptr().cast()) } == 0 { return None; }
    let translations = query(&buf, r"\VarFileInfo\Translation", 1)?;
    for pair in translations.chunks_exact(4).take(16) {
        let lang = u16::from_le_bytes([pair[0], pair[1]]);
        let cp = u16::from_le_bytes([pair[2], pair[3]]);
        let field = |name: &str| -> Option<String> {
            let bytes = query(&buf, &format!(r"\StringFileInfo\{lang:04x}{cp:04x}\{name}"), 2)?;
            let words: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
            let s = String::from_utf16(&words).ok()?;
            let s = s.trim_end_matches('\0');
            if s.is_empty() || s.contains('\0') || s.chars().count() > 256 { return None; }
            Some(s.to_string())
        };
        if let (Some(product), Some(company), Some(description), Some(original)) =
            (field("ProductName"), field("CompanyName"), field("FileDescription"), field("OriginalFilename")) {
            return Some(Version { product, company, description, original });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata() -> Version { Version { product: "Example Editor".into(), company: "Example Ltd".into(), description: "Example Editor Setup".into(), original: "EditorSetup.exe".into() } }
    fn facts() -> Facts { Facts::from_apps(vec![crate::facts::App {
        name: "Example Editor".into(), publisher: Some("Example Ltd".into()), kind: AppKind::Installed,
        locations: vec![], last_used: None, how_to_remove: "Settings".into(),
    }]) }

    #[test]
    fn identity_and_installer_evidence_are_both_required() {
        let known = facts();
        assert!(owner(&metadata(), &known).is_some());
        for field in ["product", "company", "description", "original"] {
            let mut v = metadata();
            match field { "product" => v.product = "Example Editor Pro".into(), "company" => v.company = "Different Ltd".into(),
                "description" => v.description = "Portable editor".into(), _ => v.original = "editor.exe".into() }
            assert!(owner(&v, &known).is_none(), "{field}");
        }
        let mut ambiguous = facts();
        ambiguous.apps.push(ambiguous.apps[0].clone());
        assert!(owner(&metadata(), &ambiguous).is_none());
        let mut uninstaller = metadata();
        uninstaller.original = "uninstall.exe".into();
        uninstaller.description = "Example Editor Uninstaller".into();
        assert!(owner(&uninstaller, &known).is_none(), "uninstallers are not spare setup programs");
    }

    #[test]
    fn guesses_use_existing_review_and_never_enter_scripts_or_goals() {
        let dir = std::env::temp_dir().join(format!("sprawl-exe-installer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("renamed.exe");
        std::fs::write(&path, b"not executed").unwrap();
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let found = find_with(&tree, &facts(), 1, Some(&dir), |_| Some(metadata()));
        assert_eq!(found.len(), 1);
        let f = &found[0];
        assert!(f.heuristic());
        assert_eq!((f.risk, f.remedy, f.action.clone()), (Risk::Orange, Remedy::Delete, Some(Action::Remove)));
        assert!(crate::plan::plan(&found, &[true], 1).picked.is_empty());
        assert!(crate::safety::check_in(f, &tree, &crate::safety::Never::default_list(), &[], Some(&dir), None).is_ok());
        let exported = crate::export::script(&[f], "fixture", std::time::SystemTime::now());
        assert!(!exported.contains("renamed.exe"), "guesses must not be scripted");
        assert!(version(&path).is_none(), "a fake PE has no version resources");
        assert_eq!(std::fs::read(&path).unwrap(), b"not executed");
        std::fs::create_dir_all(dir.join("second")).unwrap();
        let multiple = sprawl_core::scan_many(&[path.clone(), dir.join("second")], &sprawl_core::Progress::new()).unwrap();
        let multi_found = find_with(&multiple, &facts(), 1, Some(&dir), |_| Some(metadata()));
        assert_eq!(multi_found.len(), 1, "absolute first-tier EXE names remain eligible");
        assert_eq!(multi_found[0].path, path);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore]
    #[cfg(windows)]
    fn reads_windows_version_resource_without_running_it() {
        let path = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe");
        let v = version(&path).expect("Windows' command processor has a version resource");
        assert!(matches!(v.original.to_ascii_lowercase().as_str(), "cmd.exe" | "cmd.exe.mui"));
        assert!(!v.product.is_empty() && !v.company.is_empty());
        assert!(owner(&v, &facts()).is_none(), "an installed application is not automatically an installer");
    }
}
