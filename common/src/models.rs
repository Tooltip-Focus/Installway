// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub exe: String,
    pub files: HashMap<String, FileEntry>,
    #[serde(default)]
    pub deleted_files: Vec<String>,
    #[serde(default)]
    pub full_size: u64,
    #[serde(default)]
    pub total_patch_size: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileEntry {
    pub hash: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<PatchInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PatchInfo {
    pub file: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadKind {
    Full,
    Patch,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstallerPayload {
    pub kind: PayloadKind,
    /// Human-facing display name: ARP `DisplayName`, version-info ProductName,
    /// installer/uninstaller UI text, and the shortcut label.
    pub product: String,
    /// Registry-safe internal identifier, distinct from the display `product`.
    /// Drives the HKCU Uninstall key, association ProgIDs, the per-user data
    /// folder (`%LOCALAPPDATA%\<publisher>\Uninstall\<product_id>`) and upgrade
    /// detection. Validated at build time. `#[serde(default)]` so payloads
    /// predating the split still parse (empty → fall back to a sanitized
    /// `product`).
    #[serde(default)]
    pub product_id: String,
    /// Publisher / vendor name. Used for the per-user uninstall data folder
    /// and the Add/Remove Programs "Publisher" field. Mandatory at build time.
    #[serde(default)]
    pub publisher: String,
    pub from_version: Option<String>,
    pub to_version: String,
    pub min_installer_version: String,
    pub payload_blake3: String,
    pub created_at_unix: i64,
    pub manifest: Manifest,
    /// Optional EULA text shown on the License page of the installer UI.
    /// `None` (or missing field on older payloads) falls back to a built-in placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_text: Option<String>,
    /// File-type associations to register under `HKCU\Software\Classes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub associations: Vec<FileAssoc>,
    /// Dev flag: ignore the installed version and reinstall from scratch
    /// (skip patch from-version check, rewrite all files, remove orphans).
    #[serde(default)]
    pub force_reinstall: bool,
    /// Hide the License page in the interactive UI.
    #[serde(default)]
    pub skip_license: bool,
    /// Hide the Choose-location page; install straight to the default path.
    #[serde(default)]
    pub skip_path: bool,
    /// Default install directory the UI proposes, set per-app at build time.
    /// May contain `%VAR%` env tokens (e.g. `%LOCALAPPDATA%\Programs\MyApp`).
    /// `None` falls back to `%LOCALAPPDATA%\Programs\<product>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_install_dir: Option<String>,
    /// When set, an *upgrade* (a run over an already-installed copy) uses the
    /// compact minimal UI instead of the full wizard. The first install always
    /// uses the full wizard. Decided by this (the new installer's) payload.
    #[serde(default)]
    pub upgrade_minimal_ui: bool,
    /// Free-form registry entries (HKCU) written at install and removed at
    /// uninstall. Key/value strings are templates expanded at install time
    /// (see the registry docs for the token list).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry: Vec<RegEntry>,
}

/// Registry value type for a [`RegEntry`].
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegKind {
    Sz,
    ExpandSz,
    Dword,
    Qword,
    MultiSz,
    Binary,
}

/// A registry value's data. The variant is paired with a [`RegKind`]:
/// `Text` for sz/expand_sz (and the hex string of binary), `Int` for
/// dword/qword, `List` for multi_sz.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum RegValue {
    Text(String),
    Int(u64),
    List(Vec<String>),
}

/// One free-form registry entry. In the payload the strings are templates; in
/// `InstallInfo` they are the resolved values actually written (so the
/// uninstaller can match + remove exactly what it wrote).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RegEntry {
    /// Hive — `"HKCU"` only (the installer never elevates).
    pub hive: String,
    /// Subkey path under the hive, e.g. `Software\Acme\App`.
    pub key: String,
    /// Value name; empty = the key's `(Default)` value.
    #[serde(default)]
    pub name: String,
    pub kind: RegKind,
    pub value: RegValue,
}

/// One file-type association: extension + a human description.
/// The shell `open` verb is wired to the product's main exe with `"%1"`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileAssoc {
    /// Extension including the leading dot, e.g. ".myx".
    pub ext: String,
    /// Friendly type description shown in Explorer, e.g. "My App Document".
    pub description: String,
}

/// What gets embedded in the installer .exe as RCDATA id=2.
///
/// `payload_json` is the exact UTF-8 byte sequence the signature was computed over.
/// The verifier verifies the signature against those bytes, *then* parses
/// `InstallerPayload` from them. This avoids any serializer-determinism trap.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SignedPayload {
    pub payload_json: String,
    pub signature_hex: String,
}

/// Persisted to `<install_dir>/installer_info.json` by the installer.
/// Read by the uninstaller (and any tooling) to locate registry entries
/// and walk the manifest for cleanup.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstallInfo {
    pub product: String,
    /// Registry-safe internal id (see `InstallerPayload::product_id`). Empty on
    /// records written before the split — readers fall back to `registry_key` /
    /// a sanitized `product`.
    #[serde(default)]
    pub product_id: String,
    #[serde(default)]
    pub publisher: String,
    pub version: String,
    pub install_dir: String,
    pub installed_at_unix: i64,
    /// HKCU subkey under `Software\Microsoft\Windows\CurrentVersion\Uninstall`.
    pub registry_key: String,
    /// Optional path (relative to install_dir) of the product's main exe.
    pub exe: String,
    /// File associations registered at install time - the uninstaller removes
    /// exactly these.
    #[serde(default)]
    pub associations: Vec<FileAssoc>,
    /// Resolved registry entries written at install - the uninstaller removes
    /// exactly these (anti-stomp by value).
    #[serde(default)]
    pub registry: Vec<RegEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_parses_old_json_with_defaults() {
        // JSON predating publisher / force_reinstall / associations / license.
        let j = r#"{
            "kind":"Full","product":"P","from_version":null,"to_version":"1.0",
            "min_installer_version":"1.0.0","payload_blake3":"deadbeef",
            "created_at_unix":0,"manifest":{"version":"1.0","files":{}}
        }"#;
        let p: InstallerPayload = serde_json::from_str(j).unwrap();
        assert_eq!(p.publisher, "");
        assert_eq!(p.product_id, "");
        assert!(!p.force_reinstall);
        assert!(!p.upgrade_minimal_ui);
        assert!(p.associations.is_empty());
        assert!(p.license_text.is_none());
        assert_eq!(p.kind, PayloadKind::Full);
    }

    #[test]
    fn info_parses_old_json_with_defaults() {
        let j = r#"{
            "product":"P","version":"1.0","install_dir":"d",
            "installed_at_unix":0,"registry_key":"P","exe":"a.exe"
        }"#;
        let i: InstallInfo = serde_json::from_str(j).unwrap();
        assert_eq!(i.publisher, "");
        assert_eq!(i.product_id, "");
        assert!(i.associations.is_empty());
    }

    #[test]
    fn payload_roundtrips() {
        let p = InstallerPayload {
            kind: PayloadKind::Patch,
            product: "P".into(),
            product_id: "P_id".into(),
            publisher: "Pub".into(),
            from_version: Some("1.0".into()),
            to_version: "1.1".into(),
            min_installer_version: "1.0.0".into(),
            payload_blake3: "abc".into(),
            created_at_unix: 123,
            manifest: Manifest {
                version: "1.1".into(),
                exe: "a.exe".into(),
                files: Default::default(),
                deleted_files: vec![],
                full_size: 0,
                total_patch_size: 0,
            },
            license_text: None,
            associations: vec![FileAssoc {
                ext: ".x".into(),
                description: "X".into(),
            }],
            force_reinstall: true,
            skip_license: true,
            skip_path: false,
            default_install_dir: Some(r"%LOCALAPPDATA%\Programs\P".into()),
            upgrade_minimal_ui: true,
            registry: vec![RegEntry {
                hive: "HKCU".into(),
                key: r"Software\Acme\App".into(),
                name: "Build".into(),
                kind: RegKind::Dword,
                value: RegValue::Int(42),
            }],
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: InstallerPayload = serde_json::from_str(&s).unwrap();
        assert_eq!(back.publisher, "Pub");
        assert_eq!(back.product_id, "P_id");
        assert_eq!(back.registry.len(), 1);
        assert_eq!(back.registry[0].kind, RegKind::Dword);
        assert_eq!(back.registry[0].value, RegValue::Int(42));
        assert!(back.force_reinstall);
        assert!(back.skip_license);
        assert!(!back.skip_path);
        assert_eq!(
            back.default_install_dir.as_deref(),
            Some(r"%LOCALAPPDATA%\Programs\P")
        );
        assert!(back.upgrade_minimal_ui);
        assert_eq!(back.associations.len(), 1);
        assert_eq!(back.from_version.as_deref(), Some("1.0"));
    }
}
