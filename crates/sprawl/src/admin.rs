//! What Sprawl refuses while it runs as administrator, unless the user allows it in Settings >
//! Scanning ("While Sprawl runs as administrator, also allow"). Each is off by default, with one
//! warning above the four switches; a switch that is on is the user's answer, so nothing asks
//! again. Not elevated, nothing changes.

use std::sync::atomic::Ordering;

pub(crate) struct Lock {
    /// Its setting.
    pub key: &'static str,
    /// What it allows, as the setting names it.
    pub what: &'static str,
}

pub(crate) const SHELL_MENU: Lock = Lock {
    key: "adminShellMenu",
    what: "The Windows menu",
};
pub(crate) const HAND_OFF: Lock = Lock {
    key: "adminHandOff",
    what: "Handing an application to your uninstaller",
};
pub(crate) const CLAUDE_CODE: Lock = Lock {
    key: "adminClaudeCode",
    what: "Claude Code and Codex",
};
pub(crate) const USER_TOOLS: Lock = Lock {
    key: "adminUserTools",
    what: "Clean-up commands from your own folders",
};
/// MCP clients that aren't administrator having the disk read for them (new scans, manifests):
/// off, they get only the scan on screen.
pub(crate) const MCP_DISK: Lock = Lock {
    key: "adminMcpDisk",
    what: "MCP tools reading the disk",
};

#[cfg(test)]
pub(crate) const ALL: [&Lock; 5] = [&SHELL_MENU, &HAND_OFF, &CLAUDE_CODE, &USER_TOOLS, &MCP_DISK];

/// The one warning for all four, shown once in Settings above their switches.
pub(crate) const WARNING: &str = "Each of these lets another program run as administrator: Explorer's menu add-ons, your uninstaller, Claude Code or Codex, or a clean-up tool such as npm or pip found in your own folders; the last lets any program of yours have Sprawl list folders only an administrator can read. Leave them off unless you trust everything installed for your account.";

#[derive(Debug, PartialEq)]
pub(crate) enum Gate {
    Run,
    Refuse,
}

/// Not elevated, everything runs. Elevated, only what its setting allows: the setting is the
/// user's answer, so nothing asks again.
pub(crate) fn gate(elevated: bool, allowed: bool) -> Gate {
    if !elevated || allowed {
        Gate::Run
    } else {
        Gate::Refuse
    }
}

/// The refusal: names the setting.
pub(crate) fn refused(l: &Lock) -> String {
    format!("Not as administrator. You can allow it in Settings > Scanning ({}).", l.what)
}

/// Where the four switches are kept: not in settings.ini, which any program of this account can
/// write (it could turn one on and have the next elevated Sprawl start its `claude` or `npm` as
/// administrator), but under HKLM, where only an administrator can write. One key per account,
/// by its SID; a value of 1 is on, anything else (or nothing) off.
pub(crate) mod store {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_DWORD, REG_OPTION_NON_VOLATILE, RRF_RT_REG_DWORD, RRF_SUBKEY_WOW6464KEY,
    };

    fn path() -> Option<String> {
        let sid = sprawl_advisor::recycle::account_sid()?;
        // a SID is S-1-5-21-digits: anything else is not put into a key path
        sid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-').then(|| format!(r"SOFTWARE\Sprawl\AdminSwitches\{sid}"))
    }

    /// Whether switch `name` is on for this account.
    pub(crate) fn get(name: &str) -> bool {
        let Some(p) = path() else { return false };
        let mut v = 0u32;
        let mut n = 4u32;
        let r = unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, &HSTRING::from(p), &HSTRING::from(name), RRF_RT_REG_DWORD | RRF_SUBKEY_WOW6464KEY, None, Some((&mut v as *mut u32).cast()), Some(&mut n))
        };
        r == ERROR_SUCCESS && v == 1
    }

    /// Turn switch `name` on or off for this account: only an elevated Sprawl can.
    pub(crate) fn set(name: &str, on: bool) -> Result<(), String> {
        if !sprawl_core::is_elevated() {
            return Err("only Sprawl running as administrator can change this".into());
        }
        let p = path().ok_or("Windows couldn't say which account this is")?;
        let mut key = HKEY::default();
        let r = unsafe { RegCreateKeyExW(HKEY_LOCAL_MACHINE, &HSTRING::from(p), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE | KEY_WOW64_64KEY, None, &mut key, None) };
        if r != ERROR_SUCCESS {
            return Err(format!("couldn't open the setting ({})", r.0));
        }
        let v: u32 = on.into();
        let r = unsafe { RegSetValueExW(key, &HSTRING::from(name), None, REG_DWORD, Some(&v.to_le_bytes())) };
        let _ = unsafe { RegCloseKey(key) };
        if r == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("couldn't save the setting ({})", r.0))
        }
    }
}

/// One of the four switches' keys.
pub(crate) fn is_switch(key: &str) -> bool {
    [&SHELL_MENU, &HAND_OFF, &CLAUDE_CODE, &USER_TOOLS, &MCP_DISK].iter().any(|l| l.key == key)
}

impl crate::state::State {
    /// Whether the switch of `l` is on (in the store only an administrator can write).
    pub(crate) fn admin_on(&self, l: &Lock) -> bool {
        store::get(l.key)
    }

    /// Whether `l` may go ahead now.
    pub(crate) fn admin_allows(&mut self, l: &Lock) -> Result<(), String> {
        match gate(sprawl_core::is_elevated(), self.admin_on(l)) {
            Gate::Run => Ok(()),
            Gate::Refuse => Err(refused(l)),
        }
    }

    /// The advisor's own checks (Claude Code, tool commands) follow the settings.
    pub(crate) fn admin_flags(&self) {
        sprawl_advisor::ELEVATED_CLAUDE_CODE.store(self.admin_on(&CLAUDE_CODE), Ordering::Relaxed);
        sprawl_advisor::ELEVATED_USER_TOOLS.store(self.admin_on(&USER_TOOLS), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_refuses_on_runs_and_not_elevated_runs() {
        assert_eq!(gate(true, false), Gate::Refuse);
        assert_eq!(gate(true, true), Gate::Run, "the setting is the answer: nothing asks");
        assert_eq!(gate(false, false), Gate::Run);
        assert_eq!(gate(false, true), Gate::Run);
        for l in ALL {
            assert!(refused(l).contains("Settings > Scanning") && refused(l).contains(l.what));
        }
    }

    /// Off by default, so Reset (which puts defaults back) leaves them off; never on the wizard;
    /// in Settings > Scanning.
    #[test]
    fn off_by_default_and_not_on_the_wizard() {
        for l in ALL {
            let p = crate::prefs::get(l.key).expect(l.key);
            assert_eq!(p.default_value().as_deref(), Some("0"), "{}", l.key);
            assert_eq!(p.sec, crate::prefs::Sec::Scanning);
            assert!(crate::prefs::WIZARD.iter().all(|(_, _, _, keys)| !keys.contains(&l.key)), "{}", l.key);
        }
    }
}
