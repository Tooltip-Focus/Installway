// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Install-metadata directory paths.
//!
//! The uninstaller and its `installer_info.json` / `installer_manifest.json`
//! live OUTSIDE the application folder so a manual delete of the app folder
//! never orphans the Add/Remove Programs entry.
//!
//! Per-user install:    `%LOCALAPPDATA%\<publisher>\Uninstall\<product>\`
//! Machine-wide install: `%ProgramData%\<publisher>\Uninstall\<product>\`

use std::path::PathBuf;

/// Per-user uninstall data dir (`%LOCALAPPDATA%`). `None` if env var missing.
pub fn uninstall_dir(publisher: &str, product_id: &str) -> Option<PathBuf> {
    let base = dirs::data_local_dir()?;
    Some(
        base.join(sanitize_component(publisher))
            .join("Uninstall")
            .join(sanitize_component(product_id)),
    )
}

/// Machine-wide uninstall data dir (`%ProgramData%`). `None` if env var missing.
pub fn uninstall_dir_machine(publisher: &str, product_id: &str) -> Option<PathBuf> {
    let base = PathBuf::from(std::env::var("PROGRAMDATA").ok()?);
    Some(
        base.join(sanitize_component(publisher))
            .join("Uninstall")
            .join(sanitize_component(product_id)),
    )
}

/// Select the correct uninstall data dir based on whether admin is required.
pub fn uninstall_dir_for(publisher: &str, product_id: &str, machine: bool) -> Option<PathBuf> {
    if machine {
        uninstall_dir_machine(publisher, product_id)
    } else {
        uninstall_dir(publisher, product_id)
    }
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
