//! Takes the release build of Sprawl (`target/release/sprawl.exe` and the Windows App Runtime
//! bootstrap DLL next to it) into the installer, and gives the installer Sprawl's icon.
//! Build `sprawl` first: `cargo build --release -p sprawl` (the release tool does both).

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("..").join("..");
    // the release tool builds into its own target folder and says where
    println!("cargo:rerun-if-env-changed=SPRAWL_SETUP_FROM");
    let release = env::var_os("SPRAWL_SETUP_FROM").map(PathBuf::from).unwrap_or_else(|| root.join("target").join("release"));
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    for name in ["sprawl.exe", "Microsoft.WindowsAppRuntime.Bootstrap.dll"] {
        let from = release.join(name);
        println!("cargo:rerun-if-changed={}", from.display());
        if let Err(e) = fs::copy(&from, out.join(name)) {
            panic!("{} is missing ({e}); build Sprawl first with `cargo build --release -p sprawl`", from.display());
        }
    }

    let ico = root.join("assets").join("sprawl.ico");
    let v = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let rc = out.join("setup.rc");
    fs::write(&rc, format!("1 ICON \"{}\"\n", ico.display().to_string().replace('\\', "\\\\"))).unwrap();
    let kits = PathBuf::from(env::var_os("ProgramFiles(x86)").unwrap_or_default()).join("Windows Kits").join("10").join("bin");
    let mut versions: Vec<PathBuf> = fs::read_dir(&kits).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.join("x64").join("rc.exe").exists()).collect()).unwrap_or_default();
    versions.sort();
    if let Some(rc_exe) = versions.pop().map(|p| p.join("x64").join("rc.exe")) {
        let res = rc.with_extension("res");
        if std::process::Command::new(rc_exe).arg("/nologo").arg("/fo").arg(&res).arg(&rc).status().is_ok_and(|s| s.success()) {
            println!("cargo:rustc-link-arg-bins={}", res.display());
        }
    }
    println!("cargo:rustc-env=SPRAWL_SETUP_VERSION={v}");
}
