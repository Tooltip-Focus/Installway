// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Load / validate the ed25519 signing (private) and public keys from hex.

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use std::fs;
use std::path::Path;

/// Load a signing key from a hex file.
pub(crate) fn load_signing_key(path: &Path) -> Result<SigningKey> {
    let hex_data = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    parse_signing_key_hex(hex_data.trim())
        .with_context(|| format!("invalid private key in {}", path.display()))
}

/// Parse a 32-byte ed25519 signing key from a hex string.
pub(crate) fn parse_signing_key_hex(hex: &str) -> Result<SigningKey> {
    let bytes = hex::decode(hex.trim()).context("decode hex private key")?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("private key must be 32 bytes"))?;
    Ok(SigningKey::from_bytes(&arr))
}

/// Load a public key from a hex file, returning the validated hex string.
pub(crate) fn load_pub_key_hex(path: &Path) -> Result<String> {
    let hex_data = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    validate_pub_key_hex(hex_data.trim())
        .with_context(|| format!("invalid public key in {}", path.display()))
}

/// Validate that `hex` decodes to a 32-byte public key; returns the trimmed hex.
pub(crate) fn validate_pub_key_hex(hex: &str) -> Result<String> {
    let hex = hex.trim().to_string();
    let bytes = hex::decode(&hex).context("decode hex public key")?;
    if bytes.len() != 32 {
        bail!("public key must be 32 bytes");
    }
    Ok(hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_key_literal_valid() {
        let hex = "a".repeat(64); // 32 bytes of 0xaa
        assert!(parse_signing_key_hex(&hex).is_ok());
    }

    #[test]
    fn signing_key_literal_with_whitespace() {
        let hex = format!("  {}\n", "b".repeat(64));
        assert!(parse_signing_key_hex(&hex).is_ok());
    }

    #[test]
    fn signing_key_literal_bad_hex() {
        assert!(parse_signing_key_hex("not-hex-at-all").is_err());
    }

    #[test]
    fn signing_key_literal_wrong_length() {
        let hex = "ab".repeat(16); // 16 bytes, not 32
        assert!(parse_signing_key_hex(&hex).is_err());
    }

    #[test]
    fn pub_key_literal_valid() {
        let hex = "cc".repeat(32);
        let result = validate_pub_key_hex(&hex).unwrap();
        assert_eq!(result, hex);
    }

    #[test]
    fn pub_key_literal_with_whitespace() {
        let hex = format!("  {}\n", "dd".repeat(32));
        assert!(validate_pub_key_hex(&hex).is_ok());
    }

    #[test]
    fn pub_key_literal_bad_hex() {
        assert!(validate_pub_key_hex("not-hex").is_err());
    }

    #[test]
    fn pub_key_literal_wrong_length() {
        let hex = "ab".repeat(16); // 16 bytes, not 32
        assert!(validate_pub_key_hex(&hex).is_err());
    }
}
