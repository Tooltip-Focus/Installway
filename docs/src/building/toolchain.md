# Packaging without the Rust toolchain

`installer_builder pack` needs two binaries to assemble an installer: the
installer stub (`installer.exe`) and the uninstaller (`uninstall.exe`). There
are two ways to get them, and choosing between them is the main decision when
you set up a packaging pipeline.

| | Toolchain mode (default) | Toolchain-free mode (prebuilt kit) |
|---|---|---|
| Stub and uninstaller | Built on demand by `cargo build` | Supplied as prebuilt `.exe` files |
| Needs Rust and the source tree | Yes, on the packaging machine | No |
| Public key | Passed as `--pub-key`, compiled in per build | Already baked into the prebuilt stub |
| How you select it | Nothing; this is the default | Pass `--installer-stub` and `--uninstaller` |
| Who runs it | You or your CI, where Rust lives | Anyone on any Windows machine |

Both modes produce equivalent installers as far as the end user is concerned.
Icon stamping, version info, the overlay payload, the signature, and
Authenticode signing all behave identically. The only difference is where the
stub comes from and whether the packaging machine needs Rust.

## Toolchain mode (default)

This is what every example in [Full installers](full.md) and
[Patch installers](patch.md) uses. You pass `--pub-key`, and `pack` invokes
`cargo build` to produce a fresh stub with that public key compiled in, plus
the uninstaller:

```pwsh
.\target\release\installer_builder.exe pack `
    --product   "My App" --product-id myapp --publisher "My Company" --to-version 1.0 `
    --input     .\build\myapp-1.0 --exe myapp.exe `
    --priv-key  .\keys\priv.key `
    --pub-key   .\keys\pub.key `
    --out       .\dist\setup-myapp-1.0.exe
```

Under the hood, `pack` runs from the workspace root:

```pwsh
# installer stub, with your public key threaded in as a build-time env var
cargo build -p installer --release      # INSTALLER_PUB_KEY=<pub.key>
# uninstaller
cargo build -p uninstaller --release
```

When `pack.toml` contains `hintway_tenant_id`, `pack` also enables the
`hintway` feature on both builds. See [Install analytics](hintway.md).

This mode requires a working Rust toolchain and the Installway source tree on
the machine running `pack`.

### Speed up repeat builds

Add `--reuse-stub` to skip rebuilding the stub and uninstaller when
`target\release\installer.exe` and `uninstall.exe` already exist:

```pwsh
.\target\release\installer_builder.exe pack --config .\pack.toml --reuse-stub
```

Use it in a loop where the key has not changed; it turns a cargo rebuild into
a copy. Drop the flag whenever you change the public key, so the stub is
rebuilt with the new key.

## Toolchain-free mode (prebuilt kit)

To let someone package versions without installing Rust (a release engineer, a
build server, a CI job with no Rust step), build the binaries once yourself
and hand over a kit. The packager then runs `pack` pointing at those prebuilt
binaries.

### Step 1: build the kit, once, with the toolchain

Bake your public key into the stub and build all three binaries:

```pwsh
$env:INSTALLER_PUB_KEY = (Get-Content .\keys\pub.key).Trim()
cargo build --release -p installer -p uninstaller -p installer_builder
```

Collect a kit folder containing:

```text
kit\
    installer_builder.exe   the packer
    installer.exe           the stub, with YOUR pub.key compiled in
    uninstall.exe           the uninstaller
    priv.key                the signing key (KEEP SECRET)
    hdiffz.exe              optional, for patch deltas
```

`priv.key` in the kit is sensitive: whoever holds it can sign installers that
your stubs will accept. Hand the kit only to trusted packagers, over a secure
channel.

### Step 2: pack, anytime, on any Windows machine

```pwsh
.\installer_builder.exe pack `
    --product   "My App" --product-id myapp --publisher "My Company" --to-version 1.0 `
    --input     .\files --exe myapp.exe `
    --installer-stub .\installer.exe `
    --uninstaller    .\uninstall.exe `
    --priv-key       .\priv.key `
    --out            .\setup-myapp-1.0.exe
```

What differs from toolchain mode:

- `--installer-stub` and `--uninstaller` point at the prebuilt binaries.
  Passing them switches `pack` into toolchain-free mode; it never invokes
  `cargo`. The builder prints "Toolchain-free mode: using prebuilt binaries
  (no cargo build)".
- `--pub-key` (and the `pub_key` config key) is ignored. The public key is
  already compiled into `installer.exe`, and `pack` warns if you pass one.
- `--priv-key` must match the public key baked into the supplied stub.
  `pack` checks this; see below.

Patch builds work identically: add `--from-version` and `--from-dir`.

## The one trap: the key must match the stub

This is the single failure mode unique to toolchain-free mode. The prebuilt
`installer.exe` has a public key compiled in, from when you built the kit.
`--priv-key` signs the payload, and the stub verifies the signature against
its baked-in key at install time. If the private key does not pair with the
stub's key, or the stub was built without `INSTALLER_PUB_KEY` at all, the
installer rejects its own payload:

```text
installer was built without INSTALLER_PUB_KEY - refusing to install
```

`pack` catches this at build time. After producing the `.exe`, it runs the
installer's own `--verify` as a self-check, so a keyless or mismatched stub
fails the build:

```text
self-verify failed (...\setup.exe --verify exited 1). The produced installer
rejects its own payload ...
```

### Fixing it

A common mistake is grabbing an `installer.exe` from a plain `cargo build`,
which has no key, and pointing the kit at it. Rebuild the stub and the
uninstaller with your key, then refresh the kit:

```pwsh
$env:INSTALLER_PUB_KEY = (Get-Content .\keys\pub.key).Trim()
cargo build --release -p installer -p uninstaller
# copy target\release\installer.exe and uninstall.exe into the kit folder
```

Two rules of thumb:

- `priv.key` and the kit's `installer.exe` must come from the same `pub.key`.
- Re-issuing a stub with a new key means re-issuing the kit's `priv.key` too.

You can spot-check any built installer yourself. `--verify` prints text and
sets the exit code (`0` means ok), with no dialog:

```pwsh
.\setup-myapp-1.0.exe --verify
```

## Paired-argument rules

- `--installer-stub` and `--uninstaller` must be provided together. Passing
  one without the other is an error.
- In toolchain mode (neither prebuilt binary given), `--pub-key` or
  `--pub-key-literal` is required.
- Both prebuilt paths must exist, or `pack` stops before doing any work.
