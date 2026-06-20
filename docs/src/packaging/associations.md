# File associations

Register file types so double-clicking a document opens your app. Pass
`--assoc ".ext:Description"` (repeatable). Associations are written under
`Software\Classes` and the shell `open` verb points at the installed
`manifest.exe` with `"%1"`.

> **Per-user vs machine-wide.** A per-user install writes under
> `HKCU\Software\Classes` (no admin). A **machine-wide install** (one that lands
> in a shared location such as `Program Files`, so it elevates) writes under
> `HKLM\Software\Classes` instead, so the association is visible to every user —
> not just the elevated account that ran the installer. The uninstaller cleans
> whichever hive was used.

```pwsh
installer_builder.exe pack `
    --product "MyApp" --product-id MyApp --publisher "My Company" --to-version 1.0 `
    --input .\build\myapp --exe myapp.exe `
    --assoc ".myx:MyApp Document" `
    --assoc ".myz:MyApp Archive" `
    --priv-key .\keys\priv.key --pub-key .\keys\pub.key `
    --out .\dist\setup-myapp-1.0.exe
```

In a [config file](../building/config.md) use an array:

```toml
assoc = [".myx:MyApp Document", ".myz:MyApp Archive"]
```

## Format

`.ext:Description`

- The extension is normalized to a single leading dot. An empty extension is
  rejected.
- The description may itself contain colons — only the **first** `:` splits ext
  from description. So `.a:b:c` → ext `.a`, description `b:c`.

## Keys written

Per association (ProgID = `<product-id>.<ext>`), under `HKCU` (per-user) or
`HKLM` (machine-wide install):

```text
HKCU\Software\Classes\.myx                          (default) = MyApp.myx
HKCU\Software\Classes\MyApp.myx                      (default) = MyApp Document
HKCU\Software\Classes\MyApp.myx\DefaultIcon          (default) = "<exe>",0
HKCU\Software\Classes\MyApp.myx\shell\open\command   (default) = "<exe>" "%1"
```

`SHChangeNotify(SHCNE_ASSOCCHANGED)` fires so Explorer refreshes immediately.

## Clean removal

The chosen associations are recorded in `installer_info.json`. The uninstaller
removes exactly those ProgID trees and clears each `.ext` default **only if it
still points at our ProgID** — it never stomps an association the user later
re-pointed at another app.

## Changing associations between versions

When you install over an existing copy of the product, the installer
**reconciles** associations: any extension the previous install registered but
the new payload no longer declares is unregistered, then the current set is
(re-)registered. So dropping `.myz` in a new version removes its handler instead
of leaving an orphan that would otherwise survive even an uninstall. Extensions
present in both versions are simply refreshed. This applies to full and patch
installers and to every UI mode.

The reconcile is failure-resilient. Associations are only touched after the new
version's files are committed, so an install that fails earlier never strips the
previous version's associations. And `installer_info.json` — the record of what
was registered — is rewritten *last*, after the registry changes, so an
interrupted install (crash / power loss) leaves the old record intact and the
next run recomputes and heals the association state rather than orphaning it.

Next: [Trimming the wizard](wizard.md).
