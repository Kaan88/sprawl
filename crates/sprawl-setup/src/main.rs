//! `Sprawl-Setup-<version>.exe`: installs Sprawl for the current user, in
//! `%LOCALAPPDATA%\Programs\Sprawl`, with a Start menu shortcut and an entry in Settings > Apps.
//! No administrator rights. Running it over an installed copy updates that copy.
//!
//! `/S` installs without questions and doesn't start Sprawl afterwards.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use sprawl_install::setup::{install, install_dir, installed_version, message};

const EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprawl.exe"));
const DLL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/Microsoft.WindowsAppRuntime.Bootstrap.dll"));
const VERSION: &str = env!("SPRAWL_SETUP_VERSION");

fn main() {
    let quiet = std::env::args().skip(1).any(|a| a.eq_ignore_ascii_case("/S"));
    let dir = install_dir().map(|d| d.display().to_string()).unwrap_or_default();
    if !quiet {
        let ask = match installed_version() {
            Some(v) if v == VERSION => format!("Sprawl {VERSION} is already installed. Install it again?"),
            Some(v) if sprawl_install::newer(&v, VERSION) => format!("A newer Sprawl ({v}) is installed. Replace it with {VERSION}?"),
            Some(v) => format!("Update Sprawl from {v} to {VERSION}?\n\nYour settings and scan history are kept."),
            None => format!(
                "Install Sprawl {VERSION}?\n\nIt is installed for you only, in {dir}, with a Start menu entry. No administrator rights are needed.\n\nSprawl needs the Windows App Runtime 1.8; if it's missing, Windows offers to install it the first time Sprawl starts."
            ),
        };
        if !message(&ask, true, false) {
            return;
        }
    }
    match install(&[("sprawl.exe", EXE), ("Microsoft.WindowsAppRuntime.Bootstrap.dll", DLL)], VERSION) {
        Ok(exe) => {
            if !quiet {
                let _ = std::process::Command::new(exe).spawn();
            }
        }
        Err(e) => {
            if !quiet {
                message(&format!("Sprawl could not be installed: {e}"), false, true);
            }
            std::process::exit(1);
        }
    }
}
