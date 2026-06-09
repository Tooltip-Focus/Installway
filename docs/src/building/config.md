# Config file

`pack` takes a long command line. Put any options in a TOML file and pass
`--config` instead. **CLI args override the file**; anything absent from both
uses the built-in default (only `min_installer_version` has one: `1.0.0`).

Keys are flat, `snake_case`, matching the CLI long names. **Unknown keys are
rejected** as a typo guard. Booleans are `true`/`false`; `assoc` is an array.

```toml
# pack.toml
product    = "My App"        # display name
product_id = "myapp"         # registry-safe id, stable across versions
publisher  = "My Company"
to_version = "1.0"
input      = "build/myapp-1.0"
exe        = "myapp.exe"
license    = "legal/EULA-myapp-en.txt"
assoc      = [".myx:MyApp Document", ".myz:MyApp Archive"]
priv_key   = "keys/priv.key"
pub_key    = "keys/pub.key"
out        = "dist/setup-myapp-1.0.exe"

# default install dir the UI proposes (per-app); %VAR% tokens are expanded.
default_install_dir = "%LOCALAPPDATA%\\Programs\\MyApp"

# trim the wizard (optional)
# skip_license = true   # hide the License page
# skip_path    = true   # hide the Choose-location page

# use the compact minimal UI for upgrades (first install still uses the wizard)
# upgrade_minimal_ui = true

# patch mode (optional)
# from_version = "0.9"
# from_dir     = "build/myapp-0.9"

# toolchain-free mode (optional) — see the toolchain chapter
# installer_stub = "kit/installer.exe"
# uninstaller    = "kit/uninstall.exe"

# dev (optional)
# force_reinstall = true
# reuse_stub      = true

# free-form registry keys (HKCU) - see the Registry keys page
# [[registry]]
# hive = "HKCU"; key = "%APP_KEY%"; name = "InstallDir"; type = "sz"; value = "%INSTALL_DIR%"
```

> `[[registry]]` is an array-of-tables: put all flat keys above it, then the
> `[[registry]]` blocks at the end of the file (standard TOML ordering).

Run it:

```pwsh
# everything from the file
.\target\release\installer_builder.exe pack --config .\pack.toml

# file as a base, override one value on the CLI
.\target\release\installer_builder.exe pack --config .\pack.toml --to-version 1.1
```

## Required keys

Via CLI **or** file: `product`, `product_id`, `publisher`, `to_version`,
`input`, `exe`, `priv_key`, `out`. A missing one fails with a message naming it.
An invalid `product_id` (see [Full installer](full.md)) also fails the build.

## Merge rules

- **Scalars / paths**: CLI value wins; otherwise the file value; otherwise the
  default (or an error if required).
- **`assoc` list**: a CLI `--assoc` list *replaces* the file's list when given
  (it does not merge).
- **Booleans** (`force_reinstall`, `skip_license`, `skip_path`, `reuse_stub`):
  either source can turn them on.
- **`min_installer_version`**: defaults to `1.0.0` when set in neither.

Next: [License, icon & version info](../packaging/branding.md).
