# Full installers

A full installer carries every file of one product version. Run it on a clean
machine or over any existing install: it writes the complete set.

```pwsh
.\target\release\installer_builder.exe pack `
    --product    "My App" `
    --product-id myapp `
    --publisher  "My Company" `
    --to-version 1.0 `
    --input      .\build\myapp-1.0 `
    --exe        myapp.exe `
    --priv-key   .\keys\priv.key `
    --pub-key    .\keys\pub.key `
    --out        .\dist\setup-myapp-1.0.exe
```

## Required options

Supply each of these on the CLI or in a [config file](config.md):

| Option | Description |
|---|---|
| `--product` | Display name. Shown in the wizard, in Windows Apps, in the version-info resource, and as the default shortcut label. |
| `--product-id` | Registry-safe internal id, distinct from `--product`. Drives the Uninstall registry key, association ProgIDs, the uninstall data folder, and upgrade detection. Must match `^[A-Za-z][A-Za-z0-9._-]{0,49}$`. Keep it stable across versions. |
| `--publisher` | Vendor name. Sets the "Publisher" field in Windows Apps and the uninstall data folder `%LOCALAPPDATA%\<publisher>\Uninstall\<product-id>`. Must not be empty. |
| `--to-version` | Version string, such as `1.0` or `1.2.3`. Also parsed as `a.b.c.d` for the version-info resource. |
| `--input` | Directory containing the files to install. Scanned recursively. |
| `--priv-key` | Ed25519 private key that signs the payload. Or pass `--priv-key-literal <hex>` instead; see [Signing keys](../getting-started/signing-keys.md). |
| `--out` | Output installer path. Parent directories are created. |

`--pub-key` is also required, unless you pass a prebuilt stub. See
[Packaging without the Rust toolchain](toolchain.md).

## The main executable

`--exe` names your application's main executable, relative to `--input`, for
example `myapp.exe` or `bin\myapp.exe`. It drives:

- the "Run program now" checkbox on the wizard's final page and the
  `--launch` flag,
- the icon inherited by the setup `.exe` and the uninstaller,
- the target of [file associations](../packaging/associations.md),
- the `%EXE%` token in [shortcuts](../packaging/shortcuts.md) and
  [registry entries](../packaging/registry.md).

`--exe` is technically optional. Omit it only if your product has no
executable at all: without it there is no launch option, no icon inheritance,
and file associations and `%EXE%` tokens cannot resolve.

## What pack does, in order

1. Loads the signing key and validates arguments. It rejects an empty
   `--publisher`, an invalid `--product-id`, and case-only filename collisions
   that would clash on NTFS.
2. Scans `--input`, hashes every file with BLAKE3, and compresses the set into
   the payload zip. Already-compressed media formats are stored verbatim;
   everything else is compressed with zstd at level 19.
3. Builds the signed manifest and metadata, and signs the exact JSON bytes
   with Ed25519.
4. Produces the installer stub and the uninstaller, either through
   `cargo build` or from a prebuilt kit, and copies the stub to `--out`.
5. Embeds the signed manifest, the icon-stamped uninstaller, and a
   version-info resource, then appends the payload zip as a PE overlay.
6. Self-verifies by running the produced installer's own `--verify`. If the
   stub rejects the payload, for example because a prebuilt stub's key does
   not match `--priv-key`, the build fails here instead of shipping a broken
   installer.
7. Prints the `signtool` command for the Authenticode signing step.

The output is written to a `.tmp` path and renamed at the end, so a failed
build never leaves a half-written setup file at `--out`.

## Inspect the result

```pwsh
.\dist\setup-myapp-1.0.exe --verify
```

This verifies the embedded payload and prints its kind, versions, and size
without installing anything.
