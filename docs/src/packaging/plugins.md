# Plugins (custom DLLs)

Run your own install/uninstall logic without touching the installer's source.
A **plugin** is a native Windows DLL — written in C/C++/Rust/anything with a C
ABI — bundled into the (signed) installer payload and run at a chosen phase.

Plugins are a **migration pair**: `up` runs at install, `down` runs at
uninstall. Write `down` to reverse `up` when that's possible; otherwise make it
a no-op. The SDK + examples live in [`sdk/`](https://github.com/Tooltip-Focus/Installway/tree/main/sdk).

## The contract

A plugin exports three C functions (see
[`sdk/installway_plugin.h`](https://github.com/Tooltip-Focus/Installway/blob/main/sdk/installway_plugin.h)):

```c
uint32_t installway_abi_version(void);              // return INSTALLWAY_ABI_VERSION
int32_t  installway_up(const InstallwayContext*);   // at install   (0 = ok)
int32_t  installway_down(const InstallwayContext*); // at uninstall (0 = ok)
```

The host passes a context: `install_dir`, `product`, `product_id`, `version`,
the full `exe` path, and a `log(level, message)` callback that writes to the
install/uninstall log.

## Declaring plugins

Config-file only (`[[plugin]]` tables):

```toml
[[plugin]]
name  = "uninstall-old-msi"
dll   = "plugins/uninstall_old_msi.dll"   # path to the built DLL
phase = "pre-install"                     # pre-install | post-install
required = true                           # default true
```

| Field | Meaning |
|---|---|
| `name` | Unique id (ASCII letters/digits/`-`/`_`). Names the in-payload DLL and the log lines. |
| `dll` | Path to the DLL to bundle. |
| `phase` | `pre-install` (before any file is staged) or `post-install` (after the install is finalized). |
| `required` | If `true` (default), a non-zero `up` **fails the install**. If `false`, it's logged and the install continues. |

Plugins of the same phase run in declared order; `down` runs in reverse order
at uninstall.

## Phases & failure

| Phase | When | A required `up` failure |
|---|---|---|
| `pre-install` | before staging/commit | aborts cleanly — nothing is committed |
| `post-install` | after finalize (files in place, product registered) | fails the install (files stay; uninstall removes them) |
| `down` | at uninstall, before files are removed | always best-effort: logged, never blocks the uninstall |

## Example — switch from MSI/InstallShield

A common use is removing a previous-technology install before laying down the
new one:

```toml
[[plugin]]
name  = "uninstall-old-msi"
dll   = "plugins/uninstall_old_msi.dll"
phase = "pre-install"
[[plugin]]
name  = "uninstall-old-installshield"
dll   = "plugins/uninstall_old_is.dll"
phase = "pre-install"
```

Ready-to-edit **Rust** sources are in
[`sdk/examples/`](https://github.com/Tooltip-Focus/Installway/tree/main/sdk/examples)
(`uninstall_msi`, `uninstall_installshield`, and a minimal template). Build with
`cargo build --release`. A plugin can be any language with a C ABI — C/C++
authors use [`installway_plugin.h`](https://github.com/Tooltip-Focus/Installway/blob/main/sdk/installway_plugin.h);
see [`sdk/README.md`](https://github.com/Tooltip-Focus/Installway/blob/main/sdk/README.md).

## Toolchain-free

Plugins are just bundled binaries, so they work in
[toolchain-free packaging](../building/toolchain.md): nothing is compiled on the
packaging machine. Build the DLLs once (anywhere), then reference them from
`pack.toml`.

## Guardrails

- **Signed & hash-checked.** The DLL rides inside the Ed25519-signed payload,
  and its BLAKE3 is re-verified before it's loaded — a tampered DLL is refused.
- **Crash-isolated.** Each plugin runs in a **child process** (the
  installer/uninstaller re-launched as a hidden host), so a crashing or hanging
  plugin can't take down or stall the install. It's killed past a timeout.
- **ABI-checked.** The host refuses a plugin whose `installway_abi_version()`
  doesn't match.

## ⚠️ A note on AV

A bundled, signed DLL is far less alarming than spawning PowerShell, but loading
a DLL and spawning `msiexec` is still watched by EDR. Sign the final `.exe`
([Authenticode](signing.md)) to build reputation. Keep plugins to genuine
install needs.
