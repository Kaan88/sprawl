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

/// Check `signature` (hex) over `manifest` with `key`, then read the manifest. Every file must
/// be one Sprawl knows ([`crate::FILES`]), listed once.
pub fn verify(manifest: &[u8], signature: &[u8], key: &[u8; 32]) -> Result<Manifest, String> {
    let key = VerifyingKey::from_bytes(key).map_err(|_| "the update key is not valid".to_string())?;
    let sig = unhex(std::str::from_utf8(signature).map_err(|_| "the signature is not text")?).ok_or("the signature is not hex")?;
    let sig = Signature::from_slice(&sig).map_err(|_| "the signature has the wrong length".to_string())?;
    key.verify_strict(manifest, &sig).map_err(|_| "the update's signature does not match; it was not applied".to_string())?;
    let m: Manifest = serde_json::from_slice(manifest).map_err(|e| format!("the update's list of files can't be read: {e}"))?;
    parse_version(&m.version).ok_or("the update has no valid version")?;
    let mut seen = Vec::new();
    for f in &m.files {
        if !crate::FILES.contains(&f.name.as_str()) || seen.contains(&f.name) {
            return Err(format!("the update lists a file Sprawl doesn't know: {}", f.name));
        }
        if f.sha256.len() != 64 || unhex(&f.sha256).is_none() {
            return Err(format!("the update has a bad hash for {}", f.name));
        }
        seen.push(f.name.clone());
    }
    if !seen.iter().any(|n| n == "sprawl.exe") {
        return Err("the update has no sprawl.exe".into());
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
        assert_eq!(verify(&m, &s, &k).unwrap().version, "0.2.0");
    }

    #[test]
    fn a_changed_manifest_fails() {
        let (_, s, k) = signed(GOOD);
        let changed = GOOD.replace("0.2.0", "9.9.9");
        assert!(verify(changed.as_bytes(), &s, &k).is_err());
    }

    #[test]
    fn another_key_fails() {
        let (m, s, _) = signed(GOOD);
        let other = SigningKey::from_bytes(&[8u8; 32]).verifying_key().to_bytes();
        assert!(verify(&m, &s, &other).is_err());
    }

    #[test]
    fn unknown_or_path_names_fail() {
        for name in ["..\\\\evil.exe", "evil.dll", "sprawl.exe\",\"x"] {
            let m = GOOD.replace("sprawl.exe", name);
            let (m, s, k) = signed(&m);
            assert!(verify(&m, &s, &k).is_err(), "{name}");
        }
    }

    #[test]
    fn the_zero_key_never_verifies() {
        let (m, s, _) = signed(GOOD);
        assert!(verify(&m, &s, &[0u8; 32]).is_err());
    }
}
