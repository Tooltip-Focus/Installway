// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Shared file/registry/shortcut cleanup helpers used by both stages.

use anyhow::{Context, Result};
use common::model::install_info::InstallInfo;
use common::model::manifest::Manifest;
use common::paths::{path_components, path_under};
use common::utils::{FS_RETRIES, FS_RETRY_DELAY, wide};
use std::fs;
use std::path::{Component, Path, PathBuf};
use windows::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RegDeleteTreeW};
use windows::core::PCWSTR;

/// The folder this uninstaller runs from (the data dir), not the app dir.
/// The app dir is read from `installer_info.json`.
pub fn self_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    exe.parent()
        .map(|p| p.to_path_buf())
        .context("locate uninstaller parent dir")
}

/// True if removing files under `dir` is blocked by OS permissions (a real ACL
/// wall) and elevation would help. `false` when the dir is missing or already writable.
pub fn perm_denied(dir: &Path) -> bool {
    if !dir.exists() {
        return false;
    }
    let probe = dir.join(".uninstall_write_test");
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            false
        }
        Err(e) => common::elevation::is_permission_denied(&e),
    }
}

/// What happened to a file we tried to remove.
enum Removal {
    /// Path didn't exist, nothing to do.
    Absent,
    /// Removed now.
    Removed,
    /// Still locked; queued for deletion on next reboot (elevated runs only).
    Pending,
    /// Still locked and could not be queued, an orphan will remain.
    Stuck,
}

/// Schedule `path` for deletion on next reboot. `MoveFileEx(MOVEFILE_DELAY_-
/// UNTIL_REBOOT)` records the pending rename under HKLM, so it only succeeds
/// when elevated; `false` otherwise.
pub(crate) fn schedule_delete_on_reboot(path: &Path) -> bool {
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
    for attempt in 0..FS_RETRIES {
        match fs::remove_file(path) {
            Ok(()) => return Removal::Removed,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Removal::Absent,
            // Nothing follows the last attempt but the reboot-time fallback,
            // so don't pay for a delay that no retry will use.
            Err(_) if attempt + 1 < FS_RETRIES => std::thread::sleep(FS_RETRY_DELAY),
            Err(_) => {}
        }
    }
    // Exhausted retries.
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

/// Env vars naming folders that must never be removed as an "app dir".
const PROTECTED_DIR_VARS: &[&str] = &[
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "SYSTEMROOT",
    "PUBLIC",
    "TEMP",
];

/// Sanity check before deleting under the recorded app dir: a corrupted or
/// tampered `installer_info.json` must not be able to point the uninstaller at
/// a drive root, a profile folder, or an ancestor of one.
pub fn safe_app_dir(dir: &Path) -> bool {
    // The component-wise checks below cannot resolve `..`.
    if !dir.is_absolute() || dir.components().any(|c| c == Component::ParentDir) {
        return false;
    }
    // A bare prefix (drive/share root) has no normal components.
    if path_components(dir).len() < 2 {
        return false;
    }
    // Equal to, or ancestor of, a protected folder.
    !PROTECTED_DIR_VARS
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .any(|protected| path_under(Path::new(&protected), dir))
}

/// Remove the directories the installer created (`created_dirs`) that are now
/// empty, deepest first so a created chain unwinds. Never recursive: a
/// directory still holding anything stays. Only the install dir, what is under
/// it and its ancestors are considered, each past [`safe_app_dir`].
pub fn remove_created_dirs(info: &InstallInfo) {
    let app_dir = Path::new(&info.install_dir);
    let mut dirs: Vec<&Path> = info
        .created_dirs
        .iter()
        .map(Path::new)
        .filter(|d| (path_under(d, app_dir) || path_under(app_dir, d)) && safe_app_dir(d))
        .collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for dir in dirs {
        if let Err(e) = fs::remove_dir(dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            common::log::info(format!("kept {} ({e})", dir.display()));
        }
    }
}

pub fn unregister(key: &str, machine: bool) {
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

    #[test]
    fn remove_created_dirs_only_removes_our_empty_dirs() {
        let d = tempfile::tempdir().unwrap();
        let parent = d.path().join("Acme");
        let app = parent.join("MyApp");
        let ours_empty = app.join("bin");
        let ours_full = app.join("plugins");
        let theirs_empty = app.join("before");
        for dir in [&ours_empty, &ours_full, &theirs_empty] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(ours_full.join("addin.dll"), b"x").unwrap();
        let outside = d.path().join("elsewhere");
        fs::create_dir_all(&outside).unwrap();
        // Spelled as if under the install dir, but `..` resolves outside it.
        let escaped = app.join(r"..\..\elsewhere");

        let info = InstallInfo {
            install_dir: app.to_string_lossy().into_owned(),
            created_dirs: [&parent, &app, &ours_empty, &ours_full, &outside, &escaped]
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            ..Default::default()
        };
        remove_created_dirs(&info);

        assert!(!ours_empty.exists()); // created and empty
        assert!(ours_full.join("addin.dll").exists()); // created but not empty
        assert!(theirs_empty.exists()); // there before the install
        assert!(app.exists() && parent.exists()); // still hold the kept dirs
        assert!(outside.exists()); // not under or above the install dir, nor via `..`

        // Once the foreign content is gone, the created chain unwinds fully.
        fs::remove_dir_all(&ours_full).unwrap();
        fs::remove_dir(&theirs_empty).unwrap();
        remove_created_dirs(&info);
        assert!(!parent.exists());
    }

    #[test]
    fn safe_app_dir_rejects_roots_and_protected_dirs() {
        assert!(!safe_app_dir(Path::new(r"C:\")));
        assert!(!safe_app_dir(Path::new(r"C:\Users")));
        assert!(!safe_app_dir(Path::new("relative\\path")));
        assert!(!safe_app_dir(Path::new(r"D:\Games\MyApp\..\..")));
        if let Ok(profile) = std::env::var("USERPROFILE") {
            assert!(!safe_app_dir(Path::new(&profile)));
        }
        if let Ok(pf) = std::env::var("PROGRAMFILES") {
            assert!(!safe_app_dir(Path::new(&pf)));
            assert!(safe_app_dir(&Path::new(&pf).join("MyApp")));
        }
        assert!(safe_app_dir(Path::new(r"D:\Games\MyApp")));
    }

    #[test]
    fn remove_one_payload_and_state_files() {
        let d = tempfile::tempdir().unwrap();
        let app = d.path();
        fs::create_dir_all(app.join("bin")).unwrap();
        fs::write(app.join("bin").join("a.exe"), b"x").unwrap();
        fs::write(app.join("version.json"), b"{}").unwrap();
        fs::write(app.join("installer_manifest.json"), b"{}").unwrap();

        // One call per manifest entry, the way `do_cleanup` drives it.
        remove_one_payload(&app.join("bin").join("a.exe"));
        assert!(!app.join("bin").join("a.exe").exists());
        // A payload the user already deleted is a no-op, not a failure.
        remove_one_payload(&app.join("bin").join("gone.exe"));

        assert_eq!(remove_app_state_files(app), 2);
        assert!(!app.join("version.json").exists());
        assert!(!app.join("installer_manifest.json").exists());
        // Re-running over an already-clean dir handles nothing.
        assert_eq!(remove_app_state_files(app), 0);
    }
}
