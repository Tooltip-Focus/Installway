# Hintway analytics

Installway optionally integrates [Hintway](https://hintway.app) to report
install telemetry. The feature is compiled in **only when the `hintway` Cargo
feature is enabled** — binaries built without it contain zero Hintway code.

## What is tracked

All events are GDPR-safe: no install path, no username, no OS version, no
machine identifier, no persistent on-disk file.

| Data | Values |
|---|---|
| `operation` | `install` \| `update` |
| `mode` | `silent` \| `minimal` \| `interactive` |
| `privilege` | `admin` \| `user` \| `unknown` |
| `lang` | Detected UI language code, e.g. `en`, `fr` |

**Events sent:**

| Event | When |
|---|---|
| `app_started` | Installer launches (automatic, fired by SDK on `init`) |
| `stage_reached` | `extract` → `finalize` → `done` as each phase completes |
| `install_error` | Any failure; carries `category` + `stage` — no raw message or path |
| `app_exit` | Installer exits (automatic, fired by SDK on `shutdown`) |

Duration of each phase (and total install time) is derived server-side from the
timestamps of consecutive events — nothing extra to track.

**Error categories** (value of `install_error.category`):

`version_mismatch` · `permission_denied` · `elevation_cancelled` ·
`signature_failed` · `disk_full` · `unknown`

## Building with Hintway

Set `HINTWAY_TENANT_ID` to your tenant UUID **at build time** — it is baked
into the binary via `option_env!`. If the variable is absent the feature
compiles cleanly but telemetry is silently disabled.

```pwsh
$env:HINTWAY_TENANT_ID = "your-tenant-id"
$env:INSTALLER_PUB_KEY = (Get-Content .\keys\pub.key).Trim()

cargo build --release -p installer -p uninstaller `
    --features installer/hintway,uninstaller/hintway
```

Then pack as usual — no extra `pack` flags needed:

```pwsh
.\target\release\installer_builder.exe pack `
    --product    "My App" `
    --product-id myapp `
    --publisher  "My Company" `
    --to-version 1.2.0 `
    --input      .\build\myapp-1.2 `
    --exe        myapp.exe `
    --priv-key   .\keys\priv.key `
    --out        .\dist\setup-myapp-1.2.exe
```

## Identity & privacy

Each installer run generates a fresh random UUID as its identity. Nothing is
written to disk. There is no cross-run or cross-machine linking.

The only personal data that reaches Hintway's servers is the client IP address,
which is inherent to any HTTP request. Document this in your product's privacy
policy.

## Disabling at build time

Simply omit the `--features hintway` flag. The `hintway_analytics` crate is
declared `optional = true` and is not downloaded, compiled, or linked when the
feature is off.
