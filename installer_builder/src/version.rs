// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Build + embed a Win32 `VS_VERSIONINFO` (RT_VERSION) resource into the
//! output setup.exe so Explorer's Details tab shows FileVersion /
//! ProductVersion / Company / Description, and SmartScreen sees a complete,
//! reputable-looking binary.
//!
//! The blob is built by hand (no crate does this at runtime) following the
//! documented VS_VERSIONINFO layout: length-prefixed, 32-bit-aligned nodes.

use anyhow::{Context, Result, bail};
use std::path::Path;

const RT_VERSION: u16 = 16;
const LANG_US_EN: u16 = 0x0409;
const CODEPAGE_UNICODE: u16 = 0x04B0; // 1200

/// Parse "a.b.c.d" (any missing parts = 0) into four u16s.
fn parse_quad(v: &str) -> (u16, u16, u16, u16) {
    let mut it = v
        .split(['.', '-', '+'])
        .filter_map(|s| s.parse::<u16>().ok());
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

fn pad32(b: &mut Vec<u8>) {
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
}

fn wstr(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity((s.len() + 1) * 2);
    for u in s.encode_utf16() {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v.extend_from_slice(&0u16.to_le_bytes());
    v
}

/// One VS_VERSIONINFO node: `wLength | wValueLength | wType | szKey | pad |
/// value | (pad, child)*`. `wLength` is back-patched to the node's real size
/// (no trailing pad — the parent aligns between children).
fn node(key: &str, wtype: u16, wvaluelen: u16, value: &[u8], children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&0u16.to_le_bytes()); // wLength placeholder
    b.extend_from_slice(&wvaluelen.to_le_bytes());
    b.extend_from_slice(&wtype.to_le_bytes());
    b.extend_from_slice(&wstr(key));
    pad32(&mut b);
    if !value.is_empty() {
        b.extend_from_slice(value);
    }
    for c in children {
        pad32(&mut b); // align before each child (no-op if already aligned)
        b.extend_from_slice(c);
    }
    let len = b.len() as u16;
    b[0..2].copy_from_slice(&len.to_le_bytes());
    b
}

/// A `String` node (text key/value). wValueLength is in WORDS incl. null.
fn string_node(key: &str, val: &str) -> Vec<u8> {
    let v = wstr(val);
    let words = (v.len() / 2) as u16;
    node(key, 1, words, &v, &[])
}

fn fixed_file_info(ver: (u16, u16, u16, u16)) -> Vec<u8> {
    let (a, b, c, d) = ver;
    let ms = ((a as u32) << 16) | b as u32;
    let ls = ((c as u32) << 16) | d as u32;
    let mut v = Vec::with_capacity(52);
    let u32le = |x: u32, out: &mut Vec<u8>| out.extend_from_slice(&x.to_le_bytes());
    u32le(0xFEEF04BD, &mut v); // dwSignature
    u32le(0x0001_0000, &mut v); // dwStrucVersion
    u32le(ms, &mut v); // FileVersion MS
    u32le(ls, &mut v); // FileVersion LS
    u32le(ms, &mut v); // ProductVersion MS
    u32le(ls, &mut v); // ProductVersion LS
    u32le(0x3F, &mut v); // dwFileFlagsMask
    u32le(0, &mut v); // dwFileFlags
    u32le(0x0004_0004, &mut v); // dwFileOS = VOS_NT_WINDOWS32
    u32le(1, &mut v); // dwFileType = VFT_APP
    u32le(0, &mut v); // dwFileSubtype
    u32le(0, &mut v); // dwFileDateMS
    u32le(0, &mut v); // dwFileDateLS
    v
}

/// Build the full VS_VERSIONINFO blob.
pub fn build(product: &str, publisher: &str, version: &str, original_filename: &str) -> Vec<u8> {
    let quad = parse_quad(version);
    let desc = format!("{} Setup", product);

    // StringTable "040904B0"
    let table_key = format!("{:04X}{:04X}", LANG_US_EN, CODEPAGE_UNICODE);
    let strings = [
        string_node("CompanyName", publisher),
        string_node("FileDescription", &desc),
        string_node("FileVersion", version),
        string_node("InternalName", &desc),
        string_node("LegalCopyright", &format!("Copyright {}", publisher)),
        string_node("OriginalFilename", original_filename),
        string_node("ProductName", product),
        string_node("ProductVersion", version),
    ];
    let string_table = node(&table_key, 1, 0, &[], &strings);
    let string_file_info = node("StringFileInfo", 1, 0, &[], &[string_table]);

    // VarFileInfo / Translation = lang (WORD) + codepage (WORD)
    let mut translation = Vec::with_capacity(4);
    translation.extend_from_slice(&LANG_US_EN.to_le_bytes());
    translation.extend_from_slice(&CODEPAGE_UNICODE.to_le_bytes());
    let var = node(
        "Translation",
        0,
        translation.len() as u16,
        &translation,
        &[],
    );
    let var_file_info = node("VarFileInfo", 1, 0, &[], &[var]);

    // Root VS_VERSION_INFO with VS_FIXEDFILEINFO value.
    let ffi = fixed_file_info(quad);
    node(
        "VS_VERSION_INFO",
        0,
        ffi.len() as u16,
        &ffi,
        &[string_file_info, var_file_info],
    )
}

/// Build + write the version resource into `target`.
pub fn embed(target: &Path, product: &str, publisher: &str, version: &str) -> Result<()> {
    use windows::Win32::System::LibraryLoader::{
        BeginUpdateResourceW, EndUpdateResourceW, UpdateResourceW,
    };
    use windows::core::PCWSTR;

    let original = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "setup.exe".to_string());
    let blob = build(product, publisher, version, &original);

    let wide = crate::embed::wide_path(target);

    unsafe {
        let h = BeginUpdateResourceW(PCWSTR(wide.as_ptr()), false)
            .with_context(|| format!("BeginUpdateResource {}", target.display()))?;
        if h.is_invalid() {
            bail!(
                "BeginUpdateResource invalid handle for {}",
                target.display()
            );
        }
        UpdateResourceW(
            h,
            PCWSTR(RT_VERSION as usize as *const u16),
            PCWSTR(std::ptr::dangling::<u16>()), // resource id 1
            LANG_US_EN,
            Some(blob.as_ptr() as *const _),
            blob.len() as u32,
        )
        .context("UpdateResource RT_VERSION")?;
        EndUpdateResourceW(h, false).context("EndUpdateResource (version)")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quad_variants() {
        assert_eq!(parse_quad("1.2.3.4"), (1, 2, 3, 4));
        assert_eq!(parse_quad("1.0"), (1, 0, 0, 0));
        assert_eq!(parse_quad("2.5.1"), (2, 5, 1, 0));
        assert_eq!(parse_quad("not-a-version"), (0, 0, 0, 0));
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn build_has_fixed_info_signature_and_strings() {
        let b = build("Prod", "Pub", "1.2.3", "setup.exe");
        // VS_FIXEDFILEINFO signature 0xFEEF04BD, little-endian.
        assert!(
            contains(&b, &0xFEEF04BDu32.to_le_bytes()),
            "missing FIXEDFILEINFO sig"
        );
        // UTF-16 strings present.
        let utf16 =
            |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect() };
        assert!(contains(&b, &utf16("Prod")), "ProductName");
        assert!(contains(&b, &utf16("Pub")), "CompanyName");
        assert!(contains(&b, &utf16("1.2.3")), "version");
        assert!(contains(&b, &utf16("setup.exe")), "OriginalFilename");
        // Root wLength (first u16) equals total blob length.
        let root_len = u16::from_le_bytes([b[0], b[1]]) as usize;
        assert_eq!(root_len, b.len());
    }
}
