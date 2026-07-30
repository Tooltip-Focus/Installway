# The config file

`pack` takes a long command line. Put the options in a TOML file instead and
pass `--config`:

```pwsh
# everything from the file
.\target\release\installer_builder.exe pack --config .\pack.toml

# file as a base, override one value on the CLI
.\target\release\installer_builder.exe pack --config .\pack.toml --to-version 1.1
```

Keys are flat and `snake_case`, matching the CLI long names. Unknown keys are
rejected, which catches typos. Booleans are `true` or `false`.

## Merge rules

CLI arguments override the file. Anything absent from both uses the built-in
default, or fails with a message naming the missing key if it is required.

- **Scalars and paths.** The CLI value wins; otherwise the file value;
  otherwise the default.
- **The `assoc` list.** A CLI `--assoc` list replaces the file's list
  entirely when given. It does not merge.
- **Booleans** (`force_reinstall`, `purge_unknown_files`, `skip_license`,
  `skip_path`, `upgrade_minimal_ui`, `show_uninstall_complete`,
  `reuse_stub`). Either source can turn them on.
- **Tables** (`[[shortcut]]`, `[[registry]]`, `[[plugin]]`, `[[feature]]`)
  and `feature_mode` are config-file only. There are no CLI equivalents.

## Required keys

Via CLI or file: `product`, `product_id`, `publisher`, `to_version`, `input`,
`out`, and exactly one of `priv_key` / `priv_key_literal`. In toolchain mode,
one of `pub_key` / `pub_key_literal` is also required. An invalid
`product_id` fails the build; see [Full installers](full.md).

## Key reference

### Identity and content

| Key | Type | Description |
|---|---|---|
| `product` | string | Display name. |
| `product_id` | string | Registry-safe id, stable across versions. |
| `publisher` | string | Vendor name. |
| `hintway_tenant_id` | string | Optional Hintway tenant UUID. Enables the Hintway build in toolchain mode; see [Install analytics](hintway.md). |
| `to_version` | string | Version being packaged. |
| `input` | path | Directory of files to install. |
| `exe` | string | Main executable, relative to `input`. |
| `out` | path | Output installer path. |

### Signing and stub

| Key | Type | Description |
|---|---|---|
| `priv_key` | path | Ed25519 private key file. |
| `priv_key_literal` | string | Private key as 64 hex chars. Mutually exclusive with `priv_key`. |
| `pub_key` | path | Public key file, compiled into the stub. Ignored in toolchain-free mode. |
| `pub_key_literal` | string | Public key as 64 hex chars. Mutually exclusive with `pub_key`. |
| `installer_stub` | path | Prebuilt `installer.exe`. Switches to [toolchain-free mode](toolchain.md); requires `uninstaller`. |
| `uninstaller` | path | Prebuilt `uninstall.exe`, paired with `installer_stub`. |
| `reuse_stub` | bool | Skip rebuilding the stub and uninstaller when they already exist (toolchain mode). |

### Patch mode

| Key | Type | Description |
|---|---|---|
| `from_version` | string | Previous version. Required together with `from_dir`. |
| `from_dir` | path | Previous version's files, for delta generation. |

### Wizard and install behavior

| Key | Type | Default | Description |
|---|---|---|---|
| `license` | path | built-in placeholder | UTF-8 EULA text shown on the License page. |
| `banner` | path | flat gray header | PNG painted across the wizard header. See [Branding](../packaging/branding.md). |
| `assoc` | array | `[]` | File associations, entries of the form `".ext:Description"`. |
| `default_install_dir` | string | `%LOCALAPPDATA%\Programs\<product>` | Install path the UI proposes. `%VAR%` env tokens are expanded. |
| `skip_license` | bool | `false` | Hide the License page. |
| `skip_path` | bool | `false` | Hide the Choose-location page. |
| `install_dir_restriction` | string | `enforce` | Whether a fresh interactive install may target a non-empty folder: `enforce`, `default_dir_only`, or `bypass`. See [Wizard pages and install location](../packaging/wizard.md). |
| `launch_option` | string | `checked` | The "launch now" checkbox on the final page: `checked`, `unchecked`, or `hidden`. |
| `upgrade_minimal_ui` | bool | `false` | Upgrades use the compact minimal UI; a first install still gets the wizard. |
| `show_uninstall_complete` | bool | `false` | Show a confirmation message box at the end of an interactive uninstall. |
| `min_installer_version` | string | `1.0.0` | Minimum installer stub version allowed to run this payload. |
| `purge_unknown_files` | bool | `false` | On a full install over an existing copy, remove files not in this build. Ignored for patches. |
| `force_reinstall` | bool | `false` | Dev: rewrite all files, remove orphans, skip the from-version check. |
| `feature_mode` | string | `sticky` | How an upgrade seeds the active feature set: `sticky` or `override`. See [Feature packs](../packaging/features.md). |

### Tables

Declared as arrays of tables. Standard TOML ordering applies: put all flat
keys above them, then the table blocks at the end of the file.

| Table | Purpose |
|---|---|
| `[[shortcut]]` | Shortcuts to create. See [Shortcuts](../packaging/shortcuts.md). |
| `[[registry]]` | Free-form registry entries. See [Registry keys](../packaging/registry.md). |
| `[[plugin]]` | Native DLL plugins. See [Plugins](../packaging/plugins.md). |
| `[[feature]]` | Feature packs mapping path globs to a feature id. See [Feature packs](../packaging/features.md). |

## Complete example

```toml
# pack.toml
product    = "My App"
product_id = "myapp"
publisher  = "My Company"
hintway_tenant_id = "your-tenant-id" # optional
to_version = "1.0"
input      = "build/myapp-1.0"
exe        = "myapp.exe"
out        = "dist/setup-myapp-1.0.exe"

priv_key = "keys/priv.key"
pub_key  = "keys/pub.key"

license = "legal/EULA-myapp-en.txt"
banner  = "branding/header-1400x144.png"
assoc   = [".myx:MyApp Document", ".myz:MyApp Archive"]

default_install_dir = "%LOCALAPPDATA%\\Programs\\MyApp"
launch_option       = "checked"

# Patch mode: uncomment to build a patch instead of a full installer.
# from_version = "0.9"
# from_dir     = "build/myapp-0.9"

# Toolchain-free mode: point at prebuilt binaries instead of cargo builds.
# pub_key above is then ignored; the stub carries its own baked-in key.
# installer_stub = "kit/installer.exe"
# uninstaller    = "kit/uninstall.exe"

[[shortcut]]
dir    = "%DESKTOP%"
name   = "%PRODUCT%"
target = "%EXE%"

[[shortcut]]
dir    = "%START_MENU%"
name   = "%PRODUCT%"
target = "%EXE%"

[[registry]]
hive  = "HKCU"
key   = "%APP_KEY%"
name  = "InstallDir"
type  = "sz"
value = "%INSTALL_DIR%"
```
