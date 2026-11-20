//! The release manifest: what a release holds, signed.

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileEntry {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub files: Vec<FileEntry>,
    /// The release's Setup (`Sprawl-Setup-<version>.exe`), signed with the rest: a copy
    /// installed for all users updates by running it. Not one of the program's files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<FileEntry>,
}

/// The Setup file name of `version`.
pub fn setup_name(version: &str) -> String {
    format!("Sprawl-Setup-{version}.exe")
}

/// `1.2.3` (a leading `v` is allowed) as numbers.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let mut it = v.split('.').map(|p| p.parse::<u64>().ok());
    let r = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(r)
}

/// Whether `candidate` is a newer version than `current`.
pub fn newer(candidate: &str, current: &str) -> bool {
    matches!((parse_version(candidate), parse_version(current)), (Some(a), Some(b)) if a > b)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// A file name a release may carry: a plain `.exe` or `.dll` name in the program's folder (no
/// folders, no leading dot), so a later release can add a file without stranding older copies.
pub fn valid_name(n: &str) -> bool {
    let lower = n.to_ascii_lowercase();
    !n.is_empty()
        && n.len() <= 80
        && !n.starts_with('.')
        && !n.contains("..")
        && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && (lower.ends_with(".exe") || lower.ends_with(".dll"))
}

/// Check `signature` (hex) over `manifest` with any of `keys`, then read the manifest. Every
/// file must have a [`valid_name`], listed once, and `sprawl.exe` must be there.
pub fn verify(manifest: &[u8], signature: &[u8], keys: &[[u8; 32]]) -> Result<Manifest, String> {
    let sig = unhex(std::str::from_utf8(signature).map_err(|_| "the signature is not text")?).ok_or("the signature is not hex")?;
    let sig = Signature::from_slice(&sig).map_err(|_| "the signature has the wrong length".to_string())?;
    let signed = keys.iter().filter_map(|k| VerifyingKey::from_bytes(k).ok()).any(|k| k.verify_strict(manifest, &sig).is_ok());
    if !signed {
        return Err("the update's signature does not match; it was not applied".into());
    }
    let m: Manifest = serde_json::from_slice(manifest).map_err(|e| format!("the update's list of files can't be read: {e}"))?;
    parse_version(&m.version).ok_or("the update has no valid version")?;
    let mut seen = Vec::new();
    for f in &m.files {
        if !valid_name(&f.name) || seen.iter().any(|s: &String| s.eq_ignore_ascii_case(&f.name)) {
            return Err(format!("the update lists a file Sprawl can't take: {}. Get the new version from github.com/{}/releases.", f.name, crate::REPO));
        }
        if f.sha256.len() != 64 || unhex(&f.sha256).is_none() {
            return Err(format!("the update has a bad hash for {}", f.name));
        }
        seen.push(f.name.clone());
    }
    if !seen.iter().any(|n| n == "sprawl.exe") {
        return Err("the update has no sprawl.exe".into());
    }
    if let Some(s) = &m.setup {
        if s.name != setup_name(&m.version) || s.sha256.len() != 64 || unhex(&s.sha256).is_none() || seen.iter().any(|n| n.eq_ignore_ascii_case(&s.name)) {
            return Err("the update's Setup entry is not valid".into());
        }
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn signed(m: &str) -> (Vec<u8>, Vec<u8>, [u8; 32]) {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let sig: String = sk.sign(m.as_bytes()).to_bytes().iter().map(|b| format!("{b:02x}")).collect();
        (m.as_bytes().to_vec(), sig.into_bytes(), sk.verifying_key().to_bytes())
    }

    const GOOD: &str = r#"{"version":"0.2.0","files":[{"name":"sprawl.exe","sha256":"0000000000000000000000000000000000000000000000000000000000000000","size":1}]}"#;

    #[test]
    fn versions() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("v1.0.0", "0.9.9"));
        assert!(!newer("0.2.0", "0.2.0"));
        assert!(!newer("0.10.0", "0.10.1"));
        assert!(!newer("junk", "0.1.0"));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn a_good_signature_passes() {
        let (m, s, k) = signed(GOOD);
        assert_eq!(verify(&m, &s, &[k]).unwrap().version, "0.2.0");
        // any key of the list will do
        assert!(verify(&m, &s, &[[1u8; 32], k]).is_ok());
    }

    #[test]
    fn a_changed_manifest_fails() {
        let (_, s, k) = signed(GOOD);
        let changed = GOOD.replace("0.2.0", "9.9.9");
        assert!(verify(changed.as_bytes(), &s, &[k]).is_err());
    }

    #[test]
    fn another_key_fails() {
        let (m, s, _) = signed(GOOD);
        let other = SigningKey::from_bytes(&[8u8; 32]).verifying_key().to_bytes();
        assert!(verify(&m, &s, &[other]).is_err());
    }

    #[test]
    fn unknown_or_path_names_fail() {
        for name in ["..\\\\evil.exe", "a\\\\evil.exe", "..evil.exe", ".evil.exe", "evil.txt", "evil.exe:ads.exe", "C:evil.exe", "sprawl.exe\",\"x"] {
            let m = GOOD.replace("sprawl.exe", name);
            let (m, s, k) = signed(&m);
            assert!(verify(&m, &s, &[k]).is_err(), "{name}");
        }
        // a new, plain file name is fine
        let m = GOOD.replace(r#"}]}"#, r#"},{"name":"helper.dll","sha256":"0000000000000000000000000000000000000000000000000000000000000000","size":1}]}"#);
        let (m, s, k) = signed(&m);
        assert!(verify(&m, &s, &[k]).is_ok());
    }

    #[test]
    fn a_setup_entry_must_be_this_versions_setup() {
        let with = |name: &str| GOOD.replace(r#"}]}"#, &format!(r#"}}],"setup":{{"name":"{name}","sha256":"0000000000000000000000000000000000000000000000000000000000000000","size":1}}}}"#));
        let (m, s, k) = signed(&with("Sprawl-Setup-0.2.0.exe"));
        assert_eq!(verify(&m, &s, &[k]).unwrap().setup.unwrap().name, "Sprawl-Setup-0.2.0.exe");
        for bad in ["Sprawl-Setup-0.1.0.exe", "sprawl.exe", "Sprawl-Setup-0.2.0.exe.exe"] {
            let (m, s, k) = signed(&with(bad));
            assert!(verify(&m, &s, &[k]).is_err(), "{bad}");
        }
        // a manifest without one (1.0.4 and before) still reads, and old readers ignore it
        let (m, s, k) = signed(GOOD);
        assert!(verify(&m, &s, &[k]).unwrap().setup.is_none());
    }

    #[test]
    fn the_zero_key_never_verifies() {
        let (m, s, _) = signed(GOOD);
        assert!(verify(&m, &s, &[[0u8; 32]]).is_err());
    }
}
