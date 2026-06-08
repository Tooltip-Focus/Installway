# Full installer

A **full** installer carries every file of one product version. Run it on a
clean machine or over any existing install — it writes the complete set.

```pwsh
.\target\release\installer_builder.exe pack `
    --product   myapp `
    --publisher "My Company" `
    --to-version 1.0 `
    --input     .\build\myapp-1.0 `
    --exe       myapp.exe `
    --priv-key  .\keys\priv.key `
    --pub-key   .\keys\pub.key `
    --out       .\dist\setup-myapp-1.0.exe
```

## Required options

Supply each of these on the CLI or in a [config file](config.md):

| Option | Meaning |
|---|---|
| `--product` | Product name (the key the installer is identified by). |
| `--publisher` | Vendor name. Sets the Add/Remove Programs "Publisher" field and the per-user uninstall data folder `%LOCALAPPDATA%\<publisher>\Uninstall\<product>`. Must not be empty. |
| `--to-version` | Version string, e.g. `1.0` or `1.2.3`. Also parsed as `a.b.c.d` for the version-info resource. |
| `--input` | Directory containing the files to install. Scanned recursively. |
| `--exe` | Main executable, **relative to `--input`**, e.g. `bin\myapp.exe`. Drives shortcuts, the "Run now" checkbox, and file associations. |
| `--priv-key` | Ed25519 private key that signs the payload. |
| `--out` | Output installer path. Parent dirs are created. |

`--pub-key` is required too **unless** you pass a prebuilt stub — see
[With vs. without the Rust toolchain](toolchain.md).

## What pack does, in order

1. Loads the signing key; validates arguments (e.g. rejects empty
   `--publisher`, and case-only filename collisions that would clash on NTFS).
2. Scans `--input`, hashes every file with BLAKE3, and compresses them into the
   payload zip (`full/<rel>` entries; already-compressed media is stored
   verbatim, everything else is zstd-19).
3. Builds the signed `InstallerPayload` (manifest + metadata), signs the exact
   JSON bytes with Ed25519.
4. Produces the installer stub + uninstaller (via `cargo build`, or from a
   prebuilt kit), copies the stub to `--out`.
5. Embeds the signed manifest, the icon-stamped uninstaller, and a version-info
   resource; appends the payload zip as a PE overlay.
6. Prints the next step: the `signtool` command for Authenticode.

## Inspect the result

```pwsh
.\dist\setup-myapp-1.0.exe --verify
```

Verifies the embedded payload and prints kind / versions / payload size without
installing anything.

Next: [Patch installer](patch.md).
