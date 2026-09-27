# Entrypoint of the Installway Docker images. Packs the project mounted at
# C:\work from its installway.toml (or --config <file> / $env:INSTALLWAY_CONFIG);
# other `docker run` arguments are passed to `installer_builder pack`.
#
# - Prebuilt image (the portable kit in C:\installway): delegates to the kit's
#   pack.ps1, which signs with the kit's shared key.
# - Toolchain image (sources in C:\src): builds the stub and the uninstaller
#   with $env:INSTALLWAY_PUB_KEY compiled in, then packs with them and signs
#   with $env:INSTALLWAY_PRIV_KEY. Built here rather than by `pack` itself,
#   which looks for the Cargo workspace from the current directory (C:\work).
$ErrorActionPreference = 'Stop'

$kitPack = 'C:\installway\pack.ps1'
if (Test-Path $kitPack) {
    & $kitPack @args
    exit $LASTEXITCODE
}

$src = 'C:\src'
$release = Join-Path $src 'target\release'

foreach ($name in 'INSTALLWAY_PRIV_KEY', 'INSTALLWAY_PUB_KEY') {
    if (-not [Environment]::GetEnvironmentVariable($name)) {
        throw "$name is not set. Pass your key pair with: docker run -e INSTALLWAY_PRIV_KEY=<hex> -e INSTALLWAY_PUB_KEY=<hex> ..."
    }
}

# Same --config handling as the kit's pack.ps1 (no param() block, see there).
$config = if ($env:INSTALLWAY_CONFIG) { $env:INSTALLWAY_CONFIG } else { 'installway.toml' }
$packArgs = @()
for ($i = 0; $i -lt $args.Count; $i++) {
    if ($args[$i] -in '--config', '-Config') {
        $i++
        if ($i -ge $args.Count) { throw "$($args[$i - 1]) needs a file path" }
        $config = $args[$i]
    } else {
        $packArgs += $args[$i]
    }
}
if (-not (Test-Path -LiteralPath $config -PathType Leaf)) {
    throw "Config file '$config' not found in $(Get-Location). Mount your project with: docker run -v <project dir>:C:\work ..."
}

$hintway = (Get-Content -LiteralPath $config -Raw) -match '(?m)^\s*hintway_tenant_id\s*='
"Hintway: $(if ($hintway) { 'on' } else { 'off' }) (stub built with your public key)"

# Dependencies of both variants were compiled into the image, so only the
# stub (its build script tracks INSTALLER_PUB_KEY) is rebuilt; no network needed.
$cargoArgs = @('build', '--release', '--locked', '--offline', '-p', 'installer', '-p', 'uninstaller')
if ($hintway) { $cargoArgs += '--features', 'installer/hintway,uninstaller/hintway' }
$env:INSTALLER_PUB_KEY = $env:INSTALLWAY_PUB_KEY.Trim().ToLowerInvariant()
Push-Location $src
try { cargo @cargoArgs }
finally {
    Pop-Location
    Remove-Item Env:\INSTALLER_PUB_KEY -ErrorAction Ignore
}
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

& (Join-Path $release 'installer_builder.exe') pack `
    --config $config `
    --installer-stub (Join-Path $release 'installer.exe') `
    --uninstaller (Join-Path $release 'uninstall.exe') `
    --priv-key-literal $env:INSTALLWAY_PRIV_KEY.Trim() `
    @packArgs
exit $LASTEXITCODE
