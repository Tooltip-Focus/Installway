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

use std::path::{Path, PathBuf};

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

/// Well-known machine-wide roots: an install under one of these is shared by all
/// users. Resolved from env vars so non-`C:` Windows installs still match.
const MACHINE_ROOT_VARS: &[&str] = &[
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "PROGRAMDATA",
    "SYSTEMROOT",
];

/// True if `dir` sits under a well-known machine-wide location (Program Files,
/// ProgramData, the Windows dir), as opposed to a per-user profile location.
///
/// Drives whether an install records itself machine-wide (`%ProgramData%` + HKLM)
/// or per-user (`%LOCALAPPDATA%` + HKCU). Unknown locations (e.g. `D:\Apps`) are
/// per-user here; an install that needed elevation is flagged machine-wide by the
/// caller regardless, so an ACL'd custom dir is still handled.
pub fn is_machine_location(dir: &Path) -> bool {
    MACHINE_ROOT_VARS
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .filter(|root| !root.is_empty())
        .any(|root| path_under(dir, Path::new(&root)))
}

/// Case-insensitive, component-wise "is `dir` inside `root` (or equal)". Compares
/// whole components so `C:\Program Files Xtra` is not treated as under
/// `C:\Program Files`.
fn path_under(dir: &Path, root: &Path) -> bool {
    let norm = |p: &Path| -> Vec<String> {
        p.components()
            .filter_map(|c| match c {
                std::path::Component::Normal(s) => Some(s.to_string_lossy().to_lowercase()),
                std::path::Component::Prefix(p) => {
                    Some(p.as_os_str().to_string_lossy().to_lowercase())
                }
                _ => None,
            })
            .collect()
    };
    let (d, r) = (norm(dir), norm(root));
    !r.is_empty() && d.len() >= r.len() && d[..r.len()] == r[..]
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

    #[test]
    fn path_under_is_component_wise_and_case_insensitive() {
        let root = Path::new(r"C:\Program Files");
        assert!(path_under(Path::new(r"C:\Program Files\MyApp"), root));
        assert!(path_under(Path::new(r"c:\program files\myapp\bin"), root));
        assert!(path_under(root, root)); // equal counts as under
        // Sibling that merely shares a prefix string must not match.
        assert!(!path_under(Path::new(r"C:\Program Files Xtra\MyApp"), root));
        // Per-user location is not under a machine root.
        assert!(!path_under(
            Path::new(r"C:\Users\bob\AppData\Local\Programs\MyApp"),
            root
        ));
    }

    #[test]
    fn is_machine_location_matches_program_files() {
        if let Ok(pf) = std::env::var("PROGRAMFILES") {
            assert!(is_machine_location(&PathBuf::from(pf).join("MyApp")));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            assert!(!is_machine_location(
                &PathBuf::from(local).join("Programs").join("MyApp")
            ));
        }
    }
}
