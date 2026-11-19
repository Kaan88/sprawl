//! A PowerShell script for a set of findings. Without `-Run` it is a dry run: it measures each
//! item and says what it would free. With `-Run` it acts, item by item, and reports what was
//! actually freed. Sprawl itself never runs it.

use std::fmt::Write;
use std::time::SystemTime;

use crate::facts::month_year;
use crate::finding::Finding;
use crate::rules::{ps_quote, RM_FN};

/// Text for the script. Findings without a command are listed at the end as things to do by
/// hand. Write it with a UTF-8 byte order mark ([`write_script`]): Windows PowerShell 5.1 reads
/// a file without one as the ANSI code page, which would garble non-English paths.
/// Whether a command needs administrator rights: the rules mark it at the very end (a path
/// that merely contains the words doesn't count).
pub fn needs_admin(cmd: &str) -> bool {
    cmd.trim_end().ends_with("# as administrator")
}

pub fn script(findings: &[&Finding], scan_root: &str, made: SystemTime) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "<#\n  Sprawl clean-up script for {}, made {}.", scan_root.replace("#>", "# >"), month_year(made));
    s.push_str("  Read it before running it. As it is, it only measures (a dry run):\n");
    s.push_str("      powershell -ExecutionPolicy Bypass -File <this file>\n");
    s.push_str("  To clean up, add -Run. Items marked Admin need an administrator PowerShell.\n\n");
    s.push_str("  Unlike Clean up in Sprawl, this script deletes what it finds when it runs: it does not\n");
    s.push_str("  leave files created or changed since the scan, and it was not checked again against a\n");
    s.push_str("  new scan. (Items holding a folder you chose to keep were left out of it.) Run it soon,\n");
    s.push_str("  or scan again and save a new one.\n");
    if findings.iter().any(|f| f.risk >= crate::Risk::Orange) {
        // the app may send these to the Recycle Bin; the script can't
        s.push_str("\n  Your own files in it are deleted for good, not sent to the Recycle Bin: this script\n");
        s.push_str("  has no Recycle Bin. Clean up in Sprawl can send them there instead (Settings > Cleaning).\n");
    }
    s.push_str("#>\n");
    // -Report <file>: the bytes freed are written there at the end (Sprawl's Run now reads it)
    s.push_str("param([switch]$Run, [string]$Report)\n");
    s.push_str("$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)\n");
    s.push_str(RM_FN);
    s.push('\n');
    // measured without following links, like the deletes
    s.push_str("function sprawl-size($p) { $i = Get-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue; if (-not $i) { return [long]0 }; if (-not $i.PSIsContainer) { return [long]$i.Length }; if ($i.Attributes -band 1024) { return [long]0 }; $t = [long]0; foreach ($c in (Get-ChildItem -LiteralPath $i.FullName -Force -ErrorAction SilentlyContinue)) { $t += sprawl-size $c.FullName }; $t }\n\n");
    s.push_str("$items = @(\n");
    let mut by_hand = Vec::new();
    // privacy parts are cleaned only inside Sprawl, with the browser closed: not even listed
    let excluded = crate::exclusions::configured();
    for f in findings.iter().filter(|f| !f.privacy() && !f.heuristic() && f.forced_from.is_none() && f.carved.is_empty() && !crate::exclusions::intersects(&f.path, &excluded)) {
        let Some(cmd) = &f.command else {
            by_hand.push(f);
            continue;
        };
        let body = cmd.strip_prefix(RM_FN).map(|r| r.trim_start_matches(';').trim_start()).unwrap_or(cmd);
        let admin = needs_admin(body);
        let from = match &f.source {
            crate::finding::Source::Rule(id) if id.starts_with("local.") => format!("your rule {id}"),
            crate::finding::Source::Rule(id) => format!("Sprawl rule {id}"),
            crate::finding::Source::Model(m) => format!("AI ({m}), checked by Sprawl"),
        };
        let _ = writeln!(
            s,
            "  @{{ Title = {}; Path = {}; Risk = '{:?}'; From = {}; Admin = ${}; Planned = {}; Do = {{\n{}\n  }} }}",
            ps_quote(&f.title),
            ps_quote(&f.path.display().to_string()),
            f.risk,
            ps_quote(&from),
            admin,
            f.bytes,
            body
        );
    }
    s.push_str(")\n\n");
    s.push_str(
        "if (-not $Run) { Write-Host 'Dry run: nothing is changed. Run again with -Run to clean up.' -ForegroundColor Cyan }\n\
$total = [long]0\n\
foreach ($i in $items) {\n\
  $before = sprawl-size $i.Path\n\
  if (-not $Run) { Write-Host ('[{0}] {1}: {2:N2} GB at {3} ({4})' -f $i.Risk, $i.Title, ($before / 1GB), $i.Path, $i.From); $total += $before; continue }\n\
  if ($i.Admin -and -not $isAdmin) { Write-Warning ('Skipped, needs an administrator PowerShell: ' + $i.Title); continue }\n\
  Write-Host ('Cleaning ' + $i.Title + '...')\n\
  try { & $i.Do } catch { Write-Warning ($i.Title + ': ' + $_) }\n\
  $after = sprawl-size $i.Path\n\
  $freed = [Math]::Max([long]0, $before - $after)\n\
  $total += $freed\n\
  Write-Host ('  freed {0:N2} GB' -f ($freed / 1GB))\n\
}\n\
if ($Run) { Write-Host ('Freed {0:N2} GB in all.' -f ($total / 1GB)) -ForegroundColor Green } else { Write-Host ('Would free about {0:N2} GB.' -f ($total / 1GB)) -ForegroundColor Cyan }\n\
if ($Run -and $Report) { Set-Content -LiteralPath $Report -Value $total -Encoding ascii }\n",
    );
    if !by_hand.is_empty() {
        s.push_str("\n<# To do by hand (no command):\n");
        for f in by_hand {
            let _ = writeln!(s, "  - {}: {} ({:?})", f.title.replace("#>", "# >"), f.path.display().to_string().replace("#>", "# >"), f.remedy);
        }
        s.push_str("#>\n");
    }
    s
}

/// Write the script as UTF-8 with a byte order mark, as a new file (never over one, or through
/// a link planted at its name).
pub fn write_script(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(text.replace("\r\n", "\n").replace('\n', "\r\n").as_bytes());
    crate::write_local(path, &bytes, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::{Remedy, Risk, Source};

    fn f(title: &str, path: &str, command: Option<String>) -> Finding {
        Finding {
            path: path.into(),
            owner: None,
            bytes: 5 << 30,
            risk: Risk::Green,
            remedy: if command.is_some() { Remedy::Command } else { Remedy::Uninstall },
            command,
            title: title.into(),
            evidence: vec![],
            confidence: 1.0,
            source: Source::Rule("t".into()),
            allow_protected: false,
            trusted: true,
            builtin: true,
            on_path: true,
            action: None,
            admin: false,
            carved: Vec::new(),
            gain: None,
            forced_from: None,
        }
    }

    #[test]
    fn script_shape() {
        let a = f("Bob's cache", r"C:\Users\sam\it's", Some(format!("{RM_FN}; sprawl-clear 'C:\\Users\\sam\\it''s' 0")));
        let b = f("System temp", r"C:\Windows\Temp", Some(format!("{RM_FN}; sprawl-clear 'C:\\Windows\\Temp' 7   # as administrator")));
        let c = f("Old game", r"D:\Games\Old", None);
        let s = script(&[&a, &b, &c], r"C:\", SystemTime::now());
        // the functions once, at the top, not in every item
        assert_eq!(s.matches("function sprawl-rm").count(), 1);
        assert!(s.contains("Title = 'Bob''s cache'"));
        assert!(s.contains("Admin = $true") && s.contains("Admin = $false"));
        // the administrator comment sits on its own line, so it can't swallow the closing brace
        assert!(s.contains("7   # as administrator\n  } }"));
        assert!(s.contains("param([switch]$Run, [string]$Report)") && s.contains("Dry run"));
        assert!(s.contains("To do by hand") && s.contains(r"D:\Games\Old"));
        // only caches here; the user's own files make the script say it has no Recycle Bin
        assert!(!s.contains("has no Recycle Bin"));
        let mut own = f("Old project", r"C:\Users\sam\Old", Some(format!("{RM_FN}; sprawl-rm (Get-Item -LiteralPath 'C:\\Users\\sam\\Old' -Force)")));
        own.risk = crate::Risk::Orange;
        assert!(script(&[&own], r"C:\", SystemTime::now()).contains("has no Recycle Bin"));
        own.forced_from = Some(Remedy::Info);
        assert!(!script(&[&own], r"C:\", SystemTime::now()).contains("Title = 'Old project'"), "recycle-only second-click rows never become permanent script deletes");
        let p = std::env::temp_dir().join(format!("sprawl-export-{}.ps1", std::process::id()));
        write_script(&p, &s).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF]);
        let _ = std::fs::remove_file(&p);
    }

    /// The script really runs in Windows PowerShell 5.1: a dry run changes nothing, `-Run`
    /// empties the folder, and a junction planted inside loses nothing of its target.
    #[cfg(windows)]
    #[test]
    fn runs_in_windows_powershell() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join(format!("sprawl-export-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let cache = base.join("Bob's cache");
        std::fs::create_dir_all(cache.join("sub")).unwrap();
        std::fs::write(cache.join("sub").join("a.bin"), vec![1u8; 3000]).unwrap();
        std::fs::create_dir_all(base.join("victim")).unwrap();
        std::fs::write(base.join("victim").join("keep.txt"), "x").unwrap();
        let link = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(cache.join("link")).arg(base.join("victim")).output().unwrap();
        assert!(link.status.success());
        let cache_s = cache.canonicalize().unwrap().display().to_string().trim_start_matches(r"\\?\").to_string();
        let a = f("Bob's cache", &cache_s, Some(format!("{RM_FN}; sprawl-clear {} 0", ps_quote(&cache_s))));
        let file = base.join("clean.ps1");
        write_script(&file, &script(&[&a], "C:\\", SystemTime::now())).unwrap();
        let ps = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let run = |extra: &[&str]| {
            let out = std::process::Command::new(&ps).args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&file).args(extra).output().unwrap();
            String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr)
        };
        let dry = run(&[]);
        assert!(dry.contains("Dry run") && cache.join("sub").join("a.bin").exists(), "{dry}");
        let report = base.join("freed.txt");
        let real = run(&["-Run", "-Report", &report.display().to_string()]);
        assert!(real.contains("Freed"), "{real}");
        let freed: u64 = std::fs::read_to_string(&report).unwrap().trim().parse().unwrap();
        assert_eq!(freed, 3000, "what -Report says was freed");
        assert!(std::fs::read_dir(&cache).unwrap().next().is_none(), "emptied: {real}");
        assert!(base.join("victim").join("keep.txt").exists(), "the junction's target is untouched");
        let _ = std::fs::remove_dir_all(&base);
    }
}
