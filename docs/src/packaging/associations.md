# File associations

Register file types so double-clicking a document opens your app. Pass
`--assoc ".ext:Description"`, repeatable, or the `assoc` array in the
[config file](../building/config.md). Associations are written under
`Software\Classes`, and the shell `open` verb points at the installed main
executable with `"%1"`. Associations require `--exe` to be set.

```pwsh
installer_builder.exe pack `
    --product "MyApp" --product-id MyApp --publisher "My Company" --to-version 1.0 `
    --input .\build\myapp --exe myapp.exe `
    --assoc ".myx:MyApp Document" `
    --assoc ".myz:MyApp Archive" `
    --priv-key .\keys\priv.key --pub-key .\keys\pub.key `
    --out .\dist\setup-myapp-1.0.exe
```

In the config file:

```toml
assoc = [".myx:MyApp Document", ".myz:MyApp Archive"]
```

To pick the icon Explorer shows for the file type, use an `[[assoc]]` table
instead (config file only):

```toml
[[assoc]]
ext = ".myx"
description = "MyApp Document"
icon_index = 2            # third icon of the main exe

[[assoc]]
ext = ".myz"
description = "MyApp Archive"
icon = 'res\archive.dll'  # another file, relative to the install folder
icon_id = 101             # icon resource with id 101
```

## Format

A string entry is `.ext:Description`.

- The extension is normalized to a single leading dot. An empty extension is
  rejected.
- Only the first `:` splits the extension from the description, so the
  description may itself contain colons: `.a:b:c` gives extension `.a` and
  description `b:c`.

A table entry has these keys. Unknown keys are rejected.

| Key | Required | Meaning |
|---|---|---|
| `ext` | yes | Extension, normalized as above. |
| `description` | yes | Friendly type name shown in Explorer. |
| `icon` | no | File holding the icon: an `.exe`, `.dll` or `.ico`. Default `%EXE%` (the main executable). Accepts the tokens `%INSTALL_DIR%`, `%EXE%`, `%VERSION%`, `%PRODUCT%`, `%PRODUCT_ID%`, `%PUBLISHER%` (as for [shortcuts](shortcuts.md)), then `%VAR%` environment variables. A relative path resolves against the install folder. Use single quotes so backslashes stay literal. |
| `icon_index` | no | 0-based position of the icon in `icon`, in the order Windows lists them. Default `0`. An `.ico` file only has index `0`. |
| `icon_id` | no | Numeric resource id of the icon group in `icon`. Written as `-<id>`. |

Set at most one of `icon_index` and `icon_id`. This matches InstallShield's
"Icon file" and "Icon index" fields; a negative InstallShield index is an
`icon_id` here.

TOML does not allow `assoc = [...]` and `[[assoc]]` in the same file. To mix
strings and tables, use one inline array:
`assoc = [".old:Old", { ext = ".new", description = "New", icon_index = 1 }]`.
A CLI `--assoc` list still replaces the config list and has no icon options.

When the icon lives inside the package (the default, `%EXE%`,
`%INSTALL_DIR%\...` or a relative path), the builder checks it and prints a
warning if the file is missing, or the index or id does not exist in it. Icons
outside the package, like `%SystemRoot%\System32\shell32.dll`, are not checked.

## Keys written

Per association, with ProgID `<product-id>.<ext>`:

```text
Software\Classes\.myx                          (default) = MyApp.myx
Software\Classes\MyApp.myx                     (default) = MyApp Document
Software\Classes\MyApp.myx\DefaultIcon         (default) = "<icon>",<index>
Software\Classes\MyApp.myx\shell\open\command  (default) = "<exe>" "%1"
```

`<icon>` is the resolved `icon` path, the main exe by default. `<index>` is
`icon_index`, or `-<icon_id>`, or `0` when neither is set.

A per-user install writes these under `HKCU`; a machine-wide install writes
them under `HKLM`, so the association is visible to every user. See
[Per-user and machine-wide installs](../running/machine-wide.md). The
uninstaller cleans whichever hive was used.

After registration, the installer fires
`SHChangeNotify(SHCNE_ASSOCCHANGED)` so Explorer refreshes immediately.

## Clean removal

The chosen associations are recorded in `installer_info.json`. The
uninstaller removes exactly those ProgID trees, and clears each `.ext`
default only if it still points at our ProgID. It never stomps an
association the user later re-pointed at another app.

## Changing associations between versions

When you install over an existing copy of the product, the installer
reconciles associations. Any extension the previous install registered but
the new payload no longer declares is unregistered, then the current set is
registered. Dropping `.myz` in a new version therefore removes its handler
instead of leaving an orphan. Extensions present in both versions are simply
refreshed, including a changed icon. This applies to full and patch
installers and to every UI mode.

The reconciliation is failure-resilient. Associations are only touched after
the new version's files are committed, so an install that fails earlier never
strips the previous version's associations. And `installer_info.json`, the
record of what was registered, is rewritten last, after the registry changes.
An interrupted install (crash or power loss) leaves the old record intact,
and the next run recomputes and heals the association state.
