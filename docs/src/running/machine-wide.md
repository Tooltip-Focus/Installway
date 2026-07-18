# Per-user and machine-wide installs

The same installer serves both scopes, and the choice is made at install
time by the destination folder. A per-user location (the default,
`%LOCALAPPDATA%\Programs\<product>`) needs no admin rights and is visible
only to the installing user. A shared location such as `Program Files` makes
the install machine-wide: it elevates once and registers the product for
every user on the machine.

## How elevation works

The installer runs without elevation by default (`asInvoker` manifest). When
the chosen install folder requires administrator rights, the wizard and the
minimal UI show a UAC prompt automatically. The main window stays visible
while a hidden elevated subprocess performs the file operations.

Silent mode is the exception: it never shows a UAC prompt, since a prompt
would defeat "silent". To install silently to a machine location, run the
installer from an already-elevated context. See
[Install modes](install.md#silent).

## What changes with the scope

| | Per-user install | Machine-wide install |
|---|---|---|
| Elevation | None | One UAC prompt |
| Uninstall data folder | `%LOCALAPPDATA%\<publisher>\Uninstall\<product-id>` | `%ProgramData%\<publisher>\Uninstall\<product-id>` |
| Windows Apps entry | `HKCU\...\Uninstall`, visible to the installing user | `HKLM\...\Uninstall`, visible to every user |
| [File associations](../packaging/associations.md) | `HKCU\Software\Classes` | `HKLM\Software\Classes` |
| [Shortcut](../packaging/shortcuts.md) tokens `%DESKTOP%` / `%START_MENU%` | Per-user Desktop and Start Menu | All-Users Desktop and Start Menu |
| [Registry entries](../packaging/registry.md) with `hive = "HKLM"` | Logged and skipped | Written |
| [Plugins](../packaging/plugins.md) | Run as the user | Run in the elevated subprocess, under the admin account |

The scope is recorded in `installer_info.json` (`requires_admin`), and the
uninstaller mirrors it: uninstalling a machine-wide install elevates and
cleans `HKLM` and `%ProgramData%`; uninstalling a per-user install does not.

## Notes for plugin authors

For a machine-wide install, your `up` and `down` code runs under the elevated
admin account, not the user who launched the installer. Per-user APIs
(`%APPDATA%`, `HKEY_CURRENT_USER`, the user's Desktop) resolve to the admin's
profile there. Persist state under `ctx->data_dir` instead; it always points
at the right scope. See
[Plugins](../packaging/plugins.md#phases-and-failure).
