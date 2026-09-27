# Portable kit and Docker images

Every Installway release ships ready-made ways to build installers without
installing Rust or checking out the sources:

| | Portable kit | Docker, prebuilt | Docker, toolchain |
|---|---|---|---|
| Where | Release asset `installway-kit-<version>-x64.zip` | `ghcr.io/tooltip-focus/installway` | `ghcr.io/tooltip-focus/installway:toolchain` |
| Runs on | Any Windows machine | Windows Docker host | Windows Docker host |
| Signing key | Shared, bundled with the kit | Shared, bundled with the image | **Yours**, passed at run time |
| Build time | Seconds | Seconds | A minute or two (the stub is compiled) |

All three read the same project file, `installway.toml`: the
[config file](config.md) of `pack`, without the key and stub entries, which
they supply themselves. Paths in it are relative to the folder you run from.

> **The shared key is public.** The kit and the prebuilt image carry the same
> signing key for everyone, so the Ed25519 signature of their installers does
> not prove who built them: anyone can produce an installer these stubs
> accept. File integrity (BLAKE3) is still checked, and your
> [Authenticode signature](../packaging/signing.md) remains what identifies
> you as the publisher. For your own key, use the toolchain image, or build a
> kit yourself as described in [Packaging without the Rust toolchain](toolchain.md).

## The project file

A minimal `installway.toml`, next to the files it references:

```toml
product    = "My App"
product_id = "myapp"
publisher  = "My Company"
to_version = "1.0.0"
input      = "build"
exe        = "myapp.exe"
out        = "dist/setup-myapp-1.0.0.exe"
```

The kit contains a commented `installway.example.toml` to start from. Any
other [config key](config.md#key-reference) works, including patch mode
(`from_version`, `from_dir`) and the `[[shortcut]]`, `[[registry]]`,
`[[plugin]]` and `[[feature]]` tables.

## Portable kit

Download `installway-kit-<version>-x64.zip` from the
[releases page](https://github.com/Tooltip-Focus/Installway/releases), then
unblock the extracted files so Windows lets them run:

```pwsh
Expand-Archive .\installway-kit-v1.0.0-x64.zip C:\tools\installway
Get-ChildItem C:\tools\installway | Unblock-File
```

From your project folder, run `pack.cmd` (or `pack.ps1` from PowerShell):

```pwsh
C:\tools\installway\pack.cmd
C:\tools\installway\pack.ps1 --config .\other.toml --to-version 1.2.3
```

- The config file is `installway.toml` in the current folder, unless you pass
  `--config <file>` or set `INSTALLWAY_CONFIG`.
- Any other argument goes to `installer_builder pack` and overrides the
  config file, as described in [merge rules](config.md#merge-rules).

The kit contains:

```text
installer_builder.exe   the packer
installer.exe           installer stub
uninstall.exe           uninstaller
installer-hintway.exe   installer stub with Hintway install analytics
uninstall-hintway.exe   uninstaller with Hintway install analytics
hdiffz.exe              binary diff tool, for patch installers
priv.key, pub.key       the kit's shared signing key pair
pack.ps1, pack.cmd      wrappers around `installer_builder pack`
```

## Hintway variant

The stub and the uninstaller only contain [Hintway](hintway.md) support when
compiled with it, so the kit and the images pick the right build from the
config file. With `hintway_tenant_id` set, they use the Hintway-enabled
variant; without it, the plain one, which contains no Hintway code. The first
output line says which one:

```text
Hintway: on (installer-hintway.exe, uninstall-hintway.exe)
```

## Docker images

Both images are Windows containers based on
`mcr.microsoft.com/windows/servercore:ltsc2022`: they need a Windows Docker
host, such as a GitHub Actions `windows-2022` runner or a Windows GitLab
runner. On a newer Windows host, add `--isolation=hyperv` to `docker run`.

Mount your project folder at `C:\work`. The installer is written where `out`
points, so inside the mounted folder:

```pwsh
docker run --rm -v "${PWD}:C:\work" ghcr.io/tooltip-focus/installway:1.0
```

As with the kit, `--config <file>` selects another config file, other
arguments go to `pack`, and `INSTALLWAY_CONFIG` can be set with `-e`.

### Prebuilt image

`ghcr.io/tooltip-focus/installway` is the portable kit in a container. It
needs no key: installers are signed with the shared key.

### Toolchain image, your own key

`ghcr.io/tooltip-focus/installway:toolchain` contains Rust, MSVC and the
Installway sources, with every dependency already compiled. Each run compiles
the stub with your public key, then packs and signs with your private key.
Generate a key pair once (see [Signing keys](../getting-started/signing-keys.md)),
store both as CI secrets, and pass them as environment variables:

```pwsh
docker run --rm -v "${PWD}:C:\work" `
    -e INSTALLWAY_PRIV_KEY=$env:MYAPP_PRIV_KEY `
    -e INSTALLWAY_PUB_KEY=$env:MYAPP_PUB_KEY `
    ghcr.io/tooltip-focus/installway:1.0-toolchain
```

The image can also generate the key pair:

```pwsh
docker run --rm -v "${PWD}:C:\work" `
    --entrypoint C:\src\target\release\installer_builder.exe `
    ghcr.io/tooltip-focus/installway:toolchain keygen --out C:\work\keys
```

The image is large (several GB, mostly Visual Studio Build Tools). The run
itself needs no network access.

### Tags

| Tag | Image |
|---|---|
| `1.2.3`, `1.2`, `latest` | Prebuilt |
| `1.2.3-toolchain`, `1.2-toolchain`, `toolchain` | Toolchain |

Pre-release versions (`1.2.3-rc.1`) are only tagged with their exact version.

## CI examples

GitHub Actions:

```yaml
jobs:
  installer:
    runs-on: windows-2022
    steps:
      - uses: actions/checkout@v4
      # ... build your application into build/ ...
      - name: Build the installer
        shell: pwsh
        env:
          MYAPP_PRIV_KEY: ${{ secrets.MYAPP_PRIV_KEY }}
          MYAPP_PUB_KEY: ${{ secrets.MYAPP_PUB_KEY }}
        run: |
          docker run --rm -v "${PWD}:C:\work" `
            -e INSTALLWAY_PRIV_KEY=$env:MYAPP_PRIV_KEY `
            -e INSTALLWAY_PUB_KEY=$env:MYAPP_PUB_KEY `
            ghcr.io/tooltip-focus/installway:1.0-toolchain
          if ($LASTEXITCODE) { exit $LASTEXITCODE }
      - uses: actions/upload-artifact@v4
        with:
          name: installer
          path: dist/*.exe
```

GitLab CI, on a Windows runner with the Docker executor and the `powershell`
shell (the images only contain Windows PowerShell):

```yaml
installer:
  image:
    name: ghcr.io/tooltip-focus/installway:1.0
    entrypoint: [""]
  tags: [windows]
  script:
    - powershell -NoProfile -ExecutionPolicy Bypass -File C:\installway\entrypoint.ps1
  artifacts:
    paths: [dist/*.exe]
```

GitLab runs the job in the checked-out project folder, so the script calls
the entrypoint itself instead of relying on the `C:\work` mount. With the
toolchain image, define `INSTALLWAY_PRIV_KEY` and `INSTALLWAY_PUB_KEY` as
masked CI/CD variables.

Sign the produced installer with Authenticode in a following step, on a
machine that holds your certificate.
