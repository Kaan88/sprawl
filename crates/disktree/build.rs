//! Build script for the WinUI 3 front end.
//!
//! 1. Tells the linker where `Microsoft.WindowsAppRuntime.Bootstrap.lib` lives (from the
//!    Windows App SDK Foundation 1.8 NuGet package fetched by `tools/setup.ps1`).
//! 2. Copies `Microsoft.WindowsAppRuntime.Bootstrap.dll` next to the built program, because the
//!    program loads it at start-up to find the installed Windows App Runtime 1.8.
//!
//! If the package hasn't been fetched yet it prints a warning instead of failing, so the rest of
//! the workspace (the `disktree-core` library and its tests) still builds.

use std::env;
use std::fs;
use std::path::PathBuf;

const FOUNDATION: &str = "microsoft.windowsappsdk.foundation-1.8.260803002";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

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
