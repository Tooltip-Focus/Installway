# Installer runtime flags

Complete reference for the flags of a built installer (`setup-*.exe`) and of
the uninstaller (`uninstall.exe`). For the build tool, see
[Builder CLI](cli.md).

## Installer (`setup-*.exe`)

```pwsh
setup-myapp.exe [<install-dir>] [flags]
```

The optional positional `<install-dir>` sets the target directory for
`--silent` and `--minimal` runs.

| Flag | Description |
|---|---|
| `--silent` | Headless install. Progress prints to stdout; see [Install modes](../running/install.md#silent). |
| `--minimal` | Compact self-update UI; see [Install modes](../running/install.md#minimal-app-triggered-self-update). |
| `--launch` | Launch the installed exe after a successful silent or minimal install. |
| `--verify` | Verify the embedded payload and signature, print a summary, and exit without installing. |
| `--verify-install "<dir>"` | Re-hash an installed copy against its recorded manifest. Reports `OK`, `MISSING`, or `CORRUPT` per file. |
| `--lang <code>` | Force the UI language, for example `fr`. See below. |
| `--ignore-desktop-shortcuts` | Do not create desktop shortcuts, in any mode. See [Shortcuts](../packaging/shortcuts.md#suppressing-shortcuts-at-install-time). |
| `--ignore-start-menu-shortcuts` | Do not create Start Menu shortcuts, in any mode. |

The installer also accepts internal, hidden flags (`--elevated-worker`,
`--run-plugin`) that it passes to its own subprocesses for elevation and
plugin isolation. They are not meant to be called manually.

### Environment variables

| Variable | Effect |
|---|---|
| `INSTALLWAY_PATH` | Target install directory for `--silent` and `--minimal` when no positional path is given. |
| `INSTALLWAY_LANG` | UI language code, used when `--lang` is absent. |

### Target directory resolution

For `--silent` and `--minimal`, the target directory is resolved in this
order: the positional `<install-dir>` argument, then `INSTALLWAY_PATH`, then
the folder the product was last installed to, then the build's default
install directory, then `%LOCALAPPDATA%\Programs\<product>`.

### Language selection

The UI language is resolved in this order: `--lang`, then `INSTALLWAY_LANG`,
then the OS display language, then English. Supported languages: English
(`en`), French (`fr`), Italian (`it`). An unsupported code falls back to
English. The resolved code is passed to [plugins](../packaging/plugins.md)
as `ctx->lang`.

## Uninstaller (`uninstall.exe`)

Normally invoked from Windows Apps. It supports:

| Flag | Description |
|---|---|
| `--silent` | Skip the confirmation dialog. This is what `QuietUninstallString` invokes. |
| `--lang <code>` | Force the UI language, same resolution as the installer. |

Like the installer, it has internal hidden flags for its elevation worker,
plugin host, and second-stage cleanup. They are not meant to be called
manually.

## Exit codes

See [Exit codes](exit-codes.md).
