# Registry keys

Beyond [file associations](associations.md), you can declare arbitrary
registry entries in the config file. The installer writes them, the
uninstaller removes them, and an upgrade reconciles the set.

`HKCU` entries need no admin rights. `HKLM` entries are written only when the
install is machine-wide; on a per-user install an `HKLM` entry is logged and
skipped. See [Per-user and machine-wide installs](../running/machine-wide.md).
For per-user class registrations (`HKCR`), write under
`HKCU\Software\Classes`.

## Declaring entries

Registry entries are config-file only (`[[registry]]` tables), not CLI flags:

```toml
[[registry]]
hive  = "HKCU"
key   = "Software\\Acme\\App"   # subkey under the hive
name  = "InstallDir"            # value name; omit or "" for (Default)
type  = "sz"                    # see types below
value = "%INSTALL_DIR%"
```

### Types

| `type` | TOML `value` | Registry type |
|---|---|---|
| `sz` | string | `REG_SZ` |
| `expand_sz` | string | `REG_EXPAND_SZ` (Windows expands `%ENV%` at read time) |
| `dword` | integer, 0 to 4294967295 | `REG_DWORD` |
| `qword` | integer, 0 or greater | `REG_QWORD` |
| `multi_sz` | array of strings | `REG_MULTI_SZ` |
| `binary` | hex string of even length | `REG_BINARY` |

A `type`/`value` mismatch, an unknown `type`, an unsupported hive (only
`HKCU` and `HKLM` are allowed), an empty `key`, or a key starting with `\`
all fail the build with a message naming the entry.

## Tokens

Keys and string values are templates, expanded at install time, so they can
include the chosen install directory:

| Token | Expands to |
|---|---|
| `%APP_KEY%` | `Software\<publisher>\<product-id>`, with the publisher sanitized. |
| `%INSTALL_DIR%` | The chosen install directory. |
| `%EXE%` | Full path to the installed main exe. |
| `%VERSION%` | The `to-version`. |
| `%PRODUCT%` | The display name. |
| `%PRODUCT_ID%` | The registry-safe id. |
| `%PUBLISHER%` | The publisher (sanitized). |

Use `%APP_KEY%` for your app's own root so the path follows `product-id`
automatically:

```toml
[[registry]]
hive = "HKCU"
key = "%APP_KEY%"
name = "InstallDir"
type = "sz"
value = "%INSTALL_DIR%"

[[registry]]
hive = "HKCU"
key = "%APP_KEY%"
name = "Version"
type = "sz"
value = "%VERSION%"

[[registry]]
hive = "HKCU"
key = "%APP_KEY%\\Settings"
name = "FirstRun"
type = "dword"
value = 1
```

## Example: a custom URL protocol

Register `myapp://` so links open the app, a common need beyond file
associations:

```toml
[[registry]]
hive = "HKCU"
key = "Software\\Classes\\myapp"
name = ""
type = "sz"
value = "URL:MyApp Protocol"

[[registry]]
hive = "HKCU"
key = "Software\\Classes\\myapp"
name = "URL Protocol"
type = "sz"
value = ""

[[registry]]
hive = "HKCU"
key = "Software\\Classes\\myapp\\shell\\open\\command"
name = ""
type = "sz"
value = "\"%EXE%\" \"%1\""
```

## Uninstall and upgrade

Written entries are recorded in `installer_info.json`. On uninstall, each is
removed with an anti-stomp check: a value is deleted only if it still equals
what the installer wrote, so a value the user later changed is left alone.
Keys are then pruned only if empty, walking up the parents the installer
created. A shared key such as `...\Run` keeps its other values and is never
deleted.

On upgrade, entries the previous version declared but the new one drops are
removed (matched by hive, key, and name), and the rest are rewritten. Like
associations, this is crash-resilient: `installer_info.json` is the last
thing written, so an interrupted install self-heals on the next run.

## A note on antivirus

Registry writes are normal installer behavior, far less alarming to AV
engines than running scripts. One mild flag to be aware of: writing under
`Software\Microsoft\Windows\CurrentVersion\Run` (autostart) is a persistence
indicator. It is common for legitimate apps; just know that scanners watch
it.
