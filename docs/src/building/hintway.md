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

## Building with analytics

Set `HINTWAY_TENANT_ID` to your tenant UUID at build time; it is baked into
the binary. If the variable is absent, the feature compiles cleanly but
telemetry is silently disabled.

```pwsh
$env:HINTWAY_TENANT_ID = "your-tenant-id"
$env:INSTALLER_PUB_KEY = (Get-Content .\keys\pub.key).Trim()

cargo build --release -p installer -p uninstaller `
    --features installer/hintway,uninstaller/hintway
```

Then pack as usual. No extra `pack` flags are needed:

```pwsh
.\target\release\installer_builder.exe pack --config .\pack.toml --reuse-stub
```

Since analytics is a property of the stub and uninstaller binaries, this
pairs naturally with [toolchain-free packaging](toolchain.md): build the kit
once with the feature enabled, and every installer packed from it reports.

## Identity and privacy

Each installer run generates a fresh random UUID as its identity. Nothing is
written to disk, and there is no cross-run or cross-machine linking.

The only personal data that reaches Hintway's servers is the client IP
address, which is inherent to any HTTP request. Document this in your
product's privacy policy.

## Disabling analytics

Omit the `--features` flag. The `hintway_analytics` crate is declared
`optional = true` and is not downloaded, compiled, or linked when the feature
is off.
