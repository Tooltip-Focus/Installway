# Manifest and payload format

These are the `common` crate types that describe what an installer carries.
They are serialized to JSON, signed, and embedded as resources. Field
documentation lives in
[`common/src/model/`](https://github.com/Tooltip-Focus/Installway/tree/main/common/src/model).

## What is embedded in the installer `.exe`

| Resource | Id | Contents |
|---|---|---|
| `RT_RCDATA` | 2 | `SignedPayload` JSON: the manifest and metadata, plus the signature. |
| `RT_RCDATA` | 3 | The uninstaller `.exe`. |
| `RT_RCDATA` | 4 | The payload length, a little-endian `u64`. |
| `RT_RCDATA` | 5 | The optional header banner PNG. Not signed; see [Branding](../packaging/branding.md#header-banner). |
| PE overlay | | A magic marker followed by the payload zip, appended after all resource passes. |

## SignedPayload

```rust
struct SignedPayload {
    payload_json: String,   // exact UTF-8 bytes the signature was computed over
    signature_hex: String,  // Ed25519 signature of payload_json
}
```

The verifier checks the signature against the raw `payload_json` bytes, then
parses `InstallerPayload` from them. Signing the exact bytes avoids any
serializer-determinism trap.

## InstallerPayload

| Field | Type | Notes |
|---|---|---|
| `kind` | `Full` or `Patch` | |
| `product` | `String` | Display name. |
| `product_id` | `String` | Registry-safe id: Uninstall key, ProgIDs, data folder, upgrade detection. |
| `publisher` | `String` | Uninstall data folder and the Apps "Publisher" field. |
| `from_version` | `Option<String>` | Set for patches; pins the target version. |
| `to_version` | `String` | |
| `min_installer_version` | `String` | Minimum stub version allowed to run this payload. Default `1.0.0`. |
| `payload_blake3` | `String` | BLAKE3 of the zip, re-verified before extraction. |
| `created_at_unix` | `i64` | |
| `manifest` | `Manifest` | The per-file table; see below. |
| `license_text` | `Option<String>` | EULA shown on the License page. |
| `associations` | `Vec<FileAssoc>` | File types to register under `Software\Classes`. |
| `plugins` | `Vec<PluginEntry>` | Bundled [plugins](../packaging/plugins.md) and their phases. |
| `shortcuts` | `Vec<ShortcutEntry>` | [Shortcuts](../packaging/shortcuts.md) to create. `dir`, `target`, and `args` are token templates. None are created unless declared. |
| `registry` | `Vec<RegistryEntry>` | Free-form [registry entries](../packaging/registry.md). Key and value are token templates. |
| `force_reinstall` | `bool` | Dev: rewrite all, remove orphans, skip the from-version check. |
| `purge_unknown_files` | `bool` | Full installs: remove unknown or leftover files. Ignored for patches. |
| `skip_license`, `skip_path` | `bool` | Trim the wizard. |
| `install_dir_restriction` | `Enforce`, `DefaultDirOnly`, or `Bypass` | Whether a fresh interactive install may target a non-empty folder. Default `Enforce`. |
| `default_install_dir` | `Option<String>` | Proposed path; `%VAR%` tokens are expanded. |
| `launch_option` | `Checked`, `Unchecked`, or `Hidden` | The final-page "launch now" checkbox. |
| `upgrade_minimal_ui` | `bool` | Upgrades use the minimal UI; a first install always gets the wizard. |
| `show_uninstall_complete` | `bool` | Show the "uninstall complete" message box. Off by default. |

## Manifest, FileEntry, and PatchInfo

```rust
struct Manifest {
    version: String,
    exe: Option<String>,               // main exe, relative to the install root
    files: HashMap<String, FileEntry>, // keyed by relative path
    deleted_files: Vec<String>,        // removed at install time (patches)
    full_size: u64,
    total_patch_size: u64,
    features: Vec<String>,             // declared feature-pack ids
    default_features: Vec<String>,     // subset enabled by default on a fresh install
    feature_mode: FeatureMode,         // upgrade base: "sticky" (default) or "override"
}

struct FileEntry {
    hash: String,            // BLAKE3, checked after each write or patch
    size: u64,
    patch: Option<PatchInfo>,
    feature: Option<String>, // feature pack this file belongs to; None = base
}

struct PatchInfo {
    file: String,   // in-zip path: patches/<blake3(rel)>.patch
    size: u64,
}
```

**Payload zip layout.** Full files live under `full/<rel>`; binary patches
under `patches/<blake3(rel)>.patch`. The installer reads `PatchInfo.file`
verbatim as the in-zip path, so the name in the manifest and the actual zip
entry name are produced by one function in the builder; they always match.
Unchanged files in a patch have no zip entry, only their recorded hash.

## InstallInfo

Persisted to `<data-dir>\installer_info.json` by the installer and read by
the uninstaller. It holds `product`, `product_id`, `publisher`, `version`,
`install_dir`, `installed_at_unix`, `registry_key` (equal to `product_id`),
`exe`, the `associations`, the resolved `shortcuts`, the resolved `registry`
entries to remove, `requires_admin` (which drives the `HKLM` and
`%ProgramData%` versus `HKCU` and `%LOCALAPPDATA%` choice), and `features`,
the active feature packs. The next upgrade reads `features` to clean up
dropped features and, under `feature_mode = "sticky"`, to seed its base. See
[Feature packs](../packaging/features.md).

In the payload, `registry` and `shortcuts` hold token templates. In
`installer_info.json` they hold the resolved entries actually written, with
absolute paths, so the uninstaller matches and removes exactly those.

Records written before the product/id split have no `product_id`; readers
fall back to `registry_key` and a sanitized `product`.

## Backward compatibility

New fields use `#[serde(default)]`, so installers can read JSON written by
older versions; missing fields take sensible defaults. The round-trip is
covered by tests in the model modules.
