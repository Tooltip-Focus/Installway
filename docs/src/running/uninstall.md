# Uninstall

`uninstall.exe` and its metadata live outside the application folder, so a
manual delete of the app directory never orphans the Windows Apps entry.

The location depends on the [install scope](machine-wide.md):

| Install type | Data folder | Apps entry |
|---|---|---|
| Per-user (default) | `%LOCALAPPDATA%\<publisher>\Uninstall\<product-id>\` | `HKCU\...\Uninstall\` |
| Machine-wide | `%ProgramData%\<publisher>\Uninstall\<product-id>\` | `HKLM\...\Uninstall\` |

```text
<data-dir>\
    uninstall.exe
    installer_info.json        (the real install_dir, associations, shortcuts, ...)
    installer_manifest.json
```

The product appears in Settings > Apps > Installed apps (and in classic
Add/Remove Programs). Machine-wide installs are listed for every user;
per-user installs only for the installing user.

## What uninstall does

Uninstalling runs `uninstall.exe`, which:

1. Reads `installer_info.json` to find the real install directory.
2. Runs plugin `down` functions, best-effort and in reverse declaration
   order. See [Plugins](../packaging/plugins.md#phases-and-failure).
3. Walks `installer_manifest.json` and removes every tracked file. If the
   install was machine-wide and the current user is not elevated, a UAC
   prompt is shown first and the file operations run in a hidden elevated
   subprocess.
4. Removes the [shortcuts](../packaging/shortcuts.md) it created, the
   [file associations](../packaging/associations.md) that still point at our
   ProgID, and the [registry entries](../packaging/registry.md) it wrote
   (anti-stomp; empty created keys are pruned).
5. Removes `version.json`, `installer_manifest.json`, and empty
   subdirectories.
6. Deletes the Uninstall registry entry, in `HKCU` or `HKLM` to match the
   install.
7. Spawns a second-stage copy of itself from `%TEMP%` that deletes the app
   directory and the data directory (including `uninstall.exe` itself), then
   schedules its own removal at reboot. No `cmd.exe`, no console flash.

If the app folder was already deleted by hand, the file steps do nothing and
the registry entry and data directory are still cleaned.

## Silent uninstall

```pwsh
uninstall.exe --silent
```

Skips the confirmation dialog. This is what the registry
`QuietUninstallString` invokes.

## Completion message

By default, an interactive uninstall ends without a confirmation dialog. To
show an "uninstall complete" message box at the end, build the installer
with `--show-uninstall-complete` (config key `show_uninstall_complete`).

## Language

The uninstaller picks its UI language the same way the installer does:
`--lang <code>`, then the `INSTALLWAY_LANG` environment variable, then the
OS locale, with English as the fallback. See
[Installer runtime flags](../reference/installer-cli.md#language-selection).
