# Introduction

Installway builds single-file `.exe` installers for Windows. You point it at a
directory of files, it produces a signed, self-contained setup executable that
your users can double-click. No MSI runtime, no proprietary scripting language,
no admin rights required.

Every installer you ship is one file that carries everything:

- The file payload (a zip) is appended to the executable as a PE overlay, so
  there is no size ceiling. It is streamed on at build time and memory-mapped
  at install time.
- The signed manifest, the uninstaller, and the payload length are embedded as
  small `RT_RCDATA` resources.
- Your application's icon and a Win32 version-info resource are stamped on, so
  the setup file looks finished in Explorer.

## Why Installway

- **One file, zero setup.** Everything the install needs is inside the `.exe`.
- **Verified, not just copied.** The payload is signed with Ed25519 and every
  file is hash-checked with BLAKE3 before it touches disk.
- **Small updates.** Patch installers ship binary diffs between versions
  instead of full re-downloads.
- **Crash-safe.** Installs are transactional, with rollback and power-loss
  recovery. A failed install never leaves a broken app behind.
- **No admin rights needed.** Per-user installs get shortcuts, file
  associations, and an entry in Windows Apps without an elevation prompt.
  Machine-wide installs are supported too, with a single UAC prompt.
- **Extensible with native code.** Custom install logic is a plain Windows DLL
  written in C, C++, Rust, or anything with a C ABI. It is readable,
  debuggable, and scannable by antivirus engines like any other binary.

## Security model

Each installer carries overlapping guarantees:

1. **Ed25519 signature** over the exact JSON bytes that describe the payload.
   The public key is compiled into the installer stub at build time, never
   shipped as a swappable resource.
2. **BLAKE3 hash of the payload zip**, recorded in the signed manifest and
   re-verified before a single byte is extracted.
3. **BLAKE3 hash per file**, checked after each write or patch apply.
4. **Version floor** via `min_installer_version`: a stub that is too old
   refuses the payload.
5. **Patch pinning**: a patch installer refuses to run unless the installed
   version matches its `from_version`.

Authenticode is not handled in code. You sign the final `.exe` with `signtool`
as a post-build step, and the builder prints the exact command. See
[Authenticode signing](packaging/signing.md).

## The workspace

| Crate | Type | Purpose |
|---|---|---|
| `common` | lib | Manifest types, BLAKE3 hashing, file scan, HDiffPatch wrapper, shared helpers. |
| `installer_builder` | bin | The offline build tool. Generates Ed25519 keypairs and packs a directory (or a from/to pair) into a self-contained installer `.exe`. |
| `installer` | bin | The installer stub. Verifies the signature, checks per-file hashes, and either extracts the payload or applies HDiffPatch deltas in place. |
| `uninstaller` | bin | Built by the builder and embedded in the installer. It lives outside the app folder and registers the product in Windows Apps. |

## How these docs are organized

- [Getting started](getting-started/quickstart.md) takes you from a clean
  checkout to a working, verified installer.
- [Building installers](building/full.md) covers the build tool in depth:
  full and patch installers, the config file, and packaging on machines
  without a Rust toolchain.
- [Customizing the installer](packaging/branding.md) covers everything you can
  declare at pack time: branding, wizard behavior, shortcuts, file
  associations, registry keys, feature packs, and plugins.
- [Shipping](packaging/signing.md) covers Authenticode signing and optional
  install analytics.
- [Installing and uninstalling](running/install.md) describes what the built
  installer does at runtime: the three install modes, per-user versus
  machine-wide installs, and uninstall.
- [Reference](reference/cli.md) has the complete tables: every CLI flag, exit
  code, and the payload format.

Start with the [Quickstart](getting-started/quickstart.md).
