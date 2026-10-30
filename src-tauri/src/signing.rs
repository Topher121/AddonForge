//! Signature and checksum checks for self-updates.
//!
//! build.ps1 writes `latest.json` with the SHA-256 of the new exe, and
//! scripts/sign-release.js signs that file with the maintainer's private key
//! (kept off the PC's repo and out of chat). The app only holds the public
//! half, so a copy of the release that the signature does not match is refused.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

/// Public half of the release signing key, 32 bytes. `None` until the
/// maintainer has generated the key pair. With no key the app refuses every
/// update, which is deliberate: an unsigned update is never installed.
pub const RELEASE_PUBLIC_KEY: Option<[u8; 32]> = Some([
    0xe8, 0x38, 0x9e, 0x0f, 0x15, 0x90, 0xe3, 0xb5, 0xaa, 0xda, 0x36, 0x38, 0xae, 0x6e, 0x97, 0xef, 0x13, 0x3d, 0xb5, 0xe7, 0xe2, 0x29, 0x64, 0x02, 0x8d, 0x1f, 0xe1, 0x2d, 0xa3, 0xcd, 0xee, 0xad,
]);

pub fn verified_key() -> Result<[u8; 32]> {
    RELEASE_PUBLIC_KEY.ok_or_else(|| anyhow!("updates are turned off in this build (no release signing key)"))
}

/// `sig_b64` is the base64 Ed25519 signature over the exact bytes of `manifest`.
pub fn verify_manifest(manifest: &[u8], sig_b64: &str, key: &[u8; 32]) -> Result<()> {
    let key = VerifyingKey::from_bytes(key).context("bad release public key")?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(sig_b64.trim())
        .context("update signature is not base64")?;
    let sig = Signature::from_slice(&raw).context("update signature has the wrong length")?;
    key.verify(manifest, &sig).map_err(|_| anyhow!("update signature does not match"))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn check_sha256(bytes: &[u8], expected: &str) -> Result<()> {
    if expected.is_empty() {
        bail!("update has no checksum, refusing to install it");
    }
    if sha256_hex(bytes) != expected.to_ascii_lowercase() {
        bail!("downloaded update does not match its signed checksum");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn keys() -> (SigningKey, [u8; 32]) {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        (sk, pk)
    }

    fn b64(sig: &Signature) -> String {
        base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
    }

    #[test]
    fn accepts_a_manifest_signed_with_the_release_key() {
        let (sk, pk) = keys();
        let manifest = br#"{"version":"0.4.0","build":27}"#;
        assert!(verify_manifest(manifest, &b64(&sk.sign(manifest)), &pk).is_ok());
    }

    #[test]
    fn refuses_a_changed_manifest_or_another_key() {
        let (sk, pk) = keys();
        let manifest = br#"{"version":"0.4.0","build":27}"#;
        let sig = b64(&sk.sign(manifest));
        assert!(verify_manifest(br#"{"version":"0.4.0","build":28}"#, &sig, &pk).is_err());
        let other = SigningKey::from_bytes(&[9u8; 32]).verifying_key().to_bytes();
        assert!(verify_manifest(manifest, &sig, &other).is_err());
        assert!(verify_manifest(manifest, "not base64!", &pk).is_err());
    }

    #[test]
    fn checksum_must_match_and_cannot_be_empty() {
        let bytes = b"fake exe bytes";
        let good = sha256_hex(bytes);
        assert!(check_sha256(bytes, &good).is_ok());
        assert!(check_sha256(bytes, &good.to_uppercase()).is_ok());
        assert!(check_sha256(b"changed", &good).is_err());
        assert!(check_sha256(bytes, "").is_err());
    }
}
