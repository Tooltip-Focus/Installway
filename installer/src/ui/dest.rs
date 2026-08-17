// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Destination-folder policy shared by the installer UIs.

use common::model::install_dir_restriction::InstallDirRestriction;
use std::path::{Path, PathBuf};

/// True when `path` is an existing directory holding at least one file at any
/// depth. A missing path, an empty folder, or a tree of empty sub-directories
/// are all safe to install into.
pub(super) fn dir_has_entries(path: &str) -> bool {
    let p = path.trim();
    if p.is_empty() {
        return false;
    }
    dir_has_files_recursive(Path::new(p))
}

fn dir_has_files_recursive(path: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    for entry in entries.flatten() {
        let ep = entry.path();
        if ep.is_file() {
            return true;
        }
        if ep.is_dir() && dir_has_files_recursive(&ep) {
            return true;
        }
    }
    false
}

fn norm_dir(p: &str) -> String {
    p.trim()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

pub(super) fn same_dir(path: &str, default_path: &str) -> bool {
    !path.trim().is_empty() && norm_dir(path) == norm_dir(default_path)
}

/// Whether a non-empty destination must be refused.
///
/// The guard only applies to a fresh install where the user picks the folder.
/// On a fixed path (update, patch, or build-time `skip_path`) the destination
/// legitimately holds the product's own files. `restriction` relaxes it further
/// for apps installing over a legacy InstallShield/MSI layout.
pub(super) fn should_block_nonempty(
    restriction: InstallDirRestriction,
    skip_path: bool,
    default_path: &str,
    path: &str,
) -> bool {
    if skip_path || !dir_has_entries(path) {
        return false;
    }
    match restriction {
        InstallDirRestriction::Bypass => false,
        InstallDirRestriction::DefaultDirOnly => !same_dir(path, default_path),
        InstallDirRestriction::Enforce => true,
    }
}

/// Append `product` to a browsed parent folder, unless it is already the leaf.
pub(super) fn with_product_subdir(picked: &str, product: &str) -> String {
    let product = product.trim();
    if product.is_empty() {
        return picked.to_string();
    }
    let pb = PathBuf::from(picked);
    let already = pb
        .file_name()
        .map(|n| n.eq_ignore_ascii_case(product))
        .unwrap_or(false);
    if already {
        picked.to_string()
    } else {
        pb.join(product).to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{dir_has_entries, same_dir, should_block_nonempty, with_product_subdir};
    use common::model::install_dir_restriction::InstallDirRestriction::{
        Bypass, DefaultDirOnly, Enforce,
    };
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn dir_has_entries_false_for_missing_path() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(!dir_has_entries(&missing.to_string_lossy()));
    }

    #[test]
    fn dir_has_entries_false_for_empty_or_blank() {
        let dir = tempdir().unwrap();
        assert!(!dir_has_entries(&dir.path().to_string_lossy()));
        assert!(!dir_has_entries(""));
        assert!(!dir_has_entries("   "));
    }

    #[test]
    fn dir_has_entries_true_when_populated() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), b"x").unwrap();
        assert!(dir_has_entries(&dir.path().to_string_lossy()));
    }

    #[test]
    fn dir_has_entries_false_when_only_empty_subdirs() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("subdir")).unwrap();
        assert!(!dir_has_entries(&dir.path().to_string_lossy()));
    }

    #[test]
    fn dir_has_entries_true_when_file_nested_in_subdir() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("subdir");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("file.txt"), b"x").unwrap();
        assert!(dir_has_entries(&dir.path().to_string_lossy()));
    }

    #[test]
    fn fresh_install_blocks_on_nonempty_folder() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), b"x").unwrap();
        let path = dir.path().to_string_lossy();
        // skip_path = false: the user picked this folder, it must be empty.
        assert!(should_block_nonempty(Enforce, false, "", &path));
    }

    #[test]
    fn update_does_not_block_on_nonempty_folder() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("app.exe"), b"x").unwrap();
        let path = dir.path().to_string_lossy();
        assert!(!should_block_nonempty(Enforce, true, "", &path));
    }

    #[test]
    fn fresh_install_allows_empty_folder() {
        let dir = tempdir().unwrap();
        assert!(!should_block_nonempty(
            Enforce,
            false,
            "",
            &dir.path().to_string_lossy()
        ));
    }

    #[test]
    fn bypass_allows_any_nonempty_folder() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("legacy.dll"), b"x").unwrap();
        let path = dir.path().to_string_lossy();
        assert!(!should_block_nonempty(Bypass, false, "C:\\Other", &path));
    }

    #[test]
    fn default_dir_only_allows_default_blocks_others() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("legacy.dll"), b"x").unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        assert!(!should_block_nonempty(DefaultDirOnly, false, &path, &path));
        // A different non-empty folder stays blocked.
        assert!(should_block_nonempty(
            DefaultDirOnly,
            false,
            "C:\\Some\\Other\\Dir",
            &path
        ));
    }

    #[test]
    fn same_dir_normalizes_slashes_case_and_trailing_sep() {
        assert!(same_dir("C:/Program Files/App", "c:\\program files\\app\\"));
        assert!(!same_dir("C:\\App", "C:\\Other"));
        assert!(!same_dir("", "C:\\App"));
    }

    #[test]
    fn product_subdir_appended_unless_already_there() {
        assert_eq!(with_product_subdir(r"C:\Apps", "My App"), r"C:\Apps\My App");
        assert_eq!(
            with_product_subdir(r"C:\Apps\my app", "My App"),
            r"C:\Apps\my app"
        );
        assert_eq!(with_product_subdir(r"C:\Apps", ""), r"C:\Apps");
    }
}
