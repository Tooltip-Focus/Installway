# Installway

Local, single-file `.exe` installer for Windows in the style of InstallShield /
MSI — but written in Rust and built around a BLAKE3 + HDiffPatch manifest
format. No network, no admin elevation, no MSI runtime.

Each output `.exe` carries its own payload: the file zip is appended as a PE
**overlay** (no size ceiling, streamed on at build, `mmap`-read at install),
while the signed manifest, the uninstaller and the payload length ride as small
`RT_RCDATA` resources.

## Screenshots

<img width="1033" height="827" alt="Sample App Software Installation Progress" src="https://github.com/user-attachments/assets/f1091106-b147-427f-b278-45e645cf522b" />

## Documentation

Full guide: **<https://tooltip-focus.github.io/Installway/>**

Jump to:

- [Build the builder](https://tooltip-focus.github.io/Installway/getting-started/build-the-builder.html)
- [With vs. without the Rust toolchain](https://tooltip-focus.github.io/Installway/building/toolchain.html)
- [Install modes](https://tooltip-focus.github.io/Installway/running/install.html)
- [CLI reference](https://tooltip-focus.github.io/Installway/reference/cli.html)

## Highlights

- **Single self-contained `.exe`** — payload as a PE overlay; multi-GB capable
  at roughly constant working memory.
- **Signed & verified** — Ed25519 over the exact manifest bytes (public key
  compiled into the stub), plus BLAKE3 of the whole payload and of every file.
- **Full or patch installers** — patches ship HDiffPatch deltas (or full bytes
  when a delta would be bigger) plus a delete list; unchanged files carry
  nothing.
- **Transactional** — two-phase commit, hash-verified staging, rollback, and
  power-loss recovery from a journal.
- **No admin** — per-user install, shortcuts, file associations and an
  Add/Remove Programs entry, all under `asInvoker`.
- **Native Win32 UI** — Segoe UI, Common Controls v6, DPI-aware. Interactive,
  minimal (app self-update), and silent modes.
- **Toolchain-free packaging** — hand a prebuilt kit so a release machine
  packages versions without a Rust toolchain.

## Workspace

| Crate | Type | Purpose |
|---|---|---|
| `common` | lib | Manifest types, BLAKE3 hashing, file scan, HDiffPatch wrapper, shared retry helpers. |
| `installer_builder` | bin | Offline build tool. Generates Ed25519 keypairs and packs a directory (or a `from`/`to` pair) into a self-contained installer `.exe`. |
| `installer` | bin | The installer stub. Verifies the signature against its compiled-in public key, checks per-file hashes, and either fresh-extracts the payload or applies HDiffPatch deltas in place. |
| `uninstaller` | bin | Built by the builder and embedded in the installer; written outside the app folder and registered under HKCU so the product shows up in Windows "Apps". |

## Security model

1. **Ed25519 signature** over the exact JSON bytes describing the payload. The
   public key is **compiled** into the stub (`INSTALLER_PUB_KEY`) — never a
   swappable resource.
2. **BLAKE3 of the payload zip**, re-verified before a single byte is extracted.
3. **BLAKE3 per file**, checked after each write (full) or patch apply.
4. **Anti-rollback** via `min_installer_version`.
5. **Patch from-version pinning** — a patch refuses to run unless the target's
   `version.json` matches.

Authenticode is **not** handled in code — sign the final `.exe` with `signtool`
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

## Building & testing

### Build

```pwsh
# debug build (all crates)
cargo build

# release build (all crates)
cargo build --release

# release build (builder only — what you ship to release machines)
cargo build --release -p installer_builder
```

### Run all tests

```pwsh
cargo test
```

91 tests across four crates:

| Crate | Tests | Coverage areas |
|---|---|---|
| `common` | 41 | i18n, model round-trips, BLAKE3, file utils, registry encoding, shortcuts, paths |
| `installer` | 30 | payload verification, extract/rollback/repair, path validation, plugin wizard, UI helpers |
| `installer_builder` | 18 | CLI arg parsing, pack options, version info, association rules |
| `uninstaller` | 2 | cleanup file removal, directory pruning |

To test a single crate:

```pwsh
cargo test -p common
cargo test -p installer
cargo test -p installer_builder
cargo test -p uninstaller
```

To run a specific test by name:

```pwsh
cargo test -p installer recover_rolls_back
```

### Preview the installer UI

The installer binary ships a `--preview` flag for rendering any wizard view without a real signed payload:

```pwsh
cargo build
.\target\debug\installer.exe --preview license
.\target\debug\installer.exe --preview license-patch   # patch-mode banner
.\target\debug\installer.exe --preview choose
.\target\debug\installer.exe --preview choose-patch
.\target\debug\installer.exe --preview progress
.\target\debug\installer.exe --preview done
.\target\debug\installer.exe --preview error
.\target\debug\installer.exe --preview plugin          # canned one-page wizard
.\target\debug\installer.exe --preview minimal         # compact updater UI
```

## License

MIT © 2026 Gaëtan Dezeiraud, Louis Pinaud. See [LICENSE](LICENSE).
