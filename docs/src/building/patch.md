# Patch installer

A **patch** installer carries only what changed between two versions: binary
deltas for modified files (or full bytes where a delta would be bigger), plus
the list of files to delete. Unchanged files have no payload entry at all, so a
patch is typically a fraction of the size of a full installer.

You enter patch mode by giving both `--from-version` and `--from-dir`:

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

> Both `--from-version` **and** `--from-dir` are required together. Passing
> only one is an error.

## How the delta is chosen, per file

For each file in the new version:

- **New file** (absent in the old dir) → shipped in full.
- **Unchanged** (same BLAKE3 as the old file) → no payload entry; the installer
  keeps the file already on disk.
- **Changed** → the builder runs `hdiffz` to produce a delta. If the delta is
  smaller than the full file it ships the delta; otherwise it ships the full
  file. Files present only in the old version are recorded in
  `deleted_files`.

## hdiffz.exe is required for real deltas

Delta generation calls `hdiffz.exe` next to `installer_builder.exe`. If it's
missing, the builder prints:

```text
warning: ...\hdiffz.exe not found - patch payload will ship full files instead of HDiffPatch deltas
```

The patch installer still works — it just isn't smaller than a full one. See
[Build the builder](../getting-started/build-the-builder.md#optional-hdiffpatch-deltas).

## Version pinning

A patch installer records its `from_version` and **refuses to run** unless the
target install's `version.json` matches. A patch run against the wrong version
is a pre-flight refusal: nothing is touched, the existing install keeps working,
and (in silent mode) the process exits with code `10` so a launcher can fetch
the full installer instead. See [Exit codes](../reference/exit-codes.md).

## Dev: reinstall from scratch

`--force-reinstall` (valid on full or patch builds) produces an installer that
skips the from-version check, rewrites every file (no hash-skip), and removes
orphan files so the install matches the build exactly. It stays fully
transactional. Intended for development — ship normal installers to users.

Next: [With vs. without the Rust toolchain](toolchain.md).
