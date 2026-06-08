# Trimming the wizard

The interactive installer is a four-step wizard: **License → Choose location →
Progress → Done**. Two build-time flags trim it, and one option sets the
default location.

| Option | Effect |
|---|---|
| `--skip-license` | Hide the License page (step 1). |
| `--skip-path` | Hide the Choose-location page (step 2); install straight to the default location. With this on (and license shown), the License button reads **Install**. |
| `--default-install-dir <DIR>` | The path the Choose page proposes. May contain `%VAR%` env tokens, e.g. `%LOCALAPPDATA%\Programs\MyApp` or `C:\Games\MyApp`. |

With **both** skip flags, the wizard goes straight to Progress on launch.

```pwsh
installer_builder.exe pack `
    --product myapp --publisher "My Company" --to-version 1.0 `
    --input .\build\myapp --exe myapp.exe `
    --skip-license --skip-path `
    --default-install-dir "%LOCALAPPDATA%\Programs\MyApp" `
    --priv-key .\keys\priv.key --pub-key .\keys\pub.key `
    --out .\dist\setup-myapp-1.0.exe
```

All three are also [config-file](../building/config.md) keys
(`skip_license`, `skip_path`, `default_install_dir`).

## Minimal UI for upgrades (optional)

`--upgrade-minimal-ui` makes an **upgrade** use the compact
[minimal UI](../running/install.md#minimal-app-triggered-self-update) — the
"Applying update" window — instead of the full wizard. It's off by default.

| Run | UI with `--upgrade-minimal-ui` set |
|---|---|
| First install (no prior copy) | Full wizard — **always** |
| Upgrade / reinstall over an existing copy | Minimal UI |
| `--silent` / `--minimal` | Unchanged (flag has no effect) |

```pwsh
installer_builder.exe pack `
    --product myapp --publisher "My Company" --to-version 1.1 `
    --input .\build\myapp-1.1 --exe myapp.exe `
    --upgrade-minimal-ui `
    --priv-key .\keys\priv.key --pub-key .\keys\pub.key `
    --out .\dist\setup-myapp-1.1.exe
```

Works on **full and patch** installers alike. The choice is read from the
payload of the installer being run, so it applies to the next full install or
patch that carries the flag — never retroactively to the copy already on disk.
The TOML key is `upgrade_minimal_ui`.

> An upgrade shown in the minimal UI installs into the existing folder and
> launches the app afterward only if `--launch` is passed — there is no
> Done-page "Run program now" checkbox.

## Proposed install location, in priority order

1. An explicit path argument (`--silent` / `--minimal "<dir>"`, or the
   `INSTALLWAY_PATH` env var).
2. **The folder the product was last installed to** — a reinstall/upgrade lands
   in place, and the Choose page is skipped automatically.
3. The build's `--default-install-dir` (with `%VAR%` tokens expanded).
4. `%LOCALAPPDATA%\Programs\<product>`.

> **Reinstall/upgrade always skips the Choose page**, for both full and patch
> installers, regardless of `--skip-path`. A patch *must* land in the existing
> folder (it patches the files on disk); a full reinstall there avoids an
> accidental second copy elsewhere. The build-time `--skip-path` only affects
> *first* installs. Detection is keyed by `publisher` + `product`, so it works
> across versions.

Next: [Signing (Authenticode)](signing.md).
