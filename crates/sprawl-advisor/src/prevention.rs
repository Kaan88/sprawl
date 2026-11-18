//! A5: after a successful cleanup, explain how to avoid repeatedly cleaning the same cache.
//! Advice only: no configuration is changed and no command is run.

use crate::finding::{Finding, Source};

pub fn for_finding(f: &Finding) -> Option<&'static str> {
    if !f.builtin || !f.trusted || f.privacy() { return None; }
    let Source::Rule(id) = &f.source else { return None };
    for_rule(id)
}

pub fn for_rule(id: &str) -> Option<&'static str> {
    match id {
        "dev.npm-cache" => Some("npm will refill its cache. In npm configuration, choose a cache folder on a drive with room (the cache setting)."),
        "dev.pip-cache" => Some("pip will download wheels again. Its cache-dir setting can put the cache on another drive; keeping caching on avoids repeated downloads and builds."),
        "dev.cargo-registry" => Some("Cargo will download these crates again. CARGO_HOME can place them on another drive; it also moves Cargo configuration and installed tools, so plan that change together."),
        "win.user-temp" | "win.system-temp" => Some("Temporary files will return. Windows Settings > System > Storage > Storage Sense can maintain them; review its Recycle Bin and Downloads choices before enabling it."),
        "vm.docker-disk" => Some("Docker's disk can grow again as images and volumes accumulate. Review its disk location and resource settings in Docker Desktop, and keep only the images and volumes you need."),
        "games.steam-shadercache" => Some("Steam rebuilds shader caches as you play. Repeatedly removing them trades disk space for shader downloads and longer first starts; keep the cache for games you play often."),
        "kind.archive-extracted" => Some("Before keeping both future archives and their extracted folders, decide which copy you need; keep the archive when it is your original or backup."),
        "user.old-installers" | "kind.installed-msi" | "kind.installed-exe" => Some("Keep offline and rollback installers in a folder you manage deliberately, and remove spare downloads after checking the installed program works."),
        _ if id.starts_with("project.") => Some("Build outputs return on the next build. Keeping one project's outputs while you work on it avoids repeated rebuilds; clean projects you have finished with."),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::{Remedy, Risk};
    fn finding() -> Finding { Finding {
        path: "C:\\fixture\\cache".into(), owner: None, bytes: 1, risk: Risk::Green,
        remedy: Remedy::Delete, command: None, title: "npm cache".into(), evidence: vec![],
        confidence: 0.8, source: Source::Rule("dev.npm-cache".into()), allow_protected: false,
        trusted: true, builtin: true, on_path: true, action: None, admin: false,
        carved: vec![], gain: None, forced_from: None,
    } }

    #[test]
    fn only_known_builtin_findings_get_prevention_advice() {
        let mut f = finding();
        assert!(for_finding(&f).unwrap().contains("cache setting"));
        f.builtin = false;
        assert!(for_finding(&f).is_none());
        f.builtin = true;
        f.source = Source::Model("anything".into());
        assert!(for_finding(&f).is_none());
        f.source = Source::Rule("privacy.chrome/cookies".into());
        assert!(for_finding(&f).is_none());
        f.source = Source::Rule("unknown.rule".into());
        assert!(for_finding(&f).is_none());
    }
}
