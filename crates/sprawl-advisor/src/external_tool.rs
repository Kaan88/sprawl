//! Scope and provenance for program-owned clean-ups. No fallback executes raw command text.

use super::*;
use crate::finding::Source;
use std::ffi::OsStr;

type Runner<'a> = dyn FnMut(&str, &AtomicBool, &mut Outcome, &[(&str, &OsStr)], ToolPins) -> Option<String> + 'a;
const UNKNOWN: &str = "Sprawl can't verify everywhere this command could clean";

#[derive(Clone, Debug, PartialEq)]
enum Scope {
    Cache(PackageTool),
    Fixed(Fixed),
}

#[derive(Clone, Debug, PartialEq)]
struct Fixed {
    roots: Vec<PathBuf>,
    // Some commands remove the root itself. Keep its parent stable without preventing removal.
    pins: Vec<PathBuf>,
    env: Vec<(&'static str, PathBuf)>,
    run: String,
}

impl Fixed {
    fn new(run: &str, roots: Vec<PathBuf>) -> Self {
        Self { roots, pins: Vec::new(), env: Vec::new(), run: run.into() }
    }

    fn check(&self, policy: &ToolProtection) -> Result<Vec<Held>, String> {
        if self.roots.is_empty() { return Err(UNKNOWN.into()); }
        for path in &self.roots {
            if !path.is_absolute() { return Err(UNKNOWN.into()); }
            policy.check(path)?;
            let local = sprawl_core::local::lexical(path).map_err(|e| format!("{UNKNOWN}: {} ({e})", path.display()))?;
            policy.check(&local)?;
        }
        self.pins.iter().map(|path| {
            let pinned = hold_place(path).map_err(|e| format!("{UNKNOWN}: {} ({e})", path.display()))?;
            // A short name must not hide the long path of a protected target.
            if !same_path(path, &pinned.0.path) {
                return Err(format!("{UNKNOWN}: {} resolved to {}", path.display(), pinned.0.path.display()));
            }
            Ok(pinned)
        }).collect()
    }
}

/// Receipt for precisely the command and scope rendered as runnable. Only the review
/// builder can create one, and the worker resolves it again before launch.
#[derive(Clone, Debug)]
pub struct ReviewedExternal {
    path: PathBuf,
    rule: String,
    command: String,
    scope: Scope,
}

impl ReviewedExternal {
    pub fn matches_finding(&self, f: &Finding) -> bool {
        self.path == f.path && matches!(&f.source, Source::Rule(id) if id == &self.rule)
            && matches!(action(f), Some((Action::Tool { command }, _)) if command == self.command)
    }

    pub fn description(&self) -> String {
        let (paths, command) = match &self.scope {
            Scope::Cache(cache) => (std::slice::from_ref(&self.path), cache.run),
            Scope::Fixed(fixed) => (fixed.roots.as_slice(), fixed.run.as_str()),
        };
        let paths = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("; ");
        let verb = if self.rule == "win.windows-old" { "opens Disk Cleanup; you choose what goes" } else { "runs" };
        format!("Scope: {paths} · {verb}: {command}")
    }
}

pub fn review_external(f: &Finding, policy: &ToolProtection) -> Result<ReviewedExternal, String> {
    let (Source::Rule(id), Some((Action::Tool { command }, _))) = (&f.source, action(f)) else {
        return Err("only a built-in rule can run an external cleanup command".into());
    };
    let scope = resolve(id, &f.path, &command)?;
    match &scope {
        Scope::Cache(_) => policy.check_cache(&f.path)?,
        Scope::Fixed(fixed) => { fixed.check(policy)?; }
    }
    Ok(ReviewedExternal { path: f.path.clone(), rule: id.clone(), command, scope })
}

pub fn run_reviewed_external(tool: &ReviewedExternal, spare: &Spare, stop: &AtomicBool) -> Outcome {
    let mut out = Outcome::default();
    if !stop.load(Ordering::Relaxed) {
        let result = crate::require_write().map_err(|e| e.to_string()).and_then(|()|
            dispatch(tool, stop, &mut out, || ToolProtection::load(&spare.keep), || resolve_held(&tool.rule, &tool.path, &tool.command), &mut tool_with));
        if let Err(why) = result { out.problem(&tool.path, format!("left alone: {why}")); }
    }
    out.stopped |= stop.load(Ordering::Relaxed);
    out
}

fn dispatch<H: Send + Sync + 'static>(
    tool: &ReviewedExternal, stop: &AtomicBool, out: &mut Outcome,
    mut policy: impl FnMut() -> Result<ToolProtection, String>, resolve_now: impl FnOnce() -> Result<(Scope, H), String>, invoke: &mut Runner<'_>,
) -> Result<(), String> {
    if tool.rule == RECYCLE_BIN_RULE { return Err("Sprawl leaves the Recycle Bin alone; review it in Windows".into()); }
    // Read policy first; take the fresh target identity/config as close to launch as possible.
    let current = policy()?;
    let (scope, settings_pin) = resolve_now()?;
    if scope != tool.scope { return Err("the command's scope changed; review it again".into()); }
    match scope {
        Scope::Cache(binding) => {
            let mut first = Some(current);
            run_package(&tool.path, binding, stop, out, || first.take().map_or_else(&mut policy, Ok), invoke)
        }
        Scope::Fixed(fixed) => {
            let pins = ToolPins::hold((fixed.check(&current)?, settings_pin));
            let env: Vec<_> = fixed.env.iter().map(|(k, v)| (*k, v.as_os_str())).collect();
            // Scope validation succeeded; the runner records execution failures separately in
            // Outcome and owns these pins even if termination needs an out-of-band retry.
            if !stop.load(Ordering::Relaxed) { invoke(&fixed.run, stop, out, &env, pins); }
            Ok(())
        }
    }
}

const CACHE_RULES: &[&str] = &["dev.npm-cache", "dev.pip-cache", "dev.nuget-packages", "dev.pnpm-store",
    "dev.yarn-cache", "dev.go-build-cache", "dev.conda-pkgs"];
const WINDOWS_RULES: &[&str] = &["win.component-store", "win.delivery-optimization", "win.windows-old"];

fn pack_command(id: &str) -> Option<&'static str> {
    static PACK: std::sync::OnceLock<crate::rules::RulePack> = std::sync::OnceLock::new();
    PACK.get_or_init(crate::rules::RulePack::builtin).all_rules().find(|r| r.id == id)?.command.as_deref()
}

fn resolve(id: &str, path: &Path, command: &str) -> Result<Scope, String> {
    resolve_held(id, path, command).map(|(scope, _)| scope)
}

fn resolve_held(id: &str, path: &Path, command: &str) -> Result<(Scope, Option<sprawl_core::local::LocalFile>), String> {
    let nvm = (id == "versions.nvm").then(|| crate::versions::nvm_cleanup_settings(path)).transpose()?;
    let scope = resolve_in(id, path, command, nvm.as_ref().map(|(_, text)| text.as_str()))?;
    Ok((scope, nvm.map(|(pin, _)| pin)))
}

fn resolve_in(id: &str, path: &Path, command: &str, nvm: Option<&str>) -> Result<Scope, String> {
    if !path.is_absolute() { return Err(UNKNOWN.into()); }
    if CACHE_RULES.contains(&id) && pack_command(id) == Some(command) {
        return match package_tool(command) {
            Some(Ok(tool)) => Ok(Scope::Cache(tool)),
            Some(Err(why)) => Err(format!("{UNKNOWN}: {why}")),
            None => Err(UNKNOWN.into()),
        };
    }
    if WINDOWS_RULES.contains(&id) && pack_command(id) == Some(command) {
        let windows = windows_directory().ok_or("Windows couldn't identify its installation folder")?;
        let redirects = if id == "win.delivery-optimization" { delivery_redirects(&windows)? } else { Vec::new() };
        return windows_scope(id, path, command, &windows, redirects).map(Scope::Fixed);
    }
    if matches!(id, crate::devgit::GIT_GC | crate::devgit::WORKTREE) {
        let git = crate::devgit::cleanup_metadata(id, path, command)?;
        let mut fixed = Fixed::new(command, vec![git.clone()]);
        fixed.pins.push(git.clone());
        if id == crate::devgit::WORKTREE {
            fixed.roots.push(path.to_path_buf());
            fixed.pins.push(removal_parent(path)?);
        }
        fixed.env = vec![("GIT_DIR", git), ("GIT_CONFIG_NOSYSTEM", "1".into()), ("GIT_CONFIG_GLOBAL", "NUL".into()),
            ("GIT_TERMINAL_PROMPT", "0".into())];
        return Ok(Scope::Fixed(fixed));
    }
    if let Some((roots, env, run)) = crate::versions::cleanup_scope(id, path, command, nvm) {
        let mut fixed = Fixed::new(&run, roots);
        fixed.env = env;
        fixed.pins.push(removal_parent(path)?);
        // nvm's settings and fnm's aliases live alongside (not in) the version folder.
        if id == "versions.fnm" {
            if let Some(dir) = fixed.roots.get(1) { fixed.pins.push(dir.parent().ok_or(UNKNOWN)?.to_path_buf()); }
        }
        return Ok(Scope::Fixed(fixed));
    }
    Err(UNKNOWN.into())
}

fn removal_parent(path: &Path) -> Result<PathBuf, String> {
    // Confirm the victim is the expected plain directory before releasing its leaf
    // handle, then keep the containing directory/ancestors stable during dispatch.
    let (dir, _) = hold_place(path).map_err(|e| format!("{UNKNOWN}: {} ({e})", path.display()))?;
    if !same_path(path, &dir.path) { return Err(UNKNOWN.into()); }
    Ok(dir.path.parent().ok_or(UNKNOWN)?.to_path_buf())
}

fn drive(path: &Path) -> Option<char> {
    let p = tool_path(path);
    let b = p.to_str()?.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && &b[1..3] == b":\\").then(|| b[0].to_ascii_uppercase() as char)
}

fn windows_scope(id: &str, path: &Path, command: &str, windows: &Path, redirects: Vec<PathBuf>) -> Result<Fixed, String> {
    let system = drive(windows).ok_or(UNKNOWN)?;
    let volume = PathBuf::from(format!("{system}:\\"));
    let fixed = match id {
        "win.component-store" if same_path(path, &windows.join("WinSxS")) => Fixed::new(command, vec![windows.join("WinSxS")]),
        "win.delivery-optimization" => {
            let service = windows.join(r"ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization");
            let legacy = windows.join(r"SoftwareDistribution\DeliveryOptimization");
            if !same_path(path, &service.join("Cache")) && !same_path(path, &legacy) { return Err(UNKNOWN.into()); }
            let mut roots = vec![service, legacy];
            roots.extend(redirects);
            Fixed::new(command, roots)
        }
        "win.windows-old" if same_path(path, &volume.join("Windows.old")) => {
            // Without /sagerun this opens Disk Cleanup's UI. Do not use the elevation broker
            // to start a separate process outside the job; the existing admin gate applies.
            Fixed::new(&format!("Start-Process cleanmgr -ArgumentList '/d {system}:' -NoNewWindow -Wait"), vec![volume])
        }
        _ => return Err(UNKNOWN.into()),
    };
    Ok(fixed)
}

pub(crate) fn windows_directory() -> Option<PathBuf> {
    #[link(name = "kernel32")]
    extern "system" { fn GetSystemWindowsDirectoryW(path: *mut u16, len: u32) -> u32; }
    let mut buf = [0u16; 520];
    let n = unsafe { GetSystemWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

fn delivery_redirects(windows: &Path) -> Result<Vec<PathBuf>, String> {
    // Both Group Policy and the current MDM policy can move the cache. Check both;
    // don't assume which wins, or interpret unreadable/wrong-type values as defaults.
    const KEYS: &[&str] = &[
        r"SOFTWARE\Policies\Microsoft\Windows\DeliveryOptimization",
        r"SOFTWARE\Microsoft\PolicyManager\current\device\DeliveryOptimization",
    ];
    let values = KEYS.iter().map(|key| crate::reg::checked_machine_string(key, "DOModifyCacheDrive")).collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Delivery Optimization's cache location couldn't be read ({e})"))?;
    delivery_paths(windows, values.into_iter().flatten())
}

fn delivery_paths(windows: &Path, values: impl IntoIterator<Item = String>) -> Result<Vec<PathBuf>, String> {
    let system = drive(windows).ok_or(UNKNOWN)?;
    let mut out = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() { continue; }
        let mut result = value.to_string();
        for (var, replacement) in [("%SYSTEMDRIVE%", format!("{system}:")), ("%SYSTEMROOT%", windows.display().to_string()), ("%WINDIR%", windows.display().to_string())] {
            while let Some(i) = result.to_ascii_uppercase().find(var) {
                result.replace_range(i..i + var.len(), &replacement);
            }
        }
        if result.contains('%') { return Err("Delivery Optimization's cache location uses an unknown variable".into()); }
        if result.len() == 2 && result.ends_with(':') { result.push('\\'); }
        let path = PathBuf::from(result);
        if drive(&path).is_none() { return Err("Delivery Optimization's cache location isn't a full local path".into()); }
        // Check the configured base, not just one version's generated cache subfolder.
        out.push(path);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(kept: Vec<PathBuf>, skipped: Vec<PathBuf>) -> ToolProtection { ToolProtection { kept, skipped } }

    fn finding(id: &str, path: PathBuf, command: &str) -> Finding {
        Finding {
            path, owner: None, bytes: 1, risk: crate::Risk::Amber, remedy: crate::Remedy::Command,
            command: None, title: id.into(), evidence: vec![], confidence: 1.0, source: Source::Rule(id.into()),
            allow_protected: false, trusted: true, builtin: true, on_path: false,
            action: Some(Action::Tool { command: command.into() }), admin: false, carved: vec![], gain: None, forced_from: None,
        }
    }

    #[test]
    fn dism_checks_winsxs_and_allows_unrelated_keep() {
        let tmp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-dism-scope").unwrap();
        let windows = tmp.path().join("Windows");
        let store = windows.join("WinSxS");
        let rule = "win.component-store";
        let command = pack_command(rule).unwrap();
        let scope = Scope::Fixed(windows_scope(rule, &store, command, &windows, vec![]).unwrap());
        let tool = ReviewedExternal { path: store.clone(), rule: rule.into(), command: command.into(), scope: scope.clone() };
        let mut calls = 0;
        let invoke = &mut |script: &str, _: &AtomicBool, _: &mut Outcome, _: &[(&str, &OsStr)], _: ToolPins| {
            assert_eq!(script, command);
            calls += 1;
            None
        };
        dispatch(&tool, &AtomicBool::new(false), &mut Outcome::default(),
            || Ok(policy(vec![tmp.path().join("Documents")], vec![])), || Ok((scope.clone(), ())), invoke).unwrap();
        assert!(dispatch(&tool, &AtomicBool::new(false), &mut Outcome::default(),
            || Ok(policy(vec![store.join("keep")], vec![])), || Ok((scope.clone(), ())), invoke).unwrap_err().contains("Keep list"));
        assert_eq!(calls, 1);
        assert!(windows_scope(rule, &windows.join("Other"), command, &windows, vec![]).is_err());
    }

    #[test]
    fn git_cleanup_refuses_a_keep_inside_dot_git_in_review_and_at_launch() {
        let tmp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-gc-keep").unwrap();
        let git = tmp.path().join(".git");
        std::fs::create_dir(&git).unwrap();
        let f = finding(crate::devgit::GIT_GC, git.clone(), &crate::devgit::gc_command(tmp.path()));
        let kept = || policy(vec![git.join("objects").join("keep")], vec![]);
        assert!(review_external(&f, &kept()).unwrap_err().contains("Keep list"));
        let tool = review_external(&f, &policy(vec![], vec![])).unwrap();
        assert!(dispatch(&tool, &AtomicBool::new(false), &mut Outcome::default(), || Ok(kept()),
            || resolve_held(&tool.rule, &tool.path, &tool.command), &mut |_, _, _, _, _| panic!("held command launched"))
            .unwrap_err().contains("Keep list"));
    }

    fn fixture_git(repo: &Path, args: &[&str]) {
        let mut cmd = std::process::Command::new("git");
        for (k, _) in std::env::vars_os() {
            if k.to_string_lossy().to_ascii_uppercase().starts_with("GIT_") { cmd.env_remove(k); }
        }
        let output = cmd.current_dir(repo).env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "NUL").args(args).output().unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }

    fn fixture_commit(repo: &Path) {
        fixture_git(repo, &["init", "-q", "--template="]);
        std::fs::write(repo.join("one.txt"), b"fixture content\n").unwrap();
        fixture_git(repo, &["add", "one.txt"]);
        fixture_git(repo, &["-c", "user.name=Sprawl Fixture", "-c", "user.email=fixture@invalid", "commit", "-qm", "fixture"]);
    }

    #[test]
    fn git_gc_packs_objects_in_a_disposable_repository_through_the_reviewed_dispatch() {
        let tmp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-gc-run").unwrap();
        let repo = tmp.path();
        let git = repo.join(".git");
        fixture_commit(repo);
        let has_pack = || std::fs::read_dir(git.join(r"objects\pack")).unwrap().flatten()
            .any(|e| e.path().extension().is_some_and(|e| e == "pack"));
        assert!(!has_pack());
        let f = finding(crate::devgit::GIT_GC, git.clone(), &crate::devgit::gc_command(repo));
        let tool = review_external(&f, &policy(vec![repo.join("one.txt")], vec![])).unwrap();
        let sys = system_dir().unwrap();
        let mut out = Outcome::default();
        dispatch(&tool, &AtomicBool::new(false), &mut out, || Ok(policy(vec![repo.join("one.txt")], vec![])),
            || resolve_held(&tool.rule, &tool.path, &tool.command),
            &mut |cmd, stop, out, env, pins| run_powershell(cmd, &sys, false, stop, out, env, pins, Duration::from_secs(30))).unwrap();
        assert!(out.problems.is_empty() && has_pack(), "{:?} {:?}", out.problems, out.output);
        assert_eq!(std::fs::read(repo.join("one.txt")).unwrap(), b"fixture content\n");
    }

    #[test]
    fn git_worktree_checks_metadata_and_removes_only_the_disposable_worktree() {
        let tmp = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-worktree-run").unwrap();
        let repo = tmp.path().join("main");
        let worktree = tmp.path().join("linked");
        std::fs::create_dir(&repo).unwrap();
        fixture_commit(&repo);
        fixture_git(&repo, &["worktree", "add", "-q", "--detach", worktree.to_str().unwrap()]);
        let f = finding(crate::devgit::WORKTREE, worktree.clone(), &crate::devgit::worktree_command(&repo, &worktree));
        let metadata = repo.join(r".git\worktrees\linked");
        assert!(review_external(&f, &policy(vec![], vec![metadata.clone()])).unwrap_err().contains("Folders to skip"));
        assert!(review_external(&f, &policy(vec![worktree.join("one.txt")], vec![])).unwrap_err().contains("Keep list"));
        let tool = review_external(&f, &policy(vec![], vec![])).unwrap();
        let sys = system_dir().unwrap();
        let mut out = Outcome::default();
        dispatch(&tool, &AtomicBool::new(false), &mut out, || Ok(policy(vec![], vec![])), || resolve_held(&tool.rule, &tool.path, &tool.command),
            &mut |cmd, stop, out, env, pins| run_powershell(cmd, &sys, false, stop, out, env, pins, Duration::from_secs(30))).unwrap();
        assert!(out.problems.is_empty() && !worktree.exists() && !metadata.exists(), "{:?} {:?}", out.problems, out.output);
        assert_eq!(std::fs::read(repo.join("one.txt")).unwrap(), b"fixture content\n");
    }

    #[test]
    fn recycle_bin_rule_is_information_and_former_receipts_cannot_launch() {
        let pack = crate::rules::RulePack::builtin();
        let rule = pack.all_rules().find(|r| r.id == RECYCLE_BIN_RULE).unwrap();
        assert_eq!(rule.remedy, crate::Remedy::Info);
        assert!(rule.command.is_none());
        let cmd = "Clear-RecycleBin -DriveLetter E -Force";
        let path = std::env::temp_dir().join("fixture-bin");
        let f = finding(RECYCLE_BIN_RULE, path.clone(), cmd);
        assert!(action(&f).is_none());
        assert!(review_external(&f, &policy(vec![], vec![])).is_err());
        let scope = Scope::Fixed(Fixed::new(cmd, vec![path.clone()]));
        let tool = ReviewedExternal { path, rule: RECYCLE_BIN_RULE.into(), command: cmd.into(), scope: scope.clone() };
        assert!(dispatch(&tool, &AtomicBool::new(false), &mut Outcome::default(), || Ok(policy(vec![], vec![])),
            || Ok((scope, ())), &mut |_, _, _, _, _| panic!("the former bin command was launched")).unwrap_err().contains("leaves the Recycle Bin alone"));
    }

    #[test]
    fn nvm_settings_cannot_be_changed_or_replaced_during_dispatch() {
        let owned = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-nvm-held").unwrap();
        let home = owned.path();
        let target = home.join("v18.1.0");
        let settings = home.join("settings.txt");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(&settings, format!("root: {}\npath: {}\n", home.display(), home.join("nodejs").display())).unwrap();
        let f = finding("versions.nvm", target, "nvm uninstall '18.1.0'");
        let tool = review_external(&f, &policy(vec![], vec![])).unwrap();
        dispatch(&tool, &AtomicBool::new(false), &mut Outcome::default(), || Ok(policy(vec![], vec![])),
            || resolve_held(&tool.rule, &tool.path, &tool.command), &mut |_, _, _, _, _pins| {
                assert!(std::fs::read_to_string(&settings).is_ok());
                assert!(std::fs::write(&settings, "changed").is_err());
                assert!(std::fs::rename(&settings, home.join("other")).is_err());
                None
            }).unwrap();
        std::fs::write(&settings, "new settings").unwrap();
    }

    #[test]
    fn stopped_nvm_fixture_waits_for_native_child_before_releasing_settings() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};
        let owned = sprawl_core::local::OwnedDir::create_in(&std::env::temp_dir(), "sprawl-nvm-stop").unwrap();
        let home = owned.path();
        let target = home.join("v18.1.0");
        let settings = home.join("settings.txt");
        let ready = home.join("child-id.txt");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(&settings, format!("root: {}\npath: {}\n", home.display(), home.join("nodejs").display())).unwrap();
        let f = finding("versions.nvm", target, "nvm uninstall '18.1.0'");
        let tool = review_external(&f, &policy(vec![], vec![])).unwrap();
        let sys = system_dir().unwrap();
        let ps = crate::rules::ps_quote(&sys.join(r"WindowsPowerShell\v1.0\powershell.exe").display().to_string());
        let child_script = format!("[IO.File]::WriteAllText({}, $PID); Start-Sleep -Seconds 60", crate::rules::ps_quote(&ready.display().to_string()));
        let script = format!(r#"Start-Process -FilePath {ps} -NoNewWindow -ArgumentList '-NoProfile -NonInteractive -EncodedCommand {}'"#, encoded(&child_script));
        let stop = AtomicBool::new(false);
        let mut out = Outcome::default();
        let child = std::thread::scope(|s| {
            let signal = s.spawn(|| {
                let deadline = std::time::Instant::now() + Duration::from_secs(15);
                let pid = loop {
                    if let Some(pid) = std::fs::read_to_string(&ready).ok().and_then(|s| s.parse::<u32>().ok()) { break pid; }
                    if std::time::Instant::now() >= deadline { stop.store(true, Ordering::Relaxed); panic!("fixture child did not start"); }
                    std::thread::sleep(Duration::from_millis(20));
                };
                // Hold the actual process identity, not a later lookup of a possibly reused PID.
                let h = unsafe { OpenProcess(0x0010_0000, 0, pid) }; // SYNCHRONIZE
                let child = (!h.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(h) });
                let running = child.as_ref().is_some_and(|h| unsafe { WaitForSingleObject(h.as_raw_handle(), 0) } == WAIT_TIMEOUT);
                let pinned = std::fs::write(&settings, "changed").is_err();
                stop.store(true, Ordering::Relaxed);
                assert!(running && pinned);
                child.unwrap()
            });
            dispatch(&tool, &stop, &mut out, || Ok(policy(vec![], vec![])),
                || resolve_held(&tool.rule, &tool.path, &tool.command),
                &mut |_, stop, out, env, pins| run_powershell(&script, &sys, false, stop, out, env, pins, Duration::from_secs(20))).unwrap();
            signal.join().unwrap()
        });
        assert!(out.stopped && !out.timed_out, "{:?}", out.problems);
        assert_eq!(unsafe { WaitForSingleObject(child.as_raw_handle(), 0) }, WAIT_OBJECT_0);
        std::fs::write(&settings, "now writable").unwrap();
    }

    #[test]
    fn compiled_pack_tool_rules_are_classified_and_non_builtin_text_is_not_authorized() {
        let pack = crate::rules::RulePack::builtin();
        let declared: std::collections::BTreeSet<_> = pack.all_rules().filter(|r| r.command.is_some()).map(|r| r.id.as_str()).collect();
        assert_eq!(declared, CACHE_RULES.iter().chain(WINDOWS_RULES).copied().collect());
        let cmd = pack_command("dev.npm-cache").unwrap();
        let mut f = finding("dev.npm-cache", std::env::temp_dir(), cmd);
        f.builtin = false;
        assert!(review_external(&f, &policy(vec![], vec![])).unwrap_err().contains("built-in"));
        f.builtin = true;
        f.source = Source::Model("fixture".into());
        assert!(review_external(&f, &policy(vec![], vec![])).unwrap_err().contains("built-in"));
        f.source = Source::Rule("versions.unknown".into());
        assert_eq!(review_external(&f, &policy(vec![], vec![])).unwrap_err(), UNKNOWN);
        for id in ["dev.pnpm-store", "dev.yarn-cache", "dev.conda-pkgs"] {
            assert!(resolve(id, &f.path, pack_command(id).unwrap()).unwrap_err().contains(UNKNOWN));
        }
    }

    #[test]
    fn windows_scopes_cover_delivery_redirects_and_disk_cleanup_choices() {
        let windows = Path::new(r"C:\Windows");
        let redirect = delivery_paths(windows, ["%SystemDrive%".into(), r"D:\ServiceCache".into()]).unwrap();
        assert_eq!(redirect, [PathBuf::from(r"C:\"), PathBuf::from(r"D:\ServiceCache")]);
        assert!(delivery_paths(windows, [r"%Unknown%\Cache".into()]).is_err());
        let id = "win.delivery-optimization";
        let f = windows_scope(id, &windows.join(r"SoftwareDistribution\DeliveryOptimization"), pack_command(id).unwrap(), windows, redirect).unwrap();
        assert!(f.roots.contains(&PathBuf::from(r"D:\ServiceCache")));
        let id = "win.windows-old";
        let f = windows_scope(id, Path::new(r"C:\Windows.old"), pack_command(id).unwrap(), windows, vec![]).unwrap();
        assert_eq!(f.roots, [PathBuf::from(r"C:\")]);
        assert!(f.run.contains("'/d C:'"));
        assert!(f.run.contains("-NoNewWindow") && !f.run.contains("RunAs"));
        assert!(crate::rules::RulePack::builtin().all_rules().find(|r| r.id == id).unwrap().admin);
    }
}
