// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! PakLib payload archives.
//!
//! Format detection is always available, so a build without PakLib can still
//! reject a PakLib payload with a clear message. Reading and writing need the
//! `paklib` feature, which links the native library.

#[cfg(feature = "paklib")]
mod native;
#[cfg(feature = "paklib")]
pub use native::*;

/// First bytes of every PakLib archive.
const MAGIC: &[u8] = b"PAKLIB1\0";

/// Whether `bytes` start like a PakLib archive.
pub fn is_pak(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_magic() {
        assert!(is_pak(b"PAKLIB1\0rest"));
        assert!(!is_pak(b"PK\x03\x04"));
        assert!(!is_pak(b"PAK"));
    }
}
