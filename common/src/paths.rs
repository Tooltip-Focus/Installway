// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Per-user install-metadata paths.
//!
//! The uninstaller and its `installer_info.json` / `installer_manifest.json`
//! live OUTSIDE the application folder, so deleting the app folder by hand
//! never orphans the Add/Remove Programs entry (mirrors InstallShield's
//! "Installation Information" folder, but per-user / no admin).
//!
//! Layout: `%LOCALAPPDATA%\<publisher>\Uninstall\<product>\`

use std::path::PathBuf;

/// Folder holding `uninstall.exe` + install metadata for one product, keyed by
/// the registry-safe `product_id` (stable across versions). `None` only if
/// `%LOCALAPPDATA%` can't be resolved. `sanitize_component` stays as
/// defense-in-depth — a validated id passes through unchanged.
pub fn uninstall_dir(publisher: &str, product_id: &str) -> Option<PathBuf> {
    let base = dirs::data_local_dir()?;
    Some(
        base.join(sanitize_component(publisher))
            .join("Uninstall")
            .join(sanitize_component(product_id)),
    )
}

/// Make a string safe to use as a single path component: drop characters
/// illegal on Windows, collapse whitespace, trim trailing dots/spaces, and
/// fall back to a placeholder if nothing usable remains.
pub fn sanitize_component(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']).trim();
    if trimmed.is_empty() {
        "Unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_basic() {
        assert_eq!(sanitize_component("My Co"), "My Co");
        assert_eq!(sanitize_component("a/b:c*?<>|\\"), "a_b_c______");
        assert_eq!(sanitize_component("  trailing. "), "trailing");
        assert_eq!(sanitize_component(""), "Unknown");
        assert_eq!(sanitize_component("..."), "Unknown");
    }

    #[test]
    fn uninstall_dir_has_expected_suffix() {
        if let Some(p) = uninstall_dir("Acme Inc", "My App") {
            let s = p.to_string_lossy().replace('/', "\\");
            assert!(s.ends_with(r"Acme Inc\Uninstall\My App"), "got {s}");
        }
    }

    #[test]
    fn uninstall_dir_sanitizes_illegal() {
        if let Some(p) = uninstall_dir("Ac/me", "My:App") {
            let s = p.to_string_lossy().replace('/', "\\");
            assert!(s.ends_with(r"Ac_me\Uninstall\My_App"), "got {s}");
        }
    }
}
