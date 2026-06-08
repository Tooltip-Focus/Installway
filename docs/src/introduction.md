# Introduction

**Installway** is a local, single-file `.exe` installer for Windows in the style
of InstallShield / MSI — but written in Rust and built around a BLAKE3 +
HDiffPatch manifest format. No network, no admin elevation, no MSI runtime.

Each installer you ship is a self-contained `.exe`:

- the file payload (a zip) is appended as a **PE overlay** — no size ceiling,
  streamed on at build time, `mmap`-read at install time;
- the **signed manifest**, the **uninstaller**, and the payload length ride
  along as small `RT_RCDATA` resources;
- the application's own **icon** and a Win32 **version-info** resource are
  stamped on, so the setup file looks finished in Explorer.

## The workspace

| Crate | Type | Purpose |
|---|---|---|
| `common` | lib | Manifest types, BLAKE3 hashing, file scan, HDiffPatch wrapper, shared retry helpers. |
| `installer_builder` | bin | The **offline build tool** this guide is about. Generates Ed25519 keypairs and packs a directory (or a `from`/`to` pair) into a self-contained installer `.exe`. |
| `installer` | bin | The installer stub. Verifies the signature, checks per-file hashes, and either fresh-extracts the payload or applies HDiffPatch deltas in place. |
| `uninstaller` | bin | Built by the builder and embedded in the installer; registered under HKCU so the product shows up in Windows "Apps". |

## Security model

Every installer carries overlapping guarantees:

1. **Ed25519 signature** over the exact JSON bytes describing the payload. The
   public key is *compiled into* the installer stub (build-time
   `INSTALLER_PUB_KEY`) — never shipped as a swappable resource.
2. **BLAKE3 of the zip payload**, recorded in the signed manifest and
   re-verified before a single byte is extracted.
3. **BLAKE3 per file**, checked after each write (full extract) or patch apply.
4. **Anti-rollback** via `min_installer_version`.
5. **Patch from-version pinning**: a patch refuses to run unless the target's
   `version.json` matches its `from_version`.

> Authenticode is **not** handled in code. Sign the final `.exe` as a separate
> post-build step — see [Signing](packaging/signing.md). The builder prints the
> exact `signtool` command.

## What this book covers

This book is the operator's guide to **building and shipping** installers with
`installer_builder`: generating a key, packing full and patch installers, the
two ways to run the packer (**with or without the Rust toolchain** — the key
distinction for distributing the packaging job), and the packaging options
(license, icon, associations, signing). Install-time and uninstall-time
behavior are summarized in [Installing & uninstalling](running/install.md).

Start at [Build the builder](getting-started/build-the-builder.md).
