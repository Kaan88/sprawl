//! Goal mode: "I need N GB" becomes the lowest-risk set of findings that frees it.

use crate::finding::{Finding, Remedy, Risk};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// Indexes into the findings, in the order to do them (lowest risk first).
    pub picked: Vec<usize>,
    /// What the picked findings free together.
    pub bytes: u64,
    pub need: u64,
}

impl Plan {
    pub fn enough(&self) -> bool {
        self.bytes >= self.need
    }
}

/// Pick findings to free `need` bytes: green before amber, the biggest of each first, then drop
/// any that turned out not to be needed (the riskiest first). Orange (the user's own files, and
/// every delete a model suggested) is never planned: those are the user's call, one by one.
/// Only findings with something to run count; `counted[i]` false means finding `i` is inside
/// another one, and none is picked inside or around one already picked.
pub fn plan(findings: &[Finding], counted: &[bool], need: u64) -> Plan {
    let mut cand: Vec<usize> = (0..findings.len())
        .filter(|&i| counted.get(i).copied().unwrap_or(true))
        // only Sprawl's own rules: a local or AI-proposed rule's deletes are the user's call
        .filter(|&i| matches!(findings[i].remedy, Remedy::Delete | Remedy::Command) && findings[i].command.is_some() && findings[i].risk <= Risk::Amber && findings[i].builtin)
        // nor a guess by shape, nor anything outside the user's own profile
        .filter(|&i| !findings[i].heuristic() && findings[i].in_profile())
        .collect();
    cand.sort_by(|&a, &b| findings[a].risk.cmp(&findings[b].risk).then(findings[b].bytes.cmp(&findings[a].bytes)));
    let mut picked = Vec::new();
    let mut bytes = 0u64;
    let related = |a: &Finding, b: &Finding| {
        let (x, y) = (crate::segments(&a.path.to_string_lossy()), crate::segments(&b.path.to_string_lossy()));
        let n = x.len().min(y.len());
        x[..n] == y[..n]
    };
    for i in cand {
        if bytes >= need {
            break;
        }
        if picked.iter().any(|&p| related(&findings[p], &findings[i])) {
            continue;
        }
        picked.push(i);
        bytes = bytes.saturating_add(findings[i].bytes);
    }
    // the last ones taken are the riskiest: drop any whose bytes the rest already cover
    let mut k = picked.len();
    while k > 0 {
        k -= 1;
        let b = findings[picked[k]].bytes;
        if bytes.saturating_sub(b) >= need && picked.len() > 1 {
            bytes -= b;
            picked.remove(k);
        }
    }
    Plan { picked, bytes, need }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::Source;

    fn home() -> std::path::PathBuf {
        std::env::var_os("USERPROFILE").map(Into::into).unwrap_or_else(|| r"C:\Users\test".into())
    }

    fn f(risk: Risk, gb: u64) -> Finding {
        Finding {
            path: home().join(format!("x{gb}{risk:?}")),
            owner: None,
            bytes: gb << 30,
            risk,
            remedy: Remedy::Delete,
            command: Some("x".into()),
            title: String::new(),
            evidence: vec![],
            confidence: 1.0,
            source: Source::Rule("t".into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: true,
            action: None,
            admin: false,
        }
    }

    #[test]
    fn lowest_risk_first_and_no_more_than_needed() {
        let v = vec![f(Risk::Orange, 50), f(Risk::Green, 3), f(Risk::Amber, 10), f(Risk::Green, 8)];
        // orange is never planned
        assert!(!plan(&v, &[true; 4], 100 << 30).picked.contains(&0));
        let all = vec![true; 4];
        let p = plan(&v, &all, 10 << 30);
        // greens (8 + 3) are enough: no amber or orange
        assert_eq!(p.picked, vec![3, 1]);
        assert!(p.enough());
        // 8 + 3 + 10 is more than 15: the small green turns out not to be needed
        let p = plan(&v, &all, 15 << 30);
        assert_eq!(p.picked, vec![3, 2]);
        // more than everything: all but orange, and not enough
        let p = plan(&v, &all, 100 << 30);
        assert!(!p.enough() && p.picked.len() == 3);
        // a finding inside another doesn't count twice
        let p = plan(&v, &[true, false, true, true], 10 << 30);
        assert!(!p.picked.contains(&1));
    }

    #[test]
    fn guesses_and_other_accounts_are_never_planned() {
        let mut shape = f(Risk::Amber, 20);
        shape.source = Source::Rule(crate::cleaners::SHAPE.into());
        assert!(shape.heuristic());
        let mut other = f(Risk::Green, 30);
        other.path = r"Z:\Users\someone-else\AppData\Local\Temp".into();
        assert!(!other.in_profile());
        let mine = f(Risk::Green, 1);
        assert!(mine.in_profile());
        let v = vec![shape, other, mine];
        assert_eq!(plan(&v, &[true; 3], 100 << 30).picked, vec![2]);
    }

    #[test]
    fn within_compares_whole_folders() {
        use crate::finding::within;
        use std::path::Path;
        assert!(within(Path::new(r"C:\Users\Sam\AppData"), Path::new(r"c:\users\sam\")));
        assert!(within(Path::new(r"C:\Users\Sam"), Path::new(r"C:\Users\Sam")));
        assert!(!within(Path::new(r"C:\Users\Samantha\AppData"), Path::new(r"C:\Users\Sam")));
        assert!(!within(Path::new(r"C:\Users\Sam"), Path::new("")));
    }
}
