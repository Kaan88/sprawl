//! Sprawl's release tool. Run it from the repository root, after building it on its own
//! (`cargo build --release -p sprawl-release`), so only one cargo runs at a time:
//!
//! ```text
//! target\release\sprawl-release keygen    once: make the signing key (kept outside the repo)
//! target\release\sprawl-release keygen backup   once: the backup key, to keep offline
//! target\release\sprawl-release build     build Sprawl and its installer, sign, put it all in dist\v<version>
//! target\release\sprawl-release publish   create the GitHub release from dist\v<version> (needs gh)
//! ```
//!
//! Both `build` and `publish` want a clean working tree: `build` records the commit it was made
//! from in `dist\v<version>\commit.txt`, and `publish` tags that exact commit instead of `main`.
//! A version that is already released is refused: a copy that has it would never take it again,
//! so the patch number goes up instead.
//!
//! The signing key is `%USERPROFILE%\.sprawl-release\signing.key` (hex). Its public half must be
//! `sprawl_install::PUBLIC_KEY`, or `build` refuses: a release signed with another key would be
//! refused by every installed copy.

use std::path::{Path, PathBuf};
use std::process::Command;

use ed25519_dalek::{Signer, SigningKey};
use sprawl_install::{sha256_hex, FileEntry, Manifest, FILES, MANIFEST, PUBLIC_KEY, REPO, SIGNATURE};

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("sprawl-release: {msg}");
    std::process::exit(1);
}

fn key_path() -> PathBuf {
    PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_else(|| fail("no %USERPROFILE%"))).join(".sprawl-release").join("signing.key")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn load_key() -> SigningKey {
    let s = std::fs::read_to_string(key_path()).unwrap_or_else(|e| fail(format!("no signing key at {} ({e}); run `keygen` first", key_path().display())));
    let s = s.trim();
    let b: Vec<u8> = (0..s.len()).step_by(2).filter_map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect();
    let b: [u8; 32] = b.try_into().unwrap_or_else(|_| fail("the signing key file is not 32 bytes of hex"));
    SigningKey::from_bytes(&b)
}

/// The workspace version, from `[workspace.package]` in Cargo.toml.
fn version() -> String {
    let toml = std::fs::read_to_string("Cargo.toml").unwrap_or_else(|_| fail("run this from the repository root"));
    let after = toml.split("[workspace.package]").nth(1).unwrap_or_else(|| fail("no [workspace.package] in Cargo.toml"));
    let line = after.lines().find(|l| l.trim_start().starts_with("version")).unwrap_or_else(|| fail("no version in [workspace.package]"));
    let v = line.split('"').nth(1).unwrap_or_else(|| fail("can't read the version")).to_string();
    sprawl_install::parse_version(&v).unwrap_or_else(|| fail(format!("version {v} is not x.y.z")));
    v
}

/// The CHANGELOG.md section for `v` (the text under `## <v>` up to the next `## `).
fn notes(v: &str) -> String {
    let log = std::fs::read_to_string("CHANGELOG.md").unwrap_or_else(|_| fail("no CHANGELOG.md"));
    let mut out = Vec::new();
    let mut on = false;
    for line in log.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            if on {
                break;
            }
            on = h.split_whitespace().next().is_some_and(|x| x.trim_matches(['[', ']']) == v);
            continue;
        }
        if on {
            out.push(line);
        }
    }
    let s = out.join("\n").trim().to_string();
    if s.is_empty() {
        fail(format!("CHANGELOG.md has no `## {v}` section; write it first"));
    }
    s
}

fn run(cmd: &mut Command) {
    let what = format!("{cmd:?}");
    let ok = cmd.status().is_ok_and(|s| s.success());
    if !ok {
        fail(format!("failed: {what}"));
    }
}

/// The one line of the notes Hub shows beside the version: the first text line of the CHANGELOG
/// section, without a list marker, a heading or emphasis marks, at most 300 characters.
fn one_line(notes: &str) -> String {
    let line = notes.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).unwrap_or("A new version of Sprawl.");
    let line = line.trim_start_matches(['-', '*', '+']).trim().replace(['*', '`'], "");
    line.chars().take(300).collect()
}

/// The files list `tools/write-hub-txt.ps1` is given (`-FilesJson`): Sprawl is a setup only, run
/// silently for all users, because Hub is not elevated and cannot write to Program Files. A JSON
/// file, so no quoting crosses a command line.
fn hub_files_json(setup: &Path) -> String {
    let name = setup.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    serde_json::json!([{ "Name": name, "Path": setup.display().to_string(), "Role": "setup", "Silent": HUB_SILENT, "Scope": "machine" }]).to_string()
}

/// The silent arguments Hub runs Setup with; no restart of Sprawl (Hub starts nothing itself).
const HUB_SILENT: &str = "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART";

/// Writes `hub.txt` into `out`, through the PowerShell script every app of the family shares
/// (PowerShell 7, `pwsh`: the script's `-FilesJson` does not read a one-item list in Windows
/// PowerShell 5.1, found when this was written). It refuses anything Hub's parser
/// would refuse, so a bad release stops here.
fn write_hub_txt(out: &Path, v: &str, notes: &str, setup: &Path) {
    let list = out.join("hub-files.json");
    std::fs::write(&list, hub_files_json(setup)).unwrap_or_else(|e| fail(e));
    let script = Path::new("tools").join("write-hub-txt.ps1");
    let mut cmd = Command::new("pwsh");
    cmd.args(["-NoProfile", "-File"]).arg(&script);
    cmd.args(["-App", "sprawl", "-Repo", "sprawl", "-Version", v, "-Notes", notes, "-Kind", "setup", "-Exe", "sprawl.exe", "-Uninstall", "Sprawl_is1", "-FilesJson"]);
    cmd.arg(&list).arg("-Out").arg(out.join(HUB_TXT));
    run(&mut cmd);
    // the list was only an input
    let _ = std::fs::remove_file(&list);
}

/// The file Hub reads from the release.
const HUB_TXT: &str = "hub.txt";

/// The commit `build` was made from, written beside the files so `publish` can check that it
/// is releasing what was built.
const COMMIT: &str = "commit.txt";

fn git(args: &[&str]) -> String {
    let out = Command::new("git").args(args).output().unwrap_or_else(|e| fail(format!("git {}: {e}", args.join(" "))));
    if !out.status.success() {
        fail(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Whether `git status --porcelain` listed nothing.
fn clean_tree(porcelain: &str) -> bool {
    porcelain.lines().all(|l| l.trim().is_empty())
}

/// A full commit hash, as `git rev-parse HEAD` prints it.
fn valid_commit(s: &str) -> Option<&str> {
    let s = s.trim();
    (s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())).then_some(s)
}

/// What is built and what is tagged must be the same commit, so neither step runs on a tree
/// with changes in it.
fn require_clean_tree(what: &str) {
    if !clean_tree(&git(&["status", "--porcelain"])) {
        fail(format!("the working tree has changes; commit and push them before {what}"));
    }
}

fn head_commit() -> String {
    let head = git(&["rev-parse", "HEAD"]);
    valid_commit(&head).unwrap_or_else(|| fail(format!("git rev-parse HEAD said {head:?}"))).to_string()
}

/// What `gh release view` said about a tag: already released, not released yet, or a question
/// that couldn't be asked (no network, not signed in) — which is never taken for "not yet".
fn release_exists(ok: bool, stderr: &str) -> Result<bool, String> {
    if ok {
        return Ok(true);
    }
    if stderr.to_ascii_lowercase().contains("release not found") {
        return Ok(false);
    }
    Err(stderr.trim().to_string())
}

/// `keygen` makes the release key; `keygen backup` the backup key (`BACKUP_KEY`), which belongs
/// offline: it is only for signing if the release key is lost or leaked.
fn keygen(backup: bool) {
    let p = if backup { key_path().with_file_name("signing-backup.key") } else { key_path() };
    if p.exists() {
        fail(format!("{} already exists; a new key would lock out every installed copy", p.display()));
    }
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).unwrap_or_else(|e| fail(e));
    let key = SigningKey::from_bytes(&seed);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap_or_else(|e| fail(e));
    std::fs::write(&p, hex(&seed)).unwrap_or_else(|e| fail(e));
    let public: Vec<String> = key.verifying_key().to_bytes().iter().map(|b| format!("0x{b:02x}")).collect();
    println!("Wrote {}. Keep it safe and out of the repository.", p.display());
    let name = if backup { "BACKUP_KEY" } else { "PUBLIC_KEY" };
    println!("Put this in crates/sprawl-install/src/lib.rs:\n\npub const {name}: [u8; 32] = [{}];", public.join(", "));
}

fn dist(v: &str) -> PathBuf {
    Path::new("dist").join(format!("v{v}"))
}

fn build() {
    let v = version();
    let notes = notes(&v);
    // the release is tied to one commit: it is recorded here and checked again at publish
    require_clean_tree("building");
    let commit = head_commit();
    let key = load_key();
    if key.verifying_key().to_bytes() != PUBLIC_KEY {
        fail("the signing key doesn't match sprawl_install::PUBLIC_KEY (rebuild this tool after changing it)");
    }
    // Source paths end up in panic messages; a release names no folder of the machine it was
    // built on (the user name is in them). Separate target folder, so this doesn't throw away
    // the everyday build.
    let home = std::env::var_os("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join(".cargo"));
    let root = std::env::current_dir().unwrap_or_else(|e| fail(e));
    let mut flags = vec![format!("--remap-path-prefix={}=cargo", home.display()), format!("--remap-path-prefix={}=sprawl", root.display())];
    if let Some(p) = std::env::var_os("USERPROFILE") {
        flags.push(format!("--remap-path-prefix={}=user", PathBuf::from(p).display()));
    }
    let flags = flags.join("\u{1f}");
    // A scan slowdown like 1.0's (15-55x on folder scans) must not ship again: a timing check
    // on disposable data, in release mode, before anything is built or signed.
    run(Command::new("cargo").args(["test", "--release", "-p", "sprawl-core", "--test", "speed_guard", "--", "--ignored"]));
    // one cargo at a time
    run(Command::new("cargo").args(["build", "--release", "-p", "sprawl", "--target-dir", "target/dist"]).env("CARGO_ENCODED_RUSTFLAGS", &flags));
    let out = dist(&v);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap_or_else(|e| fail(e));
    let release = Path::new("target").join("dist").join("release");
    let mut files = Vec::new();
    for name in FILES {
        let bytes = std::fs::read(release.join(name)).unwrap_or_else(|e| fail(format!("{name}: {e}")));
        // built without sdk\ (tools\setup.ps1), the program has no Windows App Runtime start-up
        // and doesn't open at all: the amended 0.3.2 went out like that once
        if name.ends_with(".exe") && !bytes.windows(22).any(|w| w == b"MddBootstrapInitialize") {
            fail(format!("{name} was built without the Windows App SDK (no sdk\\ folder?): it would not start. Run tools\\setup.ps1, then `cargo clean --target-dir target/dist` and build again."));
        }
        files.push(FileEntry { name: name.to_string(), sha256: sha256_hex(&bytes), size: bytes.len() as u64 });
        std::fs::write(out.join(name), &bytes).unwrap_or_else(|e| fail(e));
    }
    // the installer: Inno Setup 7 (installer/sprawl.iss), from the same files
    let iscc = std::env::var_os("SPRAWL_ISCC").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files\Inno Setup 7\ISCC.exe"));
    if !iscc.is_file() {
        fail(format!("Inno Setup 7 not found at {} (set SPRAWL_ISCC)", iscc.display()));
    }
    run(Command::new(&iscc)
        .arg("/Q")
        .arg(format!("/DAppVersion={v}"))
        // the files just hashed (the copies in the release folder), not the build folder
        .arg(format!("/DSourceDir={}", root.join(&out).display()))
        .arg(format!("/O{}", root.join(&out).display()))
        .arg(root.join("installer").join("sprawl.iss")));
    let setup = out.join(format!("Sprawl-Setup-{v}.exe"));
    if !setup.is_file() {
        fail(format!("Inno Setup didn't make {}", setup.display()));
    }
    // a copy next to the builds, for trying it out
    let _ = std::fs::copy(&setup, Path::new("target").join(format!("Sprawl-Setup-{v}.exe")));
    // Setup is signed with the program's files: a copy installed for all users updates by
    // running it, after checking it against this list
    let setup_bytes = std::fs::read(&setup).unwrap_or_else(|e| fail(e));
    let setup_entry = FileEntry { name: sprawl_install::setup_name(&v), sha256: sha256_hex(&setup_bytes), size: setup_bytes.len() as u64 };
    let manifest = serde_json::to_vec_pretty(&Manifest { version: v.clone(), files, setup: Some(setup_entry) }).unwrap();
    let sig = hex(&key.sign(&manifest).to_bytes());
    // check it the way an installed copy will
    sprawl_install::verify(&manifest, sig.as_bytes(), &sprawl_install::PUBLIC_KEYS).unwrap_or_else(|e| fail(e));
    std::fs::write(out.join(MANIFEST), &manifest).unwrap_or_else(|e| fail(e));
    std::fs::write(out.join(SIGNATURE), &sig).unwrap_or_else(|e| fail(e));
    // hub.txt: what the Hub launcher reads to find this release (tools/write-hub-txt.ps1)
    write_hub_txt(&out, &v, &one_line(&notes), &setup);
    std::fs::write(out.join("notes.md"), notes).unwrap_or_else(|e| fail(e));
    std::fs::write(out.join(COMMIT), &commit).unwrap_or_else(|e| fail(e));
    println!("Sprawl {v} is ready in {} (commit {commit})", out.display());
}

fn publish() {
    let v = version();
    let out = dist(&v);
    if !out.join(SIGNATURE).exists() {
        fail(format!("{} is not built; run `build` first", out.display()));
    }
    if !out.join(HUB_TXT).exists() {
        fail(format!("{} has no {HUB_TXT} (built before Hub support); run `build` again", out.display()));
    }
    let tag = format!("v{v}");
    // A copy that already has this version never takes it again (an update has to be strictly
    // newer), so a release is never replaced: the version goes up instead.
    let view = Command::new("gh").args(["release", "view", &tag, "--repo", REPO, "--json", "tagName"]).output().unwrap_or_else(|e| fail(format!("gh: {e}")));
    match release_exists(view.status.success(), &String::from_utf8_lossy(&view.stderr)) {
        Ok(true) => fail(format!(
            "{tag} is already published, and copies that already have {v} would never see a second one: bump the patch number in Cargo.toml, write its CHANGELOG.md section, and build again"
        )),
        Ok(false) => {}
        Err(e) => fail(format!("could not ask GitHub whether {tag} is published ({e})")),
    }
    require_clean_tree("publishing");
    let built = std::fs::read_to_string(out.join(COMMIT)).unwrap_or_else(|_| fail(format!("{} doesn't say which commit it was built from; run `build` again", out.display())));
    let built = valid_commit(&built).unwrap_or_else(|| fail(format!("{} is not a commit hash", out.join(COMMIT).display()))).to_string();
    let head = head_commit();
    if built != head {
        fail(format!("{} was built from {built}, but HEAD is now {head}; check out that commit or run `build` again", out.display()));
    }
    let mut assets: Vec<PathBuf> = FILES.iter().map(|n| out.join(n)).collect();
    assets.extend([out.join(format!("Sprawl-Setup-{v}.exe")), out.join(MANIFEST), out.join(SIGNATURE), out.join(HUB_TXT)]);
    let mut cmd = Command::new("gh");
    // the tag goes on the commit that was built, not on whatever main points at
    cmd.args(["release", "create", &tag, "--repo", REPO, "--target", &built, "--title", &format!("Sprawl {v}"), "--notes-file"]).arg(out.join("notes.md"));
    cmd.args(&assets);
    run(&mut cmd);
    println!("Published Sprawl {v} from {built}.");
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("keygen") => keygen(std::env::args().nth(2).as_deref() == Some("backup")),
        Some("build") => build(),
        Some("publish") => publish(),
        _ => fail("usage: sprawl-release keygen | build | publish"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn a_tree_is_clean_only_when_git_listed_nothing() {
        assert!(clean_tree(""));
        assert!(clean_tree("\n \n"));
        assert!(!clean_tree(" M crates/sprawl/src/main.rs\n"));
        assert!(!clean_tree("?? installer/sprawl.iss"));
    }

    #[test]
    fn only_a_full_commit_hash_is_taken() {
        assert_eq!(valid_commit(&format!("{HASH}\n")), Some(HASH));
        assert_eq!(valid_commit("fc2d97c"), None);
        assert_eq!(valid_commit(&HASH.replace('0', "z")), None);
        assert_eq!(valid_commit(""), None);
    }

    #[test]
    fn a_published_version_is_told_apart_from_an_unpublished_one() {
        assert_eq!(release_exists(true, ""), Ok(true));
        assert_eq!(release_exists(false, "release not found\n"), Ok(false));
        // anything else is a question that couldn't be asked, never a free pass
        assert!(release_exists(false, "could not connect to api.github.com").is_err());
        assert!(release_exists(false, "gh auth login required").is_err());
    }

    #[test]
    fn hub_notes_are_the_first_text_line_of_the_section() {
        assert_eq!(one_line("- Faster **scans** of `D:`.\n- Other."), "Faster scans of D:.");
        assert_eq!(one_line("### Fixed\n\n* The map opens.\n"), "The map opens.");
        assert_eq!(one_line("  \n\n"), "A new version of Sprawl.");
        assert_eq!(one_line(&"x".repeat(900)).chars().count(), 300);
    }

    #[test]
    fn hub_gets_the_setup_run_silently_for_all_users() {
        let v: serde_json::Value = serde_json::from_str(&hub_files_json(Path::new("dist/v1.0.6/Sprawl-Setup-1.0.6.exe"))).unwrap();
        let f = &v[0];
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(f["Name"], "Sprawl-Setup-1.0.6.exe");
        assert_eq!(f["Role"], "setup");
        assert_eq!(f["Scope"], "machine");
        assert_eq!(f["Silent"], "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART");
        // Setup's own [Run] starts Sprawl only when not silent, so the silent run starts nothing
        assert!(!f["Silent"].as_str().unwrap().contains("RESTARTSPRAWL"));
    }
}
