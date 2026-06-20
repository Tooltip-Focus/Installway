# Uninstall

`uninstall.exe` and its metadata live **outside** the application folder so a
manual delete never orphans the Add/Remove Programs entry.

The data dir location depends on whether admin rights were required at install
time:

| Install type | Data dir | ARP entry |
|---|---|---|
| Per-user (default) | `%LOCALAPPDATA%\<publisher>\Uninstall\<product-id>\` | `HKCU\...\Uninstall\` |
| Machine-wide (Program Files) | `%ProgramData%\<publisher>\Uninstall\<product-id>\` | `HKLM\...\Uninstall\` |

```text
<data-dir>\
    uninstall.exe
    installer_info.json        (holds the real install_dir + associations)
    installer_manifest.json
```

Machine-wide installs appear in **Settings → Apps → Installed apps** for every
user on the machine. Per-user installs are visible only to the installing user.

The product appears in **Settings → Apps → Installed apps** (and classic
Add/Remove Programs). Uninstalling runs that `uninstall.exe`, which:

1. Reads `installer_info.json` to find the real `install_dir`.
2. Walks `installer_manifest.json` and removes every tracked file. If
   `requires_admin` is set and the current user is not elevated, a UAC prompt
   is shown first and the file operations run in a hidden elevated subprocess.
3. Removes the [shortcuts](../packaging/shortcuts.md) it created, file
   associations (only `.ext` defaults that still point at our ProgID), and any
   free-form [registry entries](../packaging/registry.md) (anti-stomp; empty
   created keys pruned).
4. Removes `version.json` + `installer_manifest.json` and empty subdirs.
5. Deletes the `Uninstall` registry entry (`HKCU` or `HKLM` to match install).
6. Spawns a `%TEMP%` stage-2 copy of itself that deletes the **app dir** and the
   **data dir** (including `uninstall.exe`), then schedules its own removal via
   `MoveFileExW(MOVEFILE_DELAY_UNTIL_REBOOT)`. No `cmd.exe`, no console flash.

If the app folder was already deleted by hand, steps 1–2 no-op and the registry
entry + data dir are still cleaned.

## Silent uninstall

```pwsh
uninstall.exe --silent
```

Skips the confirmation dialog — this is what the registry `QuietUninstallString`
invokes.

## Shortcuts

Shortcuts are **config-driven** — see [Shortcuts](../packaging/shortcuts.md).
The installer records each resolved `.lnk` path in `installer_info.json`, and
the uninstaller removes exactly those. An install that declares no shortcuts
creates none.

Next: [Manifest & payload format](../reference/payload.md).
