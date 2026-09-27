# Pack an installer with the portable Installway kit (no Rust toolchain needed).
#
#   .\pack.ps1                              uses .\installway.toml
#   .\pack.ps1 --config .\other.toml        another config file
#   .\pack.ps1 --to-version 1.2.3           extra arguments go to `installer_builder pack`
#
# The config file can also be set with $env:INSTALLWAY_CONFIG. The stub and
# uninstaller are picked from the config: the Hintway-enabled pair when it sets
# `hintway_tenant_id`, the plain pair otherwise. The payload is signed with the
# kit's priv.key, which matches the public key compiled into both stubs.
#
# No param() block on purpose: Windows PowerShell would bind a leading
# `--to-version` positionally, so `--config` is picked out of $args by hand.
$ErrorActionPreference = 'Stop'

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
    throw "Config file '$config' not found in $(Get-Location). Run pack.ps1 from your project folder, or pass --config <file>."
}

# `hintway_tenant_id` is a flat, config-file-only key; a commented-out line does not count.
$hintway = (Get-Content -LiteralPath $config -Raw) -match '(?m)^\s*hintway_tenant_id\s*='
$suffix = if ($hintway) { '-hintway' } else { '' }
$stub = Join-Path $PSScriptRoot "installer$suffix.exe"
$uninstaller = Join-Path $PSScriptRoot "uninstall$suffix.exe"
"Hintway: $(if ($hintway) { 'on' } else { 'off' }) (installer$suffix.exe, uninstall$suffix.exe)"

& (Join-Path $PSScriptRoot 'installer_builder.exe') pack `
    --config $config `
    --installer-stub $stub `
    --uninstaller $uninstaller `
    --priv-key (Join-Path $PSScriptRoot 'priv.key') `
    @packArgs
exit $LASTEXITCODE
