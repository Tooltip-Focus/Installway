# Exit codes

These are the exit codes of the **installer** (`setup-*.exe`) in `--silent`
mode — the ones a launcher or deployment script branches on.

| Code | Meaning |
|---|---|
| `0` | Success. |
| `10` | Wrong installed version for this **patch**. The install is untouched — run the full installer instead. |
| `1` | Any other failure: bad signature, anti-rollback (`min_installer_version`), payload/file hash mismatch, disk full, app still running and cancelled, etc. |

## Handling code 10

A patch run against a version it wasn't built for is a **pre-flight refusal**:
nothing on disk is touched and the existing install keeps working. A launcher
can branch on `10` to fetch and run the full installer automatically:

```pwsh
.\patch-myapp-1.0-to-1.1.exe --silent "C:\path\to\app"
switch ($LASTEXITCODE) {
    0  { "updated" }
    10 { "version mismatch — running full installer"; .\setup-myapp-1.1.exe --silent "C:\path\to\app" }
    default { "install failed ($LASTEXITCODE)"; exit 1 }
}
```

## `--verify-install`

`--verify-install "<dir>"` exits `0` if every file in the installed manifest is
present and matches its hash, `1` if anything is `MISSING` or `CORRUPT`.
