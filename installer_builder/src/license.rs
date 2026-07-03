// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! License-file decoding + a title preview for log lines.

use anyhow::{Result, bail};

/// Decode a license file to UTF-8 with Windows line endings.
pub(crate) fn decode_license(bytes: &[u8]) -> Result<String> {
    // Reject UTF-16 BOMs up front.
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        bail!("not UTF-8 (looks like UTF-16); re-save the license file as UTF-8");
    }
    // Strip an optional UTF-8 BOM so it does not show as a stray glyph.
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);

    let text = std::str::from_utf8(bytes)
        .map_err(|e| anyhow::anyhow!("not valid UTF-8: {e}; re-save the license file as UTF-8"))?;

    // Normalize every line-ending style (CRLF, LF, lone CR) to CRLF.
    Ok(text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n"))
}

/// First non-empty line of `s`, truncated to 60 chars — used for log preview.
pub(crate) fn trimmed_title(s: &str) -> String {
    let line = s
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() > 60 {
        format!("{}...", line.chars().take(60).collect::<String>())
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimmed_title_first_line_truncated() {
        assert_eq!(trimmed_title("\n\nHello\nworld"), "Hello");
        let long = "x".repeat(80);
        let t = trimmed_title(&long);
        assert!(t.ends_with("...") && t.chars().count() == 63);
    }

    #[test]
    fn decode_license_normalizes_lf_to_crlf() {
        assert_eq!(decode_license(b"a\nb\n").unwrap(), "a\r\nb\r\n");
    }

    #[test]
    fn decode_license_keeps_crlf_no_double() {
        assert_eq!(decode_license(b"a\r\nb").unwrap(), "a\r\nb");
    }

    #[test]
    fn decode_license_normalizes_lone_cr() {
        assert_eq!(decode_license(b"a\rb").unwrap(), "a\r\nb");
    }

    #[test]
    fn decode_license_strips_utf8_bom() {
        assert_eq!(decode_license(b"\xEF\xBB\xBFhi").unwrap(), "hi");
    }

    #[test]
    fn decode_license_rejects_utf16() {
        assert!(decode_license(&[0xFF, 0xFE, 0x41, 0x00]).is_err());
    }

    #[test]
    fn decode_license_rejects_invalid_utf8() {
        assert!(decode_license(&[0xFF, 0x28, 0x80]).is_err());
    }
}
