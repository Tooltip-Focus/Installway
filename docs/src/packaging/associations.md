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

## Format

Each entry is `.ext:Description`.

- The extension is normalized to a single leading dot. An empty extension is
  rejected.
- Only the first `:` splits the extension from the description, so the
  description may itself contain colons: `.a:b:c` gives extension `.a` and
  description `b:c`.

## Keys written

Per association, with ProgID `<product-id>.<ext>`:

```text
Software\Classes\.myx                          (default) = MyApp.myx
Software\Classes\MyApp.myx                     (default) = MyApp Document
Software\Classes\MyApp.myx\DefaultIcon         (default) = "<exe>",0
Software\Classes\MyApp.myx\shell\open\command  (default) = "<exe>" "%1"
```

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
refreshed. This applies to full and patch installers and to every UI mode.

The reconciliation is failure-resilient. Associations are only touched after
the new version's files are committed, so an install that fails earlier never
strips the previous version's associations. And `installer_info.json`, the
record of what was registered, is rewritten last, after the registry changes.
An interrupted install (crash or power loss) leaves the old record intact,
and the next run recomputes and heals the association state.
