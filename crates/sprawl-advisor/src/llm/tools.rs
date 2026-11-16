//! The tools a model uses to look closer, and the one it reports through.
//!
//! Every answer comes from the scan (`Tree`) and the app facts, never from the disk beyond what
//! the scan saw. `report_finding` is the only way a model's claim becomes a finding: the path
//! must be in the scan, the size it cites must match, Rust writes any command, and the result
//! must pass [`crate::safety::check`] (which also raises a model's deletes to Orange).

use std::collections::HashSet;
use std::fmt::Write;
use std::path::PathBuf;

use serde_json::{json, Value};
use sprawl_core::{format_size, Tree};

use super::report::{node_line, types_below};
use super::ToolSpec;
use crate::facts::{clean_text, month_year, Facts};
use crate::finding::{Finding, Remedy, Risk, Source};
use crate::rules::{ps_quote, RM_FN};
use crate::safety::{self, Never};
use crate::{find_path, segments};

pub fn specs() -> Vec<ToolSpec> {
    let path = json!({"type": "string", "description": "Absolute Windows path, as it appears in the report or a tool answer"});
    vec![
        ToolSpec {
            name: "list_children",
            description: "What is directly inside a folder, biggest first: size (with exact bytes), item counts, newest change and owning app.",
            schema: json!({"type": "object", "properties": {"path": path, "limit": {"type": "integer", "description": "At most this many (default 40, max 80)"}}, "required": ["path"]}),
        },
        ToolSpec {
            name: "largest",
            description: "The biggest files, or the biggest folders, anywhere below a folder.",
            schema: json!({"type": "object", "properties": {"path": path, "what": {"type": "string", "enum": ["files", "folders"]}, "limit": {"type": "integer", "description": "Default 25, max 60"}}, "required": ["path", "what"]}),
        },
        ToolSpec {
            name: "by_type",
            description: "Space by file extension below a folder.",
            schema: json!({"type": "object", "properties": {"path": path}, "required": ["path"]}),
        },
        ToolSpec {
            name: "owner_of",
            description: "Which installed app a path belongs to, when it was last used, and how it is removed.",
            schema: json!({"type": "object", "properties": {"path": path}, "required": ["path"]}),
        },
        ToolSpec {
            name: "report_finding",
            description: "Report one place where space can be won back. The path must be in the scan and `bytes` must be its size as the tools gave it. \
                Risk: green = comes back by itself (caches, temp, build output); amber = costs something (a re-download, a rebuild, lost history); \
                orange = the user's own files, only they can decide; red = never touch, information only. \
                Remedy: delete, uninstall, move (to another drive) or info (explain only). Sprawl writes any command itself. \
                The answer says whether it was accepted, and why not.",
            schema: json!({"type": "object", "properties": {
                "path": path,
                "bytes": {"type": "integer"},
                "title": {"type": "string", "description": "A few words for the list"},
                "risk": {"type": "string", "enum": ["green", "amber", "orange", "red"]},
                "remedy": {"type": "string", "enum": ["delete", "uninstall", "move", "info"]},
                "evidence": {"type": "array", "items": {"type": "string"}, "description": "Why, one point per item: what it is, why it is safe or what it costs"},
                "confidence": {"type": "number", "description": "0 to 1"}
            }, "required": ["path", "bytes", "title", "risk", "remedy", "evidence", "confidence"]}),
        },
        ToolSpec {
            name: "finish",
            description: "End the analysis with a short summary for the user (a few sentences: the main opportunities and anything they should know).",
            schema: json!({"type": "object", "properties": {"summary": {"type": "string"}}, "required": ["summary"]}),
        },
    ]
}

pub struct Tools<'a> {
    tree: &'a Tree,
    facts: &'a Facts,
    never: Never,
    model: String,
    /// Lower-case paths already reported (by the rules or the model).
    known: HashSet<String>,
    known_list: Vec<Vec<String>>,
    pub accepted: Vec<Finding>,
    pub rejected: Vec<(Finding, String)>,
    pub summary: Option<String>,
}

fn path_arg(input: &Value) -> Result<PathBuf, String> {
    let p = input["path"].as_str().ok_or("give a `path`")?.trim().trim_matches('"').replace('/', "\\");
    if p.is_empty() {
        return Err("give a `path`".into());
    }
    Ok(PathBuf::from(p))
}

fn limit(input: &Value, default: u64, max: u64) -> usize {
    input["limit"].as_u64().unwrap_or(default).clamp(1, max) as usize
}

impl<'a> Tools<'a> {
    pub fn new(tree: &'a Tree, facts: &'a Facts, model: &str, already: &[Finding]) -> Tools<'a> {
        let mut t = Tools {
            tree,
            facts,
            never: Never::default_list(),
            model: model.to_string(),
            known: HashSet::new(),
            known_list: Vec::new(),
            accepted: Vec::new(),
            rejected: Vec::new(),
            summary: None,
        };
        for f in already {
            t.remember(&f.path);
        }
        t
    }

    fn remember(&mut self, p: &std::path::Path) {
        self.known.insert(p.to_string_lossy().to_lowercase());
        self.known_list.push(segments(&p.to_string_lossy()));
    }

    fn node(&self, input: &Value) -> Result<(usize, PathBuf), String> {
        let p = path_arg(input)?;
        let n = find_path(self.tree, &p).ok_or_else(|| format!("{} isn't in the scan", p.display()))?;
        Ok((n, p))
    }

    /// Run one tool. `Err` is sent back to the model as an error result.
    pub fn call(&mut self, name: &str, input: &Value) -> Result<String, String> {
        match name {
            "list_children" => {
                let (n, _) = self.node(input)?;
                let node = &self.tree.nodes[n];
                if !node.is_dir {
                    return Ok(format!("{} is a file: {}", self.tree.path_of(n).display(), node_line(self.tree, self.facts, n)));
                }
                let mut kids = node.children.clone();
                kids.sort_by(|a, b| self.tree.nodes[*b].size.cmp(&self.tree.nodes[*a].size));
                let max = limit(input, 40, 80);
                let mut s = format!("{}  {}\n", self.tree.path_of(n).display(), node_line(self.tree, self.facts, n));
                for &c in kids.iter().take(max) {
                    let kind = if self.tree.nodes[c].is_dir { "folder" } else { "file" };
                    let _ = writeln!(s, "- {} [{kind}] {}", self.tree.nodes[c].name, node_line(self.tree, self.facts, c));
                }
                if kids.len() > max {
                    let _ = writeln!(s, "({} more, smaller)", kids.len() - max);
                }
                Ok(s)
            }
            "largest" => {
                let (n, _) = self.node(input)?;
                let folders = input["what"].as_str() == Some("folders");
                let max = limit(input, 25, 60);
                let mut found: Vec<(u64, usize)> = Vec::new();
                let mut stack = self.tree.nodes[n].children.clone();
                while let Some(k) = stack.pop() {
                    let node = &self.tree.nodes[k];
                    if node.is_dir == folders {
                        found.push((node.size, k));
                    }
                    if node.is_dir && !node.is_reparse_point() {
                        stack.extend_from_slice(&node.children);
                    }
                }
                found.sort_by(|a, b| b.0.cmp(&a.0));
                let mut s = String::new();
                for &(_, k) in found.iter().take(max) {
                    let _ = writeln!(s, "- {}  {}", self.tree.path_of(k).display(), node_line(self.tree, self.facts, k));
                }
                Ok(if s.is_empty() { "nothing there".into() } else { s })
            }
            "by_type" => {
                let (n, _) = self.node(input)?;
                let mut s = String::new();
                for (ext, bytes, count) in types_below(self.tree, n, 25) {
                    let name = if ext.is_empty() { "(no extension)".to_string() } else { format!(".{ext}") };
                    let _ = writeln!(s, "{name}: {} in {count} files", format_size(bytes));
                }
                Ok(s)
            }
            "owner_of" => {
                let p = path_arg(input)?;
                Ok(match self.facts.owner_of(&p) {
                    None => "No installed app is known to own it.".into(),
                    Some(a) => {
                        let used = a.last_used.map(|(t, from)| format!("last started {} (from {})", month_year(t), from.label())).unwrap_or_else(|| "last use unknown".into());
                        format!("{} ({:?}): {used}. To remove it: {}", a.name, a.kind, a.how_to_remove)
                    }
                })
            }
            "report_finding" => self.report_finding(input),
            "finish" => {
                self.summary = Some(clean_text_long(input["summary"].as_str().unwrap_or(""), 1500));
                Ok("Thanks, done.".into())
            }
            other => Err(format!("there is no tool called {other}")),
        }
    }

    /// Turn a model's claim into a finding, or say why not.
    pub fn report_finding(&mut self, input: &Value) -> Result<String, String> {
        let (n, _) = self.node(input)?;
        if n == sprawl_core::ROOT {
            return Err("that is the whole scan; report something inside it".into());
        }
        // the scan's own spelling of the path
        let path = self.tree.path_of(n);
        let bytes = input["bytes"].as_u64().ok_or("give `bytes`, the size the tools reported")?;
        let risk = match input["risk"].as_str() {
            Some("green") => Risk::Green,
            Some("amber") => Risk::Amber,
            Some("orange") => Risk::Orange,
            Some("red") => Risk::Red,
            _ => return Err("risk must be green, amber, orange or red".into()),
        };
        let remedy = match input["remedy"].as_str() {
            Some("delete") => Remedy::Delete,
            Some("uninstall") => Remedy::Uninstall,
            Some("move") => Remedy::Move,
            Some("info") => Remedy::Info,
            _ => return Err("remedy must be delete, uninstall, move or info".into()),
        };
        let lower = path.to_string_lossy().to_lowercase();
        let segs = segments(&lower);
        if self.known.contains(&lower) {
            return Err("already reported".into());
        }
        if let Some(k) = self.known_list.iter().find(|k| segs.len() > k.len() && segs[..k.len()] == k[..]) {
            return Err(format!("it is inside {}, which is already reported", k.join("\\")));
        }
        let evidence: Vec<String> = input["evidence"].as_array().into_iter().flatten().filter_map(|e| e.as_str()).map(|e| clean_text_long(e, 300)).filter(|e| !e.is_empty()).take(8).collect();
        if evidence.is_empty() {
            return Err("give at least one point of evidence".into());
        }
        let title = clean_text(input["title"].as_str().unwrap_or(""));
        let title = if title.is_empty() { self.tree.nodes[n].name.clone() } else { title };
        // the command is written here, never taken from the model
        let command = (remedy == Remedy::Delete).then(|| format!("{RM_FN}; sprawl-rm (Get-Item -LiteralPath {} -Force)", ps_quote(&path.display().to_string())));
        let f = Finding {
            owner: self.facts.owner_of(&path).map(|a| a.name.clone()),
            path: path.clone(),
            bytes,
            risk,
            remedy,
            command,
            title,
            evidence,
            confidence: input["confidence"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0) as f32,
            source: Source::Model(self.model.clone()),
            allow_protected: false,
            trusted: true,
            builtin: false,
            on_path: true,
        };
        match safety::check(&f, self.tree, &self.never) {
            Ok(ok) => {
                let note = if ok.risk != f.risk { format!(" Risk raised to {:?}: a model's deletes are checked by the user.", ok.risk) } else { String::new() };
                self.remember(&path);
                self.accepted.push(ok);
                Ok(format!("Accepted.{note}"))
            }
            Err(why) => {
                let msg = why.to_string();
                self.rejected.push((f, msg.clone()));
                Err(format!("Not accepted: {msg}."))
            }
        }
    }
}

/// [`clean_text`] with a longer limit, for evidence and summaries.
fn clean_text_long(s: &str, max: usize) -> String {
    let bidi = |c: char| matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
    s.chars().filter(|&c| (c == '\n' || !c.is_control()) && !bidi(c)).take(max).collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_are_checked() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join(format!("sprawl-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cache").join("x")).unwrap();
        std::fs::write(dir.join("cache").join("x").join("a.bin"), vec![0u8; 5000]).unwrap();
        let dir = dir.canonicalize().unwrap();
        let dir = std::path::PathBuf::from(dir.to_string_lossy().trim_start_matches(r"\\?\"));
        let tree = sprawl_core::scan(&dir, |_| {}).unwrap();
        let facts = Facts::from_apps(Vec::new());
        let mut t = Tools::new(&tree, &facts, "test-model", &[]);
        let cache = dir.join("cache");
        let size = tree.nodes[find_path(&tree, &cache).unwrap()].size;
        assert!(t.call("list_children", &json!({"path": dir.display().to_string()})).unwrap().contains("cache [folder]"));
        assert!(t.call("largest", &json!({"path": dir.display().to_string(), "what": "files"})).unwrap().contains("a.bin"));
        let claim = |bytes: u64| json!({"path": cache.display().to_string(), "bytes": bytes, "title": "Cache", "risk": "green", "remedy": "delete", "evidence": ["a cache"], "confidence": 0.9});
        // a wrong size is refused
        assert!(t.call("report_finding", &claim(size + (50 << 20))).is_err());
        // the right one is taken, risk raised to orange, with a command Sprawl wrote
        assert!(t.call("report_finding", &claim(size)).is_ok());
        let f = &t.accepted[0];
        assert_eq!(f.risk, Risk::Orange);
        assert!(f.command.as_deref().unwrap().starts_with(RM_FN));
        assert!(matches!(f.source, Source::Model(_)));
        // not twice, and not something inside it
        assert!(t.call("report_finding", &claim(size)).is_err());
        let inner = json!({"path": cache.join("x").display().to_string(), "bytes": size, "title": "x", "risk": "green", "remedy": "delete", "evidence": ["e"], "confidence": 0.5});
        assert!(t.call("report_finding", &inner).unwrap_err().contains("already reported"));
        // the scan root never
        let root = json!({"path": dir.display().to_string(), "bytes": tree.nodes[0].size, "title": "all", "risk": "green", "remedy": "info", "evidence": ["e"], "confidence": 0.5});
        assert!(t.call("report_finding", &root).unwrap_err().contains("whole scan"));
        assert!(t.call("nope", &json!({})).is_err());
        assert!(t.call("finish", &json!({"summary": "done"})).is_ok() && t.summary.as_deref() == Some("done"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
