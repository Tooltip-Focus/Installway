# Installway plugin SDK

Write custom install/uninstall logic as a **native DLL** without touching the
installer's source. A plugin is bundled into the (signed) installer payload and
run in an **isolated child process** at a chosen phase. See the full guide:
<https://tooltip-focus.github.io/Installway/packaging/plugins.html>.

## The contract

A plugin exports three C functions ([`installway_plugin.h`](installway_plugin.h)):

```c
uint32_t installway_abi_version(void);             // return INSTALLWAY_ABI_VERSION
int32_t  installway_up(const InstallwayContext*);  // at install   (0 = ok)
int32_t  installway_down(const InstallwayContext*); // at uninstall (0 = ok)
```

`up`/`down` are a migration pair: write `down` to reverse `up` when that's
possible; otherwise make it a no-op (`return 0`). The host passes an
`InstallwayContext` (install dir, product, product-id, version, exe path, and a
`log` callback that writes to the install/uninstall log).

## Examples (Rust)

A plugin can be written in **any** language that emits a DLL with this ABI; the
examples are Rust `cdylib`s (each a standalone crate):

- [`examples/uninstall_msi/`](examples/uninstall_msi) — silently uninstall a
  previous MSI before the new install (`msiexec /x <code> /qn`).
- [`examples/uninstall_installshield/`](examples/uninstall_installshield) — find
  a previous InstallShield product by name and run its uninstaller silently.
- [`examples/rust_plugin/`](examples/rust_plugin) — the bare minimal template.

C/C++ authors use [`installway_plugin.h`](installway_plugin.h) directly; the
Rust examples just declare the same `#[repr(C)]` struct + `extern "system"`
exports inline (no extra crate needed).

## Build a plugin DLL

```pwsh
# Rust (recommended)
cargo build --release --manifest-path examples\uninstall_msi\Cargo.toml
#   -> target\release\uninstall_old_msi.dll

# C, if you prefer — MSVC x64 Native Tools prompt
cl /LD /O2 /I . my_plugin.c /Fe:my_plugin.dll
```

## Use it

```toml
# pack.toml
[[plugin]]
name  = "uninstall-old-msi"
dll   = "plugins/uninstall_old_msi.dll"
phase = "pre-install"     # pre-install | post-install
required = true           # a required up failure fails the install
```

DLLs are bundled into the payload as you pack — works in
[toolchain-free mode](https://tooltip-focus.github.io/Installway/building/toolchain.html)
too (nothing to compile on the packaging machine).

## Guardrails

- The DLL rides inside the **Ed25519-signed** payload; its BLAKE3 is re-checked
  before loading.
- Runs in a **child process** — a crash or hang can't take down or stall the
  install (killed past a timeout).
- The host refuses a plugin whose `installway_abi_version()` doesn't match.
