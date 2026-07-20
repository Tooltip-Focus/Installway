# Shortcuts

Declare the `.lnk` shortcuts you want in the config file. The installer
creates them, the uninstaller removes them, and an upgrade reconciles the set,
just like file associations and registry entries.

Nothing is created unless you declare a `[[shortcut]]`. There is no automatic
desktop shortcut.

## Declaring shortcuts

Shortcuts are config-file only (`[[shortcut]]` tables), not CLI flags:

```toml
[[shortcut]]
dir     = "%DESKTOP%"   # folder the .lnk goes in
name    = "My App"      # file name without ".lnk"; also the label
target  = "%EXE%"       # what it points at (relative paths resolve to the install dir)
args    = ""            # optional command-line arguments
feature = ""            # optional feature-pack id gating this shortcut
```

| Field | Required | Description |
|---|---|---|
| `dir` | Yes | Directory the `.lnk` is placed in. Tokens are expanded at install time. |
| `name` | Yes | Shortcut file name without `.lnk`. Must be a single filename: no `\ / : * ? " < > \|`. |
| `target` | Yes | Shortcut target. A relative path resolves against the chosen install directory; an absolute path (or `%EXE%`) is used as is. |
| `args` | No | A string appended verbatim as the shortcut's command-line arguments. |
| `feature` | No | A [feature pack](features.md) id. When set, the shortcut is created only if that feature is active in the install. Empty means always created. |

The shortcut's working directory is set to the install directory. An empty
`dir`, `name`, or `target`, or an illegal character in `name`, fails the
build with a message naming the entry. A non-empty `feature` that no
`[[feature]]` declares also fails the build.

## Tokens

`dir`, `target`, and `args` are templates expanded at install time, so they
can reference the chosen install directory:

| Token | Expands to |
|---|---|
| `%DESKTOP%` | Desktop folder. All-Users for a machine-wide install, otherwise per-user. |
| `%START_MENU%` | Start Menu Programs folder. All-Users for a machine-wide install, otherwise per-user. |
| `%COMMON_DESKTOP%` | The All-Users (public) Desktop, always. Needs admin. |
| `%COMMON_START_MENU%` | The All-Users Start Menu Programs folder, always. Needs admin. |
| `%USER_DESKTOP%` | The per-user Desktop, always. |
| `%USER_START_MENU%` | The per-user Start Menu Programs folder, always. |
| `%INSTALL_DIR%` | The chosen install directory. |
| `%EXE%` | Full path to the installed main exe. |
| `%VERSION%` | The `to-version`. |
| `%PRODUCT%` | The display name. |
| `%PRODUCT_ID%` | The registry-safe id. |
| `%PUBLISHER%` | The publisher (sanitized). |

Use `%DESKTOP%` and `%START_MENU%` to follow the install scope automatically
(see [Per-user and machine-wide installs](../running/machine-wide.md)), or
the `%COMMON_*%` / `%USER_*%` variants to force a specific location. After
these, any remaining `%VAR%` is expanded as an environment variable, such as
`%APPDATA%`, so you can place a shortcut anywhere the user can write. A
shortcut whose `dir` resolves to a location the system cannot provide is
logged and skipped, not fatal.

## Examples

```toml
# Desktop shortcut to the main exe.
[[shortcut]]
dir = "%DESKTOP%"
name = "%PRODUCT%"
target = "%EXE%"

# Start Menu shortcut that launches with a flag.
[[shortcut]]
dir = "%START_MENU%"
name = "%PRODUCT%"
target = "%EXE%"
args = "--from-start-menu"

# A shortcut to a helper tool, dropped inside the install folder itself.
[[shortcut]]
dir = "%INSTALL_DIR%"
name = "Config Editor"
target = "bin/config-editor.exe"

# Group under a Start Menu subfolder.
[[shortcut]]
dir = "%START_MENU%\\My Company"
name = "%PRODUCT%"
target = "%EXE%"
```

## Gating on a feature pack

Set `feature` to a declared [feature pack](features.md) id to make a shortcut
conditional: it is created only when that feature ends up active for the
install. Features resolve after plugins run, so the decision reflects the
final active set, the same set that filters which files land on disk.

```toml
[[feature]]
id = "pro"
paths = ["pro/**"]

# This shortcut appears only when "pro" ends up active.
[[shortcut]]
dir = "%START_MENU%"
name = "%PRODUCT% Pro"
target = "%EXE%"
feature = "pro"
```

## Suppressing shortcuts at install time

Whoever runs the installer can suppress a shortcut kind regardless of what
the config declares:

| Flag | Effect |
|---|---|
| `--ignore-desktop-shortcuts` | No `.lnk` is created in any Desktop location (`%DESKTOP%`, `%COMMON_DESKTOP%`, `%USER_DESKTOP%`). |
| `--ignore-start-menu-shortcuts` | No `.lnk` is created in any Start Menu location (`%START_MENU%`, `%COMMON_START_MENU%`, `%USER_START_MENU%`). |

They apply to every [install mode](../running/install.md): wizard, minimal,
and silent. Shortcuts pointing elsewhere, such as `%INSTALL_DIR%` or a
`%VAR%` path, are unaffected. On an upgrade run with one of these flags, a
matching shortcut a previous install created is removed as part of the normal
reconciliation.

## Uninstall and upgrade

Each created shortcut's resolved `.lnk` path is recorded in
`installer_info.json`. On uninstall, every recorded `.lnk` is removed. A
locked file is retried, then queued for deletion at reboot.

On any reinstall over an existing copy, shortcuts the previous version
created but the new config no longer declares are deleted first (reconciled
by resolved `.lnk` path), then the current set is recreated. Renaming,
moving, or dropping a shortcut never leaves an orphan behind.

The reconciliation is crash-resilient: `installer_info.json` is written last,
so an interrupted install self-heals on the next run.
