# License, icon & version info

Three packaging options make the setup `.exe` look finished and legally
complete. None are required; all are driven from `pack` (CLI or `--config`).

## License text

Pass `--license <path>` and the UTF-8 text in that file becomes the EULA shown
on the installer's **License** page. Omitting it falls back to a built-in
lorem-ipsum placeholder.

```pwsh
installer_builder.exe pack `
    --product "My App" --product-id myapp --publisher "My Company" --to-version 1.0 `
    --input .\build\myapp --exe myapp.exe `
    --license .\legal\EULA-myapp-en.txt `
    --priv-key .\keys\priv.key --pub-key .\keys\pub.key `
    --out .\dist\setup-myapp-1.0.exe
```

The text rides **inside** the signed `InstallerPayload`, so tampering with it
invalidates the Ed25519 signature. `--verify` reports `License: custom
(<bytes>)` or `License: built-in placeholder`.

To hide the License page entirely, see [Trimming the wizard](wizard.md).

## Icon inheritance

At pack time the builder reads the icon resources (`RT_GROUP_ICON` + every
referenced `RT_ICON`) from `<input>\<exe>` — the application being packaged —
and stamps them into **both** the setup `.exe` and the embedded
`uninstall.exe`. Result: Explorer shows the application's own icon on the
installer and uninstaller files, and on the Add/Remove Programs entry.

- No flag needed — it happens automatically when `<input>\<exe>` has icons.
- The uninstaller is stamped on a `%TEMP%` staging copy, so the cached
  `target\release\uninstall.exe` is left untouched between pack runs.
- If the source exe has no icon resources, the build prints a notice and falls
  back to the default icon.

## Version info

Pack stamps a Win32 `VS_VERSIONINFO` (RT_VERSION) resource built from
`--product` / `--publisher` / `--to-version`. Explorer's **Details** tab then
shows FileVersion, ProductVersion, ProductName, CompanyName, FileDescription
(`<product> Setup`), OriginalFilename and copyright — a complete binary that
builds SmartScreen reputation.

`--to-version` is parsed as `a.b.c.d` (missing parts = 0), so `1.2` becomes
`1.2.0.0`.

Next: [File associations](associations.md).
