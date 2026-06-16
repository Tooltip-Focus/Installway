// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Shortcut (`.lnk`) path logic shared by the installer + uninstaller.
//!
//! Shortcuts are config-driven ([`crate::models::ShortcutEntry`]); the installer
//! resolves each entry's tokens to absolute paths, records them in
//! `installer_info.json`, and reconciles a changed set on upgrade. This module
//! holds the pure path helpers (no `.lnk` writing - that needs `mslnk`, an
//! installer-only dep): the standard location dirs, the resolved `.lnk` path of
//! an entry, and the stale-set diff for upgrades.

use crate::models::ShortcutEntry;
use std::collections::HashSet;
use std::path::PathBuf;

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
