//! What Sprawl refuses while it runs as administrator, unless the user allows it in Settings >
//! Scanning ("While Sprawl runs as administrator, also allow"). Each is off by default; when on,
//! the first use in a session asks once with the same warning. Not elevated, nothing changes.

use std::sync::atomic::Ordering;

pub(crate) struct Lock {
    /// Its setting.
    pub key: &'static str,
    /// What it allows, as the setting names it.
    pub what: &'static str,
    /// The risk, said plainly.
    pub warning: &'static str,
}

pub(crate) const SHELL_MENU: Lock = Lock {
    key: "adminShellMenu",
    what: "The Windows menu",
    warning: "Explorer's menu add-ons from other programs will run as administrator inside Sprawl.",
};
pub(crate) const HAND_OFF: Lock = Lock {
    key: "adminHandOff",
    what: "Handing an application to your uninstaller",
    warning: "That program will run as administrator.",
};
pub(crate) const CLAUDE_CODE: Lock = Lock {
    key: "adminClaudeCode",
    what: "Claude Code",
    warning: "Claude Code, found through your PATH, will run as administrator.",
};
pub(crate) const USER_TOOLS: Lock = Lock {
    key: "adminUserTools",
    what: "Clean-up commands from your own folders",
    warning: "Any program of that name in your folders (npm, pip and the like) will run as administrator.",
};

#[cfg(test)]
pub(crate) const ALL: [&Lock; 4] = [&SHELL_MENU, &HAND_OFF, &CLAUDE_CODE, &USER_TOOLS];

#[derive(Debug, PartialEq)]
pub(crate) enum Gate {
    Run,
    /// Allowed by the setting, not yet asked this session.
    Ask,
    Refuse,
}

pub(crate) fn gate(elevated: bool, allowed: bool, asked: bool) -> Gate {
    match (elevated, allowed, asked) {
        (false, _, _) => Gate::Run,
        (true, false, _) => Gate::Refuse,
        (true, true, false) => Gate::Ask,
        (true, true, true) => Gate::Run,
    }
}

/// The refusal: names the setting.
pub(crate) fn refused(l: &Lock) -> String {
    format!("Not as administrator: {} You can allow it in Settings > Scanning.", l.warning)
}

impl crate::state::State {
    /// Whether `l` may go ahead now. Elevated with the setting on, the first use this session asks.
    pub(crate) fn admin_allows(&mut self, l: &Lock) -> Result<(), String> {
        let allowed = self.settings.bool(l.key) == Some(true);
        match gate(sprawl_core::is_elevated(), allowed, self.admin_asked.contains(l.key)) {
            Gate::Run => Ok(()),
            Gate::Refuse => Err(refused(l)),
            Gate::Ask => {
                let q = format!("Sprawl runs as administrator.\n\n{}\n\nAllow it until Sprawl closes? (Settings > Scanning: {})", l.warning, l.what);
                if !crate::win32::confirm(self.hwnd, &q, "Sprawl") {
                    return Err("Not allowed this time.".into());
                }
                self.admin_asked.insert(l.key);
                self.admin_flags();
                Ok(())
            }
        }
    }

    /// The advisor's own checks (Claude Code, tool commands) follow: allowed only with the setting
    /// on and the user's yes this session.
    pub(crate) fn admin_flags(&self) {
        let ok = |l: &Lock| self.settings.bool(l.key) == Some(true) && self.admin_asked.contains(l.key);
        sprawl_advisor::ELEVATED_CLAUDE_CODE.store(ok(&CLAUDE_CODE), Ordering::Relaxed);
        sprawl_advisor::ELEVATED_USER_TOOLS.store(ok(&USER_TOOLS), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_refuses_on_asks_once_and_not_elevated_runs() {
        assert_eq!(gate(true, false, false), Gate::Refuse);
        assert_eq!(gate(true, false, true), Gate::Refuse, "asked earlier, switched off since: refused");
        assert_eq!(gate(true, true, false), Gate::Ask);
        assert_eq!(gate(true, true, true), Gate::Run);
        assert_eq!(gate(false, false, false), Gate::Run);
        assert_eq!(gate(false, true, false), Gate::Run);
        for l in ALL {
            assert!(refused(l).contains("Settings > Scanning"));
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
