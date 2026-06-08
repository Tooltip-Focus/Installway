# Uninstall

`uninstall.exe` and its metadata live **outside** the application folder, in a
per-user data dir (mirroring InstallShield's "Installation Information" folder):

```text
%LOCALAPPDATA%\<publisher>\Uninstall\<product>\
    uninstall.exe
    installer_info.json        (holds the real install_dir + associations)
    installer_manifest.json
```

So a user who deletes the app folder by hand still has a working uninstaller —
**no orphan Add/Remove Programs entry**, like a commercial installer.

The product appears in **Settings → Apps → Installed apps** (and classic
Add/Remove Programs). Uninstalling runs that `uninstall.exe`, which:

1. Reads `installer_info.json` to find the real `install_dir`.
2. Walks `installer_manifest.json` and removes every tracked file.
3. Removes desktop / Start Menu shortcuts and file associations (only `.ext`
   defaults that still point at our ProgID).
4. Removes `version.json` + `installer_manifest.json` and empty subdirs.
5. Deletes the HKCU `Uninstall` registry entry.
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

If the payload's `manifest.exe` is non-empty, the installer drops two `.lnk`
files (per-user, no admin) pointing at `<install_dir>\<exe>`:

```text
%APPDATA%\Microsoft\Windows\Start Menu\Programs\<product>.lnk
%USERPROFILE%\Desktop\<product>.lnk
```

Both are removed by the uninstaller. Shortcut path logic is shared between
installer and uninstaller so the two never drift on naming.

Next: [Manifest & payload format](../reference/payload.md).
