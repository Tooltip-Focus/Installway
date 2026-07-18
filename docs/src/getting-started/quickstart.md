# Quickstart

This page takes you from a clean checkout to a signed, verified installer in
four steps. You need a Windows machine with a Rust toolchain. (Packaging
machines without Rust are covered in
[Packaging without the Rust toolchain](../building/toolchain.md).)

## 1. Build the build tool

Everything starts from one tool: `installer_builder`. Build it once from the
workspace root:

```pwsh
cargo build --release -p installer_builder
```

The binary lands at `target\release\installer_builder.exe`. It has two
subcommands:

| Command | What it does |
|---|---|
| `keygen` | Generate an Ed25519 signing keypair. |
| `pack` | Pack a directory into a self-contained installer `.exe`. |

You do not build the installer stub yourself. By default, `pack` runs
`cargo build` for the stub and the uninstaller on demand, with your public key
compiled in.

## 2. Generate a signing key

Every installer is signed. Generate a keypair once per product:

```pwsh
.\target\release\installer_builder.exe keygen --out .\keys
```

This writes `keys\priv.key` and `keys\pub.key`. Keep `priv.key` secret and
back it up: there is no recovery if you lose it. See
[Signing keys](signing-keys.md) for how the two keys work together and how to
manage them.

## 3. Pack your first installer

Point `pack` at the directory that contains your application's files:

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

`pack` scans the input directory, hashes and compresses every file, signs the
manifest, builds the stub and uninstaller, and assembles the final `.exe`. It
finishes by running the installer's own `--verify` as a self-check, so a
broken build fails here instead of shipping.

Each option is explained in [Full installers](../building/full.md). Once the
command line grows, move the options into a TOML file and pass `--config`
instead. See [The config file](../building/config.md).

## 4. Test the result

Verify the embedded payload without installing anything:

```pwsh
.\dist\setup-myapp-1.0.exe --verify
```

Then double-click the `.exe` (or run it from a shell) to walk through the
wizard. To try a scripted install:

```pwsh
.\dist\setup-myapp-1.0.exe --silent
```

## Next steps

- **Sign the `.exe`.** Before distributing, apply an Authenticode signature
  with your code-signing certificate. The builder prints the exact `signtool`
  command after each pack. See [Authenticode signing](../packaging/signing.md).
- **Ship updates as patches.** A patch installer carries only the delta
  between two versions. See [Patch installers](../building/patch.md).
- **Customize the installer.** Add a license, a header banner, shortcuts,
  file associations, and more. Start with [Branding](../packaging/branding.md).

## Optional: hdiffz.exe for patch deltas

Patch installers can ship small binary deltas instead of whole files. Delta
generation requires `hdiffz.exe` next to `installer_builder.exe`:

```text
target\release\installer_builder.exe
target\release\hdiffz.exe            <- drop it here
```

Without `hdiffz.exe`, the builder still produces working patch installers. It
falls back to shipping each changed file in full and prints a warning. See
[Patch installers](../building/patch.md).
