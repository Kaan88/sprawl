//! F2: older versions side by side. Several installed versions of one toolchain, the older ones
//! offered for removal through the toolchain's own command (`rustup toolchain uninstall`,
//! `nvm uninstall`, `fnm uninstall`), never a folder delete. Never the default or active one,
//! never the newest, never one a project in the scan asks for (`rust-toolchain.toml`, `.nvmrc`,
//! `.node-version`), nor one rustup keeps for a folder (an override).

use std::path::{Path, PathBuf};

use sprawl_core::{format_size, Tree};

use crate::finding::{Action, Finding, Remedy, Risk, Source};
use crate::rules::ps_quote;

/// A version name Sprawl puts in a command: letters, digits, dots, dashes and underscores only.
fn plain(name: &str) -> bool {
    !name.is_empty() && name.len() <= 80 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn child(tree: &Tree, n: usize, name: &str) -> Option<usize> {
    tree.nodes[n].children.iter().copied().find(|&c| tree.nodes[c].name.eq_ignore_ascii_case(name))
}

fn dirs(tree: &Tree, n: usize) -> Vec<usize> {
    tree.nodes[n].children.iter().copied().filter(|&c| tree.nodes[c].is_dir && !tree.nodes[c].is_reparse_point() && tree.nodes[c].attributes & crate::dupes::NOT_LOCAL == 0).collect()
}

/// `1.99.1` / `v18.17.0` / `18` as numbers, for "newest"; None when it isn't a version.
fn numbers(v: &str) -> Option<Vec<u64>> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let parts: Option<Vec<u64>> = v.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

/// What projects in the scan ask for: (the file's folder, the version text).
#[derive(Default)]
pub struct Pins {
    pub rust: Vec<(PathBuf, String)>,
    pub node: Vec<(PathBuf, String)>,
    /// Android SDK packages a Gradle file asks for, as sdkmanager names them
    /// (`platforms;android-34`, `build-tools;34.0.0`, `ndk;28.2.13676358`).
    pub android: Vec<(PathBuf, String)>,
    pub rust_unknown: bool,
    pub node_unknown: bool,
    pub android_unknown: bool,
}

fn bounded_text(path: &Path, max: u64) -> Result<String, String> {
    use std::io::Read;
    if crate::exclusions::intersects(path, &crate::exclusions::configured()) { return Err("This file is excluded".into()); }
    let mut held = sprawl_core::local::file(path, max).map_err(|e| e.to_string())?;
    let mut text = String::new();
    held.file.read_to_string(&mut text).map_err(|e| e.to_string())?;
    Ok(text)
}

fn link_target(path: &Path) -> Option<PathBuf> {
    if crate::exclusions::intersects(path, &crate::exclusions::configured()) { return None; }
    let path = sprawl_core::local::lexical(path).ok()?;
    let _parent = sprawl_core::local::directory(path.parent()?).ok()?;
    let target = std::fs::read_link(&path).ok()?;
    let target = if target.is_absolute() { target } else { path.parent()?.join(target) };
    if crate::exclusions::intersects(&target, &crate::exclusions::configured()) { return None; }
    Some(sprawl_core::local::directory(&target).ok()?.path.clone())
}

/// The Android SDK packages one Gradle build file asks for: `compileSdk = 34` (or
/// `compileSdkVersion 34`, `compileSdk { version = release(36) }`), `buildToolsVersion "34.0.0"`,
/// `ndkVersion "28.2.13676358"`. Values set elsewhere (version catalogs) aren't seen.
pub fn gradle_wants(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let quoted = |rest: &str| -> Option<String> {
        let s = rest.find(['"', '\''])?;
        let rest = &rest[s + 1..];
        let e = rest.find(['"', '\''])?;
        Some(rest[..e].to_string())
    };
    let mut in_compile_block = false;
    for l in text.lines().map(str::trim) {
        if l.starts_with("//") {
            continue;
        }
        let compile = l.strip_prefix("compileSdkVersion").or_else(|| l.strip_prefix("compileSdk")).filter(|r| !r.starts_with(|c: char| c.is_ascii_alphabetic()));
        let level = |rest: &str| rest.split(|c: char| !c.is_ascii_digit()).find(|d| !d.is_empty()).map(str::to_string);
        if let Some(rest) = compile {
            match level(rest) {
                Some(n) => out.push(format!("platforms;android-{n}")),
                // the block form: the level is on a later line
                None => in_compile_block = rest.contains('{'),
            }
        } else if in_compile_block {
            if let Some(n) = level(l) {
                out.push(format!("platforms;android-{n}"));
                in_compile_block = false;
            } else if l.contains('}') {
                in_compile_block = false;
            }
        } else if let Some(rest) = l.strip_prefix("buildToolsVersion") {
            if let Some(v) = quoted(rest) {
                out.push(format!("build-tools;{v}"));
            }
        } else if let Some(rest) = l.strip_prefix("ndkVersion") {
            if let Some(v) = quoted(rest) {
                out.push(format!("ndk;{v}"));
            }
        }
    }
    out
}

/// Read the version files at the top of each project the engine found. They are a few bytes
/// each; an unreadable/invalid pin suppresses that toolchain's uninstall advice.
pub fn pins(tree: &Tree, projects: &[crate::projects::Project]) -> Pins {
    let mut p = Pins { rust_unknown: tree.errors != 0, node_unknown: tree.errors != 0, android_unknown: tree.errors != 0, ..Default::default() };
    for pr in projects {
        let dir = tree.path_of(pr.node);
        for (name, rust) in [("rust-toolchain.toml", true), ("rust-toolchain", true), (".nvmrc", false), (".node-version", false)] {
            if child(tree, pr.node, name).is_none() {
                continue;
            }
            let Ok(text) = bounded_text(&dir.join(name), 16 << 10) else {
                if rust { p.rust_unknown = true; } else { p.node_unknown = true; }
                continue;
            };
            let v = if name == "rust-toolchain.toml" {
                toml::from_str::<toml::Value>(&text).ok().and_then(|v| v.get("toolchain")?.get("channel")?.as_str().map(str::to_string))
            } else {
                text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string)
            };
            if let Some(v) = v.filter(|v| plain(v) && (rust || numbers(v).is_some())) {
                if rust { &mut p.rust } else { &mut p.node }.push((dir.clone(), v));
            } else if rust { p.rust_unknown = true; } else { p.node_unknown = true; }
        }
        // Gradle: the project's own build files and its modules' (one level down), 40 at most
        let mut gradle: Vec<PathBuf> = Vec::new();
        for n in std::iter::once(pr.node).chain(dirs(tree, pr.node)) {
            for name in ["build.gradle", "build.gradle.kts"] {
                if child(tree, n, name).is_some() {
                    if gradle.len() < 40 { gradle.push(tree.path_of(n).join(name)); } else { p.android_unknown = true; }
                }
            }
        }
        for g in gradle {
            let Ok(text) = bounded_text(&g, 256 << 10) else { p.android_unknown = true; continue };
            p.android.extend(gradle_wants(&text).into_iter().map(|w| (dir.clone(), w)));
        }
    }
    p
}

fn finding(tree: &Tree, n: usize, id: &str, owner: &str, title: String, evidence: Vec<String>, command: String) -> Finding {
    let mut f = Finding {
        path: tree.path_of(n),
        owner: Some(owner.into()),
        bytes: tree.nodes[n].size,
        risk: Risk::Amber,
        remedy: Remedy::Command,
        command: None,
        title,
        evidence,
        confidence: 0.8,
        source: Source::Rule(format!("versions.{id}")),
        allow_protected: false,
        trusted: true,
        builtin: true,
        on_path: false,
        action: None,
        admin: false,
        carved: Vec::new(),
        gain: None,
        forced_from: None,
    };
    f.set_action(Action::Tool { command }, false);
    f
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

/// Resolve the scope of the same uninstall command the rule generated. Bind the tool's
/// home explicitly; an uninstall must not retarget when the tool's defaults change.
pub(crate) fn cleanup_scope(id: &str, path: &Path, command: &str) -> Option<(Vec<PathBuf>, Vec<(&'static str, PathBuf)>, String)> {
    cleanup_scope_in(id, path, command, android_sdk)
}

fn cleanup_scope_in(id: &str, path: &Path, command: &str, sdk_root: impl FnOnce() -> Option<PathBuf>) -> Option<(Vec<PathBuf>, Vec<(&'static str, PathBuf)>, String)> {
    let name = path.file_name()?.to_str()?.to_string();
    let parent = path.parent()?;
    if !plain(&name) { return None; }
    let mut roots = vec![path.to_path_buf()];
    let mut run = command.to_owned();
    let mut env = Vec::new();
    match id {
        "versions.rust" => {
            let root = parent.parent()?;
            let (channel, target) = name.split_once('-')?;
            let target = if matches!(channel, "nightly" | "beta") { target.get(11..)? } else { target };
            if !parent.file_name()?.eq_ignore_ascii_case("toolchains")
                || command != format!("rustup toolchain uninstall {}", ps_quote(&name))
                // Only fully scoped names; rustup resolves partial names using its default host.
                || !target.split_once("-pc-windows-").is_some_and(|(arch, abi)|
                    matches!(arch, "i686" | "x86_64" | "aarch64" | "arm64ec") && matches!(abi, "msvc" | "gnu" | "gnullvm")) { return None; }
            // A legacy rustup layout can remove *all* toolchains during data migration.
            let settings = bounded_text(&root.join("settings.toml"), 256 << 10).ok()?;
            let settings = settings.parse::<toml::Table>().ok()?;
            if settings.get("version")?.as_str()? != "12" { return None; }
            roots.extend([root.join("update-hashes").join(&name), root.join("tmp")]);
            env.push(("RUSTUP_HOME", root.to_path_buf()));
        }
        "versions.nvm" | "versions.fnm" => {
            let ver = name.strip_prefix('v')?;
            if ver.split('.').count() != 3 || numbers(ver)?.len() != 3 { return None; }
            if id == "versions.nvm" {
                if command != format!("nvm uninstall {}", ps_quote(ver)) { return None; }
                let text = bounded_text(&parent.join("settings.txt"), 64 << 10).ok()?;
                let setting = |key: &str| {
                    let mut values = text.lines().filter_map(|l| l.split_once(':'))
                        .filter(|(k, _)| k.trim().eq_ignore_ascii_case(key)).map(|(_, v)| PathBuf::from(v.trim()));
                    let value = values.next()?;
                    (value.is_absolute() && values.next().is_none()).then_some(value)
                };
                let root = setting("root")?;
                let link = setting("path")?;
                if !crate::clean::same_path(parent, &root) { return None; }
                // nvm can also remove its active-version link if the selection changed.
                roots.push(link.clone());
                env.extend([("NVM_HOME", parent.to_path_buf()), ("NVM_SYMLINK", link)]);
            } else {
                if !parent.file_name()?.eq_ignore_ascii_case("node-versions")
                    || command != format!("fnm uninstall {}", ps_quote(ver)) { return None; }
                let root = parent.parent()?;
                roots.push(root.join("aliases"));
                env.push(("FNM_DIR", root.to_path_buf()));
            }
        }
        "versions.android" => {
            let sdk = sdk_root()?;
            let tool = sdkmanager(&sdk)?;
            if !crate::finding::within(path, &sdk) { return None; }
            let parts: Vec<_> = path.iter().skip(sdk.iter().count()).map(|s| s.to_str().filter(|s| plain(s))).collect::<Option<_>>()?;
            match parts.as_slice() {
                ["build-tools" | "ndk", ver] if numbers(ver).is_some() => {}
                ["platforms", ver] if ver.strip_prefix("android-").and_then(numbers).is_some() => {}
                ["system-images", api, _, _] if api.strip_prefix("android-").and_then(numbers).is_some() => {}
                _ => return None,
            }
            let pkg = parts.join(";");
            if command != format!("& {} --uninstall {}", ps_quote(tool.to_str()?), ps_quote(&pkg)) { return None; }
            // SDK Manager's temporary operation data is also inside this SDK.
            roots.push(sdk.join(".temp"));
            run.push_str(r#" "--sdk_root=$env:SPRAWL_TOOL_DIR""#);
            env.push(("SPRAWL_TOOL_DIR", sdk));
        }
        _ => return None,
    }
    Some((roots, env, run))
}

/// Everything F2 finds in `tree` (empty when the toolchains' folders aren't in it). `apps`
/// are the installed apps, for the versions only their uninstaller removes (.NET, Python).
pub fn find(tree: &Tree, projects: &[crate::projects::Project], apps: &[crate::facts::App]) -> Vec<Finding> {
    if tree.method == sprawl_core::ScanMethod::Csv { return Vec::new(); }
    let pins = pins(tree, projects);
    let mut out = rust(tree, &pins);
    out.extend(node(tree, &pins));
    out.extend(android(tree, &pins));
    // handed to Installed apps (their own uninstallers), never removed by Sprawl
    let h = handoff_pins(tree, projects);
    out.extend(python(tree, apps, &h));
    out.extend(dotnet(tree, apps, &h));
    out
}

/// The Android SDK's folder: ANDROID_HOME, ANDROID_SDK_ROOT, or Android Studio's default.
fn android_sdk() -> Option<PathBuf> {
    ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .find_map(|v| std::env::var_os(v).map(PathBuf::from))
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Android").join("Sdk")))
}

#[cfg(test)]
mod cleanup_scope_tests {
    use super::*;

    #[test]
    fn version_removal_checks_the_named_home_and_its_auxiliary_paths() {
        let tmp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-version-scope").unwrap();
        let rust = tmp.path().join("rustup");
        std::fs::create_dir(&rust).unwrap();
        let name = "1.91.0-x86_64-pc-windows-msvc";
        let rust_scope = |name| cleanup_scope("versions.rust", &rust.join("toolchains").join(name), &format!("rustup toolchain uninstall {}", ps_quote(name)));
        std::fs::write(rust.join("settings.toml"), "version = '12'\n").unwrap();
        let (paths, env, _) = rust_scope(name).unwrap();
        assert_eq!(paths, [rust.join("toolchains").join(name), rust.join("update-hashes").join(name), rust.join("tmp")]);
        assert_eq!(env, [("RUSTUP_HOME", rust.clone())]);
        assert!(rust_scope("1.91.0").is_none(), "rustup may resolve a partial name to a different directory");
        std::fs::write(rust.join("settings.toml"), "version = '2'\n").unwrap();
        assert!(rust_scope(name).is_none(), "legacy metadata migration can remove all toolchains");

        let nvm = tmp.path().join("nvm");
        std::fs::create_dir(&nvm).unwrap();
        let link = tmp.path().join("nodejs");
        let nvm_scope = || cleanup_scope("versions.nvm", &nvm.join("v18.1.0"), "nvm uninstall '18.1.0'");
        std::fs::write(nvm.join("settings.txt"), format!("root: {}\npath: {}\n", nvm.display(), link.display())).unwrap();
        let (paths, env, _) = nvm_scope().unwrap();
        assert_eq!(paths, [nvm.join("v18.1.0"), link.clone()]);
        assert_eq!(env, [("NVM_HOME", nvm.clone()), ("NVM_SYMLINK", link)]);
        std::fs::write(nvm.join("settings.txt"), format!("root: {}\npath: {}\n", tmp.path().display(), nvm.display())).unwrap();
        assert!(nvm_scope().is_none());

        let fnm = tmp.path().join("fnm");
        let version = fnm.join(r"node-versions\v18.1.0");
        let (paths, env, _) = cleanup_scope("versions.fnm", &version, "fnm uninstall '18.1.0'").unwrap();
        assert_eq!(paths, [version, fnm.join("aliases")]);
        assert_eq!(env, [("FNM_DIR", fnm)]);
    }

    #[test]
    fn android_cleanup_binds_the_sdk_and_checks_the_generated_package_id() {
        let tmp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-android-scope").unwrap();
        let sdk = tmp.path().join("Sdk");
        let bin = sdk.join(r"cmdline-tools\latest\bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("sdkmanager.bat");
        std::fs::write(&exe, b"fixture, not run").unwrap();
        let package = sdk.join(r"system-images\android-35\google_apis\x86_64");
        let cmd = format!("& {} --uninstall 'system-images;android-35;google_apis;x86_64'", ps_quote(exe.to_str().unwrap()));
        let (paths, env, run) = cleanup_scope_in("versions.android", &package, &cmd, || Some(sdk.clone())).unwrap();
        assert_eq!(paths, [package.clone(), sdk.join(".temp")]);
        assert_eq!(env, [("SPRAWL_TOOL_DIR", sdk.clone())]);
        assert!(run.ends_with(r#" "--sdk_root=$env:SPRAWL_TOOL_DIR""#));
        assert!(cleanup_scope_in("versions.android", &package, &cmd.replace("android-35", "android-34"), || Some(sdk)).is_none());
    }
}

/// The system images the emulators use: each AVD's config.ini names its image
/// (`image.sysdir.1=system-images\android-33\google_apis\x86_64\`), as an sdkmanager package.
fn avd_images() -> Option<Vec<String>> {
    let mut roots = Vec::new();
    if let Some(path) = std::env::var_os("ANDROID_AVD_HOME") { roots.push(PathBuf::from(path)); }
    for variable in ["ANDROID_EMULATOR_HOME", "ANDROID_USER_HOME"] {
        if let Some(path) = std::env::var_os(variable) { roots.push(PathBuf::from(path).join("avd")); }
    }
    if let Some(path) = std::env::var_os("ANDROID_SDK_HOME") { roots.push(PathBuf::from(path).join(".android/avd")); }
    if let Some(path) = home() { roots.push(path.join(".android/avd")); }
    if roots.is_empty() { return None; }
    roots.sort(); roots.dedup();
    let mut budget = 200;
    let mut out = Vec::new();
    for root in roots { out.extend(avd_images_in(&root, &mut budget)?); }
    out.sort(); out.dedup();
    Some(out)
}

fn ini_values(text: &str) -> Option<std::collections::BTreeMap<String, String>> {
    let mut values = std::collections::BTreeMap::new();
    for line in text.trim_start_matches('\u{feff}').lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(['#', ';'])) {
        let (key, value) = line.split_once('=')?;
        if key.trim().is_empty() || values.insert(key.trim().to_string(), value.trim().to_string()).is_some() { return None; }
    }
    Some(values)
}

fn avd_config_images(path: &Path) -> Option<Vec<String>> {
    let values = ini_values(&bounded_text(&path.join("config.ini"), 64 << 10).ok()?)?;
    let mut images = Vec::new();
    for (_, value) in values.iter().filter(|(key, _)| key.starts_with("image.sysdir")) {
        let parts: Vec<&str> = value.split(['\\', '/']).filter(|part| !part.is_empty()).collect();
        let start = parts.iter().position(|part| part.eq_ignore_ascii_case("system-images"))?;
        let package = &parts[start..];
        if package.len() != 4 || !package.iter().all(|part| plain(part)) { return None; }
        images.push(package.join(";"));
    }
    (!images.is_empty()).then_some(images)
}

fn avd_images_in(dir: &Path, budget: &mut usize) -> Option<Vec<String>> {
    if crate::exclusions::intersects(dir, &crate::exclusions::configured()) { return None; }
    let pinned = match sprawl_core::local::directory(dir) {
        Ok(pinned) => pinned,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(Vec::new()),
        Err(_) => return None,
    };
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&pinned.path).ok()? {
        *budget = budget.checked_sub(1)?;
        let entry = entry.ok()?;
        let path = entry.path();
        let leaf = sprawl_core::local::scope(&path).ok()?;
        if leaf.metadata().ok()?.is_dir() {
            if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("avd")) { out.extend(avd_config_images(&path)?); }
        } else if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("ini")) {
            let values = ini_values(&bounded_text(&path, 64 << 10).ok()?)?;
            let target = if let Some(path) = values.get("path") { PathBuf::from(path) }
                else if let Some(relative) = values.get("path.rel") {
                    let relative = Path::new(relative);
                    if relative.is_absolute() || relative.components().any(|part| matches!(part, std::path::Component::ParentDir)) { return None; }
                    pinned.path.parent()?.join(relative)
                } else { return None; };
            if !target.is_absolute() { return None; }
            // bounded_text pins the full target chain and refuses remote, redirected, cloud
            // and excluded targets before it reads a relocated AVD's configuration.
            out.extend(avd_config_images(&target)?);
        }
    }
    Some(out)
}

/// sdkmanager's own script, from the newest command-line tools.
fn sdkmanager(sdk: &Path) -> Option<PathBuf> {
    [sdk.join(r"cmdline-tools\latest\bin\sdkmanager.bat"), sdk.join(r"tools\bin\sdkmanager.bat")].into_iter().find(|p| p.is_file())
}

/// Older Android SDK packages: build-tools, platforms, system images and NDKs. Never the newest
/// of each, one a Gradle file in the scan asks for, or an image an emulator uses.
fn android(tree: &Tree, pins: &Pins) -> Vec<Finding> {
    if pins.android_unknown { return Vec::new(); }
    let Some(sdk) = android_sdk() else { return Vec::new() };
    let Some(root) = crate::find_path(tree, &sdk) else { return Vec::new() };
    let Some(tool) = sdkmanager(&sdk) else { return Vec::new() };
    let mut kept: Vec<String> = pins.android.iter().map(|(_, w)| w.to_lowercase()).collect();
    let images = avd_images();
    if let Some(images) = &images { kept.extend(images.iter().map(|w| w.to_lowercase())); }
    android_from(tree, root, &tool, &kept, images.is_some())
}

fn android_from(tree: &Tree, root: usize, tool: &Path, kept: &[String], images_known: bool) -> Vec<Finding> {
    // (package, node, version numbers for "newest", family)
    let mut pkgs: Vec<(String, usize, Vec<u64>, &str)> = Vec::new();
    let level = |name: &str| name.strip_prefix("android-").and_then(numbers);
    if let Some(bt) = child(tree, root, "build-tools") {
        for n in dirs(tree, bt) {
            let name = &tree.nodes[n].name;
            if let Some(v) = numbers(name) {
                pkgs.push((format!("build-tools;{name}"), n, v, "build-tools"));
            }
        }
    }
    if let Some(pl) = child(tree, root, "platforms") {
        for n in dirs(tree, pl) {
            let name = &tree.nodes[n].name;
            if let Some(v) = level(name) {
                pkgs.push((format!("platforms;{name}"), n, v, "platforms"));
            }
        }
    }
    if let Some(nd) = child(tree, root, "ndk") {
        for n in dirs(tree, nd) {
            let name = &tree.nodes[n].name;
            if let Some(v) = numbers(name) {
                pkgs.push((format!("ndk;{name}"), n, v, "ndk"));
            }
        }
    }
    // system-images\android-33\google_apis\x86_64: one package per ABI folder
    if let Some(si) = child(tree, root, "system-images") {
        for api in dirs(tree, si) {
            let Some(v) = level(&tree.nodes[api].name) else { continue };
            for tag in dirs(tree, api) {
                for abi in dirs(tree, tag) {
                    let pkg = format!("system-images;{};{};{}", tree.nodes[api].name, tree.nodes[tag].name, tree.nodes[abi].name);
                    pkgs.push((pkg, abi, v.clone(), "system-images"));
                }
            }
        }
    }
    let mut out = Vec::new();
    for (pkg, n, v, family) in &pkgs {
        if *family == "system-images" && !images_known { continue; }
        let newest = pkgs.iter().filter(|p| p.3 == *family).map(|p| &p.2).max();
        if Some(v) == newest || kept.iter().any(|k| *k == pkg.to_lowercase()) || !pkg.split(';').all(plain) {
            continue;
        }
        let what = match *family {
            "build-tools" => "Android build tools",
            "platforms" => "Android platform",
            "ndk" => "Android NDK",
            _ => "Android emulator image",
        };
        let ver = pkg.split(';').skip(1).collect::<Vec<_>>().join(" ");
        let newer: Vec<String> = pkgs.iter().filter(|p| p.3 == *family && p.2 > *v).map(|p| p.0.split(';').skip(1).collect::<Vec<_>>().join(" ")).collect();
        out.push(finding(
            tree,
            *n,
            "android",
            "Android SDK",
            format!("{what} {ver}"),
            vec![
                format!(
                    "An older {} beside {}. {}",
                    what.trim_start_matches("Android ").to_lowercase(),
                    newer.join(", "),
                    if *family == "system-images" { "No emulator (AVD) of yours uses it." } else { "No Gradle file in this scan asks for it." }
                ),
                format!("Tick it and Clean up, and Sprawl runs the SDK's own 'sdkmanager --uninstall \"{pkg}\"' ({}).", format_size(tree.nodes[*n].size)),
                if *family == "system-images" {
                    "An emulator made later with this image needs it downloaded again (Android Studio > SDK Manager).".into()
                } else {
                    "A project outside this scan that needs it gets it again from Android Studio or Gradle on its next build.".into()
                },
            ],
            format!("& {} --uninstall {}", ps_quote(&tool.display().to_string()), ps_quote(pkg)),
        ));
    }
    out
}

/// rustup's toolchains: a numbered (`1.99.1-...`) or dated (`nightly-2025-01-01-...`)
/// toolchain that isn't the default, an override or a project's pin. Channels (`stable`,
/// `beta`, `nightly`) update in place and are left alone.
fn rust(tree: &Tree, pins: &Pins) -> Vec<Finding> {
    let rustup = std::env::var_os("RUSTUP_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".rustup")));
    let Some(rustup) = rustup else { return Vec::new() };
    let Some(tc) = crate::find_path(tree, &rustup.join("toolchains")) else { return Vec::new() };
    let Ok(settings) = bounded_text(&rustup.join("settings.toml"), 256 << 10) else { return Vec::new(); };
    let mut found = rust_from(tree, tc, &settings, pins);
    if let Some(active) = std::env::var_os("RUSTUP_TOOLCHAIN") {
        let Some(active) = active.to_str().filter(|v| plain(v)) else { return Vec::new(); };
        if !dirs(tree, tc).iter().any(|&n| tree.nodes[n].name == active || tree.nodes[n].name.starts_with(&format!("{active}-"))) { return Vec::new(); }
        found.retain(|f| !f.path.file_name().is_some_and(|name| name.to_string_lossy() == active || name.to_string_lossy().starts_with(&format!("{active}-"))));
    }
    found
}

fn rust_from(tree: &Tree, tc: usize, settings: &str, pins: &Pins) -> Vec<Finding> {
    if pins.rust_unknown { return Vec::new(); }
    #[derive(serde::Deserialize)]
    struct Settings { default_toolchain: String, #[serde(default)] overrides: std::collections::BTreeMap<String, String> }
    let Ok(settings) = toml::from_str::<Settings>(settings) else { return Vec::new(); };
    let mut kept = vec![settings.default_toolchain];
    kept.extend(settings.overrides.into_values());
    if kept.iter().any(|v| !plain(v)) { return Vec::new(); }
    let names: Vec<(usize, String)> = dirs(tree, tc).into_iter().map(|n| (n, tree.nodes[n].name.clone())).collect();
    if names.len() < 2 || !names.iter().any(|(_, name)| name == &kept[0] || name.starts_with(&format!("{}-", kept[0]))) { return Vec::new(); }
    let all: Vec<&str> = names.iter().map(|(_, s)| s.as_str()).collect();
    let mut out = Vec::new();
    for (n, name) in &names {
        let channel = name.split('-').next().unwrap_or("");
        let numbered = channel.starts_with(|c: char| c.is_ascii_digit()) && numbers(channel).is_some();
        let dated = matches!(channel, "nightly" | "beta") && name.split('-').nth(1).is_some_and(|y| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()));
        if !(numbered || dated) || !plain(name) {
            continue;
        }
        let matches = |want: &str| name == want || name.starts_with(&format!("{want}-"));
        if kept.iter().any(|k| matches(k)) {
            continue;
        }
        if pins.rust.iter().any(|(_, v)| matches(v)) {
            continue;
        }
        // Keep the newest numbered toolchain for each target, and the newest dated
        // toolchain of each channel/target, even when the default is a rolling channel.
        if numbered {
            let target = name.strip_prefix(channel).unwrap_or("");
            let version = numbers(channel).unwrap();
            if !names.iter().any(|(_, other)| {
                let c = other.split('-').next().unwrap_or("");
                other.strip_prefix(c) == Some(target) && numbers(c).is_some_and(|v| v > version)
            }) { continue; }
        } else if dated {
            let target = name.get(channel.len() + 11..).unwrap_or("");
            if !names.iter().any(|(_, other)| other.starts_with(&format!("{channel}-"))
                && other.get(channel.len() + 11..) == Some(target) && other > name) { continue; }
        }
        let others: Vec<&str> = all.iter().copied().filter(|o| o != name).collect();
        out.push(finding(
            tree,
            *n,
            "rust",
            "Rust",
            format!("Rust toolchain {channel}{}", if dated { format!(" {}", name.splitn(3, '-').nth(1).unwrap_or("")) } else { String::new() }),
            vec![
                format!("An older Rust toolchain beside {}. Nothing in this scan asks for it: no rust-toolchain file names it and it isn't your default.", others.join(", ")),
                format!("Tick it and Clean up, and Sprawl runs rustup's own 'rustup toolchain uninstall {name}' ({}). Rust stays installed.", format_size(tree.nodes[*n].size)),
                "A project outside this scan that asks for it gets it back from rustup on its next build.".into(),
            ],
            format!("rustup toolchain uninstall {}", ps_quote(name)),
        ));
    }
    out
}

/// Node versions kept by nvm for Windows (`%APPDATA%\nvm\v18.17.0`) and fnm
/// (`%APPDATA%\fnm\node-versions\v18.17.0`): not the one in use, not the newest, not a pin.
fn node(tree: &Tree, pins: &Pins) -> Vec<Finding> {
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    let mut out = Vec::new();
    // nvm: the version in use is where its link (NVM_SYMLINK) points
    let nvm_home = std::env::var_os("NVM_HOME").map(PathBuf::from).or_else(|| appdata.as_ref().map(|a| a.join("nvm")));
    if let Some(root) = nvm_home.as_ref().and_then(|h| crate::find_path(tree, h)) {
        let link = std::env::var_os("NVM_SYMLINK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_default()).join("nodejs"));
        let active = link_target(&link);
        out.extend(node_from(tree, root, active.as_deref(), pins, "nvm", "nvm for Windows"));
    }
    // fnm: the default is the alias link aliases\default
    let fnm_dir = std::env::var_os("FNM_DIR").map(PathBuf::from).or_else(|| appdata.as_ref().map(|a| a.join("fnm")));
    if let Some(dir) = fnm_dir {
        if let Some(root) = crate::find_path(tree, &dir.join("node-versions")) {
            let default = link_target(&dir.join("aliases").join("default"));
            let active = std::env::var_os("FNM_MULTISHELL_PATH").and_then(|p| link_target(Path::new(&p)));
            if let (Some(default), Some(active)) = (default, active) {
                let Some(n) = dirs(tree, root).into_iter().find(|&n| crate::finding::within(&default, &tree.path_of(n))) else { return out; };
                let mut scoped = Pins { node: pins.node.clone(), node_unknown: pins.node_unknown, ..Default::default() };
                scoped.node.push((dir.clone(), tree.nodes[n].name.clone()));
                out.extend(node_from(tree, root, Some(&active), &scoped, "fnm", "fnm"));
            }
        }
    }
    out
}

fn node_from(tree: &Tree, root: usize, active: Option<&Path>, pins: &Pins, tool: &str, tool_name: &str) -> Vec<Finding> {
    if pins.node_unknown || active.is_none() { return Vec::new(); }
    let versions: Vec<(usize, String, Vec<u64>)> = dirs(tree, root)
        .into_iter()
        .filter_map(|n| {
            let name = tree.nodes[n].name.clone();
            let v = numbers(&name)?;
            (name.starts_with(['v', 'V']) && plain(&name)).then_some((n, name, v))
        })
        .collect();
    if versions.len() < 2 {
        return Vec::new();
    }
    let newest = versions.iter().map(|v| &v.2).max().cloned().unwrap_or_default();
    // the active one: the link names its folder (or a folder inside it)
    let is_active = |n: usize| active.is_some_and(|a| crate::finding::within(a, &tree.path_of(n)));
    if !versions.iter().any(|(n, _, _)| is_active(*n)) { return Vec::new(); }
    let pinned = |v: &[u64]| {
        pins.node.iter().any(|(_, p)| {
            let Some(want) = numbers(p) else { return false };
            // "18" pins every 18.x; "18.17.0" only that one
            v.len() >= want.len() && v[..want.len()] == want[..]
        })
    };
    let all: Vec<&str> = versions.iter().map(|v| v.1.as_str()).collect();
    let mut out = Vec::new();
    for (n, name, v) in &versions {
        if *v == newest || is_active(*n) || pinned(v) {
            continue;
        }
        let bare = name.trim_start_matches(['v', 'V']);
        let others: Vec<&str> = all.iter().copied().filter(|o| o != name).collect();
        out.push(finding(
            tree,
            *n,
            tool,
            "Node.js",
            format!("Node.js {name} ({tool_name})"),
            vec![
                format!("An older Node.js beside {}. It isn't the one in use, and no .nvmrc or .node-version in this scan asks for it.", others.join(", ")),
                format!("Tick it and Clean up, and Sprawl runs '{tool} uninstall {bare}' ({}). Its global packages go with it.", format_size(tree.nodes[*n].size)),
            ],
            format!("{tool} uninstall {}", ps_quote(bare)),
        ));
    }
    out
}

/// Installed apps whose uninstall entry names no folder but which F2 hands off: the app list
/// keeps these (and only these) without a folder.
pub const FOLDERLESS: [&str; 5] = ["Python 3.", "Microsoft .NET SDK ", "Microsoft .NET Runtime - ", "Microsoft Windows Desktop Runtime - ", "Microsoft ASP.NET Core "];

/// The version in an installed app's name: `Python 3.12.7 (64-bit)` -> [3, 12, 7].
fn version_in(name: &str) -> Option<Vec<u64>> {
    name.split_whitespace().find_map(|w| numbers(w.trim_matches(['(', ')'])).filter(|v| v.len() >= 2))
}

fn handoff(path: PathBuf, bytes: u64, app: &crate::facts::App, title: String, mut evidence: Vec<String>) -> Finding {
    evidence.push(format!("To remove it: {}", app.how_to_remove));
    Finding {
        path,
        owner: Some(app.name.clone()),
        bytes,
        risk: Risk::Amber,
        remedy: Remedy::Uninstall,
        command: None,
        title,
        evidence,
        confidence: 0.8,
        source: Source::Rule("versions.handoff".into()),
        allow_protected: false,
        trusted: true,
        builtin: true,
        on_path: false,
        action: None,
        admin: false,
        carved: Vec::new(),
        gain: None,
        forced_from: None,
    }
}

/// What projects in the scan ask of Python and .NET: `.python-version`, the base Python of each
/// virtual environment (`pyvenv.cfg`'s `home`), and `global.json`'s SDK version.
#[derive(Default)]
pub struct HandoffPins {
    pub python: Vec<String>,
    pub venv_homes: Vec<PathBuf>,
    pub dotnet_sdk: Vec<String>,
}

pub fn handoff_pins(tree: &Tree, projects: &[crate::projects::Project]) -> HandoffPins {
    let mut p = HandoffPins::default();
    let read = |n: usize, name: &str| -> Option<String> {
        child(tree, n, name)?;
        bounded_text(&tree.path_of(n).join(name), 64 << 10).ok()
    };
    for pr in projects {
        if let Some(t) = read(pr.node, ".python-version") {
            p.python.extend(t.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string));
        }
        if let Some(t) = read(pr.node, "global.json") {
            // "sdk": { "version": "8.0.100" }: the first "version" after "sdk"
            if let Some(i) = t.find("\"sdk\"") {
                let rest = &t[i..];
                if let Some(j) = rest.find("\"version\"") {
                    let v: String = rest[j + 9..].chars().skip_while(|c| *c != '"').skip(1).take_while(|c| *c != '"').collect();
                    if !v.is_empty() {
                        p.dotnet_sdk.push(v);
                    }
                }
            }
        }
        // a virtual environment in the project or one level down (.venv, venv, env...)
        for n in std::iter::once(pr.node).chain(dirs(tree, pr.node)) {
            if let Some(t) = read(n, "pyvenv.cfg") {
                for l in t.lines() {
                    if let Some((k, v)) = l.split_once('=') {
                        if k.trim() == "home" {
                            p.venv_homes.push(PathBuf::from(v.trim()));
                        }
                    }
                }
            }
        }
    }
    p
}

/// Older Pythons from python.org's installer (`Python 3.12.7 (64-bit)`), handed to Installed
/// apps: not the newest, not one a `.python-version` or a virtual environment in the scan uses.
fn python(tree: &Tree, apps: &[crate::facts::App], pins: &HandoffPins) -> Vec<Finding> {
    python_from(tree, apps, pins, &python_homes())
}

/// Where each python.org Python is installed, as it registers itself (PEP 514):
/// `Software\Python\PythonCore\3.12\InstallPath`, for this account and for all users. The tag
/// is `3.12`, or `3.12-32` for a 32-bit one.
fn python_homes() -> Vec<(String, PathBuf)> {
    use crate::reg::{Key, View, HKCU, HKLM};
    let mut out = Vec::new();
    for (root, view) in [(HKCU, View::Native), (HKLM, View::Native), (HKLM, View::Bit32)] {
        let Some(core) = Key::open(root, r"Software\Python\PythonCore", view) else { continue };
        for tag in core.subkeys() {
            if let Some(dir) = core.sub(&tag).and_then(|k| k.sub("InstallPath")).and_then(|k| k.string("")) {
                out.push((tag, PathBuf::from(dir.trim_end_matches('\\'))));
            }
        }
    }
    out
}

fn python_from(tree: &Tree, apps: &[crate::facts::App], pins: &HandoffPins, homes: &[(String, PathBuf)]) -> Vec<Finding> {
    let pythons: Vec<(&crate::facts::App, Vec<u64>, usize)> = apps
        .iter()
        .filter(|a| a.kind == crate::facts::AppKind::Installed && a.name.starts_with("Python 3.") && (a.name.ends_with("(64-bit)") || a.name.ends_with("(32-bit)")))
        .filter_map(|a| {
            let v = version_in(&a.name)?;
            // its uninstall entry names no folder: its registration does
            let tag = format!("{}.{}{}", v[0], v[1], if a.name.ends_with("(32-bit)") { "-32" } else { "" });
            let dir = a.locations.first().cloned().or_else(|| homes.iter().find(|(t, _)| *t == tag).map(|(_, d)| d.clone()))?;
            let n = crate::find_path(tree, &dir)?;
            Some((a, v, n))
        })
        .collect();
    let newest = pythons.iter().map(|p| p.1[..2].to_vec()).max();
    let mut out = Vec::new();
    for (app, v, n) in &pythons {
        let minor = v[..2].to_vec();
        if Some(&minor) == newest.as_ref() {
            continue;
        }
        // "3.12" or "3.12.7" pins this minor
        let pinned = pins.python.iter().filter_map(|p| numbers(p)).any(|want| want.len() >= 2 && want[..2] == minor[..]);
        let folder = tree.path_of(*n);
        let venv = pins.venv_homes.iter().any(|h| crate::finding::within(h, &folder));
        if pinned || venv {
            continue;
        }
        let ver = minor.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
        out.push(handoff(
            folder,
            tree.nodes[*n].size,
            app,
            format!("Older Python {ver}"),
            vec![
                format!("A newer Python is installed beside it. No .python-version and no virtual environment in this scan uses {ver}."),
                format!("Scripts started with 'py -{ver}', and virtual environments outside this scan made with it, stop working without it."),
            ],
        ));
    }
    out
}

/// Older .NET pieces, handed to Installed apps. A runtime only when a newer patch of the same
/// major.minor is installed (apps move to the newest patch by themselves); another major is
/// never offered (an app made for .NET 6 doesn't run on 8). An SDK when a newer SDK is
/// installed and no `global.json` in the scan asks for it (a newer SDK builds older projects).
fn dotnet(tree: &Tree, apps: &[crate::facts::App], pins: &HandoffPins) -> Vec<Finding> {
    let Some(pf) = std::env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join("dotnet")) else { return Vec::new() };
    dotnet_from(tree, apps, pins, &pf)
}

fn dotnet_from(tree: &Tree, apps: &[crate::facts::App], pins: &HandoffPins, root: &Path) -> Vec<Finding> {
    // (product folder under dotnet, app name start)
    const PRODUCTS: [(&str, &str); 4] = [
        ("sdk", "Microsoft .NET SDK "),
        (r"shared\Microsoft.NETCore.App", "Microsoft .NET Runtime - "),
        (r"shared\Microsoft.WindowsDesktop.App", "Microsoft Windows Desktop Runtime - "),
        (r"shared\Microsoft.AspNetCore.App", "Microsoft ASP.NET Core "),
    ];
    let mut out = Vec::new();
    for (folder, prefix) in PRODUCTS {
        let sdk = folder == "sdk";
        // (app, version, folder node)
        let have: Vec<(&crate::facts::App, Vec<u64>, usize)> = apps
            .iter()
            .filter(|a| a.kind == crate::facts::AppKind::Installed && a.name.starts_with(prefix) && a.name.contains("(x64)"))
            .filter_map(|a| {
                let v = version_in(&a.name[prefix.len()..])?;
                let ver = v.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
                let n = crate::find_path(tree, &root.join(folder).join(&ver))?;
                Some((a, v, n))
            })
            .collect();
        for (app, v, n) in &have {
            let ver = v.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
            let newer = if sdk {
                have.iter().any(|o| o.1 > *v)
            } else {
                have.iter().any(|o| o.1.len() >= 2 && v.len() >= 2 && o.1[..2] == v[..2] && o.1 > *v)
            };
            if !newer || (sdk && pins.dotnet_sdk.iter().any(|p| *p == ver)) {
                continue;
            }
            let what = if sdk { ".NET SDK".to_string() } else { prefix.trim_end_matches([' ', '-']).trim_start_matches("Microsoft ").to_string() };
            out.push(handoff(
                tree.path_of(*n),
                tree.nodes[*n].size,
                app,
                format!("Older {what} {ver}"),
                vec![if sdk {
                    "A newer .NET SDK is installed; it builds projects made for older .NET too. No global.json in this scan asks for this one.".into()
                } else {
                    format!("A newer {}.{} patch is installed; apps made for {}.{} use the newest patch by themselves.", v[0], v[1], v[0], v[1])
                }],
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use sprawl_core::ROOT;
    use super::*;

    fn tree_with(tag: &str, dirs: &[&str]) -> (PathBuf, Tree) {
        let base = std::env::temp_dir().join(format!("sprawl-versions-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for d in dirs {
            let p = base.join(d);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("f.bin"), vec![0u8; 1000]).unwrap();
        }
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        (base, tree)
    }

    #[test]
    fn rust_keeps_the_default_channels_overrides_and_pins() {
        let (base, tree) = tree_with("rust", &[
            "stable-x86_64-pc-windows-msvc",
            "1.99.1-x86_64-pc-windows-msvc",
            "1.100.0-x86_64-pc-windows-msvc",
            "1.80.0-x86_64-pc-windows-msvc",
            "1.70.0-x86_64-pc-windows-msvc",
            "nightly-2025-01-01-x86_64-pc-windows-msvc",
            "nightly-2026-01-01-x86_64-pc-windows-msvc",
            "nightly-x86_64-pc-windows-msvc",
        ]);
        let settings = "default_toolchain = \"stable-x86_64-pc-windows-msvc\"\n[overrides]\n\"C:\\\\work\\\\old\" = \"1.80.0-x86_64-pc-windows-msvc\"\n";
        let pins = Pins { rust: vec![(PathBuf::from(r"C:\p"), "1.70.0".into())], ..Default::default() };
        let f = rust_from(&tree, ROOT, settings, &pins);
        let names: Vec<String> = f.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.contains(&"1.99.1-x86_64-pc-windows-msvc".to_string()));
        assert!(names.contains(&"nightly-2025-01-01-x86_64-pc-windows-msvc".to_string()));
        let one = f.iter().find(|f| f.path.ends_with("1.99.1-x86_64-pc-windows-msvc")).unwrap();
        assert_eq!(one.risk, Risk::Amber);
        assert!(matches!(&one.action, Some(Action::Tool { command }) if command == "rustup toolchain uninstall '1.99.1-x86_64-pc-windows-msvc'"));
        assert!(crate::clean::action(one).is_some());
        for unknown in ["", "not valid TOML = [", "default_toolchain = 'missing'", "default_toolchain = 42", "default_toolchain = 'stable-x86_64-pc-windows-msvc'\n[overrides]\nx = 42"] {
            assert!(rust_from(&tree, ROOT, unknown, &pins).is_empty(), "unknown owner state: {unknown}");
        }
        let unknown_pins = Pins { rust_unknown: true, ..Default::default() };
        assert!(rust_from(&tree, ROOT, settings, &unknown_pins).is_empty());
        let (sole, only) = tree_with("rust-sole", &["1.70.0-x86_64-pc-windows-msvc"]);
        assert!(rust_from(&only, ROOT, "default_toolchain = '1.70.0-x86_64-pc-windows-msvc'", &Pins::default()).is_empty());
        std::fs::remove_dir_all(sole).unwrap();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn node_keeps_the_newest_the_active_and_pins() {
        let (base, tree) = tree_with("node", &["v16.20.0", "v18.17.0", "v18.19.1", "v20.11.0", "v14.0.0", "not-a-version"]);
        let active = base.join("v14.0.0");
        let pins = Pins { node: vec![(PathBuf::from(r"C:\p"), "18".into())], ..Default::default() };
        let f = node_from(&tree, ROOT, Some(&active), &pins, "nvm", "nvm for Windows");
        let names: Vec<String> = f.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        // v20 newest, v14 active, both v18 pinned by "18": only v16 is offered
        assert_eq!(names, vec!["v16.20.0".to_string()]);
        assert!(matches!(&f[0].action, Some(Action::Tool { command }) if command == "nvm uninstall '16.20.0'"));
        assert!(node_from(&tree, ROOT, None, &pins, "nvm", "nvm").is_empty());
        assert!(node_from(&tree, ROOT, Some(&base.join("unrecognised")), &pins, "nvm", "nvm").is_empty());
        assert!(node_from(&tree, ROOT, Some(&active), &Pins { node_unknown: true, ..Default::default() }, "nvm", "nvm").is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A project's rust-toolchain.toml and .nvmrc, found by the engine's own project finder.
    #[test]
    fn a_project_in_the_scan_pins_its_versions() {
        // not in %TEMP%: the project finder leaves everything under AppData alone
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join(format!("test-versions-pins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let p = base.join("app");
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Cargo.toml"), "[package]\nname = \"app\"\n").unwrap();
        std::fs::write(p.join("src").join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(p.join("rust-toolchain.toml"), "[toolchain]\nchannel = \"1.99.1\"\ncomponents = [\"clippy\"]\n").unwrap();
        std::fs::write(p.join("package.json"), "{}").unwrap();
        std::fs::write(p.join(".nvmrc"), "v18\n").unwrap();
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let projects = crate::projects::find(&tree);
        let pins = pins(&tree, &projects);
        assert_eq!(pins.rust.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), vec!["1.99.1"]);
        assert_eq!(pins.node.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), vec!["v18"]);
        std::fs::write(p.join(".nvmrc"), vec![b'1'; (16 << 10) + 1]).unwrap();
        std::fs::write(p.join("rust-toolchain.toml"), "[toolchain]\nchannel = [\"wrong type\"]\n").unwrap();
        let tree = sprawl_core::scan(&base, |_| {}).unwrap();
        let unknown = super::pins(&tree, &projects);
        assert!(unknown.rust_unknown && unknown.node_unknown, "invalid and oversized pins suppress uninstall advice");
        assert!(bounded_text(Path::new(r"\\unreachable.invalid\share\pin"), 16 << 10).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn gradle_files_name_their_sdk_packages() {
        let groovy = "android {\n    compileSdkVersion 33\n    buildToolsVersion \"33.0.2\"\n    ndkVersion '25.1.8937393'\n    // compileSdk 99\n}";
        assert_eq!(gradle_wants(groovy), vec!["platforms;android-33", "build-tools;33.0.2", "ndk;25.1.8937393"]);
        let kts = "android {\n    compileSdk = 35\n}";
        assert_eq!(gradle_wants(kts), vec!["platforms;android-35"]);
        let block = "android {\n    compileSdk {\n        version = release(36)\n    }\n}";
        assert_eq!(gradle_wants(block), vec!["platforms;android-36"]);
        // compileSdkPreview and friends aren't compileSdk
        assert!(gradle_wants("compileSdkPreview = \"Baklava\"").is_empty());
    }

    #[test]
    fn avd_references_are_complete_bounded_and_support_relocated_data() {
        use std::os::windows::fs::OpenOptionsExt;
        let base = std::env::temp_dir().join(format!("sprawl-avd-completeness-{}", std::process::id()));
        let root = base.join("avd");
        let relocated = base.join("elsewhere/Pixel.avd");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&relocated).unwrap();
        let config = relocated.join("config.ini");
        let valid = "image.sysdir.1=system-images\\android-33\\google_apis\\x86_64\\\n";
        std::fs::write(&config, valid).unwrap();
        let pointer = root.join("Pixel.ini");
        std::fs::write(&pointer, format!("path={}\n", relocated.display())).unwrap();
        assert_eq!(avd_images_in(&root, &mut 200).unwrap(), vec!["system-images;android-33;google_apis;x86_64"]);
        assert_eq!(avd_images_in(&base.join("absent"), &mut 200), Some(Vec::new()));
        assert_eq!(avd_images_in(&root, &mut 0), None, "truncation is unknown, not no references");
        std::fs::write(&config, "image.sysdir.1=not-an-image\n").unwrap();
        assert_eq!(avd_images_in(&root, &mut 200), None);
        std::fs::write(&config, valid).unwrap();
        let locked = std::fs::OpenOptions::new().read(true).share_mode(0).open(&config).unwrap();
        // an elevated test process can still read the locked file, which is a true answer too
        if matches!(sprawl_core::elevation(), Ok(false)) { assert_eq!(avd_images_in(&root, &mut 200), None); }
        drop(locked);
        std::fs::write(&pointer, "path=\\\\unreachable.invalid\\share\\AVD\n").unwrap();
        assert_eq!(avd_images_in(&root, &mut 200), None, "remote references refuse before content access");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn android_keeps_the_newest_what_gradle_wants_and_emulator_images() {
        let (base, tree) = tree_with(
            "android",
            &[
                r"build-tools\34.0.0",
                r"build-tools\35.0.0",
                r"build-tools\36.0.0",
                r"platforms\android-34",
                r"platforms\android-36",
                r"platforms\android-36.1",
                r"system-images\android-33\google_apis\x86_64",
                r"system-images\android-35\google_apis_playstore\x86_64",
                r"system-images\android-31\default\x86_64",
                r"ndk\28.2.13676358",
            ],
        );
        let tool = Path::new(r"C:\Users\sam\AppData\Local\Android\Sdk\cmdline-tools\latest\bin\sdkmanager.bat");
        let kept = vec!["build-tools;35.0.0".to_string(), "system-images;android-31;default;x86_64".to_string()];
        let f = android_from(&tree, ROOT, tool, &kept, true);
        assert!(android_from(&tree, ROOT, tool, &kept, false).iter().all(|f| !f.path.to_string_lossy().contains("system-images")));
        let mut titles: Vec<String> = f.iter().map(|f| f.title.clone()).collect();
        titles.sort();
        // newest kept (36.0.0, android-36.1, android-35 image, the only NDK); 35.0.0 and the
        // android-31 image kept as asked for
        assert_eq!(titles, vec!["Android build tools 34.0.0", "Android emulator image android-33 google_apis x86_64", "Android platform android-34", "Android platform android-36"]);
        let bt = f.iter().find(|f| f.title.ends_with("34.0.0")).unwrap();
        assert!(matches!(&bt.action, Some(Action::Tool { command }) if command == r"& 'C:\Users\sam\AppData\Local\Android\Sdk\cmdline-tools\latest\bin\sdkmanager.bat' --uninstall 'build-tools;34.0.0'"));
        // a user's own script is never run as administrator
        let Some(Action::Tool { command }) = &bt.action else { panic!() };
        let sys = PathBuf::from(r"C:\Windows\System32");
        assert!(crate::clean::elevated_tool_ok(command, &|p| Some(PathBuf::from(p.trim_matches('\''))), std::slice::from_ref(&sys)).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    fn app(name: &str, loc: Option<PathBuf>) -> crate::facts::App {
        crate::facts::App {
            name: name.into(),
            publisher: None,
            kind: crate::facts::AppKind::Installed,
            locations: loc.into_iter().collect(),
            last_used: None,
            how_to_remove: "Settings > Apps > Installed apps".into(),
        }
    }

    #[test]
    fn python_keeps_the_newest_pins_and_venv_bases() {
        let (base, tree) = tree_with("py", &["Python311", "Python312", "Python313", "Python310"]);
        let apps = vec![
            app("Python 3.13.1 (64-bit)", Some(base.join("Python313"))),
            // no folder in its entry, as python.org's installer leaves it: found by its registration
            app("Python 3.12.7 (64-bit)", None),
            app("Python 3.11.9 (64-bit)", Some(base.join("Python311"))),
            app("Python 3.10.11 (64-bit)", Some(base.join("Python310"))),
            app("Python Launcher", None),
        ];
        let pins = HandoffPins { python: vec!["3.11".into()], venv_homes: vec![base.join("Python310")], dotnet_sdk: vec![] };
        let f = python_from(&tree, &apps, &pins, &[("3.12".into(), base.join("Python312")), ("3.12-32".into(), base.join("nope"))]);
        // 3.13 newest, 3.11 pinned, 3.10 the base of a venv: only 3.12
        assert_eq!(f.iter().map(|f| f.title.as_str()).collect::<Vec<_>>(), vec!["Older Python 3.12"]);
        assert_eq!(f[0].remedy, Remedy::Uninstall);
        assert_eq!(f[0].owner.as_deref(), Some("Python 3.12.7 (64-bit)"));
        assert!(f[0].action.is_none() && crate::clean::action(&f[0]).is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn dotnet_offers_older_patches_and_sdks_never_another_major() {
        let (base, tree) = tree_with(
            "dotnet",
            &[
                r"sdk\8.0.100",
                r"sdk\10.0.404",
                r"shared\Microsoft.NETCore.App\6.0.36",
                r"shared\Microsoft.NETCore.App\8.0.10",
                r"shared\Microsoft.NETCore.App\8.0.34",
            ],
        );
        let apps = vec![
            app("Microsoft .NET SDK 8.0.100 (x64)", None),
            app("Microsoft .NET SDK 10.0.404 (x64)", None),
            app("Microsoft .NET Runtime - 6.0.36 (x64)", None),
            app("Microsoft .NET Runtime - 8.0.10 (x64)", None),
            app("Microsoft .NET Runtime - 8.0.34 (x64)", None),
        ];
        let f = dotnet_from(&tree, &apps, &HandoffPins::default(), &base);
        let mut t: Vec<&str> = f.iter().map(|f| f.title.as_str()).collect();
        t.sort();
        // 6.0.36 is the only 6.0: kept, whatever is newer
        assert_eq!(t, vec!["Older .NET Runtime 8.0.10", "Older .NET SDK 8.0.100"]);
        // a global.json that asks for the older SDK keeps it
        let pinned = HandoffPins { dotnet_sdk: vec!["8.0.100".into()], ..Default::default() };
        assert_eq!(dotnet_from(&tree, &apps, &pinned, &base).len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn odd_names_are_never_put_in_a_command() {
        assert!(plain("1.99.1-x86_64-pc-windows-msvc"));
        assert!(!plain("1.0'; rm -r x"));
        assert!(!plain("a b"));
        assert_eq!(numbers("v18.17.0"), Some(vec![18, 17, 0]));
        assert_eq!(numbers("stable"), None);
    }
}
