# Branding: license, banner, icon & version

Four packaging options make the setup `.exe` look finished and legally
complete: the **license** text, a custom **header banner**, the **icon** (the
app's own logo, inherited automatically) and the **version info**. None are
required; all are driven from `pack` (CLI or `--config`).

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

## Header banner

By default the wizard's header is a flat light-gray card with the product title
and a sub-line. Pass `--banner <path.png>` to paint your own image across that
whole strip instead — a quick way to brand the installer without touching code.

```pwsh
installer_builder.exe pack `
    --product "My App" --product-id myapp --publisher "My Company" --to-version 1.0 `
    --input .\build\myapp --exe myapp.exe `
    --banner .\branding\header-1400x144.png `
    --priv-key .\keys\priv.key --pub-key .\keys\pub.key `
    --out .\dist\setup-myapp-1.0.exe
```

A ready-made sample is in the repo at `docs/src/images/banner-sample.png`:

![Sample header banner](../images/banner-sample.png)

How it behaves:

- **Optional.** Omit `--banner` and the header stays the default gray card. The
  flag (or the `banner` config key) is the *only* thing that turns it on.
- **PNG only**, validated at build time (the file must start with the PNG
  signature, or the build fails). Transparency is supported.
- **Packaged in the `.exe`** as a dedicated resource — no external file, no Rust
  toolchain needed at install time (it works the same in
  [toolchain-free mode](../building/toolchain.md)).
- **Crisp at every DPI.** The image is stretched to the header at runtime with
  high-quality scaling, so the *same* file renders sharp at 100 %, 125 %, 150 %
  and 200 % scale. Author it at **2× the logical header size** —
  **1400 × 144 px** (the header is 700 × 72 logical px) — for a pixel-perfect
  result on high-DPI screens.
- **Keep the left edge light.** The product title and sub-line are drawn *on top*
  of the banner in dark text, anchored to the left. Use a light or low-contrast
  left third so the title stays readable; busier art belongs on the right (as in
  the sample above).

> The banner is pure branding, so — unlike the license — it rides as its own raw
> resource rather than inside the Ed25519-signed manifest. It is not covered by
> the signature; sign the final `.exe` with Authenticode to seal the whole file
> (see [Signing](signing.md)).

### Previewing a banner

Iterate on a banner without packing a full installer using the debug build's
preview window — point it at any PNG via an environment variable:

```pwsh
cargo build -p installer
$env:INSTALLWAY_PREVIEW_BANNER = ".\branding\header-1400x144.png"
.\target\debug\installer.exe --preview license
```

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
