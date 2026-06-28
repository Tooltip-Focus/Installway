// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Shortcut (`.lnk`) path logic shared by the installer + uninstaller.
//!
//! Shortcuts are config-driven ([`crate::model::ShortcutEntry`]); the installer
//! resolves each entry's tokens to absolute paths, records them in
//! `installer_info.json`, and reconciles a changed set on upgrade. This module
//! holds the pure path helpers: the standard location dirs, the resolved `.lnk` path of
//! an entry, and the stale-set diff for upgrades.

use crate::model::shortcut_entry::ShortcutEntry;
use std::collections::HashSet;
use std::path::PathBuf;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath};

/// Per-user Start Menu Programs directory (no admin needed).
pub fn start_menu_dir() -> Option<PathBuf> {
    let mut p = dirs::data_dir()?;
    p.push(r"Microsoft\Windows\Start Menu\Programs");
    Some(p)
}

/// Per-user Desktop directory.
pub fn desktop_dir() -> Option<PathBuf> {
    dirs::desktop_dir()
}

/// All-Users (public) Desktop directory. Used for machine-wide installs so the
/// shortcut is visible to every user, not just the elevated account that ran the
/// installer. Needs admin to write.
pub fn common_desktop_dir() -> Option<PathBuf> {
    known_folder(&windows::Win32::UI::Shell::FOLDERID_PublicDesktop)
}

/// All-Users Start Menu Programs directory (machine-wide installs). Needs admin.
pub fn common_start_menu_dir() -> Option<PathBuf> {
    known_folder(&windows::Win32::UI::Shell::FOLDERID_CommonPrograms)
}

/// Resolve a Known Folder by id to a path. `None` if the shell can't resolve it.
fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let pwstr = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        if pwstr.is_null() {
            return None;
        }
        let s = pwstr.to_string().ok();
        CoTaskMemFree(Some(pwstr.0 as *const core::ffi::c_void));
        s.map(PathBuf::from)
    }
}

/// The `.lnk` file path of a resolved entry: `<dir>\<name>.lnk`.
pub fn lnk_path(e: &ShortcutEntry) -> PathBuf {
    PathBuf::from(&e.dir).join(format!("{}.lnk", e.name))
}

/// Entries present in `prior` but no longer in `current`, compared by resolved
/// `.lnk` path (case-insensitive - Windows paths are). These are the `.lnk`
/// files an upgrade deletes before (re)creating the current set.
pub fn stale(prior: &[ShortcutEntry], current: &[ShortcutEntry]) -> Vec<ShortcutEntry> {
    let key = |e: &ShortcutEntry| lnk_path(e).to_string_lossy().to_ascii_lowercase();
    let keep: HashSet<String> = current.iter().map(&key).collect();
    prior
        .iter()
        .filter(|e| !keep.contains(&key(e)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(dir: &str, name: &str) -> ShortcutEntry {
        ShortcutEntry {
            dir: dir.to_string(),
            name: name.to_string(),
            target: "app.exe".into(),
            args: String::new(),
        }
    }

    #[test]
    fn lnk_path_joins_dir_and_name() {
        let p = lnk_path(&entry(r"C:\Users\me\Desktop", "My App"));
        assert_eq!(
            p.to_string_lossy().replace('/', "\\"),
            r"C:\Users\me\Desktop\My App.lnk"
        );
    }

    #[test]
    fn stale_returns_only_dropped_paths() {
        let prior = [entry(r"C:\a", "x"), entry(r"C:\b", "y")];
        let current = [entry(r"C:\a", "x")];
        let got: Vec<String> = stale(&prior, &current)
            .iter()
            .map(|e| e.name.clone())
            .collect();
        assert_eq!(got, vec!["y".to_string()]);
    }

    #[test]
    fn stale_is_path_case_insensitive() {
        // Same resolved .lnk path, different case → kept, not stale.
        let prior = [entry(r"C:\App", "Tool")];
        let current = [entry(r"c:\app", "tool")];
        assert!(stale(&prior, &current).is_empty());
    }

    #[test]
    fn stale_edges_empty_inputs() {
        assert!(stale(&[], &[entry(r"C:\a", "x")]).is_empty());
        assert_eq!(stale(&[entry(r"C:\a", "x")], &[]).len(), 1);
    }
}
