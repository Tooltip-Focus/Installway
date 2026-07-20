# Patch installers

A patch installer carries only what changed between two versions: binary
deltas for modified files (or full bytes where a delta would be larger), plus
the list of files to delete. Unchanged files have no payload entry at all, so
a patch is typically a fraction of the size of a full installer.

You enter patch mode by passing both `--from-version` and `--from-dir`:

```pwsh
.\target\release\installer_builder.exe pack `
    --product      "My App" `
    --product-id   myapp `
    --publisher    "My Company" `
    --from-version 1.0 --from-dir .\build\myapp-1.0 `
    --to-version   1.1 --input    .\build\myapp-1.1 `
    --exe          myapp.exe `
    --priv-key     .\keys\priv.key `
    --pub-key      .\keys\pub.key `
    --out          .\dist\patch-myapp-1.0-to-1.1.exe
```

`--from-version` and `--from-dir` are required together. Passing only one is
an error. Everything else works exactly as for a
[full installer](full.md), including the config file and toolchain-free mode.

## How the payload is chosen, per file

For each file in the new version:

| Case | What ships |
|---|---|
| New file (absent from the old directory) | The full file. |
| Unchanged (same BLAKE3 hash as the old file) | Nothing. The installer keeps the file already on disk. |
| Changed | The builder runs `hdiffz` to produce a delta. If the delta is smaller than the full file, the delta ships; otherwise the full file does. |

Files present only in the old version are recorded in `deleted_files` and
removed at install time.

## hdiffz.exe is required for real deltas

Delta generation calls `hdiffz.exe`, which must sit next to
`installer_builder.exe`. If it is missing, the builder prints:

```text
warning: ...\hdiffz.exe not found - patch payload will ship full files instead of HDiffPatch deltas
```

The patch installer still works; it just is not smaller than a full one.

## Version pinning

A patch installer records its `from_version` and refuses to run unless the
target install's `version.json` matches. This refusal happens before anything
is touched: the existing install keeps working, and in silent mode the
process exits with code `10` so a launcher can fall back to the full
installer. See [Exit codes](../reference/exit-codes.md).

## Dev option: force a reinstall from scratch

`--force-reinstall` (valid on full and patch builds) produces an installer
that skips the from-version check, rewrites every file without hash-skipping,
and removes orphan files, so the install matches the build exactly. It stays
fully transactional. Use it during development; ship normal installers to
users.
