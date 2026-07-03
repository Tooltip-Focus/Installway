// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Build a Win32 `VS_VERSIONINFO` (RT_VERSION) resource so the output
//! setup.exe's Explorer Details tab shows FileVersion / ProductVersion /
//! Company / Description, and SmartScreen sees a complete, reputable-looking
//! binary.

use editpe::constants::{CODE_PAGE_ID_EN_US, LANGUAGE_ID_EN_US};
use editpe::types::{FixedFileInfo, VersionU16, VersionU32};
use editpe::{VersionInfo, VersionStringTable};
use indexmap::IndexMap;

/// Parse "a.b.c.d" (any missing parts = 0) into four u16s. Stops at the first
/// non-numeric segment so semver prerelease/build metadata never leaks into a
/// version field ("1.2.3-beta.4" → 1.2.3.0, not 1.2.3.4).
fn parse_quad(v: &str) -> (u16, u16, u16, u16) {
    let mut it = v
        .split(['.', '-', '+'])
        .map_while(|s| s.parse::<u16>().ok());
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

/// Build the VS_VERSIONINFO resource.
pub fn build(
    product: &str,
    publisher: &str,
    version: &str,
    original_filename: &str,
) -> VersionInfo {
    let (a, b, c, d) = parse_quad(version);
    let ver = VersionU32 {
        major: ((a as u32) << 16) | b as u32,
        minor: ((c as u32) << 16) | d as u32,
    };
    let desc = format!("{product} Setup");

    let mut strings = IndexMap::default();
    strings.insert("CompanyName".to_string(), publisher.to_string());
    strings.insert("FileDescription".to_string(), desc.clone());
    strings.insert("FileVersion".to_string(), version.to_string());
    strings.insert("InternalName".to_string(), desc);
    strings.insert(
        "LegalCopyright".to_string(),
        format!("Copyright {publisher}"),
    );
    strings.insert(
        "OriginalFilename".to_string(),
        original_filename.to_string(),
    );
    strings.insert("ProductName".to_string(), product.to_string());
    strings.insert("ProductVersion".to_string(), version.to_string());

    VersionInfo {
        info: FixedFileInfo {
            file_version: ver,
            product_version: ver,
            ..Default::default()
        },
        strings: vec![VersionStringTable {
            key: format!("{LANGUAGE_ID_EN_US:04X}{CODE_PAGE_ID_EN_US:04X}"),
            strings,
        }],
        vars: vec![VersionU16 {
            major: LANGUAGE_ID_EN_US,
            minor: CODE_PAGE_ID_EN_US,
        }],
    }
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
        // Prerelease / build metadata stops the parse instead of leaking into
        // the next field.
        assert_eq!(parse_quad("1.2.3-beta.4"), (1, 2, 3, 0));
        assert_eq!(parse_quad("1.0.0+build.7"), (1, 0, 0, 0));
    }

    #[test]
    fn build_has_fixed_info_and_strings() {
        let vi = build("Prod", "Pub", "1.2.3", "setup.exe");
        assert_eq!(vi.info.signature, 0xFEEF04BD);
        assert_eq!(
            vi.info.file_version,
            VersionU32 {
                major: 1 << 16 | 2,
                minor: 3 << 16
            }
        );
        assert_eq!(vi.info.product_version, vi.info.file_version);

        let table = &vi.strings[0];
        assert_eq!(table.key, "040904B0");
        assert_eq!(table.strings["ProductName"], "Prod");
        assert_eq!(table.strings["CompanyName"], "Pub");
        assert_eq!(table.strings["FileVersion"], "1.2.3");
        assert_eq!(table.strings["OriginalFilename"], "setup.exe");
        assert_eq!(table.strings["FileDescription"], "Prod Setup");

        // Round-trips through the real VS_VERSIONINFO byte layout.
        let bytes = vi.build();
        let parsed = VersionInfo::parse(&bytes).unwrap();
        assert_eq!(parsed, vi);
    }
}
