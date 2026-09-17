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
5. Removes `version.json` and `installer_manifest.json`, then the folders
   the [install folder policy](#the-install-folder) allows.
6. Deletes the Uninstall registry entry, in `HKCU` or `HKLM` to match the
   install.
7. Spawns a second-stage copy of itself from `%TEMP%` that applies the
   [install folder policy](#the-install-folder) once the first stage has
   exited, deletes the data directory (including `uninstall.exe` itself),
   then schedules its own removal at reboot. No `cmd.exe`, no console flash.

If the app folder was already deleted by hand, the file steps do nothing and
the registry entry and data directory are still cleaned.

## The install folder

The install folder can hold files the installer never wrote: files the
application or the user added, or files that were there before the install.
`--uninstall-dir-policy` (config key `uninstall_dir_policy`) sets what the
uninstaller does with it:

| Value | Effect |
|---|---|
| `purge` | Default. Removes the whole install folder, whatever it contains. |
| `tracked` | Removes only the tracked files, then each folder the installer created once it is empty. A folder that existed before the install is never removed, even when empty. |

The installer records the folders it creates in `installer_info.json`
(`created_dirs`): the install folder and any missing parent folder, plus the
payload's sub-folders. An upgrade into the same folder adds its own to the
list. Folder removal is never recursive under `tracked`, so a folder that
still holds anything stays in place and is noted in the uninstall log.

Every install, upgrade, reinstall or patch rewrites `uninstall.exe` and
`installer_info.json`, so the policy of the most recent build applies. A
product installed by a build predating the policy is purged, as before. A
product first installed by a build predating `created_dirs` has no record of
the folders it created: under `tracked`, those folders are kept.

`purge` refuses a recorded path that is a drive root, a
profile or system folder, or a parent of one.

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
