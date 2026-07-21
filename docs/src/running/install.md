# Install modes

A built installer runs in three modes. The mode is chosen by command-line
flags; the same `.exe` serves all three. The full flag list is in
[Installer runtime flags](../reference/installer-cli.md).

## Interactive

Double-click the `.exe`. The wizard walks License, Choose location, Progress,
Done. The Done page offers a "Run program now" checkbox (its default state is
a [build-time option](../packaging/wizard.md#the-launch-now-checkbox)), and
Finish launches the product when it is ticked.

The UI uses Segoe UI, Common Controls v6 visual styles, and is DPI-aware
(PerMonitorV2). To hide pages or change the flow, see
[Wizard pages and install location](../packaging/wizard.md).

There is no elevation prompt by default. If the user picks a folder that
requires administrator rights, such as `C:\Program Files`, a UAC prompt
appears automatically and the install becomes machine-wide. See
[Per-user and machine-wide installs](machine-wide.md).

## Minimal (app-triggered self-update)

A compact windowed UI for updates an app launches for itself: no license
page, no folder picker, no Install button. It starts the moment it opens and
shows progress:

```pwsh
.\setup-myapp-1.1.exe --minimal "C:\path\to\install"
.\setup-myapp-1.1.exe --minimal "C:\path\to\install" --launch
```

It closes itself shortly after reaching 100%. On error, it stays open with
the message.

You can also make regular upgrades use this UI without passing a flag, via
the build-time
[`--upgrade-minimal-ui`](../packaging/wizard.md#minimal-ui-for-upgrades)
option.

## Silent

```pwsh
.\setup-myapp-1.0.exe --silent
.\setup-myapp-1.0.exe --silent "C:\path\to\install" --launch
```

Progress prints to stdout, and `--launch` runs the installed exe afterward.
Branch on the [exit code](../reference/exit-codes.md): notably, `10` means
the installed version does not match this patch.

Because the installer is a Windows GUI-subsystem executable, PowerShell does
not wait for it automatically. Start it as a process and wait explicitly when
the next step depends on the completed installation, for example in an Azure
Pipeline:

```pwsh
$p = Start-Process .\setup-dev.exe -ArgumentList "--silent", "$(Pipeline.Workspace)\INSTALL" -PassThru
$p.WaitForExit()
```

Silent mode never shows a UAC prompt, since a prompt would defeat "silent".
Installing to a machine location such as `Program Files` therefore fails
with a permission error unless you run the silent installer from an
already-elevated context, such as an admin shell or a deployment tool. A
per-user location needs no elevation.

If a plugin contributes wizard pages, a silent install answers them from
their declared defaults. A required field with no usable default fails the
install; see [Plugins](../packaging/plugins.md#silent-installs).

## What every mode does per file

For each file in the manifest:

1. **Already correct.** The destination exists and its BLAKE3 hash matches:
   skip. A re-run is effectively instant.
2. **Patchable.** Patch installer, the destination exists, and the manifest
   has patch info: apply the HDiffPatch delta, verify the BLAKE3 hash, and
   rename atomically. Falls back to a full extract on any failure.
3. **Full.** Read the file from the payload, verify the BLAKE3 hash, and
   rename atomically.

Files listed in `deleted_files` are removed afterward. `version.json` and
`installer_manifest.json` are written to the install root as the canonical
record, and as the state any later patch needs.

## Transactional and crash-safe

Installs are two-phase. Every changed file is staged and hash-verified before
anything in the live install is touched, then committed via backup and rename
with a retry window of about five seconds for files locked by antivirus,
Explorer, or the indexer. A failure rolls back to the exact pre-install
state. An interrupted commit self-heals from the journal on the next launch.
Disk space is pre-checked, and a named mutex per install directory prevents
two installers from racing on the same folder.

## Inspect and verify without installing

```pwsh
.\setup-myapp-1.0.exe --verify                          # check the embedded payload
.\setup-myapp-1.0.exe --verify-install "C:\path\to\app" # re-hash an installed copy
```

`--verify-install` reports `OK`, `MISSING`, or `CORRUPT` per file, and exits
`0` when clean or `1` when anything is wrong. It is handy for scripted health
checks.
