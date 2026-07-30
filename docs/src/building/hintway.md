# Install analytics

Installway can optionally report install telemetry through
[Hintway](https://hintway.app). The feature is compiled in only when the
`hintway` Cargo feature is enabled; binaries built without it contain zero
Hintway code.

## What is tracked

All events are GDPR-safe: no install path, no username, no OS version, no
machine identifier, and no persistent file on disk.

| Data | Values |
|---|---|
| `operation` | `install` or `update` |
| `mode` | `silent`, `minimal`, or `interactive` |
| `privilege` | `admin`, `user`, or `unknown` |
| `lang` | Detected UI language code, such as `en` or `fr` |

Events sent:

| Event | When |
|---|---|
| `app_started` | The installer launches. Fired automatically by the SDK on init. |
| `stage_reached` | `extract`, then `finalize`, then `done`, as each phase completes. |
| `install_error` | Any failure. Carries a `category` and the `stage`; never a raw message or path. |
| `app_exit` | The installer exits. Fired automatically by the SDK on shutdown. |

The duration of each phase, and the total install time, is derived
server-side from the timestamps of consecutive events. There is nothing extra
to track.

Error categories (the value of `install_error.category`):
`version_mismatch`, `permission_denied`, `elevation_cancelled`,
`signature_failed`, `disk_full`, `unknown`.

## Configure analytics for a project

Set the tenant UUID in the project's `pack.toml`:

```toml
hintway_tenant_id = "your-tenant-id"
```

The tenant UUID is stored in the signed installer payload, then copied to
`installer_info.json` for the uninstaller. It is configuration, not a private
key; the payload signature prevents it from being changed without detection.

Hintway support still has to be compiled into the installer and uninstaller.
In the default toolchain mode, `pack` detects `hintway_tenant_id` and enables
the `hintway` Cargo feature automatically for both binaries:

```pwsh
.\target\release\installer_builder.exe pack --config .\pack.toml
```

When using `--reuse-stub`, the existing `installer.exe` and `uninstall.exe`
must already have been built with Hintway support. Omit `--reuse-stub` once
after adding `hintway_tenant_id` so the correct variants are rebuilt.

## Build a reusable Hintway kit

For [toolchain-free packaging](toolchain.md), build a generic Hintway-enabled
kit once. No tenant is baked into these binaries:

```pwsh
$env:INSTALLER_PUB_KEY = (Get-Content .\keys\pub.key).Trim()

cargo build --release -p installer -p uninstaller `
    --features installer/hintway,uninstaller/hintway
```

Distribute those `installer.exe` and `uninstall.exe` files with
`installer_builder.exe`. Each project selects its own tenant through
`hintway_tenant_id` when it packs an installer. A kit built without the
feature ignores this field and contains no Hintway code.

## Identity and privacy

Each installer run generates a fresh random UUID as its identity. Nothing is
written to disk, and there is no cross-run or cross-machine linking.

The only personal data that reaches Hintway's servers is the client IP
address, which is inherent to any HTTP request. Document this in your
product's privacy policy.

## Disabling analytics

Omit `hintway_tenant_id` from `pack.toml`. A Hintway-enabled binary then sends
no telemetry. To produce binaries containing no Hintway code at all, also
build without the `hintway` Cargo feature; the `hintway_analytics` crate is
optional and is not linked in that variant.
