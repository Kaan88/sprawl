//! Build script for the WinUI 3 front end.
//!
//! 1. Tells the linker where `Microsoft.WindowsAppRuntime.Bootstrap.lib` lives (from the
//!    Windows App SDK Foundation 1.8 NuGet package fetched by `tools/setup.ps1`).
//! 2. Copies `Microsoft.WindowsAppRuntime.Bootstrap.dll` next to the built program, because the
//!    program loads it at start-up to find the installed Windows App Runtime 1.8.
//!
//! If the package hasn't been fetched yet it prints a warning instead of failing, so the rest of
//! the workspace (the `sprawl-core` library and its tests) still builds.

use std::env;
use std::fs;
use std::path::PathBuf;

const FOUNDATION: &str = "microsoft.windowsappsdk.foundation-1.8.260803002";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    icon_and_version();

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let sdk = manifest.join("..").join("..").join("sdk").join(FOUNDATION).join("x");
    let lib_dir = sdk.join("lib").join("native").join("x64");
    let dll = sdk
        .join("runtimes")
        .join("win-x64")
        .join("native")
        .join("Microsoft.WindowsAppRuntime.Bootstrap.dll");

    if lib_dir.join("Microsoft.WindowsAppRuntime.Bootstrap.lib").exists() {
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!("cargo:rustc-cfg=has_bootstrap");
    } else {
        println!(
            "cargo:warning=Windows App SDK Foundation package not found at {}; run tools\\setup.ps1",
            lib_dir.display()
        );
    }
    println!("cargo::rustc-check-cfg=cfg(has_bootstrap)");

    if dll.exists() {
        // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out; the program goes in target/<profile>.
        let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
        if let Some(profile_dir) = out.ancestors().nth(3) {
            let dest = profile_dir.join("Microsoft.WindowsAppRuntime.Bootstrap.dll");
            if let Err(e) = fs::copy(&dll, &dest) {
                println!("cargo:warning=could not copy Bootstrap.dll to {}: {e}", dest.display());
            }
        }
        println!("cargo:rerun-if-changed={}", dll.display());
    }
}

/// Embed the app icon (resource 1) and a version block, so Explorer, the taskbar and the file's
/// Properties show them.
fn icon_and_version() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let ico = manifest.join("..").join("..").join("assets").join("sprawl.ico");
    println!("cargo:rerun-if-changed={}", ico.display());
    let v = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let n: Vec<u32> = v.split(['.', '-']).take(3).map(|p| p.parse().unwrap_or(0)).collect();
    let (a, b, c) = (n.first().copied().unwrap_or(0), n.get(1).copied().unwrap_or(0), n.get(2).copied().unwrap_or(0));
    let rc = format!(
        r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Sprawl disk space analyser"
      VALUE "ProductName", "Sprawl"
      VALUE "FileVersion", "{v}"
      VALUE "ProductVersion", "{v}"
      VALUE "OriginalFilename", "sprawl.exe"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        ico = ico.display().to_string().replace('\\', "\\\\"),
    );
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("sprawl.rc");
    fs::write(&out, rc).expect("write sprawl.rc");
    // rc.exe from the newest Windows SDK turns it into a .res, which the MSVC linker takes as is.
    let Some(rc_exe) = find_rc() else {
        println!("cargo:warning=rc.exe (Windows SDK) not found; the program will have no icon");
        return;
    };
    let res = out.with_extension("res");
    let ok = std::process::Command::new(&rc_exe).arg("/nologo").arg("/fo").arg(&res).arg(&out).status().is_ok_and(|s| s.success());
    if ok {
        println!("cargo:rustc-link-arg-bins={}", res.display());
    } else {
        println!("cargo:warning=rc.exe failed; the program will have no icon");
    }
}

fn find_rc() -> Option<PathBuf> {
    let kits = PathBuf::from(env::var_os("ProgramFiles(x86)")?).join("Windows Kits").join("10").join("bin");
    let mut versions: Vec<PathBuf> = fs::read_dir(&kits).ok()?.flatten().map(|e| e.path()).filter(|p| p.join("x64").join("rc.exe").exists()).collect();
    versions.sort();
    versions.pop().map(|p| p.join("x64").join("rc.exe"))
}
