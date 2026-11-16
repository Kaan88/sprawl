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

/// Pick findings to free `need` bytes: green before amber before orange, the biggest of each
/// first, then drop any that turned out not to be needed (the riskiest first). Only findings
/// with something to run (a delete or a command) count; `counted[i]` false means finding `i` is
/// inside another one and its bytes are already in that one's.
pub fn plan(findings: &[Finding], counted: &[bool], need: u64) -> Plan {
    let mut cand: Vec<usize> = (0..findings.len())
        .filter(|&i| counted.get(i).copied().unwrap_or(true))
        .filter(|&i| matches!(findings[i].remedy, Remedy::Delete | Remedy::Command) && findings[i].command.is_some() && findings[i].risk < Risk::Red)
        .collect();
    cand.sort_by(|&a, &b| findings[a].risk.cmp(&findings[b].risk).then(findings[b].bytes.cmp(&findings[a].bytes)));
    let mut picked = Vec::new();
    let mut bytes = 0u64;
    for i in cand {
        if bytes >= need {
            break;
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

    fn f(risk: Risk, gb: u64) -> Finding {
        Finding {
            path: format!("C:\\x{gb}{risk:?}").into(),
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
        }
    }

    #[test]
    fn lowest_risk_first_and_no_more_than_needed() {
        let v = vec![f(Risk::Orange, 50), f(Risk::Green, 3), f(Risk::Amber, 10), f(Risk::Green, 8)];
        let all = vec![true; 4];
        let p = plan(&v, &all, 10 << 30);
        // greens (8 + 3) are enough: no amber or orange
        assert_eq!(p.picked, vec![3, 1]);
        assert!(p.enough());
        // 8 + 3 + 10 is more than 15: the small green turns out not to be needed
        let p = plan(&v, &all, 15 << 30);
        assert_eq!(p.picked, vec![3, 2]);
        // more than everything: all of it, and not enough
        let p = plan(&v, &all, 100 << 30);
        assert!(!p.enough() && p.picked.len() == 4);
        // a finding inside another doesn't count twice
        let p = plan(&v, &[true, false, true, true], 10 << 30);
        assert!(!p.picked.contains(&1));
    }
}
