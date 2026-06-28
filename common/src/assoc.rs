// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! File-type associations under `Software\Classes`, in `HKCU` (per-user, no
//! admin) or `HKLM` (machine-wide install, needs admin).
//!
//! Layout written per association (extension `.myx`, product `MyApp`):
//! ```text
//! Software\Classes\.myx                       (default) = "MyApp.myx"
//! Software\Classes\MyApp.myx                  (default) = "<description>"
//! Software\Classes\MyApp.myx\DefaultIcon      (default) = "<exe>",0
//! Software\Classes\MyApp.myx\shell\open\command (default) = "<exe>" "%1"
//! ```

use crate::model::file_assoc::FileAssoc;
use crate::registry::{close, create_registry_key, delete_tree, read_default, set_default};
use std::collections::HashSet;

use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

/// Deterministic ProgID for a (product_id, extension) pair, e.g.
/// `("MyApp", ".myx") -> "MyApp.myx"`. `product_id` is the registry-safe id
/// (validated at build time); the filter stays as defense-in-depth and to keep
/// pre-split records (which passed the display name here) resolving the same.
pub fn progid_for(product_id: &str, ext: &str) -> String {
    let prod: String = product_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .collect();
    let e = ext.trim_start_matches('.');
    format!("{}.{}", prod, e)
}

/// Normalize an extension to a leading-dot, lower-case form.
pub fn normalize_ext(ext: &str) -> String {
    let e = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    format!(".{}", e)
}

/// Associations present in `prior` but no longer in `current`, compared by
/// normalized extension. On upgrade these should be `unregister`ed so a changed
/// association list never leaves orphaned ProgIDs / extension handlers behind.
pub fn stale(prior: &[FileAssoc], current: &[FileAssoc]) -> Vec<FileAssoc> {
    let keep: HashSet<String> = current.iter().map(|a| normalize_ext(&a.ext)).collect();
    prior
        .iter()
        .filter(|a| !keep.contains(&normalize_ext(&a.ext)))
        .cloned()
        .collect()
}

/// Register associations. `machine` writes under `HKLM\Software\Classes`
/// (system-wide, needs admin) instead of `HKCU\Software\Classes`, so a
/// machine-wide install is visible to every usern not just the (elevated) admin
/// account that ran the installer.
pub fn register(product_id: &str, exe_path: &str, assocs: &[FileAssoc], machine: bool) {
    if assocs.is_empty() {
        return;
    }
    let root = class_root(machine);
    for a in assocs {
        let ext = normalize_ext(&a.ext);
        let progid = progid_for(product_id, &ext);

        // ProgID class
        if let Some(h) = create_registry_key(root, &format!(r"Software\Classes\{}", progid)) {
            set_default(h, &a.description);
            close(h);
        }
        if let Some(h) =
            create_registry_key(root, &format!(r"Software\Classes\{}\DefaultIcon", progid))
        {
            set_default(h, &format!("\"{}\",0", exe_path));
            close(h);
        }
        if let Some(h) = create_registry_key(
            root,
            &format!(r"Software\Classes\{}\shell\open\command", progid),
        ) {
            set_default(h, &format!("\"{}\" \"%1\"", exe_path));
            close(h);
        }
        // Extension -> ProgID
        if let Some(h) = create_registry_key(root, &format!(r"Software\Classes\{}", ext)) {
            set_default(h, &progid);
            close(h);
        }

        crate::log::info(format!("associated {} -> {} ({})", ext, progid, exe_path));
    }
    notify_assoc_changed();
}

/// Remove associations. `machine` must match the value used at `register` time so
/// the right hive is cleaned (`HKLM` for machine-wide, else `HKCU`).
pub fn unregister(product_id: &str, assocs: &[FileAssoc], machine: bool) {
    if assocs.is_empty() {
        return;
    }
    let root = class_root(machine);
    for a in assocs {
        let ext = normalize_ext(&a.ext);
        let progid = progid_for(product_id, &ext);

        // Only clear the extension default if it still points at us.
        if read_default(root, &format!(r"Software\Classes\{}", ext)).as_deref()
            == Some(progid.as_str())
        {
            delete_tree(root, &format!(r"Software\Classes\{}", ext));
        }
        delete_tree(root, &format!(r"Software\Classes\{}", progid));
        crate::log::info(format!("removed association {} ({})", ext, progid));
    }
    notify_assoc_changed();
}

// ---- Hive selection / shell notify --------------------------------------

/// Root for the `Software\Classes` tree: `HKLM` for a machine-wide (admin)
/// install, else the per-user `HKCU`.
fn class_root(machine: bool) -> HKEY {
    if machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    }
}

fn notify_assoc_changed() {
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progid_sanitizes_product_and_dot() {
        assert_eq!(progid_for("My App", ".myx"), "MyApp.myx");
        assert_eq!(progid_for("Acme-1", "myz"), "Acme-1.myz");
        assert_eq!(progid_for("a/b:c", ".dat"), "abc.dat");
    }

    #[test]
    fn normalize_ext_dot_and_case() {
        assert_eq!(normalize_ext("MYX"), ".myx");
        assert_eq!(normalize_ext(".TxT"), ".txt");
        assert_eq!(normalize_ext("  .Dat "), ".dat");
    }

    fn assoc(ext: &str) -> FileAssoc {
        FileAssoc {
            ext: ext.to_string(),
            description: format!("{ext} doc"),
        }
    }

    #[test]
    fn stale_returns_only_dropped_extensions() {
        let prior = [assoc(".myx"), assoc(".myz"), assoc(".abc")];
        let current = [assoc(".abc")];
        let got: Vec<String> = stale(&prior, &current)
            .iter()
            .map(|a| a.ext.clone())
            .collect();
        assert_eq!(got, vec![".myx".to_string(), ".myz".to_string()]);
    }

    #[test]
    fn stale_empty_when_all_kept() {
        let prior = [assoc(".myx"), assoc(".myz")];
        let current = [assoc(".myz"), assoc(".myx"), assoc(".new")];
        assert!(stale(&prior, &current).is_empty());
    }

    #[test]
    fn stale_is_extension_normalized() {
        // Same extension, different dot/case → kept, not stale.
        let prior = [assoc(".MYX")];
        let current = [assoc("myx")];
        assert!(stale(&prior, &current).is_empty());
    }

    #[test]
    fn stale_edges_empty_inputs() {
        // No prior install → nothing to drop.
        assert!(stale(&[], &[assoc(".abc")]).is_empty());
        // New version declares none → every prior assoc is stale.
        let prior = [assoc(".myx"), assoc(".myz")];
        assert_eq!(stale(&prior, &[]).len(), 2);
    }
}
