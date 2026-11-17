//! Installing, updating and uninstalling Sprawl, per user and without administrator rights.
//!
//! - **Where:** `%LOCALAPPDATA%\Programs\Sprawl`, a Start menu shortcut, and an entry in
//!   Settings > Apps (`HKCU\...\Uninstall\Sprawl`) whose Uninstall runs `sprawl.exe --uninstall`.
//! - **Releases:** each GitHub release carries the program files, `sprawl-update.json` (the
//!   version and each file's SHA-256 and size) and `sprawl-update.json.sig`, an Ed25519
//!   signature of that manifest. The key that signs it never leaves the release machine; the
//!   public half is [`PUBLIC_KEY`]. An update is only applied when the signature checks out, the
//!   manifest's version is the release's and is newer, and every file matches its hash.
//! - **Swapping:** a running program can be renamed on Windows, so each file is moved aside to
//!   `<name>.old` and the new one moved in; if any step fails, everything is put back. The
//!   `.old` files are removed on the next start.

pub mod manifest;
pub mod setup;

pub use manifest::{newer, parse_version, sha256_hex, verify, FileEntry, Manifest};

/// The GitHub repository releases come from.
pub const REPO: &str = "Kaan88/sprawl";

/// The Ed25519 public key that release manifests must be signed with.
pub const PUBLIC_KEY: [u8; 32] = [
    0x27, 0xd9, 0xd8, 0x04, 0x98, 0xdc, 0x83, 0x67, 0xda, 0xb5, 0xed, 0x91, 0x72, 0x70, 0x48, 0xbd, 0x05, 0x45, 0x20, 0x10, 0xde, 0x2b, 0x66, 0xfb, 0xf6, 0x80, 0xc6,
    0x3d, 0x95, 0xfd, 0x7e, 0x44,
];

/// The files a release may carry, and the only names an update may write.
pub const FILES: [&str; 2] = ["sprawl.exe", "Microsoft.WindowsAppRuntime.Bootstrap.dll"];

/// The manifest and its signature, as release assets.
pub const MANIFEST: &str = "sprawl-update.json";
pub const SIGNATURE: &str = "sprawl-update.json.sig";
