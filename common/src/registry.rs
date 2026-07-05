// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Free-form registry entries beyond file associations, in `HKCU` (per-user, no
//! admin) or `HKLM` (machine-wide install, needs admin) per the entry's `hive`.
//! Each [`RegistryEntry`] is written at install and removed at uninstall. Removal is
//! anti-stomp (only deletes a value that still equals what we wrote) and prunes
//! the keys we created once they're empty — it never deletes a key that still
//! holds anything (so shared keys like `...\Run` keep their other values).

use crate::model::registry_entry::RegistryEntry;
use crate::model::registry_kind::RegistryKind;
use crate::model::registry_value::RegistryValue;
use crate::utils::wide;
use std::collections::HashSet;
use windows::Win32::System::Registry::RegDeleteTreeW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_BINARY, REG_DWORD,
    REG_EXPAND_SZ, REG_MULTI_SZ, REG_OPTION_NON_VOLATILE, REG_QWORD, REG_SZ, REG_VALUE_TYPE,
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegDeleteValueW, RegOpenKeyExW, RegQueryInfoKeyW,
    RegQueryValueExW, RegSetValueExW,
};
use windows::core::PCWSTR;

// Write key in the registry
pub fn create_registry_key(root: HKEY, sub: &str) -> Option<HKEY> {
    let w = wide(sub);
    unsafe {
        let mut hkey = HKEY::default();
        let rc = RegCreateKeyExW(
            root,
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

/// Root hive for an entry: `HKCU` or `HKLM` (machine-wide, needs admin). `None`
/// for any other hive string, which the caller treats as a no-op.
fn hive_root(hive: &str) -> Option<HKEY> {
    if hive.eq_ignore_ascii_case("HKCU") {
        Some(HKEY_CURRENT_USER)
    } else if hive.eq_ignore_ascii_case("HKLM") {
        Some(HKEY_LOCAL_MACHINE)
    } else {
        None
    }
}

/// Write one entry. No-op on an unsupported hive or a value that can't be
/// encoded. An `HKLM` entry needs the process to be elevated; if it isn't, the
/// create simply fails and is logged.
pub fn write(e: &RegistryEntry) {
    let Some(root) = hive_root(&e.hive) else {
        return;
    };
    let Some((ty, bytes)) = encode(e.kind, &e.value) else {
        crate::log::warn(format!("registry: skip {} (bad value for type)", e.key));
        return;
    };
    if let Some(h) = create_registry_key(root, &e.key) {
        set_value(h, &e.name, ty, &bytes);
        close(h);
        crate::log::info(format!(
            "registry: set {}\\{}\\{}",
            e.hive.to_ascii_uppercase(),
            e.key,
            if e.name.is_empty() {
                "(Default)"
            } else {
                &e.name
            }
        ));
    }
}

/// Remove one entry: delete the value only if it still equals what we wrote,
/// then prune now-empty keys we created.
pub fn remove_if_ours(e: &RegistryEntry) {
    let Some(root) = hive_root(&e.hive) else {
        return;
    };
    if let Some((ty, bytes)) = encode(e.kind, &e.value)
        && read_value(root, &e.key, &e.name) == Some((ty, bytes))
    {
        delete_value(root, &e.key, &e.name);
        crate::log::info(format!(
            "registry: removed {}\\{}\\{}",
            e.hive.to_ascii_uppercase(),
            e.key,
            e.name
        ));
    }
    prune_empty(root, &e.key);
}

/// Entries present in `prior` but no longer in `current`, compared by
/// `(hive, key, name)` case-insensitively (registry keys/names are
/// case-insensitive). These are removed on upgrade.
pub fn stale(prior: &[RegistryEntry], current: &[RegistryEntry]) -> Vec<RegistryEntry> {
    let id = |e: &RegistryEntry| {
        (
            e.hive.to_ascii_uppercase(),
            e.key.to_ascii_lowercase(),
            e.name.to_ascii_lowercase(),
        )
    };
    let keep: HashSet<_> = current.iter().map(id).collect();
    prior
        .iter()
        .filter(|e| !keep.contains(&id(e)))
        .cloned()
        .collect()
}

// ---- Encoding (pure, unit-tested) ---------------------------------------

/// Encode a (kind, value) pair into the registry type tag and its raw bytes.
/// `None` if the value variant doesn't match the kind (guarded at build time).
fn encode(kind: RegistryKind, value: &RegistryValue) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    match (kind, value) {
        (RegistryKind::Sz, RegistryValue::Text(s)) => Some((REG_SZ, utf16z(s))),
        (RegistryKind::ExpandSz, RegistryValue::Text(s)) => Some((REG_EXPAND_SZ, utf16z(s))),
        (RegistryKind::Dword, RegistryValue::Int(n)) => {
            Some((REG_DWORD, (*n as u32).to_le_bytes().to_vec()))
        }
        (RegistryKind::Qword, RegistryValue::Int(n)) => Some((REG_QWORD, n.to_le_bytes().to_vec())),
        (RegistryKind::MultiSz, RegistryValue::List(items)) => {
            Some((REG_MULTI_SZ, multi_sz(items)))
        }
        (RegistryKind::Binary, RegistryValue::Text(hex)) => {
            Some((REG_BINARY, hex::decode(hex.trim()).ok()?))
        }
        _ => None,
    }
}

/// UTF-16LE bytes of `s` with a trailing null.
fn utf16z(s: &str) -> Vec<u8> {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    w.push(0);
    u16_bytes(&w)
}

/// REG_MULTI_SZ: each string null-terminated, the block ending in a double null.
fn multi_sz(items: &[String]) -> Vec<u8> {
    if items.is_empty() {
        return u16_bytes(&[0, 0]);
    }
    let mut w: Vec<u16> = Vec::new();
    for s in items {
        w.extend(s.encode_utf16());
        w.push(0);
    }
    w.push(0);
    u16_bytes(&w)
}

fn u16_bytes(w: &[u16]) -> Vec<u8> {
    let mut b = Vec::with_capacity(w.len() * 2);
    for u in w {
        b.extend_from_slice(&u.to_le_bytes());
    }
    b
}

// ---- Win32 registry helpers (HKCU / HKLM) -------------------------------

/// Value-name pointer: `(Default)` (null) for an empty name.
fn name_ptr(name_w: &[u16], name: &str) -> PCWSTR {
    if name.is_empty() {
        PCWSTR::null()
    } else {
        PCWSTR(name_w.as_ptr())
    }
}

pub(crate) fn set_value(hkey: HKEY, name: &str, ty: REG_VALUE_TYPE, bytes: &[u8]) {
    let name_w = wide(name);
    unsafe {
        let _ = RegSetValueExW(hkey, name_ptr(&name_w, name), None, ty, Some(bytes));
    }
}

/// Set the key's `(Default)` REG_SZ value (convenience over [`set_value`]).
pub(crate) fn set_default(hkey: HKEY, value: &str) {
    set_value(hkey, "", REG_SZ, &utf16z(value));
}

/// Read the key's `(Default)` value as a string (REG_SZ assumed). Decodes the
/// raw bytes from [`read_value`] as UTF-16LE up to the first null.
pub(crate) fn read_default(root: HKEY, sub: &str) -> Option<String> {
    let (_, bytes) = read_value(root, sub, "")?;
    let u: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let end = u.iter().position(|&c| c == 0).unwrap_or(u.len());
    Some(String::from_utf16_lossy(&u[..end]))
}

pub(crate) fn read_value(root: HKEY, sub: &str, name: &str) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    let w = wide(sub);
    let name_w = wide(name);
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(root, PCWSTR(w.as_ptr()), None, KEY_READ, &mut hkey).is_err() {
            return None;
        }
        let name_pcwstr = name_ptr(&name_w, name);
        let mut ty = REG_VALUE_TYPE::default();
        let mut len: u32 = 0;
        let rc = RegQueryValueExW(hkey, name_pcwstr, None, Some(&mut ty), None, Some(&mut len));
        if rc.is_err() {
            let _ = RegCloseKey(hkey);
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let rc2 = RegQueryValueExW(
            hkey,
            name_pcwstr,
            None,
            Some(&mut ty),
            Some(buf.as_mut_ptr()),
            Some(&mut len),
        );
        let _ = RegCloseKey(hkey);
        if rc2.is_err() {
            return None;
        }
        buf.truncate(len as usize);
        Some((ty, buf))
    }
}

fn delete_value(root: HKEY, sub: &str, name: &str) {
    let w = wide(sub);
    let name_w = wide(name);
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(root, PCWSTR(w.as_ptr()), None, KEY_WRITE, &mut hkey).is_ok() {
            let _ = RegDeleteValueW(hkey, name_ptr(&name_w, name));
            let _ = RegCloseKey(hkey);
        }
    }
}

/// Delete a key and its entire subtree. Unlike [`prune_empty`], this removes the
/// key even when it still holds subkeys/values — used for tearing down an
/// association ProgID tree wholesale.
pub(crate) fn delete_tree(root: HKEY, sub: &str) {
    let w = wide(sub);
    unsafe {
        let _ = RegDeleteTreeW(root, PCWSTR(w.as_ptr()));
    }
}

/// True if the subkey exists with no subkeys and no values.
fn key_is_empty(root: HKEY, sub: &str) -> bool {
    let w = wide(sub);
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(root, PCWSTR(w.as_ptr()), None, KEY_READ, &mut hkey).is_err() {
            return false;
        }
        let mut subkeys: u32 = 0;
        let mut values: u32 = 0;
        let rc = RegQueryInfoKeyW(
            hkey,
            None,
            None,
            None,
            Some(&mut subkeys),
            None,
            None,
            Some(&mut values),
            None,
            None,
            None,
            None,
        );
        let _ = RegCloseKey(hkey);
        rc.is_ok() && subkeys == 0 && values == 0
    }
}

fn delete_key(root: HKEY, sub: &str) {
    let w = wide(sub);
    unsafe {
        let _ = RegDeleteKeyW(root, PCWSTR(w.as_ptr()));
    }
}

/// Delete `key` and walk up its parents while each is empty. Stops at a
/// top-level key (no backslash, e.g. `Software`), which is never deleted, and
/// at the first non-empty key (so shared keys keep their other content).
fn prune_empty(root: HKEY, key: &str) {
    let mut cur = key.to_string();
    while cur.contains('\\') {
        if !key_is_empty(root, &cur) {
            break;
        }
        delete_key(root, &cur);
        match cur.rsplit_once('\\') {
            Some((parent, _)) => cur = parent.to_string(),
            None => break,
        }
    }
}

pub(crate) fn close(hkey: HKEY) {
    unsafe {
        let _ = RegCloseKey(hkey);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: RegistryKind, value: RegistryValue) -> RegistryEntry {
        RegistryEntry {
            hive: "HKCU".into(),
            key: r"Software\X".into(),
            name: "v".into(),
            kind,
            value,
        }
    }

    #[test]
    fn encode_sz_is_utf16_null_terminated() {
        let (ty, b) = encode(RegistryKind::Sz, &RegistryValue::Text("Hi".into())).unwrap();
        assert_eq!(ty, REG_SZ);
        assert_eq!(b, vec![b'H', 0, b'i', 0, 0, 0]);
    }

    #[test]
    fn encode_dword_and_qword_little_endian() {
        let (ty, b) = encode(RegistryKind::Dword, &RegistryValue::Int(0x0102_0304)).unwrap();
        assert_eq!(ty, REG_DWORD);
        assert_eq!(b, vec![0x04, 0x03, 0x02, 0x01]);
        let (_, q) = encode(RegistryKind::Qword, &RegistryValue::Int(1)).unwrap();
        assert_eq!(q, vec![1, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn encode_multi_sz_double_null_terminated() {
        let (ty, b) = encode(
            RegistryKind::MultiSz,
            &RegistryValue::List(vec!["a".into(), "b".into()]),
        )
        .unwrap();
        assert_eq!(ty, REG_MULTI_SZ);
        // "a\0" "b\0" "\0"
        assert_eq!(b, vec![b'a', 0, 0, 0, b'b', 0, 0, 0, 0, 0]);
    }

    #[test]
    fn encode_binary_decodes_hex() {
        let (ty, b) = encode(
            RegistryKind::Binary,
            &RegistryValue::Text("DEADbeef".into()),
        )
        .unwrap();
        assert_eq!(ty, REG_BINARY);
        assert_eq!(b, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert!(encode(RegistryKind::Binary, &RegistryValue::Text("XYZ".into())).is_none());
        assert!(encode(RegistryKind::Binary, &RegistryValue::Text("abc".into())).is_none()); // odd length
    }

    #[test]
    fn encode_rejects_type_value_mismatch() {
        assert!(encode(RegistryKind::Dword, &RegistryValue::Text("x".into())).is_none());
        assert!(encode(RegistryKind::Sz, &RegistryValue::Int(1)).is_none());
    }

    #[test]
    fn stale_by_hive_key_name_case_insensitive() {
        let prior = vec![
            entry(RegistryKind::Dword, RegistryValue::Int(1)),
            RegistryEntry {
                hive: "HKCU".into(),
                key: r"Software\Y".into(),
                name: "k".into(),
                kind: RegistryKind::Sz,
                value: RegistryValue::Text("z".into()),
            },
        ];
        // Same entry as prior[0] but different case in key/name → still "kept".
        let current = vec![RegistryEntry {
            hive: "hkcu".into(),
            key: r"software\x".into(),
            name: "V".into(),
            kind: RegistryKind::Dword,
            value: RegistryValue::Int(99),
        }];
        let s = stale(&prior, &current);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].key, r"Software\Y");
    }
}
