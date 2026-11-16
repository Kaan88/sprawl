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
pub fn script(findings: &[&Finding], scan_root: &str, made: SystemTime) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "<#\n  Sprawl clean-up script for {scan_root}, made {}.", month_year(made));
    s.push_str("  Read it before running it. As it is, it only measures (a dry run):\n");
    s.push_str("      powershell -ExecutionPolicy Bypass -File <this file>\n");
    s.push_str("  To clean up, add -Run. Items marked Admin need an administrator PowerShell.\n#>\n");
    s.push_str("param([switch]$Run)\n");
    s.push_str("$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)\n");
    s.push_str(RM_FN);
    s.push('\n');
    s.push_str("function sprawl-size($p) { if (Test-Path -LiteralPath $p) { $m = Get-ChildItem -LiteralPath $p -Recurse -Force -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum; [long]$m.Sum } else { [long]0 } }\n\n");
    s.push_str("$items = @(\n");
    let mut by_hand = Vec::new();
    for f in findings {
        let Some(cmd) = &f.command else {
            by_hand.push(f);
            continue;
        };
        let body = cmd.strip_prefix(RM_FN).map(|r| r.trim_start_matches(';').trim_start()).unwrap_or(cmd);
        let admin = body.contains("# as administrator");
        let _ = writeln!(
            s,
            "  @{{ Title = {}; Path = {}; Risk = '{:?}'; Admin = ${}; Planned = {}; Do = {{\n{}\n  }} }}",
            ps_quote(&f.title),
            ps_quote(&f.path.display().to_string()),
            f.risk,
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
  if (-not $Run) { Write-Host ('[{0}] {1}: {2:N2} GB at {3}' -f $i.Risk, $i.Title, ($before / 1GB), $i.Path); $total += $before; continue }\n\
  if ($i.Admin -and -not $isAdmin) { Write-Warning ('Skipped, needs an administrator PowerShell: ' + $i.Title); continue }\n\
  Write-Host ('Cleaning ' + $i.Title + '...')\n\
  try { & $i.Do } catch { Write-Warning ($i.Title + ': ' + $_) }\n\
  $after = sprawl-size $i.Path\n\
  $freed = [Math]::Max([long]0, $before - $after)\n\
  $total += $freed\n\
  Write-Host ('  freed {0:N2} GB' -f ($freed / 1GB))\n\
}\n\
if ($Run) { Write-Host ('Freed {0:N2} GB in all.' -f ($total / 1GB)) -ForegroundColor Green } else { Write-Host ('Would free about {0:N2} GB.' -f ($total / 1GB)) -ForegroundColor Cyan }\n",
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

/// Write the script as UTF-8 with a byte order mark.
pub fn write_script(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(text.replace("\r\n", "\n").replace('\n', "\r\n").as_bytes());
    std::fs::write(path, bytes)
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
        assert!(s.contains("param([switch]$Run)") && s.contains("Dry run"));
        assert!(s.contains("To do by hand") && s.contains(r"D:\Games\Old"));
        let p = std::env::temp_dir().join(format!("sprawl-export-{}.ps1", std::process::id()));
        write_script(&p, &s).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF]);
        let _ = std::fs::remove_file(&p);
    }
}
