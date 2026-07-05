# Manifest & payload format

These are the `common` crate types that describe what an installer carries.
They're serialized to JSON, signed, and embedded as `RT_RCDATA` id=2. Field
docs come from [`common/src/models.rs`](https://github.com/Tooltip-Focus/Installway/blob/main/common/src/models.rs).

## What's embedded in the installer `.exe`

| Resource | id | Contents |
|---|---|---|
| `RT_RCDATA` | 2 | `SignedPayload` JSON (the manifest + metadata, plus its signature) |
| `RT_RCDATA` | 3 | the uninstaller `.exe` |
| `RT_RCDATA` | 4 | payload length (`u64`, little-endian) |
| PE overlay | — | `MAGIC` + the payload zip, appended after all resource passes |

## `SignedPayload`

```rust
struct SignedPayload {
    payload_json: String,   // exact UTF-8 bytes the signature was computed over
    signature_hex: String,  // Ed25519 signature of payload_json
}
```

The verifier checks the signature against the **raw `payload_json` bytes**, then
parses `InstallerPayload` from them — avoiding any serializer-determinism trap.

## `InstallerPayload`

| Field | Type | Notes |
|---|---|---|
| `kind` | `Full` \| `Patch` | |
| `product` | `String` | display name (ARP DisplayName, version-info, UI, shortcut) |
| `product_id` | `String` | registry-safe id: Uninstall key, ProgIDs, data folder, upgrade detection |
| `publisher` | `String` | uninstall data folder + ARP "Publisher" |
| `from_version` | `Option<String>` | set for patches; pins the target version |
| `to_version` | `String` | |
| `min_installer_version` | `String` | anti-rollback floor; default `1.0.0` |
| `payload_blake3` | `String` | BLAKE3 of the zip, re-verified before extract |
| `created_at_unix` | `i64` | |
| `manifest` | `Manifest` | per-file table (below) |
| `license_text` | `Option<String>` | EULA shown on the License page |
| `associations` | `Vec<FileAssoc>` | file types to register under `Software\Classes` (HKLM if machine-wide, else HKCU) |
| `shortcuts` | `Vec<ShortcutEntry>` | [shortcuts](../packaging/shortcuts.md) to create (`dir`/`target`/`args` are token templates); none created unless declared |
| `registry` | `Vec<RegistryEntry>` | free-form HKCU/HKLM [registry entries](../packaging/registry.md) (key/value are token templates) |
| `force_reinstall` | `bool` | dev: rewrite all, remove orphans, skip from-check |
| `purge_unknown_files` | `bool` | Full installs: remove unknown/leftover files (known files still hash-skipped); ignored for patches |
| `skip_license` / `skip_path` | `bool` | trim the wizard |
| `install_dir_restriction` | `Enforce` \| `DefaultDirOnly` \| `Bypass` | whether a fresh interactive install may target a non-empty folder; default `Enforce` (block). See [Config file](../building/config.md) |
| `default_install_dir` | `Option<String>` | proposed path; `%VAR%` tokens expanded |
| `upgrade_minimal_ui` | `bool` | upgrades use the minimal UI; first install always uses the wizard |
| `show_uninstall_complete` | `bool` | show the "uninstall complete" message box at the end (off by default) |

## `Manifest` / `FileEntry` / `PatchInfo`

```rust
struct Manifest {
    version: String,
    exe: String,                       // main exe, relative to install root
    files: HashMap<String, FileEntry>, // keyed by relative path
    deleted_files: Vec<String>,        // removed at install time (patches)
    full_size: u64,
    total_patch_size: u64,
    features: Vec<String>,             // declared feature-pack ids (see Feature packs)
    default_features: Vec<String>,     // subset enabled by default on a fresh install
}

struct FileEntry {
    hash: String,            // BLAKE3, checked after each write/patch
    size: u64,
    patch: Option<PatchInfo>,
    feature: Option<String>, // feature pack this file belongs to; None = base
}

struct PatchInfo {
    file: String,   // in-zip path: `patches/<blake3(rel)>.patch`
    size: u64,
}
```

> **Payload zip layout.** Full files live under `full/<rel>`; binary patches
> under `patches/<blake3(rel)>.patch`. The installer reads `PatchInfo.file`
> verbatim as the in-zip path, so the name in the manifest and the actual zip
> entry name are produced from one function in the builder — they must always
> match. Unchanged files (in a patch) have no zip entry and no `FileEntry`
> beyond their recorded hash.

## `InstallInfo`

Persisted to `<data-dir>\installer_info.json` by the installer and read by the
uninstaller: `product`, `product_id`, `publisher`, `version`, `install_dir`,
`installed_at_unix`, `registry_key` (equal to `product_id`), `exe`, the
`associations`, the resolved `shortcuts`, the resolved `registry` entries to
remove, `requires_admin` (drives the `HKLM`/`%ProgramData%` vs
`HKCU`/`%LOCALAPPDATA%` choice), and `features` (the active feature packs, read
back on the next upgrade — see [Feature packs](../packaging/features.md)). Records written before the product/id split
have no `product_id`; readers fall back to `registry_key` and a sanitized
`product`.

In the payload, `registry` and `shortcuts` hold **token templates**; in
`installer_info.json` they hold the **resolved** entries actually written
(absolute paths), so the uninstaller matches and removes exactly those.

## Backward compatibility

New fields use `#[serde(default)]`, so installers can read JSON written by
older versions (missing fields take sensible defaults). The round-trip is
covered by tests in `models.rs`.
