//! A2: downloaded MSI packages of products Windows reports installed. Product identity comes
//! from the package's Property table, never its filename. No installer is started. Packages
//! in an installed app, a protected folder or a registered repair source are left alone.

use std::path::{Path, PathBuf};
use sprawl_core::{Tree, ROOT};
use crate::facts::Facts;
use crate::finding::{within, Action, Finding, Remedy, Risk, Source};

pub const ID: &str = "kind.installed-msi";
const MOST_PACKAGES: usize = 500;

struct Installed {
    name: String,
    code: String,
    // The cached package is a file; the other entries are registered source directories.
    cached: PathBuf,
    sources: Vec<PathBuf>,
}

pub fn find(tree: &Tree, facts: &Facts, min: u64) -> Vec<Finding> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    find_with(tree, facts, min, home.as_deref(), native::installed)
}

fn find_with(tree: &Tree, facts: &Facts, min: u64, home: Option<&Path>, mut read: impl FnMut(&Path) -> Option<Installed>) -> Vec<Finding> {
    let never = crate::safety::Never::default_list();
    if !never.complete() || tree.method == sprawl_core::ScanMethod::Csv {
        return Vec::new();
    }
    let mut stack = vec![ROOT];
    let mut out = Vec::new();
    let mut read_count = 0;
    while let Some(n) = stack.pop() {
        let node = &tree.nodes[n];
        // Unlike the scan root's name, any descendant's name must be one plain component.
        if node.is_reparse_point() || node.attributes & crate::dupes::NOT_LOCAL != 0
            || n != ROOT && node.name.contains(['\\', '/', ':']) {
            continue;
        }
        if node.is_dir {
            stack.extend_from_slice(&node.children);
            continue;
        }
        if n == ROOT || node.size < min || !node.name.to_ascii_lowercase().ends_with(".msi") {
            continue;
        }
        let path = tree.path_of(n);
        if never.hit(&path.to_string_lossy()).is_some() || facts.owner_of(&path).is_some()
            || crate::finding::other_account(&path, home) {
            continue;
        }
        if read_count == MOST_PACKAGES {
            break;
        }
        read_count += 1;
        let Some(app) = read(&path) else { continue };
        // Missing source information is not evidence that a package is spare.
        if app.cached.as_os_str().is_empty() || app.sources.is_empty()
            || repair_source(&path, &app) {
            continue;
        }
        let name = crate::facts::clean_text(&app.name);
        if name.is_empty() {
            continue;
        }
        let mut f = Finding {
            path, owner: Some(name.clone()), bytes: node.size, risk: Risk::Orange,
            remedy: Remedy::Delete, command: None,
            title: format!("{} (installer of an installed program)", node.name),
            evidence: vec![
                format!("Windows Installer reports {name} installed for this account"),
                format!("Matched the package's ProductCode {}, not its filename", app.code),
                "At this check, outside the registered installation sources and cached package; only this installer would go".into(),
                "Keep it if you need an offline copy, a rollback or installation on another PC; installed does not mean you no longer want the installer".into(),
            ],
            confidence: 0.8, source: Source::Rule(ID.into()), allow_protected: false,
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

/// Compare spelling and, for existing local paths, Windows' resolved names as well (8.3
/// names and junctions may name the same source). Never resolve a registered network path.
fn repair_source(path: &Path, app: &Installed) -> bool {
    let local = |p: &Path| {
        let s = p.to_string_lossy();
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
        let b = s.as_bytes();
        b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
    };
    let Ok(real) = std::fs::canonicalize(path) else { return true };
    std::iter::once(&app.cached).chain(app.sources.iter()).any(|source| {
        if within(path, source) { return true; }
        if !local(source) { return false; }
        match std::fs::canonicalize(source) {
            Ok(s) => within(&real, &s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => true, // cannot check: leave it alone
        }
    })
}

#[cfg(not(windows))]
mod native {
    pub(super) fn installed(_: &std::path::Path) -> Option<super::Installed> { None }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "msi")]
    extern "system" {
        fn MsiOpenDatabaseW(path: *const u16, mode: *const u16, db: *mut u32) -> u32;
        fn MsiCloseHandle(handle: u32) -> u32;
        fn MsiDatabaseOpenViewW(db: u32, query: *const u16, view: *mut u32) -> u32;
        fn MsiViewExecute(view: u32, record: u32) -> u32;
        fn MsiViewFetch(view: u32, record: *mut u32) -> u32;
        fn MsiRecordGetStringW(record: u32, field: u32, value: *mut u16, len: *mut u32) -> u32;
        fn MsiQueryProductStateW(code: *const u16) -> i32;
        fn MsiGetProductInfoW(code: *const u16, property: *const u16, value: *mut u16, len: *mut u32) -> u32;
        fn MsiSourceListEnumSourcesW(code: *const u16, sid: *const u16, context: u32, options: u32, index: u32, source: *mut u16, len: *mut u32) -> u32;
        #[cfg(test)]
        fn MsiDatabaseCommit(db: u32) -> u32;
        #[cfg(test)]
        fn MsiEnumProductsW(index: u32, code: *mut u16) -> u32;
    }

    struct Handle(u32);
    impl Drop for Handle {
        fn drop(&mut self) { unsafe { MsiCloseHandle(self.0); } }
    }

    fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() }

    // Fixed bounded buffers: too long, missing, invalid UTF-16 or any API error means no row.
    fn text(mut get: impl FnMut(*mut u16, *mut u32) -> u32) -> Option<String> {
        let mut buf = vec![0u16; 32_768];
        let mut len = buf.len() as u32;
        if get(buf.as_mut_ptr(), &mut len) != 0 || len == 0 || len as usize >= buf.len() {
            return None;
        }
        let s = String::from_utf16(&buf[..len as usize]).ok()?;
        (!s.contains('\0')).then_some(s)
    }

    pub(super) fn installed(path: &Path) -> Option<Installed> {
        let p: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if p[..p.len() - 1].contains(&0) { return None; }
        let mut raw = 0;
        // MSIDBOPEN_READONLY = 0. Database access only; never OpenPackage/InstallProduct.
        if unsafe { MsiOpenDatabaseW(p.as_ptr(), std::ptr::null(), &mut raw) } != 0 { return None; }
        let db = Handle(raw);
        let code = code_in(&db)?;
        let c = wide(&code);
        // INSTALLSTATE_DEFAULT = 5; advertised, absent, another user's or unknown is refused.
        if unsafe { MsiQueryProductStateW(c.as_ptr()) } != 5 { return None; }
        let info = |prop: &str| {
            let prop = wide(prop);
            text(|b, l| unsafe { MsiGetProductInfoW(c.as_ptr(), prop.as_ptr(), b, l) })
        };
        let name = info("InstalledProductName")?;
        let cached = PathBuf::from(info("LocalPackage")?);
        let mut sources = vec![PathBuf::from(info("InstallSource")?)];
        let mut contexts = 0;
        // Read registered local/network directory strings only; never open a network source.
        for context in [1, 2, 4] { // user-managed, user-unmanaged, machine
            let mut complete = false;
            for i in 0..64 {
                let mut buf = vec![0u16; 32_768];
                let mut len = buf.len() as u32;
                let r = unsafe { MsiSourceListEnumSourcesW(c.as_ptr(), std::ptr::null(), context, 1, i, buf.as_mut_ptr(), &mut len) };
                if r == 1605 { complete = true; break; } // no product in this context
                if r == 259 { contexts += 1; complete = true; break; } // no more sources
                if r != 0 || len == 0 || len as usize >= buf.len() { return None; }
                let s = String::from_utf16(&buf[..len as usize]).ok()?;
                if s.contains('\0') { return None; }
                sources.push(PathBuf::from(s));
            }
            if !complete { return None; }
        }
        if contexts == 0 || !cached.is_absolute() || sources.iter().any(|s| !s.is_absolute()) { return None; }
        Some(Installed { name, code, cached, sources })
    }

    fn code_in(db: &Handle) -> Option<String> {
        let query = wide("SELECT `Value` FROM `Property` WHERE `Property` = 'ProductCode'");
        let mut raw = 0;
        if unsafe { MsiDatabaseOpenViewW(db.0, query.as_ptr(), &mut raw) } != 0 { return None; }
        let view = Handle(raw);
        if unsafe { MsiViewExecute(view.0, 0) } != 0 { return None; }
        let mut raw = 0;
        if unsafe { MsiViewFetch(view.0, &mut raw) } != 0 { return None; }
        let record = Handle(raw);
        let code = text(|b, l| unsafe { MsiRecordGetStringW(record.0, 1, b, l) })?;
        if !product_code(&code) { return None; }
        Some(code)
    }

    fn product_code(s: &str) -> bool {
        let b = s.as_bytes();
        b.len() == 38 && b[0] == b'{' && b[37] == b'}' && (1..37).all(|i|
            if [9, 14, 19, 24].contains(&i) { b[i] == b'-' } else { b[i].is_ascii_hexdigit() })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn only_product_guids() {
            assert!(product_code("{12345678-1234-1234-ABCD-1234567890AB}"));
            for s in ["", "setup.exe", "{12345678-1234-1234-ABCD-1234567890AX}", "{12345678_1234-1234-ABCD-1234567890AB}"] {
                assert!(!product_code(s));
            }
        }

        #[test]
        fn reads_real_msi_database_without_installing() {
            let path = std::env::temp_dir().join(format!("sprawl-msi-{}.msi", std::process::id()));
            assert!(!path.exists(), "fixture path must be unused");
            let p: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut raw = 0;
            // Create only our throwaway database. No installation/registration calls exist.
            assert_eq!(unsafe { MsiOpenDatabaseW(p.as_ptr(), 3usize as *const u16, &mut raw) }, 0);
            let db = Handle(raw);
            for sql in [
                "CREATE TABLE `Property` (`Property` CHAR(72) NOT NULL, `Value` CHAR(0) LOCALIZABLE PRIMARY KEY `Property`)",
                "INSERT INTO `Property` (`Property`, `Value`) VALUES ('ProductCode', '{E5FEE8D7-6B9D-4F7B-8A34-0532799D31F2}')",
            ] {
                let mut raw = 0;
                assert_eq!(unsafe { MsiDatabaseOpenViewW(db.0, wide(sql).as_ptr(), &mut raw) }, 0);
                let v = Handle(raw);
                assert_eq!(unsafe { MsiViewExecute(v.0, 0) }, 0);
            }
            assert_eq!(unsafe { MsiDatabaseCommit(db.0) }, 0);
            assert_eq!(code_in(&db).as_deref(), Some("{E5FEE8D7-6B9D-4F7B-8A34-0532799D31F2}"));
            drop(db);
            let before = std::fs::read(&path).unwrap();
            assert!(installed(&path).is_none(), "unknown products must not be offered");
            assert_eq!(std::fs::read(&path).unwrap(), before, "metadata reading must not change the package");
            std::fs::remove_file(path).unwrap();
        }

        /// Explicit read-only machine check; not a prerequisite for portable fixture tests.
        #[test]
        #[ignore]
        fn current_machine_registration() {
            let mut matched = 0;
            for i in 0..32 {
                let mut code = [0u16; 39];
                let r = unsafe { MsiEnumProductsW(i, code.as_mut_ptr()) };
                if r == 259 { break; }
                assert_eq!(r, 0);
                let prop = wide("LocalPackage");
                let Some(path) = text(|b, l| unsafe { MsiGetProductInfoW(code.as_ptr(), prop.as_ptr(), b, l) }) else { continue };
                // Read the cached package only to exercise the real product and source APIs.
                // Production find excludes this protected location before opening a package.
                if installed(Path::new(&path)).is_some() { matched += 1; }
            }
            println!("Verified native package/product/source reads for {matched} installed products; no installation or cleanup run");
            assert!(matched > 0, "no eligible registration available for a positive native check");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_packages_anywhere_but_not_repair_sources() {
        let dir = std::env::temp_dir().join(format!("sprawl-installers-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("offline")).unwrap();
        std::fs::create_dir_all(dir.join("repair")).unwrap();
        std::fs::create_dir_all(dir.join("app")).unwrap();
        for p in ["offline/renamed.MSI", "offline/missing.msi", "offline/bad.msi", "repair/setup.msi", "app/update.msi", "offline/small.msi", "offline/tool.exe", "offline/cached.msi"] {
            std::fs::write(dir.join(p), [0u8; 16]).unwrap();
        }
        let facts = Facts::from_apps(vec![crate::facts::App {
            name: "Installed".into(), publisher: None, kind: crate::facts::AppKind::Installed,
            locations: vec![dir.join("app")], last_used: None, how_to_remove: "Settings".into(),
        }]);
        let mut tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let small = crate::find_path(&tree, &dir.join("offline/small.msi")).unwrap();
        tree.nodes[small].size = 1;
        let mut calls = Vec::new();
        let found = find_with(&tree, &facts, 10, Some(&dir), |p| {
            calls.push(p.to_path_buf());
            if p.file_name().unwrap() == "missing.msi" || p.file_name().unwrap() == "bad.msi" { return None; }
            Some(Installed {
                name: "Example App".into(), code: "{12345678-1234-1234-ABCD-1234567890AB}".into(),
                cached: dir.join("offline/cached.msi"), sources: vec![dir.join("repair")],
            })
        });
        assert_eq!(found.len(), 1, "{found:?}");
        let f = &found[0];
        assert_eq!(f.path, dir.join("offline/renamed.MSI"));
        assert_eq!((f.risk, f.remedy, f.action.clone()), (Risk::Orange, Remedy::Delete, Some(Action::Remove)));
        assert!(!calls.contains(&dir.join("app/update.msi")));
        assert!(!calls.contains(&dir.join("offline/small.msi")));
        assert!(!calls.contains(&dir.join("offline/tool.exe")));
        assert!(crate::safety::check_in(f, &tree, &crate::safety::Never::default_list(), &[], Some(&dir), None).is_ok());
        assert!(find_with(&tree, &facts, 10, Some(&dir), |_| Some(Installed {
            name: "Example".into(), code: String::new(), cached: dir.join("cached"), sources: vec![],
        })).is_empty(), "missing repair-source data must not produce a delete");
        // No cleanup action was run; every fixture remains.
        assert!(dir.join("offline/renamed.MSI").exists());
        let eligible = crate::find_path(&tree, &f.path).unwrap();
        tree.nodes[eligible].attributes |= crate::dupes::NOT_LOCAL;
        find_with(&tree, &facts, 10, Some(&dir), |p| {
            assert_ne!(p, f.path, "never read an offline/cloud placeholder");
            None
        });
        tree.method = sprawl_core::ScanMethod::Csv;
        assert!(find_with(&tree, &facts, 10, Some(&dir), |_| panic!("never open a path from CSV")).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
