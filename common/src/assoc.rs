// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! File-type associations under `HKCU\Software\Classes` (per-user, no admin).
//!
//! Layout written per association (extension `.myx`, product `MyApp`):
//! ```text
//! HKCU\Software\Classes\.myx                       (default) = "MyApp.myx"
//! HKCU\Software\Classes\MyApp.myx                  (default) = "<description>"
//! HKCU\Software\Classes\MyApp.myx\DefaultIcon      (default) = "<exe>",0
//! HKCU\Software\Classes\MyApp.myx\shell\open\command (default) = "<exe>" "%1"
//! ```
//! `progid_for` is shared by installer + uninstaller so both agree on the key
//! names. Uninstall only clears the `.ext` default when it still points at our
//! ProgID, so we never stomp an association the user re-pointed elsewhere.

use crate::models::FileAssoc;

/// Deterministic ProgID for a (product, extension) pair, e.g.
/// `("My App", ".myx") -> "MyApp.myx"`.
pub fn progid_for(product: &str, ext: &str) -> String {
    let prod: String = product
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
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
    use std::collections::HashSet;
    let keep: HashSet<String> = current.iter().map(|a| normalize_ext(&a.ext)).collect();
    prior
        .iter()
        .filter(|a| !keep.contains(&normalize_ext(&a.ext)))
        .cloned()
        .collect()
}

pub fn register(product: &str, exe_path: &str, assocs: &[FileAssoc]) {
    if assocs.is_empty() {
        return;
    }
    for a in assocs {
        let ext = normalize_ext(&a.ext);
        let progid = progid_for(product, &ext);

        // ProgID class
        if let Some(h) = create_key(&format!(r"Software\Classes\{}", progid)) {
            set_default(h, &a.description);
            close(h);
        }
        if let Some(h) = create_key(&format!(r"Software\Classes\{}\DefaultIcon", progid)) {
            set_default(h, &format!("\"{}\",0", exe_path));
            close(h);
        }
        if let Some(h) =
            create_key(&format!(r"Software\Classes\{}\shell\open\command", progid))
        {
            set_default(h, &format!("\"{}\" \"%1\"", exe_path));
            close(h);
        }
        // Extension -> ProgID
        if let Some(h) = create_key(&format!(r"Software\Classes\{}", ext)) {
            set_default(h, &progid);
            close(h);
        }

        crate::log::info(format!("associated {} -> {} ({})", ext, progid, exe_path));
    }
    notify_assoc_changed();
}

pub fn unregister(product: &str, assocs: &[FileAssoc]) {
    if assocs.is_empty() {
        return;
    }
    for a in assocs {
        let ext = normalize_ext(&a.ext);
        let progid = progid_for(product, &ext);

        // Only clear the extension default if it still points at us.
        if read_default(&format!(r"Software\Classes\{}", ext)).as_deref() == Some(progid.as_str())
        {
            delete_tree(&format!(r"Software\Classes\{}", ext));
        }
        delete_tree(&format!(r"Software\Classes\{}", progid));
        crate::log::info(format!("removed association {} ({})", ext, progid));
    }
    notify_assoc_changed();
}

// ---- Windows registry helpers -------------------------------------------

use windows::Win32::System::Registry::HKEY;

fn create_key(sub: &str) -> Option<HKEY> {
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, RegCreateKeyExW,
    };
    use windows::core::PCWSTR;
    let w: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut hkey = HKEY::default();
        let rc = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(w.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        );
        if rc.is_ok() { Some(hkey) } else { None }
    }
}

fn set_default(hkey: HKEY, value: &str) {
    use windows::Win32::System::Registry::{REG_SZ, RegSetValueExW};
    use windows::core::PCWSTR;
    let v: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) };
    unsafe {
        // Name = null/empty → the key's (Default) value.
        let _ = RegSetValueExW(hkey, PCWSTR::null(), None, REG_SZ, Some(bytes));
    }
}

fn close(hkey: HKEY) {
    use windows::Win32::System::Registry::RegCloseKey;
    unsafe {
        let _ = RegCloseKey(hkey);
    }
}

fn read_default(sub: &str) -> Option<String> {
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_READ, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::PCWSTR;
    let w: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(w.as_ptr()), None, KEY_READ, &mut hkey).is_err()
        {
            return None;
        }
        let mut buf = [0u16; 512];
        let mut len: u32 = (buf.len() * 2) as u32;
        let mut ty = REG_VALUE_TYPE::default();
        let rc = RegQueryValueExW(
            hkey,
            PCWSTR::null(),
            None,
            Some(&mut ty),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut len),
        );
        let _ = RegCloseKey(hkey);
        if rc.is_err() {
            return None;
        }
        let chars = (len as usize / 2).min(buf.len());
        let end = buf[..chars].iter().position(|&c| c == 0).unwrap_or(chars);
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

fn delete_tree(sub: &str) {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW};
    use windows::core::PCWSTR;
    let w: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(w.as_ptr()));
    }
}

fn notify_assoc_changed() {
    use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
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
        FileAssoc { ext: ext.to_string(), description: format!("{ext} doc") }
    }

    #[test]
    fn stale_returns_only_dropped_extensions() {
        let prior = [assoc(".myx"), assoc(".myz"), assoc(".abc")];
        let current = [assoc(".abc")];
        let got: Vec<String> = stale(&prior, &current).iter().map(|a| a.ext.clone()).collect();
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
