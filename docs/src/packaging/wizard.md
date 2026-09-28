# Wizard pages and install location

The interactive installer is a four-step wizard: License, Choose location,
Progress, Done. This page covers everything that shapes that flow at build
time: which pages appear, where the install lands, what the destination-folder
guard allows, and what happens on the final page.

All options below work on the CLI and as
[config file](../building/config.md) keys.

## Skip pages

| Option | Effect |
|---|---|
| `--skip-license` | Hide the License page. |
| `--skip-path` | Hide the Choose-location page; install straight to the default location. When the license is still shown, its button reads Install. |
| `--allow-force-path-window-state` | Let the setup be run with `--force-show-path-window` to bring back the Choose-location page hidden by `--skip-path`. Off by default. |

With both skip flags, the wizard goes straight to Progress on launch:

```pwsh
installer_builder.exe pack `
    --product "My App" --product-id myapp --publisher "My Company" --to-version 1.0 `
    --input .\build\myapp --exe myapp.exe `
    --skip-license --skip-path `
    --default-install-dir "%LOCALAPPDATA%\Programs\MyApp" `
    --priv-key .\keys\priv.key --pub-key .\keys\pub.key `
    --out .\dist\setup-myapp-1.0.exe
```

### Showing the Choose-location page on demand

`--skip-path` suits most users, but support staff or advanced users sometimes
need to install elsewhere. Build with both flags:

```pwsh
installer_builder.exe pack ... --skip-path --allow-force-path-window-state
```

The setup still hides the Choose-location page by default, and shows it when
launched with the runtime flag:

```pwsh
setup-myapp-1.0.exe --force-show-path-window
```

- Without `--allow-force-path-window-state` at build time,
  `--force-show-path-window` is silently ignored.
- It only lifts `--skip-path`. A reinstall or upgrade still skips the page
  (see below).
- It has no effect on `--silent` and `--minimal`, which have no wizard; pass
  the target folder as their positional argument instead.
- The page shown this way applies the usual
  [non-empty-folder guard](#the-non-empty-folder-guard).
- In [toolchain-free mode](../building/toolchain.md), the prebuilt stub must
  be recent enough to know the flag; an older stub rejects it.

Building with `--allow-force-path-window-state` but without `--skip-path`
prints a warning: the page is already shown, so the flag has nothing to do.

## The proposed install location

`--default-install-dir <DIR>` sets the path the Choose page proposes. It may
contain `%VAR%` environment tokens, for example
`%LOCALAPPDATA%\Programs\MyApp` or `C:\Games\MyApp`.

At runtime, the installer picks the proposed location in this order:

1. An explicit path argument (`--silent "<dir>"`, `--minimal "<dir>"`, or the
   `INSTALLWAY_PATH` environment variable).
2. The folder the product was last installed to. A reinstall or upgrade lands
   in place, and the Choose page is skipped automatically.
3. The build's `--default-install-dir`, with `%VAR%` tokens expanded.
4. `%LOCALAPPDATA%\Programs\<product>`.

A reinstall or upgrade always skips the Choose page, for full and patch
installers alike, regardless of `--skip-path`. A patch must land in the
existing folder because it patches the files on disk, and a full reinstall
there avoids an accidental second copy elsewhere. The build-time
`--skip-path` only affects first installs. Detection is keyed by `publisher`
plus `product`, so it works across versions.

## The non-empty-folder guard

By default, a fresh interactive install refuses a destination folder that is
not empty, which protects users from extracting into `C:\Users\name\Documents`
by accident. `--install-dir-restriction` tunes this:

| Value | Effect |
|---|---|
| `enforce` | Block any non-empty destination (default). |
| `default-dir-only` | Allow only the build's `--default-install-dir` to be non-empty. |
| `bypass` | Allow any folder. |

Use `default-dir-only` when replacing a legacy InstallShield or MSI install
that lives in its own fixed directory. Pair it with `--purge-unknown-files`
and a [plugin](plugins.md) that validates the old install before install and
tears it down at uninstall. What the uninstaller then removes from that folder
is set by the [install folder policy](../running/uninstall.md#the-install-folder).

## The "launch now" checkbox

The Done page shows a "launch the product now" checkbox. `--launch-option`
controls it:

| Value | Effect |
|---|---|
| `checked` | Visible and ticked (default). The product launches on Finish unless the user clears it. |
| `unchecked` | Visible but not ticked. The user opts in. |
| `hidden` | No checkbox. The installer never offers to launch the product. |

This only affects the interactive wizard's Done page. Silent and minimal
installs decide launching with the runtime
[`--launch`](../reference/installer-cli.md) flag instead.

## Minimal UI for upgrades

`--upgrade-minimal-ui` makes an upgrade use the compact
[minimal UI](../running/install.md#minimal-app-triggered-self-update), the
small "Applying update" window, instead of the full wizard. It is off by
default.

| Run | UI with `--upgrade-minimal-ui` set |
|---|---|
| First install (no prior copy) | Full wizard, always. |
| Upgrade or reinstall over an existing copy | Minimal UI. |
| `--silent` / `--minimal` | Unchanged; the flag has no effect. |

It works on full and patch installers alike. The choice is read from the
payload of the installer being run, so it applies to the next installer that
carries the flag, never retroactively to the copy already on disk.

An upgrade shown in the minimal UI installs into the existing folder and
launches the app afterward only if `--launch` is passed. There is no Done
page and no launch checkbox.
