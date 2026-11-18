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
    what: "Claude Code",
};
pub(crate) const USER_TOOLS: Lock = Lock {
    key: "adminUserTools",
    what: "Clean-up commands from your own folders",
};

#[cfg(test)]
pub(crate) const ALL: [&Lock; 4] = [&SHELL_MENU, &HAND_OFF, &CLAUDE_CODE, &USER_TOOLS];

/// The one warning for all four, shown once in Settings above their switches.
pub(crate) const WARNING: &str = "Each of these lets another program run as administrator: Explorer's menu add-ons, your uninstaller, Claude Code, or a clean-up tool such as npm or pip found in your own folders. Leave them off unless you trust everything installed for your account.";

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

impl crate::state::State {
    /// Whether the setting of `l` is on.
    pub(crate) fn admin_on(&self, l: &Lock) -> bool {
        self.settings.bool(l.key) == Some(true)
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
