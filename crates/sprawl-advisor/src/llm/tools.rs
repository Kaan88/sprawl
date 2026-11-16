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
use crate::memory::{self, Label, Memory, Pref, Proposal};
use crate::rules::{ps_quote, RM_FN};
use crate::safety::{self, Never};
use crate::{find_path, segments};

/// The tools of the "Ask your disk" chat: looking, remembering, filtering the map, reading small
/// manifests and (with the user's consent each time) searching the web. No findings here.
pub fn chat_specs() -> Vec<ToolSpec> {
    let keep = ["list_children", "largest", "by_type", "owner_of", "remember"];
    let mut v: Vec<ToolSpec> = specs().into_iter().filter(|t| keep.contains(&t.name)).collect();
    let path = json!({"type": "string", "description": "Absolute Windows path"});
    v.push(ToolSpec {
        name: "set_filter",
        description: "Show something on the user's map and tables by setting Sprawl's filter. Patterns separated by |, * and ? as wildcards,             case-insensitive. A pattern with a backslash matches the full path (C:\\Users\\*\\Downloads\\*), one without matches names (*.mp4).             A pattern without wildcards matches anywhere in the name. Empty clears the filter.",
        schema: json!({"type": "object", "properties": {"filter": {"type": "string"}}, "required": ["filter"]}),
    });
    v.push(ToolSpec {
        name: "read_manifest",
        description: "Read the start of a small text file that says what something is: package.json, a Steam .acf or .vdf, a README,             a .toml, .ini, .xml, .json or .yaml. Up to 16 KB. Not for logs, keys or anything private.",
        schema: json!({"type": "object", "properties": {"path": path}, "required": ["path"]}),
    });
    v.push(ToolSpec {
        name: "web_lookup",
        description: "Search the web to find out what a folder, file or app is. The user sees the exact query and must allow it,             so keep it short and leave out personal names and paths.",
        schema: json!({"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}),
    });
    v
}

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
            name: "remember",
            description: "Remember something for next time: a preference the user stated, or what a folder is (a label for a path pattern,                 absolute, with * for one level and ** for any depth). Labels save working it out again. Don't store guesses.",
            schema: json!({"type": "object", "properties": {
                "kind": {"type": "string", "enum": ["preference", "label"]},
                "text": {"type": "string", "description": "The preference, or the label"},
                "pattern": {"type": "string", "description": "For a label: the path pattern it applies to"}
            }, "required": ["kind", "text"]}),
        },
        ToolSpec {
            name: "propose_rule",
            description: "Propose a rule for a kind of folder that will keep coming back (an app's cache, a tool's logs), so Sprawl finds it                 by itself next time. The user approves it first. Rules can delete, suggest uninstalling, or explain; never run commands.",
            schema: json!({"type": "object", "properties": {
                "id": {"type": "string", "description": "Short id, letters and dashes"},
                "title": {"type": "string"},
                "paths": {"type": "array", "items": {"type": "string"}, "description": "Absolute patterns, e.g. %LOCALAPPDATA%\\Tool\\cache or ?:\\**\\.tool-cache"},
                "risk": {"type": "string", "enum": ["green", "amber", "orange", "red"]},
                "remedy": {"type": "string", "enum": ["delete", "uninstall", "info"]},
                "why": {"type": "string", "description": "What it is and why it can go"},
                "min_bytes": {"type": "integer"}
            }, "required": ["id", "title", "paths", "risk", "remedy", "why"]}),
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
    /// The user's memory as it was: kept folders are refused. Also a scratch copy that checks
    /// what the model asks to remember; the app adds the new entries to its own copy.
    memory: Memory,
    pub prefs: Vec<Pref>,
    pub labels: Vec<Label>,
    pub proposals: Vec<Proposal>,
    /// A filter the model asked to show (chat), for the app to apply.
    pub filter: Option<String>,
    /// Asks the user whether a web search may go out; `None`: never.
    pub consent: Option<&'a dyn Fn(&str) -> bool>,
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
    pub fn new(tree: &'a Tree, facts: &'a Facts, model: &str, already: &[Finding], memory: &Memory) -> Tools<'a> {
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
            memory: memory.clone(),
            prefs: Vec::new(),
            labels: Vec::new(),
            proposals: Vec::new(),
            filter: None,
            consent: None,
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
            "set_filter" => {
                let f = clean_text(input["filter"].as_str().unwrap_or(""));
                self.filter = Some(f.clone());
                Ok(if f.is_empty() { "Filter cleared.".into() } else { format!("The map now shows only what matches {f}.") })
            }
            "read_manifest" => self.read_manifest(input),
            "web_lookup" => {
                let q = clean_text(input["query"].as_str().unwrap_or(""));
                if q.len() < 2 {
                    return Err("give a query".into());
                }
                let allowed = self.consent.is_some_and(|ask| ask(&q));
                if !allowed {
                    return Err("The user didn't allow this search.".into());
                }
                web_search(&q)
            }
            "remember" => {
                let text = input["text"].as_str().unwrap_or("");
                match input["kind"].as_str() {
                    Some("preference") => {
                        self.memory.add_pref(text, memory::From::Model)?;
                        if let Some(p) = self.memory.prefs.last().filter(|p| p.from == memory::From::Model).cloned() {
                            self.prefs.push(p);
                        }
                        Ok("Remembered.".into())
                    }
                    Some("label") => {
                        let pattern = input["pattern"].as_str().ok_or("a label needs a `pattern`")?;
                        self.memory.add_label(pattern, text, memory::From::Model)?;
                        if let Some(l) = self.memory.labels.last().cloned() {
                            self.labels.push(l);
                        }
                        Ok("Remembered.".into())
                    }
                    _ => Err("kind must be preference or label".into()),
                }
            }
            "propose_rule" => {
                let risk = serde_json::from_value::<Risk>(input["risk"].clone()).map_err(|_| "risk must be green, amber, orange or red")?;
                let remedy = serde_json::from_value::<Remedy>(input["remedy"].clone()).map_err(|_| "remedy must be delete, uninstall or info")?;
                let p = Proposal {
                    id: input["id"].as_str().unwrap_or("").to_string(),
                    title: input["title"].as_str().unwrap_or("").to_string(),
                    paths: input["paths"].as_array().into_iter().flatten().filter_map(|x| x.as_str()).map(|x| x.replace('/', "\\")).collect(),
                    risk,
                    remedy,
                    why: input["why"].as_str().unwrap_or("").to_string(),
                    min_bytes: input["min_bytes"].as_u64(),
                };
                self.memory.propose(p)?;
                if let Some(p) = self.memory.proposals.last().cloned() {
                    self.proposals.push(p);
                }
                Ok("Proposed. The user will see it and decide.".into())
            }
            "finish" => {
                self.summary = Some(clean_text_long(input["summary"].as_str().unwrap_or(""), 1500));
                Ok("Thanks, done.".into())
            }
            other => Err(format!("there is no tool called {other}")),
        }
    }

    /// The start of a small, plainly descriptive text file in the scan.
    fn read_manifest(&self, input: &Value) -> Result<String, String> {
        let (n, _) = self.node(input)?;
        let node = &self.tree.nodes[n];
        if node.is_dir || node.is_reparse_point() {
            return Err("that is a folder or a link; give a file".into());
        }
        let name = node.name.to_lowercase();
        const EXT: &[&str] = &["json", "vdf", "acf", "md", "txt", "toml", "ini", "xml", "yaml", "yml", "cfg", "nfo", "manifest", "csproj", "gradle", "inf"];
        const PRIVATE: &[&str] = &["secret", "password", "passwd", "token", "credential", "private", "wallet", ".env", "id_rsa", "id_ed25519", "cookie", "login", "auth", "key"];
        let ext_ok = name.rsplit_once('.').is_some_and(|(_, e)| EXT.contains(&e)) || name.starts_with("readme") || name.starts_with("license");
        if !ext_ok || PRIVATE.iter().any(|p| name.contains(p)) {
            return Err("only small descriptive files (manifests, READMEs, settings files without secrets) can be read".into());
        }
        if node.size > 256 << 10 {
            return Err("over 256 KB; that isn't a manifest".into());
        }
        let path = self.tree.path_of(n);
        let mut buf = Vec::new();
        let f = std::fs::File::open(&path).map_err(|e| format!("couldn't open it: {e}"))?;
        std::io::Read::read_to_end(&mut std::io::Read::take(f, 16 << 10), &mut buf).map_err(|e| e.to_string())?;
        if buf.contains(&0) {
            return Err("it isn't text".into());
        }
        Ok(format!("{}:\n{}", path.display(), String::from_utf8_lossy(&buf)))
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
        if let Some(d) = self.memory.hides(&path) {
            return Err(match d.verdict {
                memory::Verdict::Keep => format!("the user chose to keep {}", d.path.display()),
                _ => "the user said not now to this one".into(),
            });
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

/// A web search through DuckDuckGo's HTML page: the first results' titles, addresses and
/// snippets, as text.
pub(crate) fn web_search(q: &str) -> Result<String, String> {
    let enc: String = q
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else if b == b' ' { "+".into() } else { format!("%{b:02X}") })
        .collect();
    let url = format!("https://html.duckduckgo.com/html/?q={enc}");
    super::log_request("GET (web lookup)", &url);
    let r = super::http::get(&url, &[("Accept", "text/html".into())], std::time::Duration::from_secs(20))?;
    if r.status != 200 {
        return Err(format!("the search answered HTTP {}", r.status));
    }
    let results = parse_results(&r.body);
    if results.is_empty() {
        return Ok("No results (the search may have refused an automated request).".into());
    }
    Ok(results.into_iter().take(6).map(|(t, u, s)| format!("- {t} ({u})\n  {s}")).collect::<Vec<_>>().join("\n"))
}

/// (title, address, snippet) of each result on a DuckDuckGo HTML page.
pub(crate) fn parse_results(html: &str) -> Vec<(String, String, String)> {
    let text = |s: &str| -> String {
        let mut out = String::new();
        let mut tag = false;
        for c in s.chars() {
            match c {
                '<' => tag = true,
                '>' => tag = false,
                c if !tag => out.push(c),
                _ => {}
            }
        }
        out.replace("&amp;", "&").replace("&quot;", "\"").replace("&#x27;", "'").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ").trim().to_string()
    };
    let mut out = Vec::new();
    for block in html.split("class=\"result__a\"").skip(1) {
        let title = block.split_once('>').map(|(_, r)| r.split("</a>").next().unwrap_or("")).map(text).unwrap_or_default();
        let url = block.split("href=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or("").to_string();
        let snippet = block.split("class=\"result__snippet\"").nth(1).and_then(|r| r.split_once('>')).map(|(_, r)| r.split("</a>").next().unwrap_or("")).map(text).unwrap_or_default();
        if !title.is_empty() {
            out.push((clean_text_long(&title, 200), clean_text_long(&url, 300), clean_text_long(&snippet, 400)));
        }
    }
    out
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
        let mut t = Tools::new(&tree, &facts, "test-model", &[], &Memory::default());
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
        // chat tools: a filter is passed on; a web search without consent never goes out
        assert!(t.call("set_filter", &json!({"filter": "*.bin"})).is_ok() && t.filter.as_deref() == Some("*.bin"));
        assert!(t.call("web_lookup", &json!({"query": "what is DXCache"})).unwrap_err().contains("didn't allow"));
        std::fs::write(dir.join("package.json"), "{\"name\": \"x\"}").unwrap();
        std::fs::write(dir.join("api-key.json"), "{}").unwrap();
        let tree2 = sprawl_core::scan(&dir, |_| {}).unwrap();
        let t2 = Tools::new(&tree2, &facts, "m", &[], &Memory::default());
        assert!(t2.read_manifest(&json!({"path": dir.join("package.json").display().to_string()})).unwrap().contains("\"name\""));
        assert!(t2.read_manifest(&json!({"path": dir.join("api-key.json").display().to_string()})).is_err(), "nothing that looks private");
        assert!(t2.read_manifest(&json!({"path": cache.join("x").join("a.bin").display().to_string()})).is_err(), "only descriptive files");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_results_parse() {
        let html = r#"<div><a rel="nofollow" class="result__a" href="https://example.com/a">What is <b>DXCache</b> &amp; why</a>
            <a class="result__snippet" href="x">NVIDIA&#x27;s shader cache.</a></div>"#;
        let r = parse_results(html);
        assert_eq!(r, vec![("What is DXCache & why".into(), "https://example.com/a".into(), "NVIDIA's shader cache.".into())]);
    }
}
