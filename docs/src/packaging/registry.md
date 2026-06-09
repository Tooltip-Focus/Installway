# Registry keys

Beyond [file associations](associations.md), you can declare arbitrary registry
entries in the config file. The installer writes them, the uninstaller removes
them, and an upgrade reconciles the set — InstallShield/MSI-style, but
**per-user (HKCU) only**, so no admin / no UAC.

> **HKCU only.** The installer runs `asInvoker` (never elevates), so HKLM
> (machine-wide) is not supported — declaring it fails the build. Use `HKCU`;
> per-user `HKCR` lives under `HKCU\Software\Classes`.

## Schema

Registry entries are config-file only (`[[registry]]` array-of-tables), not CLI
flags:

```toml
[[registry]]
hive = "HKCU"
key  = "Software\\Acme\\App"   # subkey under the hive
name = "InstallDir"            # value name; omit/"" = (Default)
type = "sz"                    # see types below
value = "%INSTALL_DIR%"
```

### Types

| `type` | TOML `value` | Registry type |
|---|---|---|
| `sz` | string | `REG_SZ` |
| `expand_sz` | string | `REG_EXPAND_SZ` (Windows expands `%ENV%` at read) |
| `dword` | integer (0 – 4294967295) | `REG_DWORD` |
| `qword` | integer (≥ 0) | `REG_QWORD` |
| `multi_sz` | array of strings | `REG_MULTI_SZ` |
| `binary` | hex string (even length) | `REG_BINARY` |

A `type`/`value` mismatch, an unknown `type`, an `HKLM` hive, an empty `key`, or
a key starting with `\` all fail the build with a message naming the entry.

## Tokens

Key and string values are templates, expanded at **install** time (so they can
include the chosen install dir):

| Token | Expands to |
|---|---|
| `%APP_KEY%` | `Software\<publisher>\<product-id>` (publisher sanitized) |
| `%INSTALL_DIR%` | the chosen install directory |
| `%EXE%` | full path to the installed main exe |
| `%VERSION%` | `to-version` |
| `%PRODUCT%` | display name |
| `%PRODUCT_ID%` | the registry-safe id |
| `%PUBLISHER%` | publisher (sanitized) |

Use `%APP_KEY%` for your app's own root so the path follows `product-id`
automatically and you don't repeat it:

```toml
[[registry]]
hive = "HKCU"; key = "%APP_KEY%";            name = "InstallDir"; type = "sz";    value = "%INSTALL_DIR%"
[[registry]]
hive = "HKCU"; key = "%APP_KEY%";            name = "Version";    type = "sz";    value = "%VERSION%"
[[registry]]
hive = "HKCU"; key = "%APP_KEY%\\Settings";  name = "FirstRun";   type = "dword"; value = 1
```

## Example — a custom URL protocol

Register `duckfocus://` so links open the app (a common use beyond file
associations):

```toml
[[registry]]
hive = "HKCU"; key = "Software\\Classes\\duckfocus"; name = "";             type = "sz"; value = "URL:DuckFocus Protocol"
[[registry]]
hive = "HKCU"; key = "Software\\Classes\\duckfocus"; name = "URL Protocol"; type = "sz"; value = ""
[[registry]]
hive = "HKCU"; key = "Software\\Classes\\duckfocus\\shell\\open\\command"; name = ""; type = "sz"; value = "\"%EXE%\" \"%1\""
```

## Uninstall & upgrade

Written entries are recorded in `installer_info.json`. On uninstall each is
removed **anti-stomp** — a value is deleted only if it still equals what the
installer wrote, so a value the user later changed is left alone. Keys are then
**pruned only if empty**, walking up the parents we created; a shared key such
as `...\Run` keeps its other values and is never deleted.

On upgrade, entries the previous version declared but the new one drops are
removed (reconciled by `(hive, key, name)`); the rest are rewritten. This is
crash-resilient like associations — `installer_info.json` is the last thing
written, so an interrupted install self-heals on re-run.

## A note on AV

Registry writes are normal installer behavior — far less alarming than running
scripts. The one mild flag: writing under
`Software\Microsoft\Windows\CurrentVersion\Run` (autostart) is a persistence
indicator. It's common for legitimate apps, just be aware.
