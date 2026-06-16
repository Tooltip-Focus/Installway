# Shortcuts

You declare which `.lnk` shortcuts the installer creates in the config file.
The installer creates them, the uninstaller removes them, and an upgrade
reconciles the set — like associations and registry entries, **per-user, no
admin**.

> **No automatic shortcut.** Nothing is created unless you declare a
> `[[shortcut]]`.

## Schema

Shortcuts are config-file only (`[[shortcut]]` array-of-tables), not CLI flags:

```toml
[[shortcut]]
dir    = "%DESKTOP%"   # folder the .lnk goes in (tokens below)
name   = "My App"      # file name without ".lnk"; also the label
target = "%EXE%"       # what it points at (relative → install dir)
args   = ""            # optional free-form command-line arguments
```

| Field | Required | Meaning |
|---|---|---|
| `dir` | yes | Directory the `.lnk` is placed in. Tokens expanded at install time. |
| `name` | yes | Shortcut file name **without** `.lnk` (becomes `<name>.lnk`). Must be a single filename — no `\ / : * ? " < > \|`. |
| `target` | yes | Shortcut target. A **relative** path resolves against the chosen install dir; an absolute path (or `%EXE%`) is used as-is. |
| `args` | no | A string appended verbatim as the shortcut's command-line arguments. |

The working directory is set to the install dir. An empty `dir`, `name`, or
`target`, or an illegal character in `name`, fails the build with a message
naming the entry.

## Tokens

`dir`, `target` and `args` are templates expanded at **install** time (so they
can reference the chosen install dir):

| Token | Expands to |
|---|---|
| `%DESKTOP%` | the per-user Desktop folder |
| `%START_MENU%` | the per-user Start Menu **Programs** folder |
| `%INSTALL_DIR%` | the chosen install directory |
| `%EXE%` | full path to the installed main exe (`--exe`) |
| `%VERSION%` | `to-version` |
| `%PRODUCT%` | display name |
| `%PRODUCT_ID%` | the registry-safe id |
| `%PUBLISHER%` | publisher (sanitized) |

After those, any remaining `%VAR%` is expanded as an **environment variable**
(e.g. `%APPDATA%`, `%LOCALAPPDATA%`), so you can place a shortcut anywhere the
user can write. A shortcut whose `dir` uses a `%DESKTOP%` / `%START_MENU%`
location the system can't resolve is skipped (logged), not fatal.

## Examples

```toml
# Desktop shortcut to the main exe.
[[shortcut]]
dir = "%DESKTOP%"; name = "%PRODUCT%"; target = "%EXE%"

# Start Menu shortcut that launches with a flag.
[[shortcut]]
dir = "%START_MENU%"; name = "%PRODUCT%"; target = "%EXE%"; args = "--from-start-menu"

# A second shortcut to a helper tool, dropped inside the install folder itself.
[[shortcut]]
dir = "%INSTALL_DIR%"; name = "Config Editor"; target = "bin/config-editor.exe"

# Group under a Start Menu subfolder.
[[shortcut]]
dir = "%START_MENU%\\My Company"; name = "%PRODUCT%"; target = "%EXE%"
```

## Uninstall & upgrade

Each created shortcut's resolved `.lnk` path is recorded in
`installer_info.json`. On uninstall every recorded `.lnk` is removed (a locked
file is retried, then queued for reboot-time deletion).

On upgrade (major or minor — any reinstall over an existing copy), shortcuts the
previous version created but the new config no longer declares are **deleted
first** (reconciled by resolved `.lnk` path), then the current set is
(re)created. So renaming, moving, or dropping a shortcut never leaves an orphan.
This is crash-resilient like associations — `installer_info.json` is written
last, so an interrupted install self-heals on re-run.
