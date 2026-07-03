# Installway

**A modern, open-source alternative to InstallShield, and a step up from
Inno Setup, for building Windows installers.**

Installway packages your app into a single signed `.exe` that installs fast,
verifies every byte, and updates existing installs with small binary patches
instead of full re-downloads. No proprietary scripting language to learn, no
MSI runtime, no admin rights required - just a native, antivirus-friendly
`.exe` your users can double-click and trust.

## Screenshots

<img width="1033" height="827" alt="Sample App Software Installation Progress" src="https://github.com/user-attachments/assets/f1091106-b147-427f-b278-45e645cf522b" />

## Documentation

Full guide: **<https://tooltip-focus.github.io/Installway/>**

Jump to:

- [Build the builder](https://tooltip-focus.github.io/Installway/getting-started/build-the-builder.html)
- [With vs. without the Rust toolchain](https://tooltip-focus.github.io/Installway/building/toolchain.html)
- [Install modes](https://tooltip-focus.github.io/Installway/running/install.html)
- [CLI reference](https://tooltip-focus.github.io/Installway/reference/cli.html)

## Why Installway

- **One file, zero setup**: everything your app needs is bundled into a
  single `.exe`. Nothing to install first, nothing to leave behind.
- **Fast, tight installs**: strong compression keeps downloads small, and
  installs run straight from the packed archive with no unnecessary I/O.
- **Verified, not just copied**: every installer is signed (Ed25519) and
  every file is hash-checked (BLAKE3) before it touches disk, so corrupted or
  tampered downloads are caught, not installed.
- **Small updates, not full reinstalls**: patch installers ship only the
  binary diff between versions, so a one-line code change doesn't cost users
  a multi-hundred-megabyte download.
- **No proprietary scripting language**: custom install logic is a native
  Windows DLL (C/C++/Rust, or anything with a C ABI), not an obscure bytecode
  format. That means it's readable, debuggable, and scannable by antivirus
  engines like any other native binary.
- **A modern, native interface**: a DPI-aware Win32 wizard out of the box,
  and plugins can add their own custom pages for licensing steps,
  configuration screens, or anything else your install needs.
- **No admin rights needed**: per-user installs, shortcuts, file
  associations and an Add/Remove Programs entry, all without an elevation
  prompt.
- **Crash-safe by design**: installs are transactional, with rollback and
  power-loss recovery, so a failed install never leaves a broken app behind.

## Workspace

| Crate | Type | Purpose |
|---|---|---|
| `common` | lib | Manifest types, BLAKE3 hashing, file scan, HDiffPatch wrapper, shared retry helpers. |
| `installer_builder` | bin | Offline build tool. Generates Ed25519 keypairs and packs a directory (or a `from`/`to` pair) into a self-contained installer `.exe`. |
| `installer` | bin | The installer stub. Verifies the signature against its compiled-in public key, checks per-file hashes, and either fresh-extracts the payload or applies HDiffPatch deltas in place. |
| `uninstaller` | bin | Built by the builder and embedded in the installer; written outside the app folder and registered under HKCU so the product shows up in Windows "Apps". |

## Security model

1. **Ed25519 signature** over the exact JSON bytes describing the payload. The
   public key is **compiled** into the stub (`INSTALLER_PUB_KEY`), never a
   swappable resource.
2. **BLAKE3 of the payload zip**, re-verified before a single byte is extracted.
3. **BLAKE3 per file**, checked after each write (full) or patch apply.
4. **Anti-rollback** via `min_installer_version`.
5. **Patch from-version pinning**: a patch refuses to run unless the target's
   `version.json` matches.

Authenticode is **not** handled in code. Sign the final `.exe` with `signtool`
as a post-build step (the builder prints the exact command). See
[Signing](https://tooltip-focus.github.io/Installway/packaging/signing.html).

## Quick start

```pwsh
# 1. build the packer
cargo build --release -p installer_builder

# 2. generate a signing keypair (once per product)
.\target\release\installer_builder.exe keygen --out .\keys

# 3. pack a full installer
.\target\release\installer_builder.exe pack `
    --product    "My App" --product-id myapp --publisher "My Company" --to-version 1.0 `
    --input      .\build\myapp-1.0 --exe myapp.exe `
    --priv-key   .\keys\priv.key --pub-key .\keys\pub.key `
    --out        .\dist\setup-myapp-1.0.exe
```

Patch installers, config files, packaging options (license, icon,
associations, wizard trimming), and **packaging without a Rust toolchain** are
all covered in the
[documentation](https://tooltip-focus.github.io/Installway/).

## License

MIT © 2026 Gaëtan Dezeiraud, Louis Pinaud. See [LICENSE](LICENSE).
