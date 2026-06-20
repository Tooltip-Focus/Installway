# Install modes

A built installer runs in three modes. Mode is chosen by command-line flags;
the same `.exe` serves all three.

## Interactive

Double-click the `.exe`. The wizard walks License → Choose location → Progress →
Done. The Done page offers a **"Run program now"** checkbox (checked by default
when the manifest has an exe); Finish launches it.

No admin elevation by default (`asInvoker` manifest). If the chosen install
folder requires administrator rights (e.g. `C:\Program Files\...`), a UAC
prompt is shown automatically: the main UI stays visible while a hidden elevated
subprocess performs the file operations. In that case the uninstaller data and
the Add/Remove Programs entry are registered machine-wide (`%ProgramData%` +
`HKLM`) so every user on the machine can uninstall. Segoe UI, Common Controls
v6 visual styles, DPI-aware (`PerMonitorV2`). See
[Trimming the wizard](../packaging/wizard.md) to hide pages.

## Minimal (app-triggered self-update)

A compact windowed UI for updates an app launches for itself — no license page,
no folder picker, no Install button. It starts the moment it opens and shows
progress:

```pwsh
.\setup-myapp-1.1.exe --minimal "C:\path\to\install"
.\setup-myapp-1.1.exe --minimal "C:\path\to\install" --launch
```

It closes itself shortly after reaching 100 %; on error it stays open with the
message.

## Silent (`/S` style, IT-friendly)

```pwsh
.\setup-myapp-1.0.exe --silent "C:\path\to\install"
.\setup-myapp-1.0.exe --silent "C:\path\to\install" --launch
```

Progress prints to stderr; `--launch` runs the installed exe afterward. Branch
on the [exit code](../reference/exit-codes.md) — notably `10` means "wrong
installed version for this patch."

## Runtime behavior (every mode)

For each file in the manifest:

1. **Already correct** — destination exists and its BLAKE3 matches → skip. A
   re-run is effectively instant.
2. **Patchable** — patch installer, destination exists, manifest has a
   `PatchInfo` → apply the HDiffPatch delta, verify BLAKE3, atomic rename. Falls
   back to full extract on any failure.
3. **Full** — read `full/<rel>` from the payload, verify BLAKE3, atomic rename.

Files in `deleted_files` are removed afterward. `version.json` and
`installer_manifest.json` are written to the install root as the canonical
record (and the state any later patch needs).

### Transactional & crash-safe

Installs are two-phase: every changed file is staged and hash-verified **before**
anything in the live install is touched, then committed via backup + rename with
~5 s lock-retry (AV / Explorer / indexer). A failure rolls back to the exact
pre-install state; an interrupted commit self-heals from the journal on next
launch. Disk space is pre-checked; a single named mutex per install dir prevents
two installers racing on the same folder.

## Inspect / verify without installing

```pwsh
.\setup-myapp-1.0.exe --verify                          # check the embedded payload
.\setup-myapp-1.0.exe --verify-install "C:\path\to\app" # re-hash an installed copy
```

`--verify-install` reports `OK` / `MISSING` / `CORRUPT` per file (exit `0` clean,
`1` if anything is wrong) — handy for scripted health checks.

Next: [Uninstall](uninstall.md).
