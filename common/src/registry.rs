// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Free-form registry entries under `HKCU` (per-user, no admin), beyond file
//! associations. Each [`RegEntry`] is written at install and removed at
//! uninstall. Removal is anti-stomp (only deletes a value that still equals
//! what we wrote) and prunes the keys we created once they're empty — it never
//! deletes a key that still holds anything (so shared keys like `...\Run` keep
//! their other values).

use crate::models::{RegEntry, RegKind, RegValue};
use std::collections::HashSet;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_BINARY, REG_DWORD, REG_EXPAND_SZ,
    REG_MULTI_SZ, REG_OPTION_NON_VOLATILE, REG_QWORD, REG_SZ, REG_VALUE_TYPE, RegCloseKey,
    RegCreateKeyExW, RegDeleteKeyW, RegDeleteValueW, RegOpenKeyExW, RegQueryInfoKeyW,
    RegQueryValueExW, RegSetValueExW,
};
use windows::core::PCWSTR;

fn is_hkcu(e: &RegEntry) -> bool {
    e.hive.eq_ignore_ascii_case("HKCU")
}

/// Write one entry. No-op on a non-HKCU hive or a value that can't be encoded.
pub fn write(e: &RegEntry) {
    if !is_hkcu(e) {
        return;
    }
    let Some((ty, bytes)) = encode(e.kind, &e.value) else {
        crate::log::warn(format!("registry: skip {} (bad value for type)", e.key));
        return;
    };
    if let Some(h) = create_key(&e.key) {
        set_value(h, &e.name, ty, &bytes);
        close(h);
        crate::log::info(format!(
            "registry: set HKCU\\{}\\{}",
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
pub fn remove_if_ours(e: &RegEntry) {
    if !is_hkcu(e) {
        return;
    }
    if let Some((ty, bytes)) = encode(e.kind, &e.value)
        && read_value(&e.key, &e.name) == Some((ty, bytes))
    {
        delete_value(&e.key, &e.name);
        crate::log::info(format!("registry: removed HKCU\\{}\\{}", e.key, e.name));
    }
    prune_empty(&e.key);
}

/// Entries present in `prior` but no longer in `current`, compared by
/// `(hive, key, name)` case-insensitively (registry keys/names are
/// case-insensitive). These are removed on upgrade.
pub fn stale(prior: &[RegEntry], current: &[RegEntry]) -> Vec<RegEntry> {
    let id = |e: &RegEntry| {
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
fn encode(kind: RegKind, value: &RegValue) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    match (kind, value) {
        (RegKind::Sz, RegValue::Text(s)) => Some((REG_SZ, utf16z(s))),
        (RegKind::ExpandSz, RegValue::Text(s)) => Some((REG_EXPAND_SZ, utf16z(s))),
        (RegKind::Dword, RegValue::Int(n)) => Some((REG_DWORD, (*n as u32).to_le_bytes().to_vec())),
        (RegKind::Qword, RegValue::Int(n)) => Some((REG_QWORD, n.to_le_bytes().to_vec())),
        (RegKind::MultiSz, RegValue::List(items)) => Some((REG_MULTI_SZ, multi_sz(items))),
        (RegKind::Binary, RegValue::Text(hex)) => Some((REG_BINARY, decode_hex(hex)?)),
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

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < b.len() {
        out.push((hexv(b[i])? << 4) | hexv(b[i + 1])?);
        i += 2;
    }
    Some(out)
}

fn hexv(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// ---- Win32 registry helpers (HKCU) --------------------------------------

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Value-name pointer: `(Default)` (null) for an empty name.
fn name_ptr(name_w: &[u16], name: &str) -> PCWSTR {
    if name.is_empty() {
        PCWSTR::null()
    } else {
        PCWSTR(name_w.as_ptr())
    }
}

fn create_key(sub: &str) -> Option<HKEY> {
    let w = wide(sub);
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

fn set_value(hkey: HKEY, name: &str, ty: REG_VALUE_TYPE, bytes: &[u8]) {
    let name_w = wide(name);
    unsafe {
        let _ = RegSetValueExW(hkey, name_ptr(&name_w, name), None, ty, Some(bytes));
    }
}

fn read_value(sub: &str, name: &str) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    let w = wide(sub);
    let name_w = wide(name);
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(w.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
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

fn delete_value(sub: &str, name: &str) {
    let w = wide(sub);
    let name_w = wide(name);
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(w.as_ptr()),
            None,
            KEY_WRITE,
            &mut hkey,
        )
        .is_ok()
        {
            let _ = RegDeleteValueW(hkey, name_ptr(&name_w, name));
            let _ = RegCloseKey(hkey);
        }
    }
}

/// True if the subkey exists with no subkeys and no values.
fn key_is_empty(sub: &str) -> bool {
    let w = wide(sub);
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(w.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
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

fn delete_key(sub: &str) {
    let w = wide(sub);
    unsafe {
        let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(w.as_ptr()));
    }
}

/// Delete `key` and walk up its parents while each is empty. Stops at a
/// top-level key (no backslash, e.g. `Software`), which is never deleted, and
/// at the first non-empty key (so shared keys keep their other content).
fn prune_empty(key: &str) {
    let mut cur = key.to_string();
    while cur.contains('\\') {
        if !key_is_empty(&cur) {
            break;
        }
        delete_key(&cur);
        match cur.rsplit_once('\\') {
            Some((parent, _)) => cur = parent.to_string(),
            None => break,
        }
    }
}

fn close(hkey: HKEY) {
    unsafe {
        let _ = RegCloseKey(hkey);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: RegKind, value: RegValue) -> RegEntry {
        RegEntry {
            hive: "HKCU".into(),
            key: r"Software\X".into(),
            name: "v".into(),
            kind,
            value,
        }
    }

    #[test]
    fn encode_sz_is_utf16_null_terminated() {
        let (ty, b) = encode(RegKind::Sz, &RegValue::Text("Hi".into())).unwrap();
        assert_eq!(ty, REG_SZ);
        assert_eq!(b, vec![b'H', 0, b'i', 0, 0, 0]);
    }

    #[test]
    fn encode_dword_and_qword_little_endian() {
        let (ty, b) = encode(RegKind::Dword, &RegValue::Int(0x0102_0304)).unwrap();
        assert_eq!(ty, REG_DWORD);
        assert_eq!(b, vec![0x04, 0x03, 0x02, 0x01]);
        let (_, q) = encode(RegKind::Qword, &RegValue::Int(1)).unwrap();
        assert_eq!(q, vec![1, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn encode_multi_sz_double_null_terminated() {
        let (ty, b) = encode(
            RegKind::MultiSz,
            &RegValue::List(vec!["a".into(), "b".into()]),
        )
        .unwrap();
        assert_eq!(ty, REG_MULTI_SZ);
        // "a\0" "b\0" "\0"
        assert_eq!(b, vec![b'a', 0, 0, 0, b'b', 0, 0, 0, 0, 0]);
    }

    #[test]
    fn encode_binary_decodes_hex() {
        let (ty, b) = encode(RegKind::Binary, &RegValue::Text("DEADbeef".into())).unwrap();
        assert_eq!(ty, REG_BINARY);
        assert_eq!(b, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert!(encode(RegKind::Binary, &RegValue::Text("XYZ".into())).is_none());
        assert!(encode(RegKind::Binary, &RegValue::Text("abc".into())).is_none()); // odd length
    }

    #[test]
    fn encode_rejects_type_value_mismatch() {
        assert!(encode(RegKind::Dword, &RegValue::Text("x".into())).is_none());
        assert!(encode(RegKind::Sz, &RegValue::Int(1)).is_none());
    }

    #[test]
    fn stale_by_hive_key_name_case_insensitive() {
        let prior = vec![
            entry(RegKind::Dword, RegValue::Int(1)),
            RegEntry {
                hive: "HKCU".into(),
                key: r"Software\Y".into(),
                name: "k".into(),
                kind: RegKind::Sz,
                value: RegValue::Text("z".into()),
            },
        ];
        // Same entry as prior[0] but different case in key/name → still "kept".
        let current = vec![RegEntry {
            hive: "hkcu".into(),
            key: r"software\x".into(),
            name: "V".into(),
            kind: RegKind::Dword,
            value: RegValue::Int(99),
        }];
        let s = stale(&prior, &current);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].key, r"Software\Y");
    }
}
