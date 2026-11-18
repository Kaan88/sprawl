//! Evidence that files and folders were opened (A1): Explorer's Recent items, one shortcut for
//! each file or folder opened from Explorer or a program's Open box, its own date moved each
//! time it is opened again. Read only; from each shortcut only its local target path is taken
//! (a network target, or one without a path, is skipped). It shows use, never disuse: what was
//! opened another way isn't there, and Windows keeps only so many.

use std::path::PathBuf;
use std::time::SystemTime;

/// The most shortcuts read.
const MOST: usize = 5000;

/// (lower-case target path without a trailing `\`, when it was last opened) for each shortcut in
/// this account's Recent items.
pub fn recent_items() -> Vec<(String, SystemTime)> {
    let Some(dir) = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Recent")) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten().take(MOST) {
        let p = e.path();
        if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
            continue;
        }
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() || m.len() > 64 << 10 {
            continue;
        }
        let (Ok(t), Ok(bytes)) = (m.modified(), std::fs::read(&p)) else { continue };
        if let Some(target) = lnk_target(&bytes) {
            out.push((target.to_lowercase().trim_end_matches('\\').to_string(), t));
        }
    }
    out
}

/// The newest time something at `path_lc` or inside it (lower case, no trailing `\`) was opened.
pub fn opened_in(items: &[(String, SystemTime)], path_lc: &str) -> Option<SystemTime> {
    let inside = format!("{}\\", path_lc.trim_end_matches('\\'));
    items.iter().filter(|(p, _)| p == path_lc.trim_end_matches('\\') || p.starts_with(&inside)).map(|&(_, t)| t).max()
}

fn u16_at(b: &[u8], i: usize) -> Option<usize> {
    b.get(i..i + 2).map(|x| u16::from_le_bytes([x[0], x[1]]) as usize)
}

fn u32_at(b: &[u8], i: usize) -> Option<usize> {
    b.get(i..i + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize)
}

/// A NUL-ended string of plain ASCII at `at` (the ANSI one's code page isn't known here: one
/// with other characters is left out rather than read wrong).
fn ascii(b: &[u8], at: usize) -> Option<String> {
    let s = b.get(at..)?;
    let end = s.iter().position(|&c| c == 0)?;
    s[..end].is_ascii().then(|| String::from_utf8_lossy(&s[..end]).into_owned())
}

/// A NUL-ended UTF-16 string at `at`.
fn wide(b: &[u8], at: usize) -> Option<String> {
    let s = b.get(at..)?;
    let units: Vec<u16> = s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
    String::from_utf16(&units).ok()
}

/// The local path a shell link (`.lnk`, MS-SHLLINK) points at, from its LinkInfo: the local
/// base path and the common path suffix, the Unicode ones when present. None for anything else.
pub(crate) fn lnk_target(b: &[u8]) -> Option<String> {
    // header size, then the link's flags
    if u32_at(b, 0)? != 0x4C {
        return None;
    }
    let flags = u32_at(b, 0x14)?;
    let mut i = 0x4C;
    // HasLinkTargetIDList: skipped
    if flags & 1 != 0 {
        i += 2 + u16_at(b, i)?;
    }
    // HasLinkInfo
    if flags & 2 == 0 {
        return None;
    }
    let info = b.get(i..)?;
    let info = info.get(..u32_at(info, 0)?)?;
    let header = u32_at(info, 4)?;
    // VolumeIDAndLocalBasePath
    if u32_at(info, 8)? & 1 == 0 {
        return None;
    }
    let unicode = if header >= 0x24 { Some((u32_at(info, 0x1C)?, u32_at(info, 0x20)?)) } else { None };
    let (base, suffix) = match unicode {
        Some((bu, su)) if bu != 0 => (wide(info, bu)?, if su != 0 { wide(info, su)? } else { String::new() }),
        _ => (ascii(info, u32_at(info, 0x10)?)?, ascii(info, u32_at(info, 0x18)?)?),
    };
    let mut p = base;
    if !suffix.is_empty() {
        if !p.ends_with('\\') {
            p.push('\\');
        }
        p.push_str(&suffix);
    }
    let bytes = p.as_bytes();
    (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' && !p.contains(['\0', '\n'])).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A link with only a LinkInfo: an ANSI base path and suffix, and the Unicode ones when given.
    fn link(base: &str, suffix: &str, unicode: Option<(&str, &str)>) -> Vec<u8> {
        let mut b = vec![0u8; 0x4C];
        b[0] = 0x4C;
        b[0x14] = 2 | 0x80; // HasLinkInfo, IsUnicode
        let header: usize = if unicode.is_some() { 0x24 } else { 0x1C };
        let mut body = Vec::new();
        let base_at = header;
        body.extend(base.as_bytes());
        body.push(0);
        let suffix_at = header + body.len();
        body.extend(suffix.as_bytes());
        body.push(0);
        let mut uni = (0usize, 0usize);
        if let Some((ub, us)) = unicode {
            uni.0 = header + body.len();
            body.extend(ub.encode_utf16().flat_map(|u| u.to_le_bytes()));
            body.extend([0, 0]);
            uni.1 = header + body.len();
            body.extend(us.encode_utf16().flat_map(|u| u.to_le_bytes()));
            body.extend([0, 0]);
        }
        let mut info = vec![0u8; header];
        let put = |v: &mut Vec<u8>, at: usize, x: usize| v[at..at + 4].copy_from_slice(&(x as u32).to_le_bytes());
        put(&mut info, 0, header + body.len());
        put(&mut info, 4, header);
        put(&mut info, 8, 1);
        put(&mut info, 0x10, base_at);
        put(&mut info, 0x18, suffix_at);
        if unicode.is_some() {
            put(&mut info, 0x1C, uni.0);
            put(&mut info, 0x20, uni.1);
        }
        info.extend(body);
        b.extend(info);
        b
    }

    #[test]
    fn link_targets_are_read() {
        assert_eq!(lnk_target(&link(r"D:\Films\a.mkv", "", None)).as_deref(), Some(r"D:\Films\a.mkv"));
        assert_eq!(lnk_target(&link(r"D:\", r"Films\a.mkv", None)).as_deref(), Some(r"D:\Films\a.mkv"));
        assert_eq!(lnk_target(&link(r"D:\F?lms", "", Some((r"D:\Fïlms\été.mkv", "")))).as_deref(), Some(r"D:\Fïlms\été.mkv"));
        // a non-ASCII ANSI path without the Unicode one: left out
        let mut l = link("D:\\Films", "", None);
        let at = l.iter().position(|&c| c == b'F').unwrap();
        l[at] = 0xE9;
        assert_eq!(lnk_target(&l), None);
        // not a link, cut short, a network path
        assert_eq!(lnk_target(b"hello"), None);
        assert_eq!(lnk_target(&link(r"D:\a", "", None)[..0x50]), None);
        assert_eq!(lnk_target(&link(r"\\server\share\a", "", None)), None);
    }

    #[test]
    fn opened_inside_a_folder() {
        let t = |s| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s);
        let items = vec![(r"d:\films\a.mkv".to_string(), t(10)), (r"d:\films\old\b.mkv".to_string(), t(5)), (r"d:\filmsx\c.mkv".to_string(), t(20))];
        assert_eq!(opened_in(&items, r"d:\films"), Some(t(10)));
        assert_eq!(opened_in(&items, r"d:\films\old"), Some(t(5)));
        assert_eq!(opened_in(&items, r"d:\films\a.mkv"), Some(t(10)));
        assert_eq!(opened_in(&items, r"d:\music"), None);
    }

    /// This account's own Recent items parse (run by hand: it reads the real folder).
    #[test]
    #[ignore]
    fn this_accounts_recent_items() {
        let items = recent_items();
        let n = std::fs::read_dir(PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(r"Microsoft\Windows\Recent")).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "lnk")).count();
        eprintln!("{} of {n} shortcuts read; first: {:?}", items.len(), items.iter().take(5).map(|(p, _)| p.len()).collect::<Vec<_>>());
        assert!(n == 0 || !items.is_empty());
    }
}
