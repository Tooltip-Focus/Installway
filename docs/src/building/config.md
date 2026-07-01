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
banner     = "branding/header-1400x144.png"   # PNG header banner (optional); see Branding
assoc      = [".myx:MyApp Document", ".myz:MyApp Archive"]
priv_key   = "keys/priv.key"   # path to key file; or use priv_key_literal (see below)
pub_key    = "keys/pub.key"    # path to key file; or use pub_key_literal (see below)
out        = "dist/setup-myapp-1.0.exe"

# default install dir the UI proposes (per-app); %VAR% tokens are expanded.
default_install_dir = "%LOCALAPPDATA%\\Programs\\MyApp"

# trim the wizard (optional)
# skip_license = true   # hide the License page
# skip_path    = true   # hide the Choose-location page

# allow a fresh interactive install into a NON-EMPTY folder (optional).
#   enforce          = block a non-empty destination (default)
#   default_dir_only = allow only the proposed default_install_dir
#   bypass           = allow any folder
# Use when replacing a legacy InstallShield/MSI install in its own directory:
# pair with purge_unknown_files and a plugin that validates the old install
# (pre-install) and tears it down at uninstall (down).
# install_dir_restriction = "default_dir_only"

# use the compact minimal UI for upgrades (first install still uses the wizard)
# upgrade_minimal_ui = true

# show the "uninstall complete" message box at the end of an interactive uninstall
# show_uninstall_complete = true

# the "launch now" checkbox on the installer's final page (optional)
#   checked   = visible and ticked (default)
#   unchecked = visible but not ticked (user opts in)
#   hidden    = no checkbox; never offer to launch
# launch_option = "unchecked"

# patch mode (optional)
# from_version = "0.9"
# from_dir     = "build/myapp-0.9"

# CI/CD: pass keys as hex strings instead of file paths (each mutually exclusive
# with its path counterpart).
# priv_key_literal = "a1b2c3..."   # 64 hex chars (32 bytes)
# pub_key_literal  = "a1b2c3..."   # 64 hex chars (32 bytes)

# toolchain-free mode (optional) — see the toolchain chapter.
# With these set, pub_key above is IGNORED: the stub carries its own baked-in
# key, and priv_key / priv_key_literal must match it (pack self-verifies and fails if it doesn't).
# installer_stub = "kit/installer.exe"
# uninstaller    = "kit/uninstall.exe"

# remove unknown/leftover files on a Full install (upgrade/reinstall); patches ignore it
# purge_unknown_files = true

# dev (optional)
# force_reinstall = true
# reuse_stub      = true

# shortcuts to create (.lnk) - see the Shortcuts page. None created unless declared.
# [[shortcut]]
# dir = "%DESKTOP%"; name = "My App"; target = "%EXE%"; args = ""

# free-form registry keys (HKCU, or HKLM when machine-wide) - see the Registry keys page
# [[registry]]
# hive = "HKCU"; key = "%APP_KEY%"; name = "InstallDir"; type = "sz"; value = "%INSTALL_DIR%"

# feature packs: map path globs to a feature id a plugin activates - see Feature packs
# default = true installs it on a fresh install (a plugin can override at runtime)
# [[feature]]
# id = "Maps"; paths = ["data/maps", "extra/*.pak"]; default = true
```

> `[[shortcut]]`, `[[registry]]`, `[[plugin]]` and `[[feature]]` are
> arrays-of-tables: put all flat keys above them, then the table blocks at the
> end of the file (standard TOML ordering).

Run it:

```pwsh
# everything from the file
.\target\release\installer_builder.exe pack --config .\pack.toml

# file as a base, override one value on the CLI
.\target\release\installer_builder.exe pack --config .\pack.toml --to-version 1.1
```

## Required keys

Via CLI **or** file: `product`, `product_id`, `publisher`, `to_version`,
`input`, `exe`, `out`, and exactly one of `priv_key` / `priv_key_literal`. A
missing one fails with a message naming it. An invalid `product_id` (see [Full
installer](full.md)) also fails the build.

## Merge rules

- **Scalars / paths**: CLI value wins; otherwise the file value; otherwise the
  default (or an error if required).
- **`assoc` list**: a CLI `--assoc` list *replaces* the file's list when given
  (it does not merge).
- **Booleans** (`force_reinstall`, `purge_unknown_files`, `skip_license`,
  `skip_path`, `reuse_stub`): either source can turn them on.
- **`min_installer_version`**: defaults to `1.0.0` when set in neither.
- **`install_dir_restriction`**: a scalar (CLI wins, then file); defaults to
  `enforce`. Accepts `enforce` / `default_dir_only` / `bypass` (case- and
  `_`/`-`-insensitive).
- **`launch_option`**: a scalar (CLI wins, then file); defaults to `checked`.
  Accepts `checked` / `unchecked` / `hidden` (case-insensitive).

Next: [License, icon & version info](../packaging/branding.md).
