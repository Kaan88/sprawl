//! Regenerates `crates/disktree/src/bindings.rs`: the Rust projection of the WinUI 3
//! (Windows App SDK 1.8) types the app uses.
//!
//! Run from the repository root with `cargo run -p disktree-bindgen` after `tools/setup.ps1`
//! has unpacked the SDK packages into `sdk/`. The output is checked in, so normal builds don't
//! need this step. To use a new WinUI type or member, add it to `types.txt` and rerun.

use std::path::{Path, PathBuf};

const WINUI: &str = "sdk/microsoft.windowsappsdk.winui-1.8.260803003/x/metadata";
const INTERACTIVE: &str = "sdk/microsoft.windowsappsdk.interactiveexperiences-1.8.260708001/x/metadata/10.0.18362.0";
const FOUNDATION: &str = "sdk/microsoft.windowsappsdk.foundation-1.8.260803002/x/metadata";
const WEBVIEW2: &str = "sdk/microsoft.web.webview2-1.0.3179.45/x/lib/Microsoft.Web.WebView2.Core.winmd";
const WINDOWS_SDK: &str = r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
const OUTPUT: &str = "crates/disktree/src/bindings.rs";

/// Types and members the app touches, one per line (`Namespace.Type` or `Namespace.Type::Member`).
const FILTERS: &str = include_str!("../types.txt");

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let need = |rel: &str| -> String {
        let p: PathBuf = root.join(rel);
        if !p.exists() {
            eprintln!("missing {} - run tools\\setup.ps1 first", p.display());
            std::process::exit(2);
        }
        p.display().to_string()
    };

    let mut args: Vec<String> = vec!["--in".into()];
    args.push(need(&format!("{WINUI}/Microsoft.UI.Xaml.winmd")));
    args.push(need(&format!("{WINUI}/Microsoft.UI.Text.winmd")));
    // Referenced by Microsoft.UI.Xaml.winmd; nothing from it is generated.
    args.push(need(WEBVIEW2));
    for f in ["Microsoft.UI.winmd", "Microsoft.Graphics.winmd", "Microsoft.Foundation.winmd"] {
        args.push(need(&format!("{INTERACTIVE}/{f}")));
    }
    for f in [
        "Microsoft.Windows.ApplicationModel.DynamicDependency.winmd",
        "Microsoft.Windows.ApplicationModel.WindowsAppRuntime.winmd",
        "Microsoft.Windows.Foundation.winmd",
        "Microsoft.Windows.ApplicationModel.Resources.winmd",
    ] {
        args.push(need(&format!("{FOUNDATION}/{f}")));
    }
    if !Path::new(WINDOWS_SDK).exists() {
        eprintln!("missing {WINDOWS_SDK} - install the Windows 10/11 SDK 10.0.26100");
        std::process::exit(2);
    }
    args.push(WINDOWS_SDK.into());

    let out = root.join(OUTPUT);
    args.push("--out".into());
    args.push(out.display().to_string());

    args.push("--filter".into());
    args.extend(
        FILTERS
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.starts_with('#'))
            .map(String::from),
    );

    // Interfaces the app implements in Rust (the composed Application).
    args.push("--implement".into());
    args.push("Microsoft.UI.Xaml.IApplicationOverrides".into());
    args.push("Microsoft.UI.Xaml.Markup.IXamlMetadataProvider".into());

    // In windows-bindgen 0.100.0 this returns (); bad arguments show up as a panic.
    windows_bindgen::bindgen(args);
    prune(&out);
    println!("wrote {OUTPUT}");
}

/// With member filters, windows-bindgen still emits a factory-cache helper for every factory and
/// statics interface of a class, even ones it didn't generate. Drop those helpers.
fn prune(path: &Path) {
    let src = std::fs::read_to_string(path).expect("read bindings");
    let mut out = String::with_capacity(src.len());
    let mut rest = src.as_str();
    let mut removed = 0;
    while let Some(pos) = rest.find("fn I") {
        let after = &rest[pos + 3..];
        let name: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let is_helper = after[name.len()..].starts_with('<') && !rest[..pos].ends_with("pub ");
        let line_start = rest[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
        if is_helper && name != "IActivationFactory" && !src.contains(&format!("pub struct {name}_Vtbl")) {
            // Skip to the closing brace of the function body.
            let body = pos + rest[pos..].find('{').unwrap();
            let mut depth = 0;
            let mut end = body;
            for (i, ch) in rest[body..].char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = body + i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            out.push_str(&rest[..line_start]);
            rest = &rest[end..];
            removed += 1;
        } else {
            out.push_str(&rest[..pos + 3]);
            rest = &rest[pos + 3..];
        }
    }
    out.push_str(rest);
    std::fs::write(path, out).expect("write bindings");
    println!("pruned {removed} helpers");
}
