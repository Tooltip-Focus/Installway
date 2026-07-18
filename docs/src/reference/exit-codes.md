# Exit codes

## Installer (`setup-*.exe`)

These are the exit codes a launcher or deployment script branches on. They
apply to every mode; in `--silent`, `--verify`, and `--verify-install` runs
the error text also prints to the console instead of a dialog.

| Code | Meaning |
|---|---|
| `0` | Success. |
| `10` | Wrong installed version for this patch. The install is untouched; run the full installer instead. |
| `1` | Any other failure: bad signature, installer stub older than `min_installer_version`, payload or file hash mismatch, disk full, permission error, cancellation, and so on. |

### Handling code 10

A patch run against a version it was not built for is a pre-flight refusal:
nothing on disk is touched, and the existing install keeps working. A
launcher can branch on `10` to fetch and run the full installer
automatically:

```pwsh
.\patch-myapp-1.0-to-1.1.exe --silent "C:\path\to\app"
switch ($LASTEXITCODE) {
    0  { "updated" }
    10 { "version mismatch: running full installer"; .\setup-myapp-1.1.exe --silent "C:\path\to\app" }
    default { "install failed ($LASTEXITCODE)"; exit 1 }
}
```

### Verification flags

- `--verify` exits `0` when the embedded payload verifies, `1` otherwise.
- `--verify-install "<dir>"` exits `0` when every file in the installed
  manifest is present and matches its hash, `1` if anything is `MISSING` or
  `CORRUPT`.

## Uninstaller (`uninstall.exe`)

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | Failure. |
