// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Shared file/registry/shortcut cleanup helpers used by both stages.

use anyhow::{Context, Result};
use common::model::install_info::InstallInfo;
use common::model::manifest::Manifest;
use common::utils::{FS_RETRIES, FS_RETRY_DELAY, wide};
use std::fs;
use std::path::{Path, PathBuf};

/// The folder this uninstaller runs from (the data dir), not the app dir.
/// The app dir is read from `installer_info.json`.
pub fn self_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    exe.parent()
        .map(|p| p.to_path_buf())
        .context("locate uninstaller parent dir")
}

/// What happened to a file we tried to remove.
enum Removal {
    /// Path didn't exist - nothing to do.
    Absent,
    /// Removed now.
    Removed,
    /// Still locked; queued for deletion on next reboot (elevated runs only).
    Pending,
    /// Still locked and could not be queued - an orphan will remain.
    Stuck,
}

/// Schedule `path` for deletion on next reboot. `MoveFileEx(MOVEFILE_DELAY_-
/// UNTIL_REBOOT)` records the pending rename under HKLM, so it only succeeds
/// when elevated; `false` otherwise. Best-effort last resort.
pub(crate) fn schedule_delete_on_reboot(path: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
    use windows::core::PCWSTR;
    let path_w = wide(&path.to_string_lossy());
    unsafe {
        MoveFileExW(
            PCWSTR(path_w.as_ptr()),
            PCWSTR::null(),
            MOVEFILE_DELAY_UNTIL_REBOOT,
        )
        .is_ok()
    }
}

/// Remove a file, surviving transient AV/indexer locks via the shared retry
/// policy; if still locked, fall back to a reboot-time delete.
fn remove_file_robust(path: &Path) -> Removal {
    for _ in 0..FS_RETRIES {
        match fs::remove_file(path) {
            Ok(()) => return Removal::Removed,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Removal::Absent,
            Err(_) => std::thread::sleep(FS_RETRY_DELAY),
        }
    }
    // Exhausted retries — the file is persistently locked.
    if schedule_delete_on_reboot(path) {
        Removal::Pending
    } else {
        Removal::Stuck
    }
}

/// Remove state files written into the application directory by the installer
/// (`version.json`, `installer_manifest.json`). Returns the count handled
/// (removed now or queued for reboot).
pub fn remove_app_state_files(app_dir: &Path) -> usize {
    let mut count = 0;
    for extra in ["version.json", "installer_manifest.json"] {
        let p = app_dir.join(extra);
        if matches!(remove_file_robust(&p), Removal::Removed | Removal::Pending) {
            count += 1;
        }
    }
    count
}

pub fn read_info(install_dir: &Path) -> Result<InstallInfo> {
    let p = install_dir.join("installer_info.json");
    let s = fs::read_to_string(&p)
        .with_context(|| format!("read {} - is this an installed product?", p.display()))?;
    serde_json::from_str(&s).context("parse installer_info.json")
}

pub fn read_manifest(install_dir: &Path) -> Result<Manifest> {
    let p = install_dir.join("installer_manifest.json");
    let s = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    serde_json::from_str(&s).context("parse installer_manifest.json")
}

/// Robustly remove a single payload file, logging if it stays stuck. For the
/// interactive uninstall loop, which removes files one at a time for progress.
pub fn remove_one_payload(path: &Path) {
    if let Removal::Stuck = remove_file_robust(path) {
        common::log::warn(format!("could not remove (locked): {}", path.display()));
    }
}

/// Remove every payload file from `manifest`. Returns the count handled
/// (removed now or queued for reboot); stuck files are logged.
pub fn remove_payload_files(install_dir: &Path, manifest: &Manifest) -> usize {
    let mut count = 0;
    for rel in manifest.files.keys() {
        let p = install_dir.join(rel);
        match remove_file_robust(&p) {
            Removal::Removed | Removal::Pending => count += 1,
            Removal::Stuck => {
                common::log::warn(format!("could not remove (locked): {}", p.display()));
            }
            Removal::Absent => {}
        }
    }
    count
}

/// Remove the shortcuts recorded in `installer_info.json` (the resolved `.lnk`
/// paths the installer created).
pub fn remove_shortcuts(info: &InstallInfo) {
    for e in &info.shortcuts {
        let p = common::shortcuts::lnk_path(e);
        match remove_file_robust(&p) {
            // Already gone (the user deleted it) - nothing to do, not an error.
            Removal::Absent => {}
            Removal::Removed => {}
            Removal::Pending => common::log::warn(format!(
                "shortcut deletion will be delayed for next reboot: {}",
                p.display()
            )),
            Removal::Stuck => common::log::warn(format!(
                "could not remove shortcut (locked): {}",
                p.display()
            )),
        }
    }
}

/// Recursively remove every empty subdirectory of `install_dir` (bottom-up).
/// Leaves `install_dir` itself in place. Shares one implementation with the
/// installer's post-delete prune.
pub fn remove_empty_subdirs(install_dir: &Path) {
    common::utils::prune_empty_dirs(install_dir);
}

pub fn unregister(key: &str, machine: bool) {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RegDeleteTreeW};
    use windows::core::PCWSTR;
    let sub = format!(
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{}",
        key
    );
    let sub_w = wide(&sub);
    let root = if machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    };
    unsafe {
        let _ = RegDeleteTreeW(root, PCWSTR(sub_w.as_ptr()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::model::file_entry::FileEntry;
    use common::model::manifest::Manifest;

    #[test]
    fn remove_empty_subdirs_keeps_nonempty_and_root() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        fs::create_dir_all(root.join("empty1").join("empty2")).unwrap();
        fs::create_dir_all(root.join("keep")).unwrap();
        fs::write(root.join("keep").join("f.txt"), b"x").unwrap();

        remove_empty_subdirs(root);

        assert!(root.exists()); // root left in place
        assert!(!root.join("empty1").exists()); // empty tree removed
        assert!(root.join("keep").exists()); // non-empty kept
        assert!(root.join("keep").join("f.txt").exists());
    }

    #[test]
    fn remove_payload_and_state_files() {
        let d = tempfile::tempdir().unwrap();
        let app = d.path();
        fs::create_dir_all(app.join("bin")).unwrap();
        fs::write(app.join("bin").join("a.exe"), b"x").unwrap();
        fs::write(app.join("version.json"), b"{}").unwrap();
        fs::write(app.join("installer_manifest.json"), b"{}").unwrap();

        let mut files = std::collections::HashMap::new();
        files.insert(
            "bin/a.exe".to_string(),
            FileEntry {
                hash: "h".into(),
                size: 1,
                patch: None,
            },
        );
        let m = Manifest {
            version: "1.0".into(),
            exe: "bin/a.exe".into(),
            files,
            deleted_files: vec![],
            full_size: 0,
            total_patch_size: 0,
        };

        assert_eq!(remove_payload_files(app, &m), 1);
        assert!(!app.join("bin").join("a.exe").exists());
        assert_eq!(remove_app_state_files(app), 2);
        assert!(!app.join("version.json").exists());
        assert!(!app.join("installer_manifest.json").exists());
    }
}
