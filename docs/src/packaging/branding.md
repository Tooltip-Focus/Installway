# Branding

Four packaging options make the setup `.exe` look finished and legally
complete: the license text, a custom header banner, the icon (inherited from
your application automatically), and the version-info resource. None are
required. All are driven from `pack`, on the CLI or in the
[config file](../building/config.md).

## License text

Pass `--license <path>` and the UTF-8 text in that file becomes the EULA shown
on the installer's License page. Without it, the page shows a built-in
placeholder text.

```pwsh
installer_builder.exe pack `
    --product "My App" --product-id myapp --publisher "My Company" --to-version 1.0 `
    --input .\build\myapp --exe myapp.exe `
    --license .\legal\EULA-myapp-en.txt `
    --priv-key .\keys\priv.key --pub-key .\keys\pub.key `
    --out .\dist\setup-myapp-1.0.exe
```

The text rides inside the signed payload, so tampering with it invalidates
the Ed25519 signature. `--verify` reports `License: custom (<bytes>)` or
`License: built-in placeholder`.

To hide the License page entirely, see
[Wizard pages and install location](wizard.md).

## Header banner

By default, the wizard's header is a flat light-gray card with the product
title and a sub-line. Pass `--banner <path.png>` to paint your own image
across that whole strip instead.

```pwsh
installer_builder.exe pack `
    --product "My App" --product-id myapp --publisher "My Company" --to-version 1.0 `
    --input .\build\myapp --exe myapp.exe `
    --banner .\branding\header-1400x144.png `
    --priv-key .\keys\priv.key --pub-key .\keys\pub.key `
    --out .\dist\setup-myapp-1.0.exe
```

A ready-made sample lives in the repo at `docs/src/images/banner-sample.png`:

![Sample header banner](../images/banner-sample.png)

How it behaves:

- **Optional.** Omit `--banner` and the header stays the default gray card.
- **PNG only.** The file must start with the PNG signature or the build
  fails. Transparency is supported.
- **Packaged in the `.exe`** as a dedicated resource. There is no external
  file, and it works the same in
  [toolchain-free mode](../building/toolchain.md).
- **Crisp at every DPI.** The image is stretched to the header at runtime
  with high-quality scaling. Author it at twice the logical header size,
  1400 x 144 px (the header is 700 x 72 logical px), for a sharp result at
  100%, 125%, 150%, and 200% display scale.
- **Keep the left edge light.** The product title and sub-line are drawn on
  top of the banner in dark text, anchored to the left. Use a light or
  low-contrast left third so the title stays readable; busier art belongs on
  the right, as in the sample above.

The banner is pure branding, so unlike the license it rides as its own raw
resource rather than inside the signed manifest. Sign the final `.exe` with
Authenticode to seal the whole file; see [Authenticode signing](signing.md).

### Previewing a banner

You can iterate on a banner without packing a full installer. The debug build
of the stub has a preview window that reads any PNG from an environment
variable:

```pwsh
cargo build -p installer
$env:INSTALLWAY_PREVIEW_BANNER = ".\branding\header-1400x144.png"
.\target\debug\installer.exe --preview license
```

## Icon inheritance

At pack time, the builder reads the icon resources from `<input>\<exe>` (your
application) and stamps them into both the setup `.exe` and the embedded
`uninstall.exe`. Explorer then shows your application's own icon on the
installer and uninstaller files, and on the Windows Apps entry.

- No flag is needed. It happens automatically when `<input>\<exe>` has icon
  resources.
- If the source exe has no icon resources, the build prints a notice and
  falls back to the default icon.
- The uninstaller is stamped on a staging copy in `%TEMP%`, so the cached
  `target\release\uninstall.exe` stays untouched between pack runs.

## Version info

`pack` stamps a Win32 `VS_VERSIONINFO` resource built from `--product`,
`--publisher`, and `--to-version`. Explorer's Details tab then shows
FileVersion, ProductVersion, ProductName, CompanyName, FileDescription
(`<product> Setup`), OriginalFilename, and copyright. A complete version
resource makes the binary look finished and helps build SmartScreen
reputation.

`--to-version` is parsed as `a.b.c.d` with missing parts set to zero, so
`1.2` becomes `1.2.0.0`.
