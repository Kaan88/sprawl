//! Text formatting: numbers, dates, file-type names, and default save folders.

use std::path::PathBuf;

pub(crate) static DECIMAL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn unit_base() -> (f64, [&'static str; 5]) {
    if DECIMAL.load(std::sync::atomic::Ordering::Relaxed) { (1000.0, ["bytes", "KB", "MB", "GB", "TB"]) }
    else { (1024.0, ["bytes", "KiB", "MiB", "GiB", "TiB"]) }
}

pub(crate) fn size(v: u64) -> String {
    let (base, labels) = unit_base();
    if v < base as u64 { return format!("{v} bytes"); }
    let mut amount = v as f64;
    let mut unit = 0;
    while amount >= base && unit < labels.len() - 1 { amount /= base; unit += 1; }
    format!("{amount:.1} {}", labels[unit])
}

pub(crate) fn thousands(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub(crate) fn file_type(ext: &str) -> String {
    let known = match ext {
        "(none)" => "(No Extension)",
        "exe" => "Application",
        "dll" => "Application extension",
        "sys" => "System file",
        "msi" => "Windows Installer Package",
        "zip" => "ZIP archive",
        "7z" => "7-Zip archive",
        "rar" => "RAR archive",
        "gz" => "GZip archive",
        "iso" => "Disc Image File",
        "png" => "PNG image",
        "jpg" | "jpeg" => "JPEG image",
        "gif" => "GIF image",
        "txt" => "Text Document",
        "pdf" => "PDF Document",
        "mp4" => "MP4 Video",
        "mp3" => "MP3 Audio",
        "lib" => "Object File Library",
        "pdb" => "Program Debug Database",
        "json" => "JSON File",
        "xml" => "XML Document",
        "js" => "JavaScript File",
        "ttf" => "TrueType font file",
        _ => "",
    };
    if known.is_empty() { format!("{} File", ext.to_uppercase()) } else { known.to_string() }
}

/// A long folder path shortened in the middle, so its drive and its last folders stay:
/// `C:\Users\…\target\qw\media`. The whole path goes in the tooltip.
pub(crate) fn mid_path(p: &str, max: usize) -> String {
    // 0: the column isn't measured yet, the text trims as before
    if max == 0 || p.chars().count() <= max {
        return p.to_string();
    }
    // the drive (or \\server\share) stays whole
    let (head, rest) = match p.strip_prefix("\\\\") {
        Some(unc) => {
            let mut it = unc.splitn(3, '\\');
            let (server, share, rest) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or(""));
            (format!("\\\\{server}\\{share}"), rest.to_string())
        }
        None => match p.split_once('\\') {
            Some((h, r)) => (h.to_string(), r.to_string()),
            None => return p.to_string(),
        },
    };
    let parts: Vec<&str> = rest.split('\\').filter(|s| !s.is_empty()).collect();
    let mut tail: Vec<&str> = Vec::new();
    let mut len = head.chars().count() + 3;
    for part in parts.iter().rev() {
        let n = part.chars().count() + 1;
        if !tail.is_empty() && len + n > max {
            break;
        }
        len += n;
        tail.push(part);
    }
    if tail.len() == parts.len() {
        return p.to_string();
    }
    tail.reverse();
    format!("{head}\\\u{2026}\\{}", tail.join("\\"))
}

/// A time as the user's local "2026-11-17 19:47" (the clock in the taskbar shows the same).
pub(crate) fn fmt_time(t: std::time::SystemTime) -> String {
    if let Some([y, m, d, h, mi, _]) = sprawl_core::local_parts(t) {
        return format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}");
    }
    fmt_utc(t)
}

/// The same in UTC, when Windows can't convert it.
fn fmt_utc(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

pub(crate) fn stamp() -> String {
    fmt_time(std::time::SystemTime::now()).replace(':', ".")
}

pub(crate) fn user_dir(sub: &str) -> PathBuf {
    let base = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join(sub);
    if d.is_dir() { d } else { base }
}

#[cfg(test)]
mod time_tests {
    #[test]
    fn times_are_shown_in_local_time() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_795_000_000);
        // the same clock time as the clean-up log (local), not UTC
        let local = sprawl_core::local_time(t);
        let shown = super::fmt_time(t);
        assert_eq!(shown, local[..16].replace('/', "-"));
        assert_eq!(super::fmt_utc(std::time::UNIX_EPOCH), "1970-01-01 00:00");
    }
}

#[cfg(test)]
mod mid_path_tests {
    use super::mid_path;

    #[test]
    fn a_long_path_keeps_its_drive_and_last_folders() {
        let p = r"C:\Users\someone\Documents\Craft Anything\wiztree64-2\target\qw\data\media";
        assert_eq!(mid_path(p, 200), p, "short enough: as it is");
        let s = mid_path(p, 40);
        assert!(s.starts_with("C:\\\u{2026}\\") && s.ends_with(r"\qw\data\media"), "{s}");
        assert!(s.chars().count() <= 40, "{s}");
        let unc = mid_path(r"\\nas\share\a\b\c\d\e\f\g\last", 24);
        assert!(unc.starts_with("\\\\nas\\share\\\u{2026}\\") && unc.ends_with(r"\last"), "{unc}");
        // one very long last folder is still kept whole (the column trims what is left)
        let one = mid_path(&format!(r"C:\a\b\{}", "x".repeat(50)), 20);
        assert!(one.ends_with(&"x".repeat(50)), "{one}");
    }
}
