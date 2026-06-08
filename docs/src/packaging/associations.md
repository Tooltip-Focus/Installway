# File associations

Register file types so double-clicking a document opens your app. Pass
`--assoc ".ext:Description"` (repeatable). Associations are written under
`HKCU\Software\Classes` — **per-user, no admin** — and the shell `open` verb
points at the installed `manifest.exe` with `"%1"`.

```pwsh
installer_builder.exe pack `
    --product MyApp --publisher "My Company" --to-version 1.0 `
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

Per association (ProgID = `<sanitized-product>.<ext>`):

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

Next: [Trimming the wizard](wizard.md).
