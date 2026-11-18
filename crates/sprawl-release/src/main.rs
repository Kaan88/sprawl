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
        .arg(format!("/DSourceDir={}", root.join(&release).display()))
        .arg(format!("/O{}", root.join(&out).display()))
        .arg(root.join("installer").join("sprawl.iss")));
    let setup = out.join(format!("Sprawl-Setup-{v}.exe"));
    if !setup.is_file() {
        fail(format!("Inno Setup didn't make {}", setup.display()));
    }
    // a copy next to the builds, for trying it out
    let _ = std::fs::copy(&setup, Path::new("target").join(format!("Sprawl-Setup-{v}.exe")));
    let manifest = serde_json::to_vec_pretty(&Manifest { version: v.clone(), files }).unwrap();
    let sig = hex(&key.sign(&manifest).to_bytes());
    // check it the way an installed copy will
    sprawl_install::verify(&manifest, sig.as_bytes(), &sprawl_install::PUBLIC_KEYS).unwrap_or_else(|e| fail(e));
    std::fs::write(out.join(MANIFEST), &manifest).unwrap_or_else(|e| fail(e));
    std::fs::write(out.join(SIGNATURE), &sig).unwrap_or_else(|e| fail(e));
    std::fs::write(out.join("notes.md"), notes).unwrap_or_else(|e| fail(e));
    println!("Sprawl {v} is ready in {}", out.display());
}

fn publish() {
    let v = version();
    let out = dist(&v);
    if !out.join(SIGNATURE).exists() {
        fail(format!("{} is not built; run `build` first", out.display()));
    }
    let status = Command::new("git").args(["status", "--porcelain"]).output().unwrap_or_else(|e| fail(e));
    if !status.stdout.is_empty() {
        fail("the working tree has changes; commit and push them first");
    }
    let mut assets: Vec<PathBuf> = FILES.iter().map(|n| out.join(n)).collect();
    assets.extend([out.join(format!("Sprawl-Setup-{v}.exe")), out.join(MANIFEST), out.join(SIGNATURE)]);
    let mut cmd = Command::new("gh");
    cmd.args(["release", "create", &format!("v{v}"), "--repo", REPO, "--target", "main", "--title", &format!("Sprawl {v}"), "--notes-file"]).arg(out.join("notes.md"));
    cmd.args(&assets);
    run(&mut cmd);
    println!("Published Sprawl {v}.");
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("keygen") => keygen(std::env::args().nth(2).as_deref() == Some("backup")),
        Some("build") => build(),
        Some("publish") => publish(),
        _ => fail("usage: sprawl-release keygen | build | publish"),
    }
}
