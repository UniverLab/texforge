//! Minisign verification of the release `SHA256SUMS.txt`.
//!
//! From 1.0.0 the release workflow signs `SHA256SUMS.txt` with minisign
//! and uploads `SHA256SUMS.txt.minisig` next to it. `texforge update`
//! verifies that signature before it trusts the checksums: a 1.x release
//! whose sums are missing or unsigned is refused.

use anyhow::Result;

/// The minisign public key the release workflow signs `SHA256SUMS.txt` with.
pub(super) const PUBLIC_KEY: &str = "RWRDOZx1ameWKtWCHDVp3YmIsBiJszM2U4SJbCMkZ8/iSkW9Pi1yWzDD";

/// Verify `sums` against the `.minisig` `signature` text with `public_key`.
///
/// Every failure — an undecodable key, an undecodable signature, or a
/// signature that does not cover `sums` — is an error whose text starts
/// with `Signature verification failed for SHA256SUMS.txt`.
pub(super) fn verify_sums(public_key: &str, sums: &[u8], signature: &str) -> Result<()> {
    verify_inner(public_key, sums, signature).map_err(|error| {
        anyhow::anyhow!("Signature verification failed for SHA256SUMS.txt: {error:#}")
    })
}

fn verify_inner(public_key: &str, sums: &[u8], signature: &str) -> Result<()> {
    let key = minisign_verify::PublicKey::from_base64(public_key)?;
    let decoded = minisign_verify::Signature::decode(signature)?;
    key.verify(sums, &decoded, false)?;
    Ok(())
}
