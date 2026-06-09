# With vs. without the Rust toolchain

`installer_builder pack` needs two binaries to build an installer: the
**installer stub** (`installer.exe`) and the **uninstaller** (`uninstall.exe`).
There are two ways to get them, and choosing between them is the single most
important decision when you set up a packaging pipeline.

| | **Toolchain mode** (default) | **Toolchain-free mode** (prebuilt kit) |
|---|---|---|
| Stub + uninstaller | built on demand by `cargo build` | supplied as prebuilt `.exe` files |
| Needs Rust + the source tree | **Yes**, on the packaging machine | **No** |
| Public key | passed as `--pub-key`, compiled in per build | already baked into the prebuilt stub |
| How you select it | (nothing — the default) | pass `--installer-stub` + `--uninstaller` |
| Who runs it | you / your CI, where Rust lives | anyone on any Windows box (build server, release engineer, CI without Rust) |

Both modes produce **byte-for-byte equivalent installers** as far as the end
user is concerned: icon stamping, version-info, overlay payload, signature, and
signing all behave identically. The only difference is *where the stub comes
from* and *whether the packaging machine needs a Rust toolchain*.

---

## Mode 1 — With the toolchain (default)

This is what every example in [Full installer](full.md) and
[Patch installer](patch.md) uses. You pass `--pub-key`, and `pack` invokes
`cargo build` to produce a fresh stub with that public key compiled in, plus the
uninstaller:

```pwsh
.\target\release\installer_builder.exe pack `
    --product   "My App" --product-id myapp --publisher "My Company" --to-version 1.0 `
    --input     .\build\myapp-1.0 --exe myapp.exe `
    --priv-key  .\keys\priv.key `
    --pub-key   .\keys\pub.key `
    --out       .\dist\setup-myapp-1.0.exe
```

Under the hood pack runs, from the workspace root:

```pwsh
# installer stub — your public key threaded in as a build-time env var
cargo build -p installer --release      # INSTALLER_PUB_KEY=<pub.key>
# uninstaller
cargo build -p uninstaller --release
```

**Requirements:** a working Rust toolchain **and** the Installway source tree on
the machine running `pack`.

**Speed up repeat builds.** Add `--reuse-stub` to skip rebuilding the stub and
uninstaller when `target\release\installer.exe` / `uninstall.exe` already exist:

```pwsh
.\target\release\installer_builder.exe pack --config .\pack.toml --reuse-stub
```

Use this in a loop where the key hasn't changed — it turns a cargo rebuild into
a copy. (Drop it whenever you change the public key, so the stub is rebuilt with
the new key.)

### When to use toolchain mode

- You build releases on a machine that already has Rust (your dev box, your CI).
- You want the stub rebuilt from source each release.
- You're iterating locally.

---

## Mode 2 — Without the toolchain (prebuilt kit)

To let someone package versions **without installing Rust** — a release
engineer, a build server, a CI job with no Rust step — you build the binaries
**once** yourself and hand them a kit. They then run `pack` pointing at those
prebuilt binaries.

### Step 1 — Once, with the toolchain (you, the vendor)

Bake your public key into the stub and build all three binaries:

```pwsh
$env:INSTALLER_PUB_KEY = Get-Content .\keys\pub.key
cargo build --release -p installer -p uninstaller -p installer_builder
```

Collect a **kit** folder containing:

```text
kit\
    installer_builder.exe   ← the packer
    installer.exe           ← the stub, with YOUR pub.key compiled in
    uninstall.exe           ← the uninstaller
    priv.key                ← the signing key (KEEP SECRET)
    hdiffz.exe              ← optional, next to installer_builder.exe, for patch deltas
```

> **`priv.key` in the kit is sensitive.** Whoever holds it can sign installers
> your stubs will accept. Hand the kit only to trusted packagers, over a secure
> channel.

### Step 2 — Anytime, no toolchain (the packager, on any Windows box)

```pwsh
.\installer_builder.exe pack `
    --product   "My App" --product-id myapp --publisher "My Company" --to-version 1.0 `
    --input     .\files --exe myapp.exe `
    --installer-stub .\installer.exe `
    --uninstaller    .\uninstall.exe `
    --priv-key       .\priv.key `
    --out            .\setup-myapp-1.0.exe
```

Note what's **different** from toolchain mode:

- `--installer-stub` and `--uninstaller` point at the prebuilt binaries. Passing
  them switches `pack` into toolchain-free mode — it never invokes `cargo`.
- **`--pub-key` is not used.** The public key is already compiled into
  `installer.exe`. The builder prints *"Toolchain-free mode: using prebuilt
  binaries (no cargo build)"*.
- `--priv-key` **must match** the public key baked into the supplied stub.

Patch builds work identically — add `--from-version` / `--from-dir`.

### When to use toolchain-free mode

- The packaging machine has no Rust and you don't want to add it.
- You separate roles: a trusted vendor controls the stub + key; a packaging team
  only assembles installers.
- Your release CI shouldn't pay the cost (or carry the supply-chain surface) of
  a full Rust build on every run.

---

## The one trap: the key must match the stub

This is the failure mode unique to toolchain-free mode, so it's worth stating
plainly.

`--installer-stub installer.exe` has a **public key compiled in** (from when
*you* built it in Step 1). `--priv-key priv.key` signs the payload. The stub
verifies the signature against its baked-in public key **at install time**. If
the private key in the kit does not pair with that baked-in public key:

> The build **succeeds** and produces a setup `.exe`. But that installer
> **rejects its own payload at runtime** — "bad signature" — and refuses to
> install.

There is no build-time check for this, because `pack` can't see inside the
prebuilt stub. So:

- Keep `priv.key` and the `installer.exe` you ship in a kit **from the same
  build** (Step 1 used that `pub.key` to bake the stub *and* you ship its paired
  `priv.key`).
- Re-issuing a stub with a new key means re-issuing the kit's `priv.key` too.
- Always smoke-test a kit-built installer with `--verify` and a real install on
  a throwaway folder before shipping.

```pwsh
# sanity check a kit-built installer before release
.\setup-myapp-1.0.exe --verify
.\setup-myapp-1.0.exe --silent "C:\temp\smoke-test"
```

---

## Other paired-argument rules

- `--installer-stub` and `--uninstaller` must be **provided together**. Passing
  one without the other is an error.
- In toolchain mode (neither prebuilt binary given), `--pub-key` is **required**.
- Both prebuilt paths must exist, or `pack` bails before doing any work.

Next: [Config file](config.md).
